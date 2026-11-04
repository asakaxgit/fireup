//! Transaction tokens and read sets for `fireup local`.
//!
//! Firestore transactions are three RPCs: `BeginTransaction` hands out an
//! opaque token, reads and the final `Commit` carry it, and `Rollback` throws
//! it away. The client SDK buffers the writes, so the server's job is to
//! (a) hand out and validate tokens and (b) make the commit atomic *and* refuse
//! it when something the transaction read has changed underneath it.
//!
//! ## What a transaction here actually guarantees
//!
//! **Atomicity**, from [`Store::commit`](crate::local::store::Store::commit):
//! either every write in the commit applies or none does.
//!
//! **Optimistic concurrency**, from this module: every document read inside a
//! transaction is recorded with the `update_time` it had when it was read. The
//! commit replays those as [`Mutation::Verify`](crate::local::store::Mutation::Verify)
//! entries in the same store transaction as the writes, so a document that
//! someone else wrote in between fails the commit with `ABORTED` — which is
//! the status that makes an SDK retry the transaction rather than silently
//! lose the update.
//!
//! What it is **not** is snapshot isolation: a read inside a transaction sees
//! the current state, not the state as of `BeginTransaction`. Two reads of the
//! same document in one transaction can therefore return different values,
//! where real Firestore would return the first value twice. The conflict check
//! means such a transaction cannot *commit* on stale data, so the outcome is a
//! retry rather than a wrong write; it is recorded in the fidelity matrix in
//! `docs/local/README.md` all the same.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use prost_types::Timestamp;
use tonic::Status;

use crate::protos::google::firestore::v1::transaction_options::Mode;
use crate::protos::google::firestore::v1::{Document, TransactionOptions};

use crate::local::store::{Mutation, PendingWrite, Precondition};

/// How long an abandoned transaction is remembered.
///
/// Firestore's documented transaction lifetime is 60 seconds. Matching it keeps
/// a client that holds a token too long getting the same answer here as in
/// production, and stops a crashed client's tokens from accumulating forever.
pub const TRANSACTION_TTL: Duration = Duration::from_secs(60);

/// One live transaction.
struct Active {
    read_only: bool,
    started: Instant,
    /// Every document the transaction has read, and the `update_time` it had
    /// at the time — `None` for a document that did not exist, which is just
    /// as much a fact to be invalidated as a value is.
    read_set: HashMap<String, Option<Timestamp>>,
}

impl Active {
    fn expired(&self, now: Instant) -> bool {
        now.duration_since(self.started) >= TRANSACTION_TTL
    }
}

/// The live transactions of one server.
#[derive(Default)]
pub struct Transactions {
    active: Mutex<HashMap<Vec<u8>, Active>>,
}

impl Transactions {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Vec<u8>, Active>> {
        self.active.expect_lock()
    }

    /// Start a transaction and return its token.
    pub fn begin(&self, options: &Option<TransactionOptions>) -> Result<Vec<u8>, Status> {
        let read_only = match options.as_ref().and_then(|options| options.mode.as_ref()) {
            // No options at all means a read-write transaction, which is what
            // the field's default has to mean for a client that omits it.
            None => false,
            Some(Mode::ReadWrite(_)) => false,
            Some(Mode::ReadOnly(read_only)) => {
                if read_only.consistency_selector.is_some() {
                    return Err(Status::unimplemented(
                        "fireup local does not serve read-only transactions at a read_time yet \
                         (the store only keeps the current version of a document)",
                    ));
                }
                true
            }
        };
        self.begin_mode(read_only)
    }

    /// Start a transaction by mode alone — used by the Datastore surface,
    /// whose `TransactionOptions` message is a different proto type.
    pub fn begin_mode(&self, read_only: bool) -> Result<Vec<u8>, Status> {
        // Sweeping here rather than on a timer: tokens are only ever created
        // here, so this is the one place where the map can grow.
        let now = Instant::now();
        let mut active = self.lock();
        active.retain(|_, transaction| !transaction.expired(now));

        let token = uuid::Uuid::new_v4().as_bytes().to_vec();
        active.insert(
            token.clone(),
            Active {
                read_only,
                started: now,
                read_set: HashMap::new(),
            },
        );
        Ok(token)
    }

    /// Check that `token` names a live transaction.
    fn with<T>(
        &self,
        token: &[u8],
        apply: impl FnOnce(&mut Active) -> Result<T, Status>,
    ) -> Result<T, Status> {
        let mut active = self.lock();
        let transaction = active
            .get_mut(token)
            .ok_or_else(|| Status::invalid_argument("unknown or expired transaction"))?;

        if transaction.expired(Instant::now()) {
            active.remove(token);
            return Err(Status::aborted(format!(
                "the transaction expired after {} seconds",
                TRANSACTION_TTL.as_secs()
            )));
        }
        apply(transaction)
    }

    /// Check that `token` is usable for a read.
    pub fn check(&self, token: &[u8]) -> Result<(), Status> {
        self.with(token, |_| Ok(()))
    }

    /// Record that the transaction read `documents` (present or absent).
    ///
    /// `absent` carries the names that were looked up and not found, which have
    /// to be remembered too: a transaction that decided "this does not exist
    /// yet, so I will create it" is just as wrong if someone else creates it
    /// first.
    pub fn record_reads(
        &self,
        token: &[u8],
        documents: &[Document],
        absent: &[String],
    ) -> Result<(), Status> {
        self.with(token, |transaction| {
            for document in documents {
                transaction
                    .read_set
                    .insert(document.name.clone(), document.update_time.clone());
            }
            for name in absent {
                transaction.read_set.insert(name.clone(), None);
            }
            Ok(())
        })
    }

