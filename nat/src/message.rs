use std::vec;

use log::{debug, error, warn};
use tokio::net::TcpStream;

use crate::{protocols::ProtocolType, utils, checksum::{self}, Context, crypto::{encrypt, decrypt}};

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
const NAT_OK: [u8; 1] = [0x2];
const NAT_REJEXT: [u8; 1] = [0x3];

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
	// 具体消息先 AES 加密（因 SSH 本身就是加密流量，故不加密 SSH 消息），消息头不加密
    // 前 2 个字节表示加密后的消息的字节数，无符号 u16，即最多支持 2^16 次方的消息长度，即 64 KB
	// 3、4 字节表示消息的 Checksum
    // 第 5 个字节表示协议类型: NAT(0x0), HTTP(0x1), SSH(0x2)
    // 除 NAT 类型外，第 6-9 共 4 字节表示 tracing ID
	// 对于 SSH 协议，第 10 个字节表示状态编码（0: 正常，1: 未知错误，2:...）
    pub async fn from_stream(
		ctx: &Context,
        stream: &TcpStream,
    ) -> Result<Option<Message>, Box<dyn std::error::Error + Send + Sync>> {
        let mut buffer = [0;2];
		// 此处必须按字节老老实实一个个解析，因为如果一次性读取，先读取再解析，会导致读取了下一条的消息内容
		// 导致本条消息出错，即便是处理了本条消息，也会导致下一条消息不完整
        match stream.try_read(&mut buffer) {
            Ok(0) => Err("Connection closed".into()),
            Ok(_) => {
				debug!("Message received");
                let data_size = match utils::as_u16_be(buffer.as_slice()) {
					Ok(size) => size,
					Err(err) => {
						return Err(format!("Parse data size failed: {}", err).into());
					}
				};
				// full bytes message
				let mut bytes: Vec<u8> = Vec::new();
				bytes.append(&mut buffer.clone().to_vec());
				debug!("You received {:?} bytes from NAT stream", data_size);
				// Checksum
				let mut buf = [0;2];
				stream.try_read(&mut buf).unwrap();
				bytes.append(&mut buf.clone().to_vec());

                // read protocol type
                let mut protocol_type_buf = [0;1];
                stream.try_read(&mut protocol_type_buf).unwrap();
				bytes.append(&mut protocol_type_buf.clone().to_vec());
                let protocol_type = match ProtocolType::from_slice(protocol_type_buf.as_slice()) {
                    Some(pt) => pt,
                    None => {
                        return Err(format!(
                            "Uncognizaed protocol type: {:?}",
                            protocol_type_buf.as_slice()[0] as u8,
                        )
                        .into());
                    }
                };
				// read tracing_id
				let mut tracing_id: Option<u32> = None;
				if protocol_type != ProtocolType::NAT {
					let mut tracing_id_buf = [0;4];
					stream.try_read(&mut tracing_id_buf).unwrap();
					bytes.append(&mut tracing_id_buf.clone().to_vec());
					tracing_id = Some(utils::as_u32_be(tracing_id_buf.as_slice())?);
				}

				// If SSH, read status
				let mut ssh_status: Option<SSHStatus> = None;
				if protocol_type == ProtocolType::SSH {
					let mut status = [0;1];
					stream.try_read(&mut status).unwrap();
					bytes.append(&mut status.clone().to_vec());
					ssh_status = Some(SSHStatus::from_u8(status[0] as u8));
				}

                // Read other bytes
                let mut data = Vec::with_capacity(data_size as usize);
				let mut rest: usize = data_size as usize;
				loop {
					let mut buf: Vec<u8> = Vec::with_capacity(rest);
					match stream.try_read_buf(&mut buf) {
						Ok(n) => {
							data.append(&mut buf[0..n].to_vec());
							if data.len() >= data_size as usize {
								break;
							}
							rest -= n;
						},
						Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
							warn!("Message data should receive {} bytes, still have {} bytes not received, but stream maybe blocked, so break the loop", data.len(), rest);
							break;
						},
						Err(e) => {
							error!("Read error: {}", e);
							break;
						},
					};
				}
				bytes.append(&mut data.to_vec());
				// Validate checksum
				if checksum::checksum(&bytes) != 0 {
					return Err("Checksum is not correct".into());
				}
				// decrypt data
				let mut decrypted = data.clone();
				if protocol_type != ProtocolType::SSH {
					decrypted = decrypt(ctx.get_secret(), decrypted);
				}
                Ok(Some(Message {
                    protocol: protocol_type,
                    body: decrypted,
					tracing_id,
					ssh_status,
                }))
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(format!("Unexpected error: {}", e).into()),
        }
    }

    // Write standard message to stream
    pub async fn write_to(&self, ctx: &Context, stream: &TcpStream) -> Result<(), Box<dyn std::error::Error>> {
		// Encrypt message
		let mut encrypted = self.body.clone();
		if self.protocol != ProtocolType::SSH {
			encrypted = encrypt(ctx.get_secret(), encrypted);
		}
        let size: u32 = encrypted.len() as u32;
		if size >= 2_u32.pow(16) {
			return Err("Message too big".into())
		}
        let size_arr = utils::u16_to_be(size as u16);
		// init checksum, set to zero
		let init_checksum: [u8; 2] = [0, 0];
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
        let mut r: Vec<u8> = [
            &size_arr,
			&init_checksum,
            self.protocol.bytes().to_vec().as_slice(),
			tracing_id.as_slice(),
			ssh_status.as_slice(),
            encrypted.as_slice(),
        ]
        .concat();
		// Calculate checksum
		let checksum = utils::u16_to_be(checksum::checksum(r.as_slice()));
		r[2] = checksum[0];
		r[3] = checksum[1];

        stream.writable().await.unwrap();
        match stream.try_write(&r.as_slice()) {
			Ok(_) => {
				debug!("write message successfully");
				return Ok(());
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

	pub fn is_rejected(&self) -> bool {
		self.protocol == ProtocolType::NAT && self.body == NAT_REJEXT
	}

    pub fn ping() -> Self {
        Self{protocol: ProtocolType::NAT, body: PING_BYTES.to_vec(), tracing_id: None, ssh_status: None,}
    }

    pub fn pong() -> Self {
        Self{protocol: ProtocolType::NAT, body: PONG_BYTES.to_vec(), tracing_id: None, ssh_status: None,}
    }

	pub fn nat_ok() -> Self {
		Self{protocol: ProtocolType::NAT, body: NAT_OK.to_vec(), tracing_id: None, ssh_status: None,}	
	}

	pub fn nat_reject() -> Self {
		Self{protocol: ProtocolType::NAT, body: NAT_REJEXT.to_vec(), tracing_id: None, ssh_status: None,}	
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
