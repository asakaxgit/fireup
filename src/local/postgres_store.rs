//! Durable [`Store`] implementation on PostgreSQL.
//!
//! One row per document, keyed by the full Firestore resource name, with the
//! document's fields as a JSONB payload in the shape described in
//! [`value_json`](crate::local::value_json). The key components
//! (project, database, parent path, collection, document id) are split into
//! their own columns: they are what the collection reads and queries of a later
//! phase will filter and order on, and deriving them from the name at query
//! time would mean no index could help.
//!
//! ## Why timestamps are stored as seconds + nanos
//!
//! Firestore `create_time`/`update_time` are protobuf `Timestamp`s with
//! nanosecond precision; `timestamptz` only keeps microseconds. Clients compare
//! `update_time` exactly — it is a write precondition in the API — so rounding
//! it on the way to disk would make a precondition that should succeed fail
//! after a restart. Two integer columns keep the value exact and still sort
//! correctly.

use anyhow::{anyhow, Context};
use deadpool_postgres::{Config, Pool, Runtime};
use prost_types::Timestamp;
use tokio_postgres::types::ToSql;
use tokio_postgres::NoTls;
use tracing::{debug, info};

use crate::data_importer::parse_connection_url;
use crate::local::names::parse_document_name;
use crate::local::store::{Store, StoreError, StoreResult, WriteOutcome};
use crate::local::value_json;
use crate::protos::google::firestore::v1::{Document, Value};

/// Table holding the local data plane's documents.
///
/// Prefixed and distinct from anything `fireup import` generates, so pointing
/// `fireup local` at a database that also holds migrated tables cannot collide
/// with a collection named `documents`.
pub const DOCUMENTS_TABLE: &str = "fireup_local_documents";

/// Advisory-lock key serialising schema creation. Arbitrary but fixed: every
/// Fireup process has to pick the same number for the lock to mean anything.
const SCHEMA_LOCK_KEY: i64 = 4_589_000_1;

/// Statements that bring an empty database up to the current schema.
///
/// Idempotent, and run on every start: a fresh Docker Compose volume and a
/// database from last week both end up in the same state without a separate
/// migration step to remember.
fn schema_statements() -> Vec<String> {
    vec![
        format!(
            "CREATE TABLE IF NOT EXISTS {DOCUMENTS_TABLE} (
                 name                text PRIMARY KEY,
                 project_id          text   NOT NULL,
                 database_id         text   NOT NULL,
                 parent              text   NOT NULL,
                 collection_id       text   NOT NULL,
                 document_id         text   NOT NULL,
                 fields              jsonb  NOT NULL,
                 create_time_seconds bigint NOT NULL,
                 create_time_nanos   integer NOT NULL,
                 update_time_seconds bigint NOT NULL,
                 update_time_nanos   integer NOT NULL
             )"
        ),
        // Collection reads are "every document whose parent is P and whose
        // collection is C, ordered by document id" — this index is that query.
        format!(
            "CREATE INDEX IF NOT EXISTS {DOCUMENTS_TABLE}_collection_idx
                 ON {DOCUMENTS_TABLE} (project_id, database_id, parent, collection_id, document_id)"
        ),
    ]
}

/// A PostgreSQL-backed document store.
pub struct PostgresStore {
    pool: Pool,
    /// Host/port/database, for log lines. Never contains the password.
    target: String,
}

impl PostgresStore {
    /// Connect, verify the connection, and create the schema if needed.
    ///
    /// `postgres_url` is parsed by the same code as `fireup import`'s
    /// `--postgres-url`, so a URL that works for one works for the other.
    pub async fn connect(postgres_url: &str, max_connections: usize) -> anyhow::Result<Self> {
        let config = parse_connection_url(postgres_url, max_connections, 30)
            .map_err(|error| anyhow!("{error}"))?;
        let target = format!("{}:{}/{}", config.host, config.port, config.database);

        let mut pool_config = Config::new();
        pool_config.host = Some(config.host.clone());
        pool_config.port = Some(config.port);
        pool_config.dbname = Some(config.database.clone());
        pool_config.user = Some(config.user.clone());
        if !config.password.is_empty() {
            pool_config.password = Some(config.password.clone());
        }
        pool_config.pool = Some(deadpool_postgres::PoolConfig::new(max_connections));

        let pool = pool_config
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .with_context(|| format!("failed to create a connection pool for {target}"))?;

        let store = Self { pool, target };
        store.ensure_schema().await?;
        info!(
            target_db = %store.target,
            table = DOCUMENTS_TABLE,
            "fireup local is storing documents in PostgreSQL"
        );
        Ok(store)
    }

