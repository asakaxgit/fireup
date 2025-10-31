// LevelDB parser implementation for Firestore backup files
use crate::error::{FireupError, ErrorContext};
use crate::types::FirestoreDocument;
use crate::monitoring::{get_monitoring_system, AuditOperationType, AuditResult};
use bytes::{Buf, Bytes};
use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::fs as stdfs;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tracing::{debug, info, warn, instrument};

/// LevelDB log record types according to specification
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecordType {
    Full = 1,
    First = 2,
    Middle = 3,
    Last = 4,
}

impl TryFrom<u8> for RecordType {
    type Error = FireupError;
    
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(RecordType::Full),
            2 => Ok(RecordType::First),
            3 => Ok(RecordType::Middle),
            4 => Ok(RecordType::Last),
            _ => Err(FireupError::leveldb_parse(
                format!("Invalid record type: {}", value),
                ErrorContext {
                    operation: "parse_record_type".to_string(),
                    metadata: HashMap::new(),
                    timestamp: chrono::Utc::now(),
                    call_path: vec!["leveldb_parser::parser".to_string()],
                }
            )),
        }
    }
}

/// LevelDB log record header
#[derive(Debug, Clone)]
pub struct RecordHeader {
    pub checksum: u32,
    pub length: u16,
    pub record_type: RecordType,
}

/// LevelDB log block (32KB blocks as per specification)
#[derive(Debug, Clone)]
pub struct LogBlock {
    pub data: Bytes,
    pub records: Vec<LogRecord>,
}

/// Supported backup formats
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackupFormat {
    LevelDb,
    JsonLines,
}

/// Individual log record within a block
#[derive(Debug, Clone)]
pub struct LogRecord {
    pub header: RecordHeader,
    pub data: Bytes,
}

/// LevelDB file reader with validation capabilities
pub struct LevelDBReader {
    pub file_path: String,
    block_size: usize,
}

impl LevelDBReader {
    /// Create a new LevelDB reader
    pub fn new(file_path: impl Into<String>) -> Self {
        let input = file_path.into();
        let resolved = Self::resolve_backup_path(&input).unwrap_or(input);
        Self {
            file_path: resolved,
            block_size: 32768, // 32KB blocks as per LevelDB specification
        }
    }
    
    /// Resolve a backup path that may be a directory to the actual backup file
    fn resolve_backup_path(input: &str) -> Option<String> {
        let p = Path::new(input);
        if p.is_file() {
            return Some(input.to_string());
        }
        if p.is_dir() {
            // Prefer a file named "output-0" within common firestore export structure
            if let Some(found) = Self::find_file_named(p, "output-0") {
                return Some(found.to_string_lossy().to_string());
            }
            // Fallback: first regular file found (depth-first)
            if let Some(any_file) = Self::find_any_file(p) {
                return Some(any_file.to_string_lossy().to_string());
            }
        }
        None
    }
    
