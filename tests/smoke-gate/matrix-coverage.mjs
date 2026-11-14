/**
 * Which fidelity-matrix rows the smoke suites prove, kept free of I/O so it can
 * be tested on its own (`matrix-coverage.test.mjs`).
 *
 * Every row of the matrix in docs/local/README.md has a stable id in
 * tests/conformance/matrix.json, and a status: `served`, `unimplemented`, or
 * `documented-divergence`. A smoke check names the rows it asserts, under the
 * status it asserts them with:
 *
 *   check('create() on an existing document fails with ALREADY_EXISTS', fn, { served: ['firestore.Commit'] })
 *   check('count() reports UNIMPLEMENTED', fn, { unimplemented: ['query.aggregations'] })
 *
 * `results.mjs` writes those tags into the result file next to `ok`, and this
 * rule reads them back. It fails when:
 *
 *   - a check names a row id the matrix does not have;
 *   - a check asserts a row with a status the matrix no longer gives it, which
 *     is what flipping a row without touching its tests looks like;
 *   - a row has no passing check asserting its status, and is not listed in
 *     matrix-gaps.json.
 *
 * matrix-gaps.json is the honest list of rows no smoke check proves yet, each
 * with the issue tracking it. An entry records the row's status, so flipping a
 * gap row is caught too, and an entry whose row has since gained a passing check
 * fails until it is deleted — otherwise the list only ever grows.
 */

export const STATUSES = ['served', 'unimplemented', 'documented-divergence']

/** The `served` / `unimplemented` / ... tags a check carries, as [status, id] pairs. */
const tagsOf = (covers) =>
  Object.entries(covers ?? {}).flatMap(([status, ids]) => (Array.isArray(ids) ? ids : [ids]).map((id) => [status, id]))

export const matrixCoverage = (matrix, results, gaps) => {
  const rows = new Map(matrix.features.map((feature) => [feature.id, feature]))
  const problems = []
  const provenBy = new Map()

  for (const { suite, checks } of results) {
    for (const { name, ok, covers } of checks) {
      for (const [status, id] of tagsOf(covers)) {
        const row = rows.get(id)
        if (!STATUSES.includes(status)) {
          problems.push({
            row: id,
            message: `check "${name}" (${suite}) tags row ${id} as "${status}"; a tag is one of ${STATUSES.join(', ')}`,
          })
        } else if (!row) {
          problems.push({
            row: id,
            message: `check "${name}" (${suite}) names matrix row ${id}, which tests/conformance/matrix.json does not have`,
          })
        } else if (row.status !== status) {
          problems.push({
            row: id,
            message:
              `row ${id} is "${row.status}" in the matrix, but check "${name}" (${suite}) asserts it is "${status}". ` +
              'A row and the check that proves it flip together.',
          })
        } else if (ok === true) {
          provenBy.set(id, [...(provenBy.get(id) ?? []), `${suite}: ${name}`])
        }
      }
    }
  }

  const gapRows = new Map()
  for (const gap of gaps.rows) {
    const row = rows.get(gap.id)
    if (gapRows.has(gap.id)) {
      problems.push({ row: gap.id, message: `matrix-gaps.json lists row ${gap.id} twice` })
    } else if (!row) {
      problems.push({ row: gap.id, message: `matrix-gaps.json lists row ${gap.id}, which the matrix does not have` })
    } else if (!gap.issue) {
      problems.push({ row: gap.id, message: `matrix-gaps.json lists row ${gap.id} with no issue tracking it` })
    } else if (gap.status !== row.status) {
      problems.push({
        row: gap.id,
        message:
          `row ${gap.id} is "${row.status}" in the matrix, but matrix-gaps.json records it as "${gap.status}". ` +
          'Flipping a row needs the check that proves the new status.',
      })
    } else if (provenBy.has(gap.id)) {
      problems.push({
        row: gap.id,
        message: `row ${gap.id} now has a passing check (${provenBy.get(gap.id)[0]}); delete its entry from matrix-gaps.json`,
      })
    }
    gapRows.set(gap.id, gap)
  }

  for (const row of matrix.features) {
    if (provenBy.has(row.id) || gapRows.has(row.id)) continue
    const demand = {
      served: 'a passing check that it works',
      unimplemented: 'a passing check that it is refused with an explicit error',
      'documented-divergence': 'a passing check that the documented difference still holds',
    }[row.status]
    problems.push({
      row: row.id,
      message:
        `row ${row.id} ("${row.label}") is "${row.status}" but no smoke check proves it: it needs ${demand}. ` +
        'Tag the check that asserts it, or list the row in tests/smoke-gate/matrix-gaps.json with an issue.',
    })
  }

  return { problems, proven: [...provenBy.keys()].sort(), gaps: [...gapRows.keys()].sort() }
}
