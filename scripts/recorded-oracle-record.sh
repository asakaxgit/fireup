#!/usr/bin/env bash
#
# Record doc-cited fidelity verdicts against the Spark-plan Firebase/GCP sandbox.
#
# Never run from CI. Credentials come only from Paperclip secrets (paths/env).
# Refuses emulator host variables — the Firebase Emulator Suite is never an oracle.
#
# Required env:
#   GOOGLE_APPLICATION_CREDENTIALS  path to the service-account JSON (Paperclip secret)
#   FIREUP_ORACLE_WEB_API_KEY       Firebase web API key (Paperclip secret)
# Optional:
#   FIREUP_ORACLE_FIRESTORE_PROJECT  default fireup-oracle-firestore
#   FIREUP_ORACLE_DATASTORE_PROJECT  default fireup-oracle-datastore

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
oracle_dir="$repo_root/tests/recorded-oracle"

if [[ -n "${CI:-}" || -n "${GITHUB_ACTIONS:-}" ]]; then
  echo "recorded-oracle-record.sh refuses to run in CI" >&2
  exit 2
fi

if [[ ! -d "$oracle_dir/node_modules" ]]; then
  echo "==> installing recorded-oracle client libraries"
  (cd "$oracle_dir" && npm install --no-audit --no-fund)
fi

echo "==> recording production-sandbox verdicts into $oracle_dir/fixtures"
node "$oracle_dir/record.mjs"
