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
node/observe.mjs     the same probes against something that is NOT fireup
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

## Measuring something that is not fireup

The official client libraries honour `FIRESTORE_EMULATOR_HOST` /
`DATASTORE_EMULATOR_HOST`, so the probes will drive anything that serves the
same public contract. `runner.mjs` will not: it demands the exact status
`matrix.json` claims, down to the wording of fireup's refusals, which no
stranger will ever produce. `observe.mjs` is the other half — it runs the same
probes and **records** the answer instead of asserting it:

| Verdict | What it means |
|---|---|
| `serves` | the probe passed |
| `refuses` | gRPC `UNIMPLEMENTED` (12) |
| `diverges` | the call worked and the answer was different — an assertion failed |
| `error` | any other gRPC status, or a response shape the probe could not read |
| `timeout` | the probe did not finish |
| `not-measurable` | the probe has no `served` branch, so there is nothing to point |

Start the target yourself, then:

```bash
# any other implementation of the same contract — start it yourself first, then
# point the probes at whatever port it serves Firestore on (one port for
# everything, in which case --datastore-host can be left to default)
scripts/conformance-target.sh --target '<name> <version>' \
  --firestore-host 127.0.0.1:4588 \
  --out "docs/local/measurements/<name>-<version>.json"

# Google's emulator — separate ports, and it needs a JRE 21+
firebase emulators:start --only firestore,auth --project demo-fireup-local
scripts/conformance-target.sh --target 'Firebase Emulator Suite v1.22.0' \
  --firestore-host 127.0.0.1:8080 \
  --out docs/local/measurements/firebase-emulator-v1.22.0.json

# …and again in Datastore mode, which `firebase emulators:start` cannot select.
# The JAR the CLI downloaded takes its own --database-mode flag:
java -jar ~/.cache/firebase/emulators/cloud-firestore-emulator-v1.22.0.jar \
  --host=127.0.0.1 --port=8081 --database-mode=datastore-mode
scripts/conformance-target.sh --target 'emulator v1.22.0 datastore-mode' \
  --firestore-host 127.0.0.1:8081 --only datastore \
  --out docs/local/measurements/firebase-emulator-v1.22.0-datastore-mode.json

# the negative control, and the reason any of the above is believable
scripts/conformance-target.sh --target 'fireup local' --firestore-host 127.0.0.1:4589 \
  --out docs/local/measurements/fireup-local.json
```

`--datastore-host` defaults to `--firestore-host`; pass it when the target
splits them. `--timeout` defaults to 20s, which is too short for Google's
emulator — its transaction locks mean a conflict probe can take tens of seconds,
so use `--timeout 90000` there or you will record a `timeout` where the answer
was `serves`.

**Always run the fireup control in the same session.** `observe.mjs` has no
assertions about who is answering, so a harness bug looks exactly like a
competitor divergence. Fireup scoring anything other than "every row as the
matrix claims" means the harness is broken, not the competitor. Two real harness
bugs were caught this way in SAK-217 — see
[`docs/local/measurements/README.md`](../../docs/local/measurements/README.md).

### A dormant `served` branch

A row fireup refuses has no `served` branch, so there is nothing to measure on
another implementation either — and those are the competitively interesting
rows. Three of them (`firestore.Write`, `firestore.PartitionQuery`,
`datastore.gql`) now carry a `served` branch that fireup never runs: the matrix
says `unimplemented`, so the runner picks that branch and the new one sits
dormant until the feature lands. Write one when the comparison needs to stop
quoting somebody's README. It is not dead code — `observe.mjs` runs it, and the
day the row flips the check is already there.

### What this is, and the line

Black-box conformance testing of a **published** API contract through its
vendor's own open-source client libraries — the same method this suite uses on
fireup. Running a published binary and recording what its public API answers is
observation. **Do not decompile, disassemble, or binary-analyse** the Firebase
emulator JAR or any other third-party artefact you point this at, and do not
copy code or configuration out of any of them. See
`.cursor/rules/firebase-gcp-compliance.mdc` and
`docs/licenses/FIREBASE_GCP_LOCAL.md`, which carries the method and the versions
measured.
