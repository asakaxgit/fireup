#!/usr/bin/env bash
#
# Phase 3 exit criterion: prove an *official* Datastore client SDK can talk to
# `fireup local` over DATASTORE_EMULATOR_HOST.
#
# Usage: scripts/smoke-local-datastore.sh [port]
#
# Set FIREUP_SMOKE_POSTGRES_URL to run against the PostgreSQL store.

set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
smoke_dir="$repo_root/tests/local-smoke-datastore"
# Honour CARGO_TARGET_DIR, or the binary we start is not the one we built —
# which is every run from a git worktree, where the default target directory is
# not $repo_root/target.
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
port="${1:-4589}"
log="$(mktemp -t fireup-local-datastore-smoke.XXXXXX.log)"

server_pid=""
cleanup() {
  if [[ -n "$server_pid" ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ ! -d "$smoke_dir/node_modules" ]]; then
  echo "==> installing the official Datastore client library"
  (cd "$smoke_dir" && npm install --no-audit --no-fund)
fi

echo "==> building fireup"
cargo build --bin fireup --manifest-path "$repo_root/Cargo.toml"

server_args=(local --port "$port" --project-id fireup-local --enable datastore --enable firestore)
if [[ -n "${FIREUP_SMOKE_POSTGRES_URL:-}" ]]; then
  echo "==> storage: PostgreSQL (durable)"
  server_args+=(--postgres-url "$FIREUP_SMOKE_POSTGRES_URL")
else
  echo "==> storage: in-memory"
fi

echo "==> starting fireup local on 127.0.0.1:$port"
"$target_dir/debug/fireup" "${server_args[@]}" >"$log" 2>&1 &
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
  if ! kill -0 "$server_pid" 2>/dev/null; then
    echo "fireup local exited before accepting connections:" >&2
    cat "$log" >&2
    exit 1
  fi
  sleep 0.1
done

echo "==> running the Datastore SDK smoke test"
if DATASTORE_EMULATOR_HOST="127.0.0.1:$port" \
   FIREUP_SMOKE_PROJECT_ID=fireup-local \
   node "$smoke_dir/smoke.mjs"; then
  status=0
else
  status=$?
  echo "--- fireup local log ---" >&2
  cat "$log" >&2
fi

rm -f "$log"
exit "$status"
