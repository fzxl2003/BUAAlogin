"""Discover connected IPv4 interfaces and verify the actual campus portal path."""
from dataclasses import dataclass
import ipaddress
import json
import re
import subprocess
import sys
import threading

from .LoginManager import LoginError, LoginManager


@dataclass(frozen=True)
class NetworkInterface:
    name: str
    address: str

    @property
    def binding(self):
        # Windows curl does not support adapter names.
        return self.address if sys.platform == 'win32' else self.name

    @property
    def label(self):
        return f'{self.name} · {self.address}'


def _command(args):
    try:
        result = subprocess.run(args, capture_output=True, text=True, encoding='utf-8',
                                errors='replace', timeout=10,
                                creationflags=subprocess.CREATE_NO_WINDOW if sys.platform == 'win32' else 0)
        if result.returncode:
            raise LoginError('无法读取网络接口')
        return result.stdout
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise LoginError('无法读取网络接口，请检查系统网络工具') from exc


def discover_interfaces(*, include_virtual=False):
    records = []
    if sys.platform == 'darwin':
        output = _command(['/sbin/ifconfig'])
        blocks = re.split(r'(?m)^(\w+): flags=', output)[1:]
        for name, body in zip(blocks[::2], blocks[1::2]):
            if 'status: inactive' in body:
                continue
            for address in re.findall(r'\binet (\d+\.\d+\.\d+\.\d+)\b', body):
                records.append(NetworkInterface(name, address))
    elif sys.platform == 'win32':
        script = """
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$physical = @(Get-NetAdapter -Physical | Where-Object {$_.Status -eq 'Up'} | Select-Object -ExpandProperty ifIndex)
@(Get-NetIPAddress -AddressFamily IPv4 | Where-Object {
    $_.InterfaceIndex -in $physical -and $_.AddressState -eq 'Preferred'
} | Select-Object @{n='name';e={$_.InterfaceAlias}}, @{n='address';e={$_.IPAddress}}) | ConvertTo-Json -Compress
"""
        data = json.loads(_command(['powershell.exe', '-NoProfile', '-NonInteractive', '-Command', script]) or '[]')
        if isinstance(data, dict):
            data = [data]
        records = [NetworkInterface(item['name'], item['address']) for item in data]
    else:
        data = json.loads(_command(['ip', '-j', '-4', 'address', 'show', 'up']))
        records = [NetworkInterface(item['ifname'], addr['local']) for item in data
                   for addr in item.get('addr_info', []) if addr.get('family') == 'inet']
    records = [record for record in records
               if record.name.lower() not in ('lo', 'lo0')
               and (include_virtual or not record.name.lower().startswith(('utun', 'tun', 'tap', 'wg', 'awdl', 'llw')))
               and not ipaddress.IPv4Address(record.address).is_loopback
               and (include_virtual or not ipaddress.IPv4Address(record.address).is_link_local)]
    return sorted(set(records), key=lambda item: (not item.address.startswith('10.'), item.name, item.address))


def campus_manager(interface, gateway_ip, *, timeout=3):
    return LoginManager(interface=interface.binding, gateway_ip=gateway_ip,
                        expected_ip=interface.address, timeout=timeout)


def check_interfaces(gateway_ip='10.200.21.4'):
    results = []
    for interface in discover_interfaces():
        manager = campus_manager(interface, gateway_ip)
        try:
            manager.get_ip()
            results.append({'name': interface.name, 'address': interface.address,
                            'binding': interface.binding, 'label': interface.label,
                            'campus': True, 'message': '校园网', 'ac_id': manager.ac_id})
        except (LoginError, ValueError):
            results.append({'name': interface.name, 'address': interface.address,
                            'binding': interface.binding, 'label': interface.label,
                            'campus': False, 'message': '未检测到校园网'})
        finally:
            manager.close()
    return results


class InterfaceLoginManager:
    """Try connected interfaces in order, retaining sessions for healthy interfaces."""
    def __init__(self, *, interface=None, gateway_ip='10.200.21.4', on_event=None, try_all=False):
        self.selection = interface
        self.gateway_ip = str(ipaddress.IPv4Address(gateway_ip))
        self.on_event = on_event or (lambda event: None)
        self.stop_event = threading.Event()
        self.managers = {}
        self.preferred = None
        self.try_all = try_all

    def login(self, username, password):
        candidates = discover_interfaces(include_virtual=self.try_all)
        if self.selection:
            candidates = [item for item in candidates if self.selection in (item.name, item.address, item.binding)]
        if not candidates:
            raise LoginError('没有可用的校园网接口，请检查 Wi-Fi/网线和接口选择')
        active = set(candidates)
        for interface in list(self.managers):
            if interface not in active:
                self.managers.pop(interface).close()
        # Once online, prefer the last successful interface on subsequent checks.
        candidates.sort(key=lambda item: item != self.preferred)
        errors = []
        successes = []
        for interface in candidates:
            if self.stop_event.is_set():
                raise LoginError('已停止')
            self.on_event({'state': 'checking', 'message': '检查接口 ' + interface.label})
            if interface not in self.managers:
                self.managers[interface] = campus_manager(interface, self.gateway_ip)
            manager = self.managers[interface]
            manager.stop_event = self.stop_event
            try:
                result = manager.login(username, password)
                result['interface'] = interface.label
                self.preferred = interface
                if not self.try_all:
                    return result
                successes.append(result)
            except (LoginError, ValueError) as exc:
                errors.append(interface.name + ': ' + str(exc))
                if self.try_all:
                    message = interface.label + ': ' + str(exc)
                    for secret in (username, password):
                        if secret:
                            message = message.replace(secret, '[redacted]')
                    self.on_event({'state': 'interface_error', 'message': message})
        if successes:
            result = dict(successes[0])
            result['interface'] = '；'.join(item['interface'] for item in successes)
            result['result'] = ('already_online' if all(item['result'] == 'already_online' for item in successes) else 'login_ok')
            return result
        raise LoginError('；'.join(errors))

    def status(self):
        results = []
        for interface in discover_interfaces(include_virtual=self.try_all):
            if self.selection and self.selection not in (interface.name, interface.address, interface.binding):
                continue
            manager = campus_manager(interface, self.gateway_ip)
            try:
                manager.get_ip()
                results.append(manager.status()['online'])
            except LoginError:
                results.append(False)
            finally:
                manager.close()
        return {'online': any(results)}

    def close(self):
        for manager in self.managers.values():
            manager.close()
        self.managers.clear()
