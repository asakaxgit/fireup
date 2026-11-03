# Fireup — Firebase / GCP local & API license ledger

**Status:** current as of Phase 0 (SAK-163)  
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
| `google/firestore/v1/query.proto` | googleapis @ `57aa4dbc` | Apache-2.0 | [link](https://github.com/googleapis/googleapis/blob/57aa4dbce5d23b2115b2732b10dcb7ddc8e8698d/google/firestore/v1/query.proto) | Query types (compiled; `RunQuery` not served in Phase 0) | Yes | Same pin |
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
| `tokio-stream` | 0.1 (`net`) | MIT | [crates.io](https://crates.io/crates/tokio-stream) | `TcpListenerStream` for graceful shutdown; `ReceiverStream` for server streaming | Yes | Public OSS |
| `@google-cloud/firestore` (npm, **test-only**) | ^7.11.0 | Apache-2.0 | [npm](https://www.npmjs.com/package/@google-cloud/firestore) | `tests/local-smoke/` SDK smoke test only; never linked or redistributed | Yes | Google's own *open-source* client library, used as a client against our server — the documented, non-RE way to verify API compatibility. Not vendored; installed on demand by `scripts/smoke-local-firestore.sh`. |

## Existing Fireup notes (pre-local-cloud)

| Dependency | Version / pin | SPDX | Upstream | How we use it | Attribution OK? | Decision |
|---|---|---|---|---|---|---|
| `prost` / `prost-types` / `prost-build` | see `Cargo.toml` | Apache-2.0 | crates.io | Protobuf decode/codegen for export EntityProto / Datastore entities | Yes (crate license) | Public OSS; not Google proprietary emulator code |
| Vendored / hand-written wire decoding in `src/leveldb_parser/` | first-party | MIT (repo) | Fireup | Managed export parsing | N/A | First-party; based on public LevelDB / documented export formats — not RE of Firebase Emulator Suite |

## How to add a row

1. Name the crate, npm package, or proto path.
2. Record exact version or git commit of the upstream pin.
3. Record SPDX and link.
4. State use: generated stubs only / runtime / test client / docs.
5. Confirm NOTICE/COPYING obligations.
6. One-line decision (“public Apache-2.0 googleapis pin”).
