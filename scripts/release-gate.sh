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
    ./scripts/e2e-join-request-docker.sh
    NVPN_FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}" \
      ./scripts/e2e-fips-routed-udp-docker.sh
    NVPN_FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}" \
      ./scripts/e2e-fips-nat-safe-mtu-docker.sh
    ;;
esac
