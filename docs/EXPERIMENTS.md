# Experiments

Running notes for nvpn/FIPS performance and reliability work. Keep entries
short enough to compare later: date, build/commit, setup, result, and decision.

## 2026-05-12 - macOS Wi-Fi to Ethernet, safe MTU

Setup:
- Local MacBook on Wi-Fi to Mac mini on Ethernet.
- FIPS core at `c7fb565` (`Revert "perf: parallelize fmp encryption with ordered send"`).
- Private mesh safe defaults: underlay UDP MTU 1280, tunnel MTU 1150.
- Both daemons built with local FIPS patches and ad-hoc signed.

Results:
- nvpn MacBook to mini UDP at 400 Mbit/s target: about 240 Mbit/s with near-zero loss.
- nvpn MacBook to mini TCP: about 200-223 Mbit/s depending on run.
- nvpn mini to MacBook TCP: about 345-356 Mbit/s.
- Tailscale MacBook to mini TCP at same time: about 292 Mbit/s.
- Tailscale MacBook to mini UDP at 400 Mbit/s target: reached about 400 Mbit/s but with about 3.6% loss.

Decision:
- Safe MTU is reliable but leaves LAN throughput on the table.
- Add an explicit LAN MTU/profile override (`mesh_mtu_profile = "lan"` or
  `NVPN_MESH_MTU_PROFILE=lan`) for controlled tests instead of making
  LAN-sized frames the global default.

## 2026-05-12 - fresh macOS Wi-Fi to Ethernet comparison

Setup:
- Local MacBook on Wi-Fi to Mac mini on Ethernet.
- Running daemons still at the stable safe-MTU build because launchd restart
  requires elevated `launchctl kickstart`.
- Direct LAN, Tailscale, and nvpn tested back-to-back with the same iperf3
  server.

Results:
- Direct LAN TCP: MacBook to mini about 495 Mbit/s; mini to MacBook about
  318 Mbit/s.
- Direct LAN UDP at 400 Mbit/s target: about 400 Mbit/s both directions with
  about 0.13% loss.
- Tailscale TCP: MacBook to mini about 299 Mbit/s; mini to MacBook about
  332 Mbit/s.
- Tailscale UDP at 400 Mbit/s target: about 400 Mbit/s both directions; loss
  about 0.05% forward and 3.5% reverse.
- nvpn safe-MTU TCP: MacBook to mini about 188 Mbit/s; mini to MacBook about
  323 Mbit/s.
- nvpn safe-MTU UDP at 400 Mbit/s target: MacBook to mini about 203 Mbit/s
  with about 5.1% loss; mini to MacBook about 393 Mbit/s with about 23.7% loss.

Observations:
- Direct LAN proves the path can carry 400 Mbit/s UDP and much higher forward
  TCP than nvpn currently achieves.
- Daemon logs show macOS `ENOBUFS` send backpressure during the UDP runs.
- The mini also had unrelated session AEAD recovery churn with another peer
  during the same window, so reliability work in that path may contaminate
  throughput samples until it is fixed.

Decision:
- The forward nvpn TCP gap is still worth fixing.
- Next live test is the explicit LAN MTU profile on both Macs after a privileged
  daemon restart. If it does not close most of the forward gap, the next target
  is macOS sender pacing/queueing rather than MTU.

## 2026-05-12 - explicit LAN MTU profile deployed

Setup:
- Local MacBook on Wi-Fi to Mac mini on Ethernet.
- nostr-vpn at `2509c9b` (`perf: add private mesh mtu test profile`).
- FIPS core at `c7fb565`.
- Both daemons built with local FIPS patches, ad-hoc signed, restarted through
  launchd, and configured with `mesh_mtu_profile = "lan"`.
- Live private mesh interface MTU was 1290 on both Macs.

15 second results:
- Direct LAN TCP: MacBook to mini about 499 Mbit/s; mini to MacBook about
  437 Mbit/s.
- Direct LAN UDP at 400 Mbit/s target: MacBook to mini about 397 Mbit/s with
  0% loss; mini to MacBook about 400 Mbit/s with about 1.3% loss.
- Tailscale TCP: MacBook to mini about 228 Mbit/s; mini to MacBook about
  314 Mbit/s, both with thousands of retransmits.
- Tailscale UDP at 400 Mbit/s target: MacBook to mini about 400 Mbit/s with
  about 2.3% loss; mini to MacBook about 400 Mbit/s with about 0.04% loss.
- nvpn LAN-MTU TCP: MacBook to mini about 228 Mbit/s; mini to MacBook about
  416 Mbit/s.
- nvpn LAN-MTU UDP at 400 Mbit/s target: MacBook to mini about 265 Mbit/s with
  near-zero loss; mini to MacBook about 400 Mbit/s with 0% loss.

