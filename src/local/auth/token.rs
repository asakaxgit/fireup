//! ID tokens, custom tokens, and refresh tokens for the local auth surface.
//!
//! ## Why the tokens are unsigned
//!
//! A Firebase **ID token** is a JWT whose claim set is publicly documented
//! (`iss`, `aud`, `sub`, `auth_time`, `iat`, `exp`, `firebase.identities`,
//! `firebase.sign_in_provider`, plus any custom claims). In production it is
//! signed with a Google private key that clients verify against Google's public
//! certificates.
//!
//! A local server has no such key, and minting one would mean shipping or
//! generating **production-shaped key material** — an explicit non-goal. The
//! convention official, **open-source** (Apache-2.0) Firebase SDKs follow when
//! their documented `FIREBASE_AUTH_EMULATOR_HOST` variable is set is to accept a
//! JWT with `"alg": "none"` and an empty signature, verifying the claims but not
//! a signature. So that is what this module mints: header `{"alg":"none",
//! "typ":"JWT"}`, the documented claim set, and an empty third segment.
//!
//! That shape comes from the published token reference plus the behaviour of
//! Google's open-source client libraries. No proprietary Google binary was
//! decompiled, disassembled, or otherwise analysed — see
//! `docs/licenses/FIREBASE_GCP_LOCAL.md`.
//!
//! Because the tokens are unsigned, anything that accepts them trusts whoever
//! can reach the port. That is the same trust boundary the rest of
//! `fireup local` already has, and it is why the server binds loopback by
//! default.
//!
//! ## Refresh tokens
//!
//! A refresh token is opaque to clients by contract — they hand it back
//! verbatim to `/v1/token`. So this module uses Fireup's own encoding (a
//! base64url JSON envelope) rather than imitating anyone else's. Revocation
//! works through the account's `validSince`: a refresh token issued before it is
//! refused, which is the documented meaning of `validSince` /
//! `tokensValidAfterTime`.

use serde_json::{Map, Value as Json};

use super::user::{AuthUser, SignInProvider};
use super::{base64url_decode, base64url_encode};

/// How long a minted ID token is valid. One hour, as Firebase ID tokens are.
pub const ID_TOKEN_TTL_SECONDS: i64 = 3600;

/// The `aud` an Admin SDK puts in a custom token. Documented as part of the
/// custom-token contract; checked so a plain ID token cannot be replayed as a
/// custom token.
pub const CUSTOM_TOKEN_AUDIENCE: &str =
    "https://identitytoolkit.googleapis.com/google.identity.identitytoolkit.v1.IdentityToolkit";

/// Marker in Fireup's refresh-token envelope, so a token from some other tool
/// is rejected as malformed instead of being half-parsed.
const REFRESH_TOKEN_KIND: &str = "fireup-local-refresh";

/// Claim names a caller's custom claims may not set, because the token's own
/// meaning depends on them. Setting one is an error in the Admin SDK too.
pub const RESERVED_CLAIMS: &[&str] = &[
    "aud",
    "auth_time",
    "exp",
    "firebase",
    "iat",
    "iss",
    "jti",
    "nbf",
    "sub",
    "user_id",
];

/// Format epoch millis as RFC 3339 UTC, the shape Identity Toolkit uses for
/// `lastRefreshAt`.
pub fn rfc3339_millis(millis: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis)
        .unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// `iss` for ID tokens of `project_id`.
pub fn issuer(project_id: &str) -> String {
    format!("https://securetoken.google.com/{project_id}")
}

/// Render an unsigned JWT: `base64url(header).base64url(claims).`
///
/// The trailing dot is required — a JWS with an empty signature still has three
/// segments, and a two-segment string is not a JWT at all.
fn encode_unsigned_jwt(claims: &Json) -> String {
    let header = serde_json::json!({ "alg": "none", "typ": "JWT" });
    format!(
        "{}.{}.",
        base64url_encode(header.to_string().as_bytes()),
        base64url_encode(claims.to_string().as_bytes()),
    )
}

