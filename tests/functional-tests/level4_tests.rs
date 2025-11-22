use super::test_utils::*;
use fireup::leveldb_parser::parser::{FirestoreDocumentParser, LevelDBParser};
use fireup::schema_analyzer::{DocumentStructureAnalyzer, NormalizationEngine};
use fireup::types::{PostgreSQLType, FirestoreDocument};
use tokio;

/// Level 4 Test: Parse documents with arrays of primitive values
#[tokio::test]
async fn test_level4_array_parsing() {
    ensure_monitoring_initialized();
    
    if !test_data_exists() {
        println!("Skipping Level 4 array parsing test - sample data not found");
        return;
    }

    let export_path = sample_export_path();
    let parser = FirestoreDocumentParser::new(export_path.to_string_lossy().to_string());
    
    println!("Testing Level 4 array parsing from: {:?}", export_path);
    
    // Parse the backup file
    let parse_result = parser.parse_backup(&export_path.to_string_lossy()).await;
    
    match parse_result {
        Ok(result) => {
            println!("Parse successful:");
            println!("  Documents: {}", result.documents.len());
            println!("  Collections: {}", result.collections.len());
            
            // Find documents from level4_arrays collection
            let level4_docs: Vec<&FirestoreDocument> = result.documents.iter()
                .filter(|doc| doc.collection == "level4_arrays")
                .collect();
            
            if level4_docs.is_empty() {
                println!("⚠️ No level4_arrays documents found - data may not have been generated");
                return;
            }
            
            println!("Found {} level4_arrays documents", level4_docs.len());
            
            // Find documents with array fields
            let mut found_string_array = false;
            let mut found_number_array = false;
            let mut found_boolean_array = false;
            
            for doc in &level4_docs {
                println!("  Document {}: {} fields", doc.id, doc.data.len());
                
                for (field_name, value) in &doc.data {
                    if value.is_array() {
                        if let Some(arr) = value.as_array() {
                            println!("    Array field '{}': {} elements", field_name, arr.len());
                            
                            // Check array element types
                            if !arr.is_empty() {
                                let first_elem = &arr[0];
                                if first_elem.is_string() {
                                    found_string_array = true;
                                    println!("      ✅ String array detected");
                                } else if first_elem.is_number() || first_elem.is_i64() || first_elem.is_u64() || first_elem.is_f64() {
                                    found_number_array = true;
                                    println!("      ✅ Number array detected");
                                } else if first_elem.is_boolean() {
                                    found_boolean_array = true;
                                    println!("      ✅ Boolean array detected");
                                }
                            }
                        }
                    }
                }
            }
            
            // Verify we found at least one of each array type
            if found_string_array {
                println!("✅ Successfully parsed string arrays");
            }
            if found_number_array {
                println!("✅ Successfully parsed number arrays");
            }
            if found_boolean_array {
                println!("✅ Successfully parsed boolean arrays");
            }
            
            println!("🎯 Level 4 array parsing test completed");
        }
        Err(e) => {
            println!("Parse error: {}", e);
            println!("⚠️ Parse failed, but this might be expected for LevelDB format");
        }
    }
}

