//! Durable [`UserStore`] implementation on PostgreSQL.
//!
//! One row per account in its own table, separate from the documents table, so
//! identity data and application data can be reasoned about (and dropped)
//! independently.
//!
//! Two constraints carry the semantics the API layer depends on:
//!
//! * `PRIMARY KEY (project_id, local_id)` — a uid is unique within a project,
//!   and the same uid may exist in two projects.
//! * a **partial** unique index on `(project_id, email)` where `email IS NOT
//!   NULL` — one account per email, while any number of anonymous accounts
//!   share a null email. A plain unique index would also work in PostgreSQL
//!   (nulls are distinct), but the partial index says the intent out loud and
//!   stays smaller.
//!
//! Emails arrive already normalised by
//! [`AuthUser::normalize_email`](crate::local::auth::user::AuthUser::normalize_email),
//! so the index and the lookups agree about case without a functional index.
//!
//! Timestamps are epoch-millisecond `bigint`s rather than `timestamptz`, for the
//! same reason the document store splits seconds and nanos: these values are
//! echoed back to clients verbatim, and a round trip through a type with
//! different precision would change them.

use anyhow::{anyhow, Context};
use deadpool_postgres::{Config, Pool, Runtime};
use tokio_postgres::types::ToSql;
use tokio_postgres::NoTls;
use tracing::{debug, info};

use super::store::{InsertOutcome, SharedUserStore, UpdateOutcome, UserStore};
use super::user::{AuthUser, PasswordHash, SignInProvider};
use crate::data_importer::parse_connection_url;
use crate::local::store::{StoreError, StoreResult};

/// Table holding the local auth surface's accounts.
pub const USERS_TABLE: &str = "fireup_local_auth_users";

/// Advisory-lock key serialising schema creation for this table. Distinct from
/// the documents table's key so a server and a seed creating both schemas at
/// once cannot deadlock on one lock in two orders.
const SCHEMA_LOCK_KEY: i64 = 4_589_000_2;

/// Statements that bring an empty database up to the current auth schema.
///
/// Idempotent and run on every start, like the document store's.
fn schema_statements() -> Vec<String> {
    vec![
        format!(
            "CREATE TABLE IF NOT EXISTS {USERS_TABLE} (
                 project_id           text    NOT NULL,
                 local_id             text    NOT NULL,
                 email                text,
                 email_verified       boolean NOT NULL,
                 display_name         text,
                 photo_url            text,
                 password_salt        text,
                 password_hash        text,
                 disabled             boolean NOT NULL,
                 custom_claims        jsonb,
                 provider             text    NOT NULL,
                 created_at_millis    bigint  NOT NULL,
                 last_login_at_millis bigint,
                 valid_since_millis   bigint  NOT NULL,
                 PRIMARY KEY (project_id, local_id)
             )"
        ),
        format!(
            "CREATE UNIQUE INDEX IF NOT EXISTS {USERS_TABLE}_email_idx
                 ON {USERS_TABLE} (project_id, email) WHERE email IS NOT NULL"
        ),
    ]
}

/// Every column, in the order [`row_to_user`] reads them.
const COLUMNS: &str = "project_id, local_id, email, email_verified, display_name, photo_url,
                       password_salt, password_hash, disabled, custom_claims, provider,
                       created_at_millis, last_login_at_millis, valid_since_millis";

/// A PostgreSQL-backed account store.
pub struct PostgresUserStore {
    pool: Pool,
    /// Host/port/database, for log lines. Never contains the password.
    target: String,
}

