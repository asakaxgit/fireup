//! `fireup local` — a local Firestore-, Datastore-, and Auth-compatible data plane.
//!
//! Fireup serves the **published** `google.firestore.v1` and
//! `google.datastore.v1` gRPC contracts plus a subset of the **published**
//! Identity Toolkit REST contract on a single port, so official client SDKs can
//! point at it with the documented `FIRESTORE_EMULATOR_HOST`,
//! `DATASTORE_EMULATOR_HOST`, and `FIREBASE_AUTH_EMULATOR_HOST` environment
//! variables. The same port answers HTTP/1.1 `GET /` (service discovery) and
//! `GET /health`.
//!
//! This is an independent implementation of public APIs. It is not the Firebase
//! Emulator Suite, is not affiliated with Google, and contains no code or
//! behaviour derived from proprietary Google binaries. See
//! `.cursor/rules/firebase-gcp-compliance.mdc` and
//! `docs/licenses/FIREBASE_GCP_LOCAL.md`.
//!
//! Current scope:
//!
//! * **Firestore** — document CRUD, `StructuredQuery` reads, collection
//!   listings, and transactional writes against an in-memory **or PostgreSQL**
//!   store, plus seeding from a managed export. Writes carry the
//!   [`serverTimestamp`/`increment`/`arrayUnion`/`arrayRemove` field
//!   transforms](transforms). Realtime listeners and aggregation queries are
//!   not served; those RPCs answer `UNIMPLEMENTED` with a message saying so,
//!   and `docs/local/README.md` has the full fidelity matrix.
//! * **Datastore** — Lookup/RunQuery/Commit/BeginTransaction/AllocateIds on the
//!   same store Firestore uses (kinds map to collections). `Commit` serves
//!   `property_transforms`, resolved against the fields the named properties map
//!   to. GQL, aggregations, namespaces, and `property_mask` on a mutation answer
//!   `UNIMPLEMENTED`.
//! * **Auth** — email/password and anonymous sign-in, emulator-style ID tokens,
//!   token refresh, and admin account CRUD. See [`auth`] for what is and is not
//!   served.

pub mod auth;
pub mod config;
pub mod datastore;
mod datastore_filter;
pub mod entity_map;
pub mod firestore;
pub mod http;
pub mod names;
mod postgres_filter;
pub mod postgres_store;
pub mod query;
pub mod seed;
pub mod store;
pub mod transactions;
pub mod transforms;
pub mod value_json;

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

pub use auth::{AuthApi, MemoryUserStore, PostgresUserStore, SharedUserStore, UserStore};
pub use config::{LocalConfig, Service, DEFAULT_PORT};
pub use datastore::DatastoreApi;
pub use firestore::FirestoreApi;
pub use postgres_store::PostgresStore;
pub use store::{MemoryStore, SharedStore, Store};

use crate::protos::google::datastore::v1::datastore_server::DatastoreServer;
use crate::protos::google::firestore::v1::firestore_server::FirestoreServer;

/// First bytes of the HTTP/2 connection preface (RFC 9113 §3.4). gRPC clients
/// send it immediately, which is what lets one port carry both protocols.
const HTTP2_PREFACE_PREFIX: &[u8] = b"PRI ";

