# Changelog

All notable changes to this project are documented in this file.

## Unreleased

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
