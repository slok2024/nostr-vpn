# FIPS Private Mesh Integration Notes

This document tracks the current FIPS-only private mesh architecture. Older
drafts described a selectable private data plane and a legacy mesh fallback;
that model has been removed from `nostr-vpn`.

## Current Shape

`nostr-vpn` owns:

- device identity, invites, admin rosters, aliases, and route policy
- the visible VPN adapter on each platform
- packet admission from the active roster into the local tunnel

FIPS owns:

- private mesh packet transport
- peer reachability, path selection, NAT traversal, and rendezvous
- optional Nostr relay use inside its own discovery layer

The app gives FIPS an active roster-derived peer map. FIPS may discover paths
through its configured mechanisms, but discovery is not membership authority.

## Boundaries

- Only active-network roster members can deliver packets into the private
  tunnel.
- FIPS relays or rendezvous peers are connectivity aids, not nostr-vpn roster
  peers.
- Invites may carry relay URLs for the FIPS discovery layer. Importing an
  invite must not enable the removed nostr-vpn relay-signaling protocol.
- `nostr-vpn` must not publish or consume legacy peer announcements.
- Exit-node work belongs in a separate component later, not in the main private
  mesh runtime.

## Implementation Notes

- `crates/nostr-vpn-cli/src/fips_private_mesh.rs` embeds the FIPS endpoint and
  maps active roster participants into FIPS peer config.
- `crates/nostr-vpn-core/src/fips_mesh.rs` contains private packet routing and
  peer admission helpers.
- `crates/nostr-vpn-core/src/join_requests.rs` keeps admin-signed roster state
  without relay presence helpers.
- Native shells consume FIPS-specific runtime state through
  `crates/nostr-vpn-app-core`.

## Verification

The cleanup is covered by:

- Rust workspace tests and clippy
- Docker private-mesh e2e scripts under `scripts/e2e-*-docker.sh`
- native smoke tests for Android, iOS, macOS, Linux, and Windows

When this document and the code diverge, the code wins.
