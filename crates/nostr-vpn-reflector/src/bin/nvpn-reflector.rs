use std::net::SocketAddr;

use anyhow::{Context, Result};
use clap::Parser;
use nostr_vpn_core::nat::{
    DISCOVER_REQUEST_PREFIX, ENDPOINT_RESPONSE_PREFIX, PUNCH_ACK_PREFIX, PUNCH_REQUEST_PREFIX,
};
use tokio::net::UdpSocket;

#[derive(Debug, Parser)]
#[command(name = "nvpn-reflector")]
#[command(about = "Minimal UDP endpoint reflector for NAT discovery/hole-punch testing")]
struct Args {
    #[arg(long, default_value = "0.0.0.0:3478")]
    bind: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let bind: SocketAddr = args
        .bind
        .parse()
        .with_context(|| format!("invalid bind address {}", args.bind))?;

    let socket = UdpSocket::bind(bind)
        .await
        .with_context(|| format!("failed to bind {bind}"))?;
    println!("nvpn-reflector listening on udp://{bind}");

    let mut buf = [0u8; 2048];
    loop {
        let (read, src) = socket
            .recv_from(&mut buf)
            .await
            .context("udp recv failed")?;
        let payload = std::str::from_utf8(&buf[..read]).unwrap_or_default();

        if payload.starts_with(DISCOVER_REQUEST_PREFIX) {
            let response = format!("{ENDPOINT_RESPONSE_PREFIX} {src}");
            let _ = socket.send_to(response.as_bytes(), src).await;
            continue;
        }

        if payload.starts_with(PUNCH_REQUEST_PREFIX) {
            let _ = socket.send_to(PUNCH_ACK_PREFIX.as_bytes(), src).await;
        }
    }
}
