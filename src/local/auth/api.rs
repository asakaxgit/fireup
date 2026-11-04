//! The Identity Toolkit REST handlers.
//!
//! One entry point, [`AuthApi::route`], which takes an already-parsed request
//! and returns an HTTP status plus a JSON body. It does no socket work, so every
//! endpoint below is unit-testable without binding a port; `src/local/http.rs`
//! owns the wire.
//!
//! ## Error shape
//!
//! Identity Toolkit reports failures as
//! `{"error": {"code": 400, "message": "EMAIL_EXISTS", "errors": [...]}}`, and
//! official SDKs match on `error.message`. [`error_response`] is the single
//! place that shape is built, so no handler can invent a different one and
//! quietly break a client's error handling.
//!
//! ## No API key, no bearer check
//!
//! Client endpoints carry `?key=<apiKey>` and admin endpoints an OAuth bearer
//! token in production. Both are ignored here: there is no key to validate
//! against and no IAM to consult, and pretending to check would suggest a
//! security boundary that does not exist. The real boundary is the loopback
//! bind. This is local-development tooling.

use std::sync::Arc;

use serde_json::{json, Map, Value as Json};
use tracing::debug;

use super::store::{InsertOutcome, SharedUserStore, UpdateOutcome};
use super::token::{self, RefreshToken, TokenError};
use super::user::{AuthUser, PasswordHash, SignInProvider};
use crate::local::config::LocalConfig;
use crate::local::store::StoreError;

/// Path prefix SDKs use for Identity Toolkit when `FIREBASE_AUTH_EMULATOR_HOST`
/// is set: the API host becomes the first path segment.
const IDENTITY_TOOLKIT_PREFIX: &str = "/identitytoolkit.googleapis.com";

/// The same, for the token-refresh endpoint, which lives on a different API host.
const SECURE_TOKEN_PREFIX: &str = "/securetoken.googleapis.com";

/// Shortest password Identity Toolkit accepts.
const MIN_PASSWORD_LENGTH: usize = 6;

/// Accounts returned by one `accounts:batchGet` page when the caller does not
/// ask for a size.
const DEFAULT_PAGE_SIZE: usize = 1_000;

/// Hard cap on a requested page size, so a `maxResults=100000000` cannot ask the
/// process to build one enormous response.
const MAX_PAGE_SIZE: usize = 1_000;

/// A request, already split up by the HTTP layer.
#[derive(Debug)]
pub struct AuthRequest<'a> {
    pub method: &'a str,
    /// Path with the query string already removed.
    pub path: &'a str,
    /// Raw query string, without the leading `?`.
    pub query: &'a str,
    pub body: &'a [u8],
}

/// The auth surface's state: what project it answers for, and where accounts
/// live.
pub struct AuthApi {
    config: Arc<LocalConfig>,
    users: SharedUserStore,
}

/// Strip the API-host prefix SDKs prepend, leaving a bare `/v1/...` path.
fn normalize_path(path: &str) -> &str {
    for prefix in [IDENTITY_TOOLKIT_PREFIX, SECURE_TOKEN_PREFIX] {
        if let Some(rest) = path.strip_prefix(prefix) {
            return if rest.is_empty() { "/" } else { rest };
        }
    }
    path
}

/// Whether `path` belongs to the auth surface.
///
/// Checked before the request is read as auth so that `/` and `/health` keep
/// their own meaning, and so a path this surface does not serve gets an
/// Identity-Toolkit-shaped error rather than the discovery document's 404.
pub fn handles(path: &str) -> bool {
    if path.starts_with(IDENTITY_TOOLKIT_PREFIX) || path.starts_with(SECURE_TOKEN_PREFIX) {
        return true;
    }
    let path = normalize_path(path);
    path.starts_with("/v1/accounts")
        || path == "/v1/token"
        || path.starts_with("/v1/projects/")
        || path.starts_with("/v2/")
        || path.starts_with("/emulator/v1/")
}

/// Build the Identity Toolkit error envelope.
///
/// `message` is the code SDKs match on (`EMAIL_EXISTS`, `INVALID_PASSWORD`, …),
/// optionally followed by ` : <detail>`, which is how Identity Toolkit attaches
/// detail to codes like `WEAK_PASSWORD`.
pub fn error_response(status: u16, message: &str) -> (u16, Json) {
    (
        status,
        json!({
            "error": {
                "code": status,
                "message": message,
                "errors": [{
                    "message": message,
                    "domain": "global",
                    "reason": "invalid",
                }],
            }
        }),
    )
}

fn bad_request(code: &str) -> (u16, Json) {
    error_response(400, code)
}

/// An endpoint Phase L deliberately does not serve.
///
/// 501 with a message naming the non-goal, rather than 404: the endpoint exists
/// in the API, so "not found" would send a developer looking for a typo instead
/// of telling them Fireup does not do this.
fn not_served(what: &str) -> (u16, Json) {
    error_response(
        501,
        &format!("OPERATION_NOT_ALLOWED : {what} is not served by fireup local"),
    )
}

/// Report a storage failure as a 500 with its real message.
///
/// The message is the backend's own, because a developer whose local Postgres
/// went away needs to read that, not a generic code.
fn store_failure(context: &str, error: StoreError) -> (u16, Json) {
    debug!(%error, context, "auth store failure");
    error_response(500, &format!("INTERNAL_ERROR : {context}: {error}"))
}

/// Parse a JSON request body into an object.
///
/// An empty body is an empty object: several of these endpoints take no required
/// field (anonymous `signUp`, the emulator routes), and SDKs do send `{}` or
/// nothing at all for those.
fn parse_body(body: &[u8]) -> Result<Map<String, Json>, (u16, Json)> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    match serde_json::from_slice::<Json>(body) {
        Ok(Json::Object(object)) => Ok(object),
        Ok(_) => Err(bad_request("INVALID_ARGUMENT : request body must be a JSON object")),
        Err(error) => Err(bad_request(&format!("INVALID_ARGUMENT : {error}"))),
    }
}