90 second nvpn stability results:
- TCP MacBook to mini: about 234 Mbit/s, 2697 retransmits.
- TCP mini to MacBook: about 357 Mbit/s, 2626 retransmits.
- UDP MacBook to mini at 275 Mbit/s target: about 263 Mbit/s with about
  0.055% loss.
- UDP mini to MacBook at 400 Mbit/s target: about 400 Mbit/s with about 1.0%
  loss.

Observations:
- The LAN MTU profile makes nvpn competitive with or faster than Tailscale for
  TCP on this sample and improves reverse UDP to line rate.
- Forward UDP from the Wi-Fi MacBook remains capped around 260-265 Mbit/s
  before the daemon hits macOS UDP send pressure. Earlier logs showed
  `No buffer space available` and `EncryptWorker channel full` on this path.
- The remaining gap is directional and sender-side: mini to MacBook is much
  faster over the same private mesh protocol and same tunnel MTU.

Decision:
- Keep 1280/1150 as the safe default and use the explicit LAN profile for
  controlled LAN tests.
- Continue investigating macOS sender pacing/queueing and packet-rate reduction
  for the MacBook-to-mini direction. Avoid retry/drop variants because previous
  tests increased loss and hurt TCP.

Follow-up 1452/1322 explicit override:
- Setting `mesh_underlay_udp_mtu = 1452` and `mesh_tunnel_mtu = 1322`
  kept both utuns up and improved MacBook-to-mini UDP at 400 Mbit/s target
  slightly, to about 273 Mbit/s with near-zero loss.
- MacBook-to-mini TCP regressed to about 194 Mbit/s in the same sample.
- Decision: do not promote 1452/1322 to the `lan` profile yet. It may be a
  useful one-off UDP test override, but the 1420/1290 profile is the better
  balanced default for now.

## 2026-05-12 - Darwin `sendmsg_x` batch send trial

Setup:
- FIPS experiment `985e1ab` used Darwin's private `sendmsg_x` syscall for
  connected UDP sockets, falling back to per-datagram `send(2)` if refused.
- Both Macs were rebuilt and restarted on the LAN MTU profile.

Results:
- The focused loopback unit test passed, proving the syscall is available on
  this macOS build.
- Short samples were mixed: one MacBook-to-mini TCP run reached about
  247 Mbit/s, but UDP at 400 Mbit/s target fell to about 250 Mbit/s.
- A 90 second sample regressed every leg: MacBook-to-mini TCP about
  176 Mbit/s, mini-to-MacBook TCP about 122 Mbit/s, MacBook-to-mini UDP at
  275 Mbit/s target about 228 Mbit/s, and mini-to-MacBook UDP at 400 Mbit/s
  target about 289 Mbit/s.
- An adaptive fallback on partial/`ENOBUFS` batch sends did not recover the
  loss: forward TCP stayed near baseline and UDP at 275 Mbit/s lost packets.

Decision:
- Reverted in FIPS `e4edff2`. `sendmsg_x` reduces syscall count, but sustained
  Wi-Fi behavior is worse, likely because the kernel sees burstier UDP writes.
- Keep the conservative per-datagram macOS send loop until there is a pacing
  model that improves long runs, not just short TCP bursts.

## 2026-05-12 - mini disk-full interference

Setup:
- After repeated build/restart/iperf cycles, mini returned `iperf3` errors like
  `unable to create a new stream: No space left on device`.

Findings:
- The mini data volume had only about 116 MiB free.
- Generated Rust build artifacts were the main safe cleanup target, including
  about 20 GiB in `/Users/sirius/src/fips/target` and 6.4 GiB in
  `/Users/sirius/src/nostr-vpn/target`.
- Removing generated `target` directories restored about 32 GiB free.

Post-clean sanity:
- Direct LAN and Tailscale were healthy before cleanup, but iperf server
  creation and nvpn samples were unreliable while disk was full.
- After cleanup and reverting `sendmsg_x`, nvpn 10 second samples returned to
  expected ranges: MacBook-to-mini TCP about 203 Mbit/s, mini-to-MacBook TCP
  about 322 Mbit/s, MacBook-to-mini UDP at 275 Mbit/s target reached 275 Mbit/s
  with 0% loss, and mini-to-MacBook UDP at 400 Mbit/s target reached
  400 Mbit/s with about 1.1% loss.

Decision:
- Treat very low nvpn samples while the mini is disk-full as contaminated.
- Keep at least tens of GiB free on remote bench hosts before interpreting
  daemon or iperf behavior.

## 2026-05-12 - LAN-sized MTU defaults trial

Setup:
- Same macOS Wi-Fi to Ethernet path.
- Private mesh defaults temporarily raised to underlay UDP MTU 1420 and tunnel
  MTU 1290.

