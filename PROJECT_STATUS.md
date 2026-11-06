# Fireup Project Status

**Last Updated:** November 4, 2026  
**Version:** 0.1.0  
**Status:** ⛔ NOT production ready — fixture path works; live GCP export + scale unproven

---

## Executive Summary

Fireup aims to migrate Firestore/Datastore backup data into PostgreSQL. The
internal components are built out and unit-tested. Validated on 2026-11-02
against spec-correct export fixtures (SAK-139), `fireup validate` reported
`No records found in LevelDB file` and zero of 12 entities were recovered.
**The read path is now fixed** (SAK-141): `validate` reads all 12 entities of
the fixture tree across both kinds. The remaining gaps are downstream of
parsing — see Residual Risks.

Treat the previous "Development Complete - Production Ready" assessment as
withdrawn. It was based on unit tests over synthetic in-memory inputs, none of
which exercised the real on-disk export format.

### What is built
- LevelDB log framing reader, schema analyzer with 1NF/2NF/3NF normalization,
  DDL generator, transformer, PostgreSQL importer, CLI, monitoring
- 209+ passing library unit tests
- Realistic export fixtures plus a format-level validation suite
  (`tests/fixtures/`, `tests/real_export_validation.rs`)
- An export-fidelity regression suite over deliberately awkward fixtures —
  output shards, namespace partitions, key identity, numeric limits, property
  names PostgreSQL cannot store whole (`tests/export_fidelity_tests.rs`,
  SAK-195)

### What works on a real export
- Reading a managed export end to end: masked CRC32C framing, legacy
  `EntityProto` payloads, and every kind and `output-N` shard of the tree
  (SAK-141)

### What does not work yet
- Confirmed round-trip against a **live GCP-captured** export (fixtures are
  format-correct but not project-captured)
- Large multi-shard performance characterization

---

## Implementation Status

### Core Components

#### 1. LevelDB Parser ✅ **READS REAL EXPORTS**
**Status:** Reads a managed Firestore/Datastore export end to end  
**Location:** `src/leveldb_parser/`

**Implemented:**
- 32 KiB block iteration and 7-byte record header parsing
- Masked CRC32C record checksums, per `leveldb/util/crc32c.h` (`crc32c.rs`)
- FIRST/MIDDLE/LAST fragment reassembly
- Legacy `storage_onestore_v3.EntityProto` decoding, including proto2 groups,
  `meaning`-based types, repeated-property arrays and nested `ENTITY_PROTO`
  maps (`entity_proto.rs`, a hand-written wire decoder — `prost` cannot
  represent groups)
- Datastore `v1.Entity` protobuf decoding, kept for Datastore Admin / BigQuery
  export paths and selected per record (`protobuf_entity.rs`)
- Whole-tree input resolution: every `all_namespaces/kind_*/output-N` shard of
  every kind, aggregated, with `.export_metadata` files excluded
- JSON Lines fallback path

**Measured (2026-11-02, SAK-141):** `validate --backup-file
tests/fixtures/export-onestore` reads 2 input files, 13 records, **12 of 12
entities**, 2 collections, 0 errors. The same tree previously reported
`No records found in LevelDB file`.

**Test Coverage:** `tests/real_export_validation.rs` (7 tests) checks the
fixtures with a reader written independently of `src/`, then reads them through
fireup; `tests/export_fidelity_tests.rs` (21 tests) does the same for the
edge-case trees. `cargo test --lib leveldb_parser` is 53 tests; the in-crate fixtures are
now built with masked CRC32C, so they test the format rather than mirroring the
implementation.

#### 2. Schema Analyzer ✅ **COMPLETE**
**Status:** Fully implemented with advanced features  
**Location:** `src/schema_analyzer/`

**Capabilities:**
- Automatic document structure analysis
- Field type detection and cataloging
- Nested structure and relationship identification
- Database normalization (1NF, 2NF, 3NF):
  - Eliminates repeating groups (arrays → separate tables)
  - Removes partial dependencies
  - Eliminates transitive dependencies
- Type conflict detection and resolution with warnings
- NOT NULL constraint recommendations based on data completeness
- Comprehensive transformation reporting

**Test Coverage:** Unit tests passing

#### 3. DDL Generator ✅ **COMPLETE**
**Status:** Fully functional with PostgreSQL compatibility  
**Location:** `src/schema_analyzer/ddl_generator.rs`

**Capabilities:**
- Generate CREATE TABLE statements from normalized schemas
- Primary key and foreign key definitions
- Index recommendations for common query patterns
- Constraint generation (NOT NULL, UNIQUE, CHECK)
- Output DDL files for administrator review
- Detailed transformation reports

**Test Coverage:** Unit tests passing

#### 4. Data Transformation ✅ **COMPLETE**
**Status:** Production ready  
**Location:** `src/data_importer/transformer.rs`, `type_mapper.rs`

**Capabilities:**
- Type mapping from Firestore to PostgreSQL:
  - String → VARCHAR/TEXT
  - Integer → BIGINT; double → DOUBLE PRECISION (IEEE 754 binary64, so `-0.0`
    and `NaN`/`±Infinity` survive); mixed integer/double → NUMERIC
  - Boolean → BOOLEAN
  - Timestamp → TIMESTAMP
  - Reference → UUID (foreign keys)
  - Array → Normalized tables
  - Map → JSONB or normalized tables
- Document to relational row conversion
- Foreign key relationship establishment
- Parameterized SQL generation for safe data insertion

**Test Coverage:** Unit tests passing

#### 5. Data Importer ✅ **WRITES ROWS**
**Status:** The CLI runs the full import and the row counts are verified against
a live PostgreSQL  
**Location:** `src/data_importer/importer.rs`, `src/data_importer/connection.rs`

**Fixed under SAK-142** (the defects SAK-139 found by source inspection):
- `src/main.rs` called only `create_schema` and then returned
  `total_records_imported: documents.len()` with `table_imports: vec![]`, so a
  successful-looking `import` wrote **zero rows**. The CLI now transforms the
  documents into `TableImportSpec`s that carry the rows and calls
  `execute_full_import`, which calls `import_transformed_data` per table.
- `parse_postgres_url` discarded `--postgres-url` and returned a hardcoded
  `postgres:password@localhost:5432/fireup_test`. Replaced by
  `connection::parse_connection_url`, which parses every component; no
  credential remains in source.

**Capabilities:**
- Connection pooling with `deadpool-postgres`
- Multi-row INSERTs with text parameters cast to the declared column type, so
  NULL, UUID, JSONB, array and timestamp values round-trip without
  string-formatting SQL
- Batch size honoured from `--batch-size`, clamped to PostgreSQL's 65535
  bind-parameter limit
- `--continue-on-error` retries a failed batch row by row, so the importable
  rows land and the rejected ones are named
- `--drop-existing` drops the schema's tables in one transaction
- Transaction-based schema creation with rollback
- Constraint validation after import
- Integration with PostgreSQL Docker container

**Measured (2026-11-02, SAK-142):** `tests/postgres_import_tests.rs` imports a
25-document backup through the library path and a 12-document backup through the
CLI, then counts the rows in PostgreSQL: one row per source document in both
cases. Re-importing with `--drop-existing` leaves the same count rather than
doubling it.

