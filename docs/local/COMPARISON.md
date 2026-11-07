# `fireup local` compared: the Firebase Emulator Suite, and Firestore on GCP

Three things can answer a Firestore SDK on your laptop or in your cloud project.
This is what each one does, and — more usefully — where they disagree.

Read the [fidelity matrix](README.md#fidelity-matrix) first if you only care
about Fireup: it is the complete, checked statement of what `fireup local`
serves and what it refuses. This document is the *comparison*, and until
SAK-217 it was a much weaker kind of evidence: the emulator column was a
reading of Google's documentation.

**Most of that column is now measured.** The conformance probes — Google's own
client libraries, plus raw gRPC on the public protos — were pointed at the
emulator the Firebase CLI downloads, in both of its database modes, and the
cells below say what came back. Every cell carries its provenance:

| Marker | Meaning |
|---|---|
| `[M]` | **Measured.** A probe was run against the version in the table below and this is what it answered. The raw per-row output is in [`measurements/`](measurements/). |
| `[D]` | **Documented only.** Google's own documentation, read on the date below. Nobody ran it. `not documented [D]` means the docs do not say — not that the feature is missing. |

| Column | Provenance | Version | Date |
|---|---|---|---|
| **fireup local** | `tests/conformance/matrix.json`, probed row by row through Google's own Node and Python client libraries in CI. Every row below whose label matches a matrix row is compared against it by `cargo test --test fidelity_matrix_tests`, so this column cannot drift from the suite that proves it. A `served` here means an official SDK did it, in CI, against both the in-memory and the PostgreSQL store. | this commit | continuous |
| **Firebase Emulator Suite** | `[M]` rows: `scripts/conformance-target.sh` against a running instance. `[D]` rows: [Google's documentation](#sources). The emulator ships as a closed-source JAR, so an `[M]` cell is what it *answered*, never what is inside it. | **`cloud-firestore-emulator` v1.22.0** via `firebase-tools` 15.35.0 | measured **2026-11-06**, docs read 2026-11-06 |
| **Firestore on GCP** | The production service, as documented. It is the thing the other two are imitating, and the column that decides who is right when they disagree. Nothing here was run against a billable Google service, so this column has no `[M]` cell. | current | docs read 2026-11-06 |

Google ships new emulator versions, so **an `[M]` cell is a claim about v1.22.0
and no other**. Re-measuring is one command; see
[`tests/conformance/README.md`](../../tests/conformance/README.md#measuring-something-that-is-not-fireup).

> **Independent implementation, and nothing here was reverse engineered.**
> Fireup implements **public** Google API contracts from public documentation
> and the public googleapis protobuf definitions. It is **not** the Firebase
> Emulator Suite and is **not** affiliated with or endorsed by Google.
> The measured cells come from **running** the published emulator and recording
> what its public API answered, driven by Google's own Apache-2.0 client
> libraries — the same black-box method this project already uses to verify
> itself. Google's emulator was **not decompiled, disassembled, or
> binary-analysed**, no JAR, class file, or container layer was unpacked or
> inspected, and nothing was copied out of it.
> "Firebase", "Firestore", "Datastore" and "Google Cloud"
> are Google's trademarks and are used here only to name the products being
> compared. See
> [`docs/licenses/FIREBASE_GCP_LOCAL.md`](../licenses/FIREBASE_GCP_LOCAL.md).

Other third-party local-cloud emulators exist and are not compared here. The
measurement recipe works against anything serving the same contract; publishing
a comparison against a named third party is a separate decision from being able
to make one.

## What measuring found that reading could not

Two corrections to this document, and four bugs in Fireup. A comparison that
only ever moved in our favour would not be worth publishing, so both lists are
here.

**Corrections to this document.**

1. **The Firebase Emulator Suite does emulate Datastore.** Every Datastore row
   here used to say `not emulated`. The emulator the Firebase CLI downloads
   answers every Datastore RPC with `INVALID_ARGUMENT: Database mode not set to
   Datastore Mode. If using the gCloud CLI, please specify the --database-mode
   flag.` — a refusal naming its own switch, not an unknown method. Started with
   that published flag it **serves 20 of the 20 measurable Datastore rows**:
   every `property_mask` row, every property transform, GQL, aggregations. It is
   the reference implementation there too. `firebase emulators:start` does not
   expose the flag; `gcloud emulators firestore start
   --database-mode=datastore-mode` does, and so does running the JAR directly.
2. **It refuses `PartitionQuery`** —
   `UNIMPLEMENTED: Method google.firestore.v1.Firestore/PartitionQuery is
   unimplemented`, the same gRPC status Fireup uses. That cell said
   `not documented`, and "use the emulator for `PartitionQuery`" was never
   advice that worked.

**Bugs this found in Fireup.** Pointing the suite at the reference
implementation is the only reason these are known; each has a follow-up issue.

| What Fireup does | What the emulator does | Why it matters |
|---|---|---|
| Accepts an `order_by` with a term *after* `__name__`, including a duplicate `__name__` | `INVALID_ARGUMENT: order by clause cannot contain more fields after the key` | A query that passes locally fails in production. The official Node SDK emits this shape for `.where(…, '!=', …).orderBy('__name__')` |
| Serves a **descending `__name__` scan**, and `matrix.json` has a probe *asserting* it works | `FAILED_PRECONDITION: Firestore does not support descending key scans` | A matrix row claims parity that does not exist — the worst kind of row to have |
| `accounts:batchCreate` answered `405 "INVALID_ARGUMENT : accounts:batchCreate takes POST, not POST"` | `200` | A self-contradictory refusal, from a catch-all arm that fired regardless of method. The matrix promises refusals that name the gap. **Fixed by SAK-224** — now `501 OPERATION_NOT_ALLOWED : batch account import (create accounts one at a time) is not served by fireup local` |
| `projects:createSessionCookie` answered `404 PROJECT_NOT_FOUND … not "fireup-local:createSessionCookie"` | `400 INVALID_ID_TOKEN` | The `{project}:customMethod` colon syntax was parsed into the project id, so the row refused with the wrong error. **Fixed by SAK-224** — now `501 OPERATION_NOT_ALLOWED : session cookies is not served by fireup local` |

**What the harness still cannot measure**, said plainly so no cell pretends
otherwise:

- **Duplex-streaming rows.** `Listen` and the streaming `Write` both come back
  as a bare gRPC `UNKNOWN` with no message, while unary and server-streaming
  calls to the same port are fine and the SDK's `onSnapshot()` returns a
  snapshot. That is this harness's raw duplex client, not a refusal, so those
  cells stay `[D]`.
- **15 rows have no `served` branch** to point at anything — mostly features
  Fireup refuses, where there was never a positive check to write. Three of the
  interesting ones (streaming `Write`, `PartitionQuery`, Datastore GQL) got one
  in SAK-217 and are measured; the rest stay `[D]`.
- **The auth rows** are driven by `scripts/smoke-local-auth.sh`, which starts
  its own server, so they are not in the observation runner. The rows marked
  `[M]` below were measured with direct REST calls instead.

## At a glance

Product-level facts, all of them from each project's own documentation — there
is no probe for "how many services does it have". The per-feature tables below
are where the measured cells are.

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

Measured on the 73 rows that could be measured: Fireup answers 67 as its matrix
says and refuses 6 that it says it refuses, with no divergences. The emulator
answers 44 of the 73 in Firestore-native mode plus 20 of the 20 measurable
Datastore rows in Datastore mode, with one divergence and four rows where it
rejects a query Fireup accepts. It measures like the reference implementation,
which is what it is.

## Firestore RPCs

`google.firestore.v1.Firestore` has 17 RPCs. Fireup's statuses are the
[fidelity matrix](README.md#fidelity-matrix).

| RPC | fireup local | Firebase Emulator Suite | Firestore on GCP |
|---|---|---|---|
| `GetDocument` | served | yes `[M]` | yes |
| `ListDocuments` | served | yes `[M]` | yes |
| `CreateDocument` | served | yes `[M]` | yes |
| `UpdateDocument` | served | yes `[M]` | yes |
| `DeleteDocument` | served | yes `[M]` | yes |
| `BatchGetDocuments` | served | yes `[M]` | yes |
| `BeginTransaction` | served | yes `[M]` | yes |
| `Commit` | served | yes `[M]` | yes |
| `Rollback` | served | yes `[M]` | yes |
| `RunQuery` | served | yes `[M]` | yes |
| `BatchWrite` | served | yes `[M]` | yes |
| `ListCollectionIds` | served | yes `[M]` | yes |
| `Listen` | served | yes `[D]` — the SDK's `onSnapshot()` works; our raw duplex probe gets a bare `UNKNOWN` | yes |
| `Write (streaming)` | `UNIMPLEMENTED` | yes `[D]` — same duplex limit | yes |
| `RunAggregationQuery` | `UNIMPLEMENTED` | yes `[M]` | yes |
| `PartitionQuery` | `UNIMPLEMENTED` | **no `[M]`** — `UNIMPLEMENTED: Method … /PartitionQuery is unimplemented`, the same status Fireup returns | yes |
| `ExecutePipeline` | `UNIMPLEMENTED` | not documented `[D]` | preview |

Fireup refuses four of the 17. The emulator serves two of them —
streaming `Write` and `RunAggregationQuery` — so if you need
`count()`/`sum()`/`average()` locally today, that is the honest answer: Google's
emulator, not Fireup. `PartitionQuery` is the row measurement corrected: both
refuse it, so there is no local option at all. Fireup's refusals are an explicit
`UNIMPLEMENTED` whose message names the gap, which is a different promise from
serving them, and a weaker one.

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
| `where()` with `==`, `!=`, `<`, `<=`, `>`, `>=` | served | yes `[M]` | yes |
| `where()` with `in`, `not-in`, `array-contains`, `array-contains-any` | served | yes `[M]` | yes |
| `IS_NULL` / `IS_NOT_NULL` / `IS_NAN` / `IS_NOT_NAN` | served | yes `[D]` — our probe's `orderBy('__name__')` is rejected there; see the Fireup bug table | yes |
| `AND` and `OR` composite filters, nested | served | yes `[D]` — same rejected probe shape | yes |
| `orderBy()`, ascending and descending, several terms | served | yes `[M]` | yes |
| `limit()`, `offset()` | served | yes `[M]` | yes |
| `select()` projection, including keys-only (`__name__`) | served | yes `[M]` | yes |
| `limitToLast()` | served | yes `[M]` | yes |
| Filtering and ordering on `__name__` | served | yes, ascending only `[M]` — a descending key scan is `FAILED_PRECONDITION` | yes, ascending only |
| Collection group queries (`all_descendants`) | served | yes `[M]` | yes |
| Cursors (`startAt`/`startAfter`/`endAt`/`endBefore`) | served | yes `[M]` | yes |
| Nested field paths (`a.b`) in filters, orders, projections, and read masks | served | yes `[M]` | yes |
| A top-level `update_mask` path the write does not carry | served — the field is deleted, as production does | yes `[M]` | yes |
| Nested field paths in an `update_mask` | `UNIMPLEMENTED` | yes `[D]` | yes |
| A collection group query with no collection id | `UNIMPLEMENTED` | yes `[D]` | yes |
| Vector search (`find_nearest`) | `UNIMPLEMENTED` | not documented `[D]` | yes |
| Aggregations (`count()`, `sum()`, `average()`) | `UNIMPLEMENTED` | yes `[M]` | yes |

The `__name__` row is the one to read. Fireup serves a **descending** key scan;
the emulator and production both refuse it (`FAILED_PRECONDITION: Firestore does
not support descending key scans`), and `matrix.json` currently has a probe
asserting Fireup's behaviour. That is a published parity claim that is not true,
and it has a follow-up issue.

## Field transforms

| Transform | fireup local | Firebase Emulator Suite | Firestore on GCP |
|---|---|---|---|
| `set_to_server_value: REQUEST_TIME` | served | yes `[M]` | yes |
| `increment` | served | yes `[M]` | yes |
| `append_missing_elements` | served | yes `[M]` | yes |
| `remove_all_from_array` | served | yes `[M]` | yes |
| `maximum / minimum` | `UNIMPLEMENTED` | yes `[D]` | yes |
| Nested field path (`a.b`) on a transform | `UNIMPLEMENTED` | yes `[D]` | yes |

The four served transforms were measured against the semantics the public
`write.proto` spells out, not just "it did something": a missing or non-numeric
field set to the operand, double contagion, and the array transforms treating a
non-array field as empty. Fireup and the emulator agree on all of it.

## Where the local implementations diverge from production

This is the part a feature count hides. Both local implementations differ from
GCP somewhere; what matters is whether the difference is written down.

| Behaviour | fireup local | Firebase Emulator Suite | Firestore on GCP |
|---|---|---|---|
| Composite index enforcement | no — a query that works here can still need an index in production | no `[D]` — "does not track compound indexes and instead will execute any valid query" | **yes** — a query without its index is rejected |
| Production limit enforcement (e.g. transaction size) | no | no `[D]` — "may allow transactions that would be rejected as too large" | yes |
| `__name__` is appended to every `orderBy`, in the last term's direction | served | yes `[M]` | yes |
| A document without the ordered field is absent from the results | served | yes `[M]` | yes |
| `ListDocuments` ordering and paging | served | yes `[M]` | yes |
| Every write in a `Commit` applies or none does | served | yes `[M]` | yes |
| A concurrent write to a read document aborts the commit | served | yes `[M]` — but it blocks for tens of seconds first, so a 20s client deadline reads as a hang | yes |
| Phantom reads abort a transaction | no — a query's read set is the documents it returned | `[D]` "does not currently implement all transaction behavior seen in production"; locks "may take up to 30 seconds to be released" — **measured: a Datastore-mode transaction conflict blocked past the client's 60s deadline** | yes — production conflicts on the scanned range |
| Transaction lifetime | 60s, matching the documented limit | not documented `[D]` | documented server deadlines |
| Point-in-time reads (any `read_time` consistency selector) | `UNIMPLEMENTED` | yes `[M]` — within its retention window; an older `read_time` is `FAILED_PRECONDITION: The requested 'read_time' is too old` | yes |
| An `order_by` term after `__name__` | **accepted** — and nothing warns | `INVALID_ARGUMENT: order by clause cannot contain more fields after the key` `[M]` | rejected |
| A descending `__name__` scan | **served** — and `matrix.json` asserts it | `FAILED_PRECONDITION: Firestore does not support descending key scans` `[M]` | rejected |

Publishing its divergences is the thing to actually look for in a local
emulator, and the thing to look for after that is what happens to them over
time: Fireup's are rows in `matrix.json` with probes attached, so a divergence
that silently changes fails CI.

What measurement added is the last two rows, and they are the limit of that
guarantee. A probe keeps a row from changing silently; it does not keep the row
from having been **wrong when it was written**. Both of those divergences had
passing probes the whole time, because the probe asserted what Fireup does.
Nothing but the reference implementation was ever going to catch them.

The emulator's documented lock behaviour is also now a measured number rather
than a caveat: two transaction-conflict probes that Fireup answers in
milliseconds need a probe timeout above 20 seconds there, and the Datastore-mode
conflict never came back inside the client's 60-second deadline. If you are
writing a test that exercises contention, that is the difference between a green
suite and a flake.

## Datastore

Every cell in this column said `not emulated` until SAK-217 measured it. It was
wrong — see the correction below the table.

| RPC | fireup local | Firebase Emulator Suite | Datastore on GCP |
|---|---|---|---|
| `Lookup` | served | yes `[M]`, in Datastore mode | yes |
| `RunQuery` | served | yes `[M]`, structured and GQL, in Datastore mode | yes |
| `Commit` | served | yes `[M]`, in Datastore mode | yes |
| `BeginTransaction / Rollback` | served | yes `[M]`, in Datastore mode — a conflict blocks past a 60s client deadline | yes |
| `AllocateIds` | served | yes `[M]`, in Datastore mode | yes |
| `ReserveIds` | served | yes `[M]`, in Datastore mode | yes |
| `RunAggregationQuery` | `UNIMPLEMENTED` | yes `[M]`, in Datastore mode | yes |

Fireup's extra property: Datastore kinds and Firestore collections are **the
same rows**, so an entity written through the Datastore API is readable through
the Firestore API and as a SQL row. Non-default namespaces, GQL, projection
queries, and cursors are refused.

**Correction, from measuring instead of reading.** The emulator the Firebase CLI
downloads *does* emulate Datastore. In its default mode every Datastore RPC
answers `INVALID_ARGUMENT: Database mode not set to Datastore Mode. If using the
gCloud CLI, please specify the --database-mode flag.` — a refusal that names its
own switch, not an unknown method. Started with that published flag it serves
**20 of the 20 measurable Datastore rows**, including GQL, aggregations, every
property transform, and every `property_mask` row. `firebase emulators:start`
does not expose the flag; `gcloud emulators firestore start
--database-mode=datastore-mode` does, as does running the JAR directly with
`--database-mode=datastore-mode`. It calls Datastore mode a preview on startup.

Google's *separate*, older `gcloud` Datastore emulator is a different binary and
was not measured. Its own documented limits remain sharp in places Fireup's are
not: it "does not support `IN`, `!=`, and `NOT-IN` queries" and "does not
support aggregation queries like `COUNT(*)`", and it simulates eventual
consistency by default.

### Datastore property masks

`Mutation.property_mask` — a masked entity write — is served per the proto.
Nothing documents whether the emulator honours it, which is why this table used
to be empty; it has now been asked directly, in Datastore mode.

| Mask shape | fireup local | Firebase Emulator Suite | Datastore on GCP |
|---|---|---|---|
| A path the entity carries | served | yes `[M]`, in Datastore mode | yes |
| A path the entity omits | served — deleted, per the proto | yes `[M]`, in Datastore mode | yes |
| A property the mask does not name | served — left untouched | yes `[M]`, in Datastore mode | yes |
| `__key__` | served — accepted and ignored | yes `[M]`, in Datastore mode | yes |
| A property path (`foo.bar`) or an escaped name | `UNIMPLEMENTED` | not documented `[D]` | yes |
| An empty path | served | yes `[M]`, in Datastore mode | yes |
| Any other reserved (`__`-wrapped) name | served | yes `[M]`, in Datastore mode | yes |

Six of these seven rows are the ones SAK-203 implemented straight from the
sentences in `datastore.proto`, with no way to check the reading against another
implementation. Measurement is that check, and the reading was right: Fireup and
the reference implementation agree on every one.

## Firebase Auth / Identity Toolkit

These rows are not in the observation runner — they are driven by
`scripts/smoke-local-auth.sh`, which starts its own server. The `[M]` cells
were measured with direct REST calls instead, sending the privileged paths the
way the Admin SDK does: `Authorization: Bearer owner`, the documented emulator
convention for an admin credential. Without that header both the emulator and
Fireup correctly refuse, which is itself worth knowing.

| Feature | fireup local | Firebase Auth emulator | Identity Platform on GCP |
|---|---|---|---|
| Email/password, anonymous, custom-token sign-in | served | yes `[M]` | yes |
| Admin account CRUD, list | served | yes `[D]` | yes |
| Token refresh (`/v1/token`) and revocation | served | yes `[D]` | yes |
| Unsigned ID tokens the Admin SDK accepts | served | yes `[M]` | n/a — real signatures |
| Out-of-band email codes (password reset, verify email) | served | yes `[M]` | yes |
| Email-link sign-in | served | yes `[M]` | yes |
| Sending email: the codes are real, the delivery is not | documented-divergence — read the link from `GET /emulator/v1/projects/{p}/oobCodes`; no SMTP | same model: no mail is sent `[M]` — the code is readable from the same route | n/a — real email |
| Phone auth: `sendVerificationCode`, `signInWithPhoneNumber` | `UNIMPLEMENTED` | yes, with codes readable over REST `[M]` | yes |
| Federated / OAuth providers: `signInWithIdp`, `createAuthUri` | `UNIMPLEMENTED` | yes, for OpenID Connect providers `[M]` | yes |
| Multi-factor enrolment and tenants (Identity Platform v2) | `UNIMPLEMENTED` | SMS MFA yes; tenants no `[D]` | yes |
| Session cookies: `createSessionCookie`, `verifySessionCookie` | `UNIMPLEMENTED` | yes `[M]` | yes |
| Batch account writes: `accounts:batchDelete`, `accounts:batchCreate` | `UNIMPLEMENTED` | both yes `[M]` | yes |

Auth is the surface where Fireup is closest to Google's emulator, and measuring
the shared half confirmed it: the out-of-band email flows (`PASSWORD_RESET`,
`VERIFY_EMAIL`, `EMAIL_SIGNIN`, `VERIFY_AND_CHANGE_EMAIL`), email-link sign-in,
and the `oobCodes` route all answer the same way on both. Neither sends mail;
both hand you the code. Phone, federated providers, MFA, tenants and session
cookies are the gap.

Measuring the privileged half is also where two of the Fireup bugs at the top of
this document came from. `accounts:batchCreate` answered `405` with a message
that contradicted itself, and `projects:createSessionCookie` answered
`404 PROJECT_NOT_FOUND` because the `{project}:customMethod` colon was parsed
into the project id. Both rows are `UNIMPLEMENTED` in the matrix, which was
always the right status — the refusals were the wrong shape, and the matrix
promises a refusal that names the gap.

SAK-224 fixed both, and the fireup side of those two rows was **re-measured, not
reworded** — against `fireup local` built from `main` at `5d015dc`, on
2026-11-06, with the same direct REST calls that found them:

```
POST /identitytoolkit.googleapis.com/v1/projects/{project}/accounts:batchCreate
  → 501 OPERATION_NOT_ALLOWED : batch account import (create accounts one at a
    time) is not served by fireup local
POST /identitytoolkit.googleapis.com/v1/projects/{project}:createSessionCookie
  → 501 OPERATION_NOT_ALLOWED : session cookies is not served by fireup local
```

`accounts:batchDelete` and `verifySessionCookie` answer the same way as their
partners. The observed gap against the emulator is unchanged — it serves both
and Fireup does not — so the rows stay in the table; only Fireup's half moved
from "wrong error" to "a 501 that names the gap", which is what the matrix
promised all along.

## How to choose

- **Migrating off Firestore, or want the same data in SQL** → Fireup. It is the
  only one of the three that does this, and the reason the project exists.
- **Realtime listeners** → either. Fireup serves `Listen`, without resume
  tokens or point-in-time resume.
- **Aggregations, Security Rules, or Functions triggers** → Google's emulator.
  Fireup refuses all three, explicitly.
- **A local Datastore API** → Fireup, or Google's emulator in
  `--database-mode=datastore-mode`. Both serve it, and the emulator's Datastore
  mode is the one you cannot reach through `firebase emulators:start`.
- **`PartitionQuery`** → neither. Both refuse it with the same gRPC status.
- **Fidelity you can audit** → Fireup publishes its matrix, the suite that
  proves it, and now the raw measurements of the alternative in
  [`measurements/`](measurements/). Use them to find out what will break
  *before* you move.
- They coexist: `4589` and `8080`/`9099` do not collide, and Fireup can serve a
  subset (`--enable firestore`) beside another emulator doing the rest.

## Sources

Two kinds of source, and the tables say which one each cell came from.

**Measured (`[M]`).** Each target was started locally and asked, through
Google's own Apache-2.0 client libraries and raw gRPC over the public protos.
The raw per-row output is committed in [`measurements/`](measurements/); the
command is in
[`tests/conformance/README.md`](../../tests/conformance/README.md#measuring-something-that-is-not-fireup).

| Target | What was run | Measured |
|---|---|---|
| Firebase Emulator Suite | `firebase emulators:start --only firestore,auth` with `firebase-tools` 15.35.0 — `cloud-firestore-emulator` **v1.22.0**, Firestore on `:18080`, Auth on `:19099` (the documented `8080` was occupied on the measuring host); and the same JAR run with its published `--database-mode=datastore-mode` flag on `:18081` | 2026-11-06 |
| fireup local | this commit, in-memory store, `:4589` — the negative control | 2026-11-06 |

Running a published binary and recording what its public API answers is
observation. **Nothing was decompiled, disassembled, or binary-analysed**; no
JAR, class file, or container layer was unpacked or inspected; and nothing was
copied out of it. The artefacts under `measurements/` are first-party output of
our own probes — a row id, a verdict, and the target's own error string. See
[`docs/licenses/FIREBASE_GCP_LOCAL.md`](../licenses/FIREBASE_GCP_LOCAL.md). No
request was made to a billable Google service, which is why the GCP column has
no `[M]` cell.

**Documented (`[D]`).** Read 2026-11-06.

| Column | Source |
|---|---|
| fireup local | [`docs/local/README.md`](README.md#fidelity-matrix) and `tests/conformance/matrix.json`, in this repository |
| Firebase Emulator Suite | [Emulator Suite overview](https://firebase.google.com/docs/emulator-suite), [Firestore emulator](https://firebase.google.com/docs/emulator-suite/connect_firestore), [Auth emulator](https://firebase.google.com/docs/emulator-suite/connect_auth) |
| Datastore emulator | [Datastore emulator](https://cloud.google.com/datastore/docs/tools/datastore-emulator) |
| Firestore on GCP | [Firestore documentation](https://firebase.google.com/docs/firestore) |

Quoted strings in the tables above are verbatim from those pages, or from the
emulator's own error message. Everything else marked `[D]` is a summary, and a
summary of someone else's documentation is exactly as reliable as their
documentation — which is the lesson of the whole `not emulated` Datastore
column that measurement deleted.
