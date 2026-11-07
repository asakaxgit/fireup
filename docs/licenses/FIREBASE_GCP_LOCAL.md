# Fireup — Firebase / GCP local & API license ledger

> First-party Fireup code is licensed under **Apache-2.0** (see root `LICENSE`). Brand assets under `brand/` are carved out — see `brand/LICENSE`. Dependency-tree audit: [`CARGO_LICENSE_AUDIT.md`](CARGO_LICENSE_AUDIT.md).

**Status:** current as of SAK-220 — the gRPC stack moved from `tonic` 0.11 / `prost` 0.12 to `tonic` 0.13 / `prost` 0.13, which replaces the old HTTP/2 transport chain (`axum` 0.6 → `hyper` 0.14 → `h2` 0.3.27) with `axum` 0.8 → `hyper` 1.12 → `h2` 0.4.20 and so clears `RUSTSEC-2026-0258`; the accepted-vulnerability entry was deleted from `deny.toml` in the same change, leaving `[advisories] ignore` empty. These are version bumps inside the existing transitive gRPC transport, not new third-party components, and every replacement crate is MIT and/or Apache-2.0 — machine-checked by `cargo deny --locked check ... licenses` (`licenses ok`) against the permissive-only allow list, not asserted by hand. No Google/Firebase proto or generated stub changed; the same two service protos are compiled, by a newer `tonic-build`. Prior: SAK-217 — the comparison's emulator column replaced with **measured** conformance results (the first entry covering the *running* of third-party software). Prior: SAK-218 — `deny.toml` turns the permissive-only dependency policy into a CI gate instead of a periodic manual audit, and records the one accepted vulnerability (`RUSTSEC-2026-0258` in `h2`, via the pinned `tonic` 0.11); see [`CARGO_LICENSE_AUDIT.md#how-this-is-enforced`](CARGO_LICENSE_AUDIT.md#how-this-is-enforced); SAK-216 — `Cargo.lock` committed and `--locked` restored in CI, which turns every "see `Cargo.toml`" pin in this ledger from a semver range into one exact version per commit; SAK-206 README badges, coverage, and the comparison document (the first entry covering claims about third-party software); SAK-203 Datastore `property_mask` on a mutation (masked entity writes, paired with the SAK-198 property transforms); Phase 2.1 (SAK-189) Firestore `Listen` / realtime listeners; SAK-195 managed-export fidelity regression fixtures; SAK-193 Phase L.1 out-of-band email flows (no mail transport); SAK-190 Firestore field transforms; SAK-196 first-party Apache-2.0 + brand trademark carve-out with the dependency audit recorded; SAK-174 seeding restores Datastore value types; Phase 4 (SAK-167) glue docs and the end-to-end demo; Phase 3 (SAK-166) Datastore and Phase L (SAK-168) Auth
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
`INVALID_ARGUMENT`: `maximum`, `minimum`, and a transform on a nested field
path. (The Datastore spelling, `Mutation.property_transforms`, was refused when
this note was written and is served as of SAK-198; SAK-203 below adds the
`property_mask` it pairs with.)

### SAK-205 notes — timestamp range in the stored value encoding

**No new Google or Firebase artefact, no new proto, no new Rust dependency, and
no new npm package.** The proto pin is unchanged at googleapis `57aa4dbc`.

The years 0001–9999 bound this change enforces is **published**, not inferred
from any emulator. It is the documented range of `google.protobuf.Timestamp`
itself — `0001-01-01T00:00:00Z` to `9999-12-31T23:59:59.999999999Z`, carried
into this tree in the `prost-types` doc comment on `Timestamp`
(Apache-2.0, already rowed above) — and it matches the timestamp range in the
public Firestore data-types documentation. Nothing was inspected, run, or
reverse-engineered to find it; the previous behaviour was simply wider than the
format it writes.

**Fidelity limits this change records rather than hides.** A timestamp outside
that range now fails the write with `INVALID_ARGUMENT` in both stores instead
of being written and then failing every read. Payloads a previous build wrote
with a signed five-digit year remain readable, which is a deliberately wider
read path than write path and is documented as such in
`src/local/value_json.rs` and `docs/local/README.md`.

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

### SAK-192 notes — PostgreSQL pushdown for equality/range query filters

No new dependency, no new proto, no new vendored code. What is new is first-party
SQL that applies a subset of a Firestore query's predicates in the database
before the rows reach the in-process engine.

