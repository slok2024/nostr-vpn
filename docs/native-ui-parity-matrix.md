# Native UI Parity Matrix

This is the working checklist for the Rust-core, native-front architecture
similar to `~/src/iris-chat-rs`. The legacy Svelte/Tauri app has been removed;
its feature inventory remains here only as the migration contract.

The goal is not visual sameness. The goal is feature and behavior parity across
native shells while keeping product truth in Rust.

## Target Shape

| Layer | macOS | Windows | Linux | Android | iPhone |
| --- | --- | --- | --- | --- | --- |
| Shared core | Rust app core exposed through UniFFI | Rust app core exposed through UniFFI | Rust app core used directly or through UniFFI | Rust app core exposed through UniFFI | Rust app core exposed through UniFFI |
| Native shell | SwiftUI/AppKit | WPF/.NET | GTK4/libadwaita Rust | Kotlin/Jetpack Compose | SwiftUI/UIKit |
| App state owner | Rust | Rust | Rust | Rust | Rust |
| Rendering owner | Native | Native | Native | Native | Native |
| Secure/platform effects | Keychain, launch agent, status item | Credential Manager, service/UAC, tray | Secret Service fallback, desktop entry, tray/status notifier | Keystore, VpnService, camera/share intents | Keychain, NetworkExtension, camera/share sheet |
| VPN control model | Background service + FIPS private mesh | Windows service + FIPS private mesh | Background service + tun/FIPS private mesh | Android VpnService runtime | NetworkExtension Packet Tunnel |
| Package target | `.app`/DMG or signed archive | Installer/MSIX or NSIS | AppImage/deb/rpm later | APK/AAB/Zapstore | TestFlight/App Store |

## Rust Core Boundary

| Area | Core responsibility | Native responsibility |
| --- | --- | --- |
| State projection | `UiState`, networks, participants, relays, diagnostics, service status, mobile capability flags | Render state with platform controls and local presentation state |
| Actions | Existing product commands as typed Rust actions | Dispatch actions, disable conflicting controls while actions run |
| Long-running runtime | Daemon/session lifecycle, config persistence, relay status, peer status, LAN pairing, join requests | Keep app alive enough for platform lifecycle and show system-level affordances |
| Formatting | Shared user-facing derived labels that encode policy, like mesh readiness, join request status, exit-node availability, service repair recommendation | Platform typography, layout, control affordances |
| Platform effects | Declare requested effect and update state after completion | Clipboard, startup registration, tray/status item, camera QR scan, update installer, mobile VPN permission prompts |
| Errors | Stable action errors and recoverable service repair hints | Dialog/toast/sheet presentation |

## Feature Parity Matrix

Legend:

- `Required`: must ship on that platform.
- `Desktop`: desktop-only parity.
- `Mobile`: mobile-only equivalent.
- `Hidden`: code exists today but is not mounted in the current app.
- `N/A`: intentionally not applicable.

