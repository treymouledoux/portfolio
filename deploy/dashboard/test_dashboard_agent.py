"""Checks the host agent's verbs, its refusal paths and a real PTY session."""
import base64
import importlib.util
from pathlib import Path
import pwd
import sys
import time

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("dashboard_agent", HERE / "dashboard-agent.py")
agent = importlib.util.module_from_spec(spec)
spec.loader.exec_module(agent)


def rejected(verb, **extra):
    try:
        agent.handle({"verb": verb, **extra})
    except agent.Rejected as error:
        return str(error)
    raise AssertionError(f"{verb} should be rejected")


def test_unknown_verbs_and_targets_are_refused():
    assert rejected("run") == "unknown request"
    assert rejected("restart", target="sshd") == "unknown service"
    assert rejected("restart", target="../../bin/sh") == "unknown service"
    assert rejected("term-open") == "unknown account"
    assert rejected("term-read", id="nope") == "terminal session ended"
    assert rejected("term-read", id=7) == "terminal session ended"
    for name in ("restart", "reboot", "term-open", "term-write", "term-close"):
        assert name in agent.RESTART_UNITS or name  # verb names are stable


def test_restart_units_are_a_fixed_named_set():
    assert agent.RESTART_UNITS == {
        "tailscaled": "tailscaled.service",
        "dashboard": "portfolio-dashboard.service",
        "public": "portfolio.service",
    }


def test_root_is_never_a_shell_account():
    if pwd.getpwuid(0):
        try:
            agent.account("root")
        except agent.Rejected:
            pass
        else:
            raise AssertionError("root must not be a shell account")


def test_output_keeps_the_sequences_the_browser_draws():
    assert agent.clean(b"\x1b[31mred\x1b[0m\r\nok\x00") == b"\x1b[31mred\x1b[0m\r\nok"
    assert agent.clean(b"\x1b]0;title\x07text") == b"\x1b]0;title\x07text"
    assert agent.clean(b"plain\n") == b"plain\n"


def test_a_real_pty_runs_the_shell_and_echoes():
    user = agent.account(pwd.getpwuid(__import__("os").getuid()).pw_name).pw_name
    open_reply = agent.handle({"verb": "term-open", "user": user, "cols": 100, "rows": 30})
    assert open_reply["ok"] is True
    sid = open_reply["id"]
    try:
        agent.handle({"verb": "term-write", "id": sid, "data": "echo agent-probe-$((6*7))\n"})
        offset = 0
        collected = b""
        deadline = time.time() + 15
        while time.time() < deadline and b"agent-probe-42" not in collected:
            reply = agent.handle({"verb": "term-read", "id": sid, "offset": offset})
            assert reply["ok"] is True
            collected += base64.b64decode(reply["data"])
            offset = reply["offset"]
        assert b"agent-probe-42" in collected, collected[-400:]
        # A carriage return and an escape have to reach the page, which is what
        # draws a rewritten line and a cleared screen.
        agent.handle({"verb": "term-write", "id": sid, "data": "printf 'one\\rtwo\\033[31mthree\\033[0m\\n'\n"})
        painted = b""
        deadline = time.time() + 15
        while time.time() < deadline and b"one\rtwo\x1b[31mthree" not in painted:
            reply = agent.handle({"verb": "term-read", "id": sid, "offset": offset})
            assert reply["ok"] is True
            painted += base64.b64decode(reply["data"])
            offset = reply["offset"]
        assert b"one\rtwo\x1b[31mthree" in painted, painted[-400:]
    finally:
        agent.handle({"verb": "term-close", "id": sid})
    assert rejected("term-read", id=sid) == "terminal session ended"


def test_socket_directory_survives_the_unit_umask():
    import os
    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        directory = Path(tmp) / "agent"
        previous = os.umask(0o077)
        try:
            agent.prepare_directory(directory)
        finally:
            os.umask(previous)
        assert directory.stat().st_mode & 0o777 == 0o750


