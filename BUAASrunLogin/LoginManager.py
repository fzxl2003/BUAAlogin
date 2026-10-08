"""Srun login using the campus interface, independent of system proxies."""
import ipaddress
import json
import logging
import re
import shutil
import subprocess
import sys
import tempfile
import time
from urllib.parse import parse_qs, urlencode, urljoin, urlsplit

from .encryption.srun_md5 import get_md5
from .encryption.srun_sha1 import get_sha1
from .encryption.srun_base64 import get_base64
from .encryption.srun_xencode import get_xencode


class LoginError(RuntimeError):
    pass


def parse_jsonp(text):
    text = text.strip()
    if not text.startswith('{'):
        match = re.fullmatch(r'[\w.$]+\s*\((.*)\)\s*;?', text, re.S)
        if not match:
            raise LoginError('网关返回了无法识别的 JSON/JSONP')
        text = match.group(1)
    try:
        data = json.loads(text)
    except ValueError as exc:
        raise LoginError('网关 JSON 格式错误') from exc
    if not isinstance(data, dict):
        raise LoginError('网关返回值不是对象')
    return data


class LoginManager:
    def __init__(self, url_login_page=None, url_get_challenge_api=None,
                 url_login_api=None, n='200', vtype='1', acid=None,
                 enc='srun_bx1', *, interface=None, gateway_ip='10.200.21.4',
                 timeout=15, expected_ip=None):
        self.url_login_page = url_login_page or 'https://gw.buaa.edu.cn/'
        base = self.url_login_page
        self.url_get_challenge_api = url_get_challenge_api or urljoin(base, '/cgi-bin/get_challenge')
        self.url_login_api = url_login_api or urljoin(base, '/cgi-bin/srun_portal')
        self.url_status_api = urljoin(base, '/cgi-bin/rad_user_info')
        self.n, self.vtype, self.enc = str(n), str(vtype), enc
        self.ac_id = str(acid) if acid is not None else None
        self._explicit_acid = acid is not None
        self.interface = interface if interface is not None else ('en0' if sys.platform == 'darwin' else None)
        self.gateway_ip = str(ipaddress.IPv4Address(gateway_ip)) if gateway_ip else None
        self.timeout = float(timeout)
        self.expected_ip = expected_ip
        self._cookie_dir = tempfile.TemporaryDirectory(prefix='buaa-session-')
        self._cookie_jar = self._cookie_dir.name + '/cookies'
        self.log = logging.getLogger('buaa')

    def close(self):
        self._cookie_dir.cleanup()

    def __del__(self):
        cookie_dir = getattr(self, '_cookie_dir', None)
        if cookie_dir is not None:
            cookie_dir.cleanup()

    def _debug(self, event, **fields):
        if not self.log.isEnabledFor(logging.DEBUG):
            return
        # Only explicitly selected diagnostic fields reach this method.
        message = json.dumps(fields, ensure_ascii=False, default=str)
        for attr in ('password', 'username', 'token', 'info', 'encrypted_info',
                     'md5', 'encrypted_md5', 'encrypted_chkstr'):
            secret = getattr(self, attr, None)
            if isinstance(secret, str) and secret:
                message = message.replace(json.dumps(secret, ensure_ascii=False)[1:-1], '[redacted]')
        self.log.debug('%s %s', event, message)

    def _request(self, url, params=None):
        if getattr(self, 'stop_event', None) is not None and self.stop_event.is_set():
            raise LoginError('已停止')
        if params:
            url += ('&' if '?' in url else '?') + urlencode(params)
        parts = urlsplit(url)
        if parts.scheme != 'https' or parts.hostname != urlsplit(self.url_login_page).hostname:
            raise LoginError('认证请求和重定向必须使用同一网关的 HTTPS')
        curl = shutil.which('curl')
        if not curl:
            raise LoginError('请先安装 curl')
        args = [curl, '-q', '--silent', '--show-error', '--noproxy', '*', '--ipv4',
                '--connect-timeout', str(self.timeout), '--max-time', str(self.timeout),
                '--proto', '=https', '--write-out', '\n%{http_code}\n%{redirect_url}',
                '--config', '-']
        if self.interface:
            args += ['--interface', self.interface]
        if self.gateway_ip:
            args += ['--resolve', f'{parts.hostname}:{parts.port or 443}:{self.gateway_ip}']
        args += ['--cookie', self._cookie_jar, '--cookie-jar', self._cookie_jar]
        # Persist the portal session across page, challenge and login requests.
        started = time.monotonic()
        self._debug('request.start', host=parts.hostname, path=parts.path,
                    interface=self.interface, gateway_ip=self.gateway_ip,
                    proxy='bypass', timeout=self.timeout,
                    parameter_names=sorted((params or {}).keys()))
        # Login parameters stay out of process arguments and diagnostic output.
        try:
            result = subprocess.run(args, input='url = ' + json.dumps(url) + '\n',
                                    capture_output=True, text=True, encoding='utf-8',
                                    errors='replace', timeout=self.timeout + 2,
                                    creationflags=subprocess.CREATE_NO_WINDOW if sys.platform == 'win32' else 0)
        except (OSError, subprocess.TimeoutExpired) as exc:
            self._debug('request.exception', kind=type(exc).__name__,
                        elapsed_ms=round((time.monotonic() - started) * 1000))
            raise LoginError('网关请求无法执行或超时') from exc
        self._debug('request.end', path=parts.path, curl_code=result.returncode,
                    elapsed_ms=round((time.monotonic() - started) * 1000),
                    response_bytes=len(result.stdout.encode()))
        if result.returncode:
            raise LoginError(f'校园网请求失败 (curl {result.returncode})；检查 Wi-Fi、接口和网关地址')
        try:
            body, status, redirect = result.stdout.rsplit('\n', 2)
            status = int(status)
        except ValueError as exc:
            raise LoginError('网关 HTTP 响应无法识别') from exc
        self._debug('request.http', status=status,
                    redirect_path=urlsplit(redirect).path if redirect else None)
        if status in (301, 302, 303, 307, 308):
            return body, urljoin(url, redirect)
        if status != 200:
            raise LoginError(f'网关 HTTP 错误 {status}')
        return body, None

    def get_ip(self):
        url = self.url_login_page
        for _ in range(6):
            body, redirect = self._request(url)
            if redirect:
                url = redirect
                continue
            refresh = re.search(r'<meta\b[^>]*content=[\'"][^\'"]*url=([^\'"]+)', body, re.I)
            if refresh:
                url = urljoin(url, refresh.group(1).replace('&amp;', '&'))
                continue
            ip = re.search(r'\bip\s*:\s*[\'"]([^\'"]+)', body)
            if not ip:
                ip = re.search(r'id=[\'"]user_ip[\'"]\s+value=[\'"]([^\'"]+)', body)
            if not ip:
                raise LoginError('页面没有校园网 IP；可能连接到了错误的网关')
            self.ip = str(ipaddress.IPv4Address(ip.group(1)))
            if self.expected_ip and self.ip != self.expected_ip:
                raise LoginError('网关 IP 与所选接口 IP 不一致，该接口未通过校园网验证')
            if not self._explicit_acid:
                acid = re.search(r'\bacid\s*:\s*[\'"]([^\'"]+)', body)
                self.ac_id = acid.group(1) if acid else parse_qs(urlsplit(url).query).get('ac_id', [None])[0]
            if not self.ac_id or not self.ac_id.isdigit():
                raise LoginError('页面没有有效的 ac_id')
            self._debug('page.resolved', ip=self.ip, ac_id=self.ac_id)
            return self.ip
        raise LoginError('网关重定向次数过多')

    def _api(self, url, params=None):
        params = dict(params or {}, callback='buaa' + str(time.time_ns()), _=int(time.time() * 1000))
        body, redirect = self._request(url, params)
        if redirect:
            raise LoginError('认证接口发生重定向，请检查网关地址')
        data = parse_jsonp(body)
        safe = {key: data[key] for key in (
            'error', 'error_msg', 'suc_msg', 'res', 'ecode', 'client_ip',
            'online_ip', 'expire', 'st') if key in data}
        self._debug('api.result', path=urlsplit(url).path, fields=sorted(data), result=safe)
        return data

    def status(self):
        data = self._api(self.url_status_api)
        # Do not expose the response's account/balance/device details.
        online_ip = data.get('online_ip')
        online = data.get('error') == 'ok' and bool(online_ip)
        if hasattr(self, 'ip'):
            online = online and online_ip == self.ip
        self._debug('status.checked', online=online, ip_matches=online_ip == getattr(self, 'ip', None))
        return {'online': online, 'ip': online_ip if online else None}

    def get_token(self):
        data = self._api(self.url_get_challenge_api, {'username': self.username, 'ip': self.ip})
        token = data.get('challenge')
        if not isinstance(token, str) or not token:
            raise LoginError('获取 challenge 失败：' + str(data.get('error', 'unknown')))
        if data.get('client_ip') and data['client_ip'] != self.ip:
            raise LoginError('challenge IP 与 Wi-Fi IP 不一致，请检查网络接口')
        self.token = token
        self._token_received_at = time.monotonic()
        self._debug('challenge.received', token_length=len(token), ip=self.ip)
        return token

    def _generate_info(self):
        self.info = json.dumps({'username': self.username, 'password': self.password,
                                'ip': self.ip, 'acid': self.ac_id, 'enc_ver': self.enc},
                               ensure_ascii=False, separators=(',', ':'))

    def _generate_encrypted_login_info(self):
        self._generate_info()
        self.encrypted_info = '{SRBX1}' + get_base64(get_xencode(self.info, self.token))
        self.md5 = get_md5(self.password, self.token)
        self.encrypted_md5 = '{MD5}' + self.md5
        self.chkstr = self.token + (self.token.join([
            self.username, self.md5, self.ac_id, self.ip,
            self.n, self.vtype, self.encrypted_info]))
        self.encrypted_chkstr = get_sha1(self.chkstr)

    def get_login_responce(self):
        started = time.monotonic()
        self._generate_encrypted_login_info()
        self._debug('login.submit', ip=self.ip, ac_id=self.ac_id, n=self.n,
                    type=self.vtype, encrypted_info_length=len(self.encrypted_info),
                    encryption_ms=round((time.monotonic() - started) * 1000, 2),
                    challenge_age_ms=round((time.monotonic() - self._token_received_at) * 1000, 2)
                    if hasattr(self, '_token_received_at') else None)
        data = self._api(self.url_login_api, {
            'action': 'login', 'username': self.username, 'password': self.encrypted_md5,
            'ac_id': self.ac_id, 'ip': self.ip, 'info': self.encrypted_info,
            'chksum': self.encrypted_chkstr, 'n': self.n, 'type': self.vtype,
            'double_stack': '0', 'os': sys.platform, 'name': 'Python'})
        if data.get('error') != 'ok':
            raise LoginError('登录失败：' + str(data.get('error_msg') or data.get('error') or 'unknown'))
        self._login_result = data.get('suc_msg', 'ok')
        if not self.status()['online']:
            raise LoginError('网关接受了认证，但未确认当前 Wi-Fi IP 在线')
        return {'online': True, 'ip': self.ip, 'result': self._login_result}

    def login(self, username, password):
        if not username or not password:
            raise LoginError('用户名和密码不能为空')
        self.username, self.password = username, password
        self._debug('login.start', interface=self.interface, gateway_ip=self.gateway_ip,
                    python=sys.version.split()[0], platform=sys.platform)
        try:
            # A healthy connection only needs one status request per interval.
            if hasattr(self, 'ip') and self.status()['online']:
                return {'online': True, 'ip': self.ip, 'result': 'already_online'}
            self.get_ip()
            if self.status()['online']:
                return {'online': True, 'ip': self.ip, 'result': 'already_online'}
            self.get_token()
            return self.get_login_responce()
        finally:
            self._debug('login.end')
            self.password = None
            self.info = None
