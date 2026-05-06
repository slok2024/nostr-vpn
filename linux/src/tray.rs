use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};

use glib::variant::ToVariant;
use image::GenericImageView;
use nostr_vpn_app_core::{NativeAppState, NativeNetworkState, NativeParticipantState};

const SNI_BUS_NAME_PREFIX: &str = "org.kde.StatusNotifierItem";
const SNI_OBJECT_PATH: &str = "/StatusNotifierItem";
const MENU_OBJECT_PATH: &str = "/StatusNotifierItem/Menu";
const SNI_INTERFACE: &str = "org.kde.StatusNotifierItem";
const MENU_INTERFACE: &str = "com.canonical.dbusmenu";

const SNI_XML: &str = r#"
<node>
  <interface name='org.kde.StatusNotifierItem'>
    <property name='Category' type='s' access='read'/>
    <property name='Id' type='s' access='read'/>
    <property name='Title' type='s' access='read'/>
    <property name='Status' type='s' access='read'/>
    <property name='WindowId' type='u' access='read'/>
    <property name='IconName' type='s' access='read'/>
    <property name='IconPixmap' type='a(iiay)' access='read'/>
    <property name='OverlayIconName' type='s' access='read'/>
    <property name='OverlayIconPixmap' type='a(iiay)' access='read'/>
    <property name='AttentionIconName' type='s' access='read'/>
    <property name='AttentionIconPixmap' type='a(iiay)' access='read'/>
    <property name='AttentionMovieName' type='s' access='read'/>
    <property name='ToolTip' type='(sa(iiay)ss)' access='read'/>
    <property name='ItemIsMenu' type='b' access='read'/>
    <property name='Menu' type='o' access='read'/>
    <method name='ContextMenu'>
      <arg type='i' name='x' direction='in'/>
      <arg type='i' name='y' direction='in'/>
    </method>
    <method name='Activate'>
      <arg type='i' name='x' direction='in'/>
      <arg type='i' name='y' direction='in'/>
    </method>
    <method name='SecondaryActivate'>
      <arg type='i' name='x' direction='in'/>
      <arg type='i' name='y' direction='in'/>
    </method>
    <method name='Scroll'>
      <arg type='i' name='delta' direction='in'/>
      <arg type='s' name='orientation' direction='in'/>
    </method>
    <signal name='NewTitle'/>
    <signal name='NewIcon'/>
    <signal name='NewAttentionIcon'/>
    <signal name='NewOverlayIcon'/>
    <signal name='NewToolTip'/>
    <signal name='NewStatus'>
      <arg type='s' name='status'/>
    </signal>
  </interface>
</node>
"#;

const MENU_XML: &str = r#"
<node>
  <interface name='com.canonical.dbusmenu'>
    <method name='GetLayout'>
      <arg type='i' name='parentId' direction='in'/>
      <arg type='i' name='recursionDepth' direction='in'/>
      <arg type='as' name='propertyNames' direction='in'/>
      <arg type='u' name='revision' direction='out'/>
      <arg type='(ia{sv}av)' name='layout' direction='out'/>
    </method>
    <method name='GetGroupProperties'>
      <arg type='ai' name='ids' direction='in'/>
      <arg type='as' name='propertyNames' direction='in'/>
      <arg type='a(ia{sv})' name='properties' direction='out'/>
    </method>
    <method name='GetProperty'>
      <arg type='i' name='id' direction='in'/>
      <arg type='s' name='name' direction='in'/>
      <arg type='v' name='value' direction='out'/>
    </method>
    <method name='Event'>
      <arg type='i' name='id' direction='in'/>
      <arg type='s' name='eventId' direction='in'/>
      <arg type='v' name='data' direction='in'/>
      <arg type='u' name='timestamp' direction='in'/>
    </method>
    <method name='AboutToShow'>
      <arg type='i' name='id' direction='in'/>
      <arg type='b' name='needUpdate' direction='out'/>
    </method>
    <signal name='ItemsPropertiesUpdated'>
      <arg type='a(ia{sv})' name='updatedProps'/>
      <arg type='a(ias)' name='removedProps'/>
    </signal>
    <signal name='LayoutUpdated'>
      <arg type='u' name='revision'/>
      <arg type='i' name='parent'/>
    </signal>
  </interface>
</node>
"#;

