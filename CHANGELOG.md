# Changelog

All notable changes to this project are documented in this file.

## 4.0.12 - 2026-05-13

### Changed

- **Bumped fips-endpoint to `02c00a0` — Darwin connected-UDP and tunnel
  reliability refresh.** The macOS private mesh now installs per-peer connected
  UDP sockets by default after fixing the listener/peer `SO_REUSE*` mismatch
  that made earlier Darwin connected-socket tests unstable. On the MacBook
  Wi-Fi to Ethernet-mini path, the best same-window sample improved to about
  256 Mbit/s forward and 404 Mbit/s reverse; reverse is now Tailscale-level in
  current samples, while the forward direction remains packet-rate limited by
  the Darwin Wi-Fi sender path.
- Restored the private mesh default MTU budget to the IPv6-safe
  `MESH_UNDERLAY_UDP_MTU=1280` / `MESH_TUNNEL_MTU=1150`. Larger
  LAN-sized frames can work on direct Ethernet/Wi-Fi paths, but should
  be enabled only after blackhole-safe per-path probing or an explicit
  operator override; making 1420 the global default is too optimistic
  for NAT traversal and nested tunnels.
- Added an explicit private mesh MTU test lever:
  `mesh_mtu_profile = "lan"` (or `NVPN_MESH_MTU_PROFILE=lan`) selects
  a 1420-byte underlay / 1290-byte tunnel budget for clean LAN paths,
  while `mesh_underlay_udp_mtu`, `mesh_tunnel_mtu`,
  `NVPN_MESH_UNDERLAY_UDP_MTU`, and `NVPN_MESH_TUNNEL_MTU` allow
  bounded manual overrides.
- Private FIPS mesh packet routing now moves owned packet buffers through the
  send/receive hot path instead of cloning each packet at the nvpn mesh layer.
- FIPS macOS sending now defaults to the hash-by-send-target worker path instead
  of the per-flow ordered sender thread. Live MacBook Wi-Fi to Ethernet-mini
  testing improved the weak direction from about 103-109 Mbit/s to about
  147 Mbit/s while keeping reverse around 350 Mbit/s; the ordered path remains
  available for A/B runs with `FIPS_MACOS_ORDERED_SENDER=1`.
- FIPS macOS worker drain size can now be A/B tested with
  `FIPS_MACOS_WORKER_BATCH`; the default remains 32 after smaller batches
  regressed local Wi-Fi/Ethernet throughput.
- Added runtime-only pipeline tracing and recorded the MacBook/mini and mini
  Docker performance experiments in `docs/EXPERIMENTS.md`.

### Fixed

- Windows WireGuard-upstream test/routing now supports the WinTun default-route
  path, including endpoint bypass, optional probe ping, hold/cleanup behavior,
  and safer script defaults for the Windows VM e2e helper.
- Completed join requests are cleared after roster updates so stale join state
  does not persist after the network accepts the request.
- LAN pairing workers now stay alive while the UI arms broadcast/discovery,
  fixing a startup race; LAN pairing, WG-upstream, and diagnostics test paths
  now use loopback-only sockets so local release checks do not trigger macOS
  firewall prompts.
- Private FIPS mesh session recovery now forces a fresh path after stale FSP
  sessions or route churn, reducing long-lived dead-link states after peer
  restart or network roaming.
- macOS/Linux private FIPS mesh writes to the TUN device now wait for fd
  writability and retry on `WouldBlock` instead of using boringtun's helper
  that collapses every write error to `0`. This avoids silent utun drops under
  sustained reverse UDP load on macOS.
- FIPS direct UDP worker sends on non-Linux now honor the bulk-data
  `drop_on_backpressure` policy instead of retrying every endpoint data packet
  indefinitely under Darwin UDP send pressure; mixed/control batches still retry
  so rekeys and handshakes are not stranded.

## 4.0.11 - 2026-05-11

### Changed

- **Bumped fips-endpoint to `9b7c723` — boringtun-style data-plane perf overhaul.** Single squash-merge landing 49 commits worth of FIPS receive/send hot-path work. **Single-stream TCP went from ~1.5 Gbps baseline to ~2.2 Gbps on Mac docker bench (+47%) and 2.24 Gbps on ubuntu-dev netns+veth host bench (+62% over same-host baseline).** Multi-stream throughput moves up 8-15% across 4/8-stream configurations. Highlights:
  - **Shard-owned decrypt worker pool** (std::thread + crossbeam_channel) — each worker owns its session state in a thread-local HashMap. No `Arc<RwLock<HashMap>>` cache on Node, no `Arc<Mutex<ReplayWindow>>` shared with rx_loop. Direct `&mut` access per packet, zero lock acquires per AEAD layer. Hash-by-cache_key dispatch so a session always lands on the same shard.
  - **UDP_GSO on Linux** — `sendmsg(2)` + `UDP_SEGMENT` cmsg path for uniform-size batches, falling back to `sendmmsg(2)` on EINVAL/EOPNOTSUPP. Kernel splits one super-skb into N on-the-wire UDP datagrams via a single skb walk (the same primitive WireGuard kernel + boringtun use to hit 2.5–3.2 Gbps).
  - **Connected UDP per peer on Linux** — `ConnectedPeerSocket` (SO_REUSEPORT + bind + connect) for each established peer, with a `PeerRecvDrain` std::thread feeding the existing `packet_tx`. Encrypt-worker send path uses the peer's connected fd with `msg_name = NULL`; kernel skips per-packet sockaddr handling + route lookup + neighbor resolve. Pairs cleanly with UDP_GSO.
  - **Hot-path zero-copy** — UDP RX `mem::replace` (recv buffer IS the packet buffer), `SessionDatagramRef::decode` borrowed view (no inner-payload alloc on the default rx_loop path), FSP decrypt in-place on `packet_data`, eliminated `payload.drain(..6)` 1.5 KB memmove. Sender builds the FMP wire buffer in one allocation directly. Net ~150–450 MB/sec of memory bandwidth recovered.
  - **Eager session registration** — workers receive the FMP recv state at handshake completion (`promote_connection`) rather than lazily on first packet, eliminating the legacy lazy-register path entirely.
  - **`FipsEndpoint::send` SendOneway fast path** — skips the per-packet `oneshot::channel()` allocation that the old code created and immediately dropped.
  - All env-gated experimental knobs collapsed into always-on defaults. `FIPS_ENCRYPT_WORKERS` / `FIPS_DECRYPT_WORKERS` / `FIPS_TUN_QUEUES` remain as debug overrides; their default is `num_cpus`. `FIPS_CONNECTED_UDP` is removed (unconditionally on for Linux).
  - 221 node tests pass; 8 new transport/encrypt-worker unit tests pass on Linux (5 GSO + 2 ConnectedPeerSocket + 1 PeerRecvDrain).
