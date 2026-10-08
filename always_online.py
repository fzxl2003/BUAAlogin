"""Campus authentication CLI; never infer campus status from the USB uplink."""
import argparse
import getpass
import logging
import math
import os
import signal
import sys
import time
from BUAASrunLogin.LoginManager import LoginManager, LoginError
from BUAASrunLogin.monitor import ConnectionMonitor
from BUAASrunLogin.interfaces import InterfaceLoginManager


def always_login(username, password, testip=None, checkinterval=300, *, manager=None):
    def report(event):
        if event['state'] != 'checking':
            message = event['message']
            if event.get('delay'):
                message += f"；{event['delay']:g} 秒后检查"
            print(time.strftime('%Y-%m-%d %H:%M:%S'), message, flush=True)
    monitor = ConnectionMonitor(username, password, interval=checkinterval,
                                manager=manager, on_event=report)
    previous = {}
    for signum in (signal.SIGTERM, signal.SIGINT):
        previous[signum] = signal.signal(signum, lambda *_: monitor.stop())
    try:
        monitor.run()
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)


def main(*, require_credentials=False, try_all_interfaces=False):
    parser = argparse.ArgumentParser(description='北航校园网登录（校园网请求独立绕过代理）')
    parser.add_argument('--status', action='store_true', help='只检查校园网认证状态，不需要账号密码')
    parser.add_argument('--once', action='store_true', help='登录一次后退出')
    parser.add_argument('--interface', default=os.getenv('INTERFACE') or None,
                        help='校园网接口；默认自动检测，也可设置 INTERFACE 环境变量')
    parser.add_argument('--gateway-ip', default='10.200.21.4')
    parser.add_argument('--interval', type=float, default=300)
    parser.add_argument('--debug', action='store_true', help='输出脱敏调试日志')
    parser.add_argument('--log-file', help='同时保存脱敏调试日志到文件（自动启用调试）')
    args = parser.parse_args()
    if not math.isfinite(args.interval) or not 30 <= args.interval <= 86400:
        parser.error('--interval 必须在 30–86400 秒之间')
    username = (os.getenv('USERNAME') or '').strip()
    password = os.getenv('PASSWORD') or ''
    if require_credentials and (not username or not password):
        print('错误：必须在 docker run 时通过 -e USERNAME 和 -e PASSWORD 提供非空账号密码。', file=sys.stderr)
        return 1
    if args.debug or args.log_file:
        handlers = [logging.StreamHandler()]
        if args.log_file:
            handlers.append(logging.FileHandler(args.log_file, encoding='utf-8'))
        logging.basicConfig(level=logging.DEBUG, handlers=handlers,
                            format='%(asctime)s %(levelname)s %(message)s', force=True)
    lm = (InterfaceLoginManager(interface=args.interface, gateway_ip=args.gateway_ip,
                                try_all=try_all_interfaces) if try_all_interfaces else
          LoginManager(interface=args.interface, gateway_ip=args.gateway_ip))
    try:
        if args.status:
            if isinstance(lm, InterfaceLoginManager):
                status = lm.status()
                print(f"校园网状态: {'在线' if status['online'] else '未登录'}")
            else:
                lm.get_ip()
                status = lm.status()
                print(f"校园网 IP: {lm.ip}; ac_id: {lm.ac_id}; 状态: {'在线' if status['online'] else '未登录'}")
            return 0 if status['online'] else 1
        if (not username or not password) and not sys.stdin.isatty():
            raise LoginError('非交互运行请设置非空 USERNAME 和 PASSWORD')
        username = username or input('校园网账号: ').strip()
        password = password or getpass.getpass('校园网密码: ')
        if args.once:
            result = lm.login(username, password)
            print('校园网认证状态:', result['result'])
        else:
            lm.close()
            lm = InterfaceLoginManager(interface=args.interface, gateway_ip=args.gateway_ip,
                                       try_all=try_all_interfaces)
            always_login(username, password, checkinterval=args.interval, manager=lm)
        return 0
    except LoginError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    except (KeyboardInterrupt, EOFError):
        return 0
    finally:
        lm.close()


if __name__ == '__main__':
    sys.exit(main())
