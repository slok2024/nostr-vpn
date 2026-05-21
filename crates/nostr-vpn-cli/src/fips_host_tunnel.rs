use anyhow::{Context, Result, anyhow};
use fips_core::config::{
    IdentityConfig, NostrDiscoveryPolicy, RoutingMode, TcpConfig, TransportInstances, UdpConfig,
};
use fips_core::upper::tun::TunState;
use fips_core::{Config, Identity, Node};
use nostr_vpn_core::config::AppConfig;
use std::fs;
#[cfg(target_os = "linux")]
use std::io::Write;
use std::net::Ipv6Addr;
use std::path::Path;
#[cfg(target_os = "linux")]
use std::process::{Command as ProcessCommand, Stdio};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

#[cfg(any(test, target_os = "linux"))]
pub(crate) const FIPS_HOST_ROUTE_TARGET: &str = "fd00::/8";
const FIPS_HOST_IFACE: &str = "nvpnfips0";
const FIPS_HOST_MTU: u16 = 1280;
const FIPS_HOST_DNS_BIND_ADDR: &str = "::1";
const FIPS_HOST_DNS_PORT: u16 = 5354;
#[cfg(any(test, target_os = "linux"))]
const NFT_TABLE_NAME: &str = "nvpn_fips_host";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FipsHostTunnelConfig {
    pub(crate) identity_nsec: String,
    pub(crate) fips_address: Ipv6Addr,
    pub(crate) dns_bind_addr: String,
    pub(crate) dns_port: u16,
    pub(crate) nostr_relays: Vec<String>,
    pub(crate) stun_servers: Vec<String>,
    pub(crate) share_local_candidates: bool,
    pub(crate) inbound_tcp_ports: Vec<u16>,
}

impl FipsHostTunnelConfig {
    pub(crate) fn from_app(app: &AppConfig) -> Result<Option<Self>> {
        if !app.fips_host_tunnel_enabled {
            return Ok(None);
        }

        let identity = Identity::from_secret_str(&app.nostr.secret_key)
            .context("failed to derive .fips identity from nostr secret key")?;
        let mut inbound_tcp_ports = app.fips_host_inbound_tcp_ports.clone();
        inbound_tcp_ports.sort_unstable();
        inbound_tcp_ports.dedup();

        Ok(Some(Self {
            identity_nsec: app.nostr.secret_key.clone(),
            fips_address: identity.address().to_ipv6(),
            dns_bind_addr: FIPS_HOST_DNS_BIND_ADDR.to_string(),
            dns_port: FIPS_HOST_DNS_PORT,
            nostr_relays: app.nostr.relays.clone(),
            stun_servers: app.nat.stun_servers.clone(),
            share_local_candidates: app.lan_discovery_enabled,
            inbound_tcp_ports,
        }))
    }

    fn fips_config(&self) -> Config {
        let mut config = Config::new();
        config.node.identity = IdentityConfig {
            nsec: Some(self.identity_nsec.clone()),
            persistent: false,
        };
        config.node.system_files_enabled = false;
        config.node.control.enabled = false;
        config.node.routing.mode = RoutingMode::ReplyLearned;
        config.tun.enabled = true;
        config.tun.name = Some(FIPS_HOST_IFACE.to_string());
        config.tun.mtu = Some(FIPS_HOST_MTU);
        config.dns.enabled = true;
        config.dns.bind_addr = Some(self.dns_bind_addr.clone());
        config.dns.port = Some(self.dns_port);
        config.node.discovery.nostr.enabled = !self.nostr_relays.is_empty();
        config.node.discovery.nostr.advertise = false;
        config.node.discovery.nostr.advert_relays = self.nostr_relays.clone();
        config.node.discovery.nostr.dm_relays = self.nostr_relays.clone();
        config.node.discovery.nostr.stun_servers = self.stun_servers.clone();
        config.node.discovery.nostr.share_local_candidates = self.share_local_candidates;
        config.node.discovery.nostr.policy = NostrDiscoveryPolicy::Open;
        config.node.discovery.lan.enabled = self.share_local_candidates;
        config.transports.udp = TransportInstances::Single(UdpConfig {
            outbound_only: Some(true),
            accept_connections: Some(false),
            advertise_on_nostr: Some(false),
            ..UdpConfig::default()
        });
        config.transports.tcp = TransportInstances::Single(TcpConfig {
            bind_addr: None,
            advertise_on_nostr: Some(false),
            ..TcpConfig::default()
        });
        config
    }
}