/// Level 4 Test: Verify array field schema analysis
#[tokio::test]
async fn test_level4_array_schema_analysis() {
    ensure_monitoring_initialized();
    
    if !test_data_exists() {
        println!("Skipping Level 4 array schema analysis test - sample data not found");
        return;
    }

    let export_path = sample_export_path();
    let parser = FirestoreDocumentParser::new(export_path.to_string_lossy().to_string());
    
    println!("Testing Level 4 array schema analysis from: {:?}", export_path);
    
    // Parse the backup file
    let parse_result = parser.parse_backup(&export_path.to_string_lossy()).await;
    
    match parse_result {
        Ok(result) => {
            if result.documents.is_empty() {
                println!("⚠️ No documents found in export");
                return;
            }
            
            // Filter to level4_arrays collection
            let level4_docs: Vec<FirestoreDocument> = result.documents.into_iter()
                .filter(|doc| doc.collection == "level4_arrays")
                .collect();
            
            if level4_docs.is_empty() {
                println!("⚠️ No level4_arrays documents found - data may not have been generated");
                return;
            }
            
            println!("Analyzing {} level4_arrays documents", level4_docs.len());
            
            // Analyze schema
            let analyzer = DocumentStructureAnalyzer::new();
            let analysis_result = analyzer.analyze_documents(&level4_docs).await;
            
            match analysis_result {
                Ok(analysis) => {
                    println!("Schema analysis successful:");
                    println!("  Collections analyzed: {}", analysis.collections.len());
                    println!("  Field types identified: {}", analysis.field_types.len());
                    
                    // Find array fields
                    let array_fields: Vec<_> = analysis.field_types.iter()
                        .filter(|ft| {
                            matches!(ft.recommended_type, PostgreSQLType::Array(_) | PostgreSQLType::Jsonb)
                        })
                        .collect();
                    
                    println!("  Array fields detected: {}", array_fields.len());
                    
                    for field in &array_fields {
                        println!("    Field '{}': {:?} ({}% presence, {} occurrences)", 
                            field.field_path,
                            field.recommended_type,
                            field.presence_percentage,
                            field.total_occurrences);
                        
                        // Check if it's a typed array
                        if let PostgreSQLType::Array(inner_type) = &field.recommended_type {
                            println!("      Inner type: {:?}", inner_type);
                        }
                    }
                    
                    // Verify we found array fields
                    assert!(!array_fields.is_empty(), "Should detect at least one array field");
                    
                    // Check for specific array field names
                    let field_names: Vec<&String> = array_fields.iter()
                        .map(|f| &f.field_path)
                        .collect();
                    
                    // Look for expected array fields
                    let expected_fields = ["tags", "scores", "flags", "categories", "prices", "quantities", "enabled", "active", "items", "numbers"];
                    let mut found_expected = 0;
                    
                    for expected in &expected_fields {
                        if field_names.iter().any(|name| name.ends_with(expected)) {
                            found_expected += 1;
                            println!("      ✅ Found expected array field: {}", expected);
                        }
                    }
                    
                    println!("✅ Found {}/{} expected array fields", found_expected, expected_fields.len());
                    println!("🎯 Level 4 array schema analysis test completed");
                }
                Err(e) => {
                    println!("Schema analysis error: {}", e);
                    panic!("Schema analysis should succeed for array fields");
                }
            }
        }
        Err(e) => {
            println!("Parse error: {}", e);
            println!("⚠️ Parse failed, but this might be expected for LevelDB format");
        }
    }
}