**Sources.** Firestore's published query documentation and the public
"sort order of Firestore values" reference, plus the `googleapis` protos already
vendored under `proto/` and already rowed in this ledger. The value-ordering
source is named in `src/local/query.rs`, which is the one definition of those
semantics; `src/local/postgres_filter.rs` only restates a *subset* of it in SQL.
No Google emulator binary was inspected, and no query-planner behaviour was
reconstructed — there is no planner here to clone. Deciding *what* a query means
stays in Rust; the SQL only decides what to read.

**Why that boundary matters for this rule.** The translation is deliberately
sound-but-not-exact: it must return a superset of the matching rows and may
return extra ones, and `Query::evaluate` still decides every result. A predicate
the translation cannot express faithfully is not expressed at all — the row falls
through to the engine. So the pushdown cannot introduce behaviour that was not
already derived from the public contract, and it cannot quietly diverge from the
in-memory backend. `tests/local_query_pushdown_tests.rs` enforces that by running
49 query shapes through both paths and requiring identical answers.

**Gaps stated rather than hidden**, per the "undocumented behaviours deferred"
item: every predicate that stays in-process (`!=`, `not-in`, the array
operators, `==` on arrays and maps, bytes ranges, ranges on null/geo/array/map,
an `OR` with one untranslatable branch, limit/offset/cursors, and the whole
Datastore query surface) is tabulated with its reason in `docs/local/README.md`
and in the module docs. The index added to the schema is a GIN index on the
existing `fields` column; no index is implied to the client, and the "no indexes
and no index errors" limit in the fidelity matrix is unchanged — production
Firestore still rejects queries this serves.

**Defect found, not fixed:** `value_json` cannot decode its own encoding of a
timestamp outside years 0001–9999. Filed as SAK-205 rather than silently
narrowed or worked around.
### SAK-203 notes — Datastore `property_mask` on a mutation

**No new Google or Firebase artefact, no new proto, no new Rust dependency, and
no new npm package.** The proto pin is unchanged at googleapis `57aa4dbc`;
`google/datastore/v1/datastore.proto`, already rowed above, is where
`Mutation.property_mask` and `PropertyMask` live, and both have been compiled
into the tree since Phase 3 (SAK-166) — this change serves the field instead of
refusing it, completing the pairing SAK-198 started.

#### What the implementation was written from

