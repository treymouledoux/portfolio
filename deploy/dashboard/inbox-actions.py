#!/usr/bin/python3
"""Socket-activated passkey-protected contact erasure broker."""
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
import sqlite3
import struct
import sys
import tempfile
import time
from urllib.parse import urlsplit

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric import ec
from fido2.features import webauthn_json_mapping
from fido2.server import Fido2Server
from fido2.webauthn import (
    AttestedCredentialData, AuthenticationResponse, PublicKeyCredentialUserEntity,
    RegistrationResponse, ResidentKeyRequirement, UserVerificationRequirement,
)

webauthn_json_mapping.enabled = True

STORE = Path("/var/lib/portfolio-inbox-store")
SOCKET = "/run/portfolio-inbox-actions/actions.sock"
LINE_LIMIT = 65536
ACTIONS = {"delete", "delete-all"}
WIRE_LIMIT = 32 * 1024


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


class Rejected(ValueError):
    def __init__(self, reason="denied"):
        super().__init__(reason)
        self.reason = reason


def hex_token(value):
    return isinstance(value, str) and re.fullmatch("[0-9a-f]{64}", value) is not None


def private_origin(origin):
    """One exact lowercase HTTPS origin: the Serve host on 8443 or the gateway
    hostname. No path, userinfo, fragment or trailing slash."""
    match = re.fullmatch(
        r"https://((?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)"
        r"(?::([0-9]{1,5}))?",
        origin,
    )
    if match is None:
        return False
    port = match.group(2)
    return port is None or 0 < int(port) <= 65535


