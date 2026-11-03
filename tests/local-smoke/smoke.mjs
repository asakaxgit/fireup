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
