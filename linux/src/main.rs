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
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use nostr_vpn_app_core::{
    FfiApp, NativeAppAction, NativeAppState, NativeNetworkState, NativeParticipantState,
    NativeRelayState, SettingsPatch,
};

const APP_ID: &str = "to.iris.nvpn";

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
    Routing,
    Settings,
}

#[derive(Clone, Default)]
struct Drafts {
    invite: String,
    participant_npub: String,
    participant_alias: String,
    relay: String,
    network_name: String,
    mesh_id: String,
    new_network_name: String,
    node_name: String,
    endpoint: String,
    tunnel_ip: String,
    listen_port: String,
    magic_dns_suffix: String,
    advertised_routes: String,
    exit_search: String,
}

impl Drafts {
    fn sync_from_state(&mut self, state: &NativeAppState) {
        self.node_name = state.node_name.clone();
        self.endpoint = state.endpoint.clone();
        self.tunnel_ip = state.tunnel_ip.clone();
        self.listen_port = state.listen_port.to_string();
        self.magic_dns_suffix = state.magic_dns_suffix.clone();
        self.advertised_routes = state.advertised_routes.join(", ");
        if let Some(network) = active_network(state) {
            self.network_name = display_network_name(network);
            self.mesh_id = network.network_id.clone();
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
    sidebar: gtk::Box,
    content: gtk::Box,
    drafts: Drafts,
    notice: String,
    tray: tray::TrayRuntime,
    update: updater::UpdateState,
    update_sender: Sender<updater::UpdateEvent>,
    update_receiver: Receiver<updater::UpdateEvent>,
    allow_close: bool,
    service_settling: bool,
}

impl AppModel {
    fn new(window: adw::ApplicationWindow, sidebar: gtk::Box, content: gtk::Box) -> Self {
        let core = FfiApp::new(default_data_dir(), env!("CARGO_PKG_VERSION").to_string());
        let state = core.state();
        let mut drafts = Drafts::default();
        drafts.sync_from_state(&state);
        let tray = tray::TrayRuntime::start(&state);
        let (update_sender, update_receiver) = mpsc::channel();
        Self {
            core,
            state,
            window,
            page: Page::Devices,
            sidebar,
            content,
            drafts,
            notice: String::new(),
            tray,
            update: updater::UpdateState::default(),
            update_sender,
            update_receiver,
            allow_close: false,
            service_settling: false,
        }
    }
}

fn main() -> glib::ExitCode {
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
                if arg == "--autostart" || arg == "--hidden" {
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

    let refresh_button = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh_button.set_tooltip_text(Some("Refresh"));
    header.pack_end(&refresh_button);

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

    let shell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    shell.set_hexpand(true);
    shell.set_vexpand(true);
    shell.append(&sidebar);
    shell.append(&content);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&shell));
    window.set_content(Some(&toolbar));

    let model = Rc::new(RefCell::new(AppModel::new(
        window.clone(),
        sidebar.clone(),
        content.clone(),
    )));
    *runtime.model.borrow_mut() = Some(model.clone());

    {
        let model = model.clone();
        refresh_button.connect_clicked(move |_| refresh_now(&model));
    }
    {
        let model = model.clone();
        window.connect_close_request(move |window| {
            let model = model.borrow();
            if model.state.close_to_tray_on_close && !model.allow_close {
                window.set_visible(false);
                glib::Propagation::Stop
            } else {
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

    check_updates(&model, false);

    if present {
        window.present();
    }
    drain_pending_urls(runtime);
}

fn refresh_now(app: &AppRef) {
    let core = app.borrow().core.clone();
    let state = core.refresh();
    set_state(app, state);
    render(app);
}

fn dispatch(app: &AppRef, action: NativeAppAction) {
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
                return;
            }
        }
    }
    let core = app.borrow().core.clone();
    let state = core.dispatch(action);
    set_state(app, state);
    render(app);
    if settle_service {
        start_service_settlement_polling(app);
    }
}

fn set_state(app: &AppRef, state: NativeAppState) {
    let mut model = app.borrow_mut();
    model.tray.update(&state);
    model.state = state;
}

fn drain_tray_commands(app: &AppRef) {
    let commands = app.borrow_mut().tray.drain();
    for command in commands {
        match command {
            tray::TrayCommand::ShowWindow => show_window(app),
            tray::TrayCommand::ToggleSession => {
                let active = app.borrow().state.session_active;
                dispatch(
                    app,
                    if active {
                        NativeAppAction::DisconnectSession
                    } else {
                        NativeAppAction::ConnectSession
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
            tray::TrayCommand::CopyThisDevice => {
                let value = tray::this_device_copy_value(&app.borrow().state);
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
            tray::TrayCommand::Refresh => refresh_now(app),
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
        model.update.checking = true;
        if manual {
            model.update.status = "Checking for updates".to_string();
        }
        (model.state.app_version.clone(), model.update_sender.clone())
    };
    render(app);
    updater::check(current_version, manual, sender);
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

    render(app);
}

fn show_window(app: &AppRef) {
    let window = app.borrow().window.clone();
    window.present();
}

fn quit_app(app: &AppRef) {
    let window = {
        let mut model = app.borrow_mut();
        model.allow_close = true;
        model.window.clone()
    };
    if let Some(application) = window.application() {
        application.quit();
    }
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
        model.page = Page::Share;
        model.drafts.invite.clear();
        model.notice.clear();
    }
    dispatch(app, NativeAppAction::ImportNetworkInvite { invite });
}

fn set_notice(app: &AppRef, notice: impl Into<String>) {
    app.borrow_mut().notice = notice.into();
    render(app);
}

fn set_page(app: &AppRef, page: Page) {
    app.borrow_mut().page = page;
    render(app);
}

fn render(app: &AppRef) {
    let (sidebar, content, state, page) = {
        let model = app.borrow();
        (
            model.sidebar.clone(),
            model.content.clone(),
            model.state.clone(),
            model.page,
        )
    };

    clear_box(&sidebar);
    clear_box(&content);
    build_sidebar(app, &sidebar, &state, page);

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
        Page::Routing => build_routing_page(app, &page_box, &state),
        Page::Settings => build_settings_page(app, &page_box, &state),
    }

    scroll.set_child(Some(&page_box));
    content.append(&scroll);
}

fn build_sidebar(app: &AppRef, sidebar: &gtk::Box, state: &NativeAppState, page: Page) {
    let brand = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    brand.set_margin_bottom(6);
    let label = gtk::Label::new(Some("Nostr VPN"));
    label.add_css_class("heading");
    label.set_xalign(0.0);
    brand.append(&label);
    sidebar.append(&brand);

    for (target, title, icon) in [
        (Page::Devices, "Devices", ""),
        (Page::Share, "Share", "emblem-shared-symbolic"),
        (Page::Routing, "Routing", ""),
        (Page::Settings, "Settings", "emblem-system-symbolic"),
    ] {
        let button = nav_button(title, icon, page == target);
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

    let status = gtk::Label::new(Some(&state.session_status));
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
        let card = card();
        row_label(&card, "No network", "Create a network in Settings.", "");
        page.append(&card);
        return;
    };

    let devices = card();
    section_header(&devices, "Devices", "");

    let mut participants = network.participants.clone();
    participants.sort_by_key(|participant| {
        (
            !is_self(participant, state),
            !participant.reachable,
            device_name(participant),
        )
    });

    if participants.is_empty() {
        empty_row(&devices, "No devices yet");
    } else {
        for participant in participants {
            device_row(app, &devices, &network, &participant, state);
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

            if !is_self(participant, state) {
                let remove = gtk::Button::from_icon_name("edit-delete-symbolic");
                remove.set_tooltip_text(Some("Remove device"));
                remove.add_css_class("destructive-action");
                {
                    let app = app.clone();
                    let network_id = network.id.clone();
                    let npub = participant.npub.clone();
                    remove.connect_clicked(move |_| {
                        dispatch(
                            &app,
                            NativeAppAction::RemoveParticipant {
                                network_id: network_id.clone(),
                                npub: npub.clone(),
                            },
                        );
                    });
                }
                participant_row.append(&remove);
            }

            body.append(&participant_row);
        }

        expander.set_child(Some(&body));
        devices.append(&expander);
    }

    page.append(&devices);

    if !network.inbound_join_requests.is_empty() {
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
        page.append(&requests);
    }
}

fn build_network_hero(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    let hero = card();
    hero.add_css_class("nvpn-hero");

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    top.set_valign(gtk::Align::Center);

    let status = gtk::Box::new(gtk::Orientation::Vertical, 0);
    status.add_css_class(if state.mesh_ready {
        "nvpn-status-ready"
    } else if state.session_active {
        "nvpn-status-active"
    } else {
        "nvpn-status-off"
    });
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
        if state.session_active {
            "VPN on"
        } else {
            "VPN off"
        },
        if state.session_active { "ok" } else { "muted" },
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
        if state.relay_connected {
            "Relays"
        } else {
            "Relays down"
        },
        if state.relay_connected { "ok" } else { "warn" },
    ));
    badges.append(&badge(
        if state.mesh_ready {
            "Mesh ready"
        } else {
            "Mesh pending"
        },
        if state.mesh_ready { "ok" } else { "muted" },
    ));
    if service_repair_recommended(state) {
        badges.append(&badge("Repair", "warn"));
    }
    text.append(&badges);
    top.append(&text);

    let connect = icon_text_button(
        if state.session_active {
            "Connected"
        } else {
            "Connect"
        },
        if state.session_active {
            "media-playback-stop-symbolic"
        } else {
            "media-playback-start-symbolic"
        },
    );
    connect.add_css_class("suggested-action");
    connect.set_sensitive(state.vpn_session_control_supported);
    {
        let app = app.clone();
        let active = state.session_active;
        connect.connect_clicked(move |_| {
            dispatch(
                &app,
                if active {
                    NativeAppAction::DisconnectSession
                } else {
                    NativeAppAction::ConnectSession
                },
            );
        });
    }
    top.append(&connect);
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
    if !state.exit_node.is_empty() {
        identity.append(&badge("Exit selected", "warn"));
    }
    hero.append(&identity);

    page.append(&hero);
}

fn build_share_page(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    page_title(page, "Share", "emblem-shared-symbolic");

    let Some(network) = active_network(state).cloned() else {
        return;
    };

    let invite = card();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    row.set_valign(gtk::Align::Start);
    row.append(&qr::build(&state.active_network_invite, 150));

    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.set_hexpand(true);
    section_header(&column, "Invite Devices", "emblem-shared-symbolic");

    let invite_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let code = gtk::Entry::new();
    code.set_text(&state.active_network_invite);
    code.set_editable(false);
    code.set_hexpand(true);
    code.set_placeholder_text(Some("No invite"));
    invite_row.append(&code);
    let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy.set_tooltip_text(Some("Copy invite"));
    copy.set_sensitive(!state.active_network_invite.is_empty());
    {
        let invite = state.active_network_invite.clone();
        copy.connect_clicked(move |_| copy_text(&invite));
    }
    invite_row.append(&copy);
    column.append(&invite_row);

    let import_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let invite_entry = entry("Paste invite", &app.borrow().drafts.invite);
    {
        let app = app.clone();
        invite_entry.connect_changed(move |entry| {
            app.borrow_mut().drafts.invite = entry.text().to_string();
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
    let image = gtk::Button::from_icon_name("insert-image-symbolic");
    image.set_tooltip_text(Some("Import QR image"));
    {
        let app = app.clone();
        image.connect_clicked(move |button| choose_invite_qr_image(&app, button));
    }
    let camera = gtk::Button::from_icon_name("camera-photo-symbolic");
    camera.set_tooltip_text(Some("Scan QR"));
    {
        let app = app.clone();
        camera.connect_clicked(move |button| scan_invite_qr(&app, button));
    }
    import_row.append(&invite_entry);
    import_row.append(&import);
    import_row.append(&camera);
    import_row.append(&image);
    column.append(&import_row);

    let notice = app.borrow().notice.clone();
    if !notice.trim().is_empty() {
        row_label(&column, "Import", &notice, "dialog-warning-symbolic");
    }

    if network.outbound_join_request.is_some() {
        column.append(&badge("Join requested", "warn"));
    } else if !network.invite_inviter_npub.is_empty() {
        let request = icon_text_button("Request Access", "contact-new-symbolic");
        {
            let app = app.clone();
            let network_id = network.id.clone();
            request.connect_clicked(move |_| {
                dispatch(
                    &app,
                    NativeAppAction::RequestNetworkJoin {
                        network_id: network_id.clone(),
                    },
                );
            });
        }
        column.append(&request);
    }

    row.append(&column);
    invite.append(&row);
    page.append(&invite);

    let nearby = card();
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_valign(gtk::Align::Center);
    section_header(&header, "Nearby Devices", "");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    header.append(&spacer);
    let lan_label = if state.lan_pairing_active {
        format!("{}s", state.lan_pairing_remaining_secs)
    } else {
        "Pair Nearby".to_string()
    };
    let lan = icon_text_button(
        &lan_label,
        if state.lan_pairing_active {
            "media-playback-stop-symbolic"
        } else {
            "list-add-symbolic"
        },
    );
    {
        let app = app.clone();
        let active = state.lan_pairing_active;
        lan.connect_clicked(move |_| {
            dispatch(
                &app,
                if active {
                    NativeAppAction::StopLanPairing
                } else {
                    NativeAppAction::StartLanPairing
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

fn build_routing_page(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    page_title(page, "Routing", "");

    let Some(network) = active_network(state).cloned() else {
        return;
    };

    let exit = card();
    section_header(&exit, "Exit Node", "");

    let search = entry("Search devices", &app.borrow().drafts.exit_search);
    {
        let app = app.clone();
        search.connect_changed(move |entry| {
            app.borrow_mut().drafts.exit_search = entry.text().to_string();
        });
    }
    exit.append(&search);

    route_choice(
        app,
        &exit,
        "Direct",
        "Use normal internet routing",
        state.exit_node.is_empty(),
        true,
        None,
    );

    let query = app.borrow().drafts.exit_search.to_ascii_lowercase();
    for participant in exit_node_candidates(&network)
        .into_iter()
        .filter(|participant| {
            query.is_empty()
                || device_name(participant)
                    .to_ascii_lowercase()
                    .contains(&query)
                || participant.npub.to_ascii_lowercase().contains(&query)
        })
    {
        route_choice(
            app,
            &exit,
            &device_name(&participant),
            if participant.offers_exit_node {
                non_empty_or(&participant.status_text, "Exit node")
            } else {
                "Exit not offered".to_string()
            }
            .as_str(),
            state.exit_node == participant.npub,
            participant.offers_exit_node,
            Some(participant.npub.clone()),
        );
    }
    page.append(&exit);

    let subnet = card();
    section_header(&subnet, "Subnet Routes", "");
    switch_row(
        app,
        &subnet,
        "Offer this device as an exit node",
        state.advertise_exit_node,
        |enabled| NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                advertise_exit_node: Some(enabled),
                ..SettingsPatch::default()
            },
        },
    );

    let routes = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = entry("Advertised routes", &app.borrow().drafts.advertised_routes);
    {
        let app = app.clone();
        entry.connect_changed(move |entry| {
            app.borrow_mut().drafts.advertised_routes = entry.text().to_string();
        });
    }
    let save = gtk::Button::with_label("Save");
    {
        let app = app.clone();
        save.connect_clicked(move |_| {
            let advertised_routes = app.borrow().drafts.advertised_routes.clone();
            dispatch(
                &app,
                NativeAppAction::UpdateSettings {
                    patch: SettingsPatch {
                        advertised_routes: Some(advertised_routes),
                        ..SettingsPatch::default()
                    },
                },
            );
        });
    }
    routes.append(&entry);
    routes.append(&save);
    subnet.append(&routes);
    page.append(&subnet);
}

fn route_choice(
    app: &AppRef,
    parent: &gtk::Box,
    title: &str,
    subtitle: &str,
    selected: bool,
    enabled: bool,
    exit_node: Option<String>,
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
        button.connect_clicked(move |_| {
            dispatch(
                &app,
                NativeAppAction::UpdateSettings {
                    patch: SettingsPatch {
                        exit_node: Some(exit_node.clone().unwrap_or_default()),
                        ..SettingsPatch::default()
                    },
                },
            );
        });
    }
    parent.append(&button);
}

fn build_settings_page(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    page_title(page, "Settings", "emblem-system-symbolic");

    let device = card();
    section_header(&device, "This Device", "");
    setting_entry(app, &device, "Name", "node_name");
    setting_entry(app, &device, "Tunnel IP", "tunnel_ip");
    setting_entry(app, &device, "Endpoint", "endpoint");
    setting_entry(app, &device, "Listen Port", "listen_port");
    setting_entry(app, &device, "DNS Suffix", "magic_dns_suffix");

    let save = icon_text_button("Save", "");
    save.add_css_class("suggested-action");
    save.set_halign(gtk::Align::Start);
    {
        let app = app.clone();
        save.connect_clicked(move |_| save_device_settings(&app));
    }
    device.append(&save);
    page.append(&device);

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
    network_header.append(&new_name);
    let add = gtk::Button::from_icon_name("list-add-symbolic");
    add.set_tooltip_text(Some("Add network"));
    {
        let app = app.clone();
        add.connect_clicked(move |_| {
            let name = app.borrow().drafts.new_network_name.trim().to_string();
            if name.is_empty() {
                return;
            }
            app.borrow_mut().drafts.new_network_name.clear();
            dispatch(&app, NativeAppAction::AddNetwork { name });
        });
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
        let save = gtk::Button::with_label("Save");
        {
            let app = app.clone();
            let network_id = active.id.clone();
            save.connect_clicked(move |_| {
                let name = app.borrow().drafts.network_name.trim().to_string();
                if !name.is_empty() {
                    dispatch(
                        &app,
                        NativeAppAction::RenameNetwork {
                            network_id: network_id.clone(),
                            name,
                        },
                    );
                }
            });
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
            save.connect_clicked(move |_| {
                let mesh_id = app.borrow().drafts.mesh_id.trim().to_string();
                if !mesh_id.is_empty() {
                    dispatch(
                        &app,
                        NativeAppAction::SetNetworkMeshId {
                            network_id: network_id.clone(),
                            mesh_id,
                        },
                    );
                }
            });
        }
        mesh.append(&save);
        network.append(&mesh);

        switch_row_enabled(
            app,
            &network,
            "Join requests",
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
            saved_network_row(app, &saved_body, &saved_network, state.networks.len() > 1);
        }
    }
    saved.set_child(Some(&saved_body));
    network.append(&saved);
    page.append(&network);

    let system = card();
    section_header(&system, "System", "");
    switch_row(app, &system, "Autoconnect", state.autoconnect, |enabled| {
        NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                autoconnect: Some(enabled),
                ..SettingsPatch::default()
            },
        }
    });
    if state.startup_settings_supported {
        switch_row(
            app,
            &system,
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
        switch_row(
            app,
            &system,
            "Tray on close",
            state.close_to_tray_on_close,
            |enabled| NativeAppAction::UpdateSettings {
                patch: SettingsPatch {
                    close_to_tray_on_close: Some(enabled),
                    ..SettingsPatch::default()
                },
            },
        );
    }

    let (service_settling, tray_error, update) = {
        let model = app.borrow();
        (
            model.service_settling,
            model.tray.last_error(),
            model.update.clone(),
        )
    };

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
    if service_repair_recommended(state) {
        status_row.append(&badge("Repair available", "warn"));
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
        cli.connect_clicked(move |_| dispatch(&app, NativeAppAction::InstallCli));
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
        uninstall_cli.connect_clicked(move |_| dispatch(&app, NativeAppAction::UninstallCli));
    }
    cli_row.append(&uninstall_cli);
    system.append(&cli_row);

    if state.service_supported {
        let service_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        service_row.set_halign(gtk::Align::Start);
        let service = icon_text_button(
            if service_repair_recommended(state) {
                "Repair Service"
            } else if state.service_installed {
                "Reinstall Service"
            } else {
                "Install Service"
            },
            "system-run-symbolic",
        );
        {
            let app = app.clone();
            service.connect_clicked(move |_| dispatch(&app, NativeAppAction::InstallSystemService));
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
            uninstall
                .connect_clicked(move |_| dispatch(&app, NativeAppAction::UninstallSystemService));
        }
        service_row.append(&uninstall);
        system.append(&service_row);
    }
    page.append(&system);

    let advanced = gtk::Expander::new(Some("Advanced"));
    let advanced_body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    advanced_body.set_margin_top(10);
    build_relays(app, &advanced_body, state);
    build_diagnostics(&advanced_body, state);
    advanced.set_child(Some(&advanced_body));
    page.append(&advanced);
}

fn build_relays(app: &AppRef, parent: &gtk::Box, state: &NativeAppState) {
    let relays = card();
    section_header(&relays, "Discovery Relays", "");

    let summary = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    summary.append(&badge(&format!("{} up", state.relay_summary.up), "ok"));
    summary.append(&badge(&format!("{} down", state.relay_summary.down), "bad"));
    summary.append(&badge(
        &format!("{} checking", state.relay_summary.checking),
        "muted",
    ));
    summary.append(&badge(
        &format!("{} unknown", state.relay_summary.unknown),
        "muted",
    ));
    relays.append(&summary);

    let add_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let input = entry("wss://relay.example", &app.borrow().drafts.relay);
    {
        let app = app.clone();
        input.connect_changed(move |entry| {
            app.borrow_mut().drafts.relay = entry.text().to_string();
        });
    }
    let add = icon_text_button("Add", "list-add-symbolic");
    {
        let app = app.clone();
        add.connect_clicked(move |_| {
            let relay = app.borrow().drafts.relay.trim().to_string();
            if relay.is_empty() {
                return;
            }
            app.borrow_mut().drafts.relay.clear();
            dispatch(&app, NativeAppAction::AddRelay { relay });
        });
    }
    add_row.append(&input);
    add_row.append(&add);
    relays.append(&add_row);

    if state.relays.is_empty() {
        empty_row(&relays, "No relays configured");
    } else {
        for relay in &state.relays {
            relay_row(app, &relays, relay, state.relays.len() > 1);
        }
    }
    parent.append(&relays);
}

fn build_diagnostics(parent: &gtk::Box, state: &NativeAppState) {
    let diagnostics = card();
    section_header(&diagnostics, "Diagnostics", "dialog-information-symbolic");

    let metrics = gtk::FlowBox::new();
    metrics.set_selection_mode(gtk::SelectionMode::None);
    metrics.set_column_spacing(10);
    metrics.set_row_spacing(10);
    metrics.set_max_children_per_line(3);
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
            "magic_dns_suffix" => model.drafts.magic_dns_suffix.clone(),
            _ => String::new(),
        }
    };
    let input = entry(title, &current);
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
                "magic_dns_suffix" => model.drafts.magic_dns_suffix = value,
                _ => {}
            }
        });
    }
    row.append(&input);
    parent.append(&row);
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
                magic_dns_suffix: Some(drafts.magic_dns_suffix),
                ..SettingsPatch::default()
            },
        },
    );
}

