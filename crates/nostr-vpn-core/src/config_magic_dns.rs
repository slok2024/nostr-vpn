use std::collections::{HashMap, HashSet};

const LEGACY_DEFAULT_NODE_NAME: &str = "nostr-vpn-node";

pub(crate) fn default_network_name(ordinal: usize) -> String {
    format!("Network {ordinal}")
}

pub(crate) fn default_network_entry_id(ordinal: usize) -> String {
    format!("network-{ordinal}")
}

pub(crate) fn normalize_network_entry_id(value: &str, ordinal: usize) -> String {
    normalize_magic_dns_label(value).unwrap_or_else(|| default_network_entry_id(ordinal))
}

pub(crate) fn uniquify_network_entry_id(
    candidate: String,
    used_ids: &mut HashSet<String>,
) -> String {
    if used_ids.insert(candidate.clone()) {
        return candidate;
    }

    let base = candidate;
    let mut suffix = 2_usize;
    loop {
        let next = format!("{base}-{suffix}");
        if used_ids.insert(next.clone()) {
            return next;
        }
        suffix += 1;
    }
}

pub(crate) fn default_magic_dns_suffix() -> String {
    "nvpn".to_string()
}

pub(crate) fn default_peer_aliases() -> HashMap<String, String> {
    HashMap::new()
}

pub(crate) fn default_node_name() -> String {
    LEGACY_DEFAULT_NODE_NAME.to_string()
}

pub(crate) fn uses_default_node_name(value: &str, own_pubkey_hex: Option<&str>) -> bool {
    let trimmed = value.trim();
    trimmed.is_empty()
        || trimmed == LEGACY_DEFAULT_NODE_NAME
        || looks_like_generated_hex_label(trimmed)
        || own_pubkey_hex
            .map(|pubkey_hex| trimmed == default_node_name_for_pubkey(pubkey_hex))
            .unwrap_or(false)
}

pub fn default_node_name_for_pubkey(pubkey_hex: &str) -> String {
    default_pubkey_label("device", pubkey_hex, &HashSet::new())
}

pub fn default_node_name_from_hostname(hostname: &str) -> Option<String> {
    let first_label = hostname
        .trim()
        .trim_matches('.')
        .split('.')
        .find(|label| !label.trim().is_empty())?;
    let normalized = normalize_magic_dns_label(first_label)?;
    if normalized == "localhost" || looks_like_generated_hex_label(&normalized) {
        return None;
    }
    Some(normalized)
}

pub fn default_node_name_for_hostname_or_pubkey(
    hostname: Option<&str>,
    pubkey_hex: &str,
) -> String {
    hostname
        .and_then(default_node_name_from_hostname)
        .unwrap_or_else(|| default_node_name_for_pubkey(pubkey_hex))
}

pub(crate) fn detected_hostname() -> Option<String> {
    let hostname = hostname::get().ok()?;
    Some(hostname.to_string_lossy().into_owned())
}

fn looks_like_generated_hex_label(value: &str) -> bool {
    let trimmed = value.trim();
    (12..=64).contains(&trimmed.len()) && trimmed.chars().all(|ch| ch.is_ascii_hexdigit())
}

pub fn normalize_magic_dns_suffix(value: &str) -> String {
    let mut normalized_labels = value
        .trim()
        .trim_end_matches('.')
        .split('.')
        .filter_map(normalize_magic_dns_label)
        .collect::<Vec<_>>();
    normalized_labels.retain(|label| !label.is_empty());

    if normalized_labels.is_empty() {
        return default_magic_dns_suffix();
    }

    normalized_labels.join(".")
}

pub fn normalize_magic_dns_label(value: &str) -> Option<String> {
    let mut label = String::new();
    let mut previous_dash = false;

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            label.push(ch.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash {
            label.push('-');
            previous_dash = true;
        }
    }

    while label.ends_with('-') {
        label.pop();
    }
    while label.starts_with('-') {
        label.remove(0);
    }

    if label.is_empty() {
        return None;
    }

    if label.len() > 63 {
        label.truncate(63);
        while label.ends_with('-') {
            label.pop();
        }
    }

    if label.is_empty() { None } else { Some(label) }
}

pub fn default_magic_dns_label_for_pubkey(
    pubkey_hex: &str,
    used_aliases: &HashSet<String>,
) -> String {
    default_pubkey_label("peer", pubkey_hex, used_aliases)
}

fn default_pubkey_label(prefix: &str, pubkey_hex: &str, used_aliases: &HashSet<String>) -> String {
    let hex = pubkey_hex
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .map(|ch| ch.to_ascii_lowercase())
        .collect::<String>();

    for len in [12_usize, 16, 20, 32, 64] {
        let short = hex.chars().take(len).collect::<String>();
        if short.is_empty() {
            break;
        }
        let candidate = format!("{prefix}-{short}");
        if !used_aliases.contains(&candidate) {
            return candidate;
        }
        if short.len() == hex.len() {
            break;
        }
    }

    let base = if hex.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}-{hex}")
    };
    if !used_aliases.contains(&base) {
        return base;
    }

    for counter in 2..10_000 {
        let candidate = format!("{base}-{counter}");
        if !used_aliases.contains(&candidate) {
            return candidate;
        }
    }

    base
}

pub(crate) fn uniquify_magic_dns_label(mut base: String, used: &mut HashSet<String>) -> String {
    if base.is_empty() {
        base = "peer".to_string();
    }

    if !used.contains(&base) {
        used.insert(base.clone());
        return base;
    }

    for counter in 2..10_000 {
        let suffix = format!("-{counter}");
        let max_base_len = 63usize.saturating_sub(suffix.len());
        let mut candidate_base = base.clone();
        if candidate_base.len() > max_base_len {
            candidate_base.truncate(max_base_len);
            while candidate_base.ends_with('-') {
                candidate_base.pop();
            }
        }
        let candidate = format!("{candidate_base}{suffix}");
        if !used.contains(&candidate) {
            used.insert(candidate.clone());
            return candidate;
        }
    }

    base
}
