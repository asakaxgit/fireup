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

const entityKey = (projectId, kind, name) => ({
  partition_id: { project_id: projectId },
  path: [{ kind, name }],
})

/**
 * Upsert an entity with `properties` and apply `transforms` to it, on the wire.
 * Returns the `CommitResponse` so a probe can check
 * `mutation_results[].transform_results` as well as the stored entity.
 */
const commitWithTransforms = (raw, projectId, kind, name, transforms, properties = {}) =>
  unary(raw.datastore, 'Commit', {
    project_id: projectId,
    mode: 'NON_TRANSACTIONAL',
    mutations: [
      {
        upsert: { key: entityKey(projectId, kind, name), properties },
        property_transforms: transforms,
      },
    ],
  })

/**
 * Write `properties` restricted to `mask`, on the wire, optionally with
 * `transforms` after it.
 *
 * `@google-cloud/datastore` has no handwritten surface for `property_mask`, the
 * same reason `property_transforms` is driven through the generated client —
 * `save()` cannot express either — so every mask row is a raw `Commit`.
 */
const commitMasked = (raw, projectId, kind, name, mask, properties, { transforms = [], op = 'upsert' } = {}) =>
  unary(raw.datastore, 'Commit', {
    project_id: projectId,
    mode: 'NON_TRANSACTIONAL',
    mutations: [
      {
        [op]: { key: entityKey(projectId, kind, name), properties },
        property_mask: { paths: mask },
        property_transforms: transforms,
      },
    ],
  })

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
    id: 'datastore.transaction.read-only-commit',
    served: async ({ datastore, freshKind }) => {
      const kind = freshKind('rotx')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { n: 1 } })

      // Google's public read-only sample reads, then commits.
      const reader = datastore.transaction({ readOnly: true })
      await reader.run()
      const [entity] = await reader.get(key)
      await reader.commit()
      assert.equal(entity.n, 1)

      // A read-only transaction still cannot write.
      const writer = datastore.transaction({ readOnly: true })
      await writer.run()
      writer.save({ key, data: { n: 2 } })
      const error = await expectCode(GRPC.INVALID_ARGUMENT, () => writer.commit())
      assert.match(error.message, /read-only transactions cannot modify entities/)
      assert.equal((await datastore.get(key))[0].n, 1, 'the refused write must not land')
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
    // Dormant for Fireup; it exists so observe.mjs can measure GQL against an
    // implementation that documents it, rather than taking the README's word.
    served: async ({ datastore, raw, projectId, freshKind }) => {
      const kind = await seedTasks(datastore, freshKind('gql'))
      const response = await unary(raw.datastore, 'RunQuery', {
        project_id: projectId,
        partition_id: { project_id: projectId },
        gql_query: { query_string: `SELECT * FROM ${kind}`, allow_literals: true },
      })
      const names = (response.batch?.entity_results ?? [])
        .map((result) => result.entity?.key?.path?.at(-1)?.name)
        .sort()
      assert.deepEqual(names, ['a', 'b', 'c', 'd'], 'GQL must return the seeded entities')
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

  // ---- property transforms --------------------------------------------------
  //
  // @google-cloud/datastore has no handwritten surface for
  // `Mutation.property_transforms` — `save()` has no increment — so every probe
  // below drives the published proto directly. `mutationResults[].transform_results`
  // is read back as well as the stored entity: a client reads the new counter
  // off the commit rather than with a second round trip, so a correct value in
  // the store and a wrong one in the response is still a bug.

  {
    id: 'datastore.transform.server-value',
    served: async ({ datastore, raw, projectId, freshKind }) => {
      const kind = freshKind('dstransformtime')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { n: 1 } })

      const response = await commitWithTransforms(raw, projectId, kind, 'one', [
        { property: 'at', set_to_server_value: 'REQUEST_TIME' },
      ])
      const [result] = response.mutation_results
      assert.ok(
        result.transform_results?.[0]?.timestamp_value,
        `transform_results must carry the timestamp, got ${JSON.stringify(result.transform_results)}`,
      )
      assert.ok((await datastore.get(key))[0].at, 'the property must have been written')
    },
  },

  {
    id: 'datastore.transform.increment',
    served: async ({ datastore, raw, projectId, freshKind }) => {
      const kind = freshKind('dstransformincr')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { n: datastore.int(10) } })

      // The entity carries the current value, because an unmasked update
      // replaces the entity before the transform reads it — see
      // datastore.transform.runs-after-the-operation.
      const response = await commitWithTransforms(
        raw,
        projectId,
        kind,
        'one',
        [{ property: 'n', increment: { integer_value: '5' } }],
        { n: { integer_value: '10' } },
      )
      assert.equal(
        response.mutation_results[0].transform_results[0].integer_value,
        '15',
        'an integer increment of an integer property stays exact',
      )
      assert.equal(Number((await datastore.get(key))[0].n), 15)
    },
  },

  {
    id: 'datastore.transform.array-union',
    served: async ({ datastore, raw, projectId, freshKind }) => {
      const kind = freshKind('dstransformunion')
      const key = datastore.key([kind, 'one'])

      const response = await commitWithTransforms(
        raw,
        projectId,
        kind,
        'one',
        [
          {
            property: 'tags',
            append_missing_elements: {
              values: [{ string_value: 'a' }, { string_value: 'b' }, { string_value: 'b' }],
            },
          },
        ],
        { tags: { array_value: { values: [{ string_value: 'a' }] } } },
      )
      assert.ok(
        'null_value' in (response.mutation_results[0].transform_results[0] ?? {}),
        'the array transforms report null, as the contract specifies',
      )
      assert.deepEqual((await datastore.get(key))[0].tags, ['a', 'b'])
    },
  },

  {
    id: 'datastore.transform.array-remove',
    served: async ({ datastore, raw, projectId, freshKind }) => {
      const kind = freshKind('dstransformremove')
      const key = datastore.key([kind, 'one'])

      await commitWithTransforms(
        raw,
        projectId,
        kind,
        'one',
        [{ property: 'tags', remove_all_from_array: { values: [{ string_value: 'a' }] } }],
        {
          tags: {
            array_value: {
              values: [{ string_value: 'a' }, { string_value: 'b' }, { string_value: 'a' }],
            },
          },
        },
      )
      assert.deepEqual((await datastore.get(key))[0].tags, ['b'], 'every occurrence is removed')
    },
  },

  {
    id: 'datastore.transform.maximum-minimum',
    unimplemented: async ({ datastore, raw, projectId, freshKind, expectUnimplemented }) => {
      const kind = freshKind('dstransformmax')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { n: datastore.int(1) } })

      for (const transform of [
        { property: 'n', maximum: { integer_value: '5' } },
        { property: 'n', minimum: { integer_value: '5' } },
      ]) {
        await expectUnimplemented(
          () => commitWithTransforms(raw, projectId, kind, 'one', [transform]),
          { names: /field transform/i },
        )
      }
      assert.equal(Number((await datastore.get(key))[0].n), 1, 'a refused transform writes nothing')
    },
  },

  {
    id: 'datastore.transform.property-path',
    unimplemented: async ({ datastore, raw, projectId, freshKind, expectUnimplemented }) => {
      const kind = freshKind('dstransformpath')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { n: datastore.int(1) } })

      // A path names a property inside an entity value — a nested document
      // field — and Datastore escapes `.` and `\` in a property name, so a name
      // containing either is refused too. Incrementing the wrong property is
      // worse than saying no.
      for (const property of ['outer.inner', 'odd\\name']) {
        await expectUnimplemented(
          () =>
            commitWithTransforms(raw, projectId, kind, 'one', [
              { property, increment: { integer_value: '1' } },
            ]),
          { names: /property paths in transforms/i },
        )
      }
      assert.equal(Number((await datastore.get(key))[0].n), 1)
    },
  },

  {
    id: 'datastore.transform.on-delete',
    served: async ({ datastore, raw, projectId, freshKind }) => {
      const kind = freshKind('dstransformdelete')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { n: datastore.int(1) } })

      // The proto allows transforms only on insert/update/upsert.
      await expectCode(GRPC.INVALID_ARGUMENT, () =>
        unary(raw.datastore, 'Commit', {
          project_id: projectId,
          mode: 'NON_TRANSACTIONAL',
          mutations: [
            {
              delete: entityKey(projectId, kind, 'one'),
              property_transforms: [{ property: 'n', increment: { integer_value: '1' } }],
            },
          ],
        }),
      )
      assert.ok((await datastore.get(key))[0], 'the refused delete must not have applied')
    },
  },

  {
    id: 'datastore.transform.runs-after-the-operation',
    'documented-divergence': async ({ datastore, raw, projectId, freshKind }) => {
      const kind = freshKind('dstransformorder')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { n: datastore.int(10) } })

      // An *unmasked* update replaces the whole entity before the transform
      // reads it, so an increment accumulates only when the submitted entity
      // carries the current value. Faithful to the proto, surprising in
      // practice, so it is pinned here rather than left for somebody to
      // discover as a lost counter. The masked form, which does accumulate, is
      // `datastore.property-mask.transform-accumulates` below.
      const response = await commitWithTransforms(raw, projectId, kind, 'one', [
        { property: 'n', increment: { integer_value: '5' } },
      ])
      assert.equal(
        response.mutation_results[0].transform_results[0].integer_value,
        '5',
        'docs/local/README.md: a mutation that omits the property resets it to the operand',
      )
      assert.equal(Number((await datastore.get(key))[0].n), 5)
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

  {
    id: 'datastore.property-mask.writes-a-carried-path',
    served: async ({ datastore, raw, projectId, freshKind }) => {
      const kind = freshKind('maskwrite')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { title: 'first', author: 'ada' } })

      await commitMasked(raw, projectId, kind, 'one', ['title'], {
        title: { string_value: 'second' },
      })

      assert.equal((await datastore.get(key))[0].title, 'second')
    },
  },

  {
    id: 'datastore.property-mask.deletes-an-omitted-path',
    served: async ({ datastore, raw, projectId, freshKind }) => {
      const kind = freshKind('maskdelete')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { title: 'first', draft: 1 } })

      // The proto: "properties referenced in the mask but not in the entity are
      // deleted". The mask names `draft`; the entity does not carry it.
      await commitMasked(raw, projectId, kind, 'one', ['title', 'draft'], {
        title: { string_value: 'second' },
      })

      const stored = (await datastore.get(key))[0]
      assert.equal(stored.title, 'second')
      assert.equal(
        stored.draft,
        undefined,
        'a masked property the entity omits must be deleted, not left alone',
      )
    },
  },

  {
    id: 'datastore.property-mask.leaves-unnamed-properties',
    served: async ({ datastore, raw, projectId, freshKind }) => {
      const kind = freshKind('maskleave')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { title: 'first', author: 'ada' } })

      // The entity carries a different `author`, but the mask does not name it,
      // so the stored value stands: "others are left untouched".
      await commitMasked(raw, projectId, kind, 'one', ['title'], {
        title: { string_value: 'second' },
        author: { string_value: 'ignored' },
      })

      assert.equal((await datastore.get(key))[0].author, 'ada')
    },
  },

  {
    id: 'datastore.property-mask.key-is-accepted-and-ignored',
    served: async ({ datastore, raw, projectId, freshKind }) => {
      const kind = freshKind('maskkey')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { title: 'first' } })

      // "None of the properties in the mask may have a reserved name, except
      // for `__key__`." It names no field — the key travels in the entity's
      // key — so a mask of only `__key__` writes no properties.
      await commitMasked(raw, projectId, kind, 'one', ['__key__'], {
        title: { string_value: 'ignored' },
      })

      const stored = (await datastore.get(key))[0]
      assert.equal(stored.title, 'first', 'a __key__-only mask writes no properties')
      assert.ok(stored[datastore.KEY], 'the entity still has its key')
    },
  },

  {
    id: 'datastore.property-mask.nested-path',
    unimplemented: async ({ datastore, raw, projectId, freshKind, expectUnimplemented }) => {
      const kind = freshKind('maskmutnested')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { title: 'first' } })

      await expectUnimplemented(
        () =>
          commitMasked(raw, projectId, kind, 'one', ['author.name'], {
            title: { string_value: 'second' },
          }),
        { names: /property_mask/i },
      )
      assert.equal(
        (await datastore.get(key))[0].title,
        'first',
        'the refusal must happen before the store is touched',
      )
    },
  },

  {
    id: 'datastore.property-mask.empty-path',
    served: async ({ datastore, raw, projectId, freshKind }) => {
      const kind = freshKind('maskempty')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { title: 'first' } })

      // "A path must not be empty."
      await expectCode(3, () =>
        commitMasked(raw, projectId, kind, 'one', [''], { title: { string_value: 'second' } }),
      )
      assert.equal((await datastore.get(key))[0].title, 'first')
    },
  },

  {
    id: 'datastore.property-mask.reserved-path',
    served: async ({ datastore, raw, projectId, freshKind }) => {
      const kind = freshKind('maskreserved')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { title: 'first' } })

      await expectCode(3, () =>
        commitMasked(raw, projectId, kind, 'one', ['title', '__scatter__'], {
          title: { string_value: 'second' },
        }),
      )
      assert.equal((await datastore.get(key))[0].title, 'first')
    },
  },

  {
    id: 'datastore.property-mask.ignored-on-delete',
    served: async ({ datastore, raw, projectId, freshKind }) => {
      const kind = freshKind('maskondelete')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { title: 'first', author: 'ada' } })

      // The proto: "this field is ignored for `delete`".
      await unary(raw.datastore, 'Commit', {
        project_id: projectId,
        mode: 'NON_TRANSACTIONAL',
        mutations: [
          { delete: entityKey(projectId, kind, 'one'), property_mask: { paths: ['title'] } },
        ],
      })

      assert.equal(
        (await datastore.get(key))[0],
        undefined,
        'the whole entity goes, not just the masked property',
      )
    },
  },

  {
    id: 'datastore.property-mask.transform-accumulates',
    served: async ({ datastore, raw, projectId, freshKind }) => {
      const kind = freshKind('masktransform')
      const key = datastore.key([kind, 'one'])
      await datastore.save({ key, data: { title: 'first', n: datastore.int(0) } })

      // The whole point of the pairing: the mask keeps the write off `n`, so the
      // increment reads the stored value and the counter climbs across commits
      // without the client ever sending it. Contrast
      // `datastore.transform.runs-after-the-operation`, where the unmasked form
      // resets it each time.
      for (const [round, want] of [['second', '1'], ['third', '2']]) {
        const response = await commitMasked(
          raw,
          projectId,
          kind,
          'one',
          ['title'],
          { title: { string_value: round } },
          { transforms: [{ property: 'n', increment: { integer_value: '1' } }] },
        )
        assert.equal(
          response.mutation_results[0].transform_results[0].integer_value,
          want,
          'a masked transform must accumulate on the stored value',
        )
      }

      const stored = (await datastore.get(key))[0]
      assert.equal(Number(stored.n), 2)
      assert.equal(stored.title, 'third')
    },
  },
]
