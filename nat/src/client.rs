use std::{sync::Arc, time::Duration};

use log::{debug, error, info, warn};
use tokio::{sync::{RwLock, mpsc::{self}}, net::TcpStream, time, select, io::AsyncWriteExt};

use crate::{Message, http::{handle_http}, utils, SSHStatus, Context, crypto, cache, ssh_handler::SSHClientHandler};

// Client protocol
pub struct NatClient {
	ctx: Context,
    server_url: String,
}

impl NatClient {
    pub async fn new(server_url: &str, ctx: Context) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
			ctx,
			server_url: server_url.to_string()
        })
    }

    pub async fn run_forever(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Wait for NAT server
		let mut stream = TcpStream::connect(&self.server_url).await?;
		// If there is exists SSH connection
		let ssh_existed = Arc::new(RwLock::new(false));
		// Send and receiver from ssh stream
		let (ssh_tx, mut ssh_rx) = mpsc::unbounded_channel::<Message>();
		let ssh_tx = Arc::new(RwLock::new(ssh_tx));
		let (ssh_tx_reverse, ssh_rx_reverse) = mpsc::unbounded_channel::<Message>();
		let ssh_rx_reverse = Arc::new(RwLock::new(ssh_rx_reverse));
		// Create tx and rx channel to send and receive message with remote server
		let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
		let tx = Arc::new(RwLock::new(tx));
		// Http message channel
		let (http_tx, mut http_rx) = mpsc::unbounded_channel::<Message>();
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
		// 缓存没有读完的 http response
		let http_cache: Arc<RwLock<cache::Cache<u32, Message>>> = Arc::new(RwLock::new(cache::Cache::new(Duration::from_secs(10))));
		// SSH Handler
		let ssh_handler = SSHClientHandler::new(self.ctx.get_ssh_mtu(), ssh_tx.clone(), ssh_rx_reverse.clone());
		loop {
            select! {
				// PING interval
				_ = interval.tick() => {
					// Send ping
					debug!("Send PING to server");
					tx.write().await.send(Message::ping()).unwrap();
					// clear cache
					http_cache.write().await.compact().await;
				},
				// 从服务端接收请求
                _ = stream.readable() => match Message::from_stream(&self.ctx, &stream).await {
					Ok(msg) => match msg {
						Some(msg) if msg.is_ok() => {
							info!("Received OK from server, prepare to encrypted the random bytes");
							let bytes = match msg.get_random_bytes_from_server() {
								Ok(bytes) => bytes,
								Err(_) => {
									return Err("Server don't send random bytes".into());
								}
							};
							let encrypted = crypto::encrypt(self.ctx.get_secret(), bytes.to_vec());
							tx.write().await.send(Message::nat_auth(encrypted)).unwrap();
						},
						Some(msg) if msg.is_auth_timeout() => {
							error!("Auth timeout");
							let _ = stream.shutdown().await;
							return Err("Auth timeout".into());
						},
						Some(msg) if msg.is_auth_failed() => {
							error!("Auth failed");
							let _ = stream.shutdown().await;
							return Err("Auth failed".into());
						},
						Some(msg) if msg.is_rejected() => {
							error!("Server reject us");
							let _ = stream.shutdown().await;
							return Err("Server rejected us".into());
						},
						Some(msg) if msg.is_pong() => {
							debug!("You received PONG from NAT server");
						},
						Some(msg) if msg.is_http() =>  {
							http_tx.send(msg).unwrap();
						},
						// SSH 消息，且当前未创建本地 SSH
						// 检查当前是否已有 SSH 连接，如果没有，先创建连接，连接不停等待
						Some(msg) if msg.is_ssh() && !*ssh_existed.read().await => {
							info!("Received SSH request from server");
							// 检查进来的流量是否为请求建立 SSH，如果不是，则忽略
							match std::str::from_utf8(&msg.body) {
								Ok(r) if !r.contains("SSH") => {
									warn!("Dirty SSH message")
								},
								Ok(_) => {
									ssh_handler.run_backend(msg).await;
									*ssh_existed.write().await = true;
								},
								Err(_) => {
									warn!("Dirty SSH message");
								},
							}
						},
						// SSH 消息，且当前已创建 SSH 连接
						Some(msg) if msg.is_ssh() && *ssh_existed.read().await => {
							if msg.ssh_status != Some(SSHStatus::Ok) {
								*ssh_existed.write().await = false;
							}
							// 将消息发送给 SSH thread
							ssh_tx_reverse.send(msg).unwrap();
						},
						Some(_) => {
							let _ = stream.shutdown().await;
							return Err("Unknown message, maybe your password incorrect, closed connection".into());
						},
						None => {}
					},
					Err(err) => {
						error!("Read from server failed: {:?}", err);
						// Stop SSH handler
						ssh_handler.stop().await;
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
						tx.write().await.send(msg).unwrap();
					},
					None => {}
				},
				// Read and send to server
                msg = rx.recv() => match msg {
                    Some(msg) => {
                        // Send to server
						debug!("Write message to server: {:?} {:?}", msg.protocol, msg.body.len());
                        match msg.write_to(&self.ctx, &stream).await {
							Ok(_) => {},
							Err(e) => {
								error!("Write to server failed: {:?}", e);
								// TODO 断线重连
							},
						}
                    },
                    None => {},
                },
				// Http request coming
				msg = http_rx.recv() => match msg {
					Some(msg) => {
						let tx = tx.clone();
						let ctx = self.ctx.clone();
						// 分包读取 http request
						let tracing_id = msg.tracing_id.unwrap_or(0);
						let packet_size = msg.packet_size.unwrap_or(msg.body.len() as u32);
						let mut msg = msg;
						if http_cache.read().await.contains_key(&tracing_id) || packet_size > msg.body.len() as u32 {
							// 需要分包读取
							let mut cached = http_cache.read().await.get(&tracing_id).unwrap_or(Message::new_http(Some(tracing_id), vec![], packet_size));
							cached.body = vec![cached.body, msg.body].concat();
							if packet_size <= cached.body.len() as u32 {
								// 清理缓存
								http_cache.write().await.remove(&tracing_id);
								msg = cached;
							} else {
								http_cache.write().await.put(tracing_id, cached);
								continue;
							}
						}
						// Start a async task to handle this message
						tokio::spawn(async move {
							match handle_http(&msg).await {
								Ok(res) => {
									// Send to server
									utils::send_http_by_batch(msg.tracing_id.unwrap(), ctx.get_http_mtu(), tx.clone(), res.as_bytes().to_vec()).await;
								},
								Err(e) => {
									error!("Redirect http request failed: {:?}", e);
									if e.source().is_some() {
										if "operation timed out" == format!("{}", e.source().unwrap()) {
											tx.write().await.send(Message::http_504(msg.tracing_id)).unwrap();
										} else {
											tx.write().await.send(Message::http_502(msg.tracing_id)).unwrap();
										}
									} else {
										tx.write().await.send(Message::http_502(msg.tracing_id)).unwrap();
									}
								}
							}
						});
					},
					None => {}
				}
            };
        }
		Ok(())
    }
}
