import contextlib
import io
import unittest
from unittest.mock import patch

from always_online import main


class DockerEntryTests(unittest.TestCase):
    def test_missing_credentials_fail_before_any_network_or_prompt(self):
        for env in ({}, {'USERNAME': 'test'}, {'PASSWORD': 'test'}, {'USERNAME': ' ', 'PASSWORD': 'test'}):
            with self.subTest(env=tuple(env)), patch.dict('os.environ', env, clear=True), patch('sys.argv', ['docker_entry.py']), patch('sys.stdin.isatty', return_value=True), patch('builtins.input') as prompt, patch('getpass.getpass') as password, patch('always_online.InterfaceLoginManager') as manager, contextlib.redirect_stderr(io.StringIO()) as errors:
                self.assertEqual(main(require_credentials=True, try_all_interfaces=True), 1)
                self.assertIn('必须在 docker run', errors.getvalue())
                prompt.assert_not_called()
                password.assert_not_called()
                manager.assert_not_called()

    def test_status_cannot_bypass_required_credentials(self):
        with patch.dict('os.environ', {}, clear=True), patch('sys.argv', ['docker_entry.py', '--status']), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(main(require_credentials=True, try_all_interfaces=True), 1)

    def test_run_uses_all_interfaces_without_prompt(self):
        with patch.dict('os.environ', {'USERNAME': 'test', 'PASSWORD': 'synthetic'}, clear=True), patch('sys.argv', ['docker_entry.py']), patch('always_online.InterfaceLoginManager') as manager, patch('always_online.always_login') as run, patch('builtins.input') as prompt:
            self.assertEqual(main(require_credentials=True, try_all_interfaces=True), 0)
            self.assertTrue(manager.call_args.kwargs['try_all'])
            self.assertIsNone(manager.call_args.kwargs['interface'])
            run.assert_called_once()
            prompt.assert_not_called()

    def test_explicit_interface_is_forwarded(self):
        with patch.dict('os.environ', {'USERNAME': 'test', 'PASSWORD': 'synthetic', 'INTERFACE': 'eth0'}, clear=True), patch('sys.argv', ['docker_entry.py']), patch('always_online.InterfaceLoginManager') as manager, patch('always_online.always_login'):
            self.assertEqual(main(require_credentials=True, try_all_interfaces=True), 0)
            self.assertEqual(manager.call_args.kwargs['interface'], 'eth0')


if __name__ == '__main__':
    unittest.main()
