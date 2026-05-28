mod deep_link;
mod qr;
mod qr_scan;
mod tray;
mod updater;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gio, glib};
use nostr_vpn_app_core::{
    FfiApp, NativeAppAction, NativeAppState, NativeNetworkState, NativeParticipantState,
    SettingsPatch, UpdateAutoCheckPolicy,
};

const APP_ID: &str = "to.iris.nvpn";
const DEFAULT_UPDATE_POLL_INTERVAL_SECS: u32 = 6 * 60 * 60;
const SEARCH_VISIBILITY_THRESHOLD: usize = 7;

thread_local! {
    static TRAY_APP_HOLD: RefCell<Option<gio::ApplicationHoldGuard>> = const { RefCell::new(None) };
}

type AppRef = Rc<RefCell<AppModel>>;

#[derive(Clone, Default)]
struct AppRuntime {
    model: Rc<RefCell<Option<AppRef>>>,
    pending_urls: Rc<RefCell<Vec<String>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Devices,
    Share,
    ExitNodes,
    Settings,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct PageScrollOffsets {
    devices: f64,
    share: f64,
    exit_nodes: f64,
    settings: f64,
}

impl PageScrollOffsets {
    fn get(self, page: Page) -> f64 {
        match page {
            Page::Devices => self.devices,
            Page::Share => self.share,
            Page::ExitNodes => self.exit_nodes,
            Page::Settings => self.settings,
        }
    }

    fn set(&mut self, page: Page, offset: f64) {
        match page {
            Page::Devices => self.devices = offset,
            Page::Share => self.share = offset,
            Page::ExitNodes => self.exit_nodes = offset,
            Page::Settings => self.settings = offset,
        }
    }
}

#[derive(Clone, Default)]
struct Drafts {
    invite: String,
    participant_npub: String,
    participant_alias: String,
    network_name: String,
    mesh_id: String,
    manual_join_admin_id: String,
    manual_join_network_id: String,
    new_network_name: String,
    node_name: String,
    endpoint: String,
    tunnel_ip: String,
    listen_port: String,
    relay_input: String,
    fips_host_inbound_tcp_ports: String,
    advertised_routes: String,
    exit_search: String,
    wireguard_exit_config: String,
}

impl Drafts {
    fn sync_from_state(&mut self, state: &NativeAppState) {
        self.node_name = state.node_name.clone();
        self.endpoint = state.endpoint.clone();
        self.tunnel_ip = state.tunnel_ip.clone();
        self.listen_port = state.listen_port.to_string();
        self.fips_host_inbound_tcp_ports = state.fips_host_inbound_tcp_ports.clone();
        self.advertised_routes = state.advertised_routes.join(", ");
        self.wireguard_exit_config = state.wireguard_exit_config.clone();
        if let Some(network) = active_network(state) {
            self.network_name = display_network_name(network);
            self.mesh_id = display_network_id(&network.network_id);
        } else {
            self.network_name = "Nostr VPN".to_string();
            self.mesh_id.clear();
        }
    }
}

struct AppModel {
    core: Arc<FfiApp>,
    state: NativeAppState,
    window: adw::ApplicationWindow,
    page: Page,
    rendered_page: Page,
    scroll_offsets: PageScrollOffsets,
    sidebar: gtk::Box,
    update_bar: gtk::Box,
    content: gtk::Box,
    header_status_label: gtk::Label,
    header_status_dot: gtk::Box,
    header_vpn_switch: gtk::Switch,
    selected_device_pubkey: Option<String>,
    drafts: Drafts,
    notice: String,
    tray: tray::TrayRuntime,
    tray_available: bool,
    tray_error: Option<String>,
    update: updater::UpdateState,
    update_policy: UpdateAutoCheckPolicy,
    update_sender: Sender<updater::UpdateEvent>,
    update_receiver: Receiver<updater::UpdateEvent>,
    add_network_join_status: String,
    allow_close: bool,
    service_settling: bool,
    diagnostics_expanded: bool,
}

impl AppModel {
    fn new(
        window: adw::ApplicationWindow,
        sidebar: gtk::Box,
        update_bar: gtk::Box,
        content: gtk::Box,
        header_status_label: gtk::Label,
        header_status_dot: gtk::Box,
        header_vpn_switch: gtk::Switch,
    ) -> Self {
        // Pass empty so the FFI falls back to its own CARGO_PKG_VERSION
        // (workspace-inherited). The linux crate is excluded from the workspace
        // so its CARGO_PKG_VERSION drifts from nostr-vpn-app-core's.
        let core = FfiApp::new(default_data_dir(), String::new());
        let state = core.state();
        let mut drafts = Drafts::default();
        drafts.sync_from_state(&state);
        let tray = tray::TrayRuntime::start(&state);
        let tray_available = tray.is_available();
        let tray_error = tray.last_error();
        let diagnostics_expanded = !state.health.is_empty();
        let (update_sender, update_receiver) = mpsc::channel();
        let update = updater::UpdateState {
            auto_install: load_auto_install_updates(),
            ..updater::UpdateState::default()
        };
        let update_policy =
            UpdateAutoCheckPolicy::new(Duration::from_secs(update_poll_interval_secs() as u64));
        Self {
            core,
            state,
            window,
            page: Page::Devices,
            rendered_page: Page::Devices,
            scroll_offsets: PageScrollOffsets::default(),
            sidebar,
            update_bar,
            content,
            header_status_label,
            header_status_dot,
            header_vpn_switch,
            selected_device_pubkey: None,
            drafts,
            notice: String::new(),
            tray,
            tray_available,
            tray_error,
            update,
            update_policy,
            update_sender,
            update_receiver,
            add_network_join_status: String::new(),
            allow_close: false,
            service_settling: false,
            diagnostics_expanded,
        }
    }
}

fn main() -> glib::ExitCode {
    if let Some(exit_code) = run_update_e2e_from_args() {
        return exit_code;
    }

    bootstrap_session_bus();

    let runtime = AppRuntime::default();
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(|_| {
        install_css();
        gtk::Window::set_default_icon_name("nostr-vpn");
    });
    {
        let runtime = runtime.clone();
        app.connect_activate(move |app| {
            build_ui(app, &runtime, true);
        });
    }
    {
        let runtime = runtime.clone();
        app.connect_command_line(move |app, command_line| {
            let mut present = true;
            let mut urls = Vec::new();
            for arg in command_line.arguments() {
                let arg = arg.to_string_lossy();
                if arg == "--hidden" {
                    present = false;
                }
                if arg.starts_with("nvpn://") {
                    urls.push(arg.into_owned());
                    present = true;
                }
            }
            runtime.pending_urls.borrow_mut().extend(urls);
            build_ui(app, &runtime, present);
            drain_pending_urls(&runtime);
            glib::ExitCode::SUCCESS.into()
        });
    }
    app.run()
}

fn run_update_e2e_from_args() -> Option<glib::ExitCode> {
    let args = std::env::args().collect::<Vec<_>>();
    if !args.iter().any(|arg| arg == "--nvpn-e2e-update-check") {
        return None;
    }

    let result = run_update_e2e(args.iter().any(|arg| arg == "--nvpn-e2e-install-update"));
    let output_path = std::env::var("NVPN_UPDATE_E2E_RESULT_PATH").ok();
    let success = result
        .as_ref()
        .map(|value| value["ok"].as_bool().unwrap_or(false))
        .unwrap_or(false);
    match (output_path, result) {
        (Some(path), Ok(value)) => {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(error) = std::fs::write(
                &path,
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string()),
            ) {
                eprintln!("failed to write update e2e result {path}: {error}");
                return Some(glib::ExitCode::FAILURE);
            }
        }
        (Some(path), Err(error)) => {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let value = serde_json::json!({
                "ok": false,
                "platform": "linux",
                "error": error,
            });
            let _ = std::fs::write(
                &path,
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string()),
            );
        }
        (None, Ok(value)) => {
            println!("{value}");
        }
        (None, Err(error)) => {
            eprintln!("{error}");
        }
    }

    Some(if success {
        glib::ExitCode::SUCCESS
    } else {
        glib::ExitCode::FAILURE
    })
}

fn run_update_e2e(install: bool) -> Result<serde_json::Value, String> {
    let current_version = std::env::var("NVPN_UPDATE_E2E_CURRENT_VERSION")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    let check = updater::check_blocking(&current_version)?;
    let mut downloaded_path = None;
    let mut executable = None;
    if install {
        let asset = check
            .asset
            .as_ref()
            .ok_or_else(|| "no Linux update asset selected".to_string())?;
        let path = updater::download_blocking(asset)?;
        executable = std::fs::metadata(&path).ok().map(|metadata| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                let _ = metadata;
                false
            }
        });
        downloaded_path = Some(path.display().to_string());
    }

    Ok(serde_json::json!({
        "ok": true,
        "platform": "linux",
        "available": check.newer,
        "tag": check.tag,
        "assetName": check.asset.as_ref().map(|asset| asset.name.clone()),
        "assetUrl": check.asset.as_ref().map(|asset| asset.url.clone()),
        "downloadedPath": downloaded_path,
        "downloadedExecutable": executable,
    }))
}

fn build_ui(app: &adw::Application, runtime: &AppRuntime, present: bool) {
    if let Some(window) = app
        .active_window()
        .or_else(|| app.windows().into_iter().next())
    {
        if present {
            window.present();
        }
        drain_pending_urls(runtime);
        return;
    }

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(1040)
        .default_height(720)
        .title("Nostr VPN")
        .build();
    window.add_css_class("nvpn-root");

    let header = adw::HeaderBar::new();
    let title = gtk::Label::new(Some("Nostr VPN"));
    title.add_css_class("heading");
    title.set_halign(gtk::Align::Start);
    header.set_title_widget(Some(&title));

    let header_vpn_switch = gtk::Switch::new();
    header_vpn_switch.set_valign(gtk::Align::Center);
    header_vpn_switch.set_tooltip_text(Some("Toggle VPN"));
    header.pack_end(&header_vpn_switch);

    let header_status_dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header_status_dot.add_css_class("nvpn-header-dot");
    header_status_dot.set_valign(gtk::Align::Center);
    header_status_dot.set_visible(false);
    header.pack_end(&header_status_dot);

    let header_status_label = gtk::Label::new(None);
    header_status_label.add_css_class("nvpn-header-status");
    header_status_label.add_css_class("dim-label");
    header_status_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    header_status_label.set_max_width_chars(28);
    header.pack_end(&header_status_label);

    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 10);
    sidebar.add_css_class("nvpn-sidebar");
    sidebar.set_width_request(210);
    sidebar.set_margin_top(14);
    sidebar.set_margin_bottom(14);
    sidebar.set_margin_start(14);
    sidebar.set_margin_end(10);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.set_vexpand(true);
    content.add_css_class("nvpn-content");

    let update_bar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    update_bar.add_css_class("nvpn-update-bar-host");

    let shell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    shell.set_hexpand(true);
    shell.set_vexpand(true);
    shell.append(&sidebar);
    shell.append(&content);

    let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main.set_hexpand(true);
    main.set_vexpand(true);
    main.append(&update_bar);
    main.append(&shell);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&main));
    window.set_content(Some(&toolbar));

    let model = Rc::new(RefCell::new(AppModel::new(
        window.clone(),
        sidebar.clone(),
        update_bar.clone(),
        content.clone(),
        header_status_label.clone(),
        header_status_dot.clone(),
        header_vpn_switch.clone(),
    )));
    *runtime.model.borrow_mut() = Some(model.clone());
    if model.borrow().tray_available {
        update_tray_application_hold(true, window.application());
    }
    sync_launch_on_startup_setting(&model);

    {
        let model = model.clone();
        header_vpn_switch.connect_active_notify(move |sw| {
            let target = sw.is_active();
            let current = model.borrow().state.vpn_enabled;
            if target == current {
                return;
            }
            dispatch(
                &model,
                if target {
                    NativeAppAction::ConnectVpn
                } else {
                    NativeAppAction::DisconnectVpn
                },
            );
        });
    }
    {
        let model = model.clone();
        window.connect_close_request(move |window| {
            let model = model.borrow();
            if should_close_to_tray(
                model.state.close_to_tray_on_close,
                model.tray.is_available(),
                model.allow_close,
            ) {
                update_tray_application_hold(true, window.application());
                window.set_visible(false);
                glib::Propagation::Stop
            } else {
                release_tray_application_hold();
                glib::Propagation::Proceed
            }
        });
    }

    render(&model);

    {
        let model = model.clone();
        glib::timeout_add_seconds_local(2, move || {
            refresh_now(&model);
            glib::ControlFlow::Continue
        });
    }
    {
        let model = model.clone();
        glib::timeout_add_local(Duration::from_millis(250), move || {
            drain_tray_commands(&model);
            drain_update_events(&model);
            glib::ControlFlow::Continue
        });
    }
    {
        let model = model.clone();
        glib::timeout_add_seconds_local(update_poll_interval_secs(), move || {
            check_updates_if_due(&model);
            glib::ControlFlow::Continue
        });
    }

    check_updates_if_due(&model);

    if present {
        window.present();
    }
    drain_pending_urls(runtime);
}

fn refresh_now(app: &AppRef) {
    let (core, previous_state) = {
        let model = app.borrow();
        (model.core.clone(), model.state.clone())
    };
    let state = core.refresh();
    let should_render = state_needs_render(&previous_state, &state);
    set_state(app, state);
    if should_render {
        render(app);
    }
}

fn dispatch(app: &AppRef, action: NativeAppAction) -> NativeAppState {
    let settle_service = matches!(
        &action,
        NativeAppAction::InstallSystemService
            | NativeAppAction::UninstallSystemService
            | NativeAppAction::EnableSystemService
            | NativeAppAction::DisableSystemService
    );
    if let NativeAppAction::UpdateSettings { patch } = &action {
        if let Some(enabled) = patch.launch_on_startup {
            if let Err(error) = configure_launch_on_startup(enabled) {
                set_notice(app, error);
                return app.borrow().state.clone();
            }
        }
    }
    let core = app.borrow().core.clone();
    let state = core.dispatch(action);
    let result = state.clone();
    set_state(app, state);
    render(app);
    if settle_service {
        start_service_settlement_polling(app);
    }
    result
}

fn sync_launch_on_startup_setting(app: &AppRef) {
    let enabled = {
        let model = app.borrow();
        model.state.startup_settings_supported && model.state.launch_on_startup
    };
    if let Err(error) = configure_launch_on_startup(enabled) {
        set_notice(app, error);
    }
}

fn set_state(app: &AppRef, state: NativeAppState) {
    let mut model = app.borrow_mut();
    if state.health.len() > model.state.health.len() {
        model.diagnostics_expanded = true;
    }
    model.tray.update(&state);
    model.state = state;
}

fn state_needs_render(previous: &NativeAppState, next: &NativeAppState) -> bool {
    if previous == next {
        return false;
    }
    let mut previous = previous.clone();
    previous.rev = next.rev;
    previous != *next
}

fn drain_tray_commands(app: &AppRef) {
    if sync_tray_status(app) {
        render(app);
    }

    let commands = app.borrow_mut().tray.drain();
    for command in commands {
        match command {
            tray::TrayCommand::ShowWindow => show_window(app),
            tray::TrayCommand::ToggleVpn => {
                let enabled = app.borrow().state.vpn_enabled;
                dispatch(
                    app,
                    if enabled {
                        NativeAppAction::DisconnectVpn
                    } else {
                        NativeAppAction::ConnectVpn
                    },
                );
            }
            tray::TrayCommand::ToggleExitOffer => {
                let enabled = !app.borrow().state.advertise_exit_node;
                dispatch(
                    app,
                    NativeAppAction::UpdateSettings {
                        patch: SettingsPatch {
                            advertise_exit_node: Some(enabled),
                            ..SettingsPatch::default()
                        },
                    },
                );
            }
            tray::TrayCommand::CopyDeviceId => {
                let value = app.borrow().state.own_npub.clone();
                if !value.trim().is_empty() {
                    copy_text(&value);
                }
            }
            tray::TrayCommand::CopyPeer(npub) => copy_text(&npub),
            tray::TrayCommand::SetExitNode(npub) => {
                dispatch(
                    app,
                    NativeAppAction::UpdateSettings {
                        patch: SettingsPatch {
                            exit_node: Some(npub),
                            ..SettingsPatch::default()
                        },
                    },
                );
            }
            tray::TrayCommand::Quit => quit_app(app),
        }
    }
}

