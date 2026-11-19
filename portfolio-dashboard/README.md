# Private dashboard

This service reports website HTTPS and JetKVM socket timings, JetKVM echo latency, Pi resource usage and Muse’s Tailscale presence. The pilot gives the approved Mac and iPhone dashboard access. The approved Mac and PC can use native Tailscale SSH to the Pi from their own terminals.

## Request paths

| Visitor | Request path and result |
|---|---|
| Public internet | Cloudflare → existing tunnel → public Axum listener. Any request for the dashboard hostname gets one fixed tailnet-only notice, and the apex has no dashboard path at all, so `/dashboard` there is an ordinary miss. |
| Approved browser | A restricted DNS view resolves both the apex and the dashboard hostname to the Pi. Caddy serves the public site at the apex and the dashboard only at `dashboard.treymouledoux.me`. |
| Direct private view | Tailscale Serve on TCP 8443 authenticates the transport peer, strips caller-supplied identity and forwards to a root-only Unix socket. The Rust service checks the exact human login for the page, API and assets. |
| Pi operator | The full dashboard can display the native SSH command. Run it from the authorized Mac or PC, confirm the Pi identity and complete Tailscale's reauthentication when requested. The SSH grant and policy authorize the device, human and OS account independently of dashboard access. |
| Approved inbox reader | The Messages view opens without a passkey: reading, filtering, unread counts and the JSON export need only an authorized reader. The header badge reports the locked state until someone unlocks it.
| Approved viewer, unlocked | A passkey session (fifteen minutes) unlocks marking and removing messages, the web terminal, the telemetry settings and the named host operations. The page calls the root agent only after the passkey session and origin check pass; the agent runs a shell as the configured human account and its restart/reboot verbs are a fixed list. |

