use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Core data structure representing a Firestore document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirestoreDocument {
    /// Document ID
    pub id: String,
    /// Collection name
    pub collection: String,
    /// Document data as key-value pairs
    pub data: HashMap<String, serde_json::Value>,
    /// Nested subcollections
    pub subcollections: Vec<FirestoreDocument>,
    /// Document metadata
    pub metadata: DocumentMetadata,
    /// Full Datastore key (namespace + ancestor path) when the source carried one.
    ///
    /// This is the document's natural identity and is what the generated schema
    /// keys rows on, so re-importing the same export is idempotent.
    #[serde(default)]
    pub key: Option<DatastoreKey>,
}

/// Name of the generated column holding the namespace-qualified Datastore key path.
pub const KEY_PATH_COLUMN: &str = "key_path";
/// Name of the generated column holding the Datastore namespace.
pub const KEY_NAMESPACE_COLUMN: &str = "key_namespace";
/// Name of the generated column holding the leaf kind of the Datastore key.
pub const KEY_KIND_COLUMN: &str = "key_kind";
/// Name of the generated column holding a string key name (NULL for numeric ids).
pub const KEY_NAME_COLUMN: &str = "key_name";
/// Name of the generated column holding a numeric key id (NULL for string names).
pub const KEY_ID_COLUMN: &str = "key_id";
/// Name of the generated column holding the parent entity's key path.
pub const PARENT_KEY_PATH_COLUMN: &str = "parent_key_path";

/// PostgreSQL's `NAMEDATALEN - 1`: identifiers past this many **bytes** are
/// truncated, with a notice rather than an error.
pub const MAX_IDENTIFIER_BYTES: usize = 63;

/// Shorten `name` so PostgreSQL stores it whole, keeping distinct names distinct.
///
/// PostgreSQL truncates a long identifier silently. Two Datastore property
/// names that agree on their first 63 bytes therefore become one column: the
/// `CREATE TABLE` either fails with a duplicate column or one of the fields is
/// quietly dropped. Appending a digest of the full name keeps them apart, and
/// the digest is derived only from the name, so the generated DDL does not
/// depend on the order fields happened to be analyzed in.
///
/// Names that already fit are returned untouched, so ordinary schemas read
/// exactly as before.
pub fn postgres_identifier(name: &str) -> String {
    if name.len() <= MAX_IDENTIFIER_BYTES {
        return name.to_string();
    }
    let digest = format!("{:08x}", fnv1a_32(name.as_bytes()));
    let head = truncate_to_bytes(name, MAX_IDENTIFIER_BYTES - digest.len() - 1);
    format!("{head}_{digest}")
}

