# `fireup local` SDK conformance suite

The regression shield for the [fidelity matrix](../../docs/local/README.md#fidelity-matrix).

That matrix is the promise `fireup local` makes about itself: what it serves,
and what it refuses with an error that names the gap. This suite is what keeps
the promise checkable. Run it all with:

```bash
scripts/conformance-local.sh            # in-memory store
FIREUP_CONFORMANCE_POSTGRES_URL=postgresql://... scripts/conformance-local.sh
```

## How it is put together

```
matrix.json          the matrix as data: one entry per documented row
node/runner.mjs      loads matrix.json, runs one probe per entry, enforces coverage
node/grpc-client.mjs raw gRPC clients from the vendored public protos
node/probes-*.mjs    the probes, grouped by area
python/              the same contract, re-checked with the Python SDKs
```

Plus `tests/fidelity_matrix_tests.rs`, which compares `matrix.json` against the
README tables without starting anything — that is the check that fails in a
second when the docs and the suite drift apart.

## The rule that makes it durable

`status` in `matrix.json` decides what a row demands:

| Status | What the probe must prove |
|---|---|
| `served` | the feature works through an official SDK |
| `unimplemented` | the call fails with gRPC `UNIMPLEMENTED` **and** a message naming the gap |
| `documented-divergence` | the documented, deliberately-different behaviour still holds |

A probe is `{ id, served?, unimplemented?, 'documented-divergence'? }`, and the
runner picks the branch the matrix claims. If that branch is missing, the run
**fails** — it is not skipped. So you cannot mark a row served without writing
the check that proves it, and you cannot quietly stop checking a refusal.

Coverage is enforced both ways: a matrix entry with no probe fails, and a probe
whose entry no longer exists fails.

## Implementing a feature that is currently refused

1. Implement it.
2. Flip the row in `docs/local/README.md` from `UNIMPLEMENTED` to `served`.
3. Flip `status` on the matching entry in `matrix.json`.
4. Add (or finish) the probe's `served` branch.

Steps 2 and 3 are checked against each other by `cargo test --test
fidelity_matrix_tests`, and step 4 is checked by the runner. Several rows
already carry both branches — `firestore.Listen`,
`firestore.RunAggregationQuery`, `query.aggregations` — so for those the flip is
the whole change.

This is not hypothetical. SAK-190 and SAK-191 landed field transforms,
collection group queries, cursors, and nested field paths while this suite was
being written; flipping four `status` values and filling in the `served`
branches was the entire update, and the rows those changes *added* to the README
failed `fidelity_matrix_tests` until they had probes.

## Adding a probe

Probes live in `node/probes-firestore.mjs`, `node/probes-queries.mjs`, or
`node/probes-datastore.mjs`. Each receives a context with the official SDK
clients (`db`, `datastore`), the raw gRPC clients (`raw`), `expectUnimplemented`,
and `freshCollection`/`freshKind` helpers.

Use the helpers for anything you write. They put a per-run id in every resource
name, which is what lets the suite run twice against a persistent Postgres
database — and a suite that only passes against an empty store is one people
stop trusting.

Run one probe at a time while iterating:

```bash
FIRESTORE_EMULATOR_HOST=127.0.0.1:4589 DATASTORE_EMULATOR_HOST=127.0.0.1:4589 \
  node tests/conformance/node/runner.mjs --only query.cursors
```

## Why there are three drivers

A client library only reaches the RPCs it chooses, so driving SDKs alone would
leave documented rows untested:

- **`node-sdk`** — `@google-cloud/firestore`, `@google-cloud/datastore`. The
  layer that matters most, because it is not written against our reading of the
  spec.
- **`node-grpc`** — raw gRPC over `proto/google/`, for what no SDK will send:
  `GetDocument`, `CreateDocument`, `UpdateDocument`, `DeleteDocument`,
  `PartitionQuery`, `ExecutePipeline`, the streaming `Write`, `Listen`,
  `ReserveIds`, Datastore GQL/cursors/property masks/property transforms, a
  `read_time` selector, `find_nearest`, a `maximum`/`minimum` transform, a
  collection group query with no collection id, and a literal `== NaN`.
- **`auth-smoke`** — the Identity Toolkit rows, in
  `scripts/smoke-local-auth.sh`, with `firebase-admin` and the `firebase`
  client SDK. Those entries are listed in `matrix.json` for completeness and
  reported as delegated by the runner; `scripts/conformance-local.sh` runs both.

The Python layer re-checks the subset marked `"python": true`. It is optional
and skips with a message when its libraries are missing — never counted as a
pass.
