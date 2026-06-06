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

run_auto_windows_vm_app_smoke() {
  local host="${NVPN_WINDOWS_SSH_HOST:-win11-dev}"
  if ssh -o BatchMode=yes -o ConnectTimeout=5 "$host" hostname >/dev/null 2>&1; then
    ./scripts/windows-vm-app-launch-smoke.sh "$host"
  else
    echo "Skipping Windows VM app launch smoke because ssh $host is unreachable."
  fi
}

run_desktop_app_launch_smokes() {
  local linux_gui_smoke_default=1
  case "${NVPN_RELEASE_GATE_DOCKER_E2E:-1}" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      linux_gui_smoke_default=0
      ;;
  esac

  case "${NVPN_RELEASE_GATE_LINUX_GUI_SMOKE:-$linux_gui_smoke_default}" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Skipping Linux GUI launch smoke because NVPN_RELEASE_GATE_LINUX_GUI_SMOKE=${NVPN_RELEASE_GATE_LINUX_GUI_SMOKE}"
      ;;
    *)
      ./tools/run-linux ./scripts/e2e-smoke.sh
      ;;
  esac

  case "${NVPN_RELEASE_GATE_MACOS_GUI_SMOKE:-auto}" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Skipping macOS app launch smoke because NVPN_RELEASE_GATE_MACOS_GUI_SMOKE=${NVPN_RELEASE_GATE_MACOS_GUI_SMOKE}"
      ;;
    1|true|TRUE|True|yes|YES|Yes|on|ON|On)
      ./scripts/macos-app-launch-smoke.sh
      ;;
    auto|AUTO|Auto|"")
      if [[ "$(uname -s)" == "Darwin" && -d "$ROOT_DIR/macos/Sources" ]]; then
        ./scripts/macos-app-launch-smoke.sh
      else
        echo "Skipping macOS app launch smoke on this host."
      fi
      ;;
    *)
      echo "Unsupported NVPN_RELEASE_GATE_MACOS_GUI_SMOKE=${NVPN_RELEASE_GATE_MACOS_GUI_SMOKE}" >&2
      exit 2
      ;;
  esac

  case "${NVPN_RELEASE_GATE_WINDOWS_GUI_SMOKE:-auto}" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Skipping Windows app launch smoke because NVPN_RELEASE_GATE_WINDOWS_GUI_SMOKE=${NVPN_RELEASE_GATE_WINDOWS_GUI_SMOKE}"
      ;;
    1|true|TRUE|True|yes|YES|Yes|on|ON|On|windows-vm)
      ./scripts/windows-vm-app-launch-smoke.sh "${NVPN_WINDOWS_SSH_HOST:-win11-dev}"
      ;;
    auto|AUTO|Auto|"")
      run_auto_windows_vm_app_smoke
      ;;
    *)
      echo "Unsupported NVPN_RELEASE_GATE_WINDOWS_GUI_SMOKE=${NVPN_RELEASE_GATE_WINDOWS_GUI_SMOKE}" >&2
      exit 2
      ;;
  esac
}

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

run_desktop_app_launch_smokes