class Broker:
    def __init__(self, directory, key_database, origin, admins, clock=time.monotonic, *, readers=None):
        if not private_origin(origin):
            raise ValueError("expected one exact private dashboard HTTPS origin")
        self.directory = directory
        self.origin = origin
        self.admins = frozenset(admins)
        self.readers = self.admins if readers is None else frozenset(readers)
        self.clock = clock
        self.server = Fido2Server(
            {"id": urlsplit(origin).hostname, "name": "Portfolio dashboard"},
            verify_origin=lambda value: value == origin,
            attestation="none",
        )
        self.server.timeout = 120_000
        self.server.allowed_algorithms = [p for p in self.server.allowed_algorithms if p.alg == -7]
        self.keys = sqlite3.connect(key_database)
        self.keys.execute("""CREATE TABLE IF NOT EXISTS credentials (
            login TEXT PRIMARY KEY, origin TEXT NOT NULL, handle BLOB NOT NULL,
            credential BLOB NOT NULL, counter INTEGER NOT NULL
        )""")
        self.windows = {}
        self.flows = {}
        self.sessions = {}
        self.holds = {}

    def prune(self, now):
        for table in (self.windows, self.flows, self.sessions, self.holds):
            for key, values in list(table.items()):
                if values[0] <= now:
                    del table[key]

    def credential(self, user):
        row = self.keys.execute(
            "SELECT origin, handle, credential, counter FROM credentials WHERE login = ?", (user,)
        ).fetchone()
        if row and row[0] != self.origin:
            raise Rejected()
        return row

    def begin(self, user, enroll):
        record = self.credential(user)
        if enroll:
            if user not in self.windows or record:
                raise Rejected("closed")
            handle = os.urandom(32)
            options, state = self.server.register_begin(
                PublicKeyCredentialUserEntity(id=handle, name=user, display_name=user),
                resident_key_requirement=ResidentKeyRequirement.PREFERRED,
                user_verification=UserVerificationRequirement.REQUIRED,
            )
        else:
            if not record:
                raise Rejected("unregistered")
            handle = record[1]
            options, state = self.server.authenticate_begin(
                [AttestedCredentialData(record[2])], user_verification=UserVerificationRequirement.REQUIRED
            )
        if len(self.flows) >= 128:
            raise Rejected()
        flow = secrets.token_hex(32)
        self.flows[flow] = (self.clock() + 120, user, enroll, state, handle)
        return {"ok": True, "flow": flow, "publicKey": dict(options.public_key)}

    def finish(self, request, user, enroll):
        flow = self.flows.pop(request["flow"], None)
        if not flow or (user, enroll) != flow[1:3]:
            raise Rejected()
        supplied = request["credential"]
        if not isinstance(supplied, dict) or supplied.get("type") != "public-key":
            raise Rejected()
        if not isinstance(supplied.get("id"), str) or supplied["id"] != supplied.get("rawId"):
            raise Rejected()
        # Do not parse optional untrusted extension payloads; this ceremony
        # requested none. Both verification methods still parse the signed data.
        response = {"id": supplied["id"], "response": supplied["response"], "type": "public-key"}
        parsed = (RegistrationResponse if enroll else AuthenticationResponse).from_dict(response)
        client = parsed.response.client_data
        if client.cross_origin is not False or "topOrigin" in json.loads(client):
            raise Rejected()
        record = self.credential(user)
        if enroll:
            if user not in self.windows or record:
                raise Rejected("closed")
            auth = self.server.register_complete(flow[3], parsed)
            credential = auth.credential_data
            if credential is None or credential.credential_id != parsed.id:
                raise Rejected()
            key = credential.public_key
            if set(key) != {1, 3, -1, -2, -3} or (key[1], key[3], key[-1]) != (2, -7, 1):
                raise Rejected()
            if any(not isinstance(key[k], bytes) or len(key[k]) != 32 for k in (-2, -3)):
                raise Rejected()
            # Validate the enrolled point before committing the single credential.
            ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256R1(), b"\x04" + key[-2] + key[-3])
            with self.keys:
                self.keys.execute("INSERT INTO credentials VALUES (?, ?, ?, ?, ?)",
                                  (user, self.origin, flow[4], bytes(credential), auth.counter))
            del self.windows[user]
            # A fresh assertion proves possession before any session is issued.
            return {"ok": True, "registered": True}

        if not record or parsed.response.user_handle not in (None, record[1]):
            raise Rejected()
        self.server.authenticate_complete(flow[3], [AttestedCredentialData(record[2])], parsed)
        counter = parsed.response.authenticator_data.counter
        if (record[3] or counter) and counter <= record[3]:
            raise Rejected()
        with self.keys:
            self.keys.execute("UPDATE credentials SET counter = ? WHERE login = ?", (counter, user))
        if len(self.sessions) >= 128:
            raise Rejected()
        session = secrets.token_hex(32)
        self.sessions[session] = (self.clock() + 900, user)
        return {"ok": True, "session": session}

    def handle(self, request, issuer=False):
        if not isinstance(request, dict):
            raise Rejected()
        now = self.clock()
        self.prune(now)
        action = request.get("action")
        fields = set(request)
        user = request.get("user")
        if not isinstance(user, str) or user not in self.readers:
            raise Rejected()
        basic = {"action", "user"}
        if action == "open-enrollment" and issuer and fields == basic:
            if self.credential(user):
                raise Rejected("closed")
            self.windows.setdefault(user, (now + 300,))
            return {"ok": True}
        if action == "enrollment" and fields == basic:
            # Read-only gate for the unlock screen: setup is only offered
            # while a root operator has an enrollment window open.
            return {"ok": True, "open": user in self.windows}
        if action in ("enroll-options", "login-options") and fields == basic:
            return self.begin(user, action == "enroll-options")
        if action in ("enroll-finish", "login-finish") and fields == basic | {"flow", "credential"}:
            return self.finish(request, user, action == "enroll-finish")

        session = request.get("session")
        if not hex_token(session) or self.sessions.get(session, (None, None))[1] != user:
            raise Rejected()
        basic.add("session")
        if action == "session" and fields == basic:
            return {"ok": True}
        if action == "lock" and fields == basic:
            del self.sessions[session]
            for key, value in list(self.holds.items()):
                if value[1] == session:
                    del self.holds[key]
            return {"ok": True}
        if user not in self.admins:
            raise Rejected()
        if action == "snapshot" and fields == basic:
            with locked_store(self.directory, exclusive=False) as data:
                digest, count, _ = scan(data)
            return {"ok": True, "snapshot": digest, "count": count}
        hold = request.get("hold")
        if action == "cancel" and fields == basic | {"hold"}:
            previous = self.holds.get(hold)
            if previous and previous[1] == session:
                del self.holds[hold]
            return {"ok": True}
        operation, target = request.get("operation"), request.get("id")
        if operation not in ACTIONS or not hex_token(target):
            raise Rejected()
        basic.update(("operation", "id"))
        if action == "arm" and fields == basic:
            with locked_store(self.directory, exclusive=False) as data:
                check_target(operation, target, *scan(data, selected=target))
            # Each new press invalidates earlier holds in this session.
            for key, value in list(self.holds.items()):
                if value[1] == session:
                    del self.holds[key]
            hold = secrets.token_hex(32)
            started = self.clock()
            self.holds[hold] = (started + 30, session, operation, target, started + 5)
            return {"ok": True, "hold": hold}
        if action != "commit" or fields != basic | {"hold"} or not hex_token(hold):
            raise Rejected()
        previous = self.holds.get(hold)
        if not previous or previous[1:4] != (session, operation, target):
            raise Rejected()
        if now < previous[4]:
            raise Rejected("early")
        del self.holds[hold]
        erase(self.directory, operation, target)
        return {"ok": True}


