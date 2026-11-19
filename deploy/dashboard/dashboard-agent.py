#!/usr/bin/python3
"""Root-owned host agent for the private dashboard.

The dashboard itself is unprivileged and sandboxed, so the few things that need
root happen here instead: named restart actions, a device reboot, and a PTY for
the browser terminal. The socket is owned by root and readable by the dashboard
group only, so the dashboard's own passkey check is the gate in front of it.

The shell always runs as the configured human account, never root. A caller can
only ask for one of the verbs below by name; there is no command channel.
"""
import base64
import contextlib
import json
import os
from pathlib import Path
import pwd
import re
import secrets
import select
import socket
import struct
import subprocess
import sys
import threading
import time

SOCKET = "/run/portfolio-dashboard-agent/agent.sock"
WIRE_LIMIT = 64 * 1024
# The PTY buffer keeps the last 64 KiB per session; offsets are absolute so a
# client can resume after a slow poll without losing track.
BUFFER_LIMIT = 64 * 1024
READ_WAIT = 8.0
IDLE_LIMIT = 20 * 60
# Restarting or rebooting returns before the unit actually stops, so the reply
# reaches the page that asked for it.
RESTART_UNITS = {
    "tailscaled": "tailscaled.service",
    "dashboard": "portfolio-dashboard.service",
    "public": "portfolio.service",
}
SHELL = "/bin/bash"
# CSI, OSC and single-character escapes, which a line-oriented page cannot draw.
ANSI = re.compile(rb"\x1b(?:\[[0-9;?]*[ -/]*[@-~]|\][^\x07]*\x07|[@-Z\\-_])")
CONTROL = re.compile(rb"[\x00-\x08\x0b-\x1f\x7f]")


class Rejected(ValueError):
    """A request the agent will not perform, with the reason for the client."""


def clean(data):
    """Printable text only: no escapes, no carriage returns, no NULs."""
    return CONTROL.sub(b"", ANSI.sub(b"", data).replace(b"\r", b""))


def account(name):
    record = pwd.getpwnam(name)
    if record.pw_uid == 0:
        raise Rejected("shell account must be an ordinary human account")
    return record


class Session:
    """One PTY and the buffer a browser reads it through."""

    def __init__(self, user, cols, rows):
        record = account(user)
        self.id = secrets.token_hex(16)
        self.lock = threading.Lock()
        self.buffer = bytearray()
        self.offset = 0
        self.seen = time.monotonic()
        self.closed = False
        pid, fd = os.forkpty()
        if pid == 0:  # child
            step = "identity"
            try:
                # A root agent cannot simply setuid down inside its sandbox:
                # dropping the group first clears CAP_SETUID and the next call
                # fails with EPERM. su does the same handoff, in the order the
                # kernel expects, and starts the account's login shell.
                os.environ["TERM"] = "xterm-256color"
                if os.getuid() == 0:
                    step = "su"
                    os.execvpe("/bin/su", ["/bin/su", "-l", record.pw_name], os.environ)
                step = "home"
                os.chdir(record.pw_dir or "/")
                os.environ.update(
                    HOME=record.pw_dir or "/",
                    USER=record.pw_name,
                    LOGNAME=record.pw_name,
                    SHELL=SHELL,
                )
                step = "shell"
                os.execvpe(SHELL, [SHELL, "-l"], os.environ)
            except BaseException as error:
                # The child's stderr is the PTY, so a failed start is visible
                # in the terminal instead of an empty session.
                with contextlib.suppress(OSError):
                    os.write(2, f"agent shell failed at {step}: {error}\n".encode())
            finally:
                os._exit(1)
        self.pid = pid
        self.fd = fd
        self.resize(cols, rows)
        threading.Thread(target=self.pump, daemon=True).start()

    def resize(self, cols, rows):
        with contextlib.suppress(OSError):
            import fcntl
            import termios

            size = struct.pack("HHHH", max(rows, 1), max(cols, 1), 0, 0)
            fcntl.ioctl(self.fd, termios.TIOCSWINSZ, size)

    def pump(self):
        while not self.closed:
            try:
                ready, _, _ = select.select([self.fd], [], [], 1.0)
                if not ready:
                    continue
                chunk = os.read(self.fd, 8192)
            except OSError:
                break
            if not chunk:
                break
            text = clean(chunk)
            with self.lock:
                self.buffer.extend(text)
                self.offset += len(text)
                if len(self.buffer) > BUFFER_LIMIT:
                    del self.buffer[: len(self.buffer) - BUFFER_LIMIT]
        with self.lock:
            self.closed = True

    def write(self, data):
        os.write(self.fd, data.encode("utf-8", "replace")[:4096])

    def wait(self, start, limit):
        deadline = time.monotonic() + limit
        while True:
            with self.lock:
                # The buffer holds the newest bytes; start may predate them.
                base = self.offset - len(self.buffer)
                if start < self.offset:
                    return bytes(self.buffer[max(start, base) - base:]), self.offset
                if self.closed:
                    return b"", self.offset
            if time.monotonic() >= deadline:
                return b"", start
            time.sleep(0.05)

    def stop(self):
        self.closed = True
        with contextlib.suppress(OSError):
            os.kill(self.pid, 9)
        with contextlib.suppress(OSError):
            os.close(self.fd)


