//! Datastore v1 gRPC service for `fireup local`.
//!
//! Implemented against the public `google.datastore.v1.Datastore` service
//! definition vendored under `proto/`. Phase 3 serves the subset official
//! client SDKs need for entity CRUD, kind queries, transactions, and id
//! allocation, mapped onto the same [`Store`](crate::local::store::Store) the
//! Firestore surface uses:
//!
//! | RPC                  | Status |
//! |----------------------|--------|
//! | `Lookup`             | served |
//! | `RunQuery`           | served (kind queries; GQL refused) |
//! | `Commit`             | served (incl. `property_transforms`) |
//! | `BeginTransaction`   | served |
//! | `Rollback`           | served |
//! | `AllocateIds`        | served |
//! | `ReserveIds`         | served (acknowledged; ids are not reused) |
//! | `RunAggregationQuery`| `UNIMPLEMENTED` |
//!
//! `RunQuery`'s property filters are translated into the same compiled
//! [`Predicate`](crate::local::query::Predicate) the Firestore surface
//! compiles, so a kind query narrows in the store rather than reading the whole
//! kind — and so that one comparator decides what a value comparison means on
//! both surfaces. See [`datastore_filter`](crate::local::datastore_filter) for
//! what translates and why that unification was load-bearing rather than tidy.
//!
//! This is an independent implementation of a published API contract. No
//! Firebase Emulator Suite code or behaviour was reverse-engineered; see
//! `.cursor/rules/firebase-gcp-compliance.mdc`.

use std::cmp::Ordering;
use std::sync::Arc;

use prost_types::Timestamp;
use tonic::{Request, Response, Status};
use tracing::debug;

use crate::local::datastore_filter;
use crate::local::entity_map::{
    ancestor_collection, apply_property_mask, check_project_database, compile_property_transforms,
    complete_key, document_to_entity, documents_root, ds_value_to_fs, entity_to_document,
    key_is_incomplete, key_to_document_name, masked_entity_fields, normalize_database_id,
    transform_results_to_properties,
};
use crate::local::query::{self, Prefilter};
use crate::local::store::{
    Mutation as StoreMutation, PendingWrite, Precondition, SharedStore, WriteOutcome,
};
use crate::local::transactions::Transactions;
use crate::protos::google::datastore::v1::commit_request::{
    Mode as CommitMode, TransactionSelector,
};
use crate::protos::google::datastore::v1::datastore_server::Datastore;
use crate::protos::google::datastore::v1::entity_result::ResultType;
use crate::protos::google::datastore::v1::mutation::Operation as MutationOp;
use crate::protos::google::datastore::v1::property_filter::Operator as PropertyOp;
use crate::protos::google::datastore::v1::property_order::Direction;
use crate::protos::google::datastore::v1::query_result_batch::MoreResultsType;
use crate::protos::google::datastore::v1::read_options::ConsistencyType;
use crate::protos::google::datastore::v1::transaction_options::Mode as TxMode;
use crate::protos::google::datastore::v1::value::ValueType as DsValueType;
use crate::protos::google::datastore::v1::{
    filter::FilterType, AllocateIdsRequest, AllocateIdsResponse, BeginTransactionRequest,
    BeginTransactionResponse, CommitRequest, CommitResponse, Entity, EntityResult, Filter,
    Key, LookupRequest, LookupResponse, Mutation, MutationResult, PropertyFilter, PropertyMask,
    PropertyOrder, PropertyReference, Query, QueryResultBatch, ReadOptions, ReserveIdsRequest,
    ReserveIdsResponse, RollbackRequest, RollbackResponse, RunAggregationQueryRequest,
    RunAggregationQueryResponse, RunQueryRequest, RunQueryResponse, Value as DsValue,
};
use crate::protos::google::firestore::v1::Document;

fn unimplemented(rpc: &str, plan: &str) -> Status {
    Status::unimplemented(format!(
        "fireup local does not serve {rpc} yet ({plan})"
    ))
}

fn now_timestamp() -> Timestamp {
    let now = chrono::Utc::now();
    Timestamp {
        seconds: now.timestamp(),
        nanos: now.timestamp_subsec_nanos() as i32,
    }
}