#[derive(Debug, Clone)]
pub enum TrayCommand {
    ShowWindow,
    ToggleSession,
    ToggleExitOffer,
    CopyThisDevice,
    CopyPeer(String),
    SetExitNode(String),
    Refresh,
    Quit,
}

pub struct TrayRuntime {
    state: Rc<RefCell<NativeAppState>>,
    receiver: Receiver<TrayCommand>,
    connection: Option<gio::DBusConnection>,
    sni_registration: Option<gio::RegistrationId>,
    menu_registration: Option<gio::RegistrationId>,
    owner_id: Option<gio::OwnerId>,
    menu_revision: Rc<RefCell<u32>>,
    last_error: Rc<RefCell<Option<String>>>,
}

impl TrayRuntime {
    pub fn start(state: &NativeAppState) -> Self {
        let (sender, receiver) = mpsc::channel();
        let state = Rc::new(RefCell::new(state.clone()));
        let menu_revision = Rc::new(RefCell::new(1));
        let last_error = Rc::new(RefCell::new(None));
        let icon = tray_icon();

        let mut runtime = Self {
            state: state.clone(),
            receiver,
            connection: None,
            sni_registration: None,
            menu_registration: None,
            owner_id: None,
            menu_revision: menu_revision.clone(),
            last_error: last_error.clone(),
        };

        let connection = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
            Ok(connection) => connection,
            Err(error) => {
                *last_error.borrow_mut() = Some(format!("session bus unavailable: {error}"));
                return runtime;
            }
        };

        let sni_info = match gio::DBusNodeInfo::for_xml(SNI_XML)
            .ok()
            .and_then(|node| node.lookup_interface(SNI_INTERFACE))
        {
            Some(info) => info,
            None => {
                *last_error.borrow_mut() =
                    Some("status notifier interface unavailable".to_string());
                return runtime;
            }
        };
        let menu_info = match gio::DBusNodeInfo::for_xml(MENU_XML)
            .ok()
            .and_then(|node| node.lookup_interface(MENU_INTERFACE))
        {
            Some(info) => info,
            None => {
                *last_error.borrow_mut() = Some("tray menu interface unavailable".to_string());
                return runtime;
            }
        };

        let sni_registration = register_sni_object(
            &connection,
            &sni_info,
            state.clone(),
            icon.clone(),
            sender.clone(),
        )
        .map_err(|error| {
            *last_error.borrow_mut() = Some(format!("tray registration failed: {error}"));
        })
        .ok();

        let menu_registration = register_menu_object(
            &connection,
            &menu_info,
            state.clone(),
            menu_revision.clone(),
            sender,
        )
        .map_err(|error| {
            *last_error.borrow_mut() = Some(format!("tray menu registration failed: {error}"));
        })
        .ok();

        let bus_name = format!("{SNI_BUS_NAME_PREFIX}-{}-1", std::process::id());
        let owner_id = {
            let bus_name_for_register = bus_name.clone();
            let last_error = last_error.clone();
            gio::bus_own_name_on_connection(
                &connection,
                &bus_name,
                gio::BusNameOwnerFlags::NONE,
                move |connection, _name| {
                    let parameters = (bus_name_for_register.as_str(),).to_variant();
                    if let Err(error) = connection.call_sync(
                        Some("org.kde.StatusNotifierWatcher"),
                        "/StatusNotifierWatcher",
                        "org.kde.StatusNotifierWatcher",
                        "RegisterStatusNotifierItem",
                        Some(&parameters),
                        None,
                        gio::DBusCallFlags::NONE,
                        1000,
                        gio::Cancellable::NONE,
                    ) {
                        *last_error.borrow_mut() =
                            Some(format!("status notifier watcher unavailable: {error}"));
                    }
                },
                |_connection, _name| {},
            )
        };

