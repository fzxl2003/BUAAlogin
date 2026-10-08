"""Shared, sleeping reconnect loop for desktop and container releases."""
import math
import threading
import time

from .LoginManager import LoginError, LoginManager


class ConnectionMonitor:
    def __init__(self, username, password, *, interval=300, manager=None,
                 stop_event=None, on_event=None):
        if not username or not password:
            raise ValueError('账号和密码不能为空')
        if not math.isfinite(interval) or interval <= 0:
            raise ValueError('检查间隔必须为正数')
        self.username, self.password = username, password
        self.interval = interval
        self.manager = manager or LoginManager()
        self.stop_event = stop_event or threading.Event()
        self.manager.stop_event = self.stop_event
        self.on_event = on_event or (lambda event: None)

    def stop(self):
        self.stop_event.set()

    def run(self):
        failures = 0
        try:
            while not self.stop_event.is_set():
                self.on_event({'state': 'checking', 'message': '正在检查校园网状态'})
                started = time.monotonic()
                try:
                    result = self.manager.login(self.username, self.password)
                    failures = 0
                    delay = self.interval
                    event = {'state': 'online', 'ip': result['ip'],
                             'message': '校园网在线' if result['result'] == 'already_online' else '登录成功'}
                    if result.get('interface'):
                        event['message'] += ' · ' + result['interface']
                except LoginError as exc:
                    failures += 1
                    delay = min(300, 30 * 2 ** min(failures - 1, 4))
                    event = {'state': 'error', 'message': str(exc)}
                except Exception as exc:
                    failures += 1
                    delay = min(300, 30 * 2 ** min(failures - 1, 4))
                    event = {'state': 'error', 'message': '运行异常：' + type(exc).__name__}
                # Credentials never appear in emitted status events.
                event['message'] = event['message'].replace(self.password, '[redacted]').replace(self.username, '[redacted]')
                event.update(delay=delay, next_check=time.time() + delay,
                             elapsed_ms=round((time.monotonic() - started) * 1000))
                self.on_event(event)
                # No busy loop, ping process, countdown or polling during sleep.
                if self.stop_event.wait(delay):
                    break
        finally:
            self.password = None
            self.manager.close()
            self.on_event({'state': 'stopped', 'message': '已停止自动重连'})
