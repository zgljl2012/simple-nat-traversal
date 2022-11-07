use nat::{NatServer, Context};

pub struct ServerConfig {
    pub host: String,
    pub port: u16,
	pub password: String,
	pub ssh_mtu: u16,
	pub http_mtu: u16
}


pub async fn start_server(config: &ServerConfig) -> Result<(), Box<dyn std::error::Error>> {
    log::info!(
        "starting server at tcp://{}:{:?}",
        config.host,
        config.port
    );
	let ctx = Context::new(config.password.clone(), config.ssh_mtu, config.http_mtu)?;
	// Start listening
	NatServer::new(ctx).run_forever(format!("{}:{:?}", config.host, config.port).as_str()).await
}