        runtime.connection = Some(connection);
        runtime.sni_registration = sni_registration;
        runtime.menu_registration = menu_registration;
        runtime.owner_id = Some(owner_id);
        runtime
    }

    pub fn update(&self, state: &NativeAppState) {
        self.state.replace(state.clone());
        let next_revision = self.menu_revision.borrow().wrapping_add(1).max(1);
        *self.menu_revision.borrow_mut() = next_revision;
        if let Some(connection) = &self.connection {
            let _ = connection.emit_signal(
                None,
                SNI_OBJECT_PATH,
                SNI_INTERFACE,
                "NewStatus",
                Some(&(sni_status(state),).to_variant()),
            );
            let _ =
                connection.emit_signal(None, SNI_OBJECT_PATH, SNI_INTERFACE, "NewToolTip", None);
            let _ = connection.emit_signal(
                None,
                MENU_OBJECT_PATH,
                MENU_INTERFACE,
                "LayoutUpdated",
                Some(&(*self.menu_revision.borrow(), 0i32).to_variant()),
            );
        }
    }

    pub fn drain(&mut self) -> Vec<TrayCommand> {
        let mut commands = Vec::new();
        while let Ok(command) = self.receiver.try_recv() {
            commands.push(command);
        }
        commands
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.borrow().clone()
    }
}

impl Drop for TrayRuntime {
    fn drop(&mut self) {
        if let Some(owner_id) = self.owner_id.take() {
            gio::bus_unown_name(owner_id);
        }
        if let Some(connection) = &self.connection {
            if let Some(registration) = self.sni_registration.take() {
                let _ = connection.unregister_object(registration);
            }
            if let Some(registration) = self.menu_registration.take() {
                let _ = connection.unregister_object(registration);
            }
        }
    }
}

fn register_sni_object(
    connection: &gio::DBusConnection,
    interface_info: &gio::DBusInterfaceInfo,
    state: Rc<RefCell<NativeAppState>>,
    icon: glib::Variant,
    sender: Sender<TrayCommand>,
) -> Result<gio::RegistrationId, glib::Error> {
    connection
        .register_object(SNI_OBJECT_PATH, interface_info)
        .method_call({
            let sender = sender.clone();
            move |_connection, _sender_name, _path, _interface, method, _parameters, invocation| {
                match method {
                    "Activate" | "ContextMenu" => {
                        let _ = sender.send(TrayCommand::ShowWindow);
                    }
                    "SecondaryActivate" => {
                        let _ = sender.send(TrayCommand::ToggleSession);
                    }
                    _ => {}
                }
                invocation.return_value(None);
            }
        })
        .property(
            move |_connection, _sender_name, _path, _interface, property| {
                let state = state.borrow();
                match property {
                    "Category" => "ApplicationStatus".to_variant(),
                    "Id" => "to.iris.nvpn".to_variant(),
                    "Title" => "Nostr VPN".to_variant(),
                    "Status" => sni_status(&state).to_variant(),
                    "WindowId" => 0u32.to_variant(),
                    "IconName" => "nostr-vpn".to_variant(),
                    "IconPixmap" | "OverlayIconPixmap" | "AttentionIconPixmap" => icon.clone(),
                    "OverlayIconName" | "AttentionIconName" | "AttentionMovieName" => {
                        String::new().to_variant()
                    }
                    "ToolTip" => glib::Variant::tuple_from_iter([
                        "nostr-vpn".to_variant(),
                        icon.clone(),
                        "Nostr VPN".to_variant(),
                        tray_status(&state).to_variant(),
                    ]),
                    "ItemIsMenu" => false.to_variant(),
                    "Menu" => MENU_OBJECT_PATH.to_variant(),
                    _ => glib::Variant::from_none(glib::VariantTy::VARIANT),
                }
            },
        )
        .build()
}