/// How long a new connection has to reveal which protocol it speaks.
const PROTOCOL_PEEK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// A running local server.
pub struct RunningServer {
    /// Address actually bound. With port 0 this is the OS-assigned port.
    pub addr: std::net::SocketAddr,
    /// Shared store, so callers (and tests) can inspect state directly.
    pub store: SharedStore,
    /// Shared account store, for the same reason. Present even when the auth
    /// service is disabled — it is just unreachable over HTTP then.
    pub users: SharedUserStore,
    /// The configuration this server was started with.
    pub config: Arc<LocalConfig>,
    grpc: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
    acceptor: tokio::task::JoinHandle<()>,
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl RunningServer {
    /// Stop the server and wait for it to finish.
    pub async fn shutdown(self) -> Result<()> {
        // A send error means every receiver is already gone, i.e. the server
        // stopped on its own; nothing left to signal.
        let _ = self.shutdown.send(true);

        self.acceptor.await.context("local acceptor task panicked")?;
        self.grpc
            .await
            .context("local gRPC task panicked")?
            .context("local server returned an error during shutdown")
    }
}

/// Build the store this configuration asks for.
///
/// `--postgres-url` is the only switch: present means durable, absent means
/// in-memory. There is no automatic fallback — a `--postgres-url` that cannot
/// be reached is a hard error, because silently serving an empty in-memory
/// store instead would look like data loss.
pub async fn build_store(config: &LocalConfig) -> Result<SharedStore> {
    match &config.postgres_url {
        Some(url) => {
            let store = PostgresStore::connect(url, config.max_connections)
                .await
                .context(
                    "failed to open the PostgreSQL document store; \
                     fix --postgres-url or omit it to run in memory",
                )?;
            Ok(Arc::new(store))
        }
        None => Ok(Arc::new(MemoryStore::new())),
    }
}

/// Build the account store this configuration asks for.
///
/// Follows `--postgres-url` for the same reason the document store does: a
/// developer who asked for durable documents did not ask for accounts that
/// vanish on restart. Built even when the auth service is disabled, so enabling
/// it later finds the accounts that were already there.
pub async fn build_user_store(config: &LocalConfig) -> Result<SharedUserStore> {
    match &config.postgres_url {
        Some(url) => PostgresUserStore::connect_shared(url, config.max_connections)
            .await
            .context(
                "failed to open the PostgreSQL account store; \
                 fix --postgres-url or omit it to run in memory",
            ),
        None => Ok(Arc::new(MemoryUserStore::new())),
    }
}

/// Bind and start serving in background tasks, using `config`'s stores.
///
/// Binding happens before returning so a caller (or a test) knows the port is
/// live and can connect without polling.
pub async fn spawn(config: LocalConfig) -> Result<RunningServer> {
    config.validate()?;
    let store = build_store(&config).await?;
    let users = build_user_store(&config).await?;
    spawn_with_stores(config, store, users).await
}

/// Same as [`spawn`], but with a document store the caller already built.
///
/// Accounts go to a fresh in-memory store; use [`spawn_with_stores`] to supply
/// both.
pub async fn spawn_with_store(config: LocalConfig, store: SharedStore) -> Result<RunningServer> {
    spawn_with_stores(config, store, Arc::new(MemoryUserStore::new())).await
}

/// Same as [`spawn`], but with both stores the caller already built. Used by
/// tests and by anything that wants to inspect the state it handed over.
pub async fn spawn_with_stores(
    config: LocalConfig,
    store: SharedStore,
    users: SharedUserStore,
) -> Result<RunningServer> {
    config.validate()?;

    let listener = TcpListener::bind(config.addr)
        .await
        .with_context(|| format!("failed to bind {}", config.addr))?;
    let addr = listener.local_addr().context("failed to read bound address")?;

    let config = Arc::new(LocalConfig { addr, ..config });
    let (shutdown, shutdown_rx) = tokio::sync::watch::channel(false);

    // gRPC connections are handed to tonic through this channel; HTTP/1.1
    // connections are answered by the acceptor itself.
    let (grpc_tx, grpc_rx) =
        tokio::sync::mpsc::channel::<Result<TcpStream, std::io::Error>>(32);

    let grpc = {
        let firestore_api = FirestoreApi::new(store.clone());
        let datastore_api = DatastoreApi::new(
            store.clone(),
            config.project_id.clone(),
            config.database_id.clone(),
        );
        let mut shutdown_rx = shutdown_rx.clone();
        let serve_firestore = config.enables(Service::Firestore);
        let serve_datastore = config.enables(Service::Datastore);

        tokio::spawn(async move {
            let mut builder = tonic::transport::Server::builder();
            // A disabled service is not routed at all, so a client calling it
            // gets `UNIMPLEMENTED` from tonic rather than a served response.
            let router = if serve_firestore {
                builder.add_service(FirestoreServer::new(firestore_api))
            } else {
                builder.add_optional_service(None::<FirestoreServer<FirestoreApi>>)
            };
            let router = if serve_datastore {
                router.add_service(DatastoreServer::new(datastore_api))
            } else {
                router.add_optional_service(None::<DatastoreServer<DatastoreApi>>)
            };

            router
                .serve_with_incoming_shutdown(
                    tokio_stream::wrappers::ReceiverStream::new(grpc_rx),
                    async move {
                        let _ = shutdown_rx.wait_for(|stopped| *stopped).await;
                    },
                )
                .await
        })
    };

    let acceptor = {
        let state = Arc::new(http::HttpState {
            config: config.clone(),
            store: store.clone(),
            // A disabled service has no handler at all, so it cannot be reached
            // by a path that slipped past the routing table.
            auth: config.enables(Service::Auth).then(|| {
                Arc::new(AuthApi::new(config.clone(), users.clone()))
            }),
        });
        let mut shutdown_rx = shutdown_rx.clone();

        tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    accepted = listener.accept() => accepted,
                    _ = shutdown_rx.wait_for(|stopped| *stopped) => break,
                };

                match accepted {
                    Ok((stream, peer)) => {
                        let grpc_tx = grpc_tx.clone();
                        let state = state.clone();
                        tokio::spawn(async move {
                            dispatch_connection(stream, peer, grpc_tx, state).await;
                        });
                    }
                    Err(error) => {
                        // Per-connection accept errors (fd limits, a client
                        // that vanished) must not take the listener down.
                        warn!(%error, "failed to accept a connection");
                    }
                }
            }
            debug!("local acceptor stopped");
        })
    };

    Ok(RunningServer {
        addr,
        store,
        users,
        config,
        grpc,
        acceptor,
        shutdown,
    })
}