**Identity:** rows use Datastore `key_path` as the primary key (SAK-143), so
re-import is idempotent against the fixture tree.

**Test Coverage:** library unit tests, plus `tests/postgres_import_tests.rs`
(requires `FIREUP_TEST_POSTGRES_URL`) and `tests/cli_exit_code_tests.rs`

#### 6. CLI Interface ✅ **COMPLETE**
**Status:** Fully functional with comprehensive command set  
**Location:** `src/main.rs`

**Available Commands:**
```bash
fireup import    # Import Firestore backup to PostgreSQL
fireup analyze   # Analyze schema and generate DDL
fireup validate  # Validate backup file integrity
```

**Features:**
- Configurable logging levels (error, warn, info, debug, trace)
- JSON log output support
- Customizable batch sizes and connection pools
- Schema normalization options
- Detailed progress reporting
- Comprehensive help documentation

**Test Coverage:** Integration tests available

#### 7. Monitoring & Logging ✅ **COMPLETE**
**Status:** Production ready  
**Location:** `src/monitoring.rs`

**Capabilities:**
- Structured logging with `tracing` crate
- Performance metric tracking
- Audit logging for data operations
- Operation timing and statistics
- Error tracking with full context
- Configurable log levels and formats

**Test Coverage:** Unit tests passing

---

## Build & Test Status

### Build Status ✅
```
Compiler: rustc (Rust 1.70+)
Build Time: ~35 seconds
Status: SUCCESS
Warnings: 110 warnings (mostly unused code for future features)
```

**Notes on Warnings:**
- Most warnings are for unused struct fields and methods that are part of the designed API but not yet utilized
- No errors or critical warnings
- All warnings are related to dead code analysis of derived traits
- These represent planned features and comprehensive API surface

### Test Status
```
Library unit tests:       457 passed, 0 failed    (cargo test --lib)
Real-export validation:     7 passed, 0 ignored    (cargo test --test real_export_validation)
Export-fidelity suite:     21 passed, 0 ignored    (cargo test --test export_fidelity_tests)
Whole suite:              874 passed, 0 failed, 0 ignored
                                      (cargo test --lib --tests, FIREUP_TEST_POSTGRES_URL set)
Test Time: ~6 seconds
```

A library test count does not by itself indicate format correctness: those tests
build inputs that match the implementation, so they can pass while the tool
cannot read a real export. The suites that *do* speak to it are the fixture
ones, because their inputs come from a generator and an independent reader
written from the LevelDB and `EntityProto` specs rather than from `src/`:

- `tests/real_export_validation.rs` — the representative export
- `tests/export_fidelity_tests.rs` — the awkward one (output shards, namespace
  partitions, key identity, numeric limits, over-long property names), plus the
  PostgreSQL-backed halves in `tests/ddl_postgres_load_tests.rs` and
  `tests/postgres_import_tests.rs`

The PostgreSQL-backed tests skip without `FIREUP_TEST_POSTGRES_URL`; everything
else runs with no database.

**Test Categories:**
- Unit tests for LevelDB parsing
- Schema analysis and normalization tests
- DDL generation tests
- Data transformation tests
- PostgreSQL import tests
- Integration tests available
- Deployment tests available

### Integration Testing ⛔
**Location:** `tests/`

`cargo test` does not compile — `tests/integration_tests.rs` is stale against
`src/types.rs` (residual risk 12). This is pre-existing, not a regression. The
suites below exist as files; only `real_export_validation` currently builds and
runs.

**Test Suites:**
- `real_export_validation.rs` - Real-export format validation (builds, runs)
- `integration_tests.rs` - End-to-end workflow testing (does not compile)
- `deployment_tests.rs` - Docker and PostgreSQL testing
- `functional-tests/` - Feature-specific tests
- `data-generator/` - Test data generation utilities

**Postgres test database (November 2026):**
- Integration tests require `FIREUP_TEST_POSTGRES_URL` or `TEST_DATABASE_URL` pointing at a **dedicated** fireup test database (preferred: Docker Compose on port **5433** via `docker-compose.yml`).
- Tests **skip** when no URL is set; they do **not** fall back to host Postgres on 5432.
- URLs using the Paperclip `paperclip` role or `paperclip` database are **rejected** (`tests/postgres_test_env.rs`).
- Run `./scripts/run_integration_tests.sh` to start Compose Postgres and export the correct URL.
- **No Docker socket access?** Run a throwaway cluster of your own — it touches neither the host
  cluster nor the Paperclip role, since it has its own data directory, port, and superuser:
  ```bash
  export PATH=/usr/lib/postgresql/18/bin:$PATH
  PGDIR=$(mktemp -d); initdb -D "$PGDIR/data" -U fireup --auth=trust -E UTF8
  pg_ctl -D "$PGDIR/data" -l "$PGDIR/log" -o "-p 55432 -c listen_addresses=127.0.0.1" start
  createdb -h 127.0.0.1 -p 55432 -U fireup fireup_local_test
  export FIREUP_TEST_POSTGRES_URL='postgresql://fireup@127.0.0.1:55432/fireup_local_test'
  # …run tests, then: pg_ctl -D "$PGDIR/data" stop && rm -rf "$PGDIR"
  ```
  Used on 2026-11-04 to verify the SAK-164 PostgreSQL store.

**Functional sample-data ladder:**
- Levels 1–4 generators live under `tests/data-generator/` (`npm run generate:level1` … `generate:level4`, or `generate:all-levels`).
- Rust coverage: `tests/functional-tests/level{1,2,3,4}_tests.rs` (skips when `tests/.firestore-data` fixtures are missing).
- See `tests/functional-tests/README.md` for fixture generation and `cargo test --test integration_tests functional_tests::`.

---

## Local cloud (`fireup local`)

An independent implementation of two of Google's **published** API contracts —
`google.firestore.v1` over gRPC and a subset of Identity Toolkit (Firebase Auth)
over REST — served on **one** port, so official SDKs can point at it via
`FIRESTORE_EMULATOR_HOST` and `FIREBASE_AUTH_EMULATOR_HOST`. Not the Firebase
Emulator Suite, not affiliated with Google. See `docs/local/README.md`,
`.cursor/rules/firebase-gcp-compliance.mdc`, and
`docs/licenses/FIREBASE_GCP_LOCAL.md`.

**Phase 0 (SAK-163)** — document CRUD over gRPC against an in-memory store.
**Phase 1 (SAK-164, November 4 2026)** — durable storage and seeding:

- `Store` trait with two implementations, held to one conformance suite:
  `MemoryStore` (default) and `PostgresStore` (`--postgres-url`).
- One row per document in `fireup_local_documents`; fields stored as JSONB in
  the public Firestore REST value shape; `create_time`/`update_time` kept exact
  as seconds + nanos.
- Single port carries gRPC (HTTP/2) and HTTP/1.1 `GET /` (discovery) and
  `GET /health` (a real check — it queries the store).
- `fireup local seed --backup-file …` loads a managed export through the same
  parser `fireup import` uses; upserts, and exits non-zero if any document was
  not written.
- `docker compose up -d postgres fireup-local` runs it beside Compose Postgres.