/// Read a string field, treating an empty string as absent.
fn string_field(body: &Map<String, Json>, field: &str) -> Option<String> {
    body.get(field)
        .and_then(Json::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

/// Read a boolean field that a client may have sent as the string `"true"`.
///
/// Query strings and some SDK versions send booleans as strings; accepting both
/// is cheaper than finding out from a bug report that `disableUser=\"true\"`
/// silently did nothing.
fn bool_field(body: &Map<String, Json>, field: &str) -> Option<bool> {
    match body.get(field)? {
        Json::Bool(value) => Some(*value),
        Json::String(value) => value.parse().ok(),
        _ => None,
    }
}

/// Read a field holding a 64-bit integer, which the published JSON encoding
/// allows to be either a number or a string.
fn int_field(body: &Map<String, Json>, field: &str) -> Option<i64> {
    match body.get(field)? {
        Json::Number(number) => number.as_i64(),
        Json::String(text) => text.parse().ok(),
        _ => None,
    }
}

/// Values of a repeated string field, which SDKs send as an array.
fn string_list(body: &Map<String, Json>, field: &str) -> Vec<String> {
    body.get(field)
        .and_then(Json::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Json::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `email` is plausibly an email address.
///
/// Deliberately a shape check, not RFC 5322: the point is to turn a typo into
/// `INVALID_EMAIL` at sign-up instead of an account nobody can sign in to. A
/// stricter grammar would reject addresses that are actually valid.
pub fn is_plausible_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !email.chars().any(char::is_whitespace)
}

/// One `key=value` pair from a query string or form body, percent-decoded.
fn parse_pairs(encoded: &str) -> Map<String, Json> {
    let mut pairs = Map::new();
    for pair in encoded.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        pairs.insert(
            percent_decode(key),
            Json::from(percent_decode(value)),
        );
    }
    pairs
}

/// Percent-decode a form-encoded value, turning `+` into a space.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    // Not a valid escape; keep the '%' literally rather than
                    // dropping a character the caller meant to send.
                    Err(_) => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What a successful sign-in hands back.
struct Session {
    id_token: String,
    refresh_token: String,
}

impl AuthApi {
    pub fn new(config: Arc<LocalConfig>, users: SharedUserStore) -> Self {
        Self { config, users }
    }

    pub fn users(&self) -> &SharedUserStore {
        &self.users
    }

    /// Handle a request, using the system clock.
    pub async fn route(&self, request: &AuthRequest<'_>) -> (u16, Json) {
        self.route_at(request, super::now_millis()).await
    }

    /// Handle a request at a caller-supplied instant.
    ///
    /// Separate from [`route`](Self::route) so expiry and `createdAt` can be
    /// tested without sleeping or mocking the clock.
    pub async fn route_at(&self, request: &AuthRequest<'_>, now_millis: i64) -> (u16, Json) {
        let path = normalize_path(request.path);
        let method = request.method;

        // The v2 surface is multi-factor enrolment and tenant management, both
        // explicit non-goals.
        if path.starts_with("/v2/") {
            return not_served("the Identity Platform v2 surface (MFA, tenants)");
        }

        if let Some(rest) = path.strip_prefix("/emulator/v1/projects/") {
            return self.route_emulator(method, rest).await;
        }

        // Admin endpoints are project-scoped in the path; client endpoints are
        // not and answer for the configured project.
        if let Some(rest) = path.strip_prefix("/v1/projects/") {
            return self.route_admin(method, rest, request, now_millis).await;
        }

        if path == "/v1/token" {
            return self.refresh(method, request, now_millis).await;
        }

        let Some(action) = path.strip_prefix("/v1/accounts:") else {
            return self.unknown(path);
        };
        if method != "POST" {
            return self.wrong_method(method, &format!("/v1/accounts:{action}"), "POST");
        }

        let body = match parse_body(request.body) {
            Ok(body) => body,
            Err(response) => return response,
        };

        match action {
            "signUp" => self.sign_up(&body, now_millis).await,
            "signInWithPassword" => self.sign_in_with_password(&body, now_millis).await,
            "signInWithCustomToken" => self.sign_in_with_custom_token(&body, now_millis).await,
            "lookup" => self.client_lookup(&body, now_millis).await,
            "update" => self.client_update(&body, now_millis).await,
            "delete" => self.client_delete(&body, now_millis).await,
            "sendOobCode" | "resetPassword" => {
                not_served("out-of-band email (verification and password-reset links)")
            }
            "sendVerificationCode" | "signInWithPhoneNumber" => not_served("phone authentication"),
            "signInWithIdp" | "createAuthUri" => {
                not_served("federated identity providers")
            }
            _ => self.unknown(path),
        }
    }

    fn project_id(&self) -> &str {
        &self.config.project_id
    }

    fn unknown(&self, path: &str) -> (u16, Json) {
        error_response(
            404,
            &format!("NOT_FOUND : fireup local does not serve {path}"),
        )
    }

    fn wrong_method(&self, method: &str, path: &str, expected: &str) -> (u16, Json) {
        error_response(
            405,
            &format!("INVALID_ARGUMENT : {path} takes {expected}, not {method}"),
        )
    }

    /// Start a session for `user`: an ID token plus a refresh token.
    fn open_session(&self, user: &AuthUser, now_millis: i64) -> Session {
        Session {
            id_token: token::mint_id_token(user, self.project_id(), now_millis / 1000),
            refresh_token: RefreshToken {
                local_id: user.local_id.clone(),
                project_id: self.project_id().to_string(),
                issued_at_millis: now_millis,
            }
            .encode(),
        }
    }

    /// Add the four fields every sign-in response carries.
    fn with_session(&self, mut body: Map<String, Json>, session: &Session) -> Map<String, Json> {
        body.insert("idToken".into(), Json::from(session.id_token.clone()));
        body.insert(
            "refreshToken".into(),
            Json::from(session.refresh_token.clone()),
        );
        // `expiresIn` is seconds, as a string, in the published response.
        body.insert(
            "expiresIn".into(),
            Json::from(token::ID_TOKEN_TTL_SECONDS.to_string()),
        );
        body
    }

    /// Resolve the account an `idToken` names, for the client endpoints.
    async fn user_from_id_token(
        &self,
        body: &Map<String, Json>,
        now_millis: i64,
    ) -> Result<AuthUser, (u16, Json)> {
        let Some(id_token) = string_field(body, "idToken") else {
            return Err(bad_request("MISSING_ID_TOKEN"));
        };

        let claims = token::parse_id_token(&id_token, self.project_id(), now_millis / 1000)
            .map_err(|error| bad_request(error.code()))?;

        match self
            .users
            .get_by_local_id(self.project_id(), &claims.local_id)
            .await
        {
            Ok(Some(user)) if user.disabled => Err(bad_request("USER_DISABLED")),
            Ok(Some(user)) => Ok(user),
            // The token is well-formed but its account is gone. `USER_NOT_FOUND`
            // is the code SDKs turn into `auth/user-not-found`, which is what a
            // client should see after an account is deleted underneath it.
            Ok(None) => Err(bad_request("USER_NOT_FOUND")),
            Err(error) => Err(store_failure("reading the signed-in account", error)),
        }
    }

    // ----- client endpoints -------------------------------------------------

    /// `POST /v1/accounts:signUp` — email+password sign-up, or anonymous
    /// sign-in when neither is given.
    async fn sign_up(&self, body: &Map<String, Json>, now_millis: i64) -> (u16, Json) {
        let email = string_field(body, "email");
        let password = string_field(body, "password");

        let mut user = match (&email, &password) {
            (Some(email), Some(password)) => {
                if !is_plausible_email(email) {
                    return bad_request("INVALID_EMAIL");
                }
                if password.chars().count() < MIN_PASSWORD_LENGTH {
                    return bad_request(&format!(
                        "WEAK_PASSWORD : Password should be at least {MIN_PASSWORD_LENGTH} characters"
                    ));
                }
                let mut user =
                    AuthUser::new(super::generate_uid(), SignInProvider::Password, now_millis);
                user.set_email(email);
                user.password = Some(PasswordHash::new(password));
                user
            }
            (Some(_), None) => return bad_request("MISSING_PASSWORD"),
            (None, Some(_)) => return bad_request("MISSING_EMAIL"),
            // No credentials at all is an anonymous sign-in, which is how the
            // client SDKs implement `signInAnonymously`.
            (None, None) => {
                AuthUser::new(super::generate_uid(), SignInProvider::Anonymous, now_millis)
            }
        };

        if let Some(display_name) = string_field(body, "displayName") {
            user.display_name = Some(display_name);
        }
        if let Some(photo_url) = string_field(body, "photoUrl") {
            user.photo_url = Some(photo_url);
        }
        user.last_login_at_millis = Some(now_millis);

        match self.users.insert(self.project_id(), user.clone()).await {
            Ok(InsertOutcome::Inserted) => {}
            Ok(InsertOutcome::EmailExists) => return bad_request("EMAIL_EXISTS"),
            // A generated uid that already exists means the generator collided,
            // which is not something a client can fix by retrying differently.
            Ok(InsertOutcome::LocalIdExists) => {
                return error_response(500, "INTERNAL_ERROR : generated uid collided")
            }
            Err(error) => return store_failure("creating the account", error),
        }

        let session = self.open_session(&user, now_millis);
        let mut response = Map::new();
        response.insert(
            "kind".into(),
            Json::from("identitytoolkit#SignupNewUserResponse"),
        );
        response.insert("localId".into(), Json::from(user.local_id.clone()));
        if let Some(email) = &user.email {
            response.insert("email".into(), Json::from(email.clone()));
        }
        if let Some(display_name) = &user.display_name {
            response.insert("displayName".into(), Json::from(display_name.clone()));
        }

        (200, Json::Object(self.with_session(response, &session)))
    }

    /// `POST /v1/accounts:signInWithPassword`.
    async fn sign_in_with_password(
        &self,
        body: &Map<String, Json>,
        now_millis: i64,
    ) -> (u16, Json) {
        let Some(email) = string_field(body, "email") else {
            return bad_request("MISSING_EMAIL");
        };
        let Some(password) = string_field(body, "password") else {
            return bad_request("MISSING_PASSWORD");
        };

        let normalized = AuthUser::normalize_email(&email);
        let found = match self.users.get_by_email(self.project_id(), &normalized).await {
            Ok(found) => found,
            Err(error) => return store_failure("looking the account up by email", error),
        };
        let Some(mut user) = found else {
            return bad_request("EMAIL_NOT_FOUND");
        };

        if user.disabled {
            return bad_request("USER_DISABLED");
        }
        // No stored password means this account cannot sign in this way
        // (anonymous, or created by an admin without one). `INVALID_PASSWORD` and
        // not a distinct code, so the response does not reveal which accounts
        // have passwords.
        let valid = user
            .password
            .as_ref()
            .is_some_and(|stored| stored.verify(&password));
        if !valid {
            return bad_request("INVALID_PASSWORD");
        }

        user.last_login_at_millis = Some(now_millis);
        if let Err(error) = self.users.update(self.project_id(), user.clone()).await {
            return store_failure("recording the sign-in", error);
        }

        let session = self.open_session(&user, now_millis);
        let mut response = Map::new();
        response.insert(
            "kind".into(),
            Json::from("identitytoolkit#VerifyPasswordResponse"),
        );
        response.insert("localId".into(), Json::from(user.local_id.clone()));
        response.insert("registered".into(), Json::from(true));
        if let Some(email) = &user.email {
            response.insert("email".into(), Json::from(email.clone()));
        }
        if let Some(display_name) = &user.display_name {
            response.insert("displayName".into(), Json::from(display_name.clone()));
        }
        if let Some(photo_url) = &user.photo_url {
            response.insert("profilePicture".into(), Json::from(photo_url.clone()));
        }

        (200, Json::Object(self.with_session(response, &session)))
    }

    /// `POST /v1/accounts:signInWithCustomToken`.
    ///
    /// Creates the account if the token names a uid that does not exist yet,
    /// which is the documented behaviour of custom-token sign-in and what makes
    /// `admin.auth().createCustomToken(uid)` usable for a brand-new uid.
    async fn sign_in_with_custom_token(
        &self,
        body: &Map<String, Json>,
        now_millis: i64,
    ) -> (u16, Json) {
        let Some(custom_token) = string_field(body, "token") else {
            return bad_request("MISSING_CUSTOM_TOKEN");
        };

        let claims = match token::parse_custom_token(&custom_token, now_millis / 1000) {
            Ok(claims) => claims,
            Err(TokenError::Expired) => return bad_request("TOKEN_EXPIRED"),
            Err(_) => return bad_request("INVALID_CUSTOM_TOKEN"),
        };

        let existing = match self
            .users
            .get_by_local_id(self.project_id(), &claims.uid)
            .await
        {
            Ok(existing) => existing,
            Err(error) => return store_failure("looking the account up by uid", error),
        };

        let is_new_user = existing.is_none();
        let mut user = existing.unwrap_or_else(|| {
            AuthUser::new(claims.uid.clone(), SignInProvider::Custom, now_millis)
        });

        if user.disabled {
            return bad_request("USER_DISABLED");
        }
        // Claims in the token are for this session, so they replace whatever was
        // stored: that is how an Admin SDK changes a user's claims without an
        // admin update call.
        if let Some(extra) = claims.claims {
            if let Err(reason) = token::validate_custom_claims(&extra) {
                return bad_request(&format!("INVALID_CLAIMS : {reason}"));
            }
            user.custom_claims = Some(extra);
        }
        user.last_login_at_millis = Some(now_millis);

        let stored = if is_new_user {
            self.users
                .insert(self.project_id(), user.clone())
                .await
                .map(|outcome| outcome == InsertOutcome::Inserted)
        } else {
            self.users
                .update(self.project_id(), user.clone())
                .await
                .map(|outcome| outcome == UpdateOutcome::Updated)
        };
        match stored {
            Ok(true) => {}
            Ok(false) => {
                return error_response(
                    500,
                    "INTERNAL_ERROR : the account changed while signing in; retry",
                )
            }
            Err(error) => return store_failure("storing the account", error),
        }

        let session = self.open_session(&user, now_millis);
        let mut response = Map::new();
        response.insert(
            "kind".into(),
            Json::from("identitytoolkit#VerifyCustomTokenResponse"),
        );
        response.insert("isNewUser".into(), Json::from(is_new_user));
        (200, Json::Object(self.with_session(response, &session)))
    }

    /// `POST /v1/accounts:lookup` — the signed-in account.
    async fn client_lookup(&self, body: &Map<String, Json>, now_millis: i64) -> (u16, Json) {
        let user = match self.user_from_id_token(body, now_millis).await {
            Ok(user) => user,
            Err(response) => return response,
        };

        (
            200,
            json!({
                "kind": "identitytoolkit#GetAccountInfoResponse",
                "users": [user.to_user_info()],
            }),
        )
    }

    /// `POST /v1/accounts:update` — the signed-in account changes itself.
    async fn client_update(&self, body: &Map<String, Json>, now_millis: i64) -> (u16, Json) {
        let user = match self.user_from_id_token(body, now_millis).await {
            Ok(user) => user,
            Err(response) => return response,
        };
        self.apply_update(user, body, now_millis, false).await
    }

    /// `POST /v1/accounts:delete` — the signed-in account deletes itself.
    async fn client_delete(&self, body: &Map<String, Json>, now_millis: i64) -> (u16, Json) {
        let user = match self.user_from_id_token(body, now_millis).await {
            Ok(user) => user,
            Err(response) => return response,
        };

        match self.users.delete(self.project_id(), &user.local_id).await {
            Ok(_) => (
                200,
                json!({ "kind": "identitytoolkit#DeleteAccountResponse" }),
            ),
            Err(error) => store_failure("deleting the account", error),
        }
    }

    /// `POST /v1/token` — exchange a refresh token for a fresh ID token.
    ///
    /// Lives on the `securetoken.googleapis.com` host, takes form encoding
    /// rather than JSON, and answers with snake_case OAuth field names. All
    /// three differ from every other endpoint here, because that is what the
    /// published contract says.
    async fn refresh(
        &self,
        method: &str,
        request: &AuthRequest<'_>,
        now_millis: i64,
    ) -> (u16, Json) {
        if method != "POST" {
            return self.wrong_method(method, "/v1/token", "POST");
        }

        // The JS SDK form-encodes this body; some tooling sends JSON. Accepting
        // both costs one branch and saves a confusing `MISSING_REFRESH_TOKEN`.
        let body = match parse_body(request.body) {
            Ok(body) if !body.is_empty() => body,
            _ => parse_pairs(&String::from_utf8_lossy(request.body)),
        };

        if let Some(grant_type) = string_field(&body, "grant_type") {
            if grant_type != "refresh_token" {
                return bad_request(&format!(
                    "INVALID_GRANT_TYPE : {grant_type} is not supported; use refresh_token"
                ));
            }
        }

        let Some(encoded) = string_field(&body, "refresh_token")
            .or_else(|| string_field(&body, "refreshToken"))
        else {
            return bad_request("MISSING_REFRESH_TOKEN");
        };

        let refresh = match RefreshToken::decode(&encoded) {
            Ok(refresh) => refresh,
            Err(_) => return bad_request("INVALID_REFRESH_TOKEN"),
        };
        if refresh.project_id != self.project_id() {
            return bad_request("INVALID_REFRESH_TOKEN");
        }

        let found = match self
            .users
            .get_by_local_id(self.project_id(), &refresh.local_id)
            .await
        {
            Ok(found) => found,
            Err(error) => return store_failure("reading the account to refresh", error),
        };
        let Some(user) = found else {
            return bad_request("USER_NOT_FOUND");
        };
        if user.disabled {
            return bad_request("USER_DISABLED");
        }
        // This is what revocation means: `validSince` moves forward and every
        // refresh token issued before it stops working.
        if refresh.issued_at_millis < user.valid_since_millis {
            return bad_request("TOKEN_EXPIRED");
        }

        let session = self.open_session(&user, now_millis);
        (
            200,
            json!({
                "access_token": session.id_token,
                "id_token": session.id_token,
                "refresh_token": session.refresh_token,
                "expires_in": token::ID_TOKEN_TTL_SECONDS.to_string(),
                "token_type": "Bearer",
                "user_id": user.local_id,
                "project_id": self.project_id(),
            }),
        )
    }

    // ----- admin endpoints --------------------------------------------------

    /// Routes under `/v1/projects/{project}/...`.
    async fn route_admin(
        &self,
        method: &str,
        rest: &str,
        request: &AuthRequest<'_>,
        now_millis: i64,
    ) -> (u16, Json) {
        let (project, tail) = rest.split_once('/').unwrap_or((rest, ""));
        if let Some(response) = self.check_project(project) {
            return response;
        }

        // `accounts:batchGet` is a GET and takes its parameters in the query
        // string; everything else is a POST with a JSON body. Merging the two
        // into one map — query last, so it wins — means each handler reads its
        // parameters from one place regardless of how they arrived.
        let mut body = match parse_body(request.body) {
            Ok(body) => body,
            Err(response) => return response,
        };
        body.extend(parse_pairs(request.query));

        match (method, tail) {
            ("POST", "accounts") => self.admin_create(&body, now_millis).await,
            ("POST", "accounts:lookup") => self.admin_lookup(&body).await,
            ("POST", "accounts:update") => self.admin_update(&body, now_millis).await,
            ("POST", "accounts:delete") => self.admin_delete(&body).await,
            ("GET", "accounts:batchGet") => self.admin_batch_get(&body).await,
            ("POST", "accounts:batchDelete") => {
                not_served("batch account deletion (delete accounts one at a time)")
            }
            (_, "accounts") => self.wrong_method(method, "accounts", "POST"),
            (_, tail) if tail.starts_with("accounts:") => {
                self.wrong_method(method, tail, "POST")
            }
            _ => self.unknown(&format!("/v1/projects/{project}/{tail}")),
        }
    }

    /// Admin paths name a project; serving a different one would silently give a
    /// developer an empty, parallel user directory.
    fn check_project(&self, project: &str) -> Option<(u16, Json)> {
        if project == self.project_id() {
            return None;
        }
        Some(error_response(
            404,
            &format!(
                "PROJECT_NOT_FOUND : this fireup local serves project {:?}, not {project:?}; \
                 start it with --project-id {project} or point the SDK at {:?}",
                self.project_id(),
                self.project_id()
            ),
        ))
    }

    /// `POST /v1/projects/{p}/accounts` — admin account creation.
    async fn admin_create(&self, body: &Map<String, Json>, now_millis: i64) -> (u16, Json) {
        let email = string_field(body, "email");
        if let Some(email) = &email {
            if !is_plausible_email(email) {
                return bad_request("INVALID_EMAIL");
            }
        }

        let password = string_field(body, "password");
        if let Some(password) = &password {
            if password.chars().count() < MIN_PASSWORD_LENGTH {
                return bad_request(&format!(
                    "WEAK_PASSWORD : Password should be at least {MIN_PASSWORD_LENGTH} characters"
                ));
            }
        }

        let local_id = string_field(body, "localId").unwrap_or_else(super::generate_uid);
        // A uid is a path segment in `/v1/projects/{p}/accounts/{uid}`-shaped
        // URLs and is commonly used as a Firestore document id, so a slash in it
        // would produce resources nobody can address.
        if local_id.contains('/') || local_id.len() > 128 {
            return bad_request(
                "INVALID_LOCAL_ID : localId must be at most 128 characters and contain no '/'",
            );
        }

        let provider = if password.is_some() {
            SignInProvider::Password
        } else {
            SignInProvider::Anonymous
        };
        let mut user = AuthUser::new(local_id, provider, now_millis);
        if let Some(email) = &email {
            user.set_email(email);
        }
        if let Some(password) = &password {
            user.password = Some(PasswordHash::new(password));
        }
        user.display_name = string_field(body, "displayName");
        user.photo_url = string_field(body, "photoUrl");
        user.email_verified = bool_field(body, "emailVerified").unwrap_or(false);
        user.disabled = bool_field(body, "disabled")
            .or_else(|| bool_field(body, "disableUser"))
            .unwrap_or(false);

        if let Some(raw_claims) = string_field(body, "customAttributes") {
            match self.parse_custom_attributes(&raw_claims) {
                Ok(claims) => user.custom_claims = claims,
                Err(response) => return response,
            }
        }

        match self.users.insert(self.project_id(), user.clone()).await {
            Ok(InsertOutcome::Inserted) => {}
            Ok(InsertOutcome::EmailExists) => return bad_request("EMAIL_EXISTS"),
            Ok(InsertOutcome::LocalIdExists) => return bad_request("DUPLICATE_LOCAL_ID"),
            Err(error) => return store_failure("creating the account", error),
        }

        let mut response = Map::new();
        response.insert(
            "kind".into(),
            Json::from("identitytoolkit#SignupNewUserResponse"),
        );
        response.insert("localId".into(), Json::from(user.local_id.clone()));
        if let Some(email) = &user.email {
            response.insert("email".into(), Json::from(email.clone()));
        }
        (200, Json::Object(response))
    }

    /// `POST /v1/projects/{p}/accounts:lookup` — by uid or email.
    ///
    /// Missing accounts are omitted rather than reported: the published response
    /// carries only the accounts that were found, and Admin SDKs compare the
    /// returned list against what they asked for.
    async fn admin_lookup(&self, body: &Map<String, Json>) -> (u16, Json) {
        let local_ids = string_list(body, "localId");
        let emails = string_list(body, "email");
        if local_ids.is_empty() && emails.is_empty() {
            return bad_request("MISSING_LOCAL_ID : give at least one localId or email");
        }

        let mut found: Vec<Json> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for local_id in &local_ids {
            match self.users.get_by_local_id(self.project_id(), local_id).await {
                Ok(Some(user)) => {
                    if seen.insert(user.local_id.clone()) {
                        found.push(user.to_user_info());
                    }
                }
                Ok(None) => {}
                Err(error) => return store_failure("looking accounts up by uid", error),
            }
        }
        for email in &emails {
            let normalized = AuthUser::normalize_email(email);
            match self.users.get_by_email(self.project_id(), &normalized).await {
                Ok(Some(user)) => {
                    if seen.insert(user.local_id.clone()) {
                        found.push(user.to_user_info());
                    }
                }
                Ok(None) => {}
                Err(error) => return store_failure("looking accounts up by email", error),
            }
        }

        let mut response = Map::new();
        response.insert(
            "kind".into(),
            Json::from("identitytoolkit#GetAccountInfoResponse"),
        );
        // `users` is omitted when nothing matched, which is how Admin SDKs
        // detect "no such user".
        if !found.is_empty() {
            response.insert("users".into(), Json::Array(found));
        }
        (200, Json::Object(response))
    }

    /// `POST /v1/projects/{p}/accounts:update`.
    async fn admin_update(&self, body: &Map<String, Json>, now_millis: i64) -> (u16, Json) {
        let Some(local_id) = string_field(body, "localId") else {
            return bad_request("MISSING_LOCAL_ID");
        };

        let found = match self.users.get_by_local_id(self.project_id(), &local_id).await {
            Ok(found) => found,
            Err(error) => return store_failure("reading the account to update", error),
        };
        let Some(user) = found else {
            return bad_request("USER_NOT_FOUND");
        };

        self.apply_update(user, body, now_millis, true).await
    }

    /// `POST /v1/projects/{p}/accounts:delete`.
    async fn admin_delete(&self, body: &Map<String, Json>) -> (u16, Json) {
        let Some(local_id) = string_field(body, "localId") else {
            return bad_request("MISSING_LOCAL_ID");
        };

        match self.users.delete(self.project_id(), &local_id).await {
            // Reporting a missing account rather than succeeding silently: an
            // Admin SDK's `deleteUser` is documented to reject an unknown uid.
            Ok(false) => bad_request("USER_NOT_FOUND"),
            Ok(true) => (
                200,
                json!({ "kind": "identitytoolkit#DeleteAccountResponse" }),
            ),
            Err(error) => store_failure("deleting the account", error),
        }
    }

    /// `GET /v1/projects/{p}/accounts:batchGet` — paged account listing.
    ///
    /// Takes its parameters from the query string, which the HTTP layer has
    /// already parsed into `body` for this route.
    async fn admin_batch_get(&self, body: &Map<String, Json>) -> (u16, Json) {
        let requested = int_field(body, "maxResults")
            .filter(|size| *size > 0)
            .map(|size| size as usize)
            .unwrap_or(DEFAULT_PAGE_SIZE);
        let page_size = requested.min(MAX_PAGE_SIZE);
        let cursor = string_field(body, "nextPageToken");

        // One more than the page, so the presence of a next page is known
        // without a second query.
        let mut page = match self
            .users
            .list(self.project_id(), cursor.as_deref(), page_size + 1)
            .await
        {
            Ok(page) => page,
            Err(error) => return store_failure("listing accounts", error),
        };

        let has_more = page.len() > page_size;
        page.truncate(page_size);

        let mut response = Map::new();
        response.insert(
            "kind".into(),
            Json::from("identitytoolkit#DownloadAccountResponse"),
        );
        if has_more {
            if let Some(last) = page.last() {
                response.insert(
                    "nextPageToken".into(),
                    Json::from(last.local_id.clone()),
                );
            }
        }
        response.insert(
            "users".into(),
            Json::Array(page.iter().map(AuthUser::to_user_info).collect()),
        );
        (200, Json::Object(response))
    }

    // ----- emulator-control endpoints ---------------------------------------

    /// Routes under `/emulator/v1/projects/{project}/...`.
    ///
    /// These are the publicly documented local-only control endpoints, not part
    /// of the production API.
    async fn route_emulator(&self, method: &str, rest: &str) -> (u16, Json) {
        let (project, tail) = rest.split_once('/').unwrap_or((rest, ""));
        if let Some(response) = self.check_project(project) {
            return response;
        }

        match (method, tail) {
            ("DELETE", "accounts") => match self.users.delete_all(self.project_id()).await {
                // The documented response is an empty object; the count is
                // Fireup's own addition, so a developer can see it worked.
                Ok(removed) => (200, json!({ "removedAccounts": removed })),
                Err(error) => store_failure("clearing accounts", error),
            },
            ("GET" | "HEAD", "config") => (
                200,
                json!({
                    "signIn": { "allowDuplicateEmails": false },
                    "usageMode": "DEFAULT",
                    "name": format!("projects/{}/config", self.project_id()),
                }),
            ),
            ("GET" | "HEAD", "accounts") => self.admin_batch_get(&Map::new()).await,
            (_, "accounts") => self.wrong_method(method, "accounts", "DELETE or GET"),
            _ => self.unknown(&format!("/emulator/v1/projects/{project}/{tail}")),
        }
    }

    // ----- shared update logic ----------------------------------------------

    /// Apply an update to `user`, from either the client or the admin endpoint.
    ///
    /// One function for both so the two cannot disagree about what
    /// `deleteAttribute` means or when a password change revokes sessions. The
    /// `admin` flag gates only the fields a client must not be able to set about
    /// itself — being disabled, having its email marked verified, and its custom
    /// claims.
    async fn apply_update(
        &self,
        mut user: AuthUser,
        body: &Map<String, Json>,
        now_millis: i64,
        admin: bool,
    ) -> (u16, Json) {
        // `deleteAttribute` clears fields by name, and is applied first so a
        // request that both deletes and sets an attribute ends up with it set.
        for attribute in string_list(body, "deleteAttribute") {
            match attribute.as_str() {
                "DISPLAY_NAME" => user.display_name = None,
                "PHOTO_URL" => user.photo_url = None,
                "PASSWORD" => user.password = None,
                "EMAIL" => {
                    user.email = None;
                    user.email_verified = false;
                }
                other => {
                    return bad_request(&format!(
                        "INVALID_ARGUMENT : deleteAttribute {other} is not supported"
                    ))
                }
            }
        }

        if let Some(display_name) = string_field(body, "displayName") {
            user.display_name = Some(display_name);
        }
        if let Some(photo_url) = string_field(body, "photoUrl") {
            user.photo_url = Some(photo_url);
        }

        let mut revoke_sessions = false;

        if let Some(email) = string_field(body, "email") {
            if !is_plausible_email(&email) {
                return bad_request("INVALID_EMAIL");
            }
            let normalized = AuthUser::normalize_email(&email);
            if user.email.as_deref() != Some(normalized.as_str()) {
                user.email = Some(normalized);
                // A new email is unverified until something verifies it, unless
                // this same request says otherwise (handled below).
                user.email_verified = false;
                revoke_sessions = true;
            }
        }

        if let Some(password) = string_field(body, "password") {
            if password.chars().count() < MIN_PASSWORD_LENGTH {
                return bad_request(&format!(
                    "WEAK_PASSWORD : Password should be at least {MIN_PASSWORD_LENGTH} characters"
                ));
            }
            user.password = Some(PasswordHash::new(&password));
            if user.provider == SignInProvider::Anonymous {
                // An anonymous account that gains a password has been upgraded
                // to a permanent one, which is the documented result of linking
                // an email credential.
                user.provider = SignInProvider::Password;
            }
            revoke_sessions = true;
        }

        if admin {
            if let Some(verified) = bool_field(body, "emailVerified") {
                user.email_verified = verified;
            }
            if let Some(disabled) = bool_field(body, "disableUser").or_else(|| bool_field(body, "disabled")) {
                user.disabled = disabled;
            }
            if let Some(raw_claims) = body.get("customAttributes").and_then(Json::as_str) {
                match self.parse_custom_attributes(raw_claims) {
                    Ok(claims) => user.custom_claims = claims,
                    Err(response) => return response,
                }
            }
            if let Some(valid_since_seconds) = int_field(body, "validSince") {
                user.valid_since_millis = valid_since_seconds.saturating_mul(1000);
                revoke_sessions = false; // the caller said exactly when
            }
        } else {
            // Spell out why these are ignored rather than silently dropping
            // them: a client that could disable accounts or set its own claims
            // would be a privilege escalation, local-only or not.
            for privileged in ["disableUser", "disabled", "customAttributes", "emailVerified"] {
                if body.contains_key(privileged) {
                    return error_response(
                        403,
                        &format!(
                            "PERMISSION_DENIED : {privileged} can only be set through \
                             /v1/projects/{{project}}/accounts:update",
                        ),
                    );
                }
            }
        }

        if revoke_sessions {
            // A credential change invalidates existing refresh tokens, which is
            // what `validSince` is for.
            user.valid_since_millis = now_millis;
        }

        match self.users.update(self.project_id(), user.clone()).await {
            Ok(UpdateOutcome::Updated) => {}
            Ok(UpdateOutcome::NotFound) => return bad_request("USER_NOT_FOUND"),
            Ok(UpdateOutcome::EmailExists) => return bad_request("EMAIL_EXISTS"),
            Err(error) => return store_failure("updating the account", error),
        }

        let mut response = Map::new();
        response.insert(
            "kind".into(),
            Json::from("identitytoolkit#SetAccountInfoResponse"),
        );
        response.insert("localId".into(), Json::from(user.local_id.clone()));
        if let Some(email) = &user.email {
            response.insert("email".into(), Json::from(email.clone()));
        }
        if let Some(display_name) = &user.display_name {
            response.insert("displayName".into(), Json::from(display_name.clone()));
        }
        if let Some(photo_url) = &user.photo_url {
            response.insert("photoUrl".into(), Json::from(photo_url.clone()));
        }
        if let Some(password) = &user.password {
            response.insert("passwordHash".into(), Json::from(password.hash.clone()));
        }
        response.insert(
            "emailVerified".into(),
            Json::from(user.email_verified),
        );
        let providers = user.provider_user_info();
        if !providers.is_empty() {
            response.insert("providerUserInfo".into(), Json::Array(providers));
        }

        // A client that asked for a new session gets one, so it is not left
        // holding a token the credential change just invalidated.
        if bool_field(body, "returnSecureToken").unwrap_or(false) && !user.disabled {
            let session = self.open_session(&user, now_millis);
            return (200, Json::Object(self.with_session(response, &session)));
        }
        (200, Json::Object(response))
    }

    /// Parse `customAttributes`, which the API carries as a JSON **string**.
    ///
    /// An empty string clears the claims — that is how an Admin SDK removes
    /// them — so it maps to `None` rather than to an empty object.
    fn parse_custom_attributes(
        &self,
        raw: &str,
    ) -> Result<Option<Json>, (u16, Json)> {
        if raw.trim().is_empty() {
            return Ok(None);
        }
        let parsed: Json = serde_json::from_str(raw).map_err(|error| {
            bad_request(&format!(
                "INVALID_CLAIMS : customAttributes must be a JSON object: {error}"
            ))
        })?;
        token::validate_custom_claims(&parsed)
            .map_err(|reason| bad_request(&format!("INVALID_CLAIMS : {reason}")))?;
        Ok(Some(parsed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::auth::store::{FailingUserStore, MemoryUserStore};

    const NOW: i64 = 1_700_000_000_000;

    fn api() -> AuthApi {
        let config = Arc::new(
            LocalConfig::in_memory("127.0.0.1:4589".parse().expect("addr"))
                .with_project_id("demo"),
        );
        AuthApi::new(config, Arc::new(MemoryUserStore::new()))
    }

    fn failing_api() -> AuthApi {
        let config = Arc::new(
            LocalConfig::in_memory("127.0.0.1:4589".parse().expect("addr"))
                .with_project_id("demo"),
        );
        AuthApi::new(config, Arc::new(FailingUserStore))
    }

    /// `POST <path>` with a JSON body.
    async fn post(api: &AuthApi, path: &str, body: Json) -> (u16, Json) {
        post_at(api, path, body, NOW).await
    }

    async fn post_at(api: &AuthApi, path: &str, body: Json, now: i64) -> (u16, Json) {
        let raw = body.to_string();
        api.route_at(
            &AuthRequest {
                method: "POST",
                path,
                query: "",
                body: raw.as_bytes(),
            },
            now,
        )
        .await
    }

    async fn request(api: &AuthApi, method: &str, path: &str, body: &str) -> (u16, Json) {
        api.route_at(
            &AuthRequest {
                method,
                path,
                query: "",
                body: body.as_bytes(),
            },
            NOW,
        )
        .await
    }

    fn message(body: &Json) -> String {
        body["error"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("not an error envelope: {body}"))
            .to_string()
    }

    async fn signed_up(api: &AuthApi, email: &str, password: &str) -> Json {
        let (status, body) = post(
            api,
            "/v1/accounts:signUp",
            json!({ "email": email, "password": password, "returnSecureToken": true }),
        )
        .await;
        assert_eq!(status, 200, "sign-up failed: {body}");
        body
    }

    // ----- path handling ----------------------------------------------------

    #[test]
    fn the_sdk_prefixed_and_bare_paths_are_the_same_route() {
        assert_eq!(
            normalize_path("/identitytoolkit.googleapis.com/v1/accounts:signUp"),
            "/v1/accounts:signUp"
        );
        assert_eq!(
            normalize_path("/securetoken.googleapis.com/v1/token"),
            "/v1/token"
        );
        assert_eq!(normalize_path("/v1/accounts:signUp"), "/v1/accounts:signUp");
    }

    #[test]
    fn the_auth_surface_claims_its_paths_and_leaves_the_others_alone() {
        for mine in [
            "/identitytoolkit.googleapis.com/v1/accounts:signUp",
            "/securetoken.googleapis.com/v1/token",
            "/v1/accounts:lookup",
            "/v1/token",
            "/v1/projects/demo/accounts",
            "/emulator/v1/projects/demo/accounts",
        ] {
            assert!(handles(mine), "{mine} must route to auth");
        }
        for theirs in ["/", "/health", "/nope", "/v1"] {
            assert!(!handles(theirs), "{theirs} must not route to auth");
        }
    }

    #[tokio::test]
    async fn an_unserved_endpoint_says_so_instead_of_404ing_vaguely() {
        let api = api();

        let (status, body) = post(&api, "/v1/accounts:sendOobCode", json!({})).await;
        assert_eq!(status, 501);
        assert!(message(&body).contains("out-of-band"), "{body}");

        let (status, body) = post(&api, "/v1/accounts:sendVerificationCode", json!({})).await;
        assert_eq!(status, 501);
        assert!(message(&body).contains("phone"), "{body}");

        let (status, body) = post(
            &api,
            "/identitytoolkit.googleapis.com/v2/projects/demo/mfaEnrollment:start",
            json!({}),
        )
        .await;
        assert_eq!(status, 501);
        assert!(message(&body).contains("MFA"), "{body}");
    }

    #[tokio::test]
    async fn a_get_to_a_post_endpoint_is_405_not_a_silent_success() {
        let api = api();
        let (status, body) = request(&api, "GET", "/v1/accounts:signUp", "").await;
        assert_eq!(status, 405);
        assert!(message(&body).contains("POST"), "{body}");
    }

    #[tokio::test]
    async fn admin_paths_for_another_project_are_refused_with_both_ids() {
        let api = api();
        let (status, body) = post(&api, "/v1/projects/other/accounts", json!({})).await;

        assert_eq!(status, 404);
        let message = message(&body);
        assert!(message.contains("\"demo\""), "{message}");
        assert!(message.contains("\"other\""), "{message}");
    }

    // ----- sign-up ----------------------------------------------------------

    #[tokio::test]
    async fn signing_up_returns_a_session_and_a_usable_token() {
        let api = api();
        let body = signed_up(&api, "ada@example.com", "secret12").await;

        assert_eq!(body["kind"], "identitytoolkit#SignupNewUserResponse");
        assert_eq!(body["email"], "ada@example.com");
        assert_eq!(body["expiresIn"], "3600");
        let local_id = body["localId"].as_str().expect("a localId");
        assert_eq!(local_id.len(), super::super::GENERATED_UID_LENGTH);

        let claims = token::parse_id_token(
            body["idToken"].as_str().expect("an idToken"),
            "demo",
            NOW / 1000,
        )
        .expect("the minted token must parse");
        assert_eq!(claims.local_id, local_id);
    }

    #[tokio::test]
    async fn signing_up_with_no_credentials_creates_an_anonymous_account() {
        let api = api();
        let (status, body) = post(&api, "/v1/accounts:signUp", json!({})).await;

        assert_eq!(status, 200, "{body}");
        assert!(body.get("email").is_none(), "{body}");

        let (_, looked_up) = post(
            &api,
            "/v1/accounts:lookup",
            json!({ "idToken": body["idToken"] }),
        )
        .await;
        assert!(looked_up["users"][0].get("email").is_none(), "{looked_up}");
    }

    #[tokio::test]
    async fn several_anonymous_sign_ups_succeed() {
        // They all have no email, so an email-uniqueness check that treated
        // "absent" as a value would break the second one.
        let api = api();
        let mut ids = std::collections::HashSet::new();
        for _ in 0..3 {
            let (status, body) = post(&api, "/v1/accounts:signUp", json!({})).await;
            assert_eq!(status, 200, "{body}");
            ids.insert(body["localId"].as_str().expect("localId").to_string());
        }
        assert_eq!(ids.len(), 3);
    }

    #[tokio::test]
    async fn sign_up_rejects_what_identity_toolkit_rejects() {
        let api = api();
        signed_up(&api, "taken@example.com", "secret12").await;

        for (body, expected) in [
            (json!({ "email": "taken@example.com", "password": "secret12" }), "EMAIL_EXISTS"),
            (json!({ "email": "not-an-email", "password": "secret12" }), "INVALID_EMAIL"),
            (json!({ "email": "new@example.com" }), "MISSING_PASSWORD"),
            (json!({ "password": "secret12" }), "MISSING_EMAIL"),
        ] {
            let (status, response) = post(&api, "/v1/accounts:signUp", body.clone()).await;
            assert_eq!(status, 400, "{body} -> {response}");
            assert_eq!(message(&response), expected, "for {body}");
        }

        let (status, response) = post(
            &api,
            "/v1/accounts:signUp",
            json!({ "email": "short@example.com", "password": "12345" }),
        )
        .await;
        assert_eq!(status, 400);
        assert!(
            message(&response).starts_with("WEAK_PASSWORD : "),
            "the detail after the code is what SDKs surface: {response}"
        );
    }

    #[tokio::test]
    async fn an_email_differing_only_in_case_is_the_same_account() {
        let api = api();
        signed_up(&api, "Ada@Example.com", "secret12").await;

        let (status, body) = post(
            &api,
            "/v1/accounts:signUp",
            json!({ "email": "ada@example.com", "password": "other123" }),
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(message(&body), "EMAIL_EXISTS");

        let (status, body) = post(
            &api,
            "/v1/accounts:signInWithPassword",
            json!({ "email": "ADA@EXAMPLE.COM", "password": "secret12" }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
    }

    // ----- sign-in ----------------------------------------------------------

    #[tokio::test]
    async fn signing_in_with_the_right_password_works_and_records_the_login() {
        let api = api();
        let signed_up = signed_up(&api, "ada@example.com", "secret12").await;

        let (status, body) = post_at(
            &api,
            "/v1/accounts:signInWithPassword",
            json!({ "email": "ada@example.com", "password": "secret12" }),
            NOW + 5_000,
        )
        .await;

        assert_eq!(status, 200, "{body}");
        assert_eq!(body["kind"], "identitytoolkit#VerifyPasswordResponse");
        assert_eq!(body["registered"], true);
        assert_eq!(body["localId"], signed_up["localId"]);

        let (_, looked_up) = post_at(
            &api,
            "/v1/accounts:lookup",
            json!({ "idToken": body["idToken"] }),
            NOW + 5_000,
        )
        .await;
        assert_eq!(
            looked_up["users"][0]["lastLoginAt"],
            (NOW + 5_000).to_string()
        );
    }

    #[tokio::test]
    async fn sign_in_failures_use_the_codes_sdks_match_on() {
        let api = api();
        signed_up(&api, "ada@example.com", "secret12").await;

        for (body, expected) in [
            (
                json!({ "email": "ada@example.com", "password": "wrong-one" }),
                "INVALID_PASSWORD",
            ),
            (
                json!({ "email": "nobody@example.com", "password": "secret12" }),
                "EMAIL_NOT_FOUND",
            ),
            (json!({ "password": "secret12" }), "MISSING_EMAIL"),
            (json!({ "email": "ada@example.com" }), "MISSING_PASSWORD"),
        ] {
            let (status, response) =
                post(&api, "/v1/accounts:signInWithPassword", body.clone()).await;
            assert_eq!(status, 400, "{body} -> {response}");
            assert_eq!(message(&response), expected, "for {body}");
        }
    }

    #[tokio::test]
    async fn an_anonymous_account_cannot_be_signed_into_with_a_password() {
        // It has no stored password; the response must not reveal that, so it is
        // the same code as a wrong password.
        let api = api();
        post(&api, "/v1/accounts:signUp", json!({})).await;
        let (_, created) = post(
            &api,
            "/v1/projects/demo/accounts",
            json!({ "localId": "no-pw", "email": "nopw@example.com" }),
        )
        .await;
        assert_eq!(created["localId"], "no-pw");

        let (status, body) = post(
            &api,
            "/v1/accounts:signInWithPassword",
            json!({ "email": "nopw@example.com", "password": "anything" }),
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(message(&body), "INVALID_PASSWORD");
    }

    #[tokio::test]
    async fn a_disabled_account_cannot_sign_in_or_use_its_token() {
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;
        let local_id = session["localId"].as_str().expect("localId").to_string();

        let (status, body) = post(
            &api,
            "/v1/projects/demo/accounts:update",
            json!({ "localId": local_id, "disableUser": true }),
        )
        .await;
        assert_eq!(status, 200, "{body}");

        let (status, body) = post(
            &api,
            "/v1/accounts:signInWithPassword",
            json!({ "email": "ada@example.com", "password": "secret12" }),
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(message(&body), "USER_DISABLED");

        // The token it already holds must stop working too.
        let (status, body) = post(
            &api,
            "/v1/accounts:lookup",
            json!({ "idToken": session["idToken"] }),
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(message(&body), "USER_DISABLED");
    }

    #[tokio::test]
    async fn a_custom_token_signs_in_and_creates_the_account_it_names() {
        let api = api();
        let custom = token::mint_custom_token(
            "from-admin",
            Some(&json!({ "role": "admin" })),
            NOW / 1000,
        );

        let (status, body) =
            post(&api, "/v1/accounts:signInWithCustomToken", json!({ "token": custom })).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["kind"], "identitytoolkit#VerifyCustomTokenResponse");
        assert_eq!(body["isNewUser"], true);

        // The claims from the token must be in the minted ID token.
        let id_token = body["idToken"].as_str().expect("idToken");
        let payload = id_token.split('.').nth(1).expect("claims segment");
        let claims: Json = serde_json::from_slice(
            &super::super::base64url_decode(payload).expect("base64url"),
        )
        .expect("JSON claims");
        assert_eq!(claims["role"], "admin");
        assert_eq!(claims["sub"], "from-admin");

        // Signing in again finds the account rather than creating it twice.
        let (status, body) =
            post(&api, "/v1/accounts:signInWithCustomToken", json!({ "token": custom })).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["isNewUser"], false);
    }

    #[tokio::test]
    async fn an_id_token_is_not_accepted_as_a_custom_token() {
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;

        let (status, body) = post(
            &api,
            "/v1/accounts:signInWithCustomToken",
            json!({ "token": session["idToken"] }),
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(message(&body), "INVALID_CUSTOM_TOKEN");
    }

    // ----- lookup, update, delete (client) ----------------------------------

    #[tokio::test]
    async fn lookup_needs_a_valid_token_for_this_project() {
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;

        let (status, body) = post(&api, "/v1/accounts:lookup", json!({})).await;
        assert_eq!(status, 400);
        assert_eq!(message(&body), "MISSING_ID_TOKEN");

        let (status, body) =
            post(&api, "/v1/accounts:lookup", json!({ "idToken": "garbage" })).await;
        assert_eq!(status, 400);
        assert_eq!(message(&body), "INVALID_ID_TOKEN");

        // An hour later the token is dead.
        let (status, body) = post_at(
            &api,
            "/v1/accounts:lookup",
            json!({ "idToken": session["idToken"] }),
            NOW + token::ID_TOKEN_TTL_SECONDS * 1000,
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(message(&body), "TOKEN_EXPIRED");
    }

    #[tokio::test]
    async fn a_client_can_change_its_own_profile_and_password() {
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;

        let (status, body) = post(
            &api,
            "/v1/accounts:update",
            json!({
                "idToken": session["idToken"],
                "displayName": "Ada L",
                "photoUrl": "https://example.com/a.png",
                "password": "new-secret",
                "returnSecureToken": true,
            }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["displayName"], "Ada L");
        assert!(body["idToken"].is_string(), "a new session was requested");

        // The new password works and the old one does not.
        let (status, _) = post(
            &api,
            "/v1/accounts:signInWithPassword",
            json!({ "email": "ada@example.com", "password": "new-secret" }),
        )
        .await;
        assert_eq!(status, 200);
        let (status, body) = post(
            &api,
            "/v1/accounts:signInWithPassword",
            json!({ "email": "ada@example.com", "password": "secret12" }),
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(message(&body), "INVALID_PASSWORD");
    }

    #[tokio::test]
    async fn a_client_cannot_make_itself_an_admin() {
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;

        for privileged in [
            json!({ "customAttributes": "{\"admin\":true}" }),
            json!({ "disableUser": false }),
            json!({ "emailVerified": true }),
        ] {
            let mut body = privileged.as_object().expect("object").clone();
            body.insert("idToken".into(), session["idToken"].clone());

            let (status, response) =
                post(&api, "/v1/accounts:update", Json::Object(body)).await;
            assert_eq!(status, 403, "{privileged} must be refused: {response}");
            assert!(message(&response).starts_with("PERMISSION_DENIED"), "{response}");
        }
    }

    #[tokio::test]
    async fn delete_attribute_clears_fields_by_name() {
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;
        post(
            &api,
            "/v1/accounts:update",
            json!({ "idToken": session["idToken"], "displayName": "Ada" }),
        )
        .await;

        let (status, body) = post(
            &api,
            "/v1/accounts:update",
            json!({
                "idToken": session["idToken"],
                "deleteAttribute": ["DISPLAY_NAME"],
            }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert!(body.get("displayName").is_none(), "{body}");

        let (status, body) = post(
            &api,
            "/v1/accounts:update",
            json!({ "idToken": session["idToken"], "deleteAttribute": ["NONSENSE"] }),
        )
        .await;
        assert_eq!(status, 400);
        assert!(message(&body).contains("NONSENSE"), "{body}");
    }

    #[tokio::test]
    async fn a_client_can_delete_itself_and_its_token_stops_working() {
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;

        let (status, body) = post(
            &api,
            "/v1/accounts:delete",
            json!({ "idToken": session["idToken"] }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["kind"], "identitytoolkit#DeleteAccountResponse");

        let (status, body) = post(
            &api,
            "/v1/accounts:lookup",
            json!({ "idToken": session["idToken"] }),
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(message(&body), "USER_NOT_FOUND");
    }

    // ----- refresh ----------------------------------------------------------

    #[tokio::test]
    async fn a_refresh_token_mints_a_new_id_token_from_a_form_body() {
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;
        let refresh = session["refreshToken"].as_str().expect("refreshToken");

        let form = format!("grant_type=refresh_token&refresh_token={refresh}");
        let (status, body) = api
            .route_at(
                &AuthRequest {
                    method: "POST",
                    path: "/securetoken.googleapis.com/v1/token",
                    query: "",
                    body: form.as_bytes(),
                },
                NOW + 60_000,
            )
            .await;

        assert_eq!(status, 200, "{body}");
        assert_eq!(body["token_type"], "Bearer");
        assert_eq!(body["expires_in"], "3600");
        assert_eq!(body["project_id"], "demo");
        assert_eq!(body["user_id"], session["localId"]);
        assert_eq!(
            body["access_token"], body["id_token"],
            "both names carry the ID token in the published response"
        );

        token::parse_id_token(
            body["id_token"].as_str().expect("id_token"),
            "demo",
            NOW / 1000 + 60,
        )
        .expect("the refreshed token must be usable");
    }

    #[tokio::test]
    async fn a_json_refresh_body_works_too() {
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;

        let (status, body) = post_at(
            &api,
            "/v1/token",
            json!({ "grant_type": "refresh_token", "refreshToken": session["refreshToken"] }),
            NOW + 1_000,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert!(body["id_token"].is_string(), "{body}");
    }

    #[tokio::test]
    async fn refresh_rejects_bad_tokens_and_wrong_grants() {
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;

        let (status, body) = post(&api, "/v1/token", json!({})).await;
        assert_eq!(status, 400);
        assert_eq!(message(&body), "MISSING_REFRESH_TOKEN");

        let (status, body) =
            post(&api, "/v1/token", json!({ "refresh_token": "nonsense" })).await;
        assert_eq!(status, 400);
        assert_eq!(message(&body), "INVALID_REFRESH_TOKEN");

        let (status, body) = post(
            &api,
            "/v1/token",
            json!({
                "grant_type": "authorization_code",
                "refresh_token": session["refreshToken"],
            }),
        )
        .await;
        assert_eq!(status, 400);
        assert!(message(&body).starts_with("INVALID_GRANT_TYPE"), "{body}");
    }

    #[tokio::test]
    async fn changing_a_password_revokes_the_old_refresh_token() {
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;
        let old_refresh = session["refreshToken"].as_str().expect("refreshToken").to_string();

        // A second later, the password changes.
        let (status, _) = post_at(
            &api,
            "/v1/accounts:update",
            json!({ "idToken": session["idToken"], "password": "new-secret" }),
            NOW + 1_000,
        )
        .await;
        assert_eq!(status, 200);

        let (status, body) = post_at(
            &api,
            "/v1/token",
            json!({ "refresh_token": old_refresh }),
            NOW + 2_000,
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(
            message(&body),
            "TOKEN_EXPIRED",
            "a credential change must invalidate existing sessions"
        );
    }

    #[tokio::test]
    async fn an_admin_valid_since_revokes_sessions() {
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;
        let local_id = session["localId"].as_str().expect("localId").to_string();

        let (status, _) = post_at(
            &api,
            "/v1/projects/demo/accounts:update",
            json!({ "localId": local_id, "validSince": ((NOW + 10_000) / 1000).to_string() }),
            NOW + 10_000,
        )
        .await;
        assert_eq!(status, 200);

        let (status, body) = post_at(
            &api,
            "/v1/token",
            json!({ "refresh_token": session["refreshToken"] }),
            NOW + 11_000,
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(message(&body), "TOKEN_EXPIRED");
    }

    // ----- admin ------------------------------------------------------------

    #[tokio::test]
    async fn an_admin_can_create_an_account_with_a_chosen_uid_and_claims() {
        let api = api();
        let (status, body) = post(
            &api,
            "/v1/projects/demo/accounts",
            json!({
                "localId": "chosen-uid",
                "email": "admin-made@example.com",
                "password": "secret12",
                "displayName": "Made",
                "emailVerified": true,
                "customAttributes": "{\"role\":\"editor\"}",
            }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["localId"], "chosen-uid");

        // The claims must reach the ID token after a sign-in.
        let (status, session) = post(
            &api,
            "/v1/accounts:signInWithPassword",
            json!({ "email": "admin-made@example.com", "password": "secret12" }),
        )
        .await;
        assert_eq!(status, 200, "{session}");

        let payload = session["idToken"]
            .as_str()
            .expect("idToken")
            .split('.')
            .nth(1)
            .expect("claims");
        let claims: Json = serde_json::from_slice(
            &super::super::base64url_decode(payload).expect("base64url"),
        )
        .expect("claims JSON");
        assert_eq!(claims["role"], "editor");
        assert_eq!(claims["email_verified"], true);
    }

    #[tokio::test]
    async fn admin_create_refuses_duplicates_and_bad_input() {
        let api = api();
        post(
            &api,
            "/v1/projects/demo/accounts",
            json!({ "localId": "taken", "email": "taken@example.com" }),
        )
        .await;

        for (body, expected) in [
            (json!({ "localId": "taken" }), "DUPLICATE_LOCAL_ID"),
            (
                json!({ "localId": "other", "email": "taken@example.com" }),
                "EMAIL_EXISTS",
            ),
            (json!({ "email": "nope" }), "INVALID_EMAIL"),
        ] {
            let (status, response) =
                post(&api, "/v1/projects/demo/accounts", body.clone()).await;
            assert_eq!(status, 400, "{body} -> {response}");
            assert_eq!(message(&response), expected, "for {body}");
        }

        let (status, response) = post(
            &api,
            "/v1/projects/demo/accounts",
            json!({ "localId": "a/b" }),
        )
        .await;
        assert_eq!(status, 400);
        assert!(message(&response).starts_with("INVALID_LOCAL_ID"), "{response}");
    }

    #[tokio::test]
    async fn reserved_claims_are_refused_rather_than_silently_dropped() {
        let api = api();
        let (status, body) = post(
            &api,
            "/v1/projects/demo/accounts",
            json!({ "localId": "u", "customAttributes": "{\"sub\":\"attacker\"}" }),
        )
        .await;
        assert_eq!(status, 400);
        assert!(message(&body).starts_with("INVALID_CLAIMS"), "{body}");
        assert!(message(&body).contains("sub"), "{body}");
    }

    #[tokio::test]
    async fn admin_lookup_finds_accounts_by_uid_and_email_and_omits_misses() {
        let api = api();
        post(
            &api,
            "/v1/projects/demo/accounts",
            json!({ "localId": "u1", "email": "one@example.com" }),
        )
        .await;
        post(
            &api,
            "/v1/projects/demo/accounts",
            json!({ "localId": "u2", "email": "two@example.com" }),
        )
        .await;

        let (status, body) = post(
            &api,
            "/v1/projects/demo/accounts:lookup",
            json!({ "localId": ["u1", "ghost"], "email": ["two@example.com"] }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        let users = body["users"].as_array().expect("users");
        assert_eq!(users.len(), 2, "the missing uid must be omitted: {body}");

        // Nothing found at all: `users` is absent, not an empty array.
        let (status, body) = post(
            &api,
            "/v1/projects/demo/accounts:lookup",
            json!({ "localId": ["ghost"] }),
        )
        .await;
        assert_eq!(status, 200);
        assert!(body.get("users").is_none(), "{body}");

        let (status, body) =
            post(&api, "/v1/projects/demo/accounts:lookup", json!({})).await;
        assert_eq!(status, 400);
        assert!(message(&body).starts_with("MISSING_LOCAL_ID"), "{body}");
    }

    #[tokio::test]
    async fn asking_for_one_account_twice_returns_it_once() {
        let api = api();
        post(
            &api,
            "/v1/projects/demo/accounts",
            json!({ "localId": "u1", "email": "one@example.com" }),
        )
        .await;

        let (_, body) = post(
            &api,
            "/v1/projects/demo/accounts:lookup",
            json!({ "localId": ["u1", "u1"], "email": ["one@example.com"] }),
        )
        .await;
        assert_eq!(body["users"].as_array().expect("users").len(), 1, "{body}");
    }

    #[tokio::test]
    async fn admin_update_cannot_move_an_email_onto_another_account() {
        let api = api();
        for (uid, email) in [("u1", "one@example.com"), ("u2", "two@example.com")] {
            post(
                &api,
                "/v1/projects/demo/accounts",
                json!({ "localId": uid, "email": email }),
            )
            .await;
        }

        let (status, body) = post(
            &api,
            "/v1/projects/demo/accounts:update",
            json!({ "localId": "u2", "email": "one@example.com" }),
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(message(&body), "EMAIL_EXISTS");
    }

    #[tokio::test]
    async fn admin_update_and_delete_report_a_missing_account() {
        let api = api();
        for path in [
            "/v1/projects/demo/accounts:update",
            "/v1/projects/demo/accounts:delete",
        ] {
            let (status, body) = post(&api, path, json!({ "localId": "ghost" })).await;
            assert_eq!(status, 400, "{path} -> {body}");
            assert_eq!(message(&body), "USER_NOT_FOUND", "for {path}");

            let (status, body) = post(&api, path, json!({})).await;
            assert_eq!(status, 400);
            assert_eq!(message(&body), "MISSING_LOCAL_ID", "for {path}");
        }
    }

    #[tokio::test]
    async fn clearing_custom_attributes_removes_the_claims() {
        let api = api();
        post(
            &api,
            "/v1/projects/demo/accounts",
            json!({
                "localId": "u1",
                "email": "one@example.com",
                "password": "secret12",
                "customAttributes": "{\"role\":\"editor\"}",
            }),
        )
        .await;

        let (status, _) = post(
            &api,
            "/v1/projects/demo/accounts:update",
            json!({ "localId": "u1", "customAttributes": "" }),
        )
        .await;
        assert_eq!(status, 200);

        let (_, body) = post(
            &api,
            "/v1/projects/demo/accounts:lookup",
            json!({ "localId": ["u1"] }),
        )
        .await;
        assert!(
            body["users"][0].get("customAttributes").is_none(),
            "an empty string clears the claims: {body}"
        );
    }

    #[tokio::test]
    async fn batch_get_pages_in_uid_order() {
        let api = api();
        for uid in ["a", "b", "c"] {
            post(
                &api,
                "/v1/projects/demo/accounts",
                json!({ "localId": uid, "email": format!("{uid}@example.com") }),
            )
            .await;
        }

        let (status, body) = api
            .route_at(
                &AuthRequest {
                    method: "GET",
                    path: "/v1/projects/demo/accounts:batchGet",
                    // A GET carries its parameters in the query string.
                    query: "maxResults=2",
                    body: b"",
                },
                NOW,
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["kind"], "identitytoolkit#DownloadAccountResponse");
        let page: Vec<&str> = body["users"]
            .as_array()
            .expect("users")
            .iter()
            .map(|user| user["localId"].as_str().expect("localId"))
            .collect();
        assert_eq!(page, vec!["a", "b"]);
        assert_eq!(body["nextPageToken"], "b");

        let (_, body) = api
            .route_at(
                &AuthRequest {
                    method: "GET",
                    path: "/v1/projects/demo/accounts:batchGet",
                    query: "nextPageToken=b",
                    body: b"",
                },
                NOW,
            )
            .await;
        let page: Vec<&str> = body["users"]
            .as_array()
            .expect("users")
            .iter()
            .map(|user| user["localId"].as_str().expect("localId"))
            .collect();
        assert_eq!(page, vec!["c"]);
        assert!(
            body.get("nextPageToken").is_none(),
            "the last page must not offer a cursor: {body}"
        );
    }

    // ----- emulator control -------------------------------------------------

    #[tokio::test]
    async fn the_emulator_route_clears_every_account() {
        let api = api();
        signed_up(&api, "ada@example.com", "secret12").await;
        signed_up(&api, "bob@example.com", "secret12").await;

        let (status, body) =
            request(&api, "DELETE", "/emulator/v1/projects/demo/accounts", "").await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["removedAccounts"], 2);

        let (status, body) = post(
            &api,
            "/v1/accounts:signInWithPassword",
            json!({ "email": "ada@example.com", "password": "secret12" }),
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(message(&body), "EMAIL_NOT_FOUND");
    }

    #[tokio::test]
    async fn the_emulator_config_route_answers_a_probe() {
        let api = api();
        let (status, body) =
            request(&api, "GET", "/emulator/v1/projects/demo/config", "").await;

        assert_eq!(status, 200, "{body}");
        assert_eq!(body["signIn"]["allowDuplicateEmails"], false);
        assert_eq!(body["name"], "projects/demo/config");
    }

    // ----- malformed input and broken storage -------------------------------

    #[tokio::test]
    async fn a_body_that_is_not_a_json_object_is_a_clear_400() {
        let api = api();
        for body in ["[1,2]", "\"text\"", "{not json"] {
            let (status, response) =
                request(&api, "POST", "/v1/accounts:signUp", body).await;
            assert_eq!(status, 400, "{body} -> {response}");
            assert!(
                message(&response).starts_with("INVALID_ARGUMENT"),
                "{response}"
            );
        }
    }

    #[tokio::test]
    async fn an_empty_body_is_an_empty_object_not_an_error() {
        // Anonymous sign-up sends no body at all from some clients.
        let api = api();
        let (status, body) = request(&api, "POST", "/v1/accounts:signUp", "").await;
        assert_eq!(status, 200, "{body}");
        assert!(body["localId"].is_string(), "{body}");
    }

    #[tokio::test]
    async fn a_broken_account_store_is_a_500_with_the_real_reason() {
        let api = failing_api();
        let (status, body) = post(
            &api,
            "/v1/accounts:signUp",
            json!({ "email": "ada@example.com", "password": "secret12" }),
        )
        .await;

        assert_eq!(status, 500);
        let message = message(&body);
        assert!(message.starts_with("INTERNAL_ERROR"), "{message}");
        assert!(
            message.contains("deliberately unavailable"),
            "the backend's own message must survive: {message}"
        );
    }

    // ----- helpers ----------------------------------------------------------

    #[test]
    fn plausible_emails_are_accepted_and_typos_are_not() {
        for good in ["a@b.co", "ada.lovelace+tag@example.co.uk"] {
            assert!(is_plausible_email(good), "{good}");
        }
        for bad in ["", "ada", "ada@", "@example.com", "a@b", "a b@c.com", "a@b.", "a@@b.com"] {
            assert!(!is_plausible_email(bad), "{bad} must be rejected");
        }
    }

    #[test]
    fn form_bodies_are_percent_decoded() {
        let pairs = parse_pairs("grant_type=refresh_token&refresh_token=a%2Bb%3D&name=a+b");
        assert_eq!(pairs["grant_type"], "refresh_token");
        assert_eq!(pairs["refresh_token"], "a+b=");
        assert_eq!(pairs["name"], "a b");
    }

    #[test]
    fn a_malformed_escape_is_kept_rather_than_dropped() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("a%zzb"), "a%zzb");
    }

    #[test]
    fn booleans_and_integers_are_read_in_either_json_shape() {
        let body: Map<String, Json> = serde_json::from_str(
            r#"{"a":true,"b":"true","c":"nope","d":5,"e":"6","f":"x"}"#,
        )
        .expect("JSON");

        assert_eq!(bool_field(&body, "a"), Some(true));
        assert_eq!(bool_field(&body, "b"), Some(true));
        assert_eq!(bool_field(&body, "c"), None);
        assert_eq!(int_field(&body, "d"), Some(5));
        assert_eq!(int_field(&body, "e"), Some(6));
        assert_eq!(int_field(&body, "f"), None);
    }
}