/// The Datastore data plane for `fireup local`.
#[derive(Clone)]
pub struct DatastoreApi {
    store: SharedStore,
    project_id: String,
    database_id: String,
    transactions: Arc<Transactions>,
}

impl DatastoreApi {
    pub fn new(store: SharedStore, project_id: impl Into<String>, database_id: impl Into<String>) -> Self {
        Self {
            store,
            project_id: project_id.into(),
            database_id: normalize_database_id(&database_id.into()).to_string(),
            transactions: Arc::new(Transactions::new()),
        }
    }

    fn check_scope(&self, project_id: &str, database_id: &str) -> Result<(), Status> {
        check_project_database(project_id, database_id, &self.project_id, &self.database_id)
    }

    fn root(&self) -> String {
        documents_root(&self.project_id, &self.database_id)
    }

    async fn lookup_one(
        &self,
        key: &Key,
        transaction: Option<&[u8]>,
    ) -> Result<(Option<Document>, String), Status> {
        let name = key_to_document_name(key, &self.project_id, &self.database_id)?;
        let document = self.store.get(&name).await?;
        if let Some(token) = transaction {
            match &document {
                Some(document) => self.transactions.record_reads(token, &[document.clone()], &[])?,
                None => self.transactions.record_reads(token, &[], &[name.clone()])?,
            }
        }
        Ok((document, name))
    }

    async fn entity_from_document(&self, document: &Document) -> Result<Entity, Status> {
        document_to_entity(document, &self.project_id, &self.database_id)
    }

    fn begin_from_options(
        &self,
        options: &Option<crate::protos::google::datastore::v1::TransactionOptions>,
    ) -> Result<Vec<u8>, Status> {
        let read_only = match options.as_ref().and_then(|options| options.mode.as_ref()) {
            None | Some(TxMode::ReadWrite(_)) => false,
            Some(TxMode::ReadOnly(read_only)) => {
                if read_only.read_time.is_some() {
                    return Err(unimplemented(
                        "read-only Datastore transactions at a read_time",
                        "the store only keeps the current version of an entity",
                    ));
                }
                true
            }
        };
        self.transactions.begin_mode(read_only)
    }

