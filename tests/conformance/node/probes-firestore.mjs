/**
 * Probes for the `google.firestore.v1.Firestore` RPC rows of the fidelity
 * matrix, plus the transaction and "other known limits" claims the README
 * makes in prose.
 *
 * Each probe is `{ id, served?, unimplemented?, 'documented-divergence'? }`.
 * The runner picks the branch the matrix claims, so a status flip changes what
 * CI demands without editing this file's structure.
 */

import { strict as assert } from 'node:assert'
import { createRequire } from 'node:module'

import {
  GRPC,
  databaseName,
  documentName,
  documentsRoot,
  duplexFirstOutcome,
  plainFields,
  serverStream,
  unary,
  value,
} from './grpc-client.mjs'

const require = createRequire(import.meta.url)
const { FieldValue } = require('@google-cloud/firestore')

const expectCode = async (expected, fn) => {
  let error
  try {
    await fn()
  } catch (caught) {
    error = caught
  }
  assert.ok(error, `expected gRPC code ${expected}, but the call succeeded`)
  assert.equal(error.code, expected, `expected gRPC code ${expected}, got ${error.code}: ${error.message}`)
  return error
}

export const firestoreProbes = [
  {
    id: 'firestore.GetDocument',
    served: async ({ raw, db, projectId, runId }) => {
      const path = `conf_${runId}_getdocument/d1`
      await db.doc(path).set({ title: 'raw get', rank: 7 })

      const document = await unary(raw.firestore, 'GetDocument', {
        name: documentName(projectId, path),
      })
      assert.deepEqual(plainFields(document), { title: 'raw get', rank: 7 })
      assert.ok(document.create_time, 'GetDocument must return create_time')
      assert.ok(document.update_time, 'GetDocument must return update_time')

      await expectCode(GRPC.NOT_FOUND, () =>
        unary(raw.firestore, 'GetDocument', {
          name: documentName(projectId, `conf_${runId}_getdocument/nobody`),
        }),
      )
    },
  },

  {
    id: 'firestore.ListDocuments',
    served: async ({ db, freshCollection }) => {
      const cities = freshCollection('listdocuments')
      await Promise.all([
        cities.doc('a').set({ n: 1 }),
        cities.doc('b').set({ n: 2 }),
        cities.doc('c').set({ n: 3 }),
      ])
      // A document that exists only as a parent: show_missing is what makes it
      // appear, and the SDK asks for it.
      await cities.doc('ghost').collection('kids').doc('k').set({ n: 0 })

      const refs = await cities.listDocuments()
      assert.deepEqual(
        refs.map((ref) => ref.id),
        ['a', 'b', 'c', 'ghost'],
        'ListDocuments is in name order and includes parent-only documents',
      )

      // collection_id may be empty to span every collection under a parent.
      const everything = await db.listCollections()
      assert.ok(
        everything.some((collection) => collection.id === cities.id),
        'the collection must show up at the root',
      )
    },
  },

  {
    id: 'firestore.CreateDocument',
    served: async ({ raw, projectId, runId }) => {
      const parent = documentsRoot(projectId)
      const collectionId = `conf_${runId}_createdocument`

      const created = await unary(raw.firestore, 'CreateDocument', {
        parent,
        collection_id: collectionId,
        document_id: 'd1',
        document: { fields: { title: value.string('created') } },
      })
      assert.equal(created.name, `${parent}/${collectionId}/d1`)
      assert.deepEqual(plainFields(created), { title: 'created' })

      await expectCode(GRPC.ALREADY_EXISTS, () =>
        unary(raw.firestore, 'CreateDocument', {
          parent,
          collection_id: collectionId,
          document_id: 'd1',
          document: { fields: { title: value.string('again') } },
        }),
      )

      const auto = await unary(raw.firestore, 'CreateDocument', {
        parent,
        collection_id: collectionId,
        document: { fields: { title: value.string('auto') } },
      })
      const autoId = auto.name.split('/').pop()
      assert.equal(autoId.length, 20, `auto ids are 20 characters, got "${autoId}"`)
    },
  },

  {
    id: 'firestore.UpdateDocument',
    served: async ({ raw, db, projectId, runId }) => {
      const path = `conf_${runId}_updatedocument/d1`
      await db.doc(path).set({ a: 1, b: 2 })
      const name = documentName(projectId, path)

      const merged = await unary(raw.firestore, 'UpdateDocument', {
        document: { name, fields: { a: value.integer(9) } },
        update_mask: { field_paths: ['a'] },
      })
      assert.deepEqual(plainFields(merged), { a: 9, b: 2 }, 'update_mask merges, it does not replace')

      // current_document preconditions: exists=true on a document that is gone.
      await expectCode(GRPC.NOT_FOUND, () =>
        unary(raw.firestore, 'UpdateDocument', {
          document: {
            name: documentName(projectId, `conf_${runId}_updatedocument/missing`),
            fields: { a: value.integer(1) },
          },
          update_mask: { field_paths: ['a'] },
          current_document: { exists: true },
        }),
      )
    },
  },

  {
    id: 'firestore.DeleteDocument',
    served: async ({ raw, db, projectId, runId }) => {
      const path = `conf_${runId}_deletedocument/d1`
      await db.doc(path).set({ a: 1 })
      const name = documentName(projectId, path)

      await unary(raw.firestore, 'DeleteDocument', { name })
      await expectCode(GRPC.NOT_FOUND, () => unary(raw.firestore, 'GetDocument', { name }))

      // Idempotent: a second delete is not an error and does not resurrect it.
      await unary(raw.firestore, 'DeleteDocument', { name })
      assert.equal((await db.doc(path).get()).exists, false)
    },
  },

  {
    id: 'firestore.BatchGetDocuments',
    served: async ({ db, freshCollection }) => {
      const users = freshCollection('batchget')
      await users.doc('found').set({ name: 'Ada' })

      const [found, missing] = await db.getAll(users.doc('found'), users.doc('nobody'))
      assert.equal(found.exists, true)
      assert.equal(found.get('name'), 'Ada')
      assert.equal(missing.exists, false, 'BatchGetDocuments reports a missing document, not an error')
      assert.ok(found.readTime, 'the SDK expects a read_time on each result')
    },
  },

  {
    id: 'firestore.BeginTransaction',
    served: async ({ db, freshCollection }) => {
      const counters = freshCollection('begintransaction')
      const ref = counters.doc('c')
      await ref.set({ n: 1 })

      const result = await db.runTransaction(async (transaction) => {
        const snapshot = await transaction.get(ref)
        const next = snapshot.get('n') + 1
        transaction.update(ref, { n: next })
        return next
      })
      assert.equal(result, 2)
      assert.equal((await ref.get()).get('n'), 2)
    },
  },

  {
    id: 'firestore.Commit',
    served: async ({ db, freshCollection }) => {
      const cities = freshCollection('commit')
      await cities.doc('doomed').set({ n: 0 })

      const batch = db.batch()
      batch.set(cities.doc('created'), { n: 1 })
      batch.update(cities.doc('doomed'), { n: 2 })
      batch.delete(cities.doc('doomed'))
      const writeResults = await batch.commit()

      assert.equal(writeResults.length, 3, 'Commit returns one WriteResult per write')
      assert.equal((await cities.doc('created').get()).get('n'), 1)
      assert.equal((await cities.doc('doomed').get()).exists, false)
    },
  },

  {
    id: 'firestore.Rollback',
    served: async ({ db, raw, projectId, freshCollection }) => {
      const cities = freshCollection('rollback')
      const ref = cities.doc('kyoto')
      await ref.set({ n: 15 })

      await assert.rejects(
        db.runTransaction(async (transaction) => {
          const snapshot = await transaction.get(ref)
          transaction.update(ref, { n: snapshot.get('n') + 100 })
          throw new Error('abandon this transaction')
        }),
        /abandon this transaction/,
      )
      assert.equal((await ref.get()).get('n'), 15, 'the rolled-back write must not land')

      // An unknown token is rejected rather than silently accepted.
      await expectCode(GRPC.INVALID_ARGUMENT, () =>
        unary(raw.firestore, 'Rollback', {
          database: databaseName(projectId),
          transaction: Buffer.from('not-a-transaction-token'),
        }),
      )
    },
  },

  {
    id: 'firestore.RunQuery',
    served: async ({ freshCollection }) => {
      const cities = freshCollection('runquery')
      await Promise.all([
        cities.doc('tokyo').set({ country: 'JP', population: 37 }),
        cities.doc('osaka').set({ country: 'JP', population: 19 }),
        cities.doc('seoul').set({ country: 'KR', population: 25 }),
      ])

      const snapshot = await cities.where('country', '==', 'JP').orderBy('population', 'desc').get()
      assert.deepEqual(
        snapshot.docs.map((doc) => doc.id),
        ['tokyo', 'osaka'],
      )
      assert.ok(snapshot.readTime, 'RunQuery results carry a read_time')
      assert.ok(snapshot.docs[0].createTime, 'each result carries its document metadata')
    },
  },

  {
    id: 'firestore.BatchWrite',
    served: async ({ db, freshCollection }) => {
      const cities = freshCollection('batchwrite')
      await cities.doc('exists').set({ n: 1 })

      // BulkWriter is the SDK's BatchWrite path. BatchWrite is deliberately
      // *not* atomic — one status per write — so a failing write must not take
      // its neighbours down with it.
      const writer = db.bulkWriter()
      writer.onWriteError(() => false) // do not retry; we want the first verdict
      const rejected = writer
        .create(cities.doc('exists'), { n: 2 })
        .then(() => null)
        .catch((error) => error)
      const accepted = writer.set(cities.doc('fresh'), { n: 3 })
      await writer.close()

      const error = await rejected
      assert.ok(error, 'creating an existing document must fail')
      assert.equal(error.code, GRPC.ALREADY_EXISTS, `expected ALREADY_EXISTS, got ${error.code}`)
      await accepted
      assert.equal(
        (await cities.doc('fresh').get()).get('n'),
        3,
        'the sibling write in the same BatchWrite must still land',
      )
      assert.equal((await cities.doc('exists').get()).get('n'), 1)
    },
  },

  {
    id: 'firestore.ListCollectionIds',
    served: async ({ db, freshCollection }) => {
      const cities = freshCollection('listcollectionids')
      await cities.doc('tokyo').collection('districts').doc('d1').set({ n: 1 })

      const root = await db.listCollections()
      assert.ok(
        root.some((collection) => collection.id === cities.id),
        `expected ${cities.id} among the root collections`,
      )

      const nested = await cities.doc('tokyo').listCollections()
      assert.deepEqual(
        nested.map((collection) => collection.id),
        ['districts'],
      )
    },
  },

  {
    id: 'firestore.Listen',
    // Probed on the wire rather than through onSnapshot(): this layer asserts
    // the protocol, and the SDK's own `onSnapshot()` path is covered by
    // tests/local-smoke/smoke.mjs.
    unimplemented: async ({ raw, projectId, expectUnimplemented }) => {
      await expectUnimplemented(
        () =>
          duplexFirstOutcome(raw.firestore, 'Listen', {
            database: databaseName(projectId),
            add_target: {
              target_id: 1,
              query: {
                parent: documentsRoot(projectId),
                structured_query: { from: [{ collection_id: 'anything' }] },
              },
            },
          }),
        { names: /Listen/ },
      )
    },
    served: async ({ raw, db, projectId, runId }) => {
      await db.doc(`conf_${runId}_listen/tokyo`).set({ n: 1 })

      const message = await duplexFirstOutcome(raw.firestore, 'Listen', {
        database: databaseName(projectId),
        add_target: {
          target_id: 1,
          query: {
            parent: documentsRoot(projectId),
            structured_query: { from: [{ collection_id: `conf_${runId}_listen` }] },
          },
        },
      })
      assert.ok(message, 'Listen must produce a response, not just end the stream')
      assert.ok(
        message.target_change || message.document_change,
        `expected a target or document change, got ${JSON.stringify(message)}`,
      )
    },
  },

  {
    id: 'firestore.Write',
    unimplemented: async ({ raw, projectId, expectUnimplemented }) => {
      await expectUnimplemented(
        () => duplexFirstOutcome(raw.firestore, 'Write', { database: databaseName(projectId) }),
        { names: /Write/ },
      )
    },
    // Dormant while the matrix says `unimplemented` — the runner only runs the
    // branch the status names. It exists so observe.mjs can *measure* the row
    // against an implementation that does serve it. That measurement is what
    // turns a "Fireup refuses this" row in docs/local/COMPARISON.md from a claim
    // about our own code into a recorded answer from a reference implementation
    // — see docs/local/measurements/.
    served: async ({ raw, projectId, runId }) => {
      const message = await duplexFirstOutcome(raw.firestore, 'Write', {
        database: databaseName(projectId),
        writes: [
          {
            update: {
              name: documentName(projectId, `conf_${runId}_streamwrite/d1`),
              fields: { n: value.integer(1) },
            },
          },
        ],
      })
      assert.ok(message, 'the streaming Write must answer, not just end the stream')
      assert.ok(
        message.stream_id !== undefined || (message.write_results?.length ?? 0) > 0,
        `expected a stream id or a write result, got ${JSON.stringify(message)}`,
      )
    },
  },

  {
    id: 'firestore.RunAggregationQuery',
    unimplemented: async ({ freshCollection, expectUnimplemented }) => {
      const cities = freshCollection('aggregation');
      await cities.doc('tokyo').set({ n: 1 })
      await expectUnimplemented(() => cities.count().get(), { names: /Aggregation/i })
    },
    served: async ({ freshCollection }) => {
      const cities = freshCollection('aggregation')
      await Promise.all([cities.doc('a').set({ n: 1 }), cities.doc('b').set({ n: 2 })])

      const snapshot = await cities.count().get()
      assert.equal(snapshot.data().count, 2)
    },
  },

  {
    id: 'firestore.PartitionQuery',
    unimplemented: async ({ raw, projectId, expectUnimplemented }) => {
      await expectUnimplemented(
        () =>
          unary(raw.firestore, 'PartitionQuery', {
            parent: documentsRoot(projectId),
            structured_query: { from: [{ collection_id: 'anything', all_descendants: true }] },
            partition_count: '2',
          }),
        { names: /PartitionQuery/ },
      )
    },
    // Dormant for Fireup; see the note on `firestore.Write`. A collection this
    // small legitimately partitions into *nothing* — proto3 omits an empty
    // repeated field, so a bare `{}` is a valid answer meaning "read it in one
    // go". What this measures is whether the RPC is answered at all; what it
    // pins is that any partition that does come back is a cursor.
    served: async ({ raw, db, projectId, runId }) => {
      const collection = `conf_${runId}_partition`
      await db.doc(`${collection}/d1`).set({ n: 1 })

      const response = await unary(raw.firestore, 'PartitionQuery', {
        parent: `${documentsRoot(projectId)}/__partition__/__partition__`,
        structured_query: {
          from: [{ collection_id: collection, all_descendants: true }],
          order_by: [{ field: { field_path: '__name__' }, direction: 'ASCENDING' }],
        },
        partition_count: '2',
      })
      assert.ok(response, 'PartitionQuery must answer')
      for (const partition of response.partitions ?? []) {
        assert.ok(
          Array.isArray(partition.values),
          `a partition must be a cursor over values, got ${JSON.stringify(partition)}`,
        )
      }
    },
  },

  {
    id: 'firestore.ExecutePipeline',
    unimplemented: async ({ raw, projectId, expectUnimplemented }) => {
      await expectUnimplemented(
        () =>
          serverStream(raw.firestore, 'ExecutePipeline', {
            database: databaseName(projectId),
            structured_pipeline: { pipeline: { stages: [] } },
          }),
        { names: /ExecutePipeline/ },
      )
    },
  },

  // ---- transactions ---------------------------------------------------------

  {
    id: 'transaction.atomic-commit',
    served: async ({ db, freshCollection }) => {
      const cities = freshCollection('atomic')
      await cities.doc('exists').set({ n: 1 })

      const batch = db.batch()
      batch.set(cities.doc('would-land'), { n: 2 })
      batch.create(cities.doc('exists'), { n: 3 }) // fails: already exists
      await assert.rejects(batch.commit(), (error) => error.code === GRPC.ALREADY_EXISTS)

      assert.equal(
        (await cities.doc('would-land').get()).exists,
        false,
        'a Commit is atomic: no write lands when one precondition fails',
      )
      assert.equal((await cities.doc('exists').get()).get('n'), 1)
    },
  },

  {
    id: 'transaction.optimistic-concurrency',
    served: async ({ db, freshCollection }) => {
      const cities = freshCollection('occ')
      const ref = cities.doc('osaka')
      await ref.set({ n: 19 })

      // Writing from outside during the first attempt invalidates the read set,
      // so the commit must be ABORTED — which the SDK turns into a retry. If it
      // were not aborted, the second attempt would never happen and the
      // outside write would be silently lost.
      let attempts = 0
      await db.runTransaction(async (transaction) => {
        attempts += 1
        const snapshot = await transaction.get(ref)
        if (attempts === 1) {
          await ref.update({ n: 20 })
        }
        transaction.update(ref, { n: snapshot.get('n') + 1 })
      })

      assert.ok(attempts > 1, `the commit should have been aborted and retried, attempts=${attempts}`)
      assert.equal((await ref.get()).get('n'), 21, 'the retry must read the concurrent write')
    },
  },

  {
    id: 'transaction.absent-read-is-checked',
    served: async ({ raw, db, projectId, runId }) => {
      const path = `conf_${runId}_absentread/ghost`
      const name = documentName(projectId, path)
      const database = databaseName(projectId)

      const { transaction } = await unary(raw.firestore, 'BeginTransaction', {
        database,
        options: { read_write: {} },
      })

      // Read it as absent. BatchGetDocuments reports `missing` rather than
      // failing, which is how an absent read enters the read set.
      const results = await serverStream(raw.firestore, 'BatchGetDocuments', {
        database,
        documents: [name],
        transaction,
      })
      assert.equal(results.length, 1)
      assert.equal(results[0].missing, name, 'the document must be reported missing')

      // Someone else creates it. The commit has to notice.
      await db.doc(path).set({ n: 1 })

      const error = await expectCode(GRPC.ABORTED, () =>
        unary(raw.firestore, 'Commit', {
          database,
          transaction,
          writes: [{ update: { name, fields: { n: value.integer(2) } } }],
        }),
      )
      assert.ok(error.message.length > 0, 'the abort must say something')
      assert.equal((await db.doc(path).get()).get('n'), 1, 'the aborted write must not land')
    },
  },

  {
    id: 'transaction.read-only',
    served: async ({ raw, db, projectId, runId, expectUnimplemented }) => {
      const path = `conf_${runId}_readonlytx/d1`
      await db.doc(path).set({ n: 5 })
      const database = databaseName(projectId)

      const { transaction } = await unary(raw.firestore, 'BeginTransaction', {
        database,
        options: { read_only: {} },
      })
      assert.ok(transaction?.length > 0, 'a read-only transaction must return a token')

      const results = await serverStream(raw.firestore, 'BatchGetDocuments', {
        database,
        documents: [documentName(projectId, path)],
        transaction,
      })
      assert.equal(plainFields(results[0].found).n, 5)

      // Read-only *at a read_time* is the refused variant: there is only one
      // version of a document in the store, so answering it with current data
      // would be a wrong answer rather than a missing feature.
      await expectUnimplemented(() =>
        unary(raw.firestore, 'BeginTransaction', {
          database,
          options: { read_only: { read_time: { seconds: '1700000000', nanos: 0 } } },
        }),
      )
    },
  },

  // ---- other known limits ---------------------------------------------------

  {
    id: 'limits.read-time-consistency',
    unimplemented: async ({ raw, db, projectId, runId, expectUnimplemented }) => {
      const path = `conf_${runId}_readtime/d1`
      await db.doc(path).set({ n: 1 })

      await expectUnimplemented(() =>
        unary(raw.firestore, 'GetDocument', {
          name: documentName(projectId, path),
          read_time: { seconds: '1700000000', nanos: 0 },
        }),
      )
    },
  },

  {
    id: 'query.masked-field-deletion',
    served: async ({ freshCollection }) => {
      const docs = freshCollection('maskdelete')
      const ref = docs.doc('d1')
      await ref.set({ keep: 1, gone: 2 })

      // FieldValue.delete() reaches the wire as an update_mask path the write
      // carries no value for, which is what "deleted" means for a mask on both
      // surfaces. This was a documented divergence — the field was left in
      // place — until the shared store grew field deletion in SAK-203.
      await ref.update({ gone: FieldValue.delete() })

      const snapshot = await ref.get()
      assert.equal(snapshot.get('keep'), 1, 'an unmasked field is untouched')
      assert.equal(
        snapshot.get('gone'),
        undefined,
        'docs/local/README.md: a masked path the write does not carry deletes the field',
      )
      assert.deepEqual(Object.keys(snapshot.data()), ['keep'])
    },
  },
]
