//! Firestore v1 gRPC service for `fireup local`.
//!
//! Implemented against the public `google.firestore.v1.Firestore` service
//! definition vendored under `proto/`. Phase 2 serves the whole v1 surface an
//! official client SDK uses for document work, queries, and transactions,
//! against whichever [`Store`](crate::local::store::Store) the process was
//! configured with:
//!
//! | RPC                  | Status |
//! |----------------------|--------|
//! | `GetDocument`        | served |
//! | `CreateDocument`     | served |
//! | `UpdateDocument`     | served |
//! | `DeleteDocument`     | served |
//! | `BatchGetDocuments`  | served (gRPC SDK read path) |
//! | `Commit`             | served, atomically |
//! | `RunQuery`           | served, see [`query`](crate::local::query) |
//! | `ListDocuments`      | served, incl. `show_missing` |
//! | `ListCollectionIds`  | served |
//! | everything else      | `UNIMPLEMENTED` |
//!
//! Every unimplemented RPC answers with an explicit `UNIMPLEMENTED` status and a
//! message naming the phase that will add it, so an SDK failure is immediately
//! legible instead of looking like a hang or a protocol error.
//!
//! This is an independent implementation of a published API contract. No
//! Firebase Emulator Suite code or behaviour was reverse-engineered; see
//! `.cursor/rules/firebase-gcp-compliance.mdc`.

use std::sync::Arc;

use prost_types::Timestamp;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::debug;

use crate::local::names::{parse_document_name, parse_parent_name};
use crate::local::query::Query;
use crate::local::store::{self, now_timestamp, Mutation, PendingWrite, SharedStore};
use crate::local::transactions::Transactions;
use crate::protos::google::firestore::v1::run_query_request::QueryType;
use crate::protos::google::rpc::Status as RpcStatus;
use crate::protos::google::firestore::v1::{
    firestore_server::Firestore, precondition::ConditionType, write::Operation,
    BatchGetDocumentsRequest, BatchGetDocumentsResponse, BatchWriteRequest, BatchWriteResponse,
    BeginTransactionRequest, BeginTransactionResponse, CommitRequest, CommitResponse,
    CreateDocumentRequest, DeleteDocumentRequest, Document, DocumentMask, ExecutePipelineRequest,
    ExecutePipelineResponse, GetDocumentRequest, ListCollectionIdsRequest,
    ListCollectionIdsResponse, ListDocumentsRequest, ListDocumentsResponse, ListenRequest,
    ListenResponse, PartitionQueryRequest, PartitionQueryResponse, Precondition, RollbackRequest,
    RunAggregationQueryRequest, RunAggregationQueryResponse, RunQueryRequest, RunQueryResponse,
    UpdateDocumentRequest, Value, WriteRequest, WriteResponse, WriteResult,
};

/// Alphabet Firestore client SDKs use for auto-generated document ids
/// (62 URL-safe characters, 20 characters per id).
const AUTO_ID_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const AUTO_ID_LENGTH: usize = 20;

/// Streams we never produce. Phase 0 answers the streaming RPCs with
/// `UNIMPLEMENTED` before a stream is ever created, so the concrete type only
/// has to satisfy the trait.
type UnimplementedStream<T> = ReceiverStream<Result<T, Status>>;

fn unimplemented(rpc: &str, plan: &str) -> Status {
    Status::unimplemented(format!(
        "fireup local does not serve {rpc} yet ({plan}). \
         See docs/local/README.md for the full fidelity matrix."
    ))
}

/// Send `responses` as a gRPC server stream.
///
/// The whole response set is resolved before the stream is created, so a bad
/// resource name or an unsupported query fails the call rather than poisoning
/// the stream mid-flight, where many clients report it far less clearly.
fn stream_responses<T: Send + 'static>(responses: Vec<T>) -> ReceiverStream<Result<T, Status>> {
    let (sender, receiver) = tokio::sync::mpsc::channel(responses.len().max(1));
    tokio::spawn(async move {
        for response in responses {
            if sender.send(Ok(response)).await.is_err() {
                break; // client hung up
            }
        }
    });
    ReceiverStream::new(receiver)
}