    fn take_read_transaction<'a>(
        &'a self,
        read_options: &'a Option<ReadOptions>,
    ) -> Result<(Option<&'a [u8]>, Option<Vec<u8>>), Status> {
        let Some(options) = read_options else {
            return Ok((None, None));
        };
        match options.consistency_type.as_ref() {
            None => Ok((None, None)),
            Some(ConsistencyType::ReadConsistency(_)) => Ok((None, None)),
            Some(ConsistencyType::Transaction(token)) => {
                self.transactions.check(token)?;
                Ok((Some(token.as_slice()), None))
            }
            Some(ConsistencyType::NewTransaction(options)) => {
                let token = self.begin_from_options(&Some(options.clone()))?;
                Ok((None, Some(token)))
            }
            Some(ConsistencyType::ReadTime(_)) => Err(unimplemented(
                "point-in-time Datastore reads",
                "the store only keeps the current version of an entity",
            )),
        }
    }

    async fn run_structured_query(
        &self,
        query: &Query,
        partition_ancestor: Option<&Key>,
        transaction: Option<&[u8]>,
    ) -> Result<QueryResultBatch, Status> {
        if !query.projection.is_empty() {
            return Err(unimplemented(
                "Datastore projection queries",
                "projections land with a later query pass",
            ));
        }
        if !query.distinct_on.is_empty() {
            return Err(unimplemented("Datastore distinct_on", "not in Phase 3"));
        }
        if !query.start_cursor.is_empty() || !query.end_cursor.is_empty() {
            return Err(unimplemented(
                "Datastore query cursors",
                "use offset/limit for Phase 3",
            ));
        }
        if query.find_nearest.is_some() {
            return Err(unimplemented("Datastore find_nearest", "vector search"));
        }
        if query.kind.len() > 1 {
            return Err(Status::invalid_argument(
                "a Datastore query may name at most one kind",
            ));
        }

        let kind = query
            .kind
            .first()
            .map(|kind| kind.name.as_str())
            .filter(|name| !name.is_empty());

        let ancestor = extract_ancestor(query.filter.as_ref())?;
        let ancestor = match (ancestor.as_ref(), partition_ancestor) {
            (Some(from_filter), Some(from_partition)) => {
                let a = key_to_document_name(from_filter, &self.project_id, &self.database_id)?;
                let b = key_to_document_name(from_partition, &self.project_id, &self.database_id)?;
                if a != b {
                    return Err(Status::invalid_argument(
                        "partition ancestor and __key__ HAS_ANCESTOR do not match",
                    ));
                }
                Some(from_filter.clone())
            }
            (Some(key), None) | (None, Some(key)) => Some(key.clone()),
            (None, None) => None,
        };

        // Compiled before the read, because it is what decides how wide the
        // read is. See `datastore_filter` for why the translated predicate is
        // then also what filters in process when the translation is exact.
        let property_filter = strip_ancestor_filter(query.filter.as_ref())?;
        let translated = datastore_filter::translate(
            property_filter.as_ref(),
            &self.project_id,
            &self.database_id,
        );
        let prefilter = Prefilter {
            filter: translated.as_ref(),
            // Empty on purpose, and not because Datastore queries have no
            // `order`. Firestore excludes a document that has no value for a
            // sort field, which is what lets its pre-filter demand the field be
            // present; `compare_entities` instead *orders* an entity missing
            // the property first. Copying the Firestore spelling here would
            // drop those entities from the answer.
            required_fields: &[],
        };

        let mut candidates = match (kind, ancestor.as_ref()) {
            (Some(kind), Some(ancestor)) => {
                let (parent, collection) =
                    ancestor_collection(ancestor, kind, &self.project_id, &self.database_id)?;
                self.store
                    .scan_collection_filtered(&parent, &collection, prefilter)
                    .await?
            }
            (Some(kind), None) => {
                self.store
                    .scan_kind_filtered(&self.root(), kind, prefilter)
                    .await?
            }
            (None, Some(_ancestor)) => {
                // No kind: every direct child collection under the ancestor.
                // Phase 3 refuses the unbounded form rather than scan the world.
                return Err(unimplemented(
                    "kindless Datastore queries",
                    "name a kind; kindless scans are not served yet",
                ));
            }
            (None, None) => {
                return Err(unimplemented(
                    "kindless Datastore queries",
                    "name a kind; kindless scans are not served yet",
                ));
            }
        };

        // The pre-filter narrowed the read; it does not decide the answer. The
        // Datastore evaluator still has the only vote, exactly as before this
        // translation existed — which is what leaves the translation an
        // independent reference to be wrong against, and so something
        // `tests/local_datastore_pushdown_tests.rs` can catch.
        if let Some(filter) = property_filter.as_ref() {
            candidates.retain(|document| {
                match document_to_entity(document, &self.project_id, &self.database_id) {
                    Ok(entity) => {
                        entity_matches(&entity, filter, &self.project_id, &self.database_id)
                    }
                    Err(_) => false,
                }
            });
        }

        if !query.order.is_empty() {
            candidates.sort_by(|left, right| {
                let left = document_to_entity(left, &self.project_id, &self.database_id).ok();
                let right = document_to_entity(right, &self.project_id, &self.database_id).ok();
                match (left, right) {
                    (Some(left), Some(right)) => compare_entities(
                        &left,
                        &right,
                        &query.order,
                        &self.project_id,
                        &self.database_id,
                    ),
                    _ => Ordering::Equal,
                }
            });
        }

        let offset = query.offset.max(0) as usize;
        if offset > candidates.len() {
            candidates.clear();
        } else {
            candidates = candidates.split_off(offset);
        }

        if let Some(limit) = query.limit {
            let limit = limit.max(0) as usize;
            if candidates.len() > limit {
                candidates.truncate(limit);
            }
        }

        if let Some(token) = transaction {
            self.transactions.record_reads(token, &candidates, &[])?;
        }

        let mut entity_results = Vec::with_capacity(candidates.len());
        for document in candidates {
            let entity = self.entity_from_document(&document).await?;
            entity_results.push(EntityResult {
                entity: Some(entity),
                version: document
                    .update_time
                    .as_ref()
                    .map(|ts| ts.seconds)
                    .unwrap_or(0),
                create_time: document.create_time.clone(),
                update_time: document.update_time.clone(),
                cursor: Vec::new(),
            });
        }

        Ok(QueryResultBatch {
            entity_result_type: ResultType::Full as i32,
            entity_results,
            skipped_cursor: Vec::new(),
            more_results: MoreResultsType::NoMoreResults as i32,
            end_cursor: Vec::new(),
            skipped_results: query.offset.max(0),
            snapshot_version: 0,
            read_time: Some(now_timestamp()),
        })
    }

    /// The store mutation that writes `entity`, masked by `property_mask` if the
    /// request carried one.
    ///
    /// Without a mask the entity replaces whatever is stored, which is what
    /// Datastore means by `update`/`upsert`: an unmasked write is the whole
    /// entity. With a mask only the named properties are touched, which is the
    /// shape that makes `property_transforms` useful — the transforms run after
    /// the operation, so a mask that leaves a counter out of the write lets the
    /// transform read the stored value instead of the one the client happened to
    /// submit. That is the same pairing as Firestore's `update_mask` plus
    /// `update_transforms`, and it goes through the same store primitive.
    fn entity_write(
        &self,
        entity: &Entity,
        property_mask: Option<&PropertyMask>,
    ) -> Result<StoreMutation, Status> {
        let document = entity_to_document(entity, &self.project_id, &self.database_id)?;
        match property_mask {
            None => Ok(StoreMutation::Put(document)),
            Some(mask) => Ok(StoreMutation::Merge {
                name: document.name,
                fields: masked_entity_fields(
                    entity,
                    &mask.paths,
                    &self.project_id,
                    &self.database_id,
                )?,
            }),
        }
    }

    async fn mutation_to_write(
        &self,
        mutation: &Mutation,
    ) -> Result<(PendingWrite, Option<Key>), Status> {
        // Compiled before anything touches the store, so a batch carrying one
        // transform we cannot serve fails whole instead of applying half.
        let transforms = compile_property_transforms(
            &mutation.property_transforms,
            &self.project_id,
            &self.database_id,
        )?;
        let conflict_precondition = match mutation.conflict_detection_strategy.as_ref() {
            None => None,
            Some(
                crate::protos::google::datastore::v1::mutation::ConflictDetectionStrategy::BaseVersion(
                    _,
                ),
            ) => {
                return Err(unimplemented(
                    "Datastore base_version conflict detection",
                    "use update_time or omit conflict detection",
                ));
            }
            Some(
                crate::protos::google::datastore::v1::mutation::ConflictDetectionStrategy::UpdateTime(
                    update_time,
                ),
            ) => Some(Precondition::UpdateTime(update_time.clone())),
        };

        let operation = mutation
            .operation
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("mutation has no operation"))?;

        match operation {
            MutationOp::Insert(entity) | MutationOp::Upsert(entity) => {
                let mut entity = entity.clone();
                let mut key = entity
                    .key
                    .clone()
                    .ok_or_else(|| Status::invalid_argument("mutation entity is missing a key"))?;
                let allocated = if key_is_incomplete(&key) {
                    key = complete_key(key)?;
                    entity.key = Some(key.clone());
                    Some(key.clone())
                } else {
                    None
                };
                let store_mutation = self.entity_write(&entity, mutation.property_mask.as_ref())?;
                let precondition = match (operation, conflict_precondition) {
                    (MutationOp::Insert(_), None) => Some(Precondition::Exists(false)),
                    (_, precondition) => precondition,
                };
                Ok((
                    PendingWrite::transformed(store_mutation, precondition, transforms),
                    allocated,
                ))
            }
            MutationOp::Update(entity) => {
                if entity.key.as_ref().is_some_and(key_is_incomplete) {
                    return Err(Status::invalid_argument(
                        "update mutations require a complete key",
                    ));
                }
                let store_mutation = self.entity_write(entity, mutation.property_mask.as_ref())?;
                let precondition = conflict_precondition.or(Some(Precondition::Exists(true)));
                Ok((
                    PendingWrite::transformed(store_mutation, precondition, transforms),
                    None,
                ))
            }
            MutationOp::Delete(key) => {
                if key_is_incomplete(key) {
                    return Err(Status::invalid_argument(
                        "delete mutations require a complete key",
                    ));
                }
                if !transforms.is_empty() {
                    // The proto: property_transforms "can be set only when the
                    // operation is `insert`, `update`, or `upsert`".
                    return Err(Status::invalid_argument(
                        "property_transforms may not be set on a delete mutation",
                    ));
                }
                // `property_mask` is deliberately not consulted here: the proto
                // says the field "is ignored for `delete`", so a client that
                // sets one still gets the whole entity removed rather than an
                // error or a partial delete.
                let name = key_to_document_name(key, &self.project_id, &self.database_id)?;
                Ok((
                    PendingWrite::guarded(
                        StoreMutation::Delete { name },
                        conflict_precondition,
                    ),
                    None,
                ))
            }
        }
    }
}

