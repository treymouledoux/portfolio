#!/usr/bin/python3
"""Certbot deploy hook for the private apex gateway on a systemd Linux Pi."""

import fcntl
import grp
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


DOMAIN = "treymouledoux.me"
LINEAGE = Path("/etc/letsencrypt/live/portfolio-apex")
CADDY_DIR = Path("/etc/caddy")
CADDY = "/usr/local/libexec/portfolio-caddy"
UNIT = "portfolio-gateway.service"
TRUST_STORE = "/etc/ssl/certs/ca-certificates.crt"
OPENSSL = "/usr/bin/openssl"


def output(*command):
    return subprocess.check_output(command)


def verify(generation):
    # Validate the staged bytes that Caddy will read, including during a
    # manual hook retry that overlaps another Certbot lineage rotation.
    cert = generation / "apex-fullchain.pem"
    key = generation / "apex-key.pem"
    subprocess.run([OPENSSL, "verify", "-CAfile", TRUST_STORE, "-untrusted", cert,
                    "-purpose", "sslserver", "-verify_hostname", DOMAIN, cert], check=True)
    subprocess.run([OPENSSL, "x509", "-in", cert, "-checkend", "604800", "-noout"], check=True)
    san = output(OPENSSL, "x509", "-in", cert, "-noout", "-ext", "subjectAltName")
    if b" ".join(san.splitlines()[1:]).strip() != f"DNS:{DOMAIN}".encode():
        raise ValueError("Expected only the exact apex DNS name in the certificate")
    if output(OPENSSL, "x509", "-in", cert, "-pubkey", "-noout") != output(
            OPENSSL, "pkey", "-in", key, "-pubout"):
        raise ValueError("Certificate and private key do not match")


def sync_directory(path):
    fd = os.open(path, os.O_DIRECTORY | os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def replace_link(link, target):
    # All operations take place on the same local filesystem as the live link.
    with tempfile.TemporaryDirectory(dir=link.parent, prefix=".link-") as tmp:
        pending = Path(tmp) / "next"
        pending.symlink_to(target)
        os.replace(pending, link)
    sync_directory(link.parent)


def write_owned(path, data, gid):
    with path.open("xb") as dest:
        os.fchown(dest.fileno(), -1, gid)
        os.fchmod(dest.fileno(), 0o640)
        dest.write(data)
        dest.flush()
        os.fsync(dest.fileno())


def gateway_state():
    if output("/usr/bin/systemctl", "show", "-p", "LoadState", "--value", UNIT).strip() != b"loaded":
        raise RuntimeError("Install the reviewed gateway unit before installing its certificate")
    state = output("/usr/bin/systemctl", "show", "-p", "ActiveState", "--value", UNIT).strip()
    if state not in (b"active", b"inactive"):
        raise RuntimeError("Resolve the gateway's current service state before rotating its certificate")
    return state


def restart():
    subprocess.run(["/usr/bin/systemctl", "restart", UNIT], check=True)
    subprocess.run(["/usr/bin/systemctl", "is-active", "--quiet", UNIT], check=True)


def validate(base):
    subprocess.run(["/usr/sbin/runuser", "-u", "caddy", "--", CADDY, "validate",
                    "--config", base / "Caddyfile", "--adapter", "caddyfile"], check=True)


def install(base, lineage, gid):
    # Certbot serializes renewals. The lock also covers operator-initiated retries.
    base = base.resolve(strict=True)
    with (base / ".apex-certificate.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        active = gateway_state() == b"active"
        link = base / "tls"
        generations = base / "apex-certificates"
        generations.mkdir(mode=0o750, exist_ok=True)
        os.chown(generations, -1, gid)
        os.chmod(generations, 0o750)
        previous = None
        if link.is_symlink():
            previous = link.resolve(strict=True)
            if previous.parent != generations or not previous.is_dir():
                raise RuntimeError("Refusing to replace an unmanaged certificate link")
        elif link.exists() or active:
            raise RuntimeError("Gateway already uses an unmanaged certificate directory or configuration")

        generation = Path(tempfile.mkdtemp(dir=generations, prefix="version-"))
        try:
            os.chown(generation, -1, gid)
            os.chmod(generation, 0o750)
            fullchain = (lineage / "cert.pem").read_bytes().rstrip() + b"\n" + (lineage / "chain.pem").read_bytes()
            write_owned(generation / "apex-fullchain.pem", fullchain, gid)
            write_owned(generation / "apex-key.pem", (lineage / "privkey.pem").read_bytes(), gid)
            verify(generation)
            sync_directory(generation)
            sync_directory(generations)
            replace_link(link, generation)
            attempted_restart = False
            try:
                validate(base)
                if active:
                    attempted_restart = True
                    restart()
            except BaseException:
                if previous:
                    replace_link(link, previous)
                    if attempted_restart:
                        restart()
                else:
                    link.unlink()
                    sync_directory(base)
                raise
        except BaseException:
            # Preserve a generation if the pointer could not be rolled back.
            if link.resolve() != generation:
                shutil.rmtree(generation)
            raise
        print(f"Installed apex certificate generation {generation.name}")


if __name__ == "__main__":
    if os.environ.get("RENEWED_LINEAGE") != str(LINEAGE):
        sys.exit("Unexpected Certbot lineage")
    if os.geteuid() != 0:
        sys.exit("Run this hook under the root-owned Certbot service")
    os.umask(0o077)
    install(CADDY_DIR, LINEAGE, grp.getgrnam("caddy").gr_gid)
