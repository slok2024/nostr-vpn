//! Re-export of the canonical LAN-pairing worker from `nostr-vpn-core`. The
//! implementation moved when the CLI grew its own `invite-broadcast`/`discover`
//! subcommands so headless devices can participate without going through the
//! native-app FFI.
pub use nostr_vpn_core::lan_pairing::*;