    fn find_file_named(dir: &Path, target_name: &str) -> Option<PathBuf> {
        for entry in stdfs::read_dir(dir).ok()? {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name == target_name {
                        return Some(path);
                    }
                }
            } else if path.is_dir() {
                if let Some(found) = Self::find_file_named(&path, target_name) {
                    return Some(found);
                }
            }
        }
        None
    }
    
    fn find_any_file(dir: &Path) -> Option<PathBuf> {
        for entry in stdfs::read_dir(dir).ok()? {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_file() {
                return Some(path);
            } else if path.is_dir() {
                if let Some(found) = Self::find_any_file(&path) {
                    return Some(found);
                }
            }
        }
        None
    }
    
    /// Read and validate the entire LevelDB file
    pub async fn read_file(&self) -> Result<Vec<LogBlock>, FireupError> {
        let context = ErrorContext {
            operation: "read_file".to_string(),
            metadata: HashMap::from([
                ("file_path".to_string(), self.file_path.clone()),
            ]),
            timestamp: chrono::Utc::now(),
            call_path: vec!["leveldb_parser::parser::LevelDBReader".to_string()],
        };
        
        // Validate file exists and is readable
        if !Path::new(&self.file_path).exists() {
            return Err(FireupError::leveldb_parse(
                format!("File does not exist: {}", self.file_path),
                context
            ));
        }
        
        let mut file = File::open(&self.file_path).await
            .map_err(|e| FireupError::leveldb_parse(
                format!("Failed to open file: {}", e),
                context.clone()
            ))?;
        
        let file_size = file.metadata().await
            .map_err(|e| FireupError::leveldb_parse(
                format!("Failed to get file metadata: {}", e),
                context.clone()
            ))?.len();
        
        info!("Reading LevelDB file: {} ({} bytes)", self.file_path, file_size);
        
        let mut blocks = Vec::new();
        let mut position = 0u64;
        
        while position < file_size {
            let block = self.read_block(&mut file, position).await?;
            position += self.block_size as u64;
            blocks.push(block);
        }
        
        info!("Successfully read {} blocks from LevelDB file", blocks.len());
        Ok(blocks)
    }
    
    /// Read a single 32KB block from the file
    async fn read_block(&self, file: &mut File, position: u64) -> Result<LogBlock, FireupError> {
        let context = ErrorContext {
            operation: "read_block".to_string(),
            metadata: HashMap::from([
                ("file_path".to_string(), self.file_path.clone()),
                ("position".to_string(), position.to_string()),
            ]),
            timestamp: chrono::Utc::now(),
            call_path: vec!["leveldb_parser::parser::LevelDBReader".to_string()],
        };
        
        file.seek(SeekFrom::Start(position)).await
            .map_err(|e| FireupError::leveldb_parse(
                format!("Failed to seek to position {}: {}", position, e),
                context.clone()
            ))?;
        
        let mut buffer = vec![0u8; self.block_size];
        let bytes_read = file.read(&mut buffer).await
            .map_err(|e| FireupError::leveldb_parse(
                format!("Failed to read block at position {}: {}", position, e),
                context.clone()
            ))?;
        
        // Truncate buffer to actual bytes read for the last block
        buffer.truncate(bytes_read);
        let block_data = Bytes::from(buffer);
        
        debug!("Read block at position {} ({} bytes)", position, bytes_read);
        
        // Parse records within the block
        let records = self.parse_block_records(&block_data, position)?;
        
        Ok(LogBlock {
            data: block_data,
            records,
        })
    }
    
    /// Parse all records within a block
    fn parse_block_records(&self, block_data: &Bytes, block_position: u64) -> Result<Vec<LogRecord>, FireupError> {
        let _context = ErrorContext {
            operation: "parse_block_records".to_string(),
            metadata: HashMap::from([
                ("file_path".to_string(), self.file_path.clone()),
                ("block_position".to_string(), block_position.to_string()),
                ("block_size".to_string(), block_data.len().to_string()),
            ]),
            timestamp: chrono::Utc::now(),
            call_path: vec!["leveldb_parser::parser::LevelDBReader".to_string()],
        };
        
        let mut records = Vec::new();
        let mut offset = 0;
        
        while offset + 7 <= block_data.len() { // Minimum header size is 7 bytes
            // Check for padding (zeros) at end of block
            if block_data[offset..].iter().all(|&b| b == 0) {
                debug!("Found padding at offset {} in block at position {}", offset, block_position);
                break;
            }
            
            match self.parse_record_at_offset(block_data, offset, block_position) {
                Ok((record, next_offset)) => {
                    records.push(record);
                    offset = next_offset;
                }
                Err(e) => {
                    debug!("Failed to parse record at offset {} in block {}: {}", offset, block_position, e);
                    // Skip to next potential record boundary
                    offset += 1;
                }
            }
        }
        
        debug!("Parsed {} records from block at position {}", records.len(), block_position);
        Ok(records)
    }
    
    /// Parse a single record at the given offset within a block
    fn parse_record_at_offset(&self, block_data: &Bytes, offset: usize, block_position: u64) -> Result<(LogRecord, usize), FireupError> {
        let context = ErrorContext {
            operation: "parse_record_at_offset".to_string(),
            metadata: HashMap::from([
                ("file_path".to_string(), self.file_path.clone()),
                ("block_position".to_string(), block_position.to_string()),
                ("offset".to_string(), offset.to_string()),
            ]),
            timestamp: chrono::Utc::now(),
            call_path: vec!["leveldb_parser::parser::LevelDBReader".to_string()],
        };
        
        if offset + 7 > block_data.len() {
            return Err(FireupError::leveldb_parse(
                "Not enough bytes for record header".to_string(),
                context
            ));
        }
        
        // Parse record header (7 bytes total)
        let mut header_bytes = &block_data[offset..offset + 7];
        
        let checksum = header_bytes.get_u32_le();
        let length = header_bytes.get_u16_le();
        let record_type = RecordType::try_from(header_bytes.get_u8())?;
        
        let header = RecordHeader {
            checksum,
            length,
            record_type,
        };
        
        // Validate record length
        let data_end = offset + 7 + length as usize;
        if data_end > block_data.len() {
            return Err(FireupError::leveldb_parse(
                format!("Record data extends beyond block boundary: {} > {}", data_end, block_data.len()),
                context
            ));
        }
        
        // Extract record data
        let record_data = block_data.slice(offset + 7..data_end);
        
        // Validate CRC32 checksum
        self.validate_checksum(&header, &record_data)?;
        
        let record = LogRecord {
            header,
            data: record_data,
        };
        
        debug!("Parsed record: type={:?}, length={}, checksum=0x{:08x}", 
               record.header.record_type, record.header.length, record.header.checksum);
        
        Ok((record, data_end))
    }
    
    /// Validate CRC32 checksum for a record
    fn validate_checksum(&self, header: &RecordHeader, data: &Bytes) -> Result<(), FireupError> {
        let context = ErrorContext {
            operation: "validate_checksum".to_string(),
            metadata: HashMap::from([
                ("file_path".to_string(), self.file_path.clone()),
                ("record_type".to_string(), format!("{:?}", header.record_type)),
                ("data_length".to_string(), data.len().to_string()),
            ]),
            timestamp: chrono::Utc::now(),
            call_path: vec!["leveldb_parser::parser::LevelDBReader".to_string()],
        };
        
        // Calculate CRC32 checksum
        let calculated_checksum = self.calculate_crc32(header.record_type as u8, data);
        
        if calculated_checksum != header.checksum {
            return Err(FireupError::leveldb_parse(
                format!(
                    "Checksum mismatch: expected 0x{:08x}, calculated 0x{:08x}",
                    header.checksum, calculated_checksum
                ),
                context
            ));
        }
        
        debug!("Checksum validation passed: 0x{:08x}", calculated_checksum);
        Ok(())
    }
    
    /// Calculate CRC32 checksum according to LevelDB specification
    fn calculate_crc32(&self, record_type: u8, data: &Bytes) -> u32 {
        // LevelDB uses a specific CRC32 implementation
        // For now, we'll use a simple implementation
        // In production, this should use the exact same CRC32 as LevelDB
        let mut crc = crc32fast::Hasher::new();
        crc.update(&[record_type]);
        crc.update(data);
        crc.finalize()
    }
    
    /// Get file size
    pub async fn file_size(&self) -> Result<u64, FireupError> {
        let context = ErrorContext {
            operation: "file_size".to_string(),
            metadata: HashMap::from([
                ("file_path".to_string(), self.file_path.clone()),
            ]),
            timestamp: chrono::Utc::now(),
            call_path: vec!["leveldb_parser::parser::LevelDBReader".to_string()],
        };
        
        let file = File::open(&self.file_path).await
            .map_err(|e| FireupError::leveldb_parse(
                format!("Failed to open file for size check: {}", e),
                context.clone()
            ))?;
        
        let size = file.metadata().await
            .map_err(|e| FireupError::leveldb_parse(
                format!("Failed to get file metadata: {}", e),
                context
            ))?.len();
        
        Ok(size)
    }
}