fn check_updates(app: &AppRef, manual: bool) {
    let (current_version, sender) = {
        let mut model = app.borrow_mut();
        if model.update.checking || model.update.downloading {
            return;
        }
        if manual {
            model
                .update_policy
                .note_manual_check_started(Instant::now());
        }
        model.update.checking = true;
        if manual {
            model.update.status = "Checking for updates".to_string();
        }
        (model.state.app_version.clone(), model.update_sender.clone())
    };
    render(app);
    updater::check(current_version, manual, sender);
}

fn check_updates_if_due(app: &AppRef) {
    let due = {
        let mut model = app.borrow_mut();
        model.update_policy.should_start_check(true, Instant::now())
    };
    if due {
        check_updates(app, false);
    }
}

fn download_update(app: &AppRef) {
    let (asset, sender) = {
        let mut model = app.borrow_mut();
        if model.update.checking || model.update.downloading {
            return;
        }
        let Some(asset) = model.update.asset.clone() else {
            model.update.status = "No Linux update asset found".to_string();
            render(app);
            return;
        };
        model.update.downloading = true;
        model.update.status = format!("Downloading {}", model.update.version);
        (asset, model.update_sender.clone())
    };
    render(app);
    updater::download(asset, sender);
}

fn drain_update_events(app: &AppRef) {
    let events = {
        let model = app.borrow();
        model.update_receiver.try_iter().collect::<Vec<_>>()
    };
    if events.is_empty() {
        return;
    }

    let mut auto_download = false;
    {
        let mut model = app.borrow_mut();
        for event in events {
            match event {
                updater::UpdateEvent::Checked { manual, result } => {
                    model.update.checking = false;
                    match result {
                        Ok(check) => {
                            model.update.available = check.newer;
                            model.update.version = check.tag.clone();
                            model.update.asset = if check.newer { check.asset } else { None };
                            if check.newer {
                                model.update.status = if model.update.asset.is_some() {
                                    format!("Update {} available", check.tag)
                                } else {
                                    format!(
                                        "Update {} found without a Linux desktop asset",
                                        check.tag
                                    )
                                };
                                auto_download =
                                    model.update.auto_install && model.update.asset.is_some();
                            } else if manual {
                                model.update.status = "Up to date".to_string();
                            } else {
                                model.update.status.clear();
                            }
                        }
                        Err(error) => {
                            if manual {
                                model.update.status = error;
                            } else {
                                model.update.status.clear();
                            }
                        }
                    }
                }
                updater::UpdateEvent::Downloaded(result) => {
                    model.update.downloading = false;
                    match result {
                        Ok(path) => {
                            model.update.status = format!(
                                "Downloaded {}",
                                path.file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or("update")
                            );
                        }
                        Err(error) => {
                            model.update.status = error;
                        }
                    }
                }
            }
        }
    }

    if auto_download {
        download_update(app);
    } else {
        render(app);
    }
}

fn show_window(app: &AppRef) {
    let window = app.borrow().window.clone();
    window.set_visible(true);
    window.present();
}

fn quit_app(app: &AppRef) {
    let window = {
        let mut model = app.borrow_mut();
        model.allow_close = true;
        model.window.clone()
    };
    release_tray_application_hold();
    if let Some(application) = window.application() {
        application.quit();
    }
}

fn should_close_to_tray(
    close_to_tray_on_close: bool,
    tray_available: bool,
    allow_close: bool,
) -> bool {
    close_to_tray_on_close && tray_available && !allow_close
}

fn sync_tray_status(app: &AppRef) -> bool {
    let (changed, available, application) = {
        let mut model = app.borrow_mut();
        let available = model.tray.is_available();
        let error = model.tray.last_error();
        let changed = available != model.tray_available || error != model.tray_error;
        if changed {
            model.tray_available = available;
            model.tray_error = error;
        }
        (changed, available, model.window.application())
    };

    if changed {
        update_tray_application_hold(available, application);
    }

    changed
}

fn update_tray_application_hold(available: bool, application: Option<gtk::Application>) {
    TRAY_APP_HOLD.with(|hold| {
        let mut hold = hold.borrow_mut();
        if available {
            if hold.is_none() {
                if let Some(application) = application {
                    *hold = Some(application.hold());
                }
            }
        } else {
            hold.take();
        }
    });
}

fn release_tray_application_hold() {
    TRAY_APP_HOLD.with(|hold| {
        hold.borrow_mut().take();
    });
}

fn start_service_settlement_polling(app: &AppRef) {
    app.borrow_mut().service_settling = true;
    render(app);

    let app = app.clone();
    let attempts = Rc::new(Cell::new(0));
    glib::timeout_add_local(Duration::from_millis(700), move || {
        refresh_now(&app);
        let next = attempts.get() + 1;
        attempts.set(next);
        if next >= 8 {
            app.borrow_mut().service_settling = false;
            render(&app);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn drain_pending_urls(runtime: &AppRuntime) {
    let Some(app) = runtime.model.borrow().clone() else {
        return;
    };
    let urls: Vec<String> = runtime.pending_urls.borrow_mut().drain(..).collect();
    for url in urls {
        handle_deep_link(&app, &url);
    }
}

fn handle_deep_link(app: &AppRef, raw: &str) {
    match deep_link::parse(raw) {
        Some(deep_link::DeepLink::Invite(invite)) => import_invite(app, invite),
        Some(deep_link::DeepLink::Debug(deep_link::DebugAction::Tick)) => {
            dispatch(app, NativeAppAction::Tick);
        }
        Some(deep_link::DeepLink::Debug(deep_link::DebugAction::RequestJoin { network_id })) => {
            let network_id = {
                let state = app.borrow().state.clone();
                resolve_network_id(&state, network_id)
            };
            if let Some(network_id) = network_id {
                dispatch(app, NativeAppAction::RequestNetworkJoin { network_id });
            }
        }
        Some(deep_link::DeepLink::Debug(deep_link::DebugAction::AcceptJoin {
            network_id,
            requester_npub,
        })) => {
            let (network_id, requester_npub) = {
                let state = app.borrow().state.clone();
                let network_id = resolve_network_id(&state, network_id);
                let requester_npub = requester_npub.or_else(|| {
                    network_id
                        .as_deref()
                        .and_then(|id| {
                            state
                                .networks
                                .iter()
                                .find(|network| network.id == id || network.network_id == id)
                        })
                        .or_else(|| active_network(&state))
                        .and_then(|network| network.inbound_join_requests.first())
                        .map(|request| request.requester_npub.clone())
                });
                (network_id, requester_npub)
            };
            if let (Some(network_id), Some(requester_npub)) = (network_id, requester_npub) {
                dispatch(
                    app,
                    NativeAppAction::AcceptJoinRequest {
                        network_id,
                        requester_npub,
                    },
                );
            }
        }
        None => {}
    }
}

fn import_invite(app: &AppRef, invite: String) {
    let invite = invite.trim().to_string();
    if invite.is_empty() {
        return;
    }
    {
        let mut model = app.borrow_mut();
        model.drafts.invite.clear();
        model.notice.clear();
        model.add_network_join_status.clear();
    }
    let state = dispatch(app, NativeAppAction::ImportNetworkInvite { invite });
    if active_network(&state).is_some() {
        set_page(app, Page::Share);
    }
}

fn create_network(app: &AppRef, name: String) {
    let name = non_empty_or(&name, "Private network");
    dispatch(app, NativeAppAction::AddNetwork { name });
}

fn set_notice(app: &AppRef, notice: impl Into<String>) {
    app.borrow_mut().notice = notice.into();
    render(app);
}

fn set_page(app: &AppRef, page: Page) {
    {
        let mut model = app.borrow_mut();
        if model.page == Page::Share && page != Page::Share {
            model.add_network_join_status.clear();
        }
        model.page = page;
    }
    render(app);
}

fn refresh_header(app: &AppRef, state: &NativeAppState) {
    let (label, dot, switch) = {
        let model = app.borrow();
        (
            model.header_status_label.clone(),
            model.header_status_dot.clone(),
            model.header_vpn_switch.clone(),
        )
    };

    label.set_text(&tray::vpn_status_text(state));

    let dot_visible =
        state.exit_node_blocked || state.exit_node_active || state.vpn_active || state.vpn_enabled;
    dot.set_visible(dot_visible);
    for class in ["ok", "warn", "bad"] {
        dot.remove_css_class(class);
    }
    if state.exit_node_blocked {
        dot.add_css_class("bad");
    } else if state.exit_node_active || state.vpn_active {
        dot.add_css_class("ok");
    } else if state.vpn_enabled {
        dot.add_css_class("warn");
    }

    switch.set_sensitive(state.vpn_control_supported && active_network(state).is_some());
    if switch.is_active() != state.vpn_enabled {
        switch.set_active(state.vpn_enabled);
    }
}

fn render(app: &AppRef) {
    sync_selected_device(app);

    let (sidebar, update_bar, content, state, page, rendered_page) = {
        let model = app.borrow();
        (
            model.sidebar.clone(),
            model.update_bar.clone(),
            model.content.clone(),
            model.state.clone(),
            model.page,
            model.rendered_page,
        )
    };
    let current_scroll_offset = current_content_scroll_offset(&content);
    let scroll_offset = {
        let mut model = app.borrow_mut();
        model
            .scroll_offsets
            .set(rendered_page, current_scroll_offset);
        model.rendered_page = page;
        model.scroll_offsets.get(page)
    };

    refresh_header(app, &state);
    clear_box(&sidebar);
    clear_box(&update_bar);
    clear_box(&content);
    build_sidebar(app, &sidebar, &state, page);
    build_update_stripe(app, &update_bar, &state);

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);

    let page_box = gtk::Box::new(gtk::Orientation::Vertical, 20);
    page_box.set_margin_top(28);
    page_box.set_margin_bottom(32);
    page_box.set_margin_start(28);
    page_box.set_margin_end(28);
    page_box.set_hexpand(true);
    page_box.set_valign(gtk::Align::Start);
    page_box.set_size_request(560, -1);

    match page {
        Page::Devices => build_devices_page(app, &page_box, &state),
        Page::Share => build_share_page(app, &page_box, &state),
        Page::ExitNodes => build_exit_nodes_page(app, &page_box, &state),
        Page::Settings => build_settings_page(app, &page_box, &state),
    }

    scroll.set_child(Some(&page_box));
    content.append(&scroll);
    restore_scroll_offset(&scroll, scroll_offset);
}

fn build_update_stripe(app: &AppRef, parent: &gtk::Box, state: &NativeAppState) {
    let update = app.borrow().update.clone();
    if !update.available {
        return;
    }

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("nvpn-update-stripe");
    row.set_valign(gtk::Align::Center);

    let title = gtk::Label::new(Some(&update_stripe_text(
        &update.version,
        &state.app_version,
    )));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&title);

    let auto_install = gtk::CheckButton::with_label("Install automatically");
    auto_install.set_active(update.auto_install);
    {
        let app = app.clone();
        auto_install.connect_toggled(move |button| {
            let enabled = button.is_active();
            {
                let mut model = app.borrow_mut();
                model.update.auto_install = enabled;
            }
            save_auto_install_updates(enabled);
            if enabled {
                download_update(&app);
            }
        });
    }
    row.append(&auto_install);

    let install = icon_text_button(
        if update.downloading {
            "Downloading"
        } else {
            "Install"
        },
        "folder-download-symbolic",
    );
    install.set_sensitive(
        update.available && update.asset.is_some() && !update.checking && !update.downloading,
    );
    {
        let app = app.clone();
        install.connect_clicked(move |_| download_update(&app));
    }
    row.append(&install);

    parent.append(&row);
}

fn update_stripe_text(version: &str, current: &str) -> String {
    let current = current.trim();
    if current.is_empty() {
        format!("Update available: {version}")
    } else {
        format!("Update available: {version} (you're on {current})")
    }
}

fn build_sidebar(app: &AppRef, sidebar: &gtk::Box, state: &NativeAppState, page: Page) {
    let has_incoming_join_requests = incoming_join_request_count(state) > 0;
    for (target, title, icon) in [
        (Page::Devices, "Devices", ""),
        (Page::ExitNodes, "Exit Nodes", ""),
        (Page::Settings, "Settings", ""),
    ] {
        let button = nav_button(
            title,
            icon,
            page == target,
            target == Page::Devices && has_incoming_join_requests,
        );
        let app = app.clone();
        button.connect_clicked(move |_| set_page(&app, target));
        sidebar.append(&button);
    }

    sidebar.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    if let Some(network) = active_network(state) {
        let summary = gtk::Box::new(gtk::Orientation::Vertical, 4);
        summary.add_css_class("nvpn-sidebar-summary");

        let name = gtk::Label::new(Some(&display_network_name(network)));
        name.add_css_class("caption-heading");
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        summary.append(&name);

        let count = gtk::Label::new(Some(&format!(
            "{} of {} connected",
            state.connected_peer_count, state.expected_peer_count
        )));
        count.add_css_class("caption");
        count.add_css_class("dim-label");
        count.set_xalign(0.0);
        summary.append(&count);

        sidebar.append(&summary);
    }

    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    sidebar.append(&spacer);

    let status = gtk::Label::new(Some(&state.vpn_status));
    status.add_css_class("caption");
    status.add_css_class("dim-label");
    status.set_xalign(0.0);
    status.set_wrap(true);
    sidebar.append(&status);
}

fn build_devices_page(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    build_network_hero(app, page, state);

    if !state.error.trim().is_empty() {
        let card = card();
        row_label(&card, "Status", &state.error, "dialog-warning-symbolic");
        page.append(&card);
    }

    let Some(network) = active_network(state).cloned() else {
        build_network_setup(app, page, state);
        return;
    };
    let selected_key = app.borrow().selected_device_pubkey.clone();

    let devices = card();
    devices.set_size_request(330, -1);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some("Devices"));
    title.add_css_class("title-3");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    header.append(&title);
    let add = gtk::Button::from_icon_name("list-add-symbolic");
    add.set_tooltip_text(Some("Add device"));
    {
        let app = app.clone();
        add.connect_clicked(move |_| set_page(&app, Page::Share));
    }
    header.append(&add);
    devices.append(&header);

    let participants = sorted_participants(&network, state);

    if participants.is_empty() {
        empty_row(&devices, "No devices yet");
    } else {
        for participant in participants {
            let selected = selected_key
                .as_deref()
                .map(|key| participant_key(&participant) == key)
                .unwrap_or(false);
            device_row(app, &devices, &network, &participant, state, selected);
        }
    }

    if network.local_is_admin {
        let expander = gtk::Expander::new(Some("Manage devices"));
        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        body.set_margin_top(10);

        let input_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let npub = entry("npub", &app.borrow().drafts.participant_npub);
        {
            let app = app.clone();
            npub.connect_changed(move |entry| {
                app.borrow_mut().drafts.participant_npub = entry.text().to_string();
            });
        }
        let alias = entry("Name", &app.borrow().drafts.participant_alias);
        alias.set_width_chars(16);
        {
            let app = app.clone();
            alias.connect_changed(move |entry| {
                app.borrow_mut().drafts.participant_alias = entry.text().to_string();
            });
        }
        let add = icon_text_button("Add", "list-add-symbolic");
        {
            let app = app.clone();
            let network_id = network.id.clone();
            add.connect_clicked(move |_| {
                let (npub, alias) = {
                    let model = app.borrow();
                    (
                        model.drafts.participant_npub.trim().to_string(),
                        model.drafts.participant_alias.trim().to_string(),
                    )
                };
                if npub.is_empty() {
                    return;
                }
                {
                    let mut model = app.borrow_mut();
                    model.drafts.participant_npub.clear();
                    model.drafts.participant_alias.clear();
                }
                dispatch(
                    &app,
                    NativeAppAction::AddParticipant {
                        network_id: network_id.clone(),
                        npub,
                        alias: (!alias.is_empty()).then_some(alias),
                    },
                );
            });
        }
        input_row.append(&npub);
        input_row.append(&alias);
        input_row.append(&add);
        body.append(&input_row);

        for participant in &network.participants {
            let participant_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            participant_row.set_valign(gtk::Align::Center);

            let name = gtk::Label::new(Some(&device_name(participant)));
            name.set_width_chars(16);
            name.set_xalign(0.0);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            participant_row.append(&name);

            let alias = entry("Name", &participant.magic_dns_alias);
            alias.set_width_chars(18);
            participant_row.append(&alias);

            let save = gtk::Button::with_label("Save");
            {
                let app = app.clone();
                let npub = participant.npub.clone();
                let alias = alias.clone();
                save.connect_clicked(move |_| {
                    dispatch(
                        &app,
                        NativeAppAction::SetParticipantAlias {
                            npub: npub.clone(),
                            alias: alias.text().trim().to_string(),
                        },
                    );
                });
            }
            participant_row.append(&save);

            if !is_self(participant, state) {
                let admin = gtk::Button::from_icon_name(if participant.is_admin {
                    "starred-symbolic"
                } else {
                    "non-starred-symbolic"
                });
                admin.set_tooltip_text(Some(if participant.is_admin {
                    "Remove admin"
                } else {
                    "Make admin"
                }));
                {
                    let app = app.clone();
                    let network_id = network.id.clone();
                    let npub = participant.npub.clone();
                    let is_admin = participant.is_admin;
                    admin.connect_clicked(move |_| {
                        dispatch(
                            &app,
                            if is_admin {
                                NativeAppAction::RemoveAdmin {
                                    network_id: network_id.clone(),
                                    npub: npub.clone(),
                                }
                            } else {
                                NativeAppAction::AddAdmin {
                                    network_id: network_id.clone(),
                                    npub: npub.clone(),
                                }
                            },
                        );
                    });
                }
                participant_row.append(&admin);

                let remove = gtk::Button::from_icon_name("edit-delete-symbolic");
                remove.set_tooltip_text(Some("Remove device"));
                remove.add_css_class("destructive-action");
                connect_remove_participant_confirmation(
                    &remove,
                    app,
                    network.id.clone(),
                    participant.npub.clone(),
                    device_name(participant),
                );
                participant_row.append(&remove);
            }

            body.append(&participant_row);
        }

        expander.set_child(Some(&body));
        devices.append(&expander);
    }

    let split = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    split.set_hexpand(true);
    split.set_valign(gtk::Align::Start);
    split.append(&devices);

    let detail = device_detail_card(app, &network, state, selected_key.as_deref());
    detail.set_hexpand(true);
    split.append(&detail);
    page.append(&split);

    append_join_requests(app, page, &network);
}

fn append_join_requests(app: &AppRef, parent: &gtk::Box, network: &NativeNetworkState) {
    if network.inbound_join_requests.is_empty() {
        return;
    }

    let requests = card();
    section_header(&requests, "Join Requests", "contact-new-symbolic");
    for request in &network.inbound_join_requests {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.set_valign(gtk::Align::Center);

        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let name = if request.requester_node_name.trim().is_empty() {
            "New device".to_string()
        } else {
            request.requester_node_name.clone()
        };
        let title = gtk::Label::new(Some(&name));
        title.set_xalign(0.0);
        title.add_css_class("heading");
        text.append(&title);
        let sub = gtk::Label::new(Some(&format!(
            "{}  {}",
            short_text(&request.requester_npub, 18),
            request.requested_at_text
        )));
        sub.add_css_class("caption");
        sub.add_css_class("dim-label");
        sub.set_xalign(0.0);
        text.append(&sub);
        text.set_hexpand(true);
        row.append(&text);

        let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy.set_tooltip_text(Some("Copy npub"));
        {
            let npub = request.requester_npub.clone();
            copy.connect_clicked(move |_| copy_text(&npub));
        }
        row.append(&copy);

        let reject = icon_text_button("Reject", "");
        reject.add_css_class("destructive-action");
        {
            let app = app.clone();
            let network_id = network.id.clone();
            let requester_npub = request.requester_npub.clone();
            reject.connect_clicked(move |_| {
                dispatch(
                    &app,
                    NativeAppAction::RejectJoinRequest {
                        network_id: network_id.clone(),
                        requester_npub: requester_npub.clone(),
                    },
                );
            });
        }
        row.append(&reject);

        let accept = icon_text_button("Accept", "");
        accept.add_css_class("suggested-action");
        {
            let app = app.clone();
            let network_id = network.id.clone();
            let requester_npub = request.requester_npub.clone();
            accept.connect_clicked(move |_| {
                dispatch(
                    &app,
                    NativeAppAction::AcceptJoinRequest {
                        network_id: network_id.clone(),
                        requester_npub: requester_npub.clone(),
                    },
                );
            });
        }
        row.append(&accept);
        requests.append(&row);
    }
    parent.append(&requests);
}

fn device_detail_card(
    app: &AppRef,
    network: &NativeNetworkState,
    state: &NativeAppState,
    selected_key: Option<&str>,
) -> gtk::Box {
    let detail = card();

    let Some(participant) = selected_participant(network, state, selected_key) else {
        let title = gtk::Label::new(Some("Devices"));
        title.add_css_class("title-2");
        title.set_xalign(0.0);
        detail.append(&title);
        empty_row(&detail, "No devices yet");
        return detail;
    };

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.set_valign(gtk::Align::Start);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 8);
    text.set_hexpand(true);
    let name = gtk::Label::new(Some(&device_name(&participant)));
    name.add_css_class("title-2");
    name.set_xalign(0.0);
    name.set_wrap(true);
    text.append(&name);

    let badges = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    if is_self(&participant, state) {
        badges.append(&badge("This device", "ok"));
    }
    if participant.is_admin {
        badges.append(&badge("Admin", "muted"));
    }
    if participant.offers_exit_node {
        badges.append(&badge(
            exit_node_badge_text(&participant, state),
            exit_node_badge_style(&participant, state),
        ));
    }
    match fips_path_kind(&participant) {
        FipsPathKind::Direct => badges.append(&badge("direct connection", "ok")),
        FipsPathKind::Routed => badges.append(&badge("via mesh", "muted")),
        _ => {}
    }
    text.append(&badges);
    header.append(&text);

    let status = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    status.set_valign(gtk::Align::Start);
    let dot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    dot.add_css_class(if participant.reachable {
        "nvpn-peer-online"
    } else {
        "nvpn-peer-offline"
    });
    status.append(&dot);
    let status_label = gtk::Label::new(Some(&device_status_text(&participant)));
    status_label.add_css_class("dim-label");
    status.append(&status_label);
    header.append(&status);
    detail.append(&header);

    let participant_is_self = is_self(&participant, state);
    if network.local_is_admin {
        let manage = gtk::Box::new(gtk::Orientation::Vertical, 10);
        section_header(&manage, "Manage Device", "changes-allow-symbolic");

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_valign(gtk::Align::Center);
        let label = gtk::Label::new(Some("Name"));
        label.add_css_class("dim-label");
        row.append(&label);

        let alias = entry("Name", &participant.magic_dns_alias);
        alias.set_hexpand(true);
        row.append(&alias);

        let save = gtk::Button::with_label("Save");
        {
            let app = app.clone();
            let npub = participant.npub.clone();
            let alias = alias.clone();
            save.connect_clicked(move |_| {
                dispatch(
                    &app,
                    NativeAppAction::SetParticipantAlias {
                        npub: npub.clone(),
                        alias: alias.text().trim().to_string(),
                    },
                );
            });
        }
        row.append(&save);

        if !participant_is_self {
            let hint_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            hint_row.set_valign(gtk::Align::Center);
            let hint_label = gtk::Label::new(Some("Hints"));
            hint_label.add_css_class("dim-label");
            hint_row.append(&hint_label);

            let hints = entry(
                "host or host:port",
                &participant.fips_endpoint_hints.join(", "),
            );
            hints.set_hexpand(true);
            hint_row.append(&hints);

            let save_hints = gtk::Button::with_label("Save");
            {
                let app = app.clone();
                let npub = participant.npub.clone();
                let hints = hints.clone();
                save_hints.connect_clicked(move |_| {
                    dispatch(
                        &app,
                        NativeAppAction::SetParticipantEndpointHints {
                            npub: npub.clone(),
                            endpoint_hints: parse_endpoint_hints(&hints.text()),
                        },
                    );
                });
            }
            hint_row.append(&save_hints);
            manage.append(&hint_row);

            let admin = gtk::Button::from_icon_name(if participant.is_admin {
                "starred-symbolic"
            } else {
                "non-starred-symbolic"
            });
            admin.set_tooltip_text(Some(if participant.is_admin {
                "Remove admin"
            } else {
                "Make admin"
            }));
            {
                let app = app.clone();
                let network_id = network.id.clone();
                let npub = participant.npub.clone();
                let is_admin = participant.is_admin;
                admin.connect_clicked(move |_| {
                    dispatch(
                        &app,
                        if is_admin {
                            NativeAppAction::RemoveAdmin {
                                network_id: network_id.clone(),
                                npub: npub.clone(),
                            }
                        } else {
                            NativeAppAction::AddAdmin {
                                network_id: network_id.clone(),
                                npub: npub.clone(),
                            }
                        },
                    );
                });
            }
            row.append(&admin);

            let remove = gtk::Button::from_icon_name("edit-delete-symbolic");
            remove.set_tooltip_text(Some("Remove device"));
            remove.add_css_class("destructive-action");
            connect_remove_participant_confirmation(
                &remove,
                app,
                network.id.clone(),
                participant.npub.clone(),
                device_name(&participant),
            );
            row.append(&remove);
        }
        manage.append(&row);
        detail.append(&manage);
    }

    let addresses = gtk::Box::new(gtk::Orientation::Vertical, 8);
    section_header(&addresses, "Addresses", "");
    detail_row(
        &addresses,
        "MagicDNS",
        &device_magic_dns_name(&participant, state),
    );
    detail_row(&addresses, "VPN IP", &clean_ip(&participant.tunnel_ip));
    detail_row(&addresses, "Device ID", &participant.npub);
    let copy = icon_text_button("Copy device ID", "edit-copy-symbolic");
    {
        let npub = participant.npub.clone();
        copy.connect_clicked(move |_| copy_text(&npub));
    }
    addresses.append(&copy);
    detail.append(&addresses);

    let connectivity = gtk::Box::new(gtk::Orientation::Vertical, 8);
    section_header(&connectivity, "Connectivity", "");
    let metrics = gtk::FlowBox::new();
    metrics.set_selection_mode(gtk::SelectionMode::None);
    metrics.set_max_children_per_line(3);
    metrics.set_min_children_per_line(2);
    for (title, value) in [
        ("Role", device_role_text(&participant, state)),
        ("State", device_status_text(&participant)),
        ("FIPS path", fips_path_text(&participant)),
        (
            "Address hints",
            if participant.fips_endpoint_hints.is_empty() {
                "-".to_string()
            } else {
                participant.fips_endpoint_hints.join(", ")
            },
        ),
        ("Last seen", non_empty_or(&participant.last_seen_text, "-")),
        ("Sent", format_bytes(participant.tx_bytes)),
        ("Received", format_bytes(participant.rx_bytes)),
    ] {
        let item = metric(title, &value);
        metrics.insert(&item, -1);
    }
    connectivity.append(&metrics);
    if !participant.status_text.trim().is_empty() {
        let status = gtk::Label::new(Some(&participant.status_text));
        status.add_css_class("caption");
        status.add_css_class("dim-label");
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.set_selectable(true);
        connectivity.append(&status);
    }
    detail.append(&connectivity);

    detail
}

