// SDK handshake smoke test for the `fireup local` auth surface.
//
// The Phase L counterpart to smoke.mjs. Fireup's own tests assert that the
// Identity Toolkit REST surface matches the published contract; this asserts
// that Google's *official* clients agree, which is a different claim and the
// only one a developer actually cares about:
//
//   * firebase-admin (Apache-2.0) over FIREBASE_AUTH_EMULATOR_HOST — the
//     privileged surface: createUser, getUser, updateUser, listUsers, custom
//     claims, custom tokens, verifyIdToken.
//   * firebase (the Apache-2.0 client JS SDK) over connectAuthEmulator() — the
//     end-user surface: signInWithEmailAndPassword, signInWithCustomToken,
//     token refresh.
//
// Neither SDK is given a Fireup-internal API, a hand-built request, or a hint
// about how Fireup stores anything. Every URL, header, body shape, and error
// mapping below is chosen by the SDK. If our server disagrees, this fails.
//
// Run it via scripts/smoke-local-auth.sh, which starts and stops the server.
// Assumes FIREBASE_AUTH_EMULATOR_HOST is already exported.

import { strict as assert } from 'node:assert'

import { initializeApp as initializeAdminApp, deleteApp as deleteAdminApp } from 'firebase-admin/app'
import { getAuth as getAdminAuth } from 'firebase-admin/auth'
import { initializeApp as initializeClientApp, deleteApp as deleteClientApp } from 'firebase/app'
import {
  applyActionCode,
  confirmPasswordReset,
  connectAuthEmulator,
  deleteUser,
  fetchSignInMethodsForEmail,
  getAuth as getClientAuth,
  isSignInWithEmailLink,
  sendEmailVerification,
  sendPasswordResetEmail,
  sendSignInLinkToEmail,
  signInAnonymously,
  signInWithCustomToken,
  signInWithEmailAndPassword,
  signInWithEmailLink,
  verifyBeforeUpdateEmail,
  verifyPasswordResetCode,
} from 'firebase/auth'

import { writeResults } from '../smoke-gate/results.mjs'

const PROJECT_ID = 'fireup-smoke'

// The client SDK requires an apiKey in its config and appends it to every
// request as ?key=. Against an emulator host it is never validated — the
// documented local convention is that any non-empty string works. If Fireup
// ever started checking it, that would be a finding, so it is deliberately
// obvious nonsense rather than something that looks real.
const FAKE_API_KEY = 'fireup-local-no-api-key-needed'

const checks = []

// `covers` names the fidelity-matrix rows the check asserts, keyed by the
// status it asserts them with; see tests/smoke-gate/matrix-coverage.mjs.
const check = async (name, fn, covers) => {
  try {
    await fn()
    checks.push({ name, ok: true, covers })
    console.log(`  ok   ${name}`)
  } catch (error) {
    checks.push({ name, ok: false, covers, error })
    console.log(`  FAIL ${name}`)
    console.log(`       ${error?.message ?? error}`)
  }
  writeResults(checks)
}

/// Assert a call fails with the Firebase error code the SDK assigns, not with
/// whatever string our server happened to send. The SDK's own mapping from the
/// REST envelope to `error.code` is part of what is under test.
const expectAuthCode = async (expected, fn) => {
  try {
    await fn()
  } catch (error) {
    assert.equal(
      error.code,
      expected,
      `expected SDK error code ${expected}, got ${error.code}: ${error.message}`,
    )
    return
  }
  assert.fail(`expected the call to fail with ${expected}, but it succeeded`)
}

