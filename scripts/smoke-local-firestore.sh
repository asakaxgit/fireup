#!/usr/bin/env bash
#
# Phase 0 exit criterion: prove an *official* Firestore client SDK can talk to
# `fireup local` over FIRESTORE_EMULATOR_HOST.
#
# Builds fireup, starts the local server on a free port, runs the Node smoke
# test in tests/local-smoke, and tears the server down.
#
# Usage: scripts/smoke-local-firestore.sh [port]
#
# Needs network access the first time, to npm-install @google-cloud/firestore.

set -euo pipefail

# rustup's default install location is not always on a non-login shell's PATH.
export PATH="$HOME/.cargo/bin:$PATH"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
smoke_dir="$repo_root/tests/local-smoke"
port="${1:-4589}"
log="$(mktemp -t fireup-local-smoke.XXXXXX.log)"

server_pid=""
cleanup() {
  if [[ -n "$server_pid" ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ ! -d "$smoke_dir/node_modules" ]]; then
  echo "==> installing the official Firestore client library"
  (cd "$smoke_dir" && npm install --no-audit --no-fund)
fi

echo "==> building fireup"
cargo build --bin fireup --manifest-path "$repo_root/Cargo.toml"

echo "==> starting fireup local on 127.0.0.1:$port"
"$repo_root/target/debug/fireup" local --port "$port" >"$log" 2>&1 &
server_pid=$!

# Wait for the port to accept connections rather than sleeping a guess.
for _ in $(seq 1 50); do
  if node -e "
    const net = require('net')
    const s = net.connect($port, '127.0.0.1')
    s.on('connect', () => { s.destroy(); process.exit(0) })
    s.on('error', () => process.exit(1))
  " 2>/dev/null; then
    break
  fi
  if ! kill -0 "$server_pid" 2>/dev/null; then
    echo "fireup local exited before accepting connections:" >&2
    cat "$log" >&2
    exit 1
  fi
  sleep 0.1
done

echo "==> running the SDK smoke test"
if FIRESTORE_EMULATOR_HOST="127.0.0.1:$port" node "$smoke_dir/smoke.mjs"; then
  status=0
else
  status=$?
  echo "--- fireup local log ---" >&2
  cat "$log" >&2
fi

rm -f "$log"
exit "$status"
