//! Out-of-band (OOB) codes: the email-link half of the auth surface.
//!
//! Identity Toolkit's email flows all work the same way. A client asks for an
//! action (`accounts:sendOobCode`), the service mints a single-use **oobCode**
//! and mails the user a link carrying it, and the click comes back as a second
//! call that redeems the code (`accounts:resetPassword`, `accounts:update`,
//! `accounts:signInWithEmailLink`).
//!
//! ## There is no mail transport, on purpose
//!
//! `fireup local` does not send email, and will not: an SMTP client in a local
//! development tool is a credential to configure, a port to open, and a way to
//! accidentally mail a real person from a test suite. So the link goes
//! **nowhere outbound**. Instead it is:
//!
//! * logged at `info` by the handler that minted it, so it is visible in the
//!   terminal the server runs in, and
//! * readable at `GET /emulator/v1/projects/{project}/oobCodes`, the documented
//!   local-only control endpoint whose whole purpose is letting a test read a
//!   link the user would have clicked.
//!
//! That is the same bargain the documented Auth emulator convention makes, and
//! it is what lets an end-to-end test drive "request a reset, follow the link,
//! sign in with the new password" without a mailbox.
//!
//! ## A code is single-use and short-lived
//!
//! Redeeming a code deletes it, so a link in a shared terminal scrollback stops
//! working once used. Codes also expire — see [`TTL_SECONDS`] — because a reset
//! link that works forever is the one real security property of these flows
//! that is cheap to keep, even locally.
//!
//! ## Compliance
//!
//! `requestType` values, the `mode` names in the link, and the error codes
//! clients match on come from Google's **published** Identity Toolkit REST
//! reference and the public Auth emulator documentation. The landing page the
//! link points at is Fireup's own JSON, not a copy of anyone's page: see
//! [`ACTION_PATH`]. Nothing here was derived from a Google binary.

use std::net::SocketAddr;

use super::user::AuthUser;

/// How long a minted code stays redeemable.
///
/// One hour, which is the published default lifetime of a Firebase password
/// reset link. Fireup uses it for every request type rather than giving each
/// one its own window: a local flow is finished in seconds, a single number is
/// one fewer thing for a developer to be surprised by, and the shortest
/// documented default is the conservative choice.
pub const TTL_SECONDS: i64 = 3600;

/// Raw bytes of entropy in a generated `oobCode`.
///
/// 32 bytes, so the code is 43 base64url characters — long enough that a code
/// cannot be guessed from the listing endpoint's rate of change, and
/// URL-safe without escaping.
const CODE_BYTES: usize = 32;

/// Where the generated links point.
///
/// Fireup's own path, under the `/emulator/` namespace that is local-only by
/// convention, and *not* a production Identity Toolkit route. It answers JSON
/// describing the pending action, because every other response on this port is
/// JSON and a developer following the link with `curl` is the expected case.
pub const ACTION_PATH: &str = "/emulator/action";

/// The `apiKey` a link carries when the request that asked for it sent none.
///
/// The link **must** carry an `apiKey`: the official client SDK's link parser
/// refuses a link without one outright, which makes
/// `isSignInWithEmailLink` answer false and `signInWithEmailLink` fail with an
/// argument error. That was found by running the real SDK against this surface,
/// not by reading the docs — the parameter looked optional until an actual
/// `sendSignInLinkToEmail` → `signInWithEmailLink` round trip refused to work.
///
/// Whatever key the client sent is echoed back instead, so an app that reads
/// the key out of the link gets its own. This placeholder is for `curl` and the
/// admin endpoints, which send none. It is deliberately a sentence rather than
/// a key-shaped string, because nothing here validates an API key and a value
/// that looked real would imply otherwise.
pub const UNCHECKED_API_KEY: &str = "fireup-local-does-not-check-api-keys";

/// What an email action is for.
///
/// The four flows a Firebase web app reaches after sign-up or sign-in. Phone
/// verification (`PHONE_VERIFICATION`) is deliberately absent — phone auth is a
/// hard non-goal — and is refused by name rather than being parsed into
/// something half-served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OobRequestType {
    /// `sendPasswordResetEmail` → `confirmPasswordReset`.
    PasswordReset,
    /// `sendEmailVerification` → `applyActionCode`.
    VerifyEmail,
    /// `sendSignInLinkToEmail` → `signInWithEmailLink`.
    EmailSignin,
    /// `verifyBeforeUpdateEmail` → `applyActionCode`.
    VerifyAndChangeEmail,
}