/// Decide whether a connection is gRPC (HTTP/2) or plain HTTP/1.1, and route it.
async fn dispatch_connection(
    stream: TcpStream,
    peer: std::net::SocketAddr,
    grpc_tx: tokio::sync::mpsc::Sender<Result<TcpStream, std::io::Error>>,
    state: Arc<http::HttpState>,
) {
    match peek_is_http2(&stream).await {
        Some(true) => {
            debug!(%peer, "routing connection to gRPC");
            // Peeking does not consume, so tonic still sees the full preface.
            if grpc_tx.send(Ok(stream)).await.is_err() {
                debug!(%peer, "gRPC server is gone; dropping connection");
            }
        }
        Some(false) => {
            debug!(%peer, "routing connection to the HTTP surface");
            http::serve_connection(stream, state).await;
        }
        None => debug!(%peer, "connection closed before sending anything"),
    }
}

/// Peek the first bytes to classify the protocol.
///
/// `Some(true)` for HTTP/2, `Some(false)` for anything else (treated as
/// HTTP/1.1, which answers with a parse error if it is not), `None` if the
/// connection never sent enough to tell.
async fn peek_is_http2(stream: &TcpStream) -> Option<bool> {
    let mut preface = [0u8; 4];
    let deadline = tokio::time::Instant::now() + PROTOCOL_PEEK_TIMEOUT;

    loop {
        let peeked = tokio::time::timeout_at(deadline, stream.peek(&mut preface))
            .await
            .ok()? // timed out
            .ok()?; // connection error

        if peeked == 0 {
            return None; // closed before sending anything
        }
        if peeked >= HTTP2_PREFACE_PREFIX.len() {
            return Some(&preface[..HTTP2_PREFACE_PREFIX.len()] == HTTP2_PREFACE_PREFIX);
        }
        // Fewer bytes than we need to decide: if what did arrive already
        // differs from the HTTP/2 preface, no later byte can change that.
        if preface[..peeked] != HTTP2_PREFACE_PREFIX[..peeked] {
            return Some(false);
        }
        // Still ambiguous. `peek` returns the same bytes immediately, so wait
        // a moment instead of spinning on the same partial read.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// Run the local server until the process is interrupted (Ctrl-C).
pub async fn serve(config: LocalConfig) -> Result<()> {
    let server = spawn(config).await?;
    let config = server.config.clone();

    info!("fireup local listening on {}", server.addr);
    info!(
        "storage: {} ({})",
        server.store.backend(),
        if config.postgres_url.is_some() {
            "durable — documents and accounts survive a restart"
        } else {
            "in-memory — documents and accounts are lost on exit"
        }
    );
    for service in &config.services {
        info!(
            "serving {} over {} — point an SDK at it with: export {}={}",
            service.api(),
            service.protocol(),
            service.client_env_var(),
            server.addr
        );
    }
    if config.enables(Service::Auth) {
        // Said out loud at startup, not only in the docs: the tokens this
        // process mints are unsigned and anything that reaches the port can get
        // one for any account.
        warn!(
            "the auth service is for local development only: ID tokens are unsigned and \
             no API key or admin credential is checked"
        );
    }
    info!(
        "health: curl http://{}/health   discovery: curl http://{}/",
        server.addr, server.addr
    );

    tokio::signal::ctrl_c()
        .await
        .context("failed to listen for shutdown signal")?;
    info!("shutting down fireup local");
    server.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn http2_connections_are_told_apart_from_http1() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        for (payload, expected) in [
            (&b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"[..], Some(true)),
            (&b"GET / HTTP/1.1\r\n\r\n"[..], Some(false)),
            (&b"P"[..], None), // a prefix of the preface and nothing more
        ] {
            let client = tokio::spawn(async move {
                let mut stream = TcpStream::connect(addr).await.expect("connect");
                tokio::io::AsyncWriteExt::write_all(&mut stream, payload)
                    .await
                    .expect("write");
                // Hold the connection open so a partial preface stays partial.
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            });

            let (stream, _) = listener.accept().await.expect("accept");
            let classified = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                peek_is_http2(&stream),
            )
            .await;

            match expected {
                // The undecidable case must not classify: it has to keep
                // waiting rather than guess, so the timeout fires.
                None => assert!(classified.is_err(), "a partial preface must not be classified"),
                expected => assert_eq!(classified.expect("classified in time"), expected),
            }
            client.await.expect("client task");
        }
    }

    #[tokio::test]
    async fn an_empty_connection_is_dropped_rather_than_answered() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        let client = tokio::spawn(async move {
            let stream = TcpStream::connect(addr).await.expect("connect");
            drop(stream);
        });

        let (stream, _) = listener.accept().await.expect("accept");
        assert_eq!(peek_is_http2(&stream).await, None);
        client.await.expect("client task");
    }
}
