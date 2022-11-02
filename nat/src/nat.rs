// NAT protocol

use std::{sync::Arc};

use tokio::{
    net::TcpStream, sync::RwLock,
};

pub type NatStream = Arc<RwLock<TcpStream>>;
