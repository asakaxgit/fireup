use thiserror::Error;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{error, warn};

/// Main error type for the Fireup system
#[derive(Error, Debug)]
pub enum FireupError {
    // Parse errors
    #[error("LevelDB parse error: {message}")]
    LevelDBParse { 
        message: String, 
        context: ErrorContext,
        suggestions: Vec<String>,
    },
    
    #[error("Document parse error: {message}")]
    DocumentParse { 
        message: String, 
        document_path: Option<String>,
        context: ErrorContext,
        suggestions: Vec<String>,
    },
    
    #[error("Backup validation error: {message}")]
    BackupValidation { 
        message: String, 
        file_path: String,
        context: ErrorContext,
        suggestions: Vec<String>,
    },
    
    // Schema analysis errors
    #[error("Schema analysis error: {message}")]
    SchemaAnalysis { 
        message: String, 
        collection: Option<String>,
        field_path: Option<String>,
        context: ErrorContext,
        suggestions: Vec<String>,
    },
    
    #[error("Type conflict error: {message}")]
    TypeConflict { 
        message: String, 
        field_path: String,
        conflicting_types: Vec<String>,
        context: ErrorContext,
        suggestions: Vec<String>,
    },
    
    #[error("Type mapping error: {0}")]
    TypeMapping(String),
    
    #[error("Normalization error: {message}")]
    Normalization { 
        message: String, 
        table: Option<String>,
        context: ErrorContext,
        suggestions: Vec<String>,
    },
    
    // Import errors
    #[error("Database connection error: {message}")]
    DatabaseConnection { 
        message: String, 
        connection_string: Option<String>,
        context: ErrorContext,
        suggestions: Vec<String>,
    },
    
    #[error("Data import error: {message}")]
    DataImport { 
        message: String, 
        table: Option<String>,
        batch_info: Option<BatchInfo>,
        context: ErrorContext,
        suggestions: Vec<String>,
    },
    
    #[error("Constraint violation: {message}")]
    ConstraintViolation { 
        message: String, 
        table: String,
        constraint: String,
        violating_data: Option<String>,
        context: ErrorContext,
        suggestions: Vec<String>,
    },
    
    #[error("Transaction error: {message}")]
    Transaction { 
        message: String, 
        operation: String,
        context: ErrorContext,
        suggestions: Vec<String>,
    },
    
    // System errors
    #[error("Configuration error: {message}")]
    Configuration { 
        message: String, 
        config_key: Option<String>,
        context: ErrorContext,
        suggestions: Vec<String>,
    },
    
    #[error("Resource error: {message}")]
    Resource { 
        message: String, 
        resource_type: String,
        context: ErrorContext,
        suggestions: Vec<String>,
    },
    
    #[error("Performance error: {message}")]
    Performance { 
        message: String, 
        operation: String,
        metrics: Option<PerformanceMetrics>,
        context: ErrorContext,
        suggestions: Vec<String>,
    },
    
    // External errors
    #[error("Database error: {0}")]
    Database(#[from] tokio_postgres::Error),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    
    #[error("UUID error: {0}")]
    Uuid(#[from] uuid::Error),
}

/// Context information for errors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorContext {
    /// Operation being performed when error occurred
    pub operation: String,
    /// Additional context data
    pub metadata: HashMap<String, String>,
    /// Timestamp when error occurred
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Stack trace or call path
    pub call_path: Vec<String>,
}

/// Information about batch processing when errors occur
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchInfo {
    /// Current batch number
    pub batch_number: u32,
    /// Total number of batches
    pub total_batches: u32,
    /// Number of records in current batch
    pub batch_size: u32,
    /// Number of successfully processed records
    pub processed_records: u32,
    /// Number of failed records
    pub failed_records: u32,
}

/// Performance metrics for error context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    /// Duration of the operation in milliseconds
    pub duration_ms: u64,
    /// Memory usage in bytes
    pub memory_usage_bytes: Option<u64>,
    /// CPU usage percentage
    pub cpu_usage_percent: Option<f64>,
    /// Number of records processed
    pub records_processed: Option<u64>,
    /// Throughput (records per second)
    pub throughput: Option<f64>,
}

/// Error response for API/CLI output
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// Error type identifier
    pub error_type: String,
    /// Human-readable error message
    pub message: String,
    /// Additional context information
    pub context: Option<ErrorContext>,
    /// Suggested actions to resolve the error
    pub suggestions: Vec<String>,
    /// Error severity level
    pub severity: ErrorSeverity,
    /// Unique error ID for tracking
    pub error_id: String,
}

/// Error severity levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ErrorSeverity {
    /// Informational - operation can continue
    Info,
    /// Warning - operation can continue but with potential issues
    Warning,
    /// Error - operation failed but system is stable
    Error,
    /// Critical - system stability may be compromised
    Critical,
}