/// Parse result containing documents and metadata
pub struct ParseResult {
    pub documents: Vec<FirestoreDocument>,
    pub collections: Vec<String>,
    pub metadata: BackupMetadata,
    pub errors: Vec<FireupError>,
}

/// Metadata about the backup file
#[derive(Debug, Clone)]
pub struct BackupMetadata {
    pub file_size: u64,
    pub document_count: usize,
    pub collection_count: usize,
    pub blocks_processed: usize,
    pub records_processed: usize,
}

/// Trait for LevelDB parsing operations
#[allow(async_fn_in_trait)]
pub trait LevelDBParser {
    async fn parse_backup(&self, file_path: &str) -> Result<ParseResult, FireupError>;
}

/// Firestore document parser that converts LevelDB records to Firestore documents
pub struct FirestoreDocumentParser {
    reader: LevelDBReader,
}

impl FirestoreDocumentParser {
    /// Create a new Firestore document parser
    pub fn new(file_path: impl Into<String>) -> Self {
        Self {
            reader: LevelDBReader::new(file_path),
        }
    }
    
    /// Parse the entire backup file and extract Firestore documents
    #[instrument(skip(self))]
    pub async fn parse_documents(&self) -> Result<ParseResult, FireupError> {
        let tracker = get_monitoring_system().start_operation("leveldb_parsing").await;
        tracker.add_metadata("file_path", &self.reader.file_path).await.ok();
        let context = ErrorContext {
            operation: "parse_documents".to_string(),
            metadata: HashMap::from([
                ("file_path".to_string(), self.reader.file_path.clone()),
            ]),
            timestamp: chrono::Utc::now(),
            call_path: vec!["leveldb_parser::parser::FirestoreDocumentParser".to_string()],
        };
        
        info!("Starting Firestore document parsing");
        
        let file_size = self.reader.file_size().await?;
        let format = self.detect_backup_format().await.unwrap_or(BackupFormat::LevelDb);
        
        // Parse each complete record as a Firestore document
        let mut documents = Vec::new();
        let mut collections = std::collections::HashSet::new();
        let mut errors = Vec::new();
        let mut blocks_processed: usize = 0;
        let mut records_processed: usize = 0;

        match format {
            BackupFormat::JsonLines => {
                info!(
                    "Detected JSON Lines backup; parsing as JSON lines: {}",
                    self.reader.file_path
                );
                let file_bytes = tokio::fs::read(&self.reader.file_path).await.map_err(|e| {
                    FireupError::leveldb_parse(
                        format!("Failed to read file for JSON parsing: {}", e),
                        context.clone(),
                    )
                })?;
                let mut non_empty_lines = 0usize;
                for (index, line) in file_bytes.split(|b| *b == b'\n').enumerate() {
                    let Ok(line_str) = std::str::from_utf8(line) else { continue };
                    let trimmed = line_str.trim();
                    if trimmed.is_empty() { continue; }
                    non_empty_lines += 1;
                    match serde_json::from_str::<serde_json::Value>(trimmed) {
                        Ok(json_value) => {
                            match self.parse_json_document(&json_value, index).await {
                                Ok(Some(document)) => {
                                    collections.insert(document.collection.clone());
                                    documents.push(document);
                                }
                                Ok(None) => {
                                    debug!("Skipped non-document JSON line at index {}", index);
                                }
                                Err(e) => {
                                    warn!("Failed to parse JSON line {}: {}", index, e);
                                    errors.push(e);
                                }
                            }
                        }
                        Err(e) => {
                            // Ignore lines that aren't valid JSON
                            debug!("Invalid JSON line {}: {}", index, e);
                            continue;
                        }
                    }
                }
                if documents.is_empty() {
                    warn!("JSON Lines parsing did not produce any documents ({} non-empty lines)", non_empty_lines);
                }
                blocks_processed = 0;
                records_processed = documents.len();
            }
            BackupFormat::LevelDb => {
                // Read all blocks from the LevelDB file
                let blocks = self.reader.read_file().await?;
                // Reconstruct complete records from potentially fragmented log records
                let complete_records = self.reconstruct_records(&blocks)?;
                for (index, record_data) in complete_records.iter().enumerate() {
                    match self.parse_firestore_record(record_data, index).await {
                        Ok(Some(document)) => {
                            collections.insert(document.collection.clone());
                            documents.push(document);
                        }
                        Ok(None) => {
                            // Skip non-document records (metadata, etc.)
                            debug!("Skipped non-document record at index {}", index);
                        }
                        Err(e) => {
                            warn!("Failed to parse record at index {}: {}", index, e);
                            errors.push(e);
                        }
                    }
                }
                blocks_processed = blocks.len();
                records_processed = complete_records.len();
            }
        }
        
        let collections: Vec<String> = collections.into_iter().collect();
        
        let metadata = BackupMetadata {
            file_size,
            document_count: documents.len(),
            collection_count: collections.len(),
            blocks_processed,
            records_processed,
        };
        
        info!(
            "Parsing complete: {} documents, {} collections, {} errors",
            documents.len(), collections.len(), errors.len()
        );

        // Log data access audit entry
        let mut details = HashMap::new();
        details.insert("file_path".to_string(), self.reader.file_path.clone());
        details.insert("documents_parsed".to_string(), documents.len().to_string());
        details.insert("collections_found".to_string(), collections.len().to_string());
        details.insert("blocks_processed".to_string(), metadata.blocks_processed.to_string());
        details.insert("file_size_bytes".to_string(), metadata.file_size.to_string());
        
        let audit_result = if errors.is_empty() {
            AuditResult::Success
        } else if !documents.is_empty() {
            AuditResult::PartialSuccess(format!("{} parsing errors", errors.len()))
        } else {
            AuditResult::Failure("Failed to parse any documents".to_string())
        };
        
        get_monitoring_system().log_audit_entry(
            AuditOperationType::DataAccess,
            "backup_file",
            &self.reader.file_path,
            "parse_documents",
            audit_result,
            details,
            None,
        ).await.ok();

        tracker.update_progress(documents.len() as u64, None).await.ok();
        tracker.complete_success().await.ok();
        
        Ok(ParseResult {
            documents,
            collections,
            metadata,
            errors,
        })
    }

