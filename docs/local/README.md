# `fireup local` — local Firestore-compatible data plane

Fireup serves Google's **published** `google.firestore.v1` gRPC API on one local
port so official Firestore client SDKs can point at it with the documented
`FIRESTORE_EMULATOR_HOST` environment variable: document CRUD, `StructuredQuery`
reads, collection listings, and transactions. The [fidelity
matrix](#fidelity-matrix) says exactly what is served and what is refused.

This is an independent implementation of a public API contract. It is **not** the
Firebase Emulator Suite, is **not** affiliated with Google, and contains no code
or behaviour derived from proprietary Google binaries. See
`.cursor/rules/firebase-gcp-compliance.mdc` and
`docs/licenses/FIREBASE_GCP_LOCAL.md`.

## Run it

```bash
cargo run --bin fireup -- local              # 127.0.0.1:4589, in-memory
cargo run --bin fireup -- local --port 9000  # another port
cargo run --bin fireup -- local -v           # per-RPC debug logging

# Durable: documents live in PostgreSQL and survive a restart.
cargo run --bin fireup -- local \
  --postgres-url postgresql://fireup:fireup_dev_password@localhost:5433/fireup_dev
```

Then, in your app:

```bash
export FIRESTORE_EMULATOR_HOST=127.0.0.1:4589
```

Any official SDK (Node, Python, Go, Java, Admin SDKs) honours that variable and
will talk plaintext gRPC to Fireup instead of production Firestore.

### Flags

| Flag | Default | What it does |
|---|---|---|
| `--host` | `127.0.0.1` | Bind address. Loopback by default: there is no auth. |
| `--port` | `4589` | The one port. gRPC and HTTP share it. |
| `--postgres-url` | *(unset)* | Store documents in PostgreSQL instead of memory. |
| `--project-id` | `fireup-local` | Project the data plane answers for. |
| `--database-id` | `(default)` | Firestore's default database is literally `(default)`. |
| `--enable` | `firestore` | Services to serve. Repeatable; closed set, so a typo is an error. |
| `--max-connections` | `5` | PostgreSQL pool size. |

An unreachable `--postgres-url` is a **hard error**, not a fallback to memory:
silently serving an empty in-memory store would be indistinguishable from data
loss.

### One port, two protocols

gRPC clients speak HTTP/2; `curl`, humans, and container health checks speak
HTTP/1.1 to the same port. The server tells them apart by the HTTP/2 connection
preface.

```bash
curl http://127.0.0.1:4589/          # service discovery
curl http://127.0.0.1:4589/health    # health; asks the store, so it fails when the DB is gone
```

The HTTP surface is read-only — anything but `GET`/`HEAD` answers `405`.

## Docker Compose

`docker-compose.yml` runs `fireup-local` beside the existing Postgres service:

```bash
docker compose up -d postgres fireup-local
curl http://127.0.0.1:4589/health
export FIRESTORE_EMULATOR_HOST=127.0.0.1:4589
```

The container stores documents in the Compose Postgres, so
`docker compose restart fireup-local` keeps the data. Its health check asks
`GET /health` over the real port, so it only reports healthy when the database
is reachable too.

## Seeding from an export

```bash
fireup local seed --backup-file ./export \
  --postgres-url postgresql://fireup:fireup_dev_password@localhost:5433/fireup_dev \
  --project-id my-project
```

Seeding reuses the same export parser as `fireup import`, so there is one
implementation of the export format. Writes are upserts: seeding twice is the
same as seeding once. If any document could not be written, the command prints
each reason and **exits non-zero** — a partially seeded store looks identical to
a complete one from the outside, so it is not reported as success.

Two limits worth knowing:

- The shared export parser erases Firestore value types on the way in
  (`{"timestampValue": …}` becomes a plain JSON string). Seeding therefore
  infers types from JSON and cannot restore `timestampValue`, `bytesValue`,
  `geoPointValue` or `referenceValue`. Nothing is guessed from a string's shape.
- Datastore entities in a non-default **namespace** are skipped, not merged:
  two namespaces can hold the same key path, and mapping both onto one document
  name would silently overwrite data.

### Why port 4589

`4589` is free of the common collisions: Google's Firebase emulator defaults
(8080 Firestore, 4000 UI) and floci-gcp's single port (4588). Fireup sitting at
4589 means you can run it beside either without an argument about ports.

## Fidelity matrix

What `fireup local` answers, and what it refuses. The rule throughout: **an
unsupported feature is an explicit error, never a wrong answer.** Every refusal
is a gRPC `UNIMPLEMENTED` (or `INVALID_ARGUMENT` for a malformed request) whose
message names the missing piece, so an SDK failure reads as a clear gap instead
of a hang, a protocol error, or — worst — a plausible-looking wrong result.

### RPCs

All 17 RPCs of `google.firestore.v1.Firestore`. The 12 marked **served** are the
v1 surface frozen in the `firestore-v1-contract` document on
[SAK-163](/SAK/issues/SAK-163).

| RPC | Status | Notes |
|---|---|---|
| `GetDocument` | served | Default consistency or inside a transaction. `read_time` refused. |
| `ListDocuments` | served | Name order; paged by resume token; `show_missing` supported; `collection_id` may be empty to span every collection under the parent. `order_by` refused. |
| `CreateDocument` | served | 20-character auto ids; `ALREADY_EXISTS` enforced inside the write, not by a check before it. |
| `UpdateDocument` | served | `update_mask` merge, `current_document` preconditions. |
| `DeleteDocument` | served | Idempotent. |
| `BatchGetDocuments` | served | The read path gRPC SDKs actually use. Transactions and `new_transaction` supported; `read_time` refused. |
| `BeginTransaction` | served | Read-write and read-only. Read-only *at a `read_time`* refused. |
| `Commit` | served | **Atomic.** Transactional commits verify the transaction's read set. |
| `Rollback` | served | Spends the token; rejects an unknown one. |
| `RunQuery` | served | See the query matrix below. Transactions and `new_transaction` supported; `read_time` and `explain_options` refused. |
| `BatchWrite` | served | Deliberately **not** atomic, per the API: one status per write. |
| `ListCollectionIds` | served | Paged by resume token. `read_time` refused. |
| `Listen` | `UNIMPLEMENTED` | Realtime listeners. No change feed exists to drive them. |
| `Write` (streaming) | `UNIMPLEMENTED` | `Commit` is the write path SDKs use. |
| `RunAggregationQuery` | `UNIMPLEMENTED` | `count()`/`sum()`/`average()`. |
| `PartitionQuery` | `UNIMPLEMENTED` | Parallel-scan partitioning, for data-pipeline clients. |
| `ExecutePipeline` | `UNIMPLEMENTED` | Out of scope. |

### Queries

A query is compiled before any document is read, so an unsupported shape fails
immediately rather than after a scan.

| Feature | Status |
|---|---|
| `where()` with `==`, `!=`, `<`, `<=`, `>`, `>=` | served |
| `where()` with `in`, `not-in`, `array-contains`, `array-contains-any` | served |
| `IS_NULL` / `IS_NOT_NULL` / `IS_NAN` / `IS_NOT_NAN` | served |
| `AND` and `OR` composite filters, nested | served |
| `orderBy()`, ascending and descending, several terms | served |
| `limit()`, `offset()` | served |
| `select()` projection, including keys-only (`__name__`) | served |
| `limitToLast()` | served — the SDK flips the order and reverses client-side |
| Filtering and ordering on `__name__` | served |
| Collection group queries (`all_descendants`) | `UNIMPLEMENTED` |
| Cursors (`startAt`/`startAfter`/`endAt`/`endBefore`) | `UNIMPLEMENTED` — use `limit`/`offset` |
| Nested field paths (`a.b`) in filters, orders, projections, and masks | `UNIMPLEMENTED` |
| Vector search (`find_nearest`) | `UNIMPLEMENTED` |
| Aggregations (`count()`, `sum()`, `average()`) | `UNIMPLEMENTED` |

### Query semantics worth knowing

These are the places where a query engine can look like it works and quietly be
wrong. Fireup implements the published Firestore behaviour for each:

- **A document without the filtered field never matches.** Firestore queries are
  index lookups, and a document with no value for a field has no index entry.
- **A document without an ordered field is absent from the results** — not
  sorted to one end.
- **An inequality does not cross types.** `where('age', '>', 5)` does not return
  a document whose `age` is `"old"`, even though strings sort after numbers.
- **`== NaN` never matches.** `IS_NAN` is the only way to select it. In sort
  order, `NaN` comes before every other number.
- **Integers and doubles compare numerically**, so `31` and `31.0` are one value.
- **`__name__` is appended to every `orderBy`** in the direction of the last
  explicit term, so results have a total order instead of depending on the
  order documents came out of the store.

### Transactions

- **Atomic:** every write in a `Commit` applies or none does.
- **Optimistic concurrency:** every document read inside a transaction is
  recorded with its `update_time`. The commit re-checks those in the same store
  transaction as the writes, so a document someone else wrote in the meantime
  fails the commit with `ABORTED` — the status that makes an SDK retry instead
  of losing the update. A document read as *absent* is checked too.
- **No snapshot isolation.** A read inside a transaction sees current state, not
  state as of `BeginTransaction`, so two reads of one document in a transaction
  can differ where production Firestore would repeat the first value. Such a
  transaction cannot *commit* on stale data, so the outcome is a retry rather
  than a wrong write.
- **A query inside a transaction** puts the documents it returned into the read
  set. Documents the query filtered out are not covered, so a concurrent write
  that would have *added* a match does not abort the transaction. Production
  Firestore conflicts on the scanned range.
- **Tokens last 60 seconds**, matching the documented limit, and an expired one
  is `ABORTED`.

### Other known limits

- **A query reads the whole collection.** Filtering, ordering, and limiting
  happen in the server process, not in SQL, so that both stores answer a query
  identically ([`src/local/query.rs`](../../src/local/query.rs) explains the
  trade-off). Fine for local-development volumes; not a production query planner.
- **No field transforms.** `serverTimestamp()`, `increment()`, and array
  union/remove are `UNIMPLEMENTED` rather than silently writing a wrong value.
- **No field deletion through a mask.** A masked field that the request does not
  carry is left alone instead of being deleted.
- **No point-in-time reads.** The store keeps only the current version of a
  document, so any `read_time` consistency selector is refused rather than
  answered with current data.
- **No indexes and no index errors.** Production Firestore rejects some queries
  until a composite index exists; Fireup runs them. A query that works here can
  still need an index in production.
- **No auth and no security rules.** Every request is served. Bind loopback only
  (the default).
- **In-memory unless `--postgres-url` is given**, in which case data survives a
  restart.

The frozen v1 RPC surface — what has to land before we call this
"SDK-compatible CRUD + simple query" — is the `firestore-v1-contract` document on
[SAK-163](/SAK/issues/SAK-163).

## Layout

```
src/local/
  mod.rs             server wiring: protocol multiplexing, spawn(), serve()
  config.rs          LocalConfig and the Service enum
  names.rs           resource-name parsing (projects/*/databases/*/documents/**)
  store.rs           the Store trait, MemoryStore, and the conformance suite
  postgres_store.rs  the durable Store implementation
  query.rs           StructuredQuery compilation and evaluation
  transactions.rs    transaction tokens, read sets, and the conflict check
  value_json.rs      Firestore Value ⇄ JSON, the format of data at rest
  http.rs            GET / and GET /health
  seed.rs            export → store
  firestore.rs       the google.firestore.v1.Firestore service implementation
```

`MemoryStore` uses a `BTreeMap` rather than a hash map because every query shape
Firestore needs (list a collection, scan a subtree) is a key-prefix range scan,
already in the order Firestore sorts `__name__` in.

Writes all funnel through one method, `Store::commit`, which applies a batch
atomically and checks each write's precondition inside the same store
transaction. `CreateDocument`, `UpdateDocument`, `DeleteDocument`, `Commit`, and
`BatchWrite` are all callers of it, so there is no RPC with its own idea of what
a precondition means — and no check-then-write pair for a concurrent writer to
slip into.

### How documents are stored in PostgreSQL

One row per document in `fireup_local_documents`, created on startup (under an
advisory lock, because `CREATE TABLE IF NOT EXISTS` is not race-safe):

| Column | Why |
|---|---|
| `name` | Primary key: the full Firestore resource name. |
| `project_id`, `database_id`, `parent`, `collection_id`, `document_id` | What collection reads and queries will filter and order on. Deriving them from `name` at query time would mean no index could help. |
| `fields` (`jsonb`) | The document's fields in the public Firestore REST value shape: `{"rank": {"integerValue": "7"}}`. Legible to SQL, and usable by the query engine later. |
| `create_time_seconds` / `_nanos`, `update_time_seconds` / `_nanos` | Exact. `timestamptz` only keeps microseconds, and clients compare `update_time` exactly — it is a write precondition — so rounding it would break a precondition that should succeed. |

The table is created in the connection's default schema, so a role whose
`search_path` changes between runs would look at a different table.

`src/local/value_json.rs` owns that encoding; its tests round-trip every value
type an SDK can store, including `int64` past 2^53 (written as a string),
non-finite doubles, raw bytes, and nested maps and arrays.

## Protos

The `.proto` files under `proto/google/` are vendored from the public
[googleapis](https://github.com/googleapis/googleapis) repo (Apache-2.0) at a
pinned commit. Refresh or bump them with:

```bash
scripts/vendor-googleapis-protos.sh                   # refresh at the current pin
GOOGLEAPIS_COMMIT=<sha> scripts/vendor-googleapis-protos.sh   # bump the pin
```

The script resolves imports transitively, so adding a root to its `ROOTS` list
pulls whatever that root needs. `build.rs` compiles messages with `prost-build`
and the Firestore service stubs with `tonic-build`.

**If you bump the pin, update `docs/licenses/FIREBASE_GCP_LOCAL.md`.** That is a
project rule, not a preference.

## Tests

Two layers, because they prove different things:

```bash
# Fast: drives the server over real gRPC using our generated stubs.
cargo test --test local_firestore_tests
cargo test --lib local::

# Durable store. Skips unless a dedicated test database is configured.
FIREUP_TEST_POSTGRES_URL=postgresql://fireup:pw@127.0.0.1:5433/fireup_dev \
  cargo test --test local_postgres_store_tests

# Slow but conclusive: drives it with Google's official client library.
scripts/smoke-local-firestore.sh
FIREUP_SMOKE_POSTGRES_URL=postgresql://fireup:pw@127.0.0.1:5433/fireup_dev \
  scripts/smoke-local-firestore.sh   # …against the durable store
```

`MemoryStore` and `PostgresStore` are held to one contract: the assertions in
`store::conformance` run against both, so a durable backend that disagrees about
`create_time`, merge semantics, batch atomicity, or collection reads fails
immediately rather than showing up as odd SDK behaviour later. The PostgreSQL tests additionally stop the server and start
a new one to prove documents — and every value type — come back unchanged.

Never point `FIREUP_TEST_POSTGRES_URL` at a database or role you do not own; the
shared guard in `tests/postgres_test_env.rs` refuses the host Paperclip role and
database, and the tests skip rather than fall back when no URL is set.

The smoke script builds fireup, starts `fireup local`, runs
`tests/local-smoke/smoke.mjs` against it with `@google-cloud/firestore`, and
tears the server down. It needs network access the first time to `npm install`.

The Rust tests can pass while the SDK test fails — the SDK picks its own RPCs
(`.get()` goes to `BatchGetDocuments`, `.set()` goes to `Commit`, a query goes to
`RunQuery`, `listDocuments()` goes to `ListDocuments` with `show_missing`) and
does its own serialization. Keep both.

Both layers also run in CI, in `.github/workflows/local_firestore.yml`: the Rust
tests, then the SDK smoke test against the in-memory **and** PostgreSQL stores.
Its triggers are commented out for now, matching `integration_tests.yml` — CI in
this repository is manual-only until the repo-wide re-enable, so run it with
`workflow_dispatch`.
