set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

info:
    @echo "Nostr VPN commands"
    @echo
    @echo "Run"
    @echo "  just run"
    @echo "  just run-macos"
    @echo "  just run-linux"
    @echo "  just run-windows"
    @echo "  just run-android"
    @echo "  just run-ios"
    @echo
    @echo "Build"
    @echo "  just build"
    @echo "  just release"
    @echo "  just release-publish"
    @echo
    @echo "macOS"
    @echo "  just macos-gen-swift"
    @echo "  just macos-rust"
    @echo "  just macos-xcframework"
    @echo "  just macos-xcodeproj"
    @echo "  just macos-build"
    @echo
    @echo "Linux"
    @echo "  just linux-build"
    @echo
    @echo "Windows"
    @echo "  just windows-build"
    @echo
    @echo "Android"
    @echo "  just android-build"
    @echo "  just android-install"
    @echo "  just android-smoke"
    @echo "  just android-smoke-vpn"
    @echo
    @echo "iOS"
    @echo "  just ios-build"
    @echo "  just ios-run"
    @echo "  just ios-smoke"
    @echo "  just ios-smoke-device"
    @echo
    @echo "Checks"
    @echo "  just test"
    @echo "  just mobile-test-kit"
    @echo "  just mobile-test-kit-rust"
    @echo "  just mobile-test-kit-sim"
    @echo "  just mobile-test-kit-device"
    @echo "  just release-gate"
    @echo "  just security-regressions"
    @echo "  just e2e"
    @echo "  just e2e-connect"
    @echo "  just e2e-active-network"
    @echo "  just e2e-exit-node"
    @echo "  just e2e-fips-routed-udp"
    @echo "  just e2e-lan-pairing"
    @echo "  just e2e-roster-admin"
    @echo "  just e2e-wireguard-exit"
    @echo "  just e2e-wireguard-exit-userspace"

run:
    @case "$(uname -s)" in \
        Darwin) just run-macos ;; \
        Linux) just run-linux ;; \
        MINGW*|MSYS*|CYGWIN*) just run-windows ;; \
        *) echo "No local run target for $(uname -s). Use just --list for available commands." >&2; exit 1 ;; \
    esac

run-macos:
    ./tools/run-macos

run-linux:
    ./tools/run-linux

run-windows:
    ./tools/run-windows

run-android:
    ./tools/run-android install

run-ios:
    ./tools/run-ios run

build:
    @case "$(uname -s)" in \
        Darwin) just macos-build ;; \
        Linux) just linux-build ;; \
        MINGW*|MSYS*|CYGWIN*) just windows-build ;; \
        *) cargo build -p nvpn -p nostr-vpn-reflector ;; \
    esac
    @./scripts/build-output-path

linux-build:
    ./tools/run-linux cargo build

linux-e2e-gui:
    ./tools/run-linux ./scripts/e2e-smoke.sh

windows-build:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/windows-build.ps1

android-build:
    ./tools/run-android build

android-install:
    ./tools/run-android install

android-smoke:
    ./scripts/mobile-android-smoke.sh

android-smoke-vpn:
    ./scripts/mobile-android-smoke.sh --vpn-cycle

ios-build:
    ./tools/run-ios build

ios-run:
    ./tools/run-ios run

ios-smoke:
    ./scripts/mobile-ios-smoke.sh simulator

ios-smoke-device:
    ./scripts/mobile-ios-smoke.sh device --vpn-cycle

mobile-test-kit:
    ./scripts/mobile-test-kit.sh fast

mobile-test-kit-rust:
    ./scripts/mobile-test-kit.sh rust

mobile-test-kit-sim:
    ./scripts/mobile-test-kit.sh simulator

mobile-test-kit-device:
    ./scripts/mobile-test-kit.sh device

macos-gen-swift:
    ./scripts/macos-build macos-gen-swift

macos-rust:
    ./scripts/macos-build macos-rust

macos-xcframework:
    ./scripts/macos-build macos-xcframework

macos-xcodeproj:
    ./scripts/macos-build macos-xcodeproj

macos-build:
    ./scripts/macos-build macos-build

release:
    node scripts/local-release.mjs

release-publish:
    node scripts/local-release.mjs --publish

test:
    cargo test

release-gate:
    ./scripts/release-gate.sh

security-regressions:
    cargo test -p nvpn platform_routing
    cargo test -p nostr-vpn-app-core mobile_config
    ./scripts/e2e-wireguard-exit-docker.sh

e2e:
    ./scripts/e2e-docker.sh

e2e-connect:
    ./scripts/e2e-connect-docker.sh

e2e-active-network:
    ./scripts/e2e-active-network-docker.sh

e2e-divergent-roster:
    ./scripts/e2e-divergent-roster-docker.sh

e2e-exit-node:
    ./scripts/e2e-exit-node-docker.sh

e2e-fips-routed-udp:
    ./scripts/e2e-fips-routed-udp-docker.sh

e2e-lan-pairing:
    ./scripts/e2e-lan-pairing-docker.sh

e2e-roster-admin:
    ./scripts/e2e-roster-admin-docker.sh

e2e-wireguard-exit:
    ./scripts/e2e-wireguard-exit-docker.sh

e2e-wireguard-exit-userspace:
    ./scripts/e2e-wireguard-exit-userspace-docker.sh
