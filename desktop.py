"""Native macOS desktop with event-driven AppKit UI and Keychain credentials."""
import threading
import time

import AppKit as A
import Foundation as F
import objc

from BUAASrunLogin import mac_native
from BUAASrunLogin.interfaces import InterfaceLoginManager, check_interfaces
from BUAASrunLogin.monitor import ConnectionMonitor

AUTO = '自动选择（依次检测接口）'
WIDTH, HEIGHT = 700, 550


def rect(x, top, width, height):
    return F.NSMakeRect(x, HEIGHT - top - height, width, height)


class DesktopController(F.NSObject):
    def init(self):
        self = objc.super(DesktopController, self).init()
        if self is None:
            return None
        self.monitor = None
        self.closing = self.closed = False
        self.bindings = {AUTO: None}
        self.build()
        self.tray = mac_native.MenuBar(self.show_window, self.request_quit)
        try:
            credentials = mac_native.load_credentials()
            if credentials:
                self.inputs['username'].setStringValue_(credentials[0])
                self.inputs['password'].setStringValue_(credentials[1])
                self.remember.setState_(A.NSControlStateValueOn)
        except RuntimeError as exc:
            self.status.setStringValue_(str(exc))
        self.detect_(None)
        return self

    @objc.python_method
    def label(self, text, x, top, width, height=24, size=13):
        label = A.NSTextField.alloc().initWithFrame_(rect(x, top, width, height))
        label.setStringValue_(text)
        label.setBezeled_(False)
        label.setDrawsBackground_(False)
        label.setEditable_(False)
        label.setSelectable_(False)
        label.setFont_(A.NSFont.systemFontOfSize_(size))
        self.content.addSubview_(label)
        return label

    @objc.python_method
    def button(self, title, x, top, width, action):
        button = A.NSButton.alloc().initWithFrame_(rect(x, top, width, 32))
        button.setTitle_(title)
        button.setBezelStyle_(A.NSBezelStyleRounded)
        button.setTarget_(self)
        button.setAction_(action)
        self.content.addSubview_(button)
        return button

    @objc.python_method
    def build(self):
        style = A.NSWindowStyleMaskTitled | A.NSWindowStyleMaskClosable | A.NSWindowStyleMaskMiniaturizable
        self.window = A.NSWindow.alloc().initWithContentRect_styleMask_backing_defer_(
            F.NSMakeRect(0, 0, WIDTH, HEIGHT), style, A.NSBackingStoreBuffered, False)
        self.window.setTitle_('北航校园网 · 自动重连')
        self.window.center()
        self.window.setDelegate_(self)
        self.window.setReleasedWhenClosed_(False)
        self.content = self.window.contentView()
        title = self.label('北航校园网', 24, 20, 650, 34, 24)
        title.setFont_(A.NSFont.boldSystemFontOfSize_(24))
        subtitle = self.label('定期检查，掉线自动登录', 24, 57, 650)
        subtitle.setTextColor_(A.NSColor.secondaryLabelColor())
        self.inputs = {}
        for key, title, default, top in [
            ('username', '账号', '', 94), ('password', '密码', '', 138),
            ('interval', '检查间隔（秒）', '300', 182),
            ('interface', '校园网接口', AUTO, 226),
            ('gateway', '网关 IPv4', '10.200.21.4', 270)]:
            self.label(title, 24, top + 3, 155)
            if key == 'interface':
                entry = A.NSPopUpButton.alloc().initWithFrame_pullsDown_(rect(182, top, 358, 29), False)
                entry.addItemWithTitle_(AUTO)
            else:
                cls = A.NSSecureTextField if key == 'password' else A.NSTextField
                entry = cls.alloc().initWithFrame_(rect(182, top, 488, 29))
                entry.setStringValue_(default)
            self.inputs[key] = entry
            self.content.addSubview_(entry)
        self.detect_button = self.button('检测校园网', 552, 225, 120, 'detect:')
        self.remember = self.button('记住账号密码（系统钥匙串）', 24, 311, 440, 'remember:')
        self.remember.setButtonType_(A.NSButtonTypeSwitch)
        self.start_button = self.button('开始自动重连', 24, 352, 170, 'start:')
        self.stop_button = self.button('停止', 203, 352, 90, 'stop:')
        self.stop_button.setEnabled_(False)
        self.button('最小化至托盘', 305, 352, 170, 'hide:')
        self.status = self.label('未启动', 24, 397, 650, 34)
        self.status.setLineBreakMode_(A.NSLineBreakByTruncatingTail)
        scroll = A.NSScrollView.alloc().initWithFrame_(rect(24, 439, 650, 88))
        scroll.setHasVerticalScroller_(True)
        scroll.setBorderType_(A.NSBezelBorder)
        self.history = A.NSTextView.alloc().initWithFrame_(F.NSMakeRect(0, 0, 630, 88))
        self.history.setEditable_(False)
        self.history.setFont_(A.NSFont.systemFontOfSize_(12))
        scroll.setDocumentView_(self.history)
        self.content.addSubview_(scroll)
        # A native application menu also makes Cmd-Q follow the graceful stop path.
        menu = A.NSMenu.alloc().init()
        app_item = A.NSMenuItem.alloc().init()
        submenu = A.NSMenu.alloc().initWithTitle_('BUAALogin')
        quit_item = A.NSMenuItem.alloc().initWithTitle_action_keyEquivalent_('退出北航校园网', 'quit:', 'q')
        quit_item.setTarget_(self)
        submenu.addItem_(quit_item)
        app_item.setSubmenu_(submenu)
        menu.addItem_(app_item)
        edit_item = A.NSMenuItem.alloc().init()
        edit_menu = A.NSMenu.alloc().initWithTitle_('编辑')
        for title, action, key in [('剪切', 'cut:', 'x'), ('复制', 'copy:', 'c'),
                                   ('粘贴', 'paste:', 'v'), ('全选', 'selectAll:', 'a')]:
            edit_menu.addItem_(A.NSMenuItem.alloc().initWithTitle_action_keyEquivalent_(title, action, key))
        edit_item.setSubmenu_(edit_menu)
        menu.addItem_(edit_item)
        A.NSApplication.sharedApplication().setMainMenu_(menu)

    @objc.python_method
    def alert(self, message):
        alert = A.NSAlert.alloc().init()
        alert.setMessageText_('北航校园网')
        alert.setInformativeText_(message)
        alert.runModal()

    def remember_(self, sender):
        if self.remember.state() == A.NSControlStateValueOff:
            try:
                mac_native.forget_credentials()
            except RuntimeError as exc:
                self.remember.setState_(A.NSControlStateValueOn)
                self.alert(str(exc))

    def detect_(self, sender):
        self.detect_button.setEnabled_(False)
        gateway = str(self.inputs['gateway'].stringValue()).strip()
        self.status.setStringValue_('正在依次检测联网接口…')
        def scan():
            try:
                self.publish({'state': 'interfaces', 'records': check_interfaces(gateway), 'message': '接口检测完成'})
            except Exception as exc:
                self.publish({'state': 'interfaces', 'records': [], 'message': '接口检测失败：' + type(exc).__name__})
        threading.Thread(target=scan, daemon=True, name='interface-scan').start()

    def start_(self, sender):
        if self.monitor is not None:
            return
        manager = None
        try:
            interval = int(self.inputs['interval'].stringValue())
            if not 30 <= interval <= 86400:
                raise ValueError('检查间隔须在 30–86400 秒之间')
            username = str(self.inputs['username'].stringValue()).strip()
            password = str(self.inputs['password'].stringValue())
            if not username or not password:
                raise ValueError('账号和密码不能为空')
            selection = str(self.inputs['interface'].titleOfSelectedItem())
            manager = InterfaceLoginManager(interface=self.bindings.get(selection),
                                             gateway_ip=str(self.inputs['gateway'].stringValue()).strip(), on_event=self.publish)
            if self.remember.state() == A.NSControlStateValueOn:
                mac_native.save_credentials(username, password)
            self.monitor = ConnectionMonitor(username, password, interval=interval,
                                             manager=manager, on_event=self.publish)
        except (ValueError, RuntimeError) as exc:
            if manager:
                manager.close()
            self.alert(str(exc))
            return
        self.inputs['password'].setStringValue_('')
        for entry in self.inputs.values():
            entry.setEnabled_(False)
        self.remember.setEnabled_(False)
        self.start_button.setEnabled_(False)
        self.stop_button.setEnabled_(True)
        threading.Thread(target=self.monitor.run, daemon=True, name='campus-monitor').start()

    @objc.python_method
    def publish(self, event):
        if not self.closed:
            # Cocoa dispatches exactly one main-thread callback for a new event.
            F.NSOperationQueue.mainQueue().addOperationWithBlock_(lambda: self.process_event(event))

    @objc.python_method
    def process_event(self, event):
        if self.closed:
            return
        if event['state'] == 'interfaces':
            box = self.inputs['interface']
            previous = str(box.titleOfSelectedItem())
            self.bindings = {AUTO: None}
            for item in event['records']:
                self.bindings[item['label'] + ' · ' + item['message']] = item['binding']
            box.removeAllItems()
            box.addItemsWithTitles_(list(self.bindings))
            box.selectItemWithTitle_(previous if previous in self.bindings else AUTO)
            self.detect_button.setEnabled_(True)
            self.record(event['message'] + '：' + '；'.join(item['label'] + ' ' + item['message'] for item in event['records']))
            return
        message = event['message']
        if event.get('next_check'):
            message += ' · 下次检查 ' + time.strftime('%H:%M:%S', time.localtime(event['next_check']))
        self.status.setStringValue_(message)
        self.tray.update(message)
        if event['state'] != 'checking':
            self.record(message)
        if event['state'] == 'stopped':
            self.monitor = None
            if self.closing:
                self.finish_quit()
                return
            for entry in self.inputs.values():
                entry.setEnabled_(True)
            self.remember.setEnabled_(True)
            self.start_button.setEnabled_(True)
            self.stop_button.setEnabled_(False)
            if self.remember.state() == A.NSControlStateValueOn:
                try:
                    credentials = mac_native.load_credentials()
                    if credentials:
                        self.inputs['password'].setStringValue_(credentials[1])
                except RuntimeError:
                    pass

    @objc.python_method
    def record(self, message):
        self.status.setStringValue_(message)
        lines = str(self.history.string()).splitlines()
        lines.append(time.strftime('%H:%M:%S') + '  ' + message)
        self.history.setString_('\n'.join(lines[-120:]) + '\n')

    def stop_(self, sender):
        if self.monitor:
            self.monitor.stop()
            self.stop_button.setEnabled_(False)
            self.status.setStringValue_('正在停止，等待当前请求结束…')

    def hide_(self, sender):
        self.window.orderOut_(None)
        self.tray.hide()

    @objc.python_method
    def show_window(self):
        self.tray.show()
        self.window.deminiaturize_(None)
        self.window.makeKeyAndOrderFront_(None)

    @objc.python_method
    def request_quit(self):
        self.quit_(None)

    def quit_(self, sender):
        if self.monitor:
            self.closing = True
            self.show_window()
            self.stop_(None)
        else:
            self.finish_quit()

    @objc.python_method
    def finish_quit(self):
        self.closed = True
        self.tray.close()
        A.NSApplication.sharedApplication().terminate_(None)

    def windowShouldClose_(self, window):
        self.hide_(None)
        return False

    def windowDidMiniaturize_(self, notification):
        self.hide_(None)

    def applicationShouldTerminate_(self, application):
        if self.closed:
            return A.NSTerminateNow
        self.quit_(None)
        return A.NSTerminateCancel


def main():
    app = A.NSApplication.sharedApplication()
    app.setActivationPolicy_(A.NSApplicationActivationPolicyRegular)
    controller = DesktopController.alloc().init()
    app.setDelegate_(controller)
    controller.show_window()
    app.run()


if __name__ == '__main__':
    main()
