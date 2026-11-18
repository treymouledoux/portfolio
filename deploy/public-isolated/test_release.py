"""Run on a workstation. All files are temporary; all service calls are mocked."""
import fcntl
import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest.mock import Mock, call, patch
import zipfile

import activate


class PeerIdentity(unittest.TestCase):
    connection = "100.64.1.4 54321 100.64.1.10 2222"
    signed_node = {"Addresses": ["100.64.1.4/32"], "Tags": ["tag:ci"]}

    @patch("activate.subprocess.run")
    def test_requires_tag_and_exact_peer_address(self, run):
        run.return_value.stdout = json.dumps({"Node": self.signed_node}).encode()
        activate.require_ci_peer(self.connection)
        args, kwargs = run.call_args
        self.assertEqual(args[0], ["/usr/bin/tailscale", "--socket=/var/run/tailscale/tailscaled.sock",
                                   "whois", "--json", "100.64.1.4:54321"])
        self.assertIs(kwargs["stdin"], subprocess.DEVNULL)
        self.assertTrue(kwargs["check"])
        self.assertEqual(kwargs["timeout"], 5)
        self.assertEqual(kwargs["env"], {"PATH": "/usr/bin:/bin", "LC_ALL": "C"})

    @patch("activate.activate")
    @patch("activate.subprocess.run")
    def test_untrusted_or_unavailable_identity_cannot_start_release(self, run, publish):
        stream = Mock()
        env = {"SSH_CONNECTION": self.connection, "SSH_ORIGINAL_COMMAND": "deploy 123-1 " + "0" * 64}
        for doc in ({}, [], {"Node": {"Tags": ["tag:ci"]}},
                    {"Node": {**self.signed_node, "Tags": []}},
                    {"Node": {**self.signed_node, "Tags": "tag:ci"}},
                    {"Node": {**self.signed_node, "Addresses": ["100.64.1.9/32"]}}):
            run.return_value.stdout = json.dumps(doc).encode()
            with self.subTest(doc=doc), self.assertRaises(PermissionError):
                activate.run_session(env, stream)
        run.return_value.stdout = b""
        with self.assertRaises(ValueError):
            activate.run_session(env, stream)
        run.side_effect = subprocess.TimeoutExpired("whois", 5)
        with self.assertRaises(subprocess.TimeoutExpired):
            activate.run_session(env, stream)
        stream.read.assert_not_called()
        publish.assert_not_called()

    @patch("activate.subprocess.run")
    def test_missing_or_non_tailnet_socket_rejected_before_lookup(self, run):
        for connection in ("", "192.0.2.1 444 100.64.1.10 2222", "100.64.1.4 0 100.64.1.10 2222",
                           "100.64.1.4 444 127.0.0.1 2222", "100.64.1.4 444 100.64.1.10 22"):
            with self.subTest(connection=connection), self.assertRaises(PermissionError):
                activate.require_ci_peer(connection)
        run.assert_not_called()


def package(extra=None, omit=None):
    files = {"portfolio-serve": b"binary", "assets/index.html": b"home",
             "assets/resume.pdf": b"resume", "assets/_next/app.js": b"javascript"}
    if omit:
        del files[omit]
    if extra:
        files.update(extra)
    result = io.BytesIO()
    with zipfile.ZipFile(result, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, data in files.items():
            archive.writestr(name, data)
    return result.getvalue()


class IsolatedRelease(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.old = "releases/0-1"
        (self.base / self.old).mkdir(parents=True)
        (self.base / "current").symlink_to(self.old)
        self.restart = Mock()

    def send(self, data=None, digest=None, release="123-1"):
        data = package() if data is None else data
        digest = hashlib.sha256(data).hexdigest() if digest is None else digest
        activate.activate(self.base, f"deploy {release} {digest}", io.BytesIO(data), self.restart)

    def test_release(self):
        self.send()
        self.restart.assert_called_once_with()
        current = self.base / "current"
        self.assertEqual(os.readlink(current), "releases/123-1")
        self.assertEqual((current / "portfolio-serve").stat().st_mode & 0o777, 0o555)
        self.assertEqual((current / "assets/index.html").stat().st_mode & 0o777, 0o444)
        self.assertTrue((self.base / self.old).is_dir())
        with self.assertRaises(ValueError):
            self.send()
        self.restart.assert_called_once_with()

    def test_failed_health_restores_seed(self):
        self.restart.side_effect = [RuntimeError("fake health failure"), None]
        with self.assertRaises(RuntimeError):
            self.send()
        self.assertEqual(os.readlink(self.base / "current"), self.old)
        self.assertEqual(self.restart.call_args_list, [call(), call()])
        self.assertFalse((self.base / ".rollback-target").exists())

    def test_interrupted_attempt_recovers_before_next_release(self):
        self.restart.side_effect = [RuntimeError("fake"), RuntimeError("fake rollback")]
        with self.assertRaises(RuntimeError):
            self.send()
        self.assertEqual((self.base / ".rollback-target").read_text(), self.old)
        self.restart.reset_mock(side_effect=True)
        self.send(release="123-2")
        self.assertEqual(self.restart.call_count, 2)
        self.assertFalse((self.base / ".rollback-target").exists())

    def test_invalid_uploads_leave_current_untouched(self):
        with self.assertRaises(ValueError):
            self.send(digest="0" * 64)
        with self.assertRaises(ValueError):
            self.send(data=package(omit="assets/resume.pdf"))
        for member in ("../outside", "/absolute", "assets/../unexpected", "portfolio.service"):
            with self.subTest(member=member), self.assertRaises(ValueError):
                self.send(data=package(extra={member: b"invalid"}))
        with self.assertRaises(ValueError):
            self.send(release="bad-id")
        self.restart.assert_not_called()
        self.assertEqual(os.readlink(self.base / "current"), self.old)

    def test_busy_deployment_is_rejected(self):
        with (self.base / ".deploy.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            with self.assertRaises(BlockingIOError):
                self.send()
        self.restart.assert_not_called()

    @patch("activate.http.client.HTTPConnection")
    @patch("activate.subprocess.run")
    def test_restart_uses_only_fixed_command_and_local_health(self, run, http):
        reply = http.return_value.getresponse.return_value
        reply.status, reply.read.return_value = 200, b"ok\n"
        activate.restart_and_check()
        run.assert_called_once_with(["/usr/bin/sudo", "-n", "/usr/bin/systemctl", "restart", "portfolio.service"],
                                    check=True, timeout=90)
        http.assert_called_once_with("127.0.0.1", 6050, timeout=2)
        http.return_value.request.assert_called_once_with("GET", "/healthz")


if __name__ == "__main__":
    unittest.main()