/// Apply `page_size`/`page_token` paging to an already-ordered list.
///
/// The token is the key of the last item returned, so resuming is a seek rather
/// than an offset: a document written or deleted between two pages shifts
/// nothing, where an offset would make the client skip or repeat an item.
/// `page_size` of 0 means "no limit", which is what the field's default has to
/// mean for a client that never sets it.
fn paginate<T>(
    items: Vec<T>,
    page_size: i32,
    page_token: &str,
    key: impl Fn(&T) -> String,
) -> Result<(Vec<T>, String), Status> {
    if page_size < 0 {
        return Err(Status::invalid_argument("page_size must not be negative"));
    }

    let mut remaining: Vec<T> = if page_token.is_empty() {
        items
    } else {
        items
            .into_iter()
            .filter(|item| key(item).as_bytes() > page_token.as_bytes())
            .collect()
    };

    let page = if page_size == 0 {
        remaining.len()
    } else {
        page_size as usize
    };
    if remaining.len() <= page {
        return Ok((remaining, String::new()));
    }

    remaining.truncate(page);
    let next = remaining.last().map(&key).unwrap_or_default();
    Ok((remaining, next))
}

/// Generate a Firestore-style 20-character auto id.
fn auto_id() -> String {
    // uuid v4 gives 16 random bytes; 20 ids need 20, so draw from two.
    let bytes: Vec<u8> = uuid::Uuid::new_v4()
        .as_bytes()
        .iter()
        .chain(uuid::Uuid::new_v4().as_bytes().iter())
        .take(AUTO_ID_LENGTH)
        .copied()
        .collect();

    bytes
        .iter()
        .map(|byte| AUTO_ID_ALPHABET[*byte as usize % AUTO_ID_ALPHABET.len()] as char)
        .collect()
}

/// Apply a response `DocumentMask`, dropping fields the caller did not ask for.
///
/// Phase 0 handles simple top-level field names. Nested (`a.b`) and quoted
/// (`` `a&b` ``) field paths are rejected rather than silently mishandled.
fn apply_mask(document: &mut Document, mask: &Option<DocumentMask>) -> Result<(), Status> {
    let Some(mask) = mask else {
        return Ok(());
    };
    if mask.field_paths.is_empty() {
        return Ok(());
    }

    let wanted = simple_field_paths(&mask.field_paths)?;
    document
        .fields
        .retain(|field, _| wanted.iter().any(|path| path == field));
    Ok(())
}

/// Validate that every field path is a single unquoted segment and return them.
fn simple_field_paths(field_paths: &[String]) -> Result<Vec<String>, Status> {
    for path in field_paths {
        if path.contains('.') || path.contains('`') {
            return Err(unimplemented(
                &format!("nested field path {path:?}"),
                "field-path masks land with the query engine",
            ));
        }
    }
    Ok(field_paths.to_vec())
}

/// The fields an update mask selects, in mask order.
///
/// A masked field the document does not carry is dropped rather than written as
/// null: Firestore treats that as "delete this field", and Phase 1 has no field
/// deletion, so writing a null would be worse than leaving it alone.
fn masked_fields(
    field_paths: &[String],
    document: &Document,
) -> Result<Vec<(String, Value)>, Status> {
    Ok(simple_field_paths(field_paths)?
        .into_iter()
        .filter_map(|field| {
            document
                .fields
                .get(&field)
                .cloned()
                .map(|value| (field, value))
        })
        .collect())
}

/// Translate a request's `Precondition` into the store's.
///
/// The store checks it inside the same transaction as the write, which is why
/// there is no check-then-write pair anywhere in this file: two concurrent
/// `create()` calls on one name would both pass a check done up here.
fn precondition(request: &Option<Precondition>) -> Result<Option<store::Precondition>, Status> {
    let Some(condition) = request.as_ref().and_then(|p| p.condition_type.as_ref()) else {
        return Ok(None);
    };
    Ok(Some(match condition {
        ConditionType::Exists(exists) => store::Precondition::Exists(*exists),
        ConditionType::UpdateTime(expected) => {
            store::Precondition::UpdateTime(expected.clone())
        }
    }))
}

