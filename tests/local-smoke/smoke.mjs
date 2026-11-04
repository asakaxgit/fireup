// SDK handshake smoke test for `fireup local`.
//
// This is the test that actually matters for the Phase 0 goal: it uses Google's
// *official* Firestore client library (@google-cloud/firestore, Apache-2.0),
// pointed at Fireup with the documented FIRESTORE_EMULATOR_HOST variable, and
// never touches a Fireup-internal API. If the SDK's own serialization, RPC
// selection, or error mapping disagrees with our server, this fails.
//
// Run it via scripts/smoke-local-firestore.sh, which starts and stops the
// server for you. Assumes FIRESTORE_EMULATOR_HOST is already exported.
//
// The checks are grouped as document CRUD (Phase 0/1) and then queries,
// collection listings, and transactions (Phase 2). The SDK picks its own RPCs:
// .get() on a document goes to BatchGetDocuments, .set() goes to Commit, a
// query goes to RunQuery, listDocuments() goes to ListDocuments with
// show_missing, and runTransaction() goes to BeginTransaction + Commit. That is
// exactly why this layer exists alongside the Rust tests.

import { Firestore } from '@google-cloud/firestore'
import { strict as assert } from 'node:assert'

const PROJECT_ID = 'fireup-smoke'

const checks = []

const check = async (name, fn) => {
  try {
    await fn()
    checks.push({ name, ok: true })
    console.log(`  ok   ${name}`)
  } catch (error) {
    checks.push({ name, ok: false, error })
    console.log(`  FAIL ${name}`)
    console.log(`       ${error?.message ?? error}`)
  }
}

// gRPC status codes we assert on, from the public gRPC spec.
const GRPC_ALREADY_EXISTS = 6
const GRPC_NOT_FOUND = 5
const GRPC_UNIMPLEMENTED = 12

const expectCode = async (expected, fn) => {
  try {
    await fn()
  } catch (error) {
    assert.equal(error.code, expected, `expected gRPC code ${expected}, got ${error.code}: ${error.message}`)
    return
  }
  assert.fail(`expected the call to fail with gRPC code ${expected}, but it succeeded`)
}