    /// Create the documents table and its index if they are missing.
    ///
    /// Held under an advisory lock because `CREATE TABLE IF NOT EXISTS` is not
    /// race-safe: two processes starting together (Docker Compose bringing up a
    /// server while a seed runs, or two test cases) otherwise collide in the
    /// system catalogue and one of them fails to start.
    async fn ensure_schema(&self) -> anyhow::Result<()> {
        let mut client = self
            .pool
            .get()
            .await
            .with_context(|| format!("failed to connect to {}", self.target))?;

        let transaction = client
            .transaction()
            .await
            .context("failed to begin the schema transaction")?;
        transaction
            .execute("SELECT pg_advisory_xact_lock($1)", &[&SCHEMA_LOCK_KEY])
            .await
            .context("failed to take the schema advisory lock")?;

        for statement in schema_statements() {
            transaction
                .batch_execute(&statement)
                .await
                .with_context(|| format!("failed to apply schema statement: {statement}"))?;
        }

        // Committing releases the lock, so the next process in line sees the
        // finished schema rather than a half-created one.
        transaction
            .commit()
            .await
            .context("failed to commit the schema transaction")?;
        debug!(table = DOCUMENTS_TABLE, "schema is up to date");
        Ok(())
    }

    async fn client(&self) -> StoreResult<deadpool_postgres::Object> {
        self.pool
            .get()
            .await
            .with_context(|| format!("failed to get a connection to {}", self.target))
            .map_err(StoreError::Backend)
    }
}

/// The key columns a document name maps onto.
#[derive(Debug)]
struct DocumentKey {
    project_id: String,
    database_id: String,
    parent: String,
    collection_id: String,
    document_id: String,
}

/// Split a resource name into its stored key columns.
///
/// Reuses the data plane's name parser so the store cannot disagree with the
/// API about what a valid document name is.
fn key_columns(name: &str) -> StoreResult<DocumentKey> {
    let parsed = parse_document_name(name).map_err(|status| {
        StoreError::Unsupported(format!(
            "{name:?} is not a storable document name: {}",
            status.message()
        ))
    })?;

    // `root` is projects/{project}/databases/{database}/documents.
    let root_parts: Vec<&str> = parsed.root.split('/').collect();
    Ok(DocumentKey {
        project_id: root_parts[1].to_string(),
        database_id: root_parts[3].to_string(),
        parent: parsed.parent(),
        collection_id: parsed.collection_id().to_string(),
        document_id: parsed.document_id().to_string(),
    })
}

fn encode_fields(name: &str, document: &Document) -> StoreResult<serde_json::Value> {
    value_json::encode_fields(&document.fields).map_err(|error| {
        StoreError::Unsupported(format!("document {name} cannot be stored: {error}"))
    })
}

fn row_to_document(name: &str, row: &tokio_postgres::Row) -> StoreResult<Document> {
    let payload: serde_json::Value = row.try_get("fields").map_err(|error| StoreError::Corrupt {
        name: name.to_string(),
        reason: format!("fields column is not readable as JSON: {error}"),
    })?;

    let fields = value_json::decode_fields(&payload).map_err(|error| StoreError::Corrupt {
        name: name.to_string(),
        reason: error.to_string(),
    })?;

    let timestamp = |seconds_column: &str, nanos_column: &str| -> StoreResult<Timestamp> {
        let seconds: i64 = row
            .try_get(seconds_column)
            .map_err(|error| StoreError::Corrupt {
                name: name.to_string(),
                reason: format!("{seconds_column} is not readable: {error}"),
            })?;
        let nanos: i32 = row
            .try_get(nanos_column)
            .map_err(|error| StoreError::Corrupt {
                name: name.to_string(),
                reason: format!("{nanos_column} is not readable: {error}"),
            })?;
        Ok(Timestamp { seconds, nanos })
    };

    Ok(Document {
        name: name.to_string(),
        fields,
        create_time: Some(timestamp("create_time_seconds", "create_time_nanos")?),
        update_time: Some(timestamp("update_time_seconds", "update_time_nanos")?),
    })
}

