/**
 * Black-box conformance *observation* over a published API contract.
 *
 * `runner.mjs` is a regression shield: it reads the status `matrix.json` claims
 * for a row and demands that exact answer, including the wording of fireup's
 * refusals. That makes it useless against anything that is not fireup — a
 * competitor's refusal will never say "fireup local does not serve".
 *
 * This runner inverts the relationship. It runs the same probes, through the
 * same official Google client libraries, against whatever host the
 * `FIRESTORE_EMULATOR_HOST` / `DATASTORE_EMULATOR_HOST` variables point at, and
 * *records* what came back instead of asserting it:
 *
 *   serves    the probe passed — the target does this
 *   refuses   gRPC UNIMPLEMENTED (12): the target says it does not do this
 *   diverges  the call worked but the answer was different (an assertion failed)
 *   error     some other failure — the code and message are kept verbatim
 *   timeout   the probe did not finish (a stream that never produces, usually)
 *
 * `diverges` is the valuable one. A row where a target's own documentation
 * claims a feature and the probe comes back `diverges` is a contradiction no
 * amount of README-reading can find.
 *
 * This is observation of a public API through its vendor's own client
 * libraries. Nothing here decompiles, disassembles, or binary-analyses any
 * artefact — see `docs/licenses/FIREBASE_GCP_LOCAL.md`.
 *
 * Usage (scripts/conformance-target.sh wraps this):
 *   FIRESTORE_EMULATOR_HOST=127.0.0.1:8080 DATASTORE_EMULATOR_HOST=127.0.0.1:8081 \
 *     node tests/conformance/node/observe.mjs \
 *       --target 'Firebase Emulator Suite v1.22.0' --out /tmp/observed.json [--only <id>] [--timeout 90000]
 */

