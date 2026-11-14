# Fireup — a PostgreSQL-backed migrator and local data plane for Firestore/Datastore

[![CI](https://github.com/asakaxgit/fireup/actions/workflows/ci.yml/badge.svg)](https://github.com/asakaxgit/fireup/actions/workflows/ci.yml)
[![Coverage](https://github.com/asakaxgit/fireup/actions/workflows/coverage.yml/badge.svg)](https://github.com/asakaxgit/fireup/actions/workflows/coverage.yml)
[![line coverage](https://img.shields.io/badge/line%20coverage-%E2%89%A575%25-success)](#ci-and-coverage)
[![fidelity matrix](https://img.shields.io/badge/fidelity%20matrix-95%2F121%20served-informational)](docs/local/README.md#fidelity-matrix)
[![licence](https://img.shields.io/badge/licence-Apache--2.0-blue)](LICENSE)

Fireup takes a managed Firestore/Datastore export and gives you **both** halves
of a migration at once, on one PostgreSQL database:

- **The migrator** (`fireup import`) — parse the export, analyse and normalize
  the schema, and import it into real PostgreSQL tables you can query with
  Standard SQL.
- **The local data plane** (`fireup local`) — serve the published
  `google.firestore.v1` and `google.datastore.v1` gRPC APIs plus a subset of the
  published Identity Toolkit REST API on a single local port, over that same
  PostgreSQL store, so official Firebase/Google Cloud SDKs keep working against
  it unchanged.

That combination is the point. Application code that is not ready to stop being
a Firestore client talks to `fireup local`; the reporting, the constraints, and
the migration itself happen in SQL — same data, same connection string, no
second copy to keep in sync.

```bash
docker compose up -d postgres
scripts/demo-export-to-sql.sh     # export → SQL tables → live API → SQL again
```

That script is the whole workflow end to end. The narrated version, including
where the fidelity gaps are, is **[docs/local/END_TO_END.md](docs/local/END_TO_END.md)**.

## What Fireup is, and is not

Fireup is a **Postgres-backed local data plane plus an export migrator** — not a
general-purpose GCP emulator suite. It deliberately goes deep on export
fidelity, durable PostgreSQL storage, and SQL access, and deliberately does not
go wide: no GCS, Pub/Sub, Cloud Run, KMS, or the rest of the cloud surface.

If you need that breadth, a full local-cloud suite such as **floci-gcp** is the
right tool for it. The two **complement** each other rather than compete: a
local-cloud suite is not a migration tool, and Fireup is not an emulator suite.
Fireup's default port is `4589` specifically so it can run beside floci-gcp
(`4588`) and Google's Firebase emulators (`8080`/`4000`) without an argument
about ports.

**[docs/local/COMPARISON.md](docs/local/COMPARISON.md)** is the feature matrix —
Fireup, the Firebase Emulator Suite, and the production service — including the
places where Fireup serves less and the places where it is more faithful. Most
of the emulator column is **measured**: the conformance suite was pointed at the
emulator Google ships and the raw per-row answers are committed in
[`docs/local/measurements/`](docs/local/measurements/README.md). Every cell says
which kind of evidence it is and which emulator version it is true of — four
Fireup bugs and two corrections to our own document came out of measuring it.

> **Independent implementation.** `fireup local` implements **public** Google API
> contracts from public documentation and the public
> [googleapis](https://github.com/googleapis/googleapis) protobuf definitions.
> It is **not** the Firebase Emulator Suite, is **not** affiliated with or
> endorsed by Google, and contains no code or behaviour derived from proprietary
> Google binaries. Its auth tokens are unsigned and it ships no Google key
> material; it is for local development only. See
> [`docs/licenses/FIREBASE_GCP_LOCAL.md`](docs/licenses/FIREBASE_GCP_LOCAL.md)
> for the licence and attribution ledger.

## Features

### Migrator

- Parse managed Firestore/Datastore export trees (LevelDB log framing, legacy
  `EntityProto` and `google.datastore.v1.Entity` payloads)
- Automatic schema analysis and normalization (1NF/2NF/3NF)
- Direct PostgreSQL import with generated, reviewable DDL
- Standard SQL query support, with real PostgreSQL types
- Comprehensive logging and error handling
- Docker-based development environment

### Local data plane

- `google.firestore.v1.Firestore`: document CRUD, structured queries,
  transactions, batch writes, and realtime `onSnapshot()` listeners — 13 of the
  17 RPCs, with the rest an explicit `UNIMPLEMENTED`
- `google.datastore.v1.Datastore`: `Lookup`, `RunQuery`, `Commit`,
  transactions, id allocation — over the *same* rows, since kinds map to
  collections
- Identity Toolkit (Firebase Auth) email/password, custom-token, and anonymous
  sign-in plus the Admin SDK account surface
- One port for all of it, gRPC and HTTP/1.1 on the same listener
- In-memory, or durable in PostgreSQL and legible to SQL
- Seed it straight from an export: `fireup local seed`
- Every unsupported feature is an explicit error, never a wrong answer — see the
  [fidelity matrix](docs/local/README.md#fidelity-matrix)

## Quick Start

### Prerequisites

- Rust 1.70+ 
- Docker and Docker Compose
- Git

### Development Setup

1. Clone the repository:
```bash
git clone <repository-url>
cd fireup
```

2. Copy environment configuration:
```bash
cp .env.example .env
```

3. Start PostgreSQL backend:
```bash
docker-compose up -d postgres
```

4. Build the project:
```bash
cargo build
```

5. Run the CLI:
```bash
cargo run -- --help
```

### Available Commands

- `fireup validate` - Validate backup file integrity
- `fireup analyze` - Analyze schema from backup file and generate DDL
- `fireup import` - Import Firestore backup data to PostgreSQL
- `fireup local` - Serve the Firestore/Datastore/Auth APIs on one local port
- `fireup local seed` - Load an export into the local data plane's store

### Example Usage

```bash
# Validate a backup file
cargo run -- validate --backup-file /path/to/backup.leveldb

# Analyze schema and generate DDL
cargo run -- analyze --backup-file /path/to/backup.leveldb --output schema.sql

# Import data to PostgreSQL
# (5433 is the host port docker-compose publishes; see docker-compose.yml)
cargo run -- import --backup-file /path/to/backup.leveldb --postgres-url postgresql://fireup:fireup_dev_password@localhost:5433/fireup_dev
```

### Serving the same data to Firestore SDKs

```bash
# Load the export into the local data plane's store, in the same database.
cargo run -- local seed --backup-file /path/to/export \
  --postgres-url postgresql://fireup:fireup_dev_password@localhost:5433/fireup_dev \
  --project-id my-project

# Serve it. One port: gRPC and HTTP/1.1.
cargo run -- local \
  --postgres-url postgresql://fireup:fireup_dev_password@localhost:5433/fireup_dev \
  --project-id my-project
```

Then point any official SDK at it with the documented environment variables —
no code change, no Fireup-specific shim:

```bash
export FIRESTORE_EMULATOR_HOST=127.0.0.1:4589
export DATASTORE_EMULATOR_HOST=127.0.0.1:4589
export FIREBASE_AUTH_EMULATOR_HOST=127.0.0.1:4589
curl http://127.0.0.1:4589/health
```

Flags, the auth surface, storage layout, and the full fidelity matrix are in
**[docs/local/README.md](docs/local/README.md)**.

## PostgreSQL Client Connection

Once data is imported, you can connect to PostgreSQL using any standard client:

### Using psql

```bash
# Connect to the development database
psql -h localhost -p 5433 -U fireup -d fireup_dev

# List imported tables — they land in fireup_data, not public, because
# init.sql sets the database search path
\dt fireup_data.*

# Query imported data: one table per collection in the export
SELECT * FROM fireup_data.cities LIMIT 10;
```

`fireup_data.import_summary` used to be suggested here. The container never
created it, nothing ever wrote the `fireup_data.import_audit` table it read,
and both were deleted in #120 — see
[DEPLOYMENT.md](DEPLOYMENT.md#what-this-file-no-longer-claims). Import
auditing does exist in `fireup::monitoring`, but it is held in memory and is
not queryable from SQL.

### Using pgAdmin

1. Create a new server connection:
   - Host: localhost
   - Port: 5433
   - Database: fireup_dev
   - Username: fireup
   - Password: fireup_dev_password

2. Navigate to `fireup_dev` → `Schemas` → `fireup_data` to see imported tables

### Connection String Format

```
postgresql://fireup:fireup_dev_password@localhost:5433/fireup_dev
```

## Development

### Project Structure

```
src/
├── main.rs              # CLI entry point
├── error.rs             # Error handling
├── types.rs             # Core data structures
├── monitoring.rs        # Logging and monitoring
├── leveldb_parser/      # LevelDB parsing functionality
│   ├── parser.rs        # Main parsing logic
│   └── validator.rs     # Backup validation
├── schema_analyzer/     # Schema analysis and normalization
│   ├── analyzer.rs      # Document structure analysis
│   ├── normalizer.rs    # Database normalization
│   ├── ddl_generator.rs # DDL generation
│   └── ...
├── data_importer/       # PostgreSQL import functionality
│   ├── importer.rs      # Main import logic
│   ├── transformer.rs   # Data transformation
│   └── ...
├── protos/              # Stubs generated from the public googleapis protos
└── local/               # The local data plane (`fireup local`)
    ├── mod.rs           # Server wiring and protocol multiplexing
    ├── firestore.rs     # google.firestore.v1.Firestore
    ├── datastore.rs     # google.datastore.v1.Datastore
    ├── auth/            # Identity Toolkit (Firebase Auth)
    ├── store.rs         # The Store trait, MemoryStore, conformance suite
    ├── postgres_store.rs# The durable Store implementation
    ├── seed.rs          # export → store
    └── ...              # See docs/local/README.md#layout

demo/                    # The end-to-end demo's SDK and SQL halves
docs/local/              # Local data plane documentation
proto/google/            # Vendored public googleapis .proto files (Apache-2.0)
```

### Environment Variables

Copy `.env.example` to `.env`. It configures the Docker Compose services, not
the binary:

```bash
# Read by docker-compose.yml
POSTGRES_PORT=5433                  # host port; the container always listens on 5432
POSTGRES_PASSWORD=fireup_dev_password
POSTGRES_DB=fireup_dev              # do not rename: init.sql hardcodes it
POSTGRES_USER=fireup                # do not rename: init.sql grants to this role
```

Renaming either of the last two aborts the container's first-run
initialization; both error messages are measured in
[DEPLOYMENT.md](DEPLOYMENT.md#do-not-rename-the-database-or-the-user). The CLI
itself is configured by flags — `--postgres-url`, `--batch-size`,
`--max-connections`, `--timeout`, `--log-level` — plus `PGPASSWORD`/`PGUSER`
when the URL omits them. `DATABASE_URL`, `RUST_LOG` and the `FIREUP_*`
variables are read by nothing; see
[DEPLOYMENT.md §3](DEPLOYMENT.md#3-the-knobs-that-exist).

### Running Tests

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test module
cargo test leveldb_parser

# The local data plane, over real gRPC and real HTTP/1.1
cargo test --test local_firestore_tests --test local_datastore_tests --test local_auth_tests

# The same surface driven by Google's official client SDKs, which is the layer
# that catches what tests written from our own reading of the spec cannot
scripts/smoke-local-firestore.sh
scripts/smoke-local-datastore.sh
scripts/smoke-local-auth.sh
```

Durable-store tests skip unless you point them at a dedicated database you own;
see [docs/local/README.md#tests](docs/local/README.md#tests).

### CI and coverage

Two workflows run on every commit, and the badges at the top of this file are
those two plus two numbers:

| Badge | What it actually means |
|---|---|
| **CI** | [`ci.yml`](.github/workflows/ci.yml) — two jobs. `test`: build plus every suite that needs nothing but a Rust toolchain — unit tests, the local data plane over real gRPC/HTTP, export parsing, the CLI and library surface, and the docs-vs-matrix checks. Red there means the code is broken, not that a service failed to start. `deny`: the supply-chain gate below. The badge is red if either fails. |
| **Coverage** | [`coverage.yml`](.github/workflows/coverage.yml) — runs `scripts/coverage.sh` and **fails under 75% line coverage**. The full HTML report is attached to each run as the `coverage-report` artifact, and the summary table is printed in the run's summary. |
| **line coverage** | The enforced floor, not a live reading. A number nobody checks drifts; a floor that fails the build does not. The real figure for a commit is in that run's summary. |
| **fidelity matrix** | How many of the 119 documented `fireup local` behaviours are served, from [`tests/conformance/matrix.json`](tests/conformance/matrix.json). `cargo test --test fidelity_matrix_tests` fails if the badge and the matrix disagree, so it cannot go stale. |

```bash
scripts/coverage.sh          # the same thing CI runs; report in target/coverage/
scripts/coverage.sh --open   # ... and open the HTML report
FIREUP_COVERAGE_MIN=80 scripts/coverage.sh
```

It needs `cargo install cargo-llvm-cov --locked` and `rustup component add
llvm-tools-preview` once. Scope is the `--lib` unit tests, which is why the
number is a floor rather than a boast: the suites that need PostgreSQL, Node, or
a running server are not counted, so the figure means the same thing on every
machine.

### Supply chain

[`deny.toml`](deny.toml) is the policy; the `deny` job in
[`ci.yml`](.github/workflows/ci.yml) enforces it on every commit. Run exactly
what CI runs:

```bash
cargo install cargo-deny --locked   # once; or grab the prebuilt release binary
cargo deny --locked check -D advisory-not-detected advisories licenses bans sources
```

Four checks, and what each one is for:

| Check | Policy |
|---|---|
| **advisories** | RustSec vulnerabilities and yanked crates. Nothing is accepted: `[advisories] ignore` in `deny.toml` is empty, so every advisory fails the build. The one exception the repo ever carried, `RUSTSEC-2026-0258` in `h2 0.3.27`, was removed by the tonic 0.13 / prost 0.13 migration that fixed it. |
| **licences** | Permissive only — the enforced form of the audit in [`docs/licenses/CARGO_LICENSE_AUDIT.md`](docs/licenses/CARGO_LICENSE_AUDIT.md). A dependency carrying a copyleft-only licence fails the build. |
| **bans** | Wildcard version requirements fail. Duplicate major versions only warn: the tree has eleven of them from transitive pins, none of Fireup's doing. |
| **sources** | crates.io only. A git or unknown-registry dependency fails. |

`cargo audit` is deliberately **not** used. It reads the same RustSec database
as `cargo deny check advisories`, so running both reports every vulnerability
twice; cargo-deny also covers licences, duplicates, and provenance in one
binary.

The checks were verified to actually bite, not just to pass — see the negative
controls recorded in [`docs/licenses/CARGO_LICENSE_AUDIT.md`](docs/licenses/CARGO_LICENSE_AUDIT.md#how-this-is-enforced).

The two older workflows — [`local_firestore.yml`](.github/workflows/local_firestore.yml)
(the official-SDK conformance suite) and [`integration_tests.yml`](.github/workflows/integration_tests.yml)
— are still `workflow_dispatch` only. Turning them on is a separate decision
from adding a badge: `integration_tests.yml` gates on `cargo fmt --check` and
`cargo clippy -D warnings`, both of which currently fail on `main`.

### Docker Commands

```bash
# Start PostgreSQL
docker-compose up -d postgres

# View logs
docker-compose logs -f postgres

# Stop services
docker-compose down

# Reset database (removes all data)
docker-compose down -v
docker-compose up -d postgres
```

### Logging

The application uses structured logging with the `tracing` crate:

- Set `RUST_LOG=debug` for verbose output
- Set `RUST_LOG=fireup=info` for application-specific logs
- Logs include structured fields for filtering and analysis

## Troubleshooting

### Common Issues

1. **PostgreSQL connection refused**
   - Ensure Docker container is running: `docker-compose ps`
   - Check port availability: `netstat -an | grep 5433`

2. **Permission denied errors**
   - Verify PostgreSQL user permissions in `init.sql`
   - Check Docker volume permissions

3. **Out of memory during import**
   - Reduce `FIREUP_MAX_BATCH_SIZE` in environment
   - Increase Docker memory limits in `docker-compose.yml`

4. **An SDK read returns a map where you expected a timestamp, blob, geo point,
   or document reference**
   - `fireup local seed` restores those four from the export parser's
     `__fireup_type__` tags, so a map means the tag was not one Fireup knows —
     check the stored row and re-seed with a current build. See
     [docs/local/END_TO_END.md](docs/local/END_TO_END.md#what-seeding-does-with-datastore-value-types).

5. **An SDK call fails with `UNIMPLEMENTED`**
   - Also expected: an unsupported feature is an explicit error rather than a
     wrong answer, and the message names the gap. Check the
     [fidelity matrix](docs/local/README.md#fidelity-matrix) before treating it
     as a bug.

## Documentation

| Document | What is in it |
|---|---|
| [docs/local/END_TO_END.md](docs/local/END_TO_END.md) | The end-to-end workflow: export → SQL tables → live API → SQL, and its fidelity gaps |
| [docs/local/README.md](docs/local/README.md) | `fireup local`: flags, fidelity matrix, auth surface, storage layout, tests |
| [PROJECT_STATUS.md](PROJECT_STATUS.md) | What is proven, what is not, and the residual risks |
| [DEPLOYMENT.md](DEPLOYMENT.md) | Running the migrator against the local PostgreSQL container: compose defaults, the `.env` renames that break it, where rows land, live and inert configuration |
| [docs/INTEGRATION_TESTING.md](docs/INTEGRATION_TESTING.md) | The integration test suite |
| [docs/licenses/FIREBASE_GCP_LOCAL.md](docs/licenses/FIREBASE_GCP_LOCAL.md) | Licence and attribution ledger for everything Google/Firebase-related |
| [docs/licenses/CARGO_LICENSE_AUDIT.md](docs/licenses/CARGO_LICENSE_AUDIT.md) | `cargo license` audit that cleared Apache-2.0 for the dependency tree |
| [tests/fixtures/README.md](tests/fixtures/README.md) | The managed-export fixtures and their binary format |

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

```
Copyright 2025-2026 Saks Industries

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

**Trademarks.** The Fireup name, mark, lockup and visual identity are **not**
licensed under Apache-2.0 — see [`TRADEMARKS.md`](TRADEMARKS.md), which also
says what you may do without asking (describing compatibility, attributing a
fork). Using the code does not grant permission to use the brand. The asset
masters and the marketing site built from them are held privately by Saks
Industries and are not in this repository.

Third-party material is recorded in
[docs/licenses/FIREBASE_GCP_LOCAL.md](docs/licenses/FIREBASE_GCP_LOCAL.md) — most
notably the `.proto` definitions under `proto/google/`, vendored unmodified from
[googleapis](https://github.com/googleapis/googleapis) (Apache-2.0) at a pinned
commit. The dependency-tree audit that cleared Apache-2.0 for this repo is in
[docs/licenses/CARGO_LICENSE_AUDIT.md](docs/licenses/CARGO_LICENSE_AUDIT.md).
Fireup is an independent implementation of public Google API contracts and is
not affiliated with or endorsed by Google.