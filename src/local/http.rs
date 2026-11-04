//! The HTTP/1.1 surface, on the same port as gRPC.
//!
//! `fireup local` serves one port (4589 by default). gRPC clients speak HTTP/2
//! to it; humans, `curl`, Firebase Auth SDKs, and container health checks speak
//! HTTP/1.1. [`crate::local::serve`] tells the two apart by the connection
//! preface and routes each to the right handler, so there is no second port to
//! remember or publish.
//!
//! Three things live here:
//!
//! * `GET /` — service discovery, Fireup's own document.
//! * `GET /health` — a real check, which asks the store a question.
//! * the Identity Toolkit REST surface, when `auth` is enabled — see
//!   [`crate::local::auth`].
//!
//! This is a deliberately small HTTP/1.1 responder rather than a web framework.
//! It parses a request line, headers it needs, and a `Content-Length` body, and
//! answers JSON. Anything it does not recognise gets a correct error status,
//! never a half-written response.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value as Json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::debug;

use crate::local::auth::api::{self as auth_api, AuthApi, AuthRequest};
use crate::local::config::{LocalConfig, Service};
use crate::local::store::SharedStore;

/// Longest request head we will read before giving up. The routes we serve take
/// no unusual headers; anything larger is a client doing something we do not
/// support (or nothing at all).
const MAX_HEAD_BYTES: usize = 8 * 1024;

/// Longest request body we will read. The auth endpoints take small JSON
/// objects; a megabyte is far more than any of them needs and still bounds what
/// one connection can make the process allocate.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// How long a connection may take to send a complete request head.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a connection may take to finish sending the body it announced.
const BODY_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything the routes need to answer.
pub struct HttpState {
    pub config: Arc<LocalConfig>,
    pub store: SharedStore,
    /// The auth surface, present when `auth` is enabled.
    ///
    /// `None` rather than an always-present handler behind a flag, so a disabled
    /// service has no state and cannot be reached by accident.
    pub auth: Option<Arc<AuthApi>>,
}

/// A parsed request — only the parts these routes use.
#[derive(Debug, PartialEq, Eq, Default)]
pub struct Request {
    pub method: String,
    /// Path with the query string and fragment removed.
    pub path: String,
    /// Raw query string, without the leading `?`.
    pub query: String,
    pub body: Vec<u8>,
}

impl Request {
    /// A request with no query and no body, which is every `GET` these routes
    /// care about.
    #[cfg(test)]
    pub fn get(path: &str) -> Self {
        Self {
            method: "GET".to_string(),
            path: path.to_string(),
            ..Default::default()
        }
    }
}

/// Parse the first line of an HTTP/1.x request: `GET /health HTTP/1.1`.
///
/// The query string is split off rather than dropped: `/health?probe=1` is the
/// same route as `/health`, but `accounts:batchGet?maxResults=2` needs its
/// parameters.
pub fn parse_request_line(head: &str) -> Option<Request> {
    let first_line = head.lines().next()?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?;
    // A version is required by HTTP/1.x; without it this is not a request we
    // should try to interpret.
    parts.next()?;

    // The fragment is not sent by compliant clients and is never part of the
    // resource, so it is discarded whichever side of the '?' it appears on.
    let target = target.split('#').next().unwrap_or(target);
    let (path, query) = target.split_once('?').unwrap_or((target, ""));

    Some(Request {
        method,
        path: path.to_string(),
        query: query.to_string(),
        body: Vec::new(),
    })
}

/// Read the `Content-Length` a request head announced.
///
/// `None` for a head without one, which means no body. A header that is present
/// but unparseable is `Err`: guessing zero would make the server answer a
/// request whose body it then left in the socket.
fn content_length(head: &str) -> Result<Option<usize>, String> {
    let mut found = None;
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("content-length") {
            continue;
        }
        let value = value.trim();
        let length: usize = value
            .parse()
            .map_err(|_| format!("Content-Length {value:?} is not a length"))?;
        // Two different lengths is a request-smuggling shape, not a request.
        if found.is_some_and(|existing| existing != length) {
            return Err("conflicting Content-Length headers".to_string());
        }
        found = Some(length);
    }
    Ok(found)
}

