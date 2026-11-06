# Fireup — Firebase / GCP local & API license ledger

> First-party Fireup code is licensed under **Apache-2.0** (see root `LICENSE`). Brand assets under `brand/` are carved out — see `brand/LICENSE`. Dependency-tree audit: [`CARGO_LICENSE_AUDIT.md`](CARGO_LICENSE_AUDIT.md).

**Status:** current as of Phase 2.1 (SAK-189) — Firestore `Listen` / realtime listeners. Prior: SAK-195 managed-export fidelity regression fixtures; SAK-193 Phase L.1 out-of-band email flows (no mail transport); SAK-190 Firestore field transforms; SAK-196 first-party Apache-2.0 + brand trademark carve-out with the dependency audit recorded; SAK-174 seeding restores Datastore value types; Phase 4 (SAK-167) glue docs and the end-to-end demo; Phase 3 (SAK-166) Datastore and Phase L (SAK-168) Auth
**Rule:** `.cursor/rules/firebase-gcp-compliance.mdc` (always apply)

Independent implementation over **public** Google API contracts. Not affiliated with Google. Do not reverse-engineer proprietary Firebase/Google emulator binaries.

Update this ledger on every PR that adds Google/Firebase-related dependencies, protos, or generated stubs.

## Entries

### Vendored protos (`proto/google/`)

