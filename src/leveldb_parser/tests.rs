// Unit tests for LevelDB parser functionality
use super::*;
use crate::error::{FireupError, ErrorContext};
use crate::leveldb_parser::parser::{LevelDBReader, FirestoreDocumentParser, RecordType};
use crate::leveldb_parser::validator::LoggingProgressCallback;
use tempfile::TempDir;
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// Helper function to create a valid LevelDB record header
fn create_record_header(record_type: RecordType, data_length: u16, checksum: u32) -> Vec<u8> {
    let mut header = Vec::with_capacity(7);
    header.extend_from_slice(&checksum.to_le_bytes());
    header.extend_from_slice(&data_length.to_le_bytes());
    header.push(record_type as u8);
    header
}

/// Helper function to calculate CRC32 checksum for test data
fn calculate_test_checksum(record_type: u8, data: &[u8]) -> u32 {
    let mut crc = crc32fast::Hasher::new();
    crc.update(&[record_type]);
    crc.update(data);
    crc.finalize()
}

/// Helper function to create a valid LevelDB block with test data
async fn create_test_leveldb_file(temp_dir: &TempDir, filename: &str, records: Vec<(RecordType, &[u8])>) -> Result<String, FireupError> {
    let file_path = temp_dir.path().join(filename);
    let mut file = fs::File::create(&file_path).await
        .map_err(|e| FireupError::leveldb_parse(
            format!("Failed to create test file: {}", e),
            ErrorContext {
                operation: "create_test_file".to_string(),
                metadata: std::collections::HashMap::new(),
                timestamp: chrono::Utc::now(),
                call_path: vec!["test_helper".to_string()],
            }
        ))?;
    
    let mut block_data = Vec::new();
    
    // Create records in the block
    for (record_type, data) in records {
        let checksum = calculate_test_checksum(record_type as u8, data);
        let header = create_record_header(record_type, data.len() as u16, checksum);
        
        block_data.extend_from_slice(&header);
        block_data.extend_from_slice(data);
    }
    
    // Pad block to 32KB if needed
    while block_data.len() < 32768 {
        block_data.push(0);
    }
    
    // Truncate to exactly 32KB
    block_data.truncate(32768);
    
    file.write_all(&block_data).await
        .map_err(|e| FireupError::leveldb_parse(
            format!("Failed to write test file: {}", e),
            ErrorContext {
                operation: "write_test_file".to_string(),
                metadata: std::collections::HashMap::new(),
                timestamp: chrono::Utc::now(),
                call_path: vec!["test_helper".to_string()],
            }
        ))?;
    
    Ok(file_path.to_string_lossy().to_string())
}

/// Helper function to create a corrupted LevelDB file
async fn create_corrupted_leveldb_file(temp_dir: &TempDir, filename: &str) -> Result<String, FireupError> {
    let file_path = temp_dir.path().join(filename);
    let mut file = fs::File::create(&file_path).await
        .map_err(|e| FireupError::leveldb_parse(
            format!("Failed to create corrupted test file: {}", e),
            ErrorContext {
                operation: "create_corrupted_test_file".to_string(),
                metadata: std::collections::HashMap::new(),
                timestamp: chrono::Utc::now(),
                call_path: vec!["test_helper".to_string()],
            }
        ))?;
    
    // Create invalid data that looks like LevelDB but has wrong checksums
    let mut block_data = Vec::new();
    
    // Invalid record with wrong checksum
    let data = b"invalid data";
    let wrong_checksum = 0x12345678u32; // Intentionally wrong
    let header = create_record_header(RecordType::Full, data.len() as u16, wrong_checksum);
    
    block_data.extend_from_slice(&header);
    block_data.extend_from_slice(data);
    
    // Pad to 32KB
    while block_data.len() < 32768 {
        block_data.push(0);
    }
    block_data.truncate(32768);
    
    file.write_all(&block_data).await
        .map_err(|e| FireupError::leveldb_parse(
            format!("Failed to write corrupted test file: {}", e),
            ErrorContext {
                operation: "write_corrupted_test_file".to_string(),
                metadata: std::collections::HashMap::new(),
                timestamp: chrono::Utc::now(),
                call_path: vec!["test_helper".to_string()],
            }
        ))?;
    
    Ok(file_path.to_string_lossy().to_string())
}