/// Why a token could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenError {
    /// Not a JWT, or its claims are not a JSON object.
    Malformed,
    /// Well-formed but for a different project, or not a token of this kind.
    WrongAudience,
    /// `exp` is in the past.
    Expired,
    /// No usable subject (`sub`/`user_id`/`uid`).
    MissingSubject,
}

impl TokenError {
    /// The Identity Toolkit error code clients match on.
    pub const fn code(self) -> &'static str {
        match self {
            TokenError::Expired => "TOKEN_EXPIRED",
            _ => "INVALID_ID_TOKEN",
        }
    }
}

/// Claims of an ID token this server minted, read back.
#[derive(Debug, Clone)]
pub struct IdTokenClaims {
    pub local_id: String,
    pub audience: String,
    pub expires_at_seconds: i64,
    pub issued_at_seconds: i64,
}

/// Decode the claim set of an unsigned JWT without verifying a signature.
///
/// There is nothing to verify: these tokens carry `alg: none` by design (see the
/// module docs). A token with a real signature is still read — the signature is
/// simply ignored — because a client that was handed a signed token by
/// something else should get a claim-based error, not a parse error.
fn decode_claims(token: &str) -> Result<Map<String, Json>, TokenError> {
    let mut segments = token.trim().split('.');
    let _header = segments.next().ok_or(TokenError::Malformed)?;
    let payload = segments.next().ok_or(TokenError::Malformed)?;
    // A JWS has exactly three segments; more means this is not one.
    segments.next().ok_or(TokenError::Malformed)?;
    if segments.next().is_some() {
        return Err(TokenError::Malformed);
    }

    let bytes = base64url_decode(payload).map_err(|_| TokenError::Malformed)?;
    match serde_json::from_slice::<Json>(&bytes).map_err(|_| TokenError::Malformed)? {
        Json::Object(claims) => Ok(claims),
        _ => Err(TokenError::Malformed),
    }
}

/// Mint an ID token for `user`.
///
/// Custom claims are merged in at the top level, which is where ID token custom
/// claims live and where `request.auth.token.<claim>` reads them from. Reserved
/// claims in the custom set are dropped rather than allowed to overwrite the
/// token's own meaning; [`validate_custom_claims`] rejects them at the point a
/// caller sets them, so reaching this is already a bug elsewhere.
pub fn mint_id_token(user: &AuthUser, project_id: &str, now_seconds: i64) -> String {
    let mut claims = Map::new();

    if let Some(custom) = user.custom_claims.as_ref().and_then(Json::as_object) {
        for (claim, value) in custom {
            if !RESERVED_CLAIMS.contains(&claim.as_str()) {
                claims.insert(claim.clone(), value.clone());
            }
        }
    }

    let mut identities = Map::new();
    if let (SignInProvider::Password, Some(email)) = (user.provider, &user.email) {
        identities.insert(
            "email".into(),
            Json::Array(vec![Json::from(email.clone())]),
        );
    }

    claims.insert("iss".into(), Json::from(issuer(project_id)));
    claims.insert("aud".into(), Json::from(project_id.to_string()));
    claims.insert("sub".into(), Json::from(user.local_id.clone()));
    claims.insert("user_id".into(), Json::from(user.local_id.clone()));
    claims.insert("iat".into(), Json::from(now_seconds));
    claims.insert("auth_time".into(), Json::from(now_seconds));
    claims.insert(
        "exp".into(),
        Json::from(now_seconds + ID_TOKEN_TTL_SECONDS),
    );
    if let Some(email) = &user.email {
        claims.insert("email".into(), Json::from(email.clone()));
        claims.insert("email_verified".into(), Json::from(user.email_verified));
    }
    if let Some(name) = &user.display_name {
        claims.insert("name".into(), Json::from(name.clone()));
    }
    if let Some(photo) = &user.photo_url {
        claims.insert("picture".into(), Json::from(photo.clone()));
    }
    claims.insert(
        "firebase".into(),
        serde_json::json!({
            "identities": Json::Object(identities),
            "sign_in_provider": user.provider.as_str(),
        }),
    );

    encode_unsigned_jwt(&Json::Object(claims))
}

