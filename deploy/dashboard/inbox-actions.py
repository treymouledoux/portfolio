#!/usr/bin/python3
"""Socket-activated contact erasure broker, plus local operator approval client."""
import contextlib
import fcntl
import hashlib
import hmac
import json
import os
from pathlib import Path
import pwd
import re
import secrets
import socket
import stat
import struct
import sys
import tempfile
import time

STORE = Path("/var/lib/portfolio-inbox-store")
SOCKET = "/run/portfolio-inbox-actions/actions.sock"
LINE_LIMIT = 65536
ACTIONS = {"delete", "delete-all"}
RECEIPT_TTL = 300


class Stale(Exception):
    """The selected snapshot or unique row is no longer available."""


@contextlib.contextmanager
def locked_store(directory, exclusive):
    flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
    with os.fdopen(os.open(directory / "submissions.lock", flags), "rb") as lock:
        if not stat.S_ISREG(os.fstat(lock.fileno()).st_mode):
            raise OSError("unexpected lock type")
        mode = fcntl.LOCK_EX if exclusive else fcntl.LOCK_SH
        deadline = time.monotonic() + 2
        while True:
            try:
                fcntl.flock(lock, mode | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    raise TimeoutError("store busy")
                time.sleep(0.02)
        with os.fdopen(os.open(directory / "submissions.jsonl", flags), "rb") as data:
            info = os.fstat(data.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
                raise OSError("unexpected data file")
            yield data


def scan(data, selected=None, output=None):
    digest = hashlib.sha256()
    count = matches = 0
    while line := data.readline(LINE_LIMIT + 2):
        if not line.endswith(b"\n") or len(line) > LINE_LIMIT + 1:
            raise OSError("incomplete or oversized record")
        digest.update(line)
        count += 1
        if selected is not None and hashlib.sha256(line[:-1]).hexdigest() == selected:
            matches += 1
        elif output is not None:
            output.write(line)
    return digest.hexdigest(), count, matches


def check_target(action, target, digest, count, matches):
    if action == "delete" and matches != 1:
        raise Stale()
    if action == "delete-all" and (count == 0 or not hmac.compare_digest(target, digest)):
        raise Stale()


def erase(directory, action, target):
    with locked_store(directory, exclusive=True) as data:
        fd, temporary = tempfile.mkstemp(dir=directory, prefix=".inbox-")
        try:
            with os.fdopen(fd, "wb") as replacement:
                if action == "delete":
                    result = scan(data, selected=target, output=replacement)
                else:
                    result = scan(data)
                check_target(action, target, *result)
                replacement.flush()
                # Only this ACL is copied. The previous file belongs to this
                # broker; its writer has data permission only.
                acl_name = "system.posix_acl_access"
                os.setxattr(replacement.fileno(), acl_name, os.getxattr(data.fileno(), acl_name))
                os.fsync(replacement.fileno())
            os.replace(temporary, directory / "submissions.jsonl")
            dir_fd = os.open(directory, os.O_DIRECTORY | os.O_RDONLY)
            try:
                os.fsync(dir_fd)
            finally:
                os.close(dir_fd)
        finally:
            Path(temporary).unlink(missing_ok=True)


class Broker:
    def __init__(self, directory):
        self.directory = directory
        self.receipts = {}

    def handle(self, request, issuer=False):
        if not isinstance(request, dict):
            raise ValueError()
        action = request.get("action")
        if action == "snapshot" and set(request) == {"action"}:
            with locked_store(self.directory, exclusive=False) as data:
                digest, count, _ = scan(data)
            return {"ok": True, "snapshot": digest, "count": count}

        target = request.get("id")
        if not isinstance(target, str) or not re.fullmatch("[0-9a-f]{64}", target):
            raise ValueError()
        now = time.monotonic()
        self.receipts = {k: v for k, v in self.receipts.items() if v[2] > now}
        if issuer and action == "issue" and set(request) == {"action", "operation", "id"}:
            operation = request["operation"]
            if operation not in ACTIONS:
                raise ValueError()
            with locked_store(self.directory, exclusive=False) as data:
                check_target(operation, target, *scan(data, selected=target))
            if len(self.receipts) >= 128:
                raise OSError("too many pending approvals")
            code = secrets.token_hex(16)
            self.receipts[code] = (operation, target, now + RECEIPT_TTL)
            return {"ok": True, "receipt": code}

        if action not in ACTIONS or set(request) != {"action", "id", "receipt"}:
            raise ValueError()
        code = request["receipt"]
        if not isinstance(code, str) or not re.fullmatch("[0-9a-f]{32}", code):
            raise ValueError()
        approval = self.receipts.get(code)
        if approval is None or approval[:2] != (action, target):
            raise ValueError()
        del self.receipts[code]
        erase(self.directory, action, target)
        return {"ok": True}


def read_object(conn):
    raw = bytearray()
    deadline = time.monotonic() + 2
    while len(raw) <= 1024:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError()
        conn.settimeout(remaining)
        chunk = conn.recv(1025 - len(raw))
        if not chunk:
            break
        raw.extend(chunk)
        if b"\n" in raw:
            if not raw.endswith(b"\n") or len(raw) > 1024:
                break
            return json.loads(raw.decode("ascii"))
    raise ValueError()


def send(conn, body):
    conn.settimeout(2)
    conn.sendall(json.dumps(body, separators=(",", ":")).encode("ascii") + b"\n")


def serve():
    if os.environ.get("LISTEN_PID") != str(os.getpid()) or os.environ.get("LISTEN_FDS") != "1":
        raise SystemExit("Requires its systemd socket.")
    broker = Broker(STORE)
    # A hard power loss can leave a pre-rename temporary file. Only the
    # broker has directory write permission; clear its interrupted rewrites.
    for orphan in STORE.glob(".inbox-*"):
        entry = orphan.lstat()
        if stat.S_ISREG(entry.st_mode) and entry.st_uid == os.geteuid():
            orphan.unlink()
    web_uid = pwd.getpwnam("portfolio-dashboard").pw_uid
    with socket.socket(fileno=3) as listener:
        while True:
            client, _ = listener.accept()
            with client:
                try:
                    _, uid, _ = struct.unpack("3i", client.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
                    if uid not in {0, web_uid}:
                        raise ValueError()
                    result = broker.handle(read_object(client), issuer=uid == 0)
                except Stale:
                    result = {"ok": False, "error": "stale"}
                except (ValueError, TypeError, RecursionError):
                    result = {"ok": False, "error": "denied"}
                except OSError:
                    result = {"ok": False, "error": "unavailable"}
                try:
                    send(client, result)
                except OSError:
                    pass


def issue(operation, target):
    if os.geteuid() != 0 or operation not in ACTIONS or not re.fullmatch("[0-9a-f]{64}", target):
        raise SystemExit("Run the displayed confirmation command through sudo on the Pi.")
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as conn:
        conn.settimeout(5)
        conn.connect(SOCKET)
        send(conn, {"action": "issue", "operation": operation, "id": target})
        response = read_object(conn)
    if not response.get("ok"):
        raise SystemExit("No approval issued. Refresh the selection and try again.")
    print(response["receipt"])


if __name__ == "__main__":
    if sys.argv[1:] == ["serve"]:
        serve()
    elif len(sys.argv) == 4 and sys.argv[1] == "issue":
        issue(sys.argv[2], sys.argv[3])
    else:
        raise SystemExit("Usage: portfolio-inbox-actions issue {delete|delete-all} DIGEST")