/// Whether the head asks for a body this responder cannot read.
///
/// Chunked bodies are refused rather than mis-read: no client of these endpoints
/// sends one, and silently treating the chunk framing as JSON would produce a
/// baffling parse error.
fn is_chunked(head: &str) -> bool {
    head.lines().skip(1).any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("transfer-encoding")
                && value.to_ascii_lowercase().contains("chunked")
        })
    })
}

/// The discovery document: what this process is and what it serves.
pub fn discovery_body(config: &LocalConfig, store: &dyn crate::local::store::Store) -> Json {
    json!({
        "product": "fireup local",
        "version": env!("CARGO_PKG_VERSION"),
        "notice": "Independent implementation of public Google API contracts. \
                   Not the Firebase Emulator Suite and not affiliated with Google.",
        "projectId": config.project_id,
        "databaseId": config.database_id,
        "storage": {
            "backend": store.backend(),
            "durable": config.postgres_url.is_some(),
        },
        "services": config
            .services
            .iter()
            .map(|service| json!({
                "id": service.id(),
                "api": service.api(),
                "protocol": service.protocol(),
                "enabled": true,
                "clientEnvVar": service.client_env_var(),
            }))
            .collect::<Vec<_>>(),
        "endpoints": endpoints(config),
    })
}

/// The routes this process actually answers.
///
/// Built conditionally rather than with nulls for the disabled ones, so
/// discovery never points a developer at a path that would 404.
fn endpoints(config: &LocalConfig) -> Json {
    let mut endpoints = serde_json::Map::new();
    endpoints.insert("discovery".into(), Json::from("/"));
    endpoints.insert("health".into(), Json::from("/health"));
    if config.enables(Service::Auth) {
        endpoints.insert(
            "identityToolkit".into(),
            Json::from("/identitytoolkit.googleapis.com/v1/"),
        );
        endpoints.insert(
            "secureToken".into(),
            Json::from("/securetoken.googleapis.com/v1/token"),
        );
    }
    Json::Object(endpoints)
}

/// Route a request to its response: an HTTP status and a JSON body.
///
/// Health is a real check, not a constant: it asks the store for a document
/// count, so a `fireup local` that has lost its database reports unhealthy
/// instead of cheerfully answering 200 while every RPC fails.
pub async fn route(request: &Request, state: &HttpState) -> (u16, Json) {
    // The auth surface owns its paths and its own error envelope, so it is
    // consulted before the read-only check below — its endpoints are POSTs.
    if auth_api::handles(&request.path) {
        return match &state.auth {
            Some(auth) => {
                auth.route(&AuthRequest {
                    method: &request.method,
                    path: &request.path,
                    query: &request.query,
                    body: &request.body,
                })
                .await
            }
            // Enabled services are a closed set, so a request to a disabled one
            // says it is disabled rather than that the path does not exist.
            None => auth_api::error_response(
                404,
                "NOT_FOUND : the auth service is not enabled; start fireup local with --enable auth",
            ),
        };
    }

    if request.method != "GET" && request.method != "HEAD" {
        return (
            405,
            json!({
                "error": format!(
                    "{} is not supported on {}; the discovery and health routes are read-only",
                    request.method, request.path
                ),
                "endpoints": ["/", "/health"],
            }),
        );
    }

    match request.path.as_str() {
        "/" => (200, discovery_body(&state.config, state.store.as_ref())),
        "/health" => match state.store.count().await {
            Ok(documents) => (
                200,
                json!({
                    "status": "ok",
                    "storage": {
                        "backend": state.store.backend(),
                        "documents": documents,
                    },
                }),
            ),
            Err(error) => (
                503,
                json!({
                    "status": "unhealthy",
                    "storage": { "backend": state.store.backend() },
                    "error": error.to_string(),
                }),
            ),
        },
        unknown => (
            404,
            json!({
                "error": format!("no route {unknown}"),
                "endpoints": ["/", "/health"],
            }),
        ),
    }
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

/// Render a complete HTTP/1.1 response.
///
/// `Connection: close` because this responder serves one request per
/// connection; saying so keeps clients from waiting for a keep-alive reply that
/// will never come.
pub fn render_response(status: u16, body: &Json, include_body: bool) -> Vec<u8> {
    let body = format!("{}\n", serde_json::to_string_pretty(body).unwrap_or_default());
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {length}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n",
        reason = reason_phrase(status),
        length = body.len(),
    );

    let mut out = head.into_bytes();
    if include_body {
        out.extend_from_slice(body.as_bytes());
    }
    out
}

