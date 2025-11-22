// Functional tests for the Firestore parser
pub mod parser_tests;
pub mod integration_tests;
pub mod validation_tests;
pub mod tiny_tests;
pub mod level4_tests;

// Test utilities and helpers
pub mod test_utils;

#[cfg(test)]
mod tests {
    use super::test_utils::*;
    use std::fs;
    use std::path::PathBuf;
    
    #[tokio::test]
    async fn test_functional_suite_setup() {
        println!("Running functional test suite setup");
        
        if test_data_exists() {
            println!("✓ Test data found at: {:?}", sample_export_path());
        } else {
            println!("⚠ Test data not found - some tests will be skipped");
            println!("  Expected location: {:?}", sample_export_path());
        }
        
        // Test parser creation
        let parser = create_test_parser();
        println!("✓ Parser created successfully");
        
        // Test document creation
        let test_doc = create_test_document("test1", "test_collection");
        assert_document_structure(&test_doc);
        println!("✓ Test document creation works");
        
        println!("Functional test suite setup complete");
    }
    
    #[test]
    fn test_sample_data_validation() {
        let sample_data_path = PathBuf::from("tests/.firestore-data/firestore_export/all_namespaces/all_kinds/output-0");
        
        println!("🔍 Checking sample data at: {:?}", sample_data_path);
        
        if !sample_data_path.exists() {
            println!("⚠️ Sample data not found - this is expected if data hasn't been generated yet");
            return;
        }
        
        // Verify file is readable
        let metadata = fs::metadata(&sample_data_path).expect("Should be able to read file metadata");
        println!("📊 File size: {} bytes", metadata.len());
        assert!(metadata.len() > 0, "File should not be empty");
        
        // Try to read the file
        let data = fs::read(&sample_data_path).expect("Should be able to read file");
        println!("✅ Successfully read {} bytes of sample data", data.len());
        
        // Check if it looks like LevelDB data (binary format)
        assert!(data.len() > 10, "Should have substantial data");
        
        // LevelDB files typically have specific patterns
        println!("🔍 First 16 bytes (hex): {:02x?}", &data[..16.min(data.len())]);
        
        // Check metadata file
        let metadata_path = PathBuf::from("tests/.firestore-data/firebase-export-metadata.json");
        if metadata_path.exists() {
            let metadata_content = fs::read_to_string(&metadata_path).expect("Should read metadata");
            println!("📋 Export metadata found");
            
            // Verify it's a Firebase export
            assert!(metadata_content.contains("firestore"), "Should be a Firestore export");
            assert!(metadata_content.contains("version"), "Should have version info");
        }
        
        // Look for expected content based on the data generator
        let data_str = String::from_utf8_lossy(&data);
        
        // Look for collection names
        let collections = ["cities", "users"];
        for collection in &collections {
            if data_str.contains(collection) {
                println!("✅ Found collection: {}", collection);
            }
        }
        
        // Look for document IDs
        let doc_ids = ["alovelace", "aturing", "SF", "LA", "DC", "TOK", "BJ"];
        for doc_id in &doc_ids {
            if data_str.contains(doc_id) {
                println!("✅ Found document ID: {}", doc_id);
            }
        }
        
        println!("✅ Sample data validation complete!");
        println!("🎯 This data is ready for functional testing with the Firestore parser");
    }
}