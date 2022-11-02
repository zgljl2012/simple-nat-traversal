use std::{sync::Arc, collections::HashMap};

use log::{debug, info, error};
use tokio::{sync::{RwLock, mpsc::{UnboundedSender, UnboundedReceiver, self}}, select, net::{TcpStream, TcpListener}, io::{AsyncReadExt, AsyncWriteExt}};

use crate::{NatStream, Message, parse_protocol};

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
	debug!("Http request size: {:?}", bytes.len());
	let text = std::str::from_utf8(bytes.as_slice()).unwrap().trim_matches('\u{0}').to_string();
	debug!("{}", text);
	bytes
}

pub struct NatServer {
    stream: Option<NatStream>,
    sender: Arc<RwLock<UnboundedSender<Message>>>,
    receiver: UnboundedReceiver<Message>,
    exteral_sender: Arc<RwLock<UnboundedSender<Message>>>,
    exteral_receiver: UnboundedReceiver<Message>,
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
}

impl NatServer {
    pub fn new(
        exteral_sender: Arc<RwLock<UnboundedSender<Message>>>,
        exteral_receiver: UnboundedReceiver<Message>,
    ) -> NatServer {
        let (sender, receiver) = mpsc::unbounded_channel::<Message>();
		let (nat_client_tx, nat_client_rx) = mpsc::unbounded_channel::<TcpStream>();
		let (http_conn_tx, http_conn_rx) = mpsc::unbounded_channel::<Connection>();
        Self {
			nat_client_rx,
			nat_client_tx,
			nat_client_cnt: 0,
			http_conn_rx,
			http_conn_tx,
            stream: None,
            receiver: receiver,
            sender: Arc::new(RwLock::new(sender)),
            exteral_receiver,
            exteral_sender,
			tracing_seq: 0,
        }
    }

    pub fn init(&mut self, stream: NatStream) {
        self.stream = Some(stream);
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
			if self.nat_client_cnt == 0 {
				// 如果当前没有 nat_client 连接，则返回 502 错误
				Message::http_502(None).write_to(&stream).await;
				let _ = &stream.shutdown().await?;
			}
			self.http_conn_tx.send(Connection {
				init_buf: buffer[0..usize].to_vec(),
				stream: stream
			})?;
			// tokio::spawn(async move {
			// 	// 将请求转发给客户端
			// 	sender.write().await.send(Message::new_http(Some(0), bytes)).unwrap();
			// 	// 获取所有的请求二进制
			// 	let mut recv = receiver.write().await;
			// 	select! {
			// 		msg = recv.recv() => match msg {
			// 			Some(msg) => {
			// 				match stream.write(&msg.body).await {
			// 					Ok(n) => {
			// 						let s = std::str::from_utf8(&msg.body).unwrap();
			// 						debug!("Http response successfully: {:?} bytes, body {:?}", n, s);
			// 					},
			// 					Err(e) => {
			// 						error!("Write error: {}", e);
			// 					}
			// 				}
			// 			},
			// 			None => {
			// 				error!("Receive empty message");
			// 				match stream.write(b"HTTP/1.1 500 Internal Server Error\r\n").await {
			// 					Ok(_) => {},
			// 					Err(e) => {
			// 						error!("Write error: {}", e);
			// 					}
			// 				}
			// 			},
			// 		}
			// 	}
			// 	stream.shutdown().await.unwrap();
			// 	info!("Http stream shutdown completed");
			// });
		}
		Ok(())
	}

    pub async fn run_forever(&mut self, url: &str) -> Result<(), Box<dyn std::error::Error>> {
		// Create TCP server
		let listener = TcpListener::bind(url).await?;
        // Wait for NAT server
        // let stream = self.stream.as_ref().unwrap().write().await;
        let sender = &self.sender;
        let receiver = &self.receiver;
        let exteral_receiver = &self.exteral_receiver;
        let exteral_sender = &self.exteral_sender;

		// Send to NAT client message channel
		let (ncm_tx, ncm_rx) = mpsc::unbounded_channel::<Message>();
		let ncm_rx = Arc::new(RwLock::new(ncm_rx));
		// Reply from client channel
		let (client_reply_tx, mut client_reply_rx) = mpsc::unbounded_channel::<Message>();
		let client_reply_tx = Arc::new(RwLock::new(client_reply_tx));
		// Http connections
		let mut connections: HashMap<u32, TcpStream> = HashMap::new();
        loop {
            select! {
				socket = listener.accept() => match socket {
					Ok((stream, _)) => {
						info!("Http connections count: {:?}", connections.len());
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
						tokio::spawn(async move {
							let mut ncm_rx = ncm_rx.write().await;
							loop {
								select! {
									msg = ncm_rx.recv() => match msg {
										Some(msg) => {
											msg.write_to(&stream).await;
										},
										None => {}
									},
									_ = stream.readable() => {
										match Message::from_stream(&stream).await {
											Ok(msg) => match msg {
												Some(msg) => {
													debug!("Received {:?}", msg.protocol);
													if msg.is_ping() {
														// 如果收到 Ping，就直接 Pong
														info!("You received PING from NAT client");
														Message::pong().write_to(&stream).await;
													}
													if msg.is_http() {
														info!("Received HTTP Response from client");
														let _ = client_reply_tx.write().await.send(msg);
													}
												},
												None => {}
											},
											Err(err) => {
												error!("{:?}", err);
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
				http_conn = self.http_conn_rx.recv() => match http_conn {
					Some(mut conn) if self.nat_client_cnt == 0 => {
						// 如果当前没有 nat_client 连接，则返回 502 错误
						info!("Not exists nat client, shutdown this connections");
						Message::http_502(None).write_to(&conn.stream).await;
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
				msg = client_reply_rx.recv() => match msg {
					Some(msg) => match msg.tracing_id {
						Some(id) => {
							// Reply from client, according tracing_id, send to specified stream
							let stream = connections.get(&id);
							match stream {
								Some(stream) => {
									debug!("Received HTTP reply from client: {:?}", msg.to_utf8());
									match stream.try_write(&msg.body.as_slice()) {
										Ok(_) => {
											debug!("write message to http stream successfully");
											// shutdown
											connections.remove(&id);
										}
										Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
										Err(e) => {
											error!("Write response to http stream failed: {:?}", e);
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
				}
                // exteral_msg = exteral_receiver.recv() => match exteral_msg {
                //     Some(msg) => {
                //         sender.write().await.send(msg).unwrap();
                //     },
                //     None => todo!()
                // },
                // msg = receiver.recv() => match msg {
                //     Some(msg) => {
                //         msg.write_to(&stream).await;
                //     },
                //     None => todo!(),
                // }
            }
        }
    }
}