**Phase 2 (SAK-165, November 4 2026)** — queries, collection listings, and
transactions. The 12-RPC v1 surface is now served in full:

- `RunQuery` over a `StructuredQuery`: AND/OR filters, all ten field operators,
  the four unary tests, `orderBy`, `limit`/`offset`, and `select` projection.
  The engine (`src/local/query.rs`) runs in the server process against a
  collection scan, so both stores answer a query identically.
- `ListDocuments` (including `show_missing` for documents that exist only as the
  parent of something deeper) and `ListCollectionIds`, both paged by resume
  token rather than offset.
- `BeginTransaction`/`Commit`/`Rollback` with **atomic** commits and
  **optimistic concurrency**: a transaction's read set is re-checked inside the
  same store transaction as its writes, so a document that changed since it was
  read aborts the commit (`ABORTED`) instead of being overwritten.
- `BatchWrite`, deliberately non-atomic per the API, with one status per write.
- Every write path — single-document RPCs included — now goes through one
  atomic `Store::commit`, which removed the check-then-write race in
  `CreateDocument` and the non-atomic `Commit` of Phase 0/1.
- Full fidelity matrix published in `docs/local/README.md`.

**Verified on 2026-11-04:** `cargo test --lib local::` (71), `--test
local_firestore_tests` (25, including a real same-port gRPC + HTTP check),
`--test local_postgres_store_tests` (7, against a throwaway cluster: documents
and every Firestore value type come back unchanged after a server restart), and
`scripts/smoke-local-firestore.sh` **27/27 against both stores** with Google's
official `@google-cloud/firestore` client — queries, `listDocuments()`,
`listCollections()`, `runTransaction()` (including a forced-contention retry
that the SDK itself has to retry), and `bulkWriter()`.

**Not verified here:** the `fireup-local` Docker image build (no Docker socket
access in the agent environment). The Dockerfile was fixed to copy `build.rs`
and `proto/`, without which the image could not have compiled the generated
stubs at all.

**Still unserved** (answer `UNIMPLEMENTED` with a message naming the gap):
realtime listeners (`Listen`, streaming `Write`), aggregation queries,
`PartitionQuery`, field transforms, nested paths in an `update_mask`,
point-in-time reads, and security rules. The fidelity matrix in
`docs/local/README.md` is the authoritative list.

Collection group queries, query cursors, and nested field paths in filters,
orders, projections, and read masks are served as of SAK-191.

**Phase L (SAK-168, November 4 2026)** — Firebase Auth / Identity Platform, in
parallel with Phase 2 and without waiting for a Phase 2 exit:

- Public Identity Toolkit REST only, on the same port:
  `accounts:signUp` (email+password and anonymous),
  `accounts:signInWithPassword`, `accounts:signInWithCustomToken`,
  `accounts:lookup`, `accounts:update`, `accounts:delete`, the `securetoken`
  `/v1/token` refresh, and the project-scoped admin endpoints
  (`accounts` create, `:lookup`, `:update`, `:delete`, `:batchGet`). Plus the
  documented emulator control routes
  (`DELETE /emulator/v1/projects/{p}/accounts`, `GET …/config`).
- SDKs reach these at `/identitytoolkit.googleapis.com/v1/…` (the API host
  becomes the first path segment when `FIREBASE_AUTH_EMULATOR_HOST` is set); the
  bare `/v1/…` form is accepted too, for `curl`.
- ID tokens are **unsigned** JWTs (`alg: none`) carrying the published claim
  set — the form Google's open-source SDKs accept in emulator mode. No signing
  key is generated or shipped: production key material is a non-goal.
- Accounts live in their **own** store (`UserStore`, with the same
  memory/Postgres + one-conformance-suite shape as documents), never as
  Firestore documents, so identity data is not reachable through the Firestore
  API or counted by `GET /health`.
- `fireup_local_auth_users`, keyed `(project_id, local_id)`, with a **partial**
  unique index on `(project_id, email) WHERE email IS NOT NULL` so any number of
  anonymous accounts coexist. `EMAIL_EXISTS` / `DUPLICATE_LOCAL_ID` are those
  constraints, not a racy pre-read.
- `src/local/http.rs` grew `Content-Length` body reading and POST/DELETE
  routing; `/` and `/health` stay read-only, and `Transfer-Encoding: chunked`
  is refused with a `400` rather than mis-read.

**Non-goals, answering `501` with a message naming the gap:** phone auth,
multi-factor enrolment and the rest of the Identity Platform `v2` surface,
federated/OAuth providers, production key material. (Out-of-band email flows
were on this list until SAK-193, below.)

**Local development only:** unsigned tokens, no API-key or admin-credential
check, and passwords hashed without a work factor. Anything that can reach the
port can act as any account; the bind is loopback by default and the server says
this at startup.

**Verified on 2026-11-04:** `cargo test --lib` (371, of which 77 are the auth
module), `--test local_auth_tests` (15, over real HTTP/1.1 on the shared port —
including sign-up → sign-in → lookup, refresh, admin CRUD, paging, and the
request-parsing edge cases), and `--test local_auth_postgres_tests` (8, against
a throwaway cluster: an account and its password digest survive a server
restart, uniqueness is enforced by the database, five anonymous accounts
coexist, and no row lands in the documents table). Full `cargo test` green.

**Verified with the official SDKs (SAK-171):** `scripts/smoke-local-auth.sh`
31/31, against both the in-memory and the PostgreSQL account store. It drives
the surface with `firebase-admin` 13 and the `firebase` client SDK 11 over
`FIREBASE_AUTH_EMULATOR_HOST` — `createUser`, `getUser`, `getUserByEmail`,
`updateUser`, `setCustomUserClaims`, paged `listUsers`, `deleteUser`,
`createCustomToken` → `signInWithCustomToken`, `signInWithEmailAndPassword`,
`signInAnonymously`, `verifyIdToken` accepting the unsigned emulator token, and
a forced refresh extending the session. The deliberate gaps (batch account
writes, out-of-band email flows, session cookies) are asserted to fail cleanly
rather than left untested. (The out-of-band gap was promoted into real
assertions by SAK-193, which is what that check exists to force.)

That test found two real divergences that the Rust suite could not, because the
Rust suite was written against the same reading of the contract as the code:
custom-token sign-in **overwrote the account's stored custom claims**, and it
reported the account's original `sign_in_provider` instead of `custom`. Both are
fixed, with regression tests, by separating session state from account state —
see the "Session claims vs account claims" section in `docs/local/README.md`.
The lesson generalises: a passing test count proves self-consistency, not
compatibility.

**Phase 3 (SAK-166)** — the Datastore API over the same rows. See
`docs/local/README.md#datastore-rpcs-phase-3`.

**Phase 4 (SAK-167, November 4 2026)** — the migrator and the local data plane
documented and demonstrated as one workflow:

- `docs/local/END_TO_END.md` — export → `fireup import` (normalized SQL) →
  `fireup local seed` (documents) → official SDK reads and writes → SQL over the
  same PostgreSQL database, with the fidelity gaps stated.
