// use std::{net::{TcpListener, TcpStream, Shutdown}, io::{Read, Write}, sync::{Arc, RwLock}, thread};

use std::sync::Arc;

use log::{info, error, debug};
use nat::{parse_protocol, NatServer, Message};
use tokio::{net::{TcpListener, TcpStream}, sync::{RwLock, mpsc::{self, UnboundedSender, UnboundedReceiver}}, io::{AsyncReadExt, AsyncWriteExt}, select};
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct Context {
    inited: Arc<RwLock<bool>>
}

pub async fn start_server(config: &ServerConfig) -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    log::info!(
        "starting server at tcp://{}:{:?}",
        config.host,
        config.port
    );

    // let listener = TcpListener::bind(format!("{}:{:?}", config.host, config.port)).await?;
    let (sender, external_receiver) = mpsc::unbounded_channel::<Message>();
    let (external_sender, receiver) = mpsc::unbounded_channel::<Message>();
    let mut nat_server: NatServer = NatServer::new(Arc::new(RwLock::new(external_sender)), external_receiver);
    // let sender = Arc::new(RwLock::new(sender));
    // let receiver = Arc::new(RwLock::new(receiver));
    // let ctx = Context {
    //     inited: Arc::new(RwLock::new(false)),
    // };
	// Start listening
	nat_server.run_forever(format!("{}:{:?}", config.host, config.port).as_str()).await?;
    // accept connections and process them serially
    // loop {
    //     let ns = nat_server.clone();
    //     let (socket, _) = listener.accept().await?;
    //     let _ = handle_client(ctx.clone(), ns, sender.clone(), receiver.clone(), socket).await;
    // }
	Ok(())
}
