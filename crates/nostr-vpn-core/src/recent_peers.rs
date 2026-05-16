//! On-disk cache of recently successful non-LAN FIPS peer endpoints.
//!
//! When two peers complete a handshake over a public-routable address, that
//! address is the strongest hint we have that they can reach each other again
//! without first dialing a Nostr relay. Persist a TTL'd snapshot to disk so
//! the daemon can re-seed FIPS with those addresses on the next boot, before
//! relays come up.
//!
//! LAN addresses (RFC1918, CGNAT, link-local, loopback, ULA) are excluded:
//! they're either re-learned via mDNS instantly or genuinely useless after a
//! network move. NAT-traversed source ports are inherently ephemeral; we
//! accept the staleness risk and rely on the FIPS retry path
//! (`initiate_peer_retry_connection`) to prefer just-refreshed overlay
//! adverts over stale statics when a relay becomes available.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

const CURRENT_VERSION: u8 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecentPeerEndpoints {
    #[serde(default = "default_version")]
    version: u8,
    #[serde(default)]
    entries: HashMap<String, Vec<RecentPeerEndpoint>>,
}

fn default_version() -> u8 {
    CURRENT_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentPeerEndpoint {
    pub addr: String,
    pub last_success_at: u64,
}

impl RecentPeerEndpoints {
    pub fn is_empty(&self) -> bool {
        self.entries.values().all(|endpoints| endpoints.is_empty())
    }

    /// Record a successful handshake against `addr` for `participant`.
    ///
    /// Returns `true` if the in-memory state changed (caller should persist
    /// to disk). Returns `false` and ignores LAN, loopback, link-local,
    /// CGNAT, or unparseable addresses — those are not useful as restart
    /// hints.
    pub fn note_success(&mut self, participant: &str, addr: &str, success_at: u64) -> bool {
        let addr = addr.trim();
        if !is_persistable_endpoint(addr) {
            return false;
        }

        let endpoints = self.entries.entry(participant.to_string()).or_default();

        if let Some(existing) = endpoints.iter_mut().find(|entry| entry.addr == addr) {
            if existing.last_success_at >= success_at {
                return false;
            }
            existing.last_success_at = success_at;
            return true;
        }

        endpoints.push(RecentPeerEndpoint {
            addr: addr.to_string(),
            last_success_at: success_at,
        });
        true
    }

    /// Drop entries older than `now - ttl_secs`.
    pub fn prune_stale(&mut self, now: u64, ttl_secs: u64) -> bool {
        if ttl_secs == 0 {
            return false;
        }
        let cutoff = now.saturating_sub(ttl_secs);
        let mut changed = false;

        self.entries.retain(|_, endpoints| {
            let before = endpoints.len();
            endpoints.retain(|entry| entry.last_success_at > cutoff);
            if endpoints.len() != before {
                changed = true;
            }
            !endpoints.is_empty()
        });

        changed
    }

    /// Keep only entries for npubs in `participants`, drop the rest. Used
    /// when peers leave a network so we don't accumulate dead entries.
    pub fn retain_participants(&mut self, participants: &HashSet<String>) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|participant, _| participants.contains(participant));
        self.entries.len() != before
    }

    /// Endpoint strings recorded for a single participant.
    pub fn endpoints_for(&self, participant: &str) -> Vec<String> {
        let mut endpoints: Vec<String> = self
            .entries
            .get(participant)
            .map(|entries| entries.iter().map(|entry| entry.addr.clone()).collect())
            .unwrap_or_default();
        endpoints.sort();
        endpoints.dedup();
        endpoints
    }

    /// Snapshot suitable for merging with `AppConfig.fips_peer_endpoints`
    /// before constructing the FIPS endpoint config: a sorted vector of
    /// `(participant, sorted_endpoints)`.
    pub fn as_static_peer_endpoints(&self) -> Vec<(String, Vec<String>)> {
        let mut out: Vec<(String, Vec<String>)> = self
            .entries
            .iter()
            .filter_map(|(participant, endpoints)| {
                let mut addrs: Vec<String> =
                    endpoints.iter().map(|entry| entry.addr.clone()).collect();
                addrs.sort();
                addrs.dedup();
                if addrs.is_empty() {
                    None
                } else {
                    Some((participant.clone(), addrs))
                }
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Snapshot with per-address freshness preserved. fips's dialer ranks
    /// `PeerAddress` candidates by `seen_at_ms` descending — by handing it
    /// the cache's `last_success_at_ms` we let recently-working endpoints
    /// race ahead of unstamped operator-supplied hints in the same pass,
    /// without any source-aware preference logic.
    ///
    /// `last_success_at` is stored in seconds; the returned value is in
    /// milliseconds to match fips's `PeerAddress::seen_at_ms` unit.
    pub fn as_static_peer_endpoints_with_seen_at(&self) -> Vec<(String, Vec<(String, u64)>)> {
        let mut out: Vec<(String, Vec<(String, u64)>)> = self
            .entries
            .iter()
            .filter_map(|(participant, endpoints)| {
                let mut addrs: Vec<(String, u64)> = endpoints
                    .iter()
                    .map(|entry| {
                        (
                            entry.addr.clone(),
                            entry.last_success_at.saturating_mul(1000),
                        )
                    })
                    .collect();
                addrs.sort_by(|a, b| a.0.cmp(&b.0));
                addrs.dedup_by(|a, b| a.0 == b.0);
                if addrs.is_empty() {
                    None
                } else {
                    Some((participant.clone(), addrs))
                }
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn to_json_pretty(&self) -> serde_json::Result<String> {
        let snapshot = SerializedRecentPeers {
            version: CURRENT_VERSION,
            entries: &self.entries,
        };
        serde_json::to_string_pretty(&snapshot)
    }

    pub fn from_json(raw: &str) -> serde_json::Result<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(Self::default());
        }
        let mut parsed: Self = serde_json::from_str(trimmed)?;
        if parsed.version == 0 {
            parsed.version = CURRENT_VERSION;
        }
        Ok(parsed)
    }
}

#[derive(Serialize)]
struct SerializedRecentPeers<'a> {
    version: u8,
    entries: &'a HashMap<String, Vec<RecentPeerEndpoint>>,
}

/// True for addresses we'd actually want to retry across a daemon restart:
/// IP literals with a port, on public-routable space.
fn is_persistable_endpoint(addr: &str) -> bool {
    if addr.is_empty() {
        return false;
    }
    let Ok(socket_addr) = addr.parse::<std::net::SocketAddr>() else {
        return false;
    };
    if socket_addr.port() == 0 {
        return false;
    }
    !is_private_or_local_ip(&socket_addr.ip())
}

fn is_private_or_local_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            // Documentation prefixes (192.0.2.0/24, 198.51.100.0/24,
            // 203.0.113.0/24) are intentionally NOT excluded — they're
            // unroutable on the public internet by convention but FIPS
            // can legitimately handshake against them in test overlays.
            if v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_multicast()
            {
                return true;
            }
            let octets = v4.octets();
            // RFC 6598 CGNAT 100.64.0.0/10
            if octets[0] == 100 && (64..=127).contains(&octets[1]) {
                return true;
            }
            // RFC 2544 benchmarking 198.18.0.0/15
            if octets[0] == 198 && matches!(octets[1], 18 | 19) {
                return true;
            }
            false
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || v6.is_unicast_link_local()
                || v6.is_unique_local()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_ipv4_is_persistable() {
        assert!(is_persistable_endpoint("203.0.113.5:51820"));
        assert!(is_persistable_endpoint("8.8.8.8:53"));
    }

    #[test]
    fn private_ranges_are_excluded() {
        assert!(!is_persistable_endpoint("10.0.0.1:51820"));
        assert!(!is_persistable_endpoint("192.168.1.1:51820"));
        assert!(!is_persistable_endpoint("172.16.0.1:51820"));
        assert!(!is_persistable_endpoint("100.64.0.1:51820"));
        assert!(!is_persistable_endpoint("198.18.0.1:51820"));
        assert!(!is_persistable_endpoint("127.0.0.1:51820"));
        assert!(!is_persistable_endpoint("[fe80::1]:51820"));
        assert!(!is_persistable_endpoint("[fd00::1]:51820"));
    }

    #[test]
    fn malformed_or_zero_port_rejected() {
        assert!(!is_persistable_endpoint(""));
        assert!(!is_persistable_endpoint("203.0.113.5"));
        assert!(!is_persistable_endpoint("203.0.113.5:0"));
        assert!(!is_persistable_endpoint("not-an-address"));
    }
}
