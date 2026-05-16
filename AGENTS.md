# AGENTS.md

Notes for AI coding agents working in this repo. Pair with the user's
global `~/.claude/CLAUDE.md` instructions.

## Development notes

For nvpn performance work, build only the daemon. On macOS, use `cargo build -p nvpn --bin nvpn --release` and install/ad-hoc-sign that binary on each test machine. For Linux binaries copied between machines, avoid host-glibc coupling by building `cargo build -p nvpn --bin nvpn --release --target x86_64-unknown-linux-musl` and installing `target/x86_64-unknown-linux-musl/release/nvpn`; use the default `target/release/nvpn` Linux binary only on the same distro/glibc family that built it. Then compare `iperf3` over LAN/Tailscale/nvpn in both directions (`-R`) on macOS and Linux; use `mesh_mtu_profile = "lan"` or `NVPN_MESH_MTU_PROFILE=lan` only for explicit clean-LAN MTU trials.

Ad-hoc signing is sufficient for replacing the macOS daemon binary during development, but clear extended attributes before signing/copying (`xattr -c`) and use `launchctl bootout` + `bootstrap` if launchd reports `OS_REASON_CODESIGNING`; restarting the system LaunchDaemon still requires elevated `launchctl kickstart -k system/to.nostrvpn.nvpn` unless a narrow passwordless sudo rule is installed for that restart.

For macOS launchd env-var A/Bs, edit `EnvironmentVariables` and then `launchctl bootout` + `bootstrap`; `kickstart` restarts the daemon but may keep the old loaded plist environment. Keep launchd pointed at the signed daemon binary used for testing rather than a stale app resource copy.

Before remote bench automation, make sure the SSH key is loaded into the agent (for example with `ssh-add --apple-use-keychain <key>` from an interactive shell on macOS); `BatchMode=yes` fails if the key is only in Keychain and user interaction is unavailable.

For ARMv6 Linux devices, build the daemon with `scripts/build-nvpn-armv6-musl` on a Linux Docker builder. The script uses a clean `git archive`, targets `arm-unknown-linux-musleabihf`, and can smoke-test the result with `NVPN_SMOKE_HOST=<ssh-host>` before any install. Do not use an ARMv7 binary on ARMv6 hardware; it may build successfully and then crash immediately. Keep machine names, usernames, and IP addresses out of committed docs/scripts; pass local details through environment variables or shell history instead.

For mobile-sensitive changes, include Android/iOS in the standard kit. Run `just mobile-test-kit` for Rust app-core tests plus Android and iOS debug builds; use `just mobile-test-kit-sim` when simulator/emulator launch behavior matters; use `just mobile-test-kit-device` for real VPN dataplane, reconnect, LAN discovery, roster transfer, or packet-tunnel changes. Keep physical device identifiers, signing details, local hostnames, usernames, and IPs in environment variables such as `NVPN_ANDROID_SERIAL` and `NVPN_IOS_DEVICE`, never in committed files.

Keep FIPS mobile coverage platform-neutral: protocol, routing, session, candidate, reconnect, and cross-target Rust tests belong in `fips`; Android `VpnService`, iOS NetworkExtension, FFI/JNI/C ABI, VPN permissions, and physical-device packet-path checks belong in this repo's mobile test kit.

## Before tagging a release

The release workflow (`.github/workflows/release.yml`) is triggered by
`v*` tag pushes and runs the same `Lint + Tests` checks as the regular
`CI` workflow as a gate before any artifacts are built. If those checks
fail, **no installers / binaries are produced** and the GitHub Release
isn't created — you have to push a fix, force-update the tag, and wait
through another full release run.

Always run the release gate locally first, before bumping the version
and tagging:

```sh
just release-gate
```

This runs sync-versions, fmt, clippy, Rust tests, and the routed-FIPS
Docker e2e that verifies two peers can communicate through an
intermediary when their direct UDP path is blocked. These mirror the
regular CI gate. If any step fails or warns, fix it before you cut the
release commit.

For the Linux GTK app (`linux/`, excluded from the workspace) also run:

```sh
( cd linux && cargo check )
```

## Release process

1. Update `## Unreleased` in `CHANGELOG.md` to a versioned + dated
   header like `## 4.0.10 - 2026-05-10`. The release notes generator
   (`scripts/render-release-notes.mjs` →
   `extractChangelogSection`) matches this exact pattern when looking
   up the section to put in the GitHub Release body.
2. Bump `[workspace.package].version` in the root `Cargo.toml`. This is
   the single source of truth — propagate to every other version file
   with `node scripts/sync-versions.mjs` (covers Linux Cargo.toml,
   macOS / iOS `project.yml`, Android `build.gradle.kts`, Windows
   `.csproj`). Verify with `node scripts/sync-versions.mjs --check`.
3. Run the local release gate (above).
4. Commit, tag (`git tag vX.Y.Z` — lightweight, pointing at the bump
   commit), and push the tag to `github` to trigger the release
   workflow. Also push `master` to both `github` and the htree `origin`.
5. Watch the run: `gh run list --workflow=release.yml --limit 3`.
