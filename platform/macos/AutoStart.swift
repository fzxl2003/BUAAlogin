import AppKit
import Carbon
import ServiceManagement

/// Current-user login startup only. No helper process or periodic launch job.
enum LoginStartup {
    private static let label = "cn.buaa.autologin.native.v1.login"
    private static var legacyURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/LaunchAgents/\(label).plist")
    }
    static var enabled: Bool {
        if #available(macOS 13.0, *) {
            return SMAppService.mainApp.status == .enabled || SMAppService.mainApp.status == .requiresApproval
        }
        return FileManager.default.fileExists(atPath: legacyURL.path)
    }
    static var requiresApproval: Bool {
        if #available(macOS 13.0, *) { return SMAppService.mainApp.status == .requiresApproval }
        return false
    }
    static var launchedAtLogin: Bool {
        if CommandLine.arguments.dropFirst().contains("--autostart") { return true }
        guard let event = NSAppleEventManager.shared().currentAppleEvent else { return false }
        return event.eventID == kAEOpenApplication
            && event.paramDescriptor(forKeyword: keyAEPropData)?.enumCodeValue == keyAELaunchedAsLogInItem
    }
    static func migrateLegacyIfNeeded() throws {
        if #available(macOS 13.0, *), FileManager.default.fileExists(atPath: legacyURL.path) { try setEnabled(true) }
    }
    static func setEnabled(_ enabled: Bool) throws {
        if enabled && Bundle.main.bundleURL.path.contains("/AppTranslocation/") {
            throw NSError(domain: "cn.buaa.autologin.native.login", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "请先将应用移到应用程序文件夹，再开启开机自启。"])
        }
        if #available(macOS 13.0, *) {
            let service = SMAppService.mainApp
            if enabled {
                if service.status != .enabled && service.status != .requiresApproval { try service.register() }
            } else if service.status == .enabled || service.status == .requiresApproval {
                try service.unregister()
            }
            // Remove our macOS 12 entry after an OS upgrade to avoid duplicate launches.
            if FileManager.default.fileExists(atPath: legacyURL.path) { try FileManager.default.removeItem(at: legacyURL) }
        } else if enabled {
            guard let executable = Bundle.main.executableURL else { throw CocoaError(.fileNoSuchFile) }
            let job: [String: Any] = ["Label": label, "ProgramArguments": [executable.path, "--autostart"],
                "RunAtLoad": true, "LimitLoadToSessionType": "Aqua", "ProcessType": "Interactive"]
            let data = try PropertyListSerialization.data(fromPropertyList: job, format: .xml, options: 0)
            try FileManager.default.createDirectory(at: legacyURL.deletingLastPathComponent(), withIntermediateDirectories: true)
            try data.write(to: legacyURL, options: .atomic)
        } else if FileManager.default.fileExists(atPath: legacyURL.path) {
            try FileManager.default.removeItem(at: legacyURL)
        }
    }
    static func openSystemSettings() {
        if #available(macOS 13.0, *) { SMAppService.openSystemSettingsLoginItems() }
    }
}
