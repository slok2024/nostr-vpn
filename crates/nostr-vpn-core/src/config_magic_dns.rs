use std::collections::{HashMap, HashSet};

use sha2::{Digest, Sha256};

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
    default_magic_dns_label_for_pubkey(pubkey_hex, &HashSet::new())
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
    let digest = Sha256::digest(pubkey_hex.as_bytes());
    let mut index =
        ((digest[0] as usize) << 8 | digest[1] as usize) % HASHTREE_ANIMAL_ALIASES.len();
    for _ in 0..HASHTREE_ANIMAL_ALIASES.len() {
        let candidate = HASHTREE_ANIMAL_ALIASES[index];
        if !used_aliases.contains(candidate) {
            return candidate.to_string();
        }
        index = (index + 1) % HASHTREE_ANIMAL_ALIASES.len();
    }

    let short = pubkey_hex.chars().take(12).collect::<String>();
    format!("peer-{short}")
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

// Derived from hashtree animals list:
// - apps/hashtree-cc/src/lib/data/animals.json
// - apps/iris-files/src/utils/data/animals.json
const HASHTREE_ANIMAL_ALIASES: &[&str] = &[
    "aardvark",
    "aardwolf",
    "albatross",
    "alligator",
    "alpaca",
    "anaconda",
    "angelfish",
    "ant",
    "anteater",
    "antelope",
    "ape",
    "armadillo",
    "baboon",
    "badger",
    "barracuda",
    "bat",
    "bear",
    "beaver",
    "bee",
    "beetle",
    "bison",
    "blackbird",
    "boa",
    "boar",
    "bobcat",
    "bonobo",
    "butterfly",
    "buzzard",
    "camel",
    "capybara",
    "cardinal",
    "caribou",
    "carp",
    "cat",
    "catfish",
    "centipede",
    "chameleon",
    "cheetah",
    "chicken",
    "chimpanzee",
    "chinchilla",
    "chipmunk",
    "clam",
    "clownfish",
    "cobra",
    "cockroach",
    "condor",
    "cougar",
    "cow",
    "coyote",
    "crab",
    "crane",
    "crayfish",
    "cricket",
    "crocodile",
    "crow",
    "cuckoo",
    "deer",
    "dingo",
    "dolphin",
    "donkey",
    "dove",
    "dragonfly",
    "duck",
    "eagle",
    "earthworm",
    "echidna",
    "eel",
    "egret",
    "elephant",
    "elk",
    "emu",
    "falcon",
    "ferret",
    "finch",
    "firefly",
    "fish",
    "flamingo",
    "fox",
    "frog",
    "gazelle",
    "gecko",
    "gerbil",
    "giraffe",
    "goat",
    "goldfish",
    "goose",
    "gorilla",
    "grasshopper",
    "grouse",
    "guanaco",
    "gull",
    "hamster",
    "hare",
    "hawk",
    "hedgehog",
    "heron",
    "hippopotamus",
    "hornet",
    "horse",
    "hummingbird",
    "hyena",
    "ibis",
    "iguana",
    "impala",
    "jackal",
    "jaguar",
    "jellyfish",
    "kangaroo",
    "koala",
    "koi",
    "ladybug",
    "lemur",
    "leopard",
    "lion",
    "lizard",
    "llama",
    "lobster",
    "lynx",
    "macaw",
    "magpie",
    "manatee",
    "marten",
    "meerkat",
    "mink",
    "mole",
    "mongoose",
    "monkey",
    "moose",
    "mosquito",
    "moth",
    "mouse",
    "mule",
    "narwhal",
    "newt",
    "nightingale",
    "octopus",
    "opossum",
    "orangutan",
    "orca",
    "ostrich",
    "otter",
    "owl",
    "oyster",
    "panda",
    "panther",
    "parrot",
    "peacock",
    "pelican",
    "penguin",
    "pheasant",
    "pig",
    "pigeon",
    "piranha",
    "platypus",
    "porcupine",
    "porpoise",
    "puffin",
    "python",
    "quail",
    "rabbit",
    "raccoon",
    "ram",
    "rat",
    "raven",
    "reindeer",
    "rhino",
    "salamander",
    "salmon",
    "scorpion",
    "seahorse",
    "seal",
    "shark",
    "sheep",
    "skunk",
    "sloth",
    "snail",
    "snake",
    "sparrow",
    "spider",
    "squid",
    "squirrel",
    "starfish",
    "stork",
    "swan",
    "tapir",
    "termite",
    "tiger",
    "toad",
    "toucan",
    "trout",
    "turkey",
    "turtle",
    "viper",
    "vulture",
    "walrus",
    "wasp",
    "weasel",
    "whale",
    "wildcat",
    "wolf",
    "wombat",
    "woodpecker",
    "yak",
    "zebra",
];
