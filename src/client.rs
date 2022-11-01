//! Simple websocket client.

use std::{net::TcpStream, io::{Write, Read}, time::Duration};
use std::net::{SocketAddr};

pub struct ClientConfig {
    pub server_url: String,
}

pub async fn start_client(config: &ClientConfig) {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    log::info!("starting NAT client: {}", config.server_url);
    let server: SocketAddr = config.server_url.as_str().parse().expect("Unable to parse socket address");
    let mut stream = TcpStream::connect_timeout(&server, Duration::from_secs(5)).unwrap();
    // 发送连接语句
    stream.write("NAT 0.1".as_bytes()).unwrap();
    // 创建1k的缓冲区，用于接收server发过来的内容
    let mut buffer = [0;1024];
    loop { 
        // 设置超时时间
        match stream.set_read_timeout(Some(Duration::from_secs(10))) {
            Ok(_) => {},
            Err(err) => panic!("Set read timeout failed: {}", err),
        };
        // 读取server发过来的内容
        stream.read(&mut buffer).unwrap();
        let msg = std::str::from_utf8(&buffer).unwrap();
        if msg == "OK" {
            log::info!("Connect to NAT server successfully");
        } else {
            panic!("Received unexpected from server: {:?}", msg)
        }
    }
}
