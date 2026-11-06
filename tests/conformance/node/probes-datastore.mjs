/**
 * Probes for the `google.datastore.v1.Datastore` rows of the fidelity matrix,
 * and for the refusals the README names in prose right after that table
 * (namespaces, GQL, projection queries, cursors, nested property masks).
 *
 * Driven by `@google-cloud/datastore` — the official library — except where it
 * exposes no way to send the field under test, which is the case for
 * `ReserveIds`, GQL, cursors, and property masks.
 */

import { strict as assert } from 'node:assert'
import { createRequire } from 'node:module'

import { GRPC, unary } from './grpc-client.mjs'

const require = createRequire(import.meta.url)
const { PropertyFilter, and } = require('@google-cloud/datastore')

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

const names = (rows, datastore) => rows.map((row) => row[datastore.KEY].name)

/** Four tasks of one kind, the dataset the query probes read. */
const seedTasks = async (datastore, kind) => {
  await datastore.save([
    { key: datastore.key([kind, 'a']), data: { priority: 1, owner: 'ada', done: false } },
    { key: datastore.key([kind, 'b']), data: { priority: 2, owner: 'ada', done: false } },
    { key: datastore.key([kind, 'c']), data: { priority: 3, owner: 'bob', done: true } },
    { key: datastore.key([kind, 'd']), data: { priority: 4, owner: 'bob', done: false } },
  ])
  return kind
}