    /// Detect whether the backup file is a LevelDB log or JSON Lines
    async fn detect_backup_format(&self) -> Result<BackupFormat, FireupError> {
        // Read a small prefix of the file
        let mut file = File::open(&self.reader.file_path).await
            .map_err(|e| FireupError::leveldb_parse(format!("Failed to open file for format detection: {}", e), ErrorContext {
                operation: "detect_backup_format".to_string(),
                metadata: HashMap::from([("file_path".to_string(), self.reader.file_path.clone())]),
                timestamp: chrono::Utc::now(),
                call_path: vec!["leveldb_parser::parser::FirestoreDocumentParser".to_string()],
            }))?;

        let mut buf = vec![0u8; 8192];
        let n = file.read(&mut buf).await.unwrap_or(0);
        buf.truncate(n);

        // If the prefix decodes as UTF-8 and contains braces or newlines, likely JSON lines
        if let Ok(prefix) = std::str::from_utf8(&buf) {
            let has_brace = prefix.contains('{') || prefix.contains('[');
            let has_newline = prefix.contains('\n');
            let printable_ratio = prefix.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\r' || *c == '\t').count() as f64
                / (prefix.chars().count().max(1) as f64);
            if has_brace && has_newline && printable_ratio > 0.95 {
                return Ok(BackupFormat::JsonLines);
            }
        }

        Ok(BackupFormat::LevelDb)
    }
    