Results:
- MacBook to mini UDP at 400 Mbit/s target improved to roughly 272-292 Mbit/s.
- Mini to MacBook UDP at 400 Mbit/s target could reach roughly 400 Mbit/s with low loss.
- MacBook to mini TCP improved to roughly 232 Mbit/s.
- Mini to MacBook TCP was roughly 315-392 Mbit/s.

Decision:
- Useful on clean LAN paths, but too optimistic for NAT traversal and nested
  tunnels. Restore 1280/1150 safe defaults until blackhole-safe probing exists.

## 2026-05-12 - ordered parallel FMP sender

Setup:
- FIPS experiment `85858a2` added a WireGuard-like parallel encrypt stage and
  per-destination ordered sender.
- Unit tests and `cargo test -p fips-core --lib` passed.

Results:
- Live MacBook to mini UDP at 400 Mbit/s target regressed to about 238.5 Mbit/s.
- Live MacBook to mini TCP regressed to about 182 Mbit/s.

Decision:
- Reverted in FIPS `c7fb565`. The extra per-packet ordering/channel overhead
  cost more than the parallel encryption helped on this path.

## 2026-05-12 - macOS send backpressure variants

Setup:
- macOS connected UDP send path under Wi-Fi pressure.

Results:
- Yield/retry on `WouldBlock`, `ENOBUFS`, and `ENOMEM` gives conservative
  throughput with near-zero loss.
- A bounded retry/drop variant caused high UDP loss and worse TCP behavior.
- A fixed 10 us sleep reduced throughput further.

Decision:
- Keep retry/yield behavior for reliability. Throughput work should focus on
  reducing per-packet work and using an explicit larger MTU on paths that can
  carry it, not dropping on macOS socket pressure.

## 2026-05-12 - nvpn mesh packet copies and utun write pressure

Setup:
- Local MacBook on Wi-Fi to Mac mini on Ethernet.
- LAN MTU profile on both Macs: underlay UDP MTU 1420, tunnel MTU 1290.
- FIPS core at `e4edff2` (Darwin `sendmsg_x` reverted).
- Both daemons built with local FIPS patches, ad-hoc signed, and restarted
  through launchd.

Results:
- `266595b` moved outbound FIPS mesh packets instead of cloning them. Best 15s
  samples after deploying to both Macs: MacBook-to-mini TCP about 255 Mbit/s,
  mini-to-MacBook TCP about 465 Mbit/s, MacBook-to-mini UDP at 275 Mbit/s target
  0% loss, MacBook-to-mini UDP at 400 Mbit/s target about 283 Mbit/s with near
  zero loss, and mini-to-MacBook UDP at 400 Mbit/s target about 400 Mbit/s with
  near-zero loss.
- A direct TUN-read-to-FIPS-send experiment removed the channel between TUN read
  and mesh send, but regressed MacBook-to-mini TCP to about 110 Mbit/s and added
  UDP loss. Decision: keep the channel decoupling.
- Long reverse UDP exposed silent utun write drops. Before the write fix, a 90s
  mini-to-MacBook UDP400 run lost about 19%; a 30s reproduction lost about 50%.
  Direct LAN and Tailscale over the same path sustained UDP400 with low loss
  (direct 90s reverse about 0.056% loss; Tailscale 90s reverse about 0.14%).
- `c00edda` adds raw TUN writes that wait for fd writability and retry
  `WouldBlock`, instead of using boringtun's `write4/write6` helpers that return
  `0` for every write error. Final targeted samples: 30s mini-to-MacBook UDP400
  about 400 Mbit/s with about 1.1% loss; 30s MacBook-to-mini UDP275 about
  275 Mbit/s with 0% loss.
- Full 90s no-drain/write-backpressure sample: MacBook-to-mini TCP about
  250 Mbit/s, mini-to-MacBook TCP about 401 Mbit/s, MacBook-to-mini UDP400 about
  259 Mbit/s with about 0.086% loss, and mini-to-MacBook UDP400 about
  400 Mbit/s with variable loss (observed 0.004% to 7.6% across repeated runs).
- A receive `try_recv` burst-drain experiment made reverse UDP400 recover from
  catastrophic loss but hurt forward TCP/UDP and made the receive side too
  bursty. A separate bounded TUN-write queue also worsened 350-400 Mbit/s reverse
  UDP loss. Decision: do not keep either sub-experiment.
- Mini daemon rejoin smoke after `launchctl kickstart -k`: local status already
  showed the direct UDP path on the first poll, and a follow-up `ping -c 3`
  over nvpn had 0% loss with about 7 ms average RTT. This is a smoke test only;
  it does not cover Wi-Fi roaming.

