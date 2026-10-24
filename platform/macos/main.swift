import AppKit
import Darwin
import SystemConfiguration

private let settingsKey = "cn.buaa.autologin.native.v1.settings"
private let credentialStorageKey = "cn.buaa.autologin.native.v2.credentials"
private let credentialMagic = Data("BUAACRED2".utf8)
private let credentialSecret = Array("BUAALogin/native/local/credential/v2".utf8)

private final class CallbackContext {
    weak var controller: Controller?
    let epoch: UInt64
    init(_ controller: Controller, _ epoch: UInt64) { self.controller = controller; self.epoch = epoch }
}
private let coreCallback: BuaaCallback = { pointer, json in
    guard let pointer = pointer, let json = json else { return }
    let box = Unmanaged<CallbackContext>.fromOpaque(pointer).takeUnretainedValue()
    let epoch = box.epoch
    guard let data = String(cString: json).data(using: .utf8),
          let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }
    DispatchQueue.main.async { [weak controller = box.controller] in
        guard let controller = controller, controller.epoch == epoch else { return }
        controller.receive(event)
    }
}
private func credentialHash(_ bytes: [UInt8]) -> UInt64 {
    var hash: UInt64 = 0xcbf29ce484222325
    for byte in credentialSecret + bytes {
        hash ^= UInt64(byte); hash = hash &* 0x100000001b3
    }
    return hash
}
private func credentialTransform(_ bytes: [UInt8], nonce: [UInt8]) -> [UInt8] {
    var state = credentialHash(nonce) | 1
    return bytes.map { byte in
        state ^= state << 13; state ^= state >> 7; state ^= state << 17
        return byte ^ UInt8(truncatingIfNeeded: state)
    }
}
private func credentialError() -> NSError {
    NSError(domain: "BUAALoginCredentials", code: 1, userInfo: [NSLocalizedDescriptionKey: "应用内凭据数据无效"])
}
private func saveCredentials(_ username: String, _ password: String) throws {
    let plain = try JSONSerialization.data(withJSONObject: ["username": username, "password": password])
    let nonce = (0..<16).map { _ in UInt8.random(in: .min ... .max) }
    var blob = credentialMagic
    blob.append(contentsOf: nonce)
    let tag = credentialHash(Array(plain))
    for shift in stride(from: 0, to: 64, by: 8) { blob.append(UInt8(truncatingIfNeeded: tag >> UInt64(shift))) }
    blob.append(contentsOf: credentialTransform(Array(plain), nonce: nonce))
    UserDefaults.standard.set(blob, forKey: credentialStorageKey)
}
private func loadCredentials() throws -> (String, String)? {
    guard let blob = UserDefaults.standard.data(forKey: credentialStorageKey) else { return nil }
    let prefix = credentialMagic.count
    guard blob.count >= prefix + 24, blob.prefix(prefix) == credentialMagic else { throw credentialError() }
    let bytes = Array(blob), nonce = Array(bytes[prefix..<(prefix + 16)])
    var storedTag: UInt64 = 0
    for (index, byte) in bytes[(prefix + 16)..<(prefix + 24)].enumerated() { storedTag |= UInt64(byte) << UInt64(index * 8) }
    let plain = credentialTransform(Array(bytes[(prefix + 24)...]), nonce: nonce)
    guard credentialHash(plain) == storedTag,
          let object = try JSONSerialization.jsonObject(with: Data(plain)) as? [String: String],
          let username = object["username"], let password = object["password"] else { throw credentialError() }
    return (username, password)
}
private func forgetCredentials() throws {
    UserDefaults.standard.removeObject(forKey: credentialStorageKey)
}

private final class FlippedContent: NSView {
    override var isFlipped: Bool { true }
}