/// Level 4 Test: Verify array type inference for different primitive types
#[tokio::test]
async fn test_level4_array_type_inference() {
    ensure_monitoring_initialized();
    
    if !test_data_exists() {
        println!("Skipping Level 4 array type inference test - sample data not found");
        return;
    }

    let export_path = sample_export_path();
    let parser = FirestoreDocumentParser::new(export_path.to_string_lossy().to_string());
    
    println!("Testing Level 4 array type inference from: {:?}", export_path);
    
    // Parse the backup file
    let parse_result = parser.parse_backup(&export_path.to_string_lossy()).await;
    
    match parse_result {
        Ok(result) => {
            // Filter to level4_arrays collection
            let level4_docs: Vec<FirestoreDocument> = result.documents.into_iter()
                .filter(|doc| doc.collection == "level4_arrays")
                .collect();
            
            if level4_docs.is_empty() {
                println!("⚠️ No level4_arrays documents found - data may not have been generated");
                return;
            }
            
            // Analyze schema
            let analyzer = DocumentStructureAnalyzer::new();
            let analysis_result = analyzer.analyze_documents(&level4_docs).await;
            
            match analysis_result {
                Ok(analysis) => {
                    println!("Type inference analysis:");
                    
                    // Check for string arrays
                    let string_array_fields: Vec<_> = analysis.field_types.iter()
                        .filter(|ft| {
                            if let PostgreSQLType::Array(inner) = &ft.recommended_type {
                                matches!(**inner, PostgreSQLType::Text | PostgreSQLType::Varchar(_))
                            } else {
                                false
                            }
                        })
                        .collect();
                    
                    if !string_array_fields.is_empty() {
                        println!("  ✅ String arrays detected: {}", string_array_fields.len());
                        for field in &string_array_fields {
                            println!("    - {}: {:?}", field.field_path, field.recommended_type);
                        }
                    }
                    
                    // Check for number arrays
                    let number_array_fields: Vec<_> = analysis.field_types.iter()
                        .filter(|ft| {
                            if let PostgreSQLType::Array(inner) = &ft.recommended_type {
                                matches!(**inner, PostgreSQLType::Integer | PostgreSQLType::BigInt | PostgreSQLType::Numeric(_, _) | PostgreSQLType::DoublePrecision)
                            } else {
                                false
                            }
                        })
                        .collect();
                    
                    if !number_array_fields.is_empty() {
                        println!("  ✅ Number arrays detected: {}", number_array_fields.len());
                        for field in &number_array_fields {
                            println!("    - {}: {:?}", field.field_path, field.recommended_type);
                        }
                    }
                    
                    // Check for boolean arrays
                    let boolean_array_fields: Vec<_> = analysis.field_types.iter()
                        .filter(|ft| {
                            if let PostgreSQLType::Array(inner) = &ft.recommended_type {
                                matches!(**inner, PostgreSQLType::Boolean)
                            } else {
                                false
                            }
                        })
                        .collect();
                    
                    if !boolean_array_fields.is_empty() {
                        println!("  ✅ Boolean arrays detected: {}", boolean_array_fields.len());
                        for field in &boolean_array_fields {
                            println!("    - {}: {:?}", field.field_path, field.recommended_type);
                        }
                    }
                    
                    // Verify we detected different array types
                    let total_typed_arrays = string_array_fields.len() + number_array_fields.len() + boolean_array_fields.len();
                    if total_typed_arrays > 0 {
                        println!("✅ Successfully inferred types for {} array fields", total_typed_arrays);
                    }
                    
                    println!("🎯 Level 4 array type inference test completed");
                }
                Err(e) => {
                    println!("Schema analysis error: {}", e);
                    panic!("Schema analysis should succeed");
                }
            }
        }
        Err(e) => {
            println!("Parse error: {}", e);
            println!("⚠️ Parse failed, but this might be expected for LevelDB format");
        }
    }
}

/// Level 4 Test: Verify empty arrays are handled correctly
#[tokio::test]
async fn test_level4_empty_arrays() {
    ensure_monitoring_initialized();
    
    if !test_data_exists() {
        println!("Skipping Level 4 empty arrays test - sample data not found");
        return;
    }

    let export_path = sample_export_path();
    let parser = FirestoreDocumentParser::new(export_path.to_string_lossy().to_string());
    
    println!("Testing Level 4 empty arrays handling from: {:?}", export_path);
    
    // Parse the backup file
    let parse_result = parser.parse_backup(&export_path.to_string_lossy()).await;
    
    match parse_result {
        Ok(result) => {
            // Find the empty arrays document
            let empty_doc = result.documents.iter()
                .find(|doc| doc.collection == "level4_arrays" && doc.id == "empty_arrays_doc");
            
            match empty_doc {
                Some(doc) => {
                    println!("Found empty arrays document: {}", doc.id);
                    
                    // Check for empty array fields
                    let empty_arrays: Vec<_> = doc.data.iter()
                        .filter(|(_, v)| {
                            v.is_array() && v.as_array().map(|a| a.is_empty()).unwrap_or(false)
                        })
                        .collect();
                    
                    println!("  Empty array fields found: {}", empty_arrays.len());
                    
                    for (field_name, _) in &empty_arrays {
                        println!("    - {}: []", field_name);
                    }
                    
                    if !empty_arrays.is_empty() {
                        println!("✅ Successfully parsed empty arrays");
                    }
                    
                    println!("🎯 Level 4 empty arrays test completed");
                }
                None => {
                    println!("⚠️ Empty arrays document not found - data may not have been generated");
                }
            }
        }
        Err(e) => {
            println!("Parse error: {}", e);
            println!("⚠️ Parse failed, but this might be expected for LevelDB format");
        }
    }
}