- All-platform fips-endpoint refresh — Android, iOS, macOS, Windows, Linux, GTK, WinTun, and the CLI / daemon all pick up the new perf profile via the workspace fips-endpoint dep.

### Added

- **macOS daemon now drives WireGuard upstream end-to-end.** The "WireGuard upstream" radio item in the GUI does something on Mac for the first time: when toggled on (and a config has been pasted), the daemon's `FipsPrivateTunnelRuntime` brings up a userspace tun via boringtun, runs the WG handshake against the upstream, and **only swaps the default route to the WG tun once the handshake actually completes within a 10-second watchdog window**. If the handshake doesn't complete the routing table is deliberately left untouched, so a misconfigured config or unreachable upstream cannot blackhole the host. Toggling off, changing the config, or stopping the daemon tears the tunnel back down via a `Drop`-guard that restores the original default route + deletes the WG-endpoint bypass.
- **Same flow now wired up on Windows.** `apply_daemon_wg_upstream` on Windows creates a dedicated WinTun adapter for the WG upstream (separate from the FIPS adapter), drives the boringtun runtime against it, captures the underlay default route via `route print -4 0.0.0.0`, installs a `/32` bypass for the WG endpoint at metric=1, and adds `0.0.0.0/0` via the WG WinTun adapter at metric=1 so it wins LPM against the kernel-managed default. `WindowsFullDefaultRoute::Drop` `netsh delete`s both routes on cleanup; the original default (still in the table at its higher metric) becomes active again. Same handshake-first guarantee — if the WG handshake never completes, no `netsh` calls are made.
- `crates/nostr-vpn-cli/src/wg_upstream_runtime.rs` is now tun-source-agnostic: the boringtun pump is driven by mpsc channels for tun I/O, with platform-specific reader/writer tasks (`spawn_posix_tun_reader` / `spawn_posix_tun_writer` for Linux+macOS, `spawn_wintun_reader` / `spawn_wintun_writer` for Windows). Adds `start_with_tun` (POSIX), `start_with_wintun` (Windows), and `start_with_channels` (mobile-ready: lets a host process feed plaintext packets in/out of the WG runtime via raw channels — what an iOS NEPacketTunnelProvider or Android VpnService extension would call after the OS-side route declaration).
- FIPS mesh peer routes keep going through the FIPS tunnel even when WG upstream is up: peer `/32`s are installed before the WG default-route swap, so longest-prefix-match keeps mesh traffic on the FIPS tunnel and only "the rest of the internet" goes through Mullvad/Proton. Holds on macOS (kernel routing table) and Windows (LPM + adapter metrics) without any extra daemon code.
- New unit tests for the Windows `route print -4 0.0.0.0` parser (3 tests covering the happy path, On-link skip, and missing-default cases), runnable on all platforms via `cargo test -p nostr-vpn-cli --features embedded-fips`. The handshake-against-paired-responder integration test moved with the runtime to `nostr-vpn-core` and runs there; 109 cli + 36 core tests pass.

### Notes / not yet done

- **iOS and Android now wired up too.** The boringtun pump moved from `nostr-vpn-cli` into `nostr-vpn-core::wg_upstream` so `nostr-vpn-app-core` (the mobile crate) can use the same runtime. Platform-specific tun adapters (`start_wg_runtime_with_posix_tun`, `start_wg_runtime_with_wintun`) stay in cli; mobile uses the platform-agnostic `WgUpstreamRuntime::start_with_channels`. `MobileTunnel` now spawns the WG runtime alongside the FIPS endpoint when the user has a WG upstream config and dispatches outbound packets between the two: anything that matches a mesh peer route goes through the FIPS endpoint, anything else goes through boringtun → upstream UDP. `MobileTunnelConfig` carries `excludedRoutes` (for iOS) and `wireguardExit` (for the Rust runtime) over the JSON FFI. `nostr-vpn-core` no longer enables boringtun's POSIX-only `device` feature, so the crate cross-compiles cleanly to `aarch64-apple-ios` and `aarch64-linux-android`.
- **iOS Swift glue.** `PacketTunnelProvider.swift` reads the new `excludedRoutes` field from the JSON config and sets it on `NEIPv4Settings.excludedRoutes`. The kernel routes the encrypted UDP outside the VPN tunnel automatically — no socket-protection FFI needed.
- **Android Kotlin glue.** New JNI binding `NativeCore.mobileTunnelWgSocketFd(handle): Int` exposes the boringtun UDP socket fd. `NostrVpnService` calls `protect(fd)` after `mobileTunnelNew` so the encrypted UDP escapes the VPN tun. -1 means WG upstream isn't running, which is the default.

## 4.0.10 - 2026-05-10

### Changed

