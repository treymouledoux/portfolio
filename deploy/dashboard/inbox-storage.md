# Contact inbox and permanent removal

The authoritative store is `/var/lib/portfolio-inbox-store/submissions.jsonl` on the Pi. Root provisions a dedicated local account, `portfolio-inbox-actions`, that owns the store and its directory. The public writer can traverse this directory, acquire its existing lock, and append to the existing file. It cannot list the directory, create a replacement or use the private broker socket. The dashboard receives a read-only directory mount (so it follows atomic replacements) and a named read ACL on the data file. Neither browser nor web process holds a root credential.

The erasure broker accepts bounded requests from the dashboard UID over a Unix socket. Only a root peer can open a five-minute initial passkey enrollment window for an exact configured administrator. Both the app and the broker enforce administrator membership. A registered account cannot register a replacement through the browser. The passkey is scoped to the private Serve hostname. The broker verifies the exact HTTPS origin, RP ID, random one-use challenge, user presence, user verification, and signature on login with Yubico’s `python3-fido2` package. The first registration also requires a separate signed assertion to start a session. Reopening the browser can require signing in again. A synced passkey may be available on other authorized devices through the owner's passkey provider.

The broker persists the public credential and its signature counter in the dedicated service-owned `/var/lib/portfolio-inbox-auth/credentials.sqlite3`. Challenges expire in two minutes, authenticated sessions in fifteen minutes and armed confirmations in thirty seconds. These transient values live only in memory. To remove a message the owner selects the item and holds the final button for five seconds. The broker binds the hold to the authenticated session, operation and digest, checks the elapsed time independently, and consumes it once. The browser cancels released holds; server timing alone cannot attest to continuous physical contact. A valid single-row action requires one unique matching record. A bulk action requires the exact confirmed snapshot, including all intervening appends. A stale selection must be reconfirmed. Each change takes the same lock used by the public writer, syncs a complete temporary replacement, atomically renames it, then syncs the directory. No Trash or message archive is maintained by the broker. Email deliveries, external backups and previously delivered responses have separate lifetimes.

## Install on the Pi

Use the reviewed, flock-aware public executable and an inbox-management-capable private executable. Preserve existing service drop-ins. The following steps run **as root** from a root-owned staging directory containing the deployment files. The ordinary public deploy user cannot modify them. Install Debian’s `acl` and `python3-fido2` packages. The broker is tested against Debian trixie’s python-fido2 1.2 API. The old store, its optional read-only hardlink and the new store must share a filesystem. Creating the new user and staging the units can be done while the site runs.

```sh
getent passwd portfolio-inbox-actions >/dev/null || useradd --system --user-group --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin portfolio-inbox-actions
install -o root -g root -m 0755 inbox-actions.py /usr/local/libexec/portfolio-inbox-actions
install -o root -g root -m 0755 migrate-inbox.py /usr/local/libexec/portfolio-migrate-inbox
install -d -m 0755 /etc/systemd/system/portfolio.service.d /etc/systemd/system/portfolio-dashboard.service.d
install -m 0644 portfolio-inbox-actions.socket portfolio-inbox-actions.service /etc/systemd/system/
install -m 0644 portfolio-contact-store.conf /etc/systemd/system/portfolio.service.d/inbox-store.conf
install -m 0644 portfolio-inbox.conf /etc/systemd/system/portfolio-dashboard.service.d/inbox.conf
```

In the root-owned dashboard environment file, set `DASHBOARD_INBOX_READERS` and `DASHBOARD_INBOX_ADMINS` to the approved exact human identities. Set `DASHBOARD_ADMIN_ORIGIN` to the exact HTTPS Serve origin including port 8443. An administrator must also qualify to view and read the inbox. Keep the environment file mode 0600.

Schedule a short cutover and wait for any pending public release. Install the two reviewed binaries before starting the new units. A bare restart with the old public binary does not implement the shared lock protocol.