| Source | Status | How it was used |
|---|---|---|
| The comments on `Mutation.property_mask` and `PropertyMask` in the vendored `datastore.proto` | Published Apache-2.0 googleapis definition, already in this ledger | Every rule the implementation encodes, quoted in the code and in `docs/local/README.md`: only masked properties are updated and others are left untouched; "properties referenced in the mask but not in the entity are deleted"; "none of the properties in the mask may have a reserved name, except for `__key__`"; "this field is ignored for `delete`"; "a path must not be empty"; dots separate nested property names and `.`/`\` in a name must be escaped; and — from `property_transforms` on the same message — that the transforms apply "regardless of the property mask … after the operation" |
| The vendored `google/firestore/v1/write.proto` (`Write.update_mask`) | Published Apache-2.0 googleapis definition, already rowed above | The equivalent Firestore rule, which is why both surfaces now share one masked-write primitive in `src/local/store.rs` rather than two readings of the same sentence |
| Behaviour of Google's **open-source** `@google-cloud/datastore` client (Apache-2.0) | Open source, public repository | Confirmation that a masked write round-trips as the official library encodes it, verified by running it as a **client** against our server in `tests/local-smoke-datastore/smoke.mjs`. As in SAK-198, `propertyMask` has no handwritten surface, so the mask is sent through the generated `DatastoreClient` the same package exports and read back through the handwritten `datastore.get()` |

**No reverse engineering.** The Firebase Emulator Suite and the Cloud Datastore
emulator were not decompiled, disassembled, binary-analysed, or run to derive
this behaviour, and no proprietary emulator source or data was copied or
consulted. Nothing was inferred from observing a Google service: every rule
above is a sentence in the published proto.

**Fidelity limits this change records rather than hides.** A nested mask path
(`foo.bar`) and an escaped property name stay `UNIMPLEMENTED` on both surfaces —
a nested mask has to merge *into* a stored map, and overwriting the whole parent
property would be worse than refusing. An empty path and a reserved name other
than `__key__` are `INVALID_ARGUMENT`. `docs/local/README.md` carries the full
table, every row of it is probed by the SAK-194 conformance suite against the
official SDKs, and this change closes the previously recorded "no field deletion
through a mask" limit rather than leaving it half-true — the row SAK-194 had
pinned as a `documented-divergence` is now a served row with a probe that
asserts the deletion.

### SAK-206 notes — README badges, coverage, and the comparison document

**No new Google or Firebase artefact, no new proto, no new Rust dependency, and
no new npm package.** This change adds CI workflows, a coverage script, README
badges, and `docs/local/COMPARISON.md`. It is the first change in this
repository whose deliverable is a **statement about other people's software**,
so the sourcing rules matter more than usual, not less.

#### Scope: Google's products only, by decision

The comparison was first drafted with a fourth column for a third-party
local-cloud emulator. **The CEO's call on SAK-206 was to omit it for now**, so
the document compares Fireup against Google's Firebase Emulator Suite, Google's
`gcloud` Datastore emulator, and the production service — all three of them the
thing Fireup is an independent implementation *of*, and all three documented by
the same vendor whose API contracts we implement. That keeps this ledger's
subject matter where it has always been.

The document says in prose that other third-party emulators exist and are not
compared yet, without naming or characterising any of them. Re-adding a
competitor column is gated on running a conformance suite against it rather than
reading its README (SAK-217), and on a fresh publication decision.

#### What the comparison was written from

| Source | Status | How it was used |
|---|---|---|
| `docs/local/README.md` and `tests/conformance/matrix.json` | First-party | The entire Fireup column. It is not re-typed: `cargo test --test fidelity_matrix_tests` compares every comparison row whose label matches a matrix row, so the two cannot diverge |
| [Firebase Emulator Suite docs](https://firebase.google.com/docs/emulator-suite), [Firestore emulator](https://firebase.google.com/docs/emulator-suite/connect_firestore), [Auth emulator](https://firebase.google.com/docs/emulator-suite/connect_auth), [Datastore emulator](https://cloud.google.com/datastore/docs/tools/datastore-emulator) | Public Google documentation | The emulator columns, including short verbatim quotations of documented limitations |
| [Firestore documentation](https://firebase.google.com/docs/firestore) | Public Google documentation | The production column |

**No reverse engineering, and no running of anything.** The Firebase Emulator
Suite was not decompiled, disassembled, binary-analysed, or executed to produce
the comparison. Nothing was observed on the wire. The comparison document says
this about itself, per column, and labels every unverified cell `not documented`
rather than guessing — so a reader can tell which cells are evidence (ours,
probed in CI) and which are a reading of Google's documentation.

**No code or text copied.** Quotations are short factual statements of
documented limitations, each attributed inline and linked in the document's
Sources table. No Google documentation prose was reproduced beyond those
quotations.

**Trademarks are used nominatively.** "Firebase", "Firestore", "Datastore" and
"Google Cloud" appear to identify the products being compared. The comparison
restates the independent-implementation position and links this ledger; the
top-level README's notice is unchanged.

**Claims about Google's emulator are deliberately conservative.** Where it
serves something Fireup refuses — aggregations, streaming `Write`, Security
Rules, Functions triggers — the document says so plainly, in a table and again
in prose, and recommends Google's emulator for those cases. Where Google's docs
do not say, the cell reads `not documented`, never a guess.

#### Tooling

| Tool | Version / pin | SPDX | Upstream | How we use it | Attribution OK? | Decision |
|---|---|---|---|---|---|---|
| `cargo-llvm-cov` | installed in CI, not pinned in `Cargo.toml` | Apache-2.0 OR MIT | crates.io | Measures line coverage in `scripts/coverage.sh`; never linked into the binary | Yes (crate licence) | Developer/CI tool only; not a dependency of the shipped artefact |
| shields.io badge images | n/a — hotlinked URLs | n/a | shields.io | Three static badges in `README.md` (coverage floor, fidelity-matrix count, licence) | n/a | Images rendered by a third party at read time; no code or asset vendored |

### SAK-216 notes — `Cargo.lock` committed, so this ledger becomes checkable

**No new dependency of any kind, and no dependency moved.** `Cargo.lock` was in
`.gitignore`; this change tracks it and restores `--locked` to CI. The lock file
records the 229-crate resolution the repository was already building and testing
against — `cargo metadata --locked` agrees with `Cargo.toml`, so not one version
changed.

**Why it matters here and not only in CI.** Several rows in this ledger record
their pin as "see `Cargo.toml`", which is a semver *range*, not a version. A
reader auditing whether `prost` 0.12.x was the Apache-2.0 release we claim could
not tell which 0.12.x any given build used, and two builds of the same commit
could legitimately differ. With the lock file tracked, every row's "version /
pin" resolves to one exact version per commit, and `--locked` makes CI fail
rather than silently re-resolve. The rule asks for a record that is "auditable
after the PR is merged"; this is the mechanism that makes it so.

| Artefact | Version / pin | SPDX | Upstream | How we use it | Attribution OK? | Decision |
|---|---|---|---|---|---|---|
| `Cargo.lock` | first-party, generated | Apache-2.0 (repo) | Fireup | Pins the exact resolved version of all 229 transitive crates per commit | N/A | First-party generated metadata. Every entry resolves to `registry+https://github.com/rust-lang/crates.io-index`; checked for git dependencies, local path dependencies, and credentials — none present |
| `cargo-deny` | 0.20.2, installed locally, not in CI | Apache-2.0 OR MIT | crates.io | Audited the committed lock file once, for this change; never linked into the binary | Yes (crate licence) | Developer tool. Adding it to CI is [SAK-218](/SAK/issues/SAK-218), not this change — see below |

