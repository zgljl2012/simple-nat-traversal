use std::{sync::Arc, collections::HashMap};

use log::{debug, info, error, warn};
use tokio::{sync::{RwLock, mpsc::{UnboundedSender, UnboundedReceiver, self}}, select, net::{TcpStream, TcpListener}, io::{AsyncReadExt, AsyncWriteExt}};

use crate::{Message, parse_protocol, utils::get_packet_from_stream};

#[derive(Debug)]
struct Connection {
	pub init_buf: Vec<u8>, // 因为最开始会读取一行判断协议，故此处需加上
	pub stream: TcpStream,
}

fn get_packets(conn: &Connection) -> Vec<u8> {
	// 获取所有请求报文
	let mut buffer = [0;1024];
	let mut bytes:Vec<u8> = Vec::new();
	let mut init_buf = conn.init_buf.clone();
	bytes.append(&mut init_buf);
	loop {
		match conn.stream.try_read(&mut buffer) {
			Ok(0) => break,
			Ok(n) => {
				bytes.append(&mut buffer[0..n].to_vec());
			},
			Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
			Err(err) => {
				error!("Read failed: {:?}", err);
				break;
			}
		};
	}
	debug!("request size: {:?}", bytes.len());
	bytes
}

pub struct NatServer {
    // Nat client channel
	nat_client_tx: UnboundedSender<TcpStream>,
	nat_client_rx: UnboundedReceiver<TcpStream>,
	// Connected client count
	nat_client_cnt: usize,
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
    pub fn new() -> NatServer {
        let (nat_client_tx, nat_client_rx) = mpsc::unbounded_channel::<TcpStream>();
		let (http_conn_tx, http_conn_rx) = mpsc::unbounded_channel::<Connection>();
		let (ssh_conn_tx, ssh_conn_rx) = mpsc::unbounded_channel::<Connection>();
        Self {
			nat_client_rx,
			nat_client_tx,
			nat_client_cnt: 0,
			http_conn_rx,
			http_conn_tx,
			ssh_conn_rx,
			ssh_conn_tx,
			tracing_seq: 0,
        }
    }

