# `fireup local` compared: the Firebase Emulator Suite, and Firestore on GCP

Three things can answer a Firestore SDK on your laptop or in your cloud project.
This is what each one does, and — more usefully — where they disagree.

Read the [fidelity matrix](README.md#fidelity-matrix) first if you only care
about Fireup: it is the complete, checked statement of what `fireup local`
serves and what it refuses. This document is the *comparison*, and it is a
weaker kind of evidence. Here is exactly how weak, per column:

| Column | Where the cells come from |
|---|---|
| **fireup local** | `tests/conformance/matrix.json`, probed row by row through Google's own Node and Python client libraries in CI. Every row below whose label matches a matrix row is compared against it by `cargo test --test fidelity_matrix_tests`, so this column cannot drift from the suite that proves it. A `served` here means an official SDK did it, in CI, against both the in-memory and the PostgreSQL store. |
| **Firebase Emulator Suite** | What [Google's documentation](#sources) states, read 2026-11-06. The Firestore and Datastore emulators ship as closed-source JARs, so per-feature claims here are docs plus the fact that every Firebase SDK is tested against them — not source. |
| **Firestore on GCP** | The production service, as documented. It is the thing the other two are imitating, and the column that decides who is right when they disagree. |

> **Independent implementation, and nothing here was reverse engineered.**
> Fireup implements **public** Google API contracts from public documentation
> and the public googleapis protobuf definitions. It is **not** the Firebase
> Emulator Suite and is **not** affiliated with or endorsed by Google.
> Google's emulator was not decompiled, binary-analysed, or run to write this
> page — the cells come from the documentation linked under
> [Sources](#sources). "Firebase", "Firestore", "Datastore" and "Google Cloud"
> are Google's trademarks and are used here only to name the products being
> compared. See
> [`docs/licenses/FIREBASE_GCP_LOCAL.md`](../licenses/FIREBASE_GCP_LOCAL.md).

So: `not documented` means the vendor's docs do not say, **not** that the
feature is missing. Nothing in the two right-hand columns was verified by
running it. Closing that gap — pointing `scripts/conformance-local.sh` at
Google's emulator and publishing the result — is tracked as follow-up work;
until it is done, treat those columns as a reading of Google's documentation and
nothing more.

Other third-party local-cloud emulators exist and are not compared here yet.
Comparing them properly means running a conformance suite against them rather
than reading their READMEs, which is the same follow-up work.

## At a glance

| | fireup local | Firebase Emulator Suite | Firestore on GCP |
|---|---|---|---|
| What it is | An export **migrator** plus a local data plane over the same PostgreSQL database | Google's local **Firebase** emulators | The production service |
| Services | Firestore, Datastore, Firebase Auth | Firestore, Realtime Database, Cloud Storage, Auth, Functions (beta), Pub/Sub (beta), Extensions (beta) | All of it |
| Runtime | One Rust binary. No JVM, no Node | Node CLI, plus a JRE for Firestore/Datastore — "A Java JRE (version 21 or greater)" | Hosted |
| Ports | `4589` — one port for gRPC, HTTP/1.1 and all three services | `8080` Firestore, `9099` Auth, `4000` UI | n/a |
| Store | In-memory, or **PostgreSQL JSONB** that survives a restart | In-memory: "The Cloud Firestore emulator clears database contents when shut down" — with CLI export/import | Managed |
| The same data as SQL tables | **Yes** — one database, real columns, `psql` | No | No |
| Load a managed Firestore/Datastore export | **Yes** — `fireup import` (to SQL tables) and `fireup local seed` (to the data plane) | No — its import format is the emulator's own export | Managed restore |
| Security Rules | No — every request is served | **Yes**, with a rules playground, evaluation traces and coverage reports | Yes |
| Local UI | No (SQL is the UI) | **Yes**, the Emulator Suite UI | Firebase console |
| Functions triggers | No | **Yes** | Yes |
| Published, machine-checked fidelity claims | **Yes** — 119 rows, each with a probe; CI fails if a documented row stops being true *or* a refusal stops being clear | It is the reference implementation | n/a |
| Licence | Apache-2.0 | Firebase CLI: Apache-2.0. The Firestore/Datastore emulators are **closed-source** JARs the CLI downloads | Proprietary service |

The short version: **Fireup puts the data in PostgreSQL where SQL can read it,
and imports a managed export. Google's emulator serves Security Rules and
Functions triggers, which Fireup does not, and is the reference implementation
for everything else.** They are complementary, and Fireup's default port is
chosen so they can run side by side.

## Firestore RPCs

`google.firestore.v1.Firestore` has 17 RPCs. Fireup's statuses are the
[fidelity matrix](README.md#fidelity-matrix).

| RPC | fireup local | Firebase Emulator Suite | Firestore on GCP |
|---|---|---|---|
| `GetDocument` | served | yes | yes |
| `ListDocuments` | served | yes | yes |
| `CreateDocument` | served | yes | yes |
| `UpdateDocument` | served | yes | yes |
| `DeleteDocument` | served | yes | yes |
| `BatchGetDocuments` | served | yes | yes |
| `BeginTransaction` | served | yes | yes |
| `Commit` | served | yes | yes |
| `Rollback` | served | yes | yes |
| `RunQuery` | served | yes | yes |
| `BatchWrite` | served | yes | yes |
| `ListCollectionIds` | served | yes | yes |
| `Listen` | served | yes | yes |
| `Write (streaming)` | `UNIMPLEMENTED` | yes | yes |
| `RunAggregationQuery` | `UNIMPLEMENTED` | yes | yes |
| `PartitionQuery` | `UNIMPLEMENTED` | not documented | yes |
| `ExecutePipeline` | `UNIMPLEMENTED` | not documented | preview |

Fireup refuses four of the 17. Google's emulator serves two of those four —
streaming `Write` and `RunAggregationQuery` — so if you need
`count()`/`sum()`/`average()` locally today, that is the honest answer: Google's
emulator, not Fireup. Fireup's refusals are an explicit `UNIMPLEMENTED` whose
message names the gap, which is a different promise from serving them, and a
weaker one.

### Realtime listeners, in detail

Fireup serves `Listen` without a change history behind it, and says so. The
differences from production are in what the stream does once it is open.

| Listener feature | fireup local | Firebase Emulator Suite | Firestore on GCP |
|---|---|---|---|
| `onSnapshot()` on a document or a query | served | yes | yes |
| Query targets with the full query shape (cursors, `orderBy`, `limit`) | served | yes | yes |
| Collection-group query targets | served | yes | yes |
| `Target.once` | served | yes | yes |
| `NO_CHANGE` heartbeats | sent | sent | sent |
| `ExistenceFilter` / `expected_count` | not sent; the client gets full changes | sent | sent |
| `Target.resume_token` | ignored; the target re-snapshots | resume supported | resume supported |
| `Target.read_time` (resume at a point in time) | `UNIMPLEMENTED` | supported | supported |
| A change written by *another* process to the same PostgreSQL database | **not seen** — the feed is in-process, nothing polls the table | n/a (single process) | seen |

That last row is Fireup's sharpest listener limit and it is a consequence of the
thing that makes Fireup useful: because the store is a real database other
clients can write to, a listener can miss a write that `psql` made. A local SDK
test is a single process, which is the case this is complete for.

## Queries

| Feature | fireup local | Firebase Emulator Suite | Firestore on GCP |
|---|---|---|---|
| `where()` with `==`, `!=`, `<`, `<=`, `>`, `>=` | served | yes | yes |
| `where()` with `in`, `not-in`, `array-contains`, `array-contains-any` | served | yes | yes |
| `IS_NULL` / `IS_NOT_NULL` / `IS_NAN` / `IS_NOT_NAN` | served | yes | yes |
| `AND` and `OR` composite filters, nested | served | yes | yes |
| `orderBy()`, ascending and descending, several terms | served | yes | yes |
| `limit()`, `offset()` | served | yes | yes |
| `select()` projection, including keys-only (`__name__`) | served | yes | yes |
| `limitToLast()` | served | yes | yes |
| Filtering and ordering on `__name__` | served | yes | yes |
| Collection group queries (`all_descendants`) | served | yes | yes |
| Cursors (`startAt`/`startAfter`/`endAt`/`endBefore`) | served | yes | yes |
| Nested field paths (`a.b`) in filters, orders, projections, and read masks | served | yes | yes |
| A top-level `update_mask` path the write does not carry | served — the field is deleted, as production does | yes | yes |
| Nested field paths in an `update_mask` | `UNIMPLEMENTED` | yes | yes |
| A collection group query with no collection id | `UNIMPLEMENTED` | yes | yes |
| Vector search (`find_nearest`) | `UNIMPLEMENTED` | not documented | yes |
| Aggregations (`count()`, `sum()`, `average()`) | `UNIMPLEMENTED` | yes | yes |

## Field transforms

| Transform | fireup local | Firebase Emulator Suite | Firestore on GCP |
|---|---|---|---|
| `set_to_server_value: REQUEST_TIME` | served | yes | yes |
| `increment` | served | yes | yes |
| `append_missing_elements` | served | yes | yes |
| `remove_all_from_array` | served | yes | yes |
| `maximum / minimum` | `UNIMPLEMENTED` | yes | yes |
| Nested field path (`a.b`) on a transform | `UNIMPLEMENTED` | yes | yes |

## Where the local implementations diverge from production

This is the part a feature count hides. Both local implementations differ from
GCP somewhere; what matters is whether the difference is written down.

| Behaviour | fireup local | Firebase Emulator Suite | Firestore on GCP |
|---|---|---|---|
| Composite index enforcement | no — a query that works here can still need an index in production | no — "does not track compound indexes and instead will execute any valid query" | **yes** — a query without its index is rejected |
| Production limit enforcement (e.g. transaction size) | no | no — "may allow transactions that would be rejected as too large" | yes |
| `__name__` is appended to every `orderBy`, in the last term's direction | served | yes | yes |
| A document without the ordered field is absent from the results | served | yes | yes |
| `ListDocuments` ordering and paging | served | yes | yes |
| Every write in a `Commit` applies or none does | served | yes | yes |
| A concurrent write to a read document aborts the commit | served | yes | yes |
| Phantom reads abort a transaction | no — a query's read set is the documents it returned | "does not currently implement all transaction behavior seen in production"; locks "may take up to 30 seconds to be released" | yes — production conflicts on the scanned range |
| Transaction lifetime | 60s, matching the documented limit | not documented | documented server deadlines |
| Point-in-time reads (any `read_time` consistency selector) | `UNIMPLEMENTED` | not documented | yes |

Publishing its divergences is the thing to actually look for in a local
emulator, and the thing to look for after that is what happens to them over
time: Fireup's are rows in `matrix.json` with probes attached, so a divergence
that silently changes fails CI.

## Datastore

| RPC | fireup local | Firebase Emulator Suite | Datastore on GCP |
|---|---|---|---|
| `Lookup` | served | not emulated | yes |
| `RunQuery` | served | not emulated | yes |
| `Commit` | served | not emulated | yes |
| `BeginTransaction / Rollback` | served | not emulated | yes |
| `AllocateIds` | served | not emulated | yes |
| `ReserveIds` | served | not emulated | yes |
| `RunAggregationQuery` | `UNIMPLEMENTED` | not emulated | yes |

Fireup's extra property: Datastore kinds and Firestore collections are **the
same rows**, so an entity written through the Datastore API is readable through
the Firestore API and as a SQL row. Non-default namespaces, GQL, projection
queries, and cursors are refused.

The Firebase Emulator Suite does not emulate the Datastore API at all. Google's
separate `gcloud` Datastore emulator does, and its own documented limits are
sharp in places Fireup's are not: it "does not support `IN`, `!=`, and `NOT-IN`
queries" and "does not support aggregation queries like `COUNT(*)`", and it
simulates eventual consistency by default.

### Datastore property masks

`Mutation.property_mask` — a masked entity write — is served per the proto. The
Firebase Emulator Suite does not emulate Datastore at all, so production is the
only column here with anything to say:

| Mask shape | fireup local | Firebase Emulator Suite | Datastore on GCP |
|---|---|---|---|
| A path the entity carries | served | not emulated | yes |
| A path the entity omits | served — deleted, per the proto | not emulated | yes |
| A property the mask does not name | served — left untouched | not emulated | yes |
| `__key__` | served — accepted and ignored | not emulated | yes |
| A property path (`foo.bar`) or an escaped name | `UNIMPLEMENTED` | not emulated | yes |
| An empty path | served | not emulated | yes |
| Any other reserved (`__`-wrapped) name | served | not emulated | yes |

## Firebase Auth / Identity Toolkit

| Feature | fireup local | Firebase Auth emulator | Identity Platform on GCP |
|---|---|---|---|
| Email/password, anonymous, custom-token sign-in | served | yes | yes |
| Admin account CRUD, list | served | yes | yes |
| Token refresh (`/v1/token`) and revocation | served | yes | yes |
| Unsigned ID tokens the Admin SDK accepts | served | yes | n/a — real signatures |
| Out-of-band email codes (password reset, verify email) | served | yes | yes |
| Email-link sign-in | served | yes | yes |
| Sending email: the codes are real, the delivery is not | documented-divergence — read the link from `GET /emulator/v1/projects/{p}/oobCodes`; no SMTP | same model: no mail is sent | n/a — real email |
| Phone auth: `sendVerificationCode`, `signInWithPhoneNumber` | `UNIMPLEMENTED` | yes, with codes readable over REST | yes |
| Federated / OAuth providers: `signInWithIdp`, `createAuthUri` | `UNIMPLEMENTED` | yes, for OpenID Connect providers | yes |
| Multi-factor enrolment and tenants (Identity Platform v2) | `UNIMPLEMENTED` | SMS MFA yes; tenants no | yes |
| Session cookies: `createSessionCookie`, `verifySessionCookie` | `UNIMPLEMENTED` | yes | yes |
| Batch account writes: `accounts:batchDelete`, `accounts:batchCreate` | `UNIMPLEMENTED` | yes | yes |

Auth is the surface where Fireup is closest to Google's emulator: the
out-of-band email flows (`PASSWORD_RESET`, `VERIFY_EMAIL`, `EMAIL_SIGNIN`,
`VERIFY_AND_CHANGE_EMAIL`) and email-link sign-in are served here too. Neither
sends mail; both hand you the code. Phone, federated providers, MFA, tenants
and session cookies are the gap.

## How to choose

- **Migrating off Firestore, or want the same data in SQL** → Fireup. It is the
  only one of the three that does this, and the reason the project exists.
- **Realtime listeners** → either. Fireup serves `Listen`, without resume
  tokens or point-in-time resume.
- **Aggregations, Security Rules, or Functions triggers** → Google's emulator.
  Fireup refuses all three, explicitly.
- **Fidelity you can audit** → Fireup publishes its matrix and the suite that
  proves it. Use it to find out what will break *before* you move.
- They coexist: `4589` and `8080`/`9099` do not collide, and Fireup can serve a
  subset (`--enable firestore`) beside another emulator doing the rest.

## Sources

Read 2026-11-06. Public documentation only — no proprietary binary was
inspected, decompiled, or observed to produce this comparison; see
[`docs/licenses/FIREBASE_GCP_LOCAL.md`](../licenses/FIREBASE_GCP_LOCAL.md).

| Column | Source |
|---|---|
| fireup local | [`docs/local/README.md`](README.md#fidelity-matrix) and `tests/conformance/matrix.json`, in this repository |
| Firebase Emulator Suite | [Emulator Suite overview](https://firebase.google.com/docs/emulator-suite), [Firestore emulator](https://firebase.google.com/docs/emulator-suite/connect_firestore), [Auth emulator](https://firebase.google.com/docs/emulator-suite/connect_auth) |
| Datastore emulator | [Datastore emulator](https://cloud.google.com/datastore/docs/tools/datastore-emulator) |
| Firestore on GCP | [Firestore documentation](https://firebase.google.com/docs/firestore) |

Quoted strings in the tables above are verbatim from those pages. Everything
else is a summary, and a summary of someone else's documentation is exactly as
reliable as their documentation.
