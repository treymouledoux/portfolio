#!/usr/bin/python3
"""Move the stopped public writer's live inode into the broker-owned store."""
import contextlib
import os
from pathlib import Path
import pwd
import stat
import subprocess
import sys

SOURCE = Path("/var/lib/portfolio-public/submissions.jsonl")
OLD_PIN = Path("/var/lib/portfolio-inbox/submissions.jsonl")
STORE = Path("/var/lib/portfolio-inbox-store")
UNITS = (
    "portfolio.service", "portfolio-dashboard.service", "portfolio-dashboard.socket",
    "portfolio-inbox-actions.service", "portfolio-inbox-actions.socket",
)


def optional_stat(path):
    try:
        return path.lstat()
    except FileNotFoundError:
        return None


def set_acl(fd, acl):
    # Apply only to the previously checked inode. The path in a writer-owned
    # directory may be subject to replacement; do not operate on that path.
    subprocess.run(
        ["/usr/bin/setfacl", "--set", acl, "--", f"/proc/self/fd/{fd}"],
        pass_fds=(fd,), check=True,
    )


def stopped():
    for unit in UNITS:
        result = subprocess.check_output(
            ["systemctl", "show", unit, "-p", "ActiveState", "--value"], text=True
        ).strip()
        if result not in {"inactive", "failed"}:
            raise SystemExit(f"Stop {unit} before running the migration.")


@contextlib.contextmanager
def checked_fd(path, flags, allowed_owners, is_directory=False):
    fd = os.open(path, flags | os.O_NOFOLLOW | os.O_NONBLOCK, mode=0o600)
    try:
        info = os.fstat(fd)
        expected_type = stat.S_ISDIR if is_directory else stat.S_ISREG
        if not expected_type(info.st_mode) or info.st_uid not in allowed_owners:
            raise SystemExit(f"Unexpected type or owner for {path}.")
        yield fd
    finally:
        os.close(fd)


def sync_dir(path):
    fd = os.open(path, os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def migrate(source, old_pin, directory, writer, broker, reader):
    directory.mkdir(mode=0o700, exist_ok=True)
    data = directory / "submissions.jsonl"
    lock = directory / "submissions.lock"
    with checked_fd(directory, os.O_DIRECTORY, {0, broker.pw_uid}, is_directory=True) as directory_fd:
        previous = optional_stat(source)
        current = optional_stat(data)
        if current is None:
            if previous is None or not stat.S_ISREG(previous.st_mode) or previous.st_uid != writer.pw_uid:
                raise SystemExit("Expected the existing public writer's regular submissions file.")
            os.link(source, data, follow_symlinks=False)
        with checked_fd(data, os.O_RDONLY, {writer.pw_uid, broker.pw_uid}) as data_fd:
            inode = os.fstat(data_fd)
            aliases = []
            for old in (source, old_pin):
                entry = optional_stat(old)
                if entry is not None:
                    if not os.path.samestat(inode, entry):
                        raise SystemExit(f"{old} differs from the authoritative inode. Reconcile manually.")
                    aliases.append(old)
            if inode.st_nlink != 1 + len(aliases):
                raise SystemExit("Unexpected extra hard link to submission data. Reconcile before proceeding.")
            if not aliases and inode.st_uid != broker.pw_uid:
                raise SystemExit("Cannot verify the original writer's data.")

            # All service users are stopped. Changes become authoritative at
            # the new link first. Retry after interruption before starting them.
            with checked_fd(lock, os.O_CREAT | os.O_RDONLY, {0, broker.pw_uid}) as lock_fd:
                if os.fstat(lock_fd).st_nlink != 1 or os.fstat(lock_fd).st_size != 0:
                    raise SystemExit("Expected a dedicated empty writer lock.")
                os.fchown(lock_fd, broker.pw_uid, broker.pw_gid)
                set_acl(lock_fd, f"u::rw-,u:{writer.pw_uid}:r--,g::---,m::r--,o::---")
                os.fsync(lock_fd)
            os.fchown(data_fd, broker.pw_uid, broker.pw_gid)
            set_acl(data_fd, f"u::rw-,u:{writer.pw_uid}:rw-,u:{reader.pw_uid}:r--,g::---,m::rw-,o::---")
            os.fsync(data_fd)
            os.fchown(directory_fd, broker.pw_uid, broker.pw_gid)
            set_acl(directory_fd, f"u::rwx,u:{writer.pw_uid}:--x,u:{reader.pw_uid}:--x,g::---,m::--x,o::---")
            os.fsync(directory_fd)
            sync_dir(directory.parent)
            for old in aliases:
                if not os.path.samestat(os.fstat(data_fd), old.lstat()):
                    raise SystemExit(f"Unexpected source change at {old}; inspect before starting services.")
                old.unlink()
                sync_dir(old.parent)


def main():
    if os.geteuid() != 0:
        raise SystemExit("Run on the Pi as root.")
    if sys.argv[1:]:
        raise SystemExit("This command takes no arguments.")
    stopped()
    migrate(SOURCE, OLD_PIN, STORE, pwd.getpwnam("portfolio-public"),
            pwd.getpwnam("portfolio-inbox-actions"), pwd.getpwnam("portfolio-dashboard"))
    print("Verified the authoritative file and removed obsolete live aliases.")


if __name__ == "__main__":
    main()