#### The one known advisory, now fixed rather than carried

`cargo deny check` against the committed lock file reports `advisories ok,
bans ok, licenses ok, sources ok`. Nothing is accepted; `[advisories] ignore` in
`deny.toml` is empty.

It reported one vulnerability when this section was written:
**`RUSTSEC-2026-0258`** in `h2` 0.3.27 (empty `DATA` frames queued without
limit; unbounded memory or a length-overflow panic; low severity), reached as
`fireup → tonic 0.11 → axum 0.6 → hyper 0.14 → h2 0.3`. The fix needed
`h2 >= 0.4.16`, hence hyper 1.x, hence **tonic 0.12+** — and `Cargo.toml` pinned
tonic 0.11 deliberately, so the local data plane and the export parser would
share one `prost` version. That made it a migration rather than a lock-file bump,
so [SAK-218](/SAK/issues/SAK-218) recorded the acceptance with a review date and
[SAK-220](/SAK/issues/SAK-220) did the migration: tonic 0.13 / prost 0.13, which
makes the chain `fireup → tonic 0.13 → axum 0.8 → hyper 1.12 → h2 0.4.20`, still
on a single `prost`. The `ignore` entry went with it, which
`-D advisory-not-detected` in the CI job required rather than merely encouraged.

`cargo-audit` is not installed in this workspace and overlaps
`cargo deny check advisories`, so only one of the two is worth adopting.
### SAK-217 notes — measuring the comparison instead of reading it

**No new Google or Firebase artefact enters the repository, no new proto, no new
Rust dependency, and no new npm package.** The proto pin is unchanged at
googleapis `57aa4dbc`. What this change adds is a **method**: Google's published
emulator was *executed locally* and its public API was asked questions.
SAK-206's note above says the Firebase Emulator Suite was not "decompiled,
disassembled, binary-analysed, or executed" to produce the comparison. The last
word of that list is superseded here. The first three remain true, and they are
the line that was not crossed.

#### The method

Black-box conformance testing of a **published** API contract, driven by the
vendor's own Apache-2.0 client libraries — the same method SAK-163 established
for verifying Fireup itself, pointed the other way. The official
`@google-cloud/firestore` and `@google-cloud/datastore` libraries already rowed
in this ledger honour the documented `FIRESTORE_EMULATOR_HOST` /
`DATASTORE_EMULATOR_HOST` variables, so a conformance probe reaches any server
implementing the same contract. `tests/conformance/node/observe.mjs` runs the
existing probes and records what came back (`serves` / `refuses` / `diverges` /
`error` / `timeout` / `not-measurable`) instead of asserting Fireup's claimed
status; `scripts/conformance-target.sh` points it at a host.

