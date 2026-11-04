//! Firestore v1 gRPC service for `fireup local`.
//!
//! Implemented against the public `google.firestore.v1.Firestore` service
//! definition vendored under `proto/`. Phase 0 serves the subset that official
//! client SDKs need for document CRUD:
//!
//! | RPC                  | Status |
//! |----------------------|--------|
//! | `GetDocument`        | served |
//! | `CreateDocument`     | served |
//! | `UpdateDocument`     | served |
//! | `DeleteDocument`     | served |
//! | `BatchGetDocuments`  | served (gRPC SDK read path) |
//! | `Commit`             | served, non-transactional writes only |
//! | everything else      | `UNIMPLEMENTED` |
//!
//! Every unimplemented RPC answers with an explicit `UNIMPLEMENTED` status and a
//! message naming the phase that will add it, so an SDK failure is immediately
//! legible instead of looking like a hang or a protocol error.
//!
//! This is an independent implementation of a published API contract. No
//! Firebase Emulator Suite code or behaviour was reverse-engineered; see
//! `.cursor/rules/firebase-gcp-compliance.mdc`.

use std::collections::HashMap;

use prost_types::Timestamp;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::debug;

use crate::local::names::{parse_document_name, parse_parent_name};
use crate::local::store::DocumentStore;
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
         Phase 0 serves document CRUD only."
    ))
}

