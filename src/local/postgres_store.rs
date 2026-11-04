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
use crate::local::store::{self, PendingWrite, Store, StoreError, StoreResult, WriteOutcome};
use crate::local::value_json;
use crate::protos::google::firestore::v1::Document;

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
        // collection is C" — this index is that query. `parent` leads because
        // it already encodes the project and database (it starts with
        // `projects/{p}/databases/{d}/documents`), so putting those columns
        // first would only have pushed the columns the query filters on out of
        // the index prefix.
        format!(
            "CREATE INDEX IF NOT EXISTS {DOCUMENTS_TABLE}_collection_idx
                 ON {DOCUMENTS_TABLE} (parent, collection_id, document_id)"
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

    async fn commit(
        &self,
        writes: Vec<PendingWrite>,
        now: Timestamp,
    ) -> StoreResult<Vec<WriteOutcome>> {
        // Validate every name before touching the database, so a batch with one
        // unstorable name fails without having opened a transaction.
        let keys: Vec<DocumentKey> = writes
            .iter()
            .map(|write| key_columns(write.mutation.name()))
            .collect::<StoreResult<_>>()?;

        let mut client = self.client().await?;
        let transaction = client
            .transaction()
            .await
            .context("failed to begin a write transaction")
            .map_err(StoreError::Backend)?;

        // Lock every document in the batch up front, in sorted order.
        //
        // An advisory lock rather than `SELECT … FOR UPDATE` because a row lock
        // on a row that does not exist locks nothing: two concurrent
        // `create()` calls would both read "absent", both pass their
        // `exists: false` precondition, and the second would overwrite the
        // first instead of reporting ALREADY_EXISTS. Sorting the names makes
        // two overlapping batches take their locks in the same order, so they
        // queue instead of deadlocking.
        let mut locked: Vec<&str> = writes.iter().map(|write| write.mutation.name()).collect();
        locked.sort_unstable();
        locked.dedup();
        for name in locked {
            transaction
                .execute("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))", &[&name])
                .await
                .with_context(|| format!("failed to lock document {name}"))
                .map_err(StoreError::Backend)?;
        }

        let mut outcomes = Vec::with_capacity(writes.len());
        for (write, key) in writes.into_iter().zip(keys.iter()) {
            let name = write.mutation.name().to_string();
            // Reads inside the transaction, so a second write to the same
            // document in this batch sees the first one's effect.
            let stored = read_document(&transaction, &name).await?;
            store::check_precondition(&name, stored.as_ref(), write.precondition.as_ref())?;

            let (next, outcome) = store::mutate(write.mutation, stored, &now);
            match next {
                Some(document) => upsert_document(&transaction, &name, key, &document).await?,
                None => {
                    transaction
                        .execute(
                            &format!("DELETE FROM {DOCUMENTS_TABLE} WHERE name = $1"),
                            &[&name],
                        )
                        .await
                        .with_context(|| format!("failed to delete document {name}"))
                        .map_err(StoreError::Backend)?;
                }
            }
            outcomes.push(outcome);
        }

        // Returning early anywhere above drops the transaction, which rolls it
        // back — that is what makes the batch atomic.
        transaction
            .commit()
            .await
            .context("failed to commit the write batch")
            .map_err(StoreError::Backend)?;
        Ok(outcomes)
    }

    async fn scan_collection(
        &self,
        parent: &str,
        collection_id: &str,
    ) -> StoreResult<Vec<Document>> {
        let client = self.client().await?;
        let rows = client
            .query(
                &format!(
                    "SELECT name, fields, create_time_seconds, create_time_nanos,
                            update_time_seconds, update_time_nanos
                       FROM {DOCUMENTS_TABLE}
                      WHERE parent = $1 AND collection_id = $2"
                ),
                &[&parent, &collection_id],
            )
            .await
            .with_context(|| format!("failed to read collection {parent}/{collection_id}"))
            .map_err(StoreError::Backend)?;

        let mut documents = Vec::with_capacity(rows.len());
        for row in &rows {
            let name: String = row.try_get("name").map_err(|error| StoreError::Corrupt {
                name: format!("{parent}/{collection_id}"),
                reason: format!("name column is not readable: {error}"),
            })?;
            documents.push(row_to_document(&name, row)?);
        }

        // Sorted here rather than in SQL: the contract is UTF-8 byte order,
        // which is what `MemoryStore` and Firestore's `__name__` ordering use,
        // and `ORDER BY` would instead use the database's collation.
        documents.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
        Ok(documents)
    }

    async fn collection_ids(&self, parent: &str) -> StoreResult<Vec<String>> {
        let prefix = format!("{parent}/");
        // 1-based, and one past the prefix: the first segment of what is left.
        let first_segment_at = prefix.len() as i32 + 1;

        let client = self.client().await?;
        let rows = client
            .query(
                &format!(
                    "SELECT DISTINCT split_part(substr(name, $2), '/', 1) AS collection_id
                       FROM {DOCUMENTS_TABLE}
                      WHERE starts_with(name, $1)"
                ),
                &[&prefix, &first_segment_at],
            )
            .await
            .with_context(|| format!("failed to list the collections under {parent}"))
            .map_err(StoreError::Backend)?;

        let mut ids: Vec<String> = rows
            .iter()
            .map(|row| {
                row.try_get::<_, String>("collection_id")
                    .map_err(|error| StoreError::Corrupt {
                        name: parent.to_string(),
                        reason: format!("collection id is not readable: {error}"),
                    })
            })
            .collect::<StoreResult<_>>()?;
        ids.retain(|id| !id.is_empty());
        ids.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        Ok(ids)
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

/// Read one document inside a transaction.
async fn read_document(
    transaction: &tokio_postgres::Transaction<'_>,
    name: &str,
) -> StoreResult<Option<Document>> {
    let row = transaction
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

/// Write `document` as the complete current state of its row.
///
/// The caller has already resolved what the row should contain — including
/// which `create_time` survives and which fields a merge keeps — in
/// `store::mutate`, so there is one definition of those semantics rather than
/// one per backend. `create_time` is deliberately left alone on conflict: it
/// only changes when a document is deleted and recreated, which is a delete
/// followed by an insert, not an update.
async fn upsert_document(
    transaction: &tokio_postgres::Transaction<'_>,
    name: &str,
    key: &DocumentKey,
    document: &Document,
) -> StoreResult<()> {
    let payload = encode_fields(name, document)?;
    let create_time = document.create_time.clone().unwrap_or_default();
    let update_time = document.update_time.clone().unwrap_or_default();

    let params: [&(dyn ToSql + Sync); 11] = [
        &name,
        &key.project_id,
        &key.database_id,
        &key.parent,
        &key.collection_id,
        &key.document_id,
        &payload,
        &create_time.seconds,
        &create_time.nanos,
        &update_time.seconds,
        &update_time.nanos,
    ];

    transaction
        .execute(
            &format!(
                "INSERT INTO {DOCUMENTS_TABLE}
                     (name, project_id, database_id, parent, collection_id, document_id,
                      fields, create_time_seconds, create_time_nanos,
                      update_time_seconds, update_time_nanos)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                 ON CONFLICT (name) DO UPDATE
                     SET fields = EXCLUDED.fields,
                         update_time_seconds = EXCLUDED.update_time_seconds,
                         update_time_nanos = EXCLUDED.update_time_nanos"
            ),
            &params,
        )
        .await
        .with_context(|| format!("failed to write document {name}"))
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
