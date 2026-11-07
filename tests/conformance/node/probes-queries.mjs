/**
 * Probes for the query rows of the fidelity matrix, plus the "query semantics
 * worth knowing" claims — the places where a query engine can look like it
 * works and quietly be wrong. Those are the assertions that matter most here:
 * a missing feature announces itself, a wrong answer does not.
 */

import { strict as assert } from 'node:assert'
import { createRequire } from 'node:module'

import {
  GRPC,
  documentsRoot,
  expectCode,
  serverStream,
  unary,
  value,
} from './grpc-client.mjs'

const require = createRequire(import.meta.url)
const { AggregateField, FieldPath, Filter } = require('@google-cloud/firestore')

const ids = (snapshot) => snapshot.docs.map((doc) => doc.id)

/**
 * The ids a query matched, as a set rather than a sequence.
 *
 * For the probes whose claim is *which* documents come back and not in what
 * order. They used to pin the order with `orderBy('__name__')`, which is a
 * shape production refuses as soon as the query also carries an inequality:
 * the SDK appends the implicit order on the inequality field *after* the
 * explicit key. See `semantics.nothing-follows-the-key-in-an-order`.
 */
const idSet = (snapshot) => ids(snapshot).sort()

/** The same four cities every shape-level query probe reads. */
const seedCities = async (cities) => {
  await Promise.all([
    cities.doc('tokyo').set({ country: 'JP', population: 37, tags: ['capital', 'big'] }),
    cities.doc('osaka').set({ country: 'JP', population: 19, tags: ['big'] }),
    cities.doc('kyoto').set({ country: 'JP', population: 15, tags: ['old'] }),
    cities.doc('seoul').set({ country: 'KR', population: 25, tags: ['capital'] }),
  ])
  return cities
}

