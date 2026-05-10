//! Cross-process probe for the LAN pairing worker.
//!
//! Designed for two-container e2e testing — each instance generates a fresh
//! keypair, builds an invite that lists itself as admin, and spawns a worker
//! that broadcasts + listens. Every received `LanPairingSignal` is emitted as
//! a single JSON line on stdout so the test runner can grep for the peer.
//!
//! Args (positional, all optional):
//!   1. node-name (default: "probe")
//!   2. duration-secs (default: 12)
//!
//! Stdout schema (one per line):
//!   {"event":"ready","npub":"npub1...","node":"alice"}
//!   {"event":"peer","npub":"npub1...","node":"bob","networkId":"...","networkName":"..."}
//!   {"event":"done","peers":N}

use std::io::Write;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use nostr_sdk::prelude::ToBech32;
use nostr_vpn_app_core::lan_pairing::{
    LanPairingAnnouncement, LanPairingSignal, spawn_lan_pairing_worker,
};
use serde_json::json;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let node_name = args.next().unwrap_or_else(|| "probe".to_string());
    let duration_secs: u64 = args
        .next()
        .as_deref()
        .map(str::parse)
        .transpose()
        .context("duration-secs must be a u64")?
        .unwrap_or(12);

    let keys = nostr_sdk::Keys::generate();
    let npub = keys.public_key().to_bech32().expect("npub");
    let network_id = format!("probe-{}", &npub[5..13]);
    let network_name = format!("{node_name}'s probe net");
    let invite = encode_probe_invite(&npub, &network_name, &network_id);

    let mut stdout = std::io::stdout().lock();
    emit_line(
        &mut stdout,
        &json!({"event": "ready", "npub": npub, "node": node_name}),
    );

    let mut worker = spawn_lan_pairing_worker(LanPairingAnnouncement {
        npub: npub.clone(),
        node_name: node_name.clone(),
        endpoint: "127.0.0.1:51820".to_string(),
        invite,
    })
    .context("spawn worker")?;
    let expires_at = SystemTime::now() + Duration::from_secs(duration_secs);
    worker.set_broadcast_until(expires_at);
    worker.set_listen_until(expires_at);

    let deadline = Instant::now() + Duration::from_secs(duration_secs);
    let mut peer_count = 0_u64;
    let mut seen = std::collections::HashSet::new();
    while Instant::now() < deadline {
        for signal in worker.drain() {
            if seen.insert(format!("{}:{}", signal.network_id, signal.npub)) {
                peer_count += 1;
                emit_line(&mut stdout, &peer_event(&signal));
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }

    worker.stop();
    emit_line(&mut stdout, &json!({"event": "done", "peers": peer_count}));
    if peer_count == 0 {
        std::process::exit(2);
    }
    Ok(())
}

fn encode_probe_invite(admin_npub: &str, network_name: &str, network_id: &str) -> String {
    let payload = json!({
        "v": 3,
        "networkName": network_name,
        "networkId": network_id,
        "admins": [admin_npub],
        "relays": ["wss://relay.example"],
    })
    .to_string();
    format!("nvpn://invite/{}", URL_SAFE_NO_PAD.encode(payload))
}

fn peer_event(signal: &LanPairingSignal) -> serde_json::Value {
    json!({
        "event": "peer",
        "npub": signal.npub,
        "node": signal.node_name,
        "networkId": signal.network_id,
        "networkName": signal.network_name,
    })
}

fn emit_line<W: Write>(stdout: &mut W, value: &serde_json::Value) {
    let _ = writeln!(stdout, "{value}");
    let _ = stdout.flush();
}