#[tonic::async_trait]
impl Datastore for DatastoreApi {
    async fn lookup(
        &self,
        request: Request<LookupRequest>,
    ) -> Result<Response<LookupResponse>, Status> {
        let request = request.into_inner();
        debug!(keys = request.keys.len(), "Datastore.Lookup");
        self.check_scope(&request.project_id, &request.database_id)?;

        let (existing_tx, new_tx) = self.take_read_transaction(&request.read_options)?;
        let transaction = match (&existing_tx, &new_tx) {
            (_, Some(token)) => Some(token.as_slice()),
            (Some(token), None) => Some(*token),
            (None, None) => None,
        };

        let mut found = Vec::new();
        let mut missing = Vec::new();
        for key in &request.keys {
            let (document, _name) = self.lookup_one(key, transaction).await?;
            match document {
                Some(document) => {
                    let mut entity = self.entity_from_document(&document).await?;
                    if let Some(mask) = &request.property_mask {
                        apply_property_mask(&mut entity, &mask.paths)?;
                    }
                    found.push(EntityResult {
                        entity: Some(entity),
                        version: document
                            .update_time
                            .as_ref()
                            .map(|ts| ts.seconds)
                            .unwrap_or(0),
                        create_time: document.create_time.clone(),
                        update_time: document.update_time.clone(),
                        cursor: Vec::new(),
                    });
                }
                None => {
                    missing.push(EntityResult {
                        entity: Some(Entity {
                            key: Some(key.clone()),
                            properties: Default::default(),
                        }),
                        version: 0,
                        create_time: None,
                        update_time: None,
                        cursor: Vec::new(),
                    });
                }
            }
        }

        Ok(Response::new(LookupResponse {
            found,
            missing,
            deferred: Vec::new(),
            transaction: new_tx.unwrap_or_default(),
            read_time: Some(now_timestamp()),
        }))
    }