- `scripts/demo-export-to-sql.sh` plus `demo/read-with-sdk.mjs` and
  `demo/query.sql` — that document, executable and re-runnable. Verified on the
  `export-onestore` fixture: 12/12 entities import into `"Users"`/`"Orders"`,
  the same 12 seed as documents, `@google-cloud/firestore` and
  `@google-cloud/datastore` read and write them, and SQL sees the SDK's write.
- `README.md` positioning: Postgres-backed local data plane **plus** export
  migrator, not a general GCP emulator; complements a full local-cloud suite
  rather than replacing it; independent implementation of public Google API
  contracts, not affiliated with Google.

**One documentation correction landed with it.** The seeding note previously
claimed the export parser erases Firestore value types to plain strings. It does
not: it emits a tagged map (`__fireup_type__`), which `fireup import` already
consumes to pick `bytea`/`timestamptz`/`point`/`text`. `fireup local seed`
ignored the tag, so that — not the parser — was where blobs, timestamps, geo
points, and key references lost their type. No behaviour changed in Phase 4; the
gap itself was closed next.

**SAK-174 (November 4 2026)** — seeding restores Datastore value types. The gap
Phase 4 documented is closed: `src/local/seed.rs` reads the same
`__fireup_type__` tags `fireup import` reads and produces `timestampValue`,
`bytesValue`, `geoPointValue`, and `referenceValue` instead of a `mapValue`.

- Dispatch is on the tag only, never on a string's shape, so a document holding
  the literal text of a date still holds a string. A map with an unknown tag
  value stays a `mapValue`; an unrestorable payload (bad base64, a key path that
  is not kind/id pairs, a reference into a non-default namespace) fails its
  document rather than seeding as something else.
- A key reference becomes a full resource name through the same key-path mapping
  seeding uses for a document's own name, so the reference resolves to the
  document that key seeded as.
- Verified by `tests/local_firestore_tests.rs`
  (`a_seeded_export_reads_back_as_real_firestore_value_types`), which seeds the
  `export-onestore` fixture and reads all four types back over gRPC, and by
  `scripts/demo-export-to-sql.sh`, where `@google-cloud/firestore` now reads a
  `Timestamp`, a `Buffer`, a `GeoPoint`, and a followable `DocumentReference`.
- Non-default namespaces remain skipped; that is unchanged and deliberate.

**SAK-190 (November 6 2026)** — field transforms. `serverTimestamp`,
`increment`, `arrayUnion`, and `arrayRemove` are served on `Commit` and
`BatchWrite`, both as a write's `update_transforms` and as a transform-only
`DocumentTransform` write. This supersedes the Phase 2 "still unserved" line
above for field transforms.

- The value is computed in `src/local/transforms.rs`, called from
  `Store::commit` **inside the write's own store transaction**, so ten
  concurrent `increment(1)` calls end at ten rather than at one. The proto→store
  translation is pure and runs before the store is touched, so an unsupported
  transform fails the whole batch instead of applying half of it.
- `WriteResult.transform_results` reports what each transform resolved to, in
  order, so a client reads the new counter off the commit.
- The semantics are the ones the vendored public `write.proto` specifies: a
  transform runs *after* the mutation it rides on, `increment` sets a missing or
  non-numeric field to the operand, a double on either side makes the sum a
  double, integer overflow saturates instead of wrapping, the array transforms
  treat a non-array field as empty and compare `3` to `3.0` (and `NaN` to `NaN`)
  as equal, and `serverTimestamp` is millisecond-precision and identical for
  every field in one commit.
- Still refused, explicitly: `maximum`/`minimum` (no SDK emits them) and a
  transform on a nested field path.
- Verified by 14 unit tests in `src/local/transforms.rs`, two new store
  conformance cases that both the in-memory and PostgreSQL backends must pass,
  7 new gRPC tests in `tests/local_firestore_tests.rs` (33 total), and 6 new
  checks in `scripts/smoke-local-firestore.sh` — **33/33 against both stores**
  with Google's official client, including ten concurrent increments.

**Phase L.1 (SAK-193, November 6 2026)** — the out-of-band email flows a sample
Firebase web app reaches after sign-up or sign-in. The Phase L non-goal is
retired: `sendOobCode` / `resetPassword` no longer answer `501`.

- Served: `accounts:sendOobCode` for all four request types
  (`PASSWORD_RESET`, `VERIFY_EMAIL`, `EMAIL_SIGNIN`, `VERIFY_AND_CHANGE_EMAIL`),
  `accounts:resetPassword` (both the code-check and the confirm call),
  `accounts:update` with an `oobCode` instead of an `idToken` (the SDK's
  `applyActionCode`), `accounts:signInWithEmailLink`, the admin
  `projects/{p}/accounts:sendOobCode` with `returnOobLink` (the Admin SDK's
  `generate*Link`), `GET`/`DELETE /emulator/v1/projects/{p}/oobCodes`, and
  `GET /emulator/action` so a generated link resolves rather than 404ing.
- **No mail transport, and that is the design**, not a shortcut: an SMTP client
  in a local dev tool is a credential to configure and a way to mail a real
  person from a test suite. The link is logged at `info` and listed at the
  `oobCodes` endpoint — the documented local convention — so a test can follow
  the whole flow without a mailbox.
- Codes are single-use (redeeming deletes; a *failed* redemption leaves the link
  usable), expire after an hour with `EXPIRED_OOB_CODE` distinct from
  `INVALID_OOB_CODE`, and live in their own `fireup_local_auth_oob_codes` table
  so a link survives a recompile. Six new assertions in the shared store
  conformance suite keep the memory and Postgres backends from drifting.
- Completing a reset or an email change revokes outstanding sessions via
  `validSince`; merely verifying an address does not. A stale `VERIFY_EMAIL`
  link is refused if the account's address changed after it was sent, and
  `signInWithEmailLink` requires the address the link was issued for, so a
  forwarded link cannot sign in whoever received it.
- `GET /emulator/action` is **Fireup's own** JSON route, not a copy of Google's
  hosted page, and says so in its response.

**Verified on 2026-11-06:** `cargo test --lib` 422 (119 in the auth module),
`--test local_auth_tests` 18 over real HTTP/1.1 (including reset and email-link
sign-in driven end-to-end from the listed link, and the link naming the port
the server actually bound), `--test local_auth_postgres_tests` 10 against a
throwaway PostgreSQL 18 cluster (a reset link survives a restart and its row is
gone once redeemed; a sign-in code stores with a null `local_id`). Full
`cargo test --lib --bins --tests` green, 0 failures; `cargo clippy` clean on the
auth module. The 57 doctest failures are pre-existing, all in generated proto
code.

**Verified with the official SDKs:** `scripts/smoke-local-auth.sh` **36/36**,
covering `generatePasswordResetLink`, `generateEmailVerificationLink`,
`sendPasswordResetEmail` → `verifyPasswordResetCode` → `confirmPasswordReset`,
`sendEmailVerification` → `applyActionCode`, and `sendSignInLinkToEmail` →
`isSignInWithEmailLink` → `signInWithEmailLink`.

