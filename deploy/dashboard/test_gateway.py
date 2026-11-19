"""Local integration check. Requires Caddy, CoreDNS, dig and openssl. No public DNS queries."""
import argparse
from contextlib import ExitStack, contextmanager
import errno
from http.client import HTTPConnection
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import ssl
import subprocess
import tempfile
import threading
import time

HERE = Path(__file__).resolve().parent
HOST = "portfolio.test"


@contextmanager
def process(command, work):
    with tempfile.TemporaryFile() as log:
        with subprocess.Popen(command, cwd=work, stdout=log, stderr=log,
                              env=dict(os.environ, XDG_DATA_HOME=str(work / "data"),
                                       XDG_CONFIG_HOME=str(work / "config"))) as child:
            try:
                yield child
            except BaseException:
                log.seek(0)
                print(log.read().decode(errors="replace"))
                raise
            finally:
                child.terminate()
                try:
                    child.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_tcp(port, child):
    for _ in range(150):
        if child.poll() is not None:
            raise RuntimeError(f"Service exited with status {child.returncode}")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.3):
                return
        except OSError:
            time.sleep(0.04)
    raise RuntimeError("Local fixture did not start")


class UnixServer(ThreadingHTTPServer):
    address_family = socket.AF_UNIX

    def server_bind(self):
        self.socket.bind(self.server_address)
        self.server_name = "dashboard"
        self.server_port = 0


class Origin(BaseHTTPRequestHandler):
    def do_GET(self):
        length = int(self.headers.get("Content-Length", 0))
        self.server.seen.append((self.command, self.path, self.headers, self.rfile.read(length)))
        data = b"fixture public response"
        status = 200
        if "Range" in self.headers:
            status = 206 if self.headers["Range"] == "bytes=0-6" else 416
            data = data[:7] if status == 206 else b""
        elif self.headers.get("If-None-Match") == '"fixture"':
            status, data = 304, b""
        self.send_response(status)
        if status == 206:
            self.send_header("Content-Range", "bytes 0-6/23")
        self.send_header("Set-Cookie", "fixture=remove")
        self.send_header("Alt-Svc", 'h3=":443"')
        self.end_headers()
        self.wfile.write(data)

    do_POST = do_GET

    def log_message(self, *args):
        pass


