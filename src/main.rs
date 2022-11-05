//! Simple echo websocket server.
//!
//! Open `http://localhost:8080/` in browser to test.

use clap::{Command, arg};
use log::error;
use server::{start_server, ServerConfig};
use client::{start_client, ClientConfig};

mod client;
mod server;

fn cli() -> Command {
    let port_arg = arg!(-p - -port <PORT> "Specify a port to listen or connect to").value_parser(clap::value_parser!(u16).range(3000..)).required(false);
    let host_arg = arg!(-H - -host <HOST> "Specify a host to listen or connect to").required(false);
	Command::new("Simple NAT Traversal")
        .about("NAT tool")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("server")
               .about("Start server")
               .arg(&port_arg)
               .arg(&host_arg)
        )
        .subcommand(
            Command::new("client")
               .about("Stop client")
               .arg(arg!(--"server-url" <SERVER> "specify server url").required(false))
               .arg(&host_arg)
        )
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    let matches = cli().get_matches();
    match matches.subcommand() {
        Some(("server", sub_matches)) => {
            let port = sub_matches.get_one::<u16>("port");
            let p: u16 = match port {
                Some(port) => port.clone(),
                None => 8080,
            };
            let host = sub_matches.get_one::<String>("host");
            let h = match host {
                Some(host) => host.clone(),
                None => "127.0.0.1".to_string(),
            };
            start_server(&ServerConfig {
                port: p,
                host: h,
            }).await
        },
        Some(("client", sub_matches)) => {
            let server_url = match sub_matches.get_one::<String>("server-url") {
                Some(s) => s.clone(),
                None => "ws://127.0.0.1:8080/ws".to_string(),
            };
            let _ = start_client(&ClientConfig {
                server_url
            }).await;
            Ok(())
        },
        _ => {
            error!("not implemented");
            Ok(())
        },
    }
}