fn build_network_hero(_app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    let hero = card();
    hero.add_css_class("nvpn-hero");

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    top.set_valign(gtk::Align::Center);

    let status = gtk::Box::new(gtk::Orientation::Vertical, 0);
    status.add_css_class(if state.exit_node_blocked {
        "nvpn-status-blocked"
    } else if state.mesh_ready {
        "nvpn-status-ready"
    } else if state.vpn_active {
        "nvpn-status-active"
    } else {
        "nvpn-status-off"
    });
    status.set_valign(gtk::Align::Center);
    top.append(&status);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 6);
    text.set_hexpand(true);
    let network = active_network(state);
    let network_name = network
        .map(display_network_name)
        .unwrap_or_else(|| "Nostr VPN".to_string());
    let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    title_row.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(&network_name));
    title.add_css_class("title-1");
    title.set_xalign(0.0);
    title.set_wrap(true);
    title_row.append(&title);
    if network.is_some_and(|network| network.local_is_admin) {
        title_row.append(&badge("Admin", "muted"));
    }
    text.append(&title_row);

    let subtitle = gtk::Label::new(Some(&hero_subtitle(state)));
    subtitle.add_css_class("dim-label");
    subtitle.set_xalign(0.0);
    subtitle.set_wrap(true);
    text.append(&subtitle);

    let badges = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    badges.append(&badge(
        if state.vpn_active {
            "VPN on"
        } else {
            "VPN off"
        },
        if state.vpn_active { "ok" } else { "muted" },
    ));
    badges.append(&badge(
        if state.daemon_running {
            "Daemon"
        } else {
            "Daemon off"
        },
        if state.daemon_running { "ok" } else { "muted" },
    ));
    badges.append(&badge(
        if state.mesh_ready {
            "Mesh ready"
        } else {
            "Mesh pending"
        },
        if state.mesh_ready { "ok" } else { "muted" },
    ));
    if service_update_recommended(state) {
        badges.append(&badge("Update", "warn"));
    }
    if state.exit_node_blocked {
        badges.append(&badge("Internet blocked", "bad"));
    } else if state.exit_node_active && !state.exit_node_status_text.trim().is_empty() {
        badges.append(&badge(&state.exit_node_status_text, "ok"));
    }
    text.append(&badges);
    top.append(&text);

    hero.append(&top);

    let identity = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    identity.set_valign(gtk::Align::Center);
    identity.set_margin_top(8);
    let own = gtk::Label::new(Some(&format!(
        "This device  {}",
        non_empty_or(&short_text(&state.own_npub, 18), "-")
    )));
    own.add_css_class("caption");
    own.add_css_class("dim-label");
    own.set_xalign(0.0);
    own.set_selectable(true);
    own.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    own.set_hexpand(true);
    identity.append(&own);
    let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy.set_tooltip_text(Some("Copy npub"));
    copy.set_sensitive(!state.own_npub.is_empty());
    {
        let npub = state.own_npub.clone();
        copy.connect_clicked(move |_| copy_text(&npub));
    }
    identity.append(&copy);
    if !clean_ip(&state.tunnel_ip).is_empty() {
        identity.append(&badge(&clean_ip(&state.tunnel_ip), "muted"));
    }
    if !state.exit_node_status_text.trim().is_empty() {
        identity.append(&badge(
            &state.exit_node_status_text,
            if state.exit_node_blocked {
                "bad"
            } else if state.exit_node_active {
                "ok"
            } else {
                "warn"
            },
        ));
    }
    hero.append(&identity);

    page.append(&hero);
}

fn build_network_setup(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    let create_card = card();
    section_header(&create_card, "Create Network", "list-add-symbolic");
    let create_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = entry("Network name", &app.borrow().drafts.new_network_name);
    {
        let app = app.clone();
        name.connect_changed(move |entry| {
            app.borrow_mut().drafts.new_network_name = entry.text().to_string();
        });
    }
    let create = icon_text_button("Create", "list-add-symbolic");
    {
        let app = app.clone();
        create.connect_clicked(move |_| {
            let name = {
                let mut model = app.borrow_mut();
                let name = model.drafts.new_network_name.trim().to_string();
                model.drafts.new_network_name.clear();
                name
            };
            create_network(&app, name);
        });
    }
    create_row.append(&name);
    create_row.append(&create);
    create_card.append(&create_row);
    page.append(&create_card);

    let join_card = card();
    section_header(&join_card, "Join Network", "go-down-symbolic");
    let import_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let invite_entry = entry("Paste invite", &app.borrow().drafts.invite);
    {
        let app = app.clone();
        invite_entry.connect_changed(move |entry| {
            let value = entry.text().to_string();
            app.borrow_mut().drafts.invite.clone_from(&value);
            let trimmed = value.trim();
            if trimmed.starts_with("nvpn://invite/") {
                import_invite(&app, trimmed.to_string());
            }
        });
    }
    let import = icon_text_button("Import", "go-down-symbolic");
    {
        let app = app.clone();
        import.connect_clicked(move |_| {
            let invite = app.borrow().drafts.invite.trim().to_string();
            import_invite(&app, invite);
        });
    }
    let camera = icon_text_button("Scan", "camera-photo-symbolic");
    {
        let app = app.clone();
        camera.connect_clicked(move |button| scan_invite_qr(&app, button));
    }
    let image = icon_text_button("From file", "insert-image-symbolic");
    {
        let app = app.clone();
        image.connect_clicked(move |button| choose_invite_qr_image(&app, button));
    }
    import_row.append(&invite_entry);
    import_row.append(&import);
    import_row.append(&camera);
    import_row.append(&image);
    join_card.append(&import_row);
    append_manual_join(app, &join_card);
    append_notice(app, &join_card, "Join");
    page.append(&join_card);

    let nearby = card();
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_valign(gtk::Align::Center);
    section_header(&header, "Nearby invites", "");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    header.append(&spacer);
    let nearby_label = if state.nearby_discovery_active {
        format!(
            "Finding nearby · {}",
            remaining_text(state.nearby_discovery_remaining_secs)
        )
    } else {
        "Find nearby".to_string()
    };
    let lan = icon_text_button(
        &nearby_label,
        if state.nearby_discovery_active {
            "media-playback-stop-symbolic"
        } else {
            "system-search-symbolic"
        },
    );
    {
        let app = app.clone();
        let active = state.nearby_discovery_active;
        lan.connect_clicked(move |_| {
            dispatch(
                &app,
                if active {
                    NativeAppAction::StopNearbyDiscovery
                } else {
                    NativeAppAction::StartNearbyDiscovery
                },
            );
        });
    }
    header.append(&lan);
    nearby.append(&header);
    if state.lan_peers.is_empty() {
        empty_row(&nearby, "No nearby invites");
    } else {
        for peer in &state.lan_peers {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let title = gtk::Label::new(Some(if peer.node_name.trim().is_empty() {
                &peer.network_name
            } else {
                &peer.node_name
            }));
            title.set_xalign(0.0);
            title.add_css_class("heading");
            text.append(&title);
            let sub = gtk::Label::new(Some(&peer.last_seen_text));
            sub.add_css_class("caption");
            sub.add_css_class("dim-label");
            sub.set_xalign(0.0);
            text.append(&sub);
            text.set_hexpand(true);
            row.append(&text);
            let join = icon_text_button("Join", "go-next-symbolic");
            {
                let app = app.clone();
                let invite = peer.invite.clone();
                join.connect_clicked(move |_| import_invite(&app, invite.clone()));
            }
            row.append(&join);
            nearby.append(&row);
        }
    }
    page.append(&nearby);
}