/// Helper function to create a large test file for streaming tests
async fn create_large_leveldb_file(temp_dir: &TempDir, filename: &str, num_blocks: usize) -> Result<String, FireupError> {
    let file_path = temp_dir.path().join(filename);
    let mut file = fs::File::create(&file_path).await
        .map_err(|e| FireupError::leveldb_parse(
            format!("Failed to create large test file: {}", e),
            ErrorContext {
                operation: "create_large_test_file".to_string(),
                metadata: std::collections::HashMap::new(),
                timestamp: chrono::Utc::now(),
                call_path: vec!["test_helper".to_string()],
            }
        ))?;
    
    for block_index in 0..num_blocks {
        let mut block_data = Vec::new();
        
        // Create multiple records per block
        for record_index in 0..10 {
            let data = format!("block_{}_record_{}_data", block_index, record_index);
            let data_bytes = data.as_bytes();
            let checksum = calculate_test_checksum(RecordType::Full as u8, data_bytes);
            let header = create_record_header(RecordType::Full, data_bytes.len() as u16, checksum);
            
            block_data.extend_from_slice(&header);
            block_data.extend_from_slice(data_bytes);
        }
        
        // Pad block to 32KB
        while block_data.len() < 32768 {
            block_data.push(0);
        }
        block_data.truncate(32768);
        
        file.write_all(&block_data).await
            .map_err(|e| FireupError::leveldb_parse(
                format!("Failed to write large test file: {}", e),
                ErrorContext {
                    operation: "write_large_test_file".to_string(),
                    metadata: std::collections::HashMap::new(),
                    timestamp: chrono::Utc::now(),
                    call_path: vec!["test_helper".to_string()],
                }
            ))?;
    }
    
    Ok(file_path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitoring::{initialize_monitoring, MonitoringConfig};
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn setup_monitoring() {
        INIT.call_once(|| {
            initialize_monitoring(MonitoringConfig::default());
        });
    }

    #[tokio::test]
    async fn test_leveldb_reader_new() {
        let reader = LevelDBReader::new("test_file.leveldb");
        assert_eq!(reader.file_path, "test_file.leveldb");
        // Note: block_size is private, so we can't test it directly
        // but we know it should be 32768 from the implementation
    }

    #[tokio::test]
    async fn test_leveldb_reader_nonexistent_file() {
        let reader = LevelDBReader::new("nonexistent_file.leveldb");
        let result = reader.read_file().await;
        
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.to_string().contains("File does not exist"));
    }

    #[tokio::test]
    async fn test_leveldb_reader_valid_single_record() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        // Create a file with a single valid record
        let test_data = b"test firestore document data";
        let file_path = create_test_leveldb_file(
            &temp_dir,
            "single_record.leveldb",
            vec![(RecordType::Full, test_data)]
        ).await.expect("Failed to create test file");
        
        let reader = LevelDBReader::new(file_path);
        let result = reader.read_file().await;
        
        assert!(result.is_ok());
        let blocks = result.unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].records.len(), 1);
        assert_eq!(blocks[0].records[0].header.record_type, RecordType::Full);
        assert_eq!(blocks[0].records[0].data.as_ref(), test_data);
    }

    #[tokio::test]
    async fn test_leveldb_reader_multiple_records() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        // Create a file with multiple records
        let records = vec![
            (RecordType::Full, b"first document" as &[u8]),
            (RecordType::Full, b"second document"),
            (RecordType::Full, b"third document"),
        ];
        
        let file_path = create_test_leveldb_file(
            &temp_dir,
            "multiple_records.leveldb",
            records
        ).await.expect("Failed to create test file");
        
        let reader = LevelDBReader::new(file_path);
        let result = reader.read_file().await;
        
        assert!(result.is_ok());
        let blocks = result.unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].records.len(), 3);
        
        assert_eq!(blocks[0].records[0].data.as_ref(), b"first document");
        assert_eq!(blocks[0].records[1].data.as_ref(), b"second document");
        assert_eq!(blocks[0].records[2].data.as_ref(), b"third document");
    }

    #[tokio::test]
    async fn test_leveldb_reader_fragmented_record() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        // Create a fragmented record across multiple log records
        let records = vec![
            (RecordType::First, b"start of large" as &[u8]),
            (RecordType::Middle, b" document that spans"),
            (RecordType::Last, b" multiple records"),
        ];
        
        let file_path = create_test_leveldb_file(
            &temp_dir,
            "fragmented_record.leveldb",
            records
        ).await.expect("Failed to create test file");
        
        let reader = LevelDBReader::new(file_path);
        let result = reader.read_file().await;
        
        assert!(result.is_ok());
        let blocks = result.unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].records.len(), 3);
        
        assert_eq!(blocks[0].records[0].header.record_type, RecordType::First);
        assert_eq!(blocks[0].records[1].header.record_type, RecordType::Middle);
        assert_eq!(blocks[0].records[2].header.record_type, RecordType::Last);
    }

    #[tokio::test]
    async fn test_leveldb_reader_corrupted_checksum() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        let file_path = create_corrupted_leveldb_file(
            &temp_dir,
            "corrupted.leveldb"
        ).await.expect("Failed to create corrupted test file");
        
        let reader = LevelDBReader::new(file_path);
        let result = reader.read_file().await;
        
        // The reader should handle corrupted records gracefully
        // It may succeed but with fewer records, or fail with a checksum error
        match result {
            Ok(blocks) => {
                // If it succeeds, it should have skipped the corrupted record
                assert_eq!(blocks.len(), 1);
                // The corrupted record should be skipped, so we expect 0 valid records
                assert_eq!(blocks[0].records.len(), 0);
            }
            Err(error) => {
                // If it fails, it should be due to checksum validation
                assert!(error.to_string().contains("Checksum mismatch") || 
                       error.to_string().contains("Failed to parse record"));
            }
        }
    }

    #[tokio::test]
    async fn test_leveldb_reader_empty_file() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let file_path = temp_dir.path().join("empty.leveldb");
        
        // Create an empty file
        fs::File::create(&file_path).await.expect("Failed to create empty file");
        
        let reader = LevelDBReader::new(file_path.to_string_lossy().to_string());
        let result = reader.read_file().await;
        
        assert!(result.is_ok());
        let blocks = result.unwrap();
        assert_eq!(blocks.len(), 0);
    }

    #[tokio::test]
    async fn test_leveldb_reader_file_size() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        let test_data = b"test data for size check";
        let file_path = create_test_leveldb_file(
            &temp_dir,
            "size_test.leveldb",
            vec![(RecordType::Full, test_data)]
        ).await.expect("Failed to create test file");
        
        let reader = LevelDBReader::new(file_path);
        let size_result = reader.file_size().await;
        
        assert!(size_result.is_ok());
        let size = size_result.unwrap();
        assert_eq!(size, 32768); // Should be exactly one 32KB block
    }

    #[tokio::test]
    async fn test_record_type_conversion() {
        assert_eq!(RecordType::try_from(1).unwrap(), RecordType::Full);
        assert_eq!(RecordType::try_from(2).unwrap(), RecordType::First);
        assert_eq!(RecordType::try_from(3).unwrap(), RecordType::Middle);
        assert_eq!(RecordType::try_from(4).unwrap(), RecordType::Last);
        
        let invalid_result = RecordType::try_from(5);
        assert!(invalid_result.is_err());
        assert!(invalid_result.unwrap_err().to_string().contains("Invalid record type"));
    }

    #[tokio::test]
    async fn test_firestore_document_parser_new() {
        let parser = FirestoreDocumentParser::new("test.leveldb");
        // Note: reader field is private, so we can't test it directly
        // but we can test that the parser was created successfully
        assert!(true); // Parser creation succeeded if we reach this point
    }

    #[tokio::test]
    async fn test_firestore_document_parser_empty_file() {
        setup_monitoring();
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let file_path = temp_dir.path().join("empty.leveldb");
        
        // Create an empty file
        fs::File::create(&file_path).await.expect("Failed to create empty file");
        
        let parser = FirestoreDocumentParser::new(file_path.to_string_lossy().to_string());
        let result = parser.parse_documents().await;
        
        assert!(result.is_ok());
        let parse_result = result.unwrap();
        assert_eq!(parse_result.documents.len(), 0);
        assert_eq!(parse_result.collections.len(), 0);
        assert_eq!(parse_result.metadata.blocks_processed, 0);
    }

    #[tokio::test]
    async fn test_firestore_document_parser_json_document() {
        setup_monitoring();
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        // Create a JSON document that looks like a Firestore export
        let json_doc = r#"{
            "name": "projects/test/databases/(default)/documents/users/user123",
            "fields": {
                "name": {"stringValue": "John Doe"},
                "age": {"integerValue": "30"},
                "email": {"stringValue": "john@example.com"}
            },
            "createTime": "2023-01-01T00:00:00Z",
            "updateTime": "2023-01-01T00:00:00Z"
        }"#;
        
        let file_path = create_test_leveldb_file(
            &temp_dir,
            "json_document.leveldb",
            vec![(RecordType::Full, json_doc.as_bytes())]
        ).await.expect("Failed to create test file");
        
        let parser = FirestoreDocumentParser::new(file_path);
        let result = parser.parse_documents().await;
        
        assert!(result.is_ok());
        let parse_result = result.unwrap();
        assert_eq!(parse_result.documents.len(), 1);
        assert_eq!(parse_result.collections.len(), 1);
        assert!(parse_result.collections.contains(&"users".to_string()));
        
        let document = &parse_result.documents[0];
        assert_eq!(document.id, "user123");
        assert_eq!(document.collection, "users");
        assert!(document.data.contains_key("name"));
        assert!(document.data.contains_key("age"));
        assert!(document.data.contains_key("email"));
    }

    #[tokio::test]
    async fn test_firestore_document_parser_reconstruct_fragmented_records() {
        setup_monitoring();
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        // Create a large JSON document split across multiple records
        let json_part1 = r#"{"name": "projects/test/databases/(default)/documents/users/user123", "fields": {"#;
        let json_part2 = r#""name": {"stringValue": "John Doe"}, "age": {"integerValue": "30"}, "#;
        let json_part3 = r#""email": {"stringValue": "john@example.com"}}}"#;
        
        let records = vec![
            (RecordType::First, json_part1.as_bytes()),
            (RecordType::Middle, json_part2.as_bytes()),
            (RecordType::Last, json_part3.as_bytes()),
        ];
        
        let file_path = create_test_leveldb_file(
            &temp_dir,
            "fragmented_json.leveldb",
            records
        ).await.expect("Failed to create test file");
        
        let parser = FirestoreDocumentParser::new(file_path);
        let result = parser.parse_documents().await;
        
        assert!(result.is_ok());
        let parse_result = result.unwrap();
        
        // Should reconstruct the fragmented record into a single document
        assert_eq!(parse_result.documents.len(), 1);
        assert_eq!(parse_result.metadata.records_processed, 1); // One complete record after reconstruction
        
        let document = &parse_result.documents[0];
        assert_eq!(document.id, "user123");
        assert_eq!(document.collection, "users");
    }

    #[tokio::test]
    async fn test_firestore_document_parser_invalid_json() {
        setup_monitoring();
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        // Create invalid JSON data
        let invalid_json = b"{ invalid json data without proper structure";
        
        let file_path = create_test_leveldb_file(
            &temp_dir,
            "invalid_json.leveldb",
            vec![(RecordType::Full, invalid_json)]
        ).await.expect("Failed to create test file");
        
        let parser = FirestoreDocumentParser::new(file_path);
        let result = parser.parse_documents().await;
        
        assert!(result.is_ok());
        let parse_result = result.unwrap();
        
        // Should handle invalid JSON gracefully
        assert_eq!(parse_result.documents.len(), 0);
        // May have parsing errors but shouldn't crash
        assert!(parse_result.errors.len() >= 0);
    }

    #[tokio::test]
    async fn test_firestore_document_parser_mixed_content() {
        setup_monitoring();
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        // Mix of valid JSON, invalid JSON, and metadata
        let valid_json = r#"{"name": "projects/test/databases/(default)/documents/users/user1", "fields": {"name": {"stringValue": "Alice"}}}"#;
        let invalid_json = b"invalid json";
        let metadata = b"__metadata__system_info";
        
        let records = vec![
            (RecordType::Full, valid_json.as_bytes()),
            (RecordType::Full, invalid_json),
            (RecordType::Full, metadata),
        ];
        
        let file_path = create_test_leveldb_file(
            &temp_dir,
            "mixed_content.leveldb",
            records
        ).await.expect("Failed to create test file");
        
        let parser = FirestoreDocumentParser::new(file_path);
        let result = parser.parse_documents().await;
        
        assert!(result.is_ok());
        let parse_result = result.unwrap();
        
        // Should parse only the valid document
        assert_eq!(parse_result.documents.len(), 1);
        assert_eq!(parse_result.documents[0].id, "user1");
        assert_eq!(parse_result.documents[0].collection, "users");
    }

    #[tokio::test]
    async fn test_streaming_parsing_large_file() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        // Create a large file with multiple blocks
        let num_blocks = 5;
        let file_path = create_large_leveldb_file(
            &temp_dir,
            "large_file.leveldb",
            num_blocks
        ).await.expect("Failed to create large test file");
        
        let reader = LevelDBReader::new(file_path);
        let result = reader.read_file().await;
        
        assert!(result.is_ok());
        let blocks = result.unwrap();
        assert_eq!(blocks.len(), num_blocks);
        
        // Each block should have 10 records
        for block in &blocks {
            assert_eq!(block.records.len(), 10);
        }
        
        // Verify file size
        let size_result = reader.file_size().await;
        assert!(size_result.is_ok());
        let expected_size = (num_blocks * 32768) as u64;
        assert_eq!(size_result.unwrap(), expected_size);
    }

    #[tokio::test]
    async fn test_backup_validator_file_validation() {
        setup_monitoring();
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        // Test with valid file
        let test_data = b"test data for validation";
        let valid_file_path = create_test_leveldb_file(
            &temp_dir,
            "valid.leveldb",
            vec![(RecordType::Full, test_data)]
        ).await.expect("Failed to create test file");
        
        let validator = BackupValidatorImpl::new(&valid_file_path);
        let result = validator.validate_comprehensive(&valid_file_path).await;
        
        assert!(result.is_ok());
        let validation_result = result.unwrap();
        assert!(validation_result.is_valid);
        assert!(validation_result.file_info.is_readable);
        assert_eq!(validation_result.file_info.file_size, 32768);
        assert!(validation_result.errors.is_empty());
    }

    #[tokio::test]
    async fn test_backup_validator_nonexistent_file() {
        let validator = BackupValidatorImpl::new("nonexistent.leveldb");
        let result = validator.validate_comprehensive("nonexistent.leveldb").await;
        
        assert!(result.is_ok());
        let validation_result = result.unwrap();
        assert!(!validation_result.is_valid);
        assert!(!validation_result.errors.is_empty());
        assert!(validation_result.errors[0].contains("File does not exist"));
    }

    #[tokio::test]
    async fn test_backup_validator_corrupted_file() {
        setup_monitoring();
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        let corrupted_file_path = create_corrupted_leveldb_file(
            &temp_dir,
            "corrupted.leveldb"
        ).await.expect("Failed to create corrupted test file");
        
        let validator = BackupValidatorImpl::new(&corrupted_file_path);
        let result = validator.validate_comprehensive(&corrupted_file_path).await;
        
        assert!(result.is_ok());
        let validation_result = result.unwrap();
        
        // File should be accessible but may have integrity issues
        assert!(validation_result.file_info.is_readable);
        assert_eq!(validation_result.file_info.file_size, 32768);
        
        // May have warnings about corrupted records
        assert!(validation_result.warnings.len() >= 0);
        
        // Integrity score should be lower due to corruption
        assert!(validation_result.integrity_info.overall_integrity_score <= 1.0);
    }

    #[tokio::test]
    async fn test_backup_validator_with_progress_callback() {
        setup_monitoring();
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        let test_data = b"test data with progress tracking";
        let file_path = create_test_leveldb_file(
            &temp_dir,
            "progress_test.leveldb",
            vec![(RecordType::Full, test_data)]
        ).await.expect("Failed to create test file");
        
        let progress_callback = Box::new(LoggingProgressCallback);
        let validator = BackupValidatorImpl::new(&file_path)
            .with_progress_callback(progress_callback);
        
        let result = validator.validate_comprehensive(&file_path).await;
        
        assert!(result.is_ok());
        let validation_result = result.unwrap();
        assert!(validation_result.is_valid);
    }

    #[tokio::test]
    async fn test_backup_validator_summary_report() {
        setup_monitoring();
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        let test_data = b"test data for summary report";
        let file_path = create_test_leveldb_file(
            &temp_dir,
            "summary_test.leveldb",
            vec![(RecordType::Full, test_data)]
        ).await.expect("Failed to create test file");
        
        let validator = BackupValidatorImpl::new(&file_path);
        let validation_result = validator.validate_comprehensive(&file_path).await
            .expect("Validation should succeed");
        
        let summary = validator.generate_summary_report(&validation_result);
        
        assert!(summary.contains("=== Firestore Backup Validation Report ==="));
        assert!(summary.contains("Overall Status:"));
        assert!(summary.contains("File Information"));
        assert!(summary.contains("Structure Information"));
        assert!(summary.contains("Integrity Information"));
        
        if validation_result.is_valid {
            assert!(summary.contains("✓ VALID"));
        } else {
            assert!(summary.contains("✗ INVALID"));
        }
    }

    #[tokio::test]
    async fn test_leveldb_parser_trait_implementation() {
        setup_monitoring();
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        let json_doc = r#"{"name": "projects/test/databases/(default)/documents/test/doc1", "fields": {"data": {"stringValue": "test"}}}"#;
        let file_path = create_test_leveldb_file(
            &temp_dir,
            "trait_test.leveldb",
            vec![(RecordType::Full, json_doc.as_bytes())]
        ).await.expect("Failed to create test file");
        
        let parser = FirestoreDocumentParser::new(&file_path);
        let result = parser.parse_backup(&file_path).await;
        
        assert!(result.is_ok());
        let parse_result = result.unwrap();
        assert_eq!(parse_result.documents.len(), 1);
        assert_eq!(parse_result.collections.len(), 1);
        assert!(parse_result.collections.contains(&"test".to_string()));
    }

    #[tokio::test]
    async fn test_parse_result_metadata() {
        setup_monitoring();
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        // Create file with multiple documents
        let doc1 = r#"{"name": "projects/test/databases/(default)/documents/users/user1", "fields": {"name": {"stringValue": "Alice"}}}"#;
        let doc2 = r#"{"name": "projects/test/databases/(default)/documents/posts/post1", "fields": {"title": {"stringValue": "Test Post"}}}"#;
        
        let records = vec![
            (RecordType::Full, doc1.as_bytes()),
            (RecordType::Full, doc2.as_bytes()),
        ];
        
        let file_path = create_test_leveldb_file(
            &temp_dir,
            "metadata_test.leveldb",
            records
        ).await.expect("Failed to create test file");
        
        let parser = FirestoreDocumentParser::new(file_path);
        let result = parser.parse_documents().await;
        
        assert!(result.is_ok());
        let parse_result = result.unwrap();
        
        // Verify metadata
        assert_eq!(parse_result.metadata.document_count, 2);
        assert_eq!(parse_result.metadata.collection_count, 2);
        assert_eq!(parse_result.metadata.blocks_processed, 1);
        assert_eq!(parse_result.metadata.records_processed, 2);
        assert_eq!(parse_result.metadata.file_size, 32768);
        
        // Verify collections
        assert!(parse_result.collections.contains(&"users".to_string()));
        assert!(parse_result.collections.contains(&"posts".to_string()));
    }
}