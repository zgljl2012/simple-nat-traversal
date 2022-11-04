use log::{error, debug};
use tokio::net::TcpStream;


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
