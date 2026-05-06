pub mod actions;
pub mod c_abi;
mod ffi;
mod invite;
mod lan_pairing;
pub mod native_state;
pub mod platform;
pub mod state;

pub use actions::NativeAppAction;
pub use ffi::FfiApp;
pub use native_state::{
    NativeAppState, NativeNetworkState, NativeParticipantState, NativeRelayState,
};
pub use platform::{
    NativeRuntimeCapabilities, RuntimePlatform, current_runtime_capabilities,
    current_runtime_platform, runtime_capabilities_for,
};
pub use state::{
    DaemonPeerState, DaemonRuntimeState, InboundJoinRequestView, LanPeerView, NetworkView,
    OutboundJoinRequestView, ParticipantView, RelaySummary, RelayView, SettingsPatch,
    TrayExitNodeEntry, TrayMenuItemSpec, TrayNetworkGroup, TrayRuntimeState, UiState,
};

uniffi::setup_scaffolding!();