fn build_share_page(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    page_title(page, "Share", "emblem-shared-symbolic");

    let Some(network) = active_network(state).cloned() else {
        return;
    };

    let invite = card();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    row.set_valign(gtk::Align::Start);
    row.append(&qr::build(&state.active_network_invite, 260));

    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.set_hexpand(true);
    section_header(&column, "Invite Devices", "emblem-shared-symbolic");

    let invite_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let copy = icon_text_button("Copy Link", "edit-copy-symbolic");
    copy.set_sensitive(!state.active_network_invite.is_empty());
    {
        let invite = state.active_network_invite.clone();
        copy.connect_clicked(move |_| copy_text(&invite));
    }
    invite_row.append(&copy);
    let reset = icon_text_button("Reset", "view-refresh-symbolic");
    reset.set_sensitive(network.local_is_admin && network.enabled);
    {
        let app = app.clone();
        let network_id = network.id.clone();
        reset.connect_clicked(move |_| {
            dispatch(
                &app,
                NativeAppAction::ResetNetworkInvite {
                    network_id: network_id.clone(),
                },
            );
        });
    }
    invite_row.append(&reset);
    let broadcast_label = if state.invite_broadcast_active {
        format!(
            "Sharing nearby · {}",
            remaining_text(state.invite_broadcast_remaining_secs)
        )
    } else {
        "Share invite nearby".to_string()
    };
    let broadcast = icon_text_button(
        &broadcast_label,
        if state.invite_broadcast_active {
            "media-playback-stop-symbolic"
        } else {
            "network-wireless-symbolic"
        },
    );
    {
        let app = app.clone();
        let active = state.invite_broadcast_active;
        broadcast.connect_clicked(move |_| {
            dispatch(
                &app,
                if active {
                    NativeAppAction::StopInviteBroadcast
                } else {
                    NativeAppAction::StartInviteBroadcast
                },
            );
        });
    }
    invite_row.append(&broadcast);
    column.append(&invite_row);
    switch_row_enabled(
        app,
        &column,
        "Allow join requests",
        network.join_requests_enabled,
        network.local_is_admin,
        {
            let network_id = network.id.clone();
            move |enabled| NativeAppAction::SetNetworkJoinRequestsEnabled {
                network_id: network_id.clone(),
                enabled,
            }
        },
    );

    let add_network_join_status = app.borrow().add_network_join_status.clone();
    if !add_network_join_status.trim().is_empty() || network.outbound_join_request.is_some() {
        column.append(&badge("Join request sent", "warn"));
    } else if !network.invite_inviter_npub.is_empty() {
        let request = icon_text_button("Request Access", "contact-new-symbolic");
        {
            let app = app.clone();
            let network_id = network.id.clone();
            request.connect_clicked(move |_| {
                let state = dispatch(
                    &app,
                    NativeAppAction::RequestNetworkJoin {
                        network_id: network_id.clone(),
                    },
                );
                if state.error.trim().is_empty() {
                    app.borrow_mut().add_network_join_status = "Join request sent".to_string();
                    render(&app);
                }
            });
        }
        column.append(&request);
    }

    row.append(&column);
    invite.append(&row);
    page.append(&invite);

    append_join_requests(app, page, &network);

    let join_card = card();
    section_header(&join_card, "Join Network", "go-down-symbolic");

    let import_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let invite_entry = entry("Paste invite", &app.borrow().drafts.invite);
    {
        let app = app.clone();
        invite_entry.connect_changed(move |entry| {
            let value = entry.text().to_string();
            app.borrow_mut().drafts.invite.clone_from(&value);
            // Auto-import as soon as the entry holds a valid invite URL —
            // no extra click required. Mirrors the Windows / mobile UX.
            let trimmed = value.trim();
            if trimmed.starts_with("nvpn://invite/") {
                import_invite(&app, trimmed.to_string());
            }
        });
    }
    let import = icon_text_button("Import", "go-down-symbolic");
    {
        let app = app.clone();
        import.connect_clicked(move |_| {
            let invite = app.borrow().drafts.invite.trim().to_string();
            import_invite(&app, invite);
        });
    }
    let image = icon_text_button("From file", "insert-image-symbolic");
    {
        let app = app.clone();
        image.connect_clicked(move |button| choose_invite_qr_image(&app, button));
    }
    let camera = icon_text_button("Scan", "camera-photo-symbolic");
    {
        let app = app.clone();
        camera.connect_clicked(move |button| scan_invite_qr(&app, button));
    }
    import_row.append(&invite_entry);
    import_row.append(&import);
    import_row.append(&camera);
    import_row.append(&image);
    join_card.append(&import_row);
    append_manual_join(app, &join_card);
    append_notice(app, &join_card, "Import");

    page.append(&join_card);

    let nearby = card();
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_valign(gtk::Align::Center);
    section_header(&header, "Nearby invites", "");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    header.append(&spacer);
    let nearby_label = if state.nearby_discovery_active {
        format!(
            "Finding nearby · {}",
            remaining_text(state.nearby_discovery_remaining_secs)
        )
    } else {
        "Find nearby".to_string()
    };
    let lan = icon_text_button(
        &nearby_label,
        if state.nearby_discovery_active {
            "media-playback-stop-symbolic"
        } else {
            "system-search-symbolic"
        },
    );
    {
        let app = app.clone();
        let active = state.nearby_discovery_active;
        lan.connect_clicked(move |_| {
            dispatch(
                &app,
                if active {
                    NativeAppAction::StopNearbyDiscovery
                } else {
                    NativeAppAction::StartNearbyDiscovery
                },
            );
        });
    }
    header.append(&lan);
    nearby.append(&header);

    if state.lan_peers.is_empty() {
        empty_row(&nearby, "No nearby invites");
    } else {
        for peer in &state.lan_peers {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let name = if peer.node_name.trim().is_empty() {
                short_text(&peer.npub, 20)
            } else {
                peer.node_name.clone()
            };
            let title = gtk::Label::new(Some(&name));
            title.set_xalign(0.0);
            title.add_css_class("heading");
            text.append(&title);
            let sub = gtk::Label::new(Some(&format!(
                "{}  {}",
                peer.network_name, peer.last_seen_text
            )));
            sub.add_css_class("caption");
            sub.add_css_class("dim-label");
            sub.set_xalign(0.0);
            text.append(&sub);
            text.set_hexpand(true);
            row.append(&text);

            let join = icon_text_button("Join", "go-next-symbolic");
            {
                let app = app.clone();
                let invite = peer.invite.clone();
                join.connect_clicked(move |_| {
                    import_invite(&app, invite.clone());
                });
            }
            row.append(&join);
            nearby.append(&row);
        }
    }
    page.append(&nearby);
}

fn choose_invite_qr_image(app: &AppRef, button: &gtk::Button) {
    let parent = button
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok());
    let dialog = gtk::FileDialog::builder()
        .title("Import QR image")
        .accept_label("Import")
        .build();
    let app = app.clone();
    dialog.open(parent.as_ref(), gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else {
            return;
        };
        let Some(path) = file.path() else {
            set_notice(&app, "Could not open image");
            return;
        };
        match qr_scan::decode_from_path(&path) {
            Ok(invite) => import_invite(&app, invite),
            Err(error) => set_notice(&app, error),
        }
    });
}

fn scan_invite_qr(app: &AppRef, button: &gtk::Button) {
    let parent = button
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok());
    let app_for_result = app.clone();
    let app_for_error = app.clone();
    qr_scan::open_scanner(
        parent.as_ref(),
        move |invite| import_invite(&app_for_result, invite),
        move |error| set_notice(&app_for_error, error),
    );
}

fn append_notice(app: &AppRef, parent: &gtk::Box, title: &str) {
    let notice = app.borrow().notice.clone();
    if !notice.trim().is_empty() {
        row_label(parent, title, &notice, "dialog-warning-symbolic");
    }
}

fn append_manual_join(app: &AppRef, parent: &gtk::Box) {
    let manual = gtk::Expander::new(Some("Add manually"));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.set_margin_top(8);

    let admin = entry("Admin Device ID", &app.borrow().drafts.manual_join_admin_id);
    {
        let app = app.clone();
        admin.connect_changed(move |entry| {
            app.borrow_mut().drafts.manual_join_admin_id = entry.text().to_string();
        });
    }
    body.append(&admin);

    let network = entry(
        "Network ID",
        &display_network_id(&app.borrow().drafts.manual_join_network_id),
    );
    {
        let app = app.clone();
        network.connect_changed(move |entry| {
            app.borrow_mut().drafts.manual_join_network_id = entry.text().to_string();
        });
    }
    body.append(&network);

    let add = icon_text_button("Add", "list-add-symbolic");
    add.set_halign(gtk::Align::Start);
    {
        let app = app.clone();
        add.connect_clicked(move |_| manual_add_network(&app));
    }
    body.append(&add);

    manual.set_child(Some(&body));
    parent.append(&manual);
}

fn manual_add_network(app: &AppRef) {
    let (admin_npub, mesh_network_id) = {
        let model = app.borrow();
        (
            model.drafts.manual_join_admin_id.trim().to_string(),
            normalize_network_id_input(&model.drafts.manual_join_network_id),
        )
    };
    if admin_npub.is_empty() || mesh_network_id.is_empty() {
        return;
    }
    if !is_valid_device_id(&admin_npub) {
        set_notice(app, "Not a valid device ID");
        return;
    }
    {
        let mut model = app.borrow_mut();
        model.drafts.manual_join_admin_id.clear();
        model.drafts.manual_join_network_id.clear();
        model.notice.clear();
    }
    dispatch(
        app,
        NativeAppAction::ManualAddNetwork {
            admin_npub,
            mesh_network_id,
        },
    );
}

fn build_exit_nodes_page(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    page_title(page, "Exit Nodes", "");

    let Some(network) = active_network(state).cloned() else {
        build_wireguard_settings_card(app, page, state);
        return;
    };

    let exit = card();
    section_header(&exit, "Exit Node", "");

    let all_exit_candidates = exit_node_candidates(&network, state);
    let show_search = all_exit_candidates.len() > SEARCH_VISIBILITY_THRESHOLD;
    if show_search {
        let search = entry("Search devices", &app.borrow().drafts.exit_search);
        {
            let app = app.clone();
            search.connect_changed(move |entry| {
                app.borrow_mut().drafts.exit_search = entry.text().to_string();
            });
        }
        exit.append(&search);
    }

    let direct_selected = !state.wireguard_exit_enabled && state.exit_node.is_empty();
    route_choice(
        app,
        &exit,
        "Direct",
        "Use normal internet routing",
        direct_selected,
        true,
        ExitChoice::Direct,
    );

    let wg_subtitle = if !state.wireguard_exit_configured {
        "No WireGuard config saved yet".to_string()
    } else if state.wireguard_exit_endpoint.is_empty() {
        "Configured".to_string()
    } else {
        state.wireguard_exit_endpoint.clone()
    };
    route_choice(
        app,
        &exit,
        "WireGuard upstream",
        &wg_subtitle,
        state.wireguard_exit_enabled,
        state.wireguard_exit_configured,
        ExitChoice::WireGuard,
    );

    let query = if show_search {
        app.borrow().drafts.exit_search.trim().to_ascii_lowercase()
    } else {
        String::new()
    };
    let exit_candidates = all_exit_candidates
        .into_iter()
        .filter(|participant| {
            query.is_empty()
                || device_name(participant)
                    .to_ascii_lowercase()
                    .contains(&query)
                || participant.npub.to_ascii_lowercase().contains(&query)
        })
        .collect::<Vec<_>>();
    if exit_candidates.is_empty() {
        empty_row(
            &exit,
            if query.is_empty() {
                "No exit nodes offered"
            } else {
                "No exit nodes found"
            },
        );
    } else {
        for participant in exit_candidates {
            let peer_selected =
                !state.wireguard_exit_enabled && state.exit_node == participant.npub;
            route_choice(
                app,
                &exit,
                &device_name(&participant),
                non_empty_or(&participant.status_text, "Exit node").as_str(),
                peer_selected,
                true,
                ExitChoice::Peer(participant.npub.clone()),
            );
        }
    }
    page.append(&exit);

    let offer = card();
    switch_row(
        app,
        &offer,
        "Offer this device as an exit node",
        state.advertise_exit_node,
        |enabled| NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                advertise_exit_node: Some(enabled),
                ..SettingsPatch::default()
            },
        },
    );
    switch_row(
        app,
        &offer,
        "Block internet if exit node disconnects",
        state.exit_node_leak_protection,
        |enabled| NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                exit_node_leak_protection: Some(enabled),
                ..SettingsPatch::default()
            },
        },
    );
    page.append(&offer);
    build_wireguard_settings_card(app, page, state);
}

#[derive(Clone)]
enum ExitChoice {
    Direct,
    WireGuard,
    Peer(String),
}

fn route_choice(
    app: &AppRef,
    parent: &gtk::Box,
    title: &str,
    subtitle: &str,
    selected: bool,
    enabled: bool,
    choice: ExitChoice,
) {
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("nvpn-route-choice");
    button.set_sensitive(enabled);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_valign(gtk::Align::Center);
    let icon = gtk::Image::from_icon_name(if selected {
        "object-select-symbolic"
    } else {
        "radio-symbolic"
    });
    if selected {
        icon.add_css_class("success");
    }
    row.append(&icon);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let title_label = gtk::Label::new(Some(title));
    title_label.set_xalign(0.0);
    title_label.add_css_class("heading");
    text.append(&title_label);
    let subtitle_label = gtk::Label::new(Some(subtitle));
    subtitle_label.set_xalign(0.0);
    subtitle_label.add_css_class("caption");
    subtitle_label.add_css_class("dim-label");
    text.append(&subtitle_label);
    row.append(&text);

    button.set_child(Some(&row));
    {
        let app = app.clone();
        let choice = choice.clone();
        button.connect_clicked(move |_| {
            // The daemon enforces mutual exclusion (peer vs WG), so
            // each non-direct row only sends the field it owns.
            // Direct needs to flip both because there's nothing to
            // conflict with — it means "neither".
            let patch = match choice.clone() {
                ExitChoice::Direct => SettingsPatch {
                    exit_node: Some(String::new()),
                    wireguard_exit_enabled: Some(false),
                    ..SettingsPatch::default()
                },
                ExitChoice::WireGuard => SettingsPatch {
                    wireguard_exit_enabled: Some(true),
                    ..SettingsPatch::default()
                },
                ExitChoice::Peer(npub) => SettingsPatch {
                    exit_node: Some(npub),
                    ..SettingsPatch::default()
                },
            };
            dispatch(&app, NativeAppAction::UpdateSettings { patch });
        });
    }
    parent.append(&button);
}

