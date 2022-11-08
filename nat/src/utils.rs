use std::sync::Arc;

use log::{error, debug};
use tokio::{net::TcpStream, sync::{RwLock, mpsc::UnboundedSender}};
use rand::Rng;

use crate::{Message, Connection};

// 大端序(Big endian)，字节转 u32
pub fn as_u32_be(array: &[u8]) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
	if array.len() != 4 {
		return Err(format!("This is not a big endian 4 bytes array, you passed in {:?}", array.len()).into());
	} 
    Ok(((array[0] as u32) << 24) +
    ((array[1] as u32) << 16) +
    ((array[2] as u32) <<  8) +
    ((array[3] as u32) <<  0))
}

pub fn as_u16_be(array: &[u8]) -> Result<u16, Box<dyn std::error::Error>> {
	if array.len() != 2 {
		return Err(format!("This is not a big endian 2 bytes array, you passed in {:?}", array.len()).into());
	}
    Ok(((array[0] as u16) << 8) +
    ((array[1] as u16) << 0))
}

#[allow(unused)]
pub fn u16_to_be(x: u16) -> [u8;2] {
	let b1 : u8 = ((x >> 8) & 0xff) as u8;
    let b2 : u8 = (x & 0xff) as u8;
	return [b1, b2]
}

// u32 to big endian
pub fn u32_to_be(x:u32) -> [u8;4] {
    let b1 : u8 = ((x >> 24) & 0xff) as u8;
    let b2 : u8 = ((x >> 16) & 0xff) as u8;
    let b3 : u8 = ((x >> 8) & 0xff) as u8;
    let b4 : u8 = (x & 0xff) as u8;
    return [b1, b2, b3, b4]
}

pub fn get_packet_from_stream(stream: &TcpStream) -> Vec<u8> {
	// 获取所有请求报文
	let mut buffer = [0;4096]; // Vec::with_capacity(4096);
	let mut bytes:Vec<u8> = Vec::new();
	loop {
		match stream.try_read(&mut buffer) {
			Ok(0) => break,
			Ok(n) => {
				bytes.append(&mut buffer[0..n].to_vec());
			},
			Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
			Err(err) => {
				error!("Read failed: {:?}", err);
				break;
			}
		};
	}
	debug!("packet size: {:?}", bytes.len());
	bytes
}

pub fn random_bytes() -> [u8;4] {
	u32_to_be(rand::thread_rng().gen::<u32>())
}

pub async fn send_ssh_by_batch(tracing_id: u32, batch_size: usize, tx: Arc<RwLock<UnboundedSender<Message>>>, bytes: &[u8]) {
	let mut i: usize = 0;
	loop {
		let mut end = i + batch_size;
		if end > bytes.len() {
			end = bytes.len();
		}
		tx.write().await.send(Message::new_ssh(Some(tracing_id), bytes[i..end].to_vec())).unwrap();	
		i += batch_size;
		if i >= bytes.len() {
			break;
		}
	}
}

pub async fn send_http_by_batch(tracing_id: u32, batch_size: usize, tx: Arc<RwLock<UnboundedSender<Message>>>, bytes: Vec<u8>) {
	// 根据 http_mtu 分包发送，server 端根据 content-length 进行读取
	let mut i = 0;
	let bytes_len = bytes.len() as u32;
	loop {
		let end = std::cmp::min(bytes.len(), i+ batch_size);
		tx.write().await.send(Message::new_http(Some(tracing_id), bytes[i..end].to_vec(), bytes_len)).unwrap();
		i += batch_size;
		if i >= bytes.len() {
			break;
		}
	}
}

pub async fn get_packets(conn: Arc<RwLock<Connection>>) -> Vec<u8> {
	// 获取所有请求报文
	let mut buffer = [0;1024];
	let mut bytes:Vec<u8> = Vec::new();
	let mut init_buf = conn.read().await.init_buf.clone();
	bytes.append(&mut init_buf);
	loop {
		match conn.write().await.stream.try_read(&mut buffer) {
			Ok(0) => break,
			Ok(n) => {
				bytes.append(&mut buffer[0..n].to_vec());
			},
			Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
			Err(err) => {
				error!("Read failed: {:?}", err);
				break;
			}
		};
	}
	debug!("request size: {:?}", bytes.len());
	bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_u32() {
        let i: [u8; 4] = [0, 0, 0xa, 1];
        println!("\n{:?}", as_u32_be(&i));
        println!("{:?}\n", u32_to_be(as_u32_be(&i).unwrap()));
    }
}
