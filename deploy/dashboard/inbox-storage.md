# Contact inbox storage

The public writer keeps one open append handle to `/var/lib/portfolio-public/submissions.jsonl`. A root-owned directory on the same filesystem holds a second hard link to that inode. The dashboard service receives a read-only mount of this one file in its private mount namespace. A named POSIX ACL grants its UID read access. Root checks the source type, ownership, inode identity and existing ACL before adding the reader. A later public-side rename or symlink replacement cannot redirect the private mount. The public site's SMTP credentials and state directory remain inaccessible to the dashboard UID.

On the Pi, stage the two files from this directory and use the following steps as root, after installing the inbox-capable dashboard binary. `prepare-inbox.py` requires the Debian `acl` utilities. Run it only on an existing private submissions file owned by the public runtime. An unknown existing ACL or an unexpected hard link requires operator review. Configure the inbox identity allowlist `DASHBOARD_INBOX_READERS` in the app's root-owned environment file before enabling the mount.

```sh
install -o root -g root -m 0755 prepare-inbox.py /usr/local/libexec/portfolio-prepare-inbox
/usr/local/libexec/portfolio-prepare-inbox
install -d -o root -g root -m 0755 /etc/systemd/system/portfolio-dashboard.service.d
install -o root -g root -m 0644 portfolio-inbox.conf /etc/systemd/system/portfolio-dashboard.service.d/inbox.conf
systemctl daemon-reload
systemctl restart portfolio-dashboard.service
systemctl is-active portfolio-dashboard.service
```

Check the actual service namespace without printing submissions. Run as root:

```sh
pid=$(systemctl show --property MainPID --value portfolio-dashboard.service)
nsenter --mount="/proc/$pid/ns/mnt" -- runuser -u portfolio-dashboard -- test -r /run/portfolio-inbox/submissions.jsonl
nsenter --mount="/proc/$pid/ns/mnt" -- runuser -u portfolio-dashboard -- test ! -w /run/portfolio-inbox/submissions.jsonl
runuser -u portfolio-dashboard -- test ! -r /var/lib/portfolio-public/submissions.jsonl
```

**Rotation and restore.** Renaming or replacing the source leaves running readers and writers attached to the original inode. Schedule downtime, stop the dashboard socket and service and the public writer, back up the current store as root, and check the source and pin inode numbers. Revoke the reader ACL from the old regular pin (`setfacl -x u:portfolio-dashboard /var/lib/portfolio-inbox/submissions.jsonl`) and remove just that second hard link. Preserve the authoritative archive with root-only permissions. Put the replacement JSONL file at the documented source with owner/group `portfolio-public` and mode 0600. Re-run the preparer, start the public service and dashboard socket/service, then verify the namespace and the two web views. The public release workflow does not need to touch these files.

**Rollback.** Stop the dashboard socket and service, remove the inbox drop-in, revoke the ACL from the pinned inode and remove only the private hard link. Unset inbox access in the private runtime configuration. Reload systemd and start the dashboard socket/service. The original public submission file and public writer remain in place.