    /// Reconstruct complete records from potentially fragmented log records
    fn reconstruct_records(&self, blocks: &[LogBlock]) -> Result<Vec<Bytes>, FireupError> {
        let context = ErrorContext {
            operation: "reconstruct_records".to_string(),
            metadata: HashMap::from([
                ("blocks_count".to_string(), blocks.len().to_string()),
            ]),
            timestamp: chrono::Utc::now(),
            call_path: vec!["leveldb_parser::parser::FirestoreDocumentParser".to_string()],
        };
        
        let mut complete_records = Vec::new();
        let mut current_record_parts = Vec::new();
        
        for block in blocks {
            for record in &block.records {
                match record.header.record_type {
                    RecordType::Full => {
                        // Complete record in a single log record
                        if !current_record_parts.is_empty() {
                            warn!("Found Full record while reconstructing fragmented record");
                            current_record_parts.clear();
                        }
                        complete_records.push(record.data.clone());
                    }
                    RecordType::First => {
                        // Start of a fragmented record
                        if !current_record_parts.is_empty() {
                            warn!("Found First record while reconstructing fragmented record");
                        }
                        current_record_parts.clear();
                        current_record_parts.push(record.data.clone());
                    }
                    RecordType::Middle => {
                        // Middle part of a fragmented record
                        if current_record_parts.is_empty() {
                            return Err(FireupError::leveldb_parse(
                                "Found Middle record without preceding First record".to_string(),
                                context
                            ));
                        }
                        current_record_parts.push(record.data.clone());
                    }
                    RecordType::Last => {
                        // End of a fragmented record
                        if current_record_parts.is_empty() {
                            return Err(FireupError::leveldb_parse(
                                "Found Last record without preceding First record".to_string(),
                                context
                            ));
                        }
                        current_record_parts.push(record.data.clone());
                        
                        // Combine all parts into a complete record
                        let complete_record = self.combine_record_parts(&current_record_parts);
                        complete_records.push(complete_record);
                        current_record_parts.clear();
                    }
                }
            }
        }
        
        // Check for incomplete fragmented records
        if !current_record_parts.is_empty() {
            warn!("Found incomplete fragmented record at end of file");
        }
        
        debug!("Reconstructed {} complete records from {} blocks", complete_records.len(), blocks.len());
        Ok(complete_records)
    }
    
