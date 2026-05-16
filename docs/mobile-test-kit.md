# Mobile Test Kit

Mobile coverage is split by what each layer can prove.

## Shared Rust

Run this when touching mobile state, FFI, config, reconnect, roster, or FIPS
handoff behavior:

```sh
just mobile-test-kit-rust
```

These checks belong in `nostr-vpn` because the mobile apps call
`nostr-vpn-app-core`. Plain `fips` should keep protocol, routing, session,
candidate, and cross-target Rust tests platform-neutral; real Android/iOS VPN
behavior depends on the `nostr-vpn` shells, permissions, FFI, and packet tunnel
integration.

## Build Smoke

Run the normal fast kit before handing off a mobile-sensitive change:

```sh
just mobile-test-kit
```

This runs shared Rust tests, builds the Android debug APK, and builds the iOS
simulator/device xcframework path. It catches JNI/C ABI drift, missing mobile
targets, and Xcode/Gradle project breakage.

## Simulator And Emulator Smoke

Run this when UI, launch, deep-link, or mobile packaging behavior changed:

```sh
just mobile-test-kit-sim
```

The iOS simulator can verify build, install, launch, state/action UI, and
screenshot capture. It cannot prove the NetworkExtension packet tunnel or VPN
permission path. Android emulator or attached-device smoke verifies APK install
and launch; use `NVPN_ANDROID_SERIAL` or `ANDROID_SERIAL` when more than one adb
target is online.

## Physical Device VPN Smoke

Run this after VPN dataplane, reconnect, LAN discovery, roster transfer, or
mobile tunnel config changes:

```sh
NVPN_ANDROID_SERIAL=<adb-serial> just android-smoke-vpn
NVPN_IOS_DEVICE=<device-id> just ios-smoke-device
```

The iOS device command assumes a development build is already installed and uses
debug launch arguments to cycle the VPN state. Android installs the debug APK and
uses the debug action extra. Device identifiers, signing teams, local hostnames,
IP addresses, and personal names must stay in environment variables, local shell
history, or ignored files.
