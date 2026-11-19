"""Exercise certificate installation with ephemeral PKI and no network or systemd."""

import importlib.util
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("hook", Path(__file__).with_name("acme-install.py"))
hook = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hook)


class RotationTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.lineage = self.root / "lineage"
        self.lineage.mkdir()
        self.base = self.root / "caddy"
        self.base.mkdir()
        cmd = shutil.which("openssl")
        self.patch(hook, "OPENSSL", cmd)
        self.patch(hook, "TRUST_STORE", str(self.root / "ca.pem"))
        self.state = self.patch(hook, "gateway_state")
        self.state.return_value = b"inactive"
        self.restart = self.patch(hook, "restart")
        self.validate = self.patch(hook, "validate")
        self.ssl("req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
                 "-nodes", "-days", "30", "-subj", "/CN=Test Authority",
                 "-addext", "basicConstraints=critical,CA:TRUE",
                 "-keyout", "ca.key", "-out", "ca.pem")
        self.issue()

    def patch(self, obj, name, *args):
        p = patch.object(obj, name, *args)
        self.addCleanup(p.stop)
        return p.start()

    def ssl(self, *args):
        subprocess.run([hook.OPENSSL, *args], cwd=self.root, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def issue(self, name=None, lineage=None):
        name = name or hook.APEX["domain"]
        lineage = lineage or self.lineage
        self.ssl("req", "-new", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
                 "-nodes", "-subj", f"/CN={name}", "-keyout", str(lineage / "privkey.pem"),
                 "-out", "req")
        (self.root / "ext").write_text(f"subjectAltName=DNS:{name}\nextendedKeyUsage=serverAuth\n")
        self.ssl("x509", "-req", "-in", "req", "-CA", "ca.pem", "-CAkey", "ca.key",
                 "-CAcreateserial", "-days", "20", "-extfile", "ext", "-out", str(lineage / "cert.pem"))
        shutil.copy(self.root / "ca.pem", lineage / "chain.pem")

    def install(self, spec=None):
        hook.install(self.base, self.lineage, os.getgid(), spec or hook.APEX)

    def test_initial_and_active_rotation(self):
        self.install()
        link = self.base / "tls"
        previous = link.resolve()
        self.assertTrue(link.is_symlink())
        self.assertEqual(stat.S_IMODE((link / "apex-key.pem").stat().st_mode), 0o640)
        self.restart.assert_not_called()
        self.issue()
        self.state.return_value = b"active"
        self.install()
        self.assertNotEqual(previous, link.resolve())
        self.assertTrue(previous.is_dir())
        self.assertEqual((link / "apex-key.pem").read_bytes(), (self.lineage / "privkey.pem").read_bytes())
        self.restart.assert_called_once()

    def test_failed_restart_restores_and_restarts_old_generation(self):
        self.install()
        previous = (self.base / "tls").resolve()
        self.issue()
        self.state.return_value = b"active"
        self.restart.side_effect = [RuntimeError("service failed"), None]
        with self.assertRaisesRegex(RuntimeError, "service failed"):
            self.install()
        self.assertEqual((self.base / "tls").resolve(), previous)
        self.assertEqual(self.restart.call_count, 2)
        self.assertEqual(list(previous.parent.iterdir()), [previous])

    def test_failed_validation_on_first_install_keeps_service_inactive(self):
        self.validate.side_effect = ValueError("bad caddyfile")
        with self.assertRaisesRegex(ValueError, "bad caddyfile"):
            self.install()
        self.assertFalse((self.base / "tls").is_symlink())
        self.restart.assert_not_called()

    def test_wrong_key_or_issuer_or_name_keeps_old_generation(self):
        self.install()
        previous = (self.base / "tls").resolve()
        self.issue("other.example.invalid")
        with self.assertRaises(subprocess.CalledProcessError):
            self.install()
        self.issue()
        with patch.object(hook, "TRUST_STORE", str(self.root / "missing-ca.pem")):
            with self.assertRaises(subprocess.CalledProcessError):
                self.install()
        shutil.copy(self.root / "ca.key", self.lineage / "privkey.pem")
        with self.assertRaisesRegex(ValueError, "do not match"):
            self.install()
        self.assertEqual((self.base / "tls").resolve(), previous)

    def test_dashboard_lineage_installs_its_own_generation(self):
        self.install()
        apex = (self.base / "tls").resolve()
        lineage = self.root / "dashboard-lineage"
        lineage.mkdir()
        self.issue(hook.DASHBOARD["domain"], lineage)
        self.state.return_value = b"active"
        hook.install(self.base, lineage, os.getgid(), hook.DASHBOARD)
        link = self.base / "dashboard-tls"
        self.assertEqual((link / "dashboard-key.pem").read_bytes(), (lineage / "privkey.pem").read_bytes())
        self.assertEqual(stat.S_IMODE((link / "dashboard-key.pem").stat().st_mode), 0o640)
        self.assertEqual((self.base / "tls").resolve(), apex)
        self.assertIn("DNS:dashboard.treymouledoux.me".encode(),
                      subprocess.check_output([hook.OPENSSL, "x509", "-in", str(link / "dashboard-fullchain.pem"),
                                               "-noout", "-ext", "subjectAltName"]))


if __name__ == "__main__":
    unittest.main()