/// Read one request from `stream`, answer it, and close.
pub async fn serve_connection(mut stream: TcpStream, state: Arc<HttpState>) {
    let (status, body, include_body) = match read_request(&mut stream).await {
        Ok(request) => {
            debug!(
                method = %request.method,
                path = %request.path,
                body_bytes = request.body.len(),
                "http request"
            );
            let include_body = request.method != "HEAD";
            let (status, body) = route(&request, &state).await;
            (status, body, include_body)
        }
        Err(RequestError::Malformed(reason)) => {
            (400, json!({ "error": reason }), true)
        }
        Err(RequestError::TooLarge(reason)) => (413, json!({ "error": reason }), true),
        Err(RequestError::Timeout) => (
            408,
            json!({ "error": "timed out waiting for the request" }),
            true,
        ),
        // A connection that broke before sending anything has nobody to answer.
        Err(RequestError::Closed) => return,
    };

    let response = render_response(status, &body, include_body);
    if let Err(error) = stream.write_all(&response).await {
        debug!(%error, "failed to write the HTTP response");
    }
    let _ = stream.flush().await;
}

enum RequestError {
    Malformed(String),
    TooLarge(String),
    Timeout,
    Closed,
}

/// Read a complete request: the head, then as much body as it announced.
async fn read_request(stream: &mut TcpStream) -> Result<Request, RequestError> {
    let (head, mut leftover) = read_head(stream).await?;

    let mut request = parse_request_line(&head)
        .ok_or_else(|| RequestError::Malformed("could not parse the HTTP request line".into()))?;

    if is_chunked(&head) {
        return Err(RequestError::Malformed(
            "chunked request bodies are not supported; send Content-Length".into(),
        ));
    }

    let length = content_length(&head).map_err(RequestError::Malformed)?;
    let Some(length) = length else {
        // No `Content-Length` means no body. Anything already read past the head
        // in that case is a pipelined second request, which this responder does
        // not serve; it closes the connection, so dropping it is correct.
        return Ok(request);
    };
    if length > MAX_BODY_BYTES {
        return Err(RequestError::TooLarge(format!(
            "request body of {length} bytes exceeds the {MAX_BODY_BYTES}-byte limit"
        )));
    }

    // The head read may have pulled part — or all — of the body along with it.
    leftover.truncate(length);
    let mut body = leftover;
    let mut chunk = [0u8; 4096];
    while body.len() < length {
        let wanted = (length - body.len()).min(chunk.len());
        let read = tokio::time::timeout(BODY_TIMEOUT, stream.read(&mut chunk[..wanted]))
            .await
            .map_err(|_| RequestError::Timeout)?
            .map_err(|_| RequestError::Closed)?;
        if read == 0 {
            return Err(RequestError::Malformed(format!(
                "connection closed after {} of {length} announced body bytes",
                body.len()
            )));
        }
        body.extend_from_slice(&chunk[..read]);
    }

    request.body = body;
    Ok(request)
}

