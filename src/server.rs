use std::{net::{TcpListener, TcpStream}, io::{BufReader, BufRead}};

use log::{info, error};

use crate::protocols::parse_protocol;

pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

fn handle_client(stream: TcpStream) {
    let mut reader = BufReader::new(&stream);
    // Parse the first line
    let mut first_line = String::new();
    reader.read_line(&mut first_line).expect("RECEIVE FAILURE!!!");

    // Parse protocol
    let protocol = match parse_protocol(first_line) {
        Ok(protocol) => protocol,
        Err(err) => {
            error!("Failed to parse protocol: {}", err);
            return
        }
    };
    info!("You connected to this server with protocol: {}", protocol.name());
}

pub async fn start_server(config: &ServerConfig) -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    log::info!(
        "starting server at tcp://{}:{:?}",
        config.host,
        config.port
    );

    let listener = TcpListener::bind(format!("{}:{:?}", config.host, config.port))?;

    // accept connections and process them serially
    for stream in listener.incoming() {
        handle_client(stream?);
    }

    Ok(())
}
