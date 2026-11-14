/**
 * Phase 3 exit criterion: prove an *official* Datastore client SDK can talk to
 * `fireup local` over DATASTORE_EMULATOR_HOST.
 *
 * Written against the public @google-cloud/datastore API. No emulator binary
 * was reverse-engineered.
 */

import { Datastore, DatastoreClient, or, PropertyFilter } from '@google-cloud/datastore'

import { writeResults } from '../smoke-gate/results.mjs'

const projectId = process.env.FIREUP_SMOKE_PROJECT_ID || 'fireup-local'
const datastore = new Datastore({ projectId })

/**
 * The generated `v1` client from the same package.
 *
 * `Mutation.property_transforms` has no handwritten surface in
 * @google-cloud/datastore yet — `save()` cannot express an increment — so the
 * transform check drives the library's own generated client instead. Both are
 * public exports, and `datastore.options` is where the handwritten client has
 * already resolved DATASTORE_EMULATOR_HOST into an endpoint and insecure
 * credentials, so the two talk to the same server.
 */
const rawClient = new DatastoreClient(datastore.options)

/** The key proto for a single named path element. */
const keyProto = (kind, name) => ({
  partitionId: { projectId },
  path: [{ kind, name }],
})

const checks = []

/**
 * Run one named check and record it, the same harness the Firestore and Auth
 * suites use. A failure no longer stops the run: later checks still report, so
 * the fidelity gate can tell "one behaviour broke" from "everything after it
 * broke". Checks that build on an earlier one (get after save) fail with it.
 */
// `covers` names the fidelity-matrix rows the check asserts, keyed by the
// status it asserts them with; see tests/smoke-gate/matrix-coverage.mjs.
const check = async (name, fn, covers) => {
  try {
    await fn()
    checks.push({ name, ok: true, covers })
    console.log(`  ok   ${name}`)
  } catch (error) {
    checks.push({ name, ok: false, covers, error })
    console.log(`  FAIL ${name}`)
    console.log(`       ${error?.message ?? error}`)
  }
  writeResults(checks)
}

const kind = 'SmokeTask'
const name = `smoke-${Date.now()}`
const key = datastore.key([kind, name])

await check('save an entity', async () => {
  await datastore.save({
    key,
    data: {
      title: 'smoke',
      priority: 1,
      done: false,
    },
  })
}, { served: ['datastore.Commit'] })

await check('get it back by key', async () => {
  const [fetched] = await datastore.get(key)
  if (!fetched || fetched.title !== 'smoke') {
    throw new Error(`expected saved entity, got ${JSON.stringify(fetched)}`)
  }
}, { served: ['datastore.Lookup'] })

await check('an equality query returns it', async () => {
  const query = datastore.createQuery(kind).filter('priority', '=', 1).limit(10)
  const [rows] = await datastore.runQuery(query)
  if (!rows.some((row) => row[datastore.KEY].name === name)) {
    throw new Error(`query did not return the saved entity: ${JSON.stringify(rows)}`)
  }
}, { served: ['datastore.RunQuery'] })

await check('allocateIds returns complete keys', async () => {
  const incomplete = datastore.key([kind])
  const [allocated] = await datastore.allocateIds(incomplete, 2)
  if (allocated.length !== 2 || allocated.some((k) => !k.id)) {
    throw new Error(`allocateIds failed: ${JSON.stringify(allocated)}`)
  }
}, { served: ['datastore.AllocateIds'] })

await check('a transaction commit is visible to a later read', async () => {
  const txKey = datastore.key([kind, `tx-${Date.now()}`])
  const tx = datastore.transaction()
  await tx.run()
  tx.save({ key: txKey, data: { title: 'tx', priority: 2 } })
  await tx.commit()
  const [txFetched] = await datastore.get(txKey)
  if (!txFetched || txFetched.title !== 'tx') {
    throw new Error(`transaction write missing: ${JSON.stringify(txFetched)}`)
  }
}, { served: ['datastore.BeginTransaction-Rollback', 'datastore.Commit'] })

await check('a read-only transaction reads committed data', async () => {
  const tx = datastore.transaction({ readOnly: true })
  await tx.run()
  const [fetched] = await tx.get(key)
  await tx.commit()
  if (fetched?.priority !== 1) throw new Error(`read-only transaction read ${JSON.stringify(fetched)}`)
}, { served: ['datastore.transaction.read-only-commit'] })

await check('a read-only transaction that writes stays refused with INVALID_ARGUMENT', async () => {
  const tx = datastore.transaction({ readOnly: true })
  await tx.run()
  tx.save({ key: datastore.key([kind, `ro-${Date.now()}`]), data: { title: 'ro' } })
  const error = await tx.commit().then(() => null, (caught) => caught)
  if (error?.code !== 3 || !error.message.includes('read-only transactions cannot modify entities')) {
    throw new Error(`a read-only write committed or failed wrongly: ${error?.code} ${error?.message}`)
  }
}, { served: ['datastore.transaction.read-only-commit'] })