/// Read until the end of the request head (`\r\n\r\n`).
///
/// Returns the head and whatever bytes arrived after it, which belong to the
/// body: a client that writes head and body in one packet is the normal case,
/// and discarding the tail here would hang waiting for bytes already delivered.
async fn read_head(stream: &mut TcpStream) -> Result<(String, Vec<u8>), RequestError> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];

    loop {
        // The terminator may straddle two reads, so the search starts a little
        // before the newly read bytes rather than at their start.
        let search_from = buffer.len().saturating_sub(3);
        let read = tokio::time::timeout(HEAD_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| RequestError::Timeout)?
            .map_err(|_| RequestError::Closed)?;
        if read == 0 {
            // EOF. If something arrived, try to parse it; a request without the
            // terminator but with a complete first line is still answerable.
            return if buffer.is_empty() {
                Err(RequestError::Closed)
            } else {
                Ok((String::from_utf8_lossy(&buffer).into_owned(), Vec::new()))
            };
        }
        buffer.extend_from_slice(&chunk[..read]);

        if let Some(offset) = buffer[search_from..]
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
        {
            let head_end = search_from + offset + 4;
            let rest = buffer.split_off(head_end);
            return Ok((String::from_utf8_lossy(&buffer).into_owned(), rest));
        }
        if buffer.len() > MAX_HEAD_BYTES {
            return Err(RequestError::TooLarge(format!(
                "request head exceeded {MAX_HEAD_BYTES} bytes"
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::auth::store::MemoryUserStore;
    use crate::local::config::{LocalConfig, Service};
    use crate::local::store::{MemoryStore, Store};

    fn config() -> Arc<LocalConfig> {
        Arc::new(LocalConfig::in_memory(
            "127.0.0.1:4589".parse().expect("valid address"),
        ))
    }

    fn state_with(config: Arc<LocalConfig>, store: SharedStore) -> HttpState {
        let auth = config.enables(Service::Auth).then(|| {
            Arc::new(AuthApi::new(
                config.clone(),
                Arc::new(MemoryUserStore::new()),
            ))
        });
        HttpState {
            config,
            store,
            auth,
        }
    }

    fn state() -> HttpState {
        state_with(config(), Arc::new(MemoryStore::new()))
    }

    fn get(path: &str) -> Request {
        Request::get(path)
    }

    #[test]
    fn parses_a_normal_request_line() {
        assert_eq!(
            parse_request_line("GET /health HTTP/1.1\r\nHost: x\r\n\r\n"),
            Some(get("/health"))
        );
    }

    #[test]
    fn splits_the_query_string_off_and_drops_the_fragment() {
        // `/health?probe=1` is the same route as `/health`, but
        // `accounts:batchGet?maxResults=2` needs its parameters kept.
        assert_eq!(
            parse_request_line("GET /health?probe=1 HTTP/1.1\r\n\r\n"),
            Some(Request {
                method: "GET".into(),
                path: "/health".into(),
                query: "probe=1".into(),
                body: Vec::new(),
            })
        );
        assert_eq!(
            parse_request_line("GET /health#frag HTTP/1.1\r\n\r\n"),
            Some(get("/health"))
        );
    }

    #[test]
    fn rejects_garbage_instead_of_guessing_a_route() {
        assert_eq!(parse_request_line(""), None);
        assert_eq!(parse_request_line("GET\r\n"), None);
        assert_eq!(parse_request_line("GET /health\r\n"), None, "no version");
    }

    #[test]
    fn content_length_is_read_case_insensitively_and_conflicts_are_refused() {
        assert_eq!(
            content_length("POST / HTTP/1.1\r\ncontent-length: 12\r\n\r\n"),
            Ok(Some(12))
        );
        assert_eq!(content_length("POST / HTTP/1.1\r\nHost: x\r\n\r\n"), Ok(None));
        // A duplicate header that agrees with itself is harmless.
        assert_eq!(
            content_length("POST / HTTP/1.1\r\nContent-Length: 3\r\nContent-Length: 3\r\n\r\n"),
            Ok(Some(3))
        );
        assert!(
            content_length("POST / HTTP/1.1\r\nContent-Length: 3\r\nContent-Length: 4\r\n\r\n")
                .is_err(),
            "two different lengths is not a request"
        );
        assert!(content_length("POST / HTTP/1.1\r\nContent-Length: ten\r\n\r\n").is_err());
    }

    #[test]
    fn a_chunked_body_is_detected_rather_than_mis_read() {
        assert!(is_chunked(
            "POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n"
        ));
        assert!(!is_chunked("POST / HTTP/1.1\r\nContent-Length: 2\r\n\r\n"));
    }

    #[tokio::test]
    async fn discovery_lists_the_enabled_services_and_the_backend() {
        let state = state();
        let (status, body) = route(&get("/"), &state).await;

        assert_eq!(status, 200);
        assert_eq!(body["storage"]["backend"], "memory");
        assert_eq!(body["storage"]["durable"], false);
        assert_eq!(body["services"][0]["id"], Service::Firestore.id());
        assert_eq!(body["services"][0]["api"], "google.firestore.v1.Firestore");
        assert_eq!(body["services"][0]["protocol"], "grpc");
        assert_eq!(body["projectId"], LocalConfig::DEFAULT_PROJECT_ID);
    }

    #[tokio::test]
    async fn discovery_names_the_auth_env_var_and_its_endpoints_when_enabled() {
        let (_, body) = route(&get("/"), &state()).await;

        let auth = body["services"]
            .as_array()
            .expect("services")
            .iter()
            .find(|service| service["id"] == Service::Auth.id())
            .unwrap_or_else(|| panic!("auth must be listed: {body}"));
        assert_eq!(auth["clientEnvVar"], "FIREBASE_AUTH_EMULATOR_HOST");
        assert_eq!(auth["protocol"], "rest");
        assert_eq!(auth["api"], "identitytoolkit.googleapis.com");

        assert_eq!(
            body["endpoints"]["identityToolkit"],
            "/identitytoolkit.googleapis.com/v1/"
        );
        assert_eq!(
            body["endpoints"]["secureToken"],
            "/securetoken.googleapis.com/v1/token"
        );
    }

    #[tokio::test]
    async fn discovery_omits_the_auth_endpoints_when_auth_is_off() {
        let config = Arc::new(config().as_ref().clone().with_only(Service::Firestore));
        let state = state_with(config, Arc::new(MemoryStore::new()));

        let (_, body) = route(&get("/"), &state).await;
        assert!(
            body["endpoints"].get("identityToolkit").is_none(),
            "discovery must not advertise a route that 404s: {body}"
        );
        assert_eq!(body["services"].as_array().expect("services").len(), 1);
    }

    #[tokio::test]
    async fn auth_paths_say_the_service_is_disabled_rather_than_missing() {
        let config = Arc::new(config().as_ref().clone().with_only(Service::Firestore));
        let state = state_with(config, Arc::new(MemoryStore::new()));

        let (status, body) = route(
            &Request {
                method: "POST".into(),
                path: "/identitytoolkit.googleapis.com/v1/accounts:signUp".into(),
                query: String::new(),
                body: b"{}".to_vec(),
            },
            &state,
        )
        .await;

        assert_eq!(status, 404);
        let message = body["error"]["message"].as_str().expect("an error envelope");
        assert!(message.contains("--enable auth"), "{message}");
    }

    #[tokio::test]
    async fn an_auth_post_reaches_the_auth_surface() {
        let state = state();
        let (status, body) = route(
            &Request {
                method: "POST".into(),
                path: "/identitytoolkit.googleapis.com/v1/accounts:signUp".into(),
                query: String::new(),
                body: br#"{"email":"ada@example.com","password":"secret12"}"#.to_vec(),
            },
            &state,
        )
        .await;

        assert_eq!(status, 200, "{body}");
        assert_eq!(body["email"], "ada@example.com");
    }

    #[tokio::test]
    async fn health_reports_the_document_count_from_the_store() {
        let store = MemoryStore::new();
        let state = state_with(config(), Arc::new(store.clone()));

        let (status, body) = route(&get("/health"), &state).await;
        assert_eq!(status, 200);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["storage"]["documents"], 0);

        store
            .put(
                crate::local::store::conformance::doc(
                    "projects/p/databases/(default)/documents/c/d",
                    "k",
                    "v",
                ),
                prost_types::Timestamp { seconds: 1, nanos: 0 },
            )
            .await
            .expect("put");

        let (_, body) = route(&get("/health"), &state).await;
        assert_eq!(body["storage"]["documents"], 1);
    }

    #[tokio::test]
    async fn unknown_routes_and_writes_are_refused() {
        let state = state();

        let (status, _) = route(&get("/nope"), &state).await;
        assert_eq!(status, 404);

        let (status, _) = route(
            &Request {
                method: "POST".to_string(),
                path: "/".to_string(),
                ..Default::default()
            },
            &state,
        )
        .await;
        assert_eq!(
            status, 405,
            "discovery and health must stay read-only even though auth takes POSTs"
        );
    }

    #[test]
    fn responses_declare_the_body_length_they_send() {
        let body = json!({ "status": "ok" });
        let rendered = String::from_utf8(render_response(200, &body, true)).expect("utf8");
        let (head, sent_body) = rendered.split_once("\r\n\r\n").expect("head terminator");

        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
        assert!(head.contains(&format!("Content-Length: {}", sent_body.len())), "{head}");
        assert_eq!(
            serde_json::from_str::<Json>(sent_body).expect("body is JSON"),
            body
        );
    }

    #[test]
    fn head_requests_send_headers_without_a_body() {
        let body = json!({ "status": "ok" });
        let rendered = String::from_utf8(render_response(200, &body, false)).expect("utf8");

        assert!(rendered.ends_with("\r\n\r\n"), "{rendered}");
        // The length still describes the resource, as HEAD requires.
        assert!(rendered.contains("Content-Length: "), "{rendered}");
    }
}
