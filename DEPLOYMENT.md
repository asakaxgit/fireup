# Running Fireup against a local PostgreSQL

This file covers one thing: bringing up the PostgreSQL container this repository
ships, and pointing the CLI at it on a development machine. That is what the
repository supports today.

**It is not a production deployment guide.** Fireup's own status line reads
*NOT production ready* (`PROJECT_STATUS.md`), nothing is deployed anywhere, and
the version of this file that stood until SAK-257 described a production
topology — a `docker-compose.prod.yml`, a long-running `fireup` service with
`restart: always`, resource reservations, a backup rotation cron — for a
one-shot CLI that nobody runs that way. It is in history, not gone:
`git show 3305529:DEPLOYMENT.md`.

The clone/build/first-command walkthrough is in [README.md](README.md) and is
not repeated here. `fireup local` — the Firestore/Datastore/Auth surface — is
documented in [docs/local/README.md](docs/local/README.md); this file stops at
the database underneath it.

## Prerequisites

- **A Rust toolchain.** CI builds with `rustup default stable`; the Dockerfile's
  builder stage is `rust:1.75-slim`. `Cargo.toml` declares no `rust-version` and
  no job tests an older compiler, so the "Rust 1.70+" quoted in `README.md` and
  `PROJECT_STATUS.md` is a claim nothing measures. Build with stable.
- **Docker and Compose v2.** The commands below use the `docker compose`
  plugin spelling; older `docker-compose` invocations elsewhere in these docs
  are the v1 binary.
- **A PostgreSQL client** (`psql`) if you want to look at the result.

## 1. Start PostgreSQL

```bash
cp .env.example .env
docker compose up -d postgres
docker compose ps
docker compose logs postgres
```

Defaults, from `docker-compose.yml`:

| | value |
|---|---|
| host port | **5433** (container 5432) |
| database | `fireup_dev` |
| user / password | `fireup` / `fireup_dev_password` |
| data volume | `postgres_data` (survives `docker compose restart`) |

### Do not rename the database or the user

`init.sql` is mounted into `/docker-entrypoint-initdb.d/` and runs once, the
first time the volume is initialized. It **hardcodes** both names, while
`docker-compose.yml` parameterizes them (`${POSTGRES_DB:-fireup_dev}`,
`${POSTGRES_USER:-fireup}`) and `.env.example` sets them. Overriding either in
`.env` breaks initialization.

Measured by running `init.sql` with `psql -v ON_ERROR_STOP=1` — the same flag
the image's entrypoint uses — against PostgreSQL 18:

| `.env` change | what `init.sql` hits | psql exit |
|---|---|---|
| none (the defaults above) | runs to the end; creates the schema and no tables | **0** |
| `POSTGRES_DB=anything_else` | `psql:init.sql:15`, `ALTER DATABASE fireup_dev …` → `ERROR: database "fireup_dev" does not exist` | **3** |
| `POSTGRES_USER=anyone_else` | `psql:init.sql:18`, `GRANT … TO fireup` → `ERROR: role "fireup" does not exist` | **3** |

Those line numbers are psql's own prefix, not an annotation. The two failing
arms were run against a cluster whose only role is `anyone_else`, so `fireup`
is genuinely absent rather than merely unmentioned.

The image's entrypoint runs under `set -Eeo pipefail` and executes each init
file through `psql -v ON_ERROR_STOP=1`, so either error aborts initialization
rather than leaving a half-configured database. (The psql arms above are
measured; the container arm is not — Docker was not reachable from where they
were run.)

**The quiet case is worse than the loud one.** If a `fireup_dev` database
already exists in the cluster and you point `POSTGRES_DB` at a *different*
database, `init.sql` exits **0** — and `ALTER DATABASE fireup_dev …` sets the
search path on the wrong database. Measured: the schema is created in the new
database, the search path lands on `fireup_dev`, and an unqualified
`CREATE TABLE` in the new database then goes to **`public`**, not `fireup_data`.
Nothing reports this; the import simply lands somewhere other than where every
query in these docs looks.

### Why a `sql/` directory mount does not work

There is no `sql/` directory any more, and the mount that used to carry it is
gone too. The reason is worth keeping, because the mount looked like it worked:

```yaml
# removed in SAK-257 — it never ran a single statement
- ./sql:/docker-entrypoint-initdb.d/sql:ro
```