    async fn run_query(
        &self,
        request: Request<RunQueryRequest>,
    ) -> Result<Response<RunQueryResponse>, Status> {
        let request = request.into_inner();
        debug!("Datastore.RunQuery");
        self.check_scope(&request.project_id, &request.database_id)?;

        if request.explain_options.is_some() {
            return Err(unimplemented("Datastore explain_options", "not in Phase 3"));
        }

        let (existing_tx, new_tx) = self.take_read_transaction(&request.read_options)?;
        let transaction = match (&existing_tx, &new_tx) {
            (_, Some(token)) => Some(token.as_slice()),
            (Some(token), None) => Some(*token),
            (None, None) => None,
        };

        let query = match request.query_type {
            Some(crate::protos::google::datastore::v1::run_query_request::QueryType::Query(query)) => {
                query
            }
            Some(
                crate::protos::google::datastore::v1::run_query_request::QueryType::GqlQuery(_),
            ) => {
                return Err(unimplemented(
                    "Datastore GQL",
                    "pass a structured Query; GQL is not served yet",
                ));
            }
            None => return Err(Status::invalid_argument("RunQuery requires a query")),
        };

        // PartitionId on the request scopes the query; its namespace is refused
        // inside key helpers when an ancestor key carries one.
        if let Some(partition) = &request.partition_id {
            if !partition.namespace_id.is_empty() {
                return Err(unimplemented(
                    "Datastore namespaces",
                    "queries stay in the default namespace only",
                ));
            }
        }

        let batch = self
            .run_structured_query(&query, None, transaction)
            .await?;

        Ok(Response::new(RunQueryResponse {
            batch: Some(batch),
            query: None,
            transaction: new_tx.unwrap_or_default(),
            explain_metrics: None,
        }))
    }