#[async_trait::async_trait]
impl Store for PostgresStore {
    fn backend(&self) -> &'static str {
        "postgres"
    }

    async fn get(&self, name: &str) -> StoreResult<Option<Document>> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                &format!(
                    "SELECT fields, create_time_seconds, create_time_nanos,
                            update_time_seconds, update_time_nanos
                       FROM {DOCUMENTS_TABLE} WHERE name = $1"
                ),
                &[&name],
            )
            .await
            .with_context(|| format!("failed to read document {name}"))
            .map_err(StoreError::Backend)?;

        row.as_ref().map(|row| row_to_document(name, row)).transpose()
    }

    async fn contains(&self, name: &str) -> StoreResult<bool> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                &format!("SELECT 1 FROM {DOCUMENTS_TABLE} WHERE name = $1"),
                &[&name],
            )
            .await
            .with_context(|| format!("failed to check for document {name}"))
            .map_err(StoreError::Backend)?;
        Ok(row.is_some())
    }

    async fn put(&self, document: Document, now: Timestamp) -> StoreResult<WriteOutcome> {
        let name = document.name.clone();
        let key = key_columns(&name)?;
        let payload = encode_fields(&name, &document)?;

        let mut client = self.client().await?;
        let transaction = client
            .transaction()
            .await
            .context("failed to begin a write transaction")
            .map_err(StoreError::Backend)?;

        // `FOR UPDATE` holds the row for the rest of the transaction, so two
        // concurrent writes to one document cannot interleave and lose a
        // create_time. It also tells us the outcome without relying on
        // upsert-internal state.
        let existing = transaction
            .query_opt(
                &format!(
                    "SELECT create_time_seconds, create_time_nanos
                       FROM {DOCUMENTS_TABLE} WHERE name = $1 FOR UPDATE"
                ),
                &[&name],
            )
            .await
            .with_context(|| format!("failed to lock document {name}"))
            .map_err(StoreError::Backend)?;

        let outcome = match existing {
            Some(_) => {
                transaction
                    .execute(
                        &format!(
                            "UPDATE {DOCUMENTS_TABLE}
                                SET fields = $2,
                                    update_time_seconds = $3,
                                    update_time_nanos = $4
                              WHERE name = $1"
                        ),
                        &[&name, &payload, &now.seconds, &now.nanos],
                    )
                    .await
                    .with_context(|| format!("failed to update document {name}"))
                    .map_err(StoreError::Backend)?;
                WriteOutcome::Updated
            }
            None => {
                insert_document(&transaction, &name, &key, &payload, &now).await?;
                WriteOutcome::Created
            }
        };

        transaction
            .commit()
            .await
            .with_context(|| format!("failed to commit the write of {name}"))
            .map_err(StoreError::Backend)?;
        Ok(outcome)
    }

    async fn merge(
        &self,
        name: &str,
        fields: Vec<(String, Value)>,
        now: Timestamp,
    ) -> StoreResult<WriteOutcome> {
        let key = key_columns(name)?;
        let merged: std::collections::HashMap<String, Value> = fields.into_iter().collect();
        let payload = value_json::encode_fields(&merged).map_err(|error| {
            StoreError::Unsupported(format!("fields for {name} cannot be stored: {error}"))
        })?;

        let mut client = self.client().await?;
        let transaction = client
            .transaction()
            .await
            .context("failed to begin a write transaction")
            .map_err(StoreError::Backend)?;

        let existing = transaction
            .query_opt(
                &format!("SELECT 1 FROM {DOCUMENTS_TABLE} WHERE name = $1 FOR UPDATE"),
                &[&name],
            )
            .await
            .with_context(|| format!("failed to lock document {name}"))
            .map_err(StoreError::Backend)?;

        let outcome = match existing {
            Some(_) => {
                // `||` is a shallow merge of top-level keys, which is exactly
                // the documented semantics of an update mask: named fields are
                // replaced, everything else is left alone.
                transaction
                    .execute(
                        &format!(
                            "UPDATE {DOCUMENTS_TABLE}
                                SET fields = fields || $2,
                                    update_time_seconds = $3,
                                    update_time_nanos = $4
                              WHERE name = $1"
                        ),
                        &[&name, &payload, &now.seconds, &now.nanos],
                    )
                    .await
                    .with_context(|| format!("failed to merge into document {name}"))
                    .map_err(StoreError::Backend)?;
                WriteOutcome::Updated
            }
            None => {
                insert_document(&transaction, name, &key, &payload, &now).await?;
                WriteOutcome::Created
            }
        };

        transaction
            .commit()
            .await
            .with_context(|| format!("failed to commit the merge into {name}"))
            .map_err(StoreError::Backend)?;
        Ok(outcome)
    }

    async fn delete(&self, name: &str) -> StoreResult<bool> {
        let client = self.client().await?;
        let deleted = client
            .execute(
                &format!("DELETE FROM {DOCUMENTS_TABLE} WHERE name = $1"),
                &[&name],
            )
            .await
            .with_context(|| format!("failed to delete document {name}"))
            .map_err(StoreError::Backend)?;
        Ok(deleted > 0)
    }

    async fn count(&self) -> StoreResult<usize> {
        let client = self.client().await?;
        let row = client
            .query_one(&format!("SELECT count(*) FROM {DOCUMENTS_TABLE}"), &[])
            .await
            .context("failed to count documents")
            .map_err(StoreError::Backend)?;
        let count: i64 = row.get(0);
        Ok(count as usize)
    }
}