fn register_menu_object(
    connection: &gio::DBusConnection,
    interface_info: &gio::DBusInterfaceInfo,
    state: Rc<RefCell<NativeAppState>>,
    revision: Rc<RefCell<u32>>,
    sender: Sender<TrayCommand>,
) -> Result<gio::RegistrationId, glib::Error> {
    connection
        .register_object(MENU_OBJECT_PATH, interface_info)
        .method_call(
            move |_connection, _sender_name, _path, _interface, method, parameters, invocation| {
                let state = state.borrow();
                let root = build_menu(&state);
                match method {
                    "GetLayout" => {
                        invocation.return_value(Some(
                            &(*revision.borrow(), menu_node_layout(&root)).to_variant(),
                        ));
                    }
                    "GetGroupProperties" => {
                        let ids = parameters.child_get::<Vec<i32>>(0);
                        let items = if ids.is_empty() {
                            menu_group_properties(&root)
                        } else {
                            ids.into_iter()
                                .filter_map(|id| {
                                    find_menu_node(&root, id).map(|node| {
                                        glib::Variant::tuple_from_iter([
                                            id.to_variant(),
                                            menu_properties(node).to_variant(),
                                        ])
                                    })
                                })
                                .collect::<Vec<_>>()
                        };
                        let item_type = glib::VariantTy::new("(ia{sv})").expect("menu item type");
                        let properties = glib::Variant::array_from_iter_with_type(item_type, items);
                        invocation.return_value(Some(&(properties,).to_variant()));
                    }
                    "GetProperty" => {
                        let id = parameters.child_get::<i32>(0);
                        let name = parameters.child_get::<String>(1);
                        let value = find_menu_node(&root, id)
                            .and_then(|node| menu_properties(node).remove(&name))
                            .unwrap_or_else(|| false.to_variant());
                        invocation.return_value(Some(&(value,).to_variant()));
                    }
                    "Event" => {
                        let id = parameters.child_get::<i32>(0);
                        let event = parameters.child_get::<String>(1);
                        if event == "clicked" {
                            if let Some(command) =
                                find_menu_node(&root, id).and_then(|node| node.command.clone())
                            {
                                let _ = sender.send(command);
                            }
                        }
                        invocation.return_value(None);
                    }
                    "AboutToShow" => {
                        invocation.return_value(Some(&(false,).to_variant()));
                    }
                    _ => invocation.return_value(None),
                }
            },
        )
        .build()
}

#[derive(Clone)]
struct MenuNode {
    id: i32,
    label: String,
    enabled: bool,
    separator: bool,
    command: Option<TrayCommand>,
    children: Vec<MenuNode>,
}

fn build_menu(state: &NativeAppState) -> MenuNode {
    let mut children = vec![
        item(1, "Open Nostr VPN", true, TrayCommand::ShowWindow),
        separator(2),
        item(
            3,
            if state.session_active {
                "Disconnect VPN"
            } else {
                "Connect VPN"
            },
            state.vpn_session_control_supported,
            TrayCommand::ToggleSession,
        ),
        item(
            4,
            if state.advertise_exit_node {
                "Stop Offering Exit"
            } else {
                "Offer Private Exit"
            },
            true,
            TrayCommand::ToggleExitOffer,
        ),
        separator(5),
        item(
            6,
            "Copy This Device",
            !this_device_copy_value(state).is_empty(),
            TrayCommand::CopyThisDevice,
        ),
    ];

    if let Some(network) = active_network(state) {
        children.push(MenuNode {
            id: 20,
            label: display_network_name(network),
            enabled: true,
            separator: false,
            command: None,
            children: network
                .participants
                .iter()
                .enumerate()
                .map(|(index, participant)| {
                    item(
                        100 + index as i32,
                        &participant_menu_title(participant),
                        !participant.npub.is_empty(),
                        TrayCommand::CopyPeer(participant.npub.clone()),
                    )
                })
                .collect(),
        });

        let mut exit_children = vec![item(
            30,
            "No exit node",
            true,
            TrayCommand::SetExitNode(String::new()),
        )];
        exit_children.extend(
            network
                .participants
                .iter()
                .filter(|participant| participant.offers_exit_node)
                .enumerate()
                .map(|(index, participant)| {
                    item(
                        200 + index as i32,
                        &participant_menu_title(participant),
                        !participant.npub.is_empty(),
                        TrayCommand::SetExitNode(participant.npub.clone()),
                    )
                }),
        );
        children.push(MenuNode {
            id: 31,
            label: "Exit Node".to_string(),
            enabled: true,
            separator: false,
            command: None,
            children: exit_children,
        });
    }

    children.extend([
        separator(7),
        item(8, "Refresh", true, TrayCommand::Refresh),
        item(9, "Quit", true, TrayCommand::Quit),
    ]);

    MenuNode {
        id: 0,
        label: String::new(),
        enabled: true,
        separator: false,
        command: None,
        children,
    }
}

fn item(id: i32, label: &str, enabled: bool, command: TrayCommand) -> MenuNode {
    MenuNode {
        id,
        label: label.to_string(),
        enabled,
        separator: false,
        command: Some(command),
        children: Vec::new(),
    }
}

fn separator(id: i32) -> MenuNode {
    MenuNode {
        id,
        label: String::new(),
        enabled: false,
        separator: true,
        command: None,
        children: Vec::new(),
    }
}

