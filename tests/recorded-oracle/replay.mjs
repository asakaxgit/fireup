#!/usr/bin/env node
// Offline replay: run the same doc-cited cases against fireup local and compare
// assertion maps to committed fixtures. CI holds no Google credentials.

import { readdirSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { assertCatalogShape, assertFixtureShape } from './schema.mjs'
import { runners } from './scenarios.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const fixturesDir = join(here, 'fixtures')

const loadCatalog = () => {
  const catalog = JSON.parse(readFileSync(join(here, 'catalog.json'), 'utf8'))
  assertCatalogShape(catalog)
  return catalog
}

const loadFixtures = (catalog) => {
  const byId = new Map(catalog.cases.map((entry) => [entry.id, entry]))
  const files = readdirSync(fixturesDir).filter((name) => name.endsWith('.json'))
  if (files.length === 0) {
    throw new Error('no fixtures under tests/recorded-oracle/fixtures')
  }
  const fixtures = []
  for (const name of files) {
    const fixture = JSON.parse(readFileSync(join(fixturesDir, name), 'utf8'))
    const catalogCase = byId.get(fixture.caseId)
    if (!catalogCase) throw new Error(`fixture ${name} has unknown caseId ${fixture.caseId}`)
    assertFixtureShape(fixture, catalogCase)
    fixtures.push({ fixture, catalogCase })
  }
  return fixtures
}

const localTarget = () => {
  const host = process.env.FIREUP_ORACLE_LOCAL_HOST
  if (!host) {
    throw new Error('FIREUP_ORACLE_LOCAL_HOST is required (host:port of fireup local)')
  }
  // Point official clients at fireup local via the documented emulator env vars.
  // This is replay against Fireup, not using the Emulator Suite as an oracle.
  process.env.FIRESTORE_EMULATOR_HOST = host
  process.env.DATASTORE_EMULATOR_HOST = host
  process.env.FIREBASE_AUTH_EMULATOR_HOST = host
  return {
    projectId: process.env.FIREUP_ORACLE_LOCAL_PROJECT || 'fireup-local',
    apiKey: process.env.FIREUP_ORACLE_LOCAL_API_KEY || 'fake-api-key',
    emulatorHost: host,
  }
}

const runCase = async (caseEntry, target) => {
  const runner = runners[caseEntry.scenario]
  if (!runner) throw new Error(`unknown scenario ${caseEntry.scenario}`)
  return runner({
    projectId: target.projectId,
    apiKey: target.apiKey,
    emulatorHost: target.emulatorHost,
  })
}

const main = async () => {
  const catalog = loadCatalog()
  const fixtures = loadFixtures(catalog)
  const target = localTarget()
  let failed = 0

  for (const { fixture, catalogCase } of fixtures) {
    if (fixture.verdict === 'unsupported') {
      console.log(`skip ${fixture.caseId} (unsupported against production/doc)`)
      continue
    }
    process.stdout.write(`replay ${fixture.caseId}... `)
    const actual = await runCase(catalogCase, target)
    const mismatches = []
    for (const [key, expected] of Object.entries(fixture.assertions)) {
      if (actual[key] !== expected) {
        mismatches.push(`${key}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual[key])}`)
      }
    }
    if (mismatches.length > 0) {
      failed += 1
      console.log('FAIL')
      for (const line of mismatches) console.log(`  ${line}`)
    } else {
      console.log('ok')
    }
  }

  if (failed > 0) {
    console.error(`recorded-oracle replay: ${failed} case(s) failed`)
    process.exit(1)
  }
  console.log(`recorded-oracle replay: ${fixtures.length} fixture(s) checked`)
}

main().catch((error) => {
  console.error(error?.stack || error)
  process.exit(1)
})