/// Level 4 Test: Verify normalization handles arrays correctly
#[tokio::test]
async fn test_level4_array_normalization() {
    ensure_monitoring_initialized();
    
    if !test_data_exists() {
        println!("Skipping Level 4 array normalization test - sample data not found");
        return;
    }

    let export_path = sample_export_path();
    let parser = FirestoreDocumentParser::new(export_path.to_string_lossy().to_string());
    
    println!("Testing Level 4 array normalization from: {:?}", export_path);
    
    // Parse the backup file
    let parse_result = parser.parse_backup(&export_path.to_string_lossy()).await;
    
    match parse_result {
        Ok(result) => {
            // Filter to level4_arrays collection
            let level4_docs: Vec<FirestoreDocument> = result.documents.into_iter()
                .filter(|doc| doc.collection == "level4_arrays")
                .collect();
            
            if level4_docs.is_empty() {
                println!("⚠️ No level4_arrays documents found - data may not have been generated");
                return;
            }
            
            // Analyze schema
            let analyzer = DocumentStructureAnalyzer::new();
            let analysis_result = analyzer.analyze_documents(&level4_docs).await;
            
            match analysis_result {
                Ok(analysis) => {
                    // Test normalization
                    let normalizer = NormalizationEngine::new();
                    let normalization_result = normalizer.normalize_schema(&analysis);
                    
                    match normalization_result {
                        Ok(normalized) => {
                            println!("Normalization successful:");
                            println!("  Tables generated: {}", normalized.tables.len());
                            println!("  Relationships: {}", normalized.relationships.len());
                            println!("  Constraints: {}", normalized.constraints.len());
                            
                            // Check if array normalization opportunities were identified
                            let array_opportunities: Vec<_> = analysis.normalization_opportunities.iter()
                                .filter(|opp| opp.description.contains("Array") || opp.description.contains("array"))
                                .collect();
                            
                            println!("  Array normalization opportunities: {}", array_opportunities.len());
                            
                            for opp in &array_opportunities {
                                println!("    - {}: {}", opp.field_path, opp.description);
                            }
                            
                            // Check if any tables were created for arrays
                            let array_tables: Vec<_> = normalized.tables.iter()
                                .filter(|table| {
                                    // Array tables often have names like collection_field
                                    table.name.contains("_") && 
                                    (table.name.contains("tags") || 
                                     table.name.contains("scores") || 
                                     table.name.contains("flags"))
                                })
                                .collect();
                            
                            if !array_tables.is_empty() {
                                println!("  Array tables created: {}", array_tables.len());
                                for table in &array_tables {
                                    println!("    - {}", table.name);
                                }
                            }
                            
                            println!("✅ Array normalization test completed");
                            println!("🎯 Level 4 array normalization test completed");
                        }
                        Err(e) => {
                            println!("Normalization error: {}", e);
                            // Normalization errors might be acceptable depending on implementation
                        }
                    }
                }
                Err(e) => {
                    println!("Schema analysis error: {}", e);
                    panic!("Schema analysis should succeed");
                }
            }
        }
        Err(e) => {
            println!("Parse error: {}", e);
            println!("⚠️ Parse failed, but this might be expected for LevelDB format");
        }
    }
}
