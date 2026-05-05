# Android Native Shell

Target shell: Kotlin with Jetpack Compose.

Responsibilities:

- bind to `nostr-vpn-app-core` through UniFFI
- render `UiState` with native Compose screens
- dispatch `NativeAppAction` values into the shared Rust core
- own Keystore access, camera/image QR scanning, share intents, deep links, and Android `VpnService` permission/control
- preserve the current Android VPN runtime behavior while replacing the Tauri webview UI

The parity checklist is in `docs/native-ui-parity-matrix.md`.
