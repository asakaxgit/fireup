/**
 * Probes for the Field transforms table and the semantics the README spells out
 * under it.
 *
 * A transform is computed server-side, which makes it the one write path where
 * the server can produce a *plausible wrong number* rather than an error — the
 * exact failure mode the fidelity matrix is written to prevent. So these probes
 * pin the arithmetic rules, not just that the call succeeds.
 */

import { strict as assert } from 'node:assert'
import { createRequire } from 'node:module'

import { databaseName, documentName, unary, value } from './grpc-client.mjs'

const require = createRequire(import.meta.url)
const { FieldValue } = require('@google-cloud/firestore')

export const transformProbes = [
  {
    id: 'transform.server-timestamp',
    served: async ({ freshCollection }) => {
      const ref = freshCollection('servertimestamp').doc('d')
      await ref.set({ n: 1 })
      await ref.update({ at: FieldValue.serverTimestamp(), also: FieldValue.serverTimestamp() })

      const snapshot = await ref.get()
      const at = snapshot.get('at')
      assert.ok(at, 'serverTimestamp() must have written a timestamp')
      assert.equal(
        at.nanoseconds % 1_000_000,
        0,
        `REQUEST_TIME has millisecond precision, got ${at.nanoseconds}ns`,
      )
      assert.equal(
        at.toMillis(),
        snapshot.get('also').toMillis(),
        'every field transformed in one commit gets the same value',
      )
      assert.ok(
        at.toMillis() <= snapshot.updateTime.toMillis(),
        `the stamp must be no later than update_time: ${at.toMillis()} vs ${snapshot.updateTime.toMillis()}`,
      )
    },
  },

  {
    id: 'transform.increment',
    served: async ({ freshCollection }) => {
      const ref = freshCollection('increment').doc('d')
      await ref.set({ n: 10 })

      await ref.update({ n: FieldValue.increment(5) })
      assert.equal((await ref.get()).get('n'), 15)

      await ref.update({ n: FieldValue.increment(-15) })
      assert.equal((await ref.get()).get('n'), 0)
    },
  },

  {
    id: 'transform.array-union',
    served: async ({ freshCollection }) => {
      const ref = freshCollection('arrayunion').doc('d')
      await ref.set({ tags: ['a'] })

      // 'a' is already there and 'b' is repeated in the input: the result must
      // gain exactly one 'b'.
      await ref.update({ tags: FieldValue.arrayUnion('a', 'b', 'b') })
      assert.deepEqual((await ref.get()).get('tags'), ['a', 'b'])

      // Elements compare across numeric types, so 3 and 3.0 are one element.
      await ref.set({ nums: [3] })
      await ref.update({ nums: FieldValue.arrayUnion(3.0, 4) })
      assert.deepEqual((await ref.get()).get('nums'), [3, 4])
    },
  },

  {
    id: 'transform.array-remove',
    served: async ({ freshCollection }) => {
      const ref = freshCollection('arrayremove').doc('d')
      await ref.set({ tags: ['a', 'b', 'a'] })

      await ref.update({ tags: FieldValue.arrayRemove('a') })
      assert.deepEqual(
        (await ref.get()).get('tags'),
        ['b'],
        'remove_all_from_array removes every occurrence',
      )
    },
  },

  {
    id: 'transform.maximum-minimum',
    unimplemented: async ({ raw, db, projectId, runId, expectUnimplemented }) => {
      // No SDK surface emits maximum/minimum, so they go on the wire. Served
      // would mean implemented; refused means an SDK that somehow sends one
      // sees a clear gap instead of a wrong number.
      const path = `conf_${runId}_maxmin/d`
      await db.doc(path).set({ n: 1 })
      const name = documentName(projectId, path)

      for (const [field, transform] of [
        ['maximum', { maximum: value.integer(5) }],
        ['minimum', { minimum: value.integer(5) }],
      ]) {
        await expectUnimplemented(
          () =>
            unary(raw.firestore, 'Commit', {
              database: databaseName(projectId),
              writes: [
                {
                  update: { name, fields: {} },
                  update_mask: { field_paths: [] },
                  update_transforms: [{ field_path: 'n', ...transform }],
                },
              ],
            }),
          { names: new RegExp(`${field} field transform`) },
        )
      }

      assert.equal((await db.doc(path).get()).get('n'), 1, 'a refused transform writes nothing')
    },
  },

  {
    id: 'transform.nested-field-path',
    unimplemented: async ({ freshCollection, expectUnimplemented }) => {
      const ref = freshCollection('nestedtransform').doc('d')
      await ref.set({ a: { b: 1 } })

      // A transform is a write, so it is held to the write path's field-path
      // rule: whole top-level fields only. Reads do support nested paths.
      await expectUnimplemented(() => ref.update({ 'a.b': FieldValue.increment(1) }), {
        names: /nested field transform path/i,
      })
      assert.deepEqual((await ref.get()).get('a'), { b: 1 }, 'the refusal must write nothing')
    },
  },

  // ---- the semantics that can be plausibly wrong ---------------------------

  {
    id: 'transform.runs-after-its-mutation',
    served: async ({ freshCollection }) => {
      const docs = freshCollection('transformorder')

      // set() replaces the document first, so the transform reads a cleared
      // field and stores the operand.
      const replaced = docs.doc('replaced')
      await replaced.set({ n: 10 })
      await replaced.set({ n: FieldValue.increment(5) })
      assert.equal(
        (await replaced.get()).get('n'),
        5,
        'set() clears the field before the transform reads it',
      )

      // A masked update() leaves the field alone, so the transform sees it.
      const merged = docs.doc('merged')
      await merged.set({ n: 10 })
      await merged.update({ n: FieldValue.increment(5) })
      assert.equal((await merged.get()).get('n'), 15)
    },
  },

  {
    id: 'transform.increment-numeric-rules',
    served: async ({ freshCollection }) => {
      const docs = freshCollection('incrementrules')

      // A missing field, or one holding a string, is set to the operand.
      const missing = docs.doc('missing')
      await missing.set({ other: true })
      await missing.update({ n: FieldValue.increment(3) })
      assert.equal((await missing.get()).get('n'), 3)

      const stringy = docs.doc('stringy')
      await stringy.set({ n: 'not a number' })
      await stringy.update({ n: FieldValue.increment(3) })
      assert.equal((await stringy.get()).get('n'), 3)

      // A double on either side makes the sum a double; two integers stay
      // integral. JavaScript has one number type, so the check is the value.
      const mixed = docs.doc('mixed')
      await mixed.set({ n: 1 })
      await mixed.update({ n: FieldValue.increment(0.5) })
      assert.equal((await mixed.get()).get('n'), 1.5)
    },
  },

  {
    id: 'transform.array-missing-field-is-empty',
    served: async ({ freshCollection }) => {
      const docs = freshCollection('arrayfallback')

      const removed = docs.doc('removed')
      await removed.set({ tags: 'a string, not an array' })
      await removed.update({ tags: FieldValue.arrayRemove('a') })
      assert.deepEqual(
        (await removed.get()).get('tags'),
        [],
        'arrayRemove on a non-array leaves the empty array, not the string',
      )

      const added = docs.doc('added')
      await added.set({ other: true })
      await added.update({ tags: FieldValue.arrayUnion('a') })
      assert.deepEqual((await added.get()).get('tags'), ['a'])
    },
  },
]
