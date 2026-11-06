#!/usr/bin/env bash
set -Eeuo pipefail
umask 077
base=${1:?deployment directory required}
release_id=${2:?release ID required}
[[ "$base" = /* && "$release_id" =~ ^[a-zA-Z0-9_-]+$ ]] || { echo 'Invalid deployment path or release ID' >&2; exit 1; }
release="$base/releases/$release_id"
unit_dir=${PORTFOLIO_UNIT_DIR:-/etc/systemd/system}
mkdir -p "$base/data"
exec 9>"$base/.deploy.lock"
flock -n 9 || { echo 'Another deployment is active' >&2; exit 1; }
[[ ! -e "$base/current" || -L "$base/current" ]] || { echo 'current must be a symlink' >&2; exit 1; }
(cd "$release" && sha256sum -c SHA256SUMS)
[[ -s "$release/portfolio-serve" && -s "$release/assets/index.html" && -s "$release/assets/resume.pdf" ]]
chmod +x "$release/portfolio-serve"
previous=$(readlink "$base/current" || true)
backup=$(mktemp "$base/.portfolio-unit.XXXXXX")
had_unit=false
if sudo test -f "$unit_dir/portfolio.service"; then
  sudo cat "$unit_dir/portfolio.service" > "$backup"
  had_unit=true
fi
rollback() {
  local result=$?
  trap - ERR
  echo 'Release failed; restoring previous deployment' >&2
  sudo systemctl stop portfolio.service || true
  if [[ -n "$previous" ]]; then
    ln -sfn "$previous" "$base/current.next"
    mv -Tf "$base/current.next" "$base/current"
  else
    rm -f "$base/current"
  fi
  if $had_unit; then
    sudo cp "$backup" "$unit_dir/portfolio.service"
    sudo systemctl daemon-reload
    sudo systemctl start portfolio.service || true
  else
    sudo rm -f "$unit_dir/portfolio.service"
    sudo systemctl daemon-reload
  fi
  rm -f "$backup"
  exit "$result"
}
trap rollback ERR
# All files are uploaded and verified before interrupting the running process.
if $had_unit; then sudo systemctl stop portfolio.service; fi
# Keep the original as a backup; migrate only on the first upgrade.
if [[ -f "$base/submissions.json" && ! -e "$base/data/submissions.jsonl" ]]; then
  cp "$base/submissions.json" "$base/data/submissions.jsonl.tmp"
  chmod 600 "$base/data/submissions.jsonl.tmp"
  sync "$base/data/submissions.jsonl.tmp"
  mv "$base/data/submissions.jsonl.tmp" "$base/data/submissions.jsonl"
fi
ln -sfn "$release" "$base/current.next"
mv -Tf "$base/current.next" "$base/current"
sudo cp "$release/portfolio.service" "$unit_dir/portfolio.service"
sudo systemctl daemon-reload
sudo systemctl enable portfolio.service
sudo systemctl start portfolio.service
healthy=false
for attempt in {1..20}; do
  if curl --fail --silent --max-time 2 http://127.0.0.1:6050/healthz | grep -qx 'ok'; then
    healthy=true
    break
  fi
  sleep 1
done
$healthy
sudo systemctl is-active --quiet portfolio.service
# Existing cloudflared keeps running during ordinary deployments.
if ! sudo test -f "$unit_dir/cloudflared.service"; then
  sudo cp "$release/cloudflared.service" "$unit_dir/cloudflared.service"
  sudo systemctl daemon-reload
  sudo systemctl enable cloudflared.service
fi
sudo systemctl is-active --quiet cloudflared.service || sudo systemctl start cloudflared.service
trap - ERR
rm -f "$backup"
echo "Activated release $release_id"