That smoke test earned its keep again, exactly as in SAK-171. The client SDK's
`ActionCodeURL` parser asserts on `apiKey`, `oobCode` and a known `mode`
*together* and yields nothing without all three, so a link built without an
`apiKey` — a parameter that reads as optional, and which this surface does not
validate — made `isSignInWithEmailLink` quietly answer `false` and
`signInWithEmailLink` fail. Every Rust test passed, because they were written
from the same reading of the link format as the code. The link now echoes the
caller's own `?key=`, or a plainly-labelled placeholder.

**SAK-198 (November 6 2026)** — the Datastore spelling of the same transforms.
`Mutation.property_transforms` is served on `Commit`, and
`MutationResult.transform_results` reports what each one resolved to. This
closes the one deliberate gap SAK-190 left.

- The question SAK-190 did not answer was the encoding, and that is where the
  work went. A transform names a *property*, and the value it reads is whatever
  `entity_map::entity_to_document` left in the corresponding field, so the
  operands cross the same `Value` conversion the entity's own properties do. The
  `integer_value`/`double_value` tags carry through, so an integer `increment` of
  an integer property stays exact; array elements convert too, so
  `remove_all_from_array` of a `key_value` matches the reference string the key
  is stored as. Comparing an unconverted operand would silently never match.
- Property *paths* (`foo.bar`) and names containing a Datastore escape are
  refused rather than guessed at, as is `maximum`/`minimum` — the latter through
  the same `transforms::compile_kind` the Firestore surface uses, so there is one
  definition of what is served rather than two that can drift. A transform on a
  `delete` is `INVALID_ARGUMENT`, per the proto.
- **A transform runs after the operation**, and `property_mask` on a mutation is
  still unserved, so an unmasked `update`/`upsert` replaces the entity before the
  transform reads it: a Datastore `increment` accumulates only when the submitted
  entity carries the current value. Surprising enough to be pinned by a test and
  called out in the fidelity matrix. Masked mutations are the follow-up.
- Verified by 6 new unit tests in `src/local/entity_map.rs` (9 there, 463 in the
  library), 5 new gRPC tests in `tests/local_datastore_tests.rs` (11 total) —
  including one that increments the same property alternately through the
  Datastore and the Firestore surface and gets 6 — and two new steps in
  `scripts/smoke-local-datastore.sh`, passing **against both the in-memory and
  the PostgreSQL store** with Google's official client library.

**SAK-192 (November 6 2026)** — Postgres pushdown for equality/range filters.
Until now `RunQuery` read the whole collection and filtered in-process. It still
*decides* in-process; it just no longer reads rows it cannot want.

- `Query::prefilter` hands a backend the compiled filter plus the query's sort
  fields. `Store::scan_collection_filtered` / `scan_kind_filtered` may use it to
  return fewer rows, under a one-sided contract: **a superset of the matching
  rows, never a subset**. `Query::evaluate` keeps the only vote on the result,
  so the two backends still cannot disagree about what a query means. The
  default implementations ignore the pre-filter, which is what `MemoryStore`
  uses — CI keeps testing the in-process semantics.
