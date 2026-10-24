import Foundation
import CoreWLAN
import CoreLocation

// No periodic RF scan. Scan once on start/wake/enable/disconnect, then consume
// system scan-cache updates. The portal password is never a Wi-Fi password.
final class CampusWiFi: NSObject, CWEventDelegate, CLLocationManagerDelegate {
    private let client = CWWiFiClient.shared()
    private let location = CLLocationManager()
    private let lock = NSLock()
    private let queue = DispatchQueue(label: "cn.buaa.autologin.wifi", qos: .utility)
    private var networks: [String: Set<String>] = [:]
    private var active = false
    private var sleeping = false
    private var busy = Set<String>()
    private var lastAttempt: [String: TimeInterval] = [:]
    private var observing = false
    private var epoch: UInt64 = 0
    var report: ((String) -> Void)?

    override init() { super.init(); client.delegate = self; location.delegate = self }
    func interfaceNames() -> [String] { (client.interfaces() ?? []).compactMap { $0.interfaceName }.sorted() }
    func configure(networks: [String: Set<String>], active: Bool) {
        lock.lock(); self.networks = networks.filter { !$0.value.isEmpty }; self.active = active; epoch &+= 1; lock.unlock()
        if !active || networks.values.allSatisfy({ $0.isEmpty }) { stopObserving(); return }
        authorizeAndObserve()
    }
    func suspend() { lock.lock(); sleeping = true; epoch &+= 1; lock.unlock() }
    func resume() { lock.lock(); sleeping = false; epoch &+= 1; lock.unlock(); requestScan() }
    private func authorizeAndObserve() {
        switch location.authorizationStatus {
        case .notDetermined: location.requestWhenInUseAuthorization()
        case .authorized, .authorizedAlways, .authorizedWhenInUse:
            if !observing {
                do {
                    for event in [CWEventType.scanCacheUpdated, .ssidDidChange, .linkDidChange, .powerDidChange] {
                        try client.startMonitoringEvent(with: event)
                    }
                    observing = true
                } catch { stopObserving(); emit("无法订阅 Wi-Fi 变化通知"); return }
            }
            requestScan()
        default: emit("Wi-Fi 自动连接需要系统定位权限，请在系统设置中允许")
        }
    }
    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        lock.lock(); let needed = active && !networks.isEmpty; lock.unlock()
        if needed { authorizeAndObserve() }
    }
    func requestScan() {
        lock.lock(); let names = active && !sleeping ? Array(networks.keys) : []; lock.unlock()
        for name in names { inspect(name, scan: true) }
    }
    private func inspect(_ name: String, scan: Bool) {
        lock.lock()
        guard active, !sleeping, networks[name]?.isEmpty == false, !busy.contains(name) else { lock.unlock(); return }
        let targets = networks[name] ?? []
        let token = epoch
        busy.insert(name); lock.unlock()
        queue.async { [weak self] in
            guard let self = self else { return }
            defer { self.lock.lock(); self.busy.remove(name); self.lock.unlock() }
            guard self.allowed(name, token), let interface = self.client.interface(withName: name), interface.powerOn() else { return }
            if interface.ssid().map(targets.contains) == true { return }
            do {
                let available: Set<CWNetwork>
                if scan {
                    var found = Set<CWNetwork>()
                    for ssid in targets { found.formUnion(try interface.scanForNetworks(withSSID: Data(ssid.utf8))) }
                    available = found
                } else { available = interface.cachedScanResults() ?? [] }
                guard let network = available.filter({ $0.ssid.map(targets.contains) == true }).max(by: { $0.rssiValue < $1.rssiValue }),
                      let ssid = network.ssid,
                      self.allowed(name, token) else { return }
                guard network.supportsSecurity(.none) else {
                    self.emit("\(name)：\(ssid) 需要无线凭据，请先在系统中连接；校园网密码不会用于 Wi-Fi")
                    return
                }
                self.lock.lock()
                let now = ProcessInfo.processInfo.systemUptime
                let cooledDown = self.lastAttempt[name].map { now - $0 >= 30 } ?? true
                if cooledDown { self.lastAttempt[name] = now }
                self.lock.unlock()
                guard cooledDown, self.allowed(name, token) else { return }
                self.emit("\(name)：发现 \(ssid)，正在自动连接")
                try interface.associate(to: network, password: nil)
                if self.allowed(name, token) { self.emit("\(name)：已连接 \(ssid)，等待获取 IP") }
            } catch {
                if self.allowed(name, token) { self.emit("\(name)：Wi-Fi 扫描或连接失败，请检查无线开关和定位权限") }
            }
        }
    }
    private func allowed(_ name: String, _ token: UInt64) -> Bool {
        lock.lock(); defer { lock.unlock() }; return active && !sleeping && networks[name]?.isEmpty == false && epoch == token
    }
    private func emit(_ message: String) { DispatchQueue.main.async { [weak self] in self?.report?(message) } }
    private func stopObserving() { try? client.stopMonitoringAllEvents(); observing = false }
    func scanCacheUpdatedForWiFiInterface(withName name: String) { inspect(name, scan: false) }
    func ssidDidChangeForWiFiInterface(withName name: String) { inspect(name, scan: true) }
    func linkDidChangeForWiFiInterface(withName name: String) { inspect(name, scan: true) }
    func powerStateDidChangeForWiFiInterface(withName name: String) { inspect(name, scan: true) }
    func clientConnectionInterrupted() { DispatchQueue.main.async { [weak self] in self?.requestScan() } }
    func clientConnectionInvalidated() { emit("Wi-Fi 系统连接中断，请重新启动程序") }
    func close() { configure(networks: [:], active: false); location.delegate = nil; client.delegate = nil }
}
