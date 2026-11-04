//! Configuration for `fireup local`.
//!
//! One struct, built from CLI flags, consumed by the server and the seed
//! command alike so both agree on the project, database, and storage backend.

use std::net::SocketAddr;

/// Default port for `fireup local`.
///
/// Deliberately not 8080 and not the Firebase emulator's 8080/4000 defaults, so
/// running Fireup next to other local tooling does not collide.
pub const DEFAULT_PORT: u16 = 4589;

/// A service the local process can expose.
///
/// Phase 1 has one. The enum exists so `--enable` is a closed set from the
/// start — a typo is a CLI error rather than a service that silently never
/// starts — and so the discovery document grows a row, not a new shape, when
/// Datastore lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum)]
pub enum Service {
    /// The `google.firestore.v1.Firestore` gRPC API.
    Firestore,
}

impl Service {
    pub const fn id(self) -> &'static str {
        match self {
            Service::Firestore => "firestore",
        }
    }

    /// Fully-qualified gRPC service name served for this service.
    pub const fn api(self) -> &'static str {
        match self {
            Service::Firestore => "google.firestore.v1.Firestore",
        }
    }

    /// The documented environment variable official SDKs read to be pointed at
    /// a local endpoint instead of production.
    pub const fn client_env_var(self) -> &'static str {
        match self {
            Service::Firestore => "FIRESTORE_EMULATOR_HOST",
        }
    }
}

/// Everything `fireup local` needs to start.
#[derive(Debug, Clone)]
pub struct LocalConfig {
    /// Address to bind. Loopback by default — there is no auth layer.
    pub addr: SocketAddr,
    /// Project the data plane answers for, reported by discovery and used as
    /// the default when seeding.
    pub project_id: String,
    /// Database id. Firestore's default database is literally `(default)`.
    pub database_id: String,
    /// When set, documents are stored in PostgreSQL and survive a restart.
    /// When unset, storage is in-memory.
    pub postgres_url: Option<String>,
    /// Connection pool size for the PostgreSQL store.
    pub max_connections: usize,
    /// Services to enable.
    pub services: Vec<Service>,
}

impl LocalConfig {
    pub const DEFAULT_PROJECT_ID: &'static str = "fireup-local";
    pub const DEFAULT_DATABASE_ID: &'static str = "(default)";
    pub const DEFAULT_MAX_CONNECTIONS: usize = 5;

    /// Config for an in-memory server on `addr`, with every default.
    pub fn in_memory(addr: SocketAddr) -> Self {
        Self {
            addr,
            project_id: Self::DEFAULT_PROJECT_ID.to_string(),
            database_id: Self::DEFAULT_DATABASE_ID.to_string(),
            postgres_url: None,
            max_connections: Self::DEFAULT_MAX_CONNECTIONS,
            services: vec![Service::Firestore],
        }
    }

    /// Same, but storing documents in PostgreSQL.
    pub fn with_postgres(mut self, postgres_url: impl Into<String>) -> Self {
        self.postgres_url = Some(postgres_url.into());
        self
    }

    pub fn with_project_id(mut self, project_id: impl Into<String>) -> Self {
        self.project_id = project_id.into();
        self
    }

    pub fn with_database_id(mut self, database_id: impl Into<String>) -> Self {
        self.database_id = database_id.into();
        self
    }

    /// `projects/{project}/databases/{database}/documents` — the root every
    /// document name under this config hangs off.
    pub fn documents_root(&self) -> String {
        format!(
            "projects/{}/databases/{}/documents",
            self.project_id, self.database_id
        )
    }

    pub fn enables(&self, service: Service) -> bool {
        self.services.contains(&service)
    }

    /// Reject a configuration that cannot work, before anything binds a port or
    /// opens a connection.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.project_id.trim().is_empty() {
            anyhow::bail!("--project-id must not be empty");
        }
        // The project id becomes a path segment of every document name, so a
        // slash in it would produce names that parse as something else.
        if self.project_id.contains('/') {
            anyhow::bail!("--project-id must not contain '/' (got {:?})", self.project_id);
        }
        if self.database_id.trim().is_empty() {
            anyhow::bail!("--database-id must not be empty");
        }
        if self.database_id.contains('/') {
            anyhow::bail!(
                "--database-id must not contain '/' (got {:?})",
                self.database_id
            );
        }
        if self.services.is_empty() {
            anyhow::bail!("at least one service must be enabled (--enable firestore)");
        }
        if self.max_connections == 0 {
            anyhow::bail!("--max-connections must be at least 1");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> LocalConfig {
        LocalConfig::in_memory("127.0.0.1:0".parse().expect("valid address"))
    }

    #[test]
    fn the_documents_root_matches_the_firestore_resource_shape() {
        let config = config().with_project_id("demo").with_database_id("(default)");
        assert_eq!(
            config.documents_root(),
            "projects/demo/databases/(default)/documents"
        );
        // And the data plane's own parser must accept it as a parent.
        crate::local::names::parse_parent_name(&config.documents_root())
            .expect("the configured root must be a valid parent");
    }

    #[test]
    fn defaults_are_in_memory_with_firestore_enabled() {
        let config = config();
        assert!(config.postgres_url.is_none());
        assert!(config.enables(Service::Firestore));
        config.validate().expect("defaults must be valid");
    }

    #[test]
    fn ids_that_would_corrupt_resource_names_are_refused() {
        for bad in ["", " ", "a/b"] {
            config()
                .with_project_id(bad)
                .validate()
                .expect_err(&format!("project id {bad:?} must be rejected"));
            config()
                .with_database_id(bad)
                .validate()
                .expect_err(&format!("database id {bad:?} must be rejected"));
        }
    }

    #[test]
    fn an_empty_service_list_is_refused() {
        let mut config = config();
        config.services.clear();
        config.validate().expect_err("a server with no services does nothing");
    }
}