export const datastoreProbes = [
  {
    id: 'datastore.Lookup',
    served: async ({ datastore, raw, projectId, freshKind }) => {
      const kind = freshKind('lookup')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { title: 'look me up', priority: 1 } })

      const [entity] = await datastore.get(key)
      assert.ok(entity, 'Lookup must find a saved entity')
      assert.equal(entity.title, 'look me up')
      assert.equal(entity[datastore.KEY].name, 'one')

      const [missing] = await datastore.get(datastore.key([kind, 'nobody']))
      assert.equal(missing, undefined, 'a missing entity is reported as missing, not as an error')

      // A top-level property mask is served; the SDK has no option for it, so
      // it goes on the wire.
      const response = await unary(raw.datastore, 'Lookup', {
        project_id: projectId,
        keys: [
          {
            partition_id: { project_id: projectId },
            path: [{ kind, name: 'one' }],
          },
        ],
        property_mask: { paths: ['title'] },
      })
      const properties = response.found[0].entity.properties
      assert.deepEqual(
        Object.keys(properties),
        ['title'],
        'a top-level property mask must drop the other properties',
      )
    },
  },

  {
    id: 'datastore.RunQuery',
    served: async ({ datastore, freshKind }) => {
      const kind = await seedTasks(datastore, freshKind('runquery'))

      const run = async (query) => {
        const [rows] = await datastore.runQuery(query)
        return names(rows, datastore)
      }

      assert.deepEqual(
        await run(datastore.createQuery(kind).filter(new PropertyFilter('priority', '=', 2))),
        ['b'],
      )
      assert.deepEqual(
        await run(
          datastore.createQuery(kind).filter(new PropertyFilter('priority', '<', 3)).order('priority'),
        ),
        ['a', 'b'],
      )
      assert.deepEqual(
        await run(
          datastore
            .createQuery(kind)
            .filter(new PropertyFilter('priority', '<=', 2))
            .order('priority'),
        ),
        ['a', 'b'],
      )
      assert.deepEqual(
        await run(
          datastore.createQuery(kind).filter(new PropertyFilter('priority', '>', 3)).order('priority'),
        ),
        ['d'],
      )
      assert.deepEqual(
        await run(
          datastore
            .createQuery(kind)
            .filter(new PropertyFilter('priority', '>=', 3))
            .order('priority'),
        ),
        ['c', 'd'],
      )

      // AND over two properties.
      assert.deepEqual(
        await run(
          datastore
            .createQuery(kind)
            .filter(
              and([new PropertyFilter('owner', '=', 'bob'), new PropertyFilter('done', '=', false)]),
            ),
        ),
        ['d'],
      )

      // order (both directions), limit, offset.
      assert.deepEqual(await run(datastore.createQuery(kind).order('priority')), ['a', 'b', 'c', 'd'])
      assert.deepEqual(
        await run(datastore.createQuery(kind).order('priority', { descending: true })),
        ['d', 'c', 'b', 'a'],
      )
      assert.deepEqual(await run(datastore.createQuery(kind).order('priority').limit(2)), ['a', 'b'])
      assert.deepEqual(await run(datastore.createQuery(kind).order('priority').offset(2)), ['c', 'd'])

      // HAS_ANCESTOR: a child entity under a parent key.
      const parentKind = `${kind}_Parent`
      const parent = datastore.key([parentKind, 'p1'])
      await datastore.save({ key: parent, data: { label: 'parent' } })
      await datastore.save({
        key: datastore.key([parentKind, 'p1', kind, 'child']),
        data: { priority: 9 },
      })

      const descendants = await run(datastore.createQuery(kind).hasAncestor(parent))
      assert.deepEqual(descendants, ['child'], 'HAS_ANCESTOR scopes the kind to one parent')
    },
  },

  {
    id: 'datastore.Commit',
    served: async ({ datastore, freshKind }) => {
      const kind = freshKind('commit')

      const insertKey = datastore.key([kind, 'inserted'])
      await datastore.insert({ key: insertKey, data: { n: 1 } })
      await expectCode(GRPC.ALREADY_EXISTS, () =>
        datastore.insert({ key: insertKey, data: { n: 2 } }),
      )

      await datastore.update({ key: insertKey, data: { n: 3 } })
      assert.equal((await datastore.get(insertKey))[0].n, 3)
      await expectCode(GRPC.NOT_FOUND, () =>
        datastore.update({ key: datastore.key([kind, 'ghost']), data: { n: 1 } }),
      )

      // upsert writes whether or not the entity is there.
      const upsertKey = datastore.key([kind, 'upserted'])
      await datastore.upsert({ key: upsertKey, data: { n: 1 } })
      await datastore.upsert({ key: upsertKey, data: { n: 2 } })
      assert.equal((await datastore.get(upsertKey))[0].n, 2)

      await datastore.delete(insertKey)
      assert.equal((await datastore.get(insertKey))[0], undefined)

      // An incomplete key is completed by the commit with a numeric id.
      const incomplete = datastore.key([kind])
      await datastore.save({ key: incomplete, data: { n: 7 } })
      assert.ok(incomplete.id, 'the commit must report the allocated id back')
      assert.match(String(incomplete.id), /^\d+$/, 'allocated ids are numeric')
      assert.equal((await datastore.get(datastore.key([kind, datastore.int(incomplete.id)])))[0].n, 7)
    },
  },

  {
    id: 'datastore.BeginTransaction-Rollback',
    served: async ({ datastore, freshKind }) => {
      const kind = freshKind('tx')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { n: 1 } })

      // Commit path.
      const transaction = datastore.transaction()
      await transaction.run()
      const [entity] = await transaction.get(key)
      transaction.save({ key, data: { n: entity.n + 1 } })
      await transaction.commit()
      assert.equal((await datastore.get(key))[0].n, 2)

      // Rollback path: the write must not land.
      const abandoned = datastore.transaction()
      await abandoned.run()
      abandoned.save({ key, data: { n: 99 } })
      await abandoned.rollback()
      assert.equal((await datastore.get(key))[0].n, 2, 'a rolled-back write must not land')

      // Optimistic concurrency: a concurrent write to a read entity aborts.
      const contended = datastore.transaction()
      await contended.run()
      await contended.get(key)
      await datastore.save({ key, data: { n: 50 } })
      contended.save({ key, data: { n: 3 } })
      const error = await expectCode(GRPC.ABORTED, () => contended.commit())
      assert.ok(error.message, 'the abort must say something')
      assert.equal((await datastore.get(key))[0].n, 50, 'the aborted write must not land')
    },
  },

  {
    id: 'datastore.AllocateIds',
    served: async ({ datastore, freshKind }) => {
      const kind = freshKind('allocate')
      const [keys] = await datastore.allocateIds(datastore.key([kind]), 3)

      assert.equal(keys.length, 3)
      const allocated = keys.map((key) => String(key.id))
      for (const id of allocated) {
        assert.match(id, /^\d+$/, `expected a numeric id, got ${id}`)
      }
      assert.equal(new Set(allocated).size, 3, 'allocated ids must be distinct')

      // An allocated id is usable, and does not collide with a later save.
      await datastore.save({ key: keys[0], data: { n: 1 } })
      assert.equal((await datastore.get(keys[0]))[0].n, 1)
    },
  },

  {
    id: 'datastore.ReserveIds',
    served: async ({ datastore, raw, projectId, freshKind }) => {
      // @google-cloud/datastore exposes no reserveIds(), so this is the wire.
      const kind = freshKind('reserve')
      const key = datastore.key([kind, datastore.int(4242)])
      await datastore.save({ key, data: { n: 1 } })

      const response = await unary(raw.datastore, 'ReserveIds', {
        project_id: projectId,
        keys: [
          {
            partition_id: { project_id: projectId },
            path: [{ kind, id: '4242' }],
          },
        ],
      })
      assert.ok(response, 'ReserveIds is acknowledged')

      // The acknowledgement must not have disturbed the entity.
      assert.equal((await datastore.get(key))[0].n, 1)
    },
  },

  {
    id: 'datastore.RunAggregationQuery',
    unimplemented: async ({ datastore, freshKind, expectUnimplemented }) => {
      const kind = await seedTasks(datastore, freshKind('aggregation'))
      const aggregation = datastore.createAggregationQuery(datastore.createQuery(kind)).count('total')

      await expectUnimplemented(() => datastore.runAggregationQuery(aggregation), {
        names: /Aggregation/i,
      })
    },
    served: async ({ datastore, freshKind }) => {
      const kind = await seedTasks(datastore, freshKind('aggregation'))
      const aggregation = datastore.createAggregationQuery(datastore.createQuery(kind)).count('total')

      const [[result]] = await datastore.runAggregationQuery(aggregation)
      assert.equal(Number(result.total), 4)
    },
  },

  {
    id: 'datastore.namespaces',
    unimplemented: async ({ datastore, freshKind, expectUnimplemented }) => {
      const kind = freshKind('namespace')
      // Two namespaces can hold the same key path, so answering in the default
      // namespace would be a wrong answer rather than a missing feature.
      const key = datastore.key({ namespace: 'other', path: [kind, 'one'] })

      await expectUnimplemented(() => datastore.save({ key, data: { n: 1 } }), {
        names: /namespace/i,
      })
      await expectUnimplemented(() => datastore.get(key), { names: /namespace/i })
    },
  },

  {
    id: 'datastore.gql',
    unimplemented: async ({ raw, projectId, freshKind, expectUnimplemented }) => {
      const kind = freshKind('gql')
      await expectUnimplemented(
        () =>
          unary(raw.datastore, 'RunQuery', {
            project_id: projectId,
            partition_id: { project_id: projectId },
            gql_query: { query_string: `SELECT * FROM ${kind}`, allow_literals: true },
          }),
        { names: /GQL/ },
      )
    },
  },

  {
    id: 'datastore.projection-queries',
    unimplemented: async ({ datastore, freshKind, expectUnimplemented }) => {
      const kind = await seedTasks(datastore, freshKind('projection'))
      const query = datastore.createQuery(kind).select(['priority'])

      await expectUnimplemented(() => datastore.runQuery(query), { names: /projection/i })
    },
  },

  {
    id: 'datastore.cursors',
    unimplemented: async ({ raw, projectId, freshKind, expectUnimplemented }) => {
      const kind = freshKind('cursors')
      await expectUnimplemented(
        () =>
          unary(raw.datastore, 'RunQuery', {
            project_id: projectId,
            partition_id: { project_id: projectId },
            query: { kind: [{ name: kind }], start_cursor: Buffer.from('a-cursor') },
          }),
        { names: /cursor/i },
      )
    },
  },

  {
    id: 'datastore.property-transforms',
    unimplemented: async ({ datastore, raw, projectId, freshKind, expectUnimplemented }) => {
      // The Firestore spelling of a transform is served; the Datastore one is
      // not, and @google-cloud/datastore has no sentinel for it — so this goes
      // on the wire.
      const kind = freshKind('propertytransform')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { n: 1 } })

      await expectUnimplemented(
        () =>
          unary(raw.datastore, 'Commit', {
            project_id: projectId,
            mode: 'NON_TRANSACTIONAL',
            mutations: [
              {
                update: {
                  key: { partition_id: { project_id: projectId }, path: [{ kind, name: 'one' }] },
                  properties: {},
                },
                property_transforms: [{ property: 'n', increment: { integer_value: '1' } }],
              },
            ],
          }),
        { names: /property transform/i },
      )

      assert.equal((await datastore.get(key))[0].n, 1, 'a refused transform writes nothing')
    },
  },

  {
    id: 'datastore.nested-property-mask',
    unimplemented: async ({ datastore, raw, projectId, freshKind, expectUnimplemented }) => {
      const kind = freshKind('nestedmask')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { outer: { inner: 1 }, title: 'x' } })

      await expectUnimplemented(
        () =>
          unary(raw.datastore, 'Lookup', {
            project_id: projectId,
            keys: [{ partition_id: { project_id: projectId }, path: [{ kind, name: 'one' }] }],
            property_mask: { paths: ['outer.inner'] },
          }),
        { names: /nested property mask/i },
      )
    },
  },
]
