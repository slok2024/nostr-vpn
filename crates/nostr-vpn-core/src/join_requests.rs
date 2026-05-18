use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::config::normalize_runtime_network_id;

pub const FIPS_JOIN_REQUEST_RETRY_SECS: u64 = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshJoinRequest {
    pub network_id: String,
    #[serde(default)]
    pub requester_node_name: String,
}

pub fn normalize_join_request(request: MeshJoinRequest) -> Result<MeshJoinRequest> {
    let network_id = normalize_runtime_network_id(&request.network_id);
    if network_id.is_empty() {
        return Err(anyhow!("mesh join request network_id must not be empty"));
    }

    Ok(MeshJoinRequest {
        network_id,
        requester_node_name: request.requester_node_name.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_request_normalizes_network_id_and_node_name() {
        let request = normalize_join_request(MeshJoinRequest {
            network_id: "  Mesh Home  ".to_string(),
            requester_node_name: " alice-phone ".to_string(),
        })
        .expect("normalize");

        assert_eq!(request.network_id, "Mesh Home");
        assert_eq!(request.requester_node_name, "alice-phone");
    }
}
