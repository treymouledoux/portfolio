"""Offline tests; no host commands, IPC or network access."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("guard", Path(__file__).with_name("render-host-guard.py"))
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


class HostGuard(unittest.TestCase):
    def setUp(self):
        self.inventory = {
            "local_prefixes": ["192.168.19.0/24", "2001:db8:19::/64"],
            "resolver_addresses": ["192.168.19.1", "2001:db8:19::53"],
            "jetkvm_ip": "100.100.0.14", "jetkvm_port": 443,
            "public_egress_policy": "provider-pinned",
            "public_edge_prefixes": ["1.1.1.1"], "tunnel_edge_prefixes": ["8.8.8.8"],
            "smtp_prefixes": ["9.9.9.9"], "smtp_port": 587,
        }

    def test_first_stage_preserves_public_uplink_and_scopes_exceptions(self):
        minimal = {k: self.inventory[k] for k in ("local_prefixes", "resolver_addresses")}
        rules = guard.render("first-run", minimal)
        self.assertNotIn('"caddy"', rules)
        self.assertNotIn('"portfolio-dashboard"', rules)
        self.assertNotIn('chain input', rules)
        self.assertIn('oifname "tailscale0" reject', rules)
        self.assertIn('100.64.0.0/10', rules)
        self.assertIn('fc00::/7', rules)
        self.assertIn('2001:db8:19::/64', rules)
        self.assertIn('meta skuid "portfolio-deploy" reject', rules)
        self.assertNotIn('dport 83 ', rules)
        self.assertNotIn('@', rules)

    def test_pilot_requires_fixed_targets_and_blocks_other_service_egress(self):
        rules = guard.render("pilot", self.inventory)
        self.assertIn('meta skuid "portfolio-dashboard" oifname "tailscale0" ip daddr 100.100.0.14 tcp dport 443 accept', rules)
        self.assertIn('oifname != "tailscale0" ip daddr { 192.168.19.1 }', rules)
        self.assertIn('iifname != { "lo", "tailscale0" } meta l4proto { tcp, udp } th dport 53 drop', rules)
        self.assertIn('meta skuid "portfolio-tunnel" ip daddr 127.0.0.1 tcp dport 83 accept', rules)
        self.assertIn('"portfolio-dns" } oifname != "tailscale0"', rules)
        self.assertIn('meta skuid "portfolio-tunnel" ip daddr { 8.8.8.8 } meta l4proto { tcp, udp } th dport 7844', rules)
        self.assertIn('meta skuid "portfolio-public" ip daddr { 9.9.9.9 } tcp dport 587', rules)
        self.assertIn('meta skuid "portfolio-dashboard" ip daddr { 1.1.1.1 } tcp dport 443', rules)
        self.assertTrue(rules.rstrip().endswith('reject\n    }\n}'))
        self.assertNotIn('@', rules)
        self.assertNotIn('flush ruleset', rules)
        self.assertEqual(rules.count("delete table "), 1)

    def test_rejects_unsafe_missing_and_unresolved_targets(self):
        for key, value in (("local_prefixes", []), ("resolver_addresses", ["0.0.0.0/0"]),
                           ("resolver_addresses", ["100.100.100.100"]), ("jetkvm_ip", "127.0.0.1"),
                           ("jetkvm_port", 0), ("smtp_port", True), ("smtp_prefixes", []),
                           ("tunnel_edge_prefixes", ["0.0.0.0/0"]), ("smtp_prefixes", ["192.168.19.1"]),
                           ("public_edge_prefixes", ["1.1.1.1; other nft statements"])):
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                guard.render("pilot", {**self.inventory, key: value})

    def test_ipv6_kvm_stays_bound_to_its_exact_tailnet_address(self):
        rules = guard.render("pilot", {**self.inventory, "jetkvm_ip": "fd7a:115c:a1e0::17"})
        self.assertIn('oifname "tailscale0" ip6 daddr fd7a:115c:a1e0::17 tcp dport 443 accept', rules)

    def test_port_scoped_pilot_handles_public_endpoint_rotation(self):
        inventory = {**self.inventory, "public_egress_policy": "port-scoped"}
        del inventory["smtp_prefixes"], inventory["tunnel_edge_prefixes"]
        rules = guard.render("pilot", inventory)
        self.assertIn('meta skuid "portfolio-dashboard" ip daddr { 1.1.1.1 } tcp dport 443', rules)
        self.assertLess(rules.index('oifname "tailscale0" reject'), rules.index('meta skuid "portfolio-public" tcp dport 587 accept'))
        self.assertLess(rules.index('ip6 daddr != 2000::/3 reject'), rules.index('meta skuid "portfolio-tunnel" meta l4proto { tcp, udp } th dport 7844 accept'))
        del inventory["public_egress_policy"]
        with self.assertRaises(ValueError):
            guard.render("pilot", inventory)

    def test_status_only_uses_four_uids_and_protects_only_its_private_port(self):
        inventory = {**self.inventory, "public_egress_policy": "port-scoped"}
        rules = guard.render("status-only", inventory)
        self.assertNotIn('"portfolio-dns"', rules)
        self.assertNotIn('"caddy"', rules)
        self.assertNotIn('dport 83 ', rules)
        ingress = rules.split('chain input {')[1].split('chain output {')[0]
        self.assertIn('iifname != "tailscale0" meta l4proto { tcp, udp } th dport 8443 drop', ingress)
        self.assertNotIn('443,', ingress)
        self.assertNotIn('53 ', ingress)
        self.assertIn('meta skuid "portfolio-dashboard" oifname "tailscale0" ip daddr 100.100.0.14 tcp dport 443 accept', rules)
        self.assertIn('meta skuid "portfolio-dashboard" ip daddr { 1.1.1.1 } tcp dport 443', rules)
        self.assertIn('meta skuid "portfolio-public" tcp dport 587 accept', rules)
        self.assertIn('meta skuid "portfolio-tunnel" meta l4proto { tcp, udp } th dport 7844 accept', rules)
        self.assertNotIn('@', rules)


if __name__ == "__main__":
    unittest.main()
