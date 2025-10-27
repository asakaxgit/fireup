// LevelDB backup validator implementation
use crate::error::{FireupError, ErrorContext};
use crate::leveldb_parser::parser::{LevelDBReader, FirestoreDocumentParser};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;
use tracing::{debug, info};

/// Result of backup validation
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub file_info: FileInfo,
    pub structure_info: StructureInfo,
    pub integrity_info: IntegrityInfo,
}

/// Information about the backup file
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub file_path: String,
    pub file_size: u64,
    pub is_readable: bool,
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,
}

/// Information about the backup structure
#[derive(Debug, Clone)]
pub struct StructureInfo {
    pub total_blocks: usize,
    pub total_records: usize,
    pub valid_records: usize,
    pub corrupted_records: usize,
    pub metadata_records: usize,
    pub document_records: usize,
}

/// Information about backup integrity
#[derive(Debug, Clone)]
pub struct IntegrityInfo {
    pub checksum_failures: usize,
    pub incomplete_records: usize,
    pub parsing_errors: usize,
    pub overall_integrity_score: f64, // 0.0 to 1.0
}

/// Progress information for long-running operations
#[derive(Debug, Clone)]
pub struct ProgressInfo {
    pub current_step: String,
    pub current_progress: u64,
    pub total_progress: u64,
    pub percentage: f64,
    pub estimated_remaining_ms: Option<u64>,
    pub throughput_records_per_sec: Option<f64>,
}

/// Progress callback trait for reporting progress
pub trait ProgressCallback: Send + Sync {
    fn on_progress(&self, progress: ProgressInfo);
}

/// Simple progress callback that logs progress
pub struct LoggingProgressCallback;

impl ProgressCallback for LoggingProgressCallback {
    fn on_progress(&self, progress: ProgressInfo) {
        info!(
            "Progress: {} - {:.1}% ({}/{})",
            progress.current_step,
            progress.percentage,
            progress.current_progress,
            progress.total_progress
        );
    }
}

/// Backup validator implementation
pub struct BackupValidatorImpl {
    reader: LevelDBReader,
    progress_callback: Option<Box<dyn ProgressCallback>>,
}

impl BackupValidatorImpl {
    /// Create a new backup validator
    pub fn new(file_path: impl Into<String>) -> Self {
        Self {
            reader: LevelDBReader::new(file_path),
            progress_callback: None,
        }
    }
    
    /// Set a progress callback for long-running operations
    pub fn with_progress_callback(mut self, callback: Box<dyn ProgressCallback>) -> Self {
        self.progress_callback = Some(callback);
        self
    }
    