Observations:
- The remaining MacBook-to-mini UDP400 ceiling is still around 260-280 Mbit/s;
  this is the macOS/Wi-Fi sender side and still trails Tailscale's current
  UDP400 result on this LAN.
- Reverse UDP400 no longer collapses deterministically, but it is still less
  stable than Tailscale/direct LAN over 90s. The remaining loss is not explained
  by the rekey message counter: FIPS defaults to `node.rekey.after_messages =
  2^48`; time-based rekey defaults to 120 seconds, so 90s runs do not normally
  cross the periodic rekey timer.
- Local daemon CPU during reverse UDP400 receive was about 70-74%, so the
  current reverse loss does not look like a single pegged userspace CPU core.

Decision:
- Keep owned packet movement and raw TUN write backpressure.
- Continue investigating the residual reverse UDP400 variability and the
  MacBook sender-side UDP400 ceiling before claiming parity with Tailscale.

## 2026-05-12 - pipeline trace and connected UDP activation

Setup:
- Local MacBook on Wi-Fi to Mac mini on Ethernet.
- Both daemons built with `NVPN_PIPELINE_TRACE_DEFAULT=1` and
  `FIPS_PIPELINE_TRACE_DEFAULT=1`.
- FIPS added queue-wait counters for endpoint command, FMP worker, transport,
  endpoint event, connected-vs-wildcard UDP sends, and connected UDP activation.
- Compared current nvpn against same-window Tailscale over the same machines.

Results:
- Connected UDP was not reliably active for NAT-traversal sockets until the
  traversal socket was bound with `SO_REUSEADDR`/`SO_REUSEPORT` before FIPS
  adopted it. After the reuse fix, both app-resource and `~/.cargo/bin/nvpn`
  binaries showed `connected_udp_installed`, then steady `udp_send_connected`
  traffic.
- Current 20s TCP samples after the connected-UDP fix:
  nvpn MacBook-to-mini about 227 Mbit/s receiver, nvpn mini-to-MacBook about
  350 Mbit/s receiver; same-window Tailscale was about 268 Mbit/s forward and
  348 Mbit/s reverse.
- MacBook-to-mini forward traces still show sender-side Darwin UDP pressure:
  one 5s interval saw about `24k/s` FMP worker sends, about `15.7k/s` successful
  UDP send calls, and about `91k/s` `udp_send_backpressure` events with
  `fmp_worker_queue_wait` p95 near 134 ms.
- Reverse mini-to-MacBook traces were much cleaner, with high connected send
  rates and little or no backpressure, matching the throughput result.
- Queue-cap trials: 256 was too shallow and hurt reverse throughput; 32768 hid
  saturation and inflated latency/retransmits; 1024 is the best known balance
  for pushing back toward TUN without building a large userspace buffer.

Observations:
- The remaining MacBook-to-mini gap is not from crypto and not from the
  connected-UDP fast path being absent. `sample(1)` on the MacBook sender spent
  most active worker time in `sendto`, with ChaCha20-Poly1305 a secondary cost.
- Wireguard-go and boringtun do not expose an obvious Darwin send primitive that
  we are missing: non-Linux wireguard-go uses single-packet `WriteMsgUDP`, and
  boringtun uses a plain utun fd path. Tailscale's utun shows Darwin
  offload/channel flags (`TSO4`, `TSO6`, `CHANNEL_IO`, checksum offloads) that
  this daemon's boringtun-created utun does not.
- Linux does not show the same problem because the Linux path can use UDP GSO
  and TUN offloads/batching; Darwin currently has neither in this stack.

Decision:
- Keep the connected UDP reuse fix and macOS stale-socket clearing.
- Keep the 1024 encrypt-worker queue cap.
- Do not revive Darwin `sendmsg_x`, fixed 10 us sleep, bounded retry/drop, or
  256-queue experiments; all regressed sustained runs.
- Next focused experiment is a macOS-only adaptive pause after repeated
  `ENOBUFS` bursts (`FIPS_SEND_BACKPRESSURE_SLEEP_AFTER`,
  `FIPS_SEND_BACKPRESSURE_SLEEP_MICROS`) to reduce spin-retry storms without
  sleeping on clean sends. Long-term parity may require a Darwin utun
  offload/channel backend rather than more UDP send-loop tuning.

## 2026-05-12 - adaptive ENOBUFS pause and ordered macOS sender v2

Setup:
- Same MacBook Wi-Fi to Mac mini Ethernet path, LAN MTU profile, connected UDP
  active, pipeline tracing compiled on.
- First deployed adaptive macOS send pacing: retry/yield remains the default,
  but after four consecutive `ENOBUFS`/`ENOMEM` results the sender sleeps for
  1 us before retrying.