impl FireupError {
    /// Create a new error context
    pub fn new_context(operation: impl Into<String>) -> ErrorContext {
        ErrorContext {
            operation: operation.into(),
            metadata: HashMap::new(),
            timestamp: chrono::Utc::now(),
            call_path: Vec::new(),
        }
    }
    
    /// Create a LevelDB parse error
    pub fn leveldb_parse(message: impl Into<String>, context: ErrorContext) -> Self {
        let suggestions = vec![
            "Verify the backup file is a valid LevelDB format".to_string(),
            "Check if the file is corrupted or incomplete".to_string(),
            "Ensure you have read permissions for the file".to_string(),
        ];
        
        Self::LevelDBParse { 
            message: message.into(), 
            context,
            suggestions,
        }
    }
    
    /// Create a document parse error
    pub fn document_parse(message: impl Into<String>, document_path: Option<String>, context: ErrorContext) -> Self {
        let suggestions = vec![
            "Check if the document structure is valid Firestore format".to_string(),
            "Verify all required fields are present".to_string(),
            "Review the document for unsupported data types".to_string(),
        ];
        
        Self::DocumentParse { 
            message: message.into(), 
            document_path,
            context,
            suggestions,
        }
    }
    
    /// Create a backup validation error
    pub fn backup_validation(message: impl Into<String>, file_path: impl Into<String>, context: ErrorContext) -> Self {
        let suggestions = vec![
            "Verify the backup file integrity".to_string(),
            "Check if the backup was created successfully".to_string(),
            "Try using a different backup file".to_string(),
        ];
        
        Self::BackupValidation { 
            message: message.into(), 
            file_path: file_path.into(),
            context,
            suggestions,
        }
    }
    
    /// Create a schema analysis error
    pub fn schema_analysis(message: impl Into<String>, collection: Option<String>, field_path: Option<String>, context: ErrorContext) -> Self {
        let suggestions = vec![
            "Review the document structure for inconsistencies".to_string(),
            "Check for unsupported field types".to_string(),
            "Consider manual schema definition for complex cases".to_string(),
        ];
        
        Self::SchemaAnalysis { 
            message: message.into(), 
            collection,
            field_path,
            context,
            suggestions,
        }
    }
    
    /// Create a type conflict error
    pub fn type_conflict(message: impl Into<String>, field_path: impl Into<String>, conflicting_types: Vec<String>, context: ErrorContext) -> Self {
        let suggestions = vec![
            "Choose the most common type for the field".to_string(),
            "Use a union type or JSONB for mixed types".to_string(),
            "Consider data cleaning before import".to_string(),
        ];
        
        Self::TypeConflict { 
            message: message.into(), 
            field_path: field_path.into(),
            conflicting_types,
            context,
            suggestions,
        }
    }
    
    /// Create a database connection error
    pub fn database_connection(message: impl Into<String>, connection_string: Option<String>, context: ErrorContext) -> Self {
        let suggestions = vec![
            "Verify PostgreSQL server is running".to_string(),
            "Check connection string format and credentials".to_string(),
            "Ensure network connectivity to the database".to_string(),
            "Verify database exists and user has proper permissions".to_string(),
        ];
        
        Self::DatabaseConnection { 
            message: message.into(), 
            connection_string,
            context,
            suggestions,
        }
    }
    
    /// Create a data import error
    pub fn data_import(message: impl Into<String>, table: Option<String>, batch_info: Option<BatchInfo>, context: ErrorContext) -> Self {
        let suggestions = vec![
            "Check for constraint violations in the data".to_string(),
            "Verify table schema matches the data structure".to_string(),
            "Review batch size and memory usage".to_string(),
            "Check for duplicate primary keys".to_string(),
        ];
        
        Self::DataImport { 
            message: message.into(), 
            table,
            batch_info,
            context,
            suggestions,
        }
    }
    
    /// Create a constraint violation error
    pub fn constraint_violation(
        message: impl Into<String>, 
        table: impl Into<String>, 
        constraint: impl Into<String>,
        violating_data: Option<String>,
        context: ErrorContext
    ) -> Self {
        let suggestions = vec![
            "Review the violating data and fix inconsistencies".to_string(),
            "Check if the constraint is appropriate for the data".to_string(),
            "Consider modifying the constraint or cleaning the data".to_string(),
        ];
        
        Self::ConstraintViolation { 
            message: message.into(), 
            table: table.into(),
            constraint: constraint.into(),
            violating_data,
            context,
            suggestions,
        }
    }
    