    /// Consume `token` and return the read-set checks its commit must pass.
    ///
    /// Returned as store writes so they go through the *same* store
    /// transaction as the client's writes; checking them separately first would
    /// leave a gap for a concurrent writer to slip into.
    pub fn finish(&self, token: &[u8], for_commit: bool) -> Result<Vec<PendingWrite>, Status> {
        let mut active = self.lock();
        let transaction = active
            .remove(token)
            .ok_or_else(|| Status::invalid_argument("unknown or expired transaction"))?;

        if transaction.expired(Instant::now()) {
            return Err(Status::aborted(format!(
                "the transaction expired after {} seconds",
                TRANSACTION_TTL.as_secs()
            )));
        }
        if for_commit && transaction.read_only {
            return Err(Status::invalid_argument(
                "a read-only transaction cannot be committed; roll it back instead",
            ));
        }

        Ok(transaction
            .read_set
            .into_iter()
            .map(|(name, update_time)| {
                let precondition = match update_time {
                    Some(update_time) => Precondition::UpdateTime(update_time),
                    // Read as absent, so it must still be absent.
                    None => Precondition::Exists(false),
                };
                PendingWrite::guarded(Mutation::Verify { name }, Some(precondition))
            })
            .collect())
    }

    /// How many transactions are currently live. Used by tests and the health
    /// surface; not on a request path.
    pub fn live(&self) -> usize {
        self.lock().len()
    }
}

/// `Mutex::lock` without the poisoning boilerplate at every call site.
trait ExpectLock<T> {
    fn expect_lock(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> ExpectLock<T> for Mutex<T> {
    fn expect_lock(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().expect("transaction registry lock poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protos::google::firestore::v1::transaction_options::{ReadOnly, ReadWrite};

    fn document(name: &str, update_time: Option<Timestamp>) -> Document {
        Document {
            name: name.to_string(),
            update_time,
            ..Default::default()
        }
    }

    fn ts(seconds: i64) -> Timestamp {
        Timestamp { seconds, nanos: 0 }
    }

    #[test]
    fn a_token_is_opaque_and_unique() {
        let transactions = Transactions::new();
        let first = transactions.begin(&None).expect("begin");
        let second = transactions.begin(&None).expect("begin");

        assert_ne!(first, second);
        assert!(!first.is_empty(), "a token must carry something");
        assert_eq!(transactions.live(), 2);
    }

    #[test]
    fn an_unknown_token_is_rejected_rather_than_ignored() {
        let transactions = Transactions::new();
        let status = transactions.check(b"not-a-token").expect_err("unknown token");
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn finishing_a_transaction_consumes_its_token() {
        let transactions = Transactions::new();
        let token = transactions.begin(&None).expect("begin");

        transactions.finish(&token, true).expect("first finish");
        assert_eq!(transactions.live(), 0);
        assert!(
            transactions.finish(&token, true).is_err(),
            "a token must not work twice"
        );
    }

    #[test]
    fn the_read_set_becomes_commit_preconditions() {
        let transactions = Transactions::new();
        let token = transactions.begin(&None).expect("begin");

        transactions
            .record_reads(
                &token,
                &[document("a", Some(ts(10)))],
                &["b".to_string()],
            )
            .expect("record");

        let checks = transactions.finish(&token, true).expect("finish");
        let mut found: Vec<(String, Option<Precondition>)> = checks
            .into_iter()
            .map(|write| (write.mutation.name().to_string(), write.precondition))
            .collect();
        found.sort_by(|left, right| left.0.cmp(&right.0));

        assert_eq!(
            found,
            vec![
                ("a".to_string(), Some(Precondition::UpdateTime(ts(10)))),
                // Read as absent, so it has to still be absent at commit.
                ("b".to_string(), Some(Precondition::Exists(false))),
            ]
        );
    }

    #[test]
    fn a_read_only_transaction_cannot_be_committed() {
        let transactions = Transactions::new();
        let options = Some(TransactionOptions {
            mode: Some(Mode::ReadOnly(ReadOnly {
                consistency_selector: None,
            })),
        });
        let token = transactions.begin(&options).expect("begin");

        let status = transactions
            .finish(&token, true)
            .expect_err("a read-only transaction has nothing to commit");
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn a_read_write_transaction_commits() {
        let transactions = Transactions::new();
        let options = Some(TransactionOptions {
            mode: Some(Mode::ReadWrite(ReadWrite {
                retry_transaction: Vec::new(),
                ..Default::default()
            })),
        });
        let token = transactions.begin(&options).expect("begin");

        assert!(transactions.finish(&token, true).expect("finish").is_empty());
    }

    #[test]
    fn an_expired_transaction_is_aborted_and_forgotten() {
        let transactions = Transactions::new();
        let token = transactions.begin(&None).expect("begin");

        // Age the transaction past its TTL without sleeping for a minute.
        transactions
            .lock()
            .get_mut(&token)
            .expect("live")
            .started -= TRANSACTION_TTL;

        let status = transactions.check(&token).expect_err("expired");
        assert_eq!(status.code(), tonic::Code::Aborted);
        assert_eq!(transactions.live(), 0, "an expired token is dropped");
    }
}