pub(crate) struct FipsHostTunnelRuntime {
    config: FipsHostTunnelConfig,
    shutdown_tx: Option<oneshot::Sender<()>>,
    node_task: JoinHandle<Result<()>>,
    resolver: Option<SystemResolverGuard>,
    firewall: Option<LinuxFirewallGuard>,
}

impl FipsHostTunnelRuntime {
    pub(crate) async fn start(config: FipsHostTunnelConfig) -> Result<Self> {
        let mut node = Node::new(config.fips_config()).context("failed to create .fips node")?;
        node.start().await.context("failed to start .fips node")?;
        if node.tun_state() != TunState::Active {
            let tun_state = node.tun_state();
            let _ = node.stop().await;
            return Err(anyhow!(".fips TUN did not become active: {tun_state}"));
        }
        let iface = node.tun_name().unwrap_or(FIPS_HOST_IFACE).to_string();

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let node_task = spawn_fips_node_task(node, shutdown_rx);

        let resolver = match SystemResolverGuard::install(&config) {
            Ok(guard) => guard,
            Err(error) => {
                eprintln!("fips-host: failed to install .fips resolver: {error}");
                None
            }
        };
        let firewall = match LinuxFirewallGuard::install(&iface, &config.inbound_tcp_ports) {
            Ok(guard) => guard,
            Err(error) => {
                eprintln!("fips-host: failed to install .fips firewall: {error}");
                None
            }
        };

        Ok(Self {
            config,
            shutdown_tx: Some(shutdown_tx),
            node_task,
            resolver,
            firewall,
        })
    }

    pub(crate) fn requires_restart(&self, config: &FipsHostTunnelConfig) -> bool {
        self.config != *config
    }

    pub(crate) async fn stop(mut self) -> Result<()> {
        drop(self.firewall.take());
        drop(self.resolver.take());
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.node_task
            .await
            .context(".fips node task join failed")?
            .context(".fips node task failed")
    }

    pub(crate) fn cleanup_disabled_artifacts() {
        SystemResolverGuard::cleanup_disabled_artifacts();
        LinuxFirewallGuard::cleanup_disabled_artifacts();
    }
}

fn spawn_fips_node_task(
    mut node: Node,
    shutdown_rx: oneshot::Receiver<()>,
) -> JoinHandle<Result<()>> {
    tokio::spawn(async move {
        tokio::pin!(shutdown_rx);
        let loop_result = tokio::select! {
            result = node.run_rx_loop() => result.map_err(anyhow::Error::from),
            _ = &mut shutdown_rx => Ok(()),
        };
        let stop_result = if node.state().can_stop() {
            node.stop().await.map_err(anyhow::Error::from)
        } else {
            Ok(())
        };
        loop_result?;
        stop_result
    })
}

struct SystemResolverGuard {
    backend: ResolverBackend,
}

enum ResolverBackend {
    #[cfg(target_os = "macos")]
    MacosResolver { path: String },
    #[cfg(target_os = "linux")]
    SystemdResolved { path: String },
    #[cfg(target_os = "linux")]
    Dnsmasq { path: String, service: &'static str },
}

impl SystemResolverGuard {
    fn install(config: &FipsHostTunnelConfig) -> Result<Option<Self>> {
        #[cfg(target_os = "macos")]
        {
            return install_macos_resolver(config)
                .map(|backend| backend.map(|backend| Self { backend }));
        }
        #[cfg(target_os = "linux")]
        {
            return install_linux_resolver(config)
                .map(|backend| backend.map(|backend| Self { backend }));
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = config;
            Ok(None)
        }
    }