/// Translate one `Write` from a commit batch into a store write.
///
/// Pure, so a batch is fully translated — and fully rejected if any write in it
/// is unsupported — before the store is touched at all.
fn pending_write(write: &crate::protos::google::firestore::v1::Write) -> Result<PendingWrite, Status> {
    if !write.update_transforms.is_empty() {
        return Err(unimplemented(
            "field transforms (serverTimestamp, increment, array union)",
            "a transform computes the value server-side, which needs the write pipeline",
        ));
    }

    let operation = write
        .operation
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("write has no operation"))?;

    let mutation = match operation {
        Operation::Update(document) => {
            let name = parse_document_name(&document.name)?.full();
            match &write.update_mask {
                // No mask: full replace, the `set()` path.
                None => Mutation::Put(Document {
                    name,
                    ..document.clone()
                }),
                // Mask present: merge only the named fields, the
                // `set(..., {merge: true})` / `update()` path.
                Some(mask) => Mutation::Merge {
                    name,
                    fields: masked_fields(&mask.field_paths, document)?,
                },
            }
        }
        Operation::Delete(name) => Mutation::Delete {
            name: parse_document_name(name)?.full(),
        },
        Operation::Transform(_) => {
            return Err(unimplemented(
                "DocumentTransform writes",
                "a transform computes the value server-side, which needs the write pipeline",
            ))
        }
    };

    Ok(PendingWrite::guarded(
        mutation,
        precondition(&write.current_document)?,
    ))
}

/// One `WriteResult` per applied write.
fn write_results(count: usize, now: &Timestamp) -> Vec<WriteResult> {
    (0..count)
        .map(|_| WriteResult {
            update_time: Some(now.clone()),
            transform_results: Vec::new(),
        })
        .collect()
}

/// The Firestore data plane for `fireup local`.
#[derive(Clone)]
pub struct FirestoreApi {
    store: SharedStore,
    /// Live transactions. Shared between clones of the API so every connection
    /// sees the same tokens.
    transactions: Arc<Transactions>,
}

impl FirestoreApi {
    pub fn new(store: SharedStore) -> Self {
        Self {
            store,
            transactions: Arc::new(Transactions::new()),
        }
    }

    pub fn store(&self) -> &SharedStore {
        &self.store
    }

    pub fn transactions(&self) -> &Transactions {
        &self.transactions
    }

    /// Apply one write and return the document it left behind.
    ///
    /// The single-document RPCs all go through the batch commit path, so there
    /// is one implementation of precondition handling and `create_time`
    /// preservation rather than one per RPC.
    async fn apply_one(&self, write: PendingWrite) -> Result<Timestamp, Status> {
        let now = now_timestamp();
        self.store.commit(vec![write], now.clone()).await?;
        Ok(now)
    }

    /// Read a document back after writing it, for an RPC that returns one.
    async fn read_back(&self, name: &str, mask: &Option<DocumentMask>) -> Result<Document, Status> {
        let mut document = self
            .store
            .get(name)
            .await?
            .ok_or_else(|| Status::internal(format!("document {name} vanished after a write")))?;
        apply_mask(&mut document, mask)?;
        Ok(document)
    }

}

#[tonic::async_trait]
impl Firestore for FirestoreApi {
    async fn get_document(
        &self,
        request: Request<GetDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        let request = request.into_inner();
        debug!(name = %request.name, "GetDocument");

        let mut transaction = None;
        if let Some(selector) = &request.consistency_selector {
            use crate::protos::google::firestore::v1::get_document_request::ConsistencySelector;
            match selector {
                ConsistencySelector::Transaction(token) => {
                    self.transactions.check(token)?;
                    transaction = Some(token.clone());
                }
                ConsistencySelector::ReadTime(_) => {
                    return Err(unimplemented(
                        "point-in-time GetDocument reads",
                        "the store only keeps the current version of a document",
                    ))
                }
            }
        }

        let name = parse_document_name(&request.name)?.full();
        let found = self.store.get(&name).await?;
        if let Some(token) = &transaction {
            // Recorded whether or not the document exists: "it is not there"
            // is a fact this transaction may be about to act on.
            match &found {
                Some(document) => {
                    self.transactions
                        .record_reads(token, std::slice::from_ref(document), &[])?
                }
                None => self
                    .transactions
                    .record_reads(token, &[], std::slice::from_ref(&name))?,
            }
        }

        let mut document =
            found.ok_or_else(|| Status::not_found(format!("no document at {name}")))?;
        apply_mask(&mut document, &request.mask)?;
        Ok(Response::new(document))
    }

