# Android Native Shell

Target shell: Kotlin with Jetpack Compose.

Responsibilities:

- bind to `nostr-vpn-app-core` through the shared JSON C ABI and Android JNI exports
- render `UiState` with native Compose screens
- dispatch `NativeAppAction` values into the shared Rust core
- own Keystore access, camera/image QR scanning, share intents, deep links, and Android `VpnService` permission/control
- preserve the Android VPN runtime behavior while replacing the legacy webview UI

The parity checklist is in `docs/native-ui-parity-matrix.md`.

## Build

```bash
just android-build
```

The build task cross-compiles `nostr-vpn-app-core` for `arm64-v8a` with `cargo-ndk`
and packages it into the debug APK.

## Install

```bash
just android-install
```

The first cut includes native state, invite, roster, routing, diagnostics,
deep-link, and VPN permission surfaces. Android app-core startup is mobile-aware
and does not require the desktop `nvpn` CLI. The packet tunnel service is present
but the Android data-plane loop still needs to be wired to FIPS endpoint delivery.
