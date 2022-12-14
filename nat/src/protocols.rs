// 协议

use std::{fmt::{Display, self}};

use tokio::net::TcpStream;

pub trait Protocol {
    fn validate(&self, line: String) -> bool;
    fn name(&self) -> &str;
}

// SSH protocol
struct SshProtocol {

}

impl SshProtocol {
    fn new() -> Box<dyn Protocol> {
        Box::new(SshProtocol {})
    }
}

impl Protocol for SshProtocol {
    fn name(&self) -> &str {
        "SSH"
    }
    fn validate(&self, line: String) -> bool {
        // SSH-2.0-OpenSSH_8.6
        line.contains("SSH")
    }
}

// Http protocol
struct HttpProtocol {
}

impl HttpProtocol {
    fn new() -> Box<dyn Protocol> {
        Box::new(HttpProtocol {})
    }
} 

impl Protocol for HttpProtocol {
    fn name(&self) -> &str {
        "HTTP"
    }
    fn validate(&self, line: String) -> bool {
        line.contains("HTTP")
    }
}

// Nat protocol
struct NatProtocol {
}

impl NatProtocol {
    fn new() -> Box<dyn Protocol> {
        Box::new(NatProtocol{})
    }
}

impl Protocol for NatProtocol {
    fn name(&self) -> &str {
        "NAT"
    }

    fn validate(&self, line: String) -> bool {
        line.contains("NAT")
    }
}


#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProtocolType {
    NAT, // Nat client
    SSH,
    HTTP,
}

impl ProtocolType {
    pub fn protocol(&self) -> Box<dyn Protocol> {
        match *self {
            ProtocolType::SSH => SshProtocol::new(),
            ProtocolType::HTTP => HttpProtocol::new(),
            ProtocolType::NAT => NatProtocol::new(),
        }
    }

    pub fn bytes (&self) -> [u8; 1] {
        match *self {
            ProtocolType::NAT => [0x0],
            ProtocolType::HTTP => [0x1],
            ProtocolType::SSH => [0x2]
        }
    }

    pub fn from_slice(bytes: &[u8]) -> Option<ProtocolType> {
        match bytes {
            [0x0] => Some(ProtocolType::NAT),
            [0x1] => Some(ProtocolType::HTTP),
            [0x2] => Some(ProtocolType::SSH),
            _ => None
        }
    }
}

impl Display for ProtocolType {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ProtocolType::HTTP => fmt.write_str("HTTP"),
            ProtocolType::NAT => fmt.write_str("NAT"),
            ProtocolType::SSH => fmt.write_str("SSH")
        }
    }
}

pub fn parse_protocol(line: String) -> Result<Box<dyn Protocol>, &'static str> {
    let protocols = [
        ProtocolType::SSH,
        ProtocolType::HTTP,
        ProtocolType::NAT,
    ];
    for protocol_type in &protocols {
        let protocol = protocol_type.protocol();
        if protocol.validate(line.clone()) {
            return Ok(protocol);
        }
    }
    Err("Don't support this tcp connect")
}

#[derive(Debug)]
pub struct Connection {
	pub init_buf: Vec<u8>, // 因为最开始会读取一行判断协议，故此处需加上
	pub stream: TcpStream,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol() {
        println!("{:?}", ProtocolType::SSH);
    }
}
