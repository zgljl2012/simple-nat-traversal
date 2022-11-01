// use std::{net::{TcpListener, TcpStream, Shutdown}, io::{Read, Write}, sync::{Arc, RwLock}, thread};

use std::sync::Arc;

use log::{info, error};
use tokio::{net::{TcpListener, TcpStream}, sync::RwLock, io::{AsyncReadExt, AsyncWriteExt}};

use crate::{protocols::parse_protocol, nat::NatServer};

pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

async fn handle_client(nat_server: Arc<RwLock<NatServer>>, mut stream: TcpStream) -> std::io::Result<()> {
    // Parse the first line
    let mut buffer = [0;1024];
    // 读取server发过来的内容
    let _ = stream.read(&mut buffer).await.expect("failed to read data from socket");
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
            if nat_server.read().await.is_inited() {
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
            // 通信
            nat_server.write().await.run_forever().await;
        });
    } else if protocol.name() == "HTTP" {
        if !nat_server.read().await.is_inited() {
            error!("There is no nat client connected");
            return Ok(());
        }
        // 将请求转发给客户端
        // 获取所有的请求二进制
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
    let nat_server: Arc<RwLock<NatServer>> = Arc::new(RwLock::new(NatServer::new()));

    // accept connections and process them serially
    loop {
        let ns = nat_server.clone();
        let (socket, _) = listener.accept().await?;
        let _ = handle_client(ns, socket).await;
    }
}
