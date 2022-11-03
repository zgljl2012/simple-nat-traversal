use std::{sync::Arc, time::Duration};

use log::{debug, error, info};
use tokio::{sync::{RwLock, mpsc::{UnboundedSender, UnboundedReceiver, self}}, net::TcpStream, time, task, select};

use crate::{NatStream, Message, http::handle_http, ssh::handle_ssh};

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
						Some(msg) => {
							if msg.is_ssh() {
								info!("Received SSH request from server");
								let reply = match handle_ssh(&msg).await {
									Ok(reply) => reply,
									Err(err) => {
										error!("Handle ssh failed: {:?}", err);
										b"Error".to_vec()
									}
								};
								info!("Get SSH reply from local: {:?}", reply.len());
								let _ = Message::new_ssh(msg.tracing_id, reply).write_to(&stream).await;
							}
						}
						None => {}
					},
					Err(err) => {
						error!("{:?}", err);
						break;
					},
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