impl OobRequestType {
    /// The `requestType` value on the wire.
    pub const fn as_str(self) -> &'static str {
        match self {
            OobRequestType::PasswordReset => "PASSWORD_RESET",
            OobRequestType::VerifyEmail => "VERIFY_EMAIL",
            OobRequestType::EmailSignin => "EMAIL_SIGNIN",
            OobRequestType::VerifyAndChangeEmail => "VERIFY_AND_CHANGE_EMAIL",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "PASSWORD_RESET" => Some(OobRequestType::PasswordReset),
            "VERIFY_EMAIL" => Some(OobRequestType::VerifyEmail),
            "EMAIL_SIGNIN" => Some(OobRequestType::EmailSignin),
            "VERIFY_AND_CHANGE_EMAIL" => Some(OobRequestType::VerifyAndChangeEmail),
            _ => None,
        }
    }

    /// The `mode` query parameter the generated link carries.
    ///
    /// These are the documented values an app reads off an email action link to
    /// decide which screen to show, so they have to be exactly these strings —
    /// an app switching on `mode` is the normal way these flows are wired up.
    pub const fn link_mode(self) -> &'static str {
        match self {
            OobRequestType::PasswordReset => "resetPassword",
            OobRequestType::VerifyEmail => "verifyEmail",
            OobRequestType::EmailSignin => "signIn",
            OobRequestType::VerifyAndChangeEmail => "verifyAndChangeEmail",
        }
    }

    /// The request type a link `mode` names, for the landing route.
    pub fn from_link_mode(mode: &str) -> Option<Self> {
        [
            OobRequestType::PasswordReset,
            OobRequestType::VerifyEmail,
            OobRequestType::EmailSignin,
            OobRequestType::VerifyAndChangeEmail,
        ]
        .into_iter()
        .find(|request_type| request_type.link_mode() == mode)
    }
}

/// One pending email action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OobCode {
    /// The opaque single-use code the link carries.
    pub oob_code: String,
    pub request_type: OobRequestType,
    /// The address the link was "sent" to, already normalised. For
    /// [`OobRequestType::VerifyAndChangeEmail`] that is the **new** address,
    /// because that is the mailbox that has to prove it exists.
    pub email: String,
    /// The account the action applies to.
    ///
    /// `None` only for [`OobRequestType::EmailSignin`] to an address with no
    /// account yet: following that link creates one, which is the documented
    /// behaviour of email-link sign-in.
    pub local_id: Option<String>,
    /// For [`OobRequestType::VerifyAndChangeEmail`], the address to move the
    /// account to once the link is followed.
    pub new_email: Option<String>,
    /// The app URL a client asked to be sent back to, echoed into the link.
    pub continue_url: Option<String>,
    /// The `?key=` the requesting client sent, or [`UNCHECKED_API_KEY`].
    ///
    /// Stored rather than recomputed, so the link in the `oobCodes` listing is
    /// character-for-character the one the client would have been mailed.
    pub api_key: String,
    pub created_at_millis: i64,
    pub expires_at_millis: i64,
}

impl OobCode {
    /// Mint a code for `email`, expiring [`TTL_SECONDS`] from `now_millis`.
    pub fn new(
        request_type: OobRequestType,
        email: impl Into<String>,
        now_millis: i64,
    ) -> Self {
        Self {
            oob_code: super::random_token(CODE_BYTES),
            request_type,
            email: AuthUser::normalize_email(&email.into()),
            local_id: None,
            new_email: None,
            continue_url: None,
            api_key: UNCHECKED_API_KEY.to_string(),
            created_at_millis: now_millis,
            expires_at_millis: now_millis + TTL_SECONDS * 1000,
        }
    }

    pub fn for_account(mut self, local_id: impl Into<String>) -> Self {
        self.local_id = Some(local_id.into());
        self
    }

    pub fn with_new_email(mut self, new_email: impl Into<String>) -> Self {
        self.new_email = Some(AuthUser::normalize_email(&new_email.into()));
        self
    }

    pub fn with_continue_url(mut self, continue_url: Option<String>) -> Self {
        self.continue_url = continue_url;
        self
    }

    /// Carry the caller's own API key into the link, when it sent one.
    pub fn with_api_key(mut self, api_key: Option<String>) -> Self {
        if let Some(api_key) = api_key.filter(|key| !key.is_empty()) {
            self.api_key = api_key;
        }
        self
    }

    /// Whether the code is still redeemable at `now_millis`.
    pub fn is_live_at(&self, now_millis: i64) -> bool {
        now_millis < self.expires_at_millis
    }