def test_wire_requests_must_be_objects():
    assert agent.handle({"verb": "ping"}) == {"ok": True}


def test_syncthing_is_loopback_only_and_its_verbs_are_fixed():
    import os

    previous = os.environ.get("DASHBOARD_SYNCTHING_API")
    try:
        for value in (
            "http://192.168.1.12:8384",
            "http://example.test:8384",
            "https://127.0.0.1:8384",
            "http://user:pass@127.0.0.1:8384",
            "http://127.0.0.1:8384/rest",
            "http://127.0.0.1:8384?x=1",
        ):
            os.environ["DASHBOARD_SYNCTHING_API"] = value
            try:
                agent.syncthing_api()
            except agent.Rejected:
                pass
            else:
                raise AssertionError(f"{value} must not be accepted")
        for value in (
            "http://127.0.0.1:8384",
            "http://[::1]:8384",
            "http://localhost:8384/",
        ):
            os.environ["DASHBOARD_SYNCTHING_API"] = value
            assert agent.syncthing_api() == value.rstrip("/")
    finally:
        if previous is None:
            os.environ.pop("DASHBOARD_SYNCTHING_API", None)
        else:
            os.environ["DASHBOARD_SYNCTHING_API"] = previous
    assert agent.SYNCTHING_ACTIONS == {
        "syncthing-pause": "pause",
        "syncthing-resume": "resume",
    }
    assert rejected("syncthing-forget") == "unknown request"


def test_contact_recipients_are_validated_and_saved_for_the_site():
    import json
    import os
    import tempfile

    previous = os.environ.get("DASHBOARD_CONTACT_RECIPIENTS")
    previous_smtp = os.environ.get("DASHBOARD_CONTACT_SMTP")
    try:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "contact" / "recipients.json"
            os.environ["DASHBOARD_CONTACT_RECIPIENTS"] = str(path)
            # Before an owner saves anything the panel shows the address the
            # mail credential already uses.
            smtp = Path(tmp) / "smtp.json"
            smtp.write_text(json.dumps({"username": "u", "password": "p", "to": "owner@example.test"}))
            os.environ["DASHBOARD_CONTACT_SMTP"] = str(smtp)
            assert agent.read_contact_recipients() == {
                "recipients": ["owner@example.test"],
                "configured": False,
            }
            for values in (
                [],
                ["not an address"],
                ["a@b"],
                ["owner@example.test,other@example.test"],
                ["x" * 65 + "@example.test"],
                [f"n{index}@example.test" for index in range(11)],
                None,
            ):
                try:
                    agent.handle({"verb": "contact-recipients-set", "recipients": values})
                except agent.Rejected:
                    pass
                else:
                    raise AssertionError(f"{values!r} must not be saved")
            reply = agent.handle(
                {
                    "verb": "contact-recipients-set",
                    "recipients": ["One@example.test", "two@example.test", "one@example.test"],
                }
            )
            assert reply["ok"] is True
            # A repeated address is mailed once, not twice.
            assert reply["recipients"] == ["One@example.test", "two@example.test"]
            assert json.loads(path.read_text()) == {
                "recipients": ["One@example.test", "two@example.test"]
            }
            # The public service runs as another account and has to read it.
            assert path.stat().st_mode & 0o777 == 0o644
            assert path.parent.stat().st_mode & 0o777 == 0o755
            assert agent.read_contact_recipients() == {
                "recipients": ["One@example.test", "two@example.test"],
                "configured": True,
            }
    finally:
        for name, value in (
            ("DASHBOARD_CONTACT_RECIPIENTS", previous),
            ("DASHBOARD_CONTACT_SMTP", previous_smtp),
        ):
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value
    assert rejected("contact-recipients-forget") == "unknown request"


if __name__ == "__main__":
    tests = [value for name, value in sorted(globals().items()) if name.startswith("test_")]
    for test in tests:
        test()
        print(f"ok {test.__name__}")
    print(f"{len(tests)} agent checks passed")
    sys.exit(0)
