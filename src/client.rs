//! Simple websocket client.
use std::{time::Duration, thread};

use log::info;
use nat::{NatClient, Context};

pub struct ClientConfig {
    pub server_url: String,
	pub password: String,
	pub ssh_mtu: u16
}

pub async fn start_client(config: &ClientConfig) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("starting NAT client: {}", config.server_url);
	let ctx = Context::new(config.password.clone(), config.ssh_mtu)?;
	info!("{:x?}", ctx.get_secret());
    let mut client = NatClient::new(config.server_url.as_str(), ctx).await.unwrap();
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
