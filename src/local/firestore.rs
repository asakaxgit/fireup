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
//! | `Listen`             | served, see [`watch`](crate::local::watch) |
//! | everything else      | `UNIMPLEMENTED` |
//!
//! A write's `update_transforms` — and a transform-only `DocumentTransform`
//! write — are served for `serverTimestamp`, `increment`, `arrayUnion`, and
//! `arrayRemove`; see [`transforms`](crate::local::transforms) for the
//! semantics and for what is still refused. `UpdateDocument` does not carry
//! transforms in the contract, so an SDK's `update()` with a `FieldValue`
//! sentinel arrives here as a `Commit`.
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
use crate::local::query::{self, Query};
use crate::local::store::{self, now_timestamp, Mutation, PendingWrite, SharedStore, WriteEffect};
use crate::local::transactions::{Finish, Transactions};
use crate::local::transforms;
use crate::local::watch::{self, WatchHub};
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

/// Streams we never produce. The streaming RPCs that stay out of scope answer
/// `UNIMPLEMENTED` before a stream is ever created, so the concrete type only
/// has to satisfy the trait.
type UnimplementedStream<T> = ReceiverStream<Result<T, Status>>;

pub(crate) fn unimplemented(rpc: &str, plan: &str) -> Status {
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
/// Nested (`a.b`) and backtick-quoted (`` `a&b` ``) paths are read with the
/// query engine's field-path parser, so a mask and a `select()` projection
/// select exactly the same thing: masking `a.b` returns an `a` holding only
/// `b`, not a flattened `a.b` and not the whole of `a`.
fn apply_mask(document: &mut Document, mask: &Option<DocumentMask>) -> Result<(), Status> {
    apply_parsed_mask(document, &parse_mask(mask)?);
    Ok(())
}

/// Parse a response `DocumentMask`, or `None` when it selects everything.
///
/// Separate from applying it so a caller can reject a malformed mask *before*
/// reading: a bad request should be reported as one even when the document it
/// names is also missing.
fn parse_mask(mask: &Option<DocumentMask>) -> Result<Option<Vec<query::FieldPath>>, Status> {
    let Some(mask) = mask else {
        return Ok(None);
    };
    if mask.field_paths.is_empty() {
        return Ok(None);
    }
    mask.field_paths
        .iter()
        .map(|path| query::FieldPath::parse(path))
        .collect::<Result<Vec<_>, Status>>()
        .map(Some)
}

/// Apply an already-parsed mask.
fn apply_parsed_mask(document: &mut Document, mask: &Option<Vec<query::FieldPath>>) {
    if let Some(paths) = mask {
        query::retain_field_paths(&mut document.fields, paths);
    }
}

/// The fields an update mask selects, in mask order.
///
/// A masked field the document does not carry becomes a `None`, which
/// [`Mutation::Merge`] applies as a **deletion** — the documented Firestore
/// behaviour, and the reason the store grew a per-path `Option` rather than
/// leaving such a field alone as it did before.
///
/// Unlike a read mask, this one is still restricted to top-level fields. A
/// nested update mask has to merge *into* a stored map, which is a change to
/// the store's write contract rather than to the query engine, so it stays an
/// explicit refusal instead of overwriting the whole parent map.
fn masked_fields(
    field_paths: &[String],
    document: &Document,
) -> Result<Vec<(String, Option<Value>)>, Status> {
    for path in field_paths {
        if path.contains('.') || path.contains('`') {
            return Err(unimplemented(
                &format!("the nested update_mask path {path:?}"),
                "an update mask writes whole top-level fields",
            ));
        }
    }
    Ok(field_paths
        .iter()
        .map(|field| (field.clone(), document.fields.get(field).cloned()))
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
/// is unsupported — before the store is touched at all. The transforms travel
/// with the mutation and are computed by the store inside the write's own
/// transaction; see [`transforms`](crate::local::transforms).
fn pending_write(write: &crate::protos::google::firestore::v1::Write) -> Result<PendingWrite, Status> {
    let operation = write
        .operation
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("write has no operation"))?;

    let (mutation, transforms) = match operation {
        Operation::Update(document) => {
            let name = parse_document_name(&document.name)?.full();
            let mutation = match &write.update_mask {
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
            };
            (mutation, transforms::compile(&write.update_transforms)?)
        }
        Operation::Delete(name) => {
            if !write.update_transforms.is_empty() {
                return Err(Status::invalid_argument(
                    "update_transforms may only be set on an `update` write",
                ));
            }
            (
                Mutation::Delete {
                    name: parse_document_name(name)?.full(),
                },
                Vec::new(),
            )
        }
        // A transform-only write: no field data, just the transforms. An empty
        // merge is the right base for it — the contract creates the document if
        // it is absent, and otherwise leaves every untransformed field alone.
        Operation::Transform(transform) => {
            if !write.update_transforms.is_empty() {
                return Err(Status::invalid_argument(
                    "update_transforms may only be set on an `update` write",
                ));
            }
            if transform.field_transforms.is_empty() {
                return Err(Status::invalid_argument(
                    "a transform write needs at least one field transform",
                ));
            }
            (
                Mutation::Merge {
                    name: parse_document_name(&transform.document)?.full(),
                    fields: Vec::new(),
                },
                transforms::compile(&transform.field_transforms)?,
            )
        }
    };

    Ok(PendingWrite::transformed(
        mutation,
        precondition(&write.current_document)?,
        transforms,
    ))
}

/// One `WriteResult` per applied write, carrying that write's transform
/// results — the values `increment` and `serverTimestamp` resolved to, which an
/// SDK reads back off the commit rather than with another round trip.
fn write_results(effects: &[WriteEffect], now: &Timestamp) -> Vec<WriteResult> {
    effects
        .iter()
        .map(|effect| WriteResult {
            update_time: Some(now.clone()),
            transform_results: effect.transform_results.clone(),
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
    /// The change feed `Listen` streams subscribe to. Shared for the same
    /// reason: a write arriving on one connection has to reach a listener on
    /// another.
    watch: Arc<WatchHub>,
}

impl FirestoreApi {
    /// Build an API over `store`, wrapping it in a change feed of its own.
    ///
    /// Use [`FirestoreApi::watching`] when another service shares the store, so
    /// that writes through *that* service reach these listeners too.
    pub fn new(store: SharedStore) -> Self {
        let (store, watch) = watch::attach(store);
        Self::watching(store, watch)
    }

    /// Build an API over a store that is already wrapped in `watch`.
    pub fn watching(store: SharedStore, watch: Arc<WatchHub>) -> Self {
        Self {
            store,
            transactions: Arc::new(Transactions::new()),
            watch,
        }
    }

    pub fn store(&self) -> &SharedStore {
        &self.store
    }

    pub fn transactions(&self) -> &Transactions {
        &self.transactions
    }

    pub fn watch(&self) -> &Arc<WatchHub> {
        &self.watch
    }

    /// Apply one write and report the time it landed and what its transforms
    /// resolved to.
    ///
    /// The single-document RPCs all go through the batch commit path, so there
    /// is one implementation of precondition handling and `create_time`
    /// preservation rather than one per RPC.
    async fn apply_one(&self, write: PendingWrite) -> Result<(Timestamp, WriteEffect), Status> {
        let now = now_timestamp();
        let effects = self.store.commit(vec![write], now.clone()).await?;
        let effect = effects
            .into_iter()
            .next()
            .ok_or_else(|| Status::internal("a one-write commit returned no write effect"))?;
        Ok((now, effect))
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
        // Parsed before the read, so a malformed field path is reported as the
        // bad request it is rather than as a missing document.
        let mask = parse_mask(&request.mask)?;
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
        apply_parsed_mask(&mut document, &mask);
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
            self.transactions
                .finish(&request.transaction, Finish::Commit)?
        };
        let applied = writes.len();
        batch.extend(writes);

        let now = now_timestamp();
        let mut effects = self.store.commit(batch, now.clone()).await?;

        // The read set's verifications were prepended to the batch, so the
        // caller's own writes — and their transform results — are the tail.
        let verified = effects.len().saturating_sub(applied);
        effects.drain(..verified);

        Ok(Response::new(CommitResponse {
            write_results: write_results(&effects, &now),
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
        self.transactions
            .finish(&request.transaction, Finish::Rollback)?;
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
        // Both reads hand the store the query's pre-filter. A backend may use
        // it to return fewer rows, but only ever a superset of the matching
        // ones — `evaluate` below still decides the result, so a backend that
        // pushes nothing down answers identically.
        let prefilter = query.prefilter();
        let documents = if query.all_descendants() {
            // A collection group query reads every collection with this id
            // anywhere beneath the parent, which is the scan the Datastore
            // kind queries already needed. It is also the read that most wants
            // narrowing, being the widest one here.
            self.store
                .scan_kind_filtered(&parent.full(), query.collection_id(), prefilter)
                .await?
        } else {
            self.store
                .scan_collection_filtered(&parent.full(), query.collection_id(), prefilter)
                .await?
        };
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

    type ListenStream = ReceiverStream<Result<ListenResponse, Status>>;

    async fn listen(
        &self,
        request: Request<tonic::Streaming<ListenRequest>>,
    ) -> Result<Response<Self::ListenStream>, Status> {
        debug!("Listen");

        // The stream is accepted unconditionally and each target is validated
        // as it arrives: a target this server cannot serve is reported as a
        // `TargetChange::REMOVE` carrying the cause, which is the contract's
        // per-target error channel. Failing the whole call instead would take
        // the client's other listeners down with the bad one.
        Ok(Response::new(watch::listen(
            self.store.clone(),
            &self.watch,
            request.into_inner(),
        )))
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
                Ok((now, effect)) => {
                    write_results.push(WriteResult {
                        update_time: Some(now),
                        transform_results: effect.transform_results,
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

    /// `{ address: { city: "Tokyo", zip: "100-0001" }, name: "alice" }`
    fn nested_document() -> Document {
        use crate::protos::google::firestore::v1::value::ValueType;
        use crate::protos::google::firestore::v1::MapValue;

        let text = |value: &str| Value {
            value_type: Some(ValueType::StringValue(value.to_string())),
        };
        Document {
            name: "projects/demo/databases/(default)/documents/users/alice".to_string(),
            fields: [
                (
                    "address".to_string(),
                    Value {
                        value_type: Some(ValueType::MapValue(MapValue {
                            fields: [
                                ("city".to_string(), text("Tokyo")),
                                ("zip".to_string(), text("100-0001")),
                            ]
                            .into_iter()
                            .collect(),
                        })),
                    },
                ),
                ("name".to_string(), text("alice")),
            ]
            .into_iter()
            .collect(),
            create_time: None,
            update_time: None,
        }
    }

    /// The value at `path` in `document`, as a debug string.
    fn at(document: &Document, path: &str) -> Option<String> {
        use crate::protos::google::firestore::v1::value::ValueType;

        let mut segments = path.split('.');
        let mut value = document.fields.get(segments.next()?)?;
        for segment in segments {
            match &value.value_type {
                Some(ValueType::MapValue(map)) => value = map.fields.get(segment)?,
                _ => return None,
            }
        }
        Some(format!("{:?}", value.value_type))
    }

    #[test]
    fn a_read_mask_selects_a_nested_field_without_flattening_it() {
        let mut document = nested_document();
        apply_mask(
            &mut document,
            &Some(DocumentMask {
                field_paths: vec!["address.city".to_string()],
            }),
        )
        .expect("a nested read mask is served");

        assert!(
            at(&document, "address.city").is_some(),
            "the selected leaf stays where it was: {:?}",
            document.fields
        );
        assert_eq!(
            at(&document, "address.zip"),
            None,
            "an unselected sibling inside the map is dropped"
        );
        assert_eq!(at(&document, "name"), None, "name was not selected");
        assert!(
            !document.fields.contains_key("address.city"),
            "the mask must not flatten the path into a field name"
        );
    }

    #[test]
    fn a_read_mask_selecting_a_whole_map_keeps_all_of_it() {
        let mut document = nested_document();
        apply_mask(
            &mut document,
            &Some(DocumentMask {
                field_paths: vec!["address".to_string()],
            }),
        )
        .expect("a top-level read mask is served");

        assert!(at(&document, "address.city").is_some());
        assert!(at(&document, "address.zip").is_some());
        assert_eq!(at(&document, "name"), None);
    }

    #[test]
    fn a_read_mask_path_that_is_not_there_selects_nothing() {
        let mut document = nested_document();
        apply_mask(
            &mut document,
            &Some(DocumentMask {
                // `name` is a string, so it has no `first` inside it.
                field_paths: vec!["address.country".to_string(), "name.first".to_string()],
            }),
        )
        .expect("an absent path is not an error");

        assert!(document.fields.is_empty(), "{:?}", document.fields);
    }

    #[test]
    fn a_malformed_mask_path_is_an_invalid_argument() {
        let err = apply_mask(
            &mut nested_document(),
            &Some(DocumentMask {
                field_paths: vec!["address..city".to_string()],
            }),
        )
        .expect_err("a malformed path is rejected");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn a_nested_update_mask_is_still_refused() {
        let err = masked_fields(&["address.city".to_string()], &nested_document())
            .expect_err("a nested update mask is not served");
        assert_eq!(
            err.code(),
            tonic::Code::Unimplemented,
            "writing into a stored map is a write-path change, not a query one"
        );
    }

    #[test]
    fn a_top_level_update_mask_selects_the_whole_field() {
        let fields = masked_fields(&["address".to_string()], &nested_document())
            .expect("a top-level update mask is served");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "address");
        assert!(
            fields[0].1.is_some(),
            "the document carries the field, so it is written rather than deleted"
        );
    }

    #[test]
    fn a_masked_path_the_document_lacks_becomes_a_deletion() {
        let fields = masked_fields(
            &["address".to_string(), "nickname".to_string()],
            &nested_document(),
        )
        .expect("a top-level update mask is served");
        assert_eq!(
            fields
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            vec!["address", "nickname"],
            "the result keeps mask order"
        );
        assert!(
            fields[1].1.is_none(),
            "an absent masked field is a deletion"
        );
    }
}