private final class Controller: NSObject, NSApplicationDelegate, NSWindowDelegate, NSTextFieldDelegate {
    fileprivate var epoch: UInt64 = 0
    private var client: OpaquePointer?
    private var callbackBox: CallbackContext?
    private var timer: DispatchSourceTimer?
    private var store: SCDynamicStore?
    private var source: CFRunLoopSource?
    private let wifi = CampusWiFi()
    private var wifiNetworks: [String: Set<String>] = [:]
    private var wifiPanel: NSPanel?
    private var connectionPanel: NSPanel?
    private var accountPanel: NSPanel?
    private var interfaceDetailPanel: NSPanel?
    private var trafficPanel: NSPanel?
    private var intervalPanel: NSPanel?
    private var wifiRows: [NSButton] = []
    private var monitoring = false
    private var closing = false
    private var scanning = false
    private var interfaceRecords: [[String: Any]] = []
    private var selectedBindings = Set<String>()
    private var interfaceGateways: [String: String] = [:]
    private var interfaceRows: [NSButton] = []
    private var detailBinding: String?
    private var detailInterfaceName: String?
    private let detailGateway = NSTextField(string: "")
    private let wifiDefault = NSButton(checkboxWithTitle: "BUAA-WiFi", target: nil, action: nil)
    private let wifiFree = NSButton(checkboxWithTitle: "BUAA-WiFi-Free", target: nil, action: nil)
    private let wifiCustomEnabled = NSButton(checkboxWithTitle: "自定义 Wi-Fi 名称", target: nil, action: nil)
    private let wifiCustomName = NSTextField(string: "")
    private var observers: [NSObjectProtocol] = []
    private let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 680, height: 700),
        styleMask: [.titled, .closable, .miniaturizable], backing: .buffered, defer: false)
    private let username = NSTextField(string: "")
    private let password = NSSecureTextField(string: "")
    private let interval = NSTextField(string: "300")
    private var checkInterval: UInt64 = 300
    private let intervalSettings = NSButton(title: "设置", target: nil, action: nil)
    private let intervalError = NSTextField(labelWithString: "")
    private let gateway = NSTextField(string: "10.200.21.4")
    private let interfaceList = FlippedContent(frame: .zero)
    private let remember = NSButton(checkboxWithTitle: "记住账号密码（应用内加密）", target: nil, action: nil)
    private let start = NSButton(title: "开始自动连接", target: nil, action: nil)
    private let browserFallback = NSButton(checkboxWithTitle: "协议恢复失败后自动使用后台网页兜底", target: nil, action: nil)
    private let portalAuth = NSButton(checkboxWithTitle: "校园网认证（普通连接）", target: nil, action: nil)
    private let connectivityProbe = NSButton(checkboxWithTitle: "外网可用性检测", target: nil, action: nil)
    private let portalTest = NSButton(title: "测试", target: nil, action: nil)
    private let connectivityTest = NSButton(title: "测试", target: nil, action: nil)
    private let browserTest = NSButton(title: "测试", target: nil, action: nil)
    private var taskResults: [NSTextField] = []
    private var taskNext: [NSTextField] = []
    private let openLogs = NSButton(title: "打开日志文件夹", target: nil, action: nil)
    private let browserPath = NSTextField(string: "")
    private let detect = NSButton(title: "检测校园网", target: nil, action: nil)
    private let status = NSTextField(wrappingLabelWithString: "未启动 · 原生版首次使用请重新输入凭据")
    private let history = NSTextView()
    private let historyScroll = NSScrollView()
    private let pages = NSTabView()
    private let navigation = NSSegmentedControl(labels: ["连接", "设置"], trackingMode: .selectOne, target: nil, action: nil)
    private let launchAtLogin = NSButton(checkboxWithTitle: "开机自启", target: nil, action: nil)
    private let launchHint = NSTextField(wrappingLabelWithString: "登录系统后启动；保存完整配置和密码后自动连接。")
    private let loginSettings = NSButton(title: "打开系统登录项设置…", target: nil, action: nil)
    private let settingsScroll = NSScrollView()
    private let settingsForm = FlippedContent(frame: .zero)
    private let settingsHint = NSTextField(labelWithString: "开始时保存并应用设置。")
    private var dashboardValues: [NSTextField] = []
    private let accountButton = NSButton(title: "设置…", target: nil, action: nil)
    private let interfaceButton = NSButton(title: "接口设置…", target: nil, action: nil)
    private let trafficProgress = NSProgressIndicator()
    private let trafficLabel = NSTextField(labelWithString: "尚未获取流量")
    private let trafficTotal = NSTextField(string: "")
    private let trafficSettings = NSButton(title: "设置总流量…", target: nil, action: nil)
    private let trafficSettingsSummary = NSTextField(labelWithString: "未设置总流量")
    private let trafficError = NSTextField(labelWithString: "")
    private var trafficUsedBytes: UInt64?
    private var trafficTotalBeforeEditing = ""
    private let stateIcon = NSImageView()
    private let stateTitle = NSTextField(labelWithString: "尚未开始")
    private let authBadge = NSTextField(labelWithString: "● 未知")
    private let internetBadge = NSTextField(labelWithString: "● 未知")
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let trayStatus = NSMenuItem(title: "未启动", action: nil, keyEquivalent: "")
    private var lines: [String] = []
    private var testing: UInt32?
    private lazy var logFolder: URL = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Logs/BUAALogin", isDirectory: true)

    func applicationDidFinishLaunching(_ notification: Notification) {
        let loginLaunch = LoginStartup.launchedAtLogin
        if let identifier = Bundle.main.bundleIdentifier,
           let existing = NSRunningApplication.runningApplications(withBundleIdentifier: identifier).first(where: { $0.processIdentifier != ProcessInfo.processInfo.processIdentifier && !$0.isTerminated }) {
            if !loginLaunch { existing.activate(options: [.activateIgnoringOtherApps]) }
            NSApp.terminate(nil); return
        }
        guard buaa_abi_version() == 1 else { fatalError("Unsupported core ABI") }
        window.title = "北航校园网"; window.center(); window.delegate = self
        prepareLogFolder()
        window.titlebarSeparatorStyle = .none
        window.isReleasedWhenClosed = false
        window.initialFirstResponder = navigation
        let content = FlippedContent(frame: NSRect(x: 0, y: 0, width: 680, height: 700))
        window.contentView = content
        func place(_ view: NSView, in parent: NSView, _ x: CGFloat, _ y: CGFloat, _ width: CGFloat, _ height: CGFloat = 28) {
            view.frame = NSRect(x: x, y: y, width: width, height: height); parent.addSubview(view)
        }
        func label(_ text: String, in parent: NSView, _ x: CGFloat, _ y: CGFloat, _ width: CGFloat,
                   size: CGFloat = 13, weight: NSFont.Weight = .regular, secondary: Bool = false) {
            let label = NSTextField(labelWithString: text)
            label.font = .systemFont(ofSize: size, weight: weight)
            label.textColor = secondary ? .secondaryLabelColor : .labelColor
            place(label, in: parent, x, y, width, max(22, ceil(size * 1.4)))
        }
        func configureField(_ field: NSTextField, _ title: String, _ placeholder: String) {
            field.font = .systemFont(ofSize: 14); field.placeholderString = placeholder
            field.setAccessibilityLabel(title)
        }
        navigation.segmentStyle = .rounded; navigation.selectedSegment = 0
        navigation.target = self; navigation.action = #selector(selectPage)
        navigation.setWidth(130, forSegment: 0); navigation.setWidth(130, forSegment: 1)
        place(navigation, in: content, 207, 20, 266, 30)
        pages.tabViewType = .noTabsNoBorder
        place(pages, in: content, 32, 76, 616, 592)
        var views: [NSView] = []
        for name in ["连接", "设置"] {
            let page = FlippedContent(frame: NSRect(x: 0, y: 0, width: 616, height: 592))
            let item = NSTabViewItem(identifier: name); item.label = name; item.view = page
            pages.addTabViewItem(item); views.append(page)
        }
        let overview = views[0], settings = views[1]
        stateIcon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 36, weight: .regular)
        place(stateIcon, in: overview, 12, 4, 56, 56)
        stateTitle.font = .systemFont(ofSize: 22, weight: .semibold)
        place(stateTitle, in: overview, 88, 0, 528, 32)
        status.font = .systemFont(ofSize: 12); status.textColor = .secondaryLabelColor
        status.maximumNumberOfLines = 2; status.lineBreakMode = .byTruncatingTail
        status.stringValue = "请先设置账号，再开始自动连接。"
        place(status, in: overview, 88, 38, 528, 62)
        let statusSeparator = NSBox(); statusSeparator.boxType = .separator
        place(statusSeparator, in: overview, 0, 82, 616, 1)
        for (index, title) in ["账号", "当前接口", "检查周期", "上次检查", "本地 IP", "校园网 IP"].enumerated() {
            let x = CGFloat(index % 3) * 205, y = CGFloat(index / 3) * 52 + 94
            label(title, in: overview, x, y, 190, size: 11, secondary: true)
            let value = NSTextField(labelWithString: "—")
            value.font = .systemFont(ofSize: 14, weight: .medium); value.isSelectable = true
            value.lineBreakMode = .byTruncatingMiddle
            place(value, in: overview, x, y + 19, 190, 24); dashboardValues.append(value)
        }
        accountButton.bezelStyle = .rounded; accountButton.controlSize = .small
        accountButton.target = self; accountButton.action = #selector(showAccountSettings)
        place(accountButton, in: overview, 116, 109, 74, 25)
        dashboardValues[0].frame.size.width = 108
        interfaceButton.bezelStyle = .rounded; interfaceButton.controlSize = .small
        interfaceButton.target = self; interfaceButton.action = #selector(showConnectionSettings)
        place(interfaceButton, in: overview, 307, 109, 88, 25)
        dashboardValues[1].frame.size.width = 96
        intervalSettings.bezelStyle = .rounded; intervalSettings.controlSize = .small
        intervalSettings.target = self; intervalSettings.action = #selector(showIntervalSettings)
        intervalSettings.setAccessibilityLabel("设置检查周期")
        place(intervalSettings, in: overview, 526, 109, 64, 25)
        updateState("idle")
        label("流量使用情况", in: overview, 0, 202, 190, size: 11, secondary: true)
        trafficProgress.style = .bar
        trafficProgress.isIndeterminate = false
        trafficProgress.minValue = 0; trafficProgress.maxValue = 100; trafficProgress.doubleValue = 0
        place(trafficProgress, in: overview, 0, 226, 390, 12)
        trafficLabel.font = .systemFont(ofSize: 12, weight: .medium)
        place(trafficLabel, in: overview, 0, 242, 390, 22)
        configureField(trafficTotal, "总流量（GB）", "例如 100")
        trafficTotal.delegate = self
        trafficSettings.bezelStyle = .rounded; trafficSettings.controlSize = .small
        trafficSettings.target = self; trafficSettings.action = #selector(showTrafficSettings)
        portalAuth.state = .on; connectivityProbe.state = .on; browserFallback.state = .on
        browserFallback.title = "网页兜底"
        for (index, controls) in [(portalAuth, portalTest), (connectivityProbe, connectivityTest), (browserFallback, browserTest)].enumerated() {
            let y = CGFloat(282 + index * 48)
            place(controls.0, in: overview, 0, y, 190, 26)
            let result = NSTextField(labelWithString: "尚未运行")
            result.font = .systemFont(ofSize: 12); result.textColor = .secondaryLabelColor
            place(result, in: overview, 196, y + 2, 190, 22); taskResults.append(result)
            let next = NSTextField(labelWithString: index == 2 ? "—" : "启动后立即")
            next.font = .systemFont(ofSize: 11); next.textColor = .secondaryLabelColor
            place(next, in: overview, 388, y + 2, 142, 22); taskNext.append(next)
            controls.1.bezelStyle = .rounded; controls.1.controlSize = .small
            controls.1.tag = index; controls.1.target = self; controls.1.action = #selector(runTaskTest(_:))
            place(controls.1, in: overview, 540, y - 1, 76, 28)
        }
        connectivityProbe.target = self; connectivityProbe.action = #selector(connectivityToggleChanged)
        portalAuth.target = self; portalAuth.action = #selector(taskToggleChanged)
        browserFallback.target = self; browserFallback.action = #selector(taskToggleChanged)
        start.bezelStyle = .rounded; start.controlSize = .large
        start.keyEquivalent = "\r"
        place(start, in: overview, 456, 2, 160, 34)

        settingsScroll.hasVerticalScroller = true; settingsScroll.drawsBackground = false
        place(settingsScroll, in: settings, 0, 0, 616, 592)
        settingsForm.frame = NSRect(x: 0, y: 0, width: 596, height: 580)
        settingsScroll.documentView = settingsForm
        label("通用设置", in: settingsForm, 16, 0, 560, size: 18, weight: .semibold)
        launchAtLogin.target = self; launchAtLogin.action = #selector(startupChanged)
        place(launchAtLogin, in: settingsForm, 16, 46, 300, 24)
        launchHint.font = .systemFont(ofSize: 12); launchHint.textColor = .secondaryLabelColor
        place(launchHint, in: settingsForm, 16, 78, 550, 34)
        loginSettings.bezelStyle = .rounded; loginSettings.controlSize = .small
        loginSettings.target = self; loginSettings.action = #selector(openLoginSettings)
        place(loginSettings, in: settingsForm, 340, 42, 228, 30)
        label("流量套餐", in: settingsForm, 16, 132, 550)
        trafficSettingsSummary.font = .systemFont(ofSize: 12); trafficSettingsSummary.textColor = .secondaryLabelColor
        place(trafficSettingsSummary, in: settingsForm, 16, 160, 350, 24)
        place(trafficSettings, in: settingsForm, 418, 154, 148, 28)
        label("后台浏览器", in: settingsForm, 16, 204, 550)
        configureField(browserPath, "后台浏览器可执行文件", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
        place(browserPath, in: settingsForm, 16, 230, 550, 30)
        label("留空时自动查找 Chrome、Edge 或 Chromium。", in: settingsForm, 16, 266, 550, size: 12, secondary: true)
        let formSeparator = NSBox(); formSeparator.boxType = .separator
        place(formSeparator, in: settingsForm, 16, 302, 550, 1)
        label("最近活动", in: settingsForm, 16, 324, 300, size: 18, weight: .semibold)
        openLogs.bezelStyle = .rounded; openLogs.controlSize = .small
        openLogs.target = self; openLogs.action = #selector(openLogFolder)
        place(openLogs, in: settingsForm, 418, 320, 148, 28)
        historyScroll.hasVerticalScroller = true; historyScroll.borderType = .bezelBorder
        history.isEditable = false; history.isSelectable = true; history.font = .systemFont(ofSize: 12)
        history.textColor = .secondaryLabelColor; history.backgroundColor = .textBackgroundColor
        history.textContainerInset = NSSize(width: 10, height: 8)
        history.frame = NSRect(x: 0, y: 0, width: 550, height: 174)
        history.isVerticallyResizable = true; history.autoresizingMask = [.width]
        historyScroll.documentView = history
        place(historyScroll, in: settingsForm, 16, 358, 550, 188)

        configureField(username, "校园网账号", "学号 / 工号")
        configureField(password, "校园网密码", "请输入校园网密码")
        username.delegate = self; password.delegate = self
        remember.title = "记住账号密码"; remember.toolTip = "使用应用内轻量加密保存；取消勾选后删除保存的凭据。"
        configureField(interval, "检查周期（秒）", "10–86400"); interval.delegate = self
        configureField(gateway, "网关 IPv4 地址", "10.200.21.4"); gateway.delegate = self
        browserFallback.state = .on
        let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 600, height: 500), styleMask: [.titled], backing: .buffered, defer: false)
        panel.title = "接口设置"; panel.isReleasedWhenClosed = false
        let panelContent = FlippedContent(frame: NSRect(x: 0, y: 0, width: 600, height: 500)); panel.contentView = panelContent
        label("校园网接口（可多选）", in: panelContent, 24, 20, 360, size: 16, weight: .semibold)
        detect.title = "检测校园网"; detect.bezelStyle = .rounded
        place(detect, in: panelContent, 420, 16, 154, 32)
        let interfaceScroll = NSScrollView(frame: NSRect(x: 24, y: 58, width: 550, height: 250))
        interfaceScroll.hasVerticalScroller = true; interfaceScroll.borderType = .bezelBorder
        interfaceList.frame = NSRect(x: 0, y: 0, width: 530, height: 236)
        interfaceScroll.documentView = interfaceList; panelContent.addSubview(interfaceScroll)
        label("未勾选任何接口时自动选择；每个接口可单独设置网关与 Wi-Fi。", in: panelContent, 24, 314, 550, size: 12, secondary: true)
        label("默认网关地址", in: panelContent, 24, 350, 180, size: 13, weight: .semibold)
        place(gateway, in: panelContent, 24, 378, 240, 28)
        label("仅用于没有单独设置网关的接口", in: panelContent, 280, 382, 280, size: 12, secondary: true)
        let doneConnection = NSButton(title: "完成", target: self, action: #selector(closeConnectionSettings))
        doneConnection.bezelStyle = .rounded; doneConnection.keyEquivalent = "\r"
        place(doneConnection, in: panelContent, 474, 450, 100, 30)
        connectionPanel = panel
        rebuildInterfaceRows()

        let credentialsPanel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 500, height: 252), styleMask: [.titled], backing: .buffered, defer: false)
        credentialsPanel.title = "账号设置"; credentialsPanel.isReleasedWhenClosed = false
        let credentialsContent = FlippedContent(frame: NSRect(x: 0, y: 0, width: 500, height: 252)); credentialsPanel.contentView = credentialsContent
        label("校园网账号", in: credentialsContent, 24, 20, 452)
        place(username, in: credentialsContent, 24, 46, 452, 30)
        label("密码", in: credentialsContent, 24, 92, 452)
        place(password, in: credentialsContent, 24, 118, 452, 30)
        place(remember, in: credentialsContent, 24, 164, 300, 24)
        let cancelAccount = NSButton(title: "取消", target: self, action: #selector(closeAccountSettings))
        cancelAccount.bezelStyle = .rounded; cancelAccount.keyEquivalent = "\u{1b}"
        place(cancelAccount, in: credentialsContent, 278, 198, 90, 32)
        let saveAccount = NSButton(title: "保存", target: self, action: #selector(saveAccountSettings))
        saveAccount.bezelStyle = .rounded; saveAccount.keyEquivalent = "\r"
        place(saveAccount, in: credentialsContent, 380, 198, 96, 32)
        accountPanel = credentialsPanel
        start.target = self; start.action = #selector(dashboardAction)
        detect.target = self; detect.action = #selector(scan)
        remember.target = self; remember.action = #selector(rememberChanged)
        item.button?.image = NSImage(systemSymbolName: "graduationcap", accessibilityDescription: "北航校园网")
        item.button?.image?.isTemplate = true
        let menu = NSMenu(); trayStatus.isEnabled = false; menu.addItem(trayStatus); menu.addItem(.separator())
        for (title, action) in [("显示窗口", #selector(showWindow)), ("退出", #selector(quit))] {
            let entry = NSMenuItem(title: title, action: action, keyEquivalent: ""); entry.target = self; menu.addItem(entry)
        }
        item.menu = menu
        let main = NSMenu(); let appItem = NSMenuItem(); let appMenu = NSMenu()
        let quitItem = NSMenuItem(title: "退出北航校园网", action: #selector(quit), keyEquivalent: "q")
        quitItem.target = self; appMenu.addItem(quitItem); appItem.submenu = appMenu; main.addItem(appItem)
        let editItem = NSMenuItem(); let editMenu = NSMenu(title: "编辑")
        for (title, action, key) in [("剪切", "cut:", "x"), ("复制", "copy:", "c"), ("粘贴", "paste:", "v"), ("全选", "selectAll:", "a")] {
            editMenu.addItem(withTitle: title, action: Selector(action), keyEquivalent: key)
        }
        editItem.submenu = editMenu; main.addItem(editItem); NSApp.mainMenu = main
        if let settings = UserDefaults.standard.dictionary(forKey: settingsKey) {
            if let seconds = UInt64(settings["interval"] as? String ?? "300"), (10...86400).contains(seconds) { checkInterval = seconds }
            gateway.stringValue = settings["gateway"] as? String ?? "10.200.21.4"
            browserFallback.state = (settings["browser_fallback"] as? Bool ?? true) ? .on : .off
            portalAuth.state = (settings["portal_auth_enabled"] as? Bool ?? true) ? .on : .off
            connectivityProbe.state = (settings["connectivity_probe_enabled"] as? Bool ?? true) ? .on : .off
            connectivityToggleChanged()
            browserPath.stringValue = settings["browser_path"] as? String ?? ""
            if let bindings = settings["interfaces"] as? [String] { selectedBindings = Set(bindings) }
            else if let binding = settings["interface"] as? String { selectedBindings = [binding] }
            interfaceGateways = settings["interface_gateways"] as? [String: String] ?? [:]
            trafficTotal.stringValue = settings["traffic_total_gb"] as? String ?? ""
        }
        if let stored = UserDefaults.standard.dictionary(forKey: settingsKey + ".wifiNetworks") as? [String: [String]] {
            wifiNetworks = stored.mapValues(Set.init)
        } else {
            for name in UserDefaults.standard.stringArray(forKey: settingsKey + ".wifi") ?? [] {
                wifiNetworks[name] = ["BUAA-WiFi", "BUAA-WiFi-Free"]
            }
        }
        updateTraffic(nil)
        wifi.report = { [weak self] message in self?.record(message) }
        reloadCredentials()
        subscribeNetwork()
        let center = NSWorkspace.shared.notificationCenter
        observers.append(center.addObserver(forName: NSWorkspace.willSleepNotification, object: nil, queue: .main) { [weak self] _ in
            guard let self = self else { return }; self.cancelTimer(); self.wifi.suspend(); if let c = self.client { buaa_suspend(c) }
        })
        observers.append(center.addObserver(forName: NSWorkspace.didWakeNotification, object: nil, queue: .main) { [weak self] _ in
            self?.wifi.resume(); if let c = self?.client { buaa_resume(c) }
        })
        do { try LoginStartup.migrateLegacyIfNeeded() } catch { record("开机自启注册更新失败，请在设置中重新开启。") }
        refreshStartup(); updateStartTitle(); renderHistory(); resizeWindow(for: 0, animated: false)
        if loginLaunch {
            if remember.state == .on && completeConfiguration && store != nil && source != nil {
                NSApp.setActivationPolicy(.accessory); startMonitor()
                if monitoring { hideWindow() } else { showWindow() }
            } else {
                showWindow(); showAccountSettings(); record("开机自启：请先保存完整账号和密码。")
            }
        } else { showWindow(); scan() }
    }
    private func subscribeNetwork() {
        var context = SCDynamicStoreContext(version: 0, info: Unmanaged.passUnretained(self).toOpaque(), retain: nil, release: nil, copyDescription: nil)
        store = SCDynamicStoreCreate(nil, "BUAALogin" as CFString, { _, keys, info in
            guard let info = info else { return }
            let controller = Unmanaged<Controller>.fromOpaque(info).takeUnretainedValue()
            if let c = controller.client {
                let strings = keys as! [String]
                let names = Set(strings.compactMap { key -> String? in
                    let parts = key.split(separator: "/")
                    return parts.count > 3 && parts[2] == "Interface" ? String(parts[3]) : nil
                })
                if strings.contains(where: { $0.contains("/Global/") }) || names.isEmpty { buaa_network_changed(c) }
                else { for name in names { name.withCString { buaa_network_changed_interface(c, $0) } } }
            }
        }, &context)
        guard let store = store else { record("无法订阅网络变化通知"); return }
        let patterns = ["State:/Network/Interface/.*/IPv4", "State:/Network/Global/IPv4", "State:/Network/Interface/.*/Link"] as CFArray
        guard SCDynamicStoreSetNotificationKeys(store, nil, patterns) else { record("网络变化通知注册失败"); return }
        source = SCDynamicStoreCreateRunLoopSource(nil, store, 0)
        if let source = source { CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes) }
    }
    private func rebuildInterfaceRows() {
        interfaceList.subviews.forEach { $0.removeFromSuperview() }; interfaceRows = []
        if interfaceRecords.isEmpty {
            let empty = NSTextField(labelWithString: "尚未检测接口，请点击右上角“检测校园网”。")
            empty.textColor = .secondaryLabelColor; empty.frame = NSRect(x: 14, y: 18, width: 480, height: 24)
            interfaceList.addSubview(empty); interfaceList.setFrameSize(NSSize(width: 530, height: 236)); return
        }
        let height = CGFloat(max(236, interfaceRecords.count * 48))
        interfaceList.setFrameSize(NSSize(width: 530, height: height))
        for (index, record) in interfaceRecords.enumerated() {
            let binding = record["binding"] as? String ?? ""
            let title = "\(record["label"] as? String ?? binding) · \(record["message"] as? String ?? "")"
            let row = NSButton(checkboxWithTitle: title, target: self, action: #selector(interfaceSelectionChanged(_:)))
            row.identifier = NSUserInterfaceItemIdentifier(binding); row.state = selectedBindings.contains(binding) ? .on : .off
            row.frame = NSRect(x: 12, y: CGFloat(index * 48 + 8), width: 402, height: 28)
            interfaceList.addSubview(row); interfaceRows.append(row)
            let details = NSButton(title: "详情…", target: self, action: #selector(showInterfaceDetail(_:)))
            details.bezelStyle = .rounded; details.controlSize = .small; details.tag = index
            details.frame = NSRect(x: 430, y: CGFloat(index * 48 + 7), width: 78, height: 28)
            interfaceList.addSubview(details)
        }
    }
    @objc private func interfaceSelectionChanged(_ sender: NSButton) {
        guard let binding = sender.identifier?.rawValue else { return }
        if sender.state == .on { selectedBindings.insert(binding) } else { selectedBindings.remove(binding) }
        saveConnectionPreferences()
    }
    @objc private func showConnectionSettings() {
        guard let panel = connectionPanel, window.attachedSheet == nil else { return }
        rebuildInterfaceRows(); window.beginSheet(panel)
    }
    @objc private func closeConnectionSettings() {
        saveConnectionPreferences()
        if let panel = connectionPanel { window.endSheet(panel); panel.orderOut(nil) }
    }
    @objc private func showInterfaceDetail(_ sender: NSButton) {
        guard interfaceRecords.indices.contains(sender.tag), let parent = connectionPanel else { return }
        let record = interfaceRecords[sender.tag]
        let binding = record["binding"] as? String ?? ""
        let name = record["name"] as? String ?? binding
        detailBinding = binding; detailInterfaceName = name
        detailGateway.stringValue = interfaceGateways[binding] ?? ""
        let targets = wifiNetworks[name] ?? []
        wifiDefault.state = targets.contains("BUAA-WiFi") ? .on : .off
        wifiFree.state = targets.contains("BUAA-WiFi-Free") ? .on : .off
        let custom = targets.first { $0 != "BUAA-WiFi" && $0 != "BUAA-WiFi-Free" } ?? ""
        wifiCustomEnabled.state = custom.isEmpty ? .off : .on; wifiCustomName.stringValue = custom
        let isWiFi = wifi.interfaceNames().contains(name)
        let height: CGFloat = isWiFi ? 350 : 230
        let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 500, height: height), styleMask: [.titled], backing: .buffered, defer: false)
        panel.title = "接口详情 · \(name)"; panel.isReleasedWhenClosed = false
        let content = FlippedContent(frame: NSRect(x: 0, y: 0, width: 500, height: height)); panel.contentView = content
        let gatewayTitle = NSTextField(labelWithString: "自定义网关（留空使用 10.200.21.4）")
        gatewayTitle.frame = NSRect(x: 24, y: 22, width: 452, height: 22); content.addSubview(gatewayTitle)
        detailGateway.placeholderString = "10.200.21.4"; detailGateway.frame = NSRect(x: 24, y: 50, width: 260, height: 28); content.addSubview(detailGateway)
        if isWiFi {
            let wifiTitle = NSTextField(labelWithString: "Wi-Fi 自动连接")
            wifiTitle.font = .systemFont(ofSize: 14, weight: .semibold); wifiTitle.frame = NSRect(x: 24, y: 104, width: 300, height: 22); content.addSubview(wifiTitle)
            for (button, y) in [(wifiDefault, 136), (wifiFree, 168), (wifiCustomEnabled, 200)] {
                button.frame = NSRect(x: 24, y: CGFloat(y), width: 220, height: 24); content.addSubview(button)
            }
            wifiCustomName.placeholderString = "自定义 SSID"; wifiCustomName.frame = NSRect(x: 246, y: 198, width: 230, height: 28); content.addSubview(wifiCustomName)
            let hint = NSTextField(wrappingLabelWithString: "仅在自动连接运行期间生效；校园网网页密码不会用于 Wi-Fi。")
            hint.font = .systemFont(ofSize: 12); hint.textColor = .secondaryLabelColor
            hint.frame = NSRect(x: 24, y: 242, width: 452, height: 34); content.addSubview(hint)
        }
        let cancel = NSButton(title: "取消", target: self, action: #selector(closeInterfaceDetail))
        cancel.bezelStyle = .rounded; cancel.frame = NSRect(x: 286, y: height - 50, width: 88, height: 30); content.addSubview(cancel)
        let save = NSButton(title: "保存", target: self, action: #selector(saveInterfaceDetail))
        save.bezelStyle = .rounded; save.keyEquivalent = "\r"; save.frame = NSRect(x: 388, y: height - 50, width: 88, height: 30); content.addSubview(save)
        interfaceDetailPanel = panel; parent.beginSheet(panel)
    }
    @objc private func closeInterfaceDetail() {
        if let panel = interfaceDetailPanel, let parent = connectionPanel { parent.endSheet(panel); panel.orderOut(nil) }
        interfaceDetailPanel = nil
    }
    @objc private func saveInterfaceDetail() {
        guard let binding = detailBinding, let name = detailInterfaceName else { return }
        let gatewayValue = detailGateway.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        if !gatewayValue.isEmpty {
            var address = in_addr()
            guard gatewayValue.withCString({ inet_pton(AF_INET, $0, &address) }) == 1 else { alert("网关必须是 IPv4 地址"); return }
            interfaceGateways[binding] = gatewayValue
        } else { interfaceGateways.removeValue(forKey: binding) }
        if wifi.interfaceNames().contains(name) {
            var targets = Set<String>()
            if wifiDefault.state == .on { targets.insert("BUAA-WiFi") }
            if wifiFree.state == .on { targets.insert("BUAA-WiFi-Free") }
            let custom = wifiCustomName.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            if wifiCustomEnabled.state == .on && !custom.isEmpty { targets.insert(custom) }
            if targets.isEmpty { wifiNetworks.removeValue(forKey: name) } else { wifiNetworks[name] = targets }
            persistWiFiNetworks(); wifi.configure(networks: wifiNetworks, active: monitoring)
        }
        saveConnectionPreferences(); closeInterfaceDetail()
    }
    @objc private func showIntervalSettings() {
        guard intervalPanel == nil, window.attachedSheet == nil else { return }
        let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 420, height: 190), styleMask: [.titled], backing: .buffered, defer: false)
        panel.title = "设置检查周期"; panel.isReleasedWhenClosed = false
        let content = FlippedContent(frame: NSRect(x: 0, y: 0, width: 420, height: 190)); panel.contentView = content
        let hint = NSTextField(wrappingLabelWithString: "所有自动检查和失败重试共用此周期，最小 10 秒。")
        hint.font = .systemFont(ofSize: 13); hint.frame = NSRect(x: 24, y: 20, width: 372, height: 38); content.addSubview(hint)
        interval.stringValue = String(checkInterval)
        interval.frame = NSRect(x: 24, y: 70, width: 130, height: 28); content.addSubview(interval)
        let unit = NSTextField(labelWithString: "秒"); unit.frame = NSRect(x: 166, y: 74, width: 60, height: 22); content.addSubview(unit)
        intervalError.stringValue = ""; intervalError.textColor = .systemRed; intervalError.font = .systemFont(ofSize: 12)
        intervalError.frame = NSRect(x: 24, y: 106, width: 372, height: 22); content.addSubview(intervalError)
        let cancel = NSButton(title: "取消", target: self, action: #selector(closeIntervalSettings))
        cancel.bezelStyle = .rounded; cancel.keyEquivalent = "\u{1b}"
        cancel.frame = NSRect(x: 208, y: 142, width: 88, height: 30); content.addSubview(cancel)
        let save = NSButton(title: "保存", target: self, action: #selector(saveIntervalSettings))
        save.bezelStyle = .rounded; save.keyEquivalent = "\r"
        save.frame = NSRect(x: 306, y: 142, width: 88, height: 30); content.addSubview(save)
        intervalPanel = panel; window.beginSheet(panel); panel.makeFirstResponder(interval); interval.selectText(nil)
    }
    @objc private func closeIntervalSettings() {
        if let panel = intervalPanel { window.endSheet(panel); panel.orderOut(nil) }; intervalPanel = nil
    }
    @objc private func saveIntervalSettings() {
        guard let seconds = UInt64(interval.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)), (10...86400).contains(seconds) else {
            intervalError.stringValue = "请输入 10–86400 之间的整数秒数。"
            intervalPanel?.makeFirstResponder(interval); interval.selectText(nil); return
        }
        if let client = client, buaa_set_interval(client, seconds) != 0 {
            intervalError.stringValue = "保存失败，请稍后重试。"; return
        }
        checkInterval = seconds
        var settings = UserDefaults.standard.dictionary(forKey: settingsKey) ?? [:]
        settings["interval"] = String(seconds); UserDefaults.standard.set(settings, forKey: settingsKey)
        updateStartTitle(); closeIntervalSettings()
        if !monitoring { record("检查周期已设置为 \(seconds) 秒") }
    }
    @objc private func showTrafficSettings() {
        guard trafficPanel == nil, window.attachedSheet == nil else { return }
        trafficTotalBeforeEditing = trafficTotal.stringValue
        let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 420, height: 210), styleMask: [.titled], backing: .buffered, defer: false)
        panel.title = "设置总流量"; panel.isReleasedWhenClosed = false
        let content = FlippedContent(frame: NSRect(x: 0, y: 0, width: 420, height: 210)); panel.contentView = content
        let hint = NSTextField(wrappingLabelWithString: "设置套餐总流量后，连接页会按门户返回的已用流量显示进度。留空可关闭进度比例。")
        hint.font = .systemFont(ofSize: 13); hint.frame = NSRect(x: 24, y: 20, width: 372, height: 42); content.addSubview(hint)
        trafficTotal.frame = NSRect(x: 24, y: 76, width: 180, height: 28); content.addSubview(trafficTotal)
        let unit = NSTextField(labelWithString: "GB"); unit.frame = NSRect(x: 216, y: 80, width: 50, height: 22); content.addSubview(unit)
        trafficError.stringValue = ""; trafficError.textColor = .systemRed; trafficError.font = .systemFont(ofSize: 12)
        trafficError.frame = NSRect(x: 24, y: 112, width: 372, height: 22); content.addSubview(trafficError)
        let cancel = NSButton(title: "取消", target: self, action: #selector(closeTrafficSettings))
        cancel.bezelStyle = .rounded; cancel.keyEquivalent = "\u{1b}"; cancel.frame = NSRect(x: 208, y: 158, width: 88, height: 30); content.addSubview(cancel)
        let save = NSButton(title: "保存", target: self, action: #selector(saveTrafficSettings))
        save.bezelStyle = .rounded; save.keyEquivalent = "\r"; save.frame = NSRect(x: 306, y: 158, width: 88, height: 30); content.addSubview(save)
        trafficPanel = panel; window.beginSheet(panel); panel.makeFirstResponder(trafficTotal); trafficTotal.selectText(nil)
    }
    @objc private func closeTrafficSettings() {
        trafficTotal.stringValue = trafficTotalBeforeEditing
        if let panel = trafficPanel { window.endSheet(panel); panel.orderOut(nil) }
        trafficPanel = nil
    }
    @objc private func saveTrafficSettings() {
        let value = trafficTotal.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        if !value.isEmpty && Double(value).map({ $0.isFinite && $0 > 0 }) != true {
            trafficError.stringValue = "请输入大于 0 的数字，或留空关闭进度比例。"
            trafficPanel?.makeFirstResponder(trafficTotal); trafficTotal.selectText(nil); return
        }
        trafficTotal.stringValue = value
        trafficTotalBeforeEditing = value
        saveConnectionPreferences(); updateTraffic(nil); closeTrafficSettings()
        if !value.isEmpty { record("总流量已设置为 \(value) GB") }
    }
    @objc private func showAccountSettings() {
        guard let panel = accountPanel, window.attachedSheet == nil else { return }
        username.isEnabled = !monitoring; password.isEnabled = !monitoring; remember.isEnabled = !monitoring
        window.beginSheet(panel); panel.makeFirstResponder(username)
    }
    @objc private func closeAccountSettings() {
        if let panel = accountPanel { window.endSheet(panel); panel.orderOut(nil) }
    }
    @objc private func saveAccountSettings() {
        guard !username.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !password.stringValue.isEmpty else { alert("请输入账号和密码"); return }
        do {
            if remember.state == .on { try saveCredentials(username.stringValue.trimmingCharacters(in: .whitespacesAndNewlines), password.stringValue) }
            else { try forgetCredentials() }
            updateStartTitle(); closeAccountSettings(); record("账号设置已保存")
        } catch { alert("无法保存应用内加密凭据") }
    }
    @objc private func selectPage() {
        showPage(navigation.selectedSegment)
    }
    private func showPage(_ index: Int) {
        navigation.selectedSegment = index; pages.selectTabViewItem(at: index)
        window.makeFirstResponder(index == 1 && browserPath.isEnabled ? browserPath : navigation)
        start.keyEquivalent = index == 0 ? "\r" : ""
        if index == 1 { renderHistory() }
        resizeWindow(for: index, animated: true)
    }
    private func resizeWindow(for page: Int, animated: Bool) {
        let contentHeight: CGFloat = page == 0 ? 530 : 680
        let current = window.frame
        let targetContent = NSRect(x: 0, y: 0, width: 680, height: contentHeight)
        var target = window.frameRect(forContentRect: targetContent)
        target.origin.x = current.origin.x
        target.origin.y = current.maxY - target.height
        if animated { window.animator().setFrame(target, display: true) }
        else { window.setFrame(target, display: true) }
    }
    private var completeConfiguration: Bool {
        var address = in_addr()
        return !username.stringValue.trimmingCharacters(in: .whitespaces).isEmpty && !password.stringValue.isEmpty
            && (10...86400).contains(checkInterval)
            && gateway.stringValue.trimmingCharacters(in: .whitespaces).withCString({ inet_pton(AF_INET, $0, &address) }) == 1
    }
    private func saveConnectionPreferences() {
        var settings: [String: Any] = ["interval": String(checkInterval), "gateway": gateway.stringValue.trimmingCharacters(in: .whitespaces), "browser_fallback": browserFallback.state == .on, "portal_auth_enabled": portalAuth.state == .on, "connectivity_probe_enabled": connectivityProbe.state == .on, "browser_path": browserPath.stringValue.trimmingCharacters(in: .whitespaces), "interfaces": Array(selectedBindings).sorted(), "interface_gateways": interfaceGateways, "traffic_total_gb": trafficTotal.stringValue.trimmingCharacters(in: .whitespaces)]
        settings.removeValue(forKey: "interface")
        UserDefaults.standard.set(settings, forKey: settingsKey)
    }
    private func refreshStartup() {
        launchAtLogin.state = LoginStartup.enabled ? .on : .off
        loginSettings.isHidden = !LoginStartup.requiresApproval
        launchHint.stringValue = LoginStartup.requiresApproval
            ? "请在系统登录项设置中允许自启；保存完整配置和密码后自动连接。"
            : "登录系统后启动；需记住账号密码，保存完整配置后自动连接。"
    }
    func applicationDidBecomeActive(_ notification: Notification) { refreshStartup() }
    @objc private func openLoginSettings() { LoginStartup.openSystemSettings() }
    @objc private func startupChanged() {
        let enabled = launchAtLogin.state == .on
        do {
            // Enabling from a complete form makes it usable at next login without starting now.
            if enabled && !monitoring && !scanning && completeConfiguration {
                if remember.state == .on { try saveCredentials(username.stringValue.trimmingCharacters(in: .whitespaces), password.stringValue) }
                saveConnectionPreferences()
            }
            try LoginStartup.setEnabled(enabled)
            refreshStartup()
        } catch { refreshStartup(); alert("无法修改开机自启设置：\(error.localizedDescription)") }
    }
    @objc private func dashboardAction() {
        if monitoring { stopMonitor(); return }
        if (portalAuth.state == .on || browserFallback.state == .on) && (username.stringValue.trimmingCharacters(in: .whitespaces).isEmpty || password.stringValue.isEmpty) { showAccountSettings(); return }
        startMonitor()
    }
    func controlTextDidChange(_ notification: Notification) {
        updateStartTitle()
        if let field = notification.object as? NSTextField, field === browserPath || field === gateway {
            saveConnectionPreferences()
        }
    }
    func controlTextDidBeginEditing(_ notification: Notification) {
        if let field = notification.object as? NSTextField, field === username || field === password {
            accountPanel?.contentView?.scrollToVisible(field.frame)
        }
    }
    private func updateStartTitle() {
        let credentialsRequired = portalAuth.state == .on || browserFallback.state == .on
        let configured = !credentialsRequired || (!username.stringValue.trimmingCharacters(in: .whitespaces).isEmpty && !password.stringValue.isEmpty)
        start.title = monitoring ? "停止自动连接" : (configured ? "开始自动连接" : "设置账号")
        dashboardValues[0].stringValue = username.stringValue.trimmingCharacters(in: .whitespaces).isEmpty ? "未设置" : username.stringValue.trimmingCharacters(in: .whitespaces)
        dashboardValues[2].stringValue = "\(checkInterval) 秒"
    }
    private func focusSetting(_ field: NSTextField) {
        if field === interval { showIntervalSettings(); return }
        if field === username || field === password { showAccountSettings(); return }
        showPage(1)
        window.makeFirstResponder(field); field.selectText(nil)
    }
    private func renderHistory() {
        let visible = Array(lines.suffix(5))
        history.string = visible.isEmpty ? "暂无活动记录" : visible.joined(separator: "\n")
        history.scrollToEndOfDocument(nil)
    }
    private func localTime(_ value: Any?) -> String {
        guard let timestamp = value as? UInt64 else { return "—" }
        let formatter = DateFormatter(); formatter.dateFormat = "HH:mm:ss"
        return formatter.string(from: Date(timeIntervalSince1970: TimeInterval(timestamp)))
    }
    private func setBadge(_ field: NSTextField, _ text: String, _ color: NSColor) {
        let value = NSMutableAttributedString(string: "● ", attributes: [.foregroundColor: color, .font: NSFont.systemFont(ofSize: 14, weight: .bold)])
        value.append(NSAttributedString(string: text, attributes: [.foregroundColor: NSColor.labelColor, .font: NSFont.systemFont(ofSize: 12, weight: .medium)]))
        field.attributedStringValue = value
    }
    private func applyDashboard(_ event: [String: Any]) {
        guard let snapshot = event["dashboard"] as? [String: Any] else { return }
        updateState(snapshot["phase"] as? String ?? "idle")
        dashboardValues[0].stringValue = username.stringValue.trimmingCharacters(in: .whitespaces).isEmpty ? "未设置" : username.stringValue.trimmingCharacters(in: .whitespaces)
        dashboardValues[1].stringValue = snapshot["interface"] as? String ?? "—"
        dashboardValues[2].stringValue = "\(checkInterval) 秒"
        dashboardValues[3].stringValue = localTime(snapshot["last_check_at"])
        dashboardValues[4].stringValue = snapshot["local_ip"] as? String ?? "—"
        dashboardValues[5].stringValue = snapshot["campus_ip"] as? String ?? "—"
        updateTraffic(snapshot["traffic_used_bytes"] as? UInt64)
        let auth = ["unknown": "未知", "offline": "未认证", "online": "已认证"][snapshot["authentication"] as? String ?? "unknown"] ?? "未知"
        let internet = ["unknown": "未知", "available": "可用", "unavailable": "不可用"][snapshot["connectivity"] as? String ?? "unknown"] ?? "未知"
        let authColor: NSColor = auth == "已认证" ? .systemGreen : (auth == "未认证" ? .systemOrange : .secondaryLabelColor)
        let internetColor: NSColor = internet == "可用" ? .systemGreen : (internet == "不可用" ? .systemRed : .secondaryLabelColor)
        let message = event["message"] as? String ?? ""
        status.stringValue = message; trayStatus.title = message; item.button?.toolTip = message
        if snapshot["phase"] as? String != "stopped", let tasks = snapshot["tasks"] as? [String: Any] {
            for (index, key) in ["portal_auth", "connectivity", "browser_fallback"].enumerated() {
                guard let task = tasks[key] as? [String: Any] else { continue }
                let reason = task["schedule_reason"] as? String ?? ""
                if let next = task["next_run_at"] { taskNext[index].stringValue = "下次 \(localTime(next))" }
                else { taskNext[index].stringValue = ["disabled":"已关闭", "on_start":"启动后立即", "waiting_trigger":"—"][reason] ?? "—" }
                if index == 0 { setBadge(taskResults[index], auth, authColor) }
                else if index == 1 { setBadge(taskResults[index], internet, internetColor) }
                else { taskResults[index].stringValue = reason == "waiting_trigger" ? "—" : (task["message"] as? String ?? "—") }
            }
        }
    }
    private func updateTraffic(_ used: UInt64?) {
        if let used = used { trafficUsedBytes = used }
        let used = used ?? trafficUsedBytes
        trafficProgress.isIndeterminate = false
        let configuredTotal = trafficTotal.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        trafficSettingsSummary.stringValue = configuredTotal.isEmpty ? "未设置总流量" : "当前总流量：\(configuredTotal) GB"
        let formatter = ByteCountFormatter(); formatter.countStyle = .binary
        let totalGB = Double(trafficTotal.stringValue.trimmingCharacters(in: .whitespaces))
        let totalBytes = totalGB.flatMap { $0 > 0 ? $0 * 1_000_000_000 : nil }
        if let used = used {
            trafficLabel.stringValue = totalBytes.map { _ in "已用 \(formatter.string(fromByteCount: Int64(clamping: used))) / \(trafficTotal.stringValue) GB" }
                ?? "已用 \(formatter.string(fromByteCount: Int64(clamping: used))) · 设置总流量后显示进度"
            trafficProgress.doubleValue = totalBytes.map { min(100, Double(used) / $0 * 100) } ?? 0
        } else {
            trafficLabel.stringValue = "尚未获取流量"
            trafficProgress.doubleValue = 0
        }
    }
    private func updateState(_ state: String) {
        let title: String; let symbol: String; let color: NSColor
        switch state {
        case "online": title = "网络正常"; symbol = "checkmark.circle.fill"; color = .systemGreen
        case "suspect": title = "外网异常，正在复核"; symbol = "exclamationmark.circle"; color = .systemOrange
        case "protocol_recovery": title = "协议恢复中"; symbol = "arrow.clockwise.circle"; color = .controlAccentColor
        case "browser_recovery": title = "后台网页兜底中"; symbol = "globe"; color = .controlAccentColor
        case "verifying": title = "正在验证外网"; symbol = "network"; color = .controlAccentColor
        case "cooldown": title = "恢复冷却中"; symbol = "clock"; color = .systemOrange
        case "needs_user": title = "认证需要处理"; symbol = "exclamationmark.circle.fill"; color = .systemOrange
        case "retry": title = "等待自动重试"; symbol = "arrow.clockwise.circle"; color = .systemOrange
        case "error", "failed": title = "连接需要处理"; symbol = "exclamationmark.circle.fill"; color = .systemOrange
        case "suspended": title = "睡眠期间暂停"; symbol = "moon.circle"; color = .secondaryLabelColor
        case "waiting_network": title = "等待网络"; symbol = "pause.circle"; color = .secondaryLabelColor
        case "checking": title = "正在检查连接"; symbol = "network"; color = .controlAccentColor
        case "interfaces": title = "接口检测完成"; symbol = "network"; color = .controlAccentColor
        case "stopped": title = "已停止自动重连"; symbol = "pause.circle"; color = .secondaryLabelColor
        default: title = "北航校园网"; symbol = "graduationcap"; color = .secondaryLabelColor
        }
        stateTitle.stringValue = title
        stateIcon.image = NSImage(systemSymbolName: symbol, accessibilityDescription: title)
        stateIcon.contentTintColor = color
    }
    private func persistWiFiNetworks() {
        UserDefaults.standard.set(wifiNetworks.mapValues { Array($0).sorted() }, forKey: settingsKey + ".wifiNetworks")
    }
    private func alert(_ message: String) { let a = NSAlert(); a.messageText = "北航校园网"; a.informativeText = message; a.runModal() }
    private func reloadCredentials() {
        do { if let credentials = try loadCredentials() { username.stringValue = credentials.0; password.stringValue = credentials.1; remember.state = .on } }
        catch { record("应用内凭据读取失败，请重新输入") }
        updateStartTitle()
    }
    @objc private func rememberChanged() {
        if remember.state == .off { do { try forgetCredentials() } catch { remember.state = .on; alert("无法删除应用内凭据") } }
    }
    private func createClient(_ scan: Bool, requireCredentials: Bool = true) -> Bool {
        guard client == nil else { return false }
        let seconds = checkInterval
        if !scan && requireCredentials && (username.stringValue.trimmingCharacters(in: .whitespaces).isEmpty || password.stringValue.isEmpty) { focusSetting(username.stringValue.trimmingCharacters(in: .whitespaces).isEmpty ? username : password); alert("请输入账号和密码"); return false }
        var address = in_addr()
        guard gateway.stringValue.trimmingCharacters(in: .whitespaces).withCString({ inet_pton(AF_INET, $0, &address) }) == 1 else {
            focusSetting(gateway); alert("网关必须是 IPv4 地址"); return false
        }
        var config: [String: Any] = ["username": scan ? "" : username.stringValue.trimmingCharacters(in: .whitespaces), "password": scan ? "" : password.stringValue,
            "gateway_ip": gateway.stringValue.trimmingCharacters(in: .whitespaces), "interval": seconds, "external_timer": true,
            "portal_auth_enabled": portalAuth.state == .on, "connectivity_probe_enabled": connectivityProbe.state == .on,
            "browser_fallback": browserFallback.state == .on]
        if !browserPath.stringValue.trimmingCharacters(in: .whitespaces).isEmpty { config["browser_path"] = browserPath.stringValue.trimmingCharacters(in: .whitespaces) }
        if !scan { config["interfaces"] = Array(selectedBindings).sorted(); config["interface_gateways"] = interfaceGateways }
        guard let data = try? JSONSerialization.data(withJSONObject: config), let json = String(data: data, encoding: .utf8) else { return false }
        epoch &+= 1; callbackBox = CallbackContext(self, epoch)
        client = json.withCString { buaa_create($0, coreCallback, Unmanaged.passUnretained(callbackBox!).toOpaque()) }
        guard client != nil else { callbackBox = nil; focusSetting(gateway); alert("配置无效，请检查网关 IPv4 和间隔"); return false }
        return true
    }
    private func setBusy(_ busy: Bool) {
        [username, password, gateway, browserPath].forEach { $0.isEnabled = !busy }
        portalAuth.isEnabled = !busy; connectivityProbe.isEnabled = !busy
        browserFallback.isEnabled = !busy && connectivityProbe.state == .on
        interfaceButton.isEnabled = !busy; accountButton.isEnabled = !busy; remember.isEnabled = !busy; start.isEnabled = !scanning && testing == nil; detect.isEnabled = !busy
        updateStartTitle()
    }
    @objc private func connectivityToggleChanged() {
        if connectivityProbe.state == .off { browserFallback.state = .off }
        browserFallback.isEnabled = connectivityProbe.state == .on && !monitoring && !scanning
        taskNext[1].stringValue = connectivityProbe.state == .on ? "启动后立即" : "已关闭"
        taskNext[2].stringValue = browserFallback.state == .on ? "—" : "已关闭"
        updateStartTitle()
    }
    @objc private func taskToggleChanged() {
        taskNext[0].stringValue = portalAuth.state == .on ? "启动后立即" : "已关闭"
        taskNext[2].stringValue = browserFallback.state == .on ? "—" : "已关闭"
        updateStartTitle()
    }
    @objc private func runTaskTest(_ sender: NSButton) {
        let kind = UInt32(sender.tag)
        if kind != 1 && (username.stringValue.trimmingCharacters(in: .whitespaces).isEmpty || password.stringValue.isEmpty) { focusSetting(username); return }
        if monitoring, let c = client {
            testing = kind; [portalTest, connectivityTest, browserTest].forEach { $0.isEnabled = false }
            if buaa_test(c, kind) == 0 { taskResults[Int(kind)].stringValue = "正在测试…"; record("已请求\(["认证", "外网", "网页兜底"][Int(kind)])测试") }
            return
        }
        guard createClient(false, requireCredentials: kind != 1), let c = client else { return }
        testing = kind; setBusy(true); [portalTest, connectivityTest, browserTest].forEach { $0.isEnabled = false }
        taskResults[Int(kind)].stringValue = "正在测试…"
        if buaa_test(c, kind) != 0 { testing = nil; releaseClient(); setBusy(false); record("无法启动测试") }
    }
    @objc private func scan() {
        wifi.requestScan()
        guard createClient(true) else { return }
        scanning = true; setBusy(true); updateState("checking"); record("正在检测校园网接口…")
        if buaa_scan(client) != 0 { releaseClient(); setBusy(false); record("无法启动接口检测") }
    }
    @objc private func startMonitor() {
        guard store != nil && source != nil else { alert("无法订阅网络变化通知，请重启程序后再试"); return }
        let credentialsRequired = portalAuth.state == .on || browserFallback.state == .on
        guard createClient(false, requireCredentials: credentialsRequired) else { return }
        do {
            if remember.state == .on && credentialsRequired { try saveCredentials(username.stringValue.trimmingCharacters(in: .whitespaces), password.stringValue) }
            else { try forgetCredentials() }
        } catch { releaseClient(); alert("无法保存应用内加密凭据"); return }
        saveConnectionPreferences()
        scanning = false; setBusy(true)
        if buaa_start(client) != 0 { releaseClient(); setBusy(false); alert("无法启动监控"); return }
        monitoring = true; wifi.configure(networks: wifiNetworks, active: true)
        password.stringValue = ""; showPage(0); updateStartTitle(); updateState("checking"); record("正在检查校园网…")
    }
    @objc private func stopMonitor() { wifi.configure(networks: wifiNetworks, active: false); cancelTimer(); start.isEnabled = false; start.title = "正在停止…"; if let c = client { buaa_stop(c); record("正在停止…") } }
    private func cancelTimer() { timer?.cancel(); timer = nil }
    private func releaseClient() {
        cancelTimer(); if let c = client { buaa_free(c) }; client = nil; callbackBox = nil; epoch &+= 1
    }
    fileprivate func receive(_ event: [String: Any]) {
        let state = event["state"] as? String ?? "error"
        if state == "dashboard" { applyDashboard(event); return }
        if state == "schedule" {
            cancelTimer()
            if let delay = event["delay"] as? UInt64 {
                let t = DispatchSource.makeTimerSource(queue: .main)
                t.schedule(deadline: .now() + .seconds(Int(delay)), leeway: .seconds(event["leeway"] as? Int ?? 0))
                t.setEventHandler { [weak self] in if let c = self?.client { buaa_tick(c) } }
                timer = t; t.resume()
            }
            return
        }
        if !monitoring && (state != "stopped" || !scanning) { updateState(state) }
        if state == "interfaces", let records = event["records"] as? [[String: Any]] {
            interfaceRecords = records; rebuildInterfaceRows()
        }
        if state == "stopped" {
            let wasScanning = scanning; scanning = false
            monitoring = false; wifi.configure(networks: wifiNetworks, active: false)
            testing = nil; releaseClient(); setBusy(false); reloadCredentials()
            [portalTest, connectivityTest, browserTest].forEach { $0.isEnabled = true }
            taskNext[0].stringValue = portalAuth.state == .on ? "启动后立即" : "已关闭"
            connectivityToggleChanged()
            if closing { NSApp.terminate(nil); return }
            if wasScanning { return }
        }
        if state == "test_result", let kind = testing {
            taskResults[Int(kind)].stringValue = event["message"] as? String ?? "测试完成"
            testing = nil; [portalTest, connectivityTest, browserTest].forEach { $0.isEnabled = true }
            if monitoring { start.isEnabled = true }
        }
        if var message = event["message"] as? String, !message.isEmpty {
            if state == "retry", let timestamp = event["retry_at"] as? UInt64 {
                let formatter = DateFormatter(); formatter.dateFormat = "HH:mm:ss"
                message += "（下次 \(formatter.string(from: Date(timeIntervalSince1970: TimeInterval(timestamp))))）"
            }
            record(message)
        }
    }
    private func record(_ message: String) {
        if !monitoring { status.stringValue = message; trayStatus.title = message; item.button?.toolTip = message }
        let formatter = DateFormatter(); formatter.dateFormat = "HH:mm:ss"
        let line = "\(formatter.string(from: Date()))  \(message)"
        lines.append(line); if lines.count > 120 { lines.removeFirst(lines.count-120) }
        appendLog(line)
        if navigation.selectedSegment == 1 { renderHistory() }
    }
    private func prepareLogFolder() {
        do {
            try FileManager.default.createDirectory(at: logFolder, withIntermediateDirectories: true)
            let cutoff = Date().addingTimeInterval(-30 * 86400)
            for url in try FileManager.default.contentsOfDirectory(at: logFolder, includingPropertiesForKeys: [.contentModificationDateKey]) {
                if (try? url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate).map({ $0 < cutoff }) == true { try? FileManager.default.removeItem(at: url) }
            }
        } catch { }
    }
    private func appendLog(_ line: String) {
        let date = DateFormatter(); date.dateFormat = "yyyy-MM-dd"
        let url = logFolder.appendingPathComponent("BUAALogin-\(date.string(from: Date())).log")
        let data = Data((line + "\n").utf8)
        if !FileManager.default.fileExists(atPath: url.path) { _ = FileManager.default.createFile(atPath: url.path, contents: data); return }
        if let handle = try? FileHandle(forWritingTo: url) { defer { try? handle.close() }; _ = try? handle.seekToEnd(); try? handle.write(contentsOf: data) }
    }
    @objc private func openLogFolder() {
        prepareLogFolder()
        if !NSWorkspace.shared.open(logFolder) { record("无法打开日志文件夹") }
    }
    @objc private func hideWindow() { window.orderOut(nil); NSApp.setActivationPolicy(.accessory) }
    @objc private func showWindow() { NSApp.setActivationPolicy(.regular); NSApp.activate(ignoringOtherApps: true); window.makeKeyAndOrderFront(nil) }
    @objc private func quit() { closing = true; if client != nil { stopMonitor() } else { NSApp.terminate(nil) } }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool { showWindow(); return false }
    func windowShouldClose(_ sender: NSWindow) -> Bool { hideWindow(); return false }
    func windowDidMiniaturize(_ notification: Notification) { hideWindow() }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        if client != nil { closing = true; stopMonitor(); return .terminateCancel }; return .terminateNow
    }
    func applicationWillTerminate(_ notification: Notification) {
        wifi.close(); releaseClient()
        if let source = source { CFRunLoopRemoveSource(CFRunLoopGetMain(), source, .commonModes) }
        observers.forEach { NSWorkspace.shared.notificationCenter.removeObserver($0) }
        NSStatusBar.system.removeStatusItem(item)
    }
}
let app = NSApplication.shared
private let controller = Controller()
app.delegate = controller
app.run()
