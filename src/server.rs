use nat::{NatServer, Context};

use crate::common::{BaseConfig};

pub struct ServerConfig {
    pub host: String,
    pub port: u16,
	pub base: BaseConfig,
}

pub async fn start_server(config: &ServerConfig) -> Result<(), Box<dyn std::error::Error>> {
    // Start daemon
	let ctx = Context::new(config.base.password.clone(),
		config.base.ssh_mtu, config.base.http_mtu, config.base.subnet.clone())?;
	// Start listening
	NatServer::new(ctx).run_forever(format!("{}:{:?}", config.host, config.port).as_str()).await
}