/// Longest prefix of `text` that is at most `limit` bytes and a valid `str`.
fn truncate_to_bytes(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// FNV-1a, written out rather than taken from `DefaultHasher` because the
/// standard hasher's output is explicitly not stable across Rust releases, and
/// a generated column name has to be reproducible.
fn fnv1a_32(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c_9dc5u32;
    for &byte in bytes {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Identity of a Datastore/Firestore entity: its namespace plus full ancestor path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatastoreKey {
    /// Partition namespace; empty string for the default namespace.
    pub namespace: String,
    /// Ancestor path, root first. The last element identifies this entity.
    pub path: Vec<DatastoreKeyPathElement>,
}

/// One `kind`/`id` pair of a Datastore key path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatastoreKeyPathElement {
    /// Entity kind (the collection name at this level).
    pub kind: String,
    /// Either an application-assigned name or an auto-allocated numeric id.
    pub id: DatastoreKeyId,
}

/// The identifier half of a Datastore key path element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatastoreKeyId {
    /// Application-assigned string name.
    Name(String),
    /// Auto-allocated 64-bit id. These routinely exceed `int32`.
    Id(i64),
}

impl DatastoreKeyId {
    /// Render the identifier as it appears in a key path.
    pub fn as_path_segment(&self) -> String {
        match self {
            DatastoreKeyId::Name(name) => name.clone(),
            DatastoreKeyId::Id(id) => id.to_string(),
        }
    }
}

impl DatastoreKey {
    /// Create a root-level key in the default namespace.
    pub fn root(kind: impl Into<String>, id: DatastoreKeyId) -> Self {
        Self {
            namespace: String::new(),
            path: vec![DatastoreKeyPathElement {
                kind: kind.into(),
                id,
            }],
        }
    }

    /// Leaf kind — the kind of the entity this key identifies.
    pub fn kind(&self) -> Option<&str> {
        self.path.last().map(|element| element.kind.as_str())
    }

    /// Leaf string key name, if the entity is named rather than numbered.
    pub fn name(&self) -> Option<&str> {
        match self.path.last().map(|element| &element.id) {
            Some(DatastoreKeyId::Name(name)) => Some(name.as_str()),
            _ => None,
        }
    }

    /// Leaf numeric key id, if the entity has an auto-allocated id.
    pub fn numeric_id(&self) -> Option<i64> {
        match self.path.last().map(|element| &element.id) {
            Some(DatastoreKeyId::Id(id)) => Some(*id),
            _ => None,
        }
    }

    /// Path without the namespace prefix, e.g. `Users/ada/Orders/1001`.
    pub fn path_string(&self) -> String {
        self.path
            .iter()
            .map(|element| {
                format!(
                    "{}/{}",
                    Self::render_segment(&element.kind),
                    Self::render_id(&element.id)
                )
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Render the identifier half of a path element.
    ///
    /// An int64 id renders as bare digits — its canonical form. A string name
    /// renders as itself unless that would be ambiguous, because
    /// `(kind, id 1001)` and `(kind, name "1001")` are **different entities**
    /// and [`Self::encoded_path`] is the generated primary key.
    fn render_id(id: &DatastoreKeyId) -> String {
        match id {
            DatastoreKeyId::Id(value) => value.to_string(),
            DatastoreKeyId::Name(name) => Self::render_segment(name),
        }
    }

    /// Render one path segment so that distinct segments never render alike.
    ///
    /// Segments that cannot be misread pass through untouched, so ordinary key
    /// paths (`Users/ada/Orders/1001`) look exactly as they always have. Two
    /// kinds of segment *can* be misread, and are double-quoted instead:
    ///
    /// * one that parses as an `i64` — otherwise the name `1001` collides with
    ///   the auto-allocated id `1001`
    /// * one containing `/`, `"` or `\` — otherwise the name `a/Orders/9` under
    ///   kind `Users` collides with the path `Users/a` → `Orders/9`
    ///
    /// Quoted segments always start with `"` and unquoted ones never contain
    /// `/` or `"`, so the rendering stays injective and splittable. The same
    /// rule applies to kinds, which means an all-numeric kind is quoted too;
    /// that is unnecessary but harmless, and one rule is easier to trust than
    /// two.
    fn render_segment(segment: &str) -> String {
        if Self::segment_needs_quoting(segment) {
            let escaped = segment.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{escaped}\"")
        } else {
            segment.to_string()
        }
    }

    fn segment_needs_quoting(segment: &str) -> bool {
        segment.is_empty()
            || segment.contains('/')
            || segment.contains('"')
            || segment.contains('\\')
            || segment.parse::<i64>().is_ok()
    }

    /// Canonical, namespace-qualified key path. This is the generated natural key.
    ///
    /// A non-default namespace is prefixed as `[namespace]`, so keys from
    /// different namespaces never collide in a single column.
    pub fn encoded_path(&self) -> String {
        Self::encode(&self.namespace, &self.path_string())
    }

    /// Encoded path of the parent entity, or `None` for a root entity.
    pub fn parent_encoded_path(&self) -> Option<String> {
        if self.path.len() < 2 {
            return None;
        }
        let parent = DatastoreKey {
            namespace: self.namespace.clone(),
            path: self.path[..self.path.len() - 1].to_vec(),
        };
        Some(parent.encoded_path())
    }

    fn encode(namespace: &str, path: &str) -> String {
        if namespace.is_empty() {
            path.to_string()
        } else {
            format!("[{}]{}", namespace, path)
        }
    }
}

/// Object key marking a value whose Datastore type JSON cannot represent natively.
///
/// Blobs, timestamps, geo points and key references are decoded into tagged
/// objects so the schema analyzer can map them to `BYTEA`, `TIMESTAMPTZ`,
/// `POINT` and `TEXT` instead of guessing from a stringified form.
pub const DATASTORE_TYPE_TAG: &str = "__fireup_type__";

/// Tag value for a blob; the bytes live under `base64`.
pub const DATASTORE_TYPE_BLOB: &str = "blob";
/// Tag value for a timestamp; the instant lives under `rfc3339`.
pub const DATASTORE_TYPE_TIMESTAMP: &str = "timestamp";
/// Tag value for a geo point; coordinates live under `latitude`/`longitude`.
pub const DATASTORE_TYPE_GEO_POINT: &str = "geo_point";
/// Tag value for a key reference; the encoded path lives under `key_path`.
pub const DATASTORE_TYPE_KEY: &str = "key";
/// Tag value for a non-finite double; the literal lives under `literal`.
pub const DATASTORE_TYPE_DOUBLE: &str = "double";

/// Read the Datastore type tag of a value, if it carries one.
pub fn datastore_type_tag(value: &serde_json::Value) -> Option<&str> {
    value
        .as_object()?
        .get(DATASTORE_TYPE_TAG)?
        .as_str()
}

/// Build a tagged blob value from raw bytes.
pub fn tagged_blob(base64: String) -> serde_json::Value {
    serde_json::json!({ DATASTORE_TYPE_TAG: DATASTORE_TYPE_BLOB, "base64": base64 })
}

/// Build a tagged timestamp value from an RFC 3339 instant.
pub fn tagged_timestamp(rfc3339: String) -> serde_json::Value {
    serde_json::json!({ DATASTORE_TYPE_TAG: DATASTORE_TYPE_TIMESTAMP, "rfc3339": rfc3339 })
}

/// Build a tagged geo point value.
pub fn tagged_geo_point(latitude: f64, longitude: f64) -> serde_json::Value {
    serde_json::json!({
        DATASTORE_TYPE_TAG: DATASTORE_TYPE_GEO_POINT,
        "latitude": latitude,
        "longitude": longitude,
    })
}

/// Build a tagged value for a double JSON cannot hold: `NaN` or `±Infinity`.
///
/// Datastore stores all three in an ordinary double property, and PostgreSQL
/// accepts all three as `numeric`/`double precision` input — it is only JSON
/// that has no literal for them, so `serde_json::Number::from_f64` returns
/// `None`. Tagging keeps the value instead of dropping it: nulling it is silent
/// data loss, and failing the record aborts an otherwise fine import.
///
/// The spellings are PostgreSQL's own input spellings, which are also the ones
/// `local::value_json` uses on the emulator side.
///
/// Returns `None` for a finite double, which belongs in a plain JSON number.
pub fn tagged_non_finite_double(value: f64) -> Option<serde_json::Value> {
    let literal = if value.is_nan() {
        "NaN"
    } else if value == f64::INFINITY {
        "Infinity"
    } else if value == f64::NEG_INFINITY {
        "-Infinity"
    } else {
        return None;
    };
    Some(serde_json::json!({ DATASTORE_TYPE_TAG: DATASTORE_TYPE_DOUBLE, "literal": literal }))
}

/// Build a tagged key-reference value from an encoded key path.
pub fn tagged_key_reference(key_path: String) -> serde_json::Value {
    serde_json::json!({ DATASTORE_TYPE_TAG: DATASTORE_TYPE_KEY, "key_path": key_path })
}

/// Metadata associated with a Firestore document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    /// Document creation timestamp
    pub created_at: Option<DateTime<Utc>>,
    /// Document last update timestamp
    pub updated_at: Option<DateTime<Utc>>,
    /// Document path in Firestore hierarchy
    pub path: String,
    /// Document size in bytes
    pub size_bytes: Option<u64>,
}

/// PostgreSQL table definition for normalized schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableDefinition {
    /// Table name
    pub name: String,
    /// Column definitions
    pub columns: Vec<ColumnDefinition>,
    /// Primary key definition
    pub primary_key: Option<PrimaryKeyDefinition>,
    /// Foreign key definitions
    pub foreign_keys: Vec<ForeignKeyDefinition>,
    /// Index definitions
    pub indexes: Vec<IndexDefinition>,
}

/// Primary key definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrimaryKeyDefinition {
    /// Primary key constraint name
    pub name: String,
    /// Column names that make up the primary key
    pub columns: Vec<String>,
}

