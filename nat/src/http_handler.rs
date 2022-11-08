use std::{collections::HashMap, sync::Arc};

use tokio::{sync::{RwLock, mpsc::UnboundedSender}};

use crate::{utils, Connection, Message};


pub struct HttpHandler {
	connections: HashMap<u32, Arc<RwLock<Connection>>>,
	http_mtu: usize,
	ncm_tx: Arc<RwLock<UnboundedSender<Message>>>
}

impl HttpHandler {
	pub fn new(ncm_tx: Arc<RwLock<UnboundedSender<Message>>>, http_mtu: usize) -> HttpHandler {
		let connections: HashMap<u32, Arc<RwLock<Connection>>> = HashMap::new();
		Self {
			connections,
			http_mtu,
			ncm_tx
		}
	}

	pub async fn handle(&mut self, tracing_id: u32, conn: Arc<RwLock<Connection>>) {
		// 处理请求的 Http，转发给 Nat Client
		let bytes = utils::get_packets(conn.clone()).await;
		let batch_size = self.http_mtu as usize;
		self.connections.insert(tracing_id, conn);
		utils::send_http_by_batch(tracing_id, batch_size, self.ncm_tx.clone(), bytes).await;
	}

	pub async fn handle_reply(&mut self, id: u32, msg: &Message) {
		// 收到 Nat Client 的回复，返回给访问者
		let connections = self.connections.clone();
		let conn = connections.get(&id);
		match conn {
			Some(conn) => {
				log::debug!("Received HTTP reply from client: {:?}", msg.to_utf8());
				match conn.write().await.stream.writable().await {
					Ok(_) => {},
					Err(err) => {
						log::error!("Wait stream for reply failed: {:?}, remove it", err);
						self.connections.remove(&id);
					}
				};
				match conn.write().await.stream.try_write(&msg.body.as_slice()) {
					Ok(_) => {
						log::debug!("write message to stream successfully");
						// shutdown
						if msg.is_http() {
							self.connections.remove(&id);
						}
					}
					Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
					Err(e) => {
						log::error!("Write response to stream failed: {:?}", e);
					}
				}
			},
			None => {
				log::error!("Tracing ID {:?} does not exist", id);
			}
		}
	}
}
