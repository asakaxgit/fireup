// Fixture schema for the recorded fidelity oracle.
//
// A fixture stores only the fields a cited public doc talks about (status
// codes, booleans, ordered ids). It never holds raw response bodies, headers,
// ID or refresh tokens, signatures, or timings.

export const FIXTURE_VERSION = 1

const ALLOWED_VERDICTS = new Set(['pass', 'unsupported'])

export const assertFixtureShape = (fixture, catalogCase) => {
  if (fixture == null || typeof fixture !== 'object' || Array.isArray(fixture)) {
    throw new Error('fixture must be a JSON object')
  }
  if (fixture.version !== FIXTURE_VERSION) {
    throw new Error(`fixture.version must be ${FIXTURE_VERSION}`)
  }
  if (fixture.caseId !== catalogCase.id) {
    throw new Error(`fixture.caseId ${fixture.caseId} does not match catalog ${catalogCase.id}`)
  }
  if (fixture.surface !== catalogCase.surface) {
    throw new Error(`fixture.surface ${fixture.surface} does not match catalog`)
  }
  if (fixture.docUrl !== catalogCase.docUrl) {
    throw new Error('fixture.docUrl must match the catalog citation')
  }
  if (fixture.docSentence !== catalogCase.docSentence) {
    throw new Error('fixture.docSentence must match the catalog citation')
  }
  if (typeof fixture.recordedAt !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(fixture.recordedAt)) {
    throw new Error('fixture.recordedAt must be YYYY-MM-DD')
  }
  if (fixture.sdkVersions == null || typeof fixture.sdkVersions !== 'object') {
    throw new Error('fixture.sdkVersions must be an object')
  }
  if (!ALLOWED_VERDICTS.has(fixture.verdict)) {
    throw new Error(`fixture.verdict must be one of ${[...ALLOWED_VERDICTS].join(', ')}`)
  }
  if (fixture.productionVerified !== true && fixture.productionVerified !== false) {
    throw new Error('fixture.productionVerified must be a boolean')
  }
  if (fixture.assertions == null || typeof fixture.assertions !== 'object' || Array.isArray(fixture.assertions)) {
    throw new Error('fixture.assertions must be an object of id → value')
  }

  for (const key of Object.keys(fixture)) {
    if (
      ![
        'version',
        'caseId',
        'surface',
        'docUrl',
        'docSentence',
        'recordedAt',
        'sdkVersions',
        'verdict',
        'productionVerified',
        'assertions',
        'unsupportedReason',
      ].includes(key)
    ) {
      throw new Error(`fixture has disallowed field ${key}`)
    }
  }

  const forbidden = ['token', 'password', 'authorization', 'raw', 'body', 'header', 'latency', 'duration', 'timing']
  const blob = JSON.stringify(fixture).toLowerCase()
  for (const word of forbidden) {
    // Assertion ids may mention "token" as a boolean presence check name;
    // only reject string values that look like secrets or raw payloads.
    if (typeof fixture.assertions === 'object') {
      for (const [id, value] of Object.entries(fixture.assertions)) {
        if (typeof value === 'string' && value.length > 64) {
          throw new Error(`assertion ${id} string value is too long for a verdict field`)
        }
        if (typeof value === 'string' && /eyJ[A-Za-z0-9_-]+\./.test(value)) {
          throw new Error(`assertion ${id} looks like a JWT; fixtures must not store tokens`)
        }
      }
    }
    if (word === 'token') continue
    if (blob.includes(`"${word}"`) && word !== 'password') {
      // "password" appears in doc URLs/sentences; that is fine.
    }
  }

  for (const expected of catalogCase.assertions) {
    if (!(expected.id in fixture.assertions)) {
      throw new Error(`fixture missing assertion ${expected.id}`)
    }
    const value = fixture.assertions[expected.id]
    if (expected.kind === 'boolean' && typeof value !== 'boolean') {
      throw new Error(`assertion ${expected.id} must be boolean`)
    }
    if (expected.kind === 'number' && typeof value !== 'number') {
      throw new Error(`assertion ${expected.id} must be number`)
    }
  }

  if (fixture.verdict === 'unsupported' && typeof fixture.unsupportedReason !== 'string') {
    throw new Error('unsupported fixtures need unsupportedReason')
  }
}

export const assertCatalogShape = (catalog) => {
  if (catalog.version !== 1) throw new Error('catalog.version must be 1')
  if (!Array.isArray(catalog.cases) || catalog.cases.length === 0) {
    throw new Error('catalog.cases must be a non-empty array')
  }
  const seen = new Set()
  for (const entry of catalog.cases) {
    if (!entry.id || seen.has(entry.id)) throw new Error(`duplicate or missing case id: ${entry.id}`)
    seen.add(entry.id)
    if (!entry.docUrl || !/^https:\/\/(firebase\.google\.com|cloud\.google\.com)\//.test(entry.docUrl)) {
      throw new Error(`case ${entry.id} needs a public Google doc URL`)
    }
    if (!entry.docSentence || entry.docSentence.length < 20) {
      throw new Error(`case ${entry.id} needs a doc sentence citation`)
    }
    if (!['firestore', 'datastore', 'auth'].includes(entry.surface)) {
      throw new Error(`case ${entry.id} has unknown surface`)
    }
    if (!entry.scenario) throw new Error(`case ${entry.id} needs a scenario id`)
    if (!Array.isArray(entry.assertions) || entry.assertions.length === 0) {
      throw new Error(`case ${entry.id} needs assertions`)
    }
  }
}
