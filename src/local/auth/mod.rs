//! `fireup local`'s Firebase Auth / Identity Platform surface.
//!
//! Serves a subset of the **published** Identity Toolkit REST API on the same
//! port as the Firestore gRPC data plane, so an official Firebase SDK pointed at
//! Fireup with the documented `FIREBASE_AUTH_EMULATOR_HOST` variable can sign
//! users up, sign them in, and manage accounts.
//!
//! ```text
//! export FIREBASE_AUTH_EMULATOR_HOST=127.0.0.1:4589
//! export FIRESTORE_EMULATOR_HOST=127.0.0.1:4589     # same port
//! ```
//!
//! ## What is served
//!
//! | Endpoint | Purpose |
//! |---|---|
//! | `POST /v1/accounts:signUp` | email+password sign-up, and anonymous sign-in |
//! | `POST /v1/accounts:signInWithPassword` | password sign-in |
//! | `POST /v1/accounts:signInWithCustomToken` | sign in with an Admin SDK custom token |
//! | `POST /v1/accounts:lookup` | read the signed-in account |
//! | `POST /v1/accounts:update` | change profile, email, or password; apply an email action code |
//! | `POST /v1/accounts:delete` | delete the signed-in account |
//! | `POST /v1/accounts:sendOobCode` | request a reset / verification / sign-in link |
//! | `POST /v1/accounts:resetPassword` | check a reset code, and confirm it |
//! | `POST /v1/accounts:signInWithEmailLink` | passwordless sign-in with a link |
//! | `POST /v1/token` | exchange a refresh token for a new ID token |
//! | `POST /v1/projects/{p}/accounts` | admin: create an account |
//! | `POST /v1/projects/{p}/accounts:lookup` | admin: read accounts by uid or email |
//! | `POST /v1/projects/{p}/accounts:update` | admin: change any account |
//! | `POST /v1/projects/{p}/accounts:delete` | admin: delete an account |
//! | `POST /v1/projects/{p}/accounts:sendOobCode` | admin: generate a link (`returnOobLink`) |
//! | `GET  /v1/projects/{p}/accounts:batchGet` | admin: list accounts, paged |
//! | `GET  /emulator/v1/projects/{p}/config` | local config an SDK may probe |
//! | `GET  /emulator/v1/projects/{p}/oobCodes` | read pending email links — no mail is sent |
//! | `DELETE /emulator/v1/projects/{p}/accounts` | clear every account and pending link |
//! | `GET  /emulator/action` | where a generated link points (Fireup's own, not a Google page) |
//!
//! SDKs address these through the API host as a path prefix
//! (`/identitytoolkit.googleapis.com/v1/accounts:signUp`,
//! `/securetoken.googleapis.com/v1/token`); the bare `/v1/...` form is accepted
//! too so the endpoints are reachable with `curl`.
//!
//! ## Non-goals
//!
//! Phone auth, multi-factor enrolment, federated/OAuth providers, and
//! production key material. These answer with an explicit "not served" error
//! rather than failing in some ambiguous way — see [`api`].
//!
//! ## No email is sent
//!
//! The out-of-band flows above mint real single-use codes but there is no mail
//! transport, so the link is logged and listed at
//! `GET /emulator/v1/projects/{p}/oobCodes` instead of being delivered. See
//! [`oob`].
//!
//! ## Local development only
//!
//! ID tokens are **unsigned** (see [`token`]), there is no API-key check, and
//! passwords are hashed without a work factor (see
//! [`user::PasswordHash`]). Anything that can reach the port can mint a session
//! for any account. The server binds loopback by default; keep it that way.
//!
//! ## Compliance
//!
//! An independent implementation of Google's **published** Identity Toolkit REST
//! contract and the documented `FIREBASE_AUTH_EMULATOR_HOST` convention, written
//! from public API documentation and the behaviour of Google's own open-source
//! (Apache-2.0) client SDKs. The Firebase Emulator Suite was not decompiled,
//! disassembled, or otherwise analysed, and no proprietary emulator code or
//! asset is present. This is not the Firebase Emulator Suite and is not
//! affiliated with Google. See `docs/licenses/FIREBASE_GCP_LOCAL.md` and
//! `.cursor/rules/firebase-gcp-compliance.mdc`.

pub mod api;
pub mod oob;
pub mod postgres_store;
pub mod store;
pub mod token;
pub mod user;

pub use api::AuthApi;
pub use oob::{OobCode, OobRequestType};
pub use postgres_store::PostgresUserStore;
pub use store::{MemoryUserStore, SharedUserStore, UserStore};
pub use user::{AuthUser, SignInProvider};