    async fn create_document(
        &self,
        request: Request<CreateDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        let request = request.into_inner();
        debug!(
            parent = %request.parent,
            collection_id = %request.collection_id,
            document_id = %request.document_id,
            "CreateDocument"
        );

        if request.collection_id.is_empty() {
            return Err(Status::invalid_argument("collection_id is required"));
        }
        if request.collection_id.contains('/') {
            return Err(Status::invalid_argument(
                "collection_id must be a single path segment",
            ));
        }

        let parent = parse_parent_name(&request.parent)?;
        let mut document = request
            .document
            .ok_or_else(|| Status::invalid_argument("document is required"))?;

        let document_id = if request.document_id.is_empty() {
            auto_id()
        } else {
            if request.document_id.contains('/') {
                return Err(Status::invalid_argument(
                    "document_id must be a single path segment",
                ));
            }
            request.document_id
        };

        // The API contract says the caller must not set `document.name`; the
        // server derives it from parent/collection/id.
        let name = format!("{}/{}/{}", parent.full(), request.collection_id, document_id);
        document.name = name.clone();

        // `exists: false` rather than a contains() check: the store enforces it
        // inside the write, so two concurrent creates cannot both succeed.
        self.apply_one(PendingWrite::guarded(
            Mutation::Put(document),
            Some(store::Precondition::Exists(false)),
        ))
        .await?;

        Ok(Response::new(self.read_back(&name, &request.mask).await?))
    }

    async fn update_document(
        &self,
        request: Request<UpdateDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        let request = request.into_inner();
        let document = request
            .document
            .ok_or_else(|| Status::invalid_argument("document is required"))?;
        debug!(name = %document.name, "UpdateDocument");

        let name = parse_document_name(&document.name)?.full();
        let mutation = match &request.update_mask {
            None => Mutation::Put(Document {
                name: name.clone(),
                ..document
            }),
            Some(mask) => Mutation::Merge {
                name: name.clone(),
                fields: masked_fields(&mask.field_paths, &document)?,
            },
        };

        self.apply_one(PendingWrite::guarded(
            mutation,
            precondition(&request.current_document)?,
        ))
        .await?;

        Ok(Response::new(self.read_back(&name, &request.mask).await?))
    }

    async fn delete_document(
        &self,
        request: Request<DeleteDocumentRequest>,
    ) -> Result<Response<()>, Status> {
        let request = request.into_inner();
        debug!(name = %request.name, "DeleteDocument");

        let name = parse_document_name(&request.name)?.full();
        self.apply_one(PendingWrite::guarded(
            Mutation::Delete { name },
            precondition(&request.current_document)?,
        ))
        .await?;
        Ok(Response::new(()))
    }

    type BatchGetDocumentsStream = ReceiverStream<Result<BatchGetDocumentsResponse, Status>>;

    async fn batch_get_documents(
        &self,
        request: Request<BatchGetDocumentsRequest>,
    ) -> Result<Response<Self::BatchGetDocumentsStream>, Status> {
        let request = request.into_inner();
        debug!(count = request.documents.len(), "BatchGetDocuments");

        // A point-in-time read stays unserved: the store only holds the latest
        // version of a document, so answering one would silently lie.
        let mut transaction = None;
        if let Some(selector) = &request.consistency_selector {
            use crate::protos::google::firestore::v1::batch_get_documents_request::ConsistencySelector;
            match selector {
                ConsistencySelector::Transaction(token) => {
                    self.transactions.check(token)?;
                    transaction = Some(token.clone());
                }
                // The client asked us to start a transaction for this read and
                // report the token back in the first response.
                ConsistencySelector::NewTransaction(options) => {
                    transaction = Some(self.transactions.begin(&Some(options.clone()))?);
                }
                ConsistencySelector::ReadTime(_) => {
                    return Err(unimplemented(
                        "point-in-time BatchGetDocuments reads",
                        "the store only keeps the current version of a document",
                    ))
                }
            }
        }
        let started_transaction = matches!(
            request.consistency_selector,
            Some(crate::protos::google::firestore::v1::batch_get_documents_request::ConsistencySelector::NewTransaction(_))
        );

        let read_time = now_timestamp();
        // Resolve eagerly so a bad resource name fails the call rather than
        // poisoning the stream mid-flight.
        let mut responses = Vec::with_capacity(request.documents.len());
        let mut found = Vec::new();
        let mut absent = Vec::new();
        for name in &request.documents {
            let name = parse_document_name(name)?.full();
            let result = match self.store.get(&name).await? {
                Some(document) => {
                    found.push(document.clone());
                    let mut masked = document;
                    apply_mask(&mut masked, &request.mask)?;
                    crate::protos::google::firestore::v1::batch_get_documents_response::Result::Found(
                        masked,
                    )
                }
                None => {
                    absent.push(name.clone());
                    crate::protos::google::firestore::v1::batch_get_documents_response::Result::Missing(
                        name,
                    )
                }
            };
            responses.push(BatchGetDocumentsResponse {
                result: Some(result),
                transaction: Vec::new(),
                read_time: Some(read_time.clone()),
            });
        }

        if let Some(token) = &transaction {
            // The read set is recorded from the unmasked documents, so a mask
            // cannot narrow what the commit's conflict check covers.
            self.transactions.record_reads(token, &found, &absent)?;
            if started_transaction {
                // The contract puts the new token on the first response.
                if let Some(first) = responses.first_mut() {
                    first.transaction = token.clone();
                } else {
                    responses.push(BatchGetDocumentsResponse {
                        result: None,
                        transaction: token.clone(),
                        read_time: Some(read_time.clone()),
                    });
                }
            }
        }

        Ok(Response::new(stream_responses(responses)))
    }

