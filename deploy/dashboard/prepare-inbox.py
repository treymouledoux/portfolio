#!/usr/bin/python3
"""Pin the public writer's existing regular file for the private inbox mount."""
import os
import pwd
import stat
import subprocess
from pathlib import Path

SOURCE = Path("/var/lib/portfolio-public/submissions.jsonl")
PRIVATE = Path("/var/lib/portfolio-inbox")
PIN = PRIVATE / "submissions.jsonl"


def regular_writer_file(info, writer):
    return (
        stat.S_ISREG(info.st_mode)
        and info.st_uid == writer.pw_uid
        and info.st_gid == writer.pw_gid
    )


def main():
    if os.geteuid() != 0:
        raise SystemExit("Run as root.")
    writer = pwd.getpwnam("portfolio-public")
    reader = pwd.getpwnam("portfolio-dashboard").pw_uid
    PRIVATE.mkdir(mode=0o700, exist_ok=True)
    directory = PRIVATE.lstat()
    if not stat.S_ISDIR(directory.st_mode) or directory.st_uid != 0 or stat.S_IMODE(directory.st_mode) != 0o700:
        raise SystemExit("The private pin directory must be root-owned with mode 0700.")

    # The source directory is writable by the public service. Never follow a
    # symlink from it. Perform all privileged ACL operations on the pinned
    # entry inside the root-only directory, after checking its identity.
    source = SOURCE.lstat()
    if not regular_writer_file(source, writer):
        raise SystemExit("Expected a regular file owned by the public writer.")
    created = False
    try:
        try:
            os.link(SOURCE, PIN, follow_symlinks=False)
            created = True
        except FileExistsError:
            pass
        pinned = PIN.lstat()
        if not regular_writer_file(pinned, writer) or not os.path.samestat(source, pinned) or pinned.st_nlink != 2:
            raise SystemExit("Unexpected pin or link count. Check the inbox rotation procedure.")

        original = {"user::rw-", "group::---", "other::---"}
        managed = original | {f"user:{reader}:r--", "mask::r--"}
        acl = subprocess.check_output(
            ["/usr/bin/getfacl", "-cpEn", "--", PIN], text=True
        )
        entries = set(acl.split())
        if entries == original:
            subprocess.run(["/usr/bin/setfacl", "-m", f"u:{reader}:r--", "--", PIN], check=True)
        elif entries != managed:
            raise SystemExit("Unexpected existing ACL. Inspect the pin before changing its readers.")
    except BaseException:
        if created:
            PIN.unlink()
        raise
    print("Pinned the existing submissions file for read-only mounting.")


if __name__ == "__main__":
    main()
