"""Synthetic erasure, approval, crash and migration checks; no network access."""
import concurrent.futures
import fcntl
import hashlib
import importlib.util
import os
from pathlib import Path
import pwd
import struct
import sys
import tempfile
import threading
import unittest

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import ec
from fido2.cose import ES256
from fido2.utils import websafe_encode
from fido2.webauthn import AttestationObject, AttestedCredentialData, AuthenticatorData, CollectedClientData
from unittest import mock


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


actions = load("inbox-actions")
migration = load("migrate-inbox")
FIRST = b'{"name":"synthetic A"}\n'
SECOND = b'{"name":"synthetic B"}\n'
FIRST_ID = hashlib.sha256(FIRST[:-1]).hexdigest()
ACL = "system.posix_acl_access"


class ActionsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.file = self.directory / "submissions.jsonl"
        self.file.write_bytes(FIRST + SECOND)
        (self.directory / "submissions.lock").touch()
        if sys.platform.startswith("linux"):
            uid = pwd.getpwnam("nobody").pw_uid
            entries = [(1, 6, -1), (2, 4, uid), (4, 0, -1), (16, 4, -1), (32, 0, -1)]
            acl = struct.pack("<I", 2) + b"".join(struct.pack("<HHi", *e) for e in entries)
            os.setxattr(self.file, ACL, acl)
        else:
            for name in ("getxattr", "setxattr"):
                patch = mock.patch.object(os, name, create=True, return_value=b"fixture")
                patch.start()
                self.addCleanup(patch.stop)
        self.now = 100.0
        self.user = "owner@example.invalid"
        self.origin = "https://private.example.ts.net:8443"
        self.database = self.directory / "private.db"
        self.broker = self.start_broker()
        self.private_key = ec.generate_private_key(ec.SECP256R1())
        self.credential_id = os.urandom(32)
        self.rp_hash = hashlib.sha256(b"private.example.ts.net").digest()
        self.enroll()
        self.session = self.login()

    def start_broker(self):
        broker = actions.Broker(self.directory, self.database, self.origin, {self.user}, lambda: self.now)
        self.addCleanup(broker.keys.close)
        return broker

    def call(self, action, **kwargs):
        return self.broker.handle({"action": action, "user": self.user, **kwargs})

    def ceremony(self, kind, **overrides):
        options = self.call(f"{kind}-options")
        public = options["publicKey"]
        self.assertNotIn("publicKey", public)
        create = kind == "enroll"
        client = CollectedClientData.create(
            "webauthn.create" if create else "webauthn.get",
            overrides.get("challenge", public["challenge"]),
            overrides.get("origin", self.origin),
            cross_origin=overrides.get("cross", False),
        )
        flags = 1 | (0 if overrides.get("no_uv") else 4)
        base = {"clientDataJSON": websafe_encode(client)}
        if create:
            self.assertEqual(public["authenticatorSelection"]["userVerification"], "required")
            key = ES256.from_cryptography_key(self.private_key.public_key())
            credential = AttestedCredentialData.create(bytes(16), self.credential_id, key)
            auth = AuthenticatorData.create(self.rp_hash, flags | 64, 0, credential)
            base["attestationObject"] = websafe_encode(AttestationObject.create("none", auth, {}))
        else:
            self.assertEqual(public["userVerification"], "required")
            self.assertEqual(public["allowCredentials"][0]["id"], websafe_encode(self.credential_id))
            auth = AuthenticatorData.create(overrides.get("rp_hash", self.rp_hash), flags, overrides.get("counter", 0))
            signature = self.private_key.sign(auth + client.hash, ec.ECDSA(hashes.SHA256()))
            base.update(authenticatorData=websafe_encode(auth),
                        signature=websafe_encode(signature if not overrides.get("bad_signature") else bytes(64)))
        key_id = websafe_encode(self.credential_id)
        return {"flow": options["flow"], "credential": {
            "id": key_id, "rawId": key_id, "type": "public-key", "response": base,
        }}

    def enroll(self):
        with self.assertRaises(actions.Rejected):
            self.call("enroll-options")
        open_request = {"action": "open-enrollment", "user": self.user}
        with self.assertRaises(actions.Rejected):
            self.broker.handle(open_request)
        self.broker.handle(open_request, issuer=True)
        result = self.call("enroll-finish", **self.ceremony("enroll"))
        self.assertEqual(result, {"ok": True, "registered": True})
        with self.assertRaises(actions.Rejected):
            self.broker.handle(open_request, issuer=True)

    def login(self):
        response = self.ceremony("login")
        session = self.call("login-finish", **response)["session"]
        with self.assertRaises(actions.Rejected):
            self.call("login-finish", **response)
        return session

    def approve(self, operation, target, wait=True):
        args = {"operation": operation, "id": target, "session": self.session}
        hold = self.call("arm", **args)["hold"]
        if wait:
            self.now += 5
        return {"action": "commit", "user": self.user, **args, "hold": hold}

    def test_passkey_origin_uv_binding_signature_counter_and_restart(self):
        for change in ({"origin": "https://example.invalid"}, {"no_uv": True}, {"cross": True},
                       {"challenge": websafe_encode(bytes(32))}, {"rp_hash": bytes(32)}, {"bad_signature": True}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.call("login-finish", **self.ceremony("login", **change))
        self.call("login-finish", **self.ceremony("login", counter=3))
        with self.assertRaises(actions.Rejected):
            self.call("login-finish", **self.ceremony("login", counter=2))
        self.broker = self.start_broker()
        with self.assertRaises(actions.Rejected):
            self.call("session", session=self.session)
        self.call("login-finish", **self.ceremony("login", counter=4))

    def test_challenge_expiry_and_enrollment_window_does_not_extend(self):
        expired = self.ceremony("login")
        self.now += 121
        with self.assertRaises(actions.Rejected):
            self.call("login-finish", **expired)
        fresh = self.directory / "fresh.db"
        self.broker = actions.Broker(self.directory, fresh, self.origin, {self.user}, lambda: self.now)
        self.addCleanup(self.broker.keys.close)
        open_request = {"action": "open-enrollment", "user": self.user}
        self.broker.handle(open_request, issuer=True)
        self.now += 290
        self.broker.handle(open_request, issuer=True)
        near_expiry = self.ceremony("enroll")
        self.now += 11
        with self.assertRaisesRegex(actions.Rejected, "closed"):
            self.call("enroll-finish", **near_expiry)
        with self.assertRaises(actions.Rejected):
            self.broker.handle({"action": "open-enrollment", "user": "another"}, issuer=True)

    def test_timing_cancel_replacement_and_expiry(self):
        early = self.approve("delete", FIRST_ID, wait=False)
        self.now += 4.999
        with self.assertRaisesRegex(actions.Rejected, "early"):
            self.broker.handle(early)
        self.call("cancel", session=self.session, hold=early["hold"])
        self.now += 1
        with self.assertRaises(actions.Rejected):
            self.broker.handle(early)
        first = self.approve("delete", FIRST_ID)
        later = self.approve("delete", FIRST_ID)
        with self.assertRaises(actions.Rejected):
            self.broker.handle(first)
        self.now += 31
        with self.assertRaises(actions.Rejected):
            self.broker.handle(later)
        self.now += 900
        with self.assertRaises(actions.Rejected):
            self.call("arm", session=self.session, operation="delete", id=FIRST_ID)

    def test_scoped_once_only_approval_and_replacement(self):
        request = self.approve("delete", FIRST_ID)
        with self.assertRaises(ValueError):
            self.broker.handle({**request, "operation": "delete-all"})
        with self.assertRaises(ValueError):
            self.broker.handle({**request, "id": "0" * 64})
        with self.assertRaises(ValueError):
            self.broker.handle({**request, "session": self.login()})
        original = self.file.stat()
        old_acl = os.getxattr(self.file, ACL)
        self.assertTrue(self.broker.handle(request)["ok"])
        self.assertNotEqual(original.st_ino, self.file.stat().st_ino)
        self.assertEqual(self.file.read_bytes(), SECOND)
        self.assertEqual(os.getxattr(self.file, ACL), old_acl)
        with self.assertRaises(ValueError):
            self.broker.handle(request)

    def test_expiry_ambiguous_partial_and_stale_bulk(self):
        snap = self.broker.handle({"action": "snapshot"})
        self.assertEqual(snap["count"], 2)
        bulk = self.approve("delete-all", snap["snapshot"])
        with self.file.open("ab") as f:
            f.write(FIRST)
        with self.assertRaises(actions.Stale):
            self.broker.handle(bulk)
        with self.assertRaises(actions.Stale):
            self.approve("delete", FIRST_ID)
        self.assertEqual(self.file.read_bytes(), FIRST + SECOND + FIRST)
        with self.file.open("ab") as f:
            f.write(b"partial")
        with self.assertRaises(OSError):
            self.broker.handle({"action": "snapshot"})

    def test_bulk_and_retry_after_failed_replacement(self):
        snap = self.broker.handle({"action": "snapshot"})
        bulk = self.approve("delete-all", snap["snapshot"])
        with mock.patch.object(os, "replace", side_effect=OSError("simulated failure")):
            with self.assertRaises(OSError):
                self.broker.handle(bulk)
        self.assertEqual(self.file.read_bytes(), FIRST + SECOND)
        self.assertEqual(list(self.directory.glob(".inbox-*")), [])
        bulk = self.approve("delete-all", snap["snapshot"])
        self.assertTrue(self.broker.handle(bulk)["ok"])
        self.assertEqual(self.file.read_bytes(), b"")

    def test_concurrent_writer_appends_to_latest_inode(self):
        request = self.approve("delete", FIRST_ID)
        rewriting, waiting = threading.Event(), threading.Event()
        original = actions.scan

        def scan(*args, **kwargs):
            rewriting.set()
            self.assertTrue(waiting.wait(2))
            return original(*args, **kwargs)

        def append():
            self.assertTrue(rewriting.wait(2))
            with (self.directory / "submissions.lock").open("rb") as lock:
                waiting.set()
                fcntl.flock(lock, fcntl.LOCK_EX)
                with self.file.open("ab") as current:
                    current.write(b"new\n")

        with concurrent.futures.ThreadPoolExecutor() as pool:
            future = pool.submit(append)
            with mock.patch.object(actions, "scan", side_effect=scan):
                self.broker.handle(request)
            future.result()
        self.assertEqual(self.file.read_bytes(), SECOND + b"new\n")


class MigrationTest(unittest.TestCase):
    def test_migration_with_optional_previous_pin_and_restart(self):
        account = pwd.getpwuid(os.geteuid())
        for has_pin in (False, True):
            with self.subTest(has_pin=has_pin), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                source, pin, store = root / "old", root / "pin", root / "store"
                source.write_bytes(FIRST)
                original = source.stat()
                if has_pin:
                    os.link(source, pin)
                with mock.patch.object(migration, "set_acl"), mock.patch.object(os, "fchown"):
                    for _ in range(2):
                        migration.migrate(source, pin, store, account, account, account)
                self.assertFalse(source.exists())
                self.assertFalse(pin.exists())
                new = store / "submissions.jsonl"
                self.assertEqual(new.stat().st_nlink, 1)
                self.assertEqual(new.stat().st_ino, original.st_ino)
                self.assertEqual(new.read_bytes(), FIRST)


if __name__ == "__main__":
    unittest.main()