fn saved_network_row(
    app: &AppRef,
    parent: &gtk::Box,
    network: &NativeNetworkState,
    can_remove: bool,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("nvpn-route-choice");
    row.set_valign(gtk::Align::Center);

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
    row.append(&text);

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
    row.append(&activate);

    let remove = gtk::Button::from_icon_name("edit-delete-symbolic");
    remove.set_tooltip_text(Some("Remove network"));
    remove.set_sensitive(can_remove);
    remove.add_css_class("destructive-action");
    {
        let app = app.clone();
        let network_id = network.id.clone();
        remove.connect_clicked(move |_| {
            dispatch(
                &app,
                NativeAppAction::RemoveNetwork {
                    network_id: network_id.clone(),
                },
            );
        });
    }
    row.append(&remove);
    parent.append(&row);
}

fn device_row(
    app: &AppRef,
    parent: &gtk::Box,
    network: &NativeNetworkState,
    participant: &NativeParticipantState,
    state: &NativeAppState,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("nvpn-device-row");
    row.set_valign(gtk::Align::Center);

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
        name_row.append(&badge("Exit", "warn"));
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
        {
            let app = app.clone();
            let network_id = network.id.clone();
            let npub = participant.npub.clone();
            remove.connect_clicked(move |_| {
                dispatch(
                    &app,
                    NativeAppAction::RemoveParticipant {
                        network_id: network_id.clone(),
                        npub: npub.clone(),
                    },
                );
            });
        }
        row.append(&remove);
    }

    parent.append(&row);
}

