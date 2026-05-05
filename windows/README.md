# Windows Native Shell

Target shell: WPF/.NET.

Responsibilities:

- bind to `nostr-vpn-app-core` through UniFFI-generated C# bindings
- render `UiState` with native WPF views
- dispatch `NativeAppAction` values into the shared Rust core
- own Credential Manager access, UAC/service prompts, tray integration, camera/image QR scanning, startup registration, and installer/update UX
- preserve current Windows service, Wintun/userspace tunnel, config import, deep-link, invite, LAN pairing, and exit-node behavior

The parity checklist is in `docs/native-ui-parity-matrix.md`.
