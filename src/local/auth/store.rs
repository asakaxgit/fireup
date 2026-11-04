//! Account storage for the local auth surface.
//!
//! Deliberately a **separate** store from
//! [`Store`](crate::local::store::Store), not a reserved collection inside it.
//! Accounts kept as Firestore documents would show up in `GET /health`'s
//! document count, in a `fireup local` seed, and — worst — be readable and
//! writable through the Firestore API by any client that guessed the path.
//! Identity data is not application data.
//!
//! The shape follows the document store's: a narrow trait, an in-memory default
//! that needs no setup, a PostgreSQL implementation for `--postgres-url`, and
//! one [`conformance`] suite both have to pass so the two cannot drift.
//!
//! Every method is scoped by project id. There is one project per process
//! today, but the admin endpoints are project-scoped in their URLs, so the store
//! takes the project rather than assuming it.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use super::user::AuthUser;
use crate::local::store::{StoreError, StoreResult};

/// A shared handle to whichever account store the process was configured with.
pub type SharedUserStore = Arc<dyn UserStore>;

/// Result of trying to create an account.
///
/// A distinct type rather than an error, because "that email is taken" is a
/// normal client outcome that maps to a specific Identity Toolkit code
/// (`EMAIL_EXISTS`), not a storage failure.
#[derive(Debug, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted,
    EmailExists,
    LocalIdExists,
}

/// Account storage for the local Identity Toolkit surface.
#[async_trait::async_trait]
pub trait UserStore: Send + Sync + 'static {
    /// Short backend name, for logs and the discovery endpoint.
    fn backend(&self) -> &'static str;

    async fn get_by_local_id(
        &self,
        project_id: &str,
        local_id: &str,
    ) -> StoreResult<Option<AuthUser>>;

    /// Look an account up by email. `email` is normalised by the caller via
    /// [`AuthUser::normalize_email`].
    async fn get_by_email(&self, project_id: &str, email: &str) -> StoreResult<Option<AuthUser>>;

    /// Create an account, refusing to collide with an existing uid or email.
    async fn insert(&self, project_id: &str, user: AuthUser) -> StoreResult<InsertOutcome>;

    /// Replace an existing account wholesale, matched on `local_id`.
    ///
    /// Returns `EmailExists` if the new email belongs to a different account,
    /// and `Ok(false)` — via `LocalIdExists`'s absence — is not expressible, so
    /// a missing account is reported as `None`.
    async fn update(&self, project_id: &str, user: AuthUser) -> StoreResult<UpdateOutcome>;

    /// Remove an account. Returns whether it existed.
    async fn delete(&self, project_id: &str, local_id: &str) -> StoreResult<bool>;

    /// Accounts ordered by `local_id`, starting strictly after `start_after`.
    ///
    /// Ordering plus a cursor rather than an offset because that is what
    /// `accounts:batchGet`'s `nextPageToken` is: resume from the last uid seen.
    async fn list(
        &self,
        project_id: &str,
        start_after: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<AuthUser>>;

    /// Delete every account in the project. Returns how many went.
    async fn delete_all(&self, project_id: &str) -> StoreResult<usize>;

    async fn count(&self, project_id: &str) -> StoreResult<usize>;
}

/// Result of replacing an account.
#[derive(Debug, PartialEq, Eq)]
pub enum UpdateOutcome {
    Updated,
    /// No account with that uid.
    NotFound,
    /// The requested email already belongs to another account.
    EmailExists,
}

/// In-memory accounts: a `BTreeMap` keyed by `(project_id, local_id)`.
///
/// `BTreeMap` because `accounts:batchGet` pages in uid order, which is a range
/// scan over exactly this key.
#[derive(Clone, Default)]
pub struct MemoryUserStore {
    users: Arc<RwLock<BTreeMap<(String, String), AuthUser>>>,
}

