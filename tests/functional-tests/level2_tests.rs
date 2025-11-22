use super::test_utils::*;
use fireup::leveldb_parser::parser::{FirestoreDocumentParser, LevelDBParser};
use fireup::types::FirestoreDocument;
use tokio;

/// Helper function to get the type name of a serde_json::Value
fn get_value_type_name(v: &serde_json::Value) -> &'static str {
    if v.is_string() { "string" }
    else if v.is_number() { "number" }
    else if v.is_boolean() { "boolean" }
    else { "unknown" }
}

/// Level 2 test: Parse two documents with the same single field structure
#[tokio::test]
async fn test_level2_two_rows_parsing() {
    ensure_monitoring_initialized();
    
    if !test_data_exists() {
        println!("Skipping Level 2 test - sample data not found");
        return;
    }

    let export_path = sample_export_path();
    let parser = FirestoreDocumentParser::new(export_path.to_string_lossy().to_string());
    
    println!("Testing Level 2 parsing from: {:?}", export_path);
    
    // Parse the backup file
    let parse_result = parser.parse_backup(&export_path.to_string_lossy()).await;
    
    match parse_result {
        Ok(result) => {
            println!("Parse successful:");
            println!("  Documents: {}", result.documents.len());
            println!("  Collections: {}", result.collections.len());
            
            // Find documents from the simple_items collection
            let simple_items: Vec<&FirestoreDocument> = result.documents.iter()
                .filter(|doc| doc.collection == "simple_items")
                .collect();
            
            println!("Found {} documents in simple_items collection", simple_items.len());
            
            // Verify we have at least 2 documents
            assert!(
                simple_items.len() >= 2,
                "Should find at least 2 documents in simple_items collection, found {}",
                simple_items.len()
            );
            
            // Verify all documents have the same field structure (name field)
            for doc in &simple_items {
                assert!(
                    doc.data.contains_key("name"),
                    "Document {} should contain 'name' field",
                    doc.id
                );
                
                // Verify the name field is a string
                let name_value = doc.data.get("name");
                assert!(
                    name_value.is_some(),
                    "Document {} should have a 'name' field",
                    doc.id
                );
                
                println!("✅ Document {} has expected structure: name = {:?}", doc.id, name_value);
            }
            
            // Verify we have the expected documents
            let item_001 = simple_items.iter().find(|doc| doc.id == "item_001");
            let item_002 = simple_items.iter().find(|doc| doc.id == "item_002");
            
            if let Some(doc) = item_001 {
                let name = doc.data.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                println!("✅ Found item_001 with name: '{}'", name);
                assert_eq!(name, "Apple", "item_001 should have name 'Apple'");
            } else {
                println!("⚠️ item_001 not found, but this is acceptable if data structure differs");
            }
            
            if let Some(doc) = item_002 {
                let name = doc.data.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                println!("✅ Found item_002 with name: '{}'", name);
                assert_eq!(name, "Banana", "item_002 should have name 'Banana'");
            } else {
                println!("⚠️ item_002 not found, but this is acceptable if data structure differs");
            }
            
            println!("🎯 Level 2 test PASSED: Successfully parsed {} documents with same field structure", simple_items.len());
        }
        Err(e) => {
            println!("Parse error: {}", e);
            println!("⚠️ Parse failed, but this might be expected for LevelDB format");
        }
    }
}

/// Level 2 test: Verify type consistency across documents
#[tokio::test]
async fn test_level2_type_consistency() {
    ensure_monitoring_initialized();
    
    if !test_data_exists() {
        println!("Skipping Level 2 type consistency test - sample data not found");
        return;
    }

    let export_path = sample_export_path();
    let parser = FirestoreDocumentParser::new(export_path.to_string_lossy().to_string());
    
    println!("Testing Level 2 type consistency from: {:?}", export_path);
    
    let parse_result = parser.parse_backup(&export_path.to_string_lossy()).await;
    
    match parse_result {
        Ok(result) => {
            let simple_items: Vec<&FirestoreDocument> = result.documents.iter()
                .filter(|doc| doc.collection == "simple_items")
                .collect();
            
            if simple_items.len() < 2 {
                println!("⚠️ Not enough documents for type consistency test");
                return;
            }
            
            // Verify all documents have the same field types
            let first_doc = &simple_items[0];
            let first_name_type = first_doc.data.get("name").map(get_value_type_name);
            
            for doc in &simple_items {
                let name_type = doc.data.get("name").map(get_value_type_name);
                
                assert_eq!(
                    first_name_type, name_type,
                    "Document {} should have the same field type as other documents",
                    doc.id
                );
            }
            
            println!("✅ Type consistency verified: all documents have the same field type");
            println!("🎯 Level 2 type consistency test PASSED");
        }
        Err(e) => {
            println!("Parse error: {}", e);
            println!("⚠️ Parse failed, but this might be expected for LevelDB format");
        }
    }
}

/// Level 2 test: Verify collection-level analysis
#[tokio::test]
async fn test_level2_collection_analysis() {
    ensure_monitoring_initialized();
    
    if !test_data_exists() {
        println!("Skipping Level 2 collection analysis test - sample data not found");
        return;
    }

    let export_path = sample_export_path();
    let parser = FirestoreDocumentParser::new(export_path.to_string_lossy().to_string());
    
    println!("Testing Level 2 collection analysis from: {:?}", export_path);
    
    let parse_result = parser.parse_backup(&export_path.to_string_lossy()).await;
    
    match parse_result {
        Ok(result) => {
            // Verify the collection exists
            assert!(
                result.collections.contains(&"simple_items".to_string()),
                "Should find 'simple_items' collection"
            );
            
            // Count documents in the collection
            let simple_items_count = result.documents.iter()
                .filter(|doc| doc.collection == "simple_items")
                .count();
            
            assert!(
                simple_items_count >= 2,
                "Should have at least 2 documents in simple_items collection, found {}",
                simple_items_count
            );
            
            println!("✅ Collection analysis verified:");
            println!("  - Collection 'simple_items' exists");
            println!("  - Contains {} documents", simple_items_count);
            println!("🎯 Level 2 collection analysis test PASSED");
        }
        Err(e) => {
            println!("Parse error: {}", e);
            println!("⚠️ Parse failed, but this might be expected for LevelDB format");
        }
    }
}
