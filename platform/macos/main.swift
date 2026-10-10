import AppKit
import Darwin
import Security
import SystemConfiguration

private let credentialService = "cn.buaa.autologin.native.v1.credentials"
private let settingsKey = "cn.buaa.autologin.native.v1.settings"

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
private func credentialQuery() -> [String: Any] {
    [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: credentialService]
}
private func saveCredentials(_ username: String, _ password: String) throws {
    let values: [String: Any] = [kSecAttrAccount as String: username, kSecValueData as String: Data(password.utf8)]
    var status = SecItemUpdate(credentialQuery() as CFDictionary, values as CFDictionary)
    if status == errSecItemNotFound {
        var query = credentialQuery(); values.forEach { query[$0.key] = $0.value }
        query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        status = SecItemAdd(query as CFDictionary, nil)
    }
    if status != errSecSuccess { throw NSError(domain: NSOSStatusErrorDomain, code: Int(status)) }
}
private func loadCredentials() throws -> (String, String)? {
    var query = credentialQuery()
    query[kSecReturnAttributes as String] = true; query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var result: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &result)
    if status == errSecItemNotFound { return nil }
    guard status == errSecSuccess, let item = result as? [String: Any],
          let username = item[kSecAttrAccount as String] as? String,
          let data = item[kSecValueData as String] as? Data, let password = String(data: data, encoding: .utf8)
    else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(status)) }
    return (username, password)
}
private func forgetCredentials() throws {
    let status = SecItemDelete(credentialQuery() as CFDictionary)
    if status != errSecSuccess && status != errSecItemNotFound { throw NSError(domain: NSOSStatusErrorDomain, code: Int(status)) }
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
    private var wifiEnabled = Set<String>()
    private var wifiPanel: NSPanel?
    private var wifiRows: [NSButton] = []
    private var monitoring = false
    private var closing = false
    private var scanning = false
    private var selectionBindings: [String?] = [nil]
    private var observers: [NSObjectProtocol] = []
    private let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 680, height: 600),
        styleMask: [.titled, .closable, .miniaturizable], backing: .buffered, defer: false)
    private let username = NSTextField(string: "")
    private let password = NSSecureTextField(string: "")
    private let interval = NSTextField(string: "300")
    private let gateway = NSTextField(string: "10.200.21.4")
    private let interfaces = NSPopUpButton(frame: .zero, pullsDown: false)
    private let remember = NSButton(checkboxWithTitle: "记住账号密码（系统钥匙串）", target: nil, action: nil)
    private let start = NSButton(title: "开始自动重连", target: nil, action: nil)
    private let stop = NSButton(title: "停止", target: nil, action: nil)
    private let detect = NSButton(title: "检测校园网", target: nil, action: nil)
    private let status = NSTextField(wrappingLabelWithString: "未启动 · 原生版首次使用请重新输入凭据")
    private let history = NSTextView()
    private let historyScroll = NSScrollView()
    private let pages = NSTabView()
    private let navigation = NSSegmentedControl(labels: ["连接仪表盘", "设置"], trackingMode: .selectOne, target: nil, action: nil)
    private let launchAtLogin = NSButton(checkboxWithTitle: "开机自启", target: nil, action: nil)
    private let launchHint = NSTextField(wrappingLabelWithString: "登录系统后启动；保存完整配置和密码后自动连接。")
    private let loginSettings = NSButton(title: "打开系统登录项设置…", target: nil, action: nil)
    private let settingsStart = NSButton(title: "开始自动重连", target: nil, action: nil)
    private let settingsScroll = NSScrollView()
    private let settingsForm = FlippedContent(frame: .zero)
    private let advancedSection = FlippedContent(frame: .zero)
    private let advancedButton = NSButton()
    private let settingsHint = NSTextField(labelWithString: "开始时保存并应用设置。")
    private let historyButton = NSButton(title: "查看完整记录", target: nil, action: nil)
    private var historyExpanded = false
    private var dashboardValues: [NSTextField] = []
    private let stateIcon = NSImageView()
    private let stateTitle = NSTextField(labelWithString: "尚未开始")
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let trayStatus = NSMenuItem(title: "未启动", action: nil, keyEquivalent: "")
    private var lines: [String] = []

    func applicationDidFinishLaunching(_ notification: Notification) {
        let loginLaunch = LoginStartup.launchedAtLogin
        if let identifier = Bundle.main.bundleIdentifier,
           let existing = NSRunningApplication.runningApplications(withBundleIdentifier: identifier).first(where: { $0.processIdentifier != ProcessInfo.processInfo.processIdentifier && !$0.isTerminated }) {
            if !loginLaunch { existing.activate(options: [.activateIgnoringOtherApps]) }
            NSApp.terminate(nil); return
        }
        guard buaa_abi_version() == 1 else { fatalError("Unsupported core ABI") }
        window.title = "北航校园网"; window.center(); window.delegate = self
        window.titlebarSeparatorStyle = .none
        window.isReleasedWhenClosed = false
        window.initialFirstResponder = navigation
        let content = FlippedContent(frame: NSRect(x: 0, y: 0, width: 680, height: 600))
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
        navigation.setWidth(150, forSegment: 0); navigation.setWidth(100, forSegment: 1)
        place(navigation, in: content, 207, 20, 266, 30)
        pages.tabViewType = .noTabsNoBorder
        place(pages, in: content, 32, 76, 616, 492)
        var views: [NSView] = []
        for name in ["连接仪表盘", "设置"] {
            let page = FlippedContent(frame: NSRect(x: 0, y: 0, width: 616, height: 492))
            let item = NSTabViewItem(identifier: name); item.label = name; item.view = page
            pages.addTabViewItem(item); views.append(page)
        }
        let overview = views[0], settings = views[1]
        stateIcon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 36, weight: .regular)
        place(stateIcon, in: overview, 12, 4, 56, 56)
        stateTitle.font = .systemFont(ofSize: 22, weight: .semibold)
        place(stateTitle, in: overview, 88, 0, 528, 32)
        status.font = .systemFont(ofSize: 12); status.textColor = .secondaryLabelColor
        status.maximumNumberOfLines = 4; status.lineBreakMode = .byWordWrapping
        status.stringValue = "先在设置中填写账号，再开始自动重连。"
        place(status, in: overview, 88, 38, 528, 62)
        let statusSeparator = NSBox(); statusSeparator.boxType = .separator
        place(statusSeparator, in: overview, 0, 112, 616, 1)
        for (index, title) in ["当前接口", "本地 IP", "校园网 IP", "上次检查", "检查周期", "下次重试"].enumerated() {
            let x = CGFloat(index % 2) * 322, y = CGFloat(index / 2) * 58 + 130
            label(title, in: overview, x, y, 290, size: 11, secondary: true)
            let value = NSTextField(labelWithString: "—")
            value.font = .systemFont(ofSize: 14, weight: .medium); value.isSelectable = true
            value.lineBreakMode = .byTruncatingMiddle
            place(value, in: overview, x, y + 22, 290, 24); dashboardValues.append(value)
        }
        updateState("idle")
        label("最近活动", in: overview, 0, 310, 300, size: 13, weight: .semibold)
        historyButton.bezelStyle = .rounded; historyButton.controlSize = .small
        historyButton.target = self; historyButton.action = #selector(toggleHistory)
        place(historyButton, in: overview, 470, 304, 148, 28)
        historyScroll.hasVerticalScroller = true; historyScroll.borderType = .bezelBorder
        history.isEditable = false; history.isSelectable = true; history.font = .systemFont(ofSize: 12)
        history.textColor = .secondaryLabelColor; history.backgroundColor = .textBackgroundColor
        history.textContainerInset = NSSize(width: 10, height: 8)
        history.frame = NSRect(x: 0, y: 0, width: 616, height: 96)
        history.isVerticallyResizable = true; history.autoresizingMask = [.width]
        historyScroll.documentView = history
        place(historyScroll, in: overview, 0, 338, 616, 96)
        for button in [start, stop, settingsStart] { button.bezelStyle = .rounded; button.controlSize = .large }
        start.keyEquivalent = "\r"
        place(start, in: overview, 0, 456, 182, 34); place(stop, in: overview, 194, 456, 86, 34)
        let hide = NSButton(title: "收起至菜单栏", target: self, action: #selector(hideWindow))
        hide.bezelStyle = .rounded; place(hide, in: overview, 448, 456, 168, 34)

        settingsScroll.hasVerticalScroller = true; settingsScroll.drawsBackground = false
        place(settingsScroll, in: settings, 0, 0, 616, 432)
        settingsForm.frame = NSRect(x: 0, y: 0, width: 596, height: 582)
        settingsScroll.documentView = settingsForm
        label("账号", in: settingsForm, 16, 0, 560, size: 18, weight: .semibold)
        label("用于登录校园网，凭据可保存在系统钥匙串中。", in: settingsForm, 16, 30, 560, size: 12, secondary: true)
        label("校园网账号", in: settingsForm, 16, 72, 560)
        configureField(username, "校园网账号", "学号 / 工号")
        place(username, in: settingsForm, 16, 98, 550, 30)
        label("密码", in: settingsForm, 16, 150, 560)
        configureField(password, "校园网密码", "请输入校园网密码")
        place(password, in: settingsForm, 16, 176, 550, 30)
        username.delegate = self; password.delegate = self
        remember.title = "记住账号密码"; remember.toolTip = "使用系统钥匙串保存；取消勾选后删除保存的凭据。"
        place(remember, in: settingsForm, 16, 224, 550, 24)
        launchAtLogin.target = self; launchAtLogin.action = #selector(startupChanged)
        place(launchAtLogin, in: settingsForm, 16, 266, 300, 24)
        launchHint.font = .systemFont(ofSize: 12); launchHint.textColor = .secondaryLabelColor
        place(launchHint, in: settingsForm, 16, 300, 550, 34)
        loginSettings.bezelStyle = .rounded; loginSettings.controlSize = .small
        loginSettings.target = self; loginSettings.action = #selector(openLoginSettings)
        place(loginSettings, in: settingsForm, 340, 262, 228, 30)
        let formSeparator = NSBox(); formSeparator.boxType = .separator
        place(formSeparator, in: settingsForm, 16, 344, 550, 1)
        label("连接", in: settingsForm, 16, 366, 550, size: 18, weight: .semibold)
        label("校园网接口", in: settingsForm, 16, 406, 550)
        interfaces.addItem(withTitle: "自动选择"); interfaces.setAccessibilityLabel("校园网接口")
        place(interfaces, in: settingsForm, 12, 432, 380, 32)
        detect.bezelStyle = .rounded; place(detect, in: settingsForm, 408, 432, 160, 32)
        label("检查间隔", in: settingsForm, 16, 482, 550)
        configureField(interval, "检查间隔（秒）", "10–86400"); interval.delegate = self
        place(interval, in: settingsForm, 16, 508, 100, 28)
        label("秒 · 最低 10 秒，失败时采用递增退避", in: settingsForm, 132, 513, 434, size: 12, secondary: true)
        advancedButton.setButtonType(.pushOnPushOff); advancedButton.bezelStyle = .disclosure
        advancedButton.target = self; advancedButton.action = #selector(toggleAdvanced)
        advancedButton.setAccessibilityLabel("展开或收起高级选项")
        place(advancedButton, in: settingsForm, 16, 556, 18, 18)
        label("高级选项", in: settingsForm, 42, 554, 500)
        place(advancedSection, in: settingsForm, 16, 592, 550, 170)
        label("网关地址", in: advancedSection, 0, 0, 550)
        configureField(gateway, "网关 IPv4 地址", "10.200.21.4"); gateway.delegate = self
        place(gateway, in: advancedSection, 0, 26, 250, 28)
        label("通常无需修改", in: advancedSection, 266, 31, 284, size: 12, secondary: true)
        let wifiButton = NSButton(title: "Wi-Fi 自动连接…", target: self, action: #selector(showWiFiOptions))
        wifiButton.bezelStyle = .rounded; wifiButton.image = NSImage(systemSymbolName: "wifi", accessibilityDescription: nil)
        wifiButton.imagePosition = .imageLeading
        place(wifiButton, in: advancedSection, -4, 86, 212, 32)
        label("按网卡设置 BUAA-WiFi 自动连接", in: advancedSection, 0, 128, 550, size: 12, secondary: true)
        advancedSection.isHidden = true
        settingsHint.font = .systemFont(ofSize: 12); settingsHint.textColor = .secondaryLabelColor
        place(settingsHint, in: settings, 0, 454, 388, 38)
        place(settingsStart, in: settings, 424, 456, 192, 34)
        settingsStart.target = self; settingsStart.action = #selector(startMonitor)
        start.target = self; start.action = #selector(dashboardAction)
        stop.target = self; stop.action = #selector(stopMonitor); stop.isEnabled = false
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
            interval.stringValue = settings["interval"] as? String ?? "300"
            gateway.stringValue = settings["gateway"] as? String ?? "10.200.21.4"
            if let binding = settings["interface"] as? String {
                interfaces.addItem(withTitle: "指定接口 · \(binding)"); selectionBindings.append(binding)
                interfaces.selectItem(at: 1)
            }
        }
        wifiEnabled = Set(UserDefaults.standard.stringArray(forKey: settingsKey + ".wifi") ?? [])
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
        refreshStartup(); updateStartTitle(); renderHistory()
        if loginLaunch {
            if remember.state == .on && completeConfiguration && store != nil && source != nil {
                NSApp.setActivationPolicy(.accessory); startMonitor()
                if monitoring { hideWindow() } else { showWindow() }
            } else {
                showWindow(); showPage(1); record("开机自启：请保存完整账号、密码和连接配置后启用自动连接。")
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
    @objc private func showWiFiOptions() {
        if let panel = wifiPanel { panel.makeKeyAndOrderFront(nil); return }
        let names = wifi.interfaceNames()
        let height = CGFloat(min(460, max(1, names.count) * 38 + 166))
        let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 520, height: height),
            styleMask: [.titled], backing: .buffered, defer: false)
        panel.title = "Wi-Fi 自动连接"; panel.isReleasedWhenClosed = false
        let content = FlippedContent(frame: NSRect(x: 0, y: 0, width: 520, height: height)); panel.contentView = content
        let title = NSTextField(labelWithString: "自动连接 BUAA-WiFi")
        title.font = .systemFont(ofSize: 16, weight: .semibold)
        title.frame = NSRect(x: 24, y: 20, width: 472, height: 24); content.addSubview(title)
        let label = NSTextField(wrappingLabelWithString: "开启后，在自动重连运行期间发现 BUAA-WiFi 时，会自动切换到该无线网络。")
        label.font = .systemFont(ofSize: 12); label.textColor = .secondaryLabelColor
        label.frame = NSRect(x: 24, y: 52, width: 472, height: 34); content.addSubview(label)
        wifiRows = []
        let list = FlippedContent(frame: NSRect(x: 0, y: 0, width: 464, height: CGFloat(max(1, names.count) * 38)))
        if names.isEmpty {
            let empty = NSTextField(labelWithString: "未发现 Wi-Fi 网卡")
            empty.textColor = .secondaryLabelColor; empty.frame = NSRect(x: 0, y: 6, width: 460, height: 24); list.addSubview(empty)
        }
        for (index, name) in names.enumerated() {
            let row = NSButton(checkboxWithTitle: "Wi-Fi（\(name)）", target: self, action: #selector(wifiOptionChanged(_:)))
            row.identifier = NSUserInterfaceItemIdentifier(name); row.state = wifiEnabled.contains(name) ? .on : .off
            row.frame = NSRect(x: 0, y: CGFloat(index*38), width: 460, height: 28)
            list.addSubview(row); wifiRows.append(row)
        }
        let scroll = NSScrollView(frame: NSRect(x: 24, y: 98, width: 472, height: height-154))
        scroll.hasVerticalScroller = true; scroll.drawsBackground = false; scroll.documentView = list; content.addSubview(scroll)
        let done = NSButton(title: "完成", target: self, action: #selector(closeWiFiOptions))
        done.bezelStyle = .rounded; done.keyEquivalent = "\r"
        done.frame = NSRect(x: 398, y: height-46, width: 100, height: 30); content.addSubview(done)
        wifiPanel = panel; window.beginSheet(panel)
    }
    @objc private func closeWiFiOptions() {
        if let panel = wifiPanel { window.endSheet(panel); panel.orderOut(nil) }; wifiPanel = nil
    }
    @objc private func selectPage() {
        showPage(navigation.selectedSegment)
    }
    private func showPage(_ index: Int) {
        navigation.selectedSegment = index; pages.selectTabViewItem(at: index)
        window.makeFirstResponder(index == 1 && username.isEnabled ? username : navigation)
        start.keyEquivalent = index == 0 ? "\r" : ""
        settingsStart.keyEquivalent = index == 1 ? "\r" : ""
        if index == 0 { renderHistory() }
    }
    private var completeConfiguration: Bool {
        var address = in_addr()
        return !username.stringValue.trimmingCharacters(in: .whitespaces).isEmpty && !password.stringValue.isEmpty
            && UInt64(interval.stringValue).map({ (10...86400).contains($0) }) == true
            && gateway.stringValue.trimmingCharacters(in: .whitespaces).withCString({ inet_pton(AF_INET, $0, &address) }) == 1
    }
    private func saveConnectionPreferences() {
        var settings: [String: Any] = ["interval": interval.stringValue, "gateway": gateway.stringValue.trimmingCharacters(in: .whitespaces)]
        if let binding = selectionBindings[interfaces.indexOfSelectedItem] { settings["interface"] = binding }
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
        if username.stringValue.trimmingCharacters(in: .whitespaces).isEmpty || password.stringValue.isEmpty { focusSetting(username); return }
        startMonitor()
    }
    func controlTextDidChange(_ notification: Notification) { updateStartTitle() }
    func controlTextDidBeginEditing(_ notification: Notification) {
        if let field = notification.object as? NSTextField {
            settingsForm.scrollToVisible(settingsForm.convert(field.bounds, from: field).insetBy(dx: 0, dy: -12))
        }
    }
    private func updateStartTitle() {
        start.title = monitoring || (!username.stringValue.trimmingCharacters(in: .whitespaces).isEmpty && !password.stringValue.isEmpty) ? "开始自动重连" : "设置账号"
        dashboardValues[4].stringValue = "\(interval.stringValue) 秒"
    }
    @objc private func toggleAdvanced() {
        let expanded = advancedButton.state == .on
        if !expanded { window.makeFirstResponder(advancedButton) }
        advancedSection.isHidden = !expanded
        settingsForm.setFrameSize(NSSize(width: 596, height: expanded ? 782 : 582))
    }
    private func focusSetting(_ field: NSTextField) {
        showPage(1)
        if field === gateway { advancedButton.state = .on; toggleAdvanced() }
        settingsForm.scrollToVisible(settingsForm.convert(field.bounds, from: field).insetBy(dx: 0, dy: -24))
        window.makeFirstResponder(field); field.selectText(nil)
    }
    @objc private func toggleHistory() {
        historyExpanded.toggle(); historyButton.title = historyExpanded ? "收起完整记录" : "查看完整记录"
        renderHistory()
    }
    private func renderHistory() {
        let visible = historyExpanded ? lines : Array(lines.suffix(5))
        history.string = visible.isEmpty ? "暂无活动记录" : visible.joined(separator: "\n")
        history.scrollToEndOfDocument(nil)
    }
    private func localTime(_ value: Any?) -> String {
        guard let timestamp = value as? UInt64 else { return "—" }
        let formatter = DateFormatter(); formatter.dateFormat = "HH:mm:ss"
        return formatter.string(from: Date(timeIntervalSince1970: TimeInterval(timestamp)))
    }
    private func applyDashboard(_ event: [String: Any]) {
        guard let snapshot = event["dashboard"] as? [String: Any] else { return }
        updateState(snapshot["phase"] as? String ?? "idle")
        dashboardValues[0].stringValue = snapshot["interface"] as? String ?? "—"
        dashboardValues[1].stringValue = snapshot["local_ip"] as? String ?? "—"
        dashboardValues[2].stringValue = snapshot["campus_ip"] as? String ?? "—"
        dashboardValues[3].stringValue = localTime(snapshot["last_check_at"])
        dashboardValues[4].stringValue = "\(snapshot["interval"] as? UInt64 ?? 300) 秒"
        dashboardValues[5].stringValue = localTime(snapshot["retry_at"])
        let message = event["message"] as? String ?? ""
        status.stringValue = message; trayStatus.title = message; item.button?.toolTip = message
    }
    private func updateState(_ state: String) {
        let title: String; let symbol: String; let color: NSColor
        switch state {
        case "online": title = "校园网在线"; symbol = "checkmark.circle.fill"; color = .systemGreen
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
    @objc private func wifiOptionChanged(_ sender: NSButton) {
        guard let name = sender.identifier?.rawValue else { return }
        if sender.state == .on { wifiEnabled.insert(name) } else { wifiEnabled.remove(name) }
        UserDefaults.standard.set(Array(wifiEnabled).sorted(), forKey: settingsKey + ".wifi")
        wifi.configure(enabled: wifiEnabled, active: monitoring)
    }
    private func alert(_ message: String) { let a = NSAlert(); a.messageText = "北航校园网"; a.informativeText = message; a.runModal() }
    private func reloadCredentials() {
        do { if let credentials = try loadCredentials() { username.stringValue = credentials.0; password.stringValue = credentials.1; remember.state = .on } }
        catch { record("原生版凭据读取失败，请重新输入") }
        updateStartTitle()
    }
    @objc private func rememberChanged() {
        if remember.state == .off { do { try forgetCredentials() } catch { remember.state = .on; alert("无法删除钥匙串凭据") } }
    }
    private func createClient(_ scan: Bool) -> Bool {
        guard client == nil else { return false }
        guard let seconds = UInt64(interval.stringValue), (10...86400).contains(seconds) else { focusSetting(interval); alert("检查间隔须在 10–86400 秒之间"); return false }
        if !scan && (username.stringValue.trimmingCharacters(in: .whitespaces).isEmpty || password.stringValue.isEmpty) { focusSetting(username.stringValue.trimmingCharacters(in: .whitespaces).isEmpty ? username : password); alert("请输入账号和密码"); return false }
        var address = in_addr()
        guard gateway.stringValue.trimmingCharacters(in: .whitespaces).withCString({ inet_pton(AF_INET, $0, &address) }) == 1 else {
            focusSetting(gateway); alert("网关必须是 IPv4 地址"); return false
        }
        let binding = selectionBindings[interfaces.indexOfSelectedItem]
        var config: [String: Any] = ["username": scan ? "" : username.stringValue.trimmingCharacters(in: .whitespaces), "password": scan ? "" : password.stringValue,
            "gateway_ip": gateway.stringValue.trimmingCharacters(in: .whitespaces), "interval": seconds, "external_timer": true]
        if let binding = binding, !scan { config["interface"] = binding }
        guard let data = try? JSONSerialization.data(withJSONObject: config), let json = String(data: data, encoding: .utf8) else { return false }
        epoch &+= 1; callbackBox = CallbackContext(self, epoch)
        client = json.withCString { buaa_create($0, coreCallback, Unmanaged.passUnretained(callbackBox!).toOpaque()) }
        guard client != nil else { callbackBox = nil; focusSetting(gateway); alert("配置无效，请检查网关 IPv4 和间隔"); return false }
        return true
    }
    private func setBusy(_ busy: Bool) {
        [username, password, interval, gateway].forEach { $0.isEnabled = !busy }
        interfaces.isEnabled = !busy; remember.isEnabled = !busy; start.isEnabled = !busy; detect.isEnabled = !busy
        stop.isEnabled = busy && !scanning; settingsStart.isEnabled = !busy
        settingsHint.stringValue = busy ? "修改账号或连接配置，请先到仪表盘停止。" : "开始时保存并应用设置。"
        updateStartTitle()
    }
    @objc private func scan() {
        wifi.requestScan()
        guard createClient(true) else { return }
        scanning = true; setBusy(true); updateState("checking"); record("正在检测校园网接口…")
        if buaa_scan(client) != 0 { releaseClient(); setBusy(false); record("无法启动接口检测") }
    }
    @objc private func startMonitor() {
        guard store != nil && source != nil else { alert("无法订阅网络变化通知，请重启程序后再试"); return }
        guard createClient(false) else { return }
        do {
            if remember.state == .on { try saveCredentials(username.stringValue.trimmingCharacters(in: .whitespaces), password.stringValue) }
            else { try forgetCredentials() }
        } catch { releaseClient(); alert("无法更新钥匙串凭据"); return }
        saveConnectionPreferences()
        scanning = false; setBusy(true)
        if buaa_start(client) != 0 { releaseClient(); setBusy(false); alert("无法启动监控"); return }
        monitoring = true; wifi.configure(enabled: wifiEnabled, active: true)
        password.stringValue = ""; showPage(0); updateState("checking"); record("正在检查校园网…")
    }
    @objc private func stopMonitor() { monitoring = false; wifi.configure(enabled: wifiEnabled, active: false); cancelTimer(); if let c = client { buaa_stop(c); stop.isEnabled = false; record("正在停止…") } }
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
            let old = selectionBindings[interfaces.indexOfSelectedItem]
            interfaces.removeAllItems(); interfaces.addItem(withTitle: "自动选择"); selectionBindings = [nil]
            for row in records {
                let label = row["label"] as? String ?? ""; let message = row["message"] as? String ?? ""
                interfaces.addItem(withTitle: "\(label) · \(message)"); selectionBindings.append(row["binding"] as? String)
            }
            if let index = selectionBindings.firstIndex(where: { $0 == old }) { interfaces.selectItem(at: index) }
            else if let old = old {
                interfaces.addItem(withTitle: "指定接口 · \(old)（暂不可用）"); selectionBindings.append(old)
                interfaces.selectItem(at: selectionBindings.count - 1)
            }
        }
        if state == "stopped" {
            monitoring = false; wifi.configure(enabled: wifiEnabled, active: false)
            releaseClient(); setBusy(false); reloadCredentials()
            if closing { NSApp.terminate(nil); return }
            if scanning { scanning = false; return }
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
        lines.append("\(formatter.string(from: Date()))  \(message)"); if lines.count > 120 { lines.removeFirst(lines.count-120) }
        if navigation.selectedSegment == 0 { renderHistory() }
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
