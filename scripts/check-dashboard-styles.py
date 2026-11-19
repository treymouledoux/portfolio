"""Check the CSP pins against the exact inline style and script bytes sent to browsers."""

import base64
import hashlib
from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
PAGES = {
    "portfolio-serve/src/dashboard-denied.html": "portfolio-serve/src/main.rs",
    "deploy/dashboard/messages.html": "deploy/dashboard/Caddyfile",
    "deploy/dashboard/landing.html": "deploy/dashboard/Caddyfile",
    "portfolio-dashboard/src/inbox-badge.html": "portfolio-dashboard/src/inbox.rs",
    "portfolio-dashboard/src/embed.html": "portfolio-dashboard/src/main.rs",
    "portfolio-dashboard/src/denied.html": "portfolio-dashboard/src/main.rs",
}


def check_inline(page, policy, tag, directive):
    (content,) = re.findall(fr"<{tag}>(.*?)</{tag}>".encode(), (ROOT / page).read_bytes(), re.S)
    digest = base64.b64encode(hashlib.sha256(content).digest()).decode("ascii")
    directives = re.findall(fr"{directive} ([^;]+)", (ROOT / policy).read_text())
    if not any(f"'sha256-{digest}'" in sources.split() for sources in directives):
        raise SystemExit(f"The {directive} hash in {policy} does not match {page}")


for page, policy in PAGES.items():
    check_inline(page, policy, "style", "style-src")
    if page == "deploy/dashboard/landing.html":
        check_inline(page, policy, "script", "script-src")
    print(f"Verified {page}")
