#!/usr/bin/env bash
#
# The fidelity regression gate: run every official-SDK smoke suite against every
# store, then fail if a check that passes on `main` no longer passes.
#
# A suite's own exit code is not the verdict here. A suite can be red on `main`
# for a check nobody has fixed yet, and a job that tolerates that also tolerates
# the next check to break — it is one more FAIL line in a long log. So each
# suite writes its checks to a results file (tests/smoke-gate/results.mjs), and
# tests/smoke-gate/gate.mjs compares the lot with tests/smoke-gate/baseline.json.
# Only a baseline check that fails or goes missing fails this script; a new
# check, or one that newly passes, never does.
#
# Usage: scripts/fidelity-smoke-gate.sh [results-dir]
#
# FIREUP_SMOKE_POSTGRES_URL is required, not optional: the baseline covers the
# PostgreSQL store too, and skipping those runs would report every one of their
# checks as missing. Point it only at a database you own; see PROJECT_STATUS.md.
#
# To refresh the baseline after a deliberate change, see docs/local/README.md.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
results_dir="${1:-$(mktemp -d -t fireup-smoke-results.XXXXXX)}"
mkdir -p "$results_dir"
# Stale files from an earlier run would stand in for a suite that crashed
# before writing anything this time.
rm -f "$results_dir"/*.json

if [[ -z "${FIREUP_SMOKE_POSTGRES_URL:-}" ]]; then
  echo "FIREUP_SMOKE_POSTGRES_URL is not set; the gate needs the PostgreSQL runs too" >&2
  exit 2
fi
postgres_url="$FIREUP_SMOKE_POSTGRES_URL"

# One line per suite run: <suite> <store> <script>. The suite id
# "<suite>/<store>" is the key in baseline.json.
runs=(
  "firestore memory smoke-local-firestore.sh"
  "firestore postgres smoke-local-firestore.sh"
  "auth memory smoke-local-auth.sh"
  "auth postgres smoke-local-auth.sh"
  "datastore memory smoke-local-datastore.sh"
  "datastore postgres smoke-local-datastore.sh"
)

for run in "${runs[@]}"; do
  read -r suite store script <<<"$run"
  echo
  echo "=== $suite/$store"
  url=""
  [[ "$store" == postgres ]] && url="$postgres_url"
  # `|| status=$?`, not a bare call: under `set -e` a red suite would end the
  # script here and the gate below would never say which check broke.
  status=0
  FIREUP_SMOKE_POSTGRES_URL="$url" \
    FIREUP_SMOKE_SUITE="$suite/$store" \
    FIREUP_SMOKE_RESULTS="$results_dir/$suite-$store.json" \
    "$repo_root/scripts/$script" || status=$?
  echo "=== $suite/$store exited $status"
done

echo
node "$repo_root/tests/smoke-gate/gate.mjs" "$results_dir"