fn relay_row(app: &AppRef, parent: &gtk::Box, relay: &NativeRelayState, can_remove: bool) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_valign(gtk::Align::Center);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let title = gtk::Label::new(Some(&relay.url));
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    title.add_css_class("heading");
    text.append(&title);
    let subtitle = gtk::Label::new(Some(&non_empty_or(&relay.status_text, &relay.state)));
    subtitle.add_css_class("caption");
    subtitle.add_css_class("dim-label");
    subtitle.set_xalign(0.0);
    text.append(&subtitle);
    row.append(&text);
    row.append(&badge(
        &relay.state,
        if relay.state == "up" { "ok" } else { "muted" },
    ));

    let remove = gtk::Button::from_icon_name("edit-delete-symbolic");
    remove.set_tooltip_text(Some("Remove relay"));
    remove.set_sensitive(can_remove);
    {
        let app = app.clone();
        let relay = relay.url.clone();
        remove.connect_clicked(move |_| {
            dispatch(
                &app,
                NativeAppAction::RemoveRelay {
                    relay: relay.clone(),
                },
            );
        });
    }
    row.append(&remove);
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
    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("caption");
    title_label.add_css_class("dim-label");
    title_label.set_xalign(0.0);
    title_label.set_width_chars(13);
    row.append(&title_label);

    let value_label = gtk::Label::new(Some(value));
    value_label.set_xalign(0.0);
    value_label.set_selectable(true);
    value_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    row.append(&value_label);
    parent.append(&row);
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

