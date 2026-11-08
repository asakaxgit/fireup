# Copilot instructions for Fireup

GitHub Copilot reads this file automatically for every suggestion in this
repository, which is why it must not restate anything. Until SAK-257 it held 355
lines written on 2025-11-22: they described Fireup as a migration tool with
"Standard SQL query support", listed a five-module architecture that left out
`src/local/` — 22,407 of the 42,381 lines in `src/` — and recommended `rayon`, a
dependency no code used. A stale instruction file is worse than no instruction
file: it is read with authority, and nothing fails when it goes out of date.

So this one points at the documents that are kept current, and at the rules that
are enforced.

## What Fireup is

A local Firebase/GCP data plane (`fireup local` — Firestore, Datastore, Auth
over real gRPC and HTTP) plus a Firestore managed-export → PostgreSQL importer.
`src/lib.rs` is the authoritative module list.

Read, in this order:

- [`README.md`](../README.md) — what it is, what it serves, how to run it
- [`docs/local/README.md`](../docs/local/README.md) — the `fireup local`
  surfaces, the storage layout, the fidelity matrix
- [`docs/local/COMPARISON.md`](../docs/local/COMPARISON.md) — which behaviours
  are measured against the Firebase emulator, and which are not
- [`PROJECT_STATUS.md`](../PROJECT_STATUS.md) — what is proven, what is only
  claimed, and the residual risks

## Rules, not advice

- [`AGENTS.md`](../AGENTS.md) and
  [`.cursor/rules/firebase-gcp-compliance.mdc`](../.cursor/rules/firebase-gcp-compliance.mdc)
  — no reverse-engineering of proprietary Google/Firebase binaries, no
  production key material, and every Google/Firebase dependency or proto
  recorded in the licence ledger. Board direction, not style preference.
- [`docs/licenses/FIREBASE_GCP_LOCAL.md`](../docs/licenses/FIREBASE_GCP_LOCAL.md)
  — the ledger a new dependency or vendored proto has to land in.
- [`deny.toml`](../deny.toml) — the permissive-only licence policy and the
  advisory gate, enforced by the `deny` job in [`ci.yml`](workflows/ci.yml). A
  copyleft-only dependency fails the build.

## Before adding a dependency

Check that something in the tree will use it. Four declared crates —
`crc32fast`, `dashmap`, `rayon`, `sqlparser` — were carried for a year because a
design document recommended them, and nothing complains about a dependency no
code imports: not `cargo build`, not the test suite, not `cargo deny`.

## What the checks actually are

[`ci.yml`](workflows/ci.yml) is the gate: `build and test (no external
services)` and `supply chain (cargo deny)`, alongside `actionlint`.
`cargo fmt --check` and `cargo clippy -D warnings` are **not** in `ci.yml` — the
comment at the top of that file says why, and
[`README.md`](../README.md#ci-and-coverage) says what each badge covers. Do not
tell a contributor that a check exists without reading the workflow that would
run it.