impl MemoryUserStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl UserStore for MemoryUserStore {
    fn backend(&self) -> &'static str {
        "memory"
    }

    async fn get_by_local_id(
        &self,
        project_id: &str,
        local_id: &str,
    ) -> StoreResult<Option<AuthUser>> {
        Ok(self
            .users
            .read()
            .expect("account store lock poisoned")
            .get(&(project_id.to_string(), local_id.to_string()))
            .cloned())
    }

    async fn get_by_email(&self, project_id: &str, email: &str) -> StoreResult<Option<AuthUser>> {
        Ok(self
            .users
            .read()
            .expect("account store lock poisoned")
            .iter()
            .find(|((project, _), user)| {
                project == project_id && user.email.as_deref() == Some(email)
            })
            .map(|(_, user)| user.clone()))
    }

    async fn insert(&self, project_id: &str, user: AuthUser) -> StoreResult<InsertOutcome> {
        let mut users = self.users.write().expect("account store lock poisoned");
        let key = (project_id.to_string(), user.local_id.clone());

        if users.contains_key(&key) {
            return Ok(InsertOutcome::LocalIdExists);
        }
        if let Some(email) = &user.email {
            let taken = users.iter().any(|((project, _), existing)| {
                project == project_id && existing.email.as_deref() == Some(email.as_str())
            });
            if taken {
                return Ok(InsertOutcome::EmailExists);
            }
        }

        users.insert(key, user);
        Ok(InsertOutcome::Inserted)
    }

    async fn update(&self, project_id: &str, user: AuthUser) -> StoreResult<UpdateOutcome> {
        let mut users = self.users.write().expect("account store lock poisoned");
        let key = (project_id.to_string(), user.local_id.clone());

        if !users.contains_key(&key) {
            return Ok(UpdateOutcome::NotFound);
        }
        if let Some(email) = &user.email {
            let taken = users.iter().any(|((project, local_id), existing)| {
                project == project_id
                    && local_id != &user.local_id
                    && existing.email.as_deref() == Some(email.as_str())
            });
            if taken {
                return Ok(UpdateOutcome::EmailExists);
            }
        }

        users.insert(key, user);
        Ok(UpdateOutcome::Updated)
    }

    async fn delete(&self, project_id: &str, local_id: &str) -> StoreResult<bool> {
        Ok(self
            .users
            .write()
            .expect("account store lock poisoned")
            .remove(&(project_id.to_string(), local_id.to_string()))
            .is_some())
    }

    async fn list(
        &self,
        project_id: &str,
        start_after: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<AuthUser>> {
        Ok(self
            .users
            .read()
            .expect("account store lock poisoned")
            .iter()
            .filter(|((project, local_id), _)| {
                project == project_id
                    && start_after.is_none_or(|cursor| local_id.as_str() > cursor)
            })
            .take(limit)
            .map(|(_, user)| user.clone())
            .collect())
    }

    async fn delete_all(&self, project_id: &str) -> StoreResult<usize> {
        let mut users = self.users.write().expect("account store lock poisoned");
        let before = users.len();
        users.retain(|(project, _), _| project != project_id);
        Ok(before - users.len())
    }

    async fn count(&self, project_id: &str) -> StoreResult<usize> {
        Ok(self
            .users
            .read()
            .expect("account store lock poisoned")
            .keys()
            .filter(|(project, _)| project == project_id)
            .count())
    }
}

/// A [`UserStore`] that fails every call, so the HTTP surface can be tested
/// against a broken backend without one.
#[doc(hidden)]
pub struct FailingUserStore;

#[async_trait::async_trait]
impl UserStore for FailingUserStore {
    fn backend(&self) -> &'static str {
        "failing"
    }

    async fn get_by_local_id(&self, _: &str, _: &str) -> StoreResult<Option<AuthUser>> {
        Err(broken())
    }
    async fn get_by_email(&self, _: &str, _: &str) -> StoreResult<Option<AuthUser>> {
        Err(broken())
    }
    async fn insert(&self, _: &str, _: AuthUser) -> StoreResult<InsertOutcome> {
        Err(broken())
    }
    async fn update(&self, _: &str, _: AuthUser) -> StoreResult<UpdateOutcome> {
        Err(broken())
    }
    async fn delete(&self, _: &str, _: &str) -> StoreResult<bool> {
        Err(broken())
    }
    async fn list(&self, _: &str, _: Option<&str>, _: usize) -> StoreResult<Vec<AuthUser>> {
        Err(broken())
    }
    async fn delete_all(&self, _: &str) -> StoreResult<usize> {
        Err(broken())
    }
    async fn count(&self, _: &str) -> StoreResult<usize> {
        Err(broken())
    }
}

