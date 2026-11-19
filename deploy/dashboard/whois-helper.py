#!/usr/bin/python3
"""Socket-activated Tailscale identity resolver for the private dashboard.

The dashboard cannot read tailscaled's LocalAPI itself, and neither can the
gateway. This helper is the only process with that access: it maps one client
IP to the human login Tailscale already verified, and answers one bounded
request per connection on a socket restricted to the dashboard group.
"""

import http.client
import ipaddress
import json
import os
import pwd
import socket
import struct
import sys
import time
from urllib.parse import urlencode

TAILSCALED = "/run/tailscale/tailscaled.sock"
WIRE_LIMIT = 4096
LOGIN_LIMIT = 320
TIMEOUT = 2.0


class Rejected(ValueError):
    def __init__(self, reason="denied"):
        super().__init__(reason)
        self.reason = reason


def peer_address(value):
    """Accept only one IP literal; never a hostname, port or long string."""
    if not isinstance(value, str) or len(value) > 64:
        raise Rejected()
    try:
        return str(ipaddress.ip_address(value))
    except ValueError:
        raise Rejected() from None


def login_from_profile(profile):
    if not isinstance(profile, dict):
        raise Rejected()
    login = profile.get("LoginName")
    if not isinstance(login, str):
        raise Rejected()
    login = login.strip()
    if not login or len(login) > LOGIN_LIMIT or not login.isprintable():
        raise Rejected()
    return login


class _UnixConnection(http.client.HTTPConnection):
    def __init__(self, path, timeout):
        super().__init__("local-tailscaled.sock", timeout=timeout)
        self._socket_path = path

    def connect(self):
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(self.timeout)
        try:
            sock.connect(self._socket_path)
        except OSError:
            sock.close()
            raise
        self.sock = sock


def whois(peer, path=TAILSCALED, timeout=TIMEOUT):
    """Return the login for one tailnet address, or raise Rejected."""
    address = peer_address(peer)
    connection = _UnixConnection(path, timeout)
    try:
        connection.request("GET", "/localapi/v0/whois?" + urlencode({"addr": address}))
        response = connection.getresponse()
        if response.status == 404:
            raise Rejected("unknown")
        if response.status != 200:
            raise Rejected("unavailable")
        raw = response.read(WIRE_LIMIT + 1)
        if len(raw) > WIRE_LIMIT:
            raise Rejected("unavailable")
        payload = json.loads(raw)
    except Rejected:
        raise
    except (OSError, http.client.HTTPException, ValueError):
        raise Rejected("unavailable") from None
    finally:
        connection.close()
    # A tagged or shared node has no human profile; the dashboard allowlist
    # only ever matches human logins, so that stays a denial.
    return login_from_profile(payload.get("UserProfile") if isinstance(payload, dict) else None)


def handle(request, lookup=whois):
    if not isinstance(request, dict) or set(request) != {"peer"}:
        raise Rejected()
    # Validate before any lookup: the daemon only ever sees one IP literal.
    return {"ok": True, "login": lookup(peer_address(request["peer"]))}


def read_request(conn):
    raw = bytearray()
    deadline = time.monotonic() + 2
    while len(raw) <= WIRE_LIMIT:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise Rejected()
        conn.settimeout(remaining)
        chunk = conn.recv(WIRE_LIMIT + 1 - len(raw))
        if not chunk:
            raise Rejected()
        raw.extend(chunk)
        if b"\n" in raw:
            if not raw.endswith(b"\n") or len(raw) > WIRE_LIMIT:
                raise Rejected()
            return json.loads(raw.decode("utf-8"))
    raise Rejected()


def peer_uid(conn):
    """Linux peer credential of the accepted connection."""
    if not hasattr(socket, "SO_PEERCRED"):
        raise Rejected()
    return struct.unpack("3i", conn.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))[1]


def serve(listener, allowed_uid, uid_of=peer_uid, lookup=whois):
    while True:
        try:
            client, _ = listener.accept()
        except OSError:
            return
        with client:
            try:
                if uid_of(client) != allowed_uid:
                    raise Rejected()
                result = handle(read_request(client), lookup)
            except Rejected as error:
                result = {"ok": False, "error": error.reason}
            except Exception:
                result = {"ok": False, "error": "denied"}
            try:
                client.sendall(json.dumps(result, separators=(",", ":")).encode("ascii") + b"\n")
            except OSError:
                pass


def main(argv):
    if argv == ["serve"]:
        if os.environ.get("LISTEN_PID") != str(os.getpid()) or os.environ.get("LISTEN_FDS") != "1":
            raise SystemExit("Requires its systemd socket.")
        allowed_uid = pwd.getpwnam("portfolio-dashboard").pw_uid
        with socket.socket(fileno=3) as listener:
            serve(listener, allowed_uid)
    elif len(argv) == 2 and argv[0] == "whois":
        print(whois(argv[1]))
    else:
        raise SystemExit("Usage: portfolio-whois serve | whois ADDRESS")


if __name__ == "__main__":
    main(sys.argv[1:])
