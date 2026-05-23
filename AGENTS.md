# AGENTS.md

Repo notes for AI coding agents; pair with operator-local instructions.

## Development Notes

- nvpn performance work: build only the daemon. macOS: `cargo build -p nvpn --bin nvpn --release`, then install/ad-hoc-sign that binary on each test machine. Linux cross-machine binaries: prefer musl, `cargo build -p nvpn --bin nvpn --release --target x86_64-unknown-linux-musl`, install `target/x86_64-unknown-linux-musl/release/nvpn`; use `target/release/nvpn` only on the same distro/glibc family. Compare `iperf3` over LAN/Tailscale/nvpn both directions (`-R`) on macOS and Linux. Use `mesh_mtu_profile = "lan"` / `NVPN_MESH_MTU_PROFILE=lan` only for explicit clean-LAN MTU trials.
- macOS daemon swaps: ad-hoc signing is enough, but run `xattr -c` before signing/copying. If launchd reports `OS_REASON_CODESIGNING`, use `launchctl bootout` + `bootstrap`. System LaunchDaemon restarts still need elevated `launchctl kickstart -k system/to.nostrvpn.nvpn` unless a narrow passwordless sudo rule exists.
- macOS launchd env-var A/Bs: edit `EnvironmentVariables`, then `bootout` + `bootstrap`; `kickstart` may keep the old loaded plist environment. Keep launchd pointed at the signed test daemon, not a stale app resource copy.
- Remote bench automation: load SSH keys into the agent first, e.g. `ssh-add --apple-use-keychain <key>` from interactive macOS; `BatchMode=yes` fails when the key is only in Keychain and UI is unavailable.
- Linux musl CLI builds: use `scripts/build-nvpn-linux-musl <target>` on a Linux Docker builder. It handles cross headers and the rustables workaround for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, and `arm-unknown-linux-musleabihf`. `scripts/build-nvpn-armv6-musl` wraps this for ARMv6 and preserves old target-dir defaults. Both scripts build from a clean `git archive`, can patch against local FIPS via `NVPN_FIPS_REPO_PATH=<path>`, and can smoke-test before install with `NVPN_SMOKE_HOST=<ssh-host>`. Never run ARMv7 binaries on ARMv6 hardware; they may build and then crash immediately. Keep machine names, users, IPs, signing details, hostnames, and device IDs out of committed docs/scripts; use env vars or shell history.
- Windows app checks from macOS: do not stop at local "dotnet missing". Use the configured Windows dev VM, the expected Windows build environment, when reachable: push to that host's git remote, fast-forward its checkout, then SSH-run `dotnet build windows\NostrVpn.Windows\NostrVpn.Windows.csproj -p:EnableWindowsTargeting=true`.
- Mobile-sensitive changes: include Android/iOS. `just mobile-test-kit` runs Rust app-core tests plus Android/iOS debug builds; `just mobile-test-kit-sim` when simulator/emulator launch matters; `just mobile-test-kit-device` for real VPN dataplane, reconnect, LAN discovery, roster transfer, or packet-tunnel changes. Put local details in env vars such as `NVPN_ANDROID_SERIAL` and `NVPN_IOS_DEVICE`, not committed files.
- FIPS mobile coverage: protocol/routing/session/candidate/reconnect/cross-target Rust tests belong in `fips`; Android `VpnService`, iOS NetworkExtension, FFI/JNI/C ABI, VPN permissions, and physical-device packet-path checks belong in this repo's mobile test kit.

## Before Tagging

`v*` tag pushes run `.github/workflows/release.yml`, gated by the same `Lint + Tests` as CI. If the gate fails, no artifacts or GitHub Release are produced; fix, force-update the tag, and rerun the full release.

Always run locally before bumping/tagging:

```sh
just release-gate
```

It mirrors the regular CI gate: sync-versions, fmt, clippy, Rust tests, FIPS join-request Docker e2e, routed-FIPS Docker e2e proving two peers can communicate through an intermediary when direct UDP is blocked, and NAT safe-MTU Docker e2e. Fix any failure or warning before the release commit.

For the Linux GTK app (`linux/`, outside the workspace), also run:

```sh
( cd linux && cargo check )
```

## Release Process

1. Change `CHANGELOG.md` `## Unreleased` to `## X.Y.Z - YYYY-MM-DD`; `scripts/render-release-notes.mjs` / `extractChangelogSection` requires that exact pattern for the GitHub Release body.
2. Bump root `[workspace.package].version` in `Cargo.toml`, the single source of truth. Propagate with `node scripts/sync-versions.mjs` to Linux Cargo.toml, macOS/iOS `project.yml`, Android `build.gradle.kts`, and Windows `.csproj`; verify with `node scripts/sync-versions.mjs --check`.
3. Run the local release gate above.
4. Commit, create a lightweight `git tag vX.Y.Z` at the bump commit, push the tag to `github`, and push `master` to both `github` and htree `origin`.
5. Watch with `gh run list --workflow=release.yml --limit 3`.