    async fn run_aggregation_query(
        &self,
        _request: Request<RunAggregationQueryRequest>,
    ) -> Result<Response<RunAggregationQueryResponse>, Status> {
        Err(unimplemented(
            "RunAggregationQuery",
            "aggregations land after Phase 3",
        ))
    }

    async fn begin_transaction(
        &self,
        request: Request<BeginTransactionRequest>,
    ) -> Result<Response<BeginTransactionResponse>, Status> {
        let request = request.into_inner();
        debug!("Datastore.BeginTransaction");
        self.check_scope(&request.project_id, &request.database_id)?;
        let transaction = self.begin_from_options(&request.transaction_options)?;
        Ok(Response::new(BeginTransactionResponse { transaction }))
    }

    async fn commit(
        &self,
        request: Request<CommitRequest>,
    ) -> Result<Response<CommitResponse>, Status> {
        let request = request.into_inner();
        debug!(mutations = request.mutations.len(), "Datastore.Commit");
        self.check_scope(&request.project_id, &request.database_id)?;

        let mode = CommitMode::try_from(request.mode).unwrap_or(CommitMode::Unspecified);
        let read_set_checks = match (mode, request.transaction_selector.as_ref()) {
            (_, Some(TransactionSelector::Transaction(token))) => {
                self.transactions.finish(token, true)?
            }
            (_, Some(TransactionSelector::SingleUseTransaction(options))) => {
                // Begin+commit in one RPC: spend a fresh token with an empty
                // read set so it cannot leak into the registry.
                let token = self.begin_from_options(&Some(options.clone()))?;
                self.transactions.finish(&token, true)?
            }
            (CommitMode::NonTransactional, None)
            | (CommitMode::Unspecified, None)
            | (CommitMode::Transactional, None) => {
                // A missing transaction selector is treated as a plain batch —
                // what the Node SDK sends for `save()` / `delete()`.
                Vec::new()
            }
        };

        let mut allocated_keys: Vec<Option<Key>> = Vec::with_capacity(request.mutations.len());
        let mut writes = read_set_checks;
        for mutation in &request.mutations {
            let (write, allocated) = self.mutation_to_write(mutation).await?;
            allocated_keys.push(allocated);
            writes.push(write);
        }

        let now = now_timestamp();
        let outcomes = self.store.commit(writes, now.clone()).await?;
        // Outcomes include any read-set verifies first.
        let verify_count = outcomes
            .iter()
            .take_while(|effect| matches!(effect.outcome, WriteOutcome::Verified))
            .count();
        let mutation_outcomes = &outcomes[verify_count..];

        let mut mutation_results = Vec::with_capacity(request.mutations.len());
        for (index, _mutation) in request.mutations.iter().enumerate() {
            let effect = mutation_outcomes.get(index);
            let key = allocated_keys[index].clone();
            let (create_time, update_time) = match effect.map(|effect| &effect.outcome) {
                Some(WriteOutcome::Deleted { .. }) | None => (None, None),
                Some(_) => (Some(now.clone()), Some(now.clone())),
            };
            // One value per transform on this mutation, in request order, as the
            // store resolved them inside the write's own transaction.
            let transform_results = match effect {
                Some(effect) => transform_results_to_properties(
                    &effect.transform_results,
                    &self.project_id,
                    &self.database_id,
                )?,
                None => Vec::new(),
            };
            mutation_results.push(MutationResult {
                key,
                version: now.seconds,
                create_time,
                update_time,
                transform_results,
                conflict_detected: false,
            });
        }

        Ok(Response::new(CommitResponse {
            mutation_results,
            index_updates: 0,
            commit_time: Some(now),
        }))
    }

    async fn rollback(
        &self,
        request: Request<RollbackRequest>,
    ) -> Result<Response<RollbackResponse>, Status> {
        let request = request.into_inner();
        debug!("Datastore.Rollback");
        self.check_scope(&request.project_id, &request.database_id)?;
        self.transactions.finish(&request.transaction, false)?;
        Ok(Response::new(RollbackResponse {}))
    }

