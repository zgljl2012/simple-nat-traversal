use std::{sync::Arc, collections::HashMap};

use tokio::{sync::{RwLock, mpsc::{UnboundedSender, self}}, select, io::AsyncWriteExt};

use crate::{utils::{get_packet_from_stream, get_packets}, Message, Connection, SSHStatus};


pub struct SSHServerHandler {
	ssh_mtu: usize,
	ncm_tx: Arc<RwLock<UnboundedSender<Message>>>,
	ssh_conns_tx: Arc<RwLock<HashMap<u32, UnboundedSender<Message>>>>
}

impl SSHServerHandler {
	pub fn new(ssh_mtu: usize, ncm_tx: Arc<RwLock<UnboundedSender<Message>>>) -> Self {
		Self {
			ssh_mtu,
			ncm_tx,
			ssh_conns_tx: Arc::new(RwLock::new(HashMap::new()))
		}
	}

	pub async fn handle_reply(&self, msg: Message) {
		log::info!("Received SSH reply from client");
		match self.ssh_conns_tx.read().await.get(&msg.tracing_id.unwrap()) {
			Some(tx) => {
				tx.send(msg).unwrap();
			},
			None => {}
		};
	}

	pub async fn run_server_backend(&self, tracing_id: u32, conn: Arc<RwLock<Connection>>) {
		log::info!("New SSH connection, currently count is {}", self.ssh_conns_tx.read().await.len());
		let bytes = get_packets(conn.clone()).await;
		let msg = Message::new_ssh(Some(tracing_id), bytes);
		let ncm_tx = self.ncm_tx.clone();
		let batch_size = self.ssh_mtu;

		// Send init message to client
		ncm_tx.write().await.send(msg).unwrap();

		// Create sender and receiver
		let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
		// Save tx
		self.ssh_conns_tx.write().await.insert(tracing_id, tx);
		let ssh_conns_tx = self.ssh_conns_tx.clone();
		tokio::spawn(async move {
			let mut conn = conn.write().await;
			loop {
				select! {
					// Read from stream
					s = conn.stream.readable() => match s {
						Ok(_) => {
							let bytes = get_packet_from_stream(&conn.stream);
							log::info!("Read from SSH stream: {:?}, and send to client", bytes.len());
							if bytes.len() > 0 {
								// Send to client
								ncm_tx.write().await.send(Message::new_ssh(Some(tracing_id), bytes)).unwrap();
							} else {
								log::error!("SSH disconnected");
								// 通知 Client, SSH disconnected
								ncm_tx.write().await.send(Message::new_ssh_error(Some(tracing_id))).unwrap();
								break;
							}
						},
						Err(e) => {
							log::error!("SSH disconnected: {}", e);
							// 通知 Client 端，断开 SSH 连接
							ncm_tx.write().await.send(Message::new_ssh_error(Some(tracing_id))).unwrap();
							break;
						}
					},
					// Read from NAT client
					msg = rx.recv() => match msg {
						Some(msg) => {
							// Write to stream
							if msg.ssh_status != Some(SSHStatus::Ok) {
								log::error!("SSH of client have been disconnected");
								conn.stream.shutdown().await.unwrap();
								break;
							} else {
								log::info!("Try to send {} bytes to SSH connection", msg.body.len());
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
											log::info!("Send {} bytes to SSH connection", n);
											i += batch_size;
											if i >= msg.body.len() {
												break;
											}
										}
										Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
										Err(e) => {
											has_error = true;
											log::error!("Write response to SSH stream failed: {:?}", e);
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
			// Close connection
			let _ = conn.stream.shutdown();
			ssh_conns_tx.write().await.remove(&tracing_id);
		});
	}
}
