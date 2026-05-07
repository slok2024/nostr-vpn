import AppKit
import SwiftUI

@main
struct NostrVpnMacApp: App {
    @StateObject private var manager = AppManager()
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.openWindow) private var openWindow

    var body: some Scene {
        WindowGroup("Nostr VPN", id: "main") {
            RootView(manager: manager)
                .frame(minWidth: 880, minHeight: 620)
                .onAppear {
                    appDelegate.configure(manager: manager)
                    manager.start()
                }
                .onOpenURL { url in
                    manager.handle(url: url)
                }
                .onChange(of: scenePhase) { _, phase in
                    if phase == .active {
                        manager.refresh()
                    }
                }
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(width: 1100, height: 760)
        .windowResizability(.automatic)

        MenuBarExtra("Nostr VPN", image: "TrayIcon") {
            StatusMenuView(manager: manager) {
                openWindow(id: "main")
                appDelegate.showMainWindow()
                NSApp.activate(ignoringOtherApps: true)
            }
        }
    }
}