    /// Combine multiple record parts into a single complete record
    fn combine_record_parts(&self, parts: &[Bytes]) -> Bytes {
        let total_size: usize = parts.iter().map(|p| p.len()).sum();
        let mut combined = Vec::with_capacity(total_size);
        
        for part in parts {
            combined.extend_from_slice(part);
        }
        
        Bytes::from(combined)
    }
    
    /// Parse a complete record as a Firestore document
    async fn parse_firestore_record(&self, record_data: &Bytes, record_index: usize) -> Result<Option<FirestoreDocument>, FireupError> {
        let context = ErrorContext {
            operation: "parse_firestore_record".to_string(),
            metadata: HashMap::from([
                ("record_index".to_string(), record_index.to_string()),
                ("record_size".to_string(), record_data.len().to_string()),
            ]),
            timestamp: chrono::Utc::now(),
            call_path: vec!["leveldb_parser::parser::FirestoreDocumentParser".to_string()],
        };
        
        // Try to parse as JSON first (common format for Firestore exports)
        if let Ok(json_value) = serde_json::from_slice::<serde_json::Value>(record_data) {
            return self.parse_json_document(&json_value, record_index).await;
        }
        
        // Try to parse as protobuf-encoded data
        if let Ok(document) = self.parse_protobuf_document(record_data, record_index).await {
            return Ok(document);
        }
        
        // Check if this is a metadata record (skip)
        if self.is_metadata_record(record_data) {
            debug!("Skipping metadata record at index {}", record_index);
            return Ok(None);
        }
        
        // If we can't parse it, log a warning but don't fail
        warn!("Unable to parse record at index {} ({} bytes)", record_index, record_data.len());
        Ok(None)
    }
    
    /// Parse a JSON-formatted Firestore document
    async fn parse_json_document(&self, json_value: &serde_json::Value, record_index: usize) -> Result<Option<FirestoreDocument>, FireupError> {
        let context = ErrorContext {
            operation: "parse_json_document".to_string(),
            metadata: HashMap::from([
                ("record_index".to_string(), record_index.to_string()),
            ]),
            timestamp: chrono::Utc::now(),
            call_path: vec!["leveldb_parser::parser::FirestoreDocumentParser".to_string()],
        };
        
        // Check if this looks like a Firestore document
        let obj = match json_value.as_object() {
            Some(obj) => obj,
            None => return Ok(None), // Not a document object
        };
        
        // Extract document ID and collection from the document path or name field
        let (doc_id, collection) = self.extract_document_identity(obj)?;
        
        // Extract document data (fields)
        let data = self.extract_document_fields(obj)?;
        
        // Extract metadata
        let metadata = self.extract_document_metadata(obj)?;
        
        // Parse subcollections if present
        let subcollections = self.extract_subcollections(obj).await?;
        
        let document = FirestoreDocument {
            id: doc_id,
            collection,
            data,
            subcollections,
            metadata,
        };
        
        debug!("Parsed JSON document: {}/{}", document.collection, document.id);
        Ok(Some(document))
    }
    