All of these come from the public [googleapis/googleapis](https://github.com/googleapis/googleapis)
repository, Apache-2.0, pinned to commit
[`57aa4dbc`](https://github.com/googleapis/googleapis/tree/57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d)
and fetched by `scripts/vendor-googleapis-protos.sh`. Copyright headers are
intact — the script copies the files byte-for-byte and never edits them.

These are Google's **published API definitions**. They are not emulator source,
and nothing here was obtained by decompiling or analysing a binary.

| Proto path | Version / pin | SPDX | Upstream | How we use it | Attribution OK? | Decision |
|---|---|---|---|---|---|---|
| `google/firestore/v1/firestore.proto` | googleapis @ `57aa4dbc` | Apache-2.0 | [link](https://github.com/googleapis/googleapis/blob/57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d/google/firestore/v1/firestore.proto) | prost/tonic codegen → `fireup local` service + test client | Yes (header intact) | Public Apache-2.0 googleapis pin |
| `google/firestore/v1/document.proto` | googleapis @ `57aa4dbc` | Apache-2.0 | [link](https://github.com/googleapis/googleapis/blob/57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d/google/firestore/v1/document.proto) | Document/Value wire types | Yes | Same pin |
| `google/firestore/v1/common.proto` | googleapis @ `57aa4dbc` | Apache-2.0 | [link](https://github.com/googleapis/googleapis/blob/57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d/google/firestore/v1/common.proto) | `DocumentMask`, `Precondition`, `TransactionOptions` | Yes | Same pin |
| `google/firestore/v1/write.proto` | googleapis @ `57aa4dbc` | Apache-2.0 | [link](https://github.com/googleapis/googleapis/blob/57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d/google/firestore/v1/write.proto) | `Write`/`WriteResult` for `Commit` | Yes | Same pin |
| `google/firestore/v1/query.proto` | googleapis @ `57aa4dbc` | Apache-2.0 | [link](https://github.com/googleapis/googleapis/blob/57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d/google/firestore/v1/query.proto) | `StructuredQuery` types — filters, ordering, projection — now served by `RunQuery` | Yes | Same pin |
| `google/firestore/v1/aggregation_result.proto`, `bloom_filter.proto`, `explain_stats.proto`, `pipeline.proto`, `query_profile.proto` | googleapis @ `57aa4dbc` | Apache-2.0 | [dir](https://github.com/googleapis/googleapis/tree/57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d/google/firestore/v1) | Transitive imports of `firestore.proto`; compiled, not served | Yes | Same pin — pulled automatically by the import closure |
| `google/api/annotations.proto`, `client.proto`, `field_behavior.proto`, `http.proto`, `launch_stage.proto`, `routing.proto` | googleapis @ `57aa4dbc` | Apache-2.0 | [dir](https://github.com/googleapis/googleapis/tree/57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d/google/api) | Service annotations required to compile the Firestore service | Yes | Same pin |
| `google/rpc/status.proto` | googleapis @ `57aa4dbc` | Apache-2.0 | [link](https://github.com/googleapis/googleapis/blob/57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d/google/rpc/status.proto) | Error status in batch write responses | Yes | Same pin |
| `google/type/latlng.proto` | googleapis @ `57aa4dbc` | Apache-2.0 | [link](https://github.com/googleapis/googleapis/blob/57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d/google/type/latlng.proto) | `geoPointValue`; also used by the Datastore export parser | Yes | Re-pinned to the same commit as the Firestore protos (previously an unpinned copy) |
| `google/datastore/v1/datastore.proto` | googleapis @ `57aa4dbc` | Apache-2.0 | [link](https://github.com/googleapis/googleapis/blob/57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d/google/datastore/v1/datastore.proto) | prost/tonic codegen → `fireup local` Datastore service + test client | Yes (header intact) | Public Apache-2.0 googleapis pin (Phase 3) |
| `google/datastore/v1/entity.proto`, `query.proto`, `aggregation_result.proto`, `query_profile.proto` | googleapis @ `57aa4dbc` | Apache-2.0 | [dir](https://github.com/googleapis/googleapis/tree/57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d/google/datastore/v1) | Entity/Key/Value/Query types for Datastore service + migrator Entity decode | Yes | Debt cleared — now on the same pin as Firestore via `scripts/vendor-googleapis-protos.sh` |
| `google/datastore/admin/v1/datastore_admin.proto`, `index.proto`, `migration.proto` | googleapis @ `57aa4dbc` | Apache-2.0 | [dir](https://github.com/googleapis/googleapis/tree/57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d/google/datastore/admin/v1) | Message stubs for admin surface (compiled; not served in Phase 3) | Yes | Same pin; replaces the unpinned `admin.proto` stub |
| `google/longrunning/operations.proto` | googleapis @ `57aa4dbc` | Apache-2.0 | [link](https://github.com/googleapis/googleapis/blob/57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d/google/longrunning/operations.proto) | Transitive import of Datastore Admin | Yes | Same pin |

### Rust dependencies added for `fireup local`

| Dependency | Version / pin | SPDX | Upstream | How we use it | Attribution OK? | Decision |
|---|---|---|---|---|---|---|
| `tonic` | 0.11 | MIT | [crates.io](https://crates.io/crates/tonic) | gRPC server + generated client | Yes (crate license) | Public OSS, not Google code. Pinned to the 0.11 line because it tracks `prost` 0.12, which the export parser already uses — one prost version in the tree. |
| `tonic-build` | 0.11 | MIT | [crates.io](https://crates.io/crates/tonic-build) | build-time service codegen from the public protos | Yes | Same line as `tonic` |
| `tokio-stream` | 0.1 (`net`) | MIT | [crates.io](https://crates.io/crates/tokio-stream) | `ReceiverStream` for handing multiplexed connections to tonic and for server streaming | Yes | Public OSS |
| `async-trait` | 0.1 | MIT OR Apache-2.0 | [crates.io](https://crates.io/crates/async-trait) | async methods on the `Store` trait (`src/local/store.rs`) | Yes (crate license) | Public OSS. Added in Phase 1 as a **direct** dependency; it was already in the tree via `tonic`, so no new third-party code enters the build. |
| `@google-cloud/firestore` (npm, **test- and demo-only**) | ^7.11.0 | Apache-2.0 | [npm](https://www.npmjs.com/package/@google-cloud/firestore) | `tests/local-smoke/` SDK smoke test and `demo/` (Phase 4); never linked or redistributed | Yes | Google's own *open-source* client library, used as a client against our server — the documented, non-RE way to verify API compatibility. Not vendored; installed on demand by `scripts/smoke-local-firestore.sh` and `scripts/demo-export-to-sql.sh`. |
| `@google-cloud/datastore` (npm, **test- and demo-only**) | ^9.2.1 | Apache-2.0 | [npm](https://www.npmjs.com/package/@google-cloud/datastore) | `tests/local-smoke-datastore/` SDK smoke test and `demo/` (Phase 4); never linked or redistributed | Yes | Same rationale as the Firestore smoke client. Installed on demand by `scripts/smoke-local-datastore.sh` and `scripts/demo-export-to-sql.sh`. |
| `base64` | 0.22 | MIT OR Apache-2.0 | [crates.io](https://crates.io/crates/base64) | base64url JWT segments and base64 password digests in `src/local/auth/` | Yes (crate license) | Public OSS, not Google code. Added in Phase L as a **direct** dependency; it was already in the tree transitively, so no new third-party code enters the build. |
| `sha2` | 0.11 | MIT OR Apache-2.0 | [crates.io](https://crates.io/crates/sha2) | salted SHA-256 password digests for locally stored accounts (`src/local/auth/user.rs`) | Yes (crate license) | Public OSS (RustCrypto), not Google code. Already in the tree transitively. Chosen over hand-rolling a hash; **not** a production password-storage scheme — see the Phase L notes below. |
| `firebase-admin` (npm, **test-only**) | ^13.0.0 | Apache-2.0 | [npm](https://www.npmjs.com/package/firebase-admin) · [source](https://github.com/firebase/firebase-admin-node) | `tests/local-smoke/auth-smoke.mjs` only — drives the privileged auth surface over `FIREBASE_AUTH_EMULATOR_HOST`; never linked or redistributed | Yes | Google's own *open-source* Admin SDK, used as a client against our server. Same precedent as `@google-cloud/firestore`: the documented, non-RE way to verify API compatibility, and explicitly permitted by `.cursor/rules/firebase-gcp-compliance.mdc`. Not vendored; installed on demand by `scripts/smoke-local-auth.sh`. |
| `firebase` (npm, **test-only**) | ^11.0.0 | Apache-2.0 | [npm](https://www.npmjs.com/package/firebase) · [source](https://github.com/firebase/firebase-js-sdk) | `tests/local-smoke/auth-smoke.mjs` only — drives the end-user auth surface via `connectAuthEmulator()`; never linked or redistributed | Yes | Google's own *open-source* client JS SDK. Needed in addition to `firebase-admin` because the client and admin SDKs speak to different halves of Identity Toolkit (`/v1/accounts:*` and `/v1/token` vs `/v1/projects/{p}/accounts*`), so neither alone proves the surface works. Not vendored; installed on demand. |

### Phase 1 (SAK-164) notes

No new Google or Firebase artefact was introduced. The durable store is built on
dependencies the migrator already used (`tokio-postgres`, `deadpool-postgres`),
and `async-trait` above is the only new direct dependency.

The JSONB payload format in `src/local/value_json.rs` follows the **published**
JSON encoding of `google.firestore.v1.Value` as documented in the public
Firestore REST reference and the vendored `document.proto`. It was written from
those public definitions; no emulator storage format was inspected or
reverse-engineered, and the choice of layout (one row per document, key columns
split out, seconds+nanos timestamps) is Fireup's own.

The HTTP `/` and `/health` routes are Fireup's own surface, not a reimplementation
of any Google endpoint. Their payloads are first-party.

### Phase 2 (SAK-165) notes

No new Google or Firebase artefact, no new proto, and no new Rust dependency.
The proto pin is unchanged at googleapis `57aa4dbc`; `query.proto` and
`write.proto` were already vendored and compiled, and Phase 2 only starts
serving the RPCs that use them.

The query semantics in `src/local/query.rs` — the value sort order, the
treatment of a document that lacks a filtered or ordered field, `NaN` handling,
and the implicit `__name__` tiebreak — were written from the **public** Firestore
query documentation and the vendored `query.proto`. No emulator was run,
decompiled, or observed to derive them, and no Google test suite or fixture was
used. Where our behaviour is known to differ from production Firestore, the
difference is published in the fidelity matrix in `docs/local/README.md` rather
than left for a user to discover.

The transaction model in `src/local/transactions.rs` (opaque 16-byte tokens, a
60-second lifetime matching the documented limit, and optimistic
concurrency via the read set) is Fireup's own design against the public
three-RPC contract. Google's transaction implementation was not inspected.

`@google-cloud/firestore` stays test-only and un-vendored. The Phase 2 smoke
test drives more of it (queries, `listDocuments()`, `listCollections()`,
`runTransaction()`, `bulkWriter()`), still purely as a **client** against our
server, which is the documented and non-reverse-engineering way to check API
compatibility.

### Phase L (SAK-168) notes — Firebase Auth / Identity Platform

**No new Google or Firebase artefact was introduced.** No proto was added: the
Identity Toolkit surface is REST/JSON, so there is nothing to generate stubs
from. The two new direct dependencies (`base64`, `sha2`, both rowed above) are
public Rust crates already present transitively.

#### What the implementation was written from

| Source | Status | How it was used |
|---|---|---|
| Public Identity Toolkit / Firebase Auth REST reference | Published Google documentation | Endpoint paths, request and response field names, `kind` discriminators, and the error codes clients match on (`EMAIL_EXISTS`, `INVALID_PASSWORD`, `EMAIL_NOT_FOUND`, `WEAK_PASSWORD`, `TOKEN_EXPIRED`, …) |
| Public Firebase ID-token reference | Published Google documentation | The ID token claim set: `iss`, `aud`, `sub`, `user_id`, `auth_time`, `iat`, `exp`, `email`, `email_verified`, `firebase.identities`, `firebase.sign_in_provider`, custom claims at the top level |
| `FIREBASE_AUTH_EMULATOR_HOST` | Documented emulator environment variable (explicitly allowed by the project rule) | The convention that the API host becomes the first path segment, so SDKs reach `/identitytoolkit.googleapis.com/v1/...` and `/securetoken.googleapis.com/v1/token` |
| Behaviour of Google's **open-source** client SDKs (`firebase-js-sdk`, `firebase-admin-node`, Apache-2.0) | Open source, public repositories | Confirmation that in emulator mode those clients accept a JWT with `"alg": "none"` and an empty signature, verifying claims without a signature |
| Publicly documented emulator REST control endpoints | Published Google documentation | `DELETE /emulator/v1/projects/{p}/accounts` (clear accounts) and `GET /emulator/v1/projects/{p}/config` |

**No reverse engineering.** The Firebase Emulator Suite was not decompiled,
disassembled, or binary-analysed, and no proprietary emulator source, asset, or
data file was copied or consulted. Where behaviour is not publicly documented it
is **not implemented**: phone auth, multi-factor enrolment and the rest of the
Identity Platform `v2` surface, and federated providers all answer `501` with a
message naming the gap. (Out-of-band email flows were in that list until
SAK-193; see the Phase L.1 notes below.) Reading Google's own
Apache-2.0-licensed *client* libraries to learn what a client expects is use of
open source, not reverse engineering of a closed binary.

#### Unsigned tokens, and why there is no key material

Production ID tokens are signed with a Google private key. Fireup has no such
key, and generating or shipping one would mean producing **production-shaped key
material**, an explicit non-goal of this phase. So `src/local/auth/token.rs`
mints unsigned JWTs (`alg: none`, empty signature) — the shape the open-source
SDKs accept in emulator mode. Nothing in the repository is, or generates, a
signing key, certificate, or API key.

Consequence, stated in the code, the CLI `--help`, the startup log, and
`docs/local/README.md`: anything that can reach the port can obtain a session
for any account. `fireup local` binds loopback by default.

#### First-party, not a reimplementation of anything Google ships

- The **refresh-token format** is Fireup's own base64url JSON envelope. Clients
  treat a refresh token as opaque by contract, so there was no wire format to
  match. Revocation is expressed through the documented `validSince` /
  `tokensValidAfterTime` semantics.
- The **account storage layout** (`fireup_local_auth_users`, its partial unique
  email index, epoch-millisecond `bigint` timestamps) is Fireup's own choice. No
  emulator storage format was inspected.
- **Password hashing** is a per-account salt and a single SHA-256 pass. This is
  deliberately *not* a production scheme — it has no work factor. It exists so a
  shared local database does not hold plaintext passwords. The `passwordHash`
  and `salt` fields in admin lookup responses are our own values in the
  documented field names; they are not compatible with, and must not be imported
  into, a real Identity Platform project.
- The `removedAccounts` count in the clear-accounts response is a first-party
  addition to an otherwise empty documented response body.

#### No claim of affiliation

Neither the code, the CLI help, the discovery document, nor the documentation
claims to be the Firebase Emulator Suite or to be affiliated with Google. The
discovery document at `GET /` carries the independent-implementation notice, and
`identitytoolkit.googleapis.com` appears only as the name of the *public API
contract being implemented*.

### Phase L.1 (SAK-193) notes — out-of-band email flows

**No new Google or Firebase artefact was introduced**, and no new dependency of
any kind: this is `src/local/auth/oob.rs` plus handlers, a table, and tests. The
previous Phase L note listed out-of-band email flows among the gaps that answer
`501`; they are now served, and that sentence is superseded by this section.

#### What the implementation was written from

| Source | Status | How it was used |
|---|---|---|
| Public Identity Toolkit REST reference for `accounts:sendOobCode`, `accounts:resetPassword`, `accounts:update`, `accounts:signInWithEmailLink` | Published Google documentation | Endpoint paths, the `requestType` enum values (`PASSWORD_RESET`, `VERIFY_EMAIL`, `EMAIL_SIGNIN`, `VERIFY_AND_CHANGE_EMAIL`), the `kind` discriminators, and the error codes clients match on (`INVALID_OOB_CODE`, `EXPIRED_OOB_CODE`, `MISSING_OOB_CODE`, `INVALID_REQ_TYPE`, `EMAIL_NOT_FOUND`, …) |
| Public "email action link" / custom-email-handler documentation | Published Google documentation | The link's query parameters and their names: `mode`, `oobCode`, `apiKey`, `continueUrl`, `lang`, and the `mode` values `resetPassword`, `verifyEmail`, `signIn`, `verifyAndChangeEmail` |
| Public Auth emulator documentation | Published Google documentation | That the local convention for "no mail transport" is a REST endpoint listing the pending codes — `GET /emulator/v1/projects/{p}/oobCodes` — rather than an SMTP client |
| Behaviour of Google's **open-source** client SDK (`firebase-js-sdk`, Apache-2.0) | Open source, public repository | That its `ActionCodeURL` link parser asserts on `apiKey`, `oobCode` and a known `mode` together and yields nothing without all three. Found by running the SDK against this server, then confirmed in its published source. This is why the link carries an `apiKey` even though nothing here validates one. |
| Admin SDK `generate*Link` request shapes (`firebase-admin-node`, Apache-2.0) | Open source, public repository | That those methods POST to `/v1/projects/{p}/accounts:sendOobCode` with `returnOobLink: true` and address the account by `email` rather than by session |

**No reverse engineering.** Nothing was decompiled, disassembled, or
binary-analysed, and no proprietary emulator source, asset, data file, or
rendered page was copied or consulted. Reading Google's own Apache-2.0 *client*
libraries to learn what a client requires is use of open source.

#### First-party, not a reimplementation of anything Google ships

- **The landing page is not a copy of anyone's.** Production links land on a
  Google-hosted page. Fireup serves `GET /emulator/action`, its own route under
  the local-only `/emulator/` namespace, answering **JSON** that names the
  pending action and the call that completes it. It exists so a generated link
  resolves to something instead of a 404, and its response carries a notice
  saying it is not a Google page and not part of the Identity Toolkit API. No
  Google page's markup, copy, or styling was reproduced.
- **The code format is Fireup's own**: 32 random bytes as base64url. An
  `oobCode` is opaque to clients by contract — they hand it back verbatim — so
  there was no wire format to match, exactly as with refresh tokens.
- **The one-hour lifetime is Fireup's choice**, applied uniformly to all four
  request types. It is the published default for a password reset link; a single
  number for every type is one fewer thing to be surprised by locally, and the
  shortest documented default is the conservative pick. It is not a claim about
  what any other implementation does.
- **The code storage layout** (`fireup_local_auth_oob_codes`, its own table with
  no foreign key to the accounts table, epoch-millisecond `bigint` timestamps)
  is Fireup's own choice. No emulator storage format was inspected.
- **The `apiKey` placeholder** in a link requested without one is the sentence
  `fireup-local-does-not-check-api-keys`, deliberately not key-shaped: the
  parameter is required by the official client parser, and nothing here
  validates it. No real or realistic API key appears anywhere in the repository.
- **First-party additions to documented responses:** `removedOobCodes` in the
  clear responses, and the `notice` / `mode` / `applied` / `finishWith` fields
  on the landing route, which is a first-party route to begin with.

#### What is still not served

Phone verification as an OOB request type (`requestType:
PHONE_VERIFICATION`) answers `501` naming phone auth as the non-goal, alongside
the rest of the Phase L gaps. Nothing sends email. Tokens remain unsigned and
labelled as such in the code, the CLI `--help`, the startup log, and
`docs/local/README.md`.

### Phase 3 (SAK-166) notes — Datastore API on the shared store

**New Google artefacts:** the public `google.datastore.v1.Datastore` service
proto and its import closure, re-pinned to the same googleapis commit as
Firestore (clears the ledger debt on the previously unpinned Datastore entity /
query / admin copies). No new Rust crates. One new **test-only** npm package
(`@google-cloud/datastore`).

#### What the implementation was written from

| Source | Status | How it was used |
|---|---|---|
| Public `google.datastore.v1` protos @ `57aa4dbc` | Apache-2.0 googleapis | Service RPCs, Entity/Key/Value/Mutation/Query wire types |
| Public Cloud Datastore API reference | Published Google documentation | Lookup/RunQuery/Commit/BeginTransaction/AllocateIds behaviour |
| `DATASTORE_EMULATOR_HOST` | Documented emulator environment variable (explicitly allowed) | Official Node client points at our port |
| Seed mapping already in `src/local/seed.rs` | First-party | Kind/id path elements → Firestore collection/document segments |

**No reverse engineering.** The Firebase Emulator Suite / Datastore emulator
binaries were not decompiled or consulted. Kind → collection mapping reuses the
export-seed path Fireup already owned.

#### Kind mapping and fidelity limits

- A Datastore key path `Kind/id` (and ancestors) becomes
  `projects/{p}/databases/{d}/documents/{Kind}/{id}/…` in the shared store, so
  Firestore and Datastore see the same rows.
- Datastore's empty `database_id` is normalised to Firestore's `(default)`.
- Non-default **namespaces** are refused (`UNIMPLEMENTED`), matching seed.
- Digit-only document ids round-trip as numeric Datastore ids; a string name
  that happens to be all digits is therefore indistinguishable after a store
  round-trip.
- GQL, aggregations, projection queries, cursors, property transforms, and
  nested property masks answer `UNIMPLEMENTED`.

### Phase 4 (SAK-167) notes — glue docs and the end-to-end demo

**No new Google or Firebase artefact, no new proto, no new Rust dependency, and
no new npm package.** The proto pin is unchanged at googleapis `57aa4dbc`. The
phase adds documentation (`docs/local/END_TO_END.md`, README positioning) and a
demo (`scripts/demo-export-to-sql.sh`, `demo/`).

`demo/package.json` declares `@google-cloud/firestore` and
`@google-cloud/datastore` at the same version ranges the smoke tests already
use, re-scoped in the rows above. They remain Google's own Apache-2.0
**client** libraries, installed on demand, used only to drive our server, and
never vendored, linked, or redistributed. No new third-party code enters the
build; the demo is not part of the Rust build at all.

#### Positioning claims introduced in this phase

The compliance rule governs what the project may claim, so the new README
positioning is recorded here rather than left to marketing:

| Claim | Basis |
|---|---|
| "Independent implementation of public Google API contracts" | Already the standing claim; repeated verbatim in `README.md`, `docs/local/README.md`, and the `GET /` discovery document |
| "Not the Firebase Emulator Suite, not affiliated with or endorsed by Google" | Required by the rule; now stated in the top-level `README.md` too, which previously said nothing about the local data plane at all |
| "Complements floci-gcp rather than replacing it" | A statement about Fireup's own scope — deep on export fidelity and SQL, not wide on the cloud surface. No claim is made about floci-gcp's internals, licence, or behaviour, and nothing of theirs was inspected, vendored, or copied. The port choice (`4589`, beside their `4588`) is a collision-avoidance decision, documented in `docs/local/README.md`. |
| "Auth tokens are unsigned; no Google key material ships" | Phase L notes above; unchanged |

**Documentation correction made in this phase.** The previous seeding note in
`docs/local/README.md` said the export parser "erases Firestore value types …
`{"timestampValue": …}` becomes a plain JSON string". That was wrong about our
own first-party parser: it emits a *tagged map* (`__fireup_type__`, defined in
`src/types.rs`) which `fireup import` consumes to pick `bytea`/`timestamptz`/
`point`/`text`. `fireup local seed` ignored the tag, which was the actual gap.
Both that file and `src/local/seed.rs` were corrected to describe the real
behaviour. No code behaviour changed in this phase; the gap itself was closed in
SAK-174 below.

### SAK-174 notes — seeding restores Datastore value types

**No new Google or Firebase artefact, no new proto, no new Rust dependency, and
no new npm package.** The proto pin is unchanged at googleapis `57aa4dbc`.

`src/local/seed.rs` now reads the first-party `__fireup_type__` tags
`src/types.rs` defines and restores `bytesValue`/`timestampValue`/
`geoPointValue`/`referenceValue`. Both the tag vocabulary and the target value
types are already ours or already vendored: the tags are Fireup's own invention
for JSON that has no native form for a Datastore blob, timestamp, geo point, or
key, and the value types come from the public `google/firestore/v1/document.proto`
already in the ledger. The restore reuses `src/local/value_json.rs`, so no new
parsing of any Google format was added.

The demo's SDK half (`demo/read-with-sdk.mjs`) now asserts the four real types
instead of asserting the gap. It still uses Google's own open-source
`@google-cloud/firestore` purely as a **client** against our server, which is the
documented, non-reverse-engineering way to verify API compatibility. No version
range changed.


### SAK-196 notes — repo licence decision

**No new Google or Firebase artefact, no new proto, no new Rust dependency, and
no new npm package.** Declares first-party Fireup code as **Apache-2.0** (root
`LICENSE` + `Cargo.toml` `license = "Apache-2.0"`), records the
`cargo license` audit in `docs/licenses/CARGO_LICENSE_AUDIT.md`, and carves
brand/trademark assets out under `brand/LICENSE`. Vendored googleapis protos
remain Apache-2.0 at the existing pin; this change only settles the previously
undeclared (and inconsistently MIT-labelled) first-party grant.

### SAK-190 notes — Firestore field transforms

**No new Google or Firebase artefact, no new proto, no new Rust dependency, and
no new npm package.** The proto pin is unchanged at googleapis `57aa4dbc`;
`google/firestore/v1/write.proto`, already rowed above, is where
`DocumentTransform.FieldTransform` lives, and it has been compiled into the tree
since Phase 0 — this change serves four of its cases instead of refusing them.

#### What the implementation was written from

| Source | Status | How it was used |
|---|---|---|
| The comments on `DocumentTransform.FieldTransform` in the vendored `write.proto` | Published Apache-2.0 googleapis definition, already in this ledger | The normative semantics: `increment` setting a missing or non-numeric field to the operand, double contagion, integer overflow resolving to the largest-magnitude integer, the array transforms treating a non-array field as empty, cross-type numeric equality with `NaN == NaN` and `null == null`, the array transforms' `null` transform result, and `REQUEST_TIME` being millisecond-precision and identical across one transaction |
| Public Firestore `FieldValue` documentation | Published Google documentation | Which sentinels clients actually emit, which is why `maximum`/`minimum` are refused rather than implemented |
| Behaviour of Google's **open-source** `@google-cloud/firestore` client (Apache-2.0) | Open source, public repository | Which wire shape the SDK picks for a sentinel (a masked `update` plus `update_transforms`), verified by running it as a **client** against our server in `tests/local-smoke/smoke.mjs` |

**No reverse engineering.** The Firebase Emulator Suite was not decompiled,
disassembled, or binary-analysed, and no proprietary emulator source or data was
copied or consulted. What is not served stays an explicit `UNIMPLEMENTED` or
`INVALID_ARGUMENT`: `maximum`, `minimum`, a transform on a nested field path,
and the Datastore spelling (`Mutation.property_transforms`).

### SAK-195 notes — managed-export fidelity regression fixtures

**No new Google or Firebase artefact, no new proto, no new Rust dependency, and
no new npm package.** The proto pin is unchanged at googleapis `57aa4dbc`. This
change adds test fixtures and tests, plus fixes to first-party decode/import
code in `src/leveldb_parser/`, `src/schema_analyzer/` and `src/data_importer/`.

**The fixtures are generated, not obtained.**
`tests/fixtures/generate_edge_export.py` writes both `export-edge-*` trees from
scratch, the same way `generate_datastore_export.py` already writes the base
trees. **No Google test suite, sample export, or fixture was used**, and no
export was obtained from a Google service to copy. An independent reader written
from the published LevelDB log spec
(`tests/support/export_log_reader.rs`, plus `verify_datastore_export.py`) checks
them, so the fixtures stand on the spec rather than on any artefact of Google's.

#### What the generator was written from

| Source | Status | How it was used |
|---|---|---|
| `storage_onestore_v3.EntityProto` field numbering already documented in this repo — the header comment of `src/leveldb_parser/entity_proto.rs` and the table in `tests/fixtures/README.md` | First-party Fireup documentation, written in SAK-139/141 from public sources and already covered by the `src/leveldb_parser/` row below | The legacy payload encoding this change extends: `Reference.name_space = 20` for the namespace partitions and `EntityProto.raw_property = 15` for the unindexed property |
| The vendored `google/datastore/v1/entity.proto` | Published Apache-2.0 googleapis definition, already rowed in this ledger | `PartitionId.namespace_id = 4`, for the `export-edge-v1` tree's namespaced keys |
| `leveldb/db/log_format.h` (the LevelDB log record format) | Public, BSD-licensed, already the basis of `src/leveldb_parser/` | 32 KiB blocks and the `FIRST`/`MIDDLE`/`LAST` fragmenting this change exercises for the first time |
| Public PostgreSQL documentation on identifier length (`NAMEDATALEN`) and `numeric` input (`NaN`, `Infinity`) | Public, not Google | The two fixes that depend on what the server accepts |

**No reverse engineering.** The Firebase Emulator Suite and the Cloud SDK were
not decompiled, disassembled, binary-analysed, or run to derive the managed
export layout. The directory shapes exercised here — `output-N` shards and
`namespace_<ns>/kind_<Kind>/` partitions — are from the published
`gcloud datastore export` / `gcloud firestore export` documentation.

**Fidelity limits this change records rather than hides.** A Datastore double is
mapped to PostgreSQL `numeric`, which has no negative zero, so `-0.0` survives
parsing but reads back as `0`; the fixture and test are in the tree and the gap
is filed as SAK-202. `maximum`/`minimum`-style gaps from prior phases are
unaffected.

### Phase 2.1 (SAK-189) notes — Firestore `Listen` / realtime listeners

**No new Google or Firebase artefact, no new proto, no new Rust dependency, and
no new npm package.** The proto pin is unchanged at googleapis `57aa4dbc`:
`firestore.proto` (`ListenRequest`, `ListenResponse`, `Target`, `TargetChange`)
and `write.proto` (`DocumentChange`, `DocumentDelete`, `DocumentRemove`,
`ExistenceFilter`) were already vendored and compiled, and this phase only
starts serving the RPC that uses them. The change feed uses `tokio`'s
`broadcast`/`watch` channels, already direct dependencies.

#### What the implementation was written from

| Source | Status | How it was used |
|---|---|---|
| Vendored `google/firestore/v1/firestore.proto` and `write.proto` | Public Apache-2.0 googleapis, already rowed above | The `Listen` message shapes, the five `TargetChangeType` values, and the documented meaning of each — including that `ADD`/`CURRENT`/`RESET` must eventually reach a consistent snapshot while `NO_CHANGE`/`REMOVE` need not |
| Public Firestore API reference for `Listen` | Published Google documentation | Target identity rules, `once`, the `resume_token`/`read_time` resume contract, and the `DocumentDelete` vs `DocumentRemove` distinction |
| Behaviour of Google's **open-source** client SDK (`@google-cloud/firestore`, Apache-2.0) | Open source, public repository | Confirmation that a client delivers a snapshot on `NO_CHANGE` with empty `target_ids` and a `read_time`, which is why the server emits that whole-stream marker. Read from the public source and verified by driving the published SDK as a **client**; nothing was decompiled |

**No reverse engineering.** The Firebase Emulator Suite was not run,
decompiled, disassembled, or observed to derive any of this, and no Google test
suite or fixture was used. The design choices are Fireup's own: an in-process
`broadcast` feed, a `Store` decorator as the publish point, and per-change
re-evaluation of a query target. Where the behaviour is known to differ from
production Firestore — the in-process-only feed, the per-change rescan, no
resume tokens, no `ExistenceFilter` — the difference is published in the
fidelity matrix in `docs/local/README.md` rather than left for a user to find.

`@google-cloud/firestore` stays test-only and un-vendored. The smoke test now
drives `onSnapshot()` on a document and on a query, still purely as a **client**
against our server. No version range changed.

## Existing Fireup notes (pre-local-cloud)

| Dependency | Version / pin | SPDX | Upstream | How we use it | Attribution OK? | Decision |
|---|---|---|---|---|---|---|
| `prost` / `prost-types` / `prost-build` | see `Cargo.toml` | Apache-2.0 | crates.io | Protobuf decode/codegen for export EntityProto / Datastore entities | Yes (crate license) | Public OSS; not Google proprietary emulator code |
| Vendored / hand-written wire decoding in `src/leveldb_parser/` | first-party | Apache-2.0 (repo) | Fireup | Managed export parsing | N/A | First-party; based on public LevelDB / documented export formats — not RE of Firebase Emulator Suite |

## Compliance checklist, per phase

The PR checklist in `.cursor/rules/firebase-gcp-compliance.mdc`, answered as each
phase landed.

| Checklist item | Phase 0–2 | Phase L (SAK-168) | Phase 3 (SAK-166) | Phase 4 (SAK-167) | SAK-174 | SAK-196 | SAK-190 | Phase L.1 (SAK-193) | SAK-195 | Phase 2.1 (SAK-189) |
|---|---|---|---|---|---|---|---|---|---|---|
| No RE of proprietary binaries | Yes | Yes — public REST docs, `FIREBASE_AUTH_EMULATOR_HOST`, open-source clients | Yes — public Datastore protos + docs + `DATASTORE_EMULATOR_HOST` | Yes — docs and a demo only; nothing inspected | Yes — first-party tags, already-vendored public protos | Yes — licence text / audit only; no binaries inspected | Yes — the vendored `write.proto` comments, public `FieldValue` docs, open-source client as a client | Yes — public REST + email-action-link docs, and the open-source client SDK's published link parser; no page markup or asset reproduced | Yes — fixtures generated first-party from the public LevelDB log spec and already-documented `EntityProto` numbering; no Google fixture or export used | Yes — vendored public protos, public `Listen` reference, open-source client source |
| New deps/protos logged in this ledger | Yes | Yes (`base64`, `sha2`; no new protos) | Yes (`datastore.proto` + closure; `@google-cloud/datastore` test-only) | Yes — none added; the two npm client rows re-scoped to cover `demo/` | Yes — none added | Yes — none added; first-party SPDX corrected to Apache-2.0; audit linked | Yes — none added | Yes — none added, of any kind | Yes — none added | Yes — none added |
| For a REST/JSON surface: public sources named | n/a / prior phases | Yes | n/a (protos) | n/a | n/a | n/a — no new surface | n/a (protos) | Yes — the four sources are rowed in the Phase L.1 notes | n/a — binary export format, sources named above | n/a (protos) |
| Undocumented behaviours deferred / listed unsupported | Yes | Yes — phone/MFA/`v2`/federated → `501` (OOB too, until SAK-193) | Yes — GQL, aggregations, namespaces, cursors, transforms → `UNIMPLEMENTED` | Yes — the seed value-type gap is now stated accurately and demonstrated, not hidden | Yes — the gap is closed; namespaces still skipped, an unrestorable payload still refused | n/a | Yes — `maximum`/`minimum`, nested transform paths, and Datastore property transforms stay refused | Yes — `PHONE_VERIFICATION` → `501`; mail delivery stated as a non-goal rather than faked; the one-hour code lifetime and the landing route labelled as Fireup's own choices | Yes — the `numeric` negative-zero gap is stated in the fixture README and filed as SAK-202, not hidden | Yes — a `read_time` resume is refused per target; no resume tokens and no `ExistenceFilter`, both published in the matrix |
| Independent-implementation blurb preserved where relevant | Yes | Yes | Yes | Yes — and added to the top-level `README.md`, which previously had none | Yes — unchanged | Yes — retained in README licence section | Yes — unchanged | Yes — and the landing route's own response carries a "not a Google page" notice | Yes — unchanged | Yes — on `src/local/watch.rs` and the `Listen` row of `src/local/firestore.rs` |
| No production key material | n/a | Yes — unsigned tokens; no signing keys shipped | n/a | n/a — nothing added | n/a — nothing added | n/a — nothing added | n/a — nothing added | Yes — no signing key; the link's `apiKey` placeholder is a sentence, not a key-shaped string | n/a — nothing added | n/a — nothing added |

## How to add a row

1. Name the crate, npm package, or proto path.
2. Record exact version or git commit of the upstream pin.
3. Record SPDX and link.
4. State use: generated stubs only / runtime / test client / docs.
5. Confirm NOTICE/COPYING obligations.
6. One-line decision (“public Apache-2.0 googleapis pin”).