    async fn allocate_ids(
        &self,
        request: Request<AllocateIdsRequest>,
    ) -> Result<Response<AllocateIdsResponse>, Status> {
        let request = request.into_inner();
        debug!(keys = request.keys.len(), "Datastore.AllocateIds");
        self.check_scope(&request.project_id, &request.database_id)?;

        let mut keys = Vec::with_capacity(request.keys.len());
        for key in request.keys {
            if !key_is_incomplete(&key) {
                return Err(Status::invalid_argument(
                    "AllocateIds requires incomplete keys (final path element with no id/name)",
                ));
            }
            keys.push(complete_key(key)?);
        }
        Ok(Response::new(AllocateIdsResponse { keys }))
    }

    async fn reserve_ids(
        &self,
        request: Request<ReserveIdsRequest>,
    ) -> Result<Response<ReserveIdsResponse>, Status> {
        let request = request.into_inner();
        debug!(keys = request.keys.len(), "Datastore.ReserveIds");
        self.check_scope(&request.project_id, &request.database_id)?;
        // Fireup allocates from a monotonic counter that never hands out a
        // previously completed key's numeric id on purpose; acknowledging the
        // reservation is enough for local use. Production-style reservation
        // tracking is not implemented.
        for key in &request.keys {
            if key_is_incomplete(key) {
                return Err(Status::invalid_argument(
                    "ReserveIds requires complete keys",
                ));
            }
            let _ = key_to_document_name(key, &self.project_id, &self.database_id)?;
        }
        Ok(Response::new(ReserveIdsResponse {}))
    }
}

fn extract_ancestor(filter: Option<&Filter>) -> Result<Option<Key>, Status> {
    let Some(filter) = filter else {
        return Ok(None);
    };
    match filter.filter_type.as_ref() {
        Some(FilterType::PropertyFilter(property))
            if property.op == PropertyOp::HasAncestor as i32 =>
        {
            match property.value.as_ref().and_then(|value| value.value_type.as_ref()) {
                Some(DsValueType::KeyValue(key)) => Ok(Some(key.clone())),
                _ => Err(Status::invalid_argument(
                    "HAS_ANCESTOR filter requires a key value",
                )),
            }
        }
        Some(FilterType::CompositeFilter(composite)) => {
            let mut found = None;
            for child in &composite.filters {
                if let Some(key) = extract_ancestor(Some(child))? {
                    if found.is_some() {
                        return Err(Status::invalid_argument(
                            "a query may have at most one HAS_ANCESTOR filter",
                        ));
                    }
                    found = Some(key);
                }
            }
            Ok(found)
        }
        _ => Ok(None),
    }
}

fn strip_ancestor_filter(filter: Option<&Filter>) -> Result<Option<Filter>, Status> {
    let Some(filter) = filter else {
        return Ok(None);
    };
    match filter.filter_type.as_ref() {
        Some(FilterType::PropertyFilter(property))
            if property.op == PropertyOp::HasAncestor as i32 =>
        {
            Ok(None)
        }
        Some(FilterType::CompositeFilter(composite)) => {
            let mut kept = Vec::new();
            for child in &composite.filters {
                if let Some(child) = strip_ancestor_filter(Some(child))? {
                    kept.push(child);
                }
            }
            match kept.len() {
                0 => Ok(None),
                1 => Ok(Some(kept.remove(0))),
                _ => Ok(Some(Filter {
                    filter_type: Some(FilterType::CompositeFilter(
                        crate::protos::google::datastore::v1::CompositeFilter {
                            op: composite.op,
                            filters: kept,
                        },
                    )),
                })),
            }
        }
        _ => Ok(Some(filter.clone())),
    }
}

pub(crate) fn entity_matches(
    entity: &Entity,
    filter: &Filter,
    project_id: &str,
    database_id: &str,
) -> bool {
    match filter.filter_type.as_ref() {
        Some(FilterType::CompositeFilter(composite)) => {
            // Only AND is generated by the public proto's Operator for
            // CompositeFilter in common SDK use; OR is refused at compile time
            // of the query by returning false for unknown ops.
            let op = crate::protos::google::datastore::v1::composite_filter::Operator::try_from(
                composite.op,
            );
            match op {
                Ok(crate::protos::google::datastore::v1::composite_filter::Operator::And) => {
                    composite
                        .filters
                        .iter()
                        .all(|child| entity_matches(entity, child, project_id, database_id))
                }
                _ => false,
            }
        }
        Some(FilterType::PropertyFilter(property)) => {
            property_matches(entity, property, project_id, database_id)
        }
        None => true,
    }
}

