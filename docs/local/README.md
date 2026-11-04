# `fireup local` — local Firestore-compatible data plane

Fireup serves Google's **published** `google.firestore.v1` gRPC API on one local
port so official Firestore client SDKs can point at it with the documented
`FIRESTORE_EMULATOR_HOST` environment variable.

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

## What Phase 1 serves

| RPC | Status |
|---|---|
| `GetDocument` | served (default consistency only) |
| `CreateDocument` | served, incl. 20-char auto ids |
| `UpdateDocument` | served, with `update_mask` merge |
| `DeleteDocument` | served, idempotent |
| `BatchGetDocuments` | served — this is the read path gRPC SDKs actually use |
| `Commit` | served for non-transactional writes |
| everything else | `UNIMPLEMENTED` with a message naming the gap |

Known limits, all deliberate and all visible to a client:

- **In-memory unless `--postgres-url` is given.** Without it, data is lost on
  exit. With it, data survives a restart.
- **No queries.** `RunQuery`, `RunAggregationQuery`, `ListDocuments`,
  `ListCollectionIds` return `UNIMPLEMENTED`.
- **No transactions.** `BeginTransaction`/`Rollback`, and any read or commit
  carrying a transaction token, return `UNIMPLEMENTED`.
- **No realtime listeners.** `Listen` and the streaming `Write` RPC return
  `UNIMPLEMENTED`.
- **No field transforms.** `serverTimestamp()`, `increment()`, array
  union/remove return `UNIMPLEMENTED` rather than silently writing a wrong value.
- **No nested field paths in masks.** `a.b` and `` `a&b` `` are rejected with
  `UNIMPLEMENTED`; top-level names work.
- **Commit is not atomic.** A write that fails mid-batch leaves earlier writes in
  the batch applied.
- **No auth and no security rules.** Every request is served. Bind loopback only
  (the default).

Unserved RPCs answer `UNIMPLEMENTED` with a message that names what is missing,
so an SDK failure reads as a clear gap instead of a hang or a protocol error.

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
  value_json.rs      Firestore Value ⇄ JSON, the format of data at rest
  http.rs            GET / and GET /health
  seed.rs            export → store
  firestore.rs       the google.firestore.v1.Firestore service implementation
```

`MemoryStore` uses a `BTreeMap` rather than a hash map because every query shape
Firestore needs next (list a collection, scan a subtree) is a key-prefix range
scan.

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
`create_time` or merge semantics fails immediately rather than showing up as odd
SDK behaviour later. The PostgreSQL tests additionally stop the server and start
a new one to prove documents — and every value type — come back unchanged.

Never point `FIREUP_TEST_POSTGRES_URL` at a database or role you do not own; the
shared guard in `tests/postgres_test_env.rs` refuses the host Paperclip role and
database, and the tests skip rather than fall back when no URL is set.

The smoke script builds fireup, starts `fireup local`, runs
`tests/local-smoke/smoke.mjs` against it with `@google-cloud/firestore`, and
tears the server down. It needs network access the first time to `npm install`.

The Rust tests can pass while the SDK test fails — the SDK picks its own RPCs
(`.get()` goes to `BatchGetDocuments`, `.set()` goes to `Commit`) and does its own
serialization. Keep both.
