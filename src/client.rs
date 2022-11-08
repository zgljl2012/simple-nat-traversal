//! Simple websocket client.
use std::{time::Duration, thread};

use nat::{NatClient, Context};

use crate::common::BaseConfig;

pub struct ClientConfig {
    pub server_url: String,
	pub base: BaseConfig,
}

pub async fn start_client(config: &ClientConfig) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("starting NAT client: {}", config.server_url);
	let ctx = Context::new(config.base.password.clone(),
		config.base.ssh_mtu, config.base.http_mtu, config.base.subnet.clone())?;
	let mut client = NatClient::new(config.server_url.as_str(), ctx).await.unwrap();
	loop {
		match client.run_forever().await {
			Ok(_) => {},
			Err(err) => {
				log::error!("{}", err);
				if format!("{}", err).contains("password incorrect") {
					return Err(err);
				}
				log::info!("Auto reconnect...");
				thread::sleep(Duration::from_millis(3000));
			}
		};
	}
}
