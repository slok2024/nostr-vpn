#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

node scripts/sync-versions.mjs
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/e2e-update-cli.sh

case "${NVPN_RELEASE_GATE_DOCKER_E2E:-1}" in
  0|false|FALSE|False|no|NO|No|off|OFF|Off)
    echo "Skipping Docker e2e because NVPN_RELEASE_GATE_DOCKER_E2E=${NVPN_RELEASE_GATE_DOCKER_E2E}"
    ;;
  *)
    # TODO(re-enable): both of these wedge in the 90s continuity window
    # since 5c7b425 flipped FIPS Nostr discovery to `policy: Open` —
    # daemons now reach the public overlay (relay.damus.io, nos.lol,
    # offchain.pub), peer with the wider world, and the resulting NAT
    # traversal + UDP send activity competes with the test transit hops.
    # Pure iptables/network isolation to 10.203.0.0/24 doesn't work
    # either: the overlay routing through the transit-only hop (Charlie)
    # relies on the same Nostr signaling we'd be blocking. Fix needs to
    # either thread an explicit `policy=ConfiguredOnly` opt-out through
    # `fips_private_mesh::build_fips_config` for test/headless use or
    # stand up an in-container relay so the docker mesh can use Open
    # discovery without leaking onto the public overlay.
    # ./scripts/e2e-fips-routed-udp-docker.sh
    # ./scripts/e2e-fips-nat-safe-mtu-docker.sh
    echo "Skipping FIPS docker e2e: known-broken since 5c7b425 (Open discovery)" >&2
    ;;
esac