fn nav_button(title: &str, icon_name: &str, active: bool) -> gtk::Button {
    let button = icon_text_button(title, icon_name);
    button.add_css_class("flat");
    button.add_css_class("nvpn-nav-button");
    if active {
        button.add_css_class("active");
    }
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

fn active_network(state: &NativeAppState) -> Option<&NativeNetworkState> {
    state
        .networks
        .iter()
        .find(|network| network.enabled)
        .or_else(|| state.networks.first())
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
        participant.alias.as_str(),
        participant.magic_dns_alias.as_str(),
        participant.magic_dns_name.as_str(),
    ] {
        if !value.trim().is_empty() {
            return value.to_string();
        }
    }
    short_text(&participant.npub, 18)
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
    for value in [
        participant.presence_state.as_str(),
        participant.state.as_str(),
        participant.status_text.as_str(),
    ] {
        if !value.trim().is_empty() {
            return value.to_string();
        }
    }
    if participant.reachable {
        "Online".to_string()
    } else {
        "Offline".to_string()
    }
}

fn exit_node_candidates(network: &NativeNetworkState) -> Vec<NativeParticipantState> {
    let mut candidates = network.participants.clone();
    candidates.sort_by_key(device_name);
    candidates
}

fn is_self(participant: &NativeParticipantState, state: &NativeAppState) -> bool {
    (!state.own_npub.is_empty() && participant.npub == state.own_npub)
        || (!state.own_pubkey_hex.is_empty() && participant.pubkey_hex == state.own_pubkey_hex)
}