/// PostgreSQL column definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDefinition {
    /// Column name
    pub name: String,
    /// PostgreSQL data type
    pub column_type: PostgreSQLType,
    /// Whether column allows NULL values
    pub nullable: bool,
    /// Default value for the column
    pub default_value: Option<serde_json::Value>,
    /// Additional constraints
    pub constraints: Vec<String>,
    /// Document field this column was generated from, when it differs from
    /// [`Self::name`].
    ///
    /// A column name is *derived* from a property name — dots become
    /// underscores, over-long names are shortened by [`postgres_identifier`] —
    /// so the name alone cannot be used to look the value back up. A Datastore
    /// property called `meta.source` becomes the column `meta_source`, and
    /// searching the document for `meta_source` finds nothing.
    #[serde(default)]
    pub source_field: Option<String>,
}

/// PostgreSQL data types supported by the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PostgreSQLType {
    /// Variable character with optional length
    Varchar(Option<u32>),
    /// Unlimited text
    Text,
    /// Integer (32-bit)
    Integer,
    /// Big integer (64-bit)
    BigInt,
    /// Raw bytes
    Bytea,
    /// Geometric point (x = longitude, y = latitude)
    Point,
    /// Numeric with optional precision and scale
    Numeric(Option<u32>, Option<u32>),
    /// Boolean
    Boolean,
    /// Timestamp with timezone
    Timestamp,
    /// UUID
    Uuid,
    /// JSON Binary
    Jsonb,
    /// Array of another type
    Array(Box<PostgreSQLType>),
}

