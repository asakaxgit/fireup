//! Health and service discovery over HTTP/1.1, on the same port as gRPC.
//!
//! `fireup local` serves one port (4589 by default). gRPC clients speak HTTP/2
//! to it; humans, `curl`, and container health checks speak HTTP/1.1 to `/` and
//! `/health`. [`crate::local::serve`] tells the two apart by the connection
//! preface and routes each to the right handler, so there is no second port to
//! remember or publish.
//!
//! This is a deliberately small HTTP/1.1 responder rather than a web framework:
//! it answers two fixed `GET` routes with JSON, which does not justify adding
//! an HTTP stack to the dependency tree. Anything it does not recognise gets a
//! correct error status, never a half-written response.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value as Json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::debug;

use crate::local::config::LocalConfig;
use crate::local::store::SharedStore;

/// Longest request head we will read before giving up. The two routes we serve
/// take no body and no unusual headers; anything larger is a client doing
/// something we do not support (or nothing at all).
const MAX_HEAD_BYTES: usize = 8 * 1024;

/// How long a connection may take to send a complete request head.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);

/// Everything the two routes need to answer.
pub struct HttpState {
    pub config: Arc<LocalConfig>,
    pub store: SharedStore,
}

/// A parsed request head — only the parts these routes use.
#[derive(Debug, PartialEq, Eq)]
pub struct RequestLine {
    pub method: String,
    pub path: String,
}

/// Parse the first line of an HTTP/1.x request: `GET /health HTTP/1.1`.
///
/// The query string is dropped: `/health?probe=1` is the same route, which is
/// what container health checks and browsers tend to send.
pub fn parse_request_line(head: &str) -> Option<RequestLine> {
    let first_line = head.lines().next()?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?;
    // A version is required by HTTP/1.x; without it this is not a request we
    // should try to interpret.
    parts.next()?;

    let path = target
        .split(['?', '#'])
        .next()
        .unwrap_or(target)
        .to_string();
    Some(RequestLine { method, path })
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
                "protocol": "grpc",
                "enabled": true,
                "clientEnvVar": service.client_env_var(),
            }))
            .collect::<Vec<_>>(),
        "endpoints": {
            "discovery": "/",
            "health": "/health",
        },
    })
}

/// Route a request to its response: an HTTP status and a JSON body.
///
/// Health is a real check, not a constant: it asks the store for a document
/// count, so a `fireup local` that has lost its database reports unhealthy
/// instead of cheerfully answering 200 while every RPC fails.
pub async fn route(request: &RequestLine, state: &HttpState) -> (u16, Json) {
    if request.method != "GET" && request.method != "HEAD" {
        return (
            405,
            json!({
                "error": format!("{} is not supported; the HTTP surface is read-only", request.method),
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
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Payload Too Large",
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
    let (status, body, include_body) = match read_head(&mut stream).await {
        Ok(head) => match parse_request_line(&head) {
            Some(request) => {
                debug!(method = %request.method, path = %request.path, "http request");
                let include_body = request.method != "HEAD";
                let (status, body) = route(&request, &state).await;
                (status, body, include_body)
            }
            None => (
                400,
                json!({ "error": "could not parse the HTTP request line" }),
                true,
            ),
        },
        Err(HeadError::TooLarge) => (
            413,
            json!({ "error": format!("request head exceeded {MAX_HEAD_BYTES} bytes") }),
            true,
        ),
        Err(HeadError::Timeout) => (
            408,
            json!({ "error": "timed out waiting for the request" }),
            true,
        ),
        // A connection that broke before sending anything has nobody to answer.
        Err(HeadError::Closed) => return,
    };

    let response = render_response(status, &body, include_body);
    if let Err(error) = stream.write_all(&response).await {
        debug!(%error, "failed to write the HTTP response");
    }
    let _ = stream.flush().await;
}

enum HeadError {
    TooLarge,
    Timeout,
    Closed,
}

/// Read until the end of the request head (`\r\n\r\n`).
async fn read_head(stream: &mut TcpStream) -> Result<String, HeadError> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 1024];

    loop {
        let read = tokio::time::timeout(HEAD_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| HeadError::Timeout)?
            .map_err(|_| HeadError::Closed)?;
        if read == 0 {
            // EOF. If something arrived, try to parse it; a request without the
            // terminator but with a complete first line is still answerable.
            return if head.is_empty() {
                Err(HeadError::Closed)
            } else {
                Ok(String::from_utf8_lossy(&head).into_owned())
            };
        }

        head.extend_from_slice(&chunk[..read]);
        if head.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(String::from_utf8_lossy(&head).into_owned());
        }
        if head.len() > MAX_HEAD_BYTES {
            return Err(HeadError::TooLarge);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::config::{LocalConfig, Service};
    use crate::local::store::{MemoryStore, Store};

    fn state() -> HttpState {
        HttpState {
            config: Arc::new(LocalConfig::in_memory(
                "127.0.0.1:4589".parse().expect("valid address"),
            )),
            store: Arc::new(MemoryStore::new()),
        }
    }

    fn get(path: &str) -> RequestLine {
        RequestLine {
            method: "GET".to_string(),
            path: path.to_string(),
        }
    }

    #[test]
    fn parses_a_normal_request_line() {
        assert_eq!(
            parse_request_line("GET /health HTTP/1.1\r\nHost: x\r\n\r\n"),
            Some(get("/health"))
        );
    }

    #[test]
    fn drops_the_query_string_and_fragment() {
        assert_eq!(
            parse_request_line("GET /health?probe=1 HTTP/1.1\r\n\r\n"),
            Some(get("/health"))
        );
    }

    #[test]
    fn rejects_garbage_instead_of_guessing_a_route() {
        assert_eq!(parse_request_line(""), None);
        assert_eq!(parse_request_line("GET\r\n"), None);
        assert_eq!(parse_request_line("GET /health\r\n"), None, "no version");
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
        assert_eq!(body["projectId"], LocalConfig::DEFAULT_PROJECT_ID);
    }

    #[tokio::test]
    async fn health_reports_the_document_count_from_the_store() {
        let store = MemoryStore::new();
        let state = HttpState {
            config: Arc::new(LocalConfig::in_memory(
                "127.0.0.1:4589".parse().expect("valid address"),
            )),
            store: Arc::new(store.clone()),
        };

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
            &RequestLine {
                method: "POST".to_string(),
                path: "/".to_string(),
            },
            &state,
        )
        .await;
        assert_eq!(status, 405, "the HTTP surface must stay read-only");
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
