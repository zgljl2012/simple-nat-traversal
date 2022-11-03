use tokio::net::TcpStream;

use crate::{Message, utils};


// 目前，Client 仅能代替一个 SSH 端口，默认代理 Client 所在的服务器，线上环境，建议代理跳板机（堡垒机）
pub async fn handle_ssh(msg: &Message) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
	let target = "127.0.0.1:22";
	let stream = TcpStream::connect(&target).await?;
	match stream.try_write(&msg.body) {
		Ok(_) => {},
		Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {},
		Err(e) => {
			return Err(Box::new(format!("Write to {:?} failed: {:?}", target, e)).as_str().into());
		}
	};
	stream.readable().await?;
	// Read the reply
	// 获取所有请求报文
	let bytes = utils::get_packet_from_stream(&stream);
	Ok(bytes)
}
