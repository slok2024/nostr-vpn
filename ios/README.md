# iPhone Native Shell

Target shell: SwiftUI with NetworkExtension Packet Tunnel integration.

Responsibilities:

- bind to `nostr-vpn-app-core` through UniFFI
- render `UiState` with native SwiftUI navigation and sheets
- dispatch `NativeAppAction` values into the shared Rust core
- own Keychain access, camera/image QR scanning, share sheets, deep links, and Packet Tunnel permission/control
- keep iPhone simulator capability differences visible through runtime capabilities

The parity checklist is in `docs/native-ui-parity-matrix.md`.
