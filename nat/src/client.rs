use std::{sync::Arc, time::Duration};

use log::{debug, error, info, warn};
use tokio::{sync::{RwLock, mpsc::{self}}, net::TcpStream, time, select};

use crate::{Message, http::handle_http, utils, SSHStatus};

// Client protocol
pub struct NatClient {
    server_url: String,
}

impl NatClient {
    pub async fn new(server_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
			server_url: server_url.to_string()
        })
    }

    pub async fn run_forever(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Wait for NAT server
		let stream = TcpStream::connect(&self.server_url).await?;
		// If there is exists SSH connection
		let ssh_existed = Arc::new(RwLock::new(false));
		// Send and receiver from ssh stream
		let (ssh_tx, mut ssh_rx) = mpsc::unbounded_channel::<Message>();
		let ssh_tx = Arc::new(RwLock::new(ssh_tx));
		let (ssh_tx_reverse, ssh_rx_reverse) = mpsc::unbounded_channel::<Message>();
		let ssh_rx_reverse = Arc::new(RwLock::new(ssh_rx_reverse));
		// Create tx and rx channel to send and receive message with remote server
		let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
		// Send init message
		stream.writable().await?;
        match stream.try_write("NAT 0.1".as_bytes()) {
            Ok(_) => {}
            Err(err) => {
                panic!("Write first line to server failed: {:?}", err)
            }
        };
		// Create ping timer
		let mut interval = time::interval(Duration::from_secs(5));

		loop {
            select! {
				// PING interval
				_ = interval.tick() => {
					// Send ping
					debug!("Send PING to server");
					tx.send(Message::ping()).unwrap();
				},
				// 从服务端接收请求
                _ = stream.readable() => match Message::from_stream(&stream).await {
					Ok(msg) => match msg {
						Some(msg) if msg.is_rejected() => {
							error!("Server reject us");
							break;
						},
						Some(msg) if msg.is_pong() => {
							debug!("You received PONG from NAT server");
						},
						Some(msg) if msg.is_http() => match handle_http(&msg).await {
							Ok(res) => {
								// Send to server
								tx.send(Message::new_http(msg.tracing_id, res.as_bytes().to_vec())).unwrap();
							},
							Err(e) => {
								error!("Redirect http request failed: {:?}", e);
								if e.source().is_some() {
									if "operation timed out" == format!("{}", e.source().unwrap()) {
										tx.send(Message::http_504(msg.tracing_id)).unwrap();
									} else {
										tx.send(Message::http_502(msg.tracing_id)).unwrap();
									}
								} else {
									tx.send(Message::http_502(msg.tracing_id)).unwrap();
								}
							}
						},
						// SSH 消息，且当前未创建本地 SSH
						Some(msg) if msg.is_ssh() && !*ssh_existed.read().await => {
							info!("Received SSH request from server");
							// 检查进来的流量是否为请求建立 SSH，如果不是，则忽略
							match std::str::from_utf8(&msg.body) {
								Ok(r) if !r.contains("SSH") => {
									warn!("Dirty SSH message")
								},
								Ok(_) => {
									// 检查当前是否已有 SSH 连接，如果没有，先创建连接，连接不停等待
									let ssh_tx = ssh_tx.clone();
									let bytes = msg.body.clone();
									let tracing_id = msg.tracing_id.clone();
									let ssh_rx_reverse = ssh_rx_reverse.clone();
									tokio::spawn(async move {
										let target = "127.0.0.1:22";
										let stream = match TcpStream::connect(&target).await {
											Ok(stream) => stream,
											Err(e) => {
												error!("Connect to local SSH server failed: {}", e);
												ssh_tx.write().await.send(Message::new_ssh_error(tracing_id)).unwrap();
												return;
											}
										};
										match stream.try_write(&bytes) {
											Ok(_) => {}
											// Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
											Err(e) => {
												error!("Write response to SSH stream failed: {:?}", e);
												ssh_tx.write().await.send(Message::new_ssh_error(tracing_id)).unwrap();
												return;
											}
										};
										let mut ssh_rx_reverse = ssh_rx_reverse.write().await;
										loop {
											select! {
												r = stream.readable() => match r {
													Ok(_) => {
														// 获取所有请求报文
														let bytes = utils::get_packet_from_stream(&stream);
														info!("Get SSH reply from local: {:?}", bytes.len());
														if bytes.len() == 0 {
															ssh_tx.write().await.send(Message::new_ssh_error(tracing_id)).unwrap();
															break;
														}
														ssh_tx.write().await.send(Message::new_ssh(tracing_id, bytes)).unwrap();
													},
													Err(e) => {
														error!("Local SSH disconnected: {:?}", e);
														ssh_tx.write().await.send(Message::new_ssh_error(tracing_id)).unwrap();
														break;
													}
												},
												msg = ssh_rx_reverse.recv() => match msg {
													Some(msg) => {
														match stream.try_write(&msg.body) {
															Ok(n) => {
																info!("Write to local SSH: {:?}", n);
															},
															Err(e) => {
																error!("Write response to SSH stream failed: {:?}", e);
																ssh_tx.write().await.send(Message::new_ssh_error(tracing_id)).unwrap();
																break;
															}
														};
													},
													None => {}
												}
											}
										}
									});
									*ssh_existed.write().await = true;
								},
								Err(_) => {
									warn!("Dirty SSH message");
								},
							}
						},
						// SSH 消息，且当前已创建 SSH 连接
						Some(msg) if msg.is_ssh() && *ssh_existed.read().await => {
							// 将消息发送给 SSH thread
							ssh_tx_reverse.send(msg).unwrap();
						},
						Some(_) => {},
						None => {}
					},
					Err(err) => {
						error!("Read from server failed: {:?}", err);
						break;
					},
				},
				// Read from SSH stream
				msg = ssh_rx.recv() => match msg {
					Some(msg) => {
						if msg.is_ssh() && msg.ssh_status != Some(SSHStatus::Ok) {
							*ssh_existed.write().await = false; // 说明连接断开了
						}
						// Send to server
						tx.send(msg).unwrap();
					},
					None => {}
				},
                msg = rx.recv() => match msg {
                    Some(msg) => {
                        // Send to server
                        match msg.write_to(&stream).await {
							Ok(_) => {},
							Err(e) => {
								error!("Write to server failed: {:?}", e);
								// TODO 断线重连
							},
						}
                    },
                    None => {},
                }
            };
        }
		Ok(())
    }
}
