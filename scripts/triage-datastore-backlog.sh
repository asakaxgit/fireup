#!/usr/bin/env bash
#
# The triage role's input for the Datastore surface: run the fidelity backlog
# (tests/local-smoke-datastore/backlog.mjs) against each store, then print the
# failures clustered by signature. See docs/local/TRIAGE.md for what the triager
# does with them.
#
# Usage: scripts/triage-datastore-backlog.sh [results-dir]
#
# Never a gate: a red backlog is the expected state, so this exits non-zero only
# when a run crashes before reporting. Set FIREUP_SMOKE_POSTGRES_URL to include
# the PostgreSQL store — point it only at a database you own; see
# PROJECT_STATUS.md. Without it, only the in-memory store runs, and a failure
# that only Postgres has will not show.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
results_dir="${1:-$(mktemp -d -t fireup-backlog-results.XXXXXX)}"
mkdir -p "$results_dir"
rm -f "$results_dir"/*.json

stores=(memory)
[[ -n "${FIREUP_SMOKE_POSTGRES_URL:-}" ]] && stores+=(postgres)

crashed=0
for store in "${stores[@]}"; do
  echo
  echo "=== datastore-backlog/$store"
  url=""
  [[ "$store" == postgres ]] && url="$FIREUP_SMOKE_POSTGRES_URL"
  FIREUP_SMOKE_POSTGRES_URL="$url" \
    FIREUP_SMOKE_ENTRY=backlog.mjs \
    FIREUP_SMOKE_SUITE="datastore-backlog/$store" \
    FIREUP_SMOKE_RESULTS="$results_dir/datastore-backlog-$store.json" \
    "$repo_root/scripts/smoke-local-datastore.sh" || crashed=1
done

echo
node "$repo_root/tests/smoke-gate/triage.mjs" "$results_dir"
echo
echo "results: $results_dir"
exit "$crashed"
