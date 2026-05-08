use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use anyhow::{Context, Result, anyhow};
use nostr_vpn_core::config::WireGuardExitConfig;

const WIREGUARD_EXIT_TABLE: u32 = 51_888;
const WIREGUARD_EXIT_RULE_PRIORITY: u32 = 10_888;

pub(crate) fn validate_linux_wireguard_exit_config(config: &WireGuardExitConfig) -> Result<String> {
    if !config.enabled {
        return Err(anyhow!("WireGuard exit upstream is disabled"));
    }
    let iface = config.interface.trim();
    if !linux_iface_name_is_safe(iface) {
        return Err(anyhow!("invalid WireGuard exit interface '{iface}'"));
    }
    if config.address.trim().is_empty() {
        return Err(anyhow!(
            "WireGuard exit upstream is missing a tunnel address"
        ));
    }
    if config.private_key.trim().is_empty() {
        return Err(anyhow!("WireGuard exit upstream is missing a private key"));
    }
    if config.peer_public_key.trim().is_empty() {
        return Err(anyhow!(
            "WireGuard exit upstream is missing a peer public key"
        ));
    }
    if config.endpoint.trim().is_empty() {
        return Err(anyhow!(
            "WireGuard exit upstream is missing a peer endpoint"
        ));
    }
    if !config.allowed_ips.iter().any(|route| route == "0.0.0.0/0") {
        return Err(anyhow!(
            "WireGuard exit upstream allowed IPs must include 0.0.0.0/0"
        ));
    }
    Ok(iface.to_string())
}

pub(crate) fn linux_wireguard_exit_ipv6_default(config: &WireGuardExitConfig) -> bool {
    config.allowed_ips.iter().any(|route| route == "::/0")
        && config
            .address
            .split('/')
            .next()
            .is_some_and(|ip| ip.contains(':'))
}

pub(crate) fn apply_linux_wireguard_exit_upstream(
    config: &WireGuardExitConfig,
    source_cidr: &str,
) -> Result<crate::LinuxWireGuardExitRuntime> {
    let iface = validate_linux_wireguard_exit_config(config)?;
    let created_interface = ensure_linux_wireguard_link(&iface)?;
    let private_key_file = write_temp_secret_file(&iface, "key", &config.private_key)?;
    let psk_file = if config.peer_preshared_key.trim().is_empty() {
        None
    } else {
        Some(write_temp_secret_file(
            &iface,
            "psk",
            &config.peer_preshared_key,
        )?)
    };

    let result = apply_linux_wireguard_exit_upstream_inner(
        config,
        &iface,
        source_cidr,
        &private_key_file,
        psk_file.as_ref(),
    );

    let _ = fs::remove_file(&private_key_file);
    if let Some(psk_file) = psk_file {
        let _ = fs::remove_file(psk_file);
    }

    result.map(|()| crate::LinuxWireGuardExitRuntime {
        interface: iface,
        source_cidr: source_cidr.to_string(),
        table: WIREGUARD_EXIT_TABLE,
        priority: WIREGUARD_EXIT_RULE_PRIORITY,
        created_interface,
    })
}

fn apply_linux_wireguard_exit_upstream_inner(
    config: &WireGuardExitConfig,
    iface: &str,
    source_cidr: &str,
    private_key_file: &PathBuf,
    psk_file: Option<&PathBuf>,
) -> Result<()> {
    crate::run_checked(
        ProcessCommand::new("ip")
            .arg("address")
            .arg("replace")
            .arg(config.address.trim())
            .arg("dev")
            .arg(iface),
    )?;

    let mut wg = ProcessCommand::new("wg");
    wg.arg("set")
        .arg(iface)
        .arg("private-key")
        .arg(private_key_file)
        .arg("peer")
        .arg(config.peer_public_key.trim())
        .arg("allowed-ips")
        .arg(config.allowed_ips.join(","))
        .arg("endpoint")
        .arg(config.endpoint.trim());
    if let Some(psk_file) = psk_file {
        wg.arg("preshared-key").arg(psk_file);
    }
    if config.persistent_keepalive_secs > 0 {
        wg.arg("persistent-keepalive")
            .arg(config.persistent_keepalive_secs.to_string());
    }
    crate::run_checked(&mut wg)?;

    crate::run_checked(
        ProcessCommand::new("ip")
            .arg("link")
            .arg("set")
            .arg("mtu")
            .arg(config.mtu.to_string())
            .arg("up")
            .arg("dev")
            .arg(iface),
    )?;

    crate::run_checked(
        ProcessCommand::new("ip")
            .arg("-4")
            .arg("route")
            .arg("replace")
            .arg("default")
            .arg("dev")
            .arg(iface)
            .arg("table")
            .arg(WIREGUARD_EXIT_TABLE.to_string()),
    )?;
    ensure_linux_wireguard_exit_policy_rule(source_cidr)?;
    crate::flush_linux_route_cache()
}