import { strict as assert } from 'node:assert'
import { readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { GRPC, connect } from './grpc-client.mjs'
import { firestoreProbes } from './probes-firestore.mjs'
import { queryProbes } from './probes-queries.mjs'
import { datastoreProbes } from './probes-datastore.mjs'
import { transformProbes } from './probes-transforms.mjs'

const require = createRequire(import.meta.url)
const { Firestore } = require('@google-cloud/firestore')
const { Datastore } = require('@google-cloud/datastore')

const here = dirname(fileURLToPath(import.meta.url))
const MATRIX_PATH = resolve(here, '../matrix.json')

const PROJECT_ID = process.env.FIREUP_CONFORMANCE_PROJECT_ID ?? 'fireup-local'
const NODE_DRIVERS = new Set(['node-sdk', 'node-grpc'])
const DEFAULT_TIMEOUT_MS = 20_000
/** Not in `GRPC` — only this runner needs to tell "nothing there" from "no". */
const GRPC_UNAVAILABLE = 14

const flag = (name, fallback = null) => {
  const index = process.argv.indexOf(`--${name}`)
  return index === -1 ? fallback : process.argv[index + 1]
}

/**
 * What a thrown error says about the target.
 *
 * The order matters: a probe that got an answer and then asserted something
 * about it fails with an AssertionError, and that is a *divergence*, not an
 * error. A gRPC status arrives before any assertion runs.
 */
const classify = (error) => {
  if (error?.__timeout) return { observed: 'timeout' }
  if (error?.code === 'ERR_ASSERTION' || error?.name === 'AssertionError') {
    return { observed: 'diverges', detail: error.message }
  }
  // A numeric check, not `=== GRPC.X`: an undefined constant would match every
  // error that carries no code at all, which is most probe-side crashes.
  if (typeof error?.code === 'number') {
    if (error.code === GRPC.UNIMPLEMENTED) return { observed: 'refuses', detail: error.message }
    if (error.code === GRPC_UNAVAILABLE) return { observed: 'unreachable', detail: error.message }
    return { observed: 'error', detail: `gRPC ${error.code}: ${error.message}` }
  }
  // A `TypeError` here is the target answering with a shape the probe did not
  // expect — a field the contract says is there and is not. Kept separate from
  // `diverges` so it gets read by a human rather than scored.
  const name = error?.name && error.name !== 'Error' ? `${error.name}: ` : ''
  return { observed: 'error', detail: `${name}${error?.message ?? error}` }
}

/**
 * A probe that never settles must not hang the whole run. The underlying call
 * is left to its own devices — we stop waiting for it, which is why the process
 * exits explicitly at the end.
 */
const withTimeout = (promise, ms) =>
  new Promise((resolvePromise, reject) => {
    const timer = setTimeout(() => {
      const error = new Error(`probe did not finish within ${ms}ms`)
      error.__timeout = true
      reject(error)
    }, ms)
    promise.then(
      (value) => {
        clearTimeout(timer)
        resolvePromise(value)
      },
      (error) => {
        clearTimeout(timer)
        reject(error)
      },
    )
  })

const buildContext = (db, datastore, raw, runId) => ({
  projectId: PROJECT_ID,
  runId,
  db,
  datastore,
  raw,
  GRPC,
  /**
   * The vendor-neutral half of runner.mjs's `expectUnimplemented`: a served
   * branch may assert that *part* of its surface is refused
   * (`transaction.read-only` does), and that assertion has to survive here. The
   * dropped half is the wording check — nobody else's refusal says "fireup
   * local does not serve", so only the gRPC status is required.
   */
  expectUnimplemented: async (fn) => {
    let error
    try {
      await fn()
    } catch (caught) {
      error = caught
    }
    assert.ok(error, 'expected the call to be refused, but it succeeded')
    assert.equal(
      error.code,
      GRPC.UNIMPLEMENTED,
      `expected gRPC UNIMPLEMENTED (12), got ${error.code}: ${error.message}`,
    )
    return error
  },
  freshCollection(label) {
    return db.collection(`conf_${runId}_${label.replace(/[^a-zA-Z0-9]/g, '_')}`)
  },
  freshKind(label) {
    return `Conf_${runId}_${label.replace(/[^a-zA-Z0-9]/g, '_')}`
  },
})

const main = async () => {
  const firestoreHost = process.env.FIRESTORE_EMULATOR_HOST
  const datastoreHost = process.env.DATASTORE_EMULATOR_HOST
  if (!firestoreHost) throw new Error('FIRESTORE_EMULATOR_HOST must be set')
  if (!datastoreHost) throw new Error('DATASTORE_EMULATOR_HOST must be set')

  const target = flag('target') ?? firestoreHost
  const only = flag('only')
  const out = flag('out')
  const timeoutMs = Number(flag('timeout', DEFAULT_TIMEOUT_MS))
  const runId = `${Date.now().toString(36)}${Math.floor(Math.random() * 1e4)}`

  const matrix = JSON.parse(readFileSync(MATRIX_PATH, 'utf8'))
  const probes = new Map(
    [...firestoreProbes, ...queryProbes, ...transformProbes, ...datastoreProbes].map((probe) => [probe.id, probe]),
  )

  const features = matrix.features.filter((feature) => NODE_DRIVERS.has(feature.driver))

  console.log(`Observing ${target}`)
  console.log(`  Firestore: ${firestoreHost}   Datastore: ${datastoreHost}`)
  console.log(`  Run id: ${runId}   probe timeout: ${timeoutMs}ms\n`)

  const db = new Firestore({ projectId: PROJECT_ID })
  const datastore = new Datastore({ projectId: PROJECT_ID })
  const raw = connect(firestoreHost)
  const context = buildContext(db, datastore, raw, runId)

  const rows = []
  for (const feature of features) {
    if (only && !feature.id.includes(only)) continue

    const probe = probes.get(feature.id)
    const row = {
      id: feature.id,
      label: feature.label,
      docTable: feature.docTable ?? null,
      fireupStatus: feature.status,
    }

    // A row fireup refuses usually has no `served` branch to run — there is no
    // check to point at anything. Say so rather than scoring it.
    if (!probe?.served) {
      rows.push({ ...row, observed: 'not-measurable', detail: 'the probe has no `served` branch' })
      console.log(`  n/a      ${feature.id} — no served branch`)
      continue
    }

    try {
      await withTimeout(probe.served(context), timeoutMs)
      rows.push({ ...row, observed: 'serves' })
      console.log(`  serves   ${feature.id}`)
    } catch (error) {
      const result = classify(error)
      rows.push({ ...row, ...result })
      console.log(`  ${result.observed.padEnd(8)} ${feature.id}`)
      if (result.detail) console.log(`           ${String(result.detail).split('\n')[0].slice(0, 180)}`)
    }
  }

  const tally = {}
  for (const row of rows) tally[row.observed] = (tally[row.observed] ?? 0) + 1

  console.log(`\n${target}: ${rows.length} row(s) observed`)
  for (const [observed, count] of Object.entries(tally).sort()) {
    console.log(`  ${String(count).padStart(3)} ${observed}`)
  }

  const report = {
    target,
    measuredAt: new Date().toISOString(),
    hosts: { firestore: firestoreHost, datastore: datastoreHost },
    projectId: PROJECT_ID,
    runId,
    timeoutMs,
    tally,
    rows,
  }
  if (out) {
    writeFileSync(out, `${JSON.stringify(report, null, 2)}\n`)
    console.log(`\nreport: ${out}`)
  }

  // An observed target may have left a stream open; nothing here is asserting,
  // so there is no failure exit code to report.
  await db.terminate().catch(() => {})
  raw.close()
  process.exit(0)
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