await check('property transforms report and store their results', async () => {
  const counterName = `counter-${Date.now()}`
  const counterKey = datastore.key([kind, counterName])
  const [committed] = await rawClient.commit({
    projectId,
    mode: 'NON_TRANSACTIONAL',
    mutations: [
      {
        upsert: {
          key: keyProto(kind, counterName),
          properties: {
            // Carried by the write, because an unmasked upsert replaces the
            // entity and the transforms then run against what it left behind.
            hits: { integerValue: 41 },
            tags: { arrayValue: { values: [{ stringValue: 'a' }] } },
          },
        },
        propertyTransforms: [
          { property: 'hits', increment: { integerValue: 1 } },
          { property: 'touched', setToServerValue: 'REQUEST_TIME' },
          {
            property: 'tags',
            appendMissingElements: {
              values: [{ stringValue: 'a' }, { stringValue: 'b' }],
            },
          },
        ],
      },
    ],
  })

  const [transformResults] = committed.mutationResults.map((r) => r.transformResults)
  if (transformResults.length !== 3) {
    throw new Error(`expected 3 transform results, got ${JSON.stringify(transformResults)}`)
  }
  if (Number(transformResults[0].integerValue) !== 42) {
    throw new Error(`increment reported ${JSON.stringify(transformResults[0])}, wanted 42`)
  }
  if (!transformResults[1].timestampValue) {
    throw new Error(`serverTimestamp reported ${JSON.stringify(transformResults[1])}`)
  }
  if (transformResults[2].nullValue === undefined) {
    throw new Error(`arrayUnion should report null, got ${JSON.stringify(transformResults[2])}`)
  }

  // Read it back through the handwritten client: what the ordinary SDK sees.
  const [counter] = await datastore.get(counterKey)
  if (counter.hits !== 42) {
    throw new Error(`stored hits is ${counter.hits}, wanted 42`)
  }
  if (JSON.stringify(counter.tags) !== JSON.stringify(['a', 'b'])) {
    throw new Error(`stored tags is ${JSON.stringify(counter.tags)}, wanted ["a","b"]`)
  }
  if (!(counter.touched instanceof Date)) {
    throw new Error(`stored touched is ${JSON.stringify(counter.touched)}, wanted a timestamp`)
  }
}, { served: ['datastore.transform.increment', 'datastore.transform.server-value', 'datastore.transform.array-union'] })

await check('a property_mask write accumulates and deletes', async () => {
  const maskedName = `masked-${Date.now()}`
  const maskedKey = datastore.key([kind, maskedName])

  // Seed the whole entity through the ordinary SDK.
  await datastore.save({
    key: maskedKey,
    data: { title: 'first', hits: 0, author: 'ada' },
  })

  // Two masked upserts that only ever carry `title`. Because the mask keeps the
  // write off `hits`, the increment reads the stored value instead of one the
  // client had to send — the Datastore spelling of Firestore's update_mask plus
  // update_transforms. `propertyMask` has no handwritten surface either, so this
  // drives the same generated client as the transforms above.
  for (const [round, want] of [['second', 1], ['third', 2]]) {
    const [response] = await rawClient.commit({
      projectId,
      mode: 'NON_TRANSACTIONAL',
      mutations: [
        {
          upsert: {
            key: keyProto(kind, maskedName),
            properties: { title: { stringValue: round } },
          },
          propertyMask: { paths: ['title'] },
          propertyTransforms: [{ property: 'hits', increment: { integerValue: 1 } }],
        },
      ],
    })
    const reported = Number(response.mutationResults[0].transformResults[0].integerValue)
    if (reported !== want) {
      throw new Error(`masked increment reported ${reported}, wanted ${want}`)
    }
  }

  const [masked] = await datastore.get(maskedKey)
  if (masked.title !== 'third' || masked.hits !== 2) {
    throw new Error(`masked write left ${JSON.stringify(masked)}, wanted title=third hits=2`)
  }
  if (masked.author !== 'ada') {
    throw new Error(`a property the mask never named changed: ${JSON.stringify(masked)}`)
  }

  // The other half of the proto rule: "properties referenced in the mask but not
  // in the entity are deleted". The mask names `author`, the entity omits it.
  await rawClient.commit({
    projectId,
    mode: 'NON_TRANSACTIONAL',
    mutations: [
      {
        update: {
          key: keyProto(kind, maskedName),
          properties: { title: { stringValue: 'fourth' } },
        },
        propertyMask: { paths: ['title', 'author'] },
      },
    ],
  })

  const [afterMaskedDelete] = await datastore.get(maskedKey)
  if (afterMaskedDelete.author !== undefined) {
    throw new Error(`masked property was not deleted: ${JSON.stringify(afterMaskedDelete)}`)
  }
  if (afterMaskedDelete.title !== 'fourth' || afterMaskedDelete.hits !== 2) {
    throw new Error(`masked delete disturbed other properties: ${JSON.stringify(afterMaskedDelete)}`)
  }
}, { served: ['datastore.property-mask.writes-a-carried-path', 'datastore.property-mask.deletes-an-omitted-path', 'datastore.property-mask.leaves-unnamed-properties', 'datastore.property-mask.transform-accumulates'] })

