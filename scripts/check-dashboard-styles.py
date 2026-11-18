"""Check the CSP pins against the exact inline CSS bytes sent to browsers."""

import base64
import hashlib
from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
PAGES = {
    "portfolio-serve/src/dashboard-denied.html": "portfolio-serve/src/main.rs",
    "deploy/dashboard/landing.html": "deploy/dashboard/Caddyfile",
    "portfolio-dashboard/src/embed.html": "portfolio-dashboard/src/main.rs",
    "portfolio-dashboard/src/denied.html": "portfolio-dashboard/src/main.rs",
}

for page, policy in PAGES.items():
    (css,) = re.findall(rb"<style>(.*?)</style>", (ROOT / page).read_bytes(), re.S)
    digest = base64.b64encode(hashlib.sha256(css).digest()).decode("ascii")
    if f"style-src 'sha256-{digest}'" not in (ROOT / policy).read_text():
        raise SystemExit(f"The style-src hash in {policy} does not match {page}")
    print(f"Verified {page}")
