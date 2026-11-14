/**
 * The Datastore fidelity backlog: ordinary `@google-cloud/datastore` usage that
 * `fireup local` is not yet known to serve.
 *
 * `smoke.mjs` is what the fidelity gate protects; every check in it passes on
 * `main`. This file is the other side: checks written from the public Datastore
 * v1 API and the public client library, with no regard for whether Fireup
 * passes them today. It is the input of the triage role
 * (`docs/local/TRIAGE.md`): run it, group what fails by root cause, and file one
 * fixer issue per group. A check that a fix makes pass moves into `smoke.mjs`,
 * where the gate's baseline then protects it.
 *
 * Its suite id is `datastore-backlog/<store>`, which `baseline.json` does not
 * list, so nothing here can fail the gate however red it is.
 *
 * Written against the public @google-cloud/datastore API. No emulator binary
 * was reverse-engineered.
 */

import { and, Datastore, PropertyFilter } from '@google-cloud/datastore'

import { writeResults } from '../smoke-gate/results.mjs'

const projectId = process.env.FIREUP_SMOKE_PROJECT_ID || 'fireup-local'
const datastore = new Datastore({ projectId })

// A run id in every kind and namespace, so a second run against a durable store
// starts from nothing rather than from the last run's entities.
const run = Date.now().toString(36)
const kind = `BacklogTask_${run}`
const namespace = `backlog_${run}`

const checks = []

/**
 * The same harness as `smoke.mjs`, with the gRPC status kept: the triager groups
 * failures by it, and "UNIMPLEMENTED: no cursors" and "wrong rows came back" are
 * different root causes even when they fail the same check.
 */
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
  writeResults(checks)
}

const names = (rows) => rows.map((row) => row[datastore.KEY].name).sort()

const expectNames = (rows, want, what) => {
  const got = names(rows)
  if (JSON.stringify(got) !== JSON.stringify([...want].sort())) {
    throw new Error(`${what}: got ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`)
  }
}

/** The gRPC status a call fails with, or null when it succeeds. */
const failureCode = (promise) => promise.then(() => null, (error) => error?.code ?? 'no code')

const key = (name) => datastore.key([kind, name])
const fixture = [
  { name: 'a', data: { priority: 1, tags: ['x', 'y'], done: false, owner: 'ada' } },
  { name: 'b', data: { priority: 2, tags: ['y'], done: true, owner: 'bob' } },
  { name: 'c', data: { priority: 3, tags: ['z'], done: false, owner: 'ada' } },
]

await check('seed the backlog fixture', async () => {
  await datastore.save(fixture.map(({ name, data }) => ({ key: key(name), data })))
})

// --- Projections ---------------------------------------------------------

await check('a keys-only query returns keys without properties', async () => {
  const [rows] = await datastore.runQuery(datastore.createQuery(kind).select('__key__'))
  expectNames(rows, ['a', 'b', 'c'], 'keys-only query')
  if (rows.some((row) => row.priority !== undefined)) {
    throw new Error(`keys-only rows carried properties: ${JSON.stringify(rows)}`)
  }
})

await check('a projection query returns only the selected properties', async () => {
  const [rows] = await datastore.runQuery(datastore.createQuery(kind).select('priority').order('priority'))
  const got = rows.map((row) => ({ priority: row.priority, owner: row.owner }))
  const want = [1, 2, 3].map((priority) => ({ priority, owner: undefined }))
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    throw new Error(`projection returned ${JSON.stringify(rows)}`)
  }
})

await check('groupBy returns one entity per distinct value', async () => {
  const query = datastore.createQuery(kind).select('owner').groupBy('owner').order('owner')
  const [rows] = await datastore.runQuery(query)
  const owners = rows.map((row) => row.owner)
  if (JSON.stringify(owners) !== JSON.stringify(['ada', 'bob'])) {
    throw new Error(`groupBy returned owners ${JSON.stringify(owners)}`)
  }
})

// --- Cursors -------------------------------------------------------------

await check('a limited query reports an end cursor and more results after the limit', async () => {
  const [, info] = await datastore.runQuery(datastore.createQuery(kind).order('priority').limit(2))
  if (!info?.endCursor) {
    throw new Error(`no endCursor: ${JSON.stringify(info)}`)
  }
  if (info.moreResults !== 'MORE_RESULTS_AFTER_LIMIT') {
    throw new Error(`moreResults is ${info.moreResults}, wanted MORE_RESULTS_AFTER_LIMIT`)
  }
})

await check('a query resumes from the end cursor of the previous page', async () => {
  const page = datastore.createQuery(kind).order('priority').limit(2)
  const [first, info] = await datastore.runQuery(page)
  expectNames(first, ['a', 'b'], 'first page')
  const [rest] = await datastore.runQuery(
    datastore.createQuery(kind).order('priority').start(info.endCursor),
  )
  expectNames(rest, ['c'], 'second page')
})

// --- Filters -------------------------------------------------------------