    /// Parse a protobuf-encoded Firestore document
    async fn parse_protobuf_document(&self, _record_data: &Bytes, record_index: usize) -> Result<Option<FirestoreDocument>, FireupError> {
        // For now, we'll implement a basic protobuf parser
        // In a production system, you'd use the actual Firestore protobuf definitions
        
        debug!("Attempting to parse protobuf document at index {}", record_index);
        
        // This is a placeholder implementation
        // Real protobuf parsing would require the Firestore protobuf schema
        Ok(None)
    }
    
    /// Extract document ID and collection name from JSON object
    fn extract_document_identity(&self, obj: &serde_json::Map<String, serde_json::Value>) -> Result<(String, String), FireupError> {
        let context = ErrorContext {
            operation: "extract_document_identity".to_string(),
            metadata: HashMap::new(),
            timestamp: chrono::Utc::now(),
            call_path: vec!["leveldb_parser::parser::FirestoreDocumentParser".to_string()],
        };
        
        // Try to extract from 'name' field (common in Firestore exports)
        if let Some(name_value) = obj.get("name") {
            if let Some(name_str) = name_value.as_str() {
                if let Some((collection, doc_id)) = self.parse_document_path(name_str) {
                    return Ok((doc_id, collection));
                }
            }
        }
        
        // Try to extract from 'path' field
        if let Some(path_value) = obj.get("path") {
            if let Some(path_str) = path_value.as_str() {
                if let Some((collection, doc_id)) = self.parse_document_path(path_str) {
                    return Ok((doc_id, collection));
                }
            }
        }
        
        // Try to extract from separate 'id' and 'collection' fields
        let doc_id = obj.get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        
        let collection = obj.get("collection")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        
        Ok((doc_id, collection))
    }
    
    /// Parse a Firestore document path to extract collection and document ID
    fn parse_document_path(&self, path: &str) -> Option<(String, String)> {
        // Firestore paths typically look like:
        // projects/{project}/databases/{database}/documents/{collection}/{document}
        // or just {collection}/{document}
        
        let parts: Vec<&str> = path.split('/').collect();
        
        // Look for the pattern ending with collection/document
        if parts.len() >= 2 {
            let doc_id = parts[parts.len() - 1].to_string();
            let collection = parts[parts.len() - 2].to_string();
            return Some((collection, doc_id));
        }
        
        None
    }
    
    /// Extract document fields from JSON object
    fn extract_document_fields(&self, obj: &serde_json::Map<String, serde_json::Value>) -> Result<HashMap<String, serde_json::Value>, FireupError> {
        let mut fields = HashMap::new();
        
        // Look for 'fields' object (common in Firestore exports)
        if let Some(fields_value) = obj.get("fields") {
            if let Some(fields_obj) = fields_value.as_object() {
                for (key, value) in fields_obj {
                    // Firestore fields are often wrapped in type objects
                    let unwrapped_value = self.unwrap_firestore_value(value);
                    fields.insert(key.clone(), unwrapped_value);
                }
                return Ok(fields);
            }
        }
        
        // If no 'fields' object, treat the entire object as fields (excluding metadata)
        for (key, value) in obj {
            if !self.is_metadata_field(key) {
                fields.insert(key.clone(), value.clone());
            }
        }
        
        Ok(fields)
    }
    
