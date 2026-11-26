// Scenario runners for the recorded oracle.
//
// Each runner returns only the assertion map named in catalog.json. It never
// returns tokens, raw bodies, timings, or credentials. Production recording
// and offline replay share the same runners; only the target (sandbox vs
// fireup local) changes.

import { randomBytes } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const pkg = () => JSON.parse(readFileSync(join(here, 'package.json'), 'utf8'))

const syntheticId = (prefix) => `${prefix}-${randomBytes(6).toString('hex')}`

const readCredentials = (envName = 'GOOGLE_APPLICATION_CREDENTIALS') => {
  const path = process.env[envName] || process.env.GOOGLE_APPLICATION_CREDENTIALS
  if (!path) return undefined
  return JSON.parse(readFileSync(path, 'utf8'))
}

const firestoreClient = async (projectId) => {
  const { Firestore } = await import('@google-cloud/firestore')
  // Prefer the Firestore SA when the board bound separate project secrets.
  const credentials = readCredentials('FIREUP_ORACLE_FIRESTORE_CREDENTIALS')
  return new Firestore({
    projectId,
    ...(credentials ? { credentials } : {}),
  })
}

const datastoreClient = async (projectId) => {
  const { Datastore } = await import('@google-cloud/datastore')
  const credentials = readCredentials('FIREUP_ORACLE_DATASTORE_CREDENTIALS')
  return new Datastore({
    projectId,
    ...(credentials ? { credentials } : {}),
  })
}

const authRest = async ({ apiKey, path, body, emulatorHost }) => {
  const base = emulatorHost
    ? `http://${emulatorHost}/identitytoolkit.googleapis.com/v1`
    : 'https://identitytoolkit.googleapis.com/v1'
  const url = `${base}/${path}?key=${encodeURIComponent(apiKey)}`
  const response = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
  const text = await response.text()
  let json = null
  try {
    json = JSON.parse(text)
  } catch {
    json = null
  }
  return { status: response.status, json }
}

export const sdkVersions = () => {
  const versions = pkg().dependencies
  return {
    '@google-cloud/firestore': versions['@google-cloud/firestore'],
    '@google-cloud/datastore': versions['@google-cloud/datastore'],
    firebase: versions.firebase,
    'firebase-admin': versions['firebase-admin'],
  }
}

export const firestoreCreateThenGet = async ({ projectId }) => {
  const db = await firestoreClient(projectId)
  const id = syntheticId('oracle-doc')
  const ref = db.collection('oracle_smoke').doc(id)
  const marker = `marker-${id}`
  try {
    await ref.set({ marker, source: 'fireup-recorded-oracle' })
    const snap = await ref.get()
    return {
      writeOk: true,
      readExists: snap.exists === true,
      fieldMatches: snap.exists && snap.get('marker') === marker,
    }
  } finally {
    await ref.delete().catch(() => {})
    await db.terminate?.()
  }
}

export const firestoreGetMissing = async ({ projectId }) => {
  const db = await firestoreClient(projectId)
  const id = syntheticId('oracle-missing')
  try {
    const snap = await db.collection('oracle_smoke').doc(id).get()
    return { exists: snap.exists === true }
  } finally {
    await db.terminate?.()
  }
}

export const datastoreInsertThenLookup = async ({ projectId }) => {
  const datastore = await datastoreClient(projectId)
  const id = syntheticId('oracle-entity')
  const key = datastore.key(['OracleSmoke', id])
  const marker = `marker-${id}`
  try {
    await datastore.save({ key, data: { marker, source: 'fireup-recorded-oracle' } })
    const [entity] = await datastore.get(key)
    return {
      writeOk: true,
      lookupFound: entity != null,
      propertyMatches: entity != null && entity.marker === marker,
    }
  } finally {
    await datastore.delete(key).catch(() => {})
  }
}

export const authSignUpThenSignIn = async ({ apiKey, emulatorHost }) => {
  const email = `oracle-${randomBytes(4).toString('hex')}@example.com`
  const password = `Pass-${randomBytes(6).toString('hex')}!`
  const signUp = await authRest({
    apiKey,
    emulatorHost,
    path: 'accounts:signUp',
    body: { email, password, returnSecureToken: true },
  })
  const signIn = await authRest({
    apiKey,
    emulatorHost,
    path: 'accounts:signInWithPassword',
    body: { email, password, returnSecureToken: true },
  })
  // Delete via Identity Toolkit deleteAccount when we have a token; only the
  // boolean "token was present" is recorded — never the token itself.
  const idToken = signIn.json?.idToken
  if (typeof idToken === 'string' && idToken.length > 0) {
    await authRest({
      apiKey,
      emulatorHost,
      path: 'accounts:delete',
      body: { idToken },
    }).catch(() => {})
  }
  return {
    signUpHttpStatus: signUp.status,
    signInHttpStatus: signIn.status,
    signInReturnedLocalId: typeof signIn.json?.localId === 'string' && signIn.json.localId.length > 0,
    signInReturnedIdToken: typeof idToken === 'string' && idToken.length > 0,
  }
}

export const authSignInWrongPassword = async ({ apiKey, emulatorHost }) => {
  const email = `oracle-${randomBytes(4).toString('hex')}@example.com`
  const password = `Pass-${randomBytes(6).toString('hex')}!`
  const signUp = await authRest({
    apiKey,
    emulatorHost,
    path: 'accounts:signUp',
    body: { email, password, returnSecureToken: true },
  })
  const bad = await authRest({
    apiKey,
    emulatorHost,
    path: 'accounts:signInWithPassword',
    body: { email, password: 'wrong-password', returnSecureToken: true },
  })
  const idToken = signUp.json?.idToken
  if (typeof idToken === 'string' && idToken.length > 0) {
    await authRest({
      apiKey,
      emulatorHost,
      path: 'accounts:delete',
      body: { idToken },
    }).catch(() => {})
  }
  const message = String(bad.json?.error?.message ?? bad.json?.error ?? '')
  return {
    httpStatus: bad.status,
    errorMessageContainsInvalidPassword: /INVALID_PASSWORD|INVALID_LOGIN_CREDENTIALS|EMAIL_NOT_FOUND/i.test(
      message,
    ),
  }
}

export const runners = {
  firestoreCreateThenGet,
  firestoreGetMissing,
  datastoreInsertThenLookup,
  authSignUpThenSignIn,
  authSignInWrongPassword,
}