fn broken() -> StoreError {
    StoreError::Backend(anyhow::anyhow!("account store is deliberately unavailable"))
}

/// Contract assertions every [`UserStore`] implementation must pass.
///
/// Compiled into the library (not behind `cfg(test)`) so the PostgreSQL
/// integration test runs exactly these assertions against a real database. A
/// durable backend that disagrees with the in-memory one about email uniqueness
/// or paging order is a bug that only surfaces as strange SDK behaviour weeks
/// later.
pub mod conformance {
    use super::*;
    use crate::local::auth::user::{PasswordHash, SignInProvider};

    fn user(local_id: &str, email: Option<&str>) -> AuthUser {
        let mut user = AuthUser::new(local_id, SignInProvider::Password, 1_700_000_000_000);
        if let Some(email) = email {
            user.set_email(email);
            user.password = Some(PasswordHash::new("secret12"));
        } else {
            user.provider = SignInProvider::Anonymous;
        }
        user
    }

    /// Run every contract assertion against `store`.
    ///
    /// `project_id` scopes the accounts, so concurrent runs against one shared
    /// database do not collide.
    pub async fn assert_user_store_contract(store: &dyn UserStore, project_id: &str) {
        // Start from a known-empty project; a shared database may hold rows from
        // an earlier run of this same suite.
        store.delete_all(project_id).await.expect("clear");

        insert_then_read_back_by_uid_and_email(store, project_id).await;
        a_duplicate_email_is_refused(store, project_id).await;
        a_duplicate_uid_is_refused(store, project_id).await;
        emails_are_stored_as_given_and_matched_exactly(store, project_id).await;
        update_replaces_the_whole_account(store, project_id).await;
        update_refuses_to_steal_another_accounts_email(store, project_id).await;
        update_allows_an_account_to_keep_its_own_email(store, project_id).await;
        update_of_a_missing_account_is_reported(store, project_id).await;
        anonymous_accounts_do_not_collide_on_a_null_email(store, project_id).await;
        delete_reports_whether_the_account_existed(store, project_id).await;
        listing_pages_in_uid_order(store, project_id).await;
        another_project_is_invisible(store, project_id).await;
        delete_all_empties_only_this_project(store, project_id).await;
    }

    async fn insert_then_read_back_by_uid_and_email(store: &dyn UserStore, project: &str) {
        let mut original = user("uid-read", Some("read@example.com"));
        original.display_name = Some("Reader".into());
        original.email_verified = true;
        original.custom_claims = Some(serde_json::json!({ "role": "reader" }));
        original.last_login_at_millis = Some(1_700_000_001_000);

        assert_eq!(
            store.insert(project, original.clone()).await.expect("insert"),
            InsertOutcome::Inserted
        );

        let by_uid = store
            .get_by_local_id(project, "uid-read")
            .await
            .expect("get by uid")
            .expect("account stored");
        assert_eq!(by_uid, original, "every field must survive a round trip");

        let by_email = store
            .get_by_email(project, "read@example.com")
            .await
            .expect("get by email")
            .expect("found by email");
        assert_eq!(by_email.local_id, "uid-read");

        assert!(
            by_uid
                .password
                .as_ref()
                .expect("password stored")
                .verify("secret12"),
            "a stored password must still verify"
        );
    }

    async fn a_duplicate_email_is_refused(store: &dyn UserStore, project: &str) {
        store
            .insert(project, user("uid-dup-a", Some("dup@example.com")))
            .await
            .expect("first insert");

        assert_eq!(
            store
                .insert(project, user("uid-dup-b", Some("dup@example.com")))
                .await
                .expect("second insert"),
            InsertOutcome::EmailExists
        );
        assert!(
            store
                .get_by_local_id(project, "uid-dup-b")
                .await
                .expect("get")
                .is_none(),
            "a refused insert must not have written anything"
        );
    }

