# Windows Native Shell

Target shell: WPF/.NET.

Responsibilities:

- bind to `nostr-vpn-app-core` through the explicit C ABI JSON bridge in `crates/nostr-vpn-app-core/src/c_abi.rs`
- render `UiState` with native WPF views
- dispatch `NativeAppAction` values into the shared Rust core
- own Credential Manager access, UAC/service prompts, tray integration, camera/image QR scanning, startup registration, and installer/update UX
- preserve current Windows service, Wintun/userspace tunnel, config import, deep-link, invite, LAN pairing, and exit-node behavior

The parity checklist is in `docs/native-ui-parity-matrix.md`.

## Run

From Windows:

```powershell
.\scripts\windows-build.ps1 -Run
```

From Git Bash or `just` on Windows:

```bash
just run-windows
```

The build first compiles `nostr-vpn-app-core` and `nvpn`, then builds the WPF shell and copies `nostr_vpn_app_core.dll` plus `nvpn.exe` next to the app executable.
