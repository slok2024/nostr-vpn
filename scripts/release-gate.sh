#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

source "$ROOT_DIR/scripts/release_common.sh"
enable_deterministic_build_env "$ROOT_DIR"

node scripts/sync-versions.mjs
./scripts/security-audit-rust.sh
cargo fmt --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
# Mobile VPN basics run in the blocking gate without requiring a device/emulator:
# join request over FIPS, MagicDNS from a TUN packet, and Android WG socket
# startup ordering before VpnService.protect(fd).
cargo test --locked -p nostr-vpn-app-core mobile_join_request_sends_and_records_over_real_fips_endpoint
cargo test --locked -p nostr-vpn-app-core mobile_magic_dns_answers_peer_name_from_tun_packet
cargo test --locked -p nostr-vpn-app-core mobile_wireguard_start_returns_before_handshake_watchdog
cargo test --locked -p nostr-vpn-app-core mobile_fips_exit_node_routes_default_traffic_to_selected_member
./scripts/e2e-update-cli.sh

case "${NVPN_RELEASE_GATE_DOCKER_E2E:-1}" in
  0|false|FALSE|False|no|NO|No|off|OFF|Off)
    echo "Skipping Docker e2e because NVPN_RELEASE_GATE_DOCKER_E2E=${NVPN_RELEASE_GATE_DOCKER_E2E}"
    ;;
  *)
    # The join-request flow is gated by e2e-bootstrap-discovery-docker.sh, which
    # exercises the same FIPS join-request control frame deterministically over a
    # direct static endpoint. e2e-join-request-docker.sh covers the public
    # open-discovery/relay path but depends on public Nostr relays, so it flakes
    # in CI; run it manually (or in a nightly) rather than in the blocking gate.
    ./scripts/e2e-bootstrap-discovery-docker.sh
    NVPN_FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}" \
      ./scripts/e2e-fips-routed-udp-docker.sh
    NVPN_FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}" \
      ./scripts/e2e-fips-roaming-docker.sh
    NVPN_FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}" \
      ./scripts/e2e-fips-nat-safe-mtu-docker.sh
    case "${NVPN_RELEASE_GATE_PERF_E2E:-1}" in
      0|false|FALSE|False|no|NO|No|off|OFF|Off)
        echo "Skipping Docker perf regression e2e because NVPN_RELEASE_GATE_PERF_E2E=${NVPN_RELEASE_GATE_PERF_E2E}"
        ;;
      *)
        NVPN_FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}" \
          ./scripts/e2e-fips-perf-regression-docker.sh
        ;;
    esac
    ;;
esac