def read_object(conn):
    raw = bytearray()
    deadline = time.monotonic() + 2
    while len(raw) <= WIRE_LIMIT:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError()
        conn.settimeout(remaining)
        chunk = conn.recv(WIRE_LIMIT + 1 - len(raw))
        if not chunk:
            break
        raw.extend(chunk)
        if b"\n" in raw:
            if not raw.endswith(b"\n") or len(raw) > WIRE_LIMIT:
                break
            return json.loads(raw.decode("utf-8"))
    raise ValueError()


def send(conn, body):
    conn.settimeout(2)
    conn.sendall(json.dumps(body, separators=(",", ":")).encode("ascii") + b"\n")


def serve():
    if os.environ.get("LISTEN_PID") != str(os.getpid()) or os.environ.get("LISTEN_FDS") != "1":
        raise SystemExit("Requires its systemd socket.")
    broker = Broker(
        STORE, "/var/lib/portfolio-inbox-auth/credentials.sqlite3",
        os.environ["DASHBOARD_ADMIN_ORIGIN"],
        [v.strip() for v in os.environ["DASHBOARD_INBOX_ADMINS"].split(",") if v.strip()],
        readers=[v.strip() for v in os.environ["DASHBOARD_INBOX_READERS"].split(",") if v.strip()],
    )
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
                except Rejected as e:
                    result = {"ok": False, "error": e.reason}
                except (ValueError, TypeError, KeyError, IndexError, RecursionError,
                        OverflowError, AssertionError, InvalidSignature, struct.error):
                    result = {"ok": False, "error": "denied"}
                except (OSError, sqlite3.Error):
                    result = {"ok": False, "error": "unavailable"}
                try:
                    send(client, result)
                except OSError:
                    pass


def open_enrollment(user):
    if os.geteuid() != 0:
        raise SystemExit("Open the enrollment window as the local root operator.")
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as conn:
        conn.settimeout(5)
        conn.connect(SOCKET)
        send(conn, {"action": "open-enrollment", "user": user})
        response = read_object(conn)
    if not response.get("ok"):
        raise SystemExit("Enrollment unavailable. Check the configured owner and existing credential.")
    print("Initial passkey enrollment is open for up to five minutes.")


if __name__ == "__main__":
    if sys.argv[1:] == ["serve"]:
        serve()
    elif len(sys.argv) == 3 and sys.argv[1] == "open-enrollment":
        open_enrollment(sys.argv[2])
    else:
        raise SystemExit("Usage: portfolio-inbox-actions open-enrollment OWNER_LOGIN")