def check_caddy(work, binary):
    template = (HERE / "Caddyfile").read_text()
    port = free_port()
    with ThreadingHTTPServer(("127.0.0.1", 0), Origin) as public, ExitStack() as cleanup:
        public.seen = []
        threading.Thread(target=public.serve_forever, daemon=True).start()
        cleanup.callback(public.shutdown)
        cert, key = work / "cert.pem", work / "key.pem"
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                        "-subj", f"/CN={HOST}", "-addext", f"subjectAltName=DNS:{HOST},DNS:dashboard.{HOST}",
                        "-keyout", key, "-out", cert], check=True, stderr=subprocess.DEVNULL)
        # IPv4 loopback represents the approved client; IPv6 loopback the non-viewer.
        assert template.count("bind 192.0.2.1 2001:db8::1") == 2
        cfg = template.replace("treymouledoux.me", HOST)
        for site in (HOST, f"dashboard.{HOST}"):
            cfg = cfg.replace(f"https://{site} {{", f"https://{site}:{port} {{")
        cfg = cfg.replace("bind 192.0.2.1 2001:db8::1", "bind 127.0.0.1 ::1")
        cfg = cfg.replace("192.0.2.10/32", "127.0.0.1/32").replace("127.0.0.1:6050", f"127.0.0.1:{public.server_port}")
        cfg = cfg.replace("/etc/caddy/tls/apex-fullchain.pem", str(cert)).replace("/etc/caddy/tls/apex-key.pem", str(key))
        cfg = cfg.replace("/etc/caddy/dashboard-tls/dashboard-fullchain.pem", str(cert))
        cfg = cfg.replace("/etc/caddy/dashboard-tls/dashboard-key.pem", str(key))
        dashboard_socket = work / "dashboard.sock"
        cfg = cfg.replace("unix//run/portfolio-dashboard-peer/dashboard.sock", f"unix//{dashboard_socket}")
        cfg = cfg.replace("/usr/local/share/portfolio-dashboard", f'"{work / "static"}"')
        if "127.0.0.1:6050" in cfg or "bind 127.0.0.1 ::1" not in cfg:
            raise ValueError("Fixture did not replace the production listener and upstream")
        dashboard = UnixServer(str(dashboard_socket), Origin)
        dashboard.seen = []
        threading.Thread(target=dashboard.serve_forever, daemon=True).start()
        cleanup.callback(dashboard.server_close)
        cleanup.callback(dashboard.shutdown)
        (work / "static").mkdir()
        (work / "static/handoff.html").write_bytes((HERE / "handoff.html").read_bytes())
        config = work / "Caddyfile"
        config.write_text(cfg)
        adapted = json.loads(subprocess.check_output([binary, "adapt", "--config", config, "--adapter", "caddyfile"]))
        servers = list(adapted["apps"]["http"]["servers"].values())
        assert len(servers) == 1
        assert set(servers[0]["listen"]) == {f"127.0.0.1:{port}", f"[::1]:{port}"}
        child = cleanup.enter_context(process([binary, "run", "--config", config, "--adapter", "caddyfile"], work))
        wait_tcp(port, child)
        tls = ssl.create_default_context(cafile=str(cert))

        def request(path, address="127.0.0.1", method="GET", body=None, hostname=HOST,
                    headers=None, sni=None):
            conn = HTTPConnection(HOST, timeout=3)
            with socket.create_connection((address, port), timeout=3) as raw:
                conn.sock = tls.wrap_socket(raw, server_hostname=sni or HOST)
                try:
                    fields = {
                        "Host": hostname, "Cookie": "synthetic", "Authorization": "synthetic",
                        "CF-Connecting-IP": "synthetic", "Tailscale-User-Login": "synthetic",
                        "X-Forwarded-For": "synthetic", **(headers or {}),
                    }
                    if body is not None:
                        fields["Content-Length"] = str(len(body))
                    conn.putrequest(method, path, skip_host=True, skip_accept_encoding=True)
                    for key, value in fields.items():
                        conn.putheader(key, value)
                    conn.endheaders(body)
                    response = conn.getresponse()
                    return response.status, response.headers, response.read()
                finally:
                    conn.close()

        # One public entry point for every client: the handoff page decides.
        handoff = (HERE / "handoff.html").read_bytes()
        for path in ("/dashboard?example=1", "/dashboard/", "/dashboard/messages", "/dashboard/messages/"):
            status, headers, body = request(path)
            assert status == 200 and body == handoff
            assert headers["Cache-Control"] == "private, no-store" and not headers["Location"]
        assert request("/dashboard", method="HEAD")[2] == b""
        assert not public.seen
        # Everything else under /dashboard still falls through to the public
        # denial document, whatever the client address.
        for path, peer, method in [("/dashboard/api?example=1", "::1", "GET"),
                                   ("/dashboard/other", "127.0.0.1", "GET"),
                                   ("/dashboard/api?example=1", "127.0.0.1", "POST")]:
            status, headers, _ = request(path, peer, method, b"test" if method == "POST" else None)
            seen_method, seen_path, seen_headers, seen_body = public.seen.pop()
            assert status == 403 and headers["Cache-Control"] == "private, no-store"
            assert seen_method == "GET" and seen_path.rstrip("?") == "/dashboard" and not seen_body
            assert not seen_headers["Cookie"] and not seen_headers["Authorization"]
        for path in ("/", "/resume", "/_next/static/fixture.js"):
            assert request(path)[0] == 200
            _, seen_path, forwarded, _ = public.seen.pop()
            assert seen_path == path and forwarded["Host"] == HOST
            assert all(h not in forwarded for h in (
                "Range", "If-Range", "If-Modified-Since", "If-None-Match", "Accept", "Accept-Encoding",
                "Content-Type", "Cookie", "Authorization", "CF-Connecting-IP", "Tailscale-User-Login",
                "X-Forwarded-For", "X-Forwarded-Proto", "X-Forwarded-Host",
            )), forwarded
        fields = {"Range": "bytes=0-6", "If-Range": '"fixture"', "Accept": "application/pdf", "Accept-Encoding": "identity"}
        status, headers, data = request("/resume", headers=fields)
        assert status == 206 and headers["Content-Range"] == "bytes 0-6/23" and data == b"fixture"
        forwarded = public.seen.pop()[2]
        assert all(forwarded[k] == v for k, v in fields.items())
        fields = {"If-None-Match": '"fixture"', "If-Modified-Since": "Sun, 06 Nov 1994 08:49:37 GMT"}
        assert request("/", headers=fields)[0] == 304
        forwarded = public.seen.pop()[2]
        assert all(forwarded[k] == v for k, v in fields.items()) and "Range" not in forwarded
        status, headers, _ = request("/api/contact", method="POST", body=b"{}", headers={"Content-Type": "application/json"})
        _, path, forwarded, data = public.seen.pop()
        assert status == 200 and path == "/api/contact" and data == b"{}"
        assert forwarded["CF-Connecting-IP"] == "127.0.0.1" and forwarded["Content-Type"] == "application/json"
        assert all(not forwarded[h] for h in ("Cookie", "Authorization", "Tailscale-User-Login", "X-Forwarded-For"))
        assert not headers["Set-Cookie"] and not headers["Alt-Svc"]
        # The dashboard host is one origin, reachable only through the peer socket.
        status, headers, body = request(
            "/messages", hostname=f"dashboard.{HOST}", sni=f"dashboard.{HOST}",
            headers={"X-Portfolio-Peer": "10.9.9.9"},
        )
        assert status == 200 and body == b"fixture public response"
        seen_method, seen_path, seen_headers, _ = dashboard.seen.pop()
        assert seen_method == "GET" and seen_path == "/messages"
        assert seen_headers["X-Portfolio-Peer"] == "127.0.0.1", seen_headers
        assert not seen_headers["Tailscale-User-Login"]
        assert not public.seen
        assert request("/dashboard", hostname="unexpected.test")[0] == 421
        assert request("/other", method="POST", body=b"{}")[0] == 405 and not public.seen
    print("Caddy: verified TLS host, direct peer selection, original URL, sanitized 403 and public routes.")