The entrypoint calls `docker_process_init_files /docker-entrypoint-initdb.d/*`
and switch-cases each entry on `*.sh`, `*.sql`, `*.sql.gz`, `*.sql.xz`,
`*.sql.zst`, with `*)` printing *"ignoring"*. A **directory** named `sql`
matches none of those, and the loop does not recurse into it. (Same code in the
15 and 17 entrypoints of `docker-library/postgres`; the image pinned here is
`postgres:15`.) So anything you add that way is silently skipped — put init
files directly in `/docker-entrypoint-initdb.d/`, as `init.sql` is.

What the directory held was `01_development_setup.sql`: a `sample_users`
fixture table nothing read, and a `fireup_data.import_summary` view over
`fireup_data.import_audit`. Both are deleted, along with the audit table
itself. The measurements behind that are in
[What this file no longer claims](#what-this-file-no-longer-claims).

## 2. Import a backup

```bash
export PGPASSWORD=fireup_dev_password
cargo run --release -- import \
  --backup-file /path/to/export \
  --postgres-url postgresql://fireup@localhost:5433/fireup_dev
```

A URL with no password falls back to `PGPASSWORD`, and a URL with no user falls
back to `PGUSER` (`src/data_importer/connection.rs`), which is how you keep the
password out of your shell history. Measured against a server that requires
`scram-sha-256` for that role, with the password absent from the URL in both
arms:

| arm | result |
|---|---|
| `PGPASSWORD` unset | three connection attempts, then `Import failed: Database connection error`, exit **1** |
| `PGPASSWORD` set | `2 tables processed, 7/7 records imported, 0 failed`, exit **0** |

### Where the rows land

`init.sql` runs `ALTER DATABASE fireup_dev SET search_path TO fireup_data,
public`, so the importer's unqualified DDL creates its tables in
**`fireup_data`** — not `public`. Measured after the successful import above:

```
fireup_data.cities  5 rows
fireup_data.users   2 rows
```

Those are the only two tables in the database. `init.sql` creates none, and
nothing lands in `public`.

That is why `\dt fireup_data.*` is the right way to look for imported tables.

## 3. The knobs that exist

**Flags on `fireup import`** (defaults from `src/main.rs`):

| flag | default | what it does |
|---|---|---|
| `--batch-size` | 1000 | rows per bulk insert |
| `--max-connections` | 5 | pool size |
| `--timeout` | 300 | seconds per database operation |
| `--skip-normalization` | off | import the documents as-is |
| `--drop-existing` | off | drop the target tables first |
| `--continue-on-error` | off | keep importing past constraint violations; the exit code still reports them |
| `--generate-ddl FILE` | — | write the DDL out for review before importing |

**Environment variables the binary actually reads:** `PGPASSWORD` and `PGUSER`,
as above.

**Environment variables nothing reads.** `FIREUP_MAX_BATCH_SIZE`,
`FIREUP_CONNECTION_POOL_SIZE`, `FIREUP_IMPORT_TIMEOUT_SECONDS` and
`DATABASE_URL` appear in no source file (`grep -rn … src/`), and `RUST_LOG` is
set in `.env.example` and passed into the compose service but never consulted:
the subscriber is built with `with_max_level()` from the CLI flags, not with an
`EnvFilter`. Measured on `fireup validate` against the same fixture:

| invocation | INFO lines printed |
|---|---|
| `RUST_LOG` unset (control) | 23 |
| `RUST_LOG=error` | **23** |
| `--log-level error` | **0** |

So use `--log-level` / `--verbose` / `--json-logs` for logging, and the flags
in the table above for tuning. The previous version of this file documented the
three `FIREUP_*` variables as production tuning; they have never done anything.

## 4. Looking at the data

```bash
# Connect
psql -h localhost -p 5433 -U fireup -d fireup_dev

# Imported tables (the search path puts them in fireup_data)
\dt fireup_data.*
\d fireup_data.cities

# Dump what was imported
pg_dump -h localhost -p 5433 -U fireup -d fireup_dev \
  --schema=fireup_data --data-only > fireup_data.sql

# Schema only
pg_dump -h localhost -p 5433 -U fireup -d fireup_dev \
  --schema=fireup_data --schema-only > fireup_schema.sql
```

Any standard client works — the connection string is
`postgresql://fireup:fireup_dev_password@localhost:5433/fireup_dev`, and GUI
tools need the same four values (host `localhost`, port 5433, database
`fireup_dev`, user `fireup`).

## 5. `fireup local`

`docker-compose.yml` also defines a `fireup-local` service that serves the
Firestore, Datastore and Identity Toolkit surfaces on one port (4589 by
default), backed by the same PostgreSQL service, so the data survives a
restart:

```bash
docker compose up -d fireup-local
export FIRESTORE_EMULATOR_HOST=127.0.0.1:4589
export FIREBASE_AUTH_EMULATOR_HOST=127.0.0.1:4589
curl http://127.0.0.1:4589/health
```

**Development machines only.** The auth surface mints unsigned tokens and
verifies no credential; do not publish that port. Flags, the fidelity matrix
and the storage layout are in
[docs/local/README.md](docs/local/README.md); the end-to-end workflow is in
[docs/local/END_TO_END.md](docs/local/END_TO_END.md).

## Troubleshooting

**The container will not start.**

```bash
docker compose logs postgres        # the entrypoint says which init file failed
ss -ltn | grep 5433                 # host port already taken
docker info                         # daemon reachable at all
```

If the log ends in a `psql:/docker-entrypoint-initdb.d/init.sql:NN: ERROR`,
read *Do not rename the database or the user* above — the two renames that
cause it are the common ones. Initialization only runs on a fresh volume, so
fixing `.env` is not enough on its own: `docker compose down -v` and start
again, which also deletes anything already imported.

**Connection refused.**

```bash
docker compose ps                   # healthy, or restarting?
psql -h localhost -p 5433 -U fireup -d fireup_dev -c 'select 1'
```

Port **5433** on the host, not 5432 — the CLI's own `--help` examples use 5432,
which is the container-internal port.

**Authentication failed, or `Database connection error` with no detail.** The
importer reports a pool failure rather than the server's reason. Reproduce the
same connection with `psql` to get the real message, and check `PGPASSWORD` is
exported when the URL carries no password (the table in §2).

**Tables are missing after an import that reported success.** Look in
`fireup_data`, not `public` (§2). If they are in `public`, the search path never
got set — the quiet `POSTGRES_DB` case above.

**The import is slow, or runs out of memory.** `--batch-size` and
`--max-connections` are the knobs, not the `FIREUP_*` environment variables
(§3), and each applies to one invocation.

**Queries are slow.**

```sql
ANALYZE;                                    -- refresh planner statistics
SELECT * FROM pg_stat_user_tables WHERE schemaname = 'fireup_data';
EXPLAIN ANALYZE SELECT * FROM fireup_data.cities;
```

## What this file no longer claims

Removed in SAK-257, with the reason, so nobody restores it from memory:

- **A production deployment section** — `docker-compose.prod.yml`, a `fireup`
  application container with `restart: always`, CPU/memory reservations,
  production environment variables, a security-hardening list and a backup
  rotation script. None of it was ever run, the product is not production
  ready, and the `fireup` CLI exits when its import finishes rather than
  serving anything.
- **`DATABASE_URL`** as the way to configure the connection. The flag is
  `--postgres-url`; that variable is read nowhere.
- **The `FIREUP_*` tuning variables** — measured above: inert.
- **`SELECT * FROM fireup_data.import_summary`**, and the Python and Node
  snippets that queried it. The view and the `fireup_data.import_audit` table
  it read are both deleted as of SAK-257, along with `sql/` and its mount.
  Measured before deleting them, on one cluster, changing only `init.sql`:
  with the old file `fireup_data` held `import_audit` with 1 row — the one
  `init.sql` inserted about itself — and the `uuid-ossp` extension it needed;
  with the new file `fireup_data` holds no tables and no extension, and
  `fireup import` still reports `2 tables processed, 7/7 records imported,
  0 failed` at exit 0, landing the same 5 + 2 rows. Nothing else ever wrote
  that table. Import auditing does exist, in `fireup::monitoring`
  (`log_audit_entry`, called from the importer and the parser), but it keeps
  entries in memory, caps them at 10,000, and writes them to no database — so
  the SQL table was a second, unconnected design. Note the two disagreed on
  their own vocabulary: the table had a `status` column, while
  `monitoring::AuditResult` has `result`. **If a durable audit trail is wanted,
  the work is persisting `fireup::monitoring`, not restoring this table.**
- **Performance-tuning advice** for `shared_buffers`, `work_mem` and
  `checkpoint_segments` — the last of which PostgreSQL removed in 9.5.