fn build_settings_page(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    page_title(page, "Settings", "emblem-system-symbolic");

    let (service_settling, tray_error, update, tray_available) = {
        let model = app.borrow();
        (
            model.service_settling,
            model.tray_error.clone(),
            model.update.clone(),
            model.tray_available,
        )
    };

    let device = card();
    section_header(&device, "This Device", "");
    setting_entry(app, &device, "Name", "node_name");
    setting_entry(app, &device, "Tunnel IP", "tunnel_ip");
    setting_entry(app, &device, "Endpoint", "endpoint");
    setting_entry(app, &device, "Listen Port", "listen_port");

    let save = icon_text_button("Save", "");
    save.add_css_class("suggested-action");
    save.set_halign(gtk::Align::Start);
    {
        let app = app.clone();
        save.connect_clicked(move |_| save_device_settings(&app));
    }
    device.append(&save);
    page.append(&device);

    let general = card();
    section_header(&general, "General", "");
    switch_row(
        app,
        &general,
        "Start VPN automatically",
        state.autoconnect,
        |enabled| NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                autoconnect: Some(enabled),
                ..SettingsPatch::default()
            },
        },
    );
    if state.startup_settings_supported {
        switch_row(
            app,
            &general,
            "Launch on startup",
            state.launch_on_startup,
            |enabled| NativeAppAction::UpdateSettings {
                patch: SettingsPatch {
                    launch_on_startup: Some(enabled),
                    ..SettingsPatch::default()
                },
            },
        );
    }
    if state.tray_behavior_supported {
        switch_row_enabled(
            app,
            &general,
            "Tray on close",
            state.close_to_tray_on_close,
            tray_available,
            |enabled| NativeAppAction::UpdateSettings {
                patch: SettingsPatch {
                    close_to_tray_on_close: Some(enabled),
                    ..SettingsPatch::default()
                },
            },
        );
    }
    page.append(&general);

    let fips = card();
    section_header(&fips, "FIPS", "");
    switch_row(
        app,
        &fips,
        "Connect to non-roster FIPS peers",
        state.connect_to_non_roster_fips_peers,
        |enabled| NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                connect_to_non_roster_fips_peers: Some(enabled),
                ..SettingsPatch::default()
            },
        },
    );
    switch_row(
        app,
        &fips,
        "Find peers over Nostr relays",
        state.fips_nostr_discovery_enabled,
        |enabled| NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                fips_nostr_discovery_enabled: Some(enabled),
                ..SettingsPatch::default()
            },
        },
    );
    switch_row(
        app,
        &fips,
        "Use bootstrap servers",
        state.fips_bootstrap_enabled,
        |enabled| NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                fips_bootstrap_enabled: Some(enabled),
                ..SettingsPatch::default()
            },
        },
    );
    page.append(&fips);

    let public_fips = card();
    section_header(&public_fips, "Public FIPS routing", "");
    let public_fips_help = gtk::Label::new(Some(
        "FIPS gives .fips addresses end-to-end encryption and identity-based routing. Hosts can reach each other without static IPs, domain names, TLS certificates, or NAT port forwarding.",
    ));
    public_fips_help.add_css_class("caption");
    public_fips_help.add_css_class("dim-label");
    public_fips_help.set_wrap(true);
    public_fips_help.set_xalign(0.0);
    public_fips.append(&public_fips_help);
    let learn_fips = gtk::LinkButton::with_label("https://learn.fips.network/", "Learn FIPS");
    learn_fips.set_halign(gtk::Align::Start);
    public_fips.append(&learn_fips);
    switch_row(
        app,
        &public_fips,
        "Route npub.fips outside VPN",
        state.fips_host_tunnel_enabled,
        |enabled| NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                fips_host_tunnel_enabled: Some(enabled),
                ..SettingsPatch::default()
            },
        },
    );
    let public_fips_details = gtk::Box::new(gtk::Orientation::Vertical, 10);
    public_fips_details.set_sensitive(state.fips_host_tunnel_enabled);
    let public_fips_address = public_fips_address(&state.own_npub);
    detail_row(
        &public_fips_details,
        "Your public FIPS address",
        &public_fips_address,
    );
    setting_entry_enabled(
        app,
        &public_fips_details,
        "Public .fips inbound TCP ports",
        "fips_host_inbound_tcp_ports",
        true,
    );
    let save = icon_text_button("Save", "");
    save.add_css_class("suggested-action");
    save.set_halign(gtk::Align::Start);
    save.set_sensitive(state.fips_host_tunnel_enabled);
    {
        let app = app.clone();
        save.connect_clicked(move |_| save_device_settings(&app));
    }
    public_fips_details.append(&save);
    public_fips.append(&public_fips_details);
    page.append(&public_fips);

    let relays = card();
    section_header(&relays, "Relays", "");
    let add_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    add_row.set_valign(gtk::Align::Center);
    let relay_input = entry("wss://relay.example.com", &app.borrow().drafts.relay_input);
    {
        let app = app.clone();
        relay_input.connect_changed(move |entry| {
            app.borrow_mut().drafts.relay_input = entry.text().to_string();
        });
    }
    {
        let app = app.clone();
        relay_input.connect_activate(move |_| add_relay_setting(&app));
    }
    add_row.append(&relay_input);
    let add_relay = gtk::Button::from_icon_name("list-add-symbolic");
    add_relay.set_tooltip_text(Some("Add relay"));
    {
        let app = app.clone();
        add_relay.connect_clicked(move |_| add_relay_setting(&app));
    }
    add_row.append(&add_relay);
    relays.append(&add_row);
    for relay in &state.relays {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_valign(gtk::Align::Center);
        let dot = gtk::Label::new(Some("●"));
        dot.add_css_class(if relay.enabled && relay.status == "connected" {
            "success"
        } else {
            "dim-label"
        });
        row.append(&dot);
        let url = gtk::Label::new(Some(&relay.url));
        url.set_xalign(0.0);
        url.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        url.set_hexpand(true);
        if !relay.enabled {
            url.add_css_class("dim-label");
        }
        row.append(&url);
        let relay_switch = gtk::Switch::builder().active(relay.enabled).build();
        {
            let app = app.clone();
            let url = relay.url.clone();
            relay_switch.connect_active_notify(move |relay_switch| {
                set_relay_enabled(&app, &url, relay_switch.is_active());
            });
        }
        row.append(&relay_switch);
        let delete = gtk::Button::from_icon_name("edit-delete-symbolic");
        delete.set_tooltip_text(Some("Delete relay"));
        {
            let app = app.clone();
            let url = relay.url.clone();
            delete.connect_clicked(move |_| delete_relay_setting(&app, &url));
        }
        row.append(&delete);
        relays.append(&row);
    }
    page.append(&relays);

    let network = card();
    let network_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    section_header(&network_header, "Networks", "");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    network_header.append(&spacer);
    let new_name = entry("New network", &app.borrow().drafts.new_network_name);
    new_name.set_width_chars(18);
    {
        let app = app.clone();
        new_name.connect_changed(move |entry| {
            app.borrow_mut().drafts.new_network_name = entry.text().to_string();
        });
    }
    {
        let app = app.clone();
        new_name.connect_activate(move |_| add_network_from_draft(&app));
    }
    network_header.append(&new_name);
    let add = gtk::Button::from_icon_name("list-add-symbolic");
    add.set_tooltip_text(Some("Add network"));
    {
        let app = app.clone();
        add.connect_clicked(move |_| add_network_from_draft(&app));
    }
    network_header.append(&add);
    network.append(&network_header);

    if let Some(active) = active_network(state).cloned() {
        let rename = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(Some("Active"));
        label.set_width_chars(10);
        label.set_xalign(0.0);
        label.add_css_class("dim-label");
        rename.append(&label);
        let input = entry("Network name", &app.borrow().drafts.network_name);
        {
            let app = app.clone();
            input.connect_changed(move |entry| {
                app.borrow_mut().drafts.network_name = entry.text().to_string();
            });
        }
        {
            let app = app.clone();
            let network_id = active.id.clone();
            input.connect_activate(move |_| save_active_network_name(&app, &network_id));
        }
        let save = gtk::Button::with_label("Save");
        {
            let app = app.clone();
            let network_id = active.id.clone();
            save.connect_clicked(move |_| save_active_network_name(&app, &network_id));
        }
        rename.append(&input);
        rename.append(&save);
        network.append(&rename);

        let mesh = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(Some("Network ID"));
        label.set_width_chars(10);
        label.set_xalign(0.0);
        label.add_css_class("dim-label");
        mesh.append(&label);
        let mesh_id = entry("Network ID", &app.borrow().drafts.mesh_id);
        {
            let app = app.clone();
            mesh_id.connect_changed(move |entry| {
                app.borrow_mut().drafts.mesh_id = entry.text().to_string();
            });
        }
        {
            let app = app.clone();
            let network_id = active.id.clone();
            mesh_id.connect_activate(move |_| save_active_network_mesh_id(&app, &network_id));
        }
        mesh.append(&mesh_id);
        let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy.set_tooltip_text(Some("Copy network ID"));
        {
            let network_id = active.network_id.clone();
            copy.connect_clicked(move |_| copy_text(&network_id));
        }
        mesh.append(&copy);
        let save = gtk::Button::with_label("Save");
        save.set_sensitive(active.local_is_admin);
        {
            let app = app.clone();
            let network_id = active.id.clone();
            save.connect_clicked(move |_| save_active_network_mesh_id(&app, &network_id));
        }
        mesh.append(&save);
        network.append(&mesh);

        switch_row_enabled(
            app,
            &network,
            "Allow join requests",
            active.join_requests_enabled,
            active.local_is_admin,
            {
                let network_id = active.id.clone();
                move |enabled| NativeAppAction::SetNetworkJoinRequestsEnabled {
                    network_id: network_id.clone(),
                    enabled,
                }
            },
        );

        let delete_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        delete_row.set_halign(gtk::Align::Start);
        let delete = icon_text_button("Delete network", "edit-delete-symbolic");
        delete.add_css_class("destructive-action");
        connect_remove_network_confirmation(
            &delete,
            app,
            active.id.clone(),
            display_network_name(&active),
        );
        delete_row.append(&delete);
        network.append(&delete_row);
    }

    let saved = gtk::Expander::new(Some("Saved Networks"));
    let saved_body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    saved_body.set_margin_top(10);
    let inactive = state
        .networks
        .iter()
        .filter(|network| !network.enabled)
        .cloned()
        .collect::<Vec<_>>();
    if inactive.is_empty() {
        empty_row(&saved_body, "No saved networks");
    } else {
        for saved_network in inactive {
            saved_network_row(app, &saved_body, &saved_network, state);
        }
    }
    saved.set_child(Some(&saved_body));
    network.append(&saved);
    page.append(&network);

    let system = card();
    {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_valign(gtk::Align::Center);
        let title = gtk::Label::new(Some("System"));
        title.add_css_class("title-3");
        title.set_xalign(0.0);
        row.append(&title);
        let label = system_version_label(state);
        if !label.is_empty() {
            let version = gtk::Label::new(Some(&label));
            version.add_css_class("caption");
            version.add_css_class("dim-label");
            version.set_selectable(true);
            version.set_xalign(0.0);
            row.append(&version);
        }
        system.append(&row);
    }
    let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    status_row.append(&badge(
        if state.service_installed {
            "Service installed"
        } else {
            "Service missing"
        },
        if state.service_installed {
            "ok"
        } else {
            "warn"
        },
    ));
    status_row.append(&badge(
        if state.service_running {
            "Running"
        } else {
            "Stopped"
        },
        if state.service_running { "ok" } else { "muted" },
    ));
    status_row.append(&badge(
        if state.cli_installed {
            "CLI installed"
        } else {
            "CLI missing"
        },
        if state.cli_installed { "ok" } else { "muted" },
    ));
    if service_update_recommended(state) {
        status_row.append(&badge("Update available", "warn"));
    }
    if service_settling {
        status_row.append(&badge("Settling", "muted"));
    }
    let update_badge = if update.available {
        format!("Update {}", update.version)
    } else {
        "Current".to_string()
    };
    status_row.append(&badge(
        &update_badge,
        if update.available { "warn" } else { "ok" },
    ));
    if update.checking {
        status_row.append(&badge("Checking", "muted"));
    }
    if update.downloading {
        status_row.append(&badge("Downloading", "muted"));
    }
    system.append(&status_row);

    let status_detail = first_non_empty(&[&update.status, &state.service_status_detail]);
    if let Some(status_detail) = status_detail {
        let detail = gtk::Label::new(Some(&status_detail));
        detail.add_css_class("caption");
        detail.add_css_class("dim-label");
        detail.set_xalign(0.0);
        detail.set_wrap(true);
        detail.set_selectable(true);
        system.append(&detail);
    }

    if let Some(error) = tray_error {
        if !error.trim().is_empty() {
            let detail = gtk::Label::new(Some(&format!("Tray unavailable: {error}")));
            detail.add_css_class("caption");
            detail.add_css_class("dim-label");
            detail.set_xalign(0.0);
            detail.set_wrap(true);
            system.append(&detail);
        }
    }

    let cli_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    cli_row.set_halign(gtk::Align::Start);
    let cli = icon_text_button(
        if state.cli_installed {
            "Reinstall CLI"
        } else {
            "Install CLI"
        },
        "utilities-terminal-symbolic",
    );
    cli.set_sensitive(state.cli_install_supported);
    {
        let app = app.clone();
        cli.connect_clicked(move |_| {
            dispatch(&app, NativeAppAction::InstallCli);
        });
    }
    cli_row.append(&cli);
    let check_update_button = icon_text_button("Check Updates", "view-refresh-symbolic");
    check_update_button.set_sensitive(!update.checking && !update.downloading);
    {
        let app = app.clone();
        check_update_button.connect_clicked(move |_| check_updates(&app, true));
    }
    cli_row.append(&check_update_button);
    let download_update_button = icon_text_button("Download Update", "folder-download-symbolic");
    download_update_button.set_sensitive(
        update.available && update.asset.is_some() && !update.checking && !update.downloading,
    );
    {
        let app = app.clone();
        download_update_button.connect_clicked(move |_| download_update(&app));
    }
    cli_row.append(&download_update_button);
    let uninstall_cli = icon_text_button("Uninstall CLI", "edit-delete-symbolic");
    uninstall_cli.set_sensitive(state.cli_install_supported && state.cli_installed);
    {
        let app = app.clone();
        uninstall_cli.connect_clicked(move |_| {
            dispatch(&app, NativeAppAction::UninstallCli);
        });
    }
    cli_row.append(&uninstall_cli);
    system.append(&cli_row);

    if state.service_supported {
        let service_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        service_row.set_halign(gtk::Align::Start);
        let service = icon_text_button(
            if service_update_recommended(state) {
                "Update Service"
            } else if state.service_installed {
                "Reinstall Service"
            } else {
                "Install Service"
            },
            "system-run-symbolic",
        );
        {
            let app = app.clone();
            service.connect_clicked(move |_| {
                dispatch(&app, NativeAppAction::InstallSystemService);
            });
        }
        service_row.append(&service);

        if state.service_enablement_supported && state.service_installed {
            let enable = icon_text_button(
                if state.service_disabled {
                    "Enable Service"
                } else {
                    "Disable Service"
                },
                if state.service_disabled {
                    "object-select-symbolic"
                } else {
                    "media-playback-stop-symbolic"
                },
            );
            {
                let app = app.clone();
                let disabled = state.service_disabled;
                enable.connect_clicked(move |_| {
                    dispatch(
                        &app,
                        if disabled {
                            NativeAppAction::EnableSystemService
                        } else {
                            NativeAppAction::DisableSystemService
                        },
                    );
                });
            }
            service_row.append(&enable);
        }

        let uninstall = icon_text_button("Uninstall Service", "edit-delete-symbolic");
        uninstall.set_sensitive(state.service_installed);
        {
            let app = app.clone();
            uninstall.connect_clicked(move |_| {
                dispatch(&app, NativeAppAction::UninstallSystemService);
            });
        }
        service_row.append(&uninstall);
        system.append(&service_row);
    }
    page.append(&system);

    let advanced = gtk::Expander::new(Some("Advanced"));
    advanced.set_expanded(app.borrow().diagnostics_expanded);
    {
        let app = app.clone();
        advanced.connect_expanded_notify(move |advanced| {
            app.borrow_mut().diagnostics_expanded = advanced.is_expanded();
        });
    }
    let advanced_body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    advanced_body.set_margin_top(10);
    build_diagnostics(&advanced_body, state);
    advanced.set_child(Some(&advanced_body));
    page.append(&advanced);
}

