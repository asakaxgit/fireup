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
// The checks are grouped as document CRUD (Phase 0/1), then queries, collection
// listings, and transactions (Phase 2), then realtime listeners (Phase 2.1).
// The SDK picks its own RPCs: .get() on a document goes to BatchGetDocuments,
// .set() goes to Commit, a query goes to RunQuery, listDocuments() goes to
// ListDocuments with show_missing, runTransaction() goes to BeginTransaction +
// Commit, and onSnapshot() opens a bidirectional Listen stream it drives
// itself. That is exactly why this layer exists alongside the Rust tests.

import { FieldPath, FieldValue, Firestore, Timestamp } from '@google-cloud/firestore'
import { strict as assert } from 'node:assert'

import { writeResults } from '../smoke-gate/results.mjs'

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
  writeResults(checks)
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

// How long a listener gets to deliver a snapshot. Long enough not to flake,
// short enough that a listener which never fires fails the check instead of
// hanging the run — which is the failure mode a realtime listener has.
const WATCH_TIMEOUT_MS = 5000

const withTimeout = (promise, label) => {
  let timer
  return Promise.race([
    promise.finally(() => clearTimeout(timer)),
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`timed out after ${WATCH_TIMEOUT_MS}ms waiting for ${label}`)), WATCH_TIMEOUT_MS)
    }),
  ])
}

