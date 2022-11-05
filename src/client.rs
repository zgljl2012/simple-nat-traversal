//! Simple websocket client.
use std::{time::Duration, thread};

use nat::NatClient;

pub struct ClientConfig {
    pub server_url: String,
}

pub async fn start_client(config: &ClientConfig) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("starting NAT client: {}", config.server_url);
    let mut client = NatClient::new(config.server_url.as_str()).await.unwrap();
	loop {
		match client.run_forever().await {
			Ok(_) => {},
			Err(err) => {
				log::error!("{}", err);
				log::info!("Auto reconnect...");
				thread::sleep(Duration::from_millis(3000));
			}
		};
	}
}
