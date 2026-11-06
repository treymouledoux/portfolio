# Portfolio server

Axum serves the exported Next.js site, the contact endpoint, profile redirects, and the resume. The site remains a static export; Node is only needed for building/development.

## Run locally

From the repository root:

```sh
cd portfolio-static
pnpm install --frozen-lockfile
pnpm build
cd ..
node scripts/precompress.mjs portfolio-static/out
PORTFOLIO_ASSETS_DIR="$PWD/portfolio-static/out" PORTFOLIO_DATA_DIR="$PWD/portfolio-serve/data" cargo run --manifest-path portfolio-serve/Cargo.toml
```

Open `http://127.0.0.1:6050`. This preview exercises the real Rust backend, including `/api/contact` and `/resume`. The separate Next.js dev server only previews the frontend.

Environment variables:

| Variable | Default | Purpose |
| --- | --- | --- |
| `PORTFOLIO_BIND` | `127.0.0.1:6050` | Listening address |
| `PORTFOLIO_ASSETS_DIR` | `assets` | Exported frontend directory; must include `index.html` and `resume.pdf` |
| `PORTFOLIO_DATA_DIR` | `data` | Persistent private submission storage, outside the assets directory |
| `PORTFOLIO_TRUST_CLOUDFLARE` | unset/false | Honor `CF-Connecting-IP` only from loopback peers; requires a loopback listener |

The production unit enables Cloudflare header trust because the local cloudflared tunnel supplies the client address. Keep the backend bound to loopback; do not enable this setting behind an untrusted proxy. Without a trusted header, limits use the socket peer address.

## HTTP behavior

- `/github`, `/instagram`, `/linkedin`: temporary (307) redirects, so profile destinations can change without permanent browser caching.
- `/resume`: serves `public/resume.pdf` inline as a PDF. Replace that file and rebuild to update it; `/resume.pdf` also works.
- `/_next/static/*`: successful hashed assets are immutable for one year.
- HTML and resume: `no-cache` (revalidate before reuse); ordinary images/assets: one hour; API, health, redirects and errors: `no-store`.
- Brotli/gzip sidecars are generated at build time. Compression middleware handles remaining eligible responses. PDF and raster images are not precompressed by the build script.
- `/api/contact`: maximum 64 KiB JSON body, five attempts per IP per minute, and one successful submission per IP per hour. Returns JSON errors and `Retry-After` for HTTP 429. Shared public IPs share limits. Limits are in-memory, reset on process restart, and track at most 4,096 clients; additional clients receive 503 until entries expire.
- Submissions are serialized through one writer to `submissions.jsonl`, flushed and synced before success. A write failure marks storage unhealthy until restart, avoiding additional appends to a potentially partial record. Inspect/repair the last record before restarting after a disk error. Logs omit submission names, addresses, and messages.
- `/healthz`: 200 while storage is healthy; 503 after a persistence failure. SIGTERM/Ctrl-C drain active requests; systemd allows 30 seconds before forced termination.

## Deployment and rollback

CI checks/tests Rust, builds both applications, precompresses static files, and uploads a complete release to `/home/trey/portfolio-deployment/releases/<run>-<attempt>`. A checksum manifest is verified before the running service is stopped. The activation script switches the `current` symlink atomically, installs the service unit, starts the server, and checks health. Failure restores the previous symlink/service unit (including the original flat deployment on first migration) and attempts to restart it. Existing cloudflared stays running during normal deployments.

Submissions live in `/home/trey/portfolio-deployment/data`, independent of releases. On the first upgrade, the stopped server's original `submissions.json` is copied to `data/submissions.jsonl` if no new data file exists. The original is retained as a backup. Releases are retained for rollback; remove obsolete releases manually once satisfied, preserving `current` and a known-good previous release. Never delete `data` with release cleanup.

The provided systemd unit targets user `trey` and the directory above. Update the unit together with the workflow if changing either. Cloudflared must forward the portfolio hostname to `http://127.0.0.1:6050` (or `localhost:6050`).

Checks:

```sh
cargo fmt --manifest-path portfolio-serve/Cargo.toml --check
cargo clippy --locked --manifest-path portfolio-serve/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path portfolio-serve/Cargo.toml
python3 scripts/test-deployment.py
```