    async fn a_duplicate_uid_is_refused(store: &dyn UserStore, project: &str) {
        store
            .insert(project, user("uid-same", Some("same-a@example.com")))
            .await
            .expect("first insert");

        assert_eq!(
            store
                .insert(project, user("uid-same", Some("same-b@example.com")))
                .await
                .expect("second insert"),
            InsertOutcome::LocalIdExists
        );
        let stored = store
            .get_by_local_id(project, "uid-same")
            .await
            .expect("get")
            .expect("original survives");
        assert_eq!(stored.email.as_deref(), Some("same-a@example.com"));
    }

    async fn emails_are_stored_as_given_and_matched_exactly(store: &dyn UserStore, project: &str) {
        // Normalisation is the API layer's job; the store must not second-guess
        // it, or the unique index and the lookup would disagree about case.
        store
            .insert(project, user("uid-case", Some("Case@Example.com")))
            .await
            .expect("insert");

        let stored = store
            .get_by_local_id(project, "uid-case")
            .await
            .expect("get")
            .expect("stored");
        assert_eq!(
            stored.email.as_deref(),
            Some("case@example.com"),
            "AuthUser::set_email normalises before the store sees it"
        );
        assert!(store
            .get_by_email(project, "case@example.com")
            .await
            .expect("get")
            .is_some());
    }

    async fn update_replaces_the_whole_account(store: &dyn UserStore, project: &str) {
        store
            .insert(project, user("uid-upd", Some("upd@example.com")))
            .await
            .expect("insert");

        let mut changed = user("uid-upd", Some("upd2@example.com"));
        changed.display_name = Some("Renamed".into());
        changed.disabled = true;
        changed.valid_since_millis = 1_800_000_000_000;
        changed.password = None;

        assert_eq!(
            store.update(project, changed.clone()).await.expect("update"),
            UpdateOutcome::Updated
        );
        let stored = store
            .get_by_local_id(project, "uid-upd")
            .await
            .expect("get")
            .expect("still there");
        assert_eq!(stored, changed);
        assert!(
            store
                .get_by_email(project, "upd@example.com")
                .await
                .expect("get")
                .is_none(),
            "the old email must stop resolving"
        );
    }

    async fn update_refuses_to_steal_another_accounts_email(store: &dyn UserStore, project: &str) {
        store
            .insert(project, user("uid-steal-a", Some("steal-a@example.com")))
            .await
            .expect("insert a");
        store
            .insert(project, user("uid-steal-b", Some("steal-b@example.com")))
            .await
            .expect("insert b");

        let mut thief = user("uid-steal-b", Some("steal-a@example.com"));
        thief.display_name = Some("Thief".into());
        assert_eq!(
            store.update(project, thief).await.expect("update"),
            UpdateOutcome::EmailExists
        );

        let untouched = store
            .get_by_local_id(project, "uid-steal-b")
            .await
            .expect("get")
            .expect("still there");
        assert_eq!(untouched.email.as_deref(), Some("steal-b@example.com"));
        assert!(untouched.display_name.is_none(), "a refused update writes nothing");
    }

    async fn update_allows_an_account_to_keep_its_own_email(store: &dyn UserStore, project: &str) {
        store
            .insert(project, user("uid-keep", Some("keep@example.com")))
            .await
            .expect("insert");

        let mut same_email = user("uid-keep", Some("keep@example.com"));
        same_email.email_verified = true;
        assert_eq!(
            store.update(project, same_email).await.expect("update"),
            UpdateOutcome::Updated,
            "an account must be able to update without changing its email"
        );
    }

    async fn update_of_a_missing_account_is_reported(store: &dyn UserStore, project: &str) {
        assert_eq!(
            store
                .update(project, user("uid-ghost", Some("ghost@example.com")))
                .await
                .expect("update"),
            UpdateOutcome::NotFound
        );
    }

