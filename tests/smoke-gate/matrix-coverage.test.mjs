/**
 * The matrix-coverage rule, and the tags and gap list it is applied with.
 *
 *   node --test tests/smoke-gate/matrix-coverage.test.mjs
 */

import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'

import { matrixCoverage, STATUSES } from './matrix-coverage.mjs'

const matrix = {
  features: [
    { id: 'firestore.Commit', label: 'Commit', status: 'served' },
    { id: 'query.aggregations', label: 'Aggregations', status: 'unimplemented' },
  ],
}
const noGaps = { rows: [] }

const run = (checks) => [{ suite: 'firestore/memory', checks }]
const commitCheck = { name: 'create', ok: true, covers: { served: ['firestore.Commit'] } }
const refusalCheck = { name: 'count', ok: true, covers: { unimplemented: ['query.aggregations'] } }

const rowsIn = (problems) => problems.map(({ row }) => row)

test('a matrix whose every row has a passing check of its status is clean', () => {
  const { problems, proven } = matrixCoverage(matrix, run([commitCheck, refusalCheck]), noGaps)
  assert.deepEqual(problems, [])
  assert.deepEqual(proven, ['firestore.Commit', 'query.aggregations'])
})

test('a served row with no check fails, naming the row', () => {
  const { problems } = matrixCoverage(matrix, run([refusalCheck]), noGaps)
  assert.deepEqual(rowsIn(problems), ['firestore.Commit'])
  assert.match(problems[0].message, /row firestore\.Commit \("Commit"\) is "served" but no smoke check proves it/)
  assert.match(problems[0].message, /a passing check that it works/)
})

test('an unimplemented row with no refusal check fails, naming the row', () => {
  const { problems } = matrixCoverage(matrix, run([commitCheck]), noGaps)
  assert.deepEqual(rowsIn(problems), ['query.aggregations'])
  assert.match(problems[0].message, /refused with an explicit error/)
})

test('a failing check proves nothing', () => {
  const { problems } = matrixCoverage(matrix, run([{ ...commitCheck, ok: false }, refusalCheck]), noGaps)
  assert.deepEqual(rowsIn(problems), ['firestore.Commit'])
})

test('a row must pass in every suite run that checks it', () => {
  // Proven in memory and failing on PostgreSQL is a broken row, not a proven one.
  const results = [
    { suite: 'firestore/memory', checks: [commitCheck, refusalCheck] },
    { suite: 'firestore/postgres', checks: [{ ...commitCheck, ok: false }, refusalCheck] },
  ]
  const { problems, proven } = matrixCoverage(matrix, results, noGaps)
  assert.deepEqual(rowsIn(problems), ['firestore.Commit'])
  assert.match(problems[0].message, /row firestore\.Commit \("Commit"\) is "served" but none of its checks passed in firestore\/postgres/)
  assert.deepEqual(proven, ['query.aggregations'])
})

test('a row flipped without its check fails, in both directions', () => {
  // The row now says served, the check still asserts the refusal: the check
  // is passing, so this is exactly the drift nothing else would report.
  const flipped = {
    features: [matrix.features[0], { ...matrix.features[1], status: 'served' }],
  }
  const { problems } = matrixCoverage(flipped, run([commitCheck, refusalCheck]), noGaps)
  assert.deepEqual(rowsIn(problems), ['query.aggregations', 'query.aggregations'])
  assert.match(problems[0].message, /is "served" in the matrix, but check "count" \(firestore\/memory\) asserts it is "unimplemented"/)
  assert.match(problems[1].message, /no smoke check proves it/)

  const refused = { features: [{ ...matrix.features[0], status: 'unimplemented' }, matrix.features[1]] }
  assert.deepEqual(rowsIn(matrixCoverage(refused, run([commitCheck, refusalCheck]), noGaps).problems), [
    'firestore.Commit',
    'firestore.Commit',
  ])
})

test('a check naming a row the matrix does not have fails', () => {
  const stray = { name: 'stray', ok: true, covers: { served: ['firestore.Comit'] } }
  const { problems } = matrixCoverage(matrix, run([commitCheck, refusalCheck, stray]), noGaps)
  assert.deepEqual(rowsIn(problems), ['firestore.Comit'])
  assert.match(problems[0].message, /check "stray" \(firestore\/memory\) names matrix row firestore\.Comit, which/)
})

test('a tag under a status the matrix does not use fails', () => {
  const misspelt = { name: 'misspelt', ok: true, covers: { serves: ['firestore.Commit'] } }
  const { problems } = matrixCoverage(matrix, run([misspelt, refusalCheck]), noGaps)
  assert.deepEqual(rowsIn(problems), ['firestore.Commit', 'firestore.Commit'])
  assert.match(problems[0].message, /tags row firestore\.Commit as "serves"/)
})

test('a listed gap stands in for a missing check', () => {
  const gaps = { rows: [{ id: 'query.aggregations', status: 'unimplemented', reason: 'r' }] }
  const { problems, gaps: listed } = matrixCoverage(matrix, run([commitCheck]), gaps)
  assert.deepEqual(problems, [])
  assert.deepEqual(listed, ['query.aggregations'])
})

