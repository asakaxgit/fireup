# `fireup local` — local Firestore + Firebase Auth data plane

Fireup serves two of Google's **published** API contracts on one local port:

- `google.firestore.v1` over gRPC, for official Firestore client SDKs
  (`FIRESTORE_EMULATOR_HOST`) — document CRUD, `StructuredQuery` reads,
  collection listings, and transactions;
- a subset of the Identity Toolkit REST API, for official Firebase Auth client
  and Admin SDKs (`FIREBASE_AUTH_EMULATOR_HOST`).

Both are reached through the documented emulator-host environment variables, so
pointing an app at Fireup needs no code change. The [fidelity
matrix](#fidelity-matrix) says exactly what is served and what is refused.

This is an independent implementation of public API contracts. It is **not** the
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
export FIREBASE_AUTH_EMULATOR_HOST=127.0.0.1:4589   # same port
```

Any official SDK (Node, Python, Go, Java, Admin SDKs) honours those variables and
will talk plaintext to Fireup instead of production.

### Flags

| Flag | Default | What it does |
|---|---|---|
| `--host` | `127.0.0.1` | Bind address. Loopback by default: there is no auth. |
| `--port` | `4589` | The one port. gRPC and HTTP share it. |
| `--postgres-url` | *(unset)* | Store documents **and accounts** in PostgreSQL instead of memory. |
| `--project-id` | `fireup-local` | Project the data plane answers for. |
| `--database-id` | `(default)` | Firestore's default database is literally `(default)`. |
| `--enable` | `firestore`, `auth` | Services to serve. Repeatable; closed set, so a typo is an error. |
| `--max-connections` | `5` | PostgreSQL pool size. |

Both services are on by default. `--enable firestore` alone serves no auth, and
a request to a disabled service says so rather than answering a bare `404`.

An unreachable `--postgres-url` is a **hard error**, not a fallback to memory:
silently serving an empty in-memory store would be indistinguishable from data
loss.

### One port, two protocols

gRPC clients speak HTTP/2; `curl`, humans, container health checks, and the
Firebase Auth SDKs speak HTTP/1.1 to the same port. The server tells them apart
by the HTTP/2 connection preface.

```bash
curl http://127.0.0.1:4589/          # service discovery
curl http://127.0.0.1:4589/health    # health; asks the store, so it fails when the DB is gone
```

`/` and `/health` are read-only — anything but `GET`/`HEAD` on them answers
`405`. The auth endpoints under `/identitytoolkit.googleapis.com/`,
`/securetoken.googleapis.com/` and `/emulator/v1/` take `POST`, `GET` and
`DELETE` as the published contract specifies.

Request bodies must carry `Content-Length`; `Transfer-Encoding: chunked` is
refused with a `400` naming what to send instead, rather than being mis-read as
JSON.

## Docker Compose

`docker-compose.yml` runs `fireup-local` beside the existing Postgres service:

```bash
docker compose up -d postgres fireup-local
curl http://127.0.0.1:4589/health
export FIRESTORE_EMULATOR_HOST=127.0.0.1:4589
export FIREBASE_AUTH_EMULATOR_HOST=127.0.0.1:4589
```

The container stores documents and accounts in the Compose Postgres, so
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
- **No auth and no security rules.** Every request is served; an ID token from
  the auth service is not required, checked, or consulted by the Firestore data
  plane. The two services share a port and a project id, not an authorization
  model. Bind loopback only (the default).
- **In-memory unless `--postgres-url` is given**, in which case data survives a
  restart.

## Firebase Auth / Identity Platform

> **Local development only.** The ID tokens this serves are **unsigned**, no API
> key or admin credential is checked, and passwords are hashed without a work
> factor. Anything that can reach the port can act as any account. The default
> bind is loopback; keep it that way, and never point anything holding real
> credentials at it.

```bash
export FIREBASE_AUTH_EMULATOR_HOST=127.0.0.1:4589
```

Official SDKs read that variable and address Identity Toolkit with the API host
as the first path segment, so the live routes are
`/identitytoolkit.googleapis.com/v1/accounts:signUp` and friends. The bare
`/v1/...` form is accepted too, which is what makes the `curl` examples below
work.

### What is served

| Endpoint | Purpose |
|---|---|
| `POST /v1/accounts:signUp` | email+password sign-up; **anonymous** sign-in when neither is given |
| `POST /v1/accounts:signInWithPassword` | password sign-in |
| `POST /v1/accounts:signInWithCustomToken` | sign in with an Admin SDK custom token; creates the uid if new |
| `POST /v1/accounts:lookup` | read the signed-in account (`idToken`) |
| `POST /v1/accounts:update` | change own profile, email, or password |
| `POST /v1/accounts:delete` | delete own account |
| `POST /v1/token` | exchange a refresh token for a new ID token (`securetoken` host) |
| `POST /v1/projects/{p}/accounts` | admin: create an account, optionally with a chosen uid and claims |
| `POST /v1/projects/{p}/accounts:lookup` | admin: read accounts by uid or email |
| `POST /v1/projects/{p}/accounts:update` | admin: change any account, including `disableUser` and `customAttributes` |
| `POST /v1/projects/{p}/accounts:delete` | admin: delete an account |
| `GET /v1/projects/{p}/accounts:batchGet` | admin: list accounts, paged by `maxResults`/`nextPageToken` |
| `GET /emulator/v1/projects/{p}/config` | the local config an SDK may probe |
| `DELETE /emulator/v1/projects/{p}/accounts` | clear every account |

Verified against the official SDKs, not just against our own idea of the
contract: `scripts/smoke-local-auth.sh` drives this surface with
`firebase-admin` and the `firebase` client SDK over
`FIREBASE_AUTH_EMULATOR_HOST`. In Admin SDK terms, that covers `createUser`,
`getUser`, `getUserByEmail`, `updateUser`, `setCustomUserClaims`, `listUsers`
(paged), `deleteUser`, `createCustomToken`, and `verifyIdToken`; and on the
client, `signInWithEmailAndPassword`, `signInWithCustomToken`,
`signInAnonymously`, and token refresh. See [Tests](#tests).

```bash
# Sign up, then sign in.
curl -s localhost:4589/v1/accounts:signUp \
  -H 'content-type: application/json' \
  -d '{"email":"ada@example.com","password":"secret12","returnSecureToken":true}'

curl -s localhost:4589/v1/accounts:signInWithPassword \
  -H 'content-type: application/json' \
  -d '{"email":"ada@example.com","password":"secret12"}'

# Admin: create a user with a chosen uid and a custom claim.
curl -s localhost:4589/v1/projects/fireup-local/accounts \
  -H 'content-type: application/json' \
  -d '{"localId":"u1","email":"u1@example.com","password":"secret12","customAttributes":"{\"role\":\"editor\"}"}'

# Admin: list users, and wipe them between test runs.
curl -s 'localhost:4589/v1/projects/fireup-local/accounts:batchGet?maxResults=50'
curl -s -X DELETE localhost:4589/emulator/v1/projects/fireup-local/accounts
```

### Non-goals

These answer `501` with a message naming the gap, not a vague `404`:

- **Phone auth** — `sendVerificationCode`, `signInWithPhoneNumber`.
- **Multi-factor enrolment and tenants** — the whole Identity Platform `v2`
  surface.
- **Federated / OAuth providers** — `signInWithIdp`, `createAuthUri`.
- **Out-of-band email flows** — `sendOobCode`, `resetPassword`. There is no
  mail transport here, and a link that goes nowhere is worse than a clear error.
  In Admin SDK terms this is `generateEmailVerificationLink`,
  `generatePasswordResetLink`, and friends.
- **Batch account writes** — `accounts:batchDelete` and `accounts:batchCreate`,
  so the Admin SDK's `deleteUsers()` and `importUsers()` are refused. Delete and
  create one at a time instead. Not a hard problem, just not done; the refusal
  names the gap and leaves every account untouched.
- **Session cookies** — `createSessionCookie` / `verifySessionCookie`. These are
  a separate token format from ID tokens, and nothing here needs them yet.
- **Production key material.** Nothing generates or ships a signing key.

`scripts/smoke-local-auth.sh` asserts that the out-of-band, batch-write, and
session-cookie gaps *fail cleanly* — a mapped SDK error rather than a 500, a
hang, or a half-applied write — so a gap stays a gap rather than becoming a
surprise. If one of them is implemented later, that check fails and says so.

### Tokens

ID tokens are JWTs with header `{"alg":"none","typ":"JWT"}`, the **published**
claim set (`iss`, `aud`, `sub`, `user_id`, `auth_time`, `iat`, `exp`, `email`,
`email_verified`, `firebase.identities`, `firebase.sign_in_provider`, plus custom
claims at the top level), and an empty signature. That is the shape official
open-source SDKs accept when `FIREBASE_AUTH_EMULATOR_HOST` is set: they verify
claims and skip the signature.

A local server has no Google private key, and inventing one would mean shipping
production-shaped key material — a non-goal. So tokens are unsigned, and the
trust boundary is the loopback bind.

Refresh tokens are **Fireup's own** opaque encoding (clients treat them as
opaque by contract, so there is nothing to imitate). Revocation works through
`validSince`: changing an email or password, or an admin setting `validSince`,
invalidates refresh tokens issued earlier, and `/v1/token` then answers
`TOKEN_EXPIRED`.

#### Session claims vs account claims

Two things that look alike are deliberately kept apart:

- **Account claims** — what `setCustomUserClaims` (`accounts:update`) stores on
  the account. They appear in every ID token the account gets, and only an admin
  call changes them.
- **Session claims** — the `claims` an Admin SDK puts in a custom token via
  `createCustomToken(uid, claims)`. They apply to the session that token opens,
  layered over the account's claims, and are **not** written to the account.

So signing in with a custom token does not edit the account, and
`firebase.sign_in_provider` reports how *that session* authenticated (`custom`)
rather than how the account was originally created. Both facts ride on the
refresh token, so a refresh re-issues the same session instead of silently
weakening it an hour in.

### Accounts are not documents

Accounts live in their own store, never as Firestore documents. Keeping them in
the document store would put them in `GET /health`'s document count, in a
`fireup local seed`, and — worst — make them readable and writable through the
Firestore API by any client that guessed the path. `tests/local_auth_tests.rs`
asserts that signing a user up leaves the document count at zero.

Passwords are stored as a per-account salt and a single SHA-256 digest. That is
**not** a password-storage scheme for anything real — no work factor, so it does
not resist offline guessing. It exists so a shared dev database is not full of
plaintext passwords.

### How accounts are stored in PostgreSQL

One row per account in `fireup_local_auth_users`, created on startup under its
own advisory lock:

| Constraint | Why |
|---|---|
| `PRIMARY KEY (project_id, local_id)` | A uid is unique within a project, and the same uid may exist in two projects. |
| `UNIQUE (project_id, email) WHERE email IS NOT NULL` | One account per email, while any number of **anonymous** accounts share a null email. A non-partial index would break the second anonymous sign-in. |

Both constraints are enforced by the database rather than by a read-then-write,
which two concurrent sign-ups could both win. `EMAIL_EXISTS` and
`DUPLICATE_LOCAL_ID` are the constraint violations translated back into the codes
SDKs match on.

Timestamps are epoch-millisecond `bigint`s, because they are echoed back to
clients verbatim and a round trip through a type with different precision would
change them.

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
  http.rs            the HTTP/1.1 surface: GET /, GET /health, and auth routing
  seed.rs            export → store
  firestore.rs       the google.firestore.v1.Firestore service implementation
  auth/
    mod.rs             what the auth surface serves, and what it deliberately does not
    api.rs             the Identity Toolkit REST handlers (no socket work)
    token.rs           ID / custom / refresh tokens
    user.rs            the AuthUser record and password hashing
    store.rs           the UserStore trait, MemoryUserStore, and its conformance suite
    postgres_store.rs  the durable UserStore implementation
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
# Fast: drives the server over real gRPC and real HTTP/1.1.
cargo test --test local_firestore_tests
cargo test --test local_auth_tests
cargo test --lib local::

# Durable stores. Skip unless a dedicated test database is configured.
FIREUP_TEST_POSTGRES_URL=postgresql://fireup:pw@127.0.0.1:5433/fireup_dev \
  cargo test --test local_postgres_store_tests --test local_auth_postgres_tests

# Slow but conclusive: drives it with Google's official client libraries.
scripts/smoke-local-firestore.sh   # @google-cloud/firestore
scripts/smoke-local-auth.sh        # firebase-admin + the firebase client SDK
FIREUP_SMOKE_POSTGRES_URL=postgresql://fireup:pw@127.0.0.1:5433/fireup_dev \
  scripts/smoke-local-firestore.sh   # …against the durable store
FIREUP_SMOKE_POSTGRES_URL=postgresql://fireup:pw@127.0.0.1:5433/fireup_dev \
  scripts/smoke-local-auth.sh        # …same, for accounts
```

Both smoke scripts build the binary, start the server on a free port, run the
Node test, and stop the server. They need network access the first time, to
`npm install` into `tests/local-smoke/`.

Each store trait has one contract that both of its implementations must satisfy:
`store::conformance` for documents and `auth::store::conformance` for accounts.
Those assertions run against the in-memory and the PostgreSQL implementation
alike, so a durable backend that disagrees about `create_time`, merge semantics,
batch atomicity, collection reads, or email uniqueness fails immediately rather
than showing up as odd SDK behaviour later. The PostgreSQL tests additionally
stop the server and start a new one, to prove documents (and every value type)
and accounts (and their password digests) come back unchanged.

`tests/local_auth_tests.rs` writes its HTTP requests by hand rather than using a
client crate, because the thing under test includes Fireup's own request
parsing: `Content-Length` handling, a body split across packets, a body that
never arrives, and a chunked body that must be refused.

The smoke tests are the layer that catches what our own tests cannot, because
they are not written against our idea of the contract. `tests/local-smoke/`
hands the official SDKs nothing but an emulator host and lets them choose every
URL, header, body shape, and error mapping. Two real divergences in the auth
surface were found exactly this way and are now fixed and regression-tested:
custom-token sign-in overwrote the account's stored custom claims, and it
reported the account's provider instead of `custom`. A passing count in our own
test suite would not have found either.

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