    async fn anonymous_accounts_do_not_collide_on_a_null_email(
        store: &dyn UserStore,
        project: &str,
    ) {
        // Several anonymous accounts is the normal case, and they all have no
        // email — a unique index that treated that as a value would break on the
        // second sign-in.
        for uid in ["uid-anon-1", "uid-anon-2", "uid-anon-3"] {
            assert_eq!(
                store.insert(project, user(uid, None)).await.expect("insert"),
                InsertOutcome::Inserted,
                "anonymous account {uid} must be insertable"
            );
        }
        assert!(store
            .get_by_local_id(project, "uid-anon-3")
            .await
            .expect("get")
            .is_some());
    }

    async fn delete_reports_whether_the_account_existed(store: &dyn UserStore, project: &str) {
        store
            .insert(project, user("uid-del", Some("del@example.com")))
            .await
            .expect("insert");

        assert!(store.delete(project, "uid-del").await.expect("delete"));
        assert!(
            !store.delete(project, "uid-del").await.expect("second delete"),
            "deleting a missing account is not an error"
        );
        assert!(store
            .get_by_email(project, "del@example.com")
            .await
            .expect("get")
            .is_none());
    }

    async fn listing_pages_in_uid_order(store: &dyn UserStore, project: &str) {
        store.delete_all(project).await.expect("clear");
        for uid in ["page-a", "page-b", "page-c", "page-d"] {
            store
                .insert(project, user(uid, Some(&format!("{uid}@example.com"))))
                .await
                .expect("insert");
        }

        let first = store.list(project, None, 2).await.expect("first page");
        let ids: Vec<&str> = first.iter().map(|user| user.local_id.as_str()).collect();
        assert_eq!(ids, vec!["page-a", "page-b"]);

        let second = store
            .list(project, Some("page-b"), 2)
            .await
            .expect("second page");
        let ids: Vec<&str> = second.iter().map(|user| user.local_id.as_str()).collect();
        assert_eq!(ids, vec!["page-c", "page-d"], "the cursor is exclusive");

        assert!(store
            .list(project, Some("page-d"), 2)
            .await
            .expect("past the end")
            .is_empty());
        assert_eq!(store.count(project).await.expect("count"), 4);
    }

    async fn another_project_is_invisible(store: &dyn UserStore, project: &str) {
        let other = format!("{project}-other");
        store.delete_all(&other).await.expect("clear other");
        store
            .insert(&other, user("shared-uid", Some("shared@example.com")))
            .await
            .expect("insert into the other project");

        assert!(
            store
                .get_by_local_id(project, "shared-uid")
                .await
                .expect("get")
                .is_none(),
            "accounts must not leak across projects"
        );
        assert!(store
            .get_by_email(project, "shared@example.com")
            .await
            .expect("get")
            .is_none());
        // And the same uid must be insertable in both.
        assert_eq!(
            store
                .insert(project, user("shared-uid", Some("shared@example.com")))
                .await
                .expect("insert"),
            InsertOutcome::Inserted
        );
    }

    async fn delete_all_empties_only_this_project(store: &dyn UserStore, project: &str) {
        let other = format!("{project}-other");
        let other_before = store.count(&other).await.expect("count other");
        assert!(other_before > 0, "the other project must be seeded by now");

        let removed = store.delete_all(project).await.expect("delete all");
        assert!(removed > 0, "there were accounts to remove");
        assert_eq!(store.count(project).await.expect("count"), 0);
        assert_eq!(
            store.count(&other).await.expect("count other"),
            other_before,
            "clearing one project must not touch another"
        );

        store.delete_all(&other).await.expect("clean up");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_user_store_satisfies_the_contract() {
        let store = MemoryUserStore::new();
        conformance::assert_user_store_contract(&store, "demo").await;
    }

    #[tokio::test]
    async fn clones_share_one_set_of_accounts() {
        let store = MemoryUserStore::new();
        let clone = store.clone();
        store
            .insert(
                "demo",
                AuthUser::new("uid", super::super::user::SignInProvider::Anonymous, 0),
            )
            .await
            .expect("insert");

        assert_eq!(clone.count("demo").await.expect("count"), 1);
    }
}
