// NAT protocol

use core::{panic};
use std::{
    sync::{Arc}, time::Duration,
};

use log::{info, error, debug};
use tokio::{sync::{RwLock, mpsc::{self, UnboundedSender, UnboundedReceiver}}, net::{TcpStream}, select, task, time};

use crate::{utils, protocols::ProtocolType, http::HttpRequest};

pub type NatStream = Arc<RwLock<TcpStream>>;

// Ping message
const PING: [u8; 1] = [0x0];
// Pong message
const PONG: [u8; 1] = [0x1];

#[derive(Debug, Clone)]
pub struct Message {
    pub protocol: ProtocolType,
    pub body: Vec<u8>
}

impl Message {
    async fn from_stream(stream: &TcpStream) -> Result<Option<Message>, Box<dyn std::error::Error + Send + Sync>> {
        let mut buffer = Vec::with_capacity(4);
        match stream.try_read_buf(&mut buffer) {
            Ok(0) => {
                Err("Connection closed".into())
            },
            Ok(_) => {
                let data_size = utils::as_u32_be(buffer.as_slice());
                debug!("You received {:?} bytes from NAT client", data_size);
                // read protocol type
                let mut protocol_type_buf = Vec::with_capacity(1);
                stream.try_read_buf(&mut protocol_type_buf).unwrap();
                let protocol_type = match ProtocolType::from_slice(protocol_type_buf.as_slice()) {
                    Some(pt) => pt,
                    None => {
                        return Err(format!("Uncognizaed protocol type: {:?}", utils::as_u32_be(protocol_type_buf.as_slice())).into());
                    }
                };

                // Read other bytes
                let mut data = Vec::with_capacity(data_size as usize);
                stream.try_read_buf(&mut data).unwrap();
                Ok(Some(Message { protocol: protocol_type, body: data }))
            },
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                Ok(None)
            },
            Err(e) => {
                Err(format!("Unexpected error: {}", e).into())
            }
        }
    }
}

pub struct NatServer {
    stream: Option<NatStream>,
    sender: Arc<RwLock<UnboundedSender<Message>>>,
    receiver: UnboundedReceiver<Message>,
    extern_receiver: UnboundedReceiver<Message>,
}

// Write standard message to stream
async fn write(stream: &TcpStream, msg: &Message) {
    let size: u32 = msg.body.len() as u32;
    let size_arr = utils::u32_to_be(size);
    let r: Vec<u8> = [&size_arr, msg.protocol.bytes().to_vec().as_slice(),
        msg.body.as_slice()].concat();
    stream.writable().await.unwrap();
    stream.try_write(&r.as_slice()).unwrap();
}

impl NatServer {
    pub fn new(extern_receiver: UnboundedReceiver<Message>) -> NatServer {
        let (sender, receiver) = mpsc::unbounded_channel::<Message>();
        Self { stream: None, receiver: receiver, sender: Arc::new(RwLock::new(sender)), extern_receiver}
    }

    pub fn init(&mut self, stream: NatStream) {
        self.stream = Some(stream);
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
                debug!("Send text {:?}({:?} bytes) successfully", msg, n);
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
        let sender = &mut self.sender;
        let receiver = &mut self.receiver;
        let exteral_receiver = &mut self.extern_receiver;
        loop {
            // 读取server发过来的内容
            // 前四个字节表示消息的字节数，无符号 u32，即最多支持 2^32 次方的消息长度，即 4G
            // 第五个字节表示协议类型：NAT(0x0), HTTP(0x1), SSH(0x2)
            select! {
                _ = stream.readable() => {
                    match Message::from_stream(&stream).await {
                        Ok(msg) => match msg {
                            Some(msg) => {
                                debug!("Received {:?}", msg.protocol);
                                if msg.body.as_slice() == PING {
                                    debug!("You received PING from NAT client");
                                    sender.write().await.send(Message { protocol: ProtocolType::NAT, body: PONG.to_vec() }).unwrap();
                                }
                            },
                            None => {}
                        },
                        Err(err) => {
                            error!("{:?}", err);
                            break;
                        },
                    }
                },
                exteral_msg = exteral_receiver.recv() => match exteral_msg {
                    Some(msg) => {
                        sender.write().await.send(msg).unwrap();
                    },
                    None => todo!()
                },
                msg = receiver.recv() => match msg {
                    Some(msg) => {
                        write(&stream, &msg).await;
                    },
                    None => todo!(),
                }
            }
        }
    }

}

// Client protocol
pub struct NatClient {
    stream: NatStream,
    sender: Arc<RwLock<UnboundedSender<Message>>>,
    receiver: UnboundedReceiver<Message>,
}

impl NatClient {
    pub async fn new(server_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let (sender, receiver) = mpsc::unbounded_channel::<Message>();
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
                sender.write().await.send(Message{protocol: ProtocolType::NAT, body: PING.to_vec()}).unwrap();
                interval.tick().await;
            }
        });
        Ok(())
    }

    async fn handle_http(&self, msg: &Message) -> Result<(), Box<dyn std::error::Error>> {
        // 取出 Host
        let text = std::str::from_utf8(msg.body.as_slice()).unwrap().trim_matches('\u{0}').to_string();
        info!("{:?}", text);
        let req = HttpRequest::from_utf8(text.as_str());

        info!("Redirect request to {:?}", req.request_line.url);

        // 转发给指定的 Host
        let mut body = reqwest::get(req.request_line.url)
            .await?;
            // .bytes().await?;
        // info!("--> {:?}", std::str::from_utf8(a.to_vec().as_slice()).unwrap());
        Ok(())
    }

    pub async fn run_forever(&mut self) {
        // Wait for NAT server
        let stream = self.stream.write().await;
        loop {
            // 读取server发过来的内容
            // 前四个字节表示消息的字节数，无符号 u32，即最多支持 2^32 次方的消息长度，即 4G
            // 第五个字节表示协议类型: NAT(0x0), HTTP(0x1), SSH(0x2)
            select! {
                _ = stream.readable() => {
                    match Message::from_stream(&stream).await {
                        Ok(msg) => match msg {
                            Some(msg) => {
                                debug!("Received {:?}", msg.protocol);
                                if msg.body.as_slice() == PONG {
                                    debug!("You received PONG from NAT server");
                                }
                                if msg.protocol == ProtocolType::HTTP {
                                    match self.handle_http(&msg).await {
                                        Ok(_) => {},
                                        Err(e) => {
                                            error!("Redirect http request failed: {:?}", e);
                                        }
                                    }
                                }
                            },
                            None => {}
                        },
                        Err(err) => {
                            error!("{:?}", err);
                            break;
                        },
                    }
                },  
                msg = self.receiver.recv() => match msg {
                    Some(msg) => {
                        // Send to server
                        write(&stream, &msg).await;
                    },
                    None => {},
                }
            }
        }
    }
}
