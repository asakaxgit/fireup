import assert from 'node:assert/strict'
import { test } from 'node:test'

import { clusterFailures, failuresFrom, signatureOf } from './cluster.mjs'

const refused = (name, value) => ({
  name,
  ok: false,
  error: { code: 12, message: `12 UNIMPLEMENTED: no namespaces yet (got "${value}", attempt 3)` },
})

const wrong = (name) => ({ name, ok: false, error: { code: null, message: `${name}: got [], wanted ["a"]` } })

test('a status signature blanks the run-specific values', () => {
  assert.equal(signatureOf(refused('x', 'ns_1')), signatureOf(refused('y', 'ns_2')))
  assert.equal(signatureOf(refused('x', 'ns_1')), 'UNIMPLEMENTED: no namespaces yet (got "…", attempt N)')
})

test('a wrong answer is its own cluster, keyed on the check', () => {
  assert.equal(signatureOf(wrong('IN filter')), 'wrong result: IN filter')
  assert.notEqual(signatureOf(wrong('IN filter')), signatureOf(wrong('OR filter')))
})

test('refusals with one cause cluster together; wrong answers stay apart', () => {
  const clusters = clusterFailures(
    failuresFrom([
      {
        suite: 'ds/memory',
        checks: [refused('save in ns', 'a'), refused('query in ns', 'b'), wrong('IN filter'), wrong('OR filter')],
      },
    ]),
  )
  assert.equal(clusters.length, 3)
  assert.deepEqual(clusters[0].checks.map((c) => c.name), ['save in ns', 'query in ns'])
})

test('one check failing on every store is one entry, not store-specific', () => {
  const [failure] = failuresFrom([
    { suite: 'ds/memory', checks: [wrong('IN filter')] },
    { suite: 'ds/postgres', checks: [wrong('IN filter')] },
  ])
  assert.deepEqual(failure.stores, ['memory', 'postgres'])
  assert.equal(failure.storeSpecific, false)
})

test('a check failing on one store of two is flagged store-specific', () => {
  const [failure] = failuresFrom([
    { suite: 'ds/memory', checks: [{ name: 'IN filter', ok: true }] },
    { suite: 'ds/postgres', checks: [wrong('IN filter')] },
  ])
  assert.deepEqual(failure.stores, ['postgres'])
  assert.equal(failure.storeSpecific, true)
})

test('passing checks and results from before errors were recorded do not crash it', () => {
  const failures = failuresFrom([{ suite: 'ds/memory', checks: [{ name: 'a', ok: true }, { name: 'b', ok: false }] }])
  assert.equal(failures.length, 1)
  assert.equal(failures[0].signature, 'wrong result: b')
})