test('a gap entry fails once its row is proven, so the list cannot go stale', () => {
  const gaps = { rows: [{ id: 'query.aggregations', status: 'unimplemented', reason: 'r' }] }
  const { problems } = matrixCoverage(matrix, run([commitCheck, refusalCheck]), gaps)
  assert.deepEqual(rowsIn(problems), ['query.aggregations'])
  assert.match(problems[0].message, /now has a passing check \(firestore\/memory: count\); delete its entry/)
})

test('flipping a gap row fails too, because the entry records the status', () => {
  const gaps = { rows: [{ id: 'query.aggregations', status: 'unimplemented', reason: 'r' }] }
  const flipped = { features: [matrix.features[0], { ...matrix.features[1], status: 'served' }] }
  const { problems } = matrixCoverage(flipped, run([commitCheck]), gaps)
  assert.deepEqual(rowsIn(problems), ['query.aggregations'])
  assert.match(problems[0].message, /matrix-gaps\.json records it as "unimplemented"/)
})

test('a gap entry must name a real row, once, with a reason', () => {
  const gaps = {
    rows: [
      { id: 'query.aggregations', status: 'unimplemented', reason: 'r' },
      { id: 'query.aggregations', status: 'unimplemented', reason: 'r' },
      { id: 'nope', status: 'served', reason: 'r' },
      { id: 'firestore.Commit', status: 'served' },
    ],
  }
  const messages = matrixCoverage(matrix, run([]), gaps).problems.map(({ message }) => message)
  assert.deepEqual(messages, [
    'matrix-gaps.json lists row query.aggregations twice',
    'matrix-gaps.json lists row nope, which the matrix does not have',
    'matrix-gaps.json lists row firestore.Commit with no reason given',
  ])
})

// ---------------------------------------------------------------------------
// The committed tags and gap list, checked without a server. The gate checks
// the same things from real results, but only after minutes of SDK runs.
// ---------------------------------------------------------------------------

const readText = (path) => readFileSync(new URL(path, import.meta.url), 'utf8')
const realMatrix = JSON.parse(readText('../conformance/matrix.json'))
const realRows = new Map(realMatrix.features.map((feature) => [feature.id, feature]))
const realGaps = JSON.parse(readText('./matrix-gaps.json'))

const SMOKE_SUITES = [
  '../local-smoke/smoke.mjs',
  '../local-smoke/auth-smoke.mjs',
  '../local-smoke-datastore/smoke.mjs',
]

/**
 * The tags in a smoke suite: the third argument of each `check(...)`, which
 * closes the call on a line of its own — `}, { served: ['firestore.Commit'] })`.
 */
const tagsIn = (source) =>
  [...source.matchAll(/^\s*\}, (\{ .* \})\)$/gm)].map(([, literal]) =>
    // Object literals with quoted strings and bare or quoted keys; JSON with a
    // little help.
    JSON.parse(literal.replaceAll("'", '"').replace(/([{,]\s*)([a-z]+):/g, '$1"$2":')),
  )

test('every tag in the smoke suites names a matrix row under the status the matrix gives it', () => {
  let tags = 0
  for (const suite of SMOKE_SUITES) {
    for (const covers of tagsIn(readText(suite))) {
      for (const [status, ids] of Object.entries(covers)) {
        assert.ok(STATUSES.includes(status), `${suite}: tag status "${status}" is not one of ${STATUSES}`)
        for (const id of ids) {
          tags += 1
          const row = realRows.get(id)
          assert.ok(row, `${suite} tags row ${id}, which tests/conformance/matrix.json does not have`)
          assert.equal(
            row.status,
            status,
            `${suite} asserts row ${id} is "${status}", but the matrix says "${row.status}"; flip them together`,
          )
        }
      }
    }
  }
  // A floor, so a change to how tags are written cannot leave this checking
  // nothing: 127 tags on 94 checks across the three suites when this was written.
  assert.ok(tags >= 110, `found only ${tags} tags in the smoke suites; has the tag syntax changed?`)
})

test('every gap entry names a matrix row under its current status, with a reason', () => {
  assert.ok(realGaps.rows.length > 0)
  for (const { id, status, reason } of realGaps.rows) {
    const row = realRows.get(id)
    assert.ok(row, `matrix-gaps.json lists row ${id}, which the matrix does not have`)
    assert.equal(row.status, status, `matrix-gaps.json records ${id} as "${status}", the matrix says "${row.status}"`)
    assert.ok(reason?.trim(), `matrix-gaps.json lists ${id} with no reason given`)
  }
})

test('no gap entry duplicates a row a smoke tag already claims', () => {
  // The gate catches this from results; here it is caught before the run. A
  // tagged row could still be listed while its check fails, which is what the
  // gap list is for, so this only guards the plain mistake of doing both.
  const tagged = new Set(
    SMOKE_SUITES.flatMap((suite) => tagsIn(readText(suite)).flatMap((covers) => Object.values(covers).flat())),
  )
  const both = realGaps.rows.map(({ id }) => id).filter((id) => tagged.has(id))
  assert.deepEqual(both, [], 'rows both tagged by a smoke check and listed as a gap')
})
