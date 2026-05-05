# Linux Native Shell

Target shell: Rust GTK4/libadwaita.

Responsibilities:

- use `nostr-vpn-app-core` directly or through UniFFI-compatible types
- render `UiState` with native GTK/libadwaita widgets
- dispatch `NativeAppAction` values into the shared Rust core
- own Secret Service fallback, desktop startup registration, status notifier/tray integration, file/camera QR scanning, and package-specific update/install UX
- preserve current service, deep-link, invite, LAN pairing, diagnostics, relay, and exit-node behavior

The parity checklist is in `docs/native-ui-parity-matrix.md`.