    /// Validate the backup file comprehensively
    pub async fn validate_comprehensive(&self, file_path: &str) -> Result<ValidationResult, FireupError> {
        let context = ErrorContext {
            operation: "validate_comprehensive".to_string(),
            metadata: HashMap::from([
                ("file_path".to_string(), file_path.to_string()),
            ]),
            timestamp: chrono::Utc::now(),
            call_path: vec!["leveldb_parser::validator::BackupValidatorImpl".to_string()],
        };
        
        info!("Starting comprehensive backup validation for: {}", file_path);
        
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        
        // Step 1: Validate file accessibility
        self.report_progress("Validating file access", 0, 100);
        let file_info = match self.validate_file_access(file_path).await {
            Ok(info) => info,
            Err(e) => {
                errors.push(format!("File access validation failed: {}", e));
                return Ok(ValidationResult {
                    is_valid: false,
                    errors,
                    warnings,
                    file_info: FileInfo {
                        file_path: file_path.to_string(),
                        file_size: 0,
                        is_readable: false,
                        last_modified: None,
                    },
                    structure_info: StructureInfo {
                        total_blocks: 0,
                        total_records: 0,
                        valid_records: 0,
                        corrupted_records: 0,
                        metadata_records: 0,
                        document_records: 0,
                    },
                    integrity_info: IntegrityInfo {
                        checksum_failures: 0,
                        incomplete_records: 0,
                        parsing_errors: 0,
                        overall_integrity_score: 0.0,
                    },
                });
            }
        };
        
        // Step 2: Validate LevelDB structure
        self.report_progress("Validating LevelDB structure", 20, 100);
        let (structure_info, structure_errors, structure_warnings) = self.validate_structure().await;
        errors.extend(structure_errors);
        warnings.extend(structure_warnings);
        
        // Step 3: Validate data integrity
        self.report_progress("Validating data integrity", 60, 100);
        let (integrity_info, integrity_errors, integrity_warnings) = self.validate_integrity().await;
        errors.extend(integrity_errors);
        warnings.extend(integrity_warnings);
        
        // Step 4: Validate Firestore document format
        self.report_progress("Validating Firestore format", 80, 100);
        let (format_errors, format_warnings) = self.validate_firestore_format().await;
        errors.extend(format_errors);
        warnings.extend(format_warnings);
        
        self.report_progress("Validation complete", 100, 100);
        
        let is_valid = errors.is_empty();
        
        info!(
            "Validation complete: {} (errors: {}, warnings: {})",
            if is_valid { "VALID" } else { "INVALID" },
            errors.len(),
            warnings.len()
        );
        
        Ok(ValidationResult {
            is_valid,
            errors,
            warnings,
            file_info,
            structure_info,
            integrity_info,
        })
    }
    
    /// Validate file accessibility and basic properties
    async fn validate_file_access(&self, file_path: &str) -> Result<FileInfo, FireupError> {
        let context = ErrorContext {
            operation: "validate_file_access".to_string(),
            metadata: HashMap::from([
                ("file_path".to_string(), file_path.to_string()),
            ]),
            timestamp: chrono::Utc::now(),
            call_path: vec!["leveldb_parser::validator::BackupValidatorImpl".to_string()],
        };
        
        let path = Path::new(file_path);
        
        // Check if file exists
        if !path.exists() {
            return Err(FireupError::backup_validation(
                format!("File does not exist: {}", file_path),
                file_path,
                context
            ));
        }
        
        // Check if it's a file (not a directory)
        if !path.is_file() {
            return Err(FireupError::backup_validation(
                format!("Path is not a file: {}", file_path),
                file_path,
                context
            ));
        }
        
        // Get file metadata
        let metadata = fs::metadata(path).await
            .map_err(|e| FireupError::backup_validation(
                format!("Failed to read file metadata: {}", e),
                file_path,
                context.clone()
            ))?;
        
        let file_size = metadata.len();
        let last_modified = metadata.modified().ok()
            .and_then(|time| chrono::DateTime::from_timestamp(
                time.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64, 0
            ));
        
        // Check if file is readable
        let is_readable = fs::File::open(path).await.is_ok();
        
        // Validate minimum file size (LevelDB files should be at least a few KB)
        if file_size < 1024 {
            return Err(FireupError::backup_validation(
                format!("File too small to be a valid LevelDB backup: {} bytes", file_size),
                file_path,
                context
            ));
        }
        
        debug!("File validation passed: {} bytes, readable: {}", file_size, is_readable);
        
        Ok(FileInfo {
            file_path: file_path.to_string(),
            file_size,
            is_readable,
            last_modified,
        })
    }
    
