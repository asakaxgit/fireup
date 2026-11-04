//! The account record behind the local Identity Toolkit surface.
//!
//! One struct, [`AuthUser`], shared by the REST handlers and both
//! [`UserStore`](crate::local::auth::store::UserStore) implementations. Its
//! field names follow the **published** Identity Toolkit `UserInfo` resource
//! (`localId`, `emailVerified`, `validSince`, …) so the mapping to and from the
//! wire shape in [`api`](crate::local::auth::api) is one-to-one and there is no
//! second vocabulary to keep in sync.
//!
//! Timestamps are epoch milliseconds, because that is how Identity Toolkit
//! reports `createdAt` and `lastLoginAt` — strings holding millis. Converting
//! at the edges rather than storing a different unit keeps the responses exact.

use serde_json::{Map, Value as Json};
use sha2::{Digest, Sha256};

/// How a user signs in. Identity Toolkit reports this as
/// `firebase.sign_in_provider` in the ID token and as `providerId` in
/// `providerUserInfo`.
///
/// Phase L covers the two providers that need no external service: password and
/// anonymous. Federated providers, phone, and MFA are explicit non-goals, so
/// they are absent rather than half-present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInProvider {
    Password,
    Anonymous,
    /// Signed in with a custom token minted by an Admin SDK.
    Custom,
}

impl SignInProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            SignInProvider::Password => "password",
            SignInProvider::Anonymous => "anonymous",
            SignInProvider::Custom => "custom",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "password" => Some(SignInProvider::Password),
            "anonymous" => Some(SignInProvider::Anonymous),
            "custom" => Some(SignInProvider::Custom),
            _ => None,
        }
    }
}

/// A stored password, as a salt and a digest.
///
/// # Local-development only
///
/// This is a single SHA-256 pass over `salt || password`. That is **not** a
/// password-storage scheme for anything real: it has no work factor, so it does
/// not resist offline guessing. It is here so that a local store does not sit on
/// disk full of plaintext passwords, which is the failure that actually bites
/// when a developer's seeded dev database gets shared or committed.
///
/// `fireup local` is local-development tooling and says so in every surface it
/// exposes. Do not point anything that holds real credentials at it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordHash {
    /// base64 (standard alphabet) salt, as Identity Toolkit reports `salt`.
    pub salt: String,
    /// base64 digest, as Identity Toolkit reports `passwordHash`.
    pub hash: String,
}

impl PasswordHash {
    /// Hash `password` under a freshly generated salt.
    pub fn new(password: &str) -> Self {
        let salt = super::random_token(16);
        Self {
            hash: Self::digest(&salt, password),
            salt,
        }
    }

    /// Rebuild a stored hash from its two columns.
    pub fn from_parts(salt: impl Into<String>, hash: impl Into<String>) -> Self {
        Self {
            salt: salt.into(),
            hash: hash.into(),
        }
    }

    /// Whether `password` is the one this hash was built from.
    pub fn verify(&self, password: &str) -> bool {
        // Identity Toolkit's own comparison is not something we can observe, and
        // it does not matter: a constant-time compare is the correct way to
        // compare digests regardless of what any other implementation does.
        constant_time_eq(
            Self::digest(&self.salt, password).as_bytes(),
            self.hash.as_bytes(),
        )
    }

    fn digest(salt: &str, password: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(salt.as_bytes());
        hasher.update(password.as_bytes());
        super::base64_standard(&hasher.finalize())
    }
}

/// Compare two byte strings without leaking where they first differ.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

/// One local account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthUser {
    /// Identity Toolkit's `localId` — the uid every SDK calls `uid`.
    pub local_id: String,
    /// Normalised to lowercase on write; `None` for anonymous users.
    pub email: Option<String>,
    pub email_verified: bool,
    pub display_name: Option<String>,
    pub photo_url: Option<String>,
    /// `None` for an account that cannot sign in with a password (anonymous, or
    /// created by an admin without one).
    pub password: Option<PasswordHash>,
    pub disabled: bool,
    /// Custom claims an Admin SDK set, merged into minted ID tokens. Always a
    /// JSON object when present.
    pub custom_claims: Option<Json>,
    pub provider: SignInProvider,
    pub created_at_millis: i64,
    pub last_login_at_millis: Option<i64>,
    /// Epoch millis before which refresh tokens are rejected — Identity
    /// Toolkit's `validSince`, which is how token revocation is expressed.
    pub valid_since_millis: i64,
}

impl AuthUser {
    /// A new account with everything but the identity left at its default.
    pub fn new(local_id: impl Into<String>, provider: SignInProvider, now_millis: i64) -> Self {
        Self {
            local_id: local_id.into(),
            email: None,
            email_verified: false,
            display_name: None,
            photo_url: None,
            password: None,
            disabled: false,
            custom_claims: None,
            provider,
            created_at_millis: now_millis,
            last_login_at_millis: None,
            valid_since_millis: now_millis,
        }
    }

    /// Normalise an email the way Identity Toolkit does for lookups: trimmed and
    /// lowercased, so `A@B.com` and `a@b.com` are one account.
    pub fn normalize_email(email: &str) -> String {
        email.trim().to_lowercase()
    }

    pub fn set_email(&mut self, email: &str) {
        self.email = Some(Self::normalize_email(email));
    }