fn now_timestamp() -> Timestamp {
    let now = chrono::Utc::now();
    Timestamp {
        seconds: now.timestamp(),
        nanos: now.timestamp_subsec_nanos() as i32,
    }
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

/// Check a write precondition against current store state.
fn check_precondition(
    store: &DocumentStore,
    name: &str,
    precondition: &Option<Precondition>,
) -> Result<(), Status> {
    let Some(condition) = precondition.as_ref().and_then(|p| p.condition_type.as_ref()) else {
        return Ok(());
    };

    match condition {
        ConditionType::Exists(true) => {
            if !store.contains(name) {
                return Err(Status::not_found(format!("no document to update at {name}")));
            }
        }
        ConditionType::Exists(false) => {
            if store.contains(name) {
                return Err(Status::already_exists(format!(
                    "document already exists: {name}"
                )));
            }
        }
        ConditionType::UpdateTime(expected) => {
            let actual = store.get(name).and_then(|document| document.update_time);
            if actual.as_ref() != Some(expected) {
                return Err(Status::failed_precondition(format!(
                    "update_time precondition failed for {name}"
                )));
            }
        }
    }
    Ok(())
}

/// The Firestore data plane for `fireup local`.
#[derive(Clone, Default)]
pub struct FirestoreApi {
    store: DocumentStore,
}

impl FirestoreApi {
    pub fn new(store: DocumentStore) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &DocumentStore {
        &self.store
    }

    /// Read a document, applying the response mask.
    fn read(&self, name: &str, mask: &Option<DocumentMask>) -> Result<Document, Status> {
        let parsed = parse_document_name(name)?;
        let mut document = self
            .store
            .get(&parsed.full())
            .ok_or_else(|| Status::not_found(format!("no document at {name}")))?;
        apply_mask(&mut document, mask)?;
        Ok(document)
    }

    /// Apply one `Write` from a commit batch.
    fn apply_write(
        &self,
        write: &crate::protos::google::firestore::v1::Write,
        now: &Timestamp,
    ) -> Result<WriteResult, Status> {
        if !write.update_transforms.is_empty() {
            return Err(unimplemented(
                "field transforms (serverTimestamp, increment, array union)",
                "transforms land with the write pipeline",
            ));
        }

        let operation = write
            .operation
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("write has no operation"))?;

        match operation {
            Operation::Update(document) => {
                let parsed = parse_document_name(&document.name)?;
                let name = parsed.full();
                check_precondition(&self.store, &name, &write.current_document)?;

                match &write.update_mask {
                    // No mask: full replace, the `set()` path.
                    None => {
                        self.store.put(document.clone(), now.clone());
                    }
                    // Mask present: merge only the named fields, the
                    // `set(..., {merge: true})` / `update()` path.
                    Some(mask) => {
                        let wanted = simple_field_paths(&mask.field_paths)?;
                        let fields: HashMap<String, Value> = wanted
                            .into_iter()
                            .filter_map(|field| {
                                document
                                    .fields
                                    .get(&field)
                                    .cloned()
                                    .map(|value| (field, value))
                            })
                            .collect();
                        self.store.merge(&name, fields, now.clone());
                    }
                }
            }
            Operation::Delete(name) => {
                let parsed = parse_document_name(name)?;
                let key = parsed.full();
                check_precondition(&self.store, &key, &write.current_document)?;
                self.store.delete(&key);
            }
            Operation::Transform(_) => {
                return Err(unimplemented(
                    "DocumentTransform writes",
                    "transforms land with the write pipeline",
                ));
            }
        }

        Ok(WriteResult {
            update_time: Some(now.clone()),
            transform_results: Vec::new(),
        })
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

        if let Some(selector) = &request.consistency_selector {
            use crate::protos::google::firestore::v1::get_document_request::ConsistencySelector;
            let what = match selector {
                ConsistencySelector::Transaction(_) => "transactional GetDocument",
                ConsistencySelector::ReadTime(_) => "point-in-time GetDocument reads",
            };
            return Err(unimplemented(what, "consistency modes land in a later phase"));
        }

        Ok(Response::new(self.read(&request.name, &request.mask)?))
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
        if self.store.contains(&name) {
            return Err(Status::already_exists(format!(
                "document already exists: {name}"
            )));
        }

        document.name = name.clone();
        self.store.put(document, now_timestamp());

        let mut created = self
            .store
            .get(&name)
            .ok_or_else(|| Status::internal("document vanished immediately after create"))?;
        apply_mask(&mut created, &request.mask)?;
        Ok(Response::new(created))
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

        let parsed = parse_document_name(&document.name)?;
        let name = parsed.full();
        check_precondition(&self.store, &name, &request.current_document)?;

        let now = now_timestamp();
        match &request.update_mask {
            None => {
                self.store.put(document, now);
            }
            Some(mask) => {
                let wanted = simple_field_paths(&mask.field_paths)?;
                let fields: HashMap<String, Value> = wanted
                    .into_iter()
                    .filter_map(|field| {
                        document.fields.get(&field).cloned().map(|v| (field, v))
                    })
                    .collect();
                self.store.merge(&name, fields, now);
            }
        }

        let mut updated = self
            .store
            .get(&name)
            .ok_or_else(|| Status::internal("document vanished immediately after update"))?;
        apply_mask(&mut updated, &request.mask)?;
        Ok(Response::new(updated))
    }

    async fn delete_document(
        &self,
        request: Request<DeleteDocumentRequest>,
    ) -> Result<Response<()>, Status> {
        let request = request.into_inner();
        debug!(name = %request.name, "DeleteDocument");

        let parsed = parse_document_name(&request.name)?;
        let name = parsed.full();
        check_precondition(&self.store, &name, &request.current_document)?;
        self.store.delete(&name);
        Ok(Response::new(()))
    }

    type BatchGetDocumentsStream = ReceiverStream<Result<BatchGetDocumentsResponse, Status>>;

    async fn batch_get_documents(
        &self,
        request: Request<BatchGetDocumentsRequest>,
    ) -> Result<Response<Self::BatchGetDocumentsStream>, Status> {
        let request = request.into_inner();
        debug!(count = request.documents.len(), "BatchGetDocuments");

        // `read_time` (point-in-time reads) is also unserved, but the in-memory
        // store only ever holds the latest version, so treating it as a strong
        // read would silently lie. Reject anything but default consistency.
        if let Some(selector) = &request.consistency_selector {
            use crate::protos::google::firestore::v1::batch_get_documents_request::ConsistencySelector;
            let what = match selector {
                ConsistencySelector::Transaction(_) | ConsistencySelector::NewTransaction(_) => {
                    "transactional BatchGetDocuments"
                }
                ConsistencySelector::ReadTime(_) => "point-in-time BatchGetDocuments reads",
            };
            return Err(unimplemented(what, "consistency modes land in a later phase"));
        }

        let read_time = now_timestamp();
        // Resolve eagerly so a bad resource name fails the call rather than
        // poisoning the stream mid-flight.
        let mut responses = Vec::with_capacity(request.documents.len());
        for name in &request.documents {
            let parsed = parse_document_name(name)?;
            let result = match self.store.get(&parsed.full()) {
                Some(mut document) => {
                    apply_mask(&mut document, &request.mask)?;
                    crate::protos::google::firestore::v1::batch_get_documents_response::Result::Found(
                        document,
                    )
                }
                None => {
                    crate::protos::google::firestore::v1::batch_get_documents_response::Result::Missing(
                        name.clone(),
                    )
                }
            };
            responses.push(BatchGetDocumentsResponse {
                result: Some(result),
                transaction: Vec::new(),
                read_time: Some(read_time.clone()),
            });
        }

        let (sender, receiver) = tokio::sync::mpsc::channel(responses.len().max(1));
        tokio::spawn(async move {
            for response in responses {
                if sender.send(Ok(response)).await.is_err() {
                    break; // client hung up
                }
            }
        });
        Ok(Response::new(ReceiverStream::new(receiver)))
    }

    async fn commit(
        &self,
        request: Request<CommitRequest>,
    ) -> Result<Response<CommitResponse>, Status> {
        let request = request.into_inner();
        debug!(writes = request.writes.len(), "Commit");

        if !request.transaction.is_empty() {
            return Err(unimplemented(
                "transactional Commit",
                "transactions land in a later phase",
            ));
        }

        let now = now_timestamp();
        let mut write_results = Vec::with_capacity(request.writes.len());
        for write in &request.writes {
            // Phase 0 has no rollback: a failed write mid-batch leaves earlier
            // writes applied. Recorded as known divergence in the contract doc.
            write_results.push(self.apply_write(write, &now)?);
        }

        Ok(Response::new(CommitResponse {
            write_results,
            commit_time: Some(now),
        }))
    }

    async fn list_documents(
        &self,
        _request: Request<ListDocumentsRequest>,
    ) -> Result<Response<ListDocumentsResponse>, Status> {
        Err(unimplemented("ListDocuments", "Phase 1 collection reads"))
    }

    async fn begin_transaction(
        &self,
        _request: Request<BeginTransactionRequest>,
    ) -> Result<Response<BeginTransactionResponse>, Status> {
        Err(unimplemented("BeginTransaction", "transactions phase"))
    }

    async fn rollback(&self, _request: Request<RollbackRequest>) -> Result<Response<()>, Status> {
        Err(unimplemented("Rollback", "transactions phase"))
    }

    type RunQueryStream = UnimplementedStream<RunQueryResponse>;

    async fn run_query(
        &self,
        _request: Request<RunQueryRequest>,
    ) -> Result<Response<Self::RunQueryStream>, Status> {
        Err(unimplemented("RunQuery", "Phase 1 simple-query engine"))
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
        _request: Request<ListCollectionIdsRequest>,
    ) -> Result<Response<ListCollectionIdsResponse>, Status> {
        Err(unimplemented(
            "ListCollectionIds",
            "Phase 1 collection reads",
        ))
    }

    async fn batch_write(
        &self,
        _request: Request<BatchWriteRequest>,
    ) -> Result<Response<BatchWriteResponse>, Status> {
        Err(unimplemented("BatchWrite", "Commit covers the SDK write path"))
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
