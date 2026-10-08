import unittest
from unittest.mock import Mock, patch

from BUAASrunLogin.LoginManager import LoginError, LoginManager
from BUAASrunLogin.interfaces import NetworkInterface, InterfaceLoginManager, discover_interfaces


class InterfaceTests(unittest.TestCase):
    def test_macos_skips_vpn_and_disconnected_interfaces(self):
        output = '''en0: flags=8863<UP>
    inet 10.138.1.2 netmask 0xffff0000
    status: active
utun4: flags=8051<UP>
    inet 10.99.1.2 netmask 0xffff0000
en22: flags=8863<UP>
    inet 172.20.10.2 netmask 0xffffff00
    status: active
en5: flags=8863<UP>
    inet 10.1.2.3 netmask 0xffff0000
    status: inactive
'''
        with patch('BUAASrunLogin.interfaces.sys.platform', 'darwin'), patch('BUAASrunLogin.interfaces._command', return_value=output):
            items = discover_interfaces()
        self.assertEqual([(i.name, i.address) for i in items], [('en0', '10.138.1.2'), ('en22', '172.20.10.2')])

    def test_windows_discovery_accepts_single_adapter_json(self):
        with patch('BUAASrunLogin.interfaces.sys.platform', 'win32'), patch('BUAASrunLogin.interfaces._command', return_value='{"name":"Wi-Fi","address":"10.138.1.2"}'):
            items = discover_interfaces()
            self.assertEqual(items[0].binding, '10.138.1.2')

    def test_linux_discovery(self):
        output = '[{"ifname":"lo","addr_info":[{"family":"inet","local":"127.0.0.1"}]},{"ifname":"eth0","addr_info":[{"family":"inet","local":"10.138.1.2"}]}]'
        with patch('BUAASrunLogin.interfaces.sys.platform', 'linux'), patch('BUAASrunLogin.interfaces._command', return_value=output):
            self.assertEqual(discover_interfaces(), [NetworkInterface('eth0', '10.138.1.2')])

    def test_gateway_ip_must_match_interface_address(self):
        manager = LoginManager(expected_ip='172.20.10.2')
        self.addCleanup(manager.close)
        with patch.object(manager, '_request', return_value=('var CONFIG={ip:"10.138.1.2",acid:"78"}', None)):
            with self.assertRaisesRegex(LoginError, 'IP 不一致'):
                manager.get_ip()

    def test_auto_tries_all_and_remembers_success(self):
        interfaces = [NetworkInterface('eth0', '10.1.2.3'), NetworkInterface('wlan0', '10.1.2.4')]
        first, second = Mock(), Mock()
        first.login.side_effect = LoginError('unreachable')
        second.login.return_value = {'ip': '10.1.2.4', 'result': 'login_ok'}
        manager = InterfaceLoginManager()
        self.addCleanup(manager.close)
        with patch('BUAASrunLogin.interfaces.discover_interfaces', return_value=interfaces), patch('BUAASrunLogin.interfaces.campus_manager', side_effect=[first, second]):
            self.assertEqual(manager.login('account', 'password')['ip'], '10.1.2.4')
            manager.login('account', 'password')
        first.login.assert_called_once()
        self.assertEqual(second.login.call_count, 2)

    def test_explicit_interface_does_not_use_other_uplink(self):
        manager = InterfaceLoginManager(interface='missing')
        self.addCleanup(manager.close)
        with patch('BUAASrunLogin.interfaces.discover_interfaces', return_value=[NetworkInterface('eth0', '10.1.2.3')]):
            with self.assertRaisesRegex(LoginError, '没有可用'):
                manager.login('account', 'password')

    def test_docker_checks_all_interfaces_even_after_a_success(self):
        interfaces = [NetworkInterface('eth0', '10.1.2.3'), NetworkInterface('wlan0', '10.1.2.4')]
        first, second = Mock(), Mock()
        first.login.return_value = {'ip': '10.1.2.3', 'result': 'already_online'}
        second.login.return_value = {'ip': '10.1.2.4', 'result': 'login_ok'}
        manager = InterfaceLoginManager(try_all=True)
        self.addCleanup(manager.close)
        with patch('BUAASrunLogin.interfaces.discover_interfaces', return_value=interfaces) as discover, patch('BUAASrunLogin.interfaces.campus_manager', side_effect=[first, second]):
            result = manager.login('account', 'password')
        discover.assert_called_once_with(include_virtual=True)
        first.login.assert_called_once()
        second.login.assert_called_once()
        self.assertIn('eth0', result['interface'])
        self.assertIn('wlan0', result['interface'])
        self.assertEqual(result['result'], 'login_ok')

    def test_docker_continues_after_an_unreachable_interface(self):
        interfaces = [NetworkInterface('eth0', '10.1.2.3'), NetworkInterface('wlan0', '10.1.2.4')]
        first, second = Mock(), Mock()
        first.login.side_effect = LoginError('unreachable')
        second.login.return_value = {'ip': '10.1.2.4', 'result': 'already_online'}
        manager = InterfaceLoginManager(try_all=True)
        self.addCleanup(manager.close)
        with patch('BUAASrunLogin.interfaces.discover_interfaces', return_value=interfaces), patch('BUAASrunLogin.interfaces.campus_manager', side_effect=[first, second]):
            self.assertEqual(manager.login('account', 'password')['ip'], '10.1.2.4')
        first.login.assert_called_once()
        second.login.assert_called_once()


if __name__ == '__main__':
    unittest.main()
