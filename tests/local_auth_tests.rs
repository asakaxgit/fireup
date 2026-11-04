//! `fireup local` Firebase Auth / Identity Platform surface tests.
//!
//! These drive a real server on a loopback port over real HTTP/1.1, addressing
//! the endpoints the way an official SDK does when
//! `FIREBASE_AUTH_EMULATOR_HOST` is set — the API host as the first path
//! segment (`/identitytoolkit.googleapis.com/v1/accounts:signUp`). The unit
//! tests in `src/local/auth/api.rs` cover the handler logic; this file proves
//! the wire works: bodies are read, both protocols share one port, and the
//! responses are the JSON a client parses.

use std::net::SocketAddr;

use fireup::local;
use fireup::protos::google::firestore::v1::{
    firestore_client::FirestoreClient, CreateDocumentRequest, Document,
};
use serde_json::{json, Value as Json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// The prefix an SDK prepends when it is pointed at a local host.
const IDENTITY: &str = "/identitytoolkit.googleapis.com/v1";
const SECURE_TOKEN: &str = "/securetoken.googleapis.com/v1";

const PROJECT: &str = "fireup-auth-test";

/// Start a server on an OS-assigned port with both services enabled.
async fn start() -> local::RunningServer {
    let config = local::LocalConfig::in_memory("127.0.0.1:0".parse().expect("valid bind address"))
        .with_project_id(PROJECT);
    local::spawn(config).await.expect("server binds")
}

/// An HTTP response, split into the parts these tests assert on.
struct Response {
    status: u16,
    body: Json,
}

impl Response {
    /// The Identity Toolkit error code, for a response that is an error.
    fn code(&self) -> &str {
        self.body["error"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("not an error envelope: {}", self.body))
    }
}

/// Send one request and read the whole response.
///
/// Deliberately hand-rolled rather than using an HTTP client crate: the point is
/// to exercise Fireup's own request parsing, including `Content-Length` handling,
/// against bytes a test controls.
async fn send(addr: SocketAddr, method: &str, path: &str, body: &str) -> Response {
    send_with_headers(addr, method, path, body, "Content-Type: application/json\r\n").await
}

async fn send_with_headers(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    extra_headers: &str,
) -> Response {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let request = format!(
        "{method} {path} HTTP/1.1\r\n\
         Host: {addr}\r\n\
         {extra_headers}\
         Content-Length: {length}\r\n\
         \r\n\
         {body}",
        length = body.len(),
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request");
    stream.flush().await.expect("flush");

    let mut raw = String::new();
    stream
        .read_to_string(&mut raw)
        .await
        .expect("read response");

    let (head, body) = raw
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("malformed response: {raw:?}"));
    let status: u16 = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status code in {head:?}"));

    Response {
        status,
        body: serde_json::from_str(body)
            .unwrap_or_else(|error| panic!("body is not JSON ({error}): {body:?}")),
    }
}

async fn post(addr: SocketAddr, path: &str, body: Json) -> Response {
    send(addr, "POST", path, &body.to_string()).await
}

/// Sign a new account up and return the parsed response.
async fn sign_up(addr: SocketAddr, email: &str, password: &str) -> Json {
    let response = post(
        addr,
        &format!("{IDENTITY}/accounts:signUp"),
        json!({ "email": email, "password": password, "returnSecureToken": true }),
    )
    .await;
    assert_eq!(response.status, 200, "sign-up failed: {}", response.body);
    response.body
}

/// Decode an unsigned JWT's claims, the way a client SDK in emulator mode does.
fn claims_of(id_token: &str) -> Json {
    let segments: Vec<&str> = id_token.split('.').collect();
    assert_eq!(segments.len(), 3, "a JWS has three segments: {id_token}");
    assert_eq!(segments[2], "", "local tokens are unsigned");

    // base64url, unpadded.
    let padded = match segments[1].len() % 4 {
        0 => segments[1].to_string(),
        remainder => format!("{}{}", segments[1], "=".repeat(4 - remainder)),
    };
    let decoded = padded
        .chars()
        .map(|character| match character {
            '-' => '+',
            '_' => '/',
            other => other,
        })
        .collect::<String>();
    let bytes = base64_decode(&decoded);
    serde_json::from_slice(&bytes).expect("claims are JSON")
}

/// Minimal standard-alphabet base64 decoder, so the test does not depend on the
/// same crate the implementation uses to encode.
fn base64_decode(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut accumulator: u32 = 0;
    let mut bits = 0;
    let mut out = Vec::new();

    for byte in text.bytes() {
        if byte == b'=' {
            break;
        }
        let value = ALPHABET
            .iter()
            .position(|candidate| *candidate == byte)
            .unwrap_or_else(|| panic!("{:?} is not base64", byte as char)) as u32;
        accumulator = (accumulator << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
        }
    }
    out
}

#[tokio::test]
async fn an_sdk_can_sign_up_sign_in_and_read_itself_over_one_port() {
    let server = start().await;
    let addr = server.addr;

    let signed_up = sign_up(addr, "ada@example.com", "secret12").await;
    let local_id = signed_up["localId"].as_str().expect("localId").to_string();
    assert_eq!(signed_up["email"], "ada@example.com");
    assert_eq!(signed_up["expiresIn"], "3600");

    // The ID token must carry the published claim set for this project.
    let claims = claims_of(signed_up["idToken"].as_str().expect("idToken"));
    assert_eq!(
        claims["iss"],
        format!("https://securetoken.google.com/{PROJECT}")
    );
    assert_eq!(claims["aud"], PROJECT);
    assert_eq!(claims["sub"], local_id);
    assert_eq!(claims["email"], "ada@example.com");
    assert_eq!(claims["firebase"]["sign_in_provider"], "password");

    // Sign in with the same credentials.
    let response = post(
        addr,
        &format!("{IDENTITY}/accounts:signInWithPassword"),
        json!({ "email": "ada@example.com", "password": "secret12", "returnSecureToken": true }),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(response.body["localId"], local_id);
    assert_eq!(response.body["registered"], true);

    // Read the account back with the token just issued.
    let response = post(
        addr,
        &format!("{IDENTITY}/accounts:lookup"),
        json!({ "idToken": response.body["idToken"] }),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(response.body["kind"], "identitytoolkit#GetAccountInfoResponse");
    assert_eq!(response.body["users"][0]["localId"], local_id);
    assert_eq!(response.body["users"][0]["email"], "ada@example.com");

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn the_bare_path_works_for_curl_as_well_as_the_sdk_prefixed_one() {
    let server = start().await;

    // An SDK sends the prefixed form; a developer with curl sends the bare one.
    // Both must reach the same handler, or the documented curl examples lie.
    for path in ["/v1/accounts:signUp", &format!("{IDENTITY}/accounts:signUp")] {
        let response = post(server.addr, path, json!({})).await;
        assert_eq!(response.status, 200, "{path}: {}", response.body);
        assert!(
            response.body["localId"].is_string(),
            "{path}: {}",
            response.body
        );
    }

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_refresh_token_exchanges_for_a_new_id_token_over_the_wire() {
    let server = start().await;
    let signed_up = sign_up(server.addr, "ada@example.com", "secret12").await;
    let refresh = signed_up["refreshToken"].as_str().expect("refreshToken");

    // The JS SDK form-encodes this request, so that is what the test sends.
    let form = format!("grant_type=refresh_token&refresh_token={refresh}");
    let response = send_with_headers(
        server.addr,
        "POST",
        &format!("{SECURE_TOKEN}/token"),
        &form,
        "Content-Type: application/x-www-form-urlencoded\r\n",
    )
    .await;

    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(response.body["token_type"], "Bearer");
    assert_eq!(response.body["user_id"], signed_up["localId"]);
    assert_eq!(response.body["project_id"], PROJECT);

    let claims = claims_of(response.body["id_token"].as_str().expect("id_token"));
    assert_eq!(claims["sub"], signed_up["localId"]);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn admin_account_crud_works_end_to_end() {
    let server = start().await;
    let addr = server.addr;
    let admin = format!("/v1/projects/{PROJECT}");

    // Create.
    let response = post(
        addr,
        &format!("{admin}/accounts"),
        json!({
            "localId": "chosen",
            "email": "made@example.com",
            "password": "secret12",
            "displayName": "Made",
            "emailVerified": true,
            "customAttributes": "{\"role\":\"editor\"}",
        }),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(response.body["localId"], "chosen");

    // Read.
    let response = post(
        addr,
        &format!("{admin}/accounts:lookup"),
        json!({ "localId": ["chosen"] }),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    let user = &response.body["users"][0];
    assert_eq!(user["email"], "made@example.com");
    assert_eq!(user["emailVerified"], true);
    assert_eq!(user["displayName"], "Made");
    assert_eq!(
        serde_json::from_str::<Json>(
            user["customAttributes"].as_str().expect("a JSON string")
        )
        .expect("claims parse"),
        json!({ "role": "editor" }),
        "custom claims travel as a JSON string: {}",
        response.body
    );

    // The claims must reach a real session.
    let response = post(
        addr,
        &format!("{IDENTITY}/accounts:signInWithPassword"),
        json!({ "email": "made@example.com", "password": "secret12" }),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    let claims = claims_of(response.body["idToken"].as_str().expect("idToken"));
    assert_eq!(claims["role"], "editor");
    assert_eq!(claims["email_verified"], true);

    // Update.
    let response = post(
        addr,
        &format!("{admin}/accounts:update"),
        json!({ "localId": "chosen", "displayName": "Renamed", "disableUser": true }),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(response.body["displayName"], "Renamed");

    // A disabled account cannot sign in.
    let response = post(
        addr,
        &format!("{IDENTITY}/accounts:signInWithPassword"),
        json!({ "email": "made@example.com", "password": "secret12" }),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(response.code(), "USER_DISABLED");

    // Delete.
    let response = post(
        addr,
        &format!("{admin}/accounts:delete"),
        json!({ "localId": "chosen" }),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);

    let response = post(
        addr,
        &format!("{admin}/accounts:lookup"),
        json!({ "localId": ["chosen"] }),
    )
    .await;
    assert!(
        response.body.get("users").is_none(),
        "the account must be gone: {}",
        response.body
    );

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn listing_accounts_pages_through_the_query_string() {
    let server = start().await;
    let admin = format!("/v1/projects/{PROJECT}");

    for uid in ["a", "b", "c"] {
        let response = post(
            server.addr,
            &format!("{admin}/accounts"),
            json!({ "localId": uid, "email": format!("{uid}@example.com") }),
        )
        .await;
        assert_eq!(response.status, 200, "{}", response.body);
    }

    let response = send(
        server.addr,
        "GET",
        &format!("{admin}/accounts:batchGet?maxResults=2"),
        "",
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(response.body["kind"], "identitytoolkit#DownloadAccountResponse");
    let page: Vec<&str> = response.body["users"]
        .as_array()
        .expect("users")
        .iter()
        .map(|user| user["localId"].as_str().expect("localId"))
        .collect();
    assert_eq!(page, vec!["a", "b"]);
    assert_eq!(response.body["nextPageToken"], "b");

    let response = send(
        server.addr,
        "GET",
        &format!("{admin}/accounts:batchGet?nextPageToken=b"),
        "",
    )
    .await;
    let page: Vec<&str> = response.body["users"]
        .as_array()
        .expect("users")
        .iter()
        .map(|user| user["localId"].as_str().expect("localId"))
        .collect();
    assert_eq!(page, vec!["c"]);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn the_emulator_route_clears_accounts_without_touching_documents() {
    let server = start().await;
    let addr = server.addr;

    sign_up(addr, "ada@example.com", "secret12").await;

    // Put a document in the Firestore store too, so the clear can be shown not
    // to take it with it.
    let mut firestore = FirestoreClient::connect(format!("http://{addr}"))
        .await
        .expect("gRPC client connects");
    firestore
        .create_document(CreateDocumentRequest {
            parent: format!("projects/{PROJECT}/databases/(default)/documents"),
            collection_id: "notes".to_string(),
            document_id: "n1".to_string(),
            document: Some(Document::default()),
            ..Default::default()
        })
        .await
        .expect("create a document");
    assert_eq!(server.store.count().await.expect("count"), 1);

    let response = send(
        addr,
        "DELETE",
        &format!("/emulator/v1/projects/{PROJECT}/accounts"),
        "",
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(response.body["removedAccounts"], 1);

    assert_eq!(
        server.users.count(PROJECT).await.expect("count accounts"),
        0
    );
    assert_eq!(
        server.store.count().await.expect("count documents"),
        1,
        "clearing accounts must not touch application data"
    );

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn accounts_are_invisible_to_the_firestore_api() {
    // Identity data must not be reachable as application data. The two stores
    // are separate types, and this is the test that says so from the outside.
    let server = start().await;
    sign_up(server.addr, "ada@example.com", "secret12").await;

    assert_eq!(
        server.store.count().await.expect("count documents"),
        0,
        "an account must not create a document"
    );
    let (status, body) = {
        let response = send(server.addr, "GET", "/health", "").await;
        (response.status, response.body)
    };
    assert_eq!(status, 200);
    assert_eq!(
        body["storage"]["documents"], 0,
        "health counts documents, not accounts: {body}"
    );

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn discovery_tells_a_developer_how_to_point_an_sdk_at_the_auth_surface() {
    let server = start().await;
    let response = send(server.addr, "GET", "/", "").await;

    assert_eq!(response.status, 200, "{}", response.body);
    let auth = response.body["services"]
        .as_array()
        .expect("services")
        .iter()
        .find(|service| service["id"] == "auth")
        .unwrap_or_else(|| panic!("auth must be listed: {}", response.body));

    assert_eq!(auth["api"], "identitytoolkit.googleapis.com");
    assert_eq!(auth["protocol"], "rest");
    assert_eq!(auth["clientEnvVar"], "FIREBASE_AUTH_EMULATOR_HOST");
    assert_eq!(
        response.body["endpoints"]["identityToolkit"],
        "/identitytoolkit.googleapis.com/v1/"
    );
    assert_eq!(response.body["projectId"], PROJECT);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn one_port_still_serves_grpc_after_auth_requests() {
    let server = start().await;
    let addr = server.addr;

    sign_up(addr, "ada@example.com", "secret12").await;

    let mut firestore = FirestoreClient::connect(format!("http://{addr}"))
        .await
        .expect("gRPC client connects");
    firestore
        .create_document(CreateDocumentRequest {
            parent: format!("projects/{PROJECT}/databases/(default)/documents"),
            collection_id: "notes".to_string(),
            document_id: "n1".to_string(),
            document: Some(Document::default()),
            ..Default::default()
        })
        .await
        .expect("gRPC works after HTTP auth traffic");

    // And auth still works after gRPC.
    let response = post(
        addr,
        &format!("{IDENTITY}/accounts:signInWithPassword"),
        json!({ "email": "ada@example.com", "password": "secret12" }),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn auth_is_unreachable_when_the_service_is_not_enabled() {
    let config = local::LocalConfig::in_memory("127.0.0.1:0".parse().expect("valid bind address"))
        .with_project_id(PROJECT)
        .with_only(local::Service::Firestore);
    let server = local::spawn(config).await.expect("server binds");

    let response = post(
        server.addr,
        &format!("{IDENTITY}/accounts:signUp"),
        json!({}),
    )
    .await;
    assert_eq!(response.status, 404);
    assert!(
        response.code().contains("--enable auth"),
        "the error must say how to turn it on: {}",
        response.body
    );

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn error_responses_carry_the_codes_sdks_match_on() {
    let server = start().await;
    let addr = server.addr;
    sign_up(addr, "taken@example.com", "secret12").await;

    for (path, body, expected) in [
        (
            "accounts:signUp",
            json!({ "email": "taken@example.com", "password": "secret12" }),
            "EMAIL_EXISTS",
        ),
        (
            "accounts:signInWithPassword",
            json!({ "email": "taken@example.com", "password": "wrong-one" }),
            "INVALID_PASSWORD",
        ),
        (
            "accounts:signInWithPassword",
            json!({ "email": "nobody@example.com", "password": "secret12" }),
            "EMAIL_NOT_FOUND",
        ),
        ("accounts:lookup", json!({ "idToken": "nonsense" }), "INVALID_ID_TOKEN"),
    ] {
        let response = post(addr, &format!("{IDENTITY}/{path}"), body.clone()).await;
        assert_eq!(response.status, 400, "{path} {body}: {}", response.body);
        assert_eq!(response.code(), expected, "for {path} {body}");
        // The envelope shape matters as much as the code: SDKs read
        // `error.message`, and some read `error.errors[0].reason`.
        assert_eq!(response.body["error"]["code"], 400);
        assert_eq!(response.body["error"]["errors"][0]["message"], expected);
    }

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_request_that_lies_about_its_body_length_is_refused() {
    // The body reader must not hang or answer half a request.
    let server = start().await;

    let mut stream = TcpStream::connect(server.addr).await.expect("connect");
    let request = format!(
        "POST {IDENTITY}/accounts:signUp HTTP/1.1\r\n\
         Host: x\r\n\
         Content-Length: 500\r\n\
         \r\n\
         {{}}"
    );
    stream.write_all(request.as_bytes()).await.expect("write");
    // Close the write half, so the announced body can never arrive.
    stream.shutdown().await.expect("shutdown write half");

    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.expect("read");
    assert!(
        raw.starts_with("HTTP/1.1 400"),
        "a truncated body must be a clear 400: {raw:?}"
    );

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_chunked_body_is_refused_rather_than_misread() {
    let server = start().await;

    let mut stream = TcpStream::connect(server.addr).await.expect("connect");
    let request = format!(
        "POST {IDENTITY}/accounts:signUp HTTP/1.1\r\n\
         Host: x\r\n\
         Transfer-Encoding: chunked\r\n\
         \r\n\
         2\r\n{{}}\r\n0\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.expect("write");
    stream.flush().await.expect("flush");

    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.expect("read");
    assert!(raw.starts_with("HTTP/1.1 400"), "{raw:?}");
    assert!(raw.contains("Content-Length"), "the error must say what to send: {raw:?}");

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_body_split_across_packets_is_read_whole() {
    // A client that writes the head and the body separately is normal; the
    // reader must wait for the rest rather than parsing a truncated body.
    let server = start().await;

    let body = json!({ "email": "split@example.com", "password": "secret12" }).to_string();
    let mut stream = TcpStream::connect(server.addr).await.expect("connect");
    stream
        .write_all(
            format!(
                "POST {IDENTITY}/accounts:signUp HTTP/1.1\r\n\
                 Host: x\r\n\
                 Content-Length: {}\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .expect("write head");
    stream.flush().await.expect("flush head");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    stream.write_all(body.as_bytes()).await.expect("write body");
    stream.flush().await.expect("flush body");

    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.expect("read");
    assert!(raw.starts_with("HTTP/1.1 200"), "{raw:?}");
    assert!(raw.contains("split@example.com"), "{raw:?}");

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_head_request_to_discovery_still_sends_no_body() {
    // The body reader must not change the existing HEAD behaviour.
    let server = start().await;

    let mut stream = TcpStream::connect(server.addr).await.expect("connect");
    stream
        .write_all(b"HEAD /health HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .expect("write");
    stream.flush().await.expect("flush");

    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.expect("read");
    assert!(raw.starts_with("HTTP/1.1 200"), "{raw:?}");
    assert!(raw.ends_with("\r\n\r\n"), "HEAD must send no body: {raw:?}");

    server.shutdown().await.expect("clean shutdown");
}
