use std::{sync::Arc, net::SocketAddr};

use ip_in_subnet::iface_in_subnet;
use log::{debug, info, error, warn};
use tokio::{sync::{RwLock, mpsc::{UnboundedSender, UnboundedReceiver, self}}, select, net::{TcpStream, TcpListener}, io::{AsyncReadExt, AsyncWriteExt}};

use crate::{Message, parse_protocol, Context, Connection, http_handler::HttpServerHandler, nat_handler::NatServerHandler, ssh_handler::SSHServerHandler};

pub struct NatServer {
	ctx: Context,
    // Nat client channel
	nat_client_tx: UnboundedSender<TcpStream>,
	nat_client_rx: UnboundedReceiver<TcpStream>,
	// Http client channel
	http_conn_tx: UnboundedSender<Connection>,
	http_conn_rx: UnboundedReceiver<Connection>,
	// tracing sequence
	tracing_seq: u32,
	// ssh client channel
	ssh_conn_tx: UnboundedSender<Connection>,
	ssh_conn_rx: UnboundedReceiver<Connection>,
}

impl NatServer {
    pub fn new(ctx: Context) -> NatServer {
        let (nat_client_tx, nat_client_rx) = mpsc::unbounded_channel::<TcpStream>();
		let (http_conn_tx, http_conn_rx) = mpsc::unbounded_channel::<Connection>();
		let (ssh_conn_tx, ssh_conn_rx) = mpsc::unbounded_channel::<Connection>();
        Self {
			ctx,
			nat_client_rx,
			nat_client_tx,
			http_conn_rx,
			http_conn_tx,
			ssh_conn_rx,
			ssh_conn_tx,
			tracing_seq: 0,
        }
    }

    async fn handle_client(&self, mut stream: TcpStream, socket_addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
		// Parse the first line
		let mut buffer = [0;64];
		// 读取server发过来的内容
		let usize = stream.read(&mut buffer).await.expect("failed to read data from socket");
		let first_line = match std::str::from_utf8(&buffer) {
			Ok(first_line) => {
				first_line.trim_matches('\u{0}').to_string()
			},
			Err(_) => {
				"Error".to_string()
			}
		};
	
		// Parse protocol
		let protocol = match parse_protocol(first_line) {
			Ok(protocol) => protocol,
			Err(err) => {
				error!("Failed to parse protocol: {}", err);
				return Ok(())
			}
		};
		if protocol.name() != "NAT" {
			match iface_in_subnet(socket_addr.ip().to_string().as_str(), self.ctx.get_subnet().as_str()) {
				Ok(r) => {
					if !r {
						return Err(format!("IP {:?} not allowed", socket_addr).into());
					}
				},
				Err(_) => {
					return Err(format!("IP {:?} not allowed", socket_addr).into());
				},
			};
		}
		info!("You connected to this server with protocol: {}", protocol.name());
		if protocol.name() == "NAT" {
			// Send stream to NAT client channel
			self.nat_client_tx.send(stream)?;
		} else if protocol.name() == "HTTP" {
			self.http_conn_tx.send(Connection {
				init_buf: buffer[0..usize].to_vec(),
				stream: stream
			})?;
		} else if protocol.name() == "SSH" {
			self.ssh_conn_tx.send(Connection {
				init_buf: buffer[0..usize].to_vec(),
				stream: stream
			})?;
		} else {
			return Err("Unexpected protocol".into());
		}
		Ok(())
	}

