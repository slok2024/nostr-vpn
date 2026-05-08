# iOS Native Shell

Target shell: SwiftUI with NetworkExtension Packet Tunnel integration.

Responsibilities:

- bind to `nostr-vpn-app-core` through the shared JSON C ABI
- render `UiState` with native SwiftUI navigation and sheets
- dispatch `NativeAppAction` values into the shared Rust core
- own platform effects such as share sheets, deep links, Packet Tunnel permission/control, and later Keychain plus live camera QR scanning
- keep iPhone simulator capability differences visible through runtime capabilities

The parity checklist is in `docs/native-ui-parity-matrix.md`.

## Build

```bash
just ios-build
```

The build task cross-compiles `nostr-vpn-app-core` for iOS simulator and device
static libraries, creates an xcframework, generates the Xcode project with
XcodeGen, and builds the app for the iOS simulator.

## Run

```bash
just ios-run
```

The first native cut includes the SwiftUI state/action shell, invite QR,
invite copy/share/import, roster, routing, settings, diagnostics, deep
links, app icon, and a Packet Tunnel extension shell. The packet data-plane loop
still needs to be wired to FIPS endpoint delivery before the iOS VPN runtime is
complete.