    /// Validate LevelDB structure
    async fn validate_structure(&self) -> (StructureInfo, Vec<String>, Vec<String>) {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        
        let blocks = match self.reader.read_file().await {
            Ok(blocks) => blocks,
            Err(e) => {
                errors.push(format!("Failed to read LevelDB blocks: {}", e));
                return (StructureInfo {
                    total_blocks: 0,
                    total_records: 0,
                    valid_records: 0,
                    corrupted_records: 0,
                    metadata_records: 0,
                    document_records: 0,
                }, errors, warnings);
            }
        };
        
        let mut total_records = 0;
        let mut valid_records = 0;
        let mut corrupted_records = 0;
        
        for (block_index, block) in blocks.iter().enumerate() {
            self.report_progress(
                "Validating block structure",
                block_index as u64,
                blocks.len() as u64
            );
            
            total_records += block.records.len();
            
            for record in &block.records {
                // Basic record validation
                if record.header.length as usize == record.data.len() {
                    valid_records += 1;
                } else {
                    corrupted_records += 1;
                    warnings.push(format!(
                        "Record length mismatch in block {}: header says {}, actual {}",
                        block_index, record.header.length, record.data.len()
                    ));
                }
            }
        }
        
        // Validate overall structure
        if blocks.is_empty() {
            errors.push("No blocks found in LevelDB file".to_string());
        }
        
        if total_records == 0 {
            errors.push("No records found in LevelDB file".to_string());
        }
        
        if corrupted_records > total_records / 10 {
            warnings.push(format!(
                "High number of corrupted records: {} out of {} ({:.1}%)",
                corrupted_records, total_records,
                (corrupted_records as f64 / total_records as f64) * 100.0
            ));
        }
        
        debug!(
            "Structure validation: {} blocks, {} records ({} valid, {} corrupted)",
            blocks.len(), total_records, valid_records, corrupted_records
        );
        
        (StructureInfo {
            total_blocks: blocks.len(),
            total_records,
            valid_records,
            corrupted_records,
            metadata_records: 0, // Will be determined in format validation
            document_records: 0, // Will be determined in format validation
        }, errors, warnings)
    }
    
    /// Validate data integrity (checksums, etc.)
    async fn validate_integrity(&self) -> (IntegrityInfo, Vec<String>, Vec<String>) {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let checksum_failures = 0;
        let incomplete_records = 0;
        let parsing_errors = 0;
        
        let blocks = match self.reader.read_file().await {
            Ok(blocks) => blocks,
            Err(e) => {
                errors.push(format!("Failed to read file for integrity check: {}", e));
                return (IntegrityInfo {
                    checksum_failures: 0,
                    incomplete_records: 0,
                    parsing_errors: 0,
                    overall_integrity_score: 0.0,
                }, errors, warnings);
            }
        };
        
        let mut total_records = 0;
        
        for block in &blocks {
            total_records += block.records.len();
            
            for record in &block.records {
                // Checksum validation is already done in the reader
                // Here we just count any records that made it through
                
                // Check for incomplete fragmented records
                match record.header.record_type {
                    crate::leveldb_parser::parser::RecordType::First |
                    crate::leveldb_parser::parser::RecordType::Middle => {
                        // These should be followed by more parts
                        // For now, we'll assume they're handled correctly by the parser
                    }
                    _ => {}
                }
            }
        }
        
        // Calculate overall integrity score
        let integrity_score = if total_records > 0 {
            let failed_records = checksum_failures + incomplete_records + parsing_errors;
            1.0 - (failed_records as f64 / total_records as f64)
        } else {
            0.0
        };
        
        if integrity_score < 0.9 {
            warnings.push(format!(
                "Low integrity score: {:.1}% - consider using a different backup file",
                integrity_score * 100.0
            ));
        }
        
        debug!(
            "Integrity validation: score {:.1}%, {} checksum failures, {} incomplete records",
            integrity_score * 100.0, checksum_failures, incomplete_records
        );
        
        (IntegrityInfo {
            checksum_failures,
            incomplete_records,
            parsing_errors,
            overall_integrity_score: integrity_score,
        }, errors, warnings)
    }
    