fn menu_node_layout(node: &MenuNode) -> glib::Variant {
    let child_variants = node
        .children
        .iter()
        .map(|child| menu_node_layout(child).to_variant())
        .collect::<Vec<_>>();
    glib::Variant::tuple_from_iter([
        node.id.to_variant(),
        menu_properties(node).to_variant(),
        glib::Variant::array_from_iter::<glib::Variant>(child_variants),
    ])
}

fn menu_group_properties(root: &MenuNode) -> Vec<glib::Variant> {
    let mut values = Vec::new();
    collect_menu_group_properties(root, &mut values);
    values
}

fn collect_menu_group_properties(node: &MenuNode, values: &mut Vec<glib::Variant>) {
    values.push(glib::Variant::tuple_from_iter([
        node.id.to_variant(),
        menu_properties(node).to_variant(),
    ]));
    for child in &node.children {
        collect_menu_group_properties(child, values);
    }
}

fn menu_properties(node: &MenuNode) -> HashMap<String, glib::Variant> {
    let mut properties = HashMap::new();
    properties.insert("visible".to_string(), true.to_variant());
    properties.insert("enabled".to_string(), node.enabled.to_variant());
    if node.separator {
        properties.insert("type".to_string(), "separator".to_variant());
    } else {
        properties.insert("label".to_string(), node.label.to_variant());
    }
    if !node.children.is_empty() {
        properties.insert("children-display".to_string(), "submenu".to_variant());
    }
    properties
}

fn find_menu_node(node: &MenuNode, id: i32) -> Option<&MenuNode> {
    if node.id == id {
        return Some(node);
    }
    node.children
        .iter()
        .find_map(|child| find_menu_node(child, id))
}

pub fn this_device_copy_value(state: &NativeAppState) -> String {
    if !state.own_npub.trim().is_empty() {
        return state.own_npub.clone();
    }
    state.tunnel_ip.clone()
}

fn active_network(state: &NativeAppState) -> Option<&NativeNetworkState> {
    state
        .networks
        .iter()
        .find(|network| network.enabled)
        .or_else(|| state.networks.first())
}

fn display_network_name(network: &NativeNetworkState) -> String {
    if network.name.trim().is_empty() {
        "Network Devices".to_string()
    } else {
        network.name.clone()
    }
}

fn participant_menu_title(participant: &NativeParticipantState) -> String {
    let name = [
        participant.magic_dns_name.as_str(),
        participant.alias.as_str(),
        participant.magic_dns_alias.as_str(),
        participant.npub.as_str(),
    ]
    .into_iter()
    .find(|value| !value.trim().is_empty())
    .unwrap_or("Device");
    let tunnel_ip = participant.tunnel_ip.trim();
    if tunnel_ip.is_empty() || tunnel_ip == "-" {
        name.to_string()
    } else {
        format!("{name} ({tunnel_ip})")
    }
}

fn tray_status(state: &NativeAppState) -> String {
    if state.session_active {
        format!(
            "{} of {} devices connected",
            state.connected_peer_count, state.expected_peer_count
        )
    } else if !state.session_status.trim().is_empty() {
        state.session_status.clone()
    } else {
        "Disconnected".to_string()
    }
}

fn sni_status(state: &NativeAppState) -> &'static str {
    if state.session_active {
        "Active"
    } else {
        "Passive"
    }
}

fn tray_icon() -> glib::Variant {
    let image = image::load_from_memory_with_format(
        include_bytes!("../resources/nostr-vpn-64.png"),
        image::ImageFormat::Png,
    )
    .expect("bundled tray icon is a valid PNG");
    let (width, height) = image.dimensions();
    let mut data = image.into_rgba8().into_vec();
    for pixel in data.chunks_exact_mut(4) {
        pixel.rotate_right(1);
    }
    let icon = glib::Variant::tuple_from_iter([
        (width as i32).to_variant(),
        (height as i32).to_variant(),
        data.to_variant(),
    ]);
    let icon_type = glib::VariantTy::new("(iiay)").expect("icon pixmap type");
    glib::Variant::array_from_iter_with_type(icon_type, [icon])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_layout_has_dbusmenu_type() {
        let state = NativeAppState::default();
        let root = build_menu(&state);
        let layout = menu_node_layout(&root);
        assert_eq!(layout.type_().as_str(), "(ia{sv}av)");
    }
}