/// Foreign key relationship definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKeyDefinition {
    /// Local column name
    pub column: String,
    /// Referenced table name
    pub referenced_table: String,
    /// Referenced column name
    pub referenced_column: String,
    /// Foreign key constraint name
    pub constraint_name: String,
}

/// Index definition for performance optimization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDefinition {
    /// Index name
    pub name: String,
    /// Columns included in the index
    pub columns: Vec<String>,
    /// Whether index is unique
    pub unique: bool,
    /// Index type (btree, hash, etc.)
    pub index_type: Option<String>,
}

/// Result of schema analysis process
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaAnalysis {
    /// Analysis of each collection
    pub collections: Vec<CollectionAnalysis>,
    /// Field type analysis across all documents
    pub field_types: Vec<FieldTypeAnalysis>,
    /// Detected relationships between collections
    pub relationships: Vec<DetectedRelationship>,
    /// Opportunities for normalization
    pub normalization_opportunities: Vec<NormalizationOpportunity>,
    /// Analysis metadata
    pub metadata: AnalysisMetadata,
}

/// Analysis of a specific collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionAnalysis {
    /// Collection name
    pub name: String,
    /// Number of documents analyzed
    pub document_count: u64,
    /// Unique field names found
    pub field_names: Vec<String>,
    /// Average document size in bytes
    pub avg_document_size: f64,
    /// Nested subcollections found
    pub subcollections: Vec<String>,
}

