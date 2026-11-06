# Measured conformance: what each implementation actually answered

The JSON files beside this one are raw output from
`scripts/conformance-target.sh`. They are the evidence behind the **measured**
cells in [`../COMPARISON.md`](../COMPARISON.md). Before SAK-217 that document's
emulator column was a reading of Google's documentation; these files are what
replaced the reading with answers.

Each file is one run: a target, a timestamp, the hosts it was reached on, and
one row per `tests/conformance/matrix.json` feature with a verdict —
`serves`, `refuses`, `diverges`, `error`, `timeout`, or `not-measurable`.

| File | Target | Measured |
|---|---|---|
| `fireup-local.json` | `fireup local`, in-memory store, this repository | 2026-11-06 |
| `firebase-emulator-v1.22.0.json` | `cloud-firestore-emulator` v1.22.0 via `firebase-tools` 15.35.0, Firestore-native mode | 2026-11-06 |
| `firebase-emulator-v1.22.0-datastore-mode.json` | the same JAR, `--database-mode=datastore-mode` (Datastore rows only) | 2026-11-06 |

An emulator ships new versions, so a measured cell is only true of the version
in this table. Re-running is one command per target; the recipe is in
[`../../../tests/conformance/README.md`](../../../tests/conformance/README.md#measuring-something-that-is-not-fireup).

Other third-party local-cloud emulators are not compared in this repository yet.
Publishing a comparison against one is a separate decision from being able to
measure it, and the recipe above works against anything that serves the same
contract.

## Why `fireup-local.json` is here

It is the negative control, and without it the other files are worth nothing.
`observe.mjs` reuses the suite's probes but drops every assertion about *who* is
answering, so the obvious failure mode is a harness that reports divergences
which are really its own bugs. The fireup run is the check: **68 `serves`, 6
`refuses`, 15 `not-measurable`, and zero `diverges` or `error`**. Every probe
therefore works when pointed at an implementation that claims the row, which is
what makes another target's verdicts its answers rather than ours.

Two divergences in the first draft of the runner *were* harness bugs, found
exactly this way:

- an undefined `GRPC.UNAVAILABLE` constant, so `error.code === GRPC.UNAVAILABLE`
  matched every error that carried no gRPC code at all — which is most
  probe-side crashes;
- `expectUnimplemented` demanding Fireup's own refusal wording (`/fireup local
  does not (serve|support)/`) from a stranger, which nothing else will ever say.

Both are fixed. The control is how we knew. A third was caught the same way in
SAK-223: a probe asserting three refusals through `@google-cloud/firestore`
scored `timeout` against **fireup**, because the client retries a refused query
with backoff and three of those overrun the 20s probe timeout. The fix is in
the probe, not the timeout — see
[`../../../tests/conformance/README.md`](../../../tests/conformance/README.md#measuring-something-that-is-not-fireup).

## What the emulator column changed about fireup

Measuring is not the deliverable; four rows moved because of it. The first run
found four probes building an `order_by` shape production rejects, and two
`order_by` shapes fireup served that production refuses — one of them with a
matrix probe *asserting* the false parity. SAK-223 fixed the behaviour, inverted
that probe, and rebuilt the four others on a shape production accepts:

| Row | First run | Now |
|---|---|---|
| `query.unary-filters` | `error` | `serves` |
| `query.composite-filters` | `error` | `serves` |
| `query.document-id-filter-and-order` | `error` | `serves` |
| `semantics.integer-double-compare-numerically` | `error` | `serves` |
| `semantics.nothing-follows-the-key-in-an-order` | — | `serves` (new row) |

No row in `firebase-emulator-v1.22.0.json` now rejects a query fireup accepts.
The 23 that remain are the 21 Datastore rows wanting `--database-mode` (served
in the Datastore-mode file beside this one) and the two duplex-streaming rows
the harness cannot drive.

## Reading a row

```json
{
  "id": "semantics.name-appended-to-order",
  "label": "__name__ is appended to every orderBy",
  "fireupStatus": "served",
  "observed": "serves"
}
```

- `fireupStatus` is what `matrix.json` claims about **Fireup**. It is carried
  along for context and says nothing about the target.
- `observed` is what this target did.
- `detail`, when present, is the assertion message or the gRPC status verbatim.
  For a `diverges` row it is the actual-versus-expected diff, which is the part
  worth reading.

`not-measurable` means the probe has no `served` branch to point at anything —
mostly rows Fireup refuses, where there was never a positive check to write.
Those cells in `COMPARISON.md` stay documentation-sourced, and that is why they
are marked `[D]` there rather than quietly scored.

## Two things to know before trusting a verdict

**`error` is not the same as `refuses`.** Four rows score `error` against the
emulator with a gRPC status that is an answer, not a gap:

```
query.unary-filters        INVALID_ARGUMENT: order by clause cannot contain more fields after the key
query.composite-filters    INVALID_ARGUMENT: order by clause cannot contain more fields after the key
semantics.integer-double…  INVALID_ARGUMENT: order by clause cannot contain more fields after the key
query.document-id-filter…  FAILED_PRECONDITION: Firestore does not support descending key scans
```

Those are **Fireup bugs**, not emulator gaps. The probes build a query shape
Fireup accepts and production rejects, so the reference implementation refusing
them is the finding. See the bug table in `COMPARISON.md`.

**Duplex streaming does not work through this harness against the emulator.**
`firestore.Listen` and `firestore.Write` both come back as a bare gRPC `UNKNOWN`
with an empty message, while unary and server-streaming calls to the same port
are fine and the SDK's own `onSnapshot()` returns a snapshot. That is this
harness's raw duplex client, not a refusal, and the affected cells in
`COMPARISON.md` stay `[D]` rather than claiming a gap that was not demonstrated.
