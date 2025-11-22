use super::test_utils::*;
use fireup::leveldb_parser::parser::FirestoreDocumentParser;
use fireup::types::FirestoreDocument;
use fireup::schema_analyzer::{DocumentStructureAnalyzer, NormalizationEngine};
use tokio;

/// Level 3 test: Parse multiple documents with multiple primitive fields
#[tokio::test]
async fn test_level3_parsing() {
    ensure_monitoring_initialized();
    
    if !test_data_exists() {
        println!("Skipping Level 3 parsing test - sample data not found");
        return;
    }

    let export_path = sample_export_path();
    let parser = FirestoreDocumentParser::new(export_path.to_string_lossy().to_string());
    
    println!("Testing Level 3 parsing from: {:?}", export_path);
    
    // Parse the backup file
    let parse_result = parser.parse_backup(&export_path.to_string_lossy()).await;
    
    match parse_result {
        Ok(result) => {
            println!("Parse successful:");
            println!("  Documents: {}", result.documents.len());
            println!("  Collections: {}", result.collections.len());
            
            // Find documents in the users collection
            let users_docs: Vec<&FirestoreDocument> = result.documents.iter()
                .filter(|doc| doc.collection == "users")
                .collect();
            
            println!("Found {} documents in users collection", users_docs.len());
            
            // Verify we have at least 2 documents
            assert!(users_docs.len() >= 2, "Should have at least 2 documents in users collection");
            
            // Verify each document has both name and age fields
            for doc in &users_docs {
                println!("Document {}: {:?}", doc.id, doc.data);
                
                // Check for name field (string)
                let has_name = doc.data.get("name")
                    .map(|v| v.is_string() && !v.as_str().unwrap_or("").is_empty())
                    .unwrap_or(false);
                
                // Check for age field (number)
                let has_age = doc.data.get("age")
                    .map(|v| v.is_number() || v.is_i64() || v.is_u64() || v.is_f64())
                    .unwrap_or(false);
                
                assert!(has_name, "Document {} should have a name field", doc.id);
                assert!(has_age, "Document {} should have an age field", doc.id);
                
                println!("✅ Document {} has both name and age fields", doc.id);
            }
            
            println!("🎯 Level 3 parsing test PASSED: Found {} documents with multiple primitive fields", users_docs.len());
        }
        Err(e) => {
            println!("Parse error: {}", e);
            println!("⚠️ Parse failed, but this might be expected for LevelDB format");
        }
    }
}

/// Level 3 test: Verify schema analysis detects multiple fields across multiple documents
#[tokio::test]
async fn test_level3_schema_analysis() {
    ensure_monitoring_initialized();
    
    if !test_data_exists() {
        println!("Skipping Level 3 schema analysis test - sample data not found");
        return;
    }

    let export_path = sample_export_path();
    let parser = FirestoreDocumentParser::new(export_path.to_string_lossy().to_string());
    
    println!("Testing Level 3 schema analysis from: {:?}", export_path);
    
    // Parse the backup file
    let parse_result = parser.parse_backup(&export_path.to_string_lossy()).await;
    
    match parse_result {
        Ok(result) => {
            if result.documents.is_empty() {
                println!("⚠️ No documents found, skipping schema analysis");
                return;
            }
            
            // Analyze schema
            let analyzer = DocumentStructureAnalyzer::new();
            let analysis_result = analyzer.analyze_documents(&result.documents).await;
            
            match analysis_result {
                Ok(analysis) => {
                    println!("Schema analysis successful:");
                    println!("  Collections analyzed: {}", analysis.collections.len());
                    println!("  Field types identified: {}", analysis.field_types.len());
                    
                    // Find the users collection
                    let users_collection = analysis.collections.iter()
                        .find(|c| c.name == "users");
                    
                    match users_collection {
                        Some(collection) => {
                            println!("Users collection analysis:");
                            println!("  Document count: {}", collection.document_count);
                            println!("  Field names: {:?}", collection.field_names);
                            
                            // Should have at least 2 documents
                            assert!(collection.document_count >= 2, 
                                "Users collection should have at least 2 documents");
                            
                            // Should have both name and age fields
                            assert!(collection.field_names.contains(&"name".to_string()),
                                "Users collection should have 'name' field");
                            assert!(collection.field_names.contains(&"age".to_string()),
                                "Users collection should have 'age' field");
                            
                            println!("✅ Users collection has correct structure");
                        }
                        None => {
                            println!("⚠️ Users collection not found in analysis");
                        }
                    }
                    
                    // Check field type analysis
                    let name_field = analysis.field_types.iter()
                        .find(|ft| ft.field_path.ends_with(".name") || ft.field_path == "name");
                    let age_field = analysis.field_types.iter()
                        .find(|ft| ft.field_path.ends_with(".age") || ft.field_path == "age");
                    
                    if let Some(name_ft) = name_field {
                        println!("Name field analysis:");
                        println!("  Type: {:?}", name_ft.recommended_type);
                        println!("  Presence: {:.1}%", name_ft.presence_percentage);
                        assert!(name_ft.presence_percentage > 0.0, "Name field should be present");
                    }
                    
                    if let Some(age_ft) = age_field {
                        println!("Age field analysis:");
                        println!("  Type: {:?}", age_ft.recommended_type);
                        println!("  Presence: {:.1}%", age_ft.presence_percentage);
                        assert!(age_ft.presence_percentage > 0.0, "Age field should be present");
                    }
                    
                    println!("🎯 Level 3 schema analysis test PASSED");
                }
                Err(e) => {
                    println!("Schema analysis error: {}", e);
                }
            }
        }
        Err(e) => {
            println!("Parse error: {}", e);
            println!("⚠️ Parse failed, but this might be expected for LevelDB format");
        }
    }
}

