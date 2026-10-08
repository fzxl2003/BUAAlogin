"""Windows GUI worker: read configuration over a pipe, emit JSON status lines."""
import json
import sys
import threading

from BUAASrunLogin.interfaces import InterfaceLoginManager, check_interfaces
from BUAASrunLogin.monitor import ConnectionMonitor


def main():
    if hasattr(sys.stdin, 'reconfigure'):
        sys.stdin.reconfigure(encoding='utf-8')
        sys.stdout.reconfigure(encoding='utf-8')
    stop = threading.Event()
    def emit(event):
        print(json.dumps(event, ensure_ascii=False), flush=True)
    manager = None
    try:
        config = json.loads(sys.stdin.readline())
        if config.get('action') == 'interfaces':
            emit({'state': 'interfaces', 'records': check_interfaces(config.get('gateway_ip', '10.200.21.4')),
                  'message': '接口检测完成'})
            return 0
        interval = float(config.get('interval', 300))
        if not 30 <= interval <= 86400:
            raise ValueError('检查间隔须在 30–86400 秒之间')
        manager = InterfaceLoginManager(interface=config.get('interface') or None,
                                       gateway_ip=config.get('gateway_ip', '10.200.21.4'), on_event=emit)
        monitor = ConnectionMonitor(config['username'], config['password'], interval=interval,
                                    manager=manager, stop_event=stop, on_event=emit)
        config.clear()
        def listen():
            # A blocking pipe read consumes no CPU; any command/EOF means stop.
            sys.stdin.readline()
            stop.set()
        threading.Thread(target=listen, daemon=True).start()
        monitor.run()
        return 0
    except Exception as exc:
        emit({'state': 'error', 'message': '配置或网络检测失败：' + type(exc).__name__})
        return 1
    finally:
        if manager:
            manager.close()


if __name__ == '__main__':
    sys.exit(main())