| Current feature | Current source | Core/API need | macOS | Windows | Linux | Android | iPhone | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| One snapshot app model | `UiState`, `get_state`, `tick` | `FfiApp.state()`, periodic or push updates, typed actions | Required | Required | Required | Required | Required | Keep a single state/action contract for all shells. |
| Initial boot sequencing | `AppBootstrap.svelte` | Start core, load config, first tick, ready event for tests | Required | Required | Required | Required | Required | Native tests need a replacement for `nvpn:boot-ready`. |
| Periodic refresh | `tick` every 1500ms | Prefer push updates; retain tick as fallback | Required | Required | Required | Required | Required | Mobile should avoid aggressive background polling. |
| Action lock/error recovery | `runAction`, action flags | Action in-flight state or shell-side lock | Required | Required | Required | Required | Required | Prevent overlapping config/session mutations. |
| Main status hero | `HeroStatusPanel.svelte` | Hero badge/subtext/detail helpers, active network projection | Required | Required | Required | Required | Required | Includes active network title, admin badge, mesh readiness, daemon/VPN/FIPS badges. |
| VPN on/off switch | `connect_session`, `disconnect_session` | Start/stop session action, service setup guidance | Required | Required | Required | Mobile | Mobile | Mobile uses platform VPN permission/control flow instead of desktop service. |
| Privacy disclosure | `shouldShowVpnDataDisclosure` | Capability/state flag and disclosure text | Required | Required | Required | Required | Required | Current copy should become a shared string or policy doc reference. |
| Own npub display/copy | `HeroStatusPanel.svelte` | `own_npub` in state | Required | Required | Required | Required | Required | Clipboard is native platform effect. |
| Device name editing | `update_settings.nodeName` | Typed settings patch | Required | Required | Required | Required | Required | Debounced edit, DNS-safe preview. |
| Device endpoint/tunnel summary | `UiState.endpoint`, `tunnelIp` | State fields | Required | Required | Required | Required | Required | Mobile may show platform-managed tunnel info. |
| Active network profile | `ActiveNetworkPanel.svelte` | Network name, mesh ID, local admin flag | Required | Required | Required | Required | Required | Non-admins must not edit shared network identity. |
| Mesh ID editing/validation | `mesh-id.js`, `set_network_mesh_id` | Move validation/canonicalization into Rust | Required | Required | Required | Required | Required | Current 5s idle commit plus blur/Enter commit should be preserved. |
| Mesh ID copy | `copyMeshId` | Current active network ID | Required | Required | Required | Required | Required | Copy raw canonical ID, not display grouping. |
| Network admin visibility | `networkAdminSummary`, badges | Admin summary and participant admin flags | Required | Required | Required | Required | Required | Keep admin-specific disabled states. |
| Join request listener toggle | `set_network_join_requests_enabled` | Per-network listener setting | Required | Required | Required | Required | Required | Works for active and saved networks. |
| Inbound join request list | `inboundJoinRequests` | Pending request state and accept action | Required | Required | Required | Required | Required | Accept action must remain admin-gated. |
| Outbound join request status | `outboundJoinRequest` | Request state and requested-at text | Required | Required | Required | Required | Required | Includes imported-from inviter and connected state. |
| Request join action | `request_network_join` | Action by network ID | Required | Required | Required | Required | Required | Deep links can also trigger this in test/debug flows. |
| Accept join action | `accept_join_request` | Action by network ID + requester npub | Required | Required | Required | Required | Required | Must persist acceptance even if session start fails. |
| Invite generation | `activeNetworkInvite` | Core-generated invite string | Required | Required | Required | Required | Required | Include mesh ID, inviter npub, admins, participants, relays. |
| Invite copy | `copyInvite` | Invite string in state | Required | Required | Required | Required | Required | Native share sheet can supplement copy on mobile. |
| Invite QR generation | `qrcode` in `InviteShareSection` | Prefer core QR bitmap/SVG helper or native QR library | Required | Required | Required | Required | Required | Must match current invite payload exactly. |
| Invite paste/import | `import_network_invite` | Action with parsed invite result | Required | Required | Required | Required | Required | Current auto-import after 250ms should be reconsidered for native UX but behavior must be covered. |
| Invite QR live scan | `jsQR`, `getUserMedia` | Native camera scanner effect, core import action | Required | Required | Required | Required | Required | Desktop platforms can use webcam when available. |
| Invite QR image scan | file input + `jsQR` | Native file/image picker + decoder | Required | Required | Required | Required | Required | Keep image fallback when camera is denied/unavailable. |
| Invite import confirmation | `window.confirm` with target mode | Core should expose parsed invite + import target | Required | Required | Required | Required | Required | Native alert/sheet; Cancel fills field instead of importing. |
| Auto-connect after invite import | `ensureSessionActiveAfterInviteImport` | Import action result plus session capability state | Required | Required | Required | Required | Required | On mobile this may require VPN permission prompt. |
| Manual add participant | `add_participant` | Add participant with optional alias | Required | Required | Required | Required | Required | Admin-gated. |
| Participant alias editing | `set_participant_alias` | Alias action and MagicDNS suffix | Required | Required | Required | Required | Required | Debounced, admin-gated. |
| Participant npub copy | participant rows | Participant npub in state | Required | Required | Required | Required | Required | Present in active, saved, join request, LAN peer rows. |
| Participant admin toggle | `add_admin`, `remove_admin` | Admin mutation actions | Required | Required | Required | Required | Required | Active network currently exposes toggle; saved network mainly shows admin state. |
| Participant remove | `remove_participant` | Remove participant action | Required | Required | Required | Required | Required | Admin-gated, icon button on native shells. |
| Participant status badges | `participantBadgeClass`, badge text helpers | Shared derived labels | Required | Required | Required | Required | Required | FIPS reachable/pending/offline plus mesh seen/unseen. |
| Participant traffic/path details | `participantTrafficText`, fields | tx/rx, relay path, runtime endpoint, routes | Required | Required | Required | Required | Required | Keep fallback and advertised route visibility. |
| LAN pairing start/stop | `start_lan_pairing`, `stop_lan_pairing` | Core-owned multicast pairing runtime | Required | Required | Required | Required | Required | Mobile multicast may need platform permissions/capabilities. |
| LAN pairing countdown | local deadline from state | `lanPairingActive`, remaining seconds | Required | Required | Required | Required | Required | UI ticks once per second without forcing backend refresh. |
| Nearby LAN peer list | `lanPeers` | Core pairing snapshot | Required | Required | Required | Required | Required | Filter peers already in current network. |
| Join LAN peer | `onJoinLanPeer` | Import invite action | Required | Required | Required | Required | Required | Same auto-connect behavior as invite import. |
| Saved networks list | `SavedNetworksPanel.svelte` | All networks with enabled flag | Required | Required | Required | Required | Required | Active network separate; inactive networks collapsible/listed. |
| Add saved network | `add_network` | Add network action | Required | Required | Required | Required | Required | Optional name. |
| Activate saved network | `set_network_enabled` | Set active network action | Required | Required | Required | Required | Required | Ensure daemon reload/session state is correct. |
| Delete saved network | `remove_network` | Remove network action | Required | Required | Required | Required | Required | Current button is disabled when only one inactive network remains; revisit rule in core. |
| Edit saved network profile | `SavedNetworkCard.svelte` | Same name/mesh/admin actions as active network | Required | Required | Required | Required | Required | Inactive networks still receive join requests. |
| Saved network participants | `SavedNetworkParticipantRow.svelte` | Participant list and alias actions | Required | Required | Required | Required | Required | Minimal status for inactive profiles. |
| Routing mode summary | `RoutingPanel.svelte` | Derived routing status text | Required | Required | Required | Required | Required | Direct mesh, remote exit, local exit, or both. |
| Advertise private exit node | `advertiseExitNode` | Settings patch | Required | Required | Required | Required | Required | Affects default route advertisement. |
| Advertised routes editing | `advertisedRoutes` | Settings patch + validation | Required | Required | Required | Required | Required | Debounced comma-separated input today; core should validate CIDRs. |
| Exit node search/select | `exitNode` | Candidate projection and setting | Required | Required | Required | Required | Required | Search alias, npub, tunnel IP. Disable peers not offering exit. |
| No exit node selection | `onSelectExitNode('')` | Clear exit-node setting | Required | Required | Required | Required | Required | Also exposed in desktop tray. |
| Diagnostics panel | `AdvancedPanels.svelte` | Health issues, network summary, port mapping | Required | Required | Required | Required | Required | Auto-open when health count increases. |
| Health warnings | `health` | Health issue list with severity | Required | Required | Required | Required | Required | Keep empty state and severity mapping. |
| Network diagnostics | `NetworkSummary` | Interface, local IPs, gateway, captive portal | Required | Required | Required | Required | Required | Mobile may have reduced details if OS restricts APIs. |
| Port mapping status | `PortMappingStatus` | UPnP/NAT-PMP/PCP state | Required | Required | Required | Required | Required | Show active protocol and external endpoint. |
| FIPS relay list | `relays`, `add_relay`, `remove_relay` | Relay config + status | Required | Required | Required | Required | Required | At least one relay required. |
| Relay state badges | `RelayView` | Up/down/checking/unknown status | Required | Required | Required | Required | Required | Keep status text. |
| Session options | `autoconnect` | Settings patch | Required | Required | Required | Required | Required | Text should be platform neutral. |
| Background service panel | `ServiceActionPanel.svelte` | Service status, service repair recommendation, actions | Desktop | Desktop | Desktop | N/A | N/A | Mobile should not show desktop service install/repair UI. |
| Install/reinstall service | `install_system_service` | Desktop service action | Required | Required | Required | N/A | N/A | May require admin/UAC/sudo/polkit flow. |
| Enable/disable service | `enable_system_service`, `disable_system_service` | Desktop service action | Required | Required | Required | N/A | N/A | Current macOS copy references launchd; native copy should use platform-specific terms. |
| Uninstall service | `uninstall_system_service` | Desktop service action | Required | Required | Required | N/A | N/A | Keep reachable after setup. |
| Service version repair prompt | `service-repair.js` | Core-derived repair prompt state | Required | Required | Required | N/A | N/A | Use native confirmation dialog; avoid repeated prompt per version key. |
| Service action settlement polling | `waitForServiceActionSettlement` | Core/service action status | Required | Required | Required | N/A | N/A | Native shell should show progress while launchd/service manager settles. |
| CLI install/uninstall | `install_cli`, `uninstall_cli` | Desktop CLI action | Required | Required | Required | N/A | N/A | Installs `nvpn` into PATH; may require elevation. |
| App version/config path display | `SystemPanel.svelte` | App version, config path | Required | Required | Required | Required | Required | Mobile may hide raw path behind support/debug view. |
| MagicDNS status | `magicDnsStatus` | Runtime DNS status string | Required | Required | Required | Required | Required | Mobile DNS behavior may be tunnel-scoped. |
| MagicDNS suffix editing | `magicDnsSuffix` | Settings patch | Required | Required | Required | Required | Required | Debounced. |
| Endpoint/tunnel IP/listen port settings | `SystemPanel.svelte` | Settings patch + validation | Required | Required | Required | Required | Required | Mobile may constrain endpoint/listen port by OS VPN APIs. |
| Launch on startup | Legacy autostart plugin | Native startup registration effect + config setting | Required | Required | Required | N/A | N/A | Android/iPhone use OS background/VPN behavior, not login startup. |
| Close to tray/status item | `closeToTrayOnClose` | Config setting + native close behavior | Required | Required | Required | N/A | N/A | macOS menu bar item; Windows/Linux tray/status notifier. |
| Desktop tray/status menu | `tray_runtime.rs` | Tray runtime projection and actions | Required | Required | Required | N/A | N/A | Menu: VPN status, toggle, this-device copy, network devices, exit nodes, settings, quit. |
| Tray left-click opens app | Legacy tray handler | Native shell action | Required | Required | Required | N/A | N/A | Keep menu/status item accessible. |
| Autostart hidden launch | `--autostart`, hide to tray | Launch-mode detection | Required | Required | Required | N/A | N/A | Current code mainly handles macOS conflict/defer behavior; port intentionally. |
| Single-instance handling | Legacy single-instance plugin | Native process/singleton coordination | Required | Required | Required | Mobile | Mobile | Mobile OS already single-instances app task but deep links must route to existing app. |
| Deep links | `nvpn://invite`, `nvpn://debug/...` | Core deep-link parser/action dispatcher | Required | Required | Required | Required | Required | Support startup URLs and already-running app URLs. |
| Debug automation deep links | `nvpn://debug/tick`, request/accept join | Test-only action path | Required | Required | Required | Required | Required | Keep for e2e harness parity. |
| Update banner | `UpdateBanner.svelte`, hashtree updater | Update check/download/install API | Required | Required | Required | N/A | N/A | Mobile updates go through store/TestFlight/Zapstore unless a separate allowed updater exists. |
| Manual update panel | `SystemPanel.svelte` updater section | Same updater API + prefs | Required | Required | Required | N/A | N/A | Preserve auto-check/auto-install prefs on desktop. |
| Update prefs storage | `localStorage` | Native preference storage | Required | Required | Required | N/A | N/A | Move prefs into core or native settings store. |
| Window chrome/drag region | `App.svelte`, legacy overlay titlebar | Native window style | Required | Required | Required | N/A | N/A | Native shells can use platform chrome instead of custom overlay. |
| Responsive layout | Svelte CSS | Native adaptive layouts | Required | Required | Required | Required | Required | Desktop can use multi-panel; mobile should use navigation stack/sheets. |
| Copy feedback | `copiedValue` timeout | Native transient status | Required | Required | Required | Required | Required | Snackbar/toast/checkmark, clears after roughly 2s. |
| Collapsible panels | `<details>` panels | Local UI state only | Required | Required | Required | Required | Required | Diagnostics auto-opens on new health issues. |
| Mock/demo backend | `mock-backend.ts` | Native previews/test fixtures | Required | Required | Required | Required | Required | Replace with core fixture snapshots and platform preview states. |
| Mobile VPN permission/control | `android_vpn`, `ios_vpn`, `ios_packet_tunnel` | Platform-specific native VPN bridge | N/A | N/A | N/A | Required | Required | Must preserve current Android VpnService and iPhone Packet Tunnel behavior. |
| Mobile runtime status detail | `runtime_capabilities_for_platform` | Capability flags in state | N/A | N/A | N/A | Required | Required | Keep simulator/device distinction for iPhone. |