    /// The link a user would have received.
    ///
    /// `base_url` is where this server is reachable — see [`action_base_url`].
    /// The parameter names are the documented ones (`mode`, `oobCode`,
    /// `apiKey`, `continueUrl`, `lang`) so the official client SDK's own link
    /// parser accepts this link: it requires `apiKey`, `oobCode` and a known
    /// `mode`, and rejects the link outright without all three. See
    /// [`UNCHECKED_API_KEY`] for what goes in `apiKey` and why.
    pub fn action_link(&self, base_url: &str) -> String {
        let mut link = format!(
            "{}{ACTION_PATH}?mode={}&oobCode={}&apiKey={}&lang=en",
            base_url.trim_end_matches('/'),
            self.request_type.link_mode(),
            self.oob_code,
            percent_encode_component(&self.api_key),
        );
        if let Some(continue_url) = &self.continue_url {
            link.push_str("&continueUrl=");
            link.push_str(&percent_encode_component(continue_url));
        }
        link
    }

    /// The entry the `oobCodes` control endpoint reports.
    pub fn to_listing_entry(&self, base_url: &str) -> serde_json::Value {
        serde_json::json!({
            "email": self.email,
            "requestType": self.request_type.as_str(),
            "oobCode": self.oob_code,
            "oobLink": self.action_link(base_url),
        })
    }
}

/// Where to tell a user to go, given the address the server is bound to.
///
/// A wildcard bind (`0.0.0.0`) is turned into loopback rather than written into
/// the link verbatim: `http://0.0.0.0:4589/...` is not a URL a browser or
/// `curl` can follow, and loopback is both reachable and the bind this tool
/// tells people to keep.
pub fn action_base_url(addr: SocketAddr) -> String {
    if addr.ip().is_unspecified() {
        let loopback = if addr.is_ipv6() { "[::1]" } else { "127.0.0.1" };
        return format!("http://{loopback}:{}", addr.port());
    }
    match addr {
        SocketAddr::V6(_) => format!("http://[{}]:{}", addr.ip(), addr.port()),
        SocketAddr::V4(_) => format!("http://{}:{}", addr.ip(), addr.port()),
    }
}

