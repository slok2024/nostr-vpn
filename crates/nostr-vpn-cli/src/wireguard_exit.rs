use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::time::{SystemTime, UNIX_EPOCH};

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
    previous_runtime: Option<&crate::LinuxWireGuardExitRuntime>,
    previous_default_route_hint: Option<&str>,
) -> Result<crate::LinuxWireGuardExitRuntime> {
    let iface = validate_linux_wireguard_exit_config(config)?;
    let created_interface = ensure_linux_wireguard_link(&iface)?;
    let previous_default_route = previous_runtime
        .and_then(|runtime| runtime.previous_default_route.clone())
        .or_else(|| previous_default_route_hint.map(ToOwned::to_owned))
        .or_else(|| {
            crate::linux_default_route()
                .ok()
                .filter(|route| route.dev != iface)
                .map(|route| route.line)
        });
    let endpoint_bypass_specs = linux_wireguard_exit_endpoint_bypass_specs(
        config,
        &iface,
        previous_default_route.as_deref(),
    )?;
    let endpoint_bypass_routes = endpoint_bypass_specs
        .iter()
        .map(|route| route.target.clone())
        .collect::<Vec<_>>();
    let stale_endpoint_bypass_routes = previous_runtime
        .map(|runtime| {
            runtime
                .endpoint_bypass_routes
                .iter()
                .filter(|route| !endpoint_bypass_routes.contains(route))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
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
        &endpoint_bypass_specs,
    );

    let _ = fs::remove_file(&private_key_file);
    if let Some(psk_file) = psk_file {
        let _ = fs::remove_file(psk_file);
    }

    result.map(|()| {
        for route in stale_endpoint_bypass_routes {
            let _ = crate::delete_linux_endpoint_bypass_route(&route);
        }
        crate::LinuxWireGuardExitRuntime {
            interface: iface,
            source_cidr: source_cidr.to_string(),
            table: WIREGUARD_EXIT_TABLE,
            priority: WIREGUARD_EXIT_RULE_PRIORITY,
            created_interface,
            endpoint_bypass_routes,
            previous_default_route,
        }
    })
}

fn apply_linux_wireguard_exit_upstream_inner(
    config: &WireGuardExitConfig,
    iface: &str,
    source_cidr: &str,
    private_key_file: &PathBuf,
    psk_file: Option<&PathBuf>,
    endpoint_bypass_specs: &[crate::LinuxEndpointBypassRoute],
) -> Result<()> {
    crate::run_checked(
        ProcessCommand::new("ip")
            .arg("address")
            .arg("replace")
            .arg(config.address.trim())
            .arg("dev")
            .arg(iface),
    )?;
    for route in endpoint_bypass_specs {
        crate::apply_linux_endpoint_bypass_route(route)?;
    }

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

    apply_linux_wireguard_exit_default_route(iface, config.address.trim())?;

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
    restore_linux_wireguard_exit_default_route(runtime);
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
    for route in &runtime.endpoint_bypass_routes {
        let _ = crate::delete_linux_endpoint_bypass_route(route);
    }
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

fn linux_wireguard_exit_endpoint_bypass_specs(
    config: &WireGuardExitConfig,
    iface: &str,
    previous_default_route: Option<&str>,
) -> Result<Vec<crate::LinuxEndpointBypassRoute>> {
    let hosts = wireguard_exit_endpoint_ipv4_hosts(&config.endpoint);
    if hosts.is_empty() {
        return Ok(Vec::new());
    }
    crate::linux_bypass_route_specs_for_hosts(hosts, iface, previous_default_route)
}

fn apply_linux_wireguard_exit_default_route(iface: &str, address: &str) -> Result<()> {
    let mut command = ProcessCommand::new("ip");
    command
        .arg("-4")
        .arg("route")
        .arg("replace")
        .arg("default")
        .arg("dev")
        .arg(iface);
    if let Ok(source) = crate::strip_cidr(address).parse::<Ipv4Addr>() {
        command.arg("src").arg(source.to_string());
    }
    crate::run_checked(&mut command)
}

fn restore_linux_wireguard_exit_default_route(runtime: &crate::LinuxWireGuardExitRuntime) {
    let current_is_wireguard = crate::linux_default_route()
        .map(|route| route.dev == runtime.interface)
        .unwrap_or(true);
    if !current_is_wireguard {
        return;
    }

    if let Some(route) = runtime.previous_default_route.as_deref() {
        if let Err(error) = crate::restore_linux_default_route(route) {
            eprintln!("fips: failed to restore pre-WireGuard default route: {error}");
        }
    } else if let Err(error) = delete_linux_wireguard_exit_default_route(&runtime.interface) {
        eprintln!("fips: failed to delete WireGuard exit default route: {error}");
    }
}

fn delete_linux_wireguard_exit_default_route(iface: &str) -> Result<()> {
    crate::run_checked(
        ProcessCommand::new("ip")
            .arg("-4")
            .arg("route")
            .arg("del")
            .arg("default")
            .arg("dev")
            .arg(iface),
    )
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

fn wireguard_exit_endpoint_ipv4_hosts(endpoint: &str) -> Vec<Ipv4Addr> {
    let Some((host, port)) = crate::split_host_port(endpoint, 51820) else {
        return Vec::new();
    };
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return vec![ip];
    }
    if host.parse::<IpAddr>().is_ok() {
        return Vec::new();
    }

    let mut hosts = (host.as_str(), port)
        .to_socket_addrs()
        .map(|addrs| {
            addrs
                .filter_map(|addr| match addr.ip() {
                    IpAddr::V4(ip) => Some(ip),
                    IpAddr::V6(_) => None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    hosts.sort_unstable();
    hosts.dedup();
    hosts
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
    let temp_dir = std::env::temp_dir();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    for attempt in 0..128u32 {
        let path = temp_dir.join(format!(
            "nvpn-{iface}-{suffix}-{}-{nonce}-{attempt}",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true).mode(0o600);
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("failed to create {}", path.display()));
            }
        };
        if let Err(error) = file.write_all(secret.trim().as_bytes()) {
            let _ = fs::remove_file(&path);
            return Err(error).with_context(|| format!("failed to write {}", path.display()));
        }
        if let Err(error) = file.write_all(b"\n") {
            let _ = fs::remove_file(&path);
            return Err(error).with_context(|| format!("failed to write {}", path.display()));
        }
        file.flush()
            .with_context(|| format!("failed to flush {}", path.display()))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to restrict {}", path.display()))?;
        return Ok(path);
    }
    Err(anyhow!(
        "failed to allocate unique temp secret file for {iface}"
    ))
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
    use std::net::Ipv4Addr;

    use super::{
        linux_wireguard_exit_policy_rule_exists, wireguard_exit_endpoint_ipv4_hosts,
        write_temp_secret_file,
    };

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

    #[test]
    fn endpoint_bypass_hosts_parse_ipv4_endpoint() {
        assert_eq!(
            wireguard_exit_endpoint_ipv4_hosts("198.51.100.20:51830"),
            vec!["198.51.100.20".parse::<Ipv4Addr>().unwrap()]
        );
    }

    #[test]
    fn temp_secret_files_are_unique_and_private() {
        use std::os::unix::fs::PermissionsExt;

        let first = write_temp_secret_file("nvpn-test", "key", "secret-a").expect("first secret");
        let second = write_temp_secret_file("nvpn-test", "key", "secret-b").expect("second secret");

        assert_ne!(first, second);
        assert_eq!(
            std::fs::read_to_string(&first).expect("read first"),
            "secret-a\n"
        );
        assert_eq!(
            std::fs::metadata(&first)
                .expect("first metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let _ = std::fs::remove_file(first);
        let _ = std::fs::remove_file(second);
    }
}
