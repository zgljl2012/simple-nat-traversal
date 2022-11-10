use std::{time::Duration, sync::Arc};

use tokio::{time, select, sync::{RwLock, mpsc::{UnboundedSender, UnboundedReceiver}}, net::TcpStream, io::AsyncWriteExt};

use crate::{Message, Context, crypto, utils, cache};


pub struct NatServerHandler {
	client_cnt: Arc<RwLock<usize>>,
	ncm_tx: Arc<RwLock<UnboundedSender<Message>>>,
	ncm_rx: Arc<RwLock<UnboundedReceiver<Message>>>,
	client_reply_tx:  Arc<RwLock<UnboundedSender<Message>>>
}

impl NatServerHandler {
	pub fn new(
		ncm_tx: Arc<RwLock<UnboundedSender<Message>>>,
		ncm_rx: Arc<RwLock<UnboundedReceiver<Message>>>,
		client_reply_tx:  Arc<RwLock<UnboundedSender<Message>>>
	) -> Self {
		Self {
			client_cnt: Arc::new(RwLock::new(0)),
			ncm_tx,
			ncm_rx,
			client_reply_tx
		}
	}

	pub async fn client_existed(&self) -> bool {
		*self.client_cnt.read().await > 0
	}

	// Start handler
	pub async fn run_server_backend(&mut self, ctx: Context, mut stream: TcpStream) -> Result<(), Box<dyn std::error::Error>> {
		// Check if have connection already established
		if self.client_existed().await {
			log::error!("Only support only one NAT client at a time");
			let _ = Message::nat_reject().write_to(&ctx, &stream).await;
			// shutdown the connect with anther client
			let _ = stream.shutdown().await;
			return Ok(());
		}
		// 开始握手连接, 发送 OK，以及生成一个随机数，等待 Client 加密进行认证认证
		// Client 需要对此随机数进行加密，服务端将密文解密后进行对比，若相同，则认证通过
		let random_bytes_for_clinet = utils::random_bytes();
		match Message::nat_ok(random_bytes_for_clinet).write_to(&ctx, &stream).await {
			Ok(_) => {
				log::debug!("Send OK message to client successfully");
			}
			Err(e) => {
				return Err(format!("Send reply to client failed: {:?}", e).into());
			}
		};
		let ncm_rx = self.ncm_rx.clone();
		let ncm_tx = self.ncm_tx.clone();
		let client_reply_tx = self.client_reply_tx.clone();
		let client_cnt = self.client_cnt.clone();
		*client_cnt.write().await += 1;
		tokio::spawn(async move {
			// 超时未认证，则关闭连接
			let mut auth_success = false;
			let mut interval = time::interval(Duration::from_secs(5));
			let mut ticker = 0;
			let http_cache: Arc<RwLock<cache::Cache<u32, Message>>> = Arc::new(RwLock::new(cache::Cache::new(Duration::from_secs(10))));
			let mut ncm_rx = ncm_rx.write().await;
			loop {
				select! {
					_ = interval.tick() => {
						if ticker == 1 && !auth_success {
							log::error!("Nat client auth timeout");
							let _ = Message::nat_auth_timeout().write_to(&ctx, &stream).await;
							let _ = stream.shutdown();
							break;
						}
						ticker += 1;
					},
					// 读取消息，发送给 Client
					msg = ncm_rx.recv() => match msg {
						Some(msg) => match msg.write_to(&ctx, &stream).await {
							Ok(_) => {
								log::debug!("Send request to client successfully: {:?} {:?}", msg.protocol, msg.body.len());
							}
							Err(e) => {
								// Send to client error
								log::error!("Send request to client failed: {}", e);
								break;
							}
						},
						None => {}
					},
					// 从 Client 读取消息
					r = stream.readable() => match r {
						Ok(_) => match Message::from_stream(&ctx, &stream).await {
							Ok(msg) => match msg {
								Some(msg) if msg.is_auth() => {
									log::info!("Client auth...");
									let secret = match msg.get_encrypted_bytes_by_client() {
										Ok(bytes) => bytes,
										Err(e) => {
											log::error!("Read auth message from client failed: {}", e);
											// Close the connection
											let _ = Message::nat_auth_failed().write_to(&ctx, &stream).await;
											let _ = stream.shutdown();
											break;
										}
									};
									// decrypt it
									let decrypted = crypto::decrypt(ctx.get_secret(), secret.to_vec());
									// 验证是否解密正确
									if utils::as_u32_be(&random_bytes_for_clinet).unwrap() == utils::as_u32_be(&decrypted).unwrap() {
										log::info!("Client auth successfully");
										auth_success = true;
									} else {
										let _ = Message::nat_auth_failed().write_to(&ctx, &stream).await;
										let _ = stream.shutdown();
										break;
									}
								},
								Some(msg) if msg.is_ping() => {
									// 如果收到 Ping，就直接 Pong
									log::debug!("You received PING from NAT client");
									ncm_tx.write().await.send(Message::pong()).unwrap();
								},
								Some(msg) if msg.is_http() => {
									log::debug!("Received HTTP Response from client");
									// 根据 packet_size，进行分包读取
									let packet_size = msg.packet_size.unwrap_or(msg.body.len() as u32);
									let tracing_id = msg.tracing_id.unwrap_or(0);
									if http_cache.read().await.contains_key(&tracing_id) || packet_size > msg.body.len() as u32 {
										// 需要分包读取
										let mut cached = http_cache.read().await.get(&tracing_id).unwrap_or(Message::new_http(Some(tracing_id), vec![], packet_size));
										cached.body = vec![cached.body, msg.body.clone()].concat();
										if packet_size <= cached.body.len() as u32 {
											// 清理缓存
											http_cache.write().await.remove(&tracing_id);
											// 发送真正的 Message
											let _ = client_reply_tx.write().await.send(cached);
										} else {
											http_cache.write().await.put(tracing_id, cached);
										}
									} else {
										let _ = client_reply_tx.write().await.send(msg);
									}
								},
								Some(msg) if msg.is_ssh() => {
									log::info!("Received SSH Response from client");
									let _ = client_reply_tx.write().await.send(msg);
								},
								Some(msg) => {
									log::debug!("Received {:?} {:?}", msg.protocol, msg.body);
								}
								None => {
									log::debug!("Empty message from client");
								}
							},
							Err(err) => {
								log::error!("{}", err);
								break;
							},
						},
						Err(err) => {
							log::error!("Read from cliend failed {}", err);
							break;
						},
					},
				}
			}
			// Break loop
			*client_cnt.write().await -= 1;
		});
		Ok(())
	}
}
