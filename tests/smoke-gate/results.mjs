/**
 * The machine-readable half of an SDK smoke suite's output.
 *
 * Every suite already prints an `ok` / `FAIL` line per named check. That is
 * enough for a person reading one run, and useless for answering "did a check
 * that passed on `main` stop passing?" — which is what `gate.mjs` asks. So each
 * suite also hands its checks to `writeResults`, which writes
 * `{ suite, checks: [{ name, ok }] }` to the file named by FIREUP_SMOKE_RESULTS.
 *
 * Call it after every check, not once at the end. A suite that throws halfway,
 * or that `timeout` kills because the client hung, then still leaves the checks
 * it got through on disk, and the gate names the rest as missing instead of
 * reporting the whole suite as one opaque failure.
 *
 * With FIREUP_SMOKE_RESULTS unset this does nothing, so running a suite by hand
 * is unchanged.
 */

import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'

export const writeResults = (checks) => {
  const path = process.env.FIREUP_SMOKE_RESULTS
  if (!path) return

  // The suite id is what the baseline is keyed on, and the same suite runs
  // against more than one store, so the caller has to say which run this is.
  // Guessing would file a Postgres run under the in-memory baseline.
  const suite = process.env.FIREUP_SMOKE_SUITE
  if (!suite) {
    throw new Error('FIREUP_SMOKE_RESULTS is set but FIREUP_SMOKE_SUITE is not; name the run, e.g. firestore/memory')
  }

  mkdirSync(dirname(path), { recursive: true })
  const body = { suite, checks: checks.map(({ name, ok }) => ({ name, ok })) }
  writeFileSync(path, `${JSON.stringify(body, null, 2)}\n`)
}