    async fn commit(
        &self,
        request: Request<CommitRequest>,
    ) -> Result<Response<CommitResponse>, Status> {
        let request = request.into_inner();
        debug!(writes = request.writes.len(), "Commit");

        // Translated in full before anything is written, so an unsupported
        // write in the middle of a batch cannot leave the first half applied.
        let writes: Vec<PendingWrite> = request
            .writes
            .iter()
            .map(pending_write)
            .collect::<Result<_, Status>>()?;

        // A transaction's read set is checked in the same store transaction as
        // its writes, so a document that changed since it was read aborts the
        // commit instead of being overwritten.
        let mut batch = if request.transaction.is_empty() {
            Vec::new()
        } else {
            self.transactions.finish(&request.transaction, true)?
        };
        let applied = writes.len();
        batch.extend(writes);

        let now = now_timestamp();
        self.store.commit(batch, now.clone()).await?;

        Ok(Response::new(CommitResponse {
            write_results: write_results(applied, &now),
            commit_time: Some(now),
        }))
    }

    async fn list_documents(
        &self,
        request: Request<ListDocumentsRequest>,
    ) -> Result<Response<ListDocumentsResponse>, Status> {
        let request = request.into_inner();
        debug!(
            parent = %request.parent,
            collection_id = %request.collection_id,
            "ListDocuments"
        );

        if let Some(selector) = &request.consistency_selector {
            use crate::protos::google::firestore::v1::list_documents_request::ConsistencySelector;
            match selector {
                ConsistencySelector::Transaction(token) => self.transactions.check(token)?,
                ConsistencySelector::ReadTime(_) => {
                    return Err(unimplemented(
                        "point-in-time ListDocuments reads",
                        "the store only keeps the current version of a document",
                    ))
                }
            }
        }
        if !request.order_by.is_empty() {
            return Err(unimplemented(
                &format!("ListDocuments ordered by {:?}", request.order_by),
                "documents come back ordered by name; use RunQuery to order by a field",
            ));
        }

        let parent = parse_parent_name(&request.parent)?;
        // An empty collection_id means "every collection under this parent",
        // which the published contract allows.
        let collections = if request.collection_id.is_empty() {
            self.store.child_segments(&parent.full()).await?
        } else {
            if request.collection_id.contains('/') {
                return Err(Status::invalid_argument(
                    "collection_id must be a single path segment",
                ));
            }
            vec![request.collection_id.clone()]
        };

        let mut documents = Vec::new();
        for collection_id in &collections {
            documents.extend(
                self.store
                    .scan_collection(&parent.full(), collection_id)
                    .await?,
            );

            if request.show_missing {
                // A "missing" document is one that was never written but has
                // something nested underneath it. Firestore reports it with a
                // name and no fields or timestamps, which is how an SDK's
                // `listDocuments()` sees a collection whose parents are gaps.
                let collection = format!("{}/{}", parent.full(), collection_id);
                let present: std::collections::HashSet<&str> = documents
                    .iter()
                    .map(|document: &Document| document.name.as_str())
                    .collect();
                let missing: Vec<Document> = self
                    .store
                    .child_segments(&collection)
                    .await?
                    .into_iter()
                    .map(|document_id| format!("{collection}/{document_id}"))
                    .filter(|name| !present.contains(name.as_str()))
                    .map(|name| Document {
                        name,
                        ..Default::default()
                    })
                    .collect();
                documents.extend(missing);
            }
        }

        // One order for the whole response, since several collections may have
        // contributed to it.
        documents.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
        for document in &mut documents {
            apply_mask(document, &request.mask)?;
        }

        let (documents, next_page_token) =
            paginate(documents, request.page_size, &request.page_token, |document| {
                document.name.clone()
            })?;

        Ok(Response::new(ListDocumentsResponse {
            documents,
            next_page_token,
        }))
    }