Results:
- 20s same-window TCP after adaptive pacing: nvpn MacBook-to-mini about
  248 Mbit/s receiver with 0 retransmits; nvpn mini-to-MacBook about
  409 Mbit/s receiver with 2117 retransmits. Tailscale in the same window was
  about 301 Mbit/s forward and 355 Mbit/s reverse.
- Local sender traces confirmed the pause triggers under load: representative
  5s intervals showed about `13k-16k/s` successful UDP sends, about
  `12k-18k/s` backpressure events, and about `3k-4.5k/s` micro-sleeps. This is
  much better than the earlier `80k-90k/s` retry storm, but still leaves large
  FMP worker queue waits.

Follow-up experiment:
- FIPS now has a macOS-only ordered sender v2: rx_loop still assigns counters
  sequentially, FMP encryption is spread across workers, and completed packets
  are serialized by a per-socket/per-destination sender thread. Linux keeps the
  existing GSO/sendmmsg path.
- This is different from the reverted `85858a2` experiment: the new version is
  only for macOS and only moves the FMP encrypt/send boundary, specifically to
  avoid making the Darwin UDP sender do AEAD work between kernel send attempts.

Decision:
- Adaptive pacing is worth keeping unless the ordered sender v2 shows a clear
  regression.
- The ordered sender v2 still needs deployment and live TCP/UDP comparison
  before it can replace the current macOS path.

## 2026-05-13 - macOS sender wake and receive hot-path cleanup

Setup:
- Same MacBook Wi-Fi to Mac mini Ethernet path, LAN MTU profile, connected UDP
  active. Pipeline tracing defaults are now runtime-env only so normal benches
  are trace-off unless explicitly enabled.
- Both launchd daemons were rebuilt from the same temporary FIPS ref, copied into
  `~/.cargo/bin/nvpn` and the app resource copy, ad-hoc signed, then restarted
  with `launchctl kickstart`.

Results:
- Trace-off plus corrected ENOBUFS drop accounting: nvpn about 221 Mbit/s
  MacBook-to-mini and about 249 Mbit/s mini-to-MacBook. Same-window Tailscale was
  about 290/359 Mbit/s; raw LAN was about 540/415 Mbit/s.
- macOS custom worker queue removed the earlier crossbeam
  `semaphore_signal_trap` hotspot. 20s TCP was about 205/348 Mbit/s; same-window
  Tailscale about 298/354 Mbit/s. `sample(1)` then showed the ordered sender
  completion condvar as the next sender-side cost.
- Splitting the ordered sender condvars gave about 215/428 Mbit/s in one clean
  window. Reverse beat the same-window Tailscale sample, but MacBook-to-mini
  still trailed. Sender samples were mostly Darwin UDP `sendto` plus completion
  wakeups; receiver samples still showed `npub_for_node_addr`/`encode_npub` on
  each delivered endpoint packet.
- Caching encoded npubs in the FIPS identity cache removed that receiver
  hot-path encode. It did not solve the sender bottleneck: one noisy window was
  nvpn 136/322 Mbit/s vs Tailscale 310/399 Mbit/s, while raw LAN was still
  healthy at 622/516 Mbit/s.
- Disabling the ordered sender and using the simpler hash-by-send-target worker
  path was not better: about 186/336 Mbit/s vs same-window Tailscale 263/380
  Mbit/s. Keep it only as `FIPS_MACOS_ORDERED_SENDER=0` for comparison.
- Batching ordered-sender completions by worker batch kept the better ordered
  behavior and reduced clean-run retransmits: about 215/307 Mbit/s vs
  same-window Tailscale 258/342 Mbit/s. Profiling still showed sender wakeups
  because the high worker count means many batches contain only one packet.
- Capping the default macOS encrypt pool to four workers was rejected: forward
  improved slightly to about 225 Mbit/s, but reverse collapsed to about
  198 Mbit/s while Tailscale was about 283/356 Mbit/s in the same window.
- Making the macOS worker queue signal only when its worker is actually parked
  is a keeper. Same-window 20s TCP improved to about 252/384 Mbit/s vs
  Tailscale 329/390 Mbit/s.
- Skipping endpoint identity registration on already-established sessions
  removed the remaining per-packet identity-cache work from the steady sender
  path. Repeated MacBook-to-mini samples remained variable at about
  232-255 Mbit/s, so this was correctness/CPU cleanup rather than a sender
  ceiling fix.
- Moving established endpoint FSP encryption into the existing FMP worker job
  kept wire format and nonce ordering but removed the inner ChaCha20-Poly1305
  seal from the rx-loop task. Final default-stride 20s sample: nvpn about
  232/426 Mbit/s vs same-window Tailscale about 323/369 Mbit/s. Final 90s nvpn
  stability runs stayed up: about 223 Mbit/s MacBook-to-mini and about
  391 Mbit/s mini-to-MacBook; both meshes still showed the tested peer reachable
  afterward.
