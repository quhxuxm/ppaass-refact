use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use anyhow::Result;

use crate::server::AgentServer;

pub(crate) mod command {
    pub(crate) mod socks5;
}

pub(crate) mod server;

pub(crate) mod codec {
    pub(crate) mod socks5;
}

pub(crate) mod service;

fn main() -> Result<()> {
    let agent_server = AgentServer::new();
    agent_server.run();
    Ok(())
}
