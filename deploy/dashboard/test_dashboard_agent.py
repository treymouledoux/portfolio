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


def test_output_is_plain_text():
    assert agent.clean(b"\x1b[31mred\x1b[0m\r\nok\x00") == b"red\nok"
    assert agent.clean(b"\x1b]0;title\x07text") == b"text"
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
        assert b"\x1b" not in collected
    finally:
        agent.handle({"verb": "term-close", "id": sid})
    assert rejected("term-read", id=sid) == "terminal session ended"


def test_wire_requests_must_be_objects():
    assert agent.handle({"verb": "ping"}) == {"ok": True}


if __name__ == "__main__":
    tests = [value for name, value in sorted(globals().items()) if name.startswith("test_")]
    for test in tests:
        test()
        print(f"ok {test.__name__}")
    print(f"{len(tests)} agent checks passed")
    sys.exit(0)