/// Read back an ID token this server minted, checking project and expiry.
pub fn parse_id_token(
    token: &str,
    project_id: &str,
    now_seconds: i64,
) -> Result<IdTokenClaims, TokenError> {
    let claims = decode_claims(token)?;

    let audience = claims
        .get("aud")
        .and_then(Json::as_str)
        .ok_or(TokenError::Malformed)?
        .to_string();
    if audience != project_id {
        return Err(TokenError::WrongAudience);
    }

    let local_id = claims
        .get("sub")
        .or_else(|| claims.get("user_id"))
        .and_then(Json::as_str)
        .filter(|subject| !subject.is_empty())
        .ok_or(TokenError::MissingSubject)?
        .to_string();

    let expires_at_seconds = claims
        .get("exp")
        .and_then(Json::as_i64)
        .ok_or(TokenError::Malformed)?;
    if expires_at_seconds <= now_seconds {
        return Err(TokenError::Expired);
    }

    Ok(IdTokenClaims {
        local_id,
        audience,
        issued_at_seconds: claims.get("iat").and_then(Json::as_i64).unwrap_or(0),
        expires_at_seconds,
    })
}

/// What a custom token asks for: a uid and optional extra claims.
#[derive(Debug, Clone)]
pub struct CustomTokenClaims {
    pub uid: String,
    pub claims: Option<Json>,
}

/// Read a custom token minted by an Admin SDK in emulator mode.
///
/// The documented custom-token claim set is `{iss, sub, aud, uid, iat, exp}`
/// plus an optional `claims` object. `aud` identifies it as a custom token, so
/// it is checked: without that, an ID token would be accepted here and a client
/// could mint itself a session for any uid it could read.
pub fn parse_custom_token(token: &str, now_seconds: i64) -> Result<CustomTokenClaims, TokenError> {
    let claims = decode_claims(token)?;

    match claims.get("aud").and_then(Json::as_str) {
        Some(CUSTOM_TOKEN_AUDIENCE) => {}
        _ => return Err(TokenError::WrongAudience),
    }

    // `exp` is optional here: a token without one is treated as not expiring,
    // which is harmless for a local-only, unsigned token and avoids rejecting a
    // hand-written one during development.
    if let Some(expires_at) = claims.get("exp").and_then(Json::as_i64) {
        if expires_at <= now_seconds {
            return Err(TokenError::Expired);
        }
    }

    let uid = claims
        .get("uid")
        .or_else(|| claims.get("sub"))
        .and_then(Json::as_str)
        .filter(|uid| !uid.is_empty())
        .ok_or(TokenError::MissingSubject)?
        .to_string();

    Ok(CustomTokenClaims {
        uid,
        claims: claims.get("claims").filter(|value| value.is_object()).cloned(),
    })
}

/// Mint a custom token. Only used by this crate's tests and by anything that
/// wants to exercise `accounts:signInWithCustomToken`; the real producer is an
/// Admin SDK.
pub fn mint_custom_token(uid: &str, claims: Option<&Json>, now_seconds: i64) -> String {
    let mut payload = Map::new();
    payload.insert("aud".into(), Json::from(CUSTOM_TOKEN_AUDIENCE));
    payload.insert("uid".into(), Json::from(uid.to_string()));
    payload.insert("iat".into(), Json::from(now_seconds));
    payload.insert("exp".into(), Json::from(now_seconds + ID_TOKEN_TTL_SECONDS));
    if let Some(claims) = claims {
        payload.insert("claims".into(), claims.clone());
    }
    encode_unsigned_jwt(&Json::Object(payload))
}

/// A refresh token's contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshToken {
    pub local_id: String,
    pub project_id: String,
    /// When the token was issued, in epoch millis. Compared against the
    /// account's `validSince` to implement revocation.
    pub issued_at_millis: i64,
}

impl RefreshToken {
    pub fn encode(&self) -> String {
        let envelope = serde_json::json!({
            "kind": REFRESH_TOKEN_KIND,
            "localId": self.local_id,
            "projectId": self.project_id,
            "issuedAt": self.issued_at_millis,
            // Makes two tokens issued in the same millisecond differ, so a
            // client cannot accidentally treat them as the same session.
            "nonce": super::random_token(12),
        });
        base64url_encode(envelope.to_string().as_bytes())
    }