/// Decode a JWT payload without verifying it. Used only to inspect the claims
/// the server minted; the *verification* assertions all go through the SDK's
/// own verifyIdToken.
const decodeClaims = (jwt) => {
  const [, payload] = jwt.split('.')
  return JSON.parse(Buffer.from(payload, 'base64url').toString('utf8'))
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

const main = async () => {
  const host = process.env.FIREBASE_AUTH_EMULATOR_HOST
  assert.ok(host, 'FIREBASE_AUTH_EMULATOR_HOST must be set')
  console.log(`Driving fireup local at ${host} with firebase-admin and the firebase client SDK\n`)

  // No credential is passed on purpose. In emulator mode firebase-admin skips
  // credential resolution entirely and authenticates as the bearer token
  // "owner", so needing a service account here would itself be a divergence.
  const adminApp = initializeAdminApp({ projectId: PROJECT_ID }, 'fireup-smoke-admin')
  const adminAuth = getAdminAuth(adminApp)

  const clientApp = initializeClientApp(
    { apiKey: FAKE_API_KEY, projectId: PROJECT_ID },
    'fireup-smoke-client',
  )
  const clientAuth = getClientAuth(clientApp)
  connectAuthEmulator(clientAuth, `http://${host}`, { disableWarnings: true })

  // ----- firebase-admin: the privileged account surface --------------------

  let created
  await check('admin createUser() creates an account with the fields it was given', async () => {
    created = await adminAuth.createUser({
      uid: 'smoke-alice',
      email: 'alice@example.com',
      emailVerified: false,
      password: 'correct-horse',
      displayName: 'Alice',
    })
    assert.equal(created.uid, 'smoke-alice')
    assert.equal(created.email, 'alice@example.com')
    assert.equal(created.displayName, 'Alice')
    assert.equal(created.emailVerified, false)
    assert.equal(created.disabled, false)
    assert.ok(created.metadata.creationTime, 'server must stamp a creation time')
  }, { served: ['auth.admin-create'] })

  await check('admin createUser() with a duplicate email fails with auth/email-already-exists', async () => {
    await expectAuthCode('auth/email-already-exists', () =>
      adminAuth.createUser({ email: 'alice@example.com', password: 'another-one' }),
    )
  }, { served: ['auth.admin-create'] })

  await check('admin createUser() assigns a uid when none is given', async () => {
    const user = await adminAuth.createUser({ email: 'auto@example.com', password: 'hunter2hunter2' })
    assert.ok(user.uid.length > 0, 'server must assign a uid')
    assert.equal(user.email, 'auto@example.com')
  }, { served: ['auth.admin-create'] })

  await check('admin getUser() reads the account back by uid', async () => {
    const user = await adminAuth.getUser('smoke-alice')
    assert.equal(user.email, 'alice@example.com')
    assert.equal(user.displayName, 'Alice')
    // The SDK derives providerData from the REST providerUserInfo array; an
    // empty one here would mean the server is not reporting how the account
    // signs in.
    assert.deepEqual(
      user.providerData.map((p) => p.providerId),
      ['password'],
    )
  }, { served: ['auth.admin-lookup'] })

  await check('admin getUser() on a missing uid fails with auth/user-not-found', async () => {
    await expectAuthCode('auth/user-not-found', () => adminAuth.getUser('nobody-at-all'))
  }, { served: ['auth.admin-lookup'] })

  await check('admin getUserByEmail() reads the account back by email', async () => {
    const user = await adminAuth.getUserByEmail('alice@example.com')
    assert.equal(user.uid, 'smoke-alice')
  }, { served: ['auth.admin-lookup'] })

  await check('admin getUserByEmail() on a missing email fails with auth/user-not-found', async () => {
    await expectAuthCode('auth/user-not-found', () => adminAuth.getUserByEmail('ghost@example.com'))
  }, { served: ['auth.admin-lookup'] })

  await check('admin updateUser() changes the fields it was given and leaves the rest', async () => {
    const user = await adminAuth.updateUser('smoke-alice', {
      displayName: 'Alice Updated',
      emailVerified: true,
    })
    assert.equal(user.displayName, 'Alice Updated')
    assert.equal(user.emailVerified, true)
    assert.equal(user.email, 'alice@example.com', 'update must not drop untouched fields')
  }, { served: ['auth.admin-update'] })

  await check('admin updateUser() can clear a field by passing null', async () => {
    const user = await adminAuth.updateUser('smoke-alice', { photoURL: null, displayName: null })
    assert.equal(user.displayName, undefined)
    assert.equal(user.photoURL, undefined)

    // Put the display name back; later checks read it.
    await adminAuth.updateUser('smoke-alice', { displayName: 'Alice Updated' })
  }, { served: ['auth.admin-update'] })

  await check('admin updateUser() can disable and re-enable an account', async () => {
    const disabled = await adminAuth.updateUser('smoke-alice', { disabled: true })
    assert.equal(disabled.disabled, true)

    const enabled = await adminAuth.updateUser('smoke-alice', { disabled: false })
    assert.equal(enabled.disabled, false)
  }, { served: ['auth.admin-update'] })

  await check('admin setCustomUserClaims() stores claims and getUser() reports them', async () => {
    await adminAuth.setCustomUserClaims('smoke-alice', { role: 'admin', tier: 3 })

    const user = await adminAuth.getUser('smoke-alice')
    assert.deepEqual(user.customClaims, { role: 'admin', tier: 3 })
  }, { served: ['auth.admin-update'] })

  await check('admin setCustomUserClaims(null) clears claims', async () => {
    try {
      // The SDK wire-encodes `null` as the string "{}", so this also pins down
      // that the server treats an empty claim set as no claims at all: the
      // documented value of UserRecord.customClaims for an account without
      // claims is undefined, not {}.
      await adminAuth.setCustomUserClaims('smoke-alice', null)
      const cleared = await adminAuth.getUser('smoke-alice')
      assert.equal(cleared.customClaims, undefined)
    } finally {
      // Restore them whatever happened above; the custom-token, verifyIdToken,
      // and refresh checks all read these, and leaving them cleared would turn
      // one failure into five.
      await adminAuth.setCustomUserClaims('smoke-alice', { role: 'admin', tier: 3 })
    }
  }, { served: ['auth.admin-update'] })

  await check('admin listUsers() pages through every account', async () => {
    // Enough accounts that a page size of 2 has to page more than once, so
    // this tests paging rather than just "everything fits in one response".
    const extra = []
    for (let i = 0; i < 4; i += 1) {
      extra.push(await adminAuth.createUser({ uid: `smoke-page-${i}` }))
    }

    try {
      const seen = new Set()
      let pageToken
      let pages = 0
      do {
        const page = await adminAuth.listUsers(2, pageToken)
        pages += 1
        for (const user of page.users) {
          assert.ok(!seen.has(user.uid), `listUsers returned ${user.uid} twice`)
          seen.add(user.uid)
        }
        assert.ok(page.users.length <= 2, 'server must honour the requested page size')
        pageToken = page.pageToken
        assert.ok(pages < 20, 'listUsers paging did not terminate')
      } while (pageToken)

      assert.ok(pages > 1, 'a page size of 2 over 6 accounts must take more than one page')
      assert.ok(seen.has('smoke-alice'), 'listUsers must include the account we created')
      for (const user of extra) {
        assert.ok(seen.has(user.uid), `listUsers dropped ${user.uid} while paging`)
      }
    } finally {
      for (const user of extra) {
        await adminAuth.deleteUser(user.uid)
      }
    }
  }, { served: ['auth.admin-batchGet'] })

  // ----- custom tokens: admin mints, client redeems ------------------------

  let customToken
  await check('admin createCustomToken() mints a token the emulator convention accepts', async () => {
    customToken = await adminAuth.createCustomToken('smoke-alice', { from: 'custom-token' })
    const claims = decodeClaims(customToken)
    assert.equal(claims.uid, 'smoke-alice')
    assert.deepEqual(claims.claims, { from: 'custom-token' })
    // The SDK, not Fireup, picks this audience; our server has to recognise it.
    assert.equal(
      claims.aud,
      'https://identitytoolkit.googleapis.com/google.identity.identitytoolkit.v1.IdentityToolkit',
    )
  })

  await check('client signInWithCustomToken() exchanges it for a session', async () => {
    const credential = await signInWithCustomToken(clientAuth, customToken)
    assert.equal(credential.user.uid, 'smoke-alice')

    const idToken = await credential.user.getIdToken()
    const claims = decodeClaims(idToken)
    assert.equal(claims.sub, 'smoke-alice')
    assert.equal(claims.from, 'custom-token', 'custom-token claims must reach the ID token')
    assert.equal(claims.role, 'admin', 'account custom claims must reach the ID token too')
    assert.equal(claims.firebase.sign_in_provider, 'custom')
  }, { served: ['auth.signInWithCustomToken'] })

  await check('client signInWithCustomToken() re-creates a uid that no longer exists', async () => {
    const doomed = await adminAuth.createUser({ uid: 'smoke-doomed' })
    const token = await adminAuth.createCustomToken(doomed.uid)
    await adminAuth.deleteUser(doomed.uid)

    // Custom-token sign-in creating the uid it names is the documented
    // behaviour, and it is what makes createCustomToken() usable for a uid that
    // only exists in someone else's user database.
    const credential = await signInWithCustomToken(clientAuth, token)
    assert.equal(credential.user.uid, 'smoke-doomed')
    assert.equal((await adminAuth.getUser('smoke-doomed')).uid, 'smoke-doomed')

    await adminAuth.deleteUser('smoke-doomed')
  }, { served: ['auth.signInWithCustomToken'] })

  // ----- client SDK: the end-user surface ----------------------------------

  let signedIn
  await check('client signInWithEmailAndPassword() signs the account in', async () => {
    signedIn = await signInWithEmailAndPassword(clientAuth, 'alice@example.com', 'correct-horse')
    assert.equal(signedIn.user.uid, 'smoke-alice')
    assert.equal(signedIn.user.email, 'alice@example.com')
    assert.equal(signedIn.user.emailVerified, true)
    assert.ok(signedIn.user.refreshToken, 'server must return a refresh token')
  }, { served: ['auth.signInWithPassword'] })

  // The two checks below pin the server to the classic per-reason codes
  // (INVALID_PASSWORD / EMAIL_NOT_FOUND), which the SDK maps to
  // auth/wrong-password and auth/user-not-found. A project with email
  // enumeration protection on returns the single opaque
  // INVALID_LOGIN_CREDENTIALS instead; Fireup does not implement that mode, so
  // these assert what it actually promises rather than the other dialect.
  await check('client signInWithEmailAndPassword() rejects a wrong password', async () => {
    await expectAuthCode('auth/wrong-password', () =>
      signInWithEmailAndPassword(clientAuth, 'alice@example.com', 'not-the-password'),
    )
  }, { served: ['auth.signInWithPassword'] })

  await check('client signInWithEmailAndPassword() rejects an unknown email', async () => {
    await expectAuthCode('auth/user-not-found', () =>
      signInWithEmailAndPassword(clientAuth, 'ghost@example.com', 'correct-horse'),
    )
  }, { served: ['auth.signInWithPassword'] })

  await check('client signInWithEmailAndPassword() rejects a disabled account', async () => {
    await adminAuth.updateUser('smoke-alice', { disabled: true })
    try {
      await expectAuthCode('auth/user-disabled', () =>
        signInWithEmailAndPassword(clientAuth, 'alice@example.com', 'correct-horse'),
      )
    } finally {
      await adminAuth.updateUser('smoke-alice', { disabled: false })
    }
  }, { served: ['auth.signInWithPassword'] })

  await check('client signInAnonymously() creates an anonymous account', async () => {
    const credential = await signInAnonymously(clientAuth)
    assert.ok(credential.user.uid.length > 0)
    assert.equal(credential.user.isAnonymous, true)
    assert.equal(credential.user.email, null)

    const claims = decodeClaims(await credential.user.getIdToken())
    assert.equal(claims.firebase.sign_in_provider, 'anonymous')
  }, { served: ['auth.signUp'] })

  // ----- verifyIdToken over the unsigned emulator token --------------------

  await check('admin verifyIdToken() accepts the unsigned token the client holds', async () => {
    signedIn = await signInWithEmailAndPassword(clientAuth, 'alice@example.com', 'correct-horse')
    const idToken = await signedIn.user.getIdToken()

    // The header has to be exactly what the emulator convention specifies, or
    // the Admin SDK's emulator verifier will not take it.
    const [header] = idToken.split('.')
    assert.deepEqual(JSON.parse(Buffer.from(header, 'base64url').toString('utf8')), {
      alg: 'none',
      typ: 'JWT',
    })
    assert.equal(idToken.split('.')[2], '', 'the emulator convention is an empty signature')

    const decoded = await adminAuth.verifyIdToken(idToken)
    assert.equal(decoded.uid, 'smoke-alice')
    assert.equal(decoded.sub, 'smoke-alice')
    assert.equal(decoded.aud, PROJECT_ID)
    assert.equal(decoded.iss, `https://securetoken.google.com/${PROJECT_ID}`)
    assert.equal(decoded.role, 'admin', 'custom claims must survive verification')
    assert.equal(decoded.firebase.sign_in_provider, 'password')
    assert.ok(decoded.auth_time > 0, 'auth_time is required by the verifier')
    assert.ok(decoded.exp > decoded.iat, 'exp must be after iat')
  })

  await check('admin verifyIdToken() rejects a tampered token', async () => {
    const idToken = await signedIn.user.getIdToken()
    const [header, payload] = idToken.split('.')
    const forged = JSON.parse(Buffer.from(payload, 'base64url').toString('utf8'))
    forged.sub = 'somebody-else'
    forged.aud = 'a-different-project'
    const tampered = `${header}.${Buffer.from(JSON.stringify(forged)).toString('base64url')}.`

    // Nothing here reaches Fireup: this is the Admin SDK's own claim check, and
    // it is the reason an unsigned token is not a free-for-all. Asserting it
    // keeps us honest about what the emulator convention does and does not buy.
    await assert.rejects(() => adminAuth.verifyIdToken(tampered), /incorrect "aud"/)
  })

  await check('admin verifyIdToken() rejects a token the server never minted', async () => {
    await assert.rejects(() => adminAuth.verifyIdToken('not.a.jwt'))
  })

  // ----- refresh -----------------------------------------------------------

  await check('client getIdToken(true) refreshes through the secure-token endpoint', async () => {
    signedIn = await signInWithEmailAndPassword(clientAuth, 'alice@example.com', 'correct-horse')
    const before = await signedIn.user.getIdToken()
    const beforeClaims = decodeClaims(before)

    // `exp` has one-second resolution, so a refresh issued in the same second
    // as the original would be indistinguishable from a cached token. Waiting
    // makes "the session was extended" an assertion rather than a hope.
    await sleep(1_100)

    const after = await signedIn.user.getIdToken(true)
    assert.notEqual(after, before, 'a forced refresh must return a new token')

    const afterClaims = decodeClaims(after)
    assert.equal(afterClaims.sub, beforeClaims.sub)
    assert.ok(
      afterClaims.exp > beforeClaims.exp,
      `refresh must extend the session: exp ${beforeClaims.exp} -> ${afterClaims.exp}`,
    )
    assert.equal(afterClaims.role, 'admin', 'custom claims must survive a refresh')

    // The refreshed token has to be acceptable to the Admin SDK too, not just
    // parseable by us.
    const decoded = await adminAuth.verifyIdToken(after)
    assert.equal(decoded.uid, 'smoke-alice')
  }, { served: ['auth.token'] })

  await check('a revoked refresh token stops working', async () => {
    const credential = await signInWithEmailAndPassword(clientAuth, 'alice@example.com', 'correct-horse')
    await credential.user.getIdToken()

    // Changing the password is the documented way an account invalidates its
    // outstanding sessions. The client still holds the old refresh token.
    await adminAuth.updateUser('smoke-alice', { password: 'a-brand-new-password' })

    await assert.rejects(
      () => credential.user.getIdToken(true),
      (error) => {
        assert.ok(
          typeof error.code === 'string' && error.code.startsWith('auth/'),
          `expected a mapped auth/* error, got ${error.code}: ${error.message}`,
        )
        return true
      },
      'a refresh token issued before a password change must not mint new ID tokens',
    )

    // Leave the password where the remaining checks expect it.
    await adminAuth.updateUser('smoke-alice', { password: 'correct-horse' })
  }, { served: ['auth.token'] })

  // ----- deletion ----------------------------------------------------------

  await check('admin deleteUser() removes the account', async () => {
    await adminAuth.createUser({ uid: 'smoke-temp', email: 'temp@example.com' })
    await adminAuth.deleteUser('smoke-temp')
    await expectAuthCode('auth/user-not-found', () => adminAuth.getUser('smoke-temp'))
  }, { served: ['auth.admin-delete'] })

  await check('admin deleteUser() on a missing uid fails with auth/user-not-found', async () => {
    await expectAuthCode('auth/user-not-found', () => adminAuth.deleteUser('never-existed'))
  }, { served: ['auth.admin-delete'] })

  await check('client deleteUser() removes the signed-in account', async () => {
    const user = await adminAuth.createUser({ email: 'self-delete@example.com', password: 'correct-horse' })
    try {
      const credential = await signInWithEmailAndPassword(clientAuth, 'self-delete@example.com', 'correct-horse')
      // The client SDK sends this with the session's idToken; there is no uid
      // in the request, so the server has to find the account from the token.
      await deleteUser(credential.user)

      await expectAuthCode('auth/user-not-found', () => adminAuth.getUser(user.uid))
      await expectAuthCode('auth/user-not-found', () =>
        signInWithEmailAndPassword(clientAuth, 'self-delete@example.com', 'correct-horse'),
      )
    } finally {
      await adminAuth.deleteUser(user.uid).catch(() => {})
    }
  }, { served: ['auth.delete'] })

  // ----- out-of-band email flows -------------------------------------------
  //
  // Nothing is mailed, so the link is read back from the documented local
  // control endpoint. Everything else here is the SDKs' own request shapes:
  // firebase-admin picks the URL and body for generate*Link, and the client SDK
  // picks them for sendPasswordResetEmail / confirmPasswordReset /
  // applyActionCode / signInWithEmailLink.

  /// Read a pending link from GET /emulator/v1/projects/{p}/oobCodes.
  ///
  /// This endpoint is the stand-in for a mailbox, so reading it by hand is the
  /// *documented* way a test follows these flows — not a Fireup-internal
  /// shortcut.
  const pendingOobCodes = async () => {
    const response = await fetch(`http://${host}/emulator/v1/projects/${PROJECT_ID}/oobCodes`)
    assert.equal(response.status, 200, `oobCodes listing failed: ${response.status}`)
    const { oobCodes } = await response.json()
    return oobCodes
  }

  const pendingOobCodeFor = async (email, requestType) => {
    const codes = await pendingOobCodes()
    const match = codes.filter((entry) => entry.email === email && entry.requestType === requestType).pop()
    assert.ok(match, `no pending ${requestType} for ${email}: ${JSON.stringify(codes)}`)
    return match
  }

  await check('admin generatePasswordResetLink() returns a link that actually resets the password', async () => {
    const user = await adminAuth.createUser({ email: 'reset@example.com', password: 'original-one' })
    try {
      // The Admin SDK asks for the link in the response rather than by email.
      const link = await adminAuth.generatePasswordResetLink('reset@example.com')
      assert.ok(link.startsWith(`http://${host}/`), `the link must point at this server: ${link}`)
      const oobCode = new URL(link).searchParams.get('oobCode')
      assert.ok(oobCode, `the link must carry an oobCode: ${link}`)

      // And the client SDK finishes the flow with the code out of that link.
      await confirmPasswordReset(clientAuth, oobCode, 'a-reset-password')

      const credential = await signInWithEmailAndPassword(clientAuth, 'reset@example.com', 'a-reset-password')
      assert.equal(credential.user.uid, user.uid)
      await expectAuthCode('auth/wrong-password', () =>
        signInWithEmailAndPassword(clientAuth, 'reset@example.com', 'original-one'),
      )
    } finally {
      await adminAuth.deleteUser(user.uid)
    }
  }, { served: ['auth.admin-sendOobCode', 'auth.resetPassword', 'auth.flow-password-reset'] })

  await check('client sendPasswordResetEmail() + verifyPasswordResetCode() + confirmPasswordReset() round-trips', async () => {
    const user = await adminAuth.createUser({ email: 'forgot@example.com', password: 'original-one' })
    try {
      await sendPasswordResetEmail(clientAuth, 'forgot@example.com')
      const { oobCode } = await pendingOobCodeFor('forgot@example.com', 'PASSWORD_RESET')

      // Checking the code must not spend it, or the form it gates could never
      // be submitted.
      const checkedEmail = await verifyPasswordResetCode(clientAuth, oobCode)
      assert.equal(checkedEmail, 'forgot@example.com')

      await confirmPasswordReset(clientAuth, oobCode, 'another-password')
      const credential = await signInWithEmailAndPassword(clientAuth, 'forgot@example.com', 'another-password')
      assert.equal(credential.user.uid, user.uid)

      // A spent code must be refused, with a mapped error.
      await expectAuthCode('auth/invalid-action-code', () =>
        confirmPasswordReset(clientAuth, oobCode, 'a-third-password'),
      )
    } finally {
      await adminAuth.deleteUser(user.uid)
    }
  }, { served: ['auth.sendOobCode', 'auth.resetPassword', 'auth.emulator-oob-codes', 'auth.flow-password-reset'], 'documented-divergence': ['auth.sending-email'] })

  await check('client sendEmailVerification() + applyActionCode() marks the address verified', async () => {
    const user = await adminAuth.createUser({
      email: 'verify@example.com',
      password: 'correct-horse',
      emailVerified: false,
    })
    try {
      const credential = await signInWithEmailAndPassword(clientAuth, 'verify@example.com', 'correct-horse')
      assert.equal(credential.user.emailVerified, false)

      // The client SDK sends this with the session's idToken, not an email.
      await sendEmailVerification(credential.user)
      const { oobCode } = await pendingOobCodeFor('verify@example.com', 'VERIFY_EMAIL')

      // applyActionCode runs on whatever page the link opened, with no session.
      await applyActionCode(clientAuth, oobCode)

      assert.equal(
        (await adminAuth.getUser(user.uid)).emailVerified,
        true,
        'the Admin SDK must see the address as verified',
      )
      // And the client sees it after a reload, which re-reads the account.
      await credential.user.reload()
      assert.equal(credential.user.emailVerified, true)

      await expectAuthCode('auth/invalid-action-code', () => applyActionCode(clientAuth, oobCode))
    } finally {
      await adminAuth.deleteUser(user.uid)
    }
  }, { served: ['auth.sendOobCode', 'auth.update', 'auth.lookup', 'auth.emulator-oob-codes', 'auth.flow-verify-email'] })

  await check('admin generateEmailVerificationLink() returns an applicable link', async () => {
    const user = await adminAuth.createUser({ email: 'adminverify@example.com', emailVerified: false })
    try {
      const link = await adminAuth.generateEmailVerificationLink('adminverify@example.com')
      const url = new URL(link)
      assert.equal(url.searchParams.get('mode'), 'verifyEmail', `unexpected mode in ${link}`)

      await applyActionCode(clientAuth, url.searchParams.get('oobCode'))
      assert.equal((await adminAuth.getUser(user.uid)).emailVerified, true)
    } finally {
      await adminAuth.deleteUser(user.uid)
    }
  }, { served: ['auth.admin-sendOobCode', 'auth.update', 'auth.flow-verify-email'] })

  await check('client signInWithEmailLink() signs in with no password at all', async () => {
    const email = 'passwordless@example.com'
    const actionCodeSettings = { url: 'http://localhost:3000/finish', handleCodeInApp: true }

    let uid
    try {
      await sendSignInLinkToEmail(clientAuth, email, actionCodeSettings)
      const { oobLink } = await pendingOobCodeFor(email, 'EMAIL_SIGNIN')

      // isSignInWithEmailLink is the SDK's own parse of the link we built, so
      // it checks the link shape rather than anything we assert ourselves.
      assert.ok(
        isSignInWithEmailLink(clientAuth, oobLink),
        `the client SDK must recognise this as a sign-in link: ${oobLink}`,
      )

      const credential = await signInWithEmailLink(clientAuth, email, oobLink)
      uid = credential.user.uid
      assert.equal(credential.user.email, email)
      assert.equal(credential.user.emailVerified, true, 'following the link proves the mailbox')

      // The session has to be a real one the Admin SDK will accept.
      const decoded = await adminAuth.verifyIdToken(await credential.user.getIdToken())
      assert.equal(decoded.uid, uid)

      // A spent link must not sign anyone in again.
      await expectAuthCode('auth/invalid-action-code', () =>
        signInWithEmailLink(clientAuth, email, oobLink),
      )
    } finally {
      if (uid) {
        await adminAuth.deleteUser(uid)
      }
    }
  }, { served: ['auth.sendOobCode', 'auth.signInWithEmailLink', 'auth.emulator-oob-codes', 'auth.flow-email-signin'] })

  await check('a reset link for an unknown address is refused with the mapped error', async () => {
    await expectAuthCode('auth/user-not-found', () =>
      sendPasswordResetEmail(clientAuth, 'nobody-at-all@example.com'),
    )
    await expectAuthCode('auth/invalid-action-code', () =>
      confirmPasswordReset(clientAuth, 'never-minted-code', 'some-password'),
    )
  }, { served: ['auth.sendOobCode', 'auth.resetPassword'] })

  await check('client verifyBeforeUpdateEmail() + applyActionCode() moves the account to the new address', async () => {
    const user = await adminAuth.createUser({ email: 'old-address@example.com', password: 'correct-horse' })
    try {
      const credential = await signInWithEmailAndPassword(clientAuth, 'old-address@example.com', 'correct-horse')
      await credential.user.getIdToken()

      await verifyBeforeUpdateEmail(credential.user, 'new-address@example.com')
      // Nothing changes until the link is followed: the new address is unproven.
      assert.equal((await adminAuth.getUser(user.uid)).email, 'old-address@example.com')

      const codes = await pendingOobCodes()
      const pending = codes.filter((entry) => entry.requestType === 'VERIFY_AND_CHANGE_EMAIL').pop()
      assert.ok(pending, `no pending VERIFY_AND_CHANGE_EMAIL: ${JSON.stringify(codes)}`)

      await applyActionCode(clientAuth, pending.oobCode)

      const changed = await adminAuth.getUser(user.uid)
      assert.equal(changed.email, 'new-address@example.com')
      assert.equal(changed.emailVerified, true, 'following the link proves the new mailbox')
      await signInWithEmailAndPassword(clientAuth, 'new-address@example.com', 'correct-horse')
      await expectAuthCode('auth/user-not-found', () =>
        signInWithEmailAndPassword(clientAuth, 'old-address@example.com', 'correct-horse'),
      )

      // An email change is a credential change: the session from before it
      // must not mint new ID tokens.
      await assert.rejects(
        () => credential.user.getIdToken(true),
        (error) => {
          assert.ok(
            typeof error.code === 'string' && error.code.startsWith('auth/'),
            `expected a mapped auth/* error, got ${error.code}: ${error.message}`,
          )
          return true
        },
      )

      await expectAuthCode('auth/invalid-action-code', () => applyActionCode(clientAuth, pending.oobCode))
    } finally {
      await adminAuth.deleteUser(user.uid)
    }
  }, { served: ['auth.sendOobCode', 'auth.update', 'auth.flow-verify-and-change-email'] })

  // The links above are read out of the listing and finished through the SDK.
  // A developer clicks them instead, which lands on `GET /emulator/action`.
  await check('following an emailed link applies a verification and leaves a reset pending', async () => {
    const user = await adminAuth.createUser({
      email: 'clicker@example.com',
      password: 'correct-horse',
      emailVerified: false,
    })
    try {
      const credential = await signInWithEmailAndPassword(clientAuth, 'clicker@example.com', 'correct-horse')
      await sendEmailVerification(credential.user)
      const verify = await pendingOobCodeFor('clicker@example.com', 'VERIFY_EMAIL')

      const clicked = await fetch(verify.oobLink)
      const landing = await clicked.json()
      assert.equal(clicked.status, 200, `the verification link answered ${clicked.status}: ${JSON.stringify(landing)}`)
      assert.equal(landing.applied, true)
      assert.equal(landing.mode, 'verifyEmail')
      assert.equal((await adminAuth.getUser(user.uid)).emailVerified, true, 'clicking a verification link applies it')

      // Clicking spent the code, the same as applyActionCode would have.
      const again = await fetch(verify.oobLink)
      const refused = await again.json()
      assert.equal(again.status, 400)
      assert.match(refused.error.message, /^INVALID_OOB_CODE/)

      // A reset link cannot be finished by a click — it needs the new password —
      // so the route reports it and the code stays redeemable.
      await sendPasswordResetEmail(clientAuth, 'clicker@example.com')
      const reset = await pendingOobCodeFor('clicker@example.com', 'PASSWORD_RESET')
      const resetClick = await fetch(reset.oobLink)
      const resetLanding = await resetClick.json()
      assert.equal(resetClick.status, 200, `the reset link answered ${resetClick.status}: ${JSON.stringify(resetLanding)}`)
      assert.equal(resetLanding.applied, false)
      assert.equal(resetLanding.finishWith.path, '/v1/accounts:resetPassword')

      await confirmPasswordReset(clientAuth, reset.oobCode, 'clicked-then-reset')
      await signInWithEmailAndPassword(clientAuth, 'clicker@example.com', 'clicked-then-reset')
    } finally {
      await adminAuth.deleteUser(user.uid)
    }
  }, { served: ['auth.emulator-action-route', 'auth.emulator-oob-codes'] })

  await check('DELETE oobCodes drops pending links and leaves accounts alone', async () => {
    const user = await adminAuth.createUser({ email: 'pending@example.com', password: 'correct-horse' })
    try {
      await sendPasswordResetEmail(clientAuth, 'pending@example.com')
      const { oobCode } = await pendingOobCodeFor('pending@example.com', 'PASSWORD_RESET')

      const response = await fetch(`http://${host}/emulator/v1/projects/${PROJECT_ID}/oobCodes`, { method: 'DELETE' })
      assert.equal(response.status, 200, `clearing oobCodes failed: ${response.status}`)

      assert.deepEqual(await pendingOobCodes(), [])
      // A cleared link must be dead, not just unlisted.
      await expectAuthCode('auth/invalid-action-code', () =>
        confirmPasswordReset(clientAuth, oobCode, 'should-not-take'),
      )
      assert.equal((await adminAuth.getUser(user.uid)).email, 'pending@example.com')
      await signInWithEmailAndPassword(clientAuth, 'pending@example.com', 'correct-horse')
    } finally {
      await adminAuth.deleteUser(user.uid)
    }
  }, { served: ['auth.emulator-clear-oob-codes'] })

  await check('GET config reports the local project config', async () => {
    const response = await fetch(`http://${host}/emulator/v1/projects/${PROJECT_ID}/config`)
    const config = await response.json()
    assert.equal(response.status, 200, `config answered ${response.status}: ${JSON.stringify(config)}`)
    // Duplicate emails are refused (the createUser check above relies on it),
    // and the config has to say so rather than describe a different project.
    assert.deepEqual(config.signIn, { allowDuplicateEmails: false })
    assert.equal(config.usageMode, 'DEFAULT')
    assert.equal(config.name, `projects/${PROJECT_ID}/config`)
  }, { served: ['auth.emulator-config'] })

  // ----- what is deliberately not served -----------------------------------
  //
  // These record gaps rather than guarantees. They assert that an unsupported
  // call fails *cleanly* — a mapped error the SDK understands, not a 500, a
  // hang, or a silent success that loses data. If Fireup later implements one,
  // this check fails and whoever did the work gets told to promote it into a
  // real assertion above.

  await check('admin deleteUsers() is refused cleanly (batchDelete is not served)', async () => {
    const a = await adminAuth.createUser({ email: 'batch-a@example.com' })
    const b = await adminAuth.createUser({ email: 'batch-b@example.com' })

    try {
      await assert.rejects(
        () => adminAuth.deleteUsers([a.uid, b.uid]),
        /not served by fireup local/,
        'batchDelete must fail with an explanation, not a 500 or a partial delete',
      )

      // The accounts must still be there: a refused batch must not have
      // deleted some of them on the way out.
      assert.equal((await adminAuth.getUser(a.uid)).uid, a.uid)
      assert.equal((await adminAuth.getUser(b.uid)).uid, b.uid)
    } finally {
      await adminAuth.deleteUser(a.uid)
      await adminAuth.deleteUser(b.uid)
    }
  }, { unimplemented: ['auth.batch-account-writes'] })

  await check('admin importUsers() is refused cleanly (batchCreate is not served)', async () => {
    await assert.rejects(
      () => adminAuth.importUsers([{ uid: 'smoke-import', email: 'import@example.com' }]),
      /not served by fireup local/,
      'batchCreate must fail with an explanation, not a 500 or a partial import',
    )

    // A refused import must not have created anything on the way out.
    await expectAuthCode('auth/user-not-found', () => adminAuth.getUser('smoke-import'))
  }, { unimplemented: ['auth.batch-account-writes'] })

  await check('admin createSessionCookie() is refused cleanly (session cookies are not served)', async () => {
    const idToken = await signedIn.user.getIdToken()
    await assert.rejects(
      () => adminAuth.createSessionCookie(idToken, { expiresIn: 60 * 60 * 1000 }),
      /not served by fireup local/,
      'the refusal must name the gap, not report an unknown project',
    )
  }, { unimplemented: ['auth.session-cookies'] })

  // The SDK checks above only prove the call failed. The fidelity matrix
  // promises a *shape* — 501 with a message naming the gap — and only a direct
  // REST call can see the status code. Both of these answered the wrong one
  // until PR #82: `accounts:batchCreate` fell into the wrong-method handler
  // and said "takes POST, not POST", and `projects/{p}:createSessionCookie`
  // parsed its custom-method colon into the project id and 404'd.
  await check('the unimplemented admin paths answer 501 and name the gap', async () => {
    const refusals = [
      { path: `/v1/projects/${PROJECT_ID}/accounts:batchCreate`, names: /batch account import/ },
      { path: `/v1/projects/${PROJECT_ID}/accounts:batchDelete`, names: /batch account deletion/ },
      { path: `/v1/projects/${PROJECT_ID}:createSessionCookie`, names: /session cookies/ },
      { path: `/v1/projects/${PROJECT_ID}:verifySessionCookie`, names: /session cookies/ },
    ]

    for (const { path, names } of refusals) {
      const response = await fetch(`http://${host}/identitytoolkit.googleapis.com${path}`, {
        method: 'POST',
        // How the Admin SDK authenticates against an emulator.
        headers: { 'Content-Type': 'application/json', Authorization: 'Bearer owner' },
        body: '{}',
      })
      const body = await response.json()
      const message = body.error?.message ?? JSON.stringify(body)

      assert.equal(response.status, 501, `${path} must answer 501, got ${response.status}: ${message}`)
      assert.match(message, names, `${path} must name the gap it is refusing for`)
      assert.match(message, /not served by fireup local/, `${path} must say who refused it`)
    }
  }, { unimplemented: ['auth.batch-account-writes', 'auth.session-cookies'] })

  /// POST a request this surface refuses, and assert it answers 501 with a
  /// message naming the gap — the same shape the check above asserts.
  const expectNotServed = async (path, names, body = {}) => {
    const response = await fetch(`http://${host}/identitytoolkit.googleapis.com${path}?key=${FAKE_API_KEY}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    })
    const reply = await response.json()
    const message = reply.error?.message ?? JSON.stringify(reply)

    assert.equal(response.status, 501, `${path} must answer 501, got ${response.status}: ${message}`)
    assert.match(message, names, `${path} must name the gap it is refusing for`)
    assert.match(message, /not served by fireup local/, `${path} must say who refused it`)
  }

  await check('phone auth answers 501 and names the gap, and mints no code', async () => {
    await expectNotServed('/v1/accounts:sendVerificationCode', /phone authentication/, {
      phoneNumber: '+15555550100',
      recaptchaToken: 'not-checked',
    })
    await expectNotServed('/v1/accounts:signInWithPhoneNumber', /phone authentication/, {
      sessionInfo: 'never-issued',
      code: '123456',
    })
    // Phone verification also arrives as a requestType on an endpoint that is
    // otherwise served, so it must not fall through to a code being minted.
    const before = (await pendingOobCodes()).length
    await expectNotServed('/v1/accounts:sendOobCode', /phone authentication/, {
      requestType: 'PHONE_VERIFICATION',
      email: 'alice@example.com',
    })
    assert.equal((await pendingOobCodes()).length, before, 'a refused PHONE_VERIFICATION must not mint a code')
  }, { unimplemented: ['auth.phone'] })

  await check('admin tenantManager().createTenant() is refused cleanly (the v2 surface is not served)', async () => {
    // The Admin SDK's tenant calls are the v2 surface's most likely caller.
    await assert.rejects(
      () => adminAuth.tenantManager().createTenant({ displayName: 'smoke-tenant' }),
      /not served by fireup local/,
      'createTenant must fail with an explanation, not a 404 or a fake tenant',
    )

    const v2 = /the Identity Platform v2 surface \(MFA, tenants\)/
    await expectNotServed(`/v2/projects/${PROJECT_ID}/tenants`, v2, { displayName: 'smoke-tenant' })
    await expectNotServed('/v2/accounts/mfaEnrollment:start', v2, { idToken: 'not-checked' })
    await expectNotServed('/v2/accounts/mfaSignIn:start', v2, { mfaPendingCredential: 'not-checked' })
  }, { unimplemented: ['auth.mfa-and-tenants'] })

  await check('client fetchSignInMethodsForEmail() is refused cleanly (createAuthUri is not served)', async () => {
    // The SDK reaches createAuthUri through this call; the server's 501 has to
    // come back as a mapped error, not a list of sign-in methods.
    await expectAuthCode('auth/operation-not-allowed', () =>
      fetchSignInMethodsForEmail(clientAuth, 'alice@example.com'),
    )

    const before = (await adminAuth.listUsers(1000)).users.length
    await expectNotServed('/v1/accounts:createAuthUri', /federated identity providers/, {
      identifier: 'alice@example.com',
      continueUri: 'http://localhost',
    })
    await expectNotServed('/v1/accounts:signInWithIdp', /federated identity providers/, {
      postBody: 'id_token=not-checked&providerId=google.com',
      requestUri: 'http://localhost',
      returnSecureToken: true,
    })
    // A refused IdP sign-in must not have created the account it would have.
    assert.equal((await adminAuth.listUsers(1000)).users.length, before)
  }, { unimplemented: ['auth.federated-providers'] })

  // Last, because it wipes every account the checks above left in place.
  await check('DELETE accounts clears every account and every pending link', async () => {
    const user = await adminAuth.createUser({ email: 'wiped@example.com', password: 'correct-horse' })
    await sendPasswordResetEmail(clientAuth, 'wiped@example.com')
    const { oobCode } = await pendingOobCodeFor('wiped@example.com', 'PASSWORD_RESET')
    assert.ok((await adminAuth.listUsers(1000)).users.length > 1, 'the wipe needs more than one account to clear')

    const response = await fetch(`http://${host}/emulator/v1/projects/${PROJECT_ID}/accounts`, { method: 'DELETE' })
    assert.equal(response.status, 200, `clearing accounts failed: ${response.status}`)

    assert.deepEqual((await adminAuth.listUsers(1000)).users, [])
    await expectAuthCode('auth/user-not-found', () => adminAuth.getUser(user.uid))
    assert.deepEqual(await pendingOobCodes(), [], 'pending links must go with their accounts')
    await expectAuthCode('auth/invalid-action-code', () =>
      confirmPasswordReset(clientAuth, oobCode, 'should-not-take'),
    )
  }, { served: ['auth.emulator-clear-accounts'] })

  await deleteClientApp(clientApp)
  await deleteAdminApp(adminApp)

  const failed = checks.filter((c) => !c.ok)
  console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`)
  if (failed.length > 0) {
    console.log('\nFailures:')
    for (const { name, error } of failed) {
      console.log(`- ${name}: ${error?.stack ?? error}`)
    }
    process.exit(1)
  }
  console.log('fireup local speaks the published Identity Toolkit API to the official Firebase SDKs.')
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
