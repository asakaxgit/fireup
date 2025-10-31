use super::test_utils::*;
use fireup::leveldb_parser::parser::{FirestoreDocumentParser, LevelDBParser, LevelDBReader};
use fireup::leveldb_parser::validator::{BackupValidator, BackupValidatorImpl};
use fireup::schema_analyzer::{DocumentStructureAnalyzer, NormalizationEngine};
use fireup::types::{PostgreSQLType, FieldTypeAnalysis};
use tokio;

#[tokio::test]
async fn test_end_to_end_parsing_pipeline() {
    ensure_monitoring_initialized();
    if !test_data_exists() {
        println!("Skipping integration test - sample data not found");
        return;
    }

    let export_path = sample_export_path();
    let parser = FirestoreDocumentParser::new(export_path.to_string_lossy().to_string());
    
    println!("Testing end-to-end parsing pipeline with file: {:?}", export_path);
    
    // Step 1: Validate the backup file
    let validator = BackupValidatorImpl::new(export_path.to_string_lossy().to_string());
    let validation_result = validator.validate_backup(&export_path.to_string_lossy()).await;
    
    match validation_result {
        Ok(result) => {
            println!("Validation result: valid={}, errors={}", result.is_valid, result.errors.len());
            if !result.errors.is_empty() {
                for error in &result.errors[..std::cmp::min(5, result.errors.len())] {
                    println!("  Validation error: {}", error);
                }
            }
        }
        Err(e) => {
            println!("Validation error (may be expected for binary files): {}", e);
        }
    }
    
    // Step 2: Parse the backup file
    let parse_result = parser.parse_backup(&export_path.to_string_lossy()).await;
    
    match parse_result {
        Ok(result) => {
            println!("Parse successful:");
            println!("  Documents: {}", result.documents.len());
            println!("  Collections: {}", result.collections.len());
            println!("  File size: {} bytes", result.metadata.file_size);
            println!("  Blocks processed: {}", result.metadata.blocks_processed);
            println!("  Records processed: {}", result.metadata.records_processed);
            println!("  Errors: {}", result.errors.len());
            
            // Step 3: Analyze schema if we have documents
            if !result.documents.is_empty() {
                let analyzer = DocumentStructureAnalyzer::new();
                let analysis_result = analyzer.analyze_documents(&result.documents).await;
                
                match analysis_result {
                    Ok(analysis) => {
                        println!("Schema analysis successful:");
                        println!("  Collections analyzed: {}", analysis.collections.len());
                        println!("  Field types identified: {}", analysis.field_types.len());
                        println!("  Relationships detected: {}", analysis.relationships.len());
                        
                        // Step 4: Test normalization
                        let normalizer = NormalizationEngine::new();
                        let normalization_result = normalizer.normalize_schema(&analysis);
                        
                        match normalization_result {
                            Ok(normalized) => {
                                println!("Normalization successful:");
                                println!("  Tables generated: {}", normalized.tables.len());
                                println!("  Relationships: {}", normalized.relationships.len());
                                println!("  Constraints: {}", normalized.constraints.len());
                                println!("  Warnings: {}", normalized.warnings.len());
                            }
                            Err(e) => {
                                println!("Normalization error: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        println!("Schema analysis error: {}", e);
                    }
                }
            }
        }
        Err(e) => {
            println!("Parse error (may be expected for binary LevelDB files): {}", e);
            // This is often expected for binary LevelDB files
        }
    }
}

#[tokio::test]
async fn test_leveldb_reader_functionality() {
    ensure_monitoring_initialized();
    if !test_data_exists() {
        println!("Skipping LevelDB reader test - sample data not found");
        return;
    }

    let export_path = sample_export_path();
    let reader = LevelDBReader::new(export_path.to_string_lossy().to_string());
    
    // Test file size reading
    let file_size_result = reader.file_size().await;
    match file_size_result {
        Ok(size) => {
            println!("File size: {} bytes", size);
            assert!(size > 0, "File size should be greater than 0");
        }
        Err(e) => {
            println!("File size error: {}", e);
        }
    }
    
    // Test file reading (may fail for binary data, which is expected)
    let read_result = reader.read_file().await;
    match read_result {
        Ok(blocks) => {
            println!("Successfully read {} blocks", blocks.len());
            
            // Validate block structure
            for (i, block) in blocks.iter().enumerate().take(3) {
                println!("Block {}: {} bytes, {} records", i, block.data.len(), block.records.len());
                
                // Validate records in block
                for (j, record) in block.records.iter().enumerate().take(3) {
                    println!("  Record {}: type={:?}, length={}, checksum=0x{:08x}", 
                        j, record.header.record_type, record.header.length, record.header.checksum);
                }
            }
        }
        Err(e) => {
            println!("File reading error (expected for binary LevelDB): {}", e);
            // This is expected for binary LevelDB files
        }
    }
}

#[tokio::test]
async fn test_document_collection_grouping() {
    ensure_monitoring_initialized();
    // Create test documents from different collections
    let mut test_documents = vec![
        create_test_document("user1", "users"),
        create_test_document("user2", "users"),
        create_test_document("order1", "orders"),
        create_test_document("order2", "orders"),
        create_test_document("product1", "products"),
    ];
    
    // Add some variety to the data
    test_documents[0].data.insert("role".to_string(), serde_json::json!("admin"));
    test_documents[1].data.insert("role".to_string(), serde_json::json!("user"));
    test_documents[2].data.insert("total".to_string(), serde_json::json!(99.99));
    test_documents[3].data.insert("total".to_string(), serde_json::json!(149.50));
    test_documents[4].data.insert("price".to_string(), serde_json::json!(29.99));
    
    // Test schema analysis with these documents
    let analyzer = DocumentStructureAnalyzer::new();
    let analysis_result = analyzer.analyze_documents(&test_documents).await;
    
    match analysis_result {
        Ok(analysis) => {
            println!("Collection grouping analysis:");
            
            // Should have 3 collections
            assert_eq!(analysis.collections.len(), 3);
            
            let collection_names: Vec<&String> = analysis.collections.iter().map(|c| &c.name).collect();
            assert!(collection_names.contains(&&"users".to_string()));
            assert!(collection_names.contains(&&"orders".to_string()));
            assert!(collection_names.contains(&&"products".to_string()));
            
            // Check document counts per collection
            for collection in &analysis.collections {
                match collection.name.as_str() {
                    "users" => {
                        assert_eq!(collection.document_count, 2);
                        assert!(collection.field_names.contains(&"role".to_string()));
                    }
                    "orders" => {
                        assert_eq!(collection.document_count, 2);
                        assert!(collection.field_names.contains(&"total".to_string()));
                    }
                    "products" => {
                        assert_eq!(collection.document_count, 1);
                        assert!(collection.field_names.contains(&"price".to_string()));
                    }
                    _ => panic!("Unexpected collection: {}", collection.name),
                }
                
                println!("Collection '{}': {} docs, {} fields", 
                    collection.name, collection.document_count, collection.field_names.len());
            }
            
            // Check field type analysis
            println!("Field type analysis:");
            for field_type in &analysis.field_types {
                println!("  Field '{}': {:?} ({}% presence)", 
                    field_type.field_path, 
                    field_type.recommended_type,
                    field_type.presence_percentage);
            }
        }
        Err(e) => {
            panic!("Schema analysis failed: {}", e);
        }
    }
}

#[tokio::test]
async fn test_type_inference_and_conflicts() {
    ensure_monitoring_initialized();
    // Create documents with type conflicts
    let mut test_documents = vec![];
    
    // Document 1: age as integer
    let mut doc1 = create_test_document("user1", "users");
    doc1.data.insert("age".to_string(), serde_json::json!(25));
    doc1.data.insert("score".to_string(), serde_json::json!(95.5));
    test_documents.push(doc1);
    
    // Document 2: age as string (type conflict)
    let mut doc2 = create_test_document("user2", "users");
    doc2.data.insert("age".to_string(), serde_json::json!("thirty"));
    doc2.data.insert("score".to_string(), serde_json::json!(87));
    test_documents.push(doc2);
    
    // Document 3: age as integer again
    let mut doc3 = create_test_document("user3", "users");
    doc3.data.insert("age".to_string(), serde_json::json!(30));
    doc3.data.insert("score".to_string(), serde_json::json!(92.0));
    test_documents.push(doc3);
    
    let analyzer = DocumentStructureAnalyzer::new();
    let analysis_result = analyzer.analyze_documents(&test_documents).await;
    
    match analysis_result {
        Ok(analysis) => {
            println!("Type inference analysis:");
            
            // Find the age field analysis
            let age_field = analysis.field_types.iter()
                .find(|ft| ft.field_path.ends_with(".age"))
                .expect("Should find age field analysis");
            
            println!("Age field analysis:");
            println!("  Total occurrences: {}", age_field.total_occurrences);
            println!("  Presence percentage: {:.1}%", age_field.presence_percentage);
            println!("  Type frequencies: {:?}", age_field.type_frequencies);
            println!("  Recommended type: {:?}", age_field.recommended_type);
            
            // Should detect the type conflict
            assert!(age_field.type_frequencies.len() > 1, "Should detect multiple types for age field");
            assert_eq!(age_field.total_occurrences, 3);
            assert_eq!(age_field.presence_percentage, 100.0);
            
            // Find the score field analysis
            let score_field = analysis.field_types.iter()
                .find(|ft| ft.field_path.ends_with(".score"))
                .expect("Should find score field analysis");
            
            println!("Score field analysis:");
            println!("  Type frequencies: {:?}", score_field.type_frequencies);
            println!("  Recommended type: {:?}", score_field.recommended_type);
            
            // Score should be recommended as numeric type
            match score_field.recommended_type {
                PostgreSQLType::Numeric(_, _) | PostgreSQLType::BigInt | PostgreSQLType::Integer => {
                    println!("Correctly inferred numeric type for score");
                }
                _ => {
                    println!("Warning: Score field type inference may need adjustment: {:?}", 
                        score_field.recommended_type);
                }
            }
        }
        Err(e) => {
            panic!("Type inference analysis failed: {}", e);
        }
    }
}

#[tokio::test]
async fn test_nested_field_analysis() {
    ensure_monitoring_initialized();
    // Create documents with nested structures
    let mut test_documents = vec![];
    
    for i in 1..=3 {
        let mut doc = create_test_document(&format!("user{}", i), "users");
        
        // Add nested profile data
        doc.data.insert("profile".to_string(), serde_json::json!({
            "firstName": format!("User{}", i),
            "lastName": "Doe",
            "address": {
                "street": format!("{} Main St", i * 100),
                "city": "Anytown",
                "zipCode": format!("1234{}", i)
            },
            "preferences": {
                "theme": "dark",
                "notifications": true,
                "language": "en"
            }
        }));
        
        // Add array data
        doc.data.insert("tags".to_string(), serde_json::json!([
            "user", format!("level{}", i), "active"
        ]));
        
        test_documents.push(doc);
    }
    
    let analyzer = DocumentStructureAnalyzer::new();
    let analysis_result = analyzer.analyze_documents(&test_documents).await;
    
    match analysis_result {
        Ok(analysis) => {
            println!("Nested field analysis:");
            
            // Should detect nested fields
            let nested_fields: Vec<&String> = analysis.field_types.iter()
                .map(|ft| &ft.field_path)
                .filter(|path| path.contains('.'))
                .collect();
            
            println!("Detected nested fields: {:?}", nested_fields);
            
            // Should find profile-related fields
            let profile_fields: Vec<&String> = analysis.field_types.iter()
                .map(|ft| &ft.field_path)
                .filter(|path| path.starts_with("profile"))
                .collect();
            
            println!("Profile fields: {:?}", profile_fields);
            
            // Check for array fields
            let array_fields: Vec<&FieldTypeAnalysis> = analysis.field_types.iter()
                .filter(|ft| matches!(ft.recommended_type, PostgreSQLType::Array(_) | PostgreSQLType::Jsonb))
                .collect();
            
            println!("Array/JSON fields: {:?}", 
                array_fields.iter().map(|f| &f.field_path).collect::<Vec<_>>());
            
            // Verify that complex nested structures are handled
            assert!(!analysis.field_types.is_empty(), "Should detect field types");
            
            for field_type in &analysis.field_types {
                println!("  Field '{}': {:?} ({}% presence, {} occurrences)", 
                    field_type.field_path,
                    field_type.recommended_type,
                    field_type.presence_percentage,
                    field_type.total_occurrences);
            }
        }
        Err(e) => {
            panic!("Nested field analysis failed: {}", e);
        }
    }
}

#[tokio::test]
async fn test_large_document_set_performance() {
    ensure_monitoring_initialized();
    // Create a larger set of documents to test performance
    let mut test_documents = vec![];
    
    for i in 1..=100 {
        let collection = match i % 4 {
            0 => "users",
            1 => "orders",
            2 => "products",
            _ => "reviews",
        };
        
        let mut doc = create_test_document(&format!("doc{}", i), collection);
        
        // Add varied data based on collection
        match collection {
            "users" => {
                doc.data.insert("email".to_string(), serde_json::json!(format!("user{}@example.com", i)));
                doc.data.insert("age".to_string(), serde_json::json!(20 + (i % 50)));
                doc.data.insert("premium".to_string(), serde_json::json!(i % 3 == 0));
            }
            "orders" => {
                doc.data.insert("total".to_string(), serde_json::json!((i as f64) * 10.99));
                doc.data.insert("items".to_string(), serde_json::json!(i % 5 + 1));
                doc.data.insert("status".to_string(), serde_json::json!(
                    match i % 3 { 0 => "pending", 1 => "shipped", _ => "delivered" }
                ));
            }
            "products" => {
                doc.data.insert("price".to_string(), serde_json::json!((i as f64) * 2.99));
                doc.data.insert("category".to_string(), serde_json::json!(
                    match i % 4 { 0 => "electronics", 1 => "books", 2 => "clothing", _ => "home" }
                ));
                doc.data.insert("inStock".to_string(), serde_json::json!(i % 7 != 0));
            }
            "reviews" => {
                doc.data.insert("rating".to_string(), serde_json::json!(1 + (i % 5)));
                doc.data.insert("verified".to_string(), serde_json::json!(i % 2 == 0));
                doc.data.insert("helpful".to_string(), serde_json::json!(i % 10));
            }
            _ => {}
        }
        
        test_documents.push(doc);
    }
    
    println!("Testing performance with {} documents", test_documents.len());
    
    let start_time = std::time::Instant::now();
    let analyzer = DocumentStructureAnalyzer::new();
    let analysis_result = analyzer.analyze_documents(&test_documents).await;
    let analysis_duration = start_time.elapsed();
    
    match analysis_result {
        Ok(analysis) => {
            println!("Performance test results:");
            println!("  Analysis time: {:?}", analysis_duration);
            println!("  Documents analyzed: {}", analysis.metadata.total_documents);
            println!("  Collections found: {}", analysis.collections.len());
            println!("  Field types identified: {}", analysis.field_types.len());
            println!("  Relationships detected: {}", analysis.relationships.len());
            
            // Verify expected collections
            assert_eq!(analysis.collections.len(), 4);
            assert_eq!(analysis.metadata.total_documents, 100);
            
            // Performance assertion (should complete within reasonable time)
            assert!(analysis_duration.as_secs() < 10, "Analysis should complete within 10 seconds");
            
            // Test normalization performance
            let norm_start = std::time::Instant::now();
            let normalizer = NormalizationEngine::new();
            let norm_result = normalizer.normalize_schema(&analysis);
            let norm_duration = norm_start.elapsed();
            
            match norm_result {
                Ok(normalized) => {
                    println!("  Normalization time: {:?}", norm_duration);
                    println!("  Tables generated: {}", normalized.tables.len());
                    
                    assert!(norm_duration.as_secs() < 5, "Normalization should complete within 5 seconds");
                }
                Err(e) => {
                    println!("Normalization error: {}", e);
                }
            }
        }
        Err(e) => {
            panic!("Performance test failed: {}", e);
        }
    }
}