"""Exercise the identity helper without tailscaled, systemd or network access."""

import importlib.util
import itertools
import json
import os
from pathlib import Path
import socket
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit


spec = importlib.util.spec_from_file_location("helper", Path(__file__).with_name("whois-helper.py"))
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)


class LocalAPI(BaseHTTPRequestHandler):
    def do_GET(self):
        address = parse_qs(urlsplit(self.path).query).get("addr", [""])[0]
        self.server.seen.append(address)
        profile = self.server.profiles.get(address)
        body = json.dumps({"UserProfile": profile}).encode() if profile is not None else b"{}"
        self.send_response(200 if profile is not None else 404)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


class UnixServer(ThreadingHTTPServer):
    address_family = socket.AF_UNIX

    def server_bind(self):
        self.socket.bind(self.server_address)
        self.server_name = "local-tailscaled.sock"
        self.server_port = 0


class WhoisTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.socket_path = str(self.root / "tailscaled.sock")
        self.server = UnixServer(self.socket_path, LocalAPI)
        self.server.profiles = {}
        self.server.seen = []
        self.counter = itertools.count()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def reply(self, address, profile):
        self.server.profiles[address] = profile

    def test_human_login_and_tagged_device(self):
        self.reply("100.64.1.2", {"LoginName": "alice@example.com"})
        self.reply("100.64.1.3", {})
        self.assertEqual(helper.whois("100.64.1.2", self.socket_path), "alice@example.com")
        with self.assertRaisesRegex(helper.Rejected, "denied"):
            helper.whois("100.64.1.3", self.socket_path)

    def test_unknown_address_and_missing_daemon(self):
        with self.assertRaisesRegex(helper.Rejected, "unknown"):
            helper.whois("100.64.9.9", self.socket_path)
        with self.assertRaisesRegex(helper.Rejected, "unavailable"):
            helper.whois("100.64.1.2", str(self.root / "missing.sock"))

    def test_rejects_anything_but_one_ip_literal(self):
        for value in ("", "example.com", "100.64.1.2:443", "1e2", " 100.64.1.2", "x" * 80, None, 5):
            with self.subTest(value=value), self.assertRaises(helper.Rejected):
                helper.peer_address(value)
        self.assertEqual(self.server.seen, [])

    def test_only_one_exact_request_shape(self):
        lookup = lambda peer: "alice@example.com"
        self.assertEqual(helper.handle({"peer": "100.64.1.2"}, lookup),
                         {"ok": True, "login": "alice@example.com"})
        for request in ({}, {"peer": "100.64.1.2", "extra": 1}, {"login": "x"}, [],
                        {"peer": 5}, "peer=100.64.1.2"):
            with self.subTest(request=request), self.assertRaises(helper.Rejected):
                helper.handle(request, lookup)
        with self.assertRaises(helper.Rejected):
            helper.handle({"peer": "100.64.1.2"}, lambda peer: helper.login_from_profile(
                {"LoginName": "bad\nlogin"}))

    def test_profile_must_be_one_printable_login(self):
        for profile in ({}, {"LoginName": ""}, {"LoginName": "a\x00b"}, {"LoginName": 5},
                        {"LoginName": "x" * (helper.LOGIN_LIMIT + 1)}, [], "alice"):
            with self.subTest(profile=profile), self.assertRaises(helper.Rejected):
                helper.login_from_profile(profile)
        self.assertEqual(helper.login_from_profile({"LoginName": " alice@example.com "}),
                         "alice@example.com")

    def serve_one(self, payload, uid=42):
        path = str(self.root / f"whois-{next(self.counter)}.sock")
        lookup = lambda peer: helper.whois(peer, self.socket_path)
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(path)
            listener.listen(4)
            threading.Thread(target=helper.serve,
                             args=(listener, 42, lambda conn: uid, lookup), daemon=True).start()
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
                client.settimeout(3)
                client.connect(path)
                try:
                    client.sendall(payload)
                except OSError:
                    # A rejected peer is answered and disconnected immediately.
                    pass
                return json.loads(client.makefile("rb").readline())

    def test_wire_is_bounded_and_peer_checked(self):
        self.reply("100.64.1.2", {"LoginName": "alice@example.com"})
        self.assertEqual(self.serve_one(b'{"peer":"100.64.1.2"}\n'),
                         {"ok": True, "login": "alice@example.com"})
        self.assertEqual(self.serve_one(b'{"peer":"100.64.1.2"}')["ok"], False)
        self.assertEqual(self.serve_one(b"{" + b"x" * (helper.WIRE_LIMIT + 8) + b"}\n")["ok"], False)
        self.assertEqual(self.serve_one(b'{"peer":"100.64.1.2"}\n', uid=1)["ok"], False)

    def test_serve_rejects_non_json_and_peer_errors(self):
        self.assertEqual(self.serve_one(b"not json\n")["ok"], False)
        self.assertEqual(self.serve_one(b'{"peer":"100.64.1.2","extra":1}\n')["ok"], False)
        self.assertEqual(self.serve_one(b'{"peer":"example.com"}\n')["ok"], False)


if __name__ == "__main__":
    unittest.main()
