use std::{sync::Arc, collections::HashMap};

use log::{debug, info, error, warn};
use tokio::{sync::{RwLock, mpsc::{UnboundedSender, UnboundedReceiver, self}}, select, net::{TcpStream, TcpListener}, io::{AsyncReadExt, AsyncWriteExt}};

use crate::{Message, parse_protocol, utils::get_packet_from_stream, SSHStatus, Context};

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
	ctx: Context,
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
    pub fn new(ctx: Context) -> NatServer {
        let (nat_client_tx, nat_client_rx) = mpsc::unbounded_channel::<TcpStream>();
		let (http_conn_tx, http_conn_rx) = mpsc::unbounded_channel::<Connection>();
		let (ssh_conn_tx, ssh_conn_rx) = mpsc::unbounded_channel::<Connection>();
        Self {
			ctx,
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
		let listener = TcpListener::bind(url).await?;
		// Send to NAT client message channel
		let (ncm_tx, ncm_rx) = mpsc::unbounded_channel::<Message>();
		let ncm_rx = Arc::new(RwLock::new(ncm_rx));
		let ncm_tx = Arc::new(RwLock::new(ncm_tx));
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
		// SSH connection message sender
		let mut ssh_conns_tx: HashMap<u32, UnboundedSender<Message>> = HashMap::new();
		// SSH disconnected
		let (ssh_disconnected_tx, mut ssh_disconnected_rx) = mpsc::unbounded_channel::<u32>();
		let ssh_disconnected_tx = Arc::new(RwLock::new(ssh_disconnected_tx));
		let batch_size = self.ctx.get_ssh_mtu();
        loop {
            select! {
				// Socket comming
				socket = listener.accept() => match socket {
					Ok((stream, _)) => {
						info!("Connections count: {:?}", connections.len());
						match self.handle_client(stream).await {
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
					Some(mut stream) if self.nat_client_cnt > 0 => {
						// 检测当前是否已有连接
						error!("Only support only one NAT client at a time");
						let _ = Message::nat_reject().write_to(&stream).await;
						// shutdown the connect with anther client
						let _ = stream.shutdown().await;
					},
					Some(stream) => {
						// 开始握手连接, 发送 OK
						match Message::nat_ok().write_to(&stream).await {
							Ok(_) => {
								debug!("Send reply to client successfully");
							}
							Err(e) => {
								return Err(format!("Send reply to client failed: {:?}", e).into());
							}
						};
						self.nat_client_cnt += 1;
						// 此时，已连接 nat client，则开始在异步中循环处理此异步
						let ncm_rx = ncm_rx.clone();
						let client_reply_tx = client_reply_tx.clone();
						let cc_failed_tx = cc_failed_tx.clone();
						let ncm_tx = ncm_tx.clone();
						tokio::spawn(async move {
							let mut ncm_rx = ncm_rx.write().await;
							let cc_failed_tx = cc_failed_tx.write().await;
							loop {
								select! {
									// 读取消息，发送给 Client
									msg = ncm_rx.recv() => match msg {
										Some(msg) => match msg.write_to(&stream).await {
											Ok(_) => {
												info!("Send request to client successfully");
											}
											Err(e) => {
												// Send to client error
												error!("Send request to client failed: {}", e);
												cc_failed_tx.send(true).unwrap();
												break;
											}
										},
										None => {}
									},
									// 从 Client 读取消息
									_ = stream.readable() => match Message::from_stream(&stream).await {
										Ok(msg) => match msg {
											Some(msg) if msg.is_ping() => {
												// 如果收到 Ping，就直接 Pong
												debug!("You received PING from NAT client");
												ncm_tx.write().await.send(Message::pong()).unwrap();
											},
											Some(msg) if msg.is_http() => {
												debug!("Received HTTP Response from client");
												let _ = client_reply_tx.write().await.send(msg);
											},
											Some(msg) if msg.is_ssh() => {
												info!("Received SSH Response from client");
												let _ = client_reply_tx.write().await.send(msg);
											},
											Some(msg) => {
												debug!("Received {:?}", msg.protocol);
											}
											None => {}
										},
										// Err(err) if format!("{}", err).contains("Uncognizaed protocol type") => {
										// 	warn!("Received {:?}, this a error packet, discard it", err);
										// },
										Err(err) => {
											error!("{}", err);
											cc_failed_tx.send(true).unwrap();
											break;
										},
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
						ncm_tx.write().await.send(msg).unwrap();
					},
					None => {
						error!("Http client stream from channel is none");
					}
				},
				// SSH connection coming
				ssh_conn = self.ssh_conn_rx.recv() => match ssh_conn {
					Some(mut conn) if self.nat_client_cnt == 0 => {
						warn!("Not exists nat client, shutdown this connections");
						let _ = &conn.stream.shutdown().await;
					},
					Some(mut conn) => {
						let bytes = get_packets(&conn);
						self.tracing_seq += 1;
						let tracing_id = self.tracing_seq.clone();
						let msg = Message::new_ssh(Some(tracing_id), bytes);
						// Create sender and receiver
						let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
						// Save tx
						ssh_conns_tx.insert(tracing_id, tx);
						// Send init message to client
						ncm_tx.write().await.send(msg).unwrap();
						// Start a thread to accept ssh connection
						let ncm_tx = ncm_tx.clone();
						let ssh_disconnected_tx = ssh_disconnected_tx.clone();
						tokio::spawn(async move {
							loop {
								select! {
									// Read from stream
									s = conn.stream.readable() => match s {
										Ok(_) => {
											let bytes = get_packet_from_stream(&conn.stream);
											info!("Read from SSH stream: {:?}, and send to client", bytes.len());
											if bytes.len() > 0 {
												// Send to client
												ncm_tx.write().await.send(Message::new_ssh(Some(tracing_id), bytes)).unwrap();
											} else {
												error!("SSH disconnected");
												// 通知 Client, SSH disconnected
												ncm_tx.write().await.send(Message::new_ssh_error(Some(tracing_id))).unwrap();
												break;
											}
										},
										Err(e) => {
											error!("SSH disconnected: {}", e);
											break;
										}
									},
									// Read from NAT client
									msg = rx.recv() => match msg {
										Some(msg) => {
											// Write to stream
											if msg.ssh_status != Some(SSHStatus::Ok) {
												error!("SSH of client have been disconnected");
												conn.stream.shutdown().await.unwrap();
												break;
											} else {
												info!("Try to send {} bytes to SSH connection", msg.body.len());
												// 分批次发送
												let mut i: usize = 0;
												let mut has_error: bool = false;
												loop {
													let mut end = i + batch_size;
													if end > msg.body.len() {
														end = msg.body.len();
													}
													match conn.stream.try_write(&msg.body[i..end]) {
														Ok(n) => {
															info!("Send {} bytes to SSH connection", n);
															i += batch_size;
															if i >= msg.body.len() {
																break;
															}
														}
														Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
														Err(e) => {
															has_error = true;
															error!("Write response to SSH stream failed: {:?}", e);
															break;
														}
													};
												}	
												if has_error {
													break;
												}
											}
										},
										None => {}
									}
								}
							}
							ssh_disconnected_tx.write().await.send(tracing_id).unwrap();
						});
					},
					None => {
						error!("SSH client stream from channel is none");
					}
				},
				msg = client_reply_rx.recv() => match msg {
					Some(msg) => match msg.tracing_id {
						Some(id) if msg.is_ssh() => {
							// Find sender
							info!("Received SSH reply from client");
							match ssh_conns_tx.get(&id) {
								Some(tx) => {
									tx.send(msg).unwrap();
								},
								None => {}
							};
						},
						Some(id) => {
							// Reply from client, according tracing_id, send to specified stream
							let stream = connections.get(&id);
							match stream {
								Some(stream) => {
									debug!("Received HTTP reply from client: {:?}", msg.to_utf8());
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
				id = ssh_disconnected_rx.recv() => match id {
					Some(id) => {
						ssh_conns_tx.remove(&id);
					},
					None => {}
				},
				_ = cc_failed_rx.recv() => {
					// Client disconnected
					error!("Nat Client disconnected");
					// 将 ncm_rx 中消息全部清空掉
					self.nat_client_cnt -= 1;
				}
            }
        }
    }
}
