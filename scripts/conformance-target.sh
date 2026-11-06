#!/usr/bin/env bash
#
# Point the conformance probes at *something that is not fireup* and record what
# it answers.
#
# `scripts/conformance-local.sh` builds fireup, starts it, and demands the exact
# status `tests/conformance/matrix.json` claims for every row — including the
# wording of fireup's own refusals. That is the regression shield, and it is
# deliberately useless against another implementation.
#
# This script runs the same probes, through the same official Google client
# libraries, against a host that is already running, and writes down what came
# back: serves / refuses / diverges / error / timeout. It never asserts, so it
# never fails the build; the report is the output.
#
# Usage:
#   scripts/conformance-target.sh --target 'floci-gcp 0.12.0' \
#     --firestore-host 127.0.0.1:4588 [--datastore-host 127.0.0.1:4588] \
#     [--out docs/local/measurements/floci-gcp-0.12.0.json] [--timeout 20000] \
#     [--only <matrix-id-substring>]
#
# `--datastore-host` defaults to `--firestore-host`, which is right for a
# single-port emulator and wrong for one that does not serve Datastore at all —
# in that case point it somewhere that refuses and let the report say so.
#
# What this is: black-box conformance testing of a published API contract using
# the vendor's own open-source client libraries. The same method SAK-163
# established for verifying fireup itself. Nothing here decompiles,
# disassembles, or binary-analyses any artefact, and nothing is copied out of
# the target — see `docs/licenses/FIREBASE_GCP_LOCAL.md`.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
node_dir="$repo_root/tests/conformance/node"

target=""
firestore_host=""
datastore_host=""
out=""
extra=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target) target="$2"; shift 2 ;;
    --firestore-host) firestore_host="$2"; shift 2 ;;
    --datastore-host) datastore_host="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --timeout|--only) extra+=("$1" "$2"); shift 2 ;;
    -h|--help) sed -n '2,32p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$firestore_host" ]]; then
  echo "--firestore-host is required (host:port of the already-running target)" >&2
  exit 2
fi
: "${datastore_host:=$firestore_host}"
: "${target:=$firestore_host}"

if [[ ! -d "$node_dir/node_modules" ]]; then
  echo "==> installing the official Node client libraries"
  (cd "$node_dir" && npm install --no-audit --no-fund)
fi

# Refuse to measure a port nothing is listening on: an unreachable host scores
# every row `unreachable`, which looks like a result and is not one.
if ! node -e "
  const net = require('net')
  const [host, port] = process.argv[1].split(':')
  const s = net.connect(Number(port), host)
  s.on('connect', () => { s.destroy(); process.exit(0) })
  s.on('error', () => process.exit(1))
" "$firestore_host" 2>/dev/null; then
  echo "nothing is accepting connections on $firestore_host — start the target first" >&2
  exit 1
fi

export FIRESTORE_EMULATOR_HOST="$firestore_host"
export DATASTORE_EMULATOR_HOST="$datastore_host"

args=(--target "$target")
[[ -n "$out" ]] && args+=(--out "$out")

node "$node_dir/observe.mjs" "${args[@]}" "${extra[@]+"${extra[@]}"}"