SESSIONS = {}
SESSIONS_LOCK = threading.Lock()


def session(sid):
    if not isinstance(sid, str):
        raise Rejected("terminal session ended")
    with SESSIONS_LOCK:
        found = SESSIONS.get(sid)
    if found is None:
        raise Rejected("terminal session ended")
    return found


def reap():
    now = time.monotonic()
    with SESSIONS_LOCK:
        gone = [
            sid
            for sid, s in SESSIONS.items()
            if now - s.seen > IDLE_LIMIT or (s.closed and now - s.seen > 60)
        ]
        for sid in gone:
            SESSIONS.pop(sid).stop()


def handle(request):
    verb = request.get("verb")
    if verb == "ping":
        return {"ok": True}
    if verb == "restart":
        unit = RESTART_UNITS.get(request.get("target"))
        if unit is None:
            raise Rejected("unknown service")
        subprocess.run(
            ["/bin/systemctl", "restart", "--no-block", unit],
            check=True,
            timeout=20,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        return {"ok": True}
    if verb == "reboot":
        subprocess.run(
            ["/bin/systemctl", "reboot", "--no-block"],
            check=True,
            timeout=20,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        return {"ok": True}
    if verb == "term-open":
        user = request.get("user")
        if not isinstance(user, str) or not user:
            raise Rejected("unknown account")
        cols = request.get("cols", 100)
        rows = request.get("rows", 30)
        if not isinstance(cols, int) or not isinstance(rows, int):
            raise Rejected("invalid size")
        terminal = Session(user, min(max(cols, 20), 300), min(max(rows, 5), 120))
        with SESSIONS_LOCK:
            SESSIONS[terminal.id] = terminal
        return {"ok": True, "id": terminal.id}
    if verb == "term-read":
        terminal = session(request.get("id"))
        offset = request.get("offset", 0)
        if not isinstance(offset, int) or offset < 0:
            raise Rejected("invalid offset")
        terminal.seen = time.monotonic()
        data, end = terminal.wait(offset, READ_WAIT)
        return {
            "ok": True,
            "offset": end,
            "data": base64.b64encode(data).decode("ascii"),
            "ended": terminal.closed,
        }
    if verb == "term-write":
        terminal = session(request.get("id"))
        data = request.get("data")
        if not isinstance(data, str):
            raise Rejected("invalid input")
        terminal.seen = time.monotonic()
        terminal.write(data)
        return {"ok": True}
    if verb == "term-close":
        with SESSIONS_LOCK:
            terminal = SESSIONS.pop(request.get("id"), None)
        if terminal is not None:
            terminal.stop()
        return {"ok": True}
    raise Rejected("unknown request")


def peer_uid(conn):
    raw = conn.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i"))
    _, uid, _ = struct.unpack("3i", raw)
    return uid


def serve(conn, allowed):
    try:
        if peer_uid(conn) not in allowed:
            conn.sendall(b'{"ok":false,"error":"denied"}\n')
            return
        conn.settimeout(30)
        payload = b""
        while b"\n" not in payload and len(payload) <= WIRE_LIMIT:
            chunk = conn.recv(4096)
            if not chunk:
                break
            payload += chunk
        line, _, _ = payload.partition(b"\n")
        if not line or len(line) > WIRE_LIMIT:
            conn.sendall(b'{"ok":false,"error":"denied"}\n')
            return
        try:
            request = json.loads(line)
        except ValueError:
            conn.sendall(b'{"ok":false,"error":"denied"}\n')
            return
        if not isinstance(request, dict):
            conn.sendall(b'{"ok":false,"error":"denied"}\n')
            return
        try:
            reply = handle(request)
        except Rejected as rejected:
            reply = {"ok": False, "error": str(rejected) or "denied"}
        except Exception:
            reply = {"ok": False, "error": "unavailable"}
        conn.sendall(json.dumps(reply).encode("ascii") + b"\n")
    except OSError:
        pass
    finally:
        conn.close()


def reaper():
    while True:
        time.sleep(30)
        with contextlib.suppress(Exception):
            reap()


def main():
    user = os.environ.get("DASHBOARD_AGENT_SHELL_USER", "trey")
    account(user)  # fail at start, not on the first click
    allowed = {0}
    with contextlib.suppress(KeyError):
        allowed.add(pwd.getpwnam("portfolio-dashboard").pw_uid)
    directory = Path(SOCKET).parent
    directory.mkdir(mode=0o750, exist_ok=True)
    # The dashboard group needs to traverse this directory and open the socket.
    group = None
    with contextlib.suppress(KeyError):
        group = pwd.getpwnam("portfolio-dashboard").pw_gid
        os.chown(directory, 0, group)
    with contextlib.suppress(FileNotFoundError):
        os.unlink(SOCKET)
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(SOCKET)
    os.chmod(SOCKET, 0o660)
    if group is not None:
        os.chown(SOCKET, 0, group)
    server.listen(16)
    threading.Thread(target=reaper, daemon=True).start()
    while True:
        conn, _ = server.accept()
        threading.Thread(target=serve, args=(conn, allowed), daemon=True).start()


if __name__ == "__main__":
    try:
        main()
    except (KeyError, OSError) as error:
        print(f"dashboard agent: {error}", file=sys.stderr)
        raise SystemExit(1)
