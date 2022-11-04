//! Simple websocket client.
use nat::NatClient;

pub struct ClientConfig {
    pub server_url: String,
}

pub async fn start_client(config: &ClientConfig) -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    log::info!("starting NAT client: {}", config.server_url);
    let mut client = NatClient::new(config.server_url.as_str()).await.unwrap();
    client.run_forever().await?;
    Ok(())
}
