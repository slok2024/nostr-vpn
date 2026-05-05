use std::net::Ipv4Addr;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use clap::{Args, Parser, Subcommand};
use fips_endpoint::{
    Config, FipsEndpoint, PeerConfig as FipsPeerConfig, TransportInstances, UdpConfig,
};
use nostr_vpn_core::config::normalize_nostr_pubkey;
use nostr_vpn_core::fips_mesh::{FipsMeshPeerConfig, FipsMeshRuntime};

#[derive(Debug, Parser)]
#[command(name = "nvpn-fips-probe", about = "FIPS endpoint probe for Docker e2e")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve(ProbeArgs),
    Send(ProbeArgs),
}

#[derive(Debug, Args)]
struct ProbeArgs {
    #[arg(long)]
    identity_nsec: String,
    #[arg(long)]
    bind_addr: String,
    #[arg(long)]
    peer_npub: String,
    #[arg(long)]
    peer_addr: String,
    #[arg(long)]
    local_ip: Ipv4Addr,
    #[arg(long)]
    peer_ip: Ipv4Addr,
    #[arg(long, default_value_t = 10)]
    timeout_secs: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    match Cli::parse().command {
        Command::Serve(args) => serve(args).await,
        Command::Send(args) => send(args).await,
    }
}

async fn serve(args: ProbeArgs) -> Result<()> {
    let (endpoint, mesh, expected_peer_pubkey) = start_endpoint(&args).await?;
    println!("serve: endpoint {}", endpoint.npub());

    let packet = receive_packet(
        &endpoint,
        &mesh,
        &expected_peer_pubkey,
        args.peer_ip,
        args.local_ip,
        Duration::from_secs(args.timeout_secs),
    )
    .await
    .context("serve did not receive valid probe packet")?;
    println!("serve: received packet bytes={}", packet.len());

    let reply = ipv4_packet(args.local_ip, args.peer_ip);
    send_packet_with_retry(
        &endpoint,
        &mesh,
        &reply,
        Duration::from_secs(args.timeout_secs),
    )
    .await
    .context("serve failed to send reply packet")?;
    println!("serve: sent reply bytes={}", reply.len());

    endpoint.shutdown().await.context("shutdown failed")?;
    println!("probe serve passed");
    Ok(())
}

async fn send(args: ProbeArgs) -> Result<()> {
    let (endpoint, mesh, expected_peer_pubkey) = start_endpoint(&args).await?;
    println!("send: endpoint {}", endpoint.npub());

    let packet = ipv4_packet(args.local_ip, args.peer_ip);
    send_packet_with_retry(
        &endpoint,
        &mesh,
        &packet,
        Duration::from_secs(args.timeout_secs),
    )
    .await
    .context("send failed to send probe packet")?;
    println!("send: sent packet bytes={}", packet.len());

    let reply = receive_packet(
        &endpoint,
        &mesh,
        &expected_peer_pubkey,
        args.peer_ip,
        args.local_ip,
        Duration::from_secs(args.timeout_secs),
    )
    .await
    .context("send did not receive valid reply packet")?;
    println!("send: received reply bytes={}", reply.len());

    endpoint.shutdown().await.context("shutdown failed")?;
    println!("probe send passed");
    Ok(())
}

async fn start_endpoint(args: &ProbeArgs) -> Result<(FipsEndpoint, FipsMeshRuntime, String)> {
    let peer_pubkey = normalize_nostr_pubkey(&args.peer_npub).context("invalid peer npub")?;
    let peer = FipsMeshPeerConfig::from_participant_pubkey(
        &args.peer_npub,
        vec![format!("{}/32", args.peer_ip)],
    )
    .context("failed to build FIPS mesh peer")?;
    let mesh = FipsMeshRuntime::new(vec![peer]);

    let mut config = Config::new();
    config.transports.udp = TransportInstances::Single(UdpConfig {
        bind_addr: Some(args.bind_addr.clone()),
        accept_connections: Some(true),
        ..UdpConfig::default()
    });
    config
        .peers
        .push(FipsPeerConfig::new(&args.peer_npub, "udp", &args.peer_addr));

    let endpoint = FipsEndpoint::builder()
        .config(config)
        .identity_nsec(args.identity_nsec.clone())
        .without_system_tun()
        .bind()
        .await
        .context("failed to bind FIPS endpoint")?;

    Ok((endpoint, mesh, peer_pubkey))
}

async fn send_packet_with_retry(
    endpoint: &FipsEndpoint,
    mesh: &FipsMeshRuntime,
    packet: &[u8],
    timeout: Duration,
) -> Result<()> {
    let outgoing = mesh
        .route_outbound_packet(packet)
        .ok_or_else(|| anyhow!("packet did not match any FIPS peer route"))?;
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        match endpoint
            .send(outgoing.endpoint_npub.clone(), outgoing.bytes.clone())
            .await
        {
            Ok(()) => return Ok(()),
            Err(error) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(anyhow!(error));
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn receive_packet(
    endpoint: &FipsEndpoint,
    mesh: &FipsMeshRuntime,
    expected_peer_pubkey: &str,
    expected_source: Ipv4Addr,
    expected_destination: Ipv4Addr,
    timeout: Duration,
) -> Result<Vec<u8>> {
    let receive = async {
        loop {
            let Some(message) = endpoint.recv().await else {
                return Err(anyhow!("endpoint closed"));
            };
            let Some(packet) =
                mesh.receive_endpoint_data(message.source_npub.as_deref(), &message.data)
            else {
                continue;
            };
            if packet.source_pubkey != expected_peer_pubkey {
                return Err(anyhow!(
                    "unexpected source pubkey {}, expected {}",
                    packet.source_pubkey,
                    expected_peer_pubkey
                ));
            }
            let (source, destination) =
                ipv4_source_destination(&packet.bytes).context("invalid IPv4 packet")?;
            if source != expected_source || destination != expected_destination {
                return Err(anyhow!(
                    "unexpected packet {} -> {}, expected {} -> {}",
                    source,
                    destination,
                    expected_source,
                    expected_destination
                ));
            }
            return Ok(packet.bytes);
        }
    };

    tokio::time::timeout(timeout, receive)
        .await
        .context("timed out waiting for packet")?
}

fn ipv4_packet(source: Ipv4Addr, destination: Ipv4Addr) -> Vec<u8> {
    let payload = b"nvpn-fips-probe";
    let total_len = 20 + payload.len();
    let mut packet = vec![0_u8; total_len];
    packet[0] = 0x45;
    packet[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
    packet[8] = 64;
    packet[9] = 17;
    packet[12..16].copy_from_slice(&source.octets());
    packet[16..20].copy_from_slice(&destination.octets());
    packet[20..].copy_from_slice(payload);
    packet
}

fn ipv4_source_destination(packet: &[u8]) -> Option<(Ipv4Addr, Ipv4Addr)> {
    if packet.len() < 20 || packet.first()? >> 4 != 4 {
        return None;
    }
    Some((
        Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]),
        Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]),
    ))
}
