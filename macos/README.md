# macOS Native Shell

Target shell: SwiftUI with AppKit integrations.

Responsibilities:

- bind to `nostr-vpn-app-core` through UniFFI
- render `UiState` with native SwiftUI views
- dispatch `NativeAppAction` values into the shared Rust core
- own Keychain access, LaunchAgent startup registration, status item/menu, and native update/install prompts
- preserve current desktop service, tray, deep-link, QR, invite, LAN pairing, and exit-node behavior

The parity checklist is in `docs/native-ui-parity-matrix.md`.