export const queryProbes = [
  {
    id: 'query.comparison-operators',
    served: async ({ freshCollection }) => {
      const cities = await seedCities(freshCollection('cmp'))

      assert.deepEqual(ids(await cities.where('country', '==', 'JP').orderBy('__name__').get()), [
        'kyoto',
        'osaka',
        'tokyo',
      ])
      assert.deepEqual(ids(await cities.where('country', '!=', 'JP').get()), ['seoul'])
      assert.deepEqual(ids(await cities.where('population', '<', 19).get()), ['kyoto'])
      assert.deepEqual(
        ids(await cities.where('population', '<=', 19).orderBy('population').get()),
        ['kyoto', 'osaka'],
      )
      assert.deepEqual(ids(await cities.where('population', '>', 25).get()), ['tokyo'])
      assert.deepEqual(
        ids(await cities.where('population', '>=', 25).orderBy('population').get()),
        ['seoul', 'tokyo'],
      )
    },
  },

  {
    id: 'query.membership-and-array-operators',
    served: async ({ freshCollection }) => {
      const cities = await seedCities(freshCollection('membership'))

      assert.deepEqual(ids(await cities.where('country', 'in', ['KR', 'TW']).get()), ['seoul'])
      assert.deepEqual(ids(await cities.where('country', 'not-in', ['JP']).get()), ['seoul'])
      assert.deepEqual(
        ids(await cities.where('tags', 'array-contains', 'capital').orderBy('__name__').get()),
        ['seoul', 'tokyo'],
      )
      assert.deepEqual(
        ids(
          await cities
            .where('tags', 'array-contains-any', ['old', 'capital'])
            .orderBy('__name__')
            .get(),
        ),
        ['kyoto', 'seoul', 'tokyo'],
      )
    },
  },

  {
    id: 'query.unary-filters',
    served: async ({ freshCollection }) => {
      const values = freshCollection('unary')
      await Promise.all([
        values.doc('a_null').set({ v: null }),
        values.doc('b_one').set({ v: 1 }),
        values.doc('c_nan').set({ v: Number.NaN }),
        values.doc('d_absent').set({ other: true }),
      ])

      // The SDK turns `== null` / `!= null` / `== NaN` into the unary filters.
      assert.deepEqual(ids(await values.where('v', '==', null).get()), ['a_null'], 'IS_NULL')
      assert.deepEqual(
        idSet(await values.where('v', '!=', null).get()),
        ['b_one', 'c_nan'],
        'IS_NOT_NULL matches any present non-null value, and never a missing field',
      )
      assert.deepEqual(ids(await values.where('v', '==', Number.NaN).get()), ['c_nan'], 'IS_NAN')

      const notNan = ids(await values.where('v', '!=', Number.NaN).get())
      assert.ok(notNan.includes('b_one'), `IS_NOT_NAN must match a number: ${notNan}`)
      assert.ok(!notNan.includes('c_nan'), `IS_NOT_NAN must not match NaN: ${notNan}`)
      assert.ok(!notNan.includes('d_absent'), `a missing field never matches: ${notNan}`)
    },
  },

  {
    id: 'query.composite-filters',
    served: async ({ freshCollection }) => {
      const cities = await seedCities(freshCollection('composite'))

      const and = await cities
        .where(Filter.and(Filter.where('country', '==', 'JP'), Filter.where('population', '>', 18)))
        .orderBy('population', 'desc')
        .get()
      assert.deepEqual(ids(and), ['tokyo', 'osaka'])

      // No explicit order: both branches carry an inequality on `population`,
      // so the SDK orders by it, and asking for `__name__` on top would build
      // the order shape production refuses.
      const or = await cities
        .where(Filter.or(Filter.where('country', '==', 'KR'), Filter.where('population', '<', 16)))
        .get()
      assert.deepEqual(idSet(or), ['kyoto', 'seoul'])

      // Nested: (JP AND >30) OR (KR)
      const nested = await cities
        .where(
          Filter.or(
            Filter.and(Filter.where('country', '==', 'JP'), Filter.where('population', '>', 30)),
            Filter.where('country', '==', 'KR'),
          ),
        )
        .get()
      assert.deepEqual(idSet(nested), ['seoul', 'tokyo'])
    },
  },

  {
    id: 'query.order-by',
    served: async ({ freshCollection }) => {
      const cities = await seedCities(freshCollection('orderby'))

      assert.deepEqual(ids(await cities.orderBy('population').get()), [
        'kyoto',
        'osaka',
        'seoul',
        'tokyo',
      ])
      assert.deepEqual(ids(await cities.orderBy('population', 'desc').get()), [
        'tokyo',
        'seoul',
        'osaka',
        'kyoto',
      ])
      // Several terms: country ascending, then population descending inside it.
      assert.deepEqual(ids(await cities.orderBy('country').orderBy('population', 'desc').get()), [
        'tokyo',
        'osaka',
        'kyoto',
        'seoul',
      ])
    },
  },

  {
    id: 'query.limit-offset',
    served: async ({ freshCollection }) => {
      const cities = await seedCities(freshCollection('limitoffset'))

      assert.deepEqual(ids(await cities.orderBy('population').limit(2).get()), ['kyoto', 'osaka'])
      assert.deepEqual(ids(await cities.orderBy('population').offset(2).get()), ['seoul', 'tokyo'])
      assert.deepEqual(ids(await cities.orderBy('population').offset(1).limit(2).get()), [
        'osaka',
        'seoul',
      ])
      assert.equal((await cities.orderBy('population').offset(99).get()).empty, true)
    },
  },

  {
    id: 'query.projection',
    served: async ({ freshCollection }) => {
      const cities = await seedCities(freshCollection('projection'))

      const projected = await cities.select('country').where('country', '==', 'KR').get()
      assert.deepEqual(projected.docs[0].data(), { country: 'KR' })
      assert.equal(projected.docs[0].id, 'seoul')

      // select() with no field is the keys-only query: the SDK sends __name__.
      const keysOnly = await cities.select().orderBy('__name__').get()
      assert.deepEqual(ids(keysOnly), ['kyoto', 'osaka', 'seoul', 'tokyo'])
      assert.deepEqual(keysOnly.docs[0].data(), {}, 'a keys-only result carries no fields')
    },
  },

  {
    id: 'query.limit-to-last',
    served: async ({ freshCollection }) => {
      const cities = await seedCities(freshCollection('limittolast'))

      // The SDK flips the order on the wire and reverses client-side, so this
      // pins the SDK's own round trip rather than a server feature.
      assert.deepEqual(ids(await cities.orderBy('population').limitToLast(2).get()), [
        'seoul',
        'tokyo',
      ])
    },
  },

  {
    id: 'query.document-id-filter-and-order',
    served: async ({ freshCollection }) => {
      const cities = await seedCities(freshCollection('docid'))

      const filtered = await cities.where(FieldPath.documentId(), '>=', 'osaka').get()
      assert.deepEqual(ids(filtered), ['osaka', 'seoul', 'tokyo'])

      assert.deepEqual(ids(await cities.orderBy(FieldPath.documentId()).get()), [
        'kyoto',
        'osaka',
        'seoul',
        'tokyo',
      ])

      // Inverted by SAK-223. This asserted descending name order until SAK-217
      // pointed the probes at Google's emulator and found that production
      // refuses a key scan it cannot read forwards. The matrix row claimed
      // parity, the probe was green, and the claim was false: the refusal is
      // the parity.
      const refusal = await expectCode(GRPC.FAILED_PRECONDITION, () =>
        cities.orderBy(FieldPath.documentId(), 'desc').get(),
      )
      assert.match(
        refusal.message,
        /descending key scans/,
        `the refusal must name what is unsupported, got: ${refusal.message}`,
      )

      // The descending key is only refused as the *whole* order. As a
      // tie-breaker after an explicit term it is served, which is what every
      // `orderBy(field, 'desc')` compiles to.
      assert.deepEqual(
        ids(await cities.orderBy('country', 'desc').orderBy(FieldPath.documentId(), 'desc').get()),
        ['seoul', 'tokyo', 'osaka', 'kyoto'],
      )
    },
  },

  {
    id: 'query.collection-group',
    unimplemented: async ({ db, freshCollection, expectUnimplemented, runId }) => {
      const cities = freshCollection('collectiongroup')
      await cities.doc('tokyo').collection(`kids_${runId}`).doc('k').set({ n: 1 })

      await expectUnimplemented(() => db.collectionGroup(`kids_${runId}`).get(), {
        names: /collection group/i,
      })
    },
    served: async ({ db, freshCollection, runId }) => {
      const cities = freshCollection('collectiongroup')
      await cities.doc('tokyo').collection(`kids_${runId}`).doc('k').set({ n: 1 })

      const snapshot = await db.collectionGroup(`kids_${runId}`).get()
      assert.deepEqual(ids(snapshot), ['k'])
    },
  },

  {
    id: 'query.cursors',
    unimplemented: async ({ freshCollection, expectUnimplemented }) => {
      const cities = await seedCities(freshCollection('cursors'))

      await expectUnimplemented(() => cities.orderBy('population').startAt(19).get(), {
        names: /cursor/i,
      })
      await expectUnimplemented(() => cities.orderBy('population').endBefore(25).get(), {
        names: /cursor/i,
      })
    },
    served: async ({ freshCollection }) => {
      const cities = await seedCities(freshCollection('cursors'))

      assert.deepEqual(ids(await cities.orderBy('population').startAt(19).get()), [
        'osaka',
        'seoul',
        'tokyo',
      ])
      assert.deepEqual(ids(await cities.orderBy('population').endBefore(25).get()), [
        'kyoto',
        'osaka',
      ])
    },
  },

  {
    id: 'query.nested-field-paths',
    served: async ({ db, freshCollection }) => {
      const docs = freshCollection('nestedpaths')
      await Promise.all([
        docs.doc('kyoto').set({ address: { city: 'Kyoto', zip: '600' }, rank: 1 }),
        docs.doc('osaka').set({ address: { city: 'Osaka', zip: '530' }, rank: 2 }),
      ])

      // Filters, orders, projections, and read masks.
      assert.deepEqual(ids(await docs.where('address.city', '==', 'Osaka').get()), ['osaka'])
      assert.deepEqual(ids(await docs.orderBy('address.city', 'desc').get()), ['osaka', 'kyoto'])

      const projected = await docs.select('address.city').orderBy('__name__').get()
      assert.deepEqual(projected.docs[0].data(), { address: { city: 'Kyoto' } })

      const [masked] = await db.getAll(docs.doc('osaka'), { fieldMask: ['address.zip'] })
      assert.deepEqual(masked.data(), { address: { zip: '530' } })
    },
    unimplemented: async ({ freshCollection, expectUnimplemented }) => {
      const docs = freshCollection('nestedpaths')
      await docs.doc('d1').set({ a: { b: 1 }, keep: true })

      await expectUnimplemented(() => docs.where('a.b', '==', 1).get(), {
        names: /nested field path/i,
      })
      await expectUnimplemented(() => docs.orderBy('a.b').get(), { names: /nested field path/i })
      await expectUnimplemented(() => docs.select('a.b').get(), { names: /nested field path/i })

      assert.deepEqual(
        (await docs.doc('d1').get()).data(),
        { a: { b: 1 }, keep: true },
        'a refused nested-path read must leave the document untouched',
      )
    },
  },

  {
    id: 'query.nested-update-mask',
    unimplemented: async ({ freshCollection, expectUnimplemented }) => {
      const docs = freshCollection('nestedmask')
      const ref = docs.doc('d1')
      await ref.set({ a: { b: 1, c: 2 }, keep: true })

      // Reads support nested paths; a *write* mask does not, because the store
      // writes whole top-level fields.
      await expectUnimplemented(() => ref.update({ 'a.b': 9 }), {
        names: /nested update_mask path/i,
      })
      assert.deepEqual(
        (await ref.get()).data(),
        { a: { b: 1, c: 2 }, keep: true },
        'a refused masked write must leave the document untouched',
      )
    },
  },

  {
    id: 'query.collection-group-without-id',
    unimplemented: async ({ raw, projectId, expectUnimplemented }) => {
      // The SDK will not build this: collectionGroup() requires an id. On the
      // wire it is the kindless scan of everything under the parent, which the
      // API allows and this server refuses rather than scanning the world.
      await expectUnimplemented(
        () =>
          serverStream(raw.firestore, 'RunQuery', {
            parent: documentsRoot(projectId),
            structured_query: { from: [{ collection_id: '', all_descendants: true }] },
          }),
        { names: /collection group query with no collection id/i },
      )
    },
  },

  {
    id: 'query.vector-search',
    unimplemented: async ({ raw, projectId, runId, expectUnimplemented }) => {
      // findNearest() is not in every @google-cloud/firestore minor version, so
      // the clause goes on the wire directly — it is the published proto field
      // either way.
      await expectUnimplemented(
        () =>
          serverStream(raw.firestore, 'RunQuery', {
            parent: documentsRoot(projectId),
            structured_query: {
              from: [{ collection_id: `conf_${runId}_vector` }],
              find_nearest: {
                vector_field: { field_path: 'embedding' },
                query_vector: { array_value: { values: [value.double(1), value.double(0)] } },
                distance_measure: 'EUCLIDEAN',
                limit: { value: 1 },
              },
            },
          }),
        { names: /find_nearest/ },
      )
    },
  },

  {
    id: 'query.aggregations',
    unimplemented: async ({ freshCollection, expectUnimplemented }) => {
      const cities = await seedCities(freshCollection('aggregate'))

      await expectUnimplemented(
        () => cities.aggregate({ total: AggregateField.sum('population') }).get(),
        { names: /Aggregation/i },
      )
      await expectUnimplemented(
        () => cities.aggregate({ mean: AggregateField.average('population') }).get(),
        { names: /Aggregation/i },
      )
    },
    served: async ({ freshCollection }) => {
      const cities = await seedCities(freshCollection('aggregate'))

      const snapshot = await cities
        .aggregate({ total: AggregateField.sum('population'), count: AggregateField.count() })
        .get()
      assert.equal(snapshot.data().total, 96)
      assert.equal(snapshot.data().count, 4)
    },
  },

  // ---- query semantics worth knowing ---------------------------------------

  {
    id: 'semantics.missing-field-never-matches',
    served: async ({ freshCollection }) => {
      const docs = freshCollection('missingfield')
      await Promise.all([
        docs.doc('has').set({ v: 1 }),
        docs.doc('absent').set({ other: 'x' }),
      ])

      assert.deepEqual(ids(await docs.where('v', '>', 0).get()), ['has'])
      assert.deepEqual(
        ids(await docs.where('v', '!=', 99).get()),
        ['has'],
        'not-equal is still an index lookup: no entry, no match',
      )
      assert.deepEqual(ids(await docs.where('v', 'not-in', [99]).get()), ['has'])
    },
  },

  {
    id: 'semantics.missing-order-field-absent',
    served: async ({ freshCollection }) => {
      const docs = freshCollection('missingorder')
      await Promise.all([
        docs.doc('has').set({ v: 1 }),
        docs.doc('absent').set({ other: 'x' }),
      ])

      assert.deepEqual(
        ids(await docs.orderBy('v').get()),
        ['has'],
        'a document without the ordered field is absent, not sorted to one end',
      )
      assert.deepEqual(ids(await docs.orderBy('v', 'desc').get()), ['has'])
      // It is in the collection, though: the ordering is what excludes it.
      assert.equal((await docs.get()).size, 2)
    },
  },

  {
    id: 'semantics.inequality-does-not-cross-types',
    served: async ({ freshCollection }) => {
      const people = freshCollection('crosstype')
      await Promise.all([
        people.doc('numeric').set({ age: 7 }),
        people.doc('stringy').set({ age: 'old' }),
      ])

      assert.deepEqual(
        ids(await people.where('age', '>', 5).get()),
        ['numeric'],
        'a string must not satisfy a numeric inequality even though strings sort after numbers',
      )
      assert.deepEqual(ids(await people.where('age', '>', 'a').get()), ['stringy'])
    },
  },

  {
    id: 'semantics.nan-equality-never-matches',
    served: async ({ raw, projectId, runId, freshCollection }) => {
      const label = 'nan'
      const values = freshCollection(label)
      await Promise.all([
        values.doc('nan').set({ v: Number.NaN }),
        values.doc('one').set({ v: 1 }),
        values.doc('two').set({ v: 2 }),
      ])

      // A literal EQUAL against NaN can only be sent on the wire: the SDK
      // rewrites `== NaN` into the IS_NAN unary filter.
      const matched = await serverStream(raw.firestore, 'RunQuery', {
        parent: documentsRoot(projectId),
        structured_query: {
          from: [{ collection_id: values.id }],
          where: {
            field_filter: {
              field: { field_path: 'v' },
              op: 'EQUAL',
              value: { double_value: Number.NaN },
            },
          },
        },
      })
      const names = matched.filter((message) => message.document).map((m) => m.document.name)
      assert.deepEqual(names, [], `== NaN must never match, got ${names}`)

      // IS_NAN is the only way to select it, and NaN sorts before every number.
      assert.deepEqual(ids(await values.where('v', '==', Number.NaN).get()), ['nan'])
      assert.deepEqual(ids(await values.orderBy('v').get()), ['nan', 'one', 'two'])
    },
  },

  {
    id: 'semantics.integer-double-compare-numerically',
    served: async ({ raw, projectId, freshCollection }) => {
      // The JS SDK writes 31 and 31.0 identically, so the two value types have
      // to be written on the wire to tell them apart at all.
      const docs = freshCollection('intdouble')
      const parent = documentsRoot(projectId)
      await unary(raw.firestore, 'CreateDocument', {
        parent,
        collection_id: docs.id,
        document_id: 'as_integer',
        document: { fields: { n: value.integer(31) } },
      })
      await unary(raw.firestore, 'CreateDocument', {
        parent,
        collection_id: docs.id,
        document_id: 'as_double',
        document: { fields: { n: value.double(31) } },
      })

      assert.deepEqual(
        ids(await docs.where('n', '==', 31).orderBy('__name__').get()),
        ['as_double', 'as_integer'],
        '31 and 31.0 are one value',
      )
      // An inequality, so no explicit `__name__`: the SDK orders by `n`, and
      // the two documents tie there, which the appended key breaks.
      assert.deepEqual(ids(await docs.where('n', '>', 30.5).get()), ['as_double', 'as_integer'])
    },
  },

  {
    id: 'semantics.cursor-is-a-position',
    served: async ({ freshCollection }) => {
      const docs = freshCollection('cursorposition')
      await Promise.all([
        docs.doc('a').set({ v: 31 }),
        docs.doc('b').set({ v: 31 }),
        docs.doc('c').set({ v: 32 }),
      ])

      // A cursor is a position, not a filter: startAfter(31) skips *every*
      // document whose first sort value is 31, not just one of them.
      assert.deepEqual(ids(await docs.orderBy('v').startAfter(31).get()), ['c'])
      assert.deepEqual(ids(await docs.orderBy('v').startAt(31).get()), ['a', 'b', 'c'])

      // A whole snapshot sends one value per term plus the document name,
      // which is what makes paging not repeat or drop a tie.
      const first = await docs.orderBy('v').limit(1).get()
      const next = await docs.orderBy('v').startAfter(first.docs[0]).get()
      assert.deepEqual(ids(next), ['b', 'c'], 'snapshot paging must not skip the tie')
    },
  },

  {
    id: 'semantics.cursors-before-offset-and-limit',
    served: async ({ freshCollection }) => {
      const cities = await seedCities(freshCollection('cursorcompose'))

      // Cursors choose the range; offset and limit page within it.
      const ranged = cities.orderBy('population').startAt(19).endAt(37)
      assert.deepEqual(ids(await ranged.get()), ['osaka', 'seoul', 'tokyo'])
      assert.deepEqual(ids(await ranged.offset(1).get()), ['seoul', 'tokyo'])
      assert.deepEqual(ids(await ranged.offset(1).limit(1).get()), ['seoul'])
    },
  },

  {
    id: 'semantics.dotted-field-name-is-escaped',
    served: async ({ raw, projectId, freshCollection }) => {
      const docs = freshCollection('dottedname')

      // `a.b` means "the b inside a". A field literally named "a.b" is a
      // different thing, and the two must never match each other's documents.
      await docs.doc('nested').set({ a: { b: 1 } })
      // A literal dotted field name has to be written on the wire: an
      // update_mask carrying one is refused, and set() is not a way to be
      // unambiguous about it.
      await unary(raw.firestore, 'CreateDocument', {
        parent: documentsRoot(projectId),
        collection_id: docs.id,
        document_id: 'literal',
        document: { fields: { 'a.b': value.integer(1) } },
      })

      assert.deepEqual(ids(await docs.where('a.b', '==', 1).get()), ['nested'])
      assert.deepEqual(ids(await docs.where(new FieldPath('a.b'), '==', 1).get()), ['literal'])
    },
  },

  {
    id: 'semantics.projection-keeps-nesting',
    served: async ({ freshCollection }) => {
      const docs = freshCollection('projectnesting')
      await docs.doc('d1').set({ address: { city: 'Kyoto', zip: '600' }, rank: 1 })

      const snapshot = await docs.select('address.city').get()
      assert.deepEqual(
        snapshot.docs[0].data(),
        { address: { city: 'Kyoto' } },
        'the projection keeps the nesting: not a flattened key, and not all of address',
      )
    },
  },

  {
    id: 'semantics.collection-group-scoped-to-parent',
    served: async ({ db, raw, projectId, freshCollection, runId }) => {
      const group = `scoped_${runId}`
      const owners = freshCollection('cgscope')
      await owners.doc('one').collection(group).doc('a').set({ n: 1 })
      await owners.doc('two').collection(group).doc('b').set({ n: 2 })
      // Deeper, to prove "any depth" below the root.
      await owners.doc('one').collection('mid').doc('m').collection(group).doc('c').set({ n: 3 })

      const fromRoot = await db.collectionGroup(group).get()
      assert.deepEqual(ids(fromRoot).sort(), ['a', 'b', 'c'], 'from the root: every depth')

      // Below a document it spans only that document's descendants. The SDK
      // has no API for a document-scoped collection group query, so this is
      // the same query with the parent moved, on the wire.
      const scoped = await serverStream(raw.firestore, 'RunQuery', {
        parent: `${documentsRoot(projectId)}/${owners.id}/one`,
        structured_query: { from: [{ collection_id: group, all_descendants: true }] },
      })
      const scopedIds = scoped
        .filter((message) => message.document)
        .map((message) => message.document.name.split('/').pop())
      assert.deepEqual(scopedIds.sort(), ['a', 'c'], "below a document: only that document's")
    },
  },

  {
    id: 'semantics.name-appended-to-order',
    served: async ({ freshCollection }) => {
      const docs = freshCollection('nameorder')
      await Promise.all([
        docs.doc('a').set({ v: 1 }),
        docs.doc('b').set({ v: 1 }),
        docs.doc('c').set({ v: 1 }),
      ])

      // Every value ties, so the result order is entirely __name__ — appended in
      // the direction of the last explicit term.
      assert.deepEqual(ids(await docs.orderBy('v').get()), ['a', 'b', 'c'])
      assert.deepEqual(ids(await docs.orderBy('v', 'desc').get()), ['c', 'b', 'a'])
    },
  },

  {
    id: 'semantics.nothing-follows-the-key-in-an-order',
    served: async ({ raw, projectId, freshCollection }) => {
      const docs = freshCollection('nameterminal')
      await Promise.all([docs.doc('a').set({ v: 1 }), docs.doc('b').set({ v: 2 })])

      // `__name__` is the tie-breaker that makes an order total, so a term
      // after it can never change a result — and Firestore refuses the shape
      // rather than ignoring the dead term.
      //
      // Driven through the SDK because that is the whole reason this row
      // exists: the SDK builds the shape *unprompted*, appending the implicit
      // order on an inequality field after the explicit key, so the wire
      // carries a legal-looking `[__name__]` and the query is illegal anyway.
      // Serving it locally means a query a user writes on the first day works
      // here and dies in production.
      const refusal = await expectCode(GRPC.INVALID_ARGUMENT, () =>
        docs.where('v', '!=', null).orderBy('__name__').get(),
      )
      assert.match(
        refusal.message,
        /after the key/,
        `the refusal must name the key, got: ${refusal.message}`,
      )

      // The rest at the wire level, and not only for coverage: the SDK retries
      // a refused query with backoff, so each refusal driven through it costs
      // about seven seconds against *any* host. Three of them overran this
      // suite's 20s probe timeout.
      const asc = (field) => ({ field: { field_path: field }, direction: 'ASCENDING' })
      const run = (order_by) =>
        serverStream(raw.firestore, 'RunQuery', {
          parent: documentsRoot(projectId),
          structured_query: { from: [{ collection_id: docs.id }], order_by },
        })

      for (const order_by of [
        [asc('__name__'), asc('v')],
        [asc('__name__'), asc('__name__')],
        [asc('v'), asc('__name__'), asc('w')],
      ]) {
        const error = await expectCode(GRPC.INVALID_ARGUMENT, () => run(order_by))
        assert.match(
          error.message,
          /after the key/,
          `the refusal must name the key, got: ${error.message}`,
        )
      }

      // The neighbours it must *not* refuse. A trailing key is the normal
      // shape, and the same query without the explicit key is served — the SDK
      // appends the key itself, so the refusal above costs the caller nothing.
      assert.deepEqual(
        (await run([asc('v'), asc('__name__')])).filter((m) => m.document).length,
        2,
        'a key as the last term is the ordinary shape',
      )
      assert.deepEqual(idSet(await docs.where('v', '!=', null).get()), ['a', 'b'])
    },
  },
]
