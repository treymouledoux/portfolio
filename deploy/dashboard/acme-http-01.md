# Apex HTTPS through the existing public tunnel

HTTP-01 is an alternative to the DNS-01 step in the gateway runbook. Let’s Encrypt validates the public hostname through Cloudflare. Only the ACME token path reaches Certbot. The same apex certificate then secures direct browser connections to private Caddy. Cloudflare continues to manage its own public edge certificate. The owner selected registration without an email address and approved the current Let’s Encrypt agreement for this rollout. These commands also require authorization for the tunnel and host changes. [HTTP-01 behavior](https://letsencrypt.org/docs/challenge-types/) · [Subscriber agreement](https://letsencrypt.org/repository/)

## Preconditions

- Verify the public apex, including any published IPv6 path, goes through the confirmed tunnel, and its CAA policy allows Let's Encrypt. Inventory all connector replicas. Each must reach the same challenge responder for the duration of validation. Cloudflare must accept HTTP on public port 80. A normal redirect to HTTPS on the public apex is supported. Review Workers, Access, bot challenges, redirects and cache rules for this exact token path. Preserve protection of other paths.
- Provision the isolated public tunnel UID, the reviewed Caddyfile, `caddy` user/group, `/usr/local/libexec/portfolio-caddy`, and the loaded but inactive `portfolio-gateway.service`. Leave split DNS unchanged. `/etc/caddy` and its parents must be root-owned and unwritable by services, CI and ordinary users. The hook creates `/etc/caddy/tls` as a symlink. An existing deployment with an unmanaged directory there requires a separately reviewed migration and backup.
- Install distribution-supported Certbot with its standalone plugin, OpenSSL and the normal public CA bundle. The examples assume Debian paths and a system Certbot using `/etc/letsencrypt`. Check `certbot certificates` for a conflicting `portfolio-apex` lineage, and review installed Certbot hooks before use. All ACME account and lineage files stay root-only.
- Reserve an unused **privileged loopback TCP port 83** for Certbot. Check `ss -ltn` and `sysctl net.ipv4.ip_unprivileged_port_start` in the host network namespace. The latter must exceed 83. Public and deploy users must have no privilege to bind it, including via sudo, file capabilities, containers, systemd or writable privileged units. Restrict capable services such as Caddy/CoreDNS to their intended bind ports. An unprivileged high port would let another process impersonate Certbot while the real listener is idle.

## Route and verify the challenge

Insert only this rule **before** the ordinary apex origin rule in the existing root-owned tunnel config:

```yaml
- hostname: treymouledoux.me
  path: '^/\.well-known/acme-challenge/[A-Za-z0-9_-]+$'
  service: http://127.0.0.1:83
```

Use the rendered full-apex `pilot` firewall from [the host isolation runbook](host-isolation.md). In its effective output chain, verify the tunnel UID has an exception for TCP to `127.0.0.1:83` followed by a rejection for other local UIDs to that endpoint. Confirm non-loopback ingress to TCP 83 is dropped. The earlier first-run and status-only rulesets serve other rollout stages.

From a root-controlled copy of the existing configuration, run `cloudflared tunnel --config <file> ingress validate` and `cloudflared tunnel --config <file> ingress rule <URL>`. Check both an HTTP and an HTTPS apex token URL. Check that `/dashboard`, `/`, a nested challenge path, and a different hostname retain their original destinations. Test the prepared firewall with `nft --check --file <file>`, then perform the authorized tunnel reload or restart with its previous configuration available for rollback. An idle challenge route can return 502 when Certbot is stopped. Never point the tunnel at Caddy, CoreDNS, Serve or the private application socket. [Tunnel rule ordering and verification](https://developers.cloudflare.com/tunnel/features/locally-managed-tunnels/configuration-file/)

## Issue and load

From the reviewed files, run `install -o root -g root -m 0755 acme-install.py /usr/local/libexec/portfolio-acme-install`. Confirm `/usr/bin/python3`, `/usr/bin/openssl`, `/usr/bin/systemctl`, `/usr/sbin/runuser` and `/etc/ssl/certs/ca-certificates.crt` exist. Certbot's root renewal unit must be able to write the gateway certificate directory and restart the gateway. Verify the current clock, the installed Certbot version, and that no default email contact is configured in Certbot’s CLI configuration. The commands below register without an email on Certbot 3.3 and later. For older versions, add `--register-unsafely-without-email` to each issuance command. Start with a staging challenge and leave the deployment hook disabled for the dry run. [Certbot 3.3 registration change](https://github.com/certbot/certbot/blob/v5.10.0/certbot/CHANGELOG.md#330---2025-03-11)

```sh
certbot certonly --dry-run --standalone --preferred-challenges http-01 \
  --http-01-address 127.0.0.1 --http-01-port 83 \
  --cert-name portfolio-apex -d treymouledoux.me \
  --agree-tos --non-interactive
```

Run issuance once after staging succeeds. The `--deploy-hook` belongs to this lineage and is reused on subsequent renewals.

```sh
certbot certonly --standalone --preferred-challenges http-01 \
  --http-01-address 127.0.0.1 --http-01-port 83 \
  --cert-name portfolio-apex -d treymouledoux.me \
  --agree-tos --non-interactive \
  --deploy-hook /usr/local/libexec/portfolio-acme-install
```

The hook verifies the exact apex SAN, trust chain, key match and expiry, installs the pair into a new root-owned generation, and updates one symlink atomically. Both key and chain are readable by the Caddy service group. It validates the configuration as the Caddy user. A running gateway receives a controlled restart because its admin API is disabled. If validation or restart fails, the hook restores the previous pointer and, when needed, restarts the previous generation. A first installation leaves the gateway inactive for supervised startup. If no new certificate was issued and the hook did not run, use the recovery invocation below for this existing verified lineage.

Check `readlink /etc/caddy/tls`, its ownership/modes, and the leaf serial and dates via `openssl x509 -in /etc/caddy/tls/apex-fullchain.pem -noout -serial -dates`. Start the gateway only following approval. From an authorized client, use a temporary per-request hostname override for `treymouledoux.me` to the Pi's actual tailnet IP and verify the browser-trusted chain, correct served serial, working dashboard and normal public routes. Then proceed to the separate split-DNS cutover.

## Renew and recover

Verify the distro's Certbot renewal timer is enabled, record its next run and run `certbot renew --dry-run --cert-name portfolio-apex` to exercise the saved challenge settings. Monitor the timer, hook logs, and **the certificate served by the private gateway**. Certbot can report successful issuance even if deployment fails. After correcting a hook or gateway failure, retry the same certificate without contacting the issuer:

```sh
RENEWED_LINEAGE=/etc/letsencrypt/live/portfolio-apex /usr/local/libexec/portfolio-acme-install
```

Preserve the active and preceding known-good generations in `/etc/caddy/apex-certificates`. Prune older private key copies under operator control. If deployment is withdrawn, return split DNS to its public view, retire only this lineage's renewal configuration after checking shared Certbot use, and revert this tunnel rule and loopback firewall exception. Keep the public portfolio and its 403 intact. [Certbot renewal and deployment hooks](https://eff-certbot.readthedocs.io/en/stable/using.html#renewing-certificates)