- `FIPS_MACOS_WORKER_STRIDE=4` as a compiled default was rejected twice. Before
  the FSP worker it slightly helped reverse but did not help the weak direction;
  after the FSP worker it collapsed reverse throughput to about 198 Mbit/s.

Decision:
- Keep runtime-only tracing, fixed ENOBUFS drop accounting, macOS custom worker
  queue, parked-worker-only queue signalling, ordered sender as the default,
  batched ordered completions, encoded-npub identity cache entries, established
  endpoint identity-registration skip, endpoint FSP worker preseal, and idle
  pruning for stale macOS send flows.
- Do not cap macOS encrypt workers by default; use `FIPS_ENCRYPT_WORKERS=N` only
  for experiments.
- Keep `FIPS_MACOS_WORKER_STRIDE` only as an experiment knob; the compiled
  default remains 1.
- The current MacBook-to-mini gap is still the macOS sender side, not LAN
  capacity, not receive-side npub encode, and no longer the inner FSP AEAD.
  Next likely wins are a lower-wakeup dispatch/completion design, less frequent
  diagnostics under data-plane load, or a Darwin utun/channel/offload backend
  comparable to what Tailscale appears to get from its interface.

## 2026-05-13 - Darwin sender mode and socket option retest

Setup:
- Same macOS Wi-Fi sender to Ethernet mini receiver path with the LAN MTU
  profile still enabled.
- Connected UDP is disabled by default on macOS. Earlier per-peer connected
  sockets improved the syscall shape on Linux, but on Darwin they caused direct
  FMP liveness/fallback trouble under load; `netstat` should show only the
  wildcard UDP socket on macOS unless an explicit env override is being tested.
- For launchd env-var A/Bs, `kickstart` alone was not enough: the loaded job kept
  the old plist environment. Reliable env tests require editing
  `EnvironmentVariables`, then `launchctl bootout` + `bootstrap`.

Results:
- Clean two-sided default restart with connected UDP off and ordered sender on:
  nvpn about 103-109 Mbit/s MacBook-to-mini and about 317 Mbit/s
  mini-to-MacBook. Same-window Tailscale was about 251/355 Mbit/s.
- `FIPS_PERF=1` on the sender showed the active path was `udp_send_wildcard`,
  not connected UDP, with no `udp_send_backpressure` events. Representative
  intervals were `udp_send_wildcard` about 10k-12k/s, `udp_send` average
  roughly 32-40 us, FMP encrypt average roughly 10-12 us, and worker/endpoint
  queue waits in the hundreds of microseconds.
- Properly reloaded `FIPS_MACOS_ORDERED_SENDER=0` improved the weak direction to
  about 146.8 Mbit/s with fewer retransmits, while reverse stayed strong at
  about 349.5 Mbit/s. This makes the simpler hash-by-send-target worker path the
  better Darwin default for this workload.
- `FIPS_MACOS_NET_SERVICE_TYPE=vi` on top of the simpler sender regressed the
  weak direction to about 109.9 Mbit/s. Keep Darwin service-class sockopts
  opt-in only.
- `FIPS_MACOS_WORKER_BATCH=8` on both Macs was worse than the default worker
  drain batch: about 126.6 Mbit/s MacBook-to-mini and about 277.0 Mbit/s
  mini-to-MacBook, with more retransmits in the weak direction. Default batch
  32 in the same build reached about 157-160 Mbit/s MacBook-to-mini in short
  samples.
- `FIPS_MACOS_WORKER_BATCH=64` was rejected without a throughput run because the
  Mac-to-mini link stayed relayed for more than two discovery/rekey intervals
  after the two-sided launchd restart. Default batch 32 recovered direct again.
- On this machine Tailscale's utun does show `TSO4`, `TSO6`, `CHANNEL_IO`, and
  checksum offload flags while the boringtun-created nvpn utun does not. Stock
  wireguard-go and boringtun do not enable those flags through the plain utun fd
  path, so Tailscale likely has a Darwin interface/backend advantage outside the
  UDP crypto protocol itself.

Decision:
- Default macOS FMP sending to the hash-by-send-target worker path; keep
  `FIPS_MACOS_ORDERED_SENDER=1` as an experiment for AEAD-bound cases.
- Keep connected UDP default-on for Linux and default-off for macOS.
- Keep `FIPS_MACOS_WORKER_BATCH` as an experiment knob with default 32; smaller
  batches hurt this Wi-Fi/Ethernet path and 64 exposed restart/churn fragility.
- Do not promote `SO_NET_SERVICE_TYPE` on Darwin without a fresh path-specific
  win; it can make Wi-Fi sender pacing worse.
