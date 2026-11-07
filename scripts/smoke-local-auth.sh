#!/usr/bin/env bash
#
# Phase L exit criterion: prove the *official* Firebase Auth SDKs can talk to
# `fireup local` over FIREBASE_AUTH_EMULATOR_HOST.
#
# The auth counterpart to scripts/smoke-local-firestore.sh. Builds fireup,
# starts the local server on a free port with the auth service enabled, runs the
# Node smoke test in tests/local-smoke, and tears the server down.
#
# Usage: scripts/smoke-local-auth.sh [port]
#
# Set FIREUP_SMOKE_POSTGRES_URL to run the same SDK smoke test against the
# PostgreSQL account store instead of the in-memory one. Point it only at a
# database you own; see PROJECT_STATUS.md.
#
# Needs network access the first time, to npm-install firebase-admin and the
# firebase client SDK.

set -euo pipefail

# rustup's default install location is not always on a non-login shell's PATH.
export PATH="$HOME/.cargo/bin:$PATH"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
smoke_dir="$repo_root/tests/local-smoke"
# Honour CARGO_TARGET_DIR, or the binary we build is not the one we start.
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
port="${1:-4591}"
log="$(mktemp -t fireup-local-auth-smoke.XXXXXX.log)"

server_pid=""
cleanup() {
  if [[ -n "$server_pid" ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ ! -d "$smoke_dir/node_modules/firebase-admin" ]]; then
  echo "==> installing the official Firebase Auth client libraries"
  (cd "$smoke_dir" && npm install --no-audit --no-fund)
fi

echo "==> building fireup"
cargo build --bin fireup --manifest-path "$repo_root/Cargo.toml"

# The project id has to match what the smoke test initializes its SDKs with:
# every Identity Toolkit URL carries it, and the ID tokens are scoped to it.
server_args=(local --port "$port" --project-id fireup-smoke --enable auth)
if [[ -n "${FIREUP_SMOKE_POSTGRES_URL:-}" ]]; then
  echo "==> storage: PostgreSQL (durable)"
  server_args+=(--postgres-url "$FIREUP_SMOKE_POSTGRES_URL")
else
  echo "==> storage: in-memory (set FIREUP_SMOKE_POSTGRES_URL to test the durable store)"
fi

echo "==> starting fireup local on 127.0.0.1:$port"
"$target_dir/debug/fireup" "${server_args[@]}" >"$log" 2>&1 &
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

# A durable account store keeps accounts between runs, so a second run would
# fail on the first createUser with EMAIL_EXISTS. An in-memory store starts
# empty and this is a no-op. Uses the documented emulator control endpoint —
# the same one the smoke test itself asserts is served.
echo "==> clearing accounts left by a previous run"
if ! node -e "
  fetch('http://127.0.0.1:$port/emulator/v1/projects/fireup-smoke/accounts', {
    method: 'DELETE',
  })
    .then((r) => {
      if (!r.ok) throw new Error('HTTP ' + r.status)
    })
    .catch((e) => {
      console.error('could not clear accounts: ' + e.message)
      process.exit(1)
    })
"; then
  kill "$server_pid" 2>/dev/null
  cat "$log" >&2
  exit 1
fi

echo "==> running the Auth SDK smoke test"
# Bounded for the reason given in `smoke-local-firestore.sh`: a client that hangs
# should be a red test, not a stalled CI job.
if FIREBASE_AUTH_EMULATOR_HOST="127.0.0.1:$port" timeout 120 node "$smoke_dir/auth-smoke.mjs"; then
  status=0
else
  status=$?
  if [[ "$status" -eq 124 ]]; then
    echo "the Auth SDK smoke test did not finish within 120s — the client is \
hung, not slow" >&2
  fi
  echo "--- fireup local log ---" >&2
  cat "$log" >&2
fi

rm -f "$log"
exit "$status"
