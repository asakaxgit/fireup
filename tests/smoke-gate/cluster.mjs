/**
 * The clustering rule behind the triage role's first pass, kept free of I/O so
 * it can be tested on its own (`cluster.test.mjs`). `triage.mjs` is the CLI.
 *
 * A failed check that carries a gRPC status is keyed on the status name and the
 * message with its run-specific values (quoted strings, numbers) blanked, so
 * "namespaces (got \"a\")" and "namespaces (got \"b\")" land together. A check
 * that failed on its own assertion — the server answered, wrongly — is keyed on
 * the check alone: nothing in the result says *why* the answer was wrong, so the
 * triager groups those by reading the code.
 *
 * The same check failing on several stores is one entry with the stores listed;
 * a check that fails on only some of the stores it ran on is flagged
 * `storeSpecific`, because that is a store bug rather than a surface bug.
 */

const GRPC_CODES = [
  'OK', 'CANCELLED', 'UNKNOWN', 'INVALID_ARGUMENT', 'DEADLINE_EXCEEDED', 'NOT_FOUND',
  'ALREADY_EXISTS', 'PERMISSION_DENIED', 'RESOURCE_EXHAUSTED', 'FAILED_PRECONDITION',
  'ABORTED', 'OUT_OF_RANGE', 'UNIMPLEMENTED', 'INTERNAL', 'UNAVAILABLE', 'DATA_LOSS',
  'UNAUTHENTICATED',
]

/** `12 UNIMPLEMENTED: …` is how google-gax prefixes a status; drop it. */
const stripStatusPrefix = (message) => message.replace(/^\d+ [A-Z_]+: /, '')

const normalise = (message) =>
  stripStatusPrefix(message)
    .replace(/"[^"]*"/g, '"…"')
    .replace(/\b\d+\b/g, 'N')
    .trim()

export const signatureOf = ({ name, error }) => {
  const code = error?.code
  if (typeof code === 'number' && GRPC_CODES[code]) {
    return `${GRPC_CODES[code]}: ${normalise(error.message ?? '')}`
  }
  return `wrong result: ${name}`
}

/** Failed checks, one per (suite family, check), with the stores it failed on. */
export const failuresFrom = (results) => {
  const byCheck = new Map()
  const ranOn = new Map()
  for (const { suite, checks } of results) {
    const [family, store = ''] = suite.split('/')
    if (!ranOn.has(family)) ranOn.set(family, new Set())
    ranOn.get(family).add(store)
    for (const check of checks) {
      if (check.ok) continue
      const id = `${family}\u0000${check.name}`
      if (!byCheck.has(id)) {
        byCheck.set(id, {
          family,
          name: check.name,
          stores: [],
          signature: signatureOf(check),
          message: stripStatusPrefix(check.error?.message ?? '(no error recorded)'),
        })
      }
      byCheck.get(id).stores.push(store)
    }
  }
  return [...byCheck.values()].map((failure) => ({
    ...failure,
    storeSpecific: failure.stores.length < ranOn.get(failure.family).size,
  }))
}

export const clusterFailures = (failures) => {
  const clusters = new Map()
  for (const failure of failures) {
    const key = `${failure.family}\u0000${failure.signature}`
    if (!clusters.has(key)) {
      clusters.set(key, { family: failure.family, signature: failure.signature, checks: [] })
    }
    clusters.get(key).checks.push(failure)
  }
  return [...clusters.values()].sort(
    (a, b) => b.checks.length - a.checks.length || a.signature.localeCompare(b.signature),
  )
}
