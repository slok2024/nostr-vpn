use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthIssue {
    pub code: String,
    pub severity: HealthSeverity,
    pub summary: String,
    pub detail: String,
}

impl HealthIssue {
    #[must_use]
    pub fn new(
        code: impl Into<String>,
        severity: HealthSeverity,
        summary: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            severity,
            summary: summary.into(),
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProbeState {
    Available,
    Unavailable,
    Unsupported,
    Error,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProbeStatus {
    pub state: ProbeState,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

impl ProbeStatus {
    #[must_use]
    pub fn new(state: ProbeState, detail: impl Into<String>) -> Self {
        Self {
            state,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PortMappingStatus {
    pub upnp: ProbeStatus,
    pub nat_pmp: ProbeStatus,
    pub pcp: ProbeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub good_until: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NetcheckReport {
    pub checked_at: u64,
    pub udp: bool,
    pub ipv4: bool,
    pub ipv6: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_ipv4: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_ipv6: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapping_varies_by_dest_ip: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captive_portal: Option<bool>,
    #[serde(default)]
    pub port_mapping: PortMappingStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_interface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_ipv4: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_ipv6: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_ipv4: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_ipv6: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captive_portal: Option<bool>,
}
