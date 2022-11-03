use crate::{Message, utils::{self}, NatStream};

// 目前，Client 仅能代替一个 SSH 端口，默认代理 Client 所在的服务器，线上环境，建议代理跳板机（堡垒机）
pub async fn handle_ssh(stream: &NatStream, msg: &Message) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
	match stream.write().await.try_write(&msg.body) {
		Ok(_) => {},
		Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {},
		Err(e) => {
			return Err(Box::new(format!("Write to local SSH connection failed: {:?}", e)).as_str().into());
		}
	};
	stream.write().await.readable().await?;
	// Read the reply
	// 获取所有请求报文
	let s = stream.write().await;
	let bytes = utils::get_packet_from_stream(&s);
	Ok(bytes)
}