/// Analysis of field types across documents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldTypeAnalysis {
    /// Field path (e.g., "user.profile.name")
    pub field_path: String,
    /// Detected data types and their frequencies
    pub type_frequencies: HashMap<String, u32>,
    /// Total occurrences of this field
    pub total_occurrences: u32,
    /// Percentage of documents containing this field
    pub presence_percentage: f64,
    /// Recommended PostgreSQL type
    pub recommended_type: PostgreSQLType,
}

/// Detected relationship between collections
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedRelationship {
    /// Source collection name
    pub from_collection: String,
    /// Target collection name
    pub to_collection: String,
    /// Field name containing the reference
    pub reference_field: String,
    /// Type of relationship (one-to-one, one-to-many, many-to-many)
    pub relationship_type: RelationshipType,
    /// Confidence score (0.0 to 1.0)
    pub confidence: f64,
}

/// Types of relationships between collections
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RelationshipType {
    OneToOne,
    OneToMany,
    ManyToOne,
    ManyToMany,
}

/// Opportunity for database normalization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizationOpportunity {
    /// Collection name where opportunity was found
    pub collection: String,
    /// Field path that can be normalized
    pub field_path: String,
    /// Type of normalization (1NF, 2NF, 3NF)
    pub normalization_type: NormalizationType,
    /// Description of the opportunity
    pub description: String,
    /// Estimated impact on performance
    pub impact: NormalizationImpact,
}

/// Types of database normalization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NormalizationType {
    /// First Normal Form - eliminate repeating groups
    FirstNormalForm,
    /// Second Normal Form - eliminate partial dependencies
    SecondNormalForm,
    /// Third Normal Form - eliminate transitive dependencies
    ThirdNormalForm,
}

/// Impact assessment for normalization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NormalizationImpact {
    Low,
    Medium,
    High,
}

/// Type conflict detected during analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeConflict {
    /// Field path where conflict was detected
    pub field_path: String,
    /// Conflicting data types found
    pub conflicting_types: Vec<String>,
    /// Number of occurrences of each type
    pub type_occurrences: HashMap<String, u32>,
    /// Total number of documents with this field
    pub total_occurrences: u32,
    /// Suggested resolution strategy
    pub suggested_resolution: String,
    /// Confidence in the suggested resolution
    pub resolution_confidence: f64,
}

/// Metadata about the schema analysis process
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisMetadata {
    /// Total number of documents analyzed
    pub total_documents: u64,
    /// Total number of collections analyzed
    pub total_collections: u32,
    /// Analysis start time
    pub analysis_start: DateTime<Utc>,
    /// Analysis completion time
    pub analysis_end: Option<DateTime<Utc>>,
    /// Version of the analyzer used
    pub analyzer_version: String,
}

/// Normalized schema containing all table definitions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedSchema {
    /// Table definitions
    pub tables: Vec<TableDefinition>,
    /// Relationships between tables
    pub relationships: Vec<Relationship>,
    /// Constraints to be applied
    pub constraints: Vec<Constraint>,
    /// Warnings about the schema
    pub warnings: Vec<SchemaWarning>,
    /// Schema metadata
    pub metadata: SchemaMetadata,
}

/// Relationship between normalized tables
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    /// Source table name
    pub from_table: String,
    /// Target table name
    pub to_table: String,
    /// Foreign key column in source table
    pub from_column: String,
    /// Referenced column in target table
    pub to_column: String,
    /// Relationship type
    pub relationship_type: RelationshipType,
}

/// Database constraint definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constraint {
    /// Constraint name
    pub name: String,
    /// Table the constraint applies to
    pub table: String,
    /// Type of constraint
    pub constraint_type: ConstraintType,
    /// Columns involved in the constraint
    pub columns: Vec<String>,
    /// Additional constraint parameters
    pub parameters: HashMap<String, String>,
}

/// Types of database constraints
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConstraintType {
    NotNull,
    Unique,
    Check,
    PrimaryKey,
    ForeignKey,
}