    /// Convert error to response format
    pub fn to_response(&self) -> ErrorResponse {
        let error_id = uuid::Uuid::new_v4().to_string();
        
        match self {
            FireupError::LevelDBParse { message, context, suggestions } => {
                ErrorResponse {
                    error_type: "LEVELDB_PARSE_ERROR".to_string(),
                    message: message.clone(),
                    context: Some(context.clone()),
                    suggestions: suggestions.clone(),
                    severity: ErrorSeverity::Error,
                    error_id,
                }
            }
            FireupError::TypeConflict { message, context, suggestions, .. } => {
                ErrorResponse {
                    error_type: "TYPE_CONFLICT".to_string(),
                    message: message.clone(),
                    context: Some(context.clone()),
                    suggestions: suggestions.clone(),
                    severity: ErrorSeverity::Warning,
                    error_id,
                }
            }
            FireupError::DatabaseConnection { message, context, suggestions, .. } => {
                ErrorResponse {
                    error_type: "DATABASE_CONNECTION_ERROR".to_string(),
                    message: message.clone(),
                    context: Some(context.clone()),
                    suggestions: suggestions.clone(),
                    severity: ErrorSeverity::Critical,
                    error_id,
                }
            }
            FireupError::ConstraintViolation { message, context, suggestions, .. } => {
                ErrorResponse {
                    error_type: "CONSTRAINT_VIOLATION".to_string(),
                    message: message.clone(),
                    context: Some(context.clone()),
                    suggestions: suggestions.clone(),
                    severity: ErrorSeverity::Error,
                    error_id,
                }
            }
            _ => {
                ErrorResponse {
                    error_type: "GENERAL_ERROR".to_string(),
                    message: self.to_string(),
                    context: None,
                    suggestions: vec!["Check logs for more details".to_string()],
                    severity: ErrorSeverity::Error,
                    error_id,
                }
            }
        }
    }
    
    /// Log the error with appropriate level
    pub fn log(&self) {
        match self {
            FireupError::TypeConflict { message, field_path, .. } => {
                warn!(
                    error = %self,
                    field_path = %field_path,
                    "Type conflict detected: {}", message
                );
            }
            FireupError::DatabaseConnection { message, .. } => {
                error!(
                    error = %self,
                    "Database connection failed: {}", message
                );
            }
            FireupError::ConstraintViolation { message, table, constraint, .. } => {
                error!(
                    error = %self,
                    table = %table,
                    constraint = %constraint,
                    "Constraint violation: {}", message
                );
            }
            FireupError::Performance { message, operation, metrics, .. } => {
                warn!(
                    error = %self,
                    operation = %operation,
                    metrics = ?metrics,
                    "Performance issue: {}", message
                );
            }
            _ => {
                error!(error = %self, "Fireup error occurred");
            }
        }
    }
}

impl ErrorContext {
    /// Add metadata to the context
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
    
    /// Add a call path entry
    pub fn with_call_path(mut self, path: impl Into<String>) -> Self {
        self.call_path.push(path.into());
        self
    }
}

impl BatchInfo {
    /// Create new batch info
    pub fn new(batch_number: u32, total_batches: u32, batch_size: u32) -> Self {
        Self {
            batch_number,
            total_batches,
            batch_size,
            processed_records: 0,
            failed_records: 0,
        }
    }
    
    /// Record a successful record processing
    pub fn record_success(&mut self) {
        self.processed_records += 1;
    }
    
    /// Record a failed record processing
    pub fn record_failure(&mut self) {
        self.failed_records += 1;
    }
    
    /// Calculate success rate
    pub fn success_rate(&self) -> f64 {
        if self.processed_records + self.failed_records == 0 {
            0.0
        } else {
            self.processed_records as f64 / (self.processed_records + self.failed_records) as f64
        }
    }
}

impl PerformanceMetrics {
    /// Create new performance metrics
    pub fn new(duration_ms: u64) -> Self {
        Self {
            duration_ms,
            memory_usage_bytes: None,
            cpu_usage_percent: None,
            records_processed: None,
            throughput: None,
        }
    }
    
    /// Calculate throughput if records processed is set
    pub fn calculate_throughput(&mut self) {
        if let Some(records) = self.records_processed {
            if self.duration_ms > 0 {
                self.throughput = Some(records as f64 / (self.duration_ms as f64 / 1000.0));
            }
        }
    }
}

/// Result type alias for Fireup operations
pub type FireupResult<T> = Result<T, FireupError>;

/// Macro for creating error context with current function name
#[macro_export]
macro_rules! error_context {
    ($operation:expr) => {
        $crate::error::FireupError::new_context($operation)
            .with_call_path(format!("{}::{}", module_path!(), $operation))
    };
}

/// Macro for logging and returning errors
#[macro_export]
macro_rules! log_and_return_error {
    ($error:expr) => {{
        let error = $error;
        error.log();
        Err(error)
    }};
}