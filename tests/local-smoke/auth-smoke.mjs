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
  getAuth as getClientAuth,
  isSignInWithEmailLink,
  sendEmailVerification,
  sendPasswordResetEmail,
  sendSignInLinkToEmail,
  signInAnonymously,
  signInWithCustomToken,
  signInWithEmailAndPassword,
  signInWithEmailLink,
  verifyPasswordResetCode,
} from 'firebase/auth'

const PROJECT_ID = 'fireup-smoke'

// The client SDK requires an apiKey in its config and appends it to every
// request as ?key=. Against an emulator host it is never validated — the
// documented local convention is that any non-empty string works. If Fireup
// ever started checking it, that would be a finding, so it is deliberately
// obvious nonsense rather than something that looks real.
const FAKE_API_KEY = 'fireup-local-no-api-key-needed'

const checks = []

const check = async (name, fn) => {
  try {
    await fn()
    checks.push({ name, ok: true })
    console.log(`  ok   ${name}`)
  } catch (error) {
    checks.push({ name, ok: false, error })
    console.log(`  FAIL ${name}`)
    console.log(`       ${error?.message ?? error}`)
  }
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
  })

  await check('admin createUser() with a duplicate email fails with auth/email-already-exists', async () => {
    await expectAuthCode('auth/email-already-exists', () =>
      adminAuth.createUser({ email: 'alice@example.com', password: 'another-one' }),
    )
  })

  await check('admin createUser() assigns a uid when none is given', async () => {
    const user = await adminAuth.createUser({ email: 'auto@example.com', password: 'hunter2hunter2' })
    assert.ok(user.uid.length > 0, 'server must assign a uid')
    assert.equal(user.email, 'auto@example.com')
  })

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
  })

  await check('admin getUser() on a missing uid fails with auth/user-not-found', async () => {
    await expectAuthCode('auth/user-not-found', () => adminAuth.getUser('nobody-at-all'))
  })

  await check('admin getUserByEmail() reads the account back by email', async () => {
    const user = await adminAuth.getUserByEmail('alice@example.com')
    assert.equal(user.uid, 'smoke-alice')
  })

  await check('admin getUserByEmail() on a missing email fails with auth/user-not-found', async () => {
    await expectAuthCode('auth/user-not-found', () => adminAuth.getUserByEmail('ghost@example.com'))
  })

  await check('admin updateUser() changes the fields it was given and leaves the rest', async () => {
    const user = await adminAuth.updateUser('smoke-alice', {
      displayName: 'Alice Updated',
      emailVerified: true,
    })
    assert.equal(user.displayName, 'Alice Updated')
    assert.equal(user.emailVerified, true)
    assert.equal(user.email, 'alice@example.com', 'update must not drop untouched fields')
  })

  await check('admin updateUser() can clear a field by passing null', async () => {
    const user = await adminAuth.updateUser('smoke-alice', { photoURL: null, displayName: null })
    assert.equal(user.displayName, undefined)
    assert.equal(user.photoURL, undefined)

    // Put the display name back; later checks read it.
    await adminAuth.updateUser('smoke-alice', { displayName: 'Alice Updated' })
  })

  await check('admin updateUser() can disable and re-enable an account', async () => {
    const disabled = await adminAuth.updateUser('smoke-alice', { disabled: true })
    assert.equal(disabled.disabled, true)

    const enabled = await adminAuth.updateUser('smoke-alice', { disabled: false })
    assert.equal(enabled.disabled, false)
  })

  await check('admin setCustomUserClaims() stores claims and getUser() reports them', async () => {
    await adminAuth.setCustomUserClaims('smoke-alice', { role: 'admin', tier: 3 })

    const user = await adminAuth.getUser('smoke-alice')
    assert.deepEqual(user.customClaims, { role: 'admin', tier: 3 })
  })

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
  })

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
  })

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
  })

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
  })

  // ----- client SDK: the end-user surface ----------------------------------

  let signedIn
  await check('client signInWithEmailAndPassword() signs the account in', async () => {
    signedIn = await signInWithEmailAndPassword(clientAuth, 'alice@example.com', 'correct-horse')
    assert.equal(signedIn.user.uid, 'smoke-alice')
    assert.equal(signedIn.user.email, 'alice@example.com')
    assert.equal(signedIn.user.emailVerified, true)
    assert.ok(signedIn.user.refreshToken, 'server must return a refresh token')
  })

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
  })

  await check('client signInWithEmailAndPassword() rejects an unknown email', async () => {
    await expectAuthCode('auth/user-not-found', () =>
      signInWithEmailAndPassword(clientAuth, 'ghost@example.com', 'correct-horse'),
    )
  })

  await check('client signInWithEmailAndPassword() rejects a disabled account', async () => {
    await adminAuth.updateUser('smoke-alice', { disabled: true })
    try {
      await expectAuthCode('auth/user-disabled', () =>
        signInWithEmailAndPassword(clientAuth, 'alice@example.com', 'correct-horse'),
      )
    } finally {
      await adminAuth.updateUser('smoke-alice', { disabled: false })
    }
  })

  await check('client signInAnonymously() creates an anonymous account', async () => {
    const credential = await signInAnonymously(clientAuth)
    assert.ok(credential.user.uid.length > 0)
    assert.equal(credential.user.isAnonymous, true)
    assert.equal(credential.user.email, null)

    const claims = decodeClaims(await credential.user.getIdToken())
    assert.equal(claims.firebase.sign_in_provider, 'anonymous')
  })

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
  })

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
  })

  // ----- deletion ----------------------------------------------------------

  await check('admin deleteUser() removes the account', async () => {
    await adminAuth.createUser({ uid: 'smoke-temp', email: 'temp@example.com' })
    await adminAuth.deleteUser('smoke-temp')
    await expectAuthCode('auth/user-not-found', () => adminAuth.getUser('smoke-temp'))
  })

  await check('admin deleteUser() on a missing uid fails with auth/user-not-found', async () => {
    await expectAuthCode('auth/user-not-found', () => adminAuth.deleteUser('never-existed'))
  })

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
  })

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
  })

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
  })

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
  })

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
  })

  await check('a reset link for an unknown address is refused with the mapped error', async () => {
    await expectAuthCode('auth/user-not-found', () =>
      sendPasswordResetEmail(clientAuth, 'nobody-at-all@example.com'),
    )
    await expectAuthCode('auth/invalid-action-code', () =>
      confirmPasswordReset(clientAuth, 'never-minted-code', 'some-password'),
    )
  })

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
  })

  await check('admin createSessionCookie() is refused cleanly (session cookies are not served)', async () => {
    const idToken = await signedIn.user.getIdToken()
    await assert.rejects(() => adminAuth.createSessionCookie(idToken, { expiresIn: 60 * 60 * 1000 }))
  })

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