/// Warning about schema generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaWarning {
    /// Warning severity level
    pub level: WarningLevel,
    /// Warning message
    pub message: String,
    /// Table or field the warning relates to
    pub context: String,
    /// Suggested action to resolve the warning
    pub suggestion: Option<String>,
}

/// Warning severity levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WarningLevel {
    Info,
    Warning,
    Error,
}

/// Metadata about the normalized schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaMetadata {
    /// Schema generation timestamp
    pub generated_at: DateTime<Utc>,
    /// Source analysis used for generation
    pub source_analysis_id: Uuid,
    /// Schema version
    pub version: String,
    /// Total number of tables generated
    pub table_count: u32,
    /// Total number of relationships
    pub relationship_count: u32,
}

impl FirestoreDocument {
    /// Create a new Firestore document
    pub fn new(id: String, collection: String, path: String) -> Self {
        Self {
            id,
            collection,
            data: HashMap::new(),
            subcollections: Vec::new(),
            metadata: DocumentMetadata {
                created_at: None,
                updated_at: None,
                path,
                size_bytes: None,
            },
            key: None,
        }
    }

    /// Attach the document's Datastore key.
    pub fn with_key(mut self, key: DatastoreKey) -> Self {
        self.key = Some(key);
        self
    }

    /// Natural key for this document: the encoded Datastore key path when the
    /// source carried one, otherwise the collection-qualified document path.
    pub fn natural_key(&self) -> String {
        match &self.key {
            Some(key) => key.encoded_path(),
            None => format!("{}/{}", self.collection, self.id),
        }
    }
    
    /// Add data to the document
    pub fn add_field(&mut self, key: String, value: serde_json::Value) {
        self.data.insert(key, value);
    }
    
    /// Add a subcollection document
    pub fn add_subcollection(&mut self, document: FirestoreDocument) {
        self.subcollections.push(document);
    }
    
    /// Get the full document path
    pub fn full_path(&self) -> String {
        format!("{}/{}", self.collection, self.id)
    }
}

impl TableDefinition {
    /// Create a new table definition
    pub fn new(name: String) -> Self {
        Self {
            name,
            columns: Vec::new(),
            primary_key: None,
            foreign_keys: Vec::new(),
            indexes: Vec::new(),
        }
    }
    
    /// Add a column to the table
    pub fn add_column(&mut self, column: ColumnDefinition) {
        self.columns.push(column);
    }
    
    /// Set the primary key
    pub fn set_primary_key(&mut self, primary_key: PrimaryKeyDefinition) {
        self.primary_key = Some(primary_key);
    }
    
    /// Add a foreign key relationship
    pub fn add_foreign_key(&mut self, foreign_key: ForeignKeyDefinition) {
        self.foreign_keys.push(foreign_key);
    }
    
    /// Add an index
    pub fn add_index(&mut self, index: IndexDefinition) {
        self.indexes.push(index);
    }
}

impl ColumnDefinition {
    /// Create a new column definition
    pub fn new(name: String, column_type: PostgreSQLType) -> Self {
        Self {
            name,
            column_type,
            nullable: true,
            default_value: None,
            constraints: Vec::new(),
            source_field: None,
        }
    }

    /// Record the document field this column holds, for the importer to read
    /// values back out by. See [`Self::source_field`].
    pub fn from_field(mut self, field_path: impl Into<String>) -> Self {
        self.source_field = Some(field_path.into());
        self
    }

    /// Document field whose value belongs in this column.
    pub fn field_path(&self) -> &str {
        self.source_field.as_deref().unwrap_or(&self.name)
    }

    /// Set the column as not nullable
    pub fn not_null(mut self) -> Self {
        self.nullable = false;
        self
    }
    
    /// Set a default value for the column
    pub fn with_default(mut self, value: serde_json::Value) -> Self {
        self.default_value = Some(value);
        self
    }
    
    /// Add a constraint to the column
    pub fn add_constraint(mut self, constraint: String) -> Self {
        self.constraints.push(constraint);
        self
    }
}