    async fn handle_client(&self, mut stream: TcpStream) -> Result<(), Box<dyn std::error::Error>> {
		// Parse the first line
		const BATCH_SIZE: usize = 64;
		let mut buffer = [0;BATCH_SIZE];
		// 读取server发过来的内容
		let usize = stream.read(&mut buffer).await.expect("failed to read data from socket");
		let first_line = std::str::from_utf8(&buffer).unwrap().trim_matches('\u{0}').to_string();
	
		// Parse protocol
		let protocol = match parse_protocol(first_line) {
			Ok(protocol) => protocol,
			Err(err) => {
				error!("Failed to parse protocol: {}", err);
				return Ok(())
			}
		};
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
		}
		Ok(())
	}

    pub async fn run_forever(&mut self, url: &str) -> Result<(), Box<dyn std::error::Error>> {
		// Create TCP server
		let listener = TcpListener::bind(url).await?;
		// Send to NAT client message channel
		let (ncm_tx, ncm_rx) = mpsc::unbounded_channel::<Message>();
		let ncm_rx = Arc::new(RwLock::new(ncm_rx));
		// Reply from client channel
		let (client_reply_tx, mut client_reply_rx) = mpsc::unbounded_channel::<Message>();
		let client_reply_tx = Arc::new(RwLock::new(client_reply_tx));
		// Http connections
		let mut connections: HashMap<u32, TcpStream> = HashMap::new();
		// Failed to connect to the client 
		let (cc_failed_tx, mut cc_failed_rx) = mpsc::unbounded_channel::<bool>();
		let cc_failed_tx = Arc::new(RwLock::new(cc_failed_tx));
		// Connection remove channel
		let (remove_conn_tx, mut remove_conn_rx) = mpsc::unbounded_channel::<u32>();
        loop {
            select! {
				socket = listener.accept() => match socket {
					Ok((stream, _)) => {
						info!("Connections count: {:?}", connections.len());
						self.handle_client(stream).await?;
					},
					Err(err) => {
						return Err(Box::new(err))
					}
				},
				nat_client_stream = self.nat_client_rx.recv() => match nat_client_stream {
					Some(mut stream) if self.nat_client_cnt > 0 => {
						// 检测当前是否已有连接
						error!("Only support only one NAT client at a time");
						stream.try_write("Reject".as_bytes()).unwrap();
						// shutdown the connect with anther client
						let _ = stream.shutdown().await;
					},
					Some(stream) => {
						// 开始握手连接, 发送 OK
						match stream.try_write("OK".as_bytes()) {
							Ok(_) => {
								debug!("Send reply to client successfully");
							}
							Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
							Err(e) => {
								return Err(e.into());
							}
						};
						self.nat_client_cnt += 1;
						// 此时，已连接 nat client，则开始在异步中循环处理此异步
						let ncm_rx = ncm_rx.clone();
						let client_reply_tx = client_reply_tx.clone();
						let cc_failed_tx = cc_failed_tx.clone();
						tokio::spawn(async move {
							let mut ncm_rx = ncm_rx.write().await;
							let cc_failed_tx = cc_failed_tx.write().await;
							loop {
								select! {
									// 读取消息，发送给 Client
									msg = ncm_rx.recv() => match msg {
										Some(msg) => {
											match msg.write_to(&stream).await {
												Ok(_) => {
													info!("Send request to client successfully");
												}
												Err(e) => {
													// Send to client error
													error!("Send request to client failed: {}", e);
													cc_failed_tx.send(true).unwrap();
													break;
												}
											}
										},
										None => {}
									},
									// 从 Client 读取消息
									_ = stream.readable() => {
										match Message::from_stream(&stream).await {
											Ok(msg) => match msg {
												Some(msg) => {
													debug!("Received {:?}", msg.protocol);
													if msg.is_ping() {
														// 如果收到 Ping，就直接 Pong
														debug!("You received PING from NAT client");
														match Message::pong().write_to(&stream).await {
															Ok(_) => {},
															Err(e) => {
																error!("Can't send PONG to client failed: {}", e);
																cc_failed_tx.send(true).unwrap();
																break;
															}
														};
													} else if msg.is_http() {
														debug!("Received HTTP Response from client");
														let _ = client_reply_tx.write().await.send(msg);
													} else if msg.is_ssh() {
														info!("Received SSH Response from client");
														let _ = client_reply_tx.write().await.send(msg);
													}
												},
												None => {}
											},
											Err(err) => {
												error!("{}", err);
												cc_failed_tx.send(true).unwrap();
												break;
											},
										}
									},
								}
							}
						});
					},
					None => {
						error!("Nat client stream from channel is none")
					},
				},
				// HTTP connection
				http_conn = self.http_conn_rx.recv() => match http_conn {
					Some(mut conn) if self.nat_client_cnt == 0 => {
						// 如果当前没有 nat_client 连接，则返回 502 错误
						warn!("Not exists nat client, shutdown this connections");
						let _ = &conn.stream.shutdown().await;
					},
					Some(conn) => {
						// 组装 http 数据，发送给 Nat Client message channel
						let bytes = get_packets(&conn);
						self.tracing_seq += 1;
						let msg = Message::new_http(Some(self.tracing_seq), bytes);
						connections.insert(self.tracing_seq, conn.stream);
						ncm_tx.send(msg).unwrap();
					},
					None => {
						error!("Http client stream from channel is none");
					}
				},
				// SSH connection
				ssh_conn = self.ssh_conn_rx.recv() => match ssh_conn {
					Some(mut conn) if self.nat_client_cnt == 0 => {
						warn!("Not exists nat client, shutdown this connections");
						let _ = &conn.stream.shutdown().await;
					},
					Some(conn) => {
						let bytes = get_packets(&conn);
						self.tracing_seq += 1;
						let msg = Message::new_ssh(Some(self.tracing_seq), bytes);
						connections.insert(self.tracing_seq, conn.stream);
						ncm_tx.send(msg).unwrap();
					},
					None => {
						error!("SSH client stream from channel is none");
					}
				},
				msg = client_reply_rx.recv() => match msg {
					Some(msg) => match msg.tracing_id {
						Some(id) => {
							// Reply from client, according tracing_id, send to specified stream
							let stream = connections.get(&id);
							match stream {
								Some(stream) => {
									if msg.is_http() {
										debug!("Received HTTP reply from client: {:?}", msg.to_utf8());
									}
									if msg.is_ssh() {
										info!("Received SSH reply from client: {:?}", msg.to_utf8());
									}
									match stream.writable().await {
										Ok(_) => {},
										Err(err) => {
											error!("Wait stream for reply failed: {:?}, remove it", err);
											remove_conn_tx.send(id).unwrap();
										}
									};
									match stream.try_write(&msg.body.as_slice()) {
										Ok(_) => {
											debug!("write message to stream successfully");
											// shutdown
											if msg.is_http() {
												remove_conn_tx.send(id).unwrap();
											}
											// 如果是 SSH 连接，接下来得创建线程，跟访问者有来有回了
											if msg.is_ssh() {
												select! {
													_ = stream.readable() => {
														let bytes = get_packet_from_stream(&stream);
														// 发送给 Nat client
														let _ = ncm_tx.send(Message::new_ssh(msg.tracing_id, bytes));
													}
												};
											}
										}
										Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
										Err(e) => {
											error!("Write response to stream failed: {:?}", e);
										}
									}
								},
								None => {
									error!("Tracing ID {:?} does not exist", id);
								}
							}
						},
						None => {
							error!("Message which be http but without tracing ID")
						}
					},
					None => {}
				},
				id = remove_conn_rx.recv() => match id {
					Some(id) => {
						connections.remove(&id);
					},
					None => {}
				},
				_ = cc_failed_rx.recv() => {
					// Client disconnected
					self.nat_client_cnt -= 1;
				}
            }
        }
    }
}