    pub async fn run_forever(&mut self, url: &str) -> Result<(), Box<dyn std::error::Error>> {
		// Create TCP server
		let listener = match TcpListener::bind(url).await {
			Ok(listener) => listener,
			Err(err) => return Err(format!("Bind TcpStream failed: {:?}", err).into())
		};
		log::info!("starting server at url: {}", url);
		// Send to NAT client message channel
		let (ncm_tx, ncm_rx) = mpsc::unbounded_channel::<Message>();
		let ncm_rx = Arc::new(RwLock::new(ncm_rx));
		let ncm_tx = Arc::new(RwLock::new(ncm_tx));
		// Reply from client channel
		let (client_reply_tx, mut client_reply_rx) = mpsc::unbounded_channel::<Message>();
		let client_reply_tx = Arc::new(RwLock::new(client_reply_tx));

		// Http Handler
		let mut http_protocol = HttpServerHandler::new(
			ncm_tx.clone(),
			self.ctx.get_ssh_mtu());
		// Nat Handler
		let nat_handler = Arc::new(RwLock::new(NatServerHandler::new(
			ncm_tx.clone(),
			ncm_rx.clone(),
			client_reply_tx.clone()
		)));
		// SSH Handler
		let ssh_handler = SSHServerHandler::new(
			self.ctx.get_ssh_mtu(),
			ncm_tx.clone()
		);
		loop {
            select! {
				// Socket comming
				socket = listener.accept() => match socket {
					Ok((stream, socket_addr)) => {
						debug!("New connection coming from {}", socket_addr);
						match self.handle_client(stream, socket_addr).await {
							Ok(_) => {},
							Err(e) => {
								error!("Handle client error: {:?}", e);
							},
						};
					},
					Err(err) => {
						return Err(Box::new(err))
					}
				},
				// Nat connection
				nat_client_stream = self.nat_client_rx.recv() => match nat_client_stream {
					Some(mut stream) if nat_handler.read().await.client_existed().await => {
						// 检测当前是否已有连接
						error!("Only support only one NAT client at a time");
						let _ = Message::nat_reject().write_to(&self.ctx, &stream).await;
						// shutdown the connect with anther client
						let _ = stream.shutdown().await;
					},
					Some(stream) => {
						// Nat client coming
						match nat_handler.write().await.run_server_backend(self.ctx.clone(), stream).await {
							Ok(_) => {},
							Err(err) => {
								log::error!("Run nat server failed: {}", err);
							}
						};
					},
					None => {
						error!("Nat client stream from channel is none")
					},
				},
				// HTTP connection
				http_conn = self.http_conn_rx.recv() => match http_conn {
					Some(mut conn) if !nat_handler.read().await.client_existed().await => {
						// 如果当前没有 nat_client 连接，则返回 502 错误
						warn!("Not exists nat client, shutdown this connections");
						let _ = &conn.stream.shutdown().await;
					},
					Some(conn) => {
						// 组装 http 数据，发送给 Nat Client message channel
						let conn = Arc::new(RwLock::new(conn));
						self.tracing_seq += 1;
						http_protocol.handle(self.tracing_seq, conn.clone()).await;
					},
					None => {
						error!("Http client stream from channel is none");
					}
				},
				// SSH connection coming
				ssh_conn = self.ssh_conn_rx.recv() => match ssh_conn {
					Some(mut conn) if !nat_handler.read().await.client_existed().await => {
						warn!("Not exists nat client, shutdown this connections");
						let _ = &conn.stream.shutdown().await;
					},
					Some(conn) => {
						let conn = Arc::new(RwLock::new(conn));
						self.tracing_seq += 1;
						let tracing_id = self.tracing_seq.clone();
						ssh_handler.run_server_backend(tracing_id, conn.clone()).await;
					},
					None => {
						error!("SSH client stream from channel is none");
					}
				},
				msg = client_reply_rx.recv() => match msg {
					Some(msg) => match msg.tracing_id {
						Some(_) if msg.is_ssh() => {
							// Reply from nat client, send to SSH stream
							ssh_handler.handle_reply(msg).await;
						},
						Some(id) => {
							// Reply from client, according tracing_id, send to specified stream
							http_protocol.handle_reply(id, &msg).await;
						},
						None => {
							error!("Message which be http but without tracing ID")
						}
					},
					None => {}
				}
            }
        }
    }
}
