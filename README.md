# nostr-vpn

<p align="center">
  <img src="icon.svg" alt="nostr-vpn logo" width="112">
</p>

> Canonical repository: [git.iris.to](https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/nostr-vpn) (`htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/nostr-vpn`). GitHub is a [mirror](https://github.com/mmalmi/nostr-vpn).

`nostr-vpn` is a Tailscale-style private mesh VPN built around a [FIPS]-backed data plane. It includes the `nvpn` CLI/daemon, a shared native app core, and native shells for desktop and mobile platforms.

## Downloads

- [Latest releases on git.iris.to](https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/nostr-vpn?tab=releases)
- [GitHub mirror releases](https://github.com/mmalmi/nostr-vpn/releases/latest)
- CLI from crates.io: `cargo install nvpn`
- [iOS TestFlight beta](https://testflight.apple.com/join/jPRVxbSv) exists, but public access is not live yet

Release artifacts currently cover native Apple Silicon macOS, Linux x64, Windows x64, Android arm64, and headless CLI archives for Apple Silicon macOS, Windows x64, Linux x86_64, and Linux arm64. Intel macOS is source-only for now.

## Quick Start

```bash
cargo install nvpn --force
nvpn init
MY_NPUB='<paste nostr_pubkey from nvpn init>'
nvpn set --participant "$MY_NPUB"
nvpn create-invite
nvpn start --daemon --connect
```

Use the printed invite on another device; do not paste the example placeholder:

```bash
INVITE='<paste invite copied from nvpn create-invite>'
nvpn init
nvpn import-invite "$INVITE"
nvpn start --daemon --connect
```

For the background daemon flow used by desktop apps:

```bash
nvpn start --daemon --connect
nvpn status
nvpn stop
```

For persistent startup:

```bash
sudo nvpn service install
nvpn service status
```

On Windows, run `nvpn service install` from an elevated shell instead of using `sudo`.

## Native Apps

The native apps share the Rust app-core state/action contract and use platform shells for macOS, Linux, Windows, Android, and iOS.

```bash
just build
just run
```

Use `just run-macos` or `just run-linux` when you want a specific desktop target.

<p align="center">
  <img src="docs/images/desktop-gui-overview.png" alt="Nostr VPN desktop app showing a connected Home Mesh network, device status badges, and join request controls." width="900">
</p>

## What Works Today

- Generates Nostr identity keys automatically
- Shares networks through invites and roster/admin sync
- Stores multiple named networks with one active network at a time
- Brings up [FIPS] private mesh tunnels for private network traffic
- Routes private traffic directly when possible and through [FIPS] neighbors when direct UDP is blocked
- Supports MagicDNS, route advertisement, exit-node selection, and WireGuard upstream egress
- Exposes native desktop apps, JSON status, network diagnostics, doctor bundles, desktop updates, and Linux-focused Docker e2e coverage

## Platform Status

| Platform | Status |
| --- | --- |
| Apple Silicon macOS | Native SwiftUI/AppKit app, CLI tarball, signed/notarized release artifacts when credentials are configured |
| Linux x64 | Native GTK/libadwaita app, `.deb`, CLI tarballs, Docker e2e coverage |
| Windows x64 | Native WPF app, installer, CLI zip, WinTun tunnel path |
| Android arm64 | Native app-core UI, signed APK/AAB artifacts when signing is configured, VPN runtime still being hardened |
| iOS | Native SwiftUI app and NetworkExtension target build from source and simulator; public TestFlight access is pending |
| Umbrel / StartOS | Web control panels and service packages |
| Intel macOS | Source-only |

## Further Reading

- [Protocol](docs/protocol.md): invites, admin roster sync, and the [FIPS] mesh data plane
- [Experiments](docs/EXPERIMENTS.md): current benchmark notes
- [Native UI parity matrix](docs/native-ui-parity-matrix.md): native app rewrite status
- [Contributing](CONTRIBUTING.md): maintainer commands and package notes
- [Changelog](CHANGELOG.md): release history

## Maintainer Notes

This section is intentionally compact and command-oriented. Keep user-facing product detail above; keep agent/operator reference material here.

### Config Model

`nvpn init` creates the config and keys automatically. By default, config lives in the OS app config directory:

- Linux: `~/.config/nvpn/config.toml`
- macOS: `~/Library/Application Support/nvpn/config.toml`
- Fallback when no config dir is available: `./nvpn.toml`

The config contains global app settings, Nostr relay/identity settings, NAT settings, node settings, and a `[[networks]]` list. Each network has its own stable `network_id`; only the active network participates in the live runtime.

### Validation

Normal Rust gate:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Release gate before version bumps and tags:

```bash
just release-gate
```

Useful focused checks:

```bash
pnpm --dir web/control-panel check
( cd linux && cargo check )
dotnet build windows\NostrVpn.Windows\NostrVpn.Windows.csproj -p:EnableWindowsTargeting=true
```

Run the Windows build on the configured Windows dev VM when local `dotnet` is unavailable.

### Packages and E2E

- StartOS package: [`startos`](startos)
- Umbrel package: [`umbrel`](umbrel)
- Umbrel local web check:

```bash
docker compose -f umbrel/docker-compose.local.yml up --build
just e2e-umbrel-web
```

Focused security regression kit:

```bash
just security-regressions
```

Docker e2e and desktop updater scripts live under [`scripts`](scripts). The most common entrypoints are `scripts/e2e-docker.sh`, `scripts/e2e-fips-routed-udp-docker.sh`, `scripts/e2e-fips-nat-safe-mtu-docker.sh`, `scripts/e2e-wireguard-exit-docker.sh`, and `scripts/e2e-update-desktop.sh`.

### Release

1. Move `CHANGELOG.md` from `## Unreleased` to `## X.Y.Z - YYYY-MM-DD`.
2. Bump the root `[workspace.package].version` in `Cargo.toml`.
3. Run `node scripts/sync-versions.mjs` and verify with `node scripts/sync-versions.mjs --check`.
4. Run `just release-gate`.
5. Commit, create `git tag vX.Y.Z`, push the tag to `github`, and push `master` to both `github` and htree `origin`.
6. Watch `.github/workflows/release.yml`.

For local artifact staging, use:

```bash
cp .env.release.example .env.release.local
$EDITOR .env.release.local
just release-publish
```

### Workspace Layout

- [`crates/nostr-vpn-cli`](crates/nostr-vpn-cli): `nvpn` CLI and daemon implementation
- [`crates/nostr-vpn-core`](crates/nostr-vpn-core): config, [FIPS] control state, diagnostics, MagicDNS, and NAT helpers
- [`crates/nostr-vpn-app-core`](crates/nostr-vpn-app-core): native app state/action contract and UniFFI bridge
- [`macos`](macos), [`linux`](linux), [`windows`](windows), [`android`](android), [`ios`](ios): native platform shells
- [`umbrel`](umbrel), [`startos`](startos): packaged service/web-control-panel targets
- [`scripts`](scripts): build, release, Docker e2e, and desktop updater entrypoints

[FIPS]: https://github.com/jmcorgan/fips
