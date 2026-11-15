import AppKit
import SwiftUI

@MainActor
@main
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private var window: NSWindow?
    private var controller: CloudController!

    static func main() {
        let app = NSApplication.shared
        let delegate = MainActor.assumeIsolated { AppDelegate() }
        app.delegate = delegate
        app.setActivationPolicy(.regular)
        app.run()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        controller = CloudController()
        buildMainMenu()

        let rootView = ContentView(ctrl: controller)
        let hosting = NSHostingController(rootView: rootView)
        let win = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1060, height: 740),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        win.title = "Comfy Cloud"
        win.titleVisibility = .hidden
        win.titlebarAppearsTransparent = true
        win.isMovableByWindowBackground = true
        win.minSize = NSSize(width: 1020, height: 710)
        win.contentViewController = hosting
        win.delegate = self
        win.center()
        win.makeKeyAndOrderFront(nil)
        self.window = win

        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    @objc private func refreshStatusAction() {
        Task { @MainActor in
            await controller.refreshProbe()
        }
    }

    private func buildMainMenu() {
        let mainMenu = NSMenu()

        let appItem = NSMenuItem()
        let appMenu = NSMenu()
        appMenu.addItem(NSMenuItem(title: "刷新状态", action: #selector(refreshStatusAction), keyEquivalent: "r"))
        appMenu.addItem(.separator())
        appMenu.addItem(NSMenuItem(title: "隐藏 Comfy Cloud", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h"))
        appMenu.addItem(NSMenuItem(title: "退出 Comfy Cloud", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q"))
        appItem.submenu = appMenu
        mainMenu.addItem(appItem)

        let windowItem = NSMenuItem()
        let windowMenu = NSMenu(title: "窗口")
        windowMenu.addItem(NSMenuItem(title: "最小化", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m"))
        windowMenu.addItem(NSMenuItem(title: "关闭", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w"))
        windowItem.submenu = windowMenu
        mainMenu.addItem(windowItem)

        NSApp.mainMenu = mainMenu
    }
}
