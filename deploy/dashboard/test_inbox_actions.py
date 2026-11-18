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
        self.broker = actions.Broker(self.directory)

    def approve(self, operation, target):
        result = self.broker.handle({"action": "issue", "operation": operation, "id": target}, issuer=True)
        return {"action": operation, "id": target, "receipt": result["receipt"]}

    def test_scoped_once_only_approval_and_replacement(self):
        with self.assertRaises(ValueError):
            self.broker.handle({"action": "issue", "operation": "delete", "id": FIRST_ID})
        request = self.approve("delete", FIRST_ID)
        with self.assertRaises(ValueError):
            self.broker.handle({**request, "action": "delete-all"})
        with self.assertRaises(ValueError):
            self.broker.handle({**request, "id": "0" * 64})
        original = self.file.stat()
        old_acl = os.getxattr(self.file, ACL)
        self.assertTrue(self.broker.handle(request)["ok"])
        self.assertNotEqual(original.st_ino, self.file.stat().st_ino)
        self.assertEqual(self.file.read_bytes(), SECOND)
        self.assertEqual(os.getxattr(self.file, ACL), old_acl)
        with self.assertRaises(ValueError):
            self.broker.handle(request)

    def test_expiry_ambiguous_partial_and_stale_bulk(self):
        with mock.patch.object(actions, "RECEIPT_TTL", -1):
            expired = self.approve("delete", FIRST_ID)
        with self.assertRaises(ValueError):
            self.broker.handle(expired)
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