impl PostgreSQLType {
    /// Convert to SQL type string
    pub fn to_sql(&self) -> String {
        match self {
            PostgreSQLType::Varchar(Some(len)) => format!("VARCHAR({})", len),
            PostgreSQLType::Varchar(None) => "VARCHAR".to_string(),
            PostgreSQLType::Text => "TEXT".to_string(),
            PostgreSQLType::Integer => "INTEGER".to_string(),
            PostgreSQLType::BigInt => "BIGINT".to_string(),
            PostgreSQLType::Numeric(Some(precision), Some(scale)) => {
                format!("NUMERIC({}, {})", precision, scale)
            }
            PostgreSQLType::Numeric(Some(precision), None) => {
                format!("NUMERIC({})", precision)
            }
            // A scale without a precision is not expressible in PostgreSQL.
            PostgreSQLType::Numeric(None, _) => "NUMERIC".to_string(),
            PostgreSQLType::Boolean => "BOOLEAN".to_string(),
            PostgreSQLType::Timestamp => "TIMESTAMPTZ".to_string(),
            PostgreSQLType::Uuid => "UUID".to_string(),
            PostgreSQLType::Jsonb => "JSONB".to_string(),
            PostgreSQLType::Bytea => "BYTEA".to_string(),
            PostgreSQLType::Point => "POINT".to_string(),
            PostgreSQLType::Array(inner_type) => format!("{}[]", inner_type.to_sql()),
        }
    }
}

impl SchemaAnalysis {
    /// Create a new schema analysis
    pub fn new() -> Self {
        Self {
            collections: Vec::new(),
            field_types: Vec::new(),
            relationships: Vec::new(),
            normalization_opportunities: Vec::new(),
            metadata: AnalysisMetadata {
                total_documents: 0,
                total_collections: 0,
                analysis_start: Utc::now(),
                analysis_end: None,
                analyzer_version: "0.1.0".to_string(),
            },
        }
    }
    
    /// Mark the analysis as complete
    pub fn complete(&mut self) {
        self.metadata.analysis_end = Some(Utc::now());
    }
    
    /// Add a collection analysis
    pub fn add_collection(&mut self, collection: CollectionAnalysis) {
        self.collections.push(collection);
        self.metadata.total_collections += 1;
    }
    
    /// Add field type analysis
    pub fn add_field_type(&mut self, field_type: FieldTypeAnalysis) {
        self.field_types.push(field_type);
    }
    
    /// Add detected relationship
    pub fn add_relationship(&mut self, relationship: DetectedRelationship) {
        self.relationships.push(relationship);
    }
    
    /// Add normalization opportunity
    pub fn add_normalization_opportunity(&mut self, opportunity: NormalizationOpportunity) {
        self.normalization_opportunities.push(opportunity);
    }
}

impl TypeConflict {
    /// Create a new type conflict
    pub fn new(field_path: String) -> Self {
        Self {
            field_path,
            conflicting_types: Vec::new(),
            type_occurrences: HashMap::new(),
            total_occurrences: 0,
            suggested_resolution: String::new(),
            resolution_confidence: 0.0,
        }
    }
    
    /// Add a type occurrence
    pub fn add_type_occurrence(&mut self, type_name: String) {
        *self.type_occurrences.entry(type_name.clone()).or_insert(0) += 1;
        self.total_occurrences += 1;
        
        if !self.conflicting_types.contains(&type_name) {
            self.conflicting_types.push(type_name);
        }
    }
    
    /// Calculate the dominant type (most frequent)
    pub fn dominant_type(&self) -> Option<String> {
        self.type_occurrences
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(type_name, _)| type_name.clone())
    }
    
    /// Calculate the percentage of the dominant type
    pub fn dominant_type_percentage(&self) -> f64 {
        if let Some(dominant) = self.dominant_type() {
            if let Some(count) = self.type_occurrences.get(&dominant) {
                (*count as f64 / self.total_occurrences as f64) * 100.0
            } else {
                0.0
            }
        } else {
            0.0
        }
    }
}