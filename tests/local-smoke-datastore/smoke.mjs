/**
 * Phase 3 exit criterion: prove an *official* Datastore client SDK can talk to
 * `fireup local` over DATASTORE_EMULATOR_HOST.
 *
 * Written against the public @google-cloud/datastore API. No emulator binary
 * was reverse-engineered.
 */

import { Datastore, DatastoreClient } from '@google-cloud/datastore'

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

const kind = 'SmokeTask'
const name = `smoke-${Date.now()}`
const key = datastore.key([kind, name])

console.log('1. save entity')
await datastore.save({
  key,
  data: {
    title: 'smoke',
    priority: 1,
    done: false,
  },
})

console.log('2. get by key')
const [fetched] = await datastore.get(key)
if (!fetched || fetched.title !== 'smoke') {
  throw new Error(`expected saved entity, got ${JSON.stringify(fetched)}`)
}

console.log('3. run query')
const query = datastore.createQuery(kind).filter('priority', '=', 1).limit(10)
const [rows] = await datastore.runQuery(query)
if (!rows.some((row) => row[datastore.KEY].name === name)) {
  throw new Error(`query did not return the saved entity: ${JSON.stringify(rows)}`)
}

console.log('4. allocate ids')
const incomplete = datastore.key([kind])
const [allocated] = await datastore.allocateIds(incomplete, 2)
if (allocated.length !== 2 || allocated.some((k) => !k.id)) {
  throw new Error(`allocateIds failed: ${JSON.stringify(allocated)}`)
}

console.log('5. transaction')
const txKey = datastore.key([kind, `tx-${Date.now()}`])
const tx = datastore.transaction()
await tx.run()
tx.save({ key: txKey, data: { title: 'tx', priority: 2 } })
await tx.commit()
const [txFetched] = await datastore.get(txKey)
if (!txFetched || txFetched.title !== 'tx') {
  throw new Error(`transaction write missing: ${JSON.stringify(txFetched)}`)
}

console.log('6. property transforms')
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

console.log('7. maximum stays refused')
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

console.log('8. delete')
await datastore.delete(key)
const [afterDelete] = await datastore.get(key)
if (afterDelete) {
  throw new Error('entity still present after delete')
}

console.log('OK — Datastore SDK smoke passed')