impl PostgresUserStore {
    /// Connect, verify the connection, and create the schema if needed.
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
            table = USERS_TABLE,
            "fireup local is storing auth accounts in PostgreSQL"
        );
        Ok(store)
    }

    /// Connect and hand back a [`SharedUserStore`], which is what the server
    /// holds.
    pub async fn connect_shared(
        postgres_url: &str,
        max_connections: usize,
    ) -> anyhow::Result<SharedUserStore> {
        Ok(std::sync::Arc::new(
            Self::connect(postgres_url, max_connections).await?,
        ))
    }

    async fn ensure_schema(&self) -> anyhow::Result<()> {
        let mut client = self
            .pool
            .get()
            .await
            .with_context(|| format!("failed to connect to {}", self.target))?;

        let transaction = client
            .transaction()
            .await
            .context("failed to begin the auth schema transaction")?;
        transaction
            .execute("SELECT pg_advisory_xact_lock($1)", &[&SCHEMA_LOCK_KEY])
            .await
            .context("failed to take the auth schema advisory lock")?;

        for statement in schema_statements() {
            transaction
                .batch_execute(&statement)
                .await
                .with_context(|| format!("failed to apply schema statement: {statement}"))?;
        }

        transaction
            .commit()
            .await
            .context("failed to commit the auth schema transaction")?;
        debug!(table = USERS_TABLE, "auth schema is up to date");
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

/// Rebuild an [`AuthUser`] from a row.
///
/// A row that cannot be read back is [`StoreError::Corrupt`], never silently
/// treated as a missing account: an account that exists but reads as absent is
/// how a developer ends up with a duplicate uid.
fn row_to_user(row: &tokio_postgres::Row) -> StoreResult<AuthUser> {
    let local_id: String = row.try_get("local_id").map_err(|error| corrupt("?", error))?;
    let corrupt_here = |error: tokio_postgres::Error| corrupt(&local_id, error);

    let salt: Option<String> = row.try_get("password_salt").map_err(corrupt_here)?;
    let hash: Option<String> = row.try_get("password_hash").map_err(corrupt_here)?;
    let provider_name: String = row.try_get("provider").map_err(corrupt_here)?;

    Ok(AuthUser {
        email: row.try_get("email").map_err(corrupt_here)?,
        email_verified: row.try_get("email_verified").map_err(corrupt_here)?,
        display_name: row.try_get("display_name").map_err(corrupt_here)?,
        photo_url: row.try_get("photo_url").map_err(corrupt_here)?,
        // Both columns or neither; one alone cannot verify a password, so it is
        // a corrupt row rather than a passwordless account.
        password: match (salt, hash) {
            (Some(salt), Some(hash)) => Some(PasswordHash::from_parts(salt, hash)),
            (None, None) => None,
            _ => {
                return Err(StoreError::Corrupt {
                    name: local_id,
                    reason: "exactly one of password_salt/password_hash is set".to_string(),
                })
            }
        },
        disabled: row.try_get("disabled").map_err(corrupt_here)?,
        custom_claims: row.try_get("custom_claims").map_err(corrupt_here)?,
        provider: SignInProvider::parse(&provider_name).ok_or_else(|| StoreError::Corrupt {
            name: local_id.clone(),
            reason: format!("{provider_name:?} is not a sign-in provider this build serves"),
        })?,
        created_at_millis: row.try_get("created_at_millis").map_err(corrupt_here)?,
        last_login_at_millis: row.try_get("last_login_at_millis").map_err(corrupt_here)?,
        valid_since_millis: row.try_get("valid_since_millis").map_err(corrupt_here)?,
        local_id,
    })
}

fn corrupt(local_id: &str, error: tokio_postgres::Error) -> StoreError {
    StoreError::Corrupt {
        name: local_id.to_string(),
        reason: format!("account column is not readable: {error}"),
    }
}

/// Whether a failure is the email unique index rejecting a write.
///
/// Checked by constraint name rather than by pre-reading the table: a `SELECT`
/// then `INSERT` is a race two concurrent sign-ups can both win, and the index
/// is the only thing that actually decides.
fn is_email_conflict(error: &tokio_postgres::Error) -> bool {
    let Some(db_error) = error.as_db_error() else {
        return false;
    };
    db_error.code() == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION
        && db_error
            .constraint()
            .is_some_and(|constraint| constraint == format!("{USERS_TABLE}_email_idx"))
}

/// Whether a failure is the primary key rejecting a duplicate uid.
fn is_local_id_conflict(error: &tokio_postgres::Error) -> bool {
    let Some(db_error) = error.as_db_error() else {
        return false;
    };
    db_error.code() == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION
        && !is_email_conflict(error)
        && db_error.constraint().is_some_and(|constraint| {
            constraint == format!("{USERS_TABLE}_pkey")
        })
}

#[async_trait::async_trait]
impl UserStore for PostgresUserStore {
    fn backend(&self) -> &'static str {
        "postgres"
    }

    async fn get_by_local_id(
        &self,
        project_id: &str,
        local_id: &str,
    ) -> StoreResult<Option<AuthUser>> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                &format!(
                    "SELECT {COLUMNS} FROM {USERS_TABLE}
                      WHERE project_id = $1 AND local_id = $2"
                ),
                &[&project_id, &local_id],
            )
            .await
            .with_context(|| format!("failed to read account {local_id}"))
            .map_err(StoreError::Backend)?;

        row.as_ref().map(row_to_user).transpose()
    }

    async fn get_by_email(&self, project_id: &str, email: &str) -> StoreResult<Option<AuthUser>> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                &format!(
                    "SELECT {COLUMNS} FROM {USERS_TABLE}
                      WHERE project_id = $1 AND email = $2"
                ),
                &[&project_id, &email],
            )
            .await
            .context("failed to read an account by email")
            .map_err(StoreError::Backend)?;

        row.as_ref().map(row_to_user).transpose()
    }

    async fn insert(&self, project_id: &str, user: AuthUser) -> StoreResult<InsertOutcome> {
        let client = self.client().await?;
        let salt = user.password.as_ref().map(|password| password.salt.clone());
        let hash = user.password.as_ref().map(|password| password.hash.clone());
        let provider = user.provider.as_str();

        let params: [&(dyn ToSql + Sync); 14] = [
            &project_id,
            &user.local_id,
            &user.email,
            &user.email_verified,
            &user.display_name,
            &user.photo_url,
            &salt,
            &hash,
            &user.disabled,
            &user.custom_claims,
            &provider,
            &user.created_at_millis,
            &user.last_login_at_millis,
            &user.valid_since_millis,
        ];

        let outcome = client
            .execute(
                &format!(
                    "INSERT INTO {USERS_TABLE} ({COLUMNS})
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)"
                ),
                &params,
            )
            .await;

        match outcome {
            Ok(_) => Ok(InsertOutcome::Inserted),
            Err(error) if is_email_conflict(&error) => Ok(InsertOutcome::EmailExists),
            Err(error) if is_local_id_conflict(&error) => Ok(InsertOutcome::LocalIdExists),
            Err(error) => Err(StoreError::Backend(anyhow!(
                "failed to create account {}: {error}",
                user.local_id
            ))),
        }
    }

    async fn update(&self, project_id: &str, user: AuthUser) -> StoreResult<UpdateOutcome> {
        let client = self.client().await?;
        let salt = user.password.as_ref().map(|password| password.salt.clone());
        let hash = user.password.as_ref().map(|password| password.hash.clone());
        let provider = user.provider.as_str();

        let params: [&(dyn ToSql + Sync); 14] = [
            &project_id,
            &user.local_id,
            &user.email,
            &user.email_verified,
            &user.display_name,
            &user.photo_url,
            &salt,
            &hash,
            &user.disabled,
            &user.custom_claims,
            &provider,
            &user.created_at_millis,
            &user.last_login_at_millis,
            &user.valid_since_millis,
        ];

        let outcome = client
            .execute(
                &format!(
                    "UPDATE {USERS_TABLE}
                        SET email = $3, email_verified = $4, display_name = $5, photo_url = $6,
                            password_salt = $7, password_hash = $8, disabled = $9,
                            custom_claims = $10, provider = $11, created_at_millis = $12,
                            last_login_at_millis = $13, valid_since_millis = $14
                      WHERE project_id = $1 AND local_id = $2"
                ),
                &params,
            )
            .await;

        match outcome {
            Ok(0) => Ok(UpdateOutcome::NotFound),
            Ok(_) => Ok(UpdateOutcome::Updated),
            Err(error) if is_email_conflict(&error) => Ok(UpdateOutcome::EmailExists),
            Err(error) => Err(StoreError::Backend(anyhow!(
                "failed to update account {}: {error}",
                user.local_id
            ))),
        }
    }

    async fn delete(&self, project_id: &str, local_id: &str) -> StoreResult<bool> {
        let client = self.client().await?;
        let deleted = client
            .execute(
                &format!("DELETE FROM {USERS_TABLE} WHERE project_id = $1 AND local_id = $2"),
                &[&project_id, &local_id],
            )
            .await
            .with_context(|| format!("failed to delete account {local_id}"))
            .map_err(StoreError::Backend)?;
        Ok(deleted > 0)
    }

    async fn list(
        &self,
        project_id: &str,
        start_after: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<AuthUser>> {
        let client = self.client().await?;
        // `$3 IS NULL OR local_id > $3` keeps this one prepared statement rather
        // than two, and still uses the primary key index for the bounded case.
        let page_limit = limit as i64;
        let rows = client
            .query(
                &format!(
                    "SELECT {COLUMNS} FROM {USERS_TABLE}
                      WHERE project_id = $1 AND ($2::text IS NULL OR local_id > $2)
                      ORDER BY local_id
                      LIMIT $3"
                ),
                &[&project_id, &start_after, &page_limit],
            )
            .await
            .context("failed to list accounts")
            .map_err(StoreError::Backend)?;

        rows.iter().map(row_to_user).collect()
    }

    async fn delete_all(&self, project_id: &str) -> StoreResult<usize> {
        let client = self.client().await?;
        let deleted = client
            .execute(
                &format!("DELETE FROM {USERS_TABLE} WHERE project_id = $1"),
                &[&project_id],
            )
            .await
            .context("failed to clear accounts")
            .map_err(StoreError::Backend)?;
        Ok(deleted as usize)
    }

    async fn count(&self, project_id: &str) -> StoreResult<usize> {
        let client = self.client().await?;
        let row = client
            .query_one(
                &format!("SELECT count(*) FROM {USERS_TABLE} WHERE project_id = $1"),
                &[&project_id],
            )
            .await
            .context("failed to count accounts")
            .map_err(StoreError::Backend)?;
        let count: i64 = row.get(0);
        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn the_email_index_is_partial_so_anonymous_accounts_do_not_collide() {
        let index = schema_statements()
            .into_iter()
            .find(|statement| statement.contains("_email_idx"))
            .expect("the email unique index must exist");

        assert!(index.contains("UNIQUE"), "{index}");
        assert!(
            index.contains("WHERE email IS NOT NULL"),
            "every anonymous account has a null email: {index}"
        );
    }

    #[test]
    fn the_accounts_table_is_separate_from_the_documents_table() {
        // Identity data must not be reachable through the Firestore API.
        assert_ne!(USERS_TABLE, crate::local::postgres_store::DOCUMENTS_TABLE);
    }

    #[test]
    fn the_two_schema_locks_are_distinct() {
        // Both schemas are created at startup; one shared key would serialise
        // them needlessly and two keys taken in different orders would deadlock,
        // so they are distinct and each is taken alone.
        assert_ne!(SCHEMA_LOCK_KEY, 4_589_000_1);
    }

    #[test]
    fn every_column_is_bound_in_the_insert_statement() {
        // The column list and the parameter list are written separately, so a
        // column added to one and not the other has to fail here rather than at
        // runtime.
        let columns = COLUMNS.split(',').count();
        assert_eq!(columns, 14, "COLUMNS lists {columns} columns");
    }
}