await check('a nested property_mask path stays refused with UNIMPLEMENTED', async () => {
  const refusedMask = await rawClient
    .commit({
      projectId,
      mode: 'NON_TRANSACTIONAL',
      mutations: [
        {
          upsert: {
            // Its own entity, so the refusal is checked even when the
            // property_mask check above failed before writing one.
            key: keyProto(kind, `nested-${Date.now()}`),
            properties: { title: { stringValue: 'nope' } },
          },
          propertyMask: { paths: ['author.name'] },
        },
      ],
    })
    .then(() => null, (error) => error)
  if (!refusedMask || refusedMask.code !== 12) {
    throw new Error(
      `expected UNIMPLEMENTED (12) for a nested mask path, got ${refusedMask && refusedMask.code}`,
    )
  }
}, { unimplemented: ['datastore.property-mask.nested-path', 'datastore.nested-property-mask'] })

await check('the maximum transform stays refused with UNIMPLEMENTED', async () => {
  const refused = await rawClient
    .commit({
      projectId,
      mode: 'NON_TRANSACTIONAL',
      mutations: [
        {
          upsert: { key: keyProto(kind, `max-${Date.now()}`), properties: {} },
          propertyTransforms: [{ property: 'hits', maximum: { integerValue: 1 } }],
        },
      ],
    })
    .then(() => null, (error) => error)
  if (!refused || refused.code !== 12) {
    throw new Error(`expected UNIMPLEMENTED (12) for maximum, got ${refused && refused.code}`)
  }
}, { unimplemented: ['datastore.transform.maximum-minimum'] })

// --- Filters ---------------------------------------------------------------
//
// Moved from backlog.mjs once served. A kind of their own, with a run id in it,
// so a durable store's earlier runs and the entities above do not join the
// answer.

const filterKind = `SmokeFilter_${Date.now().toString(36)}`
const filterKey = (name) => datastore.key([filterKind, name])

const names = (rows) => rows.map((row) => row[datastore.KEY].name).sort()

const expectNames = (rows, want, what) => {
  const got = names(rows)
  if (JSON.stringify(got) !== JSON.stringify([...want].sort())) {
    throw new Error(`${what}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`)
  }
}

await check('seed the filter fixture', async () => {
  await datastore.save([
    { key: filterKey('a'), data: { priority: 1, tags: ['x', 'y'], done: false, owner: 'ada' } },
    { key: filterKey('b'), data: { priority: 2, tags: ['y'], done: true, owner: 'bob' } },
    { key: filterKey('c'), data: { priority: 3, tags: ['z'], done: false, owner: 'ada' } },
  ])
}, { served: ['datastore.Commit'] })

await check('a != filter excludes the matching entity', async () => {
  const query = datastore.createQuery(filterKind).filter(new PropertyFilter('priority', '!=', 2))
  const [rows] = await datastore.runQuery(query)
  expectNames(rows, ['a', 'c'], '!= filter')
}, { served: ['datastore.RunQuery'] })

await check('an IN filter matches any listed value', async () => {
  const query = datastore.createQuery(filterKind).filter(new PropertyFilter('priority', 'IN', [1, 3]))
  const [rows] = await datastore.runQuery(query)
  expectNames(rows, ['a', 'c'], 'IN filter')
}, { served: ['datastore.RunQuery'] })

await check('a NOT_IN filter excludes the listed values', async () => {
  const query = datastore.createQuery(filterKind).filter(new PropertyFilter('priority', 'NOT_IN', [1, 3]))
  const [rows] = await datastore.runQuery(query)
  expectNames(rows, ['b'], 'NOT_IN filter')
}, { served: ['datastore.RunQuery'] })

await check('an OR composite filter matches either branch', async () => {
  const query = datastore
    .createQuery(filterKind)
    .filter(or([new PropertyFilter('priority', '=', 1), new PropertyFilter('owner', '=', 'bob')]))
  const [rows] = await datastore.runQuery(query)
  expectNames(rows, ['a', 'b'], 'OR filter')
}, { served: ['datastore.RunQuery'] })

await check('equality on an array property matches any element', async () => {
  const query = datastore.createQuery(filterKind).filter(new PropertyFilter('tags', '=', 'y'))
  const [rows] = await datastore.runQuery(query)
  expectNames(rows, ['a', 'b'], 'array equality')
}, { served: ['datastore.RunQuery'] })

await check('delete removes the entity', async () => {
  await datastore.delete(key)
  const [afterDelete] = await datastore.get(key)
  if (afterDelete) {
    throw new Error('entity still present after delete')
  }
}, { served: ['datastore.Commit'] })

const failed = checks.filter((c) => !c.ok)
console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`)
if (failed.length > 0) {
  console.log('\nFailures:')
  for (const { name, error } of failed) {
    console.log(`- ${name}: ${error?.stack ?? error}`)
  }
  // Not `process.exit(1)`: the suite has always ended by letting the clients
  // drain, and an exit code is all a failure needs to change.
  process.exitCode = 1
} else {
  console.log('OK — Datastore SDK smoke passed')
}
