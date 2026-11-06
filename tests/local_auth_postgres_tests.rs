//! The local auth surface against a real PostgreSQL account store.
//!
//! These cover what the in-memory tests cannot: that an account created through
//! the Identity Toolkit REST surface is still there after the process that
//! created it has gone away, and that the durable store enforces the same
//! uniqueness rules as the in-memory one — including the ones that are
//! constraints in SQL rather than code (`EMAIL_EXISTS`, `DUPLICATE_LOCAL_ID`).
//!
//! The tests need a reachable database and are skipped when
//! `FIREUP_TEST_POSTGRES_URL` is unset, for example:
//!
//! ```sh
//! FIREUP_TEST_POSTGRES_URL='postgresql://fireup:pw@127.0.0.1:5433/fireup_dev' \
//!   cargo test --test local_auth_postgres_tests
//! ```
//!
//! The URL goes through the shared `postgres_test_env` guard, which refuses the
//! host Paperclip role/database outright and never falls back to a default
//! connection. See `PROJECT_STATUS.md` for the throwaway-Postgres setup.
//!
//! Each test uses its own generated project id, so its accounts are disjoint
//! from every other test's and concurrent runs against one database do not
//! collide. Rows are deleted afterwards.

use std::net::SocketAddr;
use std::sync::Arc;

use fireup::local::auth::postgres_store::{PostgresUserStore, OOB_CODES_TABLE, USERS_TABLE};
use fireup::local::auth::store::{conformance, SharedUserStore};
use fireup::local::{self, LocalConfig};
use serde_json::{json, Value as Json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[path = "postgres_test_env.rs"]
#[allow(dead_code)]
mod postgres_test_env;

const IDENTITY: &str = "/identitytoolkit.googleapis.com/v1";

fn test_url() -> Option<String> {
    let url = postgres_test_env::resolve_integration_postgres_url();
    if url.is_none() {
        postgres_test_env::skip_postgres_integration_test("local PostgreSQL auth store tests");
    }
    url
}

/// A project id unique to this test, so accounts cannot collide.
fn unique_project(prefix: &str) -> String {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}-{}", &suffix[..8])
}

async fn user_store(url: &str) -> SharedUserStore {
    Arc::new(
        PostgresUserStore::connect(url, 4)
            .await
            .expect("connect to the test database and create the auth schema"),
    )
}

/// A plain connection, for cleanup and for reading rows with SQL.
async fn client(url: &str) -> deadpool_postgres::Object {
    let config = fireup::data_importer::parse_connection_url(url, 1, 10).expect("parse test URL");
    let mut pg_config = deadpool_postgres::Config::new();
    pg_config.host = Some(config.host);
    pg_config.port = Some(config.port);
    pg_config.dbname = Some(config.database);
    pg_config.user = Some(config.user.clone());
    if !config.password.is_empty() {
        pg_config.password = Some(config.password);
    }

    pg_config
        .create_pool(Some(deadpool_postgres::Runtime::Tokio1), tokio_postgres::NoTls)
        .expect("create pool")
        .get()
        .await
        .expect("connection")
}

/// Remove every account and pending link belonging to `project_id`, including
/// the conformance suite's sibling project.
async fn cleanup(url: &str, project_id: &str) {
    let client = client(url).await;
    for table in [USERS_TABLE, OOB_CODES_TABLE] {
        client
            .execute(
                &format!("DELETE FROM {table} WHERE project_id LIKE $1"),
                &[&format!("{project_id}%")],
            )
            .await
            .unwrap_or_else(|error| panic!("delete this test's rows from {table}: {error}"));
    }
}

/// Start a server backed by `url`.
async fn start(url: &str, project_id: &str) -> local::RunningServer {
    let config = LocalConfig::in_memory("127.0.0.1:0".parse().expect("valid bind address"))
        .with_postgres(url)
        .with_project_id(project_id);
    local::spawn(config).await.expect("server binds and connects")
}

/// `POST` a JSON body and parse the response.
async fn post(addr: SocketAddr, path: &str, body: Json) -> (u16, Json) {
    let body = body.to_string();
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(
            format!(
                "POST {path} HTTP/1.1\r\n\
                 Host: {addr}\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .expect("write request");
    stream.flush().await.expect("flush");

    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.expect("read response");
    let (head, body) = raw
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("malformed response: {raw:?}"));
    let status: u16 = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status in {head:?}"));

    (
        status,
        serde_json::from_str(body).unwrap_or_else(|error| {
            panic!("response body is not JSON ({error}): {body:?}")
        }),
    )
}

fn error_code(body: &Json) -> &str {
    body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("not an error envelope: {body}"))
}

