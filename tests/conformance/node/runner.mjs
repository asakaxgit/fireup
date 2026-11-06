/**
 * SDK conformance runner for `fireup local`.
 *
 * Reads `tests/conformance/matrix.json` — the fidelity matrix in
 * `docs/local/README.md`, as data — and runs one probe per feature against a
 * running server, using Google's *official* client libraries wherever a library
 * can reach the feature and raw gRPC over the vendored public protos where it
 * cannot.
 *
 * The matrix drives the assertion, not the other way round:
 *
 *   status "served"                -> the probe's `served` branch must pass
 *   status "unimplemented"         -> the probe's `unimplemented` branch must
 *                                     pass, i.e. the call must fail with
 *                                     UNIMPLEMENTED and a message that names
 *                                     the gap
 *   status "documented-divergence" -> the probe pins the documented behaviour
 *
 * So flipping a row to `served` when the feature lands flips what CI demands,
 * and a probe that has no branch for the status the matrix claims is a
 * *failure*, not a skip — you cannot mark a feature served without writing the
 * check that proves it.
 *
 * Coverage is enforced in both directions: a matrix row with no probe fails,
 * and a probe with no matrix row fails. Adding a row to the README without
 * adding a check here cannot pass.
 *
 * Usage (scripts/conformance-local.sh does all of this for you):
 *   FIRESTORE_EMULATOR_HOST=127.0.0.1:4589 \
 *   DATASTORE_EMULATOR_HOST=127.0.0.1:4589 \
 *     node tests/conformance/node/runner.mjs [--only <id-substring>]
 */

import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { strict as assert } from 'node:assert'

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

/**
 * A run id in every resource name, so a second run against a *persistent*
 * Postgres store starts from a clean namespace instead of tripping over the
 * documents the previous run left behind. A suite that only passes on an empty
 * database is a suite people stop trusting.
 */
const RUN_ID = `${Date.now().toString(36)}${Math.floor(Math.random() * 1e4)}`

const expectUnimplemented = async (fn, { names } = {}) => {
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
  // The matrix promises a refusal that *names the missing piece*, so an SDK
  // failure reads as a gap rather than as a bug in the caller's code.
  assert.match(
    error.message,
    /fireup local does not (serve|support)/,
    `the refusal must name fireup and the gap, got: ${error.message}`,
  )
  if (names) {
    assert.match(error.message, names, `the refusal must name the feature, got: ${error.message}`)
  }
  return error
}

const buildContext = (db, datastore, raw) => ({
  projectId: PROJECT_ID,
  runId: RUN_ID,
  db,
  datastore,
  raw,
  GRPC,
  expectUnimplemented,
  /** A collection nobody else in this run writes to. */
  freshCollection(label) {
    return db.collection(`conf_${RUN_ID}_${label.replace(/[^a-zA-Z0-9]/g, '_')}`)
  },
  /** A Datastore kind nobody else in this run writes to. */
  freshKind(label) {
    return `Conf_${RUN_ID}_${label.replace(/[^a-zA-Z0-9]/g, '_')}`
  },
})

const main = async () => {
  const firestoreHost = process.env.FIRESTORE_EMULATOR_HOST
  const datastoreHost = process.env.DATASTORE_EMULATOR_HOST
  assert.ok(firestoreHost, 'FIRESTORE_EMULATOR_HOST must be set')
  // Both libraries read their own variable and nothing else: handing the
  // Datastore client an apiEndpoint instead would make it try TLS.
  assert.ok(datastoreHost, 'DATASTORE_EMULATOR_HOST must be set')

  const onlyIndex = process.argv.indexOf('--only')
  const only = onlyIndex === -1 ? null : process.argv[onlyIndex + 1]

  const matrix = JSON.parse(readFileSync(MATRIX_PATH, 'utf8'))
  const probes = new Map(
    [...firestoreProbes, ...queryProbes, ...transformProbes, ...datastoreProbes].map((probe) => [probe.id, probe]),
  )

  const nodeFeatures = matrix.features.filter((feature) => NODE_DRIVERS.has(feature.driver))
  const delegated = matrix.features.filter((feature) => !NODE_DRIVERS.has(feature.driver))

  // Coverage, direction 2: a probe for a feature the matrix does not list is a
  // probe nobody will maintain, and usually means a row was renamed.
  const orphans = [...probes.keys()].filter(
    (id) => !matrix.features.some((feature) => feature.id === id),
  )

  console.log(`Driving fireup local at ${firestoreHost} (Firestore) / ${datastoreHost} (Datastore)`)
  console.log(`Matrix: ${nodeFeatures.length} feature(s) to probe here, ${delegated.length} delegated`)
  console.log(`Run id: ${RUN_ID}\n`)

  const db = new Firestore({ projectId: PROJECT_ID })
  const datastore = new Datastore({ projectId: PROJECT_ID })
  const raw = connect(firestoreHost)
  const context = buildContext(db, datastore, raw)

  const results = []
  for (const feature of nodeFeatures) {
    if (only && !feature.id.includes(only)) continue

    const probe = probes.get(feature.id)
    if (!probe) {
      results.push({
        feature,
        ok: false,
        error: new Error(
          `no probe for matrix feature "${feature.id}". Every row in the fidelity ` +
            `matrix needs a check here; add one to tests/conformance/node/.`,
        ),
      })
      console.log(`  MISSING ${feature.id}`)
      continue
    }

    const branch = probe[feature.status]
    if (!branch) {
      results.push({
        feature,
        ok: false,
        error: new Error(
          `the matrix says "${feature.id}" is ${feature.status}, but its probe has no ` +
            `"${feature.status}" branch. Write the check that proves the new status.`,
        ),
      })
      console.log(`  NO-BRANCH ${feature.id} (${feature.status})`)
      continue
    }

    try {
      await branch(context)
      results.push({ feature, ok: true })
      console.log(`  ok      ${feature.id} [${feature.status}]`)
    } catch (error) {
      results.push({ feature, ok: false, error })
      console.log(`  FAIL    ${feature.id} [${feature.status}]`)
      console.log(`          ${error?.message ?? error}`)
    }
  }

  await db.terminate()
  raw.close()

  const failed = results.filter((result) => !result.ok)

  console.log(`\n${results.length - failed.length}/${results.length} matrix features verified`)
  if (delegated.length > 0 && !only) {
    const drivers = [...new Set(delegated.map((feature) => feature.driver))].join(', ')
    console.log(`${delegated.length} feature(s) are covered by another driver (${drivers}):`)
    console.log('  the auth surface is driven by scripts/smoke-local-auth.sh with firebase-admin')
    console.log('  and the firebase client SDK. conformance-local.sh runs both.')
  }
  if (orphans.length > 0) {
    console.log(`\nProbes with no matrix row (rename or remove them): ${orphans.join(', ')}`)
  }

  if (failed.length > 0 || orphans.length > 0) {
    console.log('\nFailures:')
    for (const { feature, error } of failed) {
      console.log(`- ${feature.id} [${feature.status}] — ${feature.label}`)
      console.log(`  ${error?.stack ?? error}`)
    }
    process.exit(1)
  }

  console.log('\nfireup local matches its published fidelity matrix.')
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
