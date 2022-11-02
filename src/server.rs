use nat::NatServer;

pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}


pub async fn start_server(config: &ServerConfig) -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    log::info!(
        "starting server at tcp://{}:{:?}",
        config.host,
        config.port
    );

    // Start listening
	NatServer::new().run_forever(format!("{}:{:?}", config.host, config.port).as_str()).await
}
