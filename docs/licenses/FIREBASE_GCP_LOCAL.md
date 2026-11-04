# Fireup — Firebase / GCP local & API license ledger

**Status:** stub (created with SAK-144 project rule)  
**Rule:** `.cursor/rules/firebase-gcp-compliance.mdc` (always apply)

Independent implementation over **public** Google API contracts. Not affiliated with Google. Do not reverse-engineer proprietary Firebase/Google emulator binaries.

Update this ledger on every PR that adds Google/Firebase-related dependencies, protos, or generated stubs.

## Entries

| Dependency | Version / pin | SPDX | Upstream | How we use it | Attribution OK? | Decision |
|---|---|---|---|---|---|---|
| _(none yet for local-cloud)_ | — | — | — | — | — | Phase 0 will pin public `googleapis` Firestore/Datastore protos here |

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