    async fn begin_transaction(
        &self,
        request: Request<BeginTransactionRequest>,
    ) -> Result<Response<BeginTransactionResponse>, Status> {
        let request = request.into_inner();
        debug!(database = %request.database, "BeginTransaction");

        let transaction = self.transactions.begin(&request.options)?;
        Ok(Response::new(BeginTransactionResponse { transaction }))
    }

    async fn rollback(&self, request: Request<RollbackRequest>) -> Result<Response<()>, Status> {
        let request = request.into_inner();
        debug!("Rollback");

        // Nothing to undo: the client SDK buffers a transaction's writes and
        // only sends them with the commit, so a rolled-back transaction never
        // wrote anything. Dropping the token is the whole operation — and it
        // still has to reject an unknown one, or a client bug looks like a
        // success.
        self.transactions.finish(&request.transaction, false)?;
        Ok(Response::new(()))
    }

    type RunQueryStream = ReceiverStream<Result<RunQueryResponse, Status>>;

    async fn run_query(
        &self,
        request: Request<RunQueryRequest>,
    ) -> Result<Response<Self::RunQueryStream>, Status> {
        let request = request.into_inner();
        debug!(parent = %request.parent, "RunQuery");

        let mut transaction = None;
        let mut started_transaction = false;
        if let Some(selector) = &request.consistency_selector {
            use crate::protos::google::firestore::v1::run_query_request::ConsistencySelector;
            match selector {
                ConsistencySelector::Transaction(token) => {
                    self.transactions.check(token)?;
                    transaction = Some(token.clone());
                }
                ConsistencySelector::NewTransaction(options) => {
                    transaction = Some(self.transactions.begin(&Some(options.clone()))?);
                    started_transaction = true;
                }
                ConsistencySelector::ReadTime(_) => {
                    return Err(unimplemented(
                        "point-in-time RunQuery reads",
                        "the store only keeps the current version of a document",
                    ))
                }
            }
        }
        if request.explain_options.is_some() {
            return Err(unimplemented(
                "query explain options",
                "there is no query planner to explain",
            ));
        }

        let parent = parse_parent_name(&request.parent)?;
        let structured = match &request.query_type {
            Some(QueryType::StructuredQuery(query)) => query,
            None => return Err(Status::invalid_argument("RunQuery needs a structured_query")),
        };

        // Compiled before anything is read, so an unsupported query shape fails
        // immediately instead of after a collection scan.
        let query = Query::compile(structured)?;
        let documents = self
            .store
            .scan_collection(&parent.full(), query.collection_id())
            .await?;
        let results = query.evaluate(documents);

        if let Some(token) = &transaction {
            // Every document the query returned joins the read set, so a
            // transaction that acted on a query result cannot commit after
            // someone else changed one of those documents. Documents the query
            // filtered out are not covered — recorded in the fidelity matrix.
            self.transactions
                .record_reads(token, &results.documents, &[])?;
        }

        let read_time = now_timestamp();
        let mut responses: Vec<RunQueryResponse> = results
            .documents
            .into_iter()
            .map(|document| RunQueryResponse {
                document: Some(document),
                read_time: Some(read_time.clone()),
                skipped_results: 0,
                ..Default::default()
            })
            .collect();

        // The contract says an empty result set still reports the time the
        // query ran, so a client can tell "no matches" from "nothing happened".
        if responses.is_empty() {
            responses.push(RunQueryResponse {
                read_time: Some(read_time.clone()),
                ..Default::default()
            });
        }
        if let Some(first) = responses.first_mut() {
            first.skipped_results = i32::try_from(results.skipped).unwrap_or(i32::MAX);
            if started_transaction {
                first.transaction = transaction.clone().unwrap_or_default();
            }
        }

        Ok(Response::new(stream_responses(responses)))
    }

