//! `fireup local` — a local Firestore-compatible data plane.
//!
//! Fireup serves the **published** `google.firestore.v1` gRPC contract on a
//! single port so official Firestore client SDKs can point at it with the
//! documented `FIRESTORE_EMULATOR_HOST` environment variable.
//!
//! This is an independent implementation of a public API. It is not the Firebase
//! Emulator Suite, is not affiliated with Google, and contains no code or
//! behaviour derived from proprietary Google binaries. See
//! `.cursor/rules/firebase-gcp-compliance.mdc` and
//! `docs/licenses/FIREBASE_GCP_LOCAL.md`.
//!
//! Phase 0 scope is document CRUD against an in-memory store. Queries,
//! transactions, realtime listeners, and persistence are not served yet; those
//! RPCs answer `UNIMPLEMENTED` with a message saying so.

pub mod firestore;
pub mod names;
pub mod store;

use std::net::SocketAddr;

use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tracing::info;

pub use firestore::FirestoreApi;
pub use store::DocumentStore;

use crate::protos::google::firestore::v1::firestore_server::FirestoreServer;

/// Default port for `fireup local`.
///
/// Deliberately not 8080 and not the Firebase emulator's 8080/4000 defaults, so
/// running Fireup next to other local tooling does not collide.
pub const DEFAULT_PORT: u16 = 4589;

/// A running local server.
pub struct RunningServer {
    /// Address actually bound. With port 0 this is the OS-assigned port.
    pub addr: SocketAddr,
    /// Shared store, so callers (and tests) can inspect state directly.
    pub store: DocumentStore,
    handle: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

impl RunningServer {
    /// Stop the server and wait for it to finish.
    pub async fn shutdown(self) -> Result<()> {
        // A receive error means the server already stopped; nothing to signal.
        let _ = self.shutdown.send(());
        self.handle
            .await
            .context("local server task panicked")?
            .context("local server returned an error during shutdown")
    }
}

/// Bind `addr` and start serving in a background task.
///
/// Binding happens before returning so a caller (or a test) knows the port is
/// live and can connect without polling.
pub async fn spawn(addr: SocketAddr) -> Result<RunningServer> {
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    let bound = listener.local_addr().context("failed to read bound address")?;

    let store = DocumentStore::new();
    let api = FirestoreApi::new(store.clone());
    let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();

    let handle = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(FirestoreServer::new(api))
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                async {
                    let _ = shutdown_rx.await;
                },
            )
            .await
    });

    Ok(RunningServer {
        addr: bound,
        store,
        handle,
        shutdown,
    })
}

/// Run the local server until the process is interrupted (Ctrl-C).
pub async fn serve(addr: SocketAddr) -> Result<()> {
    let server = spawn(addr).await?;

    info!("fireup local listening on {}", server.addr);
    info!(
        "point an official Firestore SDK at it with: export FIRESTORE_EMULATOR_HOST={}",
        server.addr
    );
    info!("serving google.firestore.v1: document CRUD (in-memory, not persisted)");

    tokio::signal::ctrl_c()
        .await
        .context("failed to listen for shutdown signal")?;
    info!("shutting down fireup local");
    server.shutdown().await
}
