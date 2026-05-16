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

## Smoke

```bash
just android-smoke
```

Use `NVPN_ANDROID_SERIAL=<adb-serial>` or `ANDROID_SERIAL=<adb-serial>` when more
than one device or emulator is online. `just android-smoke-vpn` also cycles the
debug VPN action and expects the VPN permission/config path to be usable on that
device.

The native shell includes state, invite, roster, routing, diagnostics,
deep-link, VPN permission surfaces, and Android `VpnService` packet handling
backed by the shared Rust core.