/// Insert a brand-new document row. `create_time` and `update_time` both start
/// at `now`, which is what Firestore reports for a freshly created document.
async fn insert_document(
    transaction: &tokio_postgres::Transaction<'_>,
    name: &str,
    key: &DocumentKey,
    payload: &serde_json::Value,
    now: &Timestamp,
) -> StoreResult<()> {
    // A new document's create_time and update_time are the same instant, so
    // $8/$9 are bound once and referenced twice.
    let params: [&(dyn ToSql + Sync); 9] = [
        &name,
        &key.project_id,
        &key.database_id,
        &key.parent,
        &key.collection_id,
        &key.document_id,
        payload,
        &now.seconds,
        &now.nanos,
    ];

    transaction
        .execute(
            &format!(
                "INSERT INTO {DOCUMENTS_TABLE}
                     (name, project_id, database_id, parent, collection_id, document_id,
                      fields, create_time_seconds, create_time_nanos,
                      update_time_seconds, update_time_nanos)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $8, $9)"
            ),
            &params,
        )
        .await
        .with_context(|| format!("failed to insert document {name}"))
        .map_err(StoreError::Backend)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_columns_split_a_top_level_document() {
        let key = key_columns("projects/demo/databases/(default)/documents/users/alice")
            .expect("valid name");
        assert_eq!(key.project_id, "demo");
        assert_eq!(key.database_id, "(default)");
        assert_eq!(key.parent, "projects/demo/databases/(default)/documents");
        assert_eq!(key.collection_id, "users");
        assert_eq!(key.document_id, "alice");
    }

    #[test]
    fn key_columns_split_a_subcollection_document() {
        let key = key_columns("projects/demo/databases/(default)/documents/users/alice/posts/p1")
            .expect("valid name");
        assert_eq!(
            key.parent,
            "projects/demo/databases/(default)/documents/users/alice"
        );
        assert_eq!(key.collection_id, "posts");
        assert_eq!(key.document_id, "p1");
    }

    #[test]
    fn key_columns_reject_a_name_that_is_not_a_document() {
        let error = key_columns("users/alice").expect_err("unrooted name");
        assert!(matches!(error, StoreError::Unsupported(_)), "{error}");
    }

    #[test]
    fn the_schema_is_idempotent_by_construction() {
        // Startup applies these on every boot, so each one has to be safe to
        // re-run against an existing database.
        for statement in schema_statements() {
            assert!(
                statement.contains("IF NOT EXISTS"),
                "schema statement is not re-runnable: {statement}"
            );
        }
    }
}