```sh
systemctl daemon-reload
systemctl stop portfolio-dashboard.socket portfolio-dashboard.service portfolio-inbox-actions.socket portfolio-inbox-actions.service portfolio.service
/usr/local/libexec/portfolio-migrate-inbox
systemctl enable --now portfolio-inbox-actions.socket
systemctl start portfolio.service portfolio-dashboard.socket portfolio-dashboard.service
systemctl is-active portfolio.service portfolio-dashboard.service portfolio-inbox-actions.socket
```

The migrator verifies stopped units, same-inode content and all live hard links before unlinking the obsolete public and read-only alias paths. It handles both initial rollout and an already-migrated store. A differing source or unexpected hard link requires investigation. Interrupted migration can be retried with the units still stopped. Keep a working operator login during cutover.

Check permissions in the running service mount namespaces without using PAM or displaying submissions. First check dashboard read access, absence of write access, public append access and lack of public directory-write permission:

```sh
dashboard_pid=$(systemctl show -P MainPID portfolio-dashboard.service)
public_pid=$(systemctl show -P MainPID portfolio.service)
nsenter --mount="/proc/$dashboard_pid/ns/mnt" -- setpriv --reuid=portfolio-dashboard --regid=portfolio-dashboard --clear-groups test -r /run/portfolio-inbox/submissions.jsonl
nsenter --mount="/proc/$dashboard_pid/ns/mnt" -- setpriv --reuid=portfolio-dashboard --regid=portfolio-dashboard --clear-groups test ! -w /run/portfolio-inbox/submissions.jsonl
nsenter --mount="/proc/$public_pid/ns/mnt" -- setpriv --reuid=portfolio-public --regid=portfolio-public --clear-groups test -w /run/portfolio-contact-store/submissions.jsonl
nsenter --mount="/proc/$public_pid/ns/mnt" -- setpriv --reuid=portfolio-public --regid=portfolio-public --clear-groups test ! -w /run/portfolio-contact-store
```

Test the public and private routes, the reader/admin distinction and the locked writer. Use synthetic fixtures to verify erasure, stale approvals, second bulk confirmation and concurrent appends. Never test permanent deletion against real messages without an explicit selection and confirmation. When the intended owner is ready on an authorized device, run `sudo /usr/local/libexec/portfolio-inbox-actions open-enrollment OWNER_LOGIN` on the Pi with the exact configured owner login. The owner then uses the private management page to create their passkey and sign in. The enrollment window closes at the first successful registration, after five minutes, or when the broker restarts. Invoking the command again during an active window does not extend it. Routine removals use the passkey session and the red hold button.

For a broker upgrade from the old SSH approval workflow, install `python3-fido2`, replace only the broker executable, broker service unit, reviewed dashboard binary and associated UI. Preserve the running store, socket, ACLs and drop-ins. Run `systemctl daemon-reload` and restart both the broker and dashboard services. This drops outstanding legacy authorizations. systemd creates the credential state directory with mode 0700. The root-only environment file remains outside both services’ visible filesystems.

## Failure recovery

To disable deletion while preserving the latest store, remove or empty the admin list, restart the dashboard, and stop/disable the broker socket and service. This invalidates outstanding approvals and preserves read-only access. Keep the flock-aware public writer, its store mount and the dashboard's read-only mount in place. Never point a running public writer at an old alias.

If the owner loses their only passkey, recover through a separately authenticated local root session. Stop the broker socket and service to revoke sessions, preserve the database in a root-only location, and remove only the intended login’s credential from the private database during a coordinated recovery. Restart the socket and explicitly open a fresh enrollment window when the owner is ready. Changing the RP hostname also requires coordinated re-enrollment.

To revert storage placement completely, stop all five units together. Confirm the old public filename and read-only pin are absent and the authoritative reviewed file has a single hard link. Move **that current inode** to `/var/lib/portfolio-public/submissions.jsonl` on the same filesystem. Remove the named ACLs, restore owner and group `portfolio-public` and mode 0600, remove the new public writer storage drop-in, and reload systemd. Start the public writer and verify its health. Restore the previous read-only dashboard setup separately if needed. Keep archive retention and restoration outside the public release pipeline.
