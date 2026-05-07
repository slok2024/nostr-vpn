import AppKit
import Darwin
import Foundation

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private let singleInstance = SingleInstanceCoordinator()
    private weak var manager: AppManager?
    private var pendingUrls: [URL] = []
    private var startsHidden = false

    func applicationWillFinishLaunching(_ notification: Notification) {
        singleInstance.onOpen = { [weak self] urls in
            self?.route(urls: urls, activate: true)
        }
        if !singleInstance.claimOrNotifyCurrentLaunch() {
            NSApp.terminate(nil)
        }
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        installApplicationIcon()
        observeWindows()
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        showMainWindow()
        return false
    }

    func applicationWillTerminate(_ notification: Notification) {
        singleInstance.release()
    }

    func configure(manager: AppManager) {
        self.manager = manager
        startsHidden = manager.launchedHidden && !Self.launchArgumentsContainDeepLink
        observeWindows()
        route(urls: pendingUrls, activate: !startsHidden)
        pendingUrls.removeAll()
        if startsHidden {
            hideMainWindowSoon()
        }
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard sender.title == "Nostr VPN", manager?.state.closeToTrayOnClose == true else {
            return true
        }
        sender.orderOut(nil)
        return false
    }

    func showMainWindow() {
        NSApp.unhide(nil)
        NSApp.activate(ignoringOtherApps: true)
        observeWindows()
        if let window = NSApp.windows.first(where: { $0.title == "Nostr VPN" }) ?? NSApp.windows.first {
            window.makeKeyAndOrderFront(nil)
        }
    }

    private func route(urls: [URL], activate: Bool) {
        guard !urls.isEmpty else {
            if activate {
                showMainWindow()
            }
            return
        }
        guard let manager else {
            pendingUrls.append(contentsOf: urls)
            return
        }
        for url in urls {
            manager.handle(url: url)
        }
        if activate {
            showMainWindow()
        }
    }

    private func observeWindows() {
        for window in NSApp.windows where window.title == "Nostr VPN" {
            window.delegate = self
            configureMainWindow(window)
        }
    }

    private func configureMainWindow(_ window: NSWindow) {
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.styleMask.insert(.fullSizeContentView)
        window.toolbar?.isVisible = false
    }

    private func installApplicationIcon() {
        let icon =
            Bundle.main.url(forResource: "AppIcon", withExtension: "icns")
                .flatMap(NSImage.init(contentsOf:))
            ?? NSImage(named: "AppIcon")
            ?? NSWorkspace.shared.icon(forFile: Bundle.main.bundlePath)
        NSApp.applicationIconImage = icon
    }

    private func hideMainWindowSoon() {
        DispatchQueue.main.async {
            NSApp.windows.first(where: { $0.title == "Nostr VPN" })?.orderOut(nil)
        }
    }

    private static var launchArgumentsContainDeepLink: Bool {
        CommandLine.arguments.contains { $0.starts(with: "nvpn://") }
    }
}

final class SingleInstanceCoordinator: NSObject {
    private let notificationName = Notification.Name("to.iris.nvpn.macos.open")
    private var lockFd: Int32 = -1
    var onOpen: (([URL]) -> Void)?

    func claimOrNotifyCurrentLaunch() -> Bool {
        let lockPath = lockFilePath()
        let fd = open(lockPath, O_CREAT | O_RDWR, S_IRUSR | S_IWUSR)
        guard fd >= 0 else {
            return true
        }
        if flock(fd, LOCK_EX | LOCK_NB) == 0 {
            lockFd = fd
            DistributedNotificationCenter.default().addObserver(
                self,
                selector: #selector(receiveOpenNotification(_:)),
                name: notificationName,
                object: nil
            )
            return true
        }

        close(fd)
        let urls = Self.startupUrls().map(\.absoluteString)
        DistributedNotificationCenter.default().postNotificationName(
            notificationName,
            object: nil,
            userInfo: ["urls": urls],
            deliverImmediately: true
        )
        return false
    }

    func release() {
        DistributedNotificationCenter.default().removeObserver(self)
        if lockFd >= 0 {
            flock(lockFd, LOCK_UN)
            close(lockFd)
            lockFd = -1
        }
    }

    @objc private func receiveOpenNotification(_ notification: Notification) {
        let urls = (notification.userInfo?["urls"] as? [String] ?? [])
            .compactMap(URL.init(string:))
        onOpen?(urls)
    }

    private func lockFilePath() -> String {
        let dir = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first?
            .appendingPathComponent("nvpn", isDirectory: true)
            ?? FileManager.default.temporaryDirectory.appendingPathComponent("nvpn", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("NostrVpnMac.lock").path
    }

    private static func startupUrls() -> [URL] {
        CommandLine.arguments.compactMap { argument in
            guard argument.starts(with: "nvpn://") else {
                return nil
            }
            return URL(string: argument)
        }
    }
}
