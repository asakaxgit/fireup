#!/usr/bin/env bash
#
# The end-to-end demo: a managed Firestore/Datastore export becomes both
# normalized SQL tables and a live Firestore/Datastore API, on one PostgreSQL
# database, in one script.
#
#   export  →  fireup import       →  normalized SQL tables
#           →  fireup local seed   →  the local data plane's document table
#           →  fireup local        →  official SDKs read and write it
#           →  psql                →  SQL sees what the SDK just wrote
#
# The prose version, including what each step proves and where the fidelity
# gaps are, is docs/local/END_TO_END.md.
#
# Usage:
#   scripts/demo-export-to-sql.sh                      # Docker Compose Postgres
#   FIREUP_DEMO_POSTGRES_URL=postgresql://… \
#     scripts/demo-export-to-sql.sh                    # a database you own
#
# Environment:
#   FIREUP_DEMO_POSTGRES_URL  Where to put both planes. Defaults to the
#                             docker-compose Postgres on 127.0.0.1:5433.
#                             The demo CREATES AND DROPS TABLES here, so point
#                             it only at a development database you own.
#   FIREUP_DEMO_PORT          Port for `fireup local`. Default 4591, so the demo
#                             does not fight a server you already have on 4589.
#   FIREUP_DEMO_EXPORT        Export tree to use. Defaults to the repo's
#                             managed-export fixture.
#   FIREUP_DEMO_PROJECT_ID    Project the data plane answers for. Default
#                             fireup-demo.
#   FIREUP_DEMO_KEEP_RUNNING  Leave `fireup local` up at the end so you can
#                             poke at it yourself.
#
# Needs: cargo, node, npm, psql. Network access the first time, to npm-install
# the official client libraries into demo/.

set -euo pipefail

# rustup's default install location is not always on a non-login shell's PATH.
export PATH="$HOME/.cargo/bin:$PATH"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
demo_dir="$repo_root/demo"
# Honour CARGO_TARGET_DIR, or the binary we start is not the one we built.
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
fireup="$target_dir/debug/fireup"

postgres_url="${FIREUP_DEMO_POSTGRES_URL:-postgresql://fireup:fireup_dev_password@127.0.0.1:5433/fireup_dev}"
port="${FIREUP_DEMO_PORT:-4591}"
export_tree="${FIREUP_DEMO_EXPORT:-$repo_root/tests/fixtures/export-onestore}"
project_id="${FIREUP_DEMO_PROJECT_ID:-fireup-demo}"
# Keeps the long-running server quiet. The pipeline steps raise this to info
# themselves and filter it (see run_fireup). Set RUST_LOG to watch everything.
export RUST_LOG="${RUST_LOG:-warn}"

log="$(mktemp -t fireup-demo.XXXXXX.log)"
server_pid=""

cleanup() {
  if [[ -n "$server_pid" ]] && kill -0 "$server_pid" 2>/dev/null; then
    if [[ -n "${FIREUP_DEMO_KEEP_RUNNING:-}" ]]; then
      echo
      echo "fireup local is still running on 127.0.0.1:$port (pid $server_pid)."
      echo "  export FIRESTORE_EMULATOR_HOST=127.0.0.1:$port"
      echo "  export DATASTORE_EMULATOR_HOST=127.0.0.1:$port"
      echo "Stop it with: kill $server_pid"
      return
    fi
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -f "$log"
}
trap cleanup EXIT

step() {
  echo
  echo "=============================================================="
  echo "$1"
  echo "=============================================================="
}

run() {
  echo "\$ $*"
  "$@"
}

# The CLI logs each phase of the pipeline at info level, which is far too much
# for a demo. Run it with logging on, then print only the lines that report an
# outcome — and the whole log if the command fails or says nothing we matched,
# so a surprise is never swallowed.
outcome_lines='Parsing complete|Import pipeline completed|Seed complete|Store now holds|Backup file is valid'

# Echo a command without printing the database password.
echo_command() {
  local shown=()
  for arg in "$@"; do
    case "$arg" in
      postgres://*|postgresql://*) shown+=("…") ;;
      *) shown+=("$arg") ;;
    esac
  done
  echo "\$ ${shown[*]}"
}

run_fireup() {
  echo_command fireup "$@"
  local status=0
  RUST_LOG=fireup=info "$fireup" "$@" >"$log" 2>&1 || status=$?
  if (( status != 0 )); then
    indent <"$log" >&2
    return "$status"
  fi
  # Print only the lines that report an outcome — and the whole log if nothing
  # matched, so a surprise is never swallowed.
  if grep -Eq "$outcome_lines" "$log"; then
    grep -E "$outcome_lines" "$log" | sed -E 's/^.*(INFO|WARN|ERROR)[^ ]* //' | indent
  else
    indent <"$log"
  fi
}

