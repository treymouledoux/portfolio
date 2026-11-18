#!/usr/bin/python3 -I
"""Forced OpenSSH command, installed root-owned and executed as portfolio-deploy."""
import fcntl
import hashlib
import http.client
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import zipfile

BASE = Path("/srv/portfolio/public")
RELEASE_ID = r"[0-9]{1,20}-[0-9]{1,10}"
MAX_ARCHIVE = 64 * 1024 * 1024
MAX_CONTENT = 192 * 1024 * 1024


def sync_dir(directory):
    fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def upload_timeout(_signum, _frame):
    raise TimeoutError("Upload deadline exceeded")


def receive(stream, archive_path, expected):
    digest = hashlib.sha256()
    with archive_path.open("xb") as archive:
        while chunk := stream.read(256 * 1024):
            if archive.tell() + len(chunk) > MAX_ARCHIVE:
                raise ValueError("Archive exceeds upload limit")
            digest.update(chunk)
            archive.write(chunk)
    if digest.hexdigest() != expected:
        raise ValueError("Checksum mismatch")


def unpack(archive_path, destination):
    required = {"portfolio-serve", "assets/index.html", "assets/resume.pdf"}
    with zipfile.ZipFile(archive_path) as archive:
        entries = archive.infolist()
        if len(entries) > 5000 or sum(entry.file_size for entry in entries) > MAX_CONTENT:
            raise ValueError("Release exceeds extraction limits")
        for entry in entries:
            name = entry.filename.removesuffix("/")
            parts = name.split("/")
            if (parts[0] != "assets" and name != "portfolio-serve"
                    or any(part in ("", ".", "..") for part in parts)
                    or entry.orig_filename != entry.filename or "\\" in name
                    or not name.isprintable() or len(name) > 1024):
                raise ValueError("Invalid release member")
            if entry.compress_type not in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED):
                raise ValueError("Unsupported release compression")
            file_type = stat.S_IFMT(entry.external_attr >> 16)
            if file_type not in (0, stat.S_IFDIR if entry.is_dir() else stat.S_IFREG):
                raise ValueError("Release contains a special file")
            path = destination / name
            if entry.is_dir():
                path.mkdir(parents=True, exist_ok=True)
                continue
            path.parent.mkdir(parents=True, exist_ok=True)
            with archive.open(entry) as source, path.open("xb") as output:
                shutil.copyfileobj(source, output)
                output.flush()
                os.fchmod(output.fileno(), 0o555 if name == "portfolio-serve" else 0o444)
                os.fsync(output.fileno())
            if entry.file_size:
                required.discard(name)
    if required:
        raise ValueError("Release is incomplete")
    for path, _, _ in os.walk(destination, topdown=False):
        Path(path).chmod(0o755)
        sync_dir(path)


def restart_and_check():
    subprocess.run(["/usr/bin/sudo", "-n", "/usr/bin/systemctl", "restart", "portfolio.service"],
                   check=True, timeout=90)
    for attempt in range(20):
        conn = http.client.HTTPConnection("127.0.0.1", 6050, timeout=2)
        try:
            conn.request("GET", "/healthz")
            response = conn.getresponse()
            if response.status == 200 and response.read(5) in (b"ok", b"ok\n", b"ok\r\n"):
                return
        except (OSError, http.client.HTTPException):
            pass
        finally:
            conn.close()
        if attempt < 19:
            time.sleep(1)
    raise RuntimeError("Public service failed health check")


def switch(base, target):
    if not re.fullmatch(r"releases/" + RELEASE_ID, target) or not (base / target).is_dir():
        raise ValueError("Invalid rollback release")
    temporary = base / ".current.next"
    temporary.unlink(missing_ok=True)
    temporary.symlink_to(target)
    os.replace(temporary, base / "current")
    sync_dir(base)


def activate(base, command, stream, restart):
    match = re.fullmatch(r"deploy (" + RELEASE_ID + r") ([0-9a-f]{64})", command)
    if not match:
        raise ValueError("Expected an isolated deployment command")
    release_id, digest = match.groups()
    releases = base / "releases"
    with (base / ".deploy.lock").open("a") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        pending = base / ".rollback-target"
        if pending.exists():
            switch(base, pending.read_text())
            restart()
            pending.unlink()
            sync_dir(base)
        previous = os.readlink(base / "current")
        if not re.fullmatch(r"releases/" + RELEASE_ID, previous) or not (base / previous).is_dir():
            raise ValueError("Provision a known-good seed release first")
        target = releases / release_id
        if target.exists():
            raise ValueError("Release ID already used; rerun with a fresh attempt")
        with tempfile.TemporaryDirectory(prefix=".upload-", dir=base) as temp:
            workspace = Path(temp)
            archive = workspace / "release.zip"
            signal.alarm(180)
            try:
                receive(stream, archive, digest)
            finally:
                signal.alarm(0)
            stage = workspace / "stage"
            stage.mkdir()
            unpack(archive, stage)
            stage.rename(target)
            sync_dir(releases)
            marker = workspace / "previous"
            with marker.open("x") as saved:
                saved.write(previous)
                saved.flush()
                os.fsync(saved.fileno())
            os.replace(marker, pending)
            sync_dir(base)
            try:
                switch(base, "releases/" + release_id)
                restart()
            except BaseException:
                print("Activation failed; restoring previous release", file=sys.stderr)
                switch(base, previous)
                restart()
                pending.unlink()
                sync_dir(base)
                raise
            pending.unlink()
            sync_dir(base)


if __name__ == "__main__":
    os.umask(0o077)
    signal.signal(signal.SIGALRM, upload_timeout)
    try:
        activate(BASE, os.environ.get("SSH_ORIGINAL_COMMAND", ""), sys.stdin.buffer, restart_and_check)
    except Exception as error:
        sys.exit(f"Deployment failed: {type(error).__name__}: {error}")
    print("Activated and verified public release")
