use std::sync::Arc;

use log::{debug, info, error};
use tokio::{sync::{RwLock, mpsc::{UnboundedSender, UnboundedReceiver, self}}, select, net::{TcpStream, TcpListener}, io::{AsyncReadExt, AsyncWriteExt}};

use crate::{NatStream, Message, parse_protocol};


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
}

impl NatServer {
    pub fn new(
        exteral_sender: Arc<RwLock<UnboundedSender<Message>>>,
        exteral_receiver: UnboundedReceiver<Message>,
    ) -> NatServer {
        let (sender, receiver) = mpsc::unbounded_channel::<Message>();
		let (nat_client_tx, nat_client_rx) = mpsc::unbounded_channel::<TcpStream>();
        Self {
			nat_client_rx,
			nat_client_tx,
			nat_client_cnt: 0,
            stream: None,
            receiver: receiver,
            sender: Arc::new(RwLock::new(sender)),
            exteral_receiver,
            exteral_sender,
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
		let _ = stream.read(&mut buffer).await.expect("failed to read data from socket");
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
			// tokio::spawn(async move {
			// 	// 握手
			// 	nat_server.write().await.init(Arc::new(RwLock::new(stream)));
			// 	match nat_server.write().await.ok().await {
			// 		Ok(_) => {},
			// 		Err(err) => {
			// 			error!("Reply OK to client failed: {:?}", err);
			// 		},
			// 	};
			// 	// Init
			// 	*ctx.inited.write().await = true;
			// 	// 通信
			// 	nat_server.write().await.run_forever().await;
			// 	*ctx.inited.write().await = false;
			// });
		} else if protocol.name() == "HTTP" {
			// if *ctx.inited.read().await == false {
			// 	error!("There is no nat client connected");
			// 	return Ok(());
			// }
			// tokio::spawn(async move {
			// 	// 获取所有请求报文
			// 	let mut bytes:Vec<u8> = Vec::new();
			// 	bytes.append(&mut buffer[0..nsize].to_vec());
			// 	loop {
			// 		match stream.try_read(&mut buffer) {
			// 			Ok(0) => break,
			// 			Ok(n) => {
			// 				bytes.append(&mut buffer[0..n].to_vec());
			// 			},
			// 			Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
			// 			Err(err) => {
			// 				error!("Read failed: {:?}", err);
			// 				break;
			// 			}
			// 		};
			// 	}
			// 	debug!("Http request size: {:?}", bytes.len());
			// 	let text = std::str::from_utf8(bytes.as_slice()).unwrap().trim_matches('\u{0}').to_string();
			// 	debug!("{}", text);
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
        loop {
            select! {
				socket = listener.accept() => match socket {
					Ok((stream, _)) => {
						self.handle_client(stream).await?;
					},
					Err(err) => {
						return Err(Box::new(err))
					}
				},
				nat_client_stream = self.nat_client_rx.recv() => match nat_client_stream {
					Some(mut stream) => {
						// 检测当前是否已有连接
						if self.nat_client_cnt > 0 {
							error!("Only support only one NAT client at a time");
							stream.try_write("Reject".as_bytes()).unwrap();
							// shutdown the connect with anther client
							let _ = stream.shutdown().await;
							return Ok(());
						} 
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

						// 此时，已连接 nat client，则开始在异步中循环处理此异步
						tokio::spawn(async move {
							loop {
								select! {
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
													// if msg.is_http() {
													// 	info!("Received HTTP Response from client");
													// 	exteral_sender.write().await.send(msg).unwrap();
													// }
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

