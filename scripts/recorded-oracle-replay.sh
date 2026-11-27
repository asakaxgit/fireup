#!/usr/bin/env bash
#
# Offline replay of committed recorded-oracle fixtures against fireup local.
# CI holds no Google credentials. Starts fireup local on a free port, runs the
# same doc-cited scenarios, and compares assertion maps to fixtures/.
#
# Usage: scripts/recorded-oracle-replay.sh [port]
#
# Omitting the port picks a free loopback port so parallel gate runs cannot
# collide with a smoke suite on a shared host.

set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=lib/pick-free-port.sh
source "$repo_root/scripts/lib/pick-free-port.sh"
oracle_dir="$repo_root/tests/recorded-oracle"
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
port="$(fireup_resolve_port "${1:-}")"
log="$(mktemp -t fireup-recorded-oracle.XXXXXX.log)"

server_pid=""
cleanup() {
  if [[ -n "$server_pid" ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ ! -d "$oracle_dir/node_modules" ]]; then
  echo "==> installing recorded-oracle client libraries"
  (cd "$oracle_dir" && npm install --no-audit --no-fund)
fi

echo "==> building fireup"
cargo build --bin fireup --manifest-path "$repo_root/Cargo.toml"

echo "==> starting fireup local on 127.0.0.1:$port"
"$target_dir/debug/fireup" local --port "$port" >"$log" 2>&1 &
server_pid=$!

for _ in $(seq 1 50); do
  if node -e "
    const net = require('net')
    const s = net.connect($port, '127.0.0.1')
    s.on('connect', () => { s.destroy(); process.exit(0) })
    s.on('error', () => process.exit(1))
  " 2>/dev/null; then
    break
  fi
  fireup_fail_if_server_died "$server_pid" "$log" "$port"
  sleep 0.1
done
fireup_fail_if_server_died "$server_pid" "$log" "$port"

echo "==> replaying recorded-oracle fixtures"
status=0
if FIREUP_ORACLE_LOCAL_HOST="127.0.0.1:$port" \
  timeout 120 node "$oracle_dir/replay.mjs"; then
  status=0
else
  status=$?
  if [[ "$status" -eq 124 ]]; then
    echo "recorded-oracle replay did not finish within 120s" >&2
  fi
  echo "--- fireup local log ---" >&2
  cat "$log" >&2
fi

rm -f "$log"
exit "$status"