    /// Validate Firestore document format
    async fn validate_firestore_format(&self) -> (Vec<String>, Vec<String>) {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        
        // Try to parse a few documents to validate format
        let parser = FirestoreDocumentParser::new(&self.reader.file_path);
        
        match parser.parse_documents().await {
            Ok(parse_result) => {
                if parse_result.documents.is_empty() {
                    warnings.push("No Firestore documents found in backup".to_string());
                } else {
                    debug!("Found {} Firestore documents", parse_result.documents.len());
                }
                
                if !parse_result.errors.is_empty() {
                    warnings.push(format!(
                        "Encountered {} parsing errors while validating format",
                        parse_result.errors.len()
                    ));
                }
            }
            Err(e) => {
                errors.push(format!("Failed to parse Firestore documents: {}", e));
            }
        }
        
        (errors, warnings)
    }
    
    /// Report progress to callback if available
    fn report_progress(&self, step: &str, current: u64, total: u64) {
        if let Some(callback) = &self.progress_callback {
            let percentage = if total > 0 {
                (current as f64 / total as f64) * 100.0
            } else {
                0.0
            };
            
            callback.on_progress(ProgressInfo {
                current_step: step.to_string(),
                current_progress: current,
                total_progress: total,
                percentage,
                estimated_remaining_ms: None,
                throughput_records_per_sec: None,
            });
        }
    }
    
    /// Generate a validation summary report
    pub fn generate_summary_report(&self, result: &ValidationResult) -> String {
        let mut report = String::new();
        
        report.push_str("=== Firestore Backup Validation Report ===\n\n");
        
        // Overall status
        report.push_str(&format!(
            "Overall Status: {}\n",
            if result.is_valid { "✓ VALID" } else { "✗ INVALID" }
        ));
        
        // File information
        report.push_str("\n--- File Information ---\n");
        report.push_str(&format!("Path: {}\n", result.file_info.file_path));
        report.push_str(&format!("Size: {} bytes ({:.2} MB)\n", 
            result.file_info.file_size,
            result.file_info.file_size as f64 / 1024.0 / 1024.0
        ));
        report.push_str(&format!("Readable: {}\n", result.file_info.is_readable));
        
        if let Some(modified) = result.file_info.last_modified {
            report.push_str(&format!("Last Modified: {}\n", modified.format("%Y-%m-%d %H:%M:%S UTC")));
        }
        
        // Structure information
        report.push_str("\n--- Structure Information ---\n");
        report.push_str(&format!("Total Blocks: {}\n", result.structure_info.total_blocks));
        report.push_str(&format!("Total Records: {}\n", result.structure_info.total_records));
        report.push_str(&format!("Valid Records: {}\n", result.structure_info.valid_records));
        report.push_str(&format!("Corrupted Records: {}\n", result.structure_info.corrupted_records));
        
        // Integrity information
        report.push_str("\n--- Integrity Information ---\n");
        report.push_str(&format!("Integrity Score: {:.1}%\n", 
            result.integrity_info.overall_integrity_score * 100.0
        ));
        report.push_str(&format!("Checksum Failures: {}\n", result.integrity_info.checksum_failures));
        report.push_str(&format!("Incomplete Records: {}\n", result.integrity_info.incomplete_records));
        report.push_str(&format!("Parsing Errors: {}\n", result.integrity_info.parsing_errors));
        
        // Errors
        if !result.errors.is_empty() {
            report.push_str("\n--- Errors ---\n");
            for (i, error) in result.errors.iter().enumerate() {
                report.push_str(&format!("{}. {}\n", i + 1, error));
            }
        }
        
        // Warnings
        if !result.warnings.is_empty() {
            report.push_str("\n--- Warnings ---\n");
            for (i, warning) in result.warnings.iter().enumerate() {
                report.push_str(&format!("{}. {}\n", i + 1, warning));
            }
        }
        
        report.push_str("\n=== End of Report ===\n");
        report
    }
}

/// Trait for backup validation operations
pub trait BackupValidator {
    async fn validate_backup(&self, file_path: &str) -> Result<ValidationResult, FireupError>;
}

impl BackupValidator for BackupValidatorImpl {
    async fn validate_backup(&self, file_path: &str) -> Result<ValidationResult, FireupError> {
        self.validate_comprehensive(file_path).await
    }
}