- The remaining forward gap is still sender-side packet-rate efficiency on
  Darwin plus a likely utun backend/offload gap versus Tailscale. `kqueue`
  write-ready handling is not the first fix while ENOBUFS is absent. The
  realistic next steps are lower-handoff sending, safer per-path interface/socket
  specialization, a Darwin utun backend that can match Tailscale's channel/offload
  flags, or FIPS-level packet coalescing, which would be a wire-format change.

## 2026-05-13 - mini Docker nvpn versus boringtun

Setup:
- Host: `Siriuss-Mac-mini`.
- nostr-vpn at `cc8e603` on `codex/nvpn-perf-test`; FIPS sibling checkout at
  `e036c0e` on `master`.
- Docker PATH for non-interactive shell:
  `export PATH=/usr/local/bin:/Applications/Docker.app/Contents/Resources/bin:$HOME/.docker/cli-plugins:/Applications/Docker.app/Contents/Resources/cli-plugins:$PATH`.
- Commands:
  `DURATION=10 PROJECT_NAME=nvpn-perf-mini scripts/perf-docker.sh`
  and
  `DURATION=10 PROJECT_NAME=nvpn-boringtun-mini WG_THREADS_LIST='1 4' scripts/perf-docker-boringtun.sh`.
- The e2e Docker image patched embedded FIPS to the sibling BuildKit context.

Results:
- nvpn TCP: single stream 2822 Mbit/s receiver, 4 streams 2941 Mbit/s receiver,
  8 streams 3000 Mbit/s receiver.
- nvpn UDP: 200 Mbit/s target delivered 200 Mbit/s with 0% loss; 1000 Mbit/s
  target delivered 1000 Mbit/s with 0.035% loss.
- nvpn ping: 300/300 packets, avg 0.857 ms.
- boringtun `WG_THREADS=1` TCP: single stream 3340 Mbit/s receiver, 4 streams
  3336 Mbit/s receiver, 8 streams 3350 Mbit/s receiver.
- boringtun `WG_THREADS=1` UDP: 200 Mbit/s target delivered 200 Mbit/s with
  0.24% loss; 1000 Mbit/s target delivered 988 Mbit/s with 1.2% loss.
- boringtun `WG_THREADS=1` ping: 300/300 packets, avg 0.489 ms.
- boringtun `WG_THREADS=4` regressed badly: TCP receiver throughput was
  1483/1359/1229 Mbit/s for 1/4/8 streams. UDP was near target, with 0.11%
  loss at 200 Mbit/s and 0.3% loss at 1000 Mbit/s.

Profiling:
- `scripts/perf-docker-cpu.sh` with `DURATION=20 PROJECT_NAME=nvpn-cpu-mini`
  showed nvpn using about 193-198% CPU on node-a and about 267-273% CPU on
  node-b during a 2794 Mbit/s single-stream TCP run.
- `FIPS_PERF=1 NVPN_PIPELINE_TRACE=1` on a kept Docker mesh showed the sender
  TUN read/write stages were not the bottleneck. Representative sender intervals:
  TUN read about 310k/s at about 0.4 us avg, TUN-to-mesh queue wait about
  56-60 us avg, endpoint command wait about 67-72 us avg, and FMP worker queue
  wait about 300-350 us avg. The hot path is the ordered single-destination
  FIPS send pipeline and queueing behind it.

Rejected variants:
- FIPS Linux encrypt worker batch 32 -> 64 preserved wire format but collapsed
  TCP to about 1094 Mbit/s receiver with huge retransmits and made UDP lossy
  (about 4.9% loss at 200 Mbit/s and 49% loss at 1000 Mbit/s). Reverted.
- nvpn TUN-to-mesh send drain 64 -> 32 did not improve TCP materially
  (2831/2930/2998 Mbit/s receiver for 1/4/8 streams) and worsened UDP loss
  versus baseline (0.11% at 200 Mbit/s, 0.33% at 1000 Mbit/s). Reverted.

Decision:
- Keep the FIPS protocol wire format unchanged.
- Do not increase Linux FIPS worker batch size; larger bursts trade a small
  syscall win for severe TCP retransmits and UDP loss in Docker.
- The fair TCP gap versus boringtun `WG_THREADS=1` remains, but nvpn is already
  better on the UDP loss samples. Next Linux/FIPS work should reduce the
  single-destination ordered send queue cost without making bursts larger,
  likely by removing a handoff or making the ordered worker cheaper rather than
  by coalescing packets.

## 2026-05-13 - macOS connected UDP default and sender profiling

Setup:
- Same MacBook Wi-Fi sender to Ethernet mini receiver path, using the
  direct LAN endpoint between the Macs.