/// Percent-encode a value for use in a query string.
///
/// The unreserved set from RFC 3986 §2.3 is kept as-is and everything else is
/// escaped, which is stricter than necessary and therefore always correct —
/// a `continueUrl` of `http://localhost:3000/?a=b&c=d` has to survive being
/// nested inside another query string without its `&` splitting the link.
fn percent_encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_request_type_round_trips_through_its_wire_value() {
        for request_type in [
            OobRequestType::PasswordReset,
            OobRequestType::VerifyEmail,
            OobRequestType::EmailSignin,
            OobRequestType::VerifyAndChangeEmail,
        ] {
            assert_eq!(
                OobRequestType::parse(request_type.as_str()),
                Some(request_type),
                "{} must parse back",
                request_type.as_str()
            );
            assert_eq!(
                OobRequestType::from_link_mode(request_type.link_mode()),
                Some(request_type),
                "mode {} must parse back",
                request_type.link_mode()
            );
        }
    }

    #[test]
    fn phone_verification_is_not_a_request_type_this_build_parses() {
        // Phone auth is a hard non-goal; parsing it would mean minting a code
        // for a flow nothing here can finish.
        assert_eq!(OobRequestType::parse("PHONE_VERIFICATION"), None);
        assert_eq!(OobRequestType::parse("OLD_EMAIL_AGREE"), None);
    }

    #[test]
    fn a_minted_code_expires_an_hour_out_and_is_live_until_then() {
        let code = OobCode::new(OobRequestType::PasswordReset, "Ada@Example.com", 1_000);
        assert_eq!(code.expires_at_millis, 1_000 + 3_600_000);
        assert!(code.is_live_at(1_000));
        assert!(code.is_live_at(1_000 + 3_599_999));
        assert!(!code.is_live_at(1_000 + 3_600_000), "expiry is exclusive");
    }

    #[test]
    fn a_minted_code_normalises_the_address_it_targets() {
        // The lookup that redeems it goes through the same normalisation, so a
        // code stored with different case would be unredeemable.
        let code = OobCode::new(OobRequestType::EmailSignin, "  Ada@Example.COM ", 0);
        assert_eq!(code.email, "ada@example.com");
        assert_eq!(
            OobCode::new(OobRequestType::VerifyAndChangeEmail, "a@b.com", 0)
                .with_new_email("NEW@B.com")
                .new_email
                .as_deref(),
            Some("new@b.com")
        );
    }

    #[test]
    fn generated_codes_are_url_safe_and_do_not_repeat() {
        let codes: std::collections::HashSet<String> = (0..200)
            .map(|_| OobCode::new(OobRequestType::VerifyEmail, "a@b.com", 0).oob_code)
            .collect();
        assert_eq!(codes.len(), 200);
        for code in codes {
            assert!(
                code.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "a code travels in a query string: {code}"
            );
        }
    }

    #[test]
    fn the_link_carries_the_three_parameters_the_client_sdk_requires() {
        // The official client SDK's link parser asserts on apiKey, oobCode and
        // a known mode together, and returns null without all three — which is
        // how `isSignInWithEmailLink` silently answers false.
        let code = OobCode::new(OobRequestType::PasswordReset, "ada@example.com", 0);
        let link = code.action_link("http://127.0.0.1:4589");

        assert!(link.starts_with("http://127.0.0.1:4589/emulator/action?"), "{link}");
        assert!(link.contains("mode=resetPassword"), "{link}");
        assert!(link.contains(&format!("oobCode={}", code.oob_code)), "{link}");
        assert!(link.contains("apiKey="), "{link}");
    }

    #[test]
    fn the_link_echoes_the_callers_api_key_and_otherwise_says_none_is_checked() {
        let code = OobCode::new(OobRequestType::EmailSignin, "ada@example.com", 0);
        assert!(
            code.action_link("http://x")
                .contains(&format!("apiKey={UNCHECKED_API_KEY}")),
            "a request with no key still has to produce a parseable link"
        );
        assert!(
            !UNCHECKED_API_KEY.starts_with("AIza"),
            "the placeholder must not look like a real key: {UNCHECKED_API_KEY}"
        );

        let with_key = code.clone().with_api_key(Some("app-own-key".into()));
        assert!(with_key.action_link("http://x").contains("apiKey=app-own-key"));

        // An empty `?key=` is no key at all, not a key of "".
        assert_eq!(
            code.clone().with_api_key(Some(String::new())).api_key,
            UNCHECKED_API_KEY
        );
        assert_eq!(code.with_api_key(None).api_key, UNCHECKED_API_KEY);
    }

    #[test]
    fn a_continue_url_is_escaped_so_its_own_query_does_not_split_the_link() {
        let code = OobCode::new(OobRequestType::EmailSignin, "ada@example.com", 0)
            .with_continue_url(Some("http://localhost:3000/finish?next=/a&x=1".into()));
        let link = code.action_link("http://127.0.0.1:4589/");

        assert!(
            link.contains("&continueUrl=http%3A%2F%2Flocalhost%3A3000%2Ffinish%3Fnext%3D%2Fa%26x%3D1"),
            "{link}"
        );
        // Exactly one '?' — the nested one must be escaped.
        assert_eq!(link.matches('?').count(), 1, "{link}");
        // The base URL's trailing slash must not double up.
        assert!(!link.contains("4589//"), "{link}");
    }

    #[test]
    fn a_wildcard_bind_becomes_a_url_someone_can_actually_follow() {
        assert_eq!(
            action_base_url("0.0.0.0:4589".parse().expect("addr")),
            "http://127.0.0.1:4589"
        );
        assert_eq!(
            action_base_url("[::]:4589".parse().expect("addr")),
            "http://[::1]:4589"
        );
        assert_eq!(
            action_base_url("127.0.0.1:9999".parse().expect("addr")),
            "http://127.0.0.1:9999"
        );
        // A literal IPv6 host has to be bracketed or the port reads as part of
        // the address.
        assert_eq!(
            action_base_url("[::1]:4589".parse().expect("addr")),
            "http://[::1]:4589"
        );
    }

    #[test]
    fn a_listing_entry_has_the_four_fields_a_test_harness_reads() {
        let code = OobCode::new(OobRequestType::VerifyEmail, "ada@example.com", 0)
            .for_account("uid-1");
        let entry = code.to_listing_entry("http://127.0.0.1:4589");

        assert_eq!(entry["email"], "ada@example.com");
        assert_eq!(entry["requestType"], "VERIFY_EMAIL");
        assert_eq!(entry["oobCode"], code.oob_code.as_str());
        assert_eq!(entry["oobLink"], code.action_link("http://127.0.0.1:4589"));
        // The uid is internal bookkeeping, not part of the documented entry.
        assert!(entry.get("localId").is_none(), "{entry}");
    }
}