await check('an AND of two equality filters matches both', async () => {
  const query = datastore
    .createQuery(kind)
    .filter(and([new PropertyFilter('owner', '=', 'ada'), new PropertyFilter('done', '=', false)]))
  const [rows] = await datastore.runQuery(query)
  expectNames(rows, ['a', 'c'], 'AND filter')
})

await check('equality on an array property matches any element', async () => {
  const query = datastore.createQuery(kind).filter(new PropertyFilter('tags', '=', 'y'))
  const [rows] = await datastore.runQuery(query)
  expectNames(rows, ['a', 'b'], 'array equality')
})

await check('a __key__ equality filter returns that entity', async () => {
  const query = datastore.createQuery(kind).filter(new PropertyFilter('__key__', '=', key('b')))
  const [rows] = await datastore.runQuery(query)
  expectNames(rows, ['b'], '__key__ filter')
})

await check('a query orders by __key__ descending', async () => {
  const [rows] = await datastore.runQuery(datastore.createQuery(kind).order('__key__', { descending: true }))
  const got = rows.map((row) => row[datastore.KEY].name)
  if (JSON.stringify(got) !== JSON.stringify(['c', 'b', 'a'])) {
    throw new Error(`descending key order returned ${JSON.stringify(got)}`)
  }
})

// --- Aggregations --------------------------------------------------------

await check('a count() aggregation counts the matching entities', async () => {
  const query = datastore
    .createAggregationQuery(datastore.createQuery(kind).filter(new PropertyFilter('owner', '=', 'ada')))
    .count('total')
  const [[result]] = await datastore.runAggregationQuery(query)
  if (result?.total !== 2) {
    throw new Error(`count returned ${JSON.stringify(result)}, wanted total=2`)
  }
})

await check('sum() and average() aggregate a numeric property', async () => {
  const query = datastore
    .createAggregationQuery(datastore.createQuery(kind))
    .sum('priority', 'sum')
    .average('priority', 'avg')
  const [[result]] = await datastore.runAggregationQuery(query)
  if (result?.sum !== 6 || result?.avg !== 2) {
    throw new Error(`sum/average returned ${JSON.stringify(result)}, wanted sum=6 avg=2`)
  }
})

// --- Namespaces and kindless queries --------------------------------------

await check('an entity in a named namespace saves and reads back', async () => {
  const nsKey = datastore.key({ namespace, path: [kind, 'ns-entity'] })
  await datastore.save({ key: nsKey, data: { title: 'namespaced' } })
  const [fetched] = await datastore.get(nsKey)
  if (fetched?.title !== 'namespaced') {
    throw new Error(`namespaced get returned ${JSON.stringify(fetched)}`)
  }
})

await check('a query in a named namespace sees only that namespace', async () => {
  const [rows] = await datastore.runQuery(datastore.createQuery(namespace, kind))
  expectNames(rows, ['ns-entity'], 'namespaced query')
})

await check('a kindless ancestor query returns the ancestor and its descendants', async () => {
  const parent = key('parent')
  const child = datastore.key([kind, 'parent', `${kind}Child`, 'child'])
  await datastore.save([
    { key: parent, data: { role: 'parent' } },
    { key: child, data: { role: 'child' } },
  ])
  const [rows] = await datastore.runQuery(datastore.createQuery().hasAncestor(parent))
  expectNames(rows, ['child', 'parent'], 'kindless ancestor query')
})

// --- Writes and reads ----------------------------------------------------

await check('insert of an existing key fails with ALREADY_EXISTS', async () => {
  const code = await failureCode(datastore.insert({ key: key('a'), data: { priority: 9 } }))
  if (code !== 6) throw new Error(`insert of an existing key failed with ${code}, wanted 6`)
})

await check('update of a missing key fails with NOT_FOUND', async () => {
  const code = await failureCode(datastore.update({ key: key('missing'), data: { priority: 9 } }))
  if (code !== 5) throw new Error(`update of a missing key failed with ${code}, wanted 5`)
})

await check('get of several keys returns the found entities and skips missing ones', async () => {
  const [rows] = await datastore.get([key('a'), key('missing'), key('c')])
  expectNames(rows, ['a', 'c'], 'multi-get')
})

await check('merge() keeps the properties it was not given', async () => {
  await datastore.merge({ key: key('b'), data: { owner: 'bea' } })
  const [merged] = await datastore.get(key('b'))
  if (merged?.owner !== 'bea' || merged?.priority !== 2) {
    throw new Error(`merge left ${JSON.stringify(merged)}`)
  }
})

await check('a get at a past readTime returns the entity as it was then', async () => {
  const before = Date.now()
  await new Promise((resolve) => setTimeout(resolve, 50))
  await datastore.save({ key: key('versioned'), data: { v: 1 } })
  const [fetched] = await datastore.get(key('versioned'), { readTime: before })
  if (fetched !== undefined) {
    throw new Error(`an entity written after readTime was visible: ${JSON.stringify(fetched)}`)
  }
})

const failed = checks.filter((c) => !c.ok)
console.log(`\n${checks.length - failed.length}/${checks.length} backlog checks pass`)
// Always 0: a failing backlog check is the expected state, not an error. A crash
// still exits non-zero, and the results file says how far the run got.
