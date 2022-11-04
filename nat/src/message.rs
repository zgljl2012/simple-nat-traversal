use std::vec;

use log::{debug, error};
use tokio::net::TcpStream;

use crate::{protocols::ProtocolType, utils};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SSHStatus {
	Ok = 0,
	UnknownError = 1,
}

impl SSHStatus {
	fn from_u8(status: u8) -> Self {
		match status {
			0 => Self::Ok,
			_ => Self::UnknownError
		}
	}

	fn to_u8(&self) -> u8 {
		(*self as u8).clone()
	}
}

#[derive(Debug, Clone)]
pub struct Message {
    pub protocol: ProtocolType,
    pub body: Vec<u8>,
	pub tracing_id: Option<u32>,
	pub ssh_status: Option<SSHStatus>,
}

const PING_BYTES: [u8; 1] = [0x0];
const PONG_BYTES: [u8; 1] = [0x1];

impl Message {
    pub fn new_http(tracing_id: Option<u32>, body: Vec<u8>) -> Self {
        Self {
            protocol: ProtocolType::HTTP,
            body,
			tracing_id: tracing_id,
			ssh_status: None,
        }
    }
	pub fn new_ssh(tracing_id: Option<u32>, body: Vec<u8>) -> Self {
		Self {
			protocol: ProtocolType::SSH,
			body,
			tracing_id: tracing_id,
			ssh_status: Some(SSHStatus::Ok),
		}
	}
	pub fn new_ssh_error(tracing_id: Option<u32>) -> Self {
		Self {
			protocol: ProtocolType::SSH,
			body: vec![],
			tracing_id: tracing_id,
			ssh_status: Some(SSHStatus::UnknownError),
		}
	}
    pub fn http_502(tracing_id: Option<u32>) -> Self {
        Self {
            protocol: ProtocolType::HTTP,
            body: "HTTP/1.1 502 Bad Gateway\r\n".as_bytes().to_vec(),
			tracing_id: tracing_id,
			ssh_status: None,
        }
    }
    pub fn http_504(tracing_id: Option<u32>) -> Self {
        Self {
            protocol: ProtocolType::HTTP,
            body: "HTTP/1.1 504 Bad Timeout\r\n".as_bytes().to_vec(),
			tracing_id: tracing_id,
			ssh_status: None,
        }
    }

    // 二进制协议 - NAT 通信协议
    // 前 4 个字节表示消息的字节数，无符号 u32，即最多支持 2^32 次方的消息长度，即 4G
    // 第 5 个字节表示协议类型: NAT(0x0), HTTP(0x1), SSH(0x2)
    // 除 NAT 类型外，第 6-9 共 4 字节表示 tracing ID
	// 对于 SSH 协议，第 10 个字节表示状态编码（0: 正常，1: 未知错误，2:...）
    pub async fn from_stream(
        stream: &TcpStream,
    ) -> Result<Option<Message>, Box<dyn std::error::Error + Send + Sync>> {
        let mut buffer = Vec::with_capacity(4);
        match stream.try_read_buf(&mut buffer) {
            Ok(0) => Err("Connection closed".into()),
            Ok(_) => {
				debug!("Message received");
                let data_size = utils::as_u32_be(buffer.as_slice())?;
                debug!("You received {:?} bytes from NAT stream", data_size);
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
					tracing_id = Some(utils::as_u32_be(tracing_id_buf.as_slice())?);
				}

				// If SSH, read status
				let mut ssh_status: Option<SSHStatus> = None;
				if protocol_type == ProtocolType::SSH {
					let mut status = Vec::with_capacity(1);
					stream.try_read_buf(&mut status).unwrap();
					ssh_status = Some(SSHStatus::from_u8(status[0] as u8));
				}

                // Read other bytes
                let mut data = Vec::with_capacity(data_size as usize);
                match stream.try_read_buf(&mut data) {
                    Ok(_) => {},
					Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {},
                    Err(e) => {
						error!("Read error: {}", e);
					},
                };
                Ok(Some(Message {
                    protocol: protocol_type,
                    body: data,
					tracing_id,
					ssh_status,
                }))
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(format!("Unexpected error: {}", e).into()),
        }
    }

    // Write standard message to stream
    pub async fn write_to(&self, stream: &TcpStream) -> Result<(), Box<dyn std::error::Error>> {
        let size: u32 = self.body.len() as u32;
        let size_arr = utils::u32_to_be(size);
		// tracing_id
		let tracing_id: Vec<u8> = match self.tracing_id {
			Some(id) => utils::u32_to_be(id).to_vec(),
			None => vec![]
		};
		// ssh status
		let ssh_status: Vec<u8> = match &self.ssh_status {
			Some(status) => vec![status.to_u8()],
			None => {
				if self.protocol == ProtocolType::SSH {
					vec![0x0]
				} else {
					vec![]
				}
			},
		};
        let r: Vec<u8> = [
            &size_arr,
            self.protocol.bytes().to_vec().as_slice(),
			tracing_id.as_slice(),
			ssh_status.as_slice(),
            self.body.as_slice(),
        ]
        .concat();
        stream.writable().await.unwrap();
        match stream.try_write(&r.as_slice()) {
			Ok(_) => {
				debug!("write message successfully");
				return Ok(());
			},
			Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
				return Ok(())
			},
			Err(e) => {
				Err(format!("Write {:?} message#{:?} failed: {:?}", self.protocol, self.tracing_id, e).into())
			}
		}
    }

    pub fn is_http(&self) -> bool {
        self.protocol == ProtocolType::HTTP
    }

	pub fn is_ssh(&self) -> bool {
		self.protocol == ProtocolType::SSH
	}

    pub fn is_ping(&self) -> bool {
        self.protocol == ProtocolType::NAT && self.body == PING_BYTES
    }

    pub fn is_pong(&self) -> bool {
        self.protocol == ProtocolType::NAT && self.body == PONG_BYTES
    }

    pub fn ping() -> Self {
        Self{protocol: ProtocolType::NAT, body: PING_BYTES.to_vec(), tracing_id: None, ssh_status: None,}
    }

    pub fn pong() -> Self {
        Self{protocol: ProtocolType::NAT, body: PONG_BYTES.to_vec(), tracing_id: None, ssh_status: None,}
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
