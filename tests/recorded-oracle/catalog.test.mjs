import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import assert from 'node:assert/strict'

import { assertCatalogShape } from './schema.mjs'
import { runners } from './scenarios.mjs'

const here = dirname(fileURLToPath(import.meta.url))

test('catalog cites a public Google doc for every case', () => {
  const catalog = JSON.parse(readFileSync(join(here, 'catalog.json'), 'utf8'))
  assertCatalogShape(catalog)
  for (const entry of catalog.cases) {
    assert.ok(runners[entry.scenario], `${entry.id} scenario ${entry.scenario} is missing`)
  }
})

test('catalog rejects a case with no doc URL', () => {
  assert.throws(() =>
    assertCatalogShape({
      version: 1,
      cases: [
        {
          id: 'bad',
          surface: 'firestore',
          docUrl: '',
          docSentence: 'something long enough to look like a citation here',
          scenario: 'firestoreCreateThenGet',
          assertions: [{ id: 'writeOk', kind: 'boolean' }],
        },
      ],
    }),
  )
})