- FIPS commits tested and pushed to the mini checkout:
  `090e241` removed unnecessary Darwin wildcard reuse when connected UDP is
  disabled, `2d18ab7` enabled connected UDP by default on macOS, and
  `3d566c7` aligned the Darwin listener reuse default so connected siblings can
  actually bind beside the live listener.
- Both daemons were rebuilt from the local FIPS path, ad-hoc signed, installed
  as `~/.cargo/bin/nvpn`, and restarted through launchd. Final cleanup restored
  a clean launchd environment with only `OSLogRateLimit` plus launchd's own
  service variables.

Results:
- Fresh same-window TCP after connected UDP default-on: nvpn MacBook-to-mini
  about 256 Mbit/s, Tailscale MacBook-to-mini about 341 Mbit/s; nvpn
  mini-to-MacBook about 404 Mbit/s, Tailscale mini-to-MacBook about
  379 Mbit/s. Reverse can now match or beat Tailscale; the remaining gap is the
  MacBook Wi-Fi sender direction.
- Later samples varied with Wi-Fi conditions. One noisy post-restart window was
  nvpn about 169-181 Mbit/s while Tailscale was about 272-328 Mbit/s; direct LAN
  in the same period still reached about 559 Mbit/s MacBook-to-mini and about
  421 Mbit/s mini-to-MacBook.
- Final clean-env 10s sanity after removing all A/B launchd variables: nvpn
  about 174/351 Mbit/s and same-window Tailscale about 232/349 Mbit/s.
- MTU-safe UDP payloads (`iperf3 -u -l 1200`) showed the same directional cap:
  nvpn MacBook-to-mini about 223 Mbit/s with near-zero loss, while Tailscale
  could carry a 300 Mbit/s target with low loss. This points at packet-rate /
  sender path efficiency, not crypto correctness or MTU blackholing.
- Runtime tracing on the MacBook sender showed nostr-vpn's TUN-to-mesh handoff
  was small: TUN-to-mesh queue wait mostly single-digit microseconds and mesh
  send about 2 us per packet. FIPS steady-state intervals were dominated by the
  single peer worker doing FMP/FSP AEAD plus connected UDP send: roughly
  19k-20k connected UDP sends/s under tracing, FMP encrypt about 15 us avg,
  UDP send about 30 us avg, and FMP worker queue wait usually sub-millisecond.

Rejected A/Bs:
- `FIPS_MACOS_ORDERED_SENDER=1` default stride collapsed to about 8 Mbit/s.
  `FIPS_MACOS_WORKER_STRIDE=16` recovered to about 212 Mbit/s but added hundreds
  of retransmits and still did not beat the direct worker path.
- `FIPS_MACOS_WORKER_BATCH=1` fell to about 161 Mbit/s; batch 64 fell to about
  115 Mbit/s with retransmits. Keep the default batch 32.
- Darwin `SO_NET_SERVICE_TYPE` remains opt-in; earlier service-class tests
  regressed this Wi-Fi sender path.
- Backpressure drop tuning is inconclusive. Drop-after-16 improved one noisy
  sample to about 215 Mbit/s with no retransmits, and drop-after-1 once reached
  about 226 Mbit/s with normal retransmits, but a repeat drop-after-1 run fell
  to about 168 Mbit/s with heavy retransmits while same-window Tailscale reached
  about 328 Mbit/s. Do not change the compiled default without better
  ENOBUFS/drop instrumentation.

Decision:
- Keep connected UDP default-on for macOS now that listener/peer reuse flags are
  aligned; it is the only small socket change that repeatedly improved the weak
  direction.
- Keep the direct hash-by-send-target worker as the macOS default. The existing
  ordered sender is useful as a reference experiment but is too expensive in its
  current BTreeMap/condvar form.
- The remaining gap is still Darwin Wi-Fi sender packet-rate efficiency. The
  next high-leverage implementation work should be a cheaper WireGuard-like
  route/nonce -> parallel encrypt -> ordered send design, or packet coalescing
  if the protocol can grow an option bit. Avoid more MTU/service-class/batch
  guessing until the instrumentation can count ENOBUFS, dropped bulk packets,
  and per-stage queue waits in the same run.

## Bench commands

Use both directions and record both TCP and UDP:

```sh
iperf3 -c <peer-nvpn-ip> -t 15 --json
iperf3 -c <peer-nvpn-ip> -t 15 -R --json
iperf3 -u -b 400M -c <peer-nvpn-ip> -t 15 --json
iperf3 -u -b 400M -c <peer-nvpn-ip> -t 15 -R --json
```

Compare with Tailscale on the same machines and with boringtun/wireguard-go
where available. Once short runs look good, repeat with 90 second TCP/UDP runs
and churn tests: daemon restart, peer rejoin, and network roaming.