pub(crate) fn cleanup_linux_wireguard_exit_upstream(runtime: &crate::LinuxWireGuardExitRuntime) {
    let _ = crate::run_checked(
        ProcessCommand::new("ip")
            .arg("-4")
            .arg("rule")
            .arg("del")
            .arg("priority")
            .arg(runtime.priority.to_string())
            .arg("from")
            .arg(&runtime.source_cidr)
            .arg("table")
            .arg(runtime.table.to_string()),
    );
    let _ = crate::run_checked(
        ProcessCommand::new("ip")
            .arg("-4")
            .arg("route")
            .arg("flush")
            .arg("table")
            .arg(runtime.table.to_string()),
    );
    if runtime.created_interface {
        let _ = crate::run_checked(
            ProcessCommand::new("ip")
                .arg("link")
                .arg("del")
                .arg("dev")
                .arg(&runtime.interface),
        );
    }
    let _ = crate::flush_linux_route_cache();
}

fn ensure_linux_wireguard_link(iface: &str) -> Result<bool> {
    let exists = ProcessCommand::new("ip")
        .arg("link")
        .arg("show")
        .arg("dev")
        .arg(iface)
        .status()
        .with_context(|| "failed to inspect WireGuard exit interface")?
        .success();
    if exists {
        return Ok(false);
    }

    crate::run_checked(
        ProcessCommand::new("ip")
            .arg("link")
            .arg("add")
            .arg("dev")
            .arg(iface)
            .arg("type")
            .arg("wireguard"),
    )?;
    Ok(true)
}

fn ensure_linux_wireguard_exit_policy_rule(source_cidr: &str) -> Result<()> {
    let output =
        crate::command_stdout_checked(ProcessCommand::new("ip").arg("-4").arg("rule").arg("show"))?;
    if linux_wireguard_exit_policy_rule_exists(
        &output,
        source_cidr,
        WIREGUARD_EXIT_TABLE,
        WIREGUARD_EXIT_RULE_PRIORITY,
    ) {
        return Ok(());
    }
    crate::run_checked(
        ProcessCommand::new("ip")
            .arg("-4")
            .arg("rule")
            .arg("add")
            .arg("priority")
            .arg(WIREGUARD_EXIT_RULE_PRIORITY.to_string())
            .arg("from")
            .arg(source_cidr)
            .arg("table")
            .arg(WIREGUARD_EXIT_TABLE.to_string()),
    )
}

pub(crate) fn linux_wireguard_exit_policy_rule_exists(
    output: &str,
    source_cidr: &str,
    table: u32,
    priority: u32,
) -> bool {
    let priority_prefix = format!("{priority}:");
    let table_lookup = format!("lookup {table}");
    output.lines().any(|line| {
        let line = line.trim();
        line.starts_with(&priority_prefix)
            && line.contains("from ")
            && line.contains(source_cidr)
            && line.contains(&table_lookup)
    })
}

fn write_temp_secret_file(iface: &str, suffix: &str, secret: &str) -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("nvpn-{iface}-{suffix}-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    file.write_all(secret.trim().as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("failed to write {}", path.display()))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to restrict {}", path.display()))?;
    Ok(path)
}

fn linux_iface_name_is_safe(iface: &str) -> bool {
    !iface.is_empty()
        && iface.len() <= 15
        && iface
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::linux_wireguard_exit_policy_rule_exists;

    #[test]
    fn policy_rule_parser_matches_exact_managed_rule() {
        let output = "0:\tfrom all lookup local\n10888:\tfrom 10.44.0.0/16 lookup 51888\n32766:\tfrom all lookup main\n";

        assert!(linux_wireguard_exit_policy_rule_exists(
            output,
            "10.44.0.0/16",
            51_888,
            10_888
        ));
        assert!(!linux_wireguard_exit_policy_rule_exists(
            output,
            "10.45.0.0/16",
            51_888,
            10_888
        ));
    }
}