const main = async () => {
  const host = process.env.FIRESTORE_EMULATOR_HOST
  assert.ok(host, 'FIRESTORE_EMULATOR_HOST must be set')
  console.log(`Driving fireup local at ${host} with @google-cloud/firestore\n`)

  const db = new Firestore({ projectId: PROJECT_ID })
  const users = db.collection('users')

  await check('set() then get() round-trips a document', async () => {
    const ref = users.doc('alice')
    await ref.set({ name: 'Alice', age: 31, active: true })

    const snapshot = await ref.get()
    assert.ok(snapshot.exists, 'document should exist after set()')
    assert.deepEqual(snapshot.data(), { name: 'Alice', age: 31, active: true })
    assert.equal(snapshot.id, 'alice')
    assert.ok(snapshot.createTime, 'server should stamp createTime')
    assert.ok(snapshot.updateTime, 'server should stamp updateTime')
  })

  await check('get() on a missing document returns a non-existent snapshot', async () => {
    const snapshot = await users.doc('nobody').get()
    assert.equal(snapshot.exists, false)
    assert.equal(snapshot.data(), undefined)
  })

  await check('update() merges fields instead of replacing the document', async () => {
    const ref = users.doc('alice')
    await ref.update({ age: 32 })

    const snapshot = await ref.get()
    assert.deepEqual(snapshot.data(), { name: 'Alice', age: 32, active: true })
  })

  await check('set({merge:true}) merges fields', async () => {
    const ref = users.doc('alice')
    await ref.set({ nickname: 'Al' }, { merge: true })

    const snapshot = await ref.get()
    assert.equal(snapshot.get('nickname'), 'Al')
    assert.equal(snapshot.get('name'), 'Alice', 'merge must not drop existing fields')
  })

  await check('set() without merge replaces all fields', async () => {
    const ref = users.doc('alice')
    await ref.set({ name: 'Alice' })

    const snapshot = await ref.get()
    assert.deepEqual(snapshot.data(), { name: 'Alice' })
  })

  await check('create() assigns an auto id when the client asks for one', async () => {
    const ref = await users.add({ name: 'Anonymous' })
    assert.equal(ref.id.length, 20, `auto ids are 20 chars, got ${ref.id}`)

    const snapshot = await ref.get()
    assert.equal(snapshot.get('name'), 'Anonymous')
  })

  await check('create() on an existing document fails with ALREADY_EXISTS', async () => {
    const ref = users.doc('bob')
    await ref.create({ name: 'Bob' })
    await expectCode(GRPC_ALREADY_EXISTS, () => ref.create({ name: 'Bob again' }))
  })

  await check('update() on a missing document fails with NOT_FOUND', async () => {
    await expectCode(GRPC_NOT_FOUND, () => users.doc('ghost').update({ name: 'Ghost' }))
  })

  await check('delete() removes the document and is idempotent', async () => {
    const ref = users.doc('bob')
    await ref.delete()
    assert.equal((await ref.get()).exists, false)

    await ref.delete()
    assert.equal((await ref.get()).exists, false, 'a second delete must not resurrect it')
  })

  await check('getAll() reports found and missing documents in one call', async () => {
    const [found, missing] = await db.getAll(users.doc('alice'), users.doc('nobody'))
    assert.equal(found.exists, true)
    assert.equal(found.get('name'), 'Alice')
    assert.equal(missing.exists, false)
  })

  await check('subcollection documents are addressable', async () => {
    const ref = users.doc('alice').collection('posts').doc('p1')
    await ref.set({ title: 'Hello' })

    const snapshot = await ref.get()
    assert.equal(snapshot.get('title'), 'Hello')
    assert.equal(snapshot.ref.path, 'users/alice/posts/p1')
  })


  // ---- Phase 2: queries, listings, transactions ----------------------------

  const cities = db.collection('cities')

  await check('a collection can be seeded with a batch', async () => {
    const batch = db.batch()
    batch.set(cities.doc('tokyo'), { country: 'JP', population: 37, tags: ['capital', 'big'] })
    batch.set(cities.doc('osaka'), { country: 'JP', population: 19, tags: ['big'] })
    batch.set(cities.doc('kyoto'), { country: 'JP', population: 15, tags: ['old'] })
    batch.set(cities.doc('seoul'), { country: 'KR', population: 25, tags: ['capital'] })
    await batch.commit()

    assert.equal((await cities.doc('tokyo').get()).get('population'), 37)
  })

  await check('where() + orderBy() + limit() returns the right page in the right order', async () => {
    const snapshot = await cities
      .where('country', '==', 'JP')
      .orderBy('population', 'desc')
      .limit(2)
      .get()

    assert.deepEqual(snapshot.docs.map((d) => d.id), ['tokyo', 'osaka'])
    assert.equal(snapshot.size, 2)
    assert.equal(snapshot.empty, false)
  })

  await check('an inequality filter works', async () => {
    const snapshot = await cities.where('population', '>', 18).orderBy('population').get()
    assert.deepEqual(snapshot.docs.map((d) => d.id), ['osaka', 'seoul', 'tokyo'])
  })

  await check("where('in') matches any of the listed values", async () => {
    const snapshot = await cities.where('country', 'in', ['KR', 'TW']).get()
    assert.deepEqual(snapshot.docs.map((d) => d.id), ['seoul'])
  })

  await check('array-contains looks inside an array field', async () => {
    const snapshot = await cities.where('tags', 'array-contains', 'capital').get()
    assert.deepEqual(snapshot.docs.map((d) => d.id).sort(), ['seoul', 'tokyo'])
  })

  await check('a query with no matches returns an empty snapshot, not an error', async () => {
    const snapshot = await cities.where('country', '==', 'NOWHERE').get()
    assert.equal(snapshot.empty, true)
    assert.equal(snapshot.size, 0)
  })

  await check('select() returns only the projected fields', async () => {
    const snapshot = await cities.select('country').where('country', '==', 'KR').get()
    assert.deepEqual(snapshot.docs[0].data(), { country: 'KR' })
    assert.equal(snapshot.docs[0].id, 'seoul')
  })

  await check('limitToLast() returns the end of the ordered results', async () => {
    // The SDK implements this by flipping the order and reversing client-side,
    // so it is an ordinary query on the wire — worth pinning down, because the
    // fidelity matrix claims it works.
    const snapshot = await cities.orderBy('population').limitToLast(2).get()
    assert.deepEqual(snapshot.docs.map((d) => d.id), ['seoul', 'tokyo'])
  })

  await check('a document reference round-trips through a query result', async () => {
    const snapshot = await cities.where('country', '==', 'KR').get()
    const reloaded = await snapshot.docs[0].ref.get()
    assert.equal(reloaded.get('population'), 25)
  })

  await check('listDocuments() lists a collection, including documents that only have children', async () => {
    await cities.doc('ghost').collection('districts').doc('d1').set({ name: 'Nowhere' })

    const refs = await cities.listDocuments()
    const ids = refs.map((ref) => ref.id).sort()
    assert.deepEqual(ids, ['ghost', 'kyoto', 'osaka', 'seoul', 'tokyo'])
  })

  await check('listCollections() reports collections at the root and under a document', async () => {
    const root = await db.listCollections()
    const ids = root.map((collection) => collection.id)
    assert.ok(ids.includes('cities'), `expected cities in ${JSON.stringify(ids)}`)
    assert.ok(ids.includes('users'), `expected users in ${JSON.stringify(ids)}`)

    const nested = await cities.doc('ghost').listCollections()
    assert.deepEqual(nested.map((collection) => collection.id), ['districts'])
  })

  await check('runTransaction() reads and then writes atomically', async () => {
    const ref = cities.doc('tokyo')
    const result = await db.runTransaction(async (transaction) => {
      const snapshot = await transaction.get(ref)
      const next = snapshot.get('population') + 1
      transaction.update(ref, { population: next })
      return next
    })

    assert.equal(result, 38)
    assert.equal((await ref.get()).get('population'), 38)
  })

  await check('runTransaction() rolls back when the body throws', async () => {
    const ref = cities.doc('kyoto')
    await assert.rejects(
      db.runTransaction(async (transaction) => {
        const snapshot = await transaction.get(ref)
        transaction.update(ref, { population: snapshot.get('population') + 100 })
        throw new Error('nope')
      }),
      /nope/,
    )

    assert.equal((await ref.get()).get('population'), 15, 'the aborted write must not land')
  })

  await check('a transaction retries and still converges under contention', async () => {
    const ref = cities.doc('osaka')
    // The SDK retries an ABORTED transaction. Forcing one write from outside
    // during the first attempt exercises that path end to end.
    let attempts = 0
    await db.runTransaction(async (transaction) => {
      attempts += 1
      const snapshot = await transaction.get(ref)
      if (attempts === 1) {
        await ref.update({ population: 20 })
      }
      transaction.update(ref, { population: snapshot.get('population') + 1 })
    })

    assert.ok(attempts > 1, `the transaction should have been retried, attempts=${attempts}`)
    assert.equal((await ref.get()).get('population'), 21, 'the retry must read the new value')
  })

  await check('a BulkWriter applies independent writes', async () => {
    const writer = db.bulkWriter()
    writer.set(cities.doc('nagoya'), { country: 'JP', population: 9 })
    writer.set(cities.doc('sapporo'), { country: 'JP', population: 5 })
    await writer.close()

    assert.equal((await cities.doc('nagoya').get()).get('population'), 9)
    assert.equal((await cities.doc('sapporo').get()).get('population'), 5)
  })

  await check('an unsupported query reports a clear UNIMPLEMENTED instead of hanging', async () => {
    // Aggregations are deliberately not served yet. The failure has to be
    // legible: the right gRPC code and a message that names the gap.
    try {
      await cities.count().get()
    } catch (error) {
      assert.equal(error.code, GRPC_UNIMPLEMENTED, `expected UNIMPLEMENTED, got ${error.code}: ${error.message}`)
      assert.match(error.message, /fireup local/)
      return
    }
    assert.fail('count() should not have succeeded')
  })

  await db.terminate()

  const failed = checks.filter((c) => !c.ok)
  console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`)
  if (failed.length > 0) {
    console.log('\nFailures:')
    for (const { name, error } of failed) {
      console.log(`- ${name}: ${error?.stack ?? error}`)
    }
    process.exit(1)
  }
  console.log('fireup local speaks the published Firestore API to the official SDK.')
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
