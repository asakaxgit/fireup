# `fireup local` — local Firestore-compatible data plane

Fireup serves Google's **published** `google.firestore.v1` gRPC API on one local
port so official Firestore client SDKs can point at it with the documented
`FIRESTORE_EMULATOR_HOST` environment variable.

This is an independent implementation of a public API contract. It is **not** the
Firebase Emulator Suite, is **not** affiliated with Google, and contains no code
or behaviour derived from proprietary Google binaries. See
`.cursor/rules/firebase-gcp-compliance.mdc` and
`docs/licenses/FIREBASE_GCP_LOCAL.md`.

## Run it

```bash
cargo run --bin fireup -- local              # 127.0.0.1:4589
cargo run --bin fireup -- local --port 9000  # another port
cargo run --bin fireup -- local -v           # per-RPC debug logging
```

Then, in your app:

```bash
export FIRESTORE_EMULATOR_HOST=127.0.0.1:4589
```

Any official SDK (Node, Python, Go, Java, Admin SDKs) honours that variable and
will talk plaintext gRPC to Fireup instead of production Firestore.

### Why port 4589

`4589` is free of the common collisions: Google's Firebase emulator defaults
(8080 Firestore, 4000 UI) and floci-gcp's single port (4588). Fireup sitting at
4589 means you can run it beside either without an argument about ports.

## What Phase 0 serves

| RPC | Status |
|---|---|
| `GetDocument` | served (default consistency only) |
| `CreateDocument` | served, incl. 20-char auto ids |
| `UpdateDocument` | served, with `update_mask` merge |
| `DeleteDocument` | served, idempotent |
| `BatchGetDocuments` | served — this is the read path gRPC SDKs actually use |
| `Commit` | served for non-transactional writes |
| everything else | `UNIMPLEMENTED` with a message naming the gap |

Known limits, all deliberate and all visible to a client:

- **In-memory only.** Data is lost on restart. There is no `--data-dir` yet.
- **No queries.** `RunQuery`, `RunAggregationQuery`, `ListDocuments`,
  `ListCollectionIds` return `UNIMPLEMENTED`.
- **No transactions.** `BeginTransaction`/`Rollback`, and any read or commit
  carrying a transaction token, return `UNIMPLEMENTED`.
- **No realtime listeners.** `Listen` and the streaming `Write` RPC return
  `UNIMPLEMENTED`.
- **No field transforms.** `serverTimestamp()`, `increment()`, array
  union/remove return `UNIMPLEMENTED` rather than silently writing a wrong value.
- **No nested field paths in masks.** `a.b` and `` `a&b` `` are rejected with
  `UNIMPLEMENTED`; top-level names work.
- **Commit is not atomic.** A write that fails mid-batch leaves earlier writes in
  the batch applied.
- **No auth and no security rules.** Every request is served. Bind loopback only
  (the default).

Unserved RPCs answer `UNIMPLEMENTED` with a message that names what is missing,
so an SDK failure reads as a clear gap instead of a hang or a protocol error.

The frozen v1 RPC surface — what has to land before we call this
"SDK-compatible CRUD + simple query" — is the `firestore-v1-contract` document on
[SAK-163](/SAK/issues/SAK-163).

## Layout

```
src/local/
  mod.rs        server wiring: spawn(), serve(), shutdown
  names.rs      resource-name parsing (projects/*/databases/*/documents/**)
  store.rs      in-memory document store (BTreeMap keyed by resource name)
  firestore.rs  the google.firestore.v1.Firestore service implementation
```

`BTreeMap` rather than a hash map because every query shape Firestore needs next
(list a collection, scan a subtree) is a key-prefix range scan.

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
# Fast: drives the server over real gRPC using our generated stubs.
cargo test --test local_firestore_tests
cargo test --lib local::

# Slow but conclusive: drives it with Google's official client library.
scripts/smoke-local-firestore.sh
```

The smoke script builds fireup, starts `fireup local`, runs
`tests/local-smoke/smoke.mjs` against it with `@google-cloud/firestore`, and
tears the server down. It needs network access the first time to `npm install`.

The Rust tests can pass while the SDK test fails — the SDK picks its own RPCs
(`.get()` goes to `BatchGetDocuments`, `.set()` goes to `Commit`) and does its own
serialization. Keep both.
