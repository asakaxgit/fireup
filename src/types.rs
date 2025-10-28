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
        }
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
            PostgreSQLType::Numeric(None, None) => "NUMERIC".to_string(),
            PostgreSQLType::Boolean => "BOOLEAN".to_string(),
            PostgreSQLType::Timestamp => "TIMESTAMP WITH TIME ZONE".to_string(),
            PostgreSQLType::Uuid => "UUID".to_string(),
            PostgreSQLType::Jsonb => "JSONB".to_string(),
            PostgreSQLType::Array(inner_type) => format!("{}[]", inner_type.to_sql()),
            _ => "TEXT".to_string(), // Fallback for unhandled cases
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