    pub fn decode(token: &str) -> Result<Self, TokenError> {
        let bytes = base64url_decode(token.trim()).map_err(|_| TokenError::Malformed)?;
        let envelope: Json = serde_json::from_slice(&bytes).map_err(|_| TokenError::Malformed)?;

        if envelope.get("kind").and_then(Json::as_str) != Some(REFRESH_TOKEN_KIND) {
            return Err(TokenError::Malformed);
        }
        Ok(Self {
            local_id: envelope
                .get("localId")
                .and_then(Json::as_str)
                .filter(|id| !id.is_empty())
                .ok_or(TokenError::MissingSubject)?
                .to_string(),
            project_id: envelope
                .get("projectId")
                .and_then(Json::as_str)
                .ok_or(TokenError::Malformed)?
                .to_string(),
            issued_at_millis: envelope
                .get("issuedAt")
                .and_then(Json::as_i64)
                .ok_or(TokenError::Malformed)?,
        })
    }
}

/// Reject a custom-claims payload that cannot be used.
///
/// Mirrors the Admin SDK's own validation: the payload must be a JSON object and
/// must not set a reserved claim. Silently dropping a reserved claim would let a
/// caller believe `sub` had been overridden.
pub fn validate_custom_claims(claims: &Json) -> Result<(), String> {
    let object = claims
        .as_object()
        .ok_or_else(|| "custom claims must be a JSON object".to_string())?;

    let mut reserved: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|claim| RESERVED_CLAIMS.contains(claim))
        .collect();
    if reserved.is_empty() {
        Ok(())
    } else {
        reserved.sort_unstable();
        Err(format!(
            "custom claims must not contain the reserved claim(s): {}",
            reserved.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn password_user() -> AuthUser {
        let mut user = AuthUser::new("uid-1", SignInProvider::Password, 1_700_000_000_000);
        user.set_email("ada@example.com");
        user.email_verified = true;
        user.display_name = Some("Ada".into());
        user
    }

    #[test]
    fn a_minted_id_token_has_three_segments_and_an_empty_signature() {
        let token = mint_id_token(&password_user(), "demo", 1_000);
        let segments: Vec<&str> = token.split('.').collect();

        assert_eq!(segments.len(), 3, "a JWS has three segments: {token}");
        assert_eq!(segments[2], "", "local tokens are unsigned by design");

        let header: Json = serde_json::from_slice(
            &base64url_decode(segments[0]).expect("header is base64url"),
        )
        .expect("header is JSON");
        assert_eq!(header["alg"], "none");
        assert_eq!(header["typ"], "JWT");
    }

    #[test]
    fn id_token_claims_match_the_published_set() {
        let token = mint_id_token(&password_user(), "demo", 1_000);
        let claims = decode_claims(&token).expect("claims decode");

        assert_eq!(claims["iss"], "https://securetoken.google.com/demo");
        assert_eq!(claims["aud"], "demo");
        assert_eq!(claims["sub"], "uid-1");
        assert_eq!(claims["user_id"], "uid-1");
        assert_eq!(claims["iat"], 1_000);
        assert_eq!(claims["auth_time"], 1_000);
        assert_eq!(claims["exp"], 1_000 + ID_TOKEN_TTL_SECONDS);
        assert_eq!(claims["email"], "ada@example.com");
        assert_eq!(claims["email_verified"], true);
        assert_eq!(claims["name"], "Ada");
        assert_eq!(claims["firebase"]["sign_in_provider"], "password");
        assert_eq!(
            claims["firebase"]["identities"]["email"],
            serde_json::json!(["ada@example.com"])
        );
    }

    #[test]
    fn an_anonymous_token_names_its_provider_and_carries_no_identities() {
        let user = AuthUser::new("anon", SignInProvider::Anonymous, 0);
        let claims = decode_claims(&mint_id_token(&user, "demo", 10)).expect("claims");

        assert_eq!(claims["firebase"]["sign_in_provider"], "anonymous");
        assert_eq!(claims["firebase"]["identities"], serde_json::json!({}));
        assert!(claims.get("email").is_none());
    }

    #[test]
    fn custom_claims_ride_at_the_top_level_but_cannot_shadow_the_token() {
        let mut user = password_user();
        user.custom_claims = Some(serde_json::json!({ "role": "admin", "sub": "attacker" }));

        let claims = decode_claims(&mint_id_token(&user, "demo", 10)).expect("claims");
        assert_eq!(claims["role"], "admin");
        assert_eq!(claims["sub"], "uid-1", "a custom claim must not move sub");
    }

    #[test]
    fn a_minted_token_parses_back_until_it_expires() {
        let token = mint_id_token(&password_user(), "demo", 1_000);

        let parsed = parse_id_token(&token, "demo", 1_500).expect("still valid");
        assert_eq!(parsed.local_id, "uid-1");
        assert_eq!(parsed.audience, "demo");

        assert_eq!(
            parse_id_token(&token, "demo", 1_000 + ID_TOKEN_TTL_SECONDS).unwrap_err(),
            TokenError::Expired,
            "a token is dead at exp, not a second later",
        );
    }

    #[test]
    fn a_token_for_another_project_is_refused() {
        let token = mint_id_token(&password_user(), "demo", 1_000);
        assert_eq!(
            parse_id_token(&token, "other", 1_500).unwrap_err(),
            TokenError::WrongAudience
        );
    }

    #[test]
    fn garbage_is_not_mistaken_for_a_token() {
        for bad in ["", "not-a-jwt", "a.b", "a.b.c.d", "a.!!!.c"] {
            assert!(
                parse_id_token(bad, "demo", 0).is_err(),
                "{bad:?} must not parse"
            );
        }
    }

    #[test]
    fn an_id_token_cannot_be_replayed_as_a_custom_token() {
        let id_token = mint_id_token(&password_user(), "demo", 1_000);
        assert_eq!(
            parse_custom_token(&id_token, 1_100).unwrap_err(),
            TokenError::WrongAudience
        );
    }

    #[test]
    fn a_custom_token_round_trips_with_its_claims() {
        let extra = serde_json::json!({ "tier": "gold" });
        let token = mint_custom_token("uid-9", Some(&extra), 500);

        let parsed = parse_custom_token(&token, 600).expect("valid custom token");
        assert_eq!(parsed.uid, "uid-9");
        assert_eq!(parsed.claims, Some(extra));
    }

    #[test]
    fn a_refresh_token_round_trips_and_rejects_foreign_tokens() {
        let token = RefreshToken {
            local_id: "uid-1".into(),
            project_id: "demo".into(),
            issued_at_millis: 1_700_000_000_000,
        };
        let encoded = token.encode();

        assert_eq!(RefreshToken::decode(&encoded).expect("decodes"), token);
        assert!(
            RefreshToken::decode(&base64url_encode(b"{\"kind\":\"something-else\"}")).is_err(),
            "an envelope from another tool must not be accepted"
        );
        assert!(RefreshToken::decode("nonsense!!").is_err());
    }

    #[test]
    fn two_refresh_tokens_for_one_session_differ() {
        let token = RefreshToken {
            local_id: "uid-1".into(),
            project_id: "demo".into(),
            issued_at_millis: 1,
        };
        assert_ne!(token.encode(), token.encode());
    }

    #[test]
    fn reserved_custom_claims_are_reported_rather_than_dropped() {
        validate_custom_claims(&serde_json::json!({ "role": "admin" })).expect("plain claims");

        let error = validate_custom_claims(&serde_json::json!({ "sub": "x", "exp": 1 }))
            .expect_err("reserved claims must be refused");
        assert!(error.contains("exp"), "{error}");
        assert!(error.contains("sub"), "{error}");

        validate_custom_claims(&serde_json::json!("not an object"))
            .expect_err("claims must be an object");
    }

    #[test]
    fn rfc3339_formats_millis_as_utc() {
        assert_eq!(rfc3339_millis(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339_millis(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
    }
}
