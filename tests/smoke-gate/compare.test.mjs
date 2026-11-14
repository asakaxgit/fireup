/**
 * The gate's rule, and the committed baseline it is applied with.
 *
 *   node --test tests/smoke-gate/
 */

import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'

import { baselineFrom, compareToBaseline } from './compare.mjs'

const baseline = { suites: { 'firestore/memory': ['create', 'get'] } }

const run = (checks) => [{ suite: 'firestore/memory', checks }]

test('a run where every baseline check passes is clean', () => {
  const { regressions } = compareToBaseline(baseline, run([{ name: 'create', ok: true }, { name: 'get', ok: true }]))
  assert.deepEqual(regressions, [])
})

test('a baseline check that now fails is a regression, by name', () => {
  const { regressions } = compareToBaseline(baseline, run([{ name: 'create', ok: false }, { name: 'get', ok: true }]))
  assert.deepEqual(regressions, [{ suite: 'firestore/memory', name: 'create', reason: 'check failed' }])
})

test('a baseline check that disappears is a regression', () => {
  // Renamed or deleted: either way the behaviour it pinned is no longer checked.
  const { regressions } = compareToBaseline(baseline, run([{ name: 'get', ok: true }]))
  assert.deepEqual(regressions, [{ suite: 'firestore/memory', name: 'create', reason: 'check is missing' }])
})

test('a suite that reported nothing regresses every one of its checks', () => {
  const { regressions } = compareToBaseline(baseline, [])
  assert.deepEqual(
    regressions.map(({ name, reason }) => [name, reason]),
    [
      ['create', 'suite reported no results'],
      ['get', 'suite reported no results'],
    ],
  )
})

test('a new check never fails the build, passing or not', () => {
  const { regressions, newlyPassing } = compareToBaseline(
    baseline,
    run([
      { name: 'create', ok: true },
      { name: 'get', ok: true },
      { name: 'brand new and passing', ok: true },
      { name: 'brand new and failing', ok: false },
    ]),
  )
  assert.deepEqual(regressions, [])
  assert.deepEqual(newlyPassing, [{ suite: 'firestore/memory', name: 'brand new and passing' }])
})

test('a suite the baseline does not know yet never fails the build', () => {
  const results = [...run([{ name: 'create', ok: true }, { name: 'get', ok: true }])]
  results.push({ suite: 'auth/memory', checks: [{ name: 'sign up', ok: false }] })
  assert.deepEqual(compareToBaseline(baseline, results).regressions, [])
})

test('a duplicated check name is refused, so a passing copy cannot hide a failing one', () => {
  const results = run([
    { name: 'create', ok: false },
    { name: 'create', ok: true },
    { name: 'get', ok: true },
  ])
  assert.throws(() => compareToBaseline(baseline, results), /reported check "create" twice/)
})

test('two result files for one suite are refused', () => {
  const results = [...run([{ name: 'create', ok: true }]), ...run([{ name: 'get', ok: true }])]
  assert.throws(() => compareToBaseline(baseline, results), /more than one result file/)
})

test('only a literal `ok: true` counts as passing', () => {
  const { regressions } = compareToBaseline(baseline, run([{ name: 'create', ok: 'yes' }, { name: 'get' }]))
  assert.deepEqual(
    regressions.map(({ name }) => name),
    ['create', 'get'],
  )
})

test('an empty baseline is refused rather than passing everything', () => {
  assert.throws(() => compareToBaseline({ suites: {} }, []), /lists no checks/)
  assert.throws(() => compareToBaseline({ suites: { 'firestore/memory': [] } }, []), /lists no checks/)
})

test('baselineFrom keeps the passing checks, sorted', () => {
  const results = run([
    { name: 'get', ok: true },
    { name: 'broken', ok: false },
    { name: 'create', ok: true },
  ])
  assert.deepEqual(baselineFrom(results), { suites: { 'firestore/memory': ['create', 'get'] } })
})

// The committed baseline has to cover exactly the runs the gate script makes.
// Dropping a suite from baseline.json would otherwise switch its protection off
// with the gate still green; a run missing from the script is already caught,
// as a suite that reported nothing.
test('baseline.json covers every suite run in scripts/fidelity-smoke-gate.sh', () => {
  const committed = JSON.parse(readFileSync(new URL('./baseline.json', import.meta.url), 'utf8'))
  const script = readFileSync(new URL('../../scripts/fidelity-smoke-gate.sh', import.meta.url), 'utf8')
  const runs = [...script.matchAll(/^\s+"(\w+) (\w+) [\w-]+\.sh"$/gm)].map(([, suite, store]) => `${suite}/${store}`)

  assert.equal(runs.length, 8, 'expected the gate script to list eight suite runs')
  assert.deepEqual(Object.keys(committed.suites).sort(), runs.sort())
  for (const [suite, names] of Object.entries(committed.suites)) {
    assert.ok(names.length > 0, `${suite} has no baseline checks`)
  }
})