    type ExecutePipelineStream = UnimplementedStream<ExecutePipelineResponse>;

    async fn execute_pipeline(
        &self,
        _request: Request<ExecutePipelineRequest>,
    ) -> Result<Response<Self::ExecutePipelineStream>, Status> {
        Err(unimplemented("ExecutePipeline", "out of scope"))
    }

    type RunAggregationQueryStream = UnimplementedStream<RunAggregationQueryResponse>;

    async fn run_aggregation_query(
        &self,
        _request: Request<RunAggregationQueryRequest>,
    ) -> Result<Response<Self::RunAggregationQueryStream>, Status> {
        Err(unimplemented("RunAggregationQuery", "post-query-engine"))
    }

    async fn partition_query(
        &self,
        _request: Request<PartitionQueryRequest>,
    ) -> Result<Response<PartitionQueryResponse>, Status> {
        Err(unimplemented("PartitionQuery", "out of scope"))
    }

    type WriteStream = UnimplementedStream<WriteResponse>;

    async fn write(
        &self,
        _request: Request<tonic::Streaming<WriteRequest>>,
    ) -> Result<Response<Self::WriteStream>, Status> {
        Err(unimplemented(
            "the streaming Write RPC",
            "Commit covers the SDK write path",
        ))
    }

    type ListenStream = UnimplementedStream<ListenResponse>;

    async fn listen(
        &self,
        _request: Request<tonic::Streaming<ListenRequest>>,
    ) -> Result<Response<Self::ListenStream>, Status> {
        Err(unimplemented("Listen", "realtime listeners phase"))
    }

    async fn list_collection_ids(
        &self,
        request: Request<ListCollectionIdsRequest>,
    ) -> Result<Response<ListCollectionIdsResponse>, Status> {
        let request = request.into_inner();
        debug!(parent = %request.parent, "ListCollectionIds");

        if request.consistency_selector.is_some() {
            return Err(unimplemented(
                "point-in-time ListCollectionIds reads",
                "the store only keeps the current version of a document",
            ));
        }

        let parent = parse_parent_name(&request.parent)?;
        let ids = self.store.child_segments(&parent.full()).await?;
        let (collection_ids, next_page_token) =
            paginate(ids, request.page_size, &request.page_token, Clone::clone)?;

        Ok(Response::new(ListCollectionIdsResponse {
            collection_ids,
            next_page_token,
        }))
    }

    async fn batch_write(
        &self,
        request: Request<BatchWriteRequest>,
    ) -> Result<Response<BatchWriteResponse>, Status> {
        let request = request.into_inner();
        debug!(writes = request.writes.len(), "BatchWrite");

        // Unlike `Commit`, `BatchWrite` is explicitly *not* atomic: each write
        // succeeds or fails on its own and the response reports a status per
        // write. So each one is its own single-write commit.
        let mut write_results = Vec::with_capacity(request.writes.len());
        let mut statuses = Vec::with_capacity(request.writes.len());

        for write in &request.writes {
            let outcome = match pending_write(write) {
                Err(status) => Err(status),
                Ok(pending) => self.apply_one(pending).await,
            };
            match outcome {
                Ok(now) => {
                    write_results.push(WriteResult {
                        update_time: Some(now),
                        transform_results: Vec::new(),
                    });
                    statuses.push(RpcStatus::default());
                }
                Err(status) => {
                    // A failed write still occupies its slot, so the client can
                    // line results up with the writes it sent.
                    write_results.push(WriteResult::default());
                    statuses.push(RpcStatus {
                        code: status.code() as i32,
                        message: status.message().to_string(),
                        details: Vec::new(),
                    });
                }
            }
        }

        Ok(Response::new(BatchWriteResponse {
            write_results,
            status: statuses,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_ids_have_the_documented_shape() {
        let id = auto_id();
        assert_eq!(id.len(), AUTO_ID_LENGTH);
        assert!(id.bytes().all(|b| AUTO_ID_ALPHABET.contains(&b)), "{id}");
        assert_ne!(id, auto_id(), "ids must not repeat");
    }

    #[test]
    fn nested_field_paths_are_rejected_rather_than_mishandled() {
        let err = simple_field_paths(&["a.b".to_string()]).expect_err("nested path");
        assert_eq!(err.code(), tonic::Code::Unimplemented);
    }
}