    fn cleanup_disabled_artifacts() {
        #[cfg(target_os = "macos")]
        {
            remove_owned_file("/etc/resolver/fips");
        }
        #[cfg(target_os = "linux")]
        {
            if remove_owned_file("/etc/systemd/resolved.conf.d/nostr-vpn-fips.conf") {
                restart_service("systemd-resolved");
            }
            if remove_owned_file("/etc/dnsmasq.d/nostr-vpn-fips.conf") {
                reload_service("dnsmasq");
            }
            if remove_owned_file("/etc/NetworkManager/dnsmasq.d/nostr-vpn-fips.conf") {
                reload_service("NetworkManager");
            }
        }
    }
}

impl Drop for SystemResolverGuard {
    fn drop(&mut self) {
        match &self.backend {
            #[cfg(target_os = "macos")]
            ResolverBackend::MacosResolver { path } => {
                remove_owned_file(path);
            }
            #[cfg(target_os = "linux")]
            ResolverBackend::SystemdResolved { path } => {
                if remove_owned_file(path) {
                    restart_service("systemd-resolved");
                }
            }
            #[cfg(target_os = "linux")]
            ResolverBackend::Dnsmasq { path, service } => {
                if remove_owned_file(path) {
                    reload_service(service);
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn install_macos_resolver(config: &FipsHostTunnelConfig) -> Result<Option<ResolverBackend>> {
    let path = "/etc/resolver/fips";
    let contents = format!(
        "# Managed by nostr-vpn for .fips host routing.\n\
         nameserver {}\n\
         port {}\n",
        config.dns_bind_addr, config.dns_port
    );
    write_owned_file(path, &contents)?;
    Ok(Some(ResolverBackend::MacosResolver {
        path: path.to_string(),
    }))
}

#[cfg(target_os = "linux")]
fn install_linux_resolver(config: &FipsHostTunnelConfig) -> Result<Option<ResolverBackend>> {
    if service_is_active("systemd-resolved") {
        let path = "/etc/systemd/resolved.conf.d/nostr-vpn-fips.conf";
        let contents = format!(
            "# Managed by nostr-vpn for .fips host routing.\n\
             [Resolve]\n\
             DNS=[{}]:{}\n\
             Domains=~fips\n",
            config.dns_bind_addr, config.dns_port
        );
        write_owned_file(path, &contents)?;
        restart_service("systemd-resolved");
        return Ok(Some(ResolverBackend::SystemdResolved {
            path: path.to_string(),
        }));
    }

    if service_is_active("dnsmasq") && Path::new("/etc/dnsmasq.d").is_dir() {
        let path = "/etc/dnsmasq.d/nostr-vpn-fips.conf";
        let contents = format!(
            "# Managed by nostr-vpn for .fips host routing.\n\
             server=/fips/{}#{}\n",
            config.dns_bind_addr, config.dns_port
        );
        write_owned_file(path, &contents)?;
        reload_service("dnsmasq");
        return Ok(Some(ResolverBackend::Dnsmasq {
            path: path.to_string(),
            service: "dnsmasq",
        }));
    }

    if service_is_active("NetworkManager") && Path::new("/etc/NetworkManager/dnsmasq.d").is_dir() {
        let path = "/etc/NetworkManager/dnsmasq.d/nostr-vpn-fips.conf";
        let contents = format!(
            "# Managed by nostr-vpn for .fips host routing.\n\
             server=/fips/{}#{}\n",
            config.dns_bind_addr, config.dns_port
        );
        write_owned_file(path, &contents)?;
        reload_service("NetworkManager");
        return Ok(Some(ResolverBackend::Dnsmasq {
            path: path.to_string(),
            service: "NetworkManager",
        }));
    }

    Ok(None)
}

fn write_owned_file(path: &str, contents: &str) -> Result<()> {
    if let Ok(existing) = fs::read_to_string(path)
        && !existing.contains("Managed by nostr-vpn")
    {
        eprintln!("fips-host: leaving existing resolver file {path} in place");
        return Ok(());
    }
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create resolver directory {}", parent.display()))?;
    }
    fs::write(path, contents).with_context(|| format!("failed to write {path}"))
}

fn remove_owned_file(path: &str) -> bool {
    match fs::read_to_string(path) {
        Ok(contents) if contents.contains("Managed by nostr-vpn") => fs::remove_file(path).is_ok(),
        _ => false,
    }
}

#[cfg(target_os = "linux")]
fn service_is_active(service: &str) -> bool {
    ProcessCommand::new("systemctl")
        .arg("is-active")
        .arg("--quiet")
        .arg(service)
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "linux")]
fn restart_service(service: &str) {
    let _ = ProcessCommand::new("systemctl")
        .arg("restart")
        .arg(service)
        .status();
}

#[cfg(target_os = "linux")]
fn reload_service(service: &str) {
    let _ = ProcessCommand::new("systemctl")
        .arg("reload")
        .arg(service)
        .status();
}

struct LinuxFirewallGuard {
    #[allow(dead_code)]
    table_name: String,
}

impl LinuxFirewallGuard {
    fn install(iface: &str, inbound_tcp_ports: &[u16]) -> Result<Option<Self>> {
        #[cfg(target_os = "linux")]
        {
            if !command_exists("nft") {
                return Ok(None);
            }
            let rules = render_nft_firewall_rules(iface, inbound_tcp_ports);
            apply_nft_rules(&rules)?;
            return Ok(Some(Self {
                table_name: NFT_TABLE_NAME.to_string(),
            }));
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (iface, inbound_tcp_ports);
            Ok(None)
        }
    }

    fn cleanup_disabled_artifacts() {
        #[cfg(target_os = "linux")]
        {
            remove_nft_table(NFT_TABLE_NAME);
        }
    }
}

impl Drop for LinuxFirewallGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        {
            remove_nft_table(&self.table_name);
        }
    }
}

#[cfg(target_os = "linux")]
fn remove_nft_table(table_name: &str) {
    if !command_exists("nft") {
        return;
    }
    let _ = ProcessCommand::new("nft")
        .arg("delete")
        .arg("table")
        .arg("inet")
        .arg(table_name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(any(test, target_os = "linux"))]
pub(crate) fn render_nft_firewall_rules(iface: &str, inbound_tcp_ports: &[u16]) -> String {
    let mut ports = inbound_tcp_ports.to_vec();
    ports.sort_unstable();
    ports.dedup();
    let inbound_tcp_rule = match ports.as_slice() {
        [] => String::new(),
        [port] => format!("    tcp dport {port} accept\n"),
        ports => {
            let joined = ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            format!("    tcp dport {{ {joined} }} accept\n")
        }
    };

    format!(
        "table inet {NFT_TABLE_NAME} {{\n\
           chain input {{\n\
             type filter hook input priority 0; policy accept;\n\
             iifname != \"{iface}\" return\n\
             meta nfproto != ipv6 return\n\
             ip6 saddr != {FIPS_HOST_ROUTE_TARGET} return\n\
             ct state established,related accept\n\
         {inbound_tcp_rule}\
             counter drop\n\
           }}\n\
           chain output {{\n\
             type filter hook output priority 0; policy accept;\n\
             oifname != \"{iface}\" return\n\
             meta nfproto != ipv6 return\n\
             ip6 daddr != {FIPS_HOST_ROUTE_TARGET} return\n\
             ct state established,related accept\n\
             meta l4proto tcp accept\n\
             counter drop\n\
           }}\n\
         }}\n"
    )
}

#[cfg(target_os = "linux")]
fn apply_nft_rules(rules: &str) -> Result<()> {
    let _ = ProcessCommand::new("nft")
        .arg("delete")
        .arg("table")
        .arg("inet")
        .arg(NFT_TABLE_NAME)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let mut child = ProcessCommand::new("nft")
        .arg("-f")
        .arg("-")
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to start nft")?;
    let mut stdin = child.stdin.take().context("failed to open nft stdin")?;
    stdin
        .write_all(rules.as_bytes())
        .context("failed to write nft rules")?;
    drop(stdin);
    let status = child.wait().context("failed to wait for nft")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("nft exited with {status}"))
    }
}

#[cfg(target_os = "linux")]
fn command_exists(command: &str) -> bool {
    ProcessCommand::new("sh")
        .arg("-c")
        .arg(format!("command -v {command} >/dev/null 2>&1"))
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nft_rules_default_to_outbound_tcp_only() {
        let rules = render_nft_firewall_rules("nvpn0", &[]);
        assert!(rules.contains("iifname != \"nvpn0\" return"));
        assert!(rules.contains("oifname != \"nvpn0\" return"));
        assert!(rules.contains("ip6 saddr != fd00::/8 return"));
        assert!(rules.contains("ip6 daddr != fd00::/8 return"));
        assert!(rules.contains("meta l4proto tcp accept"));
        assert!(!rules.contains("tcp dport"));
    }

    #[test]
    fn nft_rules_allow_configured_inbound_tcp_ports() {
        let rules = render_nft_firewall_rules("nvpn0", &[443, 22, 22]);
        assert!(rules.contains("tcp dport { 22, 443 } accept"));
    }

    #[test]
    fn app_config_builds_outbound_only_embedded_node() {
        let mut app = AppConfig::generated();
        app.fips_host_inbound_tcp_ports = vec![443, 22, 22];

        let config = FipsHostTunnelConfig::from_app(&app)
            .expect("valid fips host config")
            .expect("enabled by default");
        assert_eq!(config.inbound_tcp_ports, vec![22, 443]);
        assert_eq!(
            config.fips_address,
            Identity::from_secret_str(&app.nostr.secret_key)
                .expect("app identity")
                .address()
                .to_ipv6()
        );

        let fips = config.fips_config();
        assert!(fips.tun.enabled);
        assert_eq!(fips.tun.name.as_deref(), Some(FIPS_HOST_IFACE));
        assert_eq!(fips.tun.mtu, Some(FIPS_HOST_MTU));
        assert!(fips.dns.enabled);
        assert_eq!(fips.dns.bind_addr.as_deref(), Some("::1"));
        assert_eq!(fips.dns.port, Some(5354));
        assert_eq!(fips.node.discovery.nostr.policy, NostrDiscoveryPolicy::Open);
        assert!(!fips.node.discovery.nostr.advertise);

        match fips.transports.udp {
            TransportInstances::Single(udp) => {
                assert_eq!(udp.outbound_only, Some(true));
                assert_eq!(udp.accept_connections, Some(false));
                assert_eq!(udp.advertise_on_nostr, Some(false));
            }
            _ => panic!("expected single udp transport"),
        }
        match fips.transports.tcp {
            TransportInstances::Single(tcp) => {
                assert!(tcp.bind_addr.is_none());
                assert_eq!(tcp.advertise_on_nostr, Some(false));
            }
            _ => panic!("expected single tcp transport"),
        }
    }

    #[test]
    fn app_config_can_disable_host_tunnel() {
        let mut app = AppConfig::generated();
        app.fips_host_tunnel_enabled = false;

        assert!(
            FipsHostTunnelConfig::from_app(&app)
                .expect("valid disabled config")
                .is_none()
        );
    }
}