    /// Unwrap Firestore-specific value encoding
    fn unwrap_firestore_value(&self, value: &serde_json::Value) -> serde_json::Value {
        // Firestore values are often encoded as {"stringValue": "text"} or {"integerValue": "123"}
        if let Some(obj) = value.as_object() {
            if obj.len() == 1 {
                for (type_key, type_value) in obj {
                    match type_key.as_str() {
                        "stringValue" => return type_value.clone(),
                        "integerValue" => {
                            if let Some(int_str) = type_value.as_str() {
                                if let Ok(int_val) = int_str.parse::<i64>() {
                                    return serde_json::Value::Number(int_val.into());
                                }
                            }
                            return type_value.clone();
                        }
                        "doubleValue" => {
                            if let Some(double_str) = type_value.as_str() {
                                if let Ok(double_val) = double_str.parse::<f64>() {
                                    return serde_json::Value::Number(
                                        serde_json::Number::from_f64(double_val).unwrap_or_else(|| 0.into())
                                    );
                                }
                            }
                            return type_value.clone();
                        }
                        "booleanValue" => return type_value.clone(),
                        "timestampValue" => return type_value.clone(),
                        "arrayValue" => {
                            if let Some(array_obj) = type_value.as_object() {
                                if let Some(values) = array_obj.get("values") {
                                    if let Some(values_array) = values.as_array() {
                                        let unwrapped: Vec<serde_json::Value> = values_array
                                            .iter()
                                            .map(|v| self.unwrap_firestore_value(v))
                                            .collect();
                                        return serde_json::Value::Array(unwrapped);
                                    }
                                }
                            }
                            return type_value.clone();
                        }
                        "mapValue" => {
                            if let Some(map_obj) = type_value.as_object() {
                                if let Some(fields) = map_obj.get("fields") {
                                    if let Some(fields_obj) = fields.as_object() {
                                        let mut unwrapped_map = serde_json::Map::new();
                                        for (k, v) in fields_obj {
                                            unwrapped_map.insert(k.clone(), self.unwrap_firestore_value(v));
                                        }
                                        return serde_json::Value::Object(unwrapped_map);
                                    }
                                }
                            }
                            return type_value.clone();
                        }
                        _ => return value.clone(),
                    }
                }
            }
        }
        
        value.clone()
    }
    
    /// Check if a field name represents metadata rather than document data
    fn is_metadata_field(&self, field_name: &str) -> bool {
        matches!(field_name, 
            "name" | "path" | "id" | "collection" | 
            "createTime" | "updateTime" | "readTime" |
            "_firestore_metadata" | "_id" | "_collection"
        )
    }
    
    /// Extract document metadata from JSON object
    fn extract_document_metadata(&self, obj: &serde_json::Map<String, serde_json::Value>) -> Result<crate::types::DocumentMetadata, FireupError> {
        let created_at = obj.get("createTime")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));
        
        let updated_at = obj.get("updateTime")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));
        
        let path = obj.get("name")
            .or_else(|| obj.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        
        Ok(crate::types::DocumentMetadata {
            created_at,
            updated_at,
            path,
            size_bytes: None, // Will be calculated later if needed
        })
    }
    
    /// Extract subcollections from JSON object
    async fn extract_subcollections(&self, _obj: &serde_json::Map<String, serde_json::Value>) -> Result<Vec<FirestoreDocument>, FireupError> {
        // For now, return empty subcollections
        // In a full implementation, this would recursively parse nested collections
        Ok(Vec::new())
    }
    
    /// Check if a record represents metadata rather than document data
    fn is_metadata_record(&self, record_data: &Bytes) -> bool {
        // Check for common metadata patterns
        if record_data.len() < 10 {
            return true; // Too small to be a document
        }
        
        // Check for specific metadata markers
        let data_str = String::from_utf8_lossy(record_data);
        data_str.contains("_metadata") || 
        data_str.contains("_system") ||
        data_str.starts_with("__")
    }
}

impl LevelDBParser for FirestoreDocumentParser {
    async fn parse_backup(&self, file_path: &str) -> Result<ParseResult, FireupError> {
        let parser = FirestoreDocumentParser::new(file_path);
        parser.parse_documents().await
    }
}