// Turn an `onSnapshot` listener into a queue a check can await one snapshot at
// a time. Written this way because the interesting assertions are about the
// *sequence* of snapshots — initial state, then one per change — and a plain
// callback makes that ordering impossible to assert.
const snapshotQueue = (attach) => {
  const queued = []
  const waiters = []

  const deliver = (value) => {
    const waiter = waiters.shift()
    if (waiter) {
      waiter(value)
    } else {
      queued.push(value)
    }
  }

  const unsubscribe = attach(
    (snapshot) => deliver({ snapshot }),
    (error) => deliver({ error }),
  )

  const next = async (label) => {
    const value =
      queued.shift() ??
      (await withTimeout(new Promise((resolve) => waiters.push(resolve)), label))
    if (value.error) {
      throw value.error
    }
    return value.snapshot
  }

  return { next, unsubscribe }
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

  // ---- Query fidelity 2.1: cursors, collection groups, nested paths --------
  //
  // The SDK builds these request shapes itself: a cursor becomes a positional
  // start_at/end_at proto, a dotted string becomes a nested field path, and a
  // FieldPath with one dotted segment becomes a backtick-quoted one. Driving
  // them from here is the check that our reading of the protos matches the
  // client's, rather than only matching itself.

  await check('startAt()/startAfter() bound the results from below', async () => {
    const ids = async (query) => (await query.get()).docs.map((d) => d.id)
    const ordered = cities.orderBy('population')

    assert.deepEqual(await ids(ordered.startAt(19)), ['osaka', 'seoul', 'tokyo'])
    assert.deepEqual(await ids(ordered.startAfter(19)), ['seoul', 'tokyo'])
  })

  await check('endAt()/endBefore() bound the results from above', async () => {
    const ids = async (query) => (await query.get()).docs.map((d) => d.id)
    const ordered = cities.orderBy('population')

    assert.deepEqual(await ids(ordered.endAt(25)), ['kyoto', 'osaka', 'seoul'])
    assert.deepEqual(await ids(ordered.endBefore(25)), ['kyoto', 'osaka'])
  })

  await check('a start and an end cursor make a range', async () => {
    const snapshot = await cities.orderBy('population').startAfter(15).endBefore(37).get()
    assert.deepEqual(snapshot.docs.map((d) => d.id), ['osaka', 'seoul'])
  })

  await check('startAfter(snapshot) pages through a collection exactly once', async () => {
    // The paging idiom from the published docs. The SDK sends every ordered
    // value plus the document's own name, so this is also the check that the
    // implicit __name__ tiebreak lines up with the cursor.
    const seen = []
    let cursor = null
    for (let page = 0; page < 10; page += 1) {
      let query = cities.orderBy('population').limit(2)
      if (cursor) {
        query = query.startAfter(cursor)
      }
      const snapshot = await query.get()
      if (snapshot.empty) {
        break
      }
      seen.push(...snapshot.docs.map((d) => d.id))
      cursor = snapshot.docs[snapshot.docs.length - 1]
    }

    assert.deepEqual(seen, ['kyoto', 'osaka', 'seoul', 'tokyo'], 'each document exactly once, in order')
  })

  await check('a cursor on a tied sort field does not repeat or drop a document', async () => {
    // Three documents share a population, so only __name__ separates them.
    const tied = db.collection('tied')
    const batch = db.batch()
    for (const id of ['a', 'b', 'c']) {
      batch.set(tied.doc(id), { population: 7 })
    }
    batch.set(tied.doc('d'), { population: 9 })
    await batch.commit()

    const first = await tied.orderBy('population').limit(2).get()
    assert.deepEqual(first.docs.map((d) => d.id), ['a', 'b'])

    const next = await tied.orderBy('population').startAfter(first.docs[1]).get()
    assert.deepEqual(next.docs.map((d) => d.id), ['c', 'd'], 'the tie must not swallow c')
  })

  await check('collectionGroup() spans every depth under the database', async () => {
    const batch = db.batch()
    batch.set(db.doc('teams/a').collection('members').doc('amy'), { rank: 2 })
    batch.set(db.doc('teams/b').collection('members').doc('zoe'), { rank: 3 })
    batch.set(db.doc('teams/a/squads/x').collection('members').doc('deep'), { rank: 1 })
    // A different collection at the same depth must not be swept in.
    batch.set(db.doc('teams/a').collection('guests').doc('gil'), { rank: 9 })
    await batch.commit()

    const snapshot = await db.collectionGroup('members').orderBy('rank').get()
    assert.deepEqual(
      snapshot.docs.map((d) => d.ref.path),
      ['teams/a/squads/x/members/deep', 'teams/a/members/amy', 'teams/b/members/zoe'],
    )
  })

  await check('collectionGroup() still filters, orders and limits', async () => {
    const snapshot = await db.collectionGroup('members').where('rank', '>', 1).orderBy('rank').limit(1).get()
    assert.deepEqual(snapshot.docs.map((d) => d.ref.path), ['teams/a/members/amy'])
  })

  const residents = db.collection('residents')

  await check('a nested field path filters on the value inside the map', async () => {
    const batch = db.batch()
    batch.set(residents.doc('alice'), { address: { city: 'Tokyo', zip: '100-0001' }, name: 'Alice' })
    batch.set(residents.doc('bob'), { address: { city: 'Kyoto', zip: '600-8216' }, name: 'Bob' })
    await batch.commit()

    const snapshot = await residents.where('address.city', '==', 'Tokyo').get()
    assert.deepEqual(snapshot.docs.map((d) => d.id), ['alice'])
  })

  await check('a nested field path orders by the value inside the map', async () => {
    const snapshot = await residents.orderBy('address.city').get()
    assert.deepEqual(snapshot.docs.map((d) => d.id), ['bob', 'alice'], 'Kyoto before Tokyo')
  })

  await check('select() on a nested path returns the nesting, not a flat key', async () => {
    const snapshot = await residents.select('address.city').where('address.city', '==', 'Tokyo').get()
    assert.deepEqual(snapshot.docs[0].data(), { address: { city: 'Tokyo' } })
    assert.equal(snapshot.docs[0].get('address.city'), 'Tokyo')
  })

  await check('a one-segment FieldPath addresses a field whose name contains a dot', async () => {
    // The SDK serializes this as the backtick-quoted `address.city`, which has
    // to select the literal field rather than walk into the map.
    const dotted = new FieldPath('address.city')
    await residents.doc('dan').set({ 'address.city': 'Tokyo' })

    const literal = await residents.where(dotted, '==', 'Tokyo').get()
    assert.deepEqual(literal.docs.map((d) => d.id), ['dan'], 'the quoted path is one field name')

    const nested = await residents.where('address.city', '==', 'Tokyo').get()
    assert.deepEqual(nested.docs.map((d) => d.id), ['alice'], 'the dotted string walks into the map')
  })

  await check('a nested fieldMask on getAll() selects only that leaf', async () => {
    // getAll() is where the SDK exposes a read mask; it goes to
    // BatchGetDocuments with a DocumentMask on the wire.
    const [snapshot] = await db.getAll(residents.doc('alice'), { fieldMask: ['address.zip'] })
    assert.deepEqual(snapshot.data(), { address: { zip: '100-0001' } })
  })

  await check('a one-segment FieldPath in a fieldMask stays one field name', async () => {
    const [snapshot] = await db.getAll(residents.doc('dan'), {
      fieldMask: [new FieldPath('address.city')],
    })
    assert.deepEqual(snapshot.data(), { 'address.city': 'Tokyo' })
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

  // ---- Field transforms ----------------------------------------------------
  //
  // The SDK decides how a FieldValue sentinel reaches the wire — a masked
  // update plus update_transforms, a transform-only write, or a transform
  // riding on a full set() — so this is the layer that proves we read whatever
  // shape it picks. The values themselves are computed by the server, which is
  // the point: the client never sends them.

  const counters = db.collection('counters')

  await check('serverTimestamp(), increment(), and arrayUnion() resolve on a set()', async () => {
    const ref = counters.doc('page')
    await ref.set({
      label: 'home',
      seen: FieldValue.serverTimestamp(),
      hits: FieldValue.increment(5),
      tags: FieldValue.arrayUnion('a', 'b'),
    })

    const snapshot = await ref.get()
    assert.equal(snapshot.get('label'), 'home')
    assert.equal(snapshot.get('hits'), 5, 'increment on a missing field sets it to the operand')
    assert.deepEqual(snapshot.get('tags'), ['a', 'b'])
    assert.ok(
      snapshot.get('seen') instanceof Timestamp,
      `serverTimestamp() should come back as a Timestamp, got ${snapshot.get('seen')}`,
    )
    assert.ok(
      snapshot.get('seen').toMillis() <= snapshot.updateTime.toMillis(),
      'the server timestamp must not be after the write that produced it',
    )
  })

  await check('increment() accumulates and arrayUnion() stays idempotent across updates', async () => {
    const ref = counters.doc('page')
    await ref.update({ hits: FieldValue.increment(3), tags: FieldValue.arrayUnion('b', 'c') })
    await ref.update({ hits: FieldValue.increment(-1) })

    const snapshot = await ref.get()
    assert.equal(snapshot.get('hits'), 7, 'each increment reads the value the last one wrote')
    assert.deepEqual(snapshot.get('tags'), ['a', 'b', 'c'], 'an element already present is not appended twice')
    assert.equal(snapshot.get('label'), 'home', 'a transform must not disturb the other fields')
  })

  await check('arrayRemove() drops the elements it names', async () => {
    const ref = counters.doc('page')
    await ref.update({ tags: FieldValue.arrayRemove('a', 'c') })

    assert.deepEqual((await ref.get()).get('tags'), ['b'])
  })

  await check('FieldValue.delete() removes the field and leaves the rest', async () => {
    // The SDK sends this as an update_mask naming the field with no value for it
    // in the document — the same wire shape a Datastore property_mask uses to
    // delete a property. Until the shared store could delete a field, this
    // silently left the field in place.
    const ref = counters.doc('deletable')
    await ref.set({ keep: 'yes', drop: 'no', count: 3 })
    await ref.update({ drop: FieldValue.delete() })

    const snapshot = await ref.get()
    assert.equal(snapshot.get('drop'), undefined, 'the deleted field is gone')
    assert.equal(snapshot.get('keep'), 'yes', 'an unnamed field is untouched')
    assert.equal(snapshot.get('count'), 3)
    assert.deepEqual(Object.keys(snapshot.data()).sort(), ['count', 'keep'])
  })

  await check('increment() on a float field stays a float', async () => {
    const ref = counters.doc('ratio')
    await ref.set({ value: 1.5 })
    await ref.update({ value: FieldValue.increment(1) })

    assert.equal((await ref.get()).get('value'), 2.5)
  })

  await check('concurrent increments all land', async () => {
    // The whole reason a transform is computed server-side: ten clients adding
    // one each must end at ten, which a read-modify-write from the client
    // cannot guarantee.
    const ref = counters.doc('contended')
    await ref.set({ hits: 0 })
    await Promise.all(Array.from({ length: 10 }, () => ref.update({ hits: FieldValue.increment(1) })))

    assert.equal((await ref.get()).get('hits'), 10, 'no increment may be lost')
  })

  await check('a transform the server does not serve reports UNIMPLEMENTED', async () => {
    // Nested field paths are a documented gap, and a transform on one has to
    // fail loudly rather than write to a top-level field called "a.b".
    await expectCode(GRPC_UNIMPLEMENTED, () =>
      counters.doc('page').update({ 'nested.at': FieldValue.serverTimestamp() }),
    )
  })

  // ---- Phase 2.1: realtime listeners ---------------------------------------
  //
  // The SDK drives these over the bidirectional `Listen` stream and does a lot
  // of its own bookkeeping: it assigns the target id, accumulates document
  // changes, and only hands a snapshot to the application when the server says
  // the stream has reached a consistent point. None of that is visible from the
  // Rust-side tests, which is exactly why these checks live here.

  const watched = db.collection('watched')

  await check('onSnapshot() on a document delivers the current state, then each change', async () => {
    const ref = watched.doc('alice')
    await ref.set({ city: 'SF' })

    const listener = snapshotQueue((onNext, onError) => ref.onSnapshot(onNext, onError))
    try {
      const initial = await listener.next('the initial document snapshot')
      assert.equal(initial.exists, true, 'a listener opens with the state already in the store')
      assert.equal(initial.get('city'), 'SF')

      await ref.update({ city: 'LA' })
      const updated = await listener.next('the snapshot after an update')
      assert.equal(updated.get('city'), 'LA')

      await ref.delete()
      const deleted = await listener.next('the snapshot after a delete')
      assert.equal(deleted.exists, false, 'a delete reaches the listener as a non-existent snapshot')
    } finally {
      listener.unsubscribe()
    }
  })

  await check('onSnapshot() on a missing document reports a non-existent snapshot', async () => {
    const ref = watched.doc('never-written')
    const listener = snapshotQueue((onNext, onError) => ref.onSnapshot(onNext, onError))
    try {
      const initial = await listener.next('the initial snapshot of a missing document')
      assert.equal(initial.exists, false)

      // And it still fires when the document appears.
      await ref.set({ city: 'NYC' })
      const created = await listener.next('the snapshot after the document is created')
      assert.equal(created.exists, true)
      assert.equal(created.get('city'), 'NYC')
    } finally {
      listener.unsubscribe()
    }
  })

  await check('onSnapshot() on a query reports documents entering and leaving the result set', async () => {
    const query = watched.where('city', '==', 'SF')
    const listener = snapshotQueue((onNext, onError) => query.onSnapshot(onNext, onError))
    try {
      const initial = await listener.next('the initial query snapshot')
      assert.equal(initial.size, 0, 'nothing matches yet')

      await watched.doc('bob').set({ city: 'SF' })
      const added = await listener.next('the snapshot after a matching write')
      assert.deepEqual(added.docs.map((d) => d.id), ['bob'])
      assert.deepEqual(
        added.docChanges().map((c) => [c.type, c.doc.id]),
        [['added', 'bob']],
        'the SDK should see this as an addition to the result set',
      )

      // Still in the collection, no longer in the result set.
      await watched.doc('bob').update({ city: 'LA' })
      const dropped = await listener.next('the snapshot after the document stops matching')
      assert.equal(dropped.size, 0)
      assert.deepEqual(
        dropped.docChanges().map((c) => [c.type, c.doc.id]),
        [['removed', 'bob']],
      )
    } finally {
      listener.unsubscribe()
    }
  })

  await check('onSnapshot() on a query ignores writes outside it', async () => {
    const query = watched.where('city', '==', 'SF')
    const listener = snapshotQueue((onNext, onError) => query.onSnapshot(onNext, onError))
    try {
      await listener.next('the initial query snapshot')

      await db.collection('unwatched').doc('x').set({ city: 'SF' })
      await users.doc('alice').update({ age: 99 })

      await assert.rejects(
        listener.next('a snapshot that should never arrive'),
        /timed out/,
        'a write outside the query must not wake its listener',
      )
    } finally {
      listener.unsubscribe()
    }
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
