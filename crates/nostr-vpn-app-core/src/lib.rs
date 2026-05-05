pub mod actions;
mod ffi;
pub mod platform;
pub mod state;

pub use actions::{
    NativeAppAction, NativeAppActionDescriptor, action_descriptors, action_descriptors_json,
    normalize_action_json, validate_action_json,
};
pub use ffi::NativeAppContract;
pub use platform::{
    NativeRuntimeCapabilities, RuntimePlatform, current_runtime_capabilities,
    current_runtime_platform, runtime_capabilities_for, runtime_capabilities_json,
};
pub use state::{
    DaemonPeerState, DaemonRuntimeState, InboundJoinRequestView, LanPeerView, NetworkView,
    OutboundJoinRequestView, ParticipantView, RelaySummary, RelayView, SettingsPatch,
    TrayExitNodeEntry, TrayMenuItemSpec, TrayNetworkGroup, TrayRuntimeState, UiState,
    empty_state_json,
};

uniffi::setup_scaffolding!();
