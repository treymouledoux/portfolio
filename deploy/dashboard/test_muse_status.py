import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("muse", Path(__file__).with_name("muse-status.py"))
muse = importlib.util.module_from_spec(spec)
spec.loader.exec_module(muse)


class PresenceTest(unittest.TestCase):
    def test_only_one_matching_peer_with_explicit_presence(self):
        ip, name = "100.64.0.20", "muse.example.ts.net"
        peer = {"TailscaleIPs": [ip], "DNSName": name + ".", "Online": False}
        status = {"BackendState": "Running", "Peer": {"a": peer}}
        self.assertIs(muse.presence(status, ip, name), False)
        peer["Online"] = True
        self.assertIs(muse.presence(status, ip, name), True)
        self.assertIsNone(muse.presence(status, ip, "different.example.ts.net"))
        self.assertIsNone(muse.presence(status | {"BackendState": "Stopped"}, ip, name))
        self.assertIsNone(muse.presence(status | {"Peer": {"a": peer, "b": peer}}, ip, name))
        del peer["Online"]
        self.assertIsNone(muse.presence(status, ip, name))


if __name__ == "__main__":
    unittest.main()