fn build_diagnostics(parent: &gtk::Box, state: &NativeAppState) {
    let diagnostics = card();
    section_header(&diagnostics, "Diagnostics", "dialog-information-symbolic");

    let metrics = gtk::FlowBox::new();
    metrics.set_selection_mode(gtk::SelectionMode::None);
    metrics.set_column_spacing(10);
    metrics.set_row_spacing(10);
    metrics.set_max_children_per_line(3);
    let peer_count = format!(
        "{}/{}",
        state.connected_peer_count, state.expected_peer_count
    );
    let fips_peer_count = format!(
        "{}/{} direct",
        state.fips_connected_peer_count, state.fips_roster_peer_count
    );
    let other_fips = state.non_fips_roster_peer_count.to_string();
    metrics.append(&metric("Peers", &peer_count));
    metrics.append(&metric("Roster FIPS", &fips_peer_count));
    metrics.append(&metric("Other FIPS", &other_fips));
    metrics.append(&metric(
        "Interface",
        &non_empty_or(&state.network.default_interface, "unknown"),
    ));
    metrics.append(&metric(
        "IPv4",
        &non_empty_or(&state.network.primary_ipv4, "-"),
    ));
    metrics.append(&metric(
        "IPv6",
        &non_empty_or(&state.network.primary_ipv6, "-"),
    ));
    metrics.append(&metric(
        "Gateway",
        &first_non_empty(&[
            state.network.gateway_ipv4.as_str(),
            state.network.gateway_ipv6.as_str(),
        ])
        .unwrap_or_else(|| "unknown".to_string()),
    ));
    metrics.append(&metric(
        "Mapping",
        &non_empty_or(&state.port_mapping.active_protocol, "none"),
    ));
    metrics.append(&metric(
        "External",
        &non_empty_or(&state.port_mapping.external_endpoint, "stun/direct"),
    ));
    diagnostics.append(&metrics);

    detail_row(&diagnostics, "This device", &state.own_npub);
    detail_row(&diagnostics, "Tunnel IP", &clean_ip(&state.tunnel_ip));
    detail_row(&diagnostics, "Endpoint", &state.endpoint);
    detail_row(&diagnostics, "Config", &state.config_path);
    detail_row(&diagnostics, "MagicDNS", &state.magic_dns_status);
    detail_row(&diagnostics, "Runtime", &state.runtime_status_detail);

    if state.health.is_empty() {
        empty_row(&diagnostics, "No health warnings");
    } else {
        for issue in &state.health {
            let title = if issue.severity.trim().is_empty() {
                issue.summary.clone()
            } else {
                format!("{}  {}", issue.severity, issue.summary)
            };
            row_label(
                &diagnostics,
                &title,
                &issue.detail,
                "dialog-warning-symbolic",
            );
        }
    }
    parent.append(&diagnostics);
}

fn setting_entry(app: &AppRef, parent: &gtk::Box, title: &str, key: &'static str) {
    setting_entry_enabled(app, parent, title, key, true);
}

fn setting_entry_enabled(
    app: &AppRef,
    parent: &gtk::Box,
    title: &str,
    key: &'static str,
    enabled: bool,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_valign(gtk::Align::Center);
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    label.set_width_chars(13);
    row.append(&label);

    let current = {
        let model = app.borrow();
        match key {
            "node_name" => model.drafts.node_name.clone(),
            "endpoint" => model.drafts.endpoint.clone(),
            "tunnel_ip" => model.drafts.tunnel_ip.clone(),
            "listen_port" => model.drafts.listen_port.clone(),
            "fips_host_inbound_tcp_ports" => model.drafts.fips_host_inbound_tcp_ports.clone(),
            _ => String::new(),
        }
    };
    let placeholder = if key == "fips_host_inbound_tcp_ports" {
        ""
    } else {
        title
    };
    let input = entry(placeholder, &current);
    input.set_sensitive(enabled);
    {
        let app = app.clone();
        input.connect_changed(move |entry| {
            let value = entry.text().to_string();
            let mut model = app.borrow_mut();
            match key {
                "node_name" => model.drafts.node_name = value,
                "endpoint" => model.drafts.endpoint = value,
                "tunnel_ip" => model.drafts.tunnel_ip = value,
                "listen_port" => model.drafts.listen_port = value,
                "fips_host_inbound_tcp_ports" => model.drafts.fips_host_inbound_tcp_ports = value,
                _ => {}
            }
        });
    }
    {
        let app = app.clone();
        input.connect_activate(move |_| save_device_settings(&app));
    }
    row.append(&input);
    parent.append(&row);
}

fn add_network_from_draft(app: &AppRef) {
    let name = app.borrow().drafts.new_network_name.trim().to_string();
    if name.is_empty() {
        return;
    }
    app.borrow_mut().drafts.new_network_name.clear();
    dispatch(app, NativeAppAction::AddNetwork { name });
}

fn save_active_network_name(app: &AppRef, network_id: &str) {
    let name = app.borrow().drafts.network_name.trim().to_string();
    if name.is_empty() {
        return;
    }
    dispatch(
        app,
        NativeAppAction::RenameNetwork {
            network_id: network_id.to_string(),
            name,
        },
    );
}

fn save_active_network_mesh_id(app: &AppRef, network_id: &str) {
    let mesh_id = normalize_network_id_input(&app.borrow().drafts.mesh_id);
    if mesh_id.is_empty() {
        return;
    }
    dispatch(
        app,
        NativeAppAction::SetNetworkMeshId {
            network_id: network_id.to_string(),
            mesh_id,
        },
    );
}

fn save_device_settings(app: &AppRef) {
    let drafts = app.borrow().drafts.clone();
    let listen_port = drafts.listen_port.trim().parse::<u16>().ok();
    dispatch(
        app,
        NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                node_name: Some(drafts.node_name),
                endpoint: Some(drafts.endpoint),
                tunnel_ip: Some(drafts.tunnel_ip),
                listen_port,
                fips_host_inbound_tcp_ports: Some(drafts.fips_host_inbound_tcp_ports),
                ..SettingsPatch::default()
            },
        },
    );
}

fn add_relay_setting(app: &AppRef) {
    let (state, input) = {
        let model = app.borrow();
        (model.state.clone(), model.drafts.relay_input.clone())
    };
    let Some(url) = normalize_relay_url(&input) else {
        return;
    };
    let (mut enabled, mut disabled) = relay_lists(&state);
    disabled.retain(|relay| relay != &url);
    if !enabled.contains(&url) {
        enabled.push(url);
    }
    app.borrow_mut().drafts.relay_input.clear();
    save_relay_lists(app, enabled, disabled);
}

fn set_relay_enabled(app: &AppRef, url: &str, enabled_value: bool) {
    let state = app.borrow().state.clone();
    let Some(url) = normalize_relay_url(url) else {
        return;
    };
    let (mut enabled, mut disabled) = relay_lists(&state);
    enabled.retain(|relay| relay != &url);
    disabled.retain(|relay| relay != &url);
    if enabled_value {
        enabled.push(url);
    } else {
        disabled.push(url);
    }
    save_relay_lists(app, enabled, disabled);
}

fn delete_relay_setting(app: &AppRef, url: &str) {
    let state = app.borrow().state.clone();
    let Some(url) = normalize_relay_url(url) else {
        return;
    };
    let (mut enabled, mut disabled) = relay_lists(&state);
    enabled.retain(|relay| relay != &url);
    disabled.retain(|relay| relay != &url);
    save_relay_lists(app, enabled, disabled);
}

fn relay_lists(state: &NativeAppState) -> (Vec<String>, Vec<String>) {
    let enabled = unique_relays(
        state
            .relays
            .iter()
            .filter(|relay| relay.enabled)
            .filter_map(|relay| normalize_relay_url(&relay.url))
            .collect(),
    );
    let mut disabled = unique_relays(
        state
            .relays
            .iter()
            .filter(|relay| !relay.enabled)
            .filter_map(|relay| normalize_relay_url(&relay.url))
            .collect(),
    );
    disabled.retain(|relay| !enabled.contains(relay));
    (enabled, disabled)
}

fn save_relay_lists(app: &AppRef, enabled: Vec<String>, disabled: Vec<String>) {
    let enabled = unique_relays(enabled);
    let mut disabled = unique_relays(disabled);
    disabled.retain(|relay| !enabled.contains(relay));
    dispatch(
        app,
        NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                relays: Some(enabled),
                disabled_relays: Some(disabled),
                ..SettingsPatch::default()
            },
        },
    );
}

fn normalize_relay_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
        Some(trimmed.to_string())
    } else {
        Some(format!("wss://{trimmed}"))
    }
}

fn unique_relays(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn save_wireguard_exit_settings(app: &AppRef) {
    let config = app.borrow().drafts.wireguard_exit_config.clone();
    dispatch(
        app,
        NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                wireguard_exit_config: Some(config),
                ..SettingsPatch::default()
            },
        },
    );
}

fn import_wireguard_exit_config_file(app: &AppRef, button: &gtk::Button) {
    let parent = button
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok());
    let dialog = gtk::FileDialog::builder()
        .title("Import WireGuard config")
        .accept_label("Import")
        .build();
    let app = app.clone();
    dialog.open(parent.as_ref(), gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else {
            return;
        };
        let Some(path) = file.path() else {
            set_notice(&app, "Could not open config file");
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(config) if config.trim().is_empty() => {
                set_notice(&app, "Selected WireGuard config is empty.");
            }
            Ok(config) => {
                app.borrow_mut().drafts.wireguard_exit_config = config.clone();
                dispatch(
                    &app,
                    NativeAppAction::UpdateSettings {
                        patch: SettingsPatch {
                            wireguard_exit_config: Some(config),
                            ..SettingsPatch::default()
                        },
                    },
                );
            }
            Err(error) => set_notice(&app, format!("Could not read config file: {error}")),
        }
    });
}

fn build_wireguard_settings_card(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    let card = card();
    section_header(&card, "WireGuard Upstream", "");
    let note = gtk::Label::new(Some(
        "Paste a WireGuard config from an upstream VPN provider such as Mullvad or Proton VPN.",
    ));
    note.add_css_class("muted");
    note.set_wrap(true);
    note.set_xalign(0.0);
    card.append(&note);
    switch_row(
        app,
        &card,
        "Use WireGuard upstream",
        state.wireguard_exit_enabled,
        |enabled| NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                wireguard_exit_enabled: Some(enabled),
                ..SettingsPatch::default()
            },
        },
    );
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_min_content_height(220);
    scroller.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scroller.set_hexpand(true);

    let config = gtk::TextView::new();
    config.set_monospace(true);
    config.set_wrap_mode(gtk::WrapMode::None);
    config
        .buffer()
        .set_text(&app.borrow().drafts.wireguard_exit_config);
    {
        let app = app.clone();
        config.buffer().connect_changed(move |buffer| {
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
            app.borrow_mut().drafts.wireguard_exit_config = text.to_string();
        });
    }
    scroller.set_child(Some(&config));
    card.append(&scroller);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::Start);

    let import_wg = icon_text_button("Import File", "document-open-symbolic");
    {
        let app = app.clone();
        import_wg.connect_clicked(move |button| import_wireguard_exit_config_file(&app, button));
    }
    actions.append(&import_wg);

    let save_wg = icon_text_button("Save WireGuard", "");
    {
        let app = app.clone();
        save_wg.connect_clicked(move |_| save_wireguard_exit_settings(&app));
    }
    actions.append(&save_wg);
    card.append(&actions);
    page.append(&card);
}

fn saved_network_row(
    app: &AppRef,
    parent: &gtk::Box,
    network: &NativeNetworkState,
    state: &NativeAppState,
) {
    let expander = gtk::Expander::new(None::<&str>);
    expander.add_css_class("nvpn-route-choice");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    header.set_valign(gtk::Align::Center);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    text.set_hexpand(true);
    let name = gtk::Label::new(Some(&display_network_name(network)));
    name.add_css_class("heading");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&name);
    let subtitle = gtk::Label::new(Some(&format!(
        "{} of {} connected  {}",
        network.online_count,
        network.expected_count,
        short_text(&network.network_id, 12)
    )));
    subtitle.add_css_class("caption");
    subtitle.add_css_class("dim-label");
    subtitle.set_xalign(0.0);
    text.append(&subtitle);
    header.append(&text);

    let activate = icon_text_button("Activate", "go-next-symbolic");
    {
        let app = app.clone();
        let network_id = network.id.clone();
        activate.connect_clicked(move |_| {
            dispatch(
                &app,
                NativeAppAction::SetNetworkEnabled {
                    network_id: network_id.clone(),
                    enabled: true,
                },
            );
        });
    }
    header.append(&activate);

    let remove = gtk::Button::from_icon_name("edit-delete-symbolic");
    remove.set_tooltip_text(Some("Remove network"));
    remove.add_css_class("destructive-action");
    connect_remove_network_confirmation(
        &remove,
        app,
        network.id.clone(),
        display_network_name(network),
    );
    header.append(&remove);
    expander.set_label_widget(Some(&header));

    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.set_margin_top(10);

    let rename = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    rename.set_valign(gtk::Align::Center);
    let label = gtk::Label::new(Some("Name"));
    label.set_width_chars(10);
    label.set_xalign(0.0);
    label.add_css_class("dim-label");
    rename.append(&label);
    let input = entry("Network name", &display_network_name(network));
    input.set_sensitive(network.local_is_admin);
    rename.append(&input);
    let save = gtk::Button::with_label("Save");
    save.set_sensitive(network.local_is_admin);
    {
        let app = app.clone();
        let network_id = network.id.clone();
        let input = input.clone();
        save.connect_clicked(move |_| save_saved_network_name(&app, &network_id, &input));
    }
    {
        let app = app.clone();
        let network_id = network.id.clone();
        input.connect_activate(move |input| save_saved_network_name(&app, &network_id, input));
    }
    rename.append(&save);
    body.append(&rename);

    let mesh = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    mesh.set_valign(gtk::Align::Center);
    let label = gtk::Label::new(Some("Network ID"));
    label.set_width_chars(10);
    label.set_xalign(0.0);
    label.add_css_class("dim-label");
    mesh.append(&label);
    let mesh_id = gtk::Label::new(Some(&display_network_id(&network.network_id)));
    mesh_id.set_xalign(0.0);
    mesh_id.set_selectable(true);
    mesh_id.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    mesh_id.set_hexpand(true);
    mesh.append(&mesh_id);
    let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy.set_tooltip_text(Some("Copy network ID"));
    {
        let network_id = network.network_id.clone();
        copy.connect_clicked(move |_| copy_text(&network_id));
    }
    mesh.append(&copy);
    body.append(&mesh);

    switch_row_enabled(
        app,
        &body,
        "Allow join requests",
        network.join_requests_enabled,
        network.local_is_admin,
        {
            let network_id = network.id.clone();
            move |enabled| NativeAppAction::SetNetworkJoinRequestsEnabled {
                network_id: network_id.clone(),
                enabled,
            }
        },
    );

    section_header(&body, "Devices", "");
    let participants = sorted_participants(network, state);
    if participants.is_empty() {
        empty_row(&body, "No devices in this network");
    } else {
        for participant in participants {
            saved_network_participant_row(app, &body, network, &participant, state);
        }
    }

    expander.set_child(Some(&body));
    parent.append(&expander);
}

fn save_saved_network_name(app: &AppRef, network_id: &str, input: &gtk::Entry) {
    let name = input.text().trim().to_string();
    if name.is_empty() {
        return;
    }
    dispatch(
        app,
        NativeAppAction::RenameNetwork {
            network_id: network_id.to_string(),
            name,
        },
    );
}

