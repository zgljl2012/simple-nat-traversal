// use std::{net::{TcpListener, TcpStream, Shutdown}, io::{Read, Write}, sync::{Arc, RwLock}, thread};

use std::sync::Arc;

use log::{info, error, debug};
use tokio::{net::{TcpListener, TcpStream}, sync::{RwLock, mpsc::{self, UnboundedSender, UnboundedReceiver}}, io::{AsyncReadExt, AsyncWriteExt}, select};

use crate::{protocols::{parse_protocol}, nat::{NatServer}, message::Message};

pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct Context {
    inited: Arc<RwLock<bool>>
}

async fn handle_client(ctx: Context, nat_server: Arc<RwLock<NatServer>>, sender: Arc<RwLock<UnboundedSender<Message>>>, receiver: Arc<RwLock<UnboundedReceiver<Message>>>, mut stream: TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    // Parse the first line
    const BATCH_SIZE: usize = 64;
    let mut buffer = [0;BATCH_SIZE];
    // 读取server发过来的内容
    let nsize = stream.read(&mut buffer).await.expect("failed to read data from socket");
    let first_line = std::str::from_utf8(&buffer).unwrap().trim_matches('\u{0}').to_string();

    // Parse protocol
    let protocol = match parse_protocol(first_line) {
        Ok(protocol) => protocol,
        Err(err) => {
            error!("Failed to parse protocol: {}", err);
            return Ok(())
        }
    };
    info!("You connected to this server with protocol: {}", protocol.name());
    if protocol.name() == "NAT" {
        tokio::spawn(async move {
            if *ctx.inited.read().await == true {
                error!("Only support only one NAT client at a time");
                stream.try_write("Reject".as_bytes()).unwrap();
                // shutdown the connect with anther client
                let _ = stream.shutdown().await;
                return;
            }
            // 握手
            nat_server.write().await.init(Arc::new(RwLock::new(stream)));
            match nat_server.write().await.ok().await {
                Ok(_) => {},
                Err(err) => {
                    error!("Reply OK to client failed: {:?}", err);
                },
            };
            // Init
            *ctx.inited.write().await = true;
            // 通信
            nat_server.write().await.run_forever().await;
            *ctx.inited.write().await = false;
        });
    } else if protocol.name() == "HTTP" {
        if *ctx.inited.read().await == false {
            error!("There is no nat client connected");
            return Ok(());
        }
        tokio::spawn(async move {
            // 获取所有请求报文
            let mut bytes:Vec<u8> = Vec::new();
            bytes.append(&mut buffer[0..nsize].to_vec());
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
            debug!("Http request size: {:?}", bytes.len());
            let text = std::str::from_utf8(bytes.as_slice()).unwrap().trim_matches('\u{0}').to_string();
            debug!("{}", text);
            // 将请求转发给客户端
            sender.write().await.send(Message::new_http(bytes)).unwrap();
            // 获取所有的请求二进制
            let mut recv = receiver.write().await;
            select! {
                msg = recv.recv() => match msg {
                    Some(msg) => {
                        match stream.write(&msg.body).await {
                            Ok(n) => {
                                let s = std::str::from_utf8(&msg.body).unwrap();
                                debug!("Http response successfully: {:?} bytes, body {:?}", n, s);
                            },
                            Err(e) => {
                                error!("Write error: {}", e);
                            }
                        }
                    },
                    None => {
                        error!("Receive empty message");
                        match stream.write(b"HTTP/1.1 500 Internal Server Error\r\n").await {
                            Ok(_) => {},
                            Err(e) => {
                                error!("Write error: {}", e);
                            }
                        }
                    },
                }
            }
            stream.shutdown().await.unwrap();
            info!("Http stream shutdown completed");
        });
    }
    Ok(())
}

pub async fn start_server(config: &ServerConfig) -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    log::info!(
        "starting server at tcp://{}:{:?}",
        config.host,
        config.port
    );

    let listener = TcpListener::bind(format!("{}:{:?}", config.host, config.port)).await?;
    let (sender, external_receiver) = mpsc::unbounded_channel::<Message>();
    let (external_sender, receiver) = mpsc::unbounded_channel::<Message>();
    let nat_server: Arc<RwLock<NatServer>> = Arc::new(RwLock::new(NatServer::new(Arc::new(RwLock::new(external_sender)), external_receiver)));
    let sender = Arc::new(RwLock::new(sender));
    let receiver = Arc::new(RwLock::new(receiver));
    let ctx = Context {
        inited: Arc::new(RwLock::new(false)),
    };
    // accept connections and process them serially
    loop {
        let ns = nat_server.clone();
        let (socket, _) = listener.accept().await?;
        let _ = handle_client(ctx.clone(), ns, sender.clone(), receiver.clone(), socket).await;
    }
}