# Strip the tracing crate's ANSI colouring (it colours even when redirected) and
# indent, so a command's output is visibly its own.
indent() {
  sed -E $'s/\033\\[[0-9;]*m//g; s/^/    /'
}

if [[ ! -d "$export_tree" && ! -f "$export_tree" ]]; then
  echo "export tree not found: $export_tree" >&2
  exit 1
fi

step "0. Preflight"
# Quiet: the crate's existing warnings would otherwise bury the demo. The build
# log is printed if it fails.
echo "\$ cargo build --bin fireup"
if ! cargo build --bin fireup --manifest-path "$repo_root/Cargo.toml" >"$log" 2>&1; then
  cat "$log" >&2
  exit 1
fi
echo "built: $fireup"
if ! psql "$postgres_url" -Atc 'select 1' >/dev/null 2>&1; then
  cat >&2 <<EOF

Cannot reach PostgreSQL at the demo URL.

  $postgres_url

Start the repo's development database with

  docker compose up -d postgres

or point FIREUP_DEMO_POSTGRES_URL at a development database you own. The demo
creates and drops tables, so do not aim it at anything you care about.
EOF
  exit 1
fi
echo "postgres: reachable"
if [[ ! -d "$demo_dir/node_modules" ]]; then
  echo "==> installing the official Firestore and Datastore client libraries"
  (cd "$demo_dir" && npm install --no-audit --no-fund)
fi

step "1. The export reads"
# Before anything is written anywhere: does fireup understand this export tree?
# Every later step reuses this one parser, so a failure here is the only
# failure worth debugging first.
run_fireup validate --backup-file "$export_tree"

step "2. Migrator: export -> normalized SQL tables"
# The original Fireup job. --drop-existing makes the demo re-runnable; the DDL
# is written out so you can read the schema fireup chose before trusting it.
run_fireup import \
  --backup-file "$export_tree" \
  --postgres-url "$postgres_url" \
  --drop-existing \
  --generate-ddl "$demo_dir/generated-schema.sql"
echo "DDL written to demo/generated-schema.sql"

step "3. Local data plane: the same export -> documents in the same database"
# Same parser, different destination: the document table `fireup local` serves
# from. Writes are upserts, so running the demo twice is the same as once.
run_fireup local seed \
  --backup-file "$export_tree" \
  --postgres-url "$postgres_url" \
  --project-id "$project_id"

step "4. Serve it"
# Started in the background with its output in $log, so the SDK and SQL output
# below is not interleaved with the server's. The log is printed if it dies.
echo "\$ fireup local --port $port --postgres-url … --project-id $project_id &"
"$fireup" local \
  --port "$port" \
  --postgres-url "$postgres_url" \
  --project-id "$project_id" >"$log" 2>&1 &
server_pid=$!
# Wait for readiness rather than sleeping a guess. /health asks the store, so a
# 200 here means the database is reachable from the server too.
ready=""
for _ in $(seq 1 100); do
  if curl -fsS "http://127.0.0.1:$port/health" >/dev/null 2>&1; then
    ready=1
    break
  fi
  if ! kill -0 "$server_pid" 2>/dev/null; then
    echo "fireup local exited before becoming healthy:" >&2
    cat "$log" >&2
    exit 1
  fi
  sleep 0.1
done
if [[ -z "$ready" ]]; then
  echo "fireup local did not become healthy on port $port" >&2
  exit 1
fi
echo
run curl -fsS "http://127.0.0.1:$port/health"

step "5. Official SDKs read it — and write to it"
(
  cd "$demo_dir"
  FIRESTORE_EMULATOR_HOST="127.0.0.1:$port" \
  DATASTORE_EMULATOR_HOST="127.0.0.1:$port" \
  FIREUP_DEMO_PROJECT_ID="$project_id" \
    node read-with-sdk.mjs
)

step "6. SQL on the same PostgreSQL database"
echo_command psql "$postgres_url" -v ON_ERROR_STOP=1 -f "$demo_dir/query.sql"
psql "$postgres_url" -v ON_ERROR_STOP=1 -f "$demo_dir/query.sql"

step "Done"
cat <<EOF
One export, one PostgreSQL database, two access paths:

  - normalized SQL tables, for reporting and for the migration itself
  - google.firestore.v1 / google.datastore.v1, for application code that is
    not ready to stop being a Firestore client

Where this is still not production Firestore: docs/local/README.md#fidelity-matrix
The narrated version of this run:                docs/local/END_TO_END.md
EOF
