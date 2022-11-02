use std::vec;

use log::{debug, error};
use tokio::net::TcpStream;

use crate::{protocols::ProtocolType, utils};


#[derive(Debug, Clone)]
pub struct Message {
    pub protocol: ProtocolType,
    pub body: Vec<u8>,
	pub tracing_id: Option<u32>,
}

const PING_BYTES: [u8; 1] = [0x0];
const PONG_BYTES: [u8; 1] = [0x1];

impl Message {
    pub fn new_http(tracing_id: Option<u32>, body: Vec<u8>) -> Self {
        Self {
            protocol: ProtocolType::HTTP,
            body,
			tracing_id: tracing_id,
        }
    }
    pub fn http_502(tracing_id: Option<u32>) -> Self {
        Self {
            protocol: ProtocolType::HTTP,
            body: "HTTP/1.1 502 Bad Gateway\r\n".as_bytes().to_vec(),
			tracing_id: tracing_id,
        }
    }
    pub fn http_504(tracing_id: Option<u32>) -> Self {
        Self {
            protocol: ProtocolType::HTTP,
            body: "HTTP/1.1 504 Bad Timeout\r\n".as_bytes().to_vec(),
			tracing_id: tracing_id,
        }
    }

    // 二进制协议 - NAT 通信协议
    // 前 4 个字节表示消息的字节数，无符号 u32，即最多支持 2^32 次方的消息长度，即 4G
    // 第 5 个字节表示协议类型: NAT(0x0), HTTP(0x1), SSH(0x2)
    // 除 NAT 类型外，第 6-9 共 4 字节表示 tracing ID
    pub async fn from_stream(
        stream: &TcpStream,
    ) -> Result<Option<Message>, Box<dyn std::error::Error + Send + Sync>> {
        let mut buffer = Vec::with_capacity(4);
        match stream.try_read_buf(&mut buffer) {
            Ok(0) => Err("Connection closed".into()),
            Ok(_) => {
                let data_size = utils::as_u32_be(buffer.as_slice());
                debug!("You received {:?} bytes from NAT client", data_size);
                // read protocol type
                let mut protocol_type_buf = Vec::with_capacity(1);
                stream.try_read_buf(&mut protocol_type_buf).unwrap();
                let protocol_type = match ProtocolType::from_slice(protocol_type_buf.as_slice()) {
                    Some(pt) => pt,
                    None => {
                        return Err(format!(
                            "Uncognizaed protocol type: {:?}",
                            utils::as_u32_be(protocol_type_buf.as_slice())
                        )
                        .into());
                    }
                };
				// read tracing_id
				let mut tracing_id: Option<u32> = None;
				if protocol_type != ProtocolType::NAT {
					let mut tracing_id_buf = Vec::with_capacity(4);
					stream.try_read_buf(&mut tracing_id_buf).unwrap();
					tracing_id = Some(utils::as_u32_be(tracing_id_buf.as_slice()));
				}

                // Read other bytes
                let mut data = Vec::with_capacity(data_size as usize);
                stream.try_read_buf(&mut data).unwrap();
                Ok(Some(Message {
                    protocol: protocol_type,
                    body: data,
					tracing_id: tracing_id,
                }))
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(format!("Unexpected error: {}", e).into()),
        }
    }

    // Write standard message to stream
    pub async fn write_to(&self, stream: &TcpStream) {
        let size: u32 = self.body.len() as u32;
        let size_arr = utils::u32_to_be(size);
		// tracing_id
		let tracing_id: Vec<u8> = match self.tracing_id {
			Some(id) => utils::u32_to_be(id).to_vec(),
			None => vec![]
		};
        let r: Vec<u8> = [
            &size_arr,
            self.protocol.bytes().to_vec().as_slice(),
			tracing_id.as_slice(),
            self.body.as_slice(),
        ]
        .concat();
        stream.writable().await.unwrap();
        match stream.try_write(&r.as_slice()) {
			Ok(_) => {
				debug!("write message successfully");
			}
			Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
			Err(e) => {
				error!("Write {:?} message#{:?} failed: {:?}", self.protocol, self.tracing_id, e);
			}
		}
    }

    pub fn is_http(&self) -> bool {
        self.protocol == ProtocolType::HTTP
    }

    pub fn is_ping(&self) -> bool {
        self.protocol == ProtocolType::NAT && self.body == PING_BYTES
    }

    pub fn is_pong(&self) -> bool {
        self.protocol == ProtocolType::NAT && self.body == PONG_BYTES
    }

    pub fn ping() -> Self {
        Self{protocol: ProtocolType::NAT, body: PING_BYTES.to_vec(), tracing_id: None,}
    }

    pub fn pong() -> Self {
        Self{protocol: ProtocolType::NAT, body: PONG_BYTES.to_vec(), tracing_id: None}
    }

	pub fn to_utf8(&self) -> &str {
		match std::str::from_utf8(&self.body) {
			Ok(s) => s,
			Err(e) => {
				error!("Convert body to utf8 failed: {:?}", e);
				""
			}
		}
	}

}
