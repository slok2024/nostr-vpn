# Linux Native Shell

Rust GTK4/libadwaita shell over `nostr-vpn-app-core`.

Run it from the repo root:

```bash
just run-linux
```

The dev target runs inside Docker with a small Xvfb/Fluxbox desktop and VNC on
`localhost:5902`. The VNC password is `nostrvpn`.

Useful commands:

```bash
just linux-build
./tools/run-linux cargo check
./tools/run-linux cargo run
```

The shell follows the current SwiftUI/AppKit app structure: Devices, Share,
Exit Nodes, Settings, and an Advanced diagnostics disclosure. It
owns the same core flows for connect/disconnect, roster presence, participant
management, invite QR/import, LAN pairing, saved networks, exit-node selection,
service/CLI actions, and diagnostics. Remaining
Linux-native work is desktop portal integration, live camera QR scanning,
tray/status notifier support, and packaged update UX.

Installed packages register `nvpn://` links through the desktop entry. From the
repo root, pass a link into the dev shell as an argument:

```bash
./tools/run-linux cargo run -- nvpn://debug/tick
```

The parity checklist is in `../docs/native-ui-parity-matrix.md`.