pub(crate) fn property_matches(
    entity: &Entity,
    filter: &PropertyFilter,
    project_id: &str,
    database_id: &str,
) -> bool {
    let Some(reference) = filter.property.as_ref() else {
        return false;
    };
    if reference.name.contains('.') {
        return false;
    }
    let Some(expected) = filter.value.as_ref() else {
        return false;
    };
    if reference.name == "__key__" {
        return false; // HAS_ANCESTOR already stripped; other key filters later.
    }
    let actual = match entity.properties.get(&reference.name) {
        Some(value) => value,
        None => return false,
    };

    // Both sides cross into the Firestore values the store actually holds, for
    // the same reason `compile_property_transforms` converts its operands: a
    // `key_value` is stored as a reference string, so comparing the
    // unconverted key would never match. A conversion that fails — a key
    // outside this database — is no match rather than an error, because a
    // filter is a question about the entity, not a write.
    let (Ok(actual), Ok(expected)) = (
        ds_value_to_fs(actual, project_id, database_id),
        ds_value_to_fs(expected, project_id, database_id),
    ) else {
        return false;
    };

    // `None` is "a filter cannot compare these two at all" — different types,
    // or a `NaN` on either side — and it is no match for *every* operator,
    // including equality. The comparator this replaced instead returned
    // `Ordering::Equal` for every type pair it did not name, so `priority = 3`
    // matched an entity whose `priority` was a string, a blob, or a list, and
    // so did `<=` and `>=`. Real Datastore returns none of those.
    let Some(ord) = query::comparable(&actual, &expected) else {
        return false;
    };
    match PropertyOp::try_from(filter.op) {
        Ok(PropertyOp::Equal) => ord == Ordering::Equal,
        Ok(PropertyOp::LessThan) => ord == Ordering::Less,
        Ok(PropertyOp::LessThanOrEqual) => {
            matches!(ord, Ordering::Less | Ordering::Equal)
        }
        Ok(PropertyOp::GreaterThan) => ord == Ordering::Greater,
        Ok(PropertyOp::GreaterThanOrEqual) => {
            matches!(ord, Ordering::Greater | Ordering::Equal)
        }
        _ => false,
    }
}

fn compare_entities(
    left: &Entity,
    right: &Entity,
    order: &[PropertyOrder],
    project_id: &str,
    database_id: &str,
) -> Ordering {
    for term in order {
        let Some(PropertyReference { name }) = term.property.as_ref() else {
            continue;
        };
        let left_value = left.properties.get(name);
        let right_value = right.properties.get(name);
        let ord = match (left_value, right_value) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some(left), Some(right)) => {
                compare_ds_values(left, right, project_id, database_id)
            }
        };
        if ord != Ordering::Equal {
            return if term.direction == Direction::Descending as i32 {
                ord.reverse()
            } else {
                ord
            };
        }
    }
    Ordering::Equal
}

/// Order two Datastore property values for a *sort*.
///
/// Delegates to the store's own total value order, via the conversion the
/// properties are stored under. The hand-rolled version this replaced named
/// eight type pairs and returned `Ordering::Equal` for every other — so two
/// entities whose sort property had different types compared equal and came
/// back in whatever order the scan happened to produce.
///
/// A value that cannot convert compares equal to everything, which is what the
/// old fall-through did for it too; there is no better answer available inside
/// a sort comparator, and nothing that reaches here has failed to convert.
fn compare_ds_values(
    left: &DsValue,
    right: &DsValue,
    project_id: &str,
    database_id: &str,
) -> Ordering {
    match (
        ds_value_to_fs(left, project_id, database_id),
        ds_value_to_fs(right, project_id, database_id),
    ) {
        (Ok(left), Ok(right)) => query::total_order(&left, &right),
        _ => Ordering::Equal,
    }
}
