/**
 * Fail when a fidelity check that passes on `main` stops passing.
 *
 *   node tests/smoke-gate/gate.mjs <results-dir>           check, exit 1 on a regression
 *   node tests/smoke-gate/gate.mjs --update <results-dir>  rewrite baseline.json
 *
 * `<results-dir>` holds the `*.json` files the smoke suites write through
 * `results.mjs`; `scripts/fidelity-smoke-gate.sh` runs every suite and then this.
 * The rule itself is in `compare.mjs`.
 *
 * The check also ties the results to the fidelity matrix: every row of
 * tests/conformance/matrix.json needs a passing check tagged with its status, or
 * an entry in matrix-gaps.json. That rule is in `matrix-coverage.mjs`.
 *
 * `--update` replaces only the suites present in `<results-dir>` and leaves the
 * others as they are, so refreshing the in-memory runs on a machine without
 * PostgreSQL does not quietly drop the Postgres half of the baseline. Removing a
 * suite is a hand edit, which is the point: a baseline change is a diff someone
 * reviews.
 */

import { readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

import { baselineFrom, compareToBaseline } from './compare.mjs'
import { matrixCoverage } from './matrix-coverage.mjs'

const BASELINE = new URL('./baseline.json', import.meta.url)
const MATRIX = new URL('../conformance/matrix.json', import.meta.url)
const GAPS = new URL('./matrix-gaps.json', import.meta.url)

const readJson = (url) => JSON.parse(readFileSync(url, 'utf8'))

const readResults = (dir) =>
  readdirSync(dir)
    .filter((file) => file.endsWith('.json'))
    .sort()
    .map((file) => JSON.parse(readFileSync(join(dir, file), 'utf8')))

const update = (dir) => {
  const current = JSON.parse(readFileSync(BASELINE, 'utf8'))
  const fresh = baselineFrom(readResults(dir))
  const merged = { ...current.suites, ...fresh.suites }
  const suites = Object.fromEntries(Object.keys(merged).sort().map((suite) => [suite, merged[suite]]))
  writeFileSync(BASELINE, `${JSON.stringify({ ...current, suites }, null, 2)}\n`)
  for (const [suite, names] of Object.entries(fresh.suites)) {
    console.log(`baseline: ${suite} — ${names.length} passing checks`)
  }
}

const check = (dir) => {
  const baseline = JSON.parse(readFileSync(BASELINE, 'utf8'))
  const results = readResults(dir)
  const { regressions, newlyPassing } = compareToBaseline(baseline, results)

  const expected = Object.values(baseline.suites).reduce((sum, names) => sum + names.length, 0)
  console.log(`fidelity gate: ${expected} baseline checks across ${Object.keys(baseline.suites).length} suite runs`)

  for (const { suite, name } of newlyPassing) {
    console.log(`  new  ${suite}: ${name}`)
  }
  if (newlyPassing.length > 0) {
    console.log('  (passing but not in the baseline — add them with --update so they are protected too)')
  }

  checkMatrix(results)

  if (regressions.length === 0) {
    console.log('fidelity gate: no regressions')
    return
  }

  console.log(`\nfidelity gate: ${regressions.length} check(s) that pass on main no longer pass:`)
  for (const { suite, name, reason } of regressions) {
    console.log(`  REGRESSED ${suite}: ${name} (${reason})`)
    // A workflow command, so each one is also an annotation on the run summary
    // rather than only a line in a long log.
    if (process.env.GITHUB_ACTIONS) {
      console.log(`::error title=Fidelity regression (${suite})::${name} — ${reason}`)
    }
  }
  console.log('\nIf a check was removed or renamed on purpose, update the baseline: see docs/local/README.md.')
  process.exitCode = 1
}

const checkMatrix = (results) => {
  const { problems, proven, gaps } = matrixCoverage(readJson(MATRIX), results, readJson(GAPS))
  console.log(`fidelity matrix: ${proven.length} rows proven by a passing smoke check, ${gaps.length} listed in matrix-gaps.json`)
  if (problems.length === 0) return

  console.log(`\nfidelity matrix: ${problems.length} row(s) the smoke results do not back:`)
  for (const { row, message } of problems) {
    console.log(`  MATRIX ${message}`)
    if (process.env.GITHUB_ACTIONS) {
      console.log(`::error title=Fidelity matrix row ${row}::${message}`)
    }
  }
  console.log('')
  process.exitCode = 1
}

const [first, second] = process.argv.slice(2)
if (first === '--update' && second) {
  update(second)
} else if (first && !first.startsWith('-') && !second) {
  check(first)
} else {
  console.error('usage: gate.mjs <results-dir> | gate.mjs --update <results-dir>')
  process.exitCode = 2
}