## macOS App Parity Status

This table tracks the SwiftUI/AppKit shell under `macos/` against the legacy
Tauri/Svelte feature contract. It is scoped to macOS only.

Status legend:

- `Ready`: implemented in the native macOS shell and build-verified.
- `Partial`: visible or wired, but missing behavior from the current app.
- `Missing`: no macOS native implementation yet.
- `Removed`: removed from current product behavior; no native parity work.

| Feature group | Legacy source | macOS status | Native macOS coverage | Remaining parity work |
| --- | --- | --- | --- | --- |
| Typed Rust core boundary | `nostr-vpn-app-core`, legacy commands | Ready | `FfiApp.state()`, `refresh()`, and typed `NativeAppAction` dispatch are used directly from Swift through UniFFI. Native state now projects service, diagnostics, routing, relays, MagicDNS, join requests, and participant details. | Keep action/state additions typed; avoid reintroducing JSON bridge helpers. |
| Initial state load | `get_state`, `AppBootstrap.svelte` | Ready | `AppManager` constructs `FfiApp`, reads initial state synchronously, and starts the refresh loop on window appearance. | Add boot-ready automation event only if native e2e harness needs it. |
| Periodic refresh | `tick` interval | Ready | `AppManager.start()` refreshes every 1500ms. | Later replace polling with a core update stream if added. |
| Action lock/error recovery | `runAction`, action flags | Ready | `AppManager` serializes typed actions, shows in-flight status, and projects core errors into the shell. | Add per-control progress text if native e2e tests require it. |
| Main status hero | `HeroStatusPanel.svelte` | Ready | Shows active network title, admin badge, mesh state, daemon/VPN/FIPS badges, peer/tunnel/exit metrics, disclosure, identity, and VPN toggle. | Tune copy if product policy text changes. |
| VPN connect/disconnect | `connect_session`, `disconnect_session` | Ready | Connect/disconnect dispatches typed native actions; Rust runs elevated `nvpn` on macOS. | None. |
| Privacy disclosure | `shouldShowVpnDataDisclosure` | Ready | Native hero renders the VPN data disclosure when VPN control/mobile-like policy applies. | Move exact policy text into core if reused by more shells. |
| Own npub display/copy | `HeroStatusPanel.svelte` | Ready | Native hero renders `ownNpub`, supports selection, and gives transient copy feedback. | None. |
| Active network summary | `ActiveNetworkPanel.svelte` | Ready | Native active-network panel supports name edit, mesh ID edit/copy, admin summary, join-request toggle/list, invite, and admin-gated controls. | None. |
| Mesh ID editing | `mesh-id.js`, `set_network_mesh_id` | Ready | Mesh ID edits dispatch typed Rust action; Rust normalizes/canonicalizes and reports errors. | Add blur/Enter auto-commit if preferred over explicit save icon. |
| Invite generation/copy | `InviteShareSection.svelte` | Ready | Invite comes from Rust, can be copied/shared, and renders a native CoreImage QR. | None. |
| Invite deep-link import | `nvpn://invite/...` handler | Ready | Native handles running-app and startup invite URLs, shows parsed confirmation, leaves canceled invite text in the field, and app-core auto-connects after import. | None. |
| Invite paste/import | `InviteImportPanel.svelte` | Ready | Native invite field imports pasted/typed `nvpn://invite/...` values through typed core action with parsed target confirmation. | None. |
| Invite QR generation | `qrcode` | Ready | Native CoreImage QR exactly encodes `activeNetworkInvite`. | None. |
| Invite QR scan | `jsQR`, camera/image input | Ready | Native image picker decodes QR images, and live AVFoundation QR scan imports through the same confirmation path. | None. |
| Participant list | `ActiveNetworkPanel.svelte` | Ready | Shows participants, reachability, transport/presence badges, npub copy, alias, MagicDNS, traffic, routes, admin, exit-node, and remove controls. | None. |
| Manual add participant | `add_participant` | Ready | Admin can add participant npub with optional alias. | None. |
| Participant alias editing | `set_participant_alias` | Ready | Alias edits dispatch typed Rust action and show MagicDNS suffix/name. | Add debounce if explicit save feels too heavy. |
| Participant admin/remove actions | `add_admin`, `remove_admin`, `remove_participant` | Ready | Admin toggle and remove icon dispatch typed core actions with local-admin gating. | Add destructive confirmation if needed. |
| Participant traffic/path details | Participant runtime fields | Ready | `NativeParticipantState` now mirrors traffic, routes, exit-node capability, tunnel IP, transport, presence, and last-signal details. | None. |
| LAN pairing | `start_lan_pairing`, `stop_lan_pairing`, `lanPeers` | Ready | Native UI exposes LAN pair start/stop and nearby-peer rows; app-core owns the multicast runtime, countdown, stale-peer pruning, and invite metadata decoding. | None. |
| Saved networks list | `SavedNetworksPanel.svelte` | Ready | Sidebar and saved-networks disclosure support add, activate, rename, delete, mesh edit, participant preview, and participant removal. | Add inactive invite/import and join-request detail expansion if needed. |
| Activate saved network | `set_network_enabled` | Ready | Inactive network rows dispatch typed activation and daemon reload handling from Rust. | None. |
| Delete saved network | `remove_network` | Ready | Native saved-network delete dispatches typed core removal and core preserves the last-network rule. | Add confirmation if needed. |
| Routing summary | `RoutingPanel.svelte` | Ready | Native routing section shows direct/exit mode, advertised route editing, offer-exit toggle, and exit candidates. | Add richer route helper text from core if desired. |
| Advertise exit node | `advertiseExitNode` | Ready | Toggle dispatches typed settings patch from routing and menu bar surfaces. | None. |
| Advertised routes editing | `advertisedRoutes` | Ready | CIDR string editor dispatches typed settings patch; Rust normalizes and validates routes. | Inline validation helper text can be added later. |
| Exit node search/select | `exitNode` | Ready | Searchable native candidate list supports no-exit selection and offered-exit peers. | None. |
| Diagnostics panel | `AdvancedPanels.svelte` | Ready | Native diagnostics disclosure renders health issues, interface/IP/gateway, captive portal, and port-mapping state; opens when health count increases. | None. |
| Relay list/status | Relay panel | Ready | Shows relay URLs, status text, summary badges, add, remove, and disables removal of the last relay. | None. |
| Session options | `autoconnect` | Ready | Native system panel includes autoconnect, launch-on-startup with LaunchAgent registration, and menu-bar-on-close settings toggles. | None. |
| Device settings | `SystemPanel.svelte` | Ready | Native system panel includes name, endpoint, tunnel IP, listen port, MagicDNS suffix/status, app/config/version fields, CLI controls, service controls, and updater controls. | None. |
| MagicDNS | `magicDnsStatus`, `magicDnsSuffix` | Ready | Native state projects MagicDNS status/name/suffix; UI exposes suffix editor and participant DNS labels. | None. |
| Background service panel | `ServiceActionPanel.svelte` | Ready | Native state queries `nvpn service status --json`; UI supports install/reinstall/enable/disable/uninstall, stale-version repair prompts, and settlement polling after service actions. | None. |
| CLI install/uninstall | `install_cli`, `uninstall_cli` | Ready | Native UI shows CLI status and runs elevated install/reinstall/uninstall actions. | None. |
| Launch on startup | Autostart plugin | Ready | Native UI persists the setting and writes/removes a per-user LaunchAgent with `--autostart`. | None. |
| Close to tray/status item | Tray runtime | Ready | Native `MenuBarExtra` supports open, VPN toggle, exit toggle, copy this device, network devices, exit-node selection, refresh, and quit; close hides the main window when enabled. | None. |
| Autostart hidden launch | `--autostart` | Ready | LaunchAgent starts the app with `--autostart`, and the main window hides after startup unless a deep link is being handled. | None. |
| Single-instance handling | Legacy singleton plugin | Ready | Native app holds a local lock and routes duplicate-process startup URLs into the existing instance through distributed notifications. | None. |
| Debug automation deep links | `nvpn://debug/...` | Ready | Native handler supports `nvpn://debug/tick`, `request-join`, and `accept-join` with active-network fallbacks. | None. |
| Hashtree updater | `UpdateBanner.svelte`, updater panel | Ready | Native updater checks the hashtree release manifest, preserves auto-check/auto-install prefs, downloads macOS assets, and can install `.app.tar.gz` archives by relaunching into the replacement app. | None. |
| Responsive/adaptive layout | Svelte CSS | Ready | Native split-view desktop layout builds and was checked with an app-window screenshot at 1100x760. | Add compact-window and accessibility passes. |
| Copy feedback | `copiedValue` timeout | Ready | Clipboard writes show transient checkmark feedback for identity, mesh, invite, and peer npub copy buttons. | None. |
| Collapsible panels | `<details>` state | Ready | Native saved networks, diagnostics, and system sections use disclosure groups; diagnostics opens when health issues increase. | Persist expanded state if users want it. |
| Mock/demo fixtures | `mock-backend.ts` | Ready | `macos/Resources/preview-state.json` provides a native state snapshot for previews and screenshot tests without the old mock backend. | None. |
| Public relay fallback UI | Removed relay fallback/public services code | Removed | Removed upstream in `origin/master`; native shell also omits those fields and controls. | No parity work unless product reintroduces a public-service feature. |

