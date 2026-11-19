#!/usr/bin/python3
"""Publish only the configured peer's control-plane presence to the dashboard."""
import ipaddress
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


def presence(status, address, name):
    if status.get("BackendState") != "Running":
        return None
    matches = [peer for peer in (status.get("Peer") or {}).values()
               if address in (peer.get("TailscaleIPs") or [])
               and peer.get("DNSName", "").rstrip(".").lower() == name]
    if len(matches) == 1 and type(matches[0].get("Online")) is bool:
        return matches[0]["Online"]
    return None


def main():
    address = str(ipaddress.IPv4Address(os.environ["MUSE_TAILNET_IP"]))
    if ipaddress.ip_address(address) not in ipaddress.ip_network("100.64.0.0/10"):
        raise ValueError("expected a Muse tailnet address")
    name = os.environ["MUSE_DNS_NAME"].rstrip(".").lower()
    if not name.endswith(".ts.net"):
        raise ValueError("expected a full Muse tailnet hostname")
    online = None
    try:
        raw = subprocess.check_output(
            ["/usr/bin/tailscale", "--socket=/run/portfolio-muse-localapi.sock", "status", "--json"],
            timeout=5, stderr=subprocess.DEVNULL)
        online = presence(json.loads(raw), address, name)
    except (OSError, subprocess.SubprocessError, ValueError, TypeError, AttributeError):
        print("Muse presence unavailable", file=sys.stderr)
    directory = Path(os.environ["RUNTIME_DIRECTORY"])
    with tempfile.NamedTemporaryFile(mode="w", dir=directory, delete=False) as output:
        try:
            json.dump({"online": online, "checked_at": int(time.time())}, output)
            output.flush()
            os.fchmod(output.fileno(), 0o640)
            os.replace(output.name, directory / "status.json")
        finally:
            Path(output.name).unlink(missing_ok=True)


if __name__ == "__main__":
    main()