fn saved_network_participant_row(
    app: &AppRef,
    parent: &gtk::Box,
    network: &NativeNetworkState,
    participant: &NativeParticipantState,
    state: &NativeAppState,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_valign(gtk::Align::Center);

    let dot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    dot.add_css_class(if participant.reachable {
        "nvpn-peer-online"
    } else {
        "nvpn-peer-offline"
    });
    row.append(&dot);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    text.set_hexpand(true);
    let name = gtk::Label::new(Some(&device_name(participant)));
    name.add_css_class("heading");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&name);
    let subtitle = gtk::Label::new(Some(&device_subtitle(participant)));
    subtitle.add_css_class("caption");
    subtitle.add_css_class("dim-label");
    subtitle.set_xalign(0.0);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    text.append(&subtitle);
    row.append(&text);

    row.append(&badge(
        &device_status_text(participant),
        if participant.reachable { "ok" } else { "muted" },
    ));
    if participant.is_admin {
        row.append(&badge("Admin", "muted"));
    }
    if participant.offers_exit_node {
        row.append(&badge(
            exit_node_badge_text(participant, state),
            exit_node_badge_style(participant, state),
        ));
    }

    let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy.set_tooltip_text(Some("Copy npub"));
    {
        let npub = participant.npub.clone();
        copy.connect_clicked(move |_| copy_text(&npub));
    }
    row.append(&copy);

    if network.local_is_admin {
        let alias = entry("Name", &participant.magic_dns_alias);
        alias.set_width_chars(14);
        alias.set_hexpand(false);
        row.append(&alias);

        let save_alias = gtk::Button::from_icon_name("object-select-symbolic");
        save_alias.set_tooltip_text(Some("Save name"));
        {
            let app = app.clone();
            let npub = participant.npub.clone();
            let alias = alias.clone();
            save_alias.connect_clicked(move |_| {
                dispatch(
                    &app,
                    NativeAppAction::SetParticipantAlias {
                        npub: npub.clone(),
                        alias: alias.text().trim().to_string(),
                    },
                );
            });
        }
        {
            let app = app.clone();
            let npub = participant.npub.clone();
            alias.connect_activate(move |alias| {
                dispatch(
                    &app,
                    NativeAppAction::SetParticipantAlias {
                        npub: npub.clone(),
                        alias: alias.text().trim().to_string(),
                    },
                );
            });
        }
        row.append(&save_alias);

        if !is_self(participant, state) {
            let admin = gtk::Button::from_icon_name(if participant.is_admin {
                "starred-symbolic"
            } else {
                "non-starred-symbolic"
            });
            admin.set_tooltip_text(Some(if participant.is_admin {
                "Remove admin"
            } else {
                "Make admin"
            }));
            {
                let app = app.clone();
                let network_id = network.id.clone();
                let npub = participant.npub.clone();
                let is_admin = participant.is_admin;
                admin.connect_clicked(move |_| {
                    dispatch(
                        &app,
                        if is_admin {
                            NativeAppAction::RemoveAdmin {
                                network_id: network_id.clone(),
                                npub: npub.clone(),
                            }
                        } else {
                            NativeAppAction::AddAdmin {
                                network_id: network_id.clone(),
                                npub: npub.clone(),
                            }
                        },
                    );
                });
            }
            row.append(&admin);

            let remove = gtk::Button::from_icon_name("edit-delete-symbolic");
            remove.set_tooltip_text(Some("Remove device"));
            remove.add_css_class("destructive-action");
            connect_remove_participant_confirmation(
                &remove,
                app,
                network.id.clone(),
                participant.npub.clone(),
                device_name(participant),
            );
            row.append(&remove);
        }
    }

    parent.append(&row);
}

fn device_row(
    app: &AppRef,
    parent: &gtk::Box,
    network: &NativeNetworkState,
    participant: &NativeParticipantState,
    state: &NativeAppState,
    selected: bool,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("nvpn-device-row");
    if selected {
        row.add_css_class("selected");
    }
    row.set_valign(gtk::Align::Center);
    let click = gtk::GestureClick::new();
    {
        let app = app.clone();
        let key = participant_key(participant);
        click.connect_released(move |_, _, _, _| {
            app.borrow_mut().selected_device_pubkey = Some(key.clone());
            render(&app);
        });
    }
    row.add_controller(click);

    let dot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    dot.add_css_class(if participant.reachable {
        "nvpn-peer-online"
    } else {
        "nvpn-peer-offline"
    });
    row.append(&dot);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.set_hexpand(true);

    let name_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = gtk::Label::new(Some(&device_name(participant)));
    name.add_css_class("heading");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name_row.append(&name);
    if participant.is_admin {
        name_row.append(&badge("Admin", "muted"));
    }
    if participant.offers_exit_node {
        name_row.append(&badge(
            exit_node_badge_text(participant, state),
            exit_node_badge_style(participant, state),
        ));
    }
    text.append(&name_row);

    let subtitle = gtk::Label::new(Some(&device_subtitle(participant)));
    subtitle.add_css_class("caption");
    subtitle.add_css_class("dim-label");
    subtitle.set_xalign(0.0);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    text.append(&subtitle);
    row.append(&text);

    row.append(&badge(
        &device_status_text(participant),
        if participant.reachable { "ok" } else { "muted" },
    ));
    if matches!(fips_path_kind(participant), FipsPathKind::Routed) {
        row.append(&badge("via mesh", "muted"));
    }

    let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy.set_tooltip_text(Some("Copy npub"));
    {
        let npub = participant.npub.clone();
        copy.connect_clicked(move |_| copy_text(&npub));
    }
    row.append(&copy);

    if network.local_is_admin && !is_self(participant, state) {
        let admin = gtk::Button::from_icon_name(if participant.is_admin {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        });
        admin.set_tooltip_text(Some(if participant.is_admin {
            "Remove admin"
        } else {
            "Make admin"
        }));
        {
            let app = app.clone();
            let network_id = network.id.clone();
            let npub = participant.npub.clone();
            let is_admin = participant.is_admin;
            admin.connect_clicked(move |_| {
                dispatch(
                    &app,
                    if is_admin {
                        NativeAppAction::RemoveAdmin {
                            network_id: network_id.clone(),
                            npub: npub.clone(),
                        }
                    } else {
                        NativeAppAction::AddAdmin {
                            network_id: network_id.clone(),
                            npub: npub.clone(),
                        }
                    },
                );
            });
        }
        row.append(&admin);

        let remove = gtk::Button::from_icon_name("edit-delete-symbolic");
        remove.set_tooltip_text(Some("Remove device"));
        remove.add_css_class("destructive-action");
        connect_remove_participant_confirmation(
            &remove,
            app,
            network.id.clone(),
            participant.npub.clone(),
            device_name(participant),
        );
        row.append(&remove);
    }

    parent.append(&row);
}

fn switch_row<F>(app: &AppRef, parent: &gtk::Box, title: &str, active: bool, action: F)
where
    F: Fn(bool) -> NativeAppAction + 'static,
{
    switch_row_enabled(app, parent, title, active, true, action);
}

fn switch_row_enabled<F>(
    app: &AppRef,
    parent: &gtk::Box,
    title: &str,
    active: bool,
    enabled: bool,
    action: F,
) where
    F: Fn(bool) -> NativeAppAction + 'static,
{
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_valign(gtk::Align::Center);
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&label);
    let switch = gtk::Switch::builder().active(active).build();
    switch.set_sensitive(enabled);
    {
        let app = app.clone();
        switch.connect_active_notify(move |switch| {
            dispatch(&app, action(switch.is_active()));
        });
    }
    row.append(&switch);
    parent.append(&row);
}

fn row_label(parent: &gtk::Box, title: &str, body: &str, icon_name: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_valign(gtk::Align::Start);
    if !icon_name.is_empty() {
        let icon = gtk::Image::from_icon_name(icon_name);
        icon.add_css_class("dim-label");
        row.append(&icon);
    }
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let title = gtk::Label::new(Some(title));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    text.append(&title);
    let body = gtk::Label::new(Some(body));
    body.add_css_class("caption");
    body.add_css_class("dim-label");
    body.set_xalign(0.0);
    body.set_wrap(true);
    body.set_selectable(true);
    text.append(&body);
    row.append(&text);
    parent.append(&row);
}

fn detail_row(parent: &gtk::Box, title: &str, value: &str) {
    if value.trim().is_empty() {
        return;
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_hexpand(true);
    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("caption");
    title_label.add_css_class("dim-label");
    title_label.set_xalign(0.0);
    title_label.set_width_chars(13);
    row.append(&title_label);

    let value_label = gtk::Label::new(Some(value));
    value_label.set_xalign(0.0);
    value_label.set_selectable(true);
    value_label.set_hexpand(true);
    value_label.set_wrap(true);
    value_label.set_wrap_mode(gtk::pango::WrapMode::Char);
    row.append(&value_label);
    parent.append(&row);
}

fn public_fips_address(own_npub: &str) -> String {
    let trimmed = own_npub.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}.fips")
    }
}

fn connect_remove_participant_confirmation(
    button: &gtk::Button,
    app: &AppRef,
    network_id: String,
    npub: String,
    device_name: String,
) {
    let app = app.clone();
    button.connect_clicked(move |_| {
        confirm_remove_participant(&app, network_id.clone(), npub.clone(), device_name.clone());
    });
}

fn connect_remove_network_confirmation(
    button: &gtk::Button,
    app: &AppRef,
    network_id: String,
    network_name: String,
) {
    let app = app.clone();
    button.connect_clicked(move |_| {
        confirm_remove_network(&app, network_id.clone(), network_name.clone());
    });
}

fn confirm_remove_network(app: &AppRef, network_id: String, network_name: String) {
    let dialog = adw::AlertDialog::new(
        Some(&format!("Remove {network_name}?")),
        Some("This deletes the network from this device."),
    );
    dialog.add_responses(&[("cancel", "Cancel"), ("remove", "Remove")]);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("cancel"));
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    {
        let app = app.clone();
        dialog.connect_response(Some("remove"), move |_, _| {
            dispatch(
                &app,
                NativeAppAction::RemoveNetwork {
                    network_id: network_id.clone(),
                },
            );
        });
    }
    let window = app.borrow().window.clone();
    dialog.present(Some(&window));
}

fn confirm_remove_participant(app: &AppRef, network_id: String, npub: String, device_name: String) {
    let dialog = adw::AlertDialog::new(
        Some(&format!("Remove {device_name}?")),
        Some("This removes the device from the network's roster. They keep the network locally but won't be in this roster anymore."),
    );
    dialog.add_responses(&[("cancel", "Cancel"), ("remove", "Remove")]);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("cancel"));
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    {
        let app = app.clone();
        dialog.connect_response(Some("remove"), move |_, _| {
            dispatch(
                &app,
                NativeAppAction::RemoveParticipant {
                    network_id: network_id.clone(),
                    npub: npub.clone(),
                },
            );
        });
    }
    let window = app.borrow().window.clone();
    dialog.present(Some(&window));
}

fn metric(title: &str, value: &str) -> gtk::Box {
    let metric = gtk::Box::new(gtk::Orientation::Vertical, 2);
    metric.add_css_class("nvpn-metric");
    metric.set_size_request(170, -1);

    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("caption");
    title_label.add_css_class("dim-label");
    title_label.set_xalign(0.0);
    metric.append(&title_label);

    let value_label = gtk::Label::new(Some(value));
    value_label.add_css_class("heading");
    value_label.set_xalign(0.0);
    value_label.set_selectable(true);
    value_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    metric.append(&value_label);

    metric
}

fn page_title(parent: &gtk::Box, title: &str, icon_name: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_valign(gtk::Align::Center);
    if !icon_name.is_empty() {
        let icon = gtk::Image::from_icon_name(icon_name);
        icon.add_css_class("accent");
        row.append(&icon);
    }
    let label = gtk::Label::new(Some(title));
    label.add_css_class("title-1");
    label.set_xalign(0.0);
    row.append(&label);
    parent.append(&row);
}

fn section_header(parent: &gtk::Box, title: &str, icon_name: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_valign(gtk::Align::Center);
    if !icon_name.is_empty() {
        let icon = gtk::Image::from_icon_name(icon_name);
        icon.add_css_class("dim-label");
        row.append(&icon);
    }
    let label = gtk::Label::new(Some(title));
    label.add_css_class("title-3");
    label.set_xalign(0.0);
    row.append(&label);
    parent.append(&row);
}

fn empty_row(parent: &gtk::Box, text: &str) {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("dim-label");
    label.set_xalign(0.0);
    label.set_margin_top(4);
    parent.append(&label);
}

fn card() -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("nvpn-card");
    card.set_hexpand(true);
    card.set_margin_bottom(2);
    card
}

fn nav_button(title: &str, icon_name: &str, active: bool, attention: bool) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("nvpn-nav-button");
    button.set_hexpand(true);
    if active {
        button.add_css_class("active");
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    row.set_valign(gtk::Align::Center);
    row.set_hexpand(true);
    if !icon_name.is_empty() {
        let icon = gtk::Image::from_icon_name(icon_name);
        row.append(&icon);
    }
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&label);
    if attention {
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        row.append(&spacer);
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.set_size_request(8, 8);
        dot.add_css_class("nvpn-attention-dot");
        row.append(&dot);
    }
    button.set_child(Some(&row));
    button
}

fn icon_text_button(title: &str, icon_name: &str) -> gtk::Button {
    let button = gtk::Button::new();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    row.set_valign(gtk::Align::Center);
    if !icon_name.is_empty() {
        let icon = gtk::Image::from_icon_name(icon_name);
        row.append(&icon);
    }
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    row.append(&label);
    button.set_child(Some(&row));
    button
}

fn entry(placeholder: &str, value: &str) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some(placeholder));
    entry.set_text(value);
    entry.set_hexpand(true);
    entry
}

fn badge(text: &str, style: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("nvpn-badge");
    label.add_css_class(style);
    label
}

fn clear_box(parent: &gtk::Box) {
    while let Some(child) = parent.first_child() {
        parent.remove(&child);
    }
}

fn current_content_scroll_offset(content: &gtk::Box) -> f64 {
    content
        .first_child()
        .and_then(|child| child.downcast::<gtk::ScrolledWindow>().ok())
        .map(|scroll| scroll.vadjustment().value())
        .unwrap_or(0.0)
}

fn restore_scroll_offset(scroll: &gtk::ScrolledWindow, offset: f64) {
    if !offset.is_finite() || offset <= 0.0 {
        return;
    }
    let adjustment = scroll.vadjustment();
    set_scroll_adjustment_value(&adjustment, offset);
    glib::idle_add_local_once(move || {
        set_scroll_adjustment_value(&adjustment, offset);
    });
}

fn set_scroll_adjustment_value(adjustment: &gtk::Adjustment, offset: f64) {
    let lower = adjustment.lower();
    let max = (adjustment.upper() - adjustment.page_size()).max(lower);
    adjustment.set_value(offset.clamp(lower, max));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_scroll_offsets_track_pages_independently() {
        let mut offsets = PageScrollOffsets::default();

        offsets.set(Page::Settings, 640.0);
        offsets.set(Page::Devices, 120.0);
        offsets.set(Page::ExitNodes, 260.0);

        assert_eq!(offsets.get(Page::Settings), 640.0);
        assert_eq!(offsets.get(Page::Devices), 120.0);
        assert_eq!(offsets.get(Page::ExitNodes), 260.0);
        assert_eq!(offsets.get(Page::Share), 0.0);
    }

    #[test]
    fn close_to_tray_requires_setting_tray_and_non_quit_close() {
        assert!(should_close_to_tray(true, true, false));
        assert!(!should_close_to_tray(false, true, false));
        assert!(!should_close_to_tray(true, false, false));
        assert!(!should_close_to_tray(true, true, true));
    }

    #[test]
    fn autostart_desktop_entry_launches_gui_hidden() {
        let entry = autostart_desktop_entry(std::path::Path::new("/opt/Nostr VPN/nostr-vpn"));

        assert!(entry.contains("Exec=/opt/Nostr\\ VPN/nostr-vpn --hidden\n"));
    }

    #[test]
    fn state_needs_render_ignores_revision_only_refreshes() {
        let previous = NativeAppState {
            rev: 40,
            vpn_status: "Connected".to_string(),
            ..NativeAppState::default()
        };
        let next = NativeAppState {
            rev: 41,
            vpn_status: "Connected".to_string(),
            ..previous.clone()
        };

        assert!(!state_needs_render(&previous, &next));
    }

    #[test]
    fn state_needs_render_detects_visible_changes() {
        let previous = NativeAppState {
            rev: 40,
            vpn_status: "Connected".to_string(),
            ..NativeAppState::default()
        };
        let next = NativeAppState {
            rev: 41,
            vpn_status: "Disconnected".to_string(),
            ..previous.clone()
        };

        assert!(state_needs_render(&previous, &next));
    }
}

fn active_network(state: &NativeAppState) -> Option<&NativeNetworkState> {
    state
        .networks
        .iter()
        .find(|network| network.enabled)
        .or_else(|| state.networks.first())
}