## Linux App Parity Status

This table tracks the GTK/libadwaita shell under `linux/` against the current
macOS shell. Tauri remains the feature inventory above, but Linux should look and
flow like the Swift app rather than the removed Svelte UI.

| Feature group | Linux status | Native Linux coverage | Remaining parity work |
| --- | --- | --- | --- |
| Typed Rust core boundary | Ready | `linux/src/main.rs` uses `FfiApp.state()`, `refresh()`, and typed `NativeAppAction` directly from Rust. | Keep additions typed and shared with macOS. |
| Initial state and refresh | Ready | The GTK shell reads initial state synchronously and refreshes through `FfiApp.refresh()` on a two-second timer plus the toolbar refresh button. | Add a boot-ready automation hook if native e2e needs it. |
| Main status hero | Ready | Matches the macOS hierarchy: active network title, admin badge, mesh/session/daemon/relay badges, identity copy, tunnel IP, exit indication, and connect/disconnect control. | Add richer service-repair prompt text if core exposes a shared helper. |
| Device roster | Ready | Shows participant name, admin/exit badges, tunnel IP, npub copy, reachability, admin toggle, remove action, and a Manage Devices disclosure. | Add destructive confirmations if needed. |
| Participant alias editing | Ready | Manage Devices includes per-participant alias save, admin toggle, and remove controls. | Add debounce if explicit save feels too heavy. |
| Join requests | Ready | Inbound request rows show requester info, npub copy, and admin-gated accept action. | None. |
| Invite share/import | Ready | Share page renders the core invite as a QR code, supports copy, paste/import, image QR import, optional `zbarcam` live QR scan, join request status, and LAN peer join. | Add share portal support only if desktop share-sheet behavior becomes important. |
| LAN pairing | Ready | Start/stop pairing, countdown, nearby peer rows, and invite import are wired through app-core actions. | Mobile permission parity is out of scope for Linux. |
| Routing | Ready | Direct route, searchable exit-node candidates, advertised route editing, and offer-exit toggle mirror the macOS Routing page. | Add richer route helper text from core if added. |
| Active network settings | Ready | Settings exposes active network name, editable/copyable network ID, and admin-gated join-request toggle. | Add blur/Enter commit polish. |
| Saved networks | Ready | Saved Networks disclosure lists inactive networks with counts, activate action, and delete action guarded by the last-network rule. | Expand inactive profiles if Linux needs the full macOS saved-network detail surface. |
| Device/system settings | Ready | Name, tunnel IP, endpoint, listen port, MagicDNS suffix, autoconnect, startup desktop-entry registration, and tray preferences dispatch typed settings patches. | None. |
| CLI and service controls | Ready | Shows CLI/service status badges and install/reinstall/uninstall plus enable/disable service actions when supported; service actions show settlement progress and stale-version repair state. | Add a richer polkit prompt only if the CLI stops being launched through an already-elevated context. |
| Relay list/status | Ready | Advanced panel shows relay summary counts, relay rows with status text, add/remove, and disables removing the last relay. | None. |
| Diagnostics | Ready | Advanced diagnostics shows interface/IP/gateway/mapping metrics, identity/config/runtime fields, MagicDNS, and health issue rows. | Auto-open diagnostics on new health issues if users need parity with macOS. |
| Tray/status menu | Ready | Native StatusNotifierItem + DBusMenu implementation exposes open, VPN toggle, exit toggle, this-device copy, network devices, exit-node selection, refresh, and quit; close-to-tray and autostart-hidden launch are wired. | Verify with installed desktop environments beyond the Docker Fluxbox harness. |
| Deep links and single-instance | Ready | GApplication routes startup and already-running `nvpn://` URLs into the existing GTK app; the desktop entry registers `x-scheme-handler/nvpn`; invite and debug automation links dispatch typed core actions. | Add installed-package smoke coverage once Linux packaging tests exist. |
| Updater | Ready | Linux checks the hashtree release manifest, shows current/update status, prefers Linux AppImage/deb assets, downloads with `curl`, marks AppImages executable, and opens the downloaded package with `xdg-open`. | Add automatic install only after Linux packaging policy is settled. |
| Screenshot/dev harness | Ready | Docker Xvfb/Fluxbox dev environment runs the GTK app and supports window screenshots over VNC on `localhost:5902`. | Add automated screenshot fixture states. |