- `src/local/postgres_filter.rs` does the translation: `==`/`in` as GIN-usable
  `@>` containment, ranges as `COLLATE "C"` byte-order comparisons (a locale
  collation disagrees with Firestore's ordering and would drop rows), numbers in
  two branches that mirror the engine's own int/double arithmetic, nested paths
  walked through the stored `mapValue`/`fields` shape. `!=`, `not-in`, the array
  operators, bytes ranges, and a half-translatable `OR` stay in-process, with
  the reasons tabulated in `docs/local/README.md`.
- A GIN index on `fields` comes with the schema. The planner ignores it at tens
  of thousands of rows (correctly — scanning is cheaper there) and switches to a
  bitmap index scan in the hundreds of thousands.

**Verified on 2026-11-06:** the load-bearing test is
`tests/local_query_pushdown_tests.rs` — it runs **49 query shapes both ways**
against a real database, full scan versus pushdown, and requires byte-identical
results, order, and `skipped` counts, plus the same for the collection group
read. Five deliberate mutations confirmed it has teeth: dropping `COLLATE "C"`
from text ranges, dropping it from `__name__`, losing the `doubleValue` branch
of a numeric comparison, letting an `OR` discard an untranslatable branch, and
re-joining a nested path with a `.` — each one fails it, the last by silently
dropping every row. `cargo test --lib` 478, `--test local_firestore_tests` 46,
`--test local_postgres_store_tests` 8 (one new end-to-end `RunQuery` over
Postgres through the gRPC server), full suite green against a throwaway
PostgreSQL 18 cluster; `cargo clippy --all-targets` clean on the new files.
Measured with `cargo run --release --example pushdown_scale`: at 20,000
documents a 1-in-1000 equality reads 20 rows instead of 20,000, a 1-in-7 reads
2,857 — roughly 4–10× less wall clock depending on selectivity.

**Also fixed here, because this change exposed it:** the shared store
conformance suite asserted an exact *store-wide* `count()` (`before + 1`). The
PostgreSQL backend shares one table across every test in its binary, so adding a
second document-writing test to that binary turned a latent race into a real
flake — measured at 1–2 failures in 8–16 runs, against 0 in 8 on the same base
without this change. A failing contract test also skips its own cleanup, which
left rows behind and cascaded into the two `local_auth_postgres_tests` that
assert the documents table is empty. Asserting the *direction* of the change
instead is no better: a concurrent insert masks the delete and you get
`11 -> 11`. The exact count contract now lives in
`conformance::assert_exclusive_count_contract`, called only from a test that
owns its store (the in-memory unit test); the shared contract asserts the same
property scoped to its own collection, which is race-free and still exact.
0 failures in 30 runs after the fix.

**Found, not fixed (tracked as SAK-205):** `value_json` cannot decode its own
encoding of a timestamp outside years 0001–9999 — it writes `+11476-08-15T…`
and `decode_timestamp` rejects it. A client that stores such a timestamp gets a
successful write and then an unreadable document, and the failure spreads to
every collection scan that includes it. Unrelated to queries; surfaced while
building the pushdown corpus. `MemoryStore` is unaffected, since it does not
encode.

---

## Dependencies & Environment

### Runtime Dependencies
- **PostgreSQL:** 15+ (via Docker)
- **Docker:** 28.0.4+ ✅ Available
- **Docker Compose:** v2.38.2+ ✅ Available
- **Rust:** 1.70+ ✅ Required for building

### Key Rust Dependencies
```toml
tokio = "1.0"                    # Async runtime
tokio-postgres = "0.7"           # PostgreSQL client
deadpool-postgres = "0.12"       # Connection pooling
serde = "1.0"                    # Serialization
serde_json = "1.0"               # JSON handling
clap = "4.0"                     # CLI parsing
tracing = "0.1"                  # Structured logging
anyhow = "1.0"                   # Error handling
chrono = "0.4"                   # Date/time handling
```

### Development Environment
- Docker Compose configuration ready
- PostgreSQL initialization scripts available (`init.sql`)
- Environment variable templates (`.env.example`)
- Comprehensive documentation

---

## Documentation Status

### Available Documentation ✅

#### User Documentation
- **README.md** - Project overview and positioning, quick start, usage examples
- **docs/local/END_TO_END.md** - The export → SQL → live API → SQL workflow
- **docs/local/README.md** - `fireup local`: flags, fidelity matrix, auth surface, storage layout
- **DEPLOYMENT.md** - Detailed deployment guide for development and production
- **docs/INTEGRATION_TESTING.md** - Integration testing procedures
- **tests/DEPLOYMENT_TESTING.md** - Deployment testing documentation
- **docs/licenses/FIREBASE_GCP_LOCAL.md** - Licence and attribution ledger
- **tests/fixtures/README.md** - Managed-export fixtures and their binary format

#### Developer Documentation
- **.kiro/specs/firestore-postgres-migration/requirements.md** - Complete requirements
- **.kiro/specs/firestore-postgres-migration/design.md** - Architecture and design
- **.kiro/specs/firestore-postgres-migration/tasks.md** - Implementation task tracking
- Inline code documentation throughout source files

#### Operational Documentation
- Docker Compose configuration with comments
- Environment variable documentation
- PostgreSQL client connection guides
- Troubleshooting sections in README and DEPLOYMENT.md

**Documentation Quality:** Excellent - All major aspects covered

---

## Features Implemented vs. Planned

### Requirements Compliance

#### ⛔ Requirement 1: Firestore Backup Import
- [x] Parse LevelDB backup files per Google's specification — 12 of 12 fixture
      entities read, across every kind and shard of the tree (SAK-141)
- [x] Validate backup integrity with CRC32 checksums — masked CRC32C, verified
      against the independently CRC-checked fixtures (SAK-141)
- [x] Provide detailed error messages for corrupted files
- [ ] Import parsed data to PostgreSQL-compatible format — schema is created,
      no rows are written (residual risk 3)
- [ ] Generate summary reports of import operations — the summary reports counts
      that were never imported (residual risk 3)

#### ✅ Requirement 2: Schema Analysis & Normalization
- [x] Automatic document structure detection
- [x] Field type and nested structure identification
- [x] Automatic normalization (1NF, 2NF, 3NF)
- [x] Type conflict detection with warnings
- [x] NOT NULL constraint recommendations
- [x] DDL file generation for review
- [x] Detailed transformation reporting

#### ⚠️ Requirement 3: SQL Query Engine (Not Implemented - By Design)
- [N/A] Standard SQL SELECT queries
- [N/A] Query translation for Firestore data
- [N/A] SQL clause support (WHERE, ORDER BY, etc.)

**Note:** This requirement was intentionally replaced with direct PostgreSQL integration. Instead of building a custom query engine, Fireup imports data directly into PostgreSQL, allowing users to leverage PostgreSQL's native query capabilities.

#### ✅ Requirement 4: PostgreSQL Compatibility
- [x] PostgreSQL Docker backend integration
- [x] Standard PostgreSQL authentication (via Docker container)
- [x] Standard PostgreSQL table and view presentation
- [x] PostgreSQL metadata compatibility
- [x] Connection pooling and session management

#### ✅ Requirement 5: Logging & Monitoring
- [x] Comprehensive operation logging with timestamps
- [x] Query performance metrics tracking
- [x] Detailed error logging with context
- [x] Progress indicators for long operations
- [x] Audit logs for data operations

---

## Residual Risks

**Evidence basis.** All items below were observed on 2026-11-02 under SAK-139 by
running the CLI against `tests/fixtures/export-onestore/` and
`tests/fixtures/export-v1-entity/`. Those fixtures are independently verified as
spec-correct by `tests/fixtures/verify_datastore_export.py`, so these are tool
defects, not fixture artifacts. Severity is "does this silently lose data?".

### Critical — silent data loss

1. ~~**Wrong record checksum algorithm**~~ — **resolved (SAK-141)**
   `calculate_crc32` compared an unmasked IEEE CRC-32 against LevelDB's masked
   CRC32C, so every record of a real export mismatched and
   `parse_block_records` advanced one byte at a time until the block was
   exhausted (`validate` → `No records found in LevelDB file`, 0 of 12
   entities). Now `leveldb_parser::crc32c` computes Castagnoli `0x82F63B78`
   masked with `((crc >> 15) | (crc << 17)) + 0xA282EAD8`; `crc32fast` cannot
   produce that, so the implementation is local and no dependency was added.

2. ~~**Wrong payload protobuf schema**~~ — **resolved (SAK-141)**
   The decoder only handled `google.datastore.v1.Entity` (the API proto), while
   a managed export carries legacy `storage_onestore_v3.EntityProto`: unrelated
   field numbers, **proto2 groups** (which `prost` cannot represent at all), and
   types carried in `Property.meaning` rather than in the value field.
   `entity_proto.rs` now decodes it with a hand-written wire decoder — groups,
   `meaning` types (`GD_WHEN` microseconds, `BLOB`, `TEXT`, `ENTITY_PROTO`,
   `EMPTY_LIST`), repeated-property arrays, and the full key including
   namespace. The `v1.Entity` path is kept for Datastore Admin / BigQuery
   exports and chosen per record. Field-level notes remain in
   `tests/fixtures/README.md`.

3. ~~**`import` reports rows it never wrote**~~ — **resolved (SAK-142)**
   Only `create_schema` was called and the result was then constructed with
   `total_records_imported: documents.len()`, so a run that created tables and
   inserted nothing reported a full successful import. `TableImportSpec` now
   carries the transformed rows, `execute_full_import` actually calls
   `import_transformed_data`, and `FullImportResult` tracks
   `total_records_expected` so `is_successful()` rejects an import that did not
   write what it was given. Values are bound as text parameters cast to the
   column type (`$1::UUID`, `$2::JSONB`, `$3::TEXT[]`), so NULLs and structured
   values round-trip without string-formatting SQL; INSERTs never retry, because
   a transparent retry duplicates rows. `tests/postgres_import_tests.rs` asserts
   the row counts actually present in PostgreSQL after an import.

4. ~~**Document identity is discarded**~~ — **resolved (SAK-141 + SAK-143)**
   Generated tables used `id UUID` populated from `uuid::Uuid::new_v4()`, and no
   column held the Datastore key name or id. Datastore keys are strings or
   `int64` (the fixture includes `ada`, `unicode-ユーザー`, and
   `4503599627370496`), so even a working import produced unjoinable,
   non-idempotent rows. Parsing preserves the identity (SAK-141): both decoders
   emit the full `DatastoreKey` (namespace plus ancestor path). The generated
   schema now carries it (SAK-143): every entity table has `key_path`
   (namespace-qualified, e.g. `[tenant]Users/ada/Orders/1001`) as its primary
   key, plus `key_namespace`, `key_kind`, `key_name`, `key_id BIGINT` and
   `parent_key_path` for the ancestor link. The surrogate UUID is gone, so
   re-importing the same export is idempotent.

### High — partial data loss or failed migrations

5. ~~**Only one file of an export tree is read**~~ — **resolved (SAK-141)**
   `resolve_backup_path` returned the first `output-0` it found, so every other
   kind and every shard past the first was dropped with no warning (the
   12-entity tree yielded one kind), and the resolver could also fall back to an
   `.export_metadata` file and parse metadata as entities. `resolve_export_inputs`
   now collects every `kind_*/output-N`, ordered by directory and shard number,
   excludes metadata files, and both `FirestoreDocumentParser` and
   `BackupValidatorImpl` aggregate across all of them.

6. ~~**Generated DDL is not always loadable**~~ — **resolved (SAK-143)**
   `Orders` DDL emitted `user TEXT NOT NULL` unquoted; `user` is reserved, so
   `psql` rejected it with `syntax error at or near "user"` and aborted every
   statement after it. Identifiers were never quoted, so table names also
   silently folded to lower case and disagreed with the names foreign keys used.
   Every generated identifier now goes through `ddl_generator::quote_identifier`
   (embedded quotes doubled), so reserved words load and `Users` stays `Users`.
   **SAK-170** closed the other half: only the DDL side quoted, so `fireup
   import` still emitted `INSERT INTO Orders (…, user)` and failed against the
   `"Orders"` table the DDL had just created. The import path (`sql_generator`,
   the `TableImportSpec` INSERTs, and the constraint-validation queries) now
   quotes through the same function, and a failing statement is reported with
   the server's own message and the statement text instead of a bare
   `db error`. DDL statements also run under per-statement savepoints, so the
   "object already exists — continuing" path no longer leaves the transaction
   aborted (`cli_import_handles_reserved_words_and_mixed_case_identifiers`).

7. ~~**`NOT NULL` emitted for sparse fields**~~ — **resolved (SAK-143)**
   `nickname` is null in 4 of 5 fixture users yet was generated `TEXT NOT NULL`,
   which any real import would violate. The cause was in the analyzer, not the
   threshold: `presence_percentage` counted explicit nulls as occurrences, so a
   field present-but-null in every document scored 100%. Nulls are now excluded
   from presence, and a column is `NOT NULL` only at 100% populated presence.

8. ~~**Type mapping loses fidelity**~~ — **resolved (SAK-143)**
   Observed in generated DDL: timestamp → `TEXT`, blob → `JSONB` (should be
   `BYTEA`), `int64` → `INTEGER` (overflows on auto-allocated key ids),
   nested map emitted both as `address JSONB` *and* flattened
   `address_city`/`address_postcode`/`address_verified` (duplicated data).
   The decoders tag the types JSON cannot express (`__fireup_type__`: blob,
   timestamp, geo point, key reference) instead of stringifying them, so the
   analyzer maps them exactly: timestamp → `TIMESTAMPTZ`, blob → `BYTEA`,
   integer → `BIGINT`, geo point → `POINT` (previously dropped), key reference →
   `TEXT`. A nested map is emitted as JSONB *or* flattened, never both, and the
   redundant `pk_<table>` unique index on the primary key is gone — PostgreSQL
   already indexes a `PRIMARY KEY`.

### Medium — operability

9. ~~**Failures are reported as success**~~ — **resolved (SAK-142 + SAK-137).**
   `analyze` and `import` exit non-zero when a non-empty source yields 0
   documents, and `import` exits non-zero on a 0-row or partial import.
   `validate` (including `--check-quality`) now treats readable LevelDB records
   that decode to zero Firestore documents as invalid and forces the integrity
   score to `0.0`, so framing success can no longer look like a completed
   quality check.

10. ~~**Hardcoded credentials / ignored connection string**~~ — **resolved
    (SAK-142).** `parse_postgres_url` validated only the `postgresql://` prefix
    and then returned a fixed `postgres:password@localhost:5432/fireup_test`.
    `data_importer::connection::parse_connection_url` now parses scheme, user,
    password, host (including bracketed IPv6), port, database and query
    parameters, percent-decoding each component. Unsupported parameters and a
    TLS-requiring `sslmode` are rejected rather than silently dropped, since the
    pool is built with `NoTls`. No credential remains in source: `--postgres-url`
    supplies them, with `PGUSER`/`PGPASSWORD` as the fallback.

11. ~~**Library API panics**~~ — **resolved (SAK-142).**
    `get_monitoring_system()` now lazily installs the default configuration
    instead of panicking, so `parse_documents()` works for a consumer that never
    called `initialize_monitoring()`. Guarded by
    `tests/library_api_tests.rs`, which runs in its own process so no other test
    can mask the regression.

12. **`cargo test` does not compile** — **resolved (SAK-142).**
    `tests/integration_tests.rs` failed with 9 errors (`parse_backup` called
    without the `LevelDBParser` trait in scope; `ColumnDefinition.data_type`,
    renamed to `column_type`), pre-existing and reproduced at `fd24bf2`. Both
    are fixed, so the whole suite compiles. `cargo test` is now meaningful.
    The 4 `functional_tests::level3_tests` tests it exposed were failing on a
    stale expectation — they asserted `name`/`age` on the `users` collection,
    which the generator never wrote — and were rewritten against the fixture's
    actual `first`/`last`/`born`/`middle` fields in SAK-145. All 46 tests in
    `tests/integration_tests.rs` now pass with nothing ignored.

13. **`--drop-existing`, `--batch-size`, `--continue-on-error`** — **resolved
    (SAK-142).** `--drop-existing` was a log line with no effect; it now runs
    `DROP TABLE IF EXISTS … CASCADE` for the schema's tables in one transaction.
    `--batch-size` sets the rows per multi-row INSERT (clamped to PostgreSQL's
    65535 bind-parameter limit) and `--continue-on-error` retries a failed batch
    row by row so the importable rows land. A partial import still exits
    non-zero: it is not a completed migration.

### Not yet assessed

- **`import` against a managed export.** Covered by
  `cli_import_managed_export_fixture_into_postgres` in
  `tests/postgres_import_tests.rs` (set `FIREUP_TEST_POSTGRES_URL` to run it).
  It imports `tests/fixtures/export-onestore/` and asserts 5 `Users` + 7
  `Orders` rows. Still not run in default CI without a Postgres URL.
- **Performance / large exports.** No benchmark has been run against a
  multi-shard export.
- **A genuine GCP-produced export.** The fixtures are built to the documented
  format, not captured from a live project. Confirming against a real
  `gcloud firestore export` remains outstanding and should gate any
  production claim.

### Non-issues

1. **Unused API Surface (Not Issues - Design Choice)**
   - Many structs and methods are implemented but not yet utilized
   - Represents comprehensive API design for future features
   - Examples: Advanced SQL generation options, custom type mappings
   - These generate compiler warnings but are intentionally preserved

3. **Test Data Availability**
   - Realistic export fixtures now live in `tests/fixtures/` (see its README),
     generated to the documented format and independently CRC-verified
   - Still missing: a capture from a live `gcloud firestore export`, which is
     what would let any production claim be made

4. **Performance Tuning**
   - Default batch sizes may need adjustment for very large datasets
   - Connection pool sizes are conservative defaults
   - PostgreSQL configuration should be tuned for specific workloads

### Compiler Warnings (110 total)

**Categories:**
- Dead code (unused struct fields) - 45%
- Unused methods - 40%
- Unused types - 15%

**Assessment:** Non-critical. These warnings represent:
- Comprehensive API design with unused features
- Future extensibility points
- Derived trait implementations on unused types

**Recommendation:** Can be addressed in future releases as features are utilized

---

## Production Readiness Assessment

### ⛔ NOT Ready for Production Use

Do not run this against data anyone depends on. Spec-correct managed-export
fixtures now round-trip through `validate` / `analyze` / `import` (when a
Postgres URL is provided), but a **live GCP-captured export** has not been
confirmed, and large multi-shard performance is unmeasured.

**Strengths:**
1. **Clean Architecture:** Well-organized, modular codebase
2. **Unit Test Breadth:** 209+ passing library tests
3. **Good Documentation:** Complete user and developer guides
4. **Modern Stack:** Rust + PostgreSQL + Docker
5. **Error Handling:** Comprehensive error types and context
6. **Monitoring:** Built-in logging and metrics

**Exit criteria before any production claim:**

1. Residual risks 1-9 fixed against the checked-in fixtures (done on `main`)
2. `validate`, `analyze`, and `import` all pass against
   `tests/fixtures/export-onestore/`, recovering all 12 entities across both
   kinds, with row counts asserted in PostgreSQL after `import`
3. The same three commands pass against an export captured from a live GCP
   project
4. Non-zero exit on total parse loss / zero-document decode (residual risk 9)

**Also recommended:**

1. **Test with Real Data** (Priority: HIGH)
   - Capture an export from a live Firestore/Datastore project
   - Test various backup sizes (small, medium, large) and multi-shard exports
   - Verify schema detection accuracy

2. **Performance Validation** (Priority: MEDIUM)
   - Benchmark import performance with large datasets
   - Tune batch sizes and connection pools
   - Optimize PostgreSQL configuration

3. **Security Review** (Priority: HIGH)
   - Review SQL generation for injection vulnerabilities
   - Validate credential management
   - Audit error messages for information disclosure

4. **Code Cleanup** (Priority: LOW)
   - Address unused code warnings
   - Add `#[allow(dead_code)]` attributes where appropriate
   - Document API surface for future features

5. **Monitoring Setup** (Priority: MEDIUM)
   - Configure production logging levels
   - Set up log aggregation
   - Establish alerting for errors

---

## Deployment Status

### Development Environment ✅
- Docker Compose configuration ready
- PostgreSQL container setup complete
- Environment variables documented
- Quick start guide available

### Production Environment ⚠️ NEEDS SETUP
- Production Docker Compose template available
- Security configurations documented
- Backup and recovery procedures documented
- **Action Required:** Create production deployment

### Client Tools Support ✅
**Verified Compatible:**
- psql (PostgreSQL CLI)
- pgAdmin (GUI)
- DBeaver (GUI)
- Python (psycopg2)
- Node.js (pg)
- Any PostgreSQL-compatible tool

---

## Next Steps & Recommendations

### Immediate (Sprint 1)
1. **Real-World Testing**
   - Obtain and test with actual Firestore backup files
   - Validate LevelDB protobuf parsing
   - Document any format variations discovered

2. **Security Audit**
   - Review SQL generation code
   - Test for injection vulnerabilities
   - Validate error handling doesn't leak sensitive data

3. **Performance Baseline**
   - Establish performance benchmarks
   - Document recommended configurations
   - Create performance tuning guide

### Short-Term (Sprint 2-3)
1. **Production Deployment**
   - Set up production environment
   - Configure monitoring and alerting
   - Establish backup procedures

2. **User Acceptance Testing**
   - Work with pilot users
   - Gather feedback on usability
   - Document common issues and solutions

3. **Documentation Enhancement**
   - Add more usage examples
   - Create troubleshooting guides
   - Document common migration patterns

### Long-Term (Future Releases)
1. **Feature Enhancements**
   - Incremental import capabilities
   - Schema migration tracking
   - Custom transformation rules
   - Web-based UI for monitoring

2. **Performance Optimization**
   - Parallel import processing
   - Memory optimization for large files
   - Query performance tuning

3. **Ecosystem Integration**
   - CI/CD pipeline templates
   - Kubernetes deployment manifests
   - Cloud platform integrations (AWS, GCP, Azure)

---

## Code Metrics

### Codebase Statistics
```
Source Files:      25 Rust files
Test Files:        11 Rust files
Total Lines:       ~15,371 lines
Documentation:     8 markdown files
```

### Component Breakdown
```
src/
├── main.rs                    ~400 lines  (CLI interface)
├── error.rs                   ~200 lines  (Error handling)
├── types.rs                   ~400 lines  (Core types)
├── monitoring.rs              ~500 lines  (Logging & metrics)
├── leveldb_parser/            ~3,000 lines (3 files)
├── schema_analyzer/           ~4,500 lines (5 files)
└── data_importer/             ~6,000 lines (4 files)
```

### Test Coverage
- LevelDB Parser: Comprehensive unit tests
- Schema Analyzer: Full normalization testing
- Data Importer: Connection and import tests
- Integration: End-to-end workflow tests
- Deployment: Docker and PostgreSQL tests

---

## Technology Stack

### Core Technologies
- **Language:** Rust 2021 Edition
- **Database:** PostgreSQL 15+
- **Containerization:** Docker & Docker Compose
- **CLI Framework:** Clap 4.0
- **Async Runtime:** Tokio 1.0

### Key Libraries
- **Database:** tokio-postgres, deadpool-postgres
- **Serialization:** serde, serde_json
- **Logging:** tracing, tracing-subscriber
- **Error Handling:** anyhow, thiserror
- **Testing:** tempfile, tokio-test

---

## Conclusion

Fireup is well-organized and broadly unit-tested. The real-export validation run
under SAK-139 showed the end-to-end migration path did not work: 0 of 12
entities were recovered from a spec-correct export, and `import` would have
reported a full successful migration while writing no rows. The read side is now
fixed — SAK-141 took the parser from 0 to 12 of 12 entities across every kind of
the export tree — so the remaining work is downstream of parsing.

The structure is sound and the gaps are specific and fixable — they are format
correctness bugs (checksum algorithm, payload proto, multi-file handling) plus
unwired plumbing, not architectural problems. The validation harness to prove
each fix now exists in the repo.

**Current State:** Implementation incomplete; blocked on format correctness  
**Recommended Next Step:** Fix residual risks 1-5, driven by the pending
acceptance tests in `tests/real_export_validation.rs`  
**Production Timeline:** Not estimable until a real export round-trips

### Verified
- ✅ 159 library unit tests passing, clean build
- ✅ Comprehensive documentation
- ✅ CLI argument surface functional
- ✅ Realistic, independently verified export fixtures in `tests/fixtures/`
- ✅ `analyze` → DDL → `psql` load path exercised (one of two generated DDL
  files loads; the other is rejected — residual risk 6)

### Outstanding Items
- ✅ Spec-correct managed-export fixtures read end to end (SAK-141)
- ✅ `import` writes rows and fails on zero/partial imports (SAK-142)
- ✅ Document identity preserved via `key_path` (SAK-143)
- ⚠️ Real GCP-captured export still untested
- ⚠️ Performance benchmarking pending
- ⚠️ Production deployment configuration needed
- ⚠️ Security audit recommended (hardcoded credential, residual risk 10)

---

**For questions or issues, please refer to:**
- [README.md](README.md) - General usage and quick start
- [DEPLOYMENT.md](DEPLOYMENT.md) - Deployment procedures
- [Issue Tracker](https://github.com/asakaxgit/fireup/issues) - Report bugs and feature requests