**No reverse engineering.** Nothing was decompiled, disassembled, or
binary-analysed. No JAR, class file, native image, or container layer was
unpacked or inspected. No code, configuration, test, fixture, or asset was
copied out of the emulator, and nothing from it was vendored here — the
artefacts in `docs/local/measurements/` are first-party output of our own probes
(a row id, a verdict, and the target's own error string). Running a published
binary and writing down what its public API answers is observation; taking it
apart is not, and was not done.

#### What was run

| Target | Artefact | Licence | How it was obtained and run | Measured |
|---|---|---|---|---|
| Firebase Emulator Suite | `cloud-firestore-emulator` **v1.22.0**, downloaded by `firebase-tools` 15.35.0 | Firebase CLI Apache-2.0; the emulator JAR is **closed-source** | `firebase emulators:start --only firestore,auth --project demo-fireup-local` with a Temurin JRE 21, Firestore on `:18080` and Auth on `:19099` (the documented `8080` was occupied on the measuring host). Separately, the **same JAR** run as `java -jar … --host=127.0.0.1 --port=18081 --database-mode=datastore-mode`. `--database-mode` is the JAR's own published CLI flag — listed in its `--help`, and what `gcloud emulators firestore start --database-mode=datastore-mode` sets — which the Firebase CLI does not expose. Invoking a documented flag on a published binary is use, not analysis | 2026-11-06 |
| `fireup local` | this commit, in-memory store, `:4589` | Apache-2.0 (first-party) | The negative control. `observe.mjs` asserts nothing about *who* is answering, so without it a harness bug is indistinguishable from a target divergence | 2026-11-06 |

No request was made to a billable Google service. The "Firestore on GCP" column
of the comparison therefore stays documentation-sourced and carries no measured
cell. The `demo-` project prefix keeps the emulator from attempting to reach any
non-emulated Google service, and no credential of any kind was supplied to it.

#### Scope: still Google's products only

SAK-206's decision above stands unchanged. This change measures Google's
emulator — the thing Fireup is an independent implementation *of* — and does not
add a column for, name, or characterise any third-party emulator. The
comparison's prose still says only that others exist and are not compared here.
The measurement recipe in `tests/conformance/README.md` is deliberately generic,
because the gate on publishing a named third-party comparison is a publication
decision, not a technical capability.

#### What the comparison now claims, and on what basis

`docs/local/COMPARISON.md` marks every cell `[M]` (measured) or `[D]`
(documented), names the emulator version and the date it was measured, and
commits the raw per-row output under `docs/local/measurements/`. What this
changes, in both directions:

| Claim | Direction | Basis |
|---|---|---|
| The Firebase Emulator Suite **does** emulate Datastore; all 20 `not emulated` cells were wrong | Against our previous document | Measured: the JAR answers every Datastore RPC with `INVALID_ARGUMENT: Database mode not set to Datastore Mode`, and serves 20/20 measurable Datastore rows under `--database-mode=datastore-mode`, including GQL, aggregations, every property transform, and every `property_mask` row |
| It **refuses** `PartitionQuery`, with the same gRPC status Fireup uses | In Fireup's favour | Measured: `UNIMPLEMENTED: Method google.firestore.v1.Firestore/PartitionQuery is unimplemented` |
| SAK-203's `property_mask` reading of `datastore.proto` was correct | In Fireup's favour | Measured: Fireup and the reference implementation agree on all six measurable mask rows |
| Four bugs in **Fireup** | Against us | Measured against the reference implementation; stated in the comparison with the emulator's verbatim error, and filed as follow-up issues |

The four Fireup bugs are worth naming here, because two of them are claims this
project published about itself and got wrong: `fireup local` accepts an
`order_by` with a term after `__name__` and serves a descending `__name__` scan,
both of which the emulator and production reject — and the second has a
conformance probe *asserting* the divergent behaviour. A probe stops a row
changing silently; it cannot stop a row from having been wrong when it was
written. On the auth surface, `accounts:batchCreate` answered `405` with a
self-contradictory message and `projects:createSessionCookie` answered
`404 PROJECT_NOT_FOUND` from mis-parsing the `{project}:customMethod` colon, so
two rows the matrix correctly calls `UNIMPLEMENTED` refused in a shape that did
not name the gap. SAK-224 fixed both; the replacement refusals were
re-measured against `main` at `5d015dc` on 2026-11-06 rather than transcribed
from the fix, and both now answer `501 OPERATION_NOT_ALLOWED` naming the
missing capability.

**Trademarks are used nominatively**, as before: "Firebase", "Firestore",
"Datastore" and "Google Cloud" appear only to identify the products being
compared. The independent-implementation notice at the top of `COMPARISON.md` is
preserved and extended to describe the measurement method and the line it does
not cross.

#### Tooling used to run the target

Neither is a dependency of anything this project ships; both were fetched into a
scratch directory outside the repository and neither is vendored.

| Tool | Version | SPDX | Upstream | How we use it | Decision |
|---|---|---|---|---|---|
| Eclipse Temurin JRE/JDK | 21.0.13+9 | GPL-2.0-with-classpath-exception | [adoptium.net](https://adoptium.net/) | The JRE Google's emulator documents as a prerequisite ("a Java JRE version 21 or greater") | Public OSS runtime, run outside the repository; nothing links against it |
| `firebase-tools` | 15.35.0 | MIT | [npm](https://www.npmjs.com/package/firebase-tools) | Downloads and starts Google's emulator. The JAR it fetches is closed-source and was **run, never inspected** | Google's own published CLI, used as published |

## Existing Fireup notes (pre-local-cloud)

| Dependency | Version / pin | SPDX | Upstream | How we use it | Attribution OK? | Decision |
|---|---|---|---|---|---|---|
| `prost` / `prost-types` / `prost-build` | see `Cargo.toml` | Apache-2.0 | crates.io | Protobuf decode/codegen for export EntityProto / Datastore entities | Yes (crate license) | Public OSS; not Google proprietary emulator code |
| Vendored / hand-written wire decoding in `src/leveldb_parser/` | first-party | Apache-2.0 (repo) | Fireup | Managed export parsing | N/A | First-party; based on public LevelDB / documented export formats — not RE of Firebase Emulator Suite |

## Compliance checklist, per phase

The PR checklist in `.cursor/rules/firebase-gcp-compliance.mdc`, answered as each
phase landed.

| Checklist item | Phase 0–2 | Phase L (SAK-168) | Phase 3 (SAK-166) | Phase 4 (SAK-167) | SAK-174 | SAK-196 | SAK-190 | Phase L.1 (SAK-193) | SAK-195 | Phase 2.1 (SAK-189) | SAK-192 | SAK-203 | SAK-206 | SAK-216 | SAK-217 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| No RE of proprietary binaries | Yes | Yes — public REST docs, `FIREBASE_AUTH_EMULATOR_HOST`, open-source clients | Yes — public Datastore protos + docs + `DATASTORE_EMULATOR_HOST` | Yes — docs and a demo only; nothing inspected | Yes — first-party tags, already-vendored public protos | Yes — licence text / audit only; no binaries inspected | Yes — the vendored `write.proto` comments, public `FieldValue` docs, open-source client as a client | Yes — public REST + email-action-link docs, and the open-source client SDK's published link parser; no page markup or asset reproduced | Yes — fixtures generated first-party from the public LevelDB log spec and already-documented `EntityProto` numbering; no Google fixture or export used | Yes — vendored public protos, public `Listen` reference, open-source client source | Yes — the SQL translation comes from the public "sort order of Firestore values" and query documentation plus the already-vendored public protos; nothing inspected, and no query planner behaviour reconstructed | Yes — the vendored `datastore.proto` comments only; the open-source client run as a client | Yes — public Google documentation only; the emulator was neither decompiled nor run, and every unverified cell is labelled `not documented` | Yes — no binary, proto, document, or third-party project was inspected at all; the change is a generated lock file and CI flags | Yes — the emulator **run** as a published binary over its public API, driven by Google's open-source client libraries; not decompiled, disassembled, binary-analysed, or unpacked, and nothing copied out of it. The method, version, and date are rowed above |
| New deps/protos logged in this ledger | Yes | Yes (`base64`, `sha2`; no new protos) | Yes (`datastore.proto` + closure; `@google-cloud/datastore` test-only) | Yes — none added; the two npm client rows re-scoped to cover `demo/` | Yes — none added | Yes — none added; first-party SPDX corrected to Apache-2.0; audit linked | Yes — none added | Yes — none added, of any kind | Yes — none added | Yes — none added | Yes — none added | Yes — none added, of any kind | Yes — none added to the build; `cargo-llvm-cov` and the shields.io badge URLs are rowed as CI/docs tooling | Yes — none added and none moved; `Cargo.lock` itself and `cargo-deny` are rowed above, and the one advisory the lock file exposes is recorded there | Yes — none added to the build or the repository; the Temurin JRE and `firebase-tools` 15.35.0 are rowed as scratch-directory tooling |
| For a REST/JSON surface: public sources named | n/a / prior phases | Yes | n/a (protos) | n/a | n/a | n/a — no new surface | n/a (protos) | Yes — the four sources are rowed in the Phase L.1 notes | n/a — binary export format, sources named above | n/a (protos) | n/a (gRPC/protos); the SQL is first-party and the value-ordering source is named in `src/local/query.rs` | n/a (protos) | n/a — no new surface; the comparison's sources are named in its own Sources table and above | n/a — no surface touched. Cargo's own published guidance to commit the lock file for a binary crate is the only source | n/a — no new surface; the measured auth rows were driven over the documented Identity Toolkit REST paths with the documented `Authorization: Bearer owner` emulator convention |
| Undocumented behaviours deferred / listed unsupported | Yes | Yes — phone/MFA/`v2`/federated → `501` (OOB too, until SAK-193) | Yes — GQL, aggregations, namespaces, cursors, transforms → `UNIMPLEMENTED` | Yes — the seed value-type gap is now stated accurately and demonstrated, not hidden | Yes — the gap is closed; namespaces still skipped, an unrestorable payload still refused | n/a | Yes — `maximum`/`minimum`, nested transform paths, and Datastore property transforms stay refused | Yes — `PHONE_VERIFICATION` → `501`; mail delivery stated as a non-goal rather than faked; the one-hour code lifetime and the landing route labelled as Fireup's own choices | Yes — the `numeric` negative-zero gap is stated in the fixture README and filed as SAK-202, not hidden | Yes — a `read_time` resume is refused per target; no resume tokens and no `ExistenceFilter`, both published in the matrix | Yes — every predicate the translation will not push is tabulated **with its reason** in `docs/local/README.md` and `src/local/postgres_filter.rs`, and falls back to the in-process engine rather than guessing. The `value_json` timestamp round-trip defect found on the way is filed as SAK-205, not hidden | Yes — nested mask paths and escaped property names stay `UNIMPLEMENTED`; an empty path and a non-`__key__` reserved name are `INVALID_ARGUMENT` | Yes — the four RPCs and the query/transform rows Fireup refuses are stated as refusals beside a reference implementation that serves them, not hidden | Yes — `RUSTSEC-2026-0258` in `h2` 0.3.27 is stated above with its path and the tonic 0.12 migration it needs, and filed as [SAK-218](/SAK/issues/SAK-218), not hidden | Yes — and the four Fireup bugs the measurement exposed are stated in `COMPARISON.md` with the reference implementation's verbatim error rather than left for a user to hit; the 15 unmeasurable rows are labelled `[D]`, not scored |
| Independent-implementation blurb preserved where relevant | Yes | Yes | Yes | Yes — and added to the top-level `README.md`, which previously had none | Yes — unchanged | Yes — retained in README licence section | Yes — unchanged | Yes — and the landing route's own response carries a "not a Google page" notice | Yes — unchanged | Yes — on `src/local/watch.rs` and the `Listen` row of `src/local/firestore.rs` | Yes — unchanged | Yes — unchanged | Yes — restated at the top of `COMPARISON.md`, with the trademark note; README unchanged | Yes — unchanged | Yes — restated at the top of `COMPARISON.md` and extended to describe the measurement method and the line it does not cross |
| No production key material | n/a | Yes — unsigned tokens; no signing keys shipped | n/a | n/a — nothing added | n/a — nothing added | n/a — nothing added | n/a — nothing added | Yes — no signing key; the link's `apiKey` placeholder is a sentence, not a key-shaped string | n/a — nothing added | n/a — nothing added | n/a — nothing added | n/a — nothing added | n/a — nothing added | Yes — the lock file was checked for credentials, git dependencies, and local path dependencies; none present | n/a — nothing added; no credential was supplied to the emulator, the `demo-` project prefix keeps it from reaching a real service, and no billable Google service was called |

## How to add a row

1. Name the crate, npm package, or proto path.
2. Record exact version or git commit of the upstream pin.
3. Record SPDX and link.
4. State use: generated stubs only / runtime / test client / docs.
5. Confirm NOTICE/COPYING obligations.
6. One-line decision (“public Apache-2.0 googleapis pin”).
