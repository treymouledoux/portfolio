#!/usr/bin/env python3
"""Render one reviewed nftables table from a host-local inventory, without applying it."""
import argparse
import ipaddress
import json
from pathlib import Path
import sys


HERE = Path(__file__).resolve().parent
PRIVATE = tuple(map(ipaddress.ip_network, (
    "0.0.0.0/8", "10.0.0.0/8", "100.64.0.0/10", "127.0.0.0/8", "169.254.0.0/16",
    "172.16.0.0/12", "192.0.0.0/24", "192.0.2.0/24", "192.88.99.0/24",
    "192.168.0.0/16", "198.18.0.0/15", "198.51.100.0/24", "203.0.113.0/24",
    "224.0.0.0/4", "240.0.0.0/4", "::/128",
    "::1/128", "::ffff:0:0/96", "64:ff9b::/96", "64:ff9b:1::/48", "2002::/16",
    "fc00::/7", "fe80::/10", "ff00::/8",
)))
TAILNET = tuple(map(ipaddress.ip_network, ("100.64.0.0/10", "fd7a:115c:a1e0::/48")))
STAGES = {
    "first-run": "first-run-guard.nft.example",
    "status-only": "status-only-guard.nft.example",
    "pilot": "private-guard.nft.example",
}


def read_networks(inventory, key, hosts=False):
    values = inventory.get(key)
    if not isinstance(values, list) or not values:
        raise ValueError(f"{key} requires at least one reviewed address or prefix")
    networks = []
    for raw in values:
        if not isinstance(raw, str) or "%" in raw:
            raise ValueError(f"invalid value in {key}")
        if hosts:
            raw = str(ipaddress.ip_address(raw))
        network = ipaddress.ip_network(raw)
        if not network.prefixlen:
            raise ValueError(f"default route is invalid in {key}")
        networks.append(network)
    return networks


def address_rules(networks, pattern):
    rules = []
    for version, family in ((4, "ip"), (6, "ip6")):
        members = ipaddress.collapse_addresses(n for n in networks if n.version == version)
        members = [str(n.network_address) if n.num_addresses == 1 else str(n) for n in members]
        if members:
            rules.append(pattern.replace("ADDRESS", family + " daddr { " + ", ".join(members) + " }"))
    return "\n        ".join(rules)


def numeric_port(inventory, key):
    value = inventory.get(key)
    if type(value) is not int or not 1 <= value <= 65535:
        raise ValueError(f"{key} requires a TCP port number")
    return value


def render(stage, inventory):
    if stage not in STAGES or not isinstance(inventory, dict):
        raise ValueError("expected stage and JSON inventory object")
    local = [*PRIVATE, *read_networks(inventory, "local_prefixes")]
    resolvers = read_networks(inventory, "resolver_addresses", hosts=True)
    if any(n.network_address.is_multicast or n.network_address.is_unspecified
           or any(n.overlaps(t) for t in TAILNET) for n in resolvers):
        raise ValueError("supply exact non-overlay DNS resolvers")
    users = '"portfolio-public", "portfolio-tunnel"'
    if stage != "first-run":
        users += ', "portfolio-dashboard"'
    if stage == "pilot":
        if any(n.network_address.is_loopback for n in resolvers):
            raise ValueError("pilot DNS ingress guard requires the actual upstream resolvers")
        users += ', "portfolio-dns"'
    dns = address_rules(resolvers, 'meta skuid { ' + users + ' } oifname != "tailscale0" ADDRESS meta l4proto { tcp, udp } th dport 53 accept')
    denied = address_rules(local, "ADDRESS reject")
    # Each stage replaces only this table in one nft transaction.
    batch = "add table inet portfolio_private_guard\ndelete table inet portfolio_private_guard\n"
    source = batch + (HERE / STAGES[stage]).read_text()
    if stage != "first-run":
        source = source.replace("@PILOT_EGRESS@", (HERE / "pilot-egress.nft.example").read_text().rstrip())
    source = source.replace("@DNS_RULES@", dns).replace("@LOCAL_DENIES@", denied)
    if stage == "first-run":
        return source

    kvm = ipaddress.ip_address(inventory.get("jetkvm_ip", ""))
    if not isinstance(inventory.get("jetkvm_ip"), str) or "%" in str(kvm) or not any(kvm in t for t in TAILNET):
        raise ValueError("jetkvm_ip must be its numeric Tailscale node address")
    port = numeric_port(inventory, "jetkvm_port")
    family = "ip" if kvm.version == 4 else "ip6"
    kvm_rule = f'meta skuid "portfolio-dashboard" oifname "tailscale0" {family} daddr {kvm} tcp dport {port} accept'
    ping = inventory.get("jetkvm_ping", False)
    if not isinstance(ping, bool) or (ping and kvm.version != 4):
        raise ValueError("jetkvm_ping requires a boolean and an IPv4 JetKVM target when enabled")
    if ping:
        kvm_rule += f'\n        meta skuid "portfolio-dashboard" oifname "tailscale0" ip daddr {kvm} icmp type echo-request accept'
    # Muse is echoed from the same service, so it needs its own reviewed target.
    muse_ping = inventory.get("muse_ping", False)
    if not isinstance(muse_ping, bool):
        raise ValueError("muse_ping requires a boolean")
    if muse_ping:
        raw = inventory.get("muse_ip")
        if not isinstance(raw, str) or "%" in raw:
            raise ValueError("muse_ping requires muse_ip")
        try:
            muse = ipaddress.IPv4Address(raw)
        except ValueError as error:
            raise ValueError("muse_ip must be one tailnet IPv4 address") from error
        if not any(muse in t for t in TAILNET):
            raise ValueError("muse_ip must be its numeric Tailscale node address")
        kvm_rule += f'\n        meta skuid "portfolio-dashboard" oifname "tailscale0" ip daddr {muse} icmp type echo-request accept'
    smtp = numeric_port(inventory, "smtp_port")
    if smtp not in (465, 587):
        raise ValueError("smtp_port must match the verified TLS submission port")
    providers = []
    policy = inventory.get("public_egress_policy")
    if policy not in ("port-scoped", "provider-pinned"):
        raise ValueError("choose port-scoped or provider-pinned public egress explicitly")
    for key, rule in [
        ("public_edge_prefixes", 'meta skuid "portfolio-dashboard" ADDRESS tcp dport 443 accept'),
        ("tunnel_edge_prefixes", 'meta skuid "portfolio-tunnel" ADDRESS meta l4proto { tcp, udp } th dport 7844 accept'),
        ("smtp_prefixes", f'meta skuid "portfolio-public" ADDRESS tcp dport {smtp} accept'),
    ]:
        if policy == "port-scoped" and key in ("smtp_prefixes", "tunnel_edge_prefixes"):
            # LAN, overlay, local and reserved destinations were rejected earlier.
            providers.append(rule.replace("ADDRESS ", ""))
            continue
        networks = read_networks(inventory, key)
        if any(not n.is_global or any(n.overlaps(p) for p in local) for n in networks):
            raise ValueError(f"{key} must contain public provider endpoints outside protected networks")
        providers.append(address_rules(networks, rule))
    return source.replace("@KVM_RULE@", kvm_rule).replace("@PROVIDER_RULES@", "\n        ".join(providers))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=STAGES)
    parser.add_argument("inventory", type=Path, help="private JSON inventory file")
    args = parser.parse_args()
    try:
        output = render(args.stage, json.loads(args.inventory.read_text()))
    except (OSError, ValueError, TypeError) as error:
        parser.exit(2, f"Invalid inventory: {error}\n")
    sys.stdout.write(output)