fn hero_subtitle(state: &NativeAppState) -> String {
    if state.session_active {
        format!(
            "{} of {} devices connected",
            state.connected_peer_count, state.expected_peer_count
        )
    } else if state.vpn_session_control_supported {
        "Ready to connect this device to your private network".to_string()
    } else {
        non_empty_or(
            &state.runtime_status_detail,
            "Session control is unavailable",
        )
    }
}

fn clean_ip(value: &str) -> String {
    value.split('/').next().unwrap_or(value).trim().to_string()
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

fn non_empty_or(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn first_non_empty(values: &[&str]) -> Option<String> {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn service_repair_recommended(state: &NativeAppState) -> bool {
    state.service_installed
        && !state.service_binary_version.is_empty()
        && !state.app_version.is_empty()
        && state.service_binary_version != state.app_version
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
        "[Desktop Entry]\nType=Application\nName=Nostr VPN\nExec={} --autostart\nIcon=nostr-vpn\nTerminal=false\nCategories=Network;Security;\nX-GNOME-Autostart-enabled=true\n",
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
    background: alpha(#3584e4, 0.14);
    color: @window_fg_color;
}

.nvpn-card {
    padding: 16px;
    border-radius: 8px;
    background: @card_bg_color;
    box-shadow: inset 0 0 0 1px alpha(@window_fg_color, 0.08);
}

.nvpn-hero {
    padding: 20px;
}

.nvpn-status-ready,
.nvpn-status-active,
.nvpn-status-off,
.nvpn-peer-online,
.nvpn-peer-offline {
    min-width: 14px;
    min-height: 14px;
    border-radius: 999px;
}

.nvpn-status-ready {
    min-width: 48px;
    min-height: 48px;
    background: #16a34a;
}

.nvpn-status-active {
    min-width: 48px;
    min-height: 48px;
    background: #d97706;
}

.nvpn-status-off {
    min-width: 48px;
    min-height: 48px;
    background: alpha(@window_fg_color, 0.22);
}

.nvpn-peer-online {
    background: #16a34a;
}

.nvpn-peer-offline {
    background: alpha(@window_fg_color, 0.24);
}

.nvpn-device-row {
    padding: 10px 0;
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
    background: alpha(#16a34a, 0.16);
    color: #15803d;
}

.nvpn-badge.warn {
    background: alpha(#d97706, 0.16);
    color: #b45309;
}

.nvpn-badge.bad {
    background: alpha(#dc2626, 0.14);
    color: #b91c1c;
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
    color: #15803d;
}
"#;
