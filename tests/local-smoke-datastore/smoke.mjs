/**
 * Phase 3 exit criterion: prove an *official* Datastore client SDK can talk to
 * `fireup local` over DATASTORE_EMULATOR_HOST.
 *
 * Written against the public @google-cloud/datastore API. No emulator binary
 * was reverse-engineered.
 */

import { Datastore } from '@google-cloud/datastore'

const projectId = process.env.FIREUP_SMOKE_PROJECT_ID || 'fireup-local'
const datastore = new Datastore({ projectId })

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

console.log('6. delete')
await datastore.delete(key)
const [afterDelete] = await datastore.get(key)
if (afterDelete) {
  throw new Error('entity still present after delete')
}

console.log('OK — Datastore SDK smoke passed')
