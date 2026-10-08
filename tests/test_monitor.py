import threading
import time
import unittest
from unittest.mock import Mock

from BUAASrunLogin.LoginManager import LoginError, LoginManager
from BUAASrunLogin.monitor import ConnectionMonitor


class FakeWait:
    def __init__(self, cycles):
        self.cycles, self.delays, self.stopped = cycles, [], False
    def is_set(self):
        return self.stopped
    def set(self):
        self.stopped = True
    def wait(self, delay):
        self.delays.append(delay)
        self.stopped = len(self.delays) >= self.cycles
        return self.stopped


class MonitorTests(unittest.TestCase):
    def test_reconnect_backoff_resets_after_success(self):
        manager = Mock()
        manager.login.side_effect = [LoginError('offline'), LoginError('offline'),
                                    {'ip': '10.1.2.3', 'result': 'login_ok'},
                                    LoginError('offline'), {'ip': '10.1.2.3', 'result': 'already_online'}]
        stop = FakeWait(5)
        events = []
        monitor = ConnectionMonitor('account', 'password', manager=manager,
                                    stop_event=stop, on_event=events.append)
        monitor.run()
        self.assertEqual(stop.delays, [30, 60, 300, 30, 300])
        self.assertEqual([e['state'] for e in events if e['state'] != 'checking'],
                         ['error', 'error', 'online', 'error', 'online', 'stopped'])
        manager.close.assert_called_once()
        self.assertIsNone(monitor.password)

    def test_retries_are_capped(self):
        manager = Mock()
        manager.login.side_effect = LoginError('offline')
        stop = FakeWait(8)
        ConnectionMonitor('account', 'password', manager=manager, stop_event=stop).run()
        self.assertEqual(stop.delays, [30, 60, 120, 240, 300, 300, 300, 300])

    def test_stop_interrupts_long_sleep(self):
        manager = Mock()
        manager.login.return_value = {'ip': '10.1.2.3', 'result': 'already_online'}
        checked = threading.Event()
        monitor = ConnectionMonitor('account', 'password', interval=86400, manager=manager,
                                    on_event=lambda e: checked.set() if e['state'] == 'online' else None)
        worker = threading.Thread(target=monitor.run)
        worker.start()
        self.assertTrue(checked.wait(1))
        started = time.monotonic()
        monitor.stop()
        worker.join(1)
        self.assertFalse(worker.is_alive())
        self.assertLess(time.monotonic() - started, 1)
        manager.login.assert_called_once()

    def test_error_events_do_not_leak_credentials(self):
        manager = Mock()
        manager.login.side_effect = LoginError('failed account:password')
        events = []
        ConnectionMonitor('account', 'password', manager=manager,
                          stop_event=FakeWait(1), on_event=events.append).run()
        self.assertNotIn('account', str(events))
        self.assertNotIn('password', str(events))

    def test_online_check_uses_only_one_status_request(self):
        manager = LoginManager()
        self.addCleanup(manager.close)
        manager.ip = '10.1.2.3'
        manager.status = Mock(return_value={'online': True, 'ip': manager.ip})
        manager.get_ip = Mock()
        manager.get_token = Mock()
        self.assertEqual(manager.login('account', 'password')['result'], 'already_online')
        manager.status.assert_called_once()
        manager.get_ip.assert_not_called()
        manager.get_token.assert_not_called()

    def test_stop_prevents_followup_network_requests(self):
        manager = LoginManager()
        self.addCleanup(manager.close)
        manager.stop_event = threading.Event()
        manager.stop_event.set()
        with self.assertRaisesRegex(LoginError, '已停止'):
            manager._request(manager.url_login_page)


if __name__ == '__main__':
    unittest.main()
