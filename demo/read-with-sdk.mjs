// The SDK half of the end-to-end demo: read the migrated export back with
// Google's *official*, open-source client libraries.
//
// Nothing here is Fireup-specific. The only thing that points these SDKs at
// Fireup instead of production Google is the documented emulator-host
// environment variable, which is the whole point: a real application's data
// layer needs no change to run against the local data plane.
//
// Run it through `scripts/demo-export-to-sql.sh`, which brings up Postgres,
// imports and seeds the export, starts `fireup local`, and then runs the SQL
// half against the same database. Assumes FIRESTORE_EMULATOR_HOST and
// DATASTORE_EMULATOR_HOST are already exported.

import { Datastore, PropertyFilter } from '@google-cloud/datastore'
import { DocumentReference, Firestore, GeoPoint, Timestamp } from '@google-cloud/firestore'
import { strict as assert } from 'node:assert'
import { Buffer } from 'node:buffer'

const projectId = process.env.FIREUP_DEMO_PROJECT_ID ?? 'fireup-demo'

// The one document this demo creates. The SQL half greps for it by name.
const WRITTEN_ID = 'demo-sdk-write'

const step = (text) => console.log(`\n  ${text}`)
const show = (label, value) => console.log(`    ${label.padEnd(26)} ${value}`)

const main = async () => {
  const firestoreHost = process.env.FIRESTORE_EMULATOR_HOST
  const datastoreHost = process.env.DATASTORE_EMULATOR_HOST
  assert.ok(firestoreHost, 'FIRESTORE_EMULATOR_HOST must be set')
  assert.ok(datastoreHost, 'DATASTORE_EMULATOR_HOST must be set')

  const db = new Firestore({ projectId })

  step(`@google-cloud/firestore → FIRESTORE_EMULATOR_HOST=${firestoreHost}`)

  // Remove the one document a previous run of this demo created, so that the
  // reads below show the export and nothing else. Seeding is an upsert, so it
  // does not clear anything a client added afterwards.
  await db.doc(`Users/ada/Orders/${WRITTEN_ID}`).delete()

  // A document that came out of the export, read by resource path.
  const ada = await db.doc('Users/ada').get()
  assert.ok(ada.exists, 'Users/ada should have been seeded from the export')
  show('Users/ada', `${Object.keys(ada.data()).length} fields`)
  show('  display_name', JSON.stringify(ada.get('display_name')))
  show('  score', JSON.stringify(ada.get('score')))
  show('  address.city', JSON.stringify(ada.get('address').city))
  show('  tags', JSON.stringify(ada.get('tags')))

  // Entities exported under a child key path seed as a subcollection, so the
  // export's parent/child structure survives as Firestore structure.
  const orders = await db.collection('Users/ada/Orders').get()
  show('Users/ada/Orders', `${orders.size} documents: ${orders.docs.map((d) => d.id).join(', ')}`)

  // A query, compiled and served by fireup local — not a client-side filter.
  const active = await db.collection('Users').where('active', '==', true).orderBy('score', 'desc').get()
  show('active users by score', active.docs.map((d) => `${d.id}=${d.get('score')}`).join(', '))

  const bigOrders = await db.collection('Users/ada/Orders').where('total_cents', '>', 1000).get()
  show('ada orders > 1000', bigOrders.docs.map((d) => `${d.id}=${d.get('total_cents')}`).join(', '))

  // A write, so the SQL half can show it landing in the same Postgres row set.
  await db.doc(`Users/ada/Orders/${WRITTEN_ID}`).set({
    currency: 'EUR',
    total_cents: 4200,
    shipped: false,
    note: 'written by the official Firestore SDK',
  })
  show('wrote', `Users/ada/Orders/${WRITTEN_ID}`)

  // The same rows, through the Datastore API. Kinds are collections and key
  // paths are document paths, so this is not a copy — it is the same store.
  step(`@google-cloud/datastore → DATASTORE_EMULATOR_HOST=${datastoreHost}`)

  const ds = new Datastore({ projectId })
  const [adaEntity] = await ds.get(ds.key(['Users', 'ada']))
  assert.ok(adaEntity, 'Users/ada should be reachable as a Datastore entity too')
  show('Users key ada', JSON.stringify(adaEntity.display_name))

  const activeQuery = ds.createQuery('Users').filter(new PropertyFilter('active', '=', true))
  const [activeEntities] = await ds.runQuery(activeQuery)
  show('kind query Users', `${activeEntities.length} entities`)

  // The SDK write above is visible here as well, for the same reason.
  const [writtenEntity] = await ds.get(ds.key(['Users', 'ada', 'Orders', WRITTEN_ID]))
  assert.ok(writtenEntity, 'the Firestore write should be visible through the Datastore API')
  show('the Firestore write', `reachable as Datastore key Users/ada > Orders/${WRITTEN_ID}`)

  // The four Datastore types JSON has no native form for. The migrator's export
  // parser hands them to `fireup local seed` as tagged maps; seeding reads those
  // tags, so what arrives here is the real Firestore value type and no client
  // needs to know that `__fireup_type__` exists.
  step('Datastore value types, as seen by the SDK')

  const grace = await db.doc('Users/grace').get()
  assert.ok(grace.exists, 'Users/grace should have been seeded from the export')

  const typeName = (value) => value?.constructor?.name ?? typeof value

  const createdAt = grace.get('created_at')
  show('created_at', `${typeName(createdAt)} ${createdAt.toDate().toISOString()}`)
  assert.ok(createdAt instanceof Timestamp, `created_at should be a Timestamp, got ${typeName(createdAt)}`)

  const avatar = grace.get('avatar')
  show('avatar', `${typeName(avatar)} of ${avatar.length} bytes`)
  assert.ok(Buffer.isBuffer(avatar), `avatar should be bytes, got ${typeName(avatar)}`)

  const location = grace.get('location')
  show('location', `${typeName(location)} ${location.latitude}, ${location.longitude}`)
  assert.ok(location instanceof GeoPoint, `location should be a GeoPoint, got ${typeName(location)}`)

  // A key reference comes back as a reference to a document in this database,
  // so it can be followed without reconstructing a path by hand.
  const manager = grace.get('manager')
  show('manager', `${typeName(manager)} → ${manager.path}`)
  assert.ok(manager instanceof DocumentReference, `manager should be a reference, got ${typeName(manager)}`)
  const managed = await manager.get()
  assert.ok(managed.exists, 'the reference should resolve to a seeded document')
  show('  followed', JSON.stringify(managed.get('display_name')))

  // An ordinary nested object is still an ordinary map.
  show('address', `${typeName(grace.get('address'))} with city ${JSON.stringify(grace.get('address').city)}`)
}

await main()
console.log('\n  SDK reads and writes: ok')