## Native Implementation Phases

| Phase | Deliverable | Exit criteria |
| --- | --- | --- |
| 0. Contract extraction | Move backend state, settings patches, action handlers, derived labels, invite parsing, mesh ID validation, and tray projections into a native-ready Rust app core | `crates/nostr-vpn-app-core` exposes typed UniFFI state/actions and the macOS shell consumes `FfiApp` directly; remaining legacy behavior is tracked as explicit native parity work. |
| 1. Desktop minimum | macOS, Windows, and Linux render the main status, active network, invite import/share, participant management, routing, diagnostics, relays, service panel, system settings, deep links, and tray/menu actions | Desktop smoke tests can import invites, request/accept join, toggle VPN, and exercise tray actions. macOS and Linux now cover the native shell surface; Windows remains the desktop gap. |
| 2. Mobile minimum | Android and iPhone render the same state/action surface with native VPN permission/session control, invite QR scan/share, LAN pairing, saved networks, routing, diagnostics, relays, and deep links | Android emulator/device and iPhone simulator/device smoke tests can import invites and start supported VPN flows |
| 3. Desktop niceties | Hashtree updater, CLI install/uninstall, startup registration, close-to-tray, service repair prompts, single-instance conflict handling | Legacy desktop e2e scenarios have native replacements |
| 4. Polish/parity hardening | Platform screenshots, accessibility pass, empty/error states, fixture preview coverage | All rows above are either implemented or explicitly marked removed/deferred in this file |

## Open Decisions

| Decision | Options | Current recommendation |
| --- | --- | --- |
| Push updates vs polling | Keep 1500ms polling, add core update stream, or hybrid | Use update stream with tick fallback; avoid mobile background polling |
| Linux shell API | Direct Rust GTK calls into the core or UniFFI like other shells | Direct Rust is simpler, but keep the same typed state/action structs so parity tests are shared |
| QR generation location | Native QR libraries per platform or Rust QR helper | Rust helper for invite QR bytes; native scanner APIs for camera/image decode |
| Derived text ownership | Keep per-shell text formatting or move helper text into core | Move policy-bearing derived labels into core; keep purely visual labels native |
| Desktop updater on Linux | Keep hashtree updater or only package-manager updates | Keep hashtree updater for parity unless Linux packaging policy says otherwise |
