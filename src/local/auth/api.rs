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
use tracing::{debug, info};

use super::oob::{self, OobCode, OobRequestType};
use super::store::{InsertOutcome, SharedUserStore, UpdateOutcome};
use super::token::{self, RefreshToken, SessionContext, TokenError};
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
        // The whole `/emulator/` namespace, not just `/emulator/v1/`: the links
        // this surface mints point at `/emulator/action`, and a link that 404s
        // because the prefix check was narrower than the link builder would be
        // a bug nobody finds until a developer clicks one.
        || path.starts_with("/emulator/")
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

        // The landing page for a minted link. Not a production route — see
        // [`oob::ACTION_PATH`] — so it is matched before the `/emulator/v1/`
        // control endpoints and never mistaken for one.
        if path == oob::ACTION_PATH {
            return self.route_action(method, request.query, now_millis).await;
        }

        if let Some(rest) = path.strip_prefix("/emulator/v1/projects/") {
            return self.route_emulator(method, rest, now_millis).await;
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
            "sendOobCode" => {
                self.send_oob_code(&body, now_millis, false, request.query)
                    .await
            }
            "resetPassword" => self.reset_password(&body, now_millis).await,
            "signInWithEmailLink" => self.sign_in_with_email_link(&body, now_millis).await,
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
        self.open_session_in(user, SessionContext::default(), now_millis)
    }

    /// Open a session carrying its own `context` — how it authenticated and the
    /// claims from the custom token that opened it, neither of which is account
    /// state. Both ride along on the refresh token so a refresh re-issues the
    /// same session rather than a weaker one.
    fn open_session_in(
        &self,
        user: &AuthUser,
        context: SessionContext,
        now_millis: i64,
    ) -> Session {
        Session {
            id_token: token::mint_id_token_for_session(
                user,
                self.project_id(),
                now_millis / 1000,
                &context,
            ),
            refresh_token: RefreshToken {
                local_id: user.local_id.clone(),
                project_id: self.project_id().to_string(),
                issued_at_millis: now_millis,
                session: context,
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
        // Claims in the token are for this session only. They are *not* written
        // onto the account: `createCustomToken(uid, claims)` documents them as
        // additional claims for the session the token opens, while the account's
        // stored custom claims are owned by `setCustomUserClaims`. Storing them
        // here would make signing in with a custom token silently destroy claims
        // an admin had set, which is how this was found — an SDK smoke test
        // signed in and the account's `role` claim was gone.
        let session_claims = match claims.claims {
            Some(extra) => {
                if let Err(reason) = token::validate_custom_claims(&extra) {
                    return bad_request(&format!("INVALID_CLAIMS : {reason}"));
                }
                Some(extra)
            }
            None => None,
        };
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

        // `sign_in_provider` is documented as how the sign-in happened, so a
        // custom token reports `custom` even when the account it names has a
        // password and was created some other way.
        let context = SessionContext {
            provider: Some(SignInProvider::Custom),
            claims: session_claims,
        };
        let session = self.open_session_in(&user, context, now_millis);
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

    /// `POST /v1/accounts:update` — the signed-in account changes itself, or an
    /// email action link is applied.
    ///
    /// The `oobCode` form takes **no** `idToken`, which is what makes it work:
    /// `applyActionCode` is called from whatever page the link opened, which is
    /// usually not signed in at all. The code is the authorisation.
    async fn client_update(&self, body: &Map<String, Json>, now_millis: i64) -> (u16, Json) {
        if let Some(oob_code) = string_field(body, "oobCode") {
            return self.apply_action_code(&oob_code, now_millis).await;
        }

        let user = match self.user_from_id_token(body, now_millis).await {
            Ok(user) => user,
            Err(response) => return response,
        };
        self.apply_update(user, body, now_millis, false).await
    }

    /// The `oobCode` arm of `accounts:update`: the SDK's `applyActionCode`.
    async fn apply_action_code(&self, oob_code: &str, now_millis: i64) -> (u16, Json) {
        let code = match self.redeemable_code(oob_code, now_millis).await {
            Ok(code) => code,
            Err(response) => return response,
        };
        let user = match self.apply_email_action(&code, now_millis).await {
            Ok(user) => user,
            Err(response) => return response,
        };

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
        response.insert("emailVerified".into(), Json::from(user.email_verified));
        let providers = user.provider_user_info();
        if !providers.is_empty() {
            response.insert("providerUserInfo".into(), Json::Array(providers));
        }
        (200, Json::Object(response))
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

        // Carry the session's own context across, so refreshing a custom-token
        // session keeps both the claims and the provider it signed in with
        // instead of silently becoming a different session an hour in.
        let session = self.open_session_in(&user, refresh.session, now_millis);
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
            // The admin form of the same endpoint, and the only one that hands
            // the link back: this is what `generatePasswordResetLink` and
            // `generateEmailVerificationLink` call, with `returnOobLink` set.
            ("POST", "accounts:sendOobCode") => {
                self.send_oob_code(&body, now_millis, true, request.query)
                    .await
            }
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
    async fn route_emulator(&self, method: &str, rest: &str, now_millis: i64) -> (u16, Json) {
        let (project, tail) = rest.split_once('/').unwrap_or((rest, ""));
        if let Some(response) = self.check_project(project) {
            return response;
        }

        match (method, tail) {
            ("DELETE", "accounts") => {
                let removed = match self.users.delete_all(self.project_id()).await {
                    Ok(removed) => removed,
                    Err(error) => return store_failure("clearing accounts", error),
                };
                // Pending links go with the accounts they were minted for.
                // Leaving them would let a reset link survive the wipe a test
                // suite does between cases and apply to whatever account later
                // takes that uid.
                let codes = match self.users.delete_oob_codes(self.project_id()).await {
                    Ok(codes) => codes,
                    Err(error) => return store_failure("clearing out-of-band codes", error),
                };
                // The documented response is an empty object; the counts are
                // Fireup's own addition, so a developer can see it worked.
                (
                    200,
                    json!({ "removedAccounts": removed, "removedOobCodes": codes }),
                )
            }
            ("GET" | "HEAD", "oobCodes") => self.list_oob_codes(now_millis).await,
            ("DELETE", "oobCodes") => match self.users.delete_oob_codes(self.project_id()).await {
                Ok(removed) => (200, json!({ "removedOobCodes": removed })),
                Err(error) => store_failure("clearing out-of-band codes", error),
            },
            (_, "oobCodes") => self.wrong_method(method, "oobCodes", "GET or DELETE"),
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

    // ----- out-of-band email flows ------------------------------------------

    /// Where the links this server mints point.
    fn action_base_url(&self) -> String {
        oob::action_base_url(self.config.addr)
    }

    /// `POST /v1/accounts:sendOobCode`, and its admin twin
    /// `POST /v1/projects/{p}/accounts:sendOobCode`.
    ///
    /// Mints a single-use code and reports the address it was "sent" to. There
    /// is no mail transport (see [`oob`]), so the link is logged at `info` and
    /// listed at `GET /emulator/v1/projects/{p}/oobCodes`. `admin` gates
    /// `returnOobLink`, which is how the Admin SDK's `generate*Link` methods ask
    /// for the link in the response instead of in an email — a client must not
    /// be able to ask for another account's reset link that way.
    async fn send_oob_code(
        &self,
        body: &Map<String, Json>,
        now_millis: i64,
        admin: bool,
        query: &str,
    ) -> (u16, Json) {
        let Some(raw_type) = string_field(body, "requestType") else {
            return bad_request("MISSING_REQ_TYPE");
        };
        let Some(request_type) = OobRequestType::parse(&raw_type) else {
            // Name the non-goal when the request type *is* one, so a developer
            // learns which gap they hit rather than doubting their spelling.
            if raw_type == "PHONE_VERIFICATION" {
                return not_served("phone authentication");
            }
            return bad_request(&format!(
                "INVALID_REQ_TYPE : {raw_type} is not a request type fireup local serves"
            ));
        };

        let continue_url = string_field(body, "continueUrl");

        let code = match request_type {
            // Reset is addressed by email and needs no session: that is the
            // whole point of "I forgot my password".
            OobRequestType::PasswordReset => {
                let Some(email) = string_field(body, "email") else {
                    return bad_request("MISSING_EMAIL");
                };
                let user = match self.account_by_email(&email).await {
                    Ok(user) => user,
                    Err(response) => return response,
                };
                OobCode::new(request_type, user.email.clone().unwrap_or(email), now_millis)
                    .for_account(&user.local_id)
            }
            // Verification is about one account's own address, so the client
            // surface takes it from the session. The admin surface cannot:
            // `generateEmailVerificationLink(email)` has no session at all, and
            // addresses the account by email.
            OobRequestType::VerifyEmail => {
                let user = match self.subject_account(body, now_millis, admin).await {
                    Ok(user) => user,
                    Err(response) => return response,
                };
                let Some(email) = user.email.clone() else {
                    // An anonymous account has nothing to verify.
                    return bad_request("MISSING_EMAIL");
                };
                OobCode::new(request_type, email, now_millis).for_account(&user.local_id)
            }
            // A sign-in link may name an address with no account yet;
            // following it creates one.
            OobRequestType::EmailSignin => {
                let Some(email) = string_field(body, "email") else {
                    return bad_request("MISSING_EMAIL");
                };
                if !is_plausible_email(&email) {
                    return bad_request("INVALID_EMAIL");
                }
                let normalized = AuthUser::normalize_email(&email);
                let existing = match self.users.get_by_email(self.project_id(), &normalized).await
                {
                    Ok(existing) => existing,
                    Err(error) => return store_failure("looking the account up by email", error),
                };
                if existing.as_ref().is_some_and(|user| user.disabled) {
                    return bad_request("USER_DISABLED");
                }
                let code = OobCode::new(request_type, normalized, now_millis);
                match existing {
                    Some(user) => code.for_account(&user.local_id),
                    None => code,
                }
            }
            // The link goes to the *new* address, because that is the mailbox
            // that has to prove it exists before the account moves to it.
            OobRequestType::VerifyAndChangeEmail => {
                let user = match self.subject_account(body, now_millis, admin).await {
                    Ok(user) => user,
                    Err(response) => return response,
                };
                let Some(new_email) = string_field(body, "newEmail") else {
                    return bad_request("MISSING_NEW_EMAIL");
                };
                if !is_plausible_email(&new_email) {
                    return bad_request("INVALID_NEW_EMAIL");
                }
                let normalized = AuthUser::normalize_email(&new_email);
                // Refuse here rather than at redemption: telling the user their
                // new address is taken while they are still in the app is far
                // better than having the link they were mailed fail later.
                match self.users.get_by_email(self.project_id(), &normalized).await {
                    Ok(Some(other)) if other.local_id != user.local_id => {
                        return bad_request("EMAIL_EXISTS")
                    }
                    Ok(_) => {}
                    Err(error) => {
                        return store_failure("checking whether the new email is taken", error)
                    }
                }
                OobCode::new(request_type, normalized.clone(), now_millis)
                    .for_account(&user.local_id)
                    .with_new_email(normalized)
            }
        };

        // Client endpoints carry `?key=<apiKey>`; nothing validates it (see the
        // module docs), but the link has to carry it back — the official SDK's
        // link parser refuses a link without an `apiKey` at all.
        let api_key = string_field(&parse_pairs(query), "key");
        let code = code.with_continue_url(continue_url).with_api_key(api_key);
        let link = code.action_link(&self.action_base_url());
        let email = code.email.clone();

        if let Err(error) = self.users.insert_oob_code(self.project_id(), code).await {
            return store_failure("storing the out-of-band code", error);
        }

        // The log line *is* the mailbox. At `info` so it shows up without
        // raising the log level, and with the whole link so it can be copied
        // out of a terminal.
        info!(
            request_type = request_type.as_str(),
            %email,
            %link,
            "fireup local does not send email; this is the link that would have been sent"
        );

        let mut response = Map::new();
        response.insert(
            "kind".into(),
            Json::from("identitytoolkit#GetOobConfirmationCodeResponse"),
        );
        response.insert("email".into(), Json::from(email));
        // Only the admin surface hands the link back, and only when asked —
        // the client response stays the shape production sends.
        if admin && bool_field(body, "returnOobLink").unwrap_or(false) {
            response.insert("oobLink".into(), Json::from(link));
        }
        (200, Json::Object(response))
    }

    /// `POST /v1/accounts:resetPassword`.
    ///
    /// Two calls in one endpoint, as the published contract has it: with only an
    /// `oobCode` it reports what the code is for (the client SDK's
    /// `verifyPasswordResetCode`), and with a `newPassword` it applies the reset
    /// (`confirmPasswordReset`).
    async fn reset_password(&self, body: &Map<String, Json>, now_millis: i64) -> (u16, Json) {
        let Some(oob_code) = string_field(body, "oobCode") else {
            return bad_request("MISSING_OOB_CODE");
        };
        let code = match self.redeemable_code(&oob_code, now_millis).await {
            Ok(code) => code,
            Err(response) => return response,
        };
        if code.request_type != OobRequestType::PasswordReset {
            // A verification link is not a reset link. Treated as invalid
            // rather than as a wrong-type error, because to a client that is
            // what it is: this code cannot be redeemed here.
            return bad_request("INVALID_OOB_CODE");
        }

        let mut response = Map::new();
        response.insert(
            "kind".into(),
            Json::from("identitytoolkit#ResetPasswordResponse"),
        );
        response.insert("email".into(), Json::from(code.email.clone()));
        response.insert(
            "requestType".into(),
            Json::from(code.request_type.as_str()),
        );

        // No new password: the caller is checking the code is good before
        // showing a form. The code is deliberately *not* consumed, or the form
        // it is about to render could never be submitted.
        let Some(new_password) = string_field(body, "newPassword") else {
            return (200, Json::Object(response));
        };

        if new_password.chars().count() < MIN_PASSWORD_LENGTH {
            return bad_request(&format!(
                "WEAK_PASSWORD : Password should be at least {MIN_PASSWORD_LENGTH} characters"
            ));
        }

        let mut user = match self.account_for_code(&code).await {
            Ok(user) => user,
            Err(response) => return response,
        };
        if user.disabled {
            return bad_request("USER_DISABLED");
        }

        user.password = Some(PasswordHash::new(&new_password));
        if user.provider == SignInProvider::Anonymous {
            // An account that now has a password is no longer anonymous, the
            // same upgrade `accounts:update` performs.
            user.provider = SignInProvider::Password;
        }
        // Completing a reset proves control of the mailbox, so the address is
        // verified as a side effect — and the old sessions die, which is the
        // point of resetting a password you think someone else has.
        user.email_verified = true;
        user.valid_since_millis = now_millis;

        match self.users.update(self.project_id(), user).await {
            Ok(UpdateOutcome::Updated) => {}
            Ok(UpdateOutcome::NotFound) => return bad_request("USER_NOT_FOUND"),
            Ok(UpdateOutcome::EmailExists) => return bad_request("EMAIL_EXISTS"),
            Err(error) => return store_failure("applying the password reset", error),
        }

        // Consume last: a link whose reset failed must still be usable.
        if let Err(error) = self.users.consume_oob_code(self.project_id(), &oob_code).await {
            return store_failure("redeeming the out-of-band code", error);
        }
        (200, Json::Object(response))
    }

    /// `POST /v1/accounts:signInWithEmailLink` — passwordless sign-in.
    ///
    /// The `email` is required alongside the code, and must match the address
    /// the link was issued for. That is not ceremony: it is what stops a link
    /// forwarded to someone else from signing *them* in, which is why the
    /// documented client flow asks the user to confirm their address when it
    /// cannot remember it.
    async fn sign_in_with_email_link(
        &self,
        body: &Map<String, Json>,
        now_millis: i64,
    ) -> (u16, Json) {
        let Some(email) = string_field(body, "email") else {
            return bad_request("MISSING_EMAIL");
        };
        let Some(oob_code) = string_field(body, "oobCode") else {
            return bad_request("MISSING_OOB_CODE");
        };
        let code = match self.redeemable_code(&oob_code, now_millis).await {
            Ok(code) => code,
            Err(response) => return response,
        };
        if code.request_type != OobRequestType::EmailSignin {
            return bad_request("INVALID_OOB_CODE");
        }
        if AuthUser::normalize_email(&email) != code.email {
            return bad_request("INVALID_EMAIL");
        }

        let existing = match self.users.get_by_email(self.project_id(), &code.email).await {
            Ok(existing) => existing,
            Err(error) => return store_failure("looking the account up by email", error),
        };
        let is_new_user = existing.is_none();

        let mut user = existing.unwrap_or_else(|| {
            // Email-link sign-in reports `password` as its provider — the
            // documented `providerId` for the email provider, whose two sign-in
            // methods are a password and a link. The account simply has no
            // password hash until it sets one.
            let mut created =
                AuthUser::new(super::generate_uid(), SignInProvider::Password, now_millis);
            created.set_email(&code.email);
            created
        });
        if user.disabled {
            return bad_request("USER_DISABLED");
        }
        // Following the link proves control of the mailbox.
        user.email_verified = true;
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

        if let Err(error) = self.users.consume_oob_code(self.project_id(), &oob_code).await {
            return store_failure("redeeming the out-of-band code", error);
        }

        let session = self.open_session(&user, now_millis);
        let mut response = Map::new();
        response.insert(
            "kind".into(),
            Json::from("identitytoolkit#EmailLinkSigninResponse"),
        );
        response.insert("localId".into(), Json::from(user.local_id.clone()));
        response.insert("email".into(), Json::from(code.email));
        response.insert("isNewUser".into(), Json::from(is_new_user));
        (200, Json::Object(self.with_session(response, &session)))
    }

    /// `GET /emulator/v1/projects/{p}/oobCodes` — the stand-in for a mailbox.
    ///
    /// Expired codes are left out. A harness that reads the last entry wants
    /// the link it can still use, and an expired one in that list is a
    /// confusing failure two calls later.
    async fn list_oob_codes(&self, now_millis: i64) -> (u16, Json) {
        let codes = match self.users.list_oob_codes(self.project_id()).await {
            Ok(codes) => codes,
            Err(error) => return store_failure("listing out-of-band codes", error),
        };
        let base_url = self.action_base_url();
        (
            200,
            json!({
                "oobCodes": codes
                    .iter()
                    .filter(|code| code.is_live_at(now_millis))
                    .map(|code| code.to_listing_entry(&base_url))
                    .collect::<Vec<_>>(),
            }),
        )
    }

    /// `GET /emulator/action?mode=…&oobCode=…` — where a minted link points.
    ///
    /// **Not a production route.** Production links land on a Google-hosted
    /// page; this is Fireup's own JSON so that a link is something a developer
    /// can actually follow with `curl` or a browser instead of a 404.
    ///
    /// Clicking a verification link applies it, which is what clicking one does
    /// in the real flow. A reset or sign-in link cannot be completed by a visit
    /// — one needs a new password, the other the address to confirm — so those
    /// report the pending action and the call that finishes it, and leave the
    /// code redeemable.
    async fn route_action(&self, method: &str, query: &str, now_millis: i64) -> (u16, Json) {
        if method != "GET" && method != "HEAD" {
            return self.wrong_method(method, oob::ACTION_PATH, "GET");
        }

        let parameters = parse_pairs(query);
        let Some(oob_code) = string_field(&parameters, "oobCode") else {
            return bad_request("MISSING_OOB_CODE");
        };
        let code = match self.redeemable_code(&oob_code, now_millis).await {
            Ok(code) => code,
            Err(response) => return response,
        };
        // `mode` is informational in the link — the code already knows what it
        // is for — but a mismatch means the link was edited, and acting on it
        // would apply an action the developer did not mean to trigger.
        if let Some(mode) = string_field(&parameters, "mode") {
            if OobRequestType::from_link_mode(&mode) != Some(code.request_type) {
                return bad_request(&format!(
                    "INVALID_OOB_CODE : this code is a {} link, not {mode}",
                    code.request_type.link_mode()
                ));
            }
        }

        let mut response = Map::new();
        response.insert(
            "notice".into(),
            Json::from(
                "fireup local's own landing route for an email action link. \
                 Not a Google page and not part of the Identity Toolkit API.",
            ),
        );
        response.insert("mode".into(), Json::from(code.request_type.link_mode()));
        response.insert(
            "requestType".into(),
            Json::from(code.request_type.as_str()),
        );
        response.insert("email".into(), Json::from(code.email.clone()));
        response.insert("oobCode".into(), Json::from(code.oob_code.clone()));
        if let Some(continue_url) = &code.continue_url {
            response.insert("continueUrl".into(), Json::from(continue_url.clone()));
        }

        match code.request_type {
            OobRequestType::VerifyEmail | OobRequestType::VerifyAndChangeEmail => {
                let user = match self.apply_email_action(&code, now_millis).await {
                    Ok(user) => user,
                    Err(response) => return response,
                };
                response.insert("applied".into(), Json::from(true));
                response.insert("localId".into(), Json::from(user.local_id));
                response.insert("email".into(), Json::from(user.email.unwrap_or_default()));
                response.insert("emailVerified".into(), Json::from(user.email_verified));
            }
            OobRequestType::PasswordReset => {
                response.insert("applied".into(), Json::from(false));
                response.insert(
                    "finishWith".into(),
                    json!({
                        "method": "POST",
                        "path": "/v1/accounts:resetPassword",
                        "body": { "oobCode": code.oob_code, "newPassword": "<the new password>" },
                    }),
                );
            }
            OobRequestType::EmailSignin => {
                response.insert("applied".into(), Json::from(false));
                response.insert(
                    "finishWith".into(),
                    json!({
                        "method": "POST",
                        "path": "/v1/accounts:signInWithEmailLink",
                        "body": { "oobCode": code.oob_code, "email": code.email },
                    }),
                );
            }
        }
        (200, Json::Object(response))
    }

    /// Read a code and refuse it if it is unknown or past its expiry.
    ///
    /// The two are different answers on purpose: `EXPIRED_OOB_CODE` tells a
    /// developer to ask for a new link, while `INVALID_OOB_CODE` tells them the
    /// code is wrong or already used. Collapsing them would make a working
    /// flow that sat idle for an hour look like a bug in the code.
    async fn redeemable_code(
        &self,
        oob_code: &str,
        now_millis: i64,
    ) -> Result<OobCode, (u16, Json)> {
        match self.users.get_oob_code(self.project_id(), oob_code).await {
            Ok(Some(code)) if code.is_live_at(now_millis) => Ok(code),
            Ok(Some(_)) => Err(bad_request("EXPIRED_OOB_CODE")),
            Ok(None) => Err(bad_request("INVALID_OOB_CODE")),
            Err(error) => Err(store_failure("reading the out-of-band code", error)),
        }
    }

    /// The account a code names.
    async fn account_for_code(&self, code: &OobCode) -> Result<AuthUser, (u16, Json)> {
        let Some(local_id) = &code.local_id else {
            return Err(bad_request("USER_NOT_FOUND"));
        };
        match self.users.get_by_local_id(self.project_id(), local_id).await {
            Ok(Some(user)) => Ok(user),
            // The account was deleted while the link sat in a terminal.
            Ok(None) => Err(bad_request("USER_NOT_FOUND")),
            Err(error) => Err(store_failure("reading the account the link names", error)),
        }
    }

    /// The account an email action is *about*, for the two flows that act on
    /// one account's own address.
    ///
    /// A client names it with its `idToken` — it can only act on itself. An
    /// admin names it with `email`, because the Admin SDK's `generate*Link`
    /// methods take an address and have no session; the bearer token that would
    /// authorise them in production is not checked here (see the module docs),
    /// so this split is about matching the published request shapes, not about
    /// being a security boundary.
    async fn subject_account(
        &self,
        body: &Map<String, Json>,
        now_millis: i64,
        admin: bool,
    ) -> Result<AuthUser, (u16, Json)> {
        if !admin {
            return self.user_from_id_token(body, now_millis).await;
        }
        let Some(email) = string_field(body, "email") else {
            return Err(bad_request("MISSING_EMAIL"));
        };
        self.account_by_email(&email).await
    }

    /// The account registered to `email`, for the flows addressed by mailbox.
    async fn account_by_email(&self, email: &str) -> Result<AuthUser, (u16, Json)> {
        let normalized = AuthUser::normalize_email(email);
        match self.users.get_by_email(self.project_id(), &normalized).await {
            Ok(Some(user)) if user.disabled => Err(bad_request("USER_DISABLED")),
            Ok(Some(user)) => Ok(user),
            // `EMAIL_NOT_FOUND` is the documented code, and the one SDKs turn
            // into `auth/user-not-found`. It does leak whether an address has
            // an account — which is exactly what the published contract does,
            // and this is local-development tooling either way.
            Ok(None) => Err(bad_request("EMAIL_NOT_FOUND")),
            Err(error) => Err(store_failure("looking the account up by email", error)),
        }
    }

    /// Apply a verification or email-change code, and redeem it.
    ///
    /// Shared by `accounts:update` (what the SDK's `applyActionCode` calls) and
    /// the landing route, so a clicked link and an SDK call cannot end up
    /// meaning different things.
    async fn apply_email_action(
        &self,
        code: &OobCode,
        now_millis: i64,
    ) -> Result<AuthUser, (u16, Json)> {
        let mut user = self.account_for_code(code).await?;
        if user.disabled {
            return Err(bad_request("USER_DISABLED"));
        }

        match code.request_type {
            OobRequestType::VerifyEmail => {
                // Verify the address the link was issued for, not whatever the
                // account holds now: an account that changed its email after
                // the link was sent must not have the *new* address marked
                // verified by a link sent to the old one.
                if user.email.as_deref() != Some(code.email.as_str()) {
                    return Err(bad_request(
                        "INVALID_OOB_CODE : the account's email changed after this link was sent",
                    ));
                }
                user.email_verified = true;
            }
            OobRequestType::VerifyAndChangeEmail => {
                let Some(new_email) = &code.new_email else {
                    return Err(error_response(
                        500,
                        "INTERNAL_ERROR : an email-change code has no new address",
                    ));
                };
                user.email = Some(new_email.clone());
                user.email_verified = true;
                // Changing the email is a credential change, so existing
                // sessions go — the same rule `accounts:update` applies.
                user.valid_since_millis = now_millis;
            }
            other => {
                return Err(bad_request(&format!(
                    "INVALID_OOB_CODE : a {} code cannot be applied here",
                    other.as_str()
                )))
            }
        }

        match self.users.update(self.project_id(), user.clone()).await {
            Ok(UpdateOutcome::Updated) => {}
            Ok(UpdateOutcome::NotFound) => return Err(bad_request("USER_NOT_FOUND")),
            Ok(UpdateOutcome::EmailExists) => return Err(bad_request("EMAIL_EXISTS")),
            Err(error) => return Err(store_failure("applying the email action", error)),
        }

        if let Err(error) = self
            .users
            .consume_oob_code(self.project_id(), &code.oob_code)
            .await
        {
            return Err(store_failure("redeeming the out-of-band code", error));
        }
        Ok(user)
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
    /// Both an empty string and an empty object clear the claims, because both
    /// are how an Admin SDK removes them: `setCustomUserClaims(uid, null)`
    /// wire-encodes as the *string* `"{}"`, not as an absent field. Keeping
    /// `Some({})` here would echo `customAttributes: "{}"` back on the next
    /// lookup, and the SDK reports any present value as claims — so
    /// `UserRecord.customClaims` would come back as `{}` where the documented
    /// value for an account with no claims is `undefined`.
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
        if parsed.as_object().is_some_and(Map::is_empty) {
            return Ok(None);
        }
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

    /// `GET <path>?<query>`.
    async fn get_at(api: &AuthApi, path: &str, query: &str, now: i64) -> (u16, Json) {
        api.route_at(
            &AuthRequest {
                method: "GET",
                path,
                query,
                body: b"",
            },
            now,
        )
        .await
    }

    async fn get(api: &AuthApi, path: &str, query: &str) -> (u16, Json) {
        get_at(api, path, query, NOW).await
    }

    fn message(body: &Json) -> String {
        body["error"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("not an error envelope: {body}"))
            .to_string()
    }

    /// The claims inside the ID token of a sign-in or refresh response.
    ///
    /// Identity Toolkit sign-in responses name the field `idToken`; the
    /// secure-token refresh endpoint is snake_case and names it `id_token`.
    /// Both are correct, so this reads whichever is present.
    fn id_token_claims(body: &Json) -> Json {
        let id_token = body["idToken"]
            .as_str()
            .or_else(|| body["id_token"].as_str())
            .unwrap_or_else(|| panic!("no ID token in {body}"));
        let payload = id_token.split('.').nth(1).expect("claims segment");
        serde_json::from_slice(&super::super::base64url_decode(payload).expect("base64url"))
            .expect("JSON claims")
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
            // The links this surface mints land here, so it has to be claimed
            // or every generated link 404s.
            oob::ACTION_PATH,
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

        // Phone verification arrives as a `requestType` on an endpoint that is
        // otherwise served, so the refusal has to come from the handler.
        let (status, body) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({ "requestType": "PHONE_VERIFICATION", "email": "a@b.com" }),
        )
        .await;
        assert_eq!(status, 501);
        assert!(message(&body).contains("phone"), "{body}");

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
    async fn custom_token_claims_are_session_scoped_and_do_not_overwrite_the_account() {
        let api = api();
        post(
            &api,
            "/v1/projects/demo/accounts",
            json!({
                "localId": "both",
                "email": "both@example.com",
                "password": "secret12",
                "customAttributes": "{\"role\":\"admin\",\"tier\":3}",
            }),
        )
        .await;

        // A custom token's `claims` are documented as additional claims for the
        // session it opens, not an edit to the account.
        let custom =
            token::mint_custom_token("both", Some(&json!({ "scope": "batch" })), NOW / 1000);
        let (status, body) =
            post(&api, "/v1/accounts:signInWithCustomToken", json!({ "token": custom })).await;
        assert_eq!(status, 200, "{body}");

        // The session sees both the account's claims and the token's, and
        // reports how *it* signed in rather than how the account was made.
        let session_claims = id_token_claims(&body);
        assert_eq!(session_claims["scope"], "batch");
        assert_eq!(session_claims["role"], "admin", "{session_claims}");
        assert_eq!(session_claims["tier"], 3, "{session_claims}");
        assert_eq!(
            session_claims["firebase"]["sign_in_provider"], "custom",
            "a custom-token sign-in signed in with a custom token: {session_claims}"
        );

        // The account itself is untouched: signing in with a custom token must
        // not destroy what an admin set.
        let (status, lookup) = post(
            &api,
            "/v1/projects/demo/accounts:lookup",
            json!({ "localId": ["both"] }),
        )
        .await;
        assert_eq!(status, 200, "{lookup}");
        assert_eq!(
            lookup["users"][0]["customAttributes"], "{\"role\":\"admin\",\"tier\":3}",
            "custom-token sign-in must leave stored claims alone: {lookup}"
        );

        // A password sign-in therefore still sees the account's claims, and not
        // the previous session's.
        let (status, password_session) = post(
            &api,
            "/v1/accounts:signInWithPassword",
            json!({ "email": "both@example.com", "password": "secret12" }),
        )
        .await;
        assert_eq!(status, 200, "{password_session}");
        let claims = id_token_claims(&password_session);
        assert_eq!(claims["role"], "admin");
        assert_eq!(claims["firebase"]["sign_in_provider"], "password");
        assert!(
            claims.get("scope").is_none(),
            "one session's claims must not leak into another: {claims}"
        );
    }

    #[tokio::test]
    async fn refreshing_a_custom_token_session_keeps_its_claims() {
        let api = api();
        let custom =
            token::mint_custom_token("refresher", Some(&json!({ "scope": "batch" })), NOW / 1000);
        let (status, body) =
            post(&api, "/v1/accounts:signInWithCustomToken", json!({ "token": custom })).await;
        assert_eq!(status, 200, "{body}");

        let refresh_token = body["refreshToken"].as_str().expect("refreshToken");
        let (status, refreshed) = request(
            &api,
            "POST",
            "/v1/token",
            &format!("grant_type=refresh_token&refresh_token={refresh_token}"),
        )
        .await;
        assert_eq!(status, 200, "{refreshed}");

        // Without the claims on the refresh token, the session would silently
        // lose them an hour in — the hardest kind of bug to attribute.
        let claims = id_token_claims(&refreshed);
        assert_eq!(claims["scope"], "batch", "{claims}");
        assert_eq!(claims["sub"], "refresher");
        assert_eq!(
            claims["firebase"]["sign_in_provider"], "custom",
            "a refresh must not change how the session signed in: {claims}"
        );
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
    async fn clearing_custom_claims_removes_the_field_rather_than_emptying_it() {
        let api = api();
        post(
            &api,
            "/v1/projects/demo/accounts",
            json!({
                "localId": "claimed",
                "customAttributes": "{\"role\":\"editor\"}",
            }),
        )
        .await;

        // `setCustomUserClaims(uid, null)` in an Admin SDK sends the *string*
        // "{}", not an absent field or an empty string. Storing that as an
        // empty object would make the next lookup report `customAttributes:
        // "{}"`, which an SDK reads back as claims of `{}` where the documented
        // value for an account with no claims is absent.
        let (status, body) = post(
            &api,
            "/v1/projects/demo/accounts:update",
            json!({ "localId": "claimed", "customAttributes": "{}" }),
        )
        .await;
        assert_eq!(status, 200, "{body}");

        let (status, body) = post(
            &api,
            "/v1/projects/demo/accounts:lookup",
            json!({ "localId": ["claimed"] }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert!(
            body["users"][0].get("customAttributes").is_none(),
            "cleared claims must not come back as an empty object: {body}"
        );
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

    // ----- out-of-band email flows ------------------------------------------

    /// The only way a client learns a code, since nothing is mailed: read the
    /// newest entry off the control endpoint. Every test below goes through
    /// this rather than reaching into the store, so what it asserts is what a
    /// developer or a test harness can actually get at.
    async fn newest_oob_code(api: &AuthApi) -> Json {
        let (status, body) =
            request(api, "GET", "/emulator/v1/projects/demo/oobCodes", "").await;
        assert_eq!(status, 200, "{body}");
        body["oobCodes"]
            .as_array()
            .expect("an array of pending codes")
            .last()
            .unwrap_or_else(|| panic!("no pending codes: {body}"))
            .clone()
    }

    /// The pending code of a given `requestType`.
    ///
    /// Used where a test has more than one link outstanding: these tests mint
    /// everything at one frozen instant, so "the newest" is not well defined
    /// among codes sharing a millisecond.
    async fn pending_oob_code_of(api: &AuthApi, request_type: &str) -> Json {
        let (status, body) =
            request(api, "GET", "/emulator/v1/projects/demo/oobCodes", "").await;
        assert_eq!(status, 200, "{body}");
        body["oobCodes"]
            .as_array()
            .expect("an array of pending codes")
            .iter()
            .find(|entry| entry["requestType"] == request_type)
            .unwrap_or_else(|| panic!("no pending {request_type} code: {body}"))
            .clone()
    }

    async fn requested_reset(api: &AuthApi, email: &str) -> String {
        let (status, body) = post(
            api,
            "/v1/accounts:sendOobCode",
            json!({ "requestType": "PASSWORD_RESET", "email": email }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["kind"], "identitytoolkit#GetOobConfirmationCodeResponse");
        assert_eq!(body["email"], email);
        // The client response must not carry the link: production sends it by
        // email, and a client that could read it could read anyone's.
        assert!(body.get("oobLink").is_none(), "{body}");

        newest_oob_code(api).await["oobCode"]
            .as_str()
            .expect("a code string")
            .to_string()
    }

    #[tokio::test]
    async fn a_password_reset_runs_end_to_end_and_the_old_password_stops_working() {
        let api = api();
        signed_up(&api, "ada@example.com", "secret12").await;

        let code = requested_reset(&api, "ada@example.com").await;

        // Checking the code first is what `verifyPasswordResetCode` does, and
        // it must not consume the code — the form has not been submitted yet.
        let (status, body) = post(
            &api,
            "/v1/accounts:resetPassword",
            json!({ "oobCode": code }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["kind"], "identitytoolkit#ResetPasswordResponse");
        assert_eq!(body["email"], "ada@example.com");
        assert_eq!(body["requestType"], "PASSWORD_RESET");

        let (status, body) = post(
            &api,
            "/v1/accounts:resetPassword",
            json!({ "oobCode": code, "newPassword": "brand-new-one" }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["email"], "ada@example.com");

        let (status, body) = post(
            &api,
            "/v1/accounts:signInWithPassword",
            json!({ "email": "ada@example.com", "password": "brand-new-one" }),
        )
        .await;
        assert_eq!(status, 200, "the new password must work: {body}");

        let (status, body) = post(
            &api,
            "/v1/accounts:signInWithPassword",
            json!({ "email": "ada@example.com", "password": "secret12" }),
        )
        .await;
        assert_eq!(status, 400, "the old password must not: {body}");
        assert_eq!(message(&body), "INVALID_PASSWORD");
    }

    #[tokio::test]
    async fn a_reset_revokes_the_sessions_that_were_open_when_it_was_requested() {
        // The reason to reset a password is that someone else may have it. A
        // reset that left their refresh token working would not accomplish
        // anything.
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;
        let refresh = session["refreshToken"].as_str().expect("refresh token");

        let code = requested_reset(&api, "ada@example.com").await;
        // A second later, so the revocation instant is after the token was
        // issued rather than in the same millisecond.
        let (status, _) = post_at(
            &api,
            "/v1/accounts:resetPassword",
            json!({ "oobCode": code, "newPassword": "brand-new-one" }),
            NOW + 1_000,
        )
        .await;
        assert_eq!(status, 200);

        let (status, body) = post_at(
            &api,
            "/v1/token",
            json!({ "grant_type": "refresh_token", "refresh_token": refresh }),
            NOW + 2_000,
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "TOKEN_EXPIRED");
    }

    #[tokio::test]
    async fn a_reset_code_works_once() {
        let api = api();
        signed_up(&api, "ada@example.com", "secret12").await;
        let code = requested_reset(&api, "ada@example.com").await;

        let (status, _) = post(
            &api,
            "/v1/accounts:resetPassword",
            json!({ "oobCode": code, "newPassword": "first-new-one" }),
        )
        .await;
        assert_eq!(status, 200);

        let (status, body) = post(
            &api,
            "/v1/accounts:resetPassword",
            json!({ "oobCode": code, "newPassword": "second-new-one" }),
        )
        .await;
        assert_eq!(status, 400, "a redeemed link must not work twice: {body}");
        assert_eq!(message(&body), "INVALID_OOB_CODE");

        // And the first reset is the one that stuck.
        let (status, _) = post(
            &api,
            "/v1/accounts:signInWithPassword",
            json!({ "email": "ada@example.com", "password": "first-new-one" }),
        )
        .await;
        assert_eq!(status, 200);
    }

    #[tokio::test]
    async fn an_expired_link_says_it_expired_rather_than_that_it_is_wrong() {
        let api = api();
        signed_up(&api, "ada@example.com", "secret12").await;
        let code = requested_reset(&api, "ada@example.com").await;

        let an_hour_later = NOW + oob::TTL_SECONDS * 1000;
        let (status, body) = post_at(
            &api,
            "/v1/accounts:resetPassword",
            json!({ "oobCode": code, "newPassword": "brand-new-one" }),
            an_hour_later,
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(
            message(&body),
            "EXPIRED_OOB_CODE",
            "a developer who waited needs told to ask for a new link"
        );

        // An expired code also drops out of the listing, so a harness that
        // takes the newest entry does not pick up a link it cannot use.
        let (status, body) =
            get_at(&api, "/emulator/v1/projects/demo/oobCodes", "", an_hour_later).await;
        assert_eq!(status, 200);
        assert!(
            body["oobCodes"].as_array().expect("array").is_empty(),
            "{body}"
        );
    }

    #[tokio::test]
    async fn a_reset_for_an_unknown_or_disabled_account_is_refused_by_name() {
        let api = api();
        let (status, body) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({ "requestType": "PASSWORD_RESET", "email": "nobody@example.com" }),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "EMAIL_NOT_FOUND");

        signed_up(&api, "ada@example.com", "secret12").await;
        let (status, _) = post(
            &api,
            "/v1/projects/demo/accounts:update",
            json!({ "email": "ada@example.com", "disableUser": true }),
        )
        .await;
        assert_eq!(status, 400, "the admin endpoint takes a localId, not an email");

        let user = post(
            &api,
            "/v1/projects/demo/accounts:lookup",
            json!({ "email": ["ada@example.com"] }),
        )
        .await
        .1;
        let local_id = user["users"][0]["localId"].as_str().expect("uid").to_string();
        let (status, _) = post(
            &api,
            "/v1/projects/demo/accounts:update",
            json!({ "localId": local_id, "disableUser": true }),
        )
        .await;
        assert_eq!(status, 200);

        let (status, body) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({ "requestType": "PASSWORD_RESET", "email": "ada@example.com" }),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "USER_DISABLED");
    }

    #[tokio::test]
    async fn send_oob_code_rejects_what_it_cannot_act_on() {
        let api = api();

        let (status, body) = post(&api, "/v1/accounts:sendOobCode", json!({})).await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "MISSING_REQ_TYPE");

        let (status, body) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({ "requestType": "OLD_EMAIL_AGREE", "email": "a@b.com" }),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert!(message(&body).starts_with("INVALID_REQ_TYPE"), "{body}");

        let (status, body) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({ "requestType": "PASSWORD_RESET" }),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "MISSING_EMAIL");

        // Verification is addressed by session, so no token is a token error.
        let (status, body) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({ "requestType": "VERIFY_EMAIL" }),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "MISSING_ID_TOKEN");

        let (status, body) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({ "requestType": "EMAIL_SIGNIN", "email": "not-an-email" }),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "INVALID_EMAIL");
    }

    #[tokio::test]
    async fn email_verification_runs_end_to_end_through_the_sdk_shaped_calls() {
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;
        let id_token = session["idToken"].as_str().expect("id token").to_string();

        // A fresh password sign-up is unverified, which is the point.
        let (status, body) = post(&api, "/v1/accounts:lookup", json!({ "idToken": id_token })).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["users"][0]["emailVerified"], false);

        let (status, body) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({ "requestType": "VERIFY_EMAIL", "idToken": id_token }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["email"], "ada@example.com");

        let entry = newest_oob_code(&api).await;
        assert_eq!(entry["requestType"], "VERIFY_EMAIL");
        let code = entry["oobCode"].as_str().expect("code").to_string();

        // `applyActionCode` posts the code with no idToken at all: the page the
        // link opened is usually not signed in.
        let (status, body) = post(&api, "/v1/accounts:update", json!({ "oobCode": code })).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["kind"], "identitytoolkit#SetAccountInfoResponse");
        assert_eq!(body["emailVerified"], true);
        assert_eq!(body["email"], "ada@example.com");

        let (status, body) = post(&api, "/v1/accounts:lookup", json!({ "idToken": id_token })).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["users"][0]["emailVerified"], true);

        // Verification is not a credential change, so the session it was
        // requested from keeps working — otherwise every app would sign its
        // user out the moment they verified their address.
        let (status, refreshed) = post_at(
            &api,
            "/v1/token",
            json!({
                "grant_type": "refresh_token",
                "refresh_token": session["refreshToken"].as_str().expect("refresh"),
            }),
            NOW + 2_000,
        )
        .await;
        assert_eq!(status, 200, "{refreshed}");
        assert_eq!(
            id_token_claims(&refreshed)["email_verified"],
            true,
            "and the refreshed token reports the address as verified"
        );
    }

    #[tokio::test]
    async fn a_verification_link_does_not_verify_an_address_the_account_moved_to() {
        // Sent to the old address, then the user changes their email. Applying
        // the stale link must not mark the *new* address verified.
        let api = api();
        let session = signed_up(&api, "old@example.com", "secret12").await;
        let id_token = session["idToken"].as_str().expect("id token").to_string();

        let (status, _) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({ "requestType": "VERIFY_EMAIL", "idToken": id_token }),
        )
        .await;
        assert_eq!(status, 200);
        let code = newest_oob_code(&api).await["oobCode"]
            .as_str()
            .expect("code")
            .to_string();

        let (status, body) = post(
            &api,
            "/v1/accounts:update",
            json!({ "idToken": id_token, "email": "new@example.com" }),
        )
        .await;
        assert_eq!(status, 200, "{body}");

        let (status, body) = post(&api, "/v1/accounts:update", json!({ "oobCode": code })).await;
        assert_eq!(status, 400, "{body}");
        assert!(message(&body).contains("changed after this link was sent"), "{body}");
    }

    #[tokio::test]
    async fn verify_and_change_email_moves_the_account_once_the_new_address_is_confirmed() {
        let api = api();
        let session = signed_up(&api, "old@example.com", "secret12").await;
        let id_token = session["idToken"].as_str().expect("id token").to_string();

        let (status, body) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({
                "requestType": "VERIFY_AND_CHANGE_EMAIL",
                "idToken": id_token,
                "newEmail": "New@Example.com",
            }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(
            body["email"], "new@example.com",
            "the link goes to the address that has to prove it exists"
        );

        // Until the link is followed, the account still holds the old address.
        let (_, body) = post(&api, "/v1/accounts:lookup", json!({ "idToken": id_token })).await;
        assert_eq!(body["users"][0]["email"], "old@example.com");

        let code = newest_oob_code(&api).await["oobCode"]
            .as_str()
            .expect("code")
            .to_string();
        // A second later, so revocation lands after the session was opened.
        let (status, body) = post_at(
            &api,
            "/v1/accounts:update",
            json!({ "oobCode": code }),
            NOW + 1_000,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["email"], "new@example.com");
        assert_eq!(body["emailVerified"], true);

        // The move is a credential change, so the old session is gone.
        let (status, body) = post(&api, "/v1/accounts:lookup", json!({ "idToken": id_token })).await;
        assert_eq!(status, 200, "an unexpired ID token still reads: {body}");
        let (status, body) = post_at(
            &api,
            "/v1/token",
            json!({
                "grant_type": "refresh_token",
                "refresh_token": session["refreshToken"].as_str().expect("refresh"),
            }),
            NOW + 2_000,
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "TOKEN_EXPIRED");

        let (status, _) = post(
            &api,
            "/v1/accounts:signInWithPassword",
            json!({ "email": "new@example.com", "password": "secret12" }),
        )
        .await;
        assert_eq!(status, 200, "the account signs in at its new address");
    }

    #[tokio::test]
    async fn an_email_change_to_an_address_already_taken_is_refused_while_the_user_is_still_there()
    {
        // Refusing at `sendOobCode` rather than at redemption: the user is in
        // the app and can pick another address, instead of finding out from a
        // link that fails later.
        let api = api();
        signed_up(&api, "taken@example.com", "secret12").await;
        let session = signed_up(&api, "mine@example.com", "secret12").await;
        let id_token = session["idToken"].as_str().expect("id token").to_string();

        let (status, body) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({
                "requestType": "VERIFY_AND_CHANGE_EMAIL",
                "idToken": id_token,
                "newEmail": "taken@example.com",
            }),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "EMAIL_EXISTS");

        let (status, body) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({ "requestType": "VERIFY_AND_CHANGE_EMAIL", "idToken": id_token }),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "MISSING_NEW_EMAIL");
    }

    #[tokio::test]
    async fn an_email_link_signs_in_a_new_account_and_then_an_existing_one() {
        let api = api();

        let (status, body) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({
                "requestType": "EMAIL_SIGNIN",
                "email": "link@example.com",
                "continueUrl": "http://localhost:3000/finish",
            }),
        )
        .await;
        assert_eq!(status, 200, "a link may name an address with no account: {body}");

        let entry = newest_oob_code(&api).await;
        assert_eq!(entry["requestType"], "EMAIL_SIGNIN");
        assert!(
            entry["oobLink"]
                .as_str()
                .expect("a link")
                .contains("mode=signIn"),
            "{entry}"
        );
        let code = entry["oobCode"].as_str().expect("code").to_string();

        let (status, body) = post(
            &api,
            "/v1/accounts:signInWithEmailLink",
            json!({ "email": "link@example.com", "oobCode": code }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["kind"], "identitytoolkit#EmailLinkSigninResponse");
        assert_eq!(body["isNewUser"], true);
        assert_eq!(body["email"], "link@example.com");
        let claims = id_token_claims(&body);
        assert_eq!(claims["email"], "link@example.com");
        assert_eq!(
            claims["email_verified"], true,
            "following the link proves the mailbox"
        );
        assert_eq!(
            claims["firebase"]["sign_in_provider"], "password",
            "the email provider's id is `password`, whichever of its two methods was used"
        );

        // A second link for the same address signs the same account in.
        let (status, _) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({ "requestType": "EMAIL_SIGNIN", "email": "link@example.com" }),
        )
        .await;
        assert_eq!(status, 200);
        let code = newest_oob_code(&api).await["oobCode"]
            .as_str()
            .expect("code")
            .to_string();

        let (status, second) = post(
            &api,
            "/v1/accounts:signInWithEmailLink",
            json!({ "email": "link@example.com", "oobCode": code }),
        )
        .await;
        assert_eq!(status, 200, "{second}");
        assert_eq!(second["isNewUser"], false);
        assert_eq!(second["localId"], body["localId"]);
    }

    #[tokio::test]
    async fn an_email_link_refuses_an_address_it_was_not_issued_for() {
        // A forwarded link must not sign in whoever received it. The documented
        // client flow asks the user to confirm their address for this reason.
        let api = api();
        let (status, _) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({ "requestType": "EMAIL_SIGNIN", "email": "mine@example.com" }),
        )
        .await;
        assert_eq!(status, 200);
        let code = newest_oob_code(&api).await["oobCode"]
            .as_str()
            .expect("code")
            .to_string();

        let (status, body) = post(
            &api,
            "/v1/accounts:signInWithEmailLink",
            json!({ "email": "someone-else@example.com", "oobCode": code }),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "INVALID_EMAIL");

        // Case and surrounding space are not a mismatch, though — the address
        // is normalised the same way everywhere else.
        let (status, body) = post(
            &api,
            "/v1/accounts:signInWithEmailLink",
            json!({ "email": " Mine@Example.COM ", "oobCode": code }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
    }

    #[tokio::test]
    async fn a_code_cannot_be_redeemed_at_the_wrong_endpoint() {
        let api = api();
        signed_up(&api, "ada@example.com", "secret12").await;
        let reset = requested_reset(&api, "ada@example.com").await;

        // A reset code is not a sign-in link, and not an action code.
        let (status, body) = post(
            &api,
            "/v1/accounts:signInWithEmailLink",
            json!({ "email": "ada@example.com", "oobCode": reset }),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "INVALID_OOB_CODE");

        let (status, body) = post(&api, "/v1/accounts:update", json!({ "oobCode": reset })).await;
        assert_eq!(status, 400, "{body}");
        assert!(message(&body).starts_with("INVALID_OOB_CODE"), "{body}");

        // And the reset code still works where it belongs: a wrong-endpoint
        // attempt must not burn it.
        let (status, body) = post(
            &api,
            "/v1/accounts:resetPassword",
            json!({ "oobCode": reset, "newPassword": "brand-new-one" }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
    }

    #[tokio::test]
    async fn redeeming_endpoints_name_the_field_they_are_missing() {
        let api = api();

        for (path, body, expected) in [
            ("/v1/accounts:resetPassword", json!({}), "MISSING_OOB_CODE"),
            (
                "/v1/accounts:signInWithEmailLink",
                json!({ "oobCode": "whatever" }),
                "MISSING_EMAIL",
            ),
            (
                "/v1/accounts:signInWithEmailLink",
                json!({ "email": "a@b.com" }),
                "MISSING_OOB_CODE",
            ),
        ] {
            let (status, response) = post(&api, path, body.clone()).await;
            assert_eq!(status, 400, "{path} {body} -> {response}");
            assert_eq!(message(&response), expected, "{path} {body}");
        }

        // An unknown code is invalid, not a 500 or a 404.
        let (status, response) = post(
            &api,
            "/v1/accounts:resetPassword",
            json!({ "oobCode": "never-minted" }),
        )
        .await;
        assert_eq!(status, 400, "{response}");
        assert_eq!(message(&response), "INVALID_OOB_CODE");
    }

    #[tokio::test]
    async fn a_weak_new_password_is_refused_and_leaves_the_link_usable() {
        let api = api();
        signed_up(&api, "ada@example.com", "secret12").await;
        let code = requested_reset(&api, "ada@example.com").await;

        let (status, body) = post(
            &api,
            "/v1/accounts:resetPassword",
            json!({ "oobCode": code, "newPassword": "short" }),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert!(message(&body).starts_with("WEAK_PASSWORD"), "{body}");

        let (status, body) = post(
            &api,
            "/v1/accounts:resetPassword",
            json!({ "oobCode": code, "newPassword": "long-enough" }),
        )
        .await;
        assert_eq!(
            status, 200,
            "a rejected password must not burn the link: {body}"
        );
    }

    #[tokio::test]
    async fn a_link_for_a_deleted_account_reports_the_account_not_the_link() {
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;
        let code = requested_reset(&api, "ada@example.com").await;

        let (status, _) = post(
            &api,
            "/v1/accounts:delete",
            json!({ "idToken": session["idToken"].as_str().expect("id token") }),
        )
        .await;
        assert_eq!(status, 200);

        let (status, body) = post(
            &api,
            "/v1/accounts:resetPassword",
            json!({ "oobCode": code, "newPassword": "brand-new-one" }),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(
            message(&body),
            "USER_NOT_FOUND",
            "the link was fine; the account is gone"
        );
    }

    #[tokio::test]
    async fn the_admin_endpoint_hands_back_the_link_and_the_client_one_does_not() {
        // This is `generatePasswordResetLink` / `generateEmailVerificationLink`:
        // the Admin SDK asks for the link instead of an email being sent.
        let api = api();
        signed_up(&api, "ada@example.com", "secret12").await;

        let (status, body) = post(
            &api,
            "/v1/projects/demo/accounts:sendOobCode",
            json!({
                "requestType": "PASSWORD_RESET",
                "email": "ada@example.com",
                "returnOobLink": true,
            }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        let link = body["oobLink"].as_str().expect("a link");
        assert!(link.starts_with("http://127.0.0.1:4589/emulator/action?"), "{link}");
        assert!(link.contains("mode=resetPassword"), "{link}");

        // Without `returnOobLink` the admin response is the plain one.
        let (status, body) = post(
            &api,
            "/v1/projects/demo/accounts:sendOobCode",
            json!({ "requestType": "PASSWORD_RESET", "email": "ada@example.com" }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert!(body.get("oobLink").is_none(), "{body}");

        // And a client asking for it is ignored rather than obliged: a reset
        // link for any address is exactly what an attacker would want.
        let (status, body) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({
                "requestType": "PASSWORD_RESET",
                "email": "ada@example.com",
                "returnOobLink": true,
            }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert!(body.get("oobLink").is_none(), "{body}");
    }

    #[tokio::test]
    async fn the_admin_endpoint_addresses_verification_by_email_not_by_session() {
        // `generateEmailVerificationLink(email)` and
        // `generateVerifyAndChangeEmailLink(email, newEmail)` have no session to
        // offer. Requiring an idToken there would make both Admin SDK methods
        // unusable while looking like a client-side mistake.
        let api = api();
        signed_up(&api, "ada@example.com", "secret12").await;

        let (status, body) = post(
            &api,
            "/v1/projects/demo/accounts:sendOobCode",
            json!({
                "requestType": "VERIFY_EMAIL",
                "email": "ada@example.com",
                "returnOobLink": true,
            }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert!(
            body["oobLink"]
                .as_str()
                .expect("a link")
                .contains("mode=verifyEmail"),
            "{body}"
        );

        let (status, body) = post(
            &api,
            "/v1/projects/demo/accounts:sendOobCode",
            json!({
                "requestType": "VERIFY_AND_CHANGE_EMAIL",
                "email": "ada@example.com",
                "newEmail": "moved@example.com",
                "returnOobLink": true,
            }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["email"], "moved@example.com");

        // Applying it moves the account, so the admin-generated link is a real
        // one and not just a string. Two links are outstanding at the same
        // frozen instant, so pick by type rather than by order.
        let code = pending_oob_code_of(&api, "VERIFY_AND_CHANGE_EMAIL").await["oobCode"]
            .as_str()
            .expect("code")
            .to_string();
        let (status, body) = post(&api, "/v1/accounts:update", json!({ "oobCode": code })).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["email"], "moved@example.com");

        // And an unknown address is reported as such rather than as a missing
        // token.
        let (status, body) = post(
            &api,
            "/v1/projects/demo/accounts:sendOobCode",
            json!({ "requestType": "VERIFY_EMAIL", "email": "nobody@example.com" }),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "EMAIL_NOT_FOUND");

        let (status, body) = post(
            &api,
            "/v1/projects/demo/accounts:sendOobCode",
            json!({ "requestType": "VERIFY_EMAIL" }),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "MISSING_EMAIL");
    }

    #[tokio::test]
    async fn the_admin_send_endpoint_still_refuses_another_project() {
        let api = api();
        let (status, body) = post(
            &api,
            "/v1/projects/other/accounts:sendOobCode",
            json!({ "requestType": "PASSWORD_RESET", "email": "ada@example.com" }),
        )
        .await;
        assert_eq!(status, 404, "{body}");
        assert!(message(&body).starts_with("PROJECT_NOT_FOUND"), "{body}");
    }

    #[tokio::test]
    async fn following_a_verification_link_applies_it() {
        let api = api();
        let session = signed_up(&api, "ada@example.com", "secret12").await;
        let id_token = session["idToken"].as_str().expect("id token").to_string();

        let (status, _) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({ "requestType": "VERIFY_EMAIL", "idToken": id_token }),
        )
        .await;
        assert_eq!(status, 200);
        let entry = newest_oob_code(&api).await;
        let code = entry["oobCode"].as_str().expect("code").to_string();

        let (status, body) = get(
            &api,
            oob::ACTION_PATH,
            &format!("mode=verifyEmail&oobCode={code}&lang=en"),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["applied"], true);
        assert_eq!(body["emailVerified"], true);
        assert!(
            body["notice"].as_str().expect("a notice").contains("Not a Google page"),
            "the landing route must not pass itself off as Google's: {body}"
        );

        let (_, body) = post(&api, "/v1/accounts:lookup", json!({ "idToken": id_token })).await;
        assert_eq!(body["users"][0]["emailVerified"], true);
    }

    #[tokio::test]
    async fn following_a_reset_link_reports_the_call_that_finishes_it_without_consuming_it() {
        let api = api();
        signed_up(&api, "ada@example.com", "secret12").await;
        let code = requested_reset(&api, "ada@example.com").await;

        let (status, body) = get(
            &api,
            oob::ACTION_PATH,
            &format!("mode=resetPassword&oobCode={code}"),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["applied"], false, "a visit cannot invent a password");
        assert_eq!(body["mode"], "resetPassword");
        assert_eq!(body["email"], "ada@example.com");
        assert_eq!(body["finishWith"]["path"], "/v1/accounts:resetPassword");

        // The code survives the visit, or the form it points at could never be
        // submitted.
        let (status, body) = post(
            &api,
            "/v1/accounts:resetPassword",
            json!({ "oobCode": code, "newPassword": "brand-new-one" }),
        )
        .await;
        assert_eq!(status, 200, "{body}");
    }

    #[tokio::test]
    async fn an_edited_link_is_refused_rather_than_applying_a_different_action() {
        let api = api();
        signed_up(&api, "ada@example.com", "secret12").await;
        let code = requested_reset(&api, "ada@example.com").await;

        let (status, body) = get(
            &api,
            oob::ACTION_PATH,
            &format!("mode=verifyEmail&oobCode={code}"),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert!(message(&body).starts_with("INVALID_OOB_CODE"), "{body}");

        let (status, body) = get(&api, oob::ACTION_PATH, "mode=verifyEmail").await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "MISSING_OOB_CODE");

        let (status, body) = request(&api, "POST", oob::ACTION_PATH, "").await;
        assert_eq!(status, 405, "{body}");
        assert!(message(&body).contains("GET"), "{body}");
    }

    #[tokio::test]
    async fn a_continue_url_survives_the_round_trip_through_the_link() {
        let api = api();
        let app_url = "http://localhost:3000/finish?next=/dashboard&tab=1";
        let (status, _) = post(
            &api,
            "/v1/accounts:sendOobCode",
            json!({
                "requestType": "EMAIL_SIGNIN",
                "email": "link@example.com",
                "continueUrl": app_url,
            }),
        )
        .await;
        assert_eq!(status, 200);

        let entry = newest_oob_code(&api).await;
        let link = entry["oobLink"].as_str().expect("a link");
        let query = link.split_once('?').expect("a query string").1;
        let parameters = parse_pairs(query);

        assert_eq!(
            parameters["continueUrl"], app_url,
            "an app's own query must not be mangled by being nested in the link: {link}"
        );
        assert_eq!(parameters["mode"], "signIn");
        assert_eq!(parameters["oobCode"], entry["oobCode"]);

        // And the landing route echoes it, so a page that opened the link knows
        // where to send the user next.
        let (status, body) = get(&api, oob::ACTION_PATH, query).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["continueUrl"], app_url);
    }

    #[tokio::test]
    async fn clearing_the_project_clears_pending_links_too() {
        // A reset link that survived a test suite's wipe could apply to
        // whatever account later takes that uid.
        let api = api();
        signed_up(&api, "ada@example.com", "secret12").await;
        let code = requested_reset(&api, "ada@example.com").await;

        let (status, body) =
            request(&api, "DELETE", "/emulator/v1/projects/demo/accounts", "").await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["removedAccounts"], 1);
        assert_eq!(body["removedOobCodes"], 1);

        let (status, body) = post(
            &api,
            "/v1/accounts:resetPassword",
            json!({ "oobCode": code, "newPassword": "brand-new-one" }),
        )
        .await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(message(&body), "INVALID_OOB_CODE");
    }

    #[tokio::test]
    async fn pending_links_can_be_cleared_without_touching_the_accounts() {
        let api = api();
        signed_up(&api, "ada@example.com", "secret12").await;
        requested_reset(&api, "ada@example.com").await;

        let (status, body) =
            request(&api, "DELETE", "/emulator/v1/projects/demo/oobCodes", "").await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["removedOobCodes"], 1);

        let (status, body) =
            request(&api, "GET", "/emulator/v1/projects/demo/oobCodes", "").await;
        assert_eq!(status, 200, "{body}");
        assert!(body["oobCodes"].as_array().expect("array").is_empty(), "{body}");

        let (status, _) = post(
            &api,
            "/v1/accounts:signInWithPassword",
            json!({ "email": "ada@example.com", "password": "secret12" }),
        )
        .await;
        assert_eq!(status, 200, "the account must still be there");
    }

    #[tokio::test]
    async fn the_oob_listing_refuses_a_method_it_does_not_serve() {
        let api = api();
        let (status, body) =
            request(&api, "POST", "/emulator/v1/projects/demo/oobCodes", "").await;
        assert_eq!(status, 405, "{body}");
        assert!(message(&body).contains("GET or DELETE"), "{body}");
    }

    #[tokio::test]
    async fn a_broken_store_fails_the_oob_flows_as_a_500_with_the_reason() {
        let api = failing_api();

        for (path, body) in [
            (
                "/v1/accounts:sendOobCode",
                json!({ "requestType": "PASSWORD_RESET", "email": "ada@example.com" }),
            ),
            (
                "/v1/accounts:resetPassword",
                json!({ "oobCode": "whatever", "newPassword": "brand-new-one" }),
            ),
            (
                "/v1/accounts:signInWithEmailLink",
                json!({ "email": "ada@example.com", "oobCode": "whatever" }),
            ),
        ] {
            let (status, response) = post(&api, path, body.clone()).await;
            assert_eq!(status, 500, "{path} -> {response}");
            assert!(
                message(&response).contains("deliberately unavailable"),
                "{path} must report the backend's own reason: {response}"
            );
        }

        let (status, response) =
            request(&api, "GET", "/emulator/v1/projects/demo/oobCodes", "").await;
        assert_eq!(status, 500, "{response}");
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
