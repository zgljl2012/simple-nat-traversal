use std::{sync::Arc, time::Duration};

use log::{debug, error, info};
use tokio::{sync::{RwLock, mpsc::{UnboundedSender, UnboundedReceiver, self}}, net::TcpStream, time, task, select};

use crate::{NatStream, Message, http::handle_http, utils};

// Client protocol
pub struct NatClient {
    stream: NatStream,
    sender: Arc<RwLock<UnboundedSender<Message>>>,
    receiver: UnboundedReceiver<Message>,
}

impl NatClient {
    pub async fn new(server_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let (sender, receiver) = mpsc::unbounded_channel::<Message>();
        let stream = TcpStream::connect(&server_url).await?;
        Ok(Self {
            sender: Arc::new(RwLock::new(sender)),
            receiver,
            stream: Arc::new(RwLock::new(stream)),
        })
    }

    pub async fn connect(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let stream = self.stream.write().await;
        stream.writable().await?;
        match stream.try_write("NAT 0.1".as_bytes()) {
            Ok(_) => {}
            Err(err) => {
                panic!("Write first line to server failed: {:?}", err)
            }
        };
        let mut buffer = Vec::with_capacity(1024);
        // 读取server发过来的内容
        stream.readable().await?;
        match stream.try_read_buf(&mut buffer) {
            Ok(_) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // pass
            }
            Err(err) => {
                panic!("Read from server failed: {:?}", err)
            }
        };
        let msg = std::str::from_utf8(&buffer).unwrap().trim_matches('\u{0}');
        if msg == "OK" {
            log::info!("Connect to NAT server successfully");
        } else {
            panic!("Received unexpected from server: {:?}", msg)
        }
        // Create a interval to ping the server
        let sender = self.sender.clone();
        task::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(5));
            loop {
                sender
                    .write()
                    .await
                    .send(Message::ping())
                    .unwrap();
                interval.tick().await;
            }
        });
        Ok(())
    }

    pub async fn run_forever(&mut self) {
        // Wait for NAT server
        let stream = self.stream.write().await;
		let mut ssh_existed = false;
		// Send and receiver from ssh stream
		let (ssh_tx, mut ssh_rx) = mpsc::unbounded_channel::<Message>();
		let ssh_tx = Arc::new(RwLock::new(ssh_tx));
		let (ssh_tx_reverse, ssh_rx_reverse) = mpsc::unbounded_channel::<Message>();
		let ssh_rx_reverse = Arc::new(RwLock::new(ssh_rx_reverse));
        loop {
            select! {
				// 从服务端接收请求
                _ = stream.readable() => match Message::from_stream(&stream).await {
					Ok(msg) => match msg {
						Some(msg) if msg.is_pong() => {
							debug!("You received PONG from NAT server");
						},
						Some(msg) if msg.is_http() => match handle_http(&msg).await {
							Ok(res) => {
								// Send to server
								let _ = Message::new_http(msg.tracing_id, res.as_bytes().to_vec()).write_to(&stream).await;
							},
							Err(e) => {
								error!("Redirect http request failed: {:?}", e);
								if e.source().is_some() {
									if "operation timed out" == format!("{}", e.source().unwrap()) {
										let _ = Message::http_504(msg.tracing_id).write_to(&stream).await;
									} else {
										let _ = Message::http_502(msg.tracing_id).write_to(&stream).await;
									}
								} else {
									let _ = Message::http_502(msg.tracing_id).write_to(&stream).await;
								}
							}
						},
						// SSH 消息，且当前未创建本地 SSH
						Some(msg) if msg.is_ssh() && !ssh_existed => {
							info!("Received SSH request from server");
							// 检查当前是否已有 SSH 连接，如果没有，先创建连接，连接不停等待
							let ssh_tx = ssh_tx.clone();
							let bytes = msg.body.clone();
							let tracing_id = msg.tracing_id.clone();
							let ssh_rx_reverse = ssh_rx_reverse.clone();
							tokio::spawn(async move {
								let target = "127.0.0.1:22";
								let stream = TcpStream::connect(&target).await.unwrap();
								stream.try_write(&bytes).unwrap();
								let mut ssh_rx_reverse = ssh_rx_reverse.write().await;
								loop {
									select! {
										_ = stream.readable() => {
											// 获取所有请求报文
											let bytes = utils::get_packet_from_stream(&stream);
											info!("Get SSH reply from local: {:?}", bytes.len());
											ssh_tx.write().await.send(Message::new_ssh(tracing_id, bytes)).unwrap();
										},
										msg = ssh_rx_reverse.recv() => match msg {
											Some(msg) => {
												stream.try_write(&msg.body).unwrap();
											},
											None => {}
										}
									}
								}
							});
							ssh_existed = true;
						},
						// SSH 消息，且当前已创建 SSH 连接
						Some(msg) if msg.is_ssh() && ssh_existed => {
							// 将消息发送给 SSH thread
							ssh_tx_reverse.send(msg).unwrap();
						},
						Some(_) => {},
						None => {}
					},
					Err(err) => {
						error!("{:?}", err);
						break;
					},
				},
				// Read from SSH stream
				msg = ssh_rx.recv() => match msg {
					Some(msg) => {
						// Send to server
						msg.write_to(&stream).await.unwrap();
					},
					None => {}
				},
                msg = self.receiver.recv() => match msg {
                    Some(msg) => {
                        // Send to server
                        let _ = msg.write_to(&stream).await;
                    },
                    None => {},
                }
            }
        }
    }
}
