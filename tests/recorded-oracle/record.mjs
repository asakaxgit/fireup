#!/usr/bin/env node
// Record doc-cited verdict fixtures against the Spark-plan sandbox.
//
// Credentials come only from the environment (Paperclip secrets). This never
// runs in CI. It refuses if emulator host variables are set — the Firebase
// Emulator Suite is never an oracle.

import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { assertCatalogShape, assertFixtureShape, FIXTURE_VERSION } from './schema.mjs'
import { runners, sdkVersions } from './scenarios.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const fixturesDir = join(here, 'fixtures')

const loadCatalog = () => {
  const catalog = JSON.parse(readFileSync(join(here, 'catalog.json'), 'utf8'))
  assertCatalogShape(catalog)
  return catalog
}

const requireProductionTarget = () => {
  const emulatorVars = [
    'FIRESTORE_EMULATOR_HOST',
    'DATASTORE_EMULATOR_HOST',
    'FIREBASE_AUTH_EMULATOR_HOST',
  ]
  for (const name of emulatorVars) {
    if (process.env[name]) {
      throw new Error(
        `${name} is set; the recorder refuses emulator hosts. The Firebase Emulator Suite is never an oracle.`,
      )
    }
  }
  if (!process.env.GOOGLE_APPLICATION_CREDENTIALS) {
    throw new Error(
      'GOOGLE_APPLICATION_CREDENTIALS is required for production recording (Paperclip secret path).',
    )
  }
  if (!process.env.FIREUP_ORACLE_WEB_API_KEY) {
    throw new Error('FIREUP_ORACLE_WEB_API_KEY is required for Auth cases (Paperclip secret).')
  }
  const firestoreProject = process.env.FIREUP_ORACLE_FIRESTORE_PROJECT || 'fireup-oracle-firestore'
  const datastoreProject = process.env.FIREUP_ORACLE_DATASTORE_PROJECT || 'fireup-oracle'
  return {
    firestoreProject,
    datastoreProject,
    apiKey: process.env.FIREUP_ORACLE_WEB_API_KEY,
  }
}

const expectedFromDoc = (caseEntry) => {
  // Doc-derived expectations used to decide pass vs unsupported. These are the
  // fields the cited sentence talks about — never raw production payloads.
  switch (caseEntry.id) {
    case 'firestore.create-then-get':
      return { writeOk: true, readExists: true, fieldMatches: true }
    case 'firestore.get-missing':
      return { exists: false }
    case 'datastore.insert-then-lookup':
      return { writeOk: true, lookupFound: true, propertyMatches: true }
    case 'auth.signUp-then-signIn':
      return {
        signUpHttpStatus: 200,
        signInHttpStatus: 200,
        signInReturnedLocalId: true,
        signInReturnedIdToken: true,
      }
    case 'auth.signIn-wrong-password':
      return { httpStatus: 400, errorMessageContainsInvalidPassword: true }
    default:
      throw new Error(`no doc expectation for ${caseEntry.id}`)
  }
}

const sameAssertions = (actual, expected) => {
  for (const [key, value] of Object.entries(expected)) {
    if (actual[key] !== value) return false
  }
  return true
}

const runCase = async (caseEntry, target) => {
  if (!caseEntry.docUrl || !caseEntry.docSentence) {
    throw new Error(`refusing case ${caseEntry.id}: no doc citation`)
  }
  const runner = runners[caseEntry.scenario]
  if (!runner) throw new Error(`unknown scenario ${caseEntry.scenario}`)

  const ctx = {
    projectId:
      caseEntry.surface === 'datastore' ? target.datastoreProject : target.firestoreProject,
    apiKey: target.apiKey,
    emulatorHost: undefined,
  }
  return runner(ctx)
}

const main = async () => {
  const catalog = loadCatalog()
  const target = requireProductionTarget()
  mkdirSync(fixturesDir, { recursive: true })
  const today = new Date().toISOString().slice(0, 10)
  const versions = sdkVersions()

  for (const caseEntry of catalog.cases) {
    process.stdout.write(`record ${caseEntry.id}... `)
    const actual = await runCase(caseEntry, target)
    const expected = expectedFromDoc(caseEntry)
    const matched = sameAssertions(actual, expected)
    const fixture = {
      version: FIXTURE_VERSION,
      caseId: caseEntry.id,
      surface: caseEntry.surface,
      docUrl: caseEntry.docUrl,
      docSentence: caseEntry.docSentence,
      recordedAt: today,
      sdkVersions: versions,
      verdict: matched ? 'pass' : 'unsupported',
      productionVerified: true,
      assertions: expected,
      ...(matched
        ? {}
        : {
            unsupportedReason:
              'production diverged from the cited public doc; Fireup must not copy production',
          }),
    }
    // Store the doc-expected assertion values when production matched. When it
    // diverged, still store the doc expectations and mark unsupported — never
    // store production's divergent shape as something to implement.
    assertFixtureShape(fixture, caseEntry)
    const path = join(fixturesDir, `${caseEntry.id}.json`)
    writeFileSync(path, `${JSON.stringify(fixture, null, 2)}\n`)
    console.log(fixture.verdict)
  }
}

main().catch((error) => {
  console.error(error?.stack || error)
  process.exit(1)
})
