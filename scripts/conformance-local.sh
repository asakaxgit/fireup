#!/usr/bin/env bash
#
# Run the whole SDK conformance suite against `fireup local`: every row of the
# fidelity matrix in docs/local/README.md, driven by Google's official client
# libraries (and by raw gRPC over the vendored public protos for the RPCs and
# request fields no library will emit).
#
# Three layers, each proving something the others cannot:
#
#   1. Node  (required)  tests/conformance/node     — the whole matrix
#   2. Auth  (required)  scripts/smoke-local-auth.sh — the Identity Toolkit rows
#   3. Python (optional) tests/conformance/python   — a second language's SDKs
#
# Usage: scripts/conformance-local.sh [port]
#
#   FIREUP_CONFORMANCE_POSTGRES_URL   run against the durable store instead of
#                                     memory. Point it only at a database you
#                                     own; see PROJECT_STATUS.md.
#   FIREUP_CONFORMANCE_PYTHON         python interpreter for layer 3
#                                     (default: python3)
#   FIREUP_CONFORMANCE_SKIP_PYTHON=1  skip layer 3 outright
#   FIREUP_CONFORMANCE_SKIP_AUTH=1    skip layer 2
#   FIREUP_SMOKE_RESULTS              write layer 1's probes to this file for
#                                     the fidelity gate (tests/smoke-gate/);
#                                     scripts/fidelity-smoke-gate.sh sets it,
#                                     along with FIREUP_SMOKE_SUITE
#
# Needs network access the first time, to npm-install the client libraries.
# The Python layer *skips with a message* when its libraries are missing — it
# never reports a missing optional dependency as a pass.

set -euo pipefail

# rustup's default install location is not always on a non-login shell's PATH.
export PATH="$HOME/.cargo/bin:$PATH"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
node_dir="$repo_root/tests/conformance/node"
python_dir="$repo_root/tests/conformance/python"
# Honour CARGO_TARGET_DIR, or the binary we start is not the one we built —
# which is also what makes this work from a git worktree.
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
port="${1:-4589}"
log="$(mktemp -t fireup-conformance.XXXXXX.log)"

server_pid=""
cleanup() {
  if [[ -n "$server_pid" ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ ! -d "$node_dir/node_modules" ]]; then
  echo "==> installing the official Node client libraries"
  (cd "$node_dir" && npm install --no-audit --no-fund)
fi

echo "==> building fireup"
cargo build --bin fireup --manifest-path "$repo_root/Cargo.toml"

server_args=(local --port "$port")
if [[ -n "${FIREUP_CONFORMANCE_POSTGRES_URL:-}" ]]; then
  echo "==> storage: PostgreSQL (durable)"
  server_args+=(--postgres-url "$FIREUP_CONFORMANCE_POSTGRES_URL")
else
  echo "==> storage: in-memory (set FIREUP_CONFORMANCE_POSTGRES_URL for the durable store)"
fi

echo "==> starting fireup local on 127.0.0.1:$port"
"$target_dir/debug/fireup" "${server_args[@]}" >"$log" 2>&1 &
server_pid=$!

# Wait for the port to accept connections rather than sleeping a guess.
for _ in $(seq 1 100); do
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

export FIRESTORE_EMULATOR_HOST="127.0.0.1:$port"
export DATASTORE_EMULATOR_HOST="127.0.0.1:$port"
export FIREBASE_AUTH_EMULATOR_HOST="127.0.0.1:$port"

status=0
dump_log_on_failure() {
  echo "--- fireup local log ---" >&2
  cat "$log" >&2
}

echo
echo "==> layer 1/3: the fidelity matrix, with the official Node SDKs"
# `timeout`, as in the smoke scripts: an SDK client that never settles would
# otherwise hang the fidelity gate's required check until the job is killed.
if ! timeout 600 node "$node_dir/runner.mjs"; then
  status=1
  dump_log_on_failure
fi

if [[ "${FIREUP_CONFORMANCE_SKIP_AUTH:-}" == "1" ]]; then
  echo
  echo "==> layer 2/3: auth surface — skipped (FIREUP_CONFORMANCE_SKIP_AUTH=1)"
else
  echo
  echo "==> layer 2/3: the auth rows, with firebase-admin and the firebase client SDK"
  # smoke-local-auth.sh starts its own server on its own port, which keeps the
  # auth accounts out of this run's document store. FIREUP_SMOKE_RESULTS is
  # layer 1's file: left set, the auth suite would overwrite it.
  if ! env -u FIREUP_SMOKE_RESULTS -u FIREUP_SMOKE_SUITE \
    "$repo_root/scripts/smoke-local-auth.sh" "$((port + 1))"; then
    status=1
  fi
fi

python_bin="${FIREUP_CONFORMANCE_PYTHON:-python3}"
if [[ "${FIREUP_CONFORMANCE_SKIP_PYTHON:-}" == "1" ]]; then
  echo
  echo "==> layer 3/3: Python SDKs — skipped (FIREUP_CONFORMANCE_SKIP_PYTHON=1)"
elif ! command -v "$python_bin" >/dev/null 2>&1; then
  echo
  echo "==> layer 3/3: Python SDKs — SKIPPED: no $python_bin on PATH"
elif ! "$python_bin" -c 'import google.cloud.firestore, google.cloud.datastore' 2>/dev/null; then
  echo
  echo "==> layer 3/3: Python SDKs — SKIPPED: the official libraries are not installed"
  echo "    $python_bin -m pip install -r tests/conformance/python/requirements.txt"
else
  echo
  echo "==> layer 3/3: the same contract, with the official Python SDKs"
  if ! "$python_bin" "$python_dir/conformance.py"; then
    status=1
    dump_log_on_failure
  fi
fi

rm -f "$log"
if [[ "$status" -eq 0 ]]; then
  echo
  echo "conformance: every probed row of the fidelity matrix holds."
fi
exit "$status"