- Bumped fips-endpoint to `1abda1c`. New commits since 4.0.9:
  - `1abda1c` **encrypted: single-borrow refactor of handle_encrypted_frame fast path.** Mirrors the FSP refactor from 4.0.9 on the FMP layer. Per-thread CPU sampling on the v4.0.9 build identified one tokio worker pegged at 99.9% on a single core on both nodes during a TCP bench while 14 other workers idle, confirming the rx_loop pipeline as the single-thread bottleneck. The new shape compresses the per-packet work in `handle_encrypted_frame` from ~5 hashmap operations (sanity check + K-bit detection + decrypt + stats — three separate `peers.get(_mut)` calls) down to 2 (one immutable borrow for K-bit detection, one mutable borrow that runs decrypt + inner-header parse + MMP + link stats + touch in straight-line code), funneling the result through an `FmpFrameOutcome` enum so dispatch_link_message runs after the peer borrow is dropped. **TCP single-stream now averages ~1530 Mbps with peaks at 1552 Mbps**; multi-stream throughput moves up 2-9% (16-stream 1562 → 1575, 32-stream 1485 → 1622, 64-stream 1571 → 1623, 4-stream zerocopy 1487 → 1521 Mbps); UDP receiver ceiling rises from ~1278 to ~1306 Mbps at 1.5G load. UDP @1 Gbit stays lossless. All 1092 fips-core unit tests pass.

### Fixed

- Join-network UX overhaul, all platforms (Windows, macOS, iOS, Android, Linux GTK):
  - **"Invite Devices" and "Join Network" are now separate cards.** Sharing your network and joining someone else's are clearly distinct actions instead of sharing the same composite card. The QR / your-invite / "Broadcast invite" toggle live in *Invite Devices*; the paste field, paste-from-clipboard, scan QR, from-file picker, and the *Look for nearby* toggle + nearby-invites list live in *Join Network*.
  - **Auto-import on paste**: dropping any string starting with `nvpn://invite/` into the paste field triggers import immediately — no extra click. The field is cleared on dispatch, so the same import never re-fires and a stale invite from a previous session can't sit in the field across launches (which on Windows looked like it had been "pre-filled with the user's own invite").
  - **Scan / Paste / From-file buttons now have text labels**, not just icons (Windows, macOS, iOS, Android, Linux). Camera-only / icon-only buttons were ambiguous about what each did.
  - **Nearby pairing split into two independent 15-min toggles**: *Broadcast invite* (advertise our active network's invite over LAN) and *Look for nearby* (listen for other devices' invites). Previously a single "Pair nearby" toggle did both at once, which was confusing and meant you couldn't, say, broadcast for 15 min while not listening, or look for nearby invites without exposing your own. Each timer has its own remaining-seconds display in the button text.
