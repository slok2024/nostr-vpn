use super::*;

pub(super) fn macos_default_routes_from_netstat(output: &str) -> Vec<MacosRouteSpec> {
    let mut routes = Vec::new();

    for line in output.lines().map(str::trim) {
        let tokens = line.split_whitespace().collect::<Vec<_>>();
        if tokens.first().copied() != Some("default") || tokens.len() < 4 {
            continue;
        }

        let iface_index = if tokens.last().copied() == Some("!") {
            tokens.len().saturating_sub(2)
        } else {
            tokens.len().saturating_sub(1)
        };
        let Some(interface) = tokens.get(iface_index) else {
            continue;
        };

        routes.push(MacosRouteSpec {
            gateway: (!tokens[1].starts_with("link#")).then(|| tokens[1].to_string()),
            interface: (*interface).to_string(),
        });
    }

    routes
}

pub(crate) fn macos_has_underlay_default_route(output: &str) -> bool {
    macos_underlay_default_route_from_routes(&macos_default_routes_from_netstat(output)).is_some()
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn macos_has_tunnel_split_default_routes(output: &str) -> bool {
    output.lines().map(str::trim).any(|line| {
        let tokens = line.split_whitespace().collect::<Vec<_>>();
        if tokens.len() < 4 {
            return false;
        }

        let target = tokens[0];
        let iface_index = if tokens.last().copied() == Some("!") {
            tokens.len().saturating_sub(2)
        } else {
            tokens.len().saturating_sub(1)
        };
        let Some(interface) = tokens.get(iface_index).copied() else {
            return false;
        };

        interface.starts_with("utun")
            && matches!(target, "0/1" | "0.0.0.0/1" | "128/1" | "128.0.0.0/1")
    })
}

pub(super) fn macos_underlay_default_route_from_routes(
    routes: &[MacosRouteSpec],
) -> Option<MacosRouteSpec> {
    routes
        .iter()
        .find(|route| {
            route.gateway.is_some()
                && !route.interface.starts_with("utun")
                && !route.interface.starts_with("bridge")
                && route.interface != "lo0"
        })
        .cloned()
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn macos_interface_names_from_ifconfig_list(output: &str) -> Vec<String> {
    output
        .split_whitespace()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn macos_ipconfig_router_from_output(output: &str) -> Option<Ipv4Addr> {
    for line in output.lines().map(str::trim) {
        let value = if let Some(value) = line.strip_prefix("router (ip):") {
            value.trim()
        } else if let Some(value) = line.strip_prefix("router (ip_mult):") {
            value.trim().trim_start_matches('{').trim_end_matches('}')
        } else {
            continue;
        };

        if let Ok(router) = value.parse::<Ipv4Addr>() {
            return Some(router);
        }
    }

    None
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_ipconfig_ipv4_for_interface(iface: &str) -> Result<Option<Ipv4Addr>> {
    match command_stdout_checked(ProcessCommand::new("ipconfig").arg("getifaddr").arg(iface)) {
        Ok(output) => Ok(output.trim().parse::<Ipv4Addr>().ok()),
        Err(error) => {
            if error.to_string().to_ascii_lowercase().contains("not found") {
                Ok(None)
            } else {
                Err(error)
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_ipconfig_router_for_interface(iface: &str) -> Result<Option<Ipv4Addr>> {
    if let Ok(output) = command_stdout_checked(
        ProcessCommand::new("ipconfig")
            .arg("getoption")
            .arg(iface)
            .arg("router"),
    ) && let Ok(router) = output.trim().parse::<Ipv4Addr>()
    {
        return Ok(Some(router));
    }

    let output =
        command_stdout_checked(ProcessCommand::new("ipconfig").arg("getpacket").arg(iface))?;
    Ok(macos_ipconfig_router_from_output(&output))
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_underlay_default_route_from_system() -> Result<Option<MacosRouteSpec>> {
    let output = command_stdout_checked(ProcessCommand::new("ifconfig").arg("-l"))?;
    for iface in macos_interface_names_from_ifconfig_list(&output) {
        if iface.starts_with("utun")
            || iface.starts_with("bridge")
            || iface == "lo0"
            || iface == "gif0"
            || iface == "stf0"
            || iface.starts_with("anpi")
            || iface.starts_with("awdl")
            || iface.starts_with("llw")
        {
            continue;
        }

        let Ok(Some(_ipv4)) = macos_ipconfig_ipv4_for_interface(&iface) else {
            continue;
        };
        let Ok(Some(router)) = macos_ipconfig_router_for_interface(&iface) else {
            continue;
        };

        return Ok(Some(MacosRouteSpec {
            gateway: Some(router.to_string()),
            interface: iface,
        }));
    }

    Ok(None)
}

#[cfg(target_os = "macos")]
fn macos_unscoped_default_route_works() -> bool {
    ProcessCommand::new("route")
        .arg("-n")
        .arg("get")
        .arg("1.1.1.1")
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_tunnel_interfaces_with_ipv4(tunnel_ip: Ipv4Addr) -> Result<Vec<String>> {
    let output = command_stdout_checked(ProcessCommand::new("ifconfig").arg("-l"))?;
    let mut matches = Vec::new();
    for iface in macos_interface_names_from_ifconfig_list(&output) {
        if !iface.starts_with("utun") {
            continue;
        }
        if macos_iface_has_ipv4_address(&iface, tunnel_ip)? {
            matches.push(iface);
        }
    }
    Ok(matches)
}

#[cfg(target_os = "macos")]
pub(crate) fn renew_macos_interface_dhcp(iface: &str) -> Result<()> {
    run_checked(
        ProcessCommand::new("ipconfig")
            .arg("set")
            .arg(iface)
            .arg("DHCP"),
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn ensure_macos_underlay_default_route() -> Result<bool> {
    let output = command_stdout_checked(
        ProcessCommand::new("netstat")
            .arg("-rn")
            .arg("-f")
            .arg("inet"),
    )?;
    if macos_has_tunnel_split_default_routes(&output)
        || (macos_has_underlay_default_route(&output) && macos_unscoped_default_route_works())
    {
        return Ok(false);
    }

    let Some(underlay) = macos_underlay_default_route_from_system()? else {
        return Ok(false);
    };

    if restore_macos_default_route(&underlay).is_ok() {
        let refreshed_output = command_stdout_checked(
            ProcessCommand::new("netstat")
                .arg("-rn")
                .arg("-f")
                .arg("inet"),
        )?;
        if macos_has_underlay_default_route(&refreshed_output) {
            return Ok(true);
        }
    }

    let _ = renew_macos_interface_dhcp(&underlay.interface);
    let refreshed_output = command_stdout_checked(
        ProcessCommand::new("netstat")
            .arg("-rn")
            .arg("-f")
            .arg("inet"),
    )?;
    if macos_has_underlay_default_route(&refreshed_output)
        || macos_has_tunnel_split_default_routes(&refreshed_output)
    {
        return Ok(true);
    }

    restore_macos_default_route(&underlay)?;
    Ok(true)
}

#[cfg(target_os = "macos")]
pub(super) fn macos_default_routes() -> Result<Vec<MacosRouteSpec>> {
    let output = command_stdout_checked(
        ProcessCommand::new("netstat")
            .arg("-rn")
            .arg("-f")
            .arg("inet"),
    )?;
    Ok(macos_default_routes_from_netstat(&output))
}

#[cfg(target_os = "macos")]
pub(super) fn delete_macos_managed_route(
    target: &str,
    gateway: Option<&str>,
    interface: Option<&str>,
) -> Result<()> {
    if gateway.is_none()
        && let Some(iface) = interface
    {
        return delete_macos_direct_route_variants(target, iface);
    }

    delete_macos_route_spec(target, None)
}

#[cfg(target_os = "macos")]
pub(super) fn restore_macos_default_route(route: &MacosRouteSpec) -> Result<()> {
    apply_macos_default_route(route.gateway.as_deref(), Some(route.interface.as_str()))
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn macos_tunnel_default_route_targets() -> &'static [&'static str] {
    &["0.0.0.0/1", "128.0.0.0/1"]
}

#[cfg(any(target_os = "macos", test))]
fn macos_gateway_route_args(action: &str, target: &str, gateway: &str) -> Vec<String> {
    let target_ip = strip_cidr(target);
    let is_host = target.ends_with("/32") || !target.contains('/');

    let mut args = vec!["-n".to_string(), action.to_string()];
    if is_host {
        args.push("-host".to_string());
        args.push(target_ip.to_string());
    } else if target == "0.0.0.0/0" {
        args.push("default".to_string());
    } else {
        args.push("-net".to_string());
        args.push(target.to_string());
    }
    args.push(gateway.to_string());
    args
}

#[cfg(test)]
pub(crate) fn macos_gateway_route_args_for_test(
    action: &str,
    target: &str,
    gateway: &str,
) -> Vec<String> {
    macos_gateway_route_args(action, target, gateway)
}

#[cfg(target_os = "macos")]
pub(super) fn apply_macos_default_route(
    gateway: Option<&str>,
    ifscope: Option<&str>,
) -> Result<()> {
    if let Some(ifscope) = ifscope {
        let _ = delete_macos_default_route_for_interface(ifscope);
    }

    if gateway.is_none() {
        let iface = ifscope.ok_or_else(|| anyhow!("missing interface for direct default route"))?;
        for target in macos_tunnel_default_route_targets() {
            apply_macos_route_spec(target, None, Some(iface)).with_context(|| {
                format!("failed to install macOS default route target {target} on {iface}")
            })?;
        }
        return Ok(());
    }

    let mut change = ProcessCommand::new("route");
    change.arg("-n").arg("change").arg("default");
    change.arg(gateway.expect("gateway checked above"));

    match run_checked(&mut change) {
        Ok(()) => Ok(()),
        Err(_) => {
            let mut add = ProcessCommand::new("route");
            add.arg("-n").arg("add").arg("default");
            add.arg(gateway.expect("gateway checked above"));
            run_checked(&mut add)
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn delete_macos_default_route_for_interface(iface: &str) -> Result<()> {
    let mut failures = Vec::new();
    for target in
        std::iter::once("0.0.0.0/0").chain(macos_tunnel_default_route_targets().iter().copied())
    {
        if let Err(error) = delete_macos_direct_route_variants(target, iface) {
            failures.push(format!("remove {target} on {iface}: {error}"));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(failures.join("; ")))
    }
}

pub(super) fn macos_ifconfig_has_ipv4(output: &str, needle: Ipv4Addr) -> bool {
    output.lines().map(str::trim).any(|line| {
        line.strip_prefix("inet ")
            .and_then(|rest| rest.split_whitespace().next())
            .is_some_and(|value| value == needle.to_string())
    })
}

#[cfg(target_os = "macos")]
pub(super) fn macos_iface_has_ipv4_address(iface: &str, needle: Ipv4Addr) -> Result<bool> {
    let output = command_stdout_checked(ProcessCommand::new("ifconfig").arg(iface))?;
    Ok(macos_ifconfig_has_ipv4(&output, needle))
}

#[cfg(target_os = "macos")]
pub(super) fn apply_macos_route_spec(
    target: &str,
    gateway: Option<&str>,
    ifscope: Option<&str>,
) -> Result<()> {
    let target_ip = strip_cidr(target);
    let is_host = target.ends_with("/32") || !target.contains('/');

    let mut add = ProcessCommand::new("route");
    if let Some(gateway) = gateway {
        add.args(macos_gateway_route_args("add", target, gateway));
    } else {
        if let Some(iface) = ifscope {
            let _ = delete_macos_route_spec(target, Some(iface));
            let _ = delete_macos_route_spec(target, None);
        }
        add.arg("-n").arg("add");
        if is_host {
            add.arg("-host").arg(target_ip);
        } else if target == "0.0.0.0/0" {
            add.arg("default");
        } else {
            add.arg("-net").arg(target);
        }
        let iface = ifscope.ok_or_else(|| anyhow!("missing interface for direct route"))?;
        add.arg("-interface").arg(iface);
    }

    match run_checked(&mut add) {
        Ok(()) => Ok(()),
        Err(_) => {
            let mut change = ProcessCommand::new("route");
            if let Some(gateway) = gateway {
                change.args(macos_gateway_route_args("change", target, gateway));
            } else {
                change.arg("-n").arg("change");
                if is_host {
                    change.arg("-host").arg(target_ip);
                } else if target == "0.0.0.0/0" {
                    change.arg("default");
                } else {
                    change.arg("-net").arg(target);
                }
                let iface = ifscope.ok_or_else(|| anyhow!("missing interface for direct route"))?;
                change.arg("-interface").arg(iface);
            }
            run_checked(&mut change)
        }
    }
}

#[cfg(target_os = "macos")]
fn delete_macos_route_spec(target: &str, ifscope: Option<&str>) -> Result<()> {
    let target_ip = strip_cidr(target);
    let is_host = target.ends_with("/32") || !target.contains('/');

    let mut delete = ProcessCommand::new("route");
    delete.arg("-n").arg("delete");
    if let Some(ifscope) = ifscope {
        delete.arg("-ifscope").arg(ifscope);
    }
    if is_host {
        delete.arg("-host").arg(target_ip);
    } else if target == "0.0.0.0/0" {
        delete.arg("default");
    } else {
        delete.arg("-net").arg(target);
    }

    run_checked(&mut delete)
}

#[cfg(target_os = "macos")]
fn delete_macos_direct_route_variants(target: &str, iface: &str) -> Result<()> {
    let mut failures = Vec::new();

    for attempt in [Some(iface), None] {
        if let Err(error) = delete_macos_route_spec(target, attempt)
            && !crate::daemon_runtime::macos_route_delete_error_is_absent(&error.to_string())
        {
            failures.push(error.to_string());
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(failures.join("; ")))
    }
}

#[cfg(target_os = "macos")]
pub(super) fn read_macos_ip_forward() -> Result<bool> {
    Ok(command_stdout_checked(
        ProcessCommand::new("sysctl")
            .arg("-n")
            .arg("net.inet.ip.forwarding"),
    )?
    .trim()
        == "1")
}

#[cfg(target_os = "macos")]
pub(super) fn write_macos_ip_forward(enabled: bool) -> Result<()> {
    run_checked(ProcessCommand::new("sysctl").arg("-w").arg(format!(
        "net.inet.ip.forwarding={}",
        if enabled { "1" } else { "0" }
    )))
}

#[cfg(target_os = "macos")]
const MACOS_PF_EXIT_ANCHOR: &str = "com.apple/to.nostrvpn/exit";

#[cfg(target_os = "macos")]
pub(super) fn cleanup_macos_pf_nat() -> Result<()> {
    run_checked(
        ProcessCommand::new("pfctl")
            .arg("-a")
            .arg(MACOS_PF_EXIT_ANCHOR)
            .arg("-F")
            .arg("nat"),
    )
}