#[tokio::test]
async fn postgres_user_store_satisfies_the_same_contract_as_the_memory_store() {
    let Some(url) = test_url() else { return };
    let project_id = unique_project("auth-contract");

    let store = user_store(&url).await;
    assert_eq!(store.backend(), "postgres");
    conformance::assert_user_store_contract(store.as_ref(), &project_id).await;

    cleanup(&url, &project_id).await;
}

#[tokio::test]
async fn an_account_and_its_password_survive_a_restart() {
    let Some(url) = test_url() else { return };
    let project_id = unique_project("auth-restart");

    // Sign up through the REST surface, then stop the server entirely.
    let first = start(&url, &project_id).await;
    let (status, signed_up) = post(
        first.addr,
        &format!("{IDENTITY}/accounts:signUp"),
        json!({ "email": "ada@example.com", "password": "secret12", "displayName": "Ada" }),
    )
    .await;
    assert_eq!(status, 200, "{signed_up}");
    let local_id = signed_up["localId"].as_str().expect("localId").to_string();
    first.shutdown().await.expect("clean shutdown");

    // A brand-new process, pointed at the same database, must be able to sign
    // the same account in — which only works if the salt and digest both
    // survived exactly.
    let second = start(&url, &project_id).await;
    let (status, session) = post(
        second.addr,
        &format!("{IDENTITY}/accounts:signInWithPassword"),
        json!({ "email": "ada@example.com", "password": "secret12" }),
    )
    .await;
    assert_eq!(status, 200, "{session}");
    assert_eq!(session["localId"], local_id);
    assert_eq!(session["displayName"], "Ada");

    let (status, body) = post(
        second.addr,
        &format!("{IDENTITY}/accounts:signInWithPassword"),
        json!({ "email": "ada@example.com", "password": "wrong-one" }),
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(
        error_code(&body),
        "INVALID_PASSWORD",
        "a restart must not turn the stored digest into something that matches anything"
    );

    second.shutdown().await.expect("clean shutdown");
    cleanup(&url, &project_id).await;
}

#[tokio::test]
async fn a_refresh_token_still_works_after_a_restart() {
    let Some(url) = test_url() else { return };
    let project_id = unique_project("auth-refresh");

    let first = start(&url, &project_id).await;
    let (status, signed_up) = post(
        first.addr,
        &format!("{IDENTITY}/accounts:signUp"),
        json!({ "email": "ada@example.com", "password": "secret12" }),
    )
    .await;
    assert_eq!(status, 200, "{signed_up}");
    first.shutdown().await.expect("clean shutdown");

    // Refresh tokens are stateless but are checked against the account's
    // `validSince`, so this also proves that column round-tripped.
    let second = start(&url, &project_id).await;
    let (status, refreshed) = post(
        second.addr,
        "/securetoken.googleapis.com/v1/token",
        json!({ "grant_type": "refresh_token", "refresh_token": signed_up["refreshToken"] }),
    )
    .await;

    assert_eq!(status, 200, "{refreshed}");
    assert_eq!(refreshed["user_id"], signed_up["localId"]);

    second.shutdown().await.expect("clean shutdown");
    cleanup(&url, &project_id).await;
}

#[tokio::test]
async fn a_reset_link_still_works_after_a_restart() {
    // The reason pending links are stored rather than kept in a process-local
    // map: a developer recompiles while a reset link sits in their terminal,
    // and the link has to still work.
    let Some(url) = test_url() else { return };
    let project_id = unique_project("auth-oob-restart");

    let first = start(&url, &project_id).await;
    let (status, body) = post(
        first.addr,
        &format!("{IDENTITY}/accounts:signUp"),
        json!({ "email": "ada@example.com", "password": "secret12" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = post(
        first.addr,
        &format!("{IDENTITY}/accounts:sendOobCode"),
        json!({ "requestType": "PASSWORD_RESET", "email": "ada@example.com" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    // Read the stored row directly, so this asserts the code is in the
    // database rather than only in the answering process's memory.
    let client = client(&url).await;
    let row = client
        .query_one(
            &format!(
                "SELECT oob_code, request_type, email, local_id, expires_at_millis
                   FROM {OOB_CODES_TABLE} WHERE project_id = $1"
            ),
            &[&project_id],
        )
        .await
        .expect("the code must be a row, not process state");
    let code: String = row.get("oob_code");
    let request_type: String = row.get("request_type");
    let email: String = row.get("email");
    let local_id: Option<String> = row.get("local_id");
    assert_eq!(request_type, "PASSWORD_RESET");
    assert_eq!(email, "ada@example.com");
    assert!(local_id.is_some(), "a reset code names the account it resets");

    first.shutdown().await.expect("clean shutdown");

    let second = start(&url, &project_id).await;
    let (status, body) = post(
        second.addr,
        &format!("{IDENTITY}/accounts:resetPassword"),
        json!({ "oobCode": code, "newPassword": "brand-new-one" }),
    )
    .await;
    assert_eq!(status, 200, "a link must survive a restart: {body}");

    let (status, body) = post(
        second.addr,
        &format!("{IDENTITY}/accounts:signInWithPassword"),
        json!({ "email": "ada@example.com", "password": "brand-new-one" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    // Redeeming it removed the row, so the single-use rule is the database's
    // too and not just the in-memory store's.
    let remaining: i64 = client
        .query_one(
            &format!("SELECT count(*) FROM {OOB_CODES_TABLE} WHERE project_id = $1"),
            &[&project_id],
        )
        .await
        .expect("count codes")
        .get(0);
    assert_eq!(remaining, 0, "a redeemed code must be gone from the table");

    second.shutdown().await.expect("clean shutdown");
    cleanup(&url, &project_id).await;
}

#[tokio::test]
async fn pending_links_live_in_their_own_table_away_from_accounts_and_documents() {
    let Some(url) = test_url() else { return };
    let project_id = unique_project("auth-oob-table");
    let server = start(&url, &project_id).await;

    let (status, body) = post(
        server.addr,
        &format!("{IDENTITY}/accounts:sendOobCode"),
        json!({ "requestType": "EMAIL_SIGNIN", "email": "link@example.com" }),
    )
    .await;
    assert_eq!(
        status, 200,
        "a sign-in link needs no account, so no FK may block it: {body}"
    );

    let client = client(&url).await;
    // No account was created, so a sign-in code has to be storable with a null
    // `local_id`.
    let accounts: i64 = client
        .query_one(
            &format!("SELECT count(*) FROM {USERS_TABLE} WHERE project_id = $1"),
            &[&project_id],
        )
        .await
        .expect("count accounts")
        .get(0);
    assert_eq!(accounts, 0, "requesting a link must not create an account");

    let row = client
        .query_one(
            &format!(
                "SELECT local_id, new_email, continue_url FROM {OOB_CODES_TABLE}
                  WHERE project_id = $1"
            ),
            &[&project_id],
        )
        .await
        .expect("the code row");
    assert_eq!(row.get::<_, Option<String>>("local_id"), None);
    assert_eq!(row.get::<_, Option<String>>("new_email"), None);
    assert_eq!(row.get::<_, Option<String>>("continue_url"), None);

    // And following it creates the account, in the accounts table.
    let code: String = client
        .query_one(
            &format!("SELECT oob_code FROM {OOB_CODES_TABLE} WHERE project_id = $1"),
            &[&project_id],
        )
        .await
        .expect("the code")
        .get(0);
    let (status, body) = post(
        server.addr,
        &format!("{IDENTITY}/accounts:signInWithEmailLink"),
        json!({ "email": "link@example.com", "oobCode": code }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["isNewUser"], true);

    let accounts: i64 = client
        .query_one(
            &format!("SELECT count(*) FROM {USERS_TABLE} WHERE project_id = $1"),
            &[&project_id],
        )
        .await
        .expect("count accounts")
        .get(0);
    assert_eq!(accounts, 1);
    assert_eq!(
        server.store.count().await.expect("count documents"),
        0,
        "identity data must not land in the documents table"
    );

    server.shutdown().await.expect("clean shutdown");
    cleanup(&url, &project_id).await;
}

#[tokio::test]
async fn uniqueness_is_enforced_by_the_database_not_just_by_a_pre_read() {
    let Some(url) = test_url() else { return };
    let project_id = unique_project("auth-unique");
    let server = start(&url, &project_id).await;
    let admin = format!("/v1/projects/{project_id}");

    let (status, created) = post(
        server.addr,
        &format!("{admin}/accounts"),
        json!({ "localId": "taken", "email": "taken@example.com", "password": "secret12" }),
    )
    .await;
    assert_eq!(status, 200, "{created}");

    let (status, body) = post(
        server.addr,
        &format!("{admin}/accounts"),
        json!({ "localId": "other", "email": "taken@example.com" }),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(error_code(&body), "EMAIL_EXISTS");

    let (status, body) = post(
        server.addr,
        &format!("{admin}/accounts"),
        json!({ "localId": "taken", "email": "fresh@example.com" }),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(error_code(&body), "DUPLICATE_LOCAL_ID");

    // Exactly one row survived both attempts.
    let rows = client(&url)
        .await
        .query(
            &format!("SELECT local_id FROM {USERS_TABLE} WHERE project_id = $1"),
            &[&project_id],
        )
        .await
        .expect("read accounts");
    assert_eq!(rows.len(), 1, "a refused create must not have written a row");

    server.shutdown().await.expect("clean shutdown");
    cleanup(&url, &project_id).await;
}

#[tokio::test]
async fn many_anonymous_accounts_coexist_under_the_partial_email_index() {
    // Every anonymous account has a null email. A non-partial unique index would
    // break on the second sign-in, and only a real database can show that.
    let Some(url) = test_url() else { return };
    let project_id = unique_project("auth-anon");
    let server = start(&url, &project_id).await;

    for attempt in 0..5 {
        let (status, body) =
            post(server.addr, &format!("{IDENTITY}/accounts:signUp"), json!({})).await;
        assert_eq!(status, 200, "anonymous sign-up {attempt} failed: {body}");
    }

    let count: i64 = client(&url)
        .await
        .query_one(
            &format!("SELECT count(*) FROM {USERS_TABLE} WHERE project_id = $1"),
            &[&project_id],
        )
        .await
        .expect("count accounts")
        .get(0);
    assert_eq!(count, 5);

    server.shutdown().await.expect("clean shutdown");
    cleanup(&url, &project_id).await;
}

#[tokio::test]
async fn accounts_are_stored_in_their_own_table_away_from_documents() {
    let Some(url) = test_url() else { return };
    let project_id = unique_project("auth-separate");
    let server = start(&url, &project_id).await;

    let (status, body) = post(
        server.addr,
        &format!("{IDENTITY}/accounts:signUp"),
        json!({
            "email": "ada@example.com",
            "password": "secret12",
        }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let local_id = body["localId"].as_str().expect("localId").to_string();

    let client = client(&url).await;

    // The account is a row in the auth table…
    let row = client
        .query_one(
            &format!(
                "SELECT email, email_verified, provider, password_salt, password_hash,
                        disabled, custom_claims, valid_since_millis
                   FROM {USERS_TABLE} WHERE project_id = $1 AND local_id = $2"
            ),
            &[&project_id, &local_id],
        )
        .await
        .expect("the account row exists");
    assert_eq!(row.get::<_, Option<String>>("email").as_deref(), Some("ada@example.com"));
    assert_eq!(row.get::<_, bool>("email_verified"), false);
    assert_eq!(row.get::<_, String>("provider"), "password");
    assert_eq!(row.get::<_, bool>("disabled"), false);
    assert!(
        row.get::<_, Option<Json>>("custom_claims").is_none(),
        "an account without claims must store NULL, not an empty object"
    );
    assert!(row.get::<_, Option<String>>("password_salt").is_some());
    let hash = row
        .get::<_, Option<String>>("password_hash")
        .expect("a stored digest");
    assert_ne!(
        hash, "secret12",
        "the password must never be at rest in plaintext"
    );

    // …and nothing was written to the documents table.
    let documents: i64 = client
        .query_one(
            &format!(
                "SELECT count(*) FROM {} WHERE project_id = $1",
                fireup::local::postgres_store::DOCUMENTS_TABLE
            ),
            &[&project_id],
        )
        .await
        .expect("count documents")
        .get(0);
    assert_eq!(
        documents, 0,
        "identity data must not be reachable through the Firestore API"
    );
    assert_eq!(server.store.count().await.expect("count"), 0);

    server.shutdown().await.expect("clean shutdown");
    cleanup(&url, &project_id).await;
}

#[tokio::test]
async fn an_unreachable_database_fails_startup_instead_of_serving_empty_accounts() {
    // No test database needed: the point is that a bad URL is a hard error, not
    // a server that answers EMAIL_NOT_FOUND for every real account.
    let config = LocalConfig::in_memory("127.0.0.1:0".parse().expect("valid address"))
        .with_postgres("postgresql://fireup:nope@127.0.0.1:1/fireup_nonexistent");

    let rendered = match local::spawn(config).await {
        Ok(_) => panic!("an unreachable store must not start a server"),
        Err(error) => format!("{error:#}"),
    };
    assert!(
        rendered.contains("PostgreSQL"),
        "the error should name the storage backend: {rendered}"
    );
}
