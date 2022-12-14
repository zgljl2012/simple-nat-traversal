//! Simple echo websocket server.
//!
//! Open `http://localhost:8080/` in browser to test.

use clap::{Command, arg, ArgMatches};
use common::BaseConfig;
use log::error;
use server::{start_server, ServerConfig};
use client::{start_client, ClientConfig};

mod common;
mod client;
mod server;

fn cli() -> Command {
    let port_arg = arg!(-p - -port <PORT> "Specify a port to listen or connect to").value_parser(clap::value_parser!(u16).range(3000..)).required(false);
    let host_arg = arg!(-H - -host <HOST> "Specify a host to listen or connect to").required(false);
	let password = arg!(-P - -password <PASSWORD> "Password of server").required(true);
	let ssh_mtu = arg!(-M - -"ssh-mtu" <SSH_MTU> "MTU of the SSH packet per packet").required(false).value_parser(clap::value_parser!(u16).range(64..2048));
	let http_mtu = arg!(--"http-mtu" <HTTP_MTU> "MTU of the HTTP packet per packet").required(false).value_parser(clap::value_parser!(u16).range(64..2048));
	let subnet = arg!(--"subnet" <SUBNET> "Subnet").required(false);
	Command::new("Simple NAT Traversal")
        .about("NAT tool")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("server")
               .about("Start server")
               .arg(&port_arg)
               .arg(&host_arg)
			   .arg(&password)
			   .arg(&ssh_mtu)
			   .arg(&http_mtu)
			   .arg(&subnet)
        )
        .subcommand(
            Command::new("client")
               .about("Stop client")
               .arg(arg!(--"server-url" <SERVER> "specify server url").required(false))
               .arg(&host_arg)
			   .arg(&password)
			   .arg(&ssh_mtu)
			   .arg(&http_mtu)
			   .arg(&subnet)
        )
}

fn get_password(sub_matches: &ArgMatches) -> String {
	sub_matches.get_one::<String>("password").unwrap().clone()
}

fn get_ssh_mtu(sub_matches: &ArgMatches) -> u16 {
	sub_matches.get_one::<u16>("ssh-mtu").unwrap_or(&512).clone()
}

fn get_http_mtu(sub_matches: &ArgMatches) -> u16 {
	sub_matches.get_one::<u16>("http-mtu").unwrap_or(&1024).clone()
}

fn get_subnet(sub_matches: &ArgMatches) -> String {
	sub_matches.get_one::<String>("subnet").unwrap_or(&"127.0.0.1/32".to_string()).to_string()
}

fn get_base_config(sub_matches: &ArgMatches) -> BaseConfig {
	BaseConfig {
		password: get_password(&sub_matches),
		ssh_mtu: get_ssh_mtu(&sub_matches),
		http_mtu: get_http_mtu(&sub_matches),
		subnet: get_subnet(&sub_matches),
	}
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
            match start_server(&ServerConfig {
                port: p,
                host: h,
				base: get_base_config(sub_matches)
            }).await {
				Ok(()) => {},
				Err(err) => {
					panic!("Start server failed: {:?}", err);
				}
			};
			Ok(())
        },
        Some(("client", sub_matches)) => {
            let server_url = match sub_matches.get_one::<String>("server-url") {
                Some(s) => s.clone(),
                None => "ws://127.0.0.1:8080/ws".to_string(),
            };
            let _ = start_client(&ClientConfig {
                server_url,
				base: get_base_config(sub_matches)
            }).await;
            Ok(())
        },
        _ => {
            error!("not implemented");
            Ok(())
        },
    }
}
