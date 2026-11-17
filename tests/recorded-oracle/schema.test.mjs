import { readdirSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import assert from 'node:assert/strict'

import { assertFixtureShape } from './schema.mjs'

const here = dirname(fileURLToPath(import.meta.url))

test('every committed fixture matches the catalog and forbids tokens', () => {
  const catalog = JSON.parse(readFileSync(join(here, 'catalog.json'), 'utf8'))
  const byId = new Map(catalog.cases.map((entry) => [entry.id, entry]))
  const files = readdirSync(join(here, 'fixtures')).filter((name) => name.endsWith('.json'))
  assert.ok(files.length > 0, 'fixtures directory is empty')
  for (const name of files) {
    const fixture = JSON.parse(readFileSync(join(here, 'fixtures', name), 'utf8'))
    const catalogCase = byId.get(fixture.caseId)
    assert.ok(catalogCase, `unknown case ${fixture.caseId}`)
    assertFixtureShape(fixture, catalogCase)
    const blob = JSON.stringify(fixture)
    assert.equal(/eyJ[A-Za-z0-9_-]+\./.test(blob), false, `${name} must not contain a JWT`)
  }
})

test('fixture schema rejects a raw token field', () => {
  const catalogCase = {
    id: 'firestore.get-missing',
    surface: 'firestore',
    docUrl: 'https://firebase.google.com/docs/firestore/query-data/get-data',
    docSentence: 'Note: If there is no document at the location referenced by docRef, the resulting document will be empty, and calling exists on it will return false.',
    assertions: [{ id: 'exists', kind: 'boolean' }],
  }
  assert.throws(() =>
    assertFixtureShape(
      {
        version: 1,
        caseId: 'firestore.get-missing',
        surface: 'firestore',
        docUrl: catalogCase.docUrl,
        docSentence: catalogCase.docSentence,
        recordedAt: '2026-11-16',
        sdkVersions: {},
        verdict: 'pass',
        productionVerified: false,
        assertions: { exists: false },
        raw: { token: 'secret' },
      },
      catalogCase,
    ),
  )
})
