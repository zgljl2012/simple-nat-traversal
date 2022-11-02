// NAT protocol

use core::panic;
use std::{sync::Arc, time::Duration};

use log::{debug, error, info};
use reqwest::ClientBuilder;
use tokio::{
    net::TcpStream,
    select,
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        RwLock,
    },
    task, time,
};

use crate::{http::HttpRequest, message::{Message}};

pub type NatStream = Arc<RwLock<TcpStream>>;

pub struct NatServer {
    stream: Option<NatStream>,
    sender: Arc<RwLock<UnboundedSender<Message>>>,
    receiver: UnboundedReceiver<Message>,
    exteral_sender: Arc<RwLock<UnboundedSender<Message>>>,
    exteral_receiver: UnboundedReceiver<Message>,
}

impl NatServer {
    pub fn new(
        exteral_sender: Arc<RwLock<UnboundedSender<Message>>>,
        exteral_receiver: UnboundedReceiver<Message>,
    ) -> NatServer {
        let (sender, receiver) = mpsc::unbounded_channel::<Message>();
        Self {
            stream: None,
            receiver: receiver,
            sender: Arc::new(RwLock::new(sender)),
            exteral_receiver,
            exteral_sender,
        }
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
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
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
        let exteral_receiver = &mut self.exteral_receiver;
        let exteral_sender = &mut self.exteral_sender;
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
                                if msg.is_ping() {
                                    debug!("You received PING from NAT client");
                                    sender.write().await.send(Message::pong()).unwrap();
                                }
                                if msg.is_http() {
                                    info!("Received HTTP Response from client");
                                    exteral_sender.write().await.send(msg).unwrap();
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
                        msg.write_to(&stream).await;
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
            Ok(_) => {}
            Err(err) => {
                panic!("Write first line to server failed: {:?}", err)
            }
        };
        let mut buffer = Vec::with_capacity(1024);
        // 读取server发过来的内容
        stream.readable().await?;
        match stream.try_read_buf(&mut buffer) {
            Ok(_) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // pass
            }
            Err(err) => {
                panic!("Read from server failed: {:?}", err)
            }
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
                sender
                    .write()
                    .await
                    .send(Message::ping())
                    .unwrap();
                interval.tick().await;
            }
        });
        Ok(())
    }

    async fn handle_http(&self, msg: &Message) -> Result<String, Box<dyn std::error::Error>> {
        // 取出 Host
        let text = std::str::from_utf8(msg.body.as_slice())
            .unwrap()
            .trim_matches('\u{0}')
            .to_string();
        debug!("{}", text);
        let req = HttpRequest::from_utf8(text.as_str());

        info!("Redirect request to {:?}", req.request_line.url);

        // Create client with timeout
        let client = ClientBuilder::new().timeout(Duration::from_secs(2)).build()?;
        // 转发给指定的 Host
        let body = client.get(req.request_line.url).send().await?;
        
        let mut res_text = String::new();
        res_text += &format!("{:?} {:?}\r\n", body.version(), body.status().as_u16());
        let specify = "FROM CPChain----\r\n";
        for (key, value) in body.headers() {
            let mut v = value.to_str().unwrap().to_string();
            if key.to_string() == "content-length" {
                let mut size = v.parse::<usize>().unwrap();
                size += specify.len();
                v = format!("{:?}", size)
            }
            res_text += &format!(
                "{}: {}\r\n",
                key.to_string(),
                v
            )
        }
        let text = body.text().await?;
        res_text += &format!("\r\n{}{} \r\n", specify, text);
        debug!("{}", res_text);
        Ok(res_text)
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
                                if msg.is_pong() {
                                    debug!("You received PONG from NAT server");
                                }
                                if msg.is_http() {
                                    match self.handle_http(&msg).await {
                                        Ok(res) => {
                                            // Send to server
                                            Message::new_http(res.as_bytes().to_vec()).write_to(&stream).await;
                                        },
                                        Err(e) => {
                                            error!("Redirect http request failed: {:?}", e);
                                            if e.source().is_some() {
                                                if "operation timed out" == format!("{}", e.source().unwrap()) {
                                                    Message::http_504().write_to(&stream).await;
                                                } else {
                                                    Message::http_502().write_to(&stream).await;
                                                }
                                            } else {
                                                Message::http_502().write_to(&stream).await;
                                            }
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
                        msg.write_to(&stream).await;
                    },
                    None => {},
                }
            }
        }
    }
}
