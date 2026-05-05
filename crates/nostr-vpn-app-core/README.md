# nostr-vpn-app-core

Native shells use this crate as the shared app contract while the runtime is
being extracted from the current Tauri backend.

It currently owns:

- the UI snapshot structs that mirror the shipped Svelte/Tauri `UiState`
- the complete action set corresponding to the current Tauri commands
- platform capability projection for desktop, Android, and iPhone
- a small UniFFI JSON bridge for Swift, Kotlin, C#, and other native shells

The JSON bridge is intentionally narrow. The long-term target is typed UniFFI
records and enums once the backend actor has moved out of the Tauri crate.