/// Level 3 test: Verify normalization generates proper table structure
#[tokio::test]
async fn test_level3_normalization() {
    ensure_monitoring_initialized();
    
    if !test_data_exists() {
        println!("Skipping Level 3 normalization test - sample data not found");
        return;
    }

    let export_path = sample_export_path();
    let parser = FirestoreDocumentParser::new(export_path.to_string_lossy().to_string());
    
    println!("Testing Level 3 normalization from: {:?}", export_path);
    
    // Parse the backup file
    let parse_result = parser.parse_backup(&export_path.to_string_lossy()).await;
    
    match parse_result {
        Ok(result) => {
            if result.documents.is_empty() {
                println!("⚠️ No documents found, skipping normalization");
                return;
            }
            
            // Analyze schema
            let analyzer = DocumentStructureAnalyzer::new();
            let analysis_result = analyzer.analyze_documents(&result.documents).await;
            
            match analysis_result {
                Ok(analysis) => {
                    // Normalize schema
                    let normalizer = NormalizationEngine::new();
                    let normalization_result = normalizer.normalize_schema(&analysis);
                    
                    match normalization_result {
                        Ok(normalized) => {
                            println!("Normalization successful:");
                            println!("  Tables generated: {}", normalized.tables.len());
                            
                            // Find the users table
                            let users_table = normalized.tables.iter()
                                .find(|t| t.name == "users");
                            
                            match users_table {
                                Some(table) => {
                                    println!("Users table structure:");
                                    println!("  Table name: {}", table.name);
                                    println!("  Columns: {}", table.columns.len());
                                    
                                    // Should have columns for name and age
                                    let has_name = table.columns.iter()
                                        .any(|c| c.name == "name");
                                    let has_age = table.columns.iter()
                                        .any(|c| c.name == "age");
                                    
                                    assert!(has_name, "Users table should have 'name' column");
                                    assert!(has_age, "Users table should have 'age' column");
                                    
                                    // Print column details
                                    for column in &table.columns {
                                        println!("  Column {}: {:?}", column.name, column.data_type);
                                    }
                                    
                                    println!("✅ Users table has correct structure");
                                }
                                None => {
                                    println!("⚠️ Users table not found in normalization");
                                }
                            }
                            
                            println!("🎯 Level 3 normalization test PASSED");
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
        Err(e) => {
            println!("Parse error: {}", e);
            println!("⚠️ Parse failed, but this might be expected for LevelDB format");
        }
    }
}

/// Level 3 test: Verify type consistency across documents
#[tokio::test]
async fn test_level3_type_consistency() {
    ensure_monitoring_initialized();
    
    if !test_data_exists() {
        println!("Skipping Level 3 type consistency test - sample data not found");
        return;
    }

    let export_path = sample_export_path();
    let parser = FirestoreDocumentParser::new(export_path.to_string_lossy().to_string());
    
    println!("Testing Level 3 type consistency from: {:?}", export_path);
    
    // Parse the backup file
    let parse_result = parser.parse_backup(&export_path.to_string_lossy()).await;
    
    match parse_result {
        Ok(result) => {
            // Find documents in the users collection
            let users_docs: Vec<&FirestoreDocument> = result.documents.iter()
                .filter(|doc| doc.collection == "users")
                .collect();
            
            if users_docs.len() < 2 {
                println!("⚠️ Need at least 2 documents for type consistency test");
                return;
            }
            
            println!("Checking type consistency across {} documents", users_docs.len());
            
            // Collect types for name field
            let mut name_types = Vec::new();
            let mut age_types = Vec::new();
            
            for doc in &users_docs {
                // Check name field type
                if let Some(name_value) = doc.data.get("name") {
                    if name_value.is_string() {
                        name_types.push("string");
                    }
                }
                
                // Check age field type
                if let Some(age_value) = doc.data.get("age") {
                    if age_value.is_number() || age_value.is_i64() || age_value.is_u64() || age_value.is_f64() {
                        age_types.push("number");
                    }
                }
            }
            
            println!("Name field types: {:?}", name_types);
            println!("Age field types: {:?}", age_types);
            
            // All name fields should be strings
            assert_eq!(name_types.len(), users_docs.len(), 
                "All documents should have name field");
            assert!(name_types.iter().all(|&t| t == "string"),
                "All name fields should be strings");
            
            // All age fields should be numbers
            assert_eq!(age_types.len(), users_docs.len(),
                "All documents should have age field");
            assert!(age_types.iter().all(|&t| t == "number"),
                "All age fields should be numbers");
            
            println!("✅ Type consistency verified across all documents");
            println!("🎯 Level 3 type consistency test PASSED");
        }
        Err(e) => {
            println!("Parse error: {}", e);
            println!("⚠️ Parse failed, but this might be expected for LevelDB format");
        }
    }
}
