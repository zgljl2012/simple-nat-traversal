// NAT protocol

use core::panic;
use std::{
    sync::{Arc}, time::Duration,
};

use log::{info, error, debug};
use tokio::{sync::{RwLock, mpsc::{self, UnboundedSender, UnboundedReceiver}}, net::{TcpStream}, select, task, time};

use crate::utils;

pub type NatStream = Arc<RwLock<TcpStream>>;

// Ping message
const PING: [u8; 1] = [0x0];
// Pong message
const PONG: [u8; 1] = [0x1];

pub struct NatServer {
    stream: Option<NatStream>,
}

// Write standard message to stream
async fn write(stream: &TcpStream, buf: &[u8]) {
    let size: u32 = buf.len() as u32;
    let size_arr = utils::u32_to_be(size);
    let r: Vec<u8> = [&size_arr, buf].concat();
    stream.writable().await.unwrap();
    stream.try_write(&r.as_slice()).unwrap();
}

impl NatServer {
    pub fn new() -> NatServer {
        Self { stream: None }
    }

    pub fn init(&mut self, stream: NatStream) {
        // Only can init once
        if self.stream.is_some() {
            return;
        }
        self.stream = Some(stream);
    }

    pub fn is_inited(&self) -> bool {
        return self.stream.is_some();
    }

    async fn send_text(&mut self, msg: &str) -> std::io::Result<()> {
        if self.stream.is_none() {
            return Ok(());
        }
        let binding = self.stream.as_ref().unwrap();
        let stream = binding.write().await;
        stream.writable().await?;
        match stream.try_write(msg.as_bytes()) {
            Ok(n) => {
                info!("Send text {:?}({:?} bytes) successfully", msg, n);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
            }
            Err(e) => {
                return Err(e.into());
            }
        };
        Ok(())
    }

    pub async fn ok(&mut self) -> std::io::Result<()> {
        info!("Reply OK to client");
        self.send_text("OK").await
    }

    pub async fn run_forever(&mut self) {
        // Wait for NAT server
        let stream = self.stream.as_ref().unwrap().write().await;
        loop {
            // 读取server发过来的内容
            // 前四个字节表示消息的字节数，无符号 u32，即最多支持 2^32 次方的消息长度，即 4G
            select! {
                _ = stream.readable() => {
                    let mut buffer = Vec::with_capacity(4);
                    match stream.try_read_buf(&mut buffer) {
                        Ok(0) => {
                            error!("Server closed connection");
                            break;
                        },
                        Ok(_) => {
                            let data_size = utils::as_u32_be(buffer.as_slice());
                            debug!("You received {:?} bytes from NAT client", data_size);
                            // Read other bytes
                            let mut data = Vec::with_capacity(data_size as usize);
                            stream.try_read_buf(&mut data).unwrap();
                            if data.as_slice() == PING {
                                info!("You received PING from NAT client");
                                write(&stream, &PONG).await;
                            }
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            continue;
                        }
                        Err(e) => {
                            error!("Unexpected error: {}", e);
                        }
                    };
                }
            }
        }
    }

}

// Client protocol
pub struct NatClient {
    stream: NatStream,
    sender: Arc<RwLock<UnboundedSender<Vec<u8>>>>,
    receiver: UnboundedReceiver<Vec<u8>>,
}

impl NatClient {
    pub async fn new(server_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let (sender, receiver) = mpsc::unbounded_channel::<Vec<u8>>();
        let stream = TcpStream::connect(&server_url).await?;
        Ok(Self {
            sender: Arc::new(RwLock::new(sender)),
            receiver,
            stream: Arc::new(RwLock::new(stream)),
        })
    }

    pub async fn connect(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let stream = self.stream.write().await;
        stream.writable().await?;
        match stream.try_write("NAT 0.1".as_bytes()) {
            Ok(_) => {},
            Err(err) => {
                panic!("Write first line to server failed: {:?}", err)
            },
        };
        let mut buffer = Vec::with_capacity(1024);
        // 读取server发过来的内容
        stream.readable().await?;
        match stream.try_read_buf(&mut buffer) {
            Ok(_) => {},
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // pass
            }
            Err(err) => {
                panic!("Read from server failed: {:?}", err)
            },
        };
        let msg = std::str::from_utf8(&buffer).unwrap().trim_matches('\u{0}');
        if msg == "OK" {
            log::info!("Connect to NAT server successfully");
        } else {
            panic!("Received unexpected from server: {:?}", msg)
        }
        // Create a interval to ping the server
        let sender = self.sender.clone();
        task::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(5));
            loop {
                sender.write().await.send(PING.to_vec()).unwrap();
                interval.tick().await;
            }
        });
        Ok(())
    }

    pub async fn run_forever(&mut self) {
        // Wait for NAT server
        let stream = self.stream.write().await;
        loop {
            // 读取server发过来的内容
            // 前四个字节表示消息的字节数，无符号 u32，即最多支持 2^32 次方的消息长度，即 4G
            select! {
                _ = stream.readable() => {
                    let mut buffer = Vec::with_capacity(4);
                    match stream.try_read_buf(&mut buffer) {
                        Ok(0) => {
                            error!("Server closed connection");
                            break;
                        },
                        Ok(_) => {
                            let data_size = utils::as_u32_be(buffer.as_slice());
                            debug!("You received {:?} bytes from NAT server", data_size);
                            // Read other bytes
                            let mut data = Vec::with_capacity(data_size as usize);
                            stream.try_read_buf(&mut data).unwrap();
                            if data.as_slice() == PONG {
                                info!("You received PONG from NAT server");
                                write(&stream, &PONG).await;
                            }
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            continue;
                        }
                        Err(e) => {
                            error!("Unexpected error: {}", e);
                        }
                    };
                },
                msg = self.receiver.recv() => match msg {
                    Some(data) => {
                        if data.as_slice() == PING {
                            write(&stream, data.as_slice()).await;
                        }
                    },
                    None => {},
                }
            }
        }
    }
}
