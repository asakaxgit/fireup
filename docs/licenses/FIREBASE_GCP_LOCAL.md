# Fireup — Firebase / GCP local & API license ledger

**Status:** current as of Phase 2 (SAK-165) and Phase L (SAK-168) — Firestore queries/transactions and Firebase Auth / Identity Platform  
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
| `google/datastore/v1/entity.proto`, `query.proto`, `datastore/admin/v1/admin.proto` | pre-existing copies, pin not recorded at the time | Apache-2.0 | [googleapis](https://github.com/googleapis/googleapis/tree/master/google/datastore) | Datastore export parsing (migrator path) | Yes (headers intact) | **Debt:** not yet moved onto the pinned fetch. Add them to `ROOTS` in `scripts/vendor-googleapis-protos.sh` when the Datastore surface is next touched. |

### Rust dependencies added for `fireup local`

| Dependency | Version / pin | SPDX | Upstream | How we use it | Attribution OK? | Decision |
|---|---|---|---|---|---|---|
| `tonic` | 0.11 | MIT | [crates.io](https://crates.io/crates/tonic) | gRPC server + generated client | Yes (crate license) | Public OSS, not Google code. Pinned to the 0.11 line because it tracks `prost` 0.12, which the export parser already uses — one prost version in the tree. |
| `tonic-build` | 0.11 | MIT | [crates.io](https://crates.io/crates/tonic-build) | build-time service codegen from the public protos | Yes | Same line as `tonic` |
| `tokio-stream` | 0.1 (`net`) | MIT | [crates.io](https://crates.io/crates/tokio-stream) | `ReceiverStream` for handing multiplexed connections to tonic and for server streaming | Yes | Public OSS |
| `async-trait` | 0.1 | MIT OR Apache-2.0 | [crates.io](https://crates.io/crates/async-trait) | async methods on the `Store` trait (`src/local/store.rs`) | Yes (crate license) | Public OSS. Added in Phase 1 as a **direct** dependency; it was already in the tree via `tonic`, so no new third-party code enters the build. |
| `@google-cloud/firestore` (npm, **test-only**) | ^7.11.0 | Apache-2.0 | [npm](https://www.npmjs.com/package/@google-cloud/firestore) | `tests/local-smoke/` SDK smoke test only; never linked or redistributed | Yes | Google's own *open-source* client library, used as a client against our server — the documented, non-RE way to verify API compatibility. Not vendored; installed on demand by `scripts/smoke-local-firestore.sh`. |
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
Identity Platform `v2` surface, federated providers, and out-of-band email flows
all answer `501` with a message naming the gap. Reading Google's own
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

## Existing Fireup notes (pre-local-cloud)

| Dependency | Version / pin | SPDX | Upstream | How we use it | Attribution OK? | Decision |
|---|---|---|---|---|---|---|
| `prost` / `prost-types` / `prost-build` | see `Cargo.toml` | Apache-2.0 | crates.io | Protobuf decode/codegen for export EntityProto / Datastore entities | Yes (crate license) | Public OSS; not Google proprietary emulator code |
| Vendored / hand-written wire decoding in `src/leveldb_parser/` | first-party | MIT (repo) | Fireup | Managed export parsing | N/A | First-party; based on public LevelDB / documented export formats — not RE of Firebase Emulator Suite |

## Compliance checklist, per phase

The PR checklist in `.cursor/rules/firebase-gcp-compliance.mdc`, answered as each
phase landed. A row is only "yes" if someone can check it from the repository.

| Checklist item | Phase 0 (SAK-163) | Phase 1 (SAK-164) | Phase L (SAK-168) |
|---|---|---|---|
| No RE of proprietary binaries | Yes | Yes | Yes — written from public REST docs, the documented `FIREBASE_AUTH_EMULATOR_HOST` convention, and Google's Apache-2.0 open-source *clients*; no emulator binary inspected |
| New deps/protos logged in this ledger | Yes (`tonic`, `tonic-build`, `tokio-stream`, googleapis protos) | Yes (`async-trait`) | Yes (`base64`, `sha2`; no new protos — the surface is REST/JSON) |
| Undocumented behaviours deferred / listed unsupported | Yes (queries, transactions, listeners → `UNIMPLEMENTED`) | Yes | Yes — phone auth, MFA/`v2`, federated providers, OOB email → `501` naming the gap; listed in `docs/local/README.md` |
| Independent-implementation blurb preserved where relevant | Yes (`GET /`, CLI `--help`, module docs) | Yes | Yes (`src/local/auth/mod.rs`, CLI `--help`, startup log, README) |
| No production key material | n/a | n/a | Yes — tokens are unsigned; nothing in the repo is or generates a signing key, certificate, or API key |

## How to add a row

1. Name the crate, npm package, or proto path.
2. Record exact version or git commit of the upstream pin.
3. Record SPDX and link.
4. State use: generated stubs only / runtime / test client / docs.
5. Confirm NOTICE/COPYING obligations.
6. One-line decision (“public Apache-2.0 googleapis pin”).
