import hashlib
import hmac
import json
import unittest
from unittest.mock import patch

from BUAASrunLogin.LoginManager import LoginManager, LoginError, parse_jsonp


class LoginTests(unittest.TestCase):
    def test_jsonp_spacing_and_failure(self):
        self.assertEqual(parse_jsonp('callback( {"error": "ok"} );'), {'error': 'ok'})
        self.assertEqual(parse_jsonp('{"error":"ok"}'), {'error': 'ok'})
        with self.assertRaises(LoginError):
            parse_jsonp('<html>error</html>')

    def test_modern_page_redirect_and_acid(self):
        lm = LoginManager()
        self.addCleanup(lm.close)
        replies = [('<meta http-equiv="refresh" content="0;url=/srun_portal_pc?ac_id=78&amp;theme=buaa">', None),
                   ('var CONFIG = {acid: "78", ip: "10.138.1.2"};', None)]
        with patch.object(lm, '_request', side_effect=replies) as request:
            self.assertEqual(lm.get_ip(), '10.138.1.2')
            self.assertEqual(lm.ac_id, '78')
            self.assertIn('ac_id=78&theme=buaa', request.call_args.args[0])

    def test_legacy_page_explicit_acid(self):
        lm = LoginManager(acid='1')
        self.addCleanup(lm.close)
        with patch.object(lm, '_request', return_value=('<input id="user_ip" value="10.1.2.3">', None)):
            self.assertEqual(lm.get_ip(), '10.1.2.3')
            self.assertEqual(lm.ac_id, '1')

    def test_password_hmac_and_json_are_preserved(self):
        lm = LoginManager(acid='78')
        self.addCleanup(lm.close)
        lm.username, lm.password = 'test', 'a b\'"\\c'
        lm.ip, lm.token = '10.1.2.3', '0123456789abcdef' * 4
        lm._generate_encrypted_login_info()
        self.assertEqual(json.loads(lm.info)['password'], lm.password)
        expected = hmac.new(lm.token.encode(), lm.password.encode(), hashlib.md5).hexdigest()
        self.assertEqual(lm.md5, expected)
        values = [lm.username, expected, '78', lm.ip, '200', '1', lm.encrypted_info]
        checksum = hashlib.sha1(''.join(lm.token + value for value in values).encode()).hexdigest()
        self.assertEqual(lm.encrypted_chkstr, checksum)

    def test_offline_login_flow_and_cleanup(self):
        lm = LoginManager(acid='78')
        self.addCleanup(lm.close)
        def page():
            lm.ip = '10.1.2.3'
        responses = [{'error': 'not_online_error'}, {'error': 'ok', 'challenge': 'abc123'},
                     {'error': 'ok', 'suc_msg': 'login_ok'}, {'error': 'ok', 'online_ip': '10.1.2.3'}]
        with patch.object(lm, 'get_ip', side_effect=page), patch.object(lm, '_api', side_effect=responses) as api:
            self.assertEqual(lm.login('test', 'test password')['result'], 'login_ok')
            self.assertEqual(api.call_args_list[2].args[1]['ac_id'], '78')
        self.assertIsNone(lm.password)
        self.assertIsNone(lm.info)

    def test_auth_failure_does_not_require_suc_msg(self):
        lm = LoginManager(acid='78')
        self.addCleanup(lm.close)
        lm.username, lm.password, lm.ip, lm.token = 'test', 'bad', '10.1.2.3', 'abc'
        with patch.object(lm, '_api', return_value={'error': 'E2531', 'error_msg': 'password_error'}):
            with self.assertRaisesRegex(LoginError, 'password_error'):
                lm.get_login_responce()

    def test_status_belongs_to_campus_ip(self):
        lm = LoginManager()
        self.addCleanup(lm.close)
        lm.ip = '10.1.2.3'
        with patch.object(lm, '_api', return_value={'error': 'ok', 'online_ip': '172.20.10.2'}):
            self.assertFalse(lm.status()['online'])

    def test_transport_bypasses_proxy_and_binds_wifi(self):
        lm = LoginManager()
        self.addCleanup(lm.close)
        with patch('subprocess.run') as run:
            run.return_value.returncode = 0
            run.return_value.stdout = '{}\n200\n'
            lm._request('https://gw.buaa.edu.cn/cgi-bin/srun_portal', {'password': 'secret'})
            args = run.call_args.args[0]
            self.assertEqual(args[args.index('--noproxy') + 1], '*')
            self.assertEqual(args[args.index('--interface') + 1], 'en0')
            self.assertIn('gw.buaa.edu.cn:443:10.200.21.4', args)
            self.assertNotIn('secret', ' '.join(args))
            self.assertNotIn('--insecure', args)
            self.assertEqual(args[args.index('--cookie') + 1],
                             args[args.index('--cookie-jar') + 1])
        with self.assertRaises(LoginError):
            lm._request('https://external.example/login')

    def test_debug_redacts_secrets(self):
        lm = LoginManager()
        self.addCleanup(lm.close)
        lm.username, lm.password, lm.token = 'test-user', 'a b\"secret', 'secret-token'
        with self.assertLogs('buaa', level='DEBUG') as logs:
            lm._debug('test', response='test-user a b\"secret secret-token')
        output = '\n'.join(logs.output)
        self.assertNotIn(lm.username, output)
        self.assertNotIn('secret', output)
        self.assertIn('[redacted]', output)


if __name__ == '__main__':
    unittest.main()
