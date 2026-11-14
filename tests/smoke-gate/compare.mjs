/**
 * The rule the fidelity gate enforces, kept free of I/O so it can be tested on
 * its own (`compare.test.mjs`).
 *
 * The baseline lists, per suite run, the names of the checks that pass on
 * `main`. A run regresses when a baseline check now fails, or is not reported at
 * all. Absence counts as failure on purpose: renaming a check, deleting it, or a
 * suite that crashes before reaching it must not read as green. Everything else
 * is allowed — a new check, or a check that newly passes, never fails the build.
 */

/**
 * Index result files by suite, refusing input the comparison could be fooled
 * by: two files claiming the same suite, or one check name reported twice
 * (a passing duplicate would hide a failing one).
 */
export const indexResults = (results) => {
  const bySuite = new Map()
  for (const { suite, checks } of results) {
    if (typeof suite !== 'string' || !Array.isArray(checks)) {
      throw new Error(`malformed result: ${JSON.stringify({ suite, checks })}`)
    }
    if (bySuite.has(suite)) {
      throw new Error(`suite ${suite} was reported by more than one result file`)
    }
    const byName = new Map()
    for (const { name, ok } of checks) {
      if (byName.has(name)) {
        throw new Error(`suite ${suite} reported check "${name}" twice; check names must be unique`)
      }
      byName.set(name, ok === true)
    }
    bySuite.set(suite, byName)
  }
  return bySuite
}

export const compareToBaseline = (baseline, results) => {
  // An empty baseline protects nothing and would pass every run, including one
  // where no suite reported at all. That is a broken baseline, not a green one.
  const suites = Object.entries(baseline.suites ?? {})
  if (suites.length === 0 || suites.some(([, names]) => names.length === 0)) {
    throw new Error('the baseline lists no checks for some suite; it would pass a run that checked nothing')
  }

  const bySuite = indexResults(results)
  const regressions = []
  const newlyPassing = []

  for (const [suite, names] of suites) {
    const reported = bySuite.get(suite)
    for (const name of names) {
      if (!reported) {
        regressions.push({ suite, name, reason: 'suite reported no results' })
      } else if (!reported.has(name)) {
        regressions.push({ suite, name, reason: 'check is missing' })
      } else if (!reported.get(name)) {
        regressions.push({ suite, name, reason: 'check failed' })
      }
    }
  }

  for (const [suite, reported] of bySuite) {
    const known = new Set(baseline.suites[suite] ?? [])
    for (const [name, ok] of reported) {
      if (ok && !known.has(name)) newlyPassing.push({ suite, name })
    }
  }

  return { regressions, newlyPassing }
}

/** The baseline a set of results would justify: every passing check, sorted. */
export const baselineFrom = (results) => {
  const suites = {}
  const bySuite = indexResults(results)
  for (const suite of [...bySuite.keys()].sort()) {
    suites[suite] = [...bySuite.get(suite)]
      .filter(([, ok]) => ok)
      .map(([name]) => name)
      .sort()
  }
  return { suites }
}