    /// `providerUserInfo`, the per-provider identity list in lookup responses.
    ///
    /// Anonymous accounts have no provider entries, which is what the published
    /// resource describes and what Admin SDKs expect when they build
    /// `UserRecord.providerData`.
    pub fn provider_user_info(&self) -> Vec<Json> {
        match (self.provider, &self.email) {
            (SignInProvider::Password, Some(email)) => {
                let mut entry = Map::new();
                entry.insert("providerId".into(), Json::from("password"));
                entry.insert("federatedId".into(), Json::from(email.clone()));
                entry.insert("email".into(), Json::from(email.clone()));
                entry.insert("rawId".into(), Json::from(email.clone()));
                if let Some(name) = &self.display_name {
                    entry.insert("displayName".into(), Json::from(name.clone()));
                }
                if let Some(photo) = &self.photo_url {
                    entry.insert("photoUrl".into(), Json::from(photo.clone()));
                }
                vec![Json::Object(entry)]
            }
            _ => Vec::new(),
        }
    }

    /// The `UserInfo` JSON object returned by `accounts:lookup` and the admin
    /// account endpoints.
    ///
    /// Absent values are omitted rather than sent as `null`: Identity Toolkit
    /// omits them, and an Admin SDK that sees `"email": null` would report an
    /// account with an email of `null` instead of no email.
    pub fn to_user_info(&self) -> Json {
        let mut info = Map::new();
        info.insert("localId".into(), Json::from(self.local_id.clone()));
        if let Some(email) = &self.email {
            info.insert("email".into(), Json::from(email.clone()));
            info.insert("emailVerified".into(), Json::from(self.email_verified));
        }
        if let Some(name) = &self.display_name {
            info.insert("displayName".into(), Json::from(name.clone()));
        }
        if let Some(photo) = &self.photo_url {
            info.insert("photoUrl".into(), Json::from(photo.clone()));
        }
        if let Some(password) = &self.password {
            info.insert("passwordHash".into(), Json::from(password.hash.clone()));
            info.insert("salt".into(), Json::from(password.salt.clone()));
        }
        if self.disabled {
            info.insert("disabled".into(), Json::from(true));
        }
        if let Some(claims) = &self.custom_claims {
            // Identity Toolkit carries custom claims as a JSON *string*, not an
            // object; Admin SDKs parse it. Sending an object here would make
            // `UserRecord.customClaims` come back as a string of `[object ...]`.
            info.insert(
                "customAttributes".into(),
                Json::from(claims.to_string()),
            );
        }
        // Millisecond epochs as strings, which is how the published resource
        // encodes 64-bit integers in JSON.
        info.insert("createdAt".into(), Json::from(self.created_at_millis.to_string()));
        if let Some(last_login) = self.last_login_at_millis {
            info.insert("lastLoginAt".into(), Json::from(last_login.to_string()));
            info.insert(
                "lastRefreshAt".into(),
                Json::from(crate::local::auth::token::rfc3339_millis(last_login)),
            );
        }
        info.insert(
            "validSince".into(),
            // `validSince` is epoch *seconds*, unlike the `*At` fields.
            Json::from((self.valid_since_millis / 1000).to_string()),
        );
        let providers = self.provider_user_info();
        if !providers.is_empty() {
            info.insert("providerUserInfo".into(), Json::Array(providers));
        }
        Json::Object(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_round_trips_through_its_salt_and_digest() {
        let hash = PasswordHash::new("correct horse");
        assert!(hash.verify("correct horse"));
        assert!(!hash.verify("Correct horse"));
        assert!(!hash.verify(""));
    }

    #[test]
    fn two_accounts_with_one_password_get_different_digests() {
        // Without a per-account salt, a stolen dev database would show which
        // accounts share a password.
        let first = PasswordHash::new("same");
        let second = PasswordHash::new("same");
        assert_ne!(first.salt, second.salt);
        assert_ne!(first.hash, second.hash);
    }

    #[test]
    fn a_rebuilt_hash_still_verifies() {
        let original = PasswordHash::new("hunter2");
        let restored = PasswordHash::from_parts(original.salt.clone(), original.hash.clone());
        assert!(restored.verify("hunter2"));
    }

    #[test]
    fn emails_are_matched_case_insensitively() {
        let mut user = AuthUser::new("u1", SignInProvider::Password, 1_000);
        user.set_email("  Ada@Example.COM ");
        assert_eq!(user.email.as_deref(), Some("ada@example.com"));
    }

    #[test]
    fn user_info_omits_absent_fields_instead_of_nulling_them() {
        let user = AuthUser::new("anon1", SignInProvider::Anonymous, 1_700_000_000_000);
        let info = user.to_user_info();

        assert_eq!(info["localId"], "anon1");
        for absent in ["email", "emailVerified", "displayName", "photoUrl", "disabled"] {
            assert!(
                info.get(absent).is_none(),
                "{absent} must be omitted, not null: {info}"
            );
        }
        // 64-bit epochs are strings in the published JSON encoding.
        assert_eq!(info["createdAt"], "1700000000000");
        assert_eq!(info["validSince"], "1700000000");
    }

    #[test]
    fn user_info_reports_custom_claims_as_a_json_string() {
        let mut user = AuthUser::new("u1", SignInProvider::Password, 0);
        user.custom_claims = Some(serde_json::json!({ "admin": true }));

        let info = user.to_user_info();
        let raw = info["customAttributes"].as_str().expect("a JSON string");
        assert_eq!(
            serde_json::from_str::<Json>(raw).expect("parses back"),
            serde_json::json!({ "admin": true })
        );
    }

    #[test]
    fn anonymous_users_have_no_provider_entries() {
        let anonymous = AuthUser::new("a", SignInProvider::Anonymous, 0);
        assert!(anonymous.provider_user_info().is_empty());

        let mut password = AuthUser::new("p", SignInProvider::Password, 0);
        password.set_email("ada@example.com");
        let providers = password.provider_user_info();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0]["providerId"], "password");
        assert_eq!(providers[0]["email"], "ada@example.com");
    }
}