def check_dns(work, binary):
    port, upstream_port = free_port(), free_port()
    assert port != upstream_port
    template = (HERE / "Corefile.example").read_text()
    assert template.count("bind 192.0.2.1 2001:db8::1") == 2
    config = template.replace("treymouledoux.me:53", f"{HOST}:{port}").replace("treymouledoux[.]me", "portfolio[.]test")
    config = config.replace("bind 192.0.2.1 2001:db8::1", "bind 127.0.0.1 ::1")
    config = config.replace("192.0.2.10/32", "127.0.0.1/32").replace("/etc/portfolio-dns/public-resolv.conf", f"127.0.0.1:{upstream_port}")
    (work / "zone").write_text("""$ORIGIN portfolio.test.
$TTL 60
@ IN SOA ns hostmaster 1 60 60 60 60
@ IN NS ns
ns IN A 203.0.113.3
@ IN A 203.0.113.8
@ IN AAAA 2001:db8::8
@ IN HTTPS 1 .
@ IN SVCB 1 .
@ IN MX 10 mail
@ IN TXT "fixture"
www IN A 203.0.113.9
""")
    cfg = work / "Corefile"
    binds = [line.strip() for line in config.splitlines() if line.strip().startswith("bind ")]
    forwards = [line.strip() for line in config.splitlines() if line.strip().startswith("forward ")]
    assert binds == ["bind 127.0.0.1 ::1"] * 2 and forwards == [f"forward . 127.0.0.1:{upstream_port}"] * 2
    cfg.write_text(config + f'\n{HOST}:{upstream_port} {{\n    bind 127.0.0.1\n    file "{work / "zone"}"\n}}\n')
    with process([binary, "-conf", cfg], work) as child:
        wait_tcp(port, child)

        def query(name, kind, source, tcp=False):
            args = ["dig", f"@{source}", "-p", str(port), name, kind, "+time=2", "+tries=1",
                    "+noall", "+comments", "+answer", "+tcp" if tcp else "+notcp"]
            reply = subprocess.check_output(args, text=True)
            assert "status: NOERROR" in reply, reply
            return [line for line in reply.splitlines() if line and not line.startswith(";")]

        for tcp in (False, True):
            assert query("PorTFolio.TeSt.", "A", "127.0.0.1", tcp)[0].endswith("192.0.2.1")
            assert query(HOST, "A", "::1", tcp)[0].endswith("203.0.113.8")
            for kind in ("AAAA", "TYPE65", "TYPE64"):
                assert not query(HOST, kind, "127.0.0.1", tcp)
                assert query(HOST, kind, "::1", tcp)
            for peer in ("127.0.0.1", "::1"):
                assert "mail.portfolio.test." in query(HOST, "MX", peer, tcp)[0]
                assert '"fixture"' in query(HOST, "TXT", peer, tcp)[0]
                assert query("www." + HOST, "A", peer, tcp)[0].endswith("203.0.113.9")
    print("CoreDNS: verified direct IPv4/IPv6 client views, TCP/UDP, apex overrides and public forwarding.")

    # Some OSes own all of 127/8. Exercise missing-address startup only where
    # the kernel confirms that the chosen loopback address is unavailable.
    with socket.socket() as probe:
        try:
            probe.bind(("127.0.0.250", 0))
        except OSError as error:
            if error.errno != errno.EADDRNOTAVAIL:
                raise
        else:
            return
    cfg.write_text(config.replace("bind 127.0.0.1 ::1", "bind 127.0.0.250"))
    failed = subprocess.run([binary, "-conf", cfg], capture_output=True, text=True, timeout=5)
    assert failed.returncode != 0 and "127.0.0.250" in failed.stdout + failed.stderr
    print("CoreDNS: missing configured address refused startup.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--caddy", default="caddy")
    parser.add_argument("--coredns", default="coredns")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="gateway-test-") as directory:
        check_caddy(Path(directory), args.caddy)
        check_dns(Path(directory), args.coredns)
