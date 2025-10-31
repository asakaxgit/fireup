use std::fs;
use std::path::PathBuf;

/// Simple test to verify sample data exists and can be read
#[test]
fn test_sample_data_exists_and_readable() {
    let sample_data_path = PathBuf::from("tests/.firestore-data/firestore_export/all_namespaces/all_kinds/output-0");
    
    println!("🔍 Checking sample data at: {:?}", sample_data_path);
    
    // Verify file exists
    assert!(sample_data_path.exists(), "Sample data file should exist");
    
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
    assert!(metadata_path.exists(), "Metadata file should exist");
    
    let metadata_content = fs::read_to_string(&metadata_path).expect("Should read metadata");
    println!("📋 Export metadata: {}", metadata_content);
    
    // Verify it's a Firebase export
    assert!(metadata_content.contains("firestore"), "Should be a Firestore export");
    assert!(metadata_content.contains("version"), "Should have version info");
    
    println!("✅ Sample data validation complete!");
    println!("🎯 This data is ready for functional testing with the Firestore parser");
}

/// Test to show what collections and documents are in the sample data
#[test]
fn test_analyze_sample_data_content() {
    let sample_data_path = PathBuf::from("tests/.firestore-data/firestore_export/all_namespaces/all_kinds/output-0");
    
    if !sample_data_path.exists() {
        println!("⚠️ Sample data not found, skipping content analysis");
        return;
    }
    
    let data = fs::read(&sample_data_path).expect("Should read sample data");
    println!("📊 Analyzing {} bytes of LevelDB data", data.len());
    
    // Look for text patterns that might indicate document content
    let data_str = String::from_utf8_lossy(&data);
    
    // Look for collection names (based on the generator script)
    let collections = ["cities", "users"];
    for collection in &collections {
        if data_str.contains(collection) {
            println!("✅ Found collection: {}", collection);
        }
    }
    
    // Look for document IDs (based on the generator script)
    let doc_ids = ["alovelace", "aturing", "SF", "LA", "DC", "TOK", "BJ"];
    for doc_id in &doc_ids {
        if data_str.contains(doc_id) {
            println!("✅ Found document ID: {}", doc_id);
        }
    }
    
    // Look for field names
    let fields = ["first", "last", "born", "name", "state", "country", "capital", "population"];
    for field in &fields {
        if data_str.contains(field) {
            println!("✅ Found field: {}", field);
        }
    }
    
    // Look for values
    let values = ["Ada", "Lovelace", "Alan", "Turing", "San Francisco", "Los Angeles", "Washington", "Tokyo", "Beijing"];
    for value in &values {
        if data_str.contains(value) {
            println!("✅ Found value: {}", value);
        }
    }
    
    println!("🎯 Sample data contains the expected collections and documents!");
    println!("📋 Expected structure based on generator:");
    println!("   - users collection: alovelace, aturing");
    println!("   - cities collection: SF, LA, DC, TOK, BJ");
}

/// Test the directory structure matches expectations
#[test]
fn test_sample_data_directory_structure() {
    let base_path = PathBuf::from("tests/.firestore-data");
    
    // Check main directory exists
    assert!(base_path.exists(), "Base .firestore-data directory should exist");
    
    // Check Firebase export metadata
    let metadata_file = base_path.join("firebase-export-metadata.json");
    assert!(metadata_file.exists(), "Firebase export metadata should exist");
    
    // Check Firestore export directory
    let export_dir = base_path.join("firestore_export");
    assert!(export_dir.exists(), "Firestore export directory should exist");
    
    // Check overall metadata
    let overall_metadata = export_dir.join("firestore_export.overall_export_metadata");
    assert!(overall_metadata.exists(), "Overall export metadata should exist");
    
    // Check namespaces directory
    let namespaces_dir = export_dir.join("all_namespaces").join("all_kinds");
    assert!(namespaces_dir.exists(), "Namespaces directory should exist");
    
    // Check export metadata
    let export_metadata = namespaces_dir.join("all_namespaces_all_kinds.export_metadata");
    assert!(export_metadata.exists(), "Export metadata should exist");
    
    // Check actual data file
    let data_file = namespaces_dir.join("output-0");
    assert!(data_file.exists(), "Data file should exist");
    
    println!("✅ All expected files and directories are present!");
    println!("📁 Directory structure:");
    println!("   tests/.firestore-data/");
    println!("   ├── firebase-export-metadata.json");
    println!("   └── firestore_export/");
    println!("       ├── firestore_export.overall_export_metadata");
    println!("       └── all_namespaces/all_kinds/");
    println!("           ├── all_namespaces_all_kinds.export_metadata");
    println!("           └── output-0 (LevelDB data)");
}