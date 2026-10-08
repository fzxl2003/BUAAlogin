"""Native menu bar item and Keychain storage; imported only by macOS desktop."""
import AppKit
import Foundation
import Security

SERVICE = 'cn.buaa.autologin.credentials'


def _query():
    return {Security.kSecClass: Security.kSecClassGenericPassword,
            Security.kSecAttrService: SERVICE}


def load_credentials():
    query = _query()
    query.update({Security.kSecReturnAttributes: True, Security.kSecReturnData: True,
                  Security.kSecMatchLimit: Security.kSecMatchLimitOne})
    status, result = Security.SecItemCopyMatching(query, None)
    if status == Security.errSecItemNotFound:
        return None
    if status:
        raise RuntimeError('无法读取钥匙串凭据（状态 %s）' % status)
    return str(result[Security.kSecAttrAccount]), bytes(result[Security.kSecValueData]).decode('utf-8')


def save_credentials(username, password):
    encoded = password.encode('utf-8')
    data = Foundation.NSData.dataWithBytes_length_(encoded, len(encoded))
    values = {Security.kSecAttrAccount: username, Security.kSecValueData: data}
    status = Security.SecItemUpdate(_query(), values)
    if status == Security.errSecItemNotFound:
        query = _query()
        query.update(values)
        status, _ = Security.SecItemAdd(query, None)
    if status:
        raise RuntimeError('无法保存钥匙串凭据（状态 %s）' % status)


def forget_credentials():
    status = Security.SecItemDelete(_query())
    if status not in (0, Security.errSecItemNotFound):
        raise RuntimeError('无法删除钥匙串凭据（状态 %s）' % status)


class MenuActions(Foundation.NSObject):
    def show_(self, sender):
        self.show_callback()

    def quit_(self, sender):
        self.quit_callback()


class MenuBar:
    def __init__(self, show, quit):
        self.target = MenuActions.alloc().init()
        self.target.show_callback, self.target.quit_callback = show, quit
        self.item = AppKit.NSStatusBar.systemStatusBar().statusItemWithLength_(AppKit.NSVariableStatusItemLength)
        self.item.button().setTitle_('北航')
        self.item.button().setToolTip_('北航校园网 · 未启动')
        menu = AppKit.NSMenu.alloc().init()
        self.status = AppKit.NSMenuItem.alloc().initWithTitle_action_keyEquivalent_('未启动', None, '')
        self.status.setEnabled_(False)
        menu.addItem_(self.status)
        menu.addItem_(AppKit.NSMenuItem.separatorItem())
        for title, action in [('显示窗口', 'show:'), ('退出', 'quit:')]:
            item = AppKit.NSMenuItem.alloc().initWithTitle_action_keyEquivalent_(title, action, '')
            item.setTarget_(self.target)
            menu.addItem_(item)
        self.item.setMenu_(menu)

    def update(self, message):
        self.status.setTitle_(message)
        self.item.button().setToolTip_('北航校园网 · ' + message)

    def hide(self):
        AppKit.NSApplication.sharedApplication().setActivationPolicy_(AppKit.NSApplicationActivationPolicyAccessory)

    def show(self):
        app = AppKit.NSApplication.sharedApplication()
        app.setActivationPolicy_(AppKit.NSApplicationActivationPolicyRegular)
        app.activateIgnoringOtherApps_(True)

    def close(self):
        AppKit.NSStatusBar.systemStatusBar().removeStatusItem_(self.item)
