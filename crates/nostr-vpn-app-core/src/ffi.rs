use std::sync::Arc;

#[derive(uniffi::Object, Debug, Default)]
pub struct NativeAppContract;

#[uniffi::export]
impl NativeAppContract {
    #[uniffi::constructor]
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }

    #[must_use]
    pub fn state_schema_version(&self) -> String {
        "ui-state-v1".to_string()
    }

    #[must_use]
    pub fn empty_state_json(&self) -> String {
        crate::state::empty_state_json()
    }

    #[must_use]
    pub fn action_descriptors_json(&self) -> String {
        crate::actions::action_descriptors_json()
    }

    #[allow(clippy::needless_pass_by_value)]
    #[must_use]
    pub fn validate_action_json(&self, action_json: String) -> bool {
        crate::actions::validate_action_json(&action_json)
    }

    #[allow(clippy::needless_pass_by_value)]
    #[must_use]
    pub fn normalize_action_json(&self, action_json: String) -> String {
        crate::actions::normalize_action_json(&action_json)
    }

    #[allow(clippy::needless_pass_by_value)]
    #[must_use]
    pub fn runtime_capabilities_json(&self, platform: String, ios_simulator: bool) -> String {
        crate::platform::runtime_capabilities_json(&platform, ios_simulator)
    }
}
