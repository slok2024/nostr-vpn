# Protocol

This document describes the protocol shape that `nostr-vpn` currently implements after the FIPS-only private mesh cleanup.

The `README.md` stays product-facing. Protocol details live here so they can track the code more closely.

## Scope

`nostr-vpn` is split into three layers:

- Out-of-band bootstrapping with invite payloads and QR codes
- Network membership and admin roster state
- A FIPS-backed private mesh data plane for tunnel traffic

Only the active network participates in the live runtime. `nostr-vpn` no longer runs its legacy relay-announced peer roster or WireGuard mesh mode.

## Identities And Stable IDs

| Name | Purpose | Format |
| --- | --- | --- |
| Nostr identity keypair | Authenticates device identity, invites, admin actions, and FIPS discovery identity | `nsec`/`npub` at the edges, normalized to hex internally |
| `network_id` | Stable mesh identifier used for roster scope and tunnel-IP derivation | String |

Important details:

- `network_id` is normalized at runtime by stripping the legacy `nostr-vpn:` prefix if present.
- Each configured network carries its own participant and admin set.
- If tunnel IP auto-configuration is enabled, the local node derives its `/32` as:
  - `SHA256(network_id + "\n" + own_nostr_pubkey_hex)`
  - `10.44.(digest[0] % 254 + 1).(digest[1] % 254 + 1)/32`

## Invites

Network invites are `nvpn://invite` payloads. They carry enough information for another device to join or request access:

- network name and stable `network_id`
- inviter identity
- optional admin and participant metadata
- optional relay list for the discovery layer

Relay URLs in invites configure the discovery/rendezvous layer used by FIPS. They are not a nostr-vpn relay roster, and importing an invite does not enable the removed legacy peer-announcement protocol.

## Admin Roster Sync

Network membership is represented as an admin-signed roster. Operationally, the roster is the authority for:

- network name
- participants
- admins
- join-request settings
- per-peer aliases and route settings

When a newer valid roster arrives, peers reload the active network membership and keep the FIPS mesh runtime aligned with that roster.

## FIPS Private Mesh

`nvpn` uses FIPS as the only private mesh data plane.

The CLI builds FIPS peer configuration from the active network participants:

- each participant is mapped to its derived tunnel address
- advertised routes are included in peer allowed IPs
- static FIPS peer endpoints may be supplied with `fips_peer_endpoints`
- FIPS discovery may use configured Nostr relays internally

The daemon reports each peer with FIPS-specific state:

- `endpoint` and `runtime_endpoint` are `"fips"` or a FIPS transport address
- `fips_endpoint_npub`, `fips_transport_addr`, and `fips_transport_type` describe the selected FIPS link
- `last_mesh_seen_at` and `last_fips_seen_at` replace the legacy presence/signal timestamps
- packet and byte counters are read from FIPS link state

`nostr-vpn` should not publish or consume its old Nostr relay peer announcements. If FIPS uses relays, that behavior belongs to FIPS discovery/rendezvous, not to a separate nostr-vpn signaling model.

## NAT Discovery

NAT discovery remains a local endpoint aid:

- reflectors and STUN servers can discover a public UDP endpoint
- discovery is bound to the active listen port when possible
- port-mapping state is surfaced in diagnostics

The discovered endpoint is input to FIPS configuration. It is not a WireGuard endpoint announcement.

## Routing

Route targets come from the active network roster and local node settings. FIPS receives the route map and carries private mesh traffic for the selected peers.

Exit-node behavior is represented in config and UI state. A node can optionally use a local WireGuard upstream while offering FIPS exit-node service, but peers still see only the normal FIPS exit-node route advertisement. WireGuard is not a mesh data plane and nostr-vpn does not announce or signal WireGuard peers through its old relay model.

## Canonical Source

If this document and the code diverge, the code wins. The main protocol implementations currently live in:

- `crates/nostr-vpn-core/src/fips_control.rs`
- `crates/nostr-vpn-core/src/fips_mesh.rs`
- `crates/nostr-vpn-core/src/join_requests.rs`
- `crates/nostr-vpn-core/src/config.rs`
- `crates/nostr-vpn-cli/src/fips_private_mesh.rs`
- `crates/nostr-vpn-cli/src/session_runtime.rs`
- `crates/nostr-vpn-app-core/src/ffi.rs`