fn incoming_join_request_count(state: &NativeAppState) -> usize {
    state
        .networks
        .iter()
        .map(|network| network.inbound_join_requests.len())
        .sum()
}

fn sync_selected_device(app: &AppRef) {
    let mut model = app.borrow_mut();
    let current = model.selected_device_pubkey.clone();
    let next = {
        let state = &model.state;
        active_network(state).and_then(|network| {
            let participants = sorted_participants(network, state);
            if let Some(current) = current.as_deref() {
                if participants
                    .iter()
                    .any(|participant| participant_key(participant) == current)
                {
                    Some(current.to_string())
                } else {
                    participants.first().map(participant_key)
                }
            } else {
                participants.first().map(participant_key)
            }
        })
    };
    model.selected_device_pubkey = next;
}

fn sorted_participants(
    network: &NativeNetworkState,
    state: &NativeAppState,
) -> Vec<NativeParticipantState> {
    let mut participants = network.participants.clone();
    participants.sort_by_key(|participant| {
        (
            !is_self(participant, state),
            !participant.reachable,
            device_name(participant).to_ascii_lowercase(),
        )
    });
    participants
}

fn selected_participant(
    network: &NativeNetworkState,
    state: &NativeAppState,
    selected_key: Option<&str>,
) -> Option<NativeParticipantState> {
    let participants = sorted_participants(network, state);
    if let Some(selected_key) = selected_key {
        if let Some(selected) = participants
            .iter()
            .find(|participant| participant_key(participant) == selected_key)
        {
            return Some(selected.clone());
        }
    }
    participants.first().cloned()
}

fn participant_key(participant: &NativeParticipantState) -> String {
    if participant.pubkey_hex.trim().is_empty() {
        participant.npub.clone()
    } else {
        participant.pubkey_hex.clone()
    }
}

fn resolve_network_id(state: &NativeAppState, requested: Option<String>) -> Option<String> {
    if let Some(requested) = requested {
        if let Some(network) = state
            .networks
            .iter()
            .find(|network| network.id == requested || network.network_id == requested)
        {
            return Some(network.id.clone());
        }
        return Some(requested);
    }
    active_network(state).map(|network| network.id.clone())
}

fn display_network_name(network: &NativeNetworkState) -> String {
    if network.name.trim().is_empty() {
        "Network".to_string()
    } else {
        network.name.clone()
    }
}

fn device_name(participant: &NativeParticipantState) -> String {
    for value in [
        participant.magic_dns_name.as_str(),
        participant.alias.as_str(),
        participant.magic_dns_alias.as_str(),
    ] {
        if !value.trim().is_empty() {
            return value.to_string();
        }
    }
    short_text(&participant.npub, 18)
}

fn parse_endpoint_hints(input: &str) -> Vec<String> {
    let mut hints = input
        .split([',', '\n', '\r', '\t', ' '])
        .map(str::trim)
        .filter(|hint| !hint.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    hints.sort();
    hints.dedup();
    hints
}

fn device_magic_dns_name(participant: &NativeParticipantState, state: &NativeAppState) -> String {
    if !participant.magic_dns_name.trim().is_empty() {
        return participant.magic_dns_name.clone();
    }
    if is_self(participant, state) && !state.self_magic_dns_name.trim().is_empty() {
        return state.self_magic_dns_name.clone();
    }
    if !participant.magic_dns_alias.trim().is_empty() && !state.magic_dns_suffix.trim().is_empty() {
        return format!(
            "{}.{}",
            participant.magic_dns_alias.trim(),
            state.magic_dns_suffix.trim()
        );
    }
    String::new()
}

fn device_role_text(participant: &NativeParticipantState, state: &NativeAppState) -> String {
    let mut roles = Vec::new();
    if is_self(participant, state) {
        roles.push("This device");
    }
    if participant.is_admin {
        roles.push("Admin");
    }
    if participant.offers_exit_node {
        roles.push(exit_node_badge_text(participant, state));
    }
    if roles.is_empty() {
        "Member".to_string()
    } else {
        roles.join(", ")
    }
}

fn device_subtitle(participant: &NativeParticipantState) -> String {
    let ip = clean_ip(&participant.tunnel_ip);
    let id = short_text(&participant.npub, 18);
    if ip.is_empty() {
        id
    } else {
        format!("{id}  {ip}")
    }
}

fn device_status_text(participant: &NativeParticipantState) -> String {
    match participant.state.as_str() {
        "off" => "Off".to_string(),
        "local" | "online" | "present" => "Online".to_string(),
        "pending" => "Connecting".to_string(),
        "offline" => "Offline".to_string(),
        _ if participant.reachable => "Online".to_string(),
        _ => "Unknown".to_string(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FipsPathKind {
    Local,
    Direct,
    Routed,
    Offline,
}

fn fips_path_kind(participant: &NativeParticipantState) -> FipsPathKind {
    if participant.state == "local" {
        FipsPathKind::Local
    } else if participant.reachable && !participant.fips_transport_addr.trim().is_empty() {
        FipsPathKind::Direct
    } else if participant.reachable {
        FipsPathKind::Routed
    } else {
        FipsPathKind::Offline
    }
}

fn fips_path_text(participant: &NativeParticipantState) -> String {
    match fips_path_kind(participant) {
        FipsPathKind::Local => "This device".to_string(),
        FipsPathKind::Direct => {
            let transport = if participant.fips_transport_type.trim().is_empty() {
                String::new()
            } else {
                format!(" ({})", participant.fips_transport_type.to_uppercase())
            };
            if participant.fips_srtt_ms > 0 {
                format!(
                    "Direct connection{}, {} ms",
                    transport, participant.fips_srtt_ms
                )
            } else {
                format!("Direct connection{}", transport)
            }
        }
        FipsPathKind::Routed => {
            if participant.fips_srtt_ms > 0 {
                format!("Via mesh, {} ms", participant.fips_srtt_ms)
            } else {
                "Via mesh".to_string()
            }
        }
        FipsPathKind::Offline => "Offline".to_string(),
    }
}

fn exit_node_candidates(
    network: &NativeNetworkState,
    state: &NativeAppState,
) -> Vec<NativeParticipantState> {
    let mut candidates = network
        .participants
        .iter()
        .filter(|participant| participant.offers_exit_node && !is_self(participant, state))
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by_key(device_name);
    candidates
}

fn is_self(participant: &NativeParticipantState, state: &NativeAppState) -> bool {
    (!state.own_npub.is_empty() && participant.npub == state.own_npub)
        || (!state.own_pubkey_hex.is_empty() && participant.pubkey_hex == state.own_pubkey_hex)
}

fn is_active_exit_participant(
    participant: &NativeParticipantState,
    state: &NativeAppState,
) -> bool {
    state.exit_node_active && !state.exit_node.is_empty() && participant.npub == state.exit_node
}

fn exit_node_badge_text(
    participant: &NativeParticipantState,
    state: &NativeAppState,
) -> &'static str {
    if is_active_exit_participant(participant, state) {
        "Exit active"
    } else {
        "Exit offered"
    }
}

fn exit_node_badge_style(
    participant: &NativeParticipantState,
    state: &NativeAppState,
) -> &'static str {
    if is_active_exit_participant(participant, state) {
        "ok"
    } else {
        "warn"
    }
}

fn hero_subtitle(state: &NativeAppState) -> String {
    if state.vpn_active {
        format!(
            "{} of {} devices connected",
            state.connected_peer_count, state.expected_peer_count
        )
    } else if state.vpn_control_supported {
        "Ready to connect this device to your private network".to_string()
    } else {
        non_empty_or(&state.runtime_status_detail, "VPN control is unavailable")
    }
}

fn clean_ip(value: &str) -> String {
    value.split('/').next().unwrap_or(value).trim().to_string()
}

fn format_bytes(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit_index = 0usize;
    while value >= 1024.0 && unit_index < units.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }
    if unit_index == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", units[unit_index])
    }
}

fn load_auto_install_updates() -> bool {
    std::fs::read_to_string(update_preferences_path())
        .map(|value| value.lines().any(|line| line.trim() == "auto_install=true"))
        .unwrap_or(false)
}

fn save_auto_install_updates(enabled: bool) {
    let path = update_preferences_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let value = if enabled {
        "auto_install=true\n"
    } else {
        "auto_install=false\n"
    };
    let _ = std::fs::write(path, value);
}

fn update_poll_interval_secs() -> u32 {
    std::env::var("NVPN_UPDATE_POLL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(|seconds| seconds.min(u32::MAX as u64) as u32)
        .unwrap_or(DEFAULT_UPDATE_POLL_INTERVAL_SECS)
}

fn update_preferences_path() -> PathBuf {
    PathBuf::from(default_data_dir()).join("desktop-updates.conf")
}

fn short_text(value: &str, keep: usize) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= keep * 2 + 3 {
        return trimmed.to_string();
    }
    format!(
        "{}...{}",
        &trimmed[..keep],
        &trimmed[trimmed.len() - keep..]
    )
}

fn remaining_text(seconds: u64) -> String {
    if seconds == 0 {
        return "off".to_string();
    }
    let minutes = seconds / 60;
    if minutes == 0 {
        return format!("{seconds}s");
    }
    let secs = seconds % 60;
    if secs == 0 {
        format!("{minutes}m")
    } else {
        format!("{minutes}m{secs:02}s")
    }
}

fn non_empty_or(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn display_network_id(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= 4 || !trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return trimmed.to_string();
    }
    trimmed
        .as_bytes()
        .chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
}

fn normalize_network_id_input(value: &str) -> String {
    let trimmed = value.trim();
    let compact = trimmed
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-')
        .collect::<String>();
    if compact.is_empty() && trimmed.chars().all(|ch| ch.is_whitespace() || ch == '-') {
        return String::new();
    }
    if !compact.is_empty() && compact.chars().all(|ch| ch.is_ascii_hexdigit()) {
        compact.to_ascii_lowercase()
    } else {
        trimmed.to_string()
    }
}

fn is_valid_device_id(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.len() != 63 || !trimmed.starts_with("npub1") {
        return false;
    }
    trimmed[5..]
        .chars()
        .all(|ch| "qpzry9x8gf2tvdw0s3jn54khce6mua7l".contains(ch))
}

fn first_non_empty(values: &[&str]) -> Option<String> {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn service_update_recommended(state: &NativeAppState) -> bool {
    state.service_installed
        && !state.service_binary_version.is_empty()
        && !state.expected_service_binary_version.is_empty()
        && state.service_binary_version != state.expected_service_binary_version
}

fn system_version_label(state: &NativeAppState) -> String {
    let app = state.app_version.trim();
    let daemon = state.daemon_binary_version.trim();
    match (app.is_empty(), daemon.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!("gui v{app}"),
        (true, false) => format!("daemon v{daemon}"),
        (false, false) if app == daemon => format!("v{app}"),
        (false, false) => format!("gui v{app} · daemon v{daemon}"),
    }
}

fn copy_text(value: &str) {
    if let Some(display) = gtk::gdk::Display::default() {
        display.clipboard().set_text(value);
    }
}

fn configure_launch_on_startup(enabled: bool) -> Result<(), String> {
    let path = autostart_desktop_path().ok_or_else(|| "Autostart path unavailable".to_string())?;
    if enabled {
        let executable = std::env::current_exe()
            .map_err(|error| format!("App executable not found: {error}"))?;
        let parent = path
            .parent()
            .ok_or_else(|| "Autostart path unavailable".to_string())?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create autostart directory: {error}"))?;
        std::fs::write(&path, autostart_desktop_entry(&executable))
            .map_err(|error| format!("Could not write autostart entry: {error}"))?;
    } else if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|error| format!("Could not remove autostart entry: {error}"))?;
    }
    Ok(())
}

fn autostart_desktop_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|config| config.join("autostart").join("to.iris.nvpn.desktop"))
}

fn autostart_desktop_entry(executable: &std::path::Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Nostr VPN\nExec={} --hidden\nIcon=nostr-vpn\nTerminal=false\nCategories=Network;Security;\nX-GNOME-Autostart-enabled=true\n",
        desktop_exec_escape(&executable.to_string_lossy())
    )
}

fn desktop_exec_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(
            ch,
            ' ' | '\t'
                | '\n'
                | '"'
                | '\''
                | '\\'
                | '>'
                | '<'
                | '~'
                | '|'
                | '&'
                | ';'
                | '$'
                | '*'
                | '?'
                | '#'
                | '('
                | ')'
                | '`'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn default_data_dir() -> String {
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(data_home)
            .join("nostr-vpn")
            .to_string_lossy()
            .to_string();
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("nostr-vpn")
            .to_string_lossy()
            .to_string();
    }
    "nostr-vpn".to_string()
}

fn bootstrap_session_bus() {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
        return;
    }
    let socket = "/tmp/nostr-vpn-dbus.sock";
    if std::path::Path::new(socket).exists() {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={socket}"));
    }
}

fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

const CSS: &str = r#"
.nvpn-root,
.nvpn-content,
.nvpn-content viewport {
    background: @window_bg_color;
}

.nvpn-sidebar {
    padding: 8px;
    border-radius: 8px;
    background: alpha(@card_bg_color, 0.58);
}

.nvpn-sidebar-summary {
    padding: 8px 10px;
}

.nvpn-nav-button {
    padding: 8px 10px;
    border-radius: 8px;
}

.nvpn-nav-button.active {
    background: alpha(@accent_color, 0.14);
    color: @window_fg_color;
}

.nvpn-attention-dot {
    min-width: 8px;
    min-height: 8px;
    border-radius: 999px;
    background: @error_color;
}

.nvpn-card {
    padding: 16px;
    border-radius: 8px;
    background: @card_bg_color;
    box-shadow: inset 0 0 0 1px alpha(@window_fg_color, 0.08);
}

.nvpn-header-dot {
    min-width: 8px;
    min-height: 8px;
    border-radius: 999px;
    background: alpha(@window_fg_color, 0.4);
}

.nvpn-header-dot.ok {
    background: @success_color;
}

.nvpn-header-dot.warn {
    background: @warning_color;
}

.nvpn-header-dot.bad {
    background: @error_color;
}

.nvpn-header-status {
    font-size: 0.85em;
}

.nvpn-update-stripe {
    padding: 6px 16px;
    background: alpha(@window_fg_color, 0.05);
    box-shadow: inset 0 -1px 0 alpha(@window_fg_color, 0.08);
}

.nvpn-update-stripe label {
    font-size: 0.95em;
}

.nvpn-hero {
    padding: 20px;
}

.nvpn-status-ready,
.nvpn-status-active,
.nvpn-status-off,
.nvpn-status-blocked,
.nvpn-peer-online,
.nvpn-peer-offline {
    min-width: 14px;
    min-height: 14px;
    border-radius: 999px;
}

.nvpn-status-ready {
    min-width: 12px;
    min-height: 12px;
    background: @success_color;
}

.nvpn-status-active {
    min-width: 12px;
    min-height: 12px;
    background: @accent_color;
}

.nvpn-status-off {
    min-width: 12px;
    min-height: 12px;
    background: alpha(@window_fg_color, 0.22);
}

.nvpn-status-blocked {
    min-width: 12px;
    min-height: 12px;
    background: @error_color;
}

.nvpn-peer-online {
    background: @success_color;
}

.nvpn-peer-offline {
    background: alpha(@window_fg_color, 0.24);
}

.nvpn-device-row {
    padding: 10px;
    border-radius: 8px;
}

.nvpn-device-row.selected {
    background: alpha(@accent_color, 0.14);
}

.nvpn-route-choice {
    padding: 0;
}

.nvpn-route-choice > box {
    padding: 10px;
    border-radius: 8px;
    background: alpha(@window_fg_color, 0.04);
}

.nvpn-badge {
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 0.78em;
}

.nvpn-badge.ok {
    background: alpha(@success_color, 0.16);
    color: @success_color;
}

.nvpn-badge.warn {
    background: alpha(@warning_color, 0.16);
    color: @warning_color;
}

.nvpn-badge.bad {
    background: alpha(@error_color, 0.14);
    color: @error_color;
}

.nvpn-badge.muted {
    background: alpha(@window_fg_color, 0.08);
    color: alpha(@window_fg_color, 0.72);
}

.nvpn-metric {
    padding: 8px 10px;
    border-radius: 8px;
    background: alpha(@window_fg_color, 0.04);
}

.success,
.accent {
    color: @success_color;
}
"#;