Nothing of the dashboard is framed or embedded anywhere: the public apex neither hosts nor proxies it. The server requires an authorized inbox reader before serving submissions and a verified passkey session before any change to them, and the inbox denies framing. The enrolled passkey is scoped to the private hostname. A compromised portfolio could still imitate the unlock screen, so reach the dashboard by typing its address rather than following a link. [Tailscale Serve identity forwarding](https://github.com/tailscale/tailscale/blob/v1.104.7/ipn/ipnlocal/serve.go) · [Tailscale SSH](https://tailscale.com/docs/features/tailscale-ssh)

## Configuration and verification

Use the [gateway runbook](../deploy/dashboard/README.md) and [restricted public deployment migration](../deploy/public-isolated/README.md) together. The one-Pi design shares the host kernel, root account, Tailscale daemon and node identity across workloads. Complete the public deploy migration, process/UID isolation, firewall and policy checks before serving private data. Retain independent recovery access to the Pi.

To enable the inbox, list approved contact-message readers in `DASHBOARD_INBOX_READERS` using the same login syntax as `DASHBOARD_VIEWERS`. Both lists must authorize a reader. The inbox reads only the mount at `/run/portfolio-inbox/submissions.jsonl`, newest first, with bounded reads and cursor pagination. Keep that mount read-only and available only inside the service. Omit the reader setting to deny all inbox access. Message bodies are available only on the private origin.

The Messages tab shows unread and total counts for the current reader. Messages begin unread. Readers can filter the inbox immediately; unlocking it with the passkey adds marking submissions read or unread in place. The header badge is the single session indicator: it counts the fifteen minutes down and locks the session on click, and the Settings passkey card enrols and signs in. The inbox systemd drop-in keeps per-reader markers under `/var/lib/portfolio-inbox-read`. Readers can export a JSON snapshot of the contact archive from the private origin. Treat the downloaded file as private contact data.

### Permanent inbox removal

Set `DASHBOARD_INBOX_ADMINS` to exact authorized logins and `DASHBOARD_ADMIN_ORIGIN` to the full private Serve origin, including HTTPS and port 8443. Administrators must also be viewers and inbox readers. Configure the private origin and provision a passkey before enabling access to submissions. First-time enrollment requires an operator-opened window and happens from the Settings view. Unlocking from the Messages view or the Settings passkey card opens a session for up to 15 minutes, shown and ended by the header badge. An administrator can select Remove on a message or Remove all, review the scope and hold the armed button for five seconds on that page. Releasing early cancels the gesture. Removing a local submission leaves already delivered email and existing backups intact.

The private form and JSON endpoints require a same-origin POST carrying the exact configured Origin. The broker at `/run/portfolio-inbox-actions/actions.sock` verifies WebAuthn challenges and binds each hold to the verified user session, operation and target. The browser stores the opaque session in an HTTPS-only, HttpOnly, host-prefixed, SameSite=Strict cookie. The broker enforces a five-second minimum and single use. The private inbox denies framing. A single-message target is the SHA-256 of the raw JSON line excluding its terminating LF. Bulk removal is bound to a snapshot and fails if the inbox changes. The dashboard service retains read-only file access. Grant the action socket only to the dedicated service and provision the broker and writer together before enabling the admin role.

### Status monitoring

Configure `DASHBOARD_VIEWERS`, `DASHBOARD_JETKVM_IP` and `DASHBOARD_JETKVM_PORT` in the protected host environment. Set the optional `DASHBOARD_PI_MACHINE` to the Pi’s verified MagicDNS FQDN and `DASHBOARD_PI_SSH_USER` to the designated local account to display the terminal command on the full private dashboard. The command is plain text. The app holds no SSH credentials and initiates no SSH connections. A positive JetKVM result means the configured port accepted a TCP connection. The website probe requires a verified HTTPS 200 with the expected health body. Probes run independently of page requests every 30 seconds. The visible overview polls the status and count frames every 30 seconds, and its fixed test links request a fresh measurement. The optional Muse publisher reports the one configured peer’s online flag. See the gateway runbook for service installation, firewall and ping permissions.

The [CoreDNS example](../deploy/dashboard/Corefile.example) is one option for a restricted nameserver. Put actual approved client addresses and the Pi address only in the installed copy. Use a CoreDNS build with the view, template, bind and forward plugins. Provide ordinary public upstream nameservers in `/etc/portfolio-dns/public-resolv.conf`. Stage UDP and TCP queries from a viewer and nonviewer, covering A, AAAA, HTTPS/SVCB, MX/TXT and a public subdomain. Browsers using a different resolver or an existing public connection can continue to receive the safe public 403 until they reconnect. Peers blocked at TCP cannot receive an HTTP page. [CoreDNS client views](https://coredns.io/plugins/view/) · [Tailscale DNS](https://tailscale.com/docs/reference/dns-in-tailscale)

From this directory:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
python3 ../scripts/check-dashboard-styles.py
```

[Dashboard CI](../.github/workflows/dashboard.yml) runs the checks and builds on Linux ARM64. A manual build of main retains just the executable in a tarball for three days. The workflow has no tailnet identity and cannot install software on the Pi. Confirm the source revision, artifact provenance, target architecture and libc compatibility before a separately approved, root-owned installation. Keep site-specific configuration outside every build archive.

## Rollout order

1. Ship the public 403 through the existing portfolio workflow after approval.
2. Migrate the public release identity and preserve the contact data. Verify the restricted account can deploy and roll back the public site without touching gateway files, keys or units.
3. Stage the private socket, status worker, Serve, firewall and grants. Require both the intended person and eligible source devices. Verify unapproved devices under the same owner account are excluded.
4. Stage DNS, apex TLS and Caddy, then test real browsers on and off the tailnet. Verify public pages, contact rate limiting, denied requests, both probes, credential-free apex traffic and SSH reauthentication.
5. Enable this read-only pilot only after the checks and explicit approval. Moving privileged automation onto this host would enlarge the effects of a public-host compromise. Revisit isolation and independent approval before adding writes.