use base64::Engine as _;

/// Alphabet for generated uids. Alphanumeric, like the uids Identity Toolkit
/// hands out, so a uid from Fireup is accepted anywhere one from production is —
/// including as a Firestore document id, which rules out `/`.
const UID_ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Length of a generated uid. 28 characters, matching the uids Firebase Auth
/// issues, so a client that stores uids in a fixed-width column does not have to
/// be changed between local and production.
pub const GENERATED_UID_LENGTH: usize = 28;

/// base64url without padding, as JWT segments are encoded (RFC 7515 §2).
pub fn base64url_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Decode a base64url segment, tolerating padding a producer may have added.
pub fn base64url_decode(text: &str) -> Result<Vec<u8>, base64::DecodeError> {
    let engine = base64::engine::general_purpose::GeneralPurpose::new(
        &base64::alphabet::URL_SAFE,
        base64::engine::general_purpose::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    engine.decode(text)
}

/// Standard-alphabet base64 with padding, which is how Identity Toolkit reports
/// `passwordHash` and `salt`.
pub fn base64_standard(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Random bytes, as base64url.
///
/// Sourced from `uuid`'s v4 generator — already a dependency, and backed by the
/// operating system's CSPRNG — rather than adding a second random-number crate.
pub fn random_token(bytes: usize) -> String {
    let mut raw = Vec::with_capacity(bytes + 16);
    while raw.len() < bytes {
        raw.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    raw.truncate(bytes);
    base64url_encode(&raw)
}

/// A fresh uid in the shape Identity Toolkit issues.
pub fn generate_uid() -> String {
    let mut uid = String::with_capacity(GENERATED_UID_LENGTH);
    while uid.len() < GENERATED_UID_LENGTH {
        for byte in uuid::Uuid::new_v4().as_bytes() {
            if uid.len() == GENERATED_UID_LENGTH {
                break;
            }
            // Modulo bias over a 62-character alphabet is negligible here: these
            // ids need to be unique and unguessable-enough for local
            // development, not to carry cryptographic entropy.
            uid.push(UID_ALPHABET[*byte as usize % UID_ALPHABET.len()] as char);
        }
    }
    uid
}

/// Epoch milliseconds now.
///
/// Clamped at zero: a system clock before 1970 would otherwise produce negative
/// `createdAt` values that clients parse as nonsense dates.
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_round_trips_without_padding() {
        let encoded = base64url_encode(b"any carnal pleasure.");
        assert!(!encoded.contains('='), "JWT segments are unpadded: {encoded}");
        assert!(!encoded.contains('+') && !encoded.contains('/'), "{encoded}");
        assert_eq!(
            base64url_decode(&encoded).expect("decodes"),
            b"any carnal pleasure."
        );
    }

    #[test]
    fn padded_base64url_still_decodes() {
        // Not every producer strips padding; refusing those would reject
        // perfectly good tokens.
        assert_eq!(base64url_decode("YQ==").expect("decodes"), b"a");
        assert_eq!(base64url_decode("YQ").expect("decodes"), b"a");
    }

    #[test]
    fn a_generated_uid_looks_like_a_firebase_uid() {
        let uid = generate_uid();
        assert_eq!(uid.len(), GENERATED_UID_LENGTH);
        assert!(
            uid.chars().all(|character| character.is_ascii_alphanumeric()),
            "a uid is also used as a document id, so it must stay path-safe: {uid}"
        );
    }

    #[test]
    fn generated_uids_and_tokens_do_not_repeat() {
        let uids: std::collections::HashSet<String> =
            (0..200).map(|_| generate_uid()).collect();
        assert_eq!(uids.len(), 200, "generated uids must be distinct");

        let tokens: std::collections::HashSet<String> =
            (0..200).map(|_| random_token(16)).collect();
        assert_eq!(tokens.len(), 200);
    }

    #[test]
    fn random_tokens_have_the_requested_entropy() {
        // 24 raw bytes is 32 base64url characters.
        assert_eq!(random_token(24).len(), 32);
        assert_eq!(base64url_decode(&random_token(40)).expect("decodes").len(), 40);
    }

    #[test]
    fn the_clock_is_in_milliseconds_and_not_in_1970() {
        // 1.7e12 ms is late 2023; a seconds-vs-millis mix-up fails this.
        assert!(now_millis() > 1_700_000_000_000, "{}", now_millis());
    }
}
