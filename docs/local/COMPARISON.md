# `fireup local` compared: floci-gcp, the Firebase Emulator Suite, and Firestore on GCP

Four things can answer a Firestore SDK on your laptop. This is what each one
does, and — more usefully — where they disagree.

Read the [fidelity matrix](README.md#fidelity-matrix) first if you only care
about Fireup: it is the complete, checked statement of what `fireup local`
serves and what it refuses. This document is the *comparison*, and it is a
weaker kind of evidence. Here is exactly how weak, per column:

| Column | Where the cells come from |
|---|---|
| **fireup local** | `tests/conformance/matrix.json`, probed row by row through Google's own Node and Python client libraries in CI. Every row below whose label matches a matrix row is compared against it by `cargo test --test fidelity_matrix_tests`, so this column cannot drift from the suite that proves it. A `served` here means an official SDK did it, in CI, against both the in-memory and the PostgreSQL store. |
| **floci-gcp** | What [floci-gcp's own documentation](#sources) claims, read 2026-11-06. **We have not run a conformance suite against it.** |
| **Firebase Emulator Suite** | What [Google's documentation](#sources) states, read 2026-11-06. The Firestore and Datastore emulators ship as closed-source JARs, so per-feature claims here are docs plus the fact that every Firebase SDK is tested against them — not source. |
| **Firestore on GCP** | The production service, as documented. It is the thing the other three are imitating, and the column that decides who is right when two of them disagree. |

> **Independent implementation, and nothing here was reverse engineered.**
> Fireup implements **public** Google API contracts from public documentation
> and the public googleapis protobuf definitions. It is **not** the Firebase
> Emulator Suite and is **not** affiliated with or endorsed by Google.
> Neither Google's emulator nor floci-gcp was decompiled, binary-analysed, or
> run to write this page — the cells come from the documentation linked under
> [Sources](#sources). "Firebase", "Firestore", "Datastore" and "Google Cloud"
> are Google's trademarks and are used here only to name the products being
> compared; "floci-gcp" is its authors'. See
> [`docs/licenses/FIREBASE_GCP_LOCAL.md`](../licenses/FIREBASE_GCP_LOCAL.md).

So: `not documented` means the vendor's docs do not say, **not** that the
feature is missing. Nothing in the two middle columns was verified by running
it. Closing that gap — pointing `scripts/conformance-local.sh` at floci-gcp and
at Google's emulator and publishing the result — is tracked as follow-up work;
until it is done, treat those columns as a reading of their README and nothing
more.

## At a glance

| | fireup local | floci-gcp | Firebase Emulator Suite | Firestore on GCP |
|---|---|---|---|---|
| What it is | An export **migrator** plus a local data plane over the same PostgreSQL database | A local **GCP emulator suite**, 20+ services | Google's local **Firebase** emulators | The production service |
| Services | Firestore, Datastore, Firebase Auth | Firestore, Datastore, Firebase Auth, Pub/Sub, GCS, BigQuery, KMS, IAM, Cloud Run, Cloud SQL, … | Firestore, Realtime Database, Cloud Storage, Auth, Functions (beta), Pub/Sub (beta), Extensions (beta) | All of it |
| Runtime | One Rust binary. No JVM, no Node | Java (native binaries and a container image) | Node CLI, plus a JRE for Firestore/Datastore — "A Java JRE (version 21 or greater)" | Hosted |
| Ports | `4589` — one port for gRPC, HTTP/1.1 and all three services | `4588` — one port | `8080` Firestore, `9099` Auth, `4000` UI | n/a |
| Store | In-memory, or **PostgreSQL JSONB** that survives a restart | `memory`, `persistent`, `hybrid`, `wal` | In-memory: "The Cloud Firestore emulator clears database contents when shut down" — with CLI export/import | Managed |
| The same data as SQL tables | **Yes** — one database, real columns, `psql` | No | No | No |
| Load a managed Firestore/Datastore export | **Yes** — `fireup import` (to SQL tables) and `fireup local seed` (to the data plane) | Not documented | No — its import format is the emulator's own export | Managed restore |
| Security Rules | No — every request is served | Not documented | **Yes**, with a rules playground, evaluation traces and coverage reports | Yes |
| Local UI | No (SQL is the UI) | `floci-ui`, a separate project | **Yes**, the Emulator Suite UI | Firebase console |
| Functions triggers | No | Cloud Functions service | **Yes** | Yes |
| Published, machine-checked fidelity claims | **Yes** — 119 rows, each with a probe; CI fails if a documented row stops being true *or* a refusal stops being clear | Deviations documented in prose; no published suite | It is the reference implementation | n/a |
| Licence | Apache-2.0 | MIT | Firebase CLI: Apache-2.0. The Firestore/Datastore emulators are **closed-source** JARs the CLI downloads | Proprietary service |

The short version: **floci-gcp is wider and serves more of the Firestore API
than Fireup does. Fireup is the only one of the four that puts the data in
PostgreSQL where SQL can read it, and the only one that imports a managed
export.** They are complementary, which is why Fireup's default port is one
digit away from floci-gcp's rather than on top of it.

## Firestore RPCs

`google.firestore.v1.Firestore` has 17 RPCs. Fireup's statuses are the
[fidelity matrix](README.md#fidelity-matrix); floci-gcp's are its documented
"Supported Operations" list.

| RPC | fireup local | floci-gcp | Firebase Emulator Suite | Firestore on GCP |
|---|---|---|---|---|
| `GetDocument` | served | yes | yes | yes |
| `ListDocuments` | served | yes, with divergences | yes | yes |
| `CreateDocument` | served | yes | yes | yes |
| `UpdateDocument` | served | yes | yes | yes |
| `DeleteDocument` | served | yes | yes | yes |
| `BatchGetDocuments` | served | yes | yes | yes |
| `BeginTransaction` | served | yes | yes | yes |
| `Commit` | served | yes | yes | yes |
| `Rollback` | served | yes, no-op | yes | yes |
| `RunQuery` | served | yes | yes | yes |
| `BatchWrite` | served | yes | yes | yes |
| `ListCollectionIds` | served | yes | yes | yes |
| `Listen` | served | yes | yes | yes |
| `Write (streaming)` | `UNIMPLEMENTED` | yes | yes | yes |
| `RunAggregationQuery` | `UNIMPLEMENTED` | yes | yes | yes |
| `PartitionQuery` | `UNIMPLEMENTED` | yes | not documented | yes |
| `ExecutePipeline` | `UNIMPLEMENTED` | not documented | not documented | preview |

Three of Fireup's four refusals are things floci-gcp serves. If you need
`count()`/`sum()`/`average()` or `PartitionQuery` locally today, that is the
honest answer: floci-gcp or Google's emulator, not Fireup. Fireup's refusals are
an explicit `UNIMPLEMENTED` whose message names the gap, which is a different
promise from serving them, and a weaker one.

### Realtime listeners, in detail

Both local implementations serve `Listen` without a change history behind it,
and both say so. The differences are in what the stream does once it is open.

| Listener feature | fireup local | floci-gcp | Firebase Emulator Suite | Firestore on GCP |
|---|---|---|---|---|
| `onSnapshot()` on a document or a query | served | yes | yes | yes |
| Query targets with the full query shape (cursors, `orderBy`, `limit`) | served | not documented | yes | yes |
| Collection-group query targets | served | not documented | yes | yes |
| `Target.once` | served | **ignored** | yes | yes |
| `NO_CHANGE` heartbeats | sent | **never sent** | sent | sent |
| `ExistenceFilter` / `expected_count` | not sent; the client gets full changes | never sent | sent | sent |
| `Target.resume_token` | ignored; the target re-snapshots | opaque, round-trips only | resume supported | resume supported |
| `Target.read_time` (resume at a point in time) | `UNIMPLEMENTED` | `RESET` plus full state | supported | supported |
| A change written by *another* process to the same PostgreSQL database | **not seen** — the feed is in-process, nothing polls the table | n/a | n/a (single process) | seen |

That last row is Fireup's sharpest listener limit and it is a consequence of the
thing that makes Fireup useful: because the store is a real database other
clients can write to, a listener can miss a write that `psql` made. A local SDK
test is a single process, which is the case this is complete for.

## Queries

| Feature | fireup local | floci-gcp | Firebase Emulator Suite | Firestore on GCP |
|---|---|---|---|---|
| `where()` with `==`, `!=`, `<`, `<=`, `>`, `>=` | served | yes | yes | yes |
| `where()` with `in`, `not-in`, `array-contains`, `array-contains-any` | served | not documented | yes | yes |
| `IS_NULL` / `IS_NOT_NULL` / `IS_NAN` / `IS_NOT_NAN` | served | not documented | yes | yes |
| `AND` and `OR` composite filters, nested | served | not documented | yes | yes |
| `orderBy()`, ascending and descending, several terms | served | yes | yes | yes |
| `limit()`, `offset()` | served | not documented | yes | yes |
| `select()` projection, including keys-only (`__name__`) | served | not documented | yes | yes |
| `limitToLast()` | served | not documented | yes | yes |
| Filtering and ordering on `__name__` | served | not documented | yes | yes |
| Collection group queries (`all_descendants`) | served | not documented | yes | yes |
| Cursors (`startAt`/`startAfter`/`endAt`/`endBefore`) | served | yes | yes | yes |
| Nested field paths (`a.b`) in filters, orders, projections, and read masks | served | not documented | yes | yes |
| A top-level `update_mask` path the write does not carry | served — the field is deleted, as production does | not documented | yes | yes |
| Nested field paths in an `update_mask` | `UNIMPLEMENTED` | not documented | yes | yes |
| A collection group query with no collection id | `UNIMPLEMENTED` | not documented | yes | yes |
| Vector search (`find_nearest`) | `UNIMPLEMENTED` | not documented | not documented | yes |
| Aggregations (`count()`, `sum()`, `average()`) | `UNIMPLEMENTED` | yes | yes | yes |

## Field transforms

| Transform | fireup local | floci-gcp | Firebase Emulator Suite | Firestore on GCP |
|---|---|---|---|---|
| `set_to_server_value: REQUEST_TIME` | served | yes | yes | yes |
| `increment` | served | yes | yes | yes |
| `append_missing_elements` | served | yes | yes | yes |
| `remove_all_from_array` | served | yes | yes | yes |
| `maximum / minimum` | `UNIMPLEMENTED` | not documented | yes | yes |
| Nested field path (`a.b`) on a transform | `UNIMPLEMENTED` | not documented | yes | yes |

floci-gcp documents "field transforms" as a group rather than per transform; the
four rows marked `yes` are the ones its Firestore page names.

## Where the local implementations diverge from production

This is the part a feature count hides. All three local implementations differ
from GCP somewhere; what matters is whether the difference is written down.

| Behaviour | fireup local | floci-gcp | Firebase Emulator Suite | Firestore on GCP |
|---|---|---|---|---|
| Composite index enforcement | no — a query that works here can still need an index in production | not documented | no — "does not track compound indexes and instead will execute any valid query" | **yes** — a query without its index is rejected |
| Production limit enforcement (e.g. transaction size) | no | not documented | no — "may allow transactions that would be rejected as too large" | yes |
| `__name__` is appended to every `orderBy`, in the last term's direction | served | **diverges** — "Queries do not add the implicit trailing `__name__` order", so equal values come back in storage order | yes | yes |
| A document without the ordered field is absent from the results | served | **diverges** — "Documents that lack an `order_by` field are not excluded from the results" | yes | yes |
| `ListDocuments` ordering and paging | served | **diverges** — "ignores `order_by`, `page_size` and `page_token`" | yes | yes |
| Every write in a `Commit` applies or none does | served | yes | yes | yes |
| A concurrent write to a read document aborts the commit | served | yes | yes | yes |
| Phantom reads abort a transaction | no — a query's read set is the documents it returned | no — same limitation, documented | "does not currently implement all transaction behavior seen in production"; locks "may take up to 30 seconds to be released" | yes — production conflicts on the scanned range |
| Aggregations participate in a transaction's read set | n/a — no aggregations | **diverges** — "`RunAggregationQuery` ignores `transaction` and `newTransaction`" | yes | yes |
| Transaction lifetime | 60s, matching the documented limit | 15 minutes; transaction state is in memory and lost on restart | not documented | documented server deadlines |
| Point-in-time reads (any `read_time` consistency selector) | `UNIMPLEMENTED` | not documented | not documented | yes |

Both Fireup and floci-gcp publish their divergences, which is the thing to
actually look for in a local emulator. The difference is what happens to one
over time: Fireup's are rows in `matrix.json` with probes attached, so a
divergence that silently changes fails CI.

## Datastore

| RPC | fireup local | floci-gcp | Firebase Emulator Suite | Datastore on GCP |
|---|---|---|---|---|
| `Lookup` | served | yes (gRPC + HTTP/protobuf) | not emulated | yes |
| `RunQuery` | served | yes, structured **and GQL** | not emulated | yes |
| `Commit` | served | yes | not emulated | yes |
| `BeginTransaction / Rollback` | served | yes, limited semantics | not emulated | yes |
| `AllocateIds` | served | yes | not emulated | yes |
| `ReserveIds` | served | yes | not emulated | yes |
| `RunAggregationQuery` | `UNIMPLEMENTED` | yes (COUNT, SUM, AVG) | not emulated | yes |

Fireup's extra property: Datastore kinds and Firestore collections are **the
same rows**, so an entity written through the Datastore API is readable through
the Firestore API and as a SQL row. Non-default namespaces, GQL, projection
queries, and cursors are refused.

The Firebase Emulator Suite does not emulate the Datastore API at all. Google's
separate `gcloud` Datastore emulator does, and its own documented limits are
sharper than either local alternative's: it "does not support `IN`, `!=`, and
`NOT-IN` queries" and "does not support aggregation queries like `COUNT(*)`",
and it simulates eventual consistency by default.

### Datastore property masks

`Mutation.property_mask` — a masked entity write — is served per the proto, and
is one of the few places this comparison can say nothing at all about the
alternatives, because neither of them documents it:

| Mask shape | fireup local | floci-gcp | Firebase Emulator Suite | Datastore on GCP |
|---|---|---|---|---|
| A path the entity carries | served | not documented | not emulated | yes |
| A path the entity omits | served — deleted, per the proto | not documented | not emulated | yes |
| A property the mask does not name | served — left untouched | not documented | not emulated | yes |
| `__key__` | served — accepted and ignored | not documented | not emulated | yes |
| A property path (`foo.bar`) or an escaped name | `UNIMPLEMENTED` | not documented | not emulated | yes |
| An empty path | served | not documented | not emulated | yes |
| Any other reserved (`__`-wrapped) name | served | not documented | not emulated | yes |

## Firebase Auth / Identity Toolkit

| Feature | fireup local | floci-gcp | Firebase Auth emulator | Identity Platform on GCP |
|---|---|---|---|---|
| Email/password, anonymous, custom-token sign-in | served | yes | yes | yes |
| Admin account CRUD, list | served | yes | yes | yes |
| Token refresh (`/v1/token`) and revocation | served | yes | yes | yes |
| Unsigned ID tokens the Admin SDK accepts | served | yes | yes | n/a — real signatures |
| Out-of-band email codes (password reset, verify email) | served | **not yet** — returns 404 | yes | yes |
| Email-link sign-in | served | **not yet** | yes | yes |
| Sending email: the codes are real, the delivery is not | documented-divergence — read the link from `GET /emulator/v1/projects/{p}/oobCodes`; no SMTP | not applicable yet | same model: no mail is sent | n/a — real email |
| Phone auth: `sendVerificationCode`, `signInWithPhoneNumber` | `UNIMPLEMENTED` | not yet | yes, with codes readable over REST | yes |
| Federated / OAuth providers: `signInWithIdp`, `createAuthUri` | `UNIMPLEMENTED` | not yet | yes, for OpenID Connect providers | yes |
| Multi-factor enrolment and tenants (Identity Platform v2) | `UNIMPLEMENTED` | not yet | SMS MFA yes; tenants no | yes |
| Session cookies: `createSessionCookie`, `verifySessionCookie` | `UNIMPLEMENTED` | not yet (tenant-scoped path returns 404) | yes | yes |
| Batch account writes: `accounts:batchDelete`, `accounts:batchCreate` | `UNIMPLEMENTED` | `batchDelete` yes | yes | yes |

Auth is the one surface where Fireup currently serves **more** than floci-gcp:
the out-of-band email flows (`PASSWORD_RESET`, `VERIFY_EMAIL`, `EMAIL_SIGNIN`,
`VERIFY_AND_CHANGE_EMAIL`) and email-link sign-in are served here and documented
as not-yet-implemented there. Neither sends mail; both hand you the code.

## How to choose

- **Migrating off Firestore, or want the same data in SQL** → Fireup. It is the
  only one of the four that does this, and the reason the project exists.
- **Breadth of GCP services locally** (Pub/Sub, GCS, BigQuery, Cloud Run…) →
  floci-gcp. Fireup deliberately does not go wide.
- **Realtime listeners, aggregations, or Security Rules locally** → Google's
  emulator or floci-gcp. Fireup refuses all three, explicitly.
- **Testing Security Rules, or Functions triggers** → the Firebase Emulator
  Suite. Nothing else emulates rules.
- **Fidelity you can audit** → Fireup publishes its matrix and the suite that
  proves it. Use it to find out what will break *before* you move.
- They coexist: `4589`, `4588`, and `8080`/`9099` do not collide, and Fireup can
  serve a subset (`--enable firestore`) beside another emulator doing the rest.

## Sources

Read 2026-11-06. Public documentation only — no proprietary binary was
inspected, decompiled, or observed to produce this comparison; see
[`docs/licenses/FIREBASE_GCP_LOCAL.md`](../licenses/FIREBASE_GCP_LOCAL.md).

| Column | Source |
|---|---|
| fireup local | [`docs/local/README.md`](README.md#fidelity-matrix) and `tests/conformance/matrix.json`, in this repository |
| floci-gcp | [`floci-io/floci-gcp`](https://github.com/floci-io/floci-gcp) — `README.md`, `docs/services/firestore.md`, `docs/services/datastore.md`, `docs/services/firebase-auth.md` (MIT) |
| Firebase Emulator Suite | [Emulator Suite overview](https://firebase.google.com/docs/emulator-suite), [Firestore emulator](https://firebase.google.com/docs/emulator-suite/connect_firestore), [Auth emulator](https://firebase.google.com/docs/emulator-suite/connect_auth) |
| Datastore emulator | [Datastore emulator](https://cloud.google.com/datastore/docs/tools/datastore-emulator) |
| Firestore on GCP | [Firestore documentation](https://firebase.google.com/docs/firestore) |

Quoted strings in the tables above are verbatim from those pages. Everything
else is a summary, and a summary of someone else's README is exactly as reliable
as their README.