- LAN pairing (`crates/nostr-vpn-app-core/src/lan_pairing.rs`): Windows now actually receives multicast invites broadcast by macOS / Linux peers on the same LAN. The previous single `join_multicast_v4(&addr, &INADDR_ANY)` left it to the OS to pick *one* interface to subscribe on, and Windows routinely picked a virtual adapter (Hyper-V vEthernet, WSL, Tailscale) instead of the physical Wi-Fi/Ethernet — so the Mac saw the Windows announcements (Mac's join was correct) but not vice versa. The new code enumerates every non-loopback IPv4 interface via `netdev`, joins multicast on each, and on every send fans out: per-interface multicast (via `set_multicast_if_v4` per send) plus a per-interface directed broadcast plus a global `255.255.255.255` last-resort. `SO_BROADCAST` is enabled so the receive side accepts all of them. Same fix applies to macOS / iOS / Android / Linux — multi-NIC hosts (Wi-Fi + Ethernet, dev VMs with several bridges) now reach peers on every L2 segment they're attached to instead of only whichever the routing table happened to prefer.
- Windows app: the read-only "your invite" display textbox (which was visually indistinguishable from the empty paste box right next to it) now sits inside the *Invite Devices* card under a clear "Your invite" caption, with the paste box moved to a separate *Join Network* card — so what looked like "the paste field is pre-filled with our own invite" is now obviously two different fields with two different roles.
- Windows app: console window no longer flashes every ~2s. The GUI's periodic `nvpn status` refresh (and other CLI invocations through `run_nvpn`) now spawn `nvpn.exe` with `CREATE_NO_WINDOW`, suppressing the conhost popup that fired on every `DispatcherTimer` tick. Implemented via a small `CommandWindowExt::hide_console_window` helper in `nostr-vpn-core::process_ext` that's a no-op on non-Windows.
- macOS Exit Nodes page: WireGuard upstream config textarea no longer wipes itself out every ~1s while typing. Each draft field (node name, endpoint, tunnel IP, listen port, MagicDNS suffix, WireGuard config) now syncs from upstream only when its own upstream value actually changes, instead of on every state-rev tick. Network-name and participant-alias drafts get the same treatment so in-flight edits survive periodic refreshes.
- Linux daemon: WireGuard upstream now activates as the local node's own egress when `wireguard_exit.enabled` is set, even if the node is not advertising itself as a mesh exit. Previously the WG tunnel was only brought up when the node was serving as an exit for the rest of the mesh, so toggling "Use WireGuard upstream" alone routed nothing through it.

### Changed

- macOS Exit Nodes page: WireGuard upstream now appears as a radio item in the same exit-target list as Direct and mesh peer exits, so the three options are mutually exclusive and visually unified. The standalone "Use WireGuard upstream" toggle is gone — selecting the radio enables it, selecting Direct or a peer disables it. The paste-config card moves to a "Configure WireGuard Upstream" disclosure that auto-expands on first run when no config has been pasted yet.

### Added

- Userspace WireGuard upstream runtime (`crates/nostr-vpn-cli/src/wg_upstream_runtime.rs`) — boringtun-based single-peer-upstream pump that brings WG-as-a-VPN-client (Mullvad / Proton style) to platforms where the daemon doesn't have a kernel-WG implementation wired in (i.e. macOS today). Owns the `Tunn`, a `UdpSocket` to the upstream, and optionally a `TunSocket`; runs three feeder tasks (timer / udp-rx / tun-rx) into a single coordinator that dispatches into `Tunn` sequentially, so the `Tunn` doesn't need a mutex.
- `nvpn wg-upstream-test --config-file <path>` is the safe-by-construction entry point: parses a wg-quick / Mullvad / Proton config, runs the userspace state machine against the upstream, reports whether the WG handshake completes within the timeout. **Without `--scoped-host`, does not create a tun device, does not modify routes** — running it can never blackhole the host's internet, even if the config is broken or the server is unreachable.
- `nvpn wg-upstream-test --scoped-host <ip>` adds the next layer: brings up a userspace tun, installs **only a single host route** to the given IP through it (default route is untouched), runs the WG handshake, then `ping`s the target through the tunnel and tears everything back down via a `Drop`-guard that runs the matching `route delete`. The host route is the only thing this can possibly break — `route delete -host <ip>` puts the host back exactly as it was even if the process is killed mid-run. Requires sudo on macOS / Linux because tun creation does.
- `nvpn wg-upstream-test --replace-default` is the dangerous mode: routes ALL outbound traffic through the WG tunnel, Mullvad/Proton-style. Designed handshake-first — the tun comes up, the WG runtime starts, and only after the WG handshake completes within `--timeout-secs` does the default route get swapped. If the handshake never completes the routing table is **never modified**, so a misconfigured config or unreachable upstream cannot take the host offline. Once the swap happens, a `FullDefaultRoute` Drop guard restores the original default + deletes the WG-endpoint bypass on cleanup, and the `--hold-secs` window catches Ctrl-C so SIGINT during the hold falls through to the revert path. Verified end-to-end in `e2e-wireguard-exit-userspace-docker.sh` stage 3.
- `apply_full_default_route(iface, address, upstream, mtu) -> FullDefaultRoute` in `wg_upstream_runtime.rs` is the reusable primitive: captures the underlay default route, installs a /32 bypass for the WG endpoint via that gateway, swaps the default to dev `<iface>`, and returns a guard whose Drop / `revert()` puts everything back. Cross-platform (Linux + macOS); will be the building block for daemon integration on macOS.
- `scripts/e2e-wireguard-exit-userspace-docker.sh` (`just e2e-wireguard-exit-userspace`) drives both stages inside Docker: stage 1 is the handshake-only probe, stage 2 brings up the userspace tun and verifies that ICMP traffic to a target on a separate subnet reachable only via the WG upstream actually traverses the tunnel (packet counter on the target asserts the source IP is the upstream's MASQUERADEd public IP, not the bridge IP).
- `scripts/e2e-wireguard-exit-docker.sh` + `docker-compose.wireguard-exit-e2e.yml` (`just e2e-wireguard-exit`) verify on Linux that an nvpn node with `wireguard_exit.enabled=true` and no advertised exit routes still routes its own internet traffic through the WireGuard upstream tunnel. The test puts the internet target on a separate subnet only reachable via the WG upstream's public-side eth, so any successful ping proves the tunnel is actually carrying the traffic, and a packet counter on the target confirms the source IP was MASQUERADEd to the upstream's public IP rather than leaking out the local bridge.

## 4.0.9 - 2026-05-09

### Changed

- Bumped fips-endpoint to `0b96f9c`. New commits since 4.0.8:
  - `0b96f9c` **udp: amortise per-packet sendto via sendmmsg(2) batching.** Per-region timing on the send hot path identified the kernel `sendto(2)` syscall as 52% of send-path CPU (2588 ns/pkt). Per-transport pending-send buffer + `sendmmsg(2)` flush at threshold (8 packets) + end-of-drain flush from the rx_loop amortises that cost across batches. Linux only; non-Linux falls through to per-packet send. **TCP single-stream 1066 → 1548 Mbps (1.45×)** on the 2-node Docker e2e bench, consistent across 1/4/8 streams. UDP @1 Gbit stays lossless at line rate.
  - `5c8deb3` udp: clippy hygiene under `-D warnings` on Linux.
  - `156cc4e` udp recv_batch: sample SO_RXQ_OVFL per batch + drop unsafe transmute on the inbound batched path.
  - `6f3d35b` **session: collapse handle_encrypted_session_msg to a single sessions borrow.** Down from 7 `self.sessions` operations per packet (`get` + `get` + `get_mut` + `remove` + `insert` + `get_mut` + `get_mut`) to one `get_mut` held inside a labeled block, with an `FspFrameOutcome` enum carrying slow-path decisions out for `&mut self` handling. Receive-side CPU drops 17.7% (3706 → 3050 ns/pkt). Throughput unchanged on its own — receive wasn't CPU-bound — but frees headroom for many-peer scaling and lower battery draw. Preserves the auto-rehandshake feature from `a38334b`.
  - `a38334b` session: auto re-handshake after consecutive AEAD decryption failures. Recovers from stale session state on either side (peer restart with new keys, etc.) without requiring a manual daemon restart.
- Cumulative bench trajectory from session start: TCP single-stream 1.57 Mbps → **1548 Mbps (~985×)**, with ring (NEON) AEAD + recvmmsg + drain batching + the FSP refactor + sendmmsg batching all stacking. The remaining gap to boringtun-`--threads=1` (3252 Mbps) is dominated by the dual-AEAD architecture (FSP + FMP encrypt per packet, vs WireGuard's single AEAD) plus the MTU difference (nvpn 1150 vs WG 1420).

## 4.0.8 - 2026-05-09

### Changed

- Bumped fips-endpoint to `6ce3bbc` to swap the AEAD from chacha20poly1305 (RustCrypto soft backend, ~600–800 MB/s/core on aarch64 because the chacha20 crate has no NEON path) to ring 0.17 (BoringSSL ChaCha20-Poly1305 with hand-tuned NEON on aarch64 and AVX2/AVX-512 on x86_64). Same wire format — fips-core's 1091 tests including IK + XK handshakes and the 100-node session test all pass byte-for-byte. ring is already transitively in the dep tree via rustls so no compile/link cost added.
- Bumped fips-endpoint to `9ca1e8b` to expose an additive off-task encrypt/decrypt API on `CipherState` and `NoiseSession` (`encrypt_with_counter[_and_aad]`, `cipher_clone`, `take_send_counter`, `accept_replay`) that lays the groundwork for a future parallel-worker pipelined dispatcher. All additive — zero behavior change on the existing rx/tx paths.
- Docker e2e bench impact (DURATION=10, identical hardware before/after):
  - 2-node direct (A↔B): TCP 1-stream 437 → 1097 Mbps (2.51×); TCP 4-stream 439 → 1109 Mbps (2.53×); TCP 8-stream 445 → 1069 Mbps (2.40×); UDP @1000 Mbit offered 599/40% loss → 1000 Mbps lossless; ping under load 0.63 → 0.71 ms.
  - 3-node forced transit (A → C → B): TCP 1-stream 438 → 1019 Mbps (2.33×); TCP 4-stream 421 → 982 Mbps; TCP 8-stream 443 → 1031 Mbps; UDP @1000 Mbit 475/52% loss → 1000 Mbps lossless; ping-under-load tail 215 → 3.6 ms max (~10× — the relay was crypto-bound, so once AEAD is NEON the queue stops accumulating).
- Cumulative trajectory from session start: TCP single-stream 1.57 → 1097 Mbps (700×); ping under load 456 → 0.71 ms (640×).

### Added

- `scripts/sync-versions.mjs` propagates the `[workspace.package].version` in `/Cargo.toml` to every other version-bearing file (linux/Cargo.toml, macos/project.yml, ios/project.yml, android/app/build.gradle.kts versionCode + versionName, Windows .csproj). Hooked into `local-release` runVerify and the `macos-build` xcodegen step so platform versions can no longer drift from the workspace silently.

### Fixed

- Service-repair recommendations now compare the installed daemon binary version against the bundled `nvpn` CLI version (queried via `nvpn version --json`) instead of against `app_version`. The new `expected_service_binary_version` field on `NativeAppState` is what `service install --force` would actually deploy, so the comparison is now apples-to-apples and stops false-positive repair prompts when the bundled CLI is at a different version than the app shell.
- Windows GUI no longer crashes at XAML startup; the `Run.Text` bindings are pinned to OneWay.

## 4.0.7 - 2026-05-09

### Fixed

- Bumped fips-endpoint to `38babf8` to pick up four upstream data-path fixes: nostr peers running an unspeakable FMP version no longer trigger a per-minute STUN-offer-answer-punch retry storm; proactive `PathMtuNotification` now feeds the path-MTU lookup so new TCP flows on long-lived paths use up-to-date MSS clamping; mid-chain ancestor swaps in deep mesh trees propagate to leaf coords (was 100% loss to non-parent destinations until depth or parent also changed); dead `SessionSetup`/`SessionAck` variants removed.

## 4.0.6 - 2026-05-09

### Fixed

- NAT-traversed sessions no longer drop full-sized tunnel datagrams. The recent 1320 B tunnel MTU bump (encrypted wire ~1426 B) silently broke any session promoted onto a NAT-traversed link, because `Node::adopt_established_traversal` was creating the adopted UDP transport with `UdpConfig::default()` (MTU 1280) and oversized packets were dropped at the socket layer. Reverted nostr-vpn-core constants to `MESH_TUNNEL_MTU=1150` / `MESH_UNDERLAY_UDP_MTU=1280`, and bumped fips-endpoint to `4031be2` so adopted transports inherit the operator's primary `[transports.udp]` config (defensive — both ends now match at 1280 regardless).

## 4.0.5 - 2026-05-09

### Fixed

- Windows release build no longer references the deleted `nostr-vpn-reflector` crate, which had been broken since the FIPS-only mesh cleanup landed.

## 4.0.4 - 2026-05-09

### Changed

- FIPS endpoint `run_rx_loop` drain cap raised from 64 to 256, keeping the worker on the hot path through ~400KB of contiguous traffic between scheduler hops. Bench: TCP single-stream 120 → 423 Mbps (+253%), UDP 200M offered 178 (11% loss) → 200 Mbps (0.0005% loss), ping under load 0.84 → 0.71 ms.

### Fixed

- macOS tray submenus stay open across state refreshes. The previous SwiftUI `MenuBarExtra` rebuilt its menu hierarchy every time the daemon state was republished (~1.5s), dismissing any open submenu within ~1s. The tray is now an `NSStatusItem` with `NSMenu` items mutated in place.

## 4.0.3 - 2026-05-09

### Fixed

- FIPS spanning-tree: nodes whose only smaller-NodeAddr parent disappeared no longer advertise an ancestry whose advertised root is not the path's minimum entry. Previously such announces were rejected by recipients with `invalid ancestry: advertised root X is not the minimum path entry Y`, blocking mesh transit (e.g. mini→ubuntu-dev / mini→win11 through a shared mac peer).

## 4.0.2 - 2026-05-08

### Changed

- FIPS-backed private meshes now use the updated scoped discovery defaults, so same-LAN peers can share local candidates and prefer direct local underlay paths before routed internet paths.

### Fixed

- LAN invite broadcast remains active for 15 minutes or until cancelled, with reusable multicast sockets for multiple local app instances and Linux Docker e2e coverage for looped multicast invite exchange.

## 4.0.1 - 2026-05-08

### Added

- Exit-node leak protection can block internet access while a selected/enabled exit node is not active, with native status shown in app headers, trays, and menus.

### Changed

- WireGuard upstream setup now lives under Exit Nodes and accepts a pasted full WireGuard config block.
- Device rows now distinguish direct FIPS paths from routed FIPS paths on desktop and mobile.

### Fixed

- macOS release artifacts are signed/notarized `.dmg` downloads plus signed/notarized `.app.tar.gz` updater archives again; local and GitHub release paths now fail before publishing if signing or notarization is missing.
- GitHub macOS release builds now use the same Apple ID notarization secrets as the local release path when App Store Connect API key secrets are not configured.
- Linux, Windows, and Android GUI release artifacts are first-class release outputs again, and public release staging now fails if the app artifacts are incomplete or Android artifacts are unsigned.
- Desktop update stripes now restore the auto-install checkbox.
- Android and iOS now show the active network name outside the device rows, keep VPN on/off in the top bar, and list this device as a normal participant row instead of treating the first peer as a hero.
- WireGuard-backed exit-node providers now route their own default internet traffic through the WireGuard upstream too, while preserving the WireGuard peer endpoint on the underlay route.
- Native settings now expose the persisted WireGuard upstream fields used by exit-node providers.
- macOS no longer reapplies the FIPS utun address and peer routes on every heartbeat when the route set is unchanged.
- Linux Magic DNS falls back to managed `/etc/hosts` entries for `.nvpn` names when `systemd-resolved`/`resolvectl` is unavailable.
- New Android and iOS installs seed the editable device name from the phone/tablet instead of falling back to host-derived or generated labels.

## 4.0.0 - 2026-05-07

Changes since `0.3.23` on 2026-05-05.

### Added

- Native desktop/mobile shells now cover macOS, Linux, Windows, Android, and iOS through the shared app-core state/action contract, replacing the legacy Tauri frontend.
- FIPS is now the default private mesh data plane, with open peer discovery, mobile peer discovery, local traversal candidates, verified adverts, transit UDP, and Docker e2e coverage.
- Desktop updater e2e coverage now checks local release manifests and update asset preparation on macOS, Linux, and Windows.
- Local workflow recipes now expose platform run/build commands and build output paths.

### Changed

- macOS now uses a Tailscale-style three-column desktop layout with a toolbar VPN switch, sidebar settings, device detail actions, and daemon-level desired VPN state.
- Linux and Windows native shells were brought closer to macOS parity, including tray/deep-link/update/service behavior.
- Mobile apps now use switches for VPN on/off and keep device sharing behind the Devices plus button.

### Fixed

- Native app peer counts now exclude self devices, FIPS mesh status is surfaced consistently, and self/non-admin peer actions are hidden where they are not valid.
- macOS normal VPN on/off no longer requests administrator privileges; admin prompts are reserved for explicit background service management.
- Release validation now guards against incomplete Linux desktop asset sets and keeps versionless CLI assets in local release notes.

## 0.3.23 - 2026-05-05

Changes since `0.3.22` earlier on 2026-05-05.

### Fixed

- Linux Docker step now defaults to the host architecture (`linux/arm64` on Apple Silicon, `linux/amd64` on Intel) instead of forcing `linux/amd64`. The forced cross caused Docker to run an emulated x86_64 image under QEMU on Apple Silicon, where tauri-cli panicked with `Option::unwrap on None at rust.rs:1142` → SIGABRT during the AppImage/deb bundle step. Native-arch builds skip QEMU entirely. Override is still available via `NVPN_LINUX_DOCKER_PLATFORM` for hosts with a real x86_64 environment that want cross-arch artifacts.
- Linux artifact filenames now match the actual built arch — `*-linux-arm64.AppImage` / `*-linux-arm64.deb` and `nvpn-*-aarch64-unknown-linux-musl.tar.gz` on arm64 hosts, the `x64` / `x86_64` equivalents on amd64. Previously the script always wrote `-linux-x64` regardless of what was built.
- Linux AppImage bundling now has `xdg-mime` available in the Docker image via `xdg-utils`; Tauri's bundler calls it while assembling the AppImage.
- Local release now fails on selected platform build failures by default instead of publishing partial releases with missing assets. Use `--allow-partial` / `NVPN_RELEASE_ALLOW_PARTIAL=1` only when that is intentional.

### Changed

- `Dockerfile.linux-release`: dropped the hardcoded `rustup target add x86_64-unknown-linux-musl` so the inner build script adds whichever musl target matches the running arch. Added `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc` so aarch64 musl builds link cleanly with the same toolchain.

## 0.3.22 - 2026-05-05

Changes since `0.3.21` earlier on 2026-05-05.

### Fixed

- Linux Docker step: invoking the inner build with `bash -lc` made it a login shell, which re-sourced `/etc/profile` and dropped the rust:bookworm image's `/usr/local/cargo/bin` from PATH. `pnpm install` worked (pnpm is in /usr/bin) but the subsequent `tauri build` ran `cargo metadata` and got `bash: line 1: cargo: command not found`, surfaced by tauri-cli as the cryptic `failed to run 'cargo metadata' command…` line. Switched to `bash -c` so the Dockerfile-set PATH wins and cargo is reachable.

## 0.3.21 - 2026-05-05

Changes since `0.3.20` earlier on 2026-05-05.

### Fixed

- SystemPanel "Updates" section now keeps the muted "Last checked …" timestamp visible after a manual check instead of replacing it with the status message. The status line ("You're up to date.", "Installed …", "Available …") sits above it.
- "You're up to date." and error messages auto-clear back to idle after ~4 s — they were lingering forever, even though they aren't actionable. Available / installing / installed states still stay (those need user follow-up).
- Linux Docker release step now actually runs: forces `linux/amd64` platform (previously it inherited Apple Silicon's `linux/arm64` and would have produced mis-named aarch64 artifacts), passes `CI=true` so pnpm purges the host's macOS `node_modules` non-interactively, and snapshots the source into `/build` inside the container so the host's `node_modules` and `target/` aren't trashed cross-platform. Configurable via `NVPN_LINUX_DOCKER_PLATFORM` for hosts that prefer native arm64.

## 0.3.20 - 2026-05-05

Changes since `0.3.19` on 2026-05-04.

### Added

- Local release pipeline now builds Linux x64 desktop artifacts in Docker: signed AppImage and Debian package alongside an `x86_64-unknown-linux-musl` static CLI tarball. Uses a new `Dockerfile.linux-release` (Tauri/GTK toolchain + Rust + musl-tools); requires only Docker on the host. Wired in as the `linux` release step alongside `macos` / `android` / `windows`.

### Removed

- The boilerplate "Linux release artifacts are not built by this host script unless run on Linux or extended with a working local cross toolchain." line that the script unconditionally appended to the skipped section of release notes from non-Linux hosts. With Docker doing the Linux build natively, the disclaimer is obsolete.

## 0.3.19 - 2026-05-04

Changes since `0.3.18` earlier on 2026-05-04.

### Fixed

- In-app updater install no longer fails with `manifest was not found at manifest.json`. The plugin's `check()` defaulted `manifest_path` to `"release.json"` (matching what `htree release publish` writes), but `download_and_install()` defaulted to `"manifest.json"` — so checks succeeded and installs failed. Worked around by pinning `manifest_path: "release.json"` in `tauri.conf.json`; the plugin-side default is also being fixed upstream.

## 0.3.18 - 2026-05-04

Changes since `0.3.17` earlier on 2026-05-04.

### Changed

- macOS release artifacts: now ship a signed + notarized + stapled `.dmg` (drag-to-Applications disk image) for first-install users, alongside the `.app.tar.gz` consumed by the in-app hashtree updater. Dropped the redundant `.zip` since `.dmg` covers the human-download case more idiomatically and `.app.tar.gz` covers the updater.
- SystemPanel "Updates" status messages (`You're up to date.`, `Installed …`, available-version line, install progress, errors) now render directly under the Check button — the spot the user just clicked — instead of below the auto-check / auto-install toggles. Up-to-date and Installed states render in green.

### Fixed

- "You're up to date." was rendering below the toggle rows in muted gray, making it look like unrelated body text. It now sits beside the Check button in the success color used elsewhere in the panel.

## 0.3.17 - 2026-05-04

Changes since `0.3.16` earlier on 2026-05-04.

### Fixed

- Update banner now actually appears when an update is available. The banner used to call a 6h-throttled auto-check on mount, so once the SystemPanel "Check for updates" button had run, every subsequent launch would silently skip the check and stay hidden. Banner and SystemPanel now share a single in-memory `latestUpdate` store, the launch-time check is unthrottled (still gated on the "Check for updates automatically" pref), and SystemPanel reflects the same state — so reopening the panel after a launch check shows the available version without forcing a manual re-check. The available-update payload itself is intentionally not persisted across launches; only `lastCheckMs` and `dismissedVersion` are.

## 0.3.16 - 2026-05-04

Changes since `0.3.15` earlier on 2026-05-04.

### Fixed

- Background Service panel no longer shows "Enable the service to keep VPN control out of the GUI process" when the service is already installed and running. The instruction text + helper line now only appear when something is actually actionable (install in flight, repair recommended, or first-time setup); the steady-state panel relies on the green Installed/Running/Daemon Reachable badges and the running-pid status line.

## 0.3.15 - 2026-05-04

Changes since `0.3.14` on 2026-04-29.

### Changed

- Tauri desktop app pulls `tauri-plugin-hashtree-updater` from crates.io (`0.2`) instead of a local path checkout, so release builds no longer need the `~/src/hashtree` clone alongside the repo.
- macOS release pipeline now produces a real `.app.tar.gz` (gzipped tar of the signed/notarized `.app`) alongside the existing `.zip`, which lets the in-app hashtree updater install AppBundle updates.

## 0.3.14 - 2026-04-29

Changes since `0.3.13` on 2026-04-20.

### Fixed

- Same-port NAT recovery now skips disruptive local-endpoint punching for stale non-exit peers once that peer already has an established WireGuard runtime path, avoiding avoidable macOS tunnel churn during otherwise healthy Screen Sharing sessions.
- macOS daemon service logs are compacted in-process at startup and periodically, capping growth while retaining a recent tail for debugging.
- Default daemon logging now suppresses high-volume internal relay-pool and WireGuard timer noise, and the hot-loop macOS peer-planning line was removed.

## 0.3.13 - 2026-04-20

Changes since `v0.3.12` on 2026-04-19.

### Fixed

- Sleep, wake, and network-move recovery now clears stale peer path state, refreshes public signal endpoints, and reconnects relays immediately so peers stop sticking to obsolete public ports after resume.
- Known peers now keep receiving handshake heartbeats and targeted private announce retries until a fresh WireGuard handshake lands, reducing multi-minute reconnect stalls after missed announces or daemon restarts.

## 0.3.12 - 2026-04-19

Changes since `v0.3.11` on 2026-04-19.

### Fixed

- GitHub's Ubuntu `clippy` lane now passes again after tightening the local Nostr relay test helper and relay operator rate-sampling code for current Rust lint behavior.
- Local release automation now runs Windows guest PowerShell via encoded commands and restores the tracked Android ACL manifest after builds, avoiding quoting breakage and dirty release worktrees.

## 0.3.11 - 2026-04-19

Changes since `v0.3.10` on 2026-04-15.

### Fixed

- Same-port NAT recovery now skips disruptive local-endpoint punching for stale non-exit routed peers once another mesh peer is healthy, so working direct traffic stays up while the selected exit peer still gets aggressive recovery.
- Linux systemd service units now write daemon logs with unquoted `append:` targets, which restores `StandardOutput` and `StandardError` log redirection for the supervised daemon.

## 0.3.10 - 2026-04-15

Changes since `v0.3.9` on 2026-04-09.

### Fixed

- Session reconnect logic now drops stale public signaling endpoints after a network change and reconnects relays so roaming between networks recovers cleanly instead of continuing to announce obsolete addresses.

## 0.3.9 - 2026-04-09

Changes since `v0.3.8` on 2026-04-08.

### Added

- A new `nvpn stats` CLI command for inspecting relay-operator state files in either human-readable or JSON form.
- A path-maintenance architecture note describing the staged move from disruptive same-port NAT recovery toward a more stable transport manager.

### Fixed

- Same-port NAT recovery on Unix now avoids disruptive punching for unrelated stale peers when the mesh already has another healthy peer, reducing unnecessary tunnel churn.

## 0.3.7 - 2026-04-08

Changes since `v0.3.6` on 2026-04-06.

### Fixed

- The desktop Diagnostics section now keeps your manual open or closed state across background refreshes, while still auto-opening if new health warnings appear.
- Background-service management controls now stay visible after initial setup, so reinstall, enable, disable, and uninstall actions remain reachable from the GUI.

## 0.3.6 - 2026-04-06

Changes since `v0.3.4` on 2026-04-02.

### Added

- A leaner invite bootstrap flow where QR codes carry only mesh/bootstrap metadata and the rest of the network state is fetched over signed Nostr roster updates.
- New Docker end-to-end coverage for NAT private-to-public reachability and selected-exit-node routing through a dedicated exit-node topology.

### Changed

- Shared participant aliases are now treated as roster state and republished by admins, so renames propagate across peers instead of remaining local-only.
- Desktop invite onboarding is split into a dedicated import panel, and the request-join flow now matches the backend’s automatic join-request behavior after invite import.
- Partial-mesh desktop status wording now favors explicit mesh counts over vague “connecting” copy, and service mismatch prompts show the exact app/service versions involved.

### Fixed

- Exit-node reconnects now recover from stale public endpoint state and still keep punching the selected exit peer when direct WireGuard paths need to be refreshed.
- macOS underlay repair now restarts the tunnel cleanly after network recovery instead of trying to reuse a broken in-memory tunnel handle.
- Desktop background-service timeout warnings now clear once the service actually recovers, instead of lingering after a successful reinstall or restart.
- Non-admin devices can no longer edit shared network identity fields in the GUI, reducing accidental roster drift and local/shared state confusion.
- Docker and Tauri end-to-end lanes now reflect the real join-request and mesh-ready UI states again, restoring full release-path coverage.

## 0.3.4 - 2026-04-02

Changes since `v0.3.3` on 2026-04-01.

### Added

- A new `nostr-vpn-web` HTTP API service plus `VITE_NVPN_API_BASE` frontend support, so the GUI can run against a web backend instead of only the Tauri desktop bridge.

### Fixed

- Desktop session toggles now keep the daemon as the source of truth for `VPN On` and `VPN Off`, while showing `VPN Starting` and `VPN Stopping` during in-flight control requests.
- Daemon pause and resume control requests are now polled every 100ms and persist their runtime state before slower disconnect and NAT refresh work, cutting the long on/off lag that could stretch to several seconds.
- The Docker Tauri driver wrapper now forwards `TAURI_E2E_SCENARIO`, so targeted end-to-end GUI checks run the requested scenario instead of silently defaulting to smoke coverage.

## 0.3.3 - 2026-04-01

Changes since `v0.3.2` on 2026-04-01.

### Fixed

- Relay fallback now uses the peer's active-session age for its direct-handshake grace period, so healthy periodic announces no longer prevent fallback from ever engaging.
- Runtime path caching now drops stale relay ingress endpoints when newer peer announcements stop advertising them, preventing clients from sticking to expired relay ports.
- GUI session toggles now flip immediately to the requested on/off state while the background service finishes the control request, making the desktop switch feel responsive again.
- Daemon pause and resume control now wait for the daemon's completion record instead of timing out on a short intermediate state window, avoiding false "background service did not respond in time" errors while shutting down.
- Daemon control requests now clear stale result files and log when pause, resume, or reload handling starts and completes, making stuck-control diagnosis easier in the bounded debug log.
- Docker relay-fallback end-to-end verification now passes again for the blocked-direct-UDP scenario, with both peers converging on the reachable relay ingress and completing tunnel traffic through it.

## 0.3.2 - 2026-04-01

Changes since `v0.3.1` on 2026-03-31.

### Changed

- Desktop advanced settings now expose the relay-routing toggle in both `Routing & Sharing` and `Session & Relays`, using clearer “Enable routing over relay when direct path fails” wording.
- Pending peer status in the desktop UI now reports when WireGuard is waiting on a relay endpoint instead of implying the direct endpoint is still in use.

### Fixed

- Relay routing no longer preempts a freshly selected direct path immediately; direct UDP now gets a retry window before relay routing takes over.
- Public relay fallback requests now wait for active peer presence and a short direct-handshake grace period instead of firing immediately on just-seen or stale peers.
- Disabling relay routing now drops cached relay endpoints from runtime path selection so the setting takes effect right away.
- Daemon and relay-operator debug logs are now trimmed to a bounded rolling tail instead of growing without limit.

## 0.3.1 - 2026-03-31

Changes since `v0.3.0` on 2026-03-31.

### Changed

- Signaling relays now stay connected so roster updates, exit-node capability changes, and other control-plane updates keep propagating after the mesh is established.
- Private exit-node announcements are refreshed to known peers after reconnects and reloads, reducing stale `Not offered` state after toggling the feature.
- Exit-node wording in the desktop UI and tray now explicitly describes the current mode as a private exit node, leaving room for a future public mode.

### Fixed

- Docker and Tauri coverage now reflects the keep-relays-connected policy instead of expecting relay pause after mesh completion.
- Legacy configs that still contain `auto_disconnect_relays_when_mesh_ready = true` are forced off on load and no longer reserialize that field.

## 0.3.0 - 2026-03-31

Changes since `v0.2.28` on 2026-03-26.

### Breaking Changes

- Invite format moved to version 2. `0.3.0` can still import v1 invites, but older builds that only understand invite v1 will not import invites generated by `0.3.0`.
- Admin-signed roster sync was added to the signaling protocol. Mixed-version peers can still connect at the base mesh layer, but older peers will not participate in the newer admin roster management model.

### Added

- Admin-managed network rosters shared over signed Nostr events, including admin promotion and removal.
- Invite payload support for network names, admin lists, and participant lists.
- Join requests addressed to all known network admins instead of a single inviter.
- Public relay services and relay failover support, including the `nvpn-udp-relay` binary and relay fallback end-to-end coverage.
- GUI service repair and auto-restart flows for updated or broken background services.
- Stable admin visibility in the desktop UI, including admin summaries, participant admin badges, and admin toggle actions.
- New end-to-end coverage for relay fallback, join-request admin propagation in Tauri, and three-peer roster/admin add-remove-rejoin flows in Docker.

### Changed

- The daemon now republishes and applies newer valid shared rosters across peers, with timestamp checks and existing-admin signature checks.
- Desktop startup now keeps background signaling alive when the local device is an admin listening for join requests, not only when autoconnect is enabled.
- The GUI now shows full peer npubs in more places and surfaces admin-specific network management state more clearly.
- Local and CI release paths now include newer signing and release automation updates, including Azure Trusted Signing fallback for Windows artifacts.

### Fixed

- Join-request handling between peers when the owner app was open only for admin/listener duties.
- Docker end-to-end coverage for roster mutations by fixing multiline TOML roster edits in the test harness.
- Several service reload and repair paths that previously required more manual recovery after config or binary changes.

## 0.2.28 - 2026-03-26

- Release `v0.2.28`.
