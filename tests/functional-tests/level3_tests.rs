use super::test_utils::*;
use fireup::leveldb_parser::parser::{FirestoreDocumentParser, LevelDBParser};
use fireup::types::FirestoreDocument;
use fireup::schema_analyzer::{DocumentStructureAnalyzer, NormalizationEngine};
use tokio;

// The `users` collection in tests/.firestore-data is written by
// tests/data-generator/src/main.ts: `alovelace` as {first, last, born} and
// `aturing` as {first, middle, last, born}. So `first`/`last` are strings on
// every document, `born` is a number on every document, and `middle` is a
// sparse field present on exactly one of the two. There is no `name` or `age`
// field on `users` — `name` only exists on `cities`.
const ALWAYS_PRESENT_STRING_FIELDS: [&str; 2] = ["first", "last"];
const ALWAYS_PRESENT_NUMBER_FIELD: &str = "born";
const SPARSE_FIELD: &str = "middle";

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
    let result = parser.parse_backup(&export_path.to_string_lossy()).await
        .expect("parsing the checked-in sample export should succeed");

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

    // Verify each document carries the primitive fields the generator writes
    let mut sparse_field_count = 0;
    for doc in &users_docs {
        println!("Document {}: {:?}", doc.id, doc.data);

        for field in ALWAYS_PRESENT_STRING_FIELDS {
            let is_non_empty_string = doc.data.get(field)
                .map(|v| v.is_string() && !v.as_str().unwrap_or("").is_empty())
                .unwrap_or(false);
            assert!(is_non_empty_string,
                "Document {} should have a non-empty string `{}` field, got {:?}",
                doc.id, field, doc.data.get(field));
        }

        let is_number = doc.data.get(ALWAYS_PRESENT_NUMBER_FIELD)
            .map(|v| v.is_number())
            .unwrap_or(false);
        assert!(is_number,
            "Document {} should have a numeric `{}` field, got {:?}",
            doc.id, ALWAYS_PRESENT_NUMBER_FIELD, doc.data.get(ALWAYS_PRESENT_NUMBER_FIELD));

        if doc.data.contains_key(SPARSE_FIELD) {
            sparse_field_count += 1;
        }

        println!("✅ Document {} has the expected string and numeric fields", doc.id);
    }

    // `middle` is on `aturing` only, so the parser must not invent it elsewhere
    // and must not drop it where it exists.
    assert_eq!(sparse_field_count, 1,
        "`{}` should be present on exactly one users document", SPARSE_FIELD);

    println!("🎯 Level 3 parsing test PASSED: Found {} documents with multiple primitive fields", users_docs.len());
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
    let result = parser.parse_backup(&export_path.to_string_lossy()).await
        .expect("parsing the checked-in sample export should succeed");
    assert!(!result.documents.is_empty(), "Sample export should yield documents");

    // Analyze schema
    let analyzer = DocumentStructureAnalyzer::new();
    let analysis = analyzer.analyze_documents(&result.documents).await
        .expect("schema analysis should succeed");

    println!("Schema analysis successful:");
    println!("  Collections analyzed: {}", analysis.collections.len());
    println!("  Field types identified: {}", analysis.field_types.len());

    // Find the users collection
    let collection = analysis.collections.iter()
        .find(|c| c.name == "users")
        .expect("Users collection should appear in the analysis");

    println!("Users collection analysis:");
    println!("  Document count: {}", collection.document_count);
    println!("  Field names: {:?}", collection.field_names);

    // Should have at least 2 documents
    assert!(collection.document_count >= 2,
        "Users collection should have at least 2 documents");

    // Should have every field the generator writes, including the sparse one
    for field in ALWAYS_PRESENT_STRING_FIELDS.iter()
        .chain([ALWAYS_PRESENT_NUMBER_FIELD, SPARSE_FIELD].iter())
    {
        assert!(collection.field_names.contains(&field.to_string()),
            "Users collection should have '{}' field, got {:?}", field, collection.field_names);
    }

    println!("✅ Users collection has correct structure");

    // Check field type analysis. Field paths are `<collection>.<field>`, so
    // scope the lookup to `users` — `cities` has its own field set.
    let users_field = |field: &str| {
        let path = format!("users.{}", field);
        analysis.field_types.iter()
            .find(|ft| ft.field_path == path)
            .unwrap_or_else(|| panic!("Analysis should include a field type for `{}`", path))
    };

    for field in ALWAYS_PRESENT_STRING_FIELDS.iter().chain([ALWAYS_PRESENT_NUMBER_FIELD].iter()) {
        let ft = users_field(field);
        println!("users.{} analysis: type {:?}, presence {:.1}%", field, ft.recommended_type, ft.presence_percentage);
        assert_eq!(ft.presence_percentage, 100.0,
            "`users.{}` is on every document, so presence should be 100%", field);
    }

    let sparse_ft = users_field(SPARSE_FIELD);
    println!("users.{} analysis: type {:?}, presence {:.1}%", SPARSE_FIELD, sparse_ft.recommended_type, sparse_ft.presence_percentage);
    assert!(sparse_ft.presence_percentage > 0.0 && sparse_ft.presence_percentage < 100.0,
        "`users.{}` is on 1 of 2 documents, so presence should be partial, got {:.1}%",
        SPARSE_FIELD, sparse_ft.presence_percentage);

    println!("🎯 Level 3 schema analysis test PASSED");
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
    let result = parser.parse_backup(&export_path.to_string_lossy()).await
        .expect("parsing the checked-in sample export should succeed");
    assert!(!result.documents.is_empty(), "Sample export should yield documents");

    // Analyze schema
    let analyzer = DocumentStructureAnalyzer::new();
    let analysis = analyzer.analyze_documents(&result.documents).await
        .expect("schema analysis should succeed");

    // Normalize schema
    let normalizer = NormalizationEngine::new();
    let normalized = normalizer.normalize_schema(&analysis)
        .expect("normalization should succeed");

    println!("Normalization successful:");
    println!("  Tables generated: {}", normalized.tables.len());

    // Find the users table
    let table = normalized.tables.iter()
        .find(|t| t.name == "users")
        .expect("Normalization should produce a `users` table");

    println!("Users table structure:");
    println!("  Table name: {}", table.name);
    println!("  Columns: {}", table.columns.len());
    for column in &table.columns {
        println!("  Column {}: {:?} (nullable: {})", column.name, column.column_type, column.nullable);
    }

    // Every field the generator writes should become a column
    for field in ALWAYS_PRESENT_STRING_FIELDS.iter()
        .chain([ALWAYS_PRESENT_NUMBER_FIELD, SPARSE_FIELD].iter())
    {
        assert!(table.columns.iter().any(|c| c.name == *field),
            "Users table should have '{}' column", field);
    }

    // A field missing from one of the two documents must be nullable, or the
    // import would reject the row that omits it.
    let sparse_column = table.columns.iter()
        .find(|c| c.name == SPARSE_FIELD)
        .expect("checked above");
    assert!(sparse_column.nullable,
        "`{}` is absent from one users document, so its column must be nullable", SPARSE_FIELD);

    println!("✅ Users table has correct structure");
    println!("🎯 Level 3 normalization test PASSED");
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
    let result = parser.parse_backup(&export_path.to_string_lossy()).await
        .expect("parsing the checked-in sample export should succeed");

    // Find documents in the users collection
    let users_docs: Vec<&FirestoreDocument> = result.documents.iter()
        .filter(|doc| doc.collection == "users")
        .collect();

    assert!(users_docs.len() >= 2,
        "Need at least 2 documents for the type consistency test, found {}", users_docs.len());

    println!("Checking type consistency across {} documents", users_docs.len());

    // The same field must deserialize to the same JSON type on every document,
    // otherwise the generated column type would be a lie.
    for field in ALWAYS_PRESENT_STRING_FIELDS {
        let string_count = users_docs.iter()
            .filter(|doc| doc.data.get(field).map(|v| v.is_string()).unwrap_or(false))
            .count();
        println!("`{}` is a string on {}/{} documents", field, string_count, users_docs.len());
        assert_eq!(string_count, users_docs.len(),
            "`{}` should be a string on every users document", field);
    }

    let number_count = users_docs.iter()
        .filter(|doc| doc.data.get(ALWAYS_PRESENT_NUMBER_FIELD).map(|v| v.is_number()).unwrap_or(false))
        .count();
    println!("`{}` is a number on {}/{} documents", ALWAYS_PRESENT_NUMBER_FIELD, number_count, users_docs.len());
    assert_eq!(number_count, users_docs.len(),
        "`{}` should be a number on every users document", ALWAYS_PRESENT_NUMBER_FIELD);

    // Where the sparse field exists it must still be a string.
    for doc in &users_docs {
        if let Some(value) = doc.data.get(SPARSE_FIELD) {
            assert!(value.is_string(),
                "`{}` on document {} should be a string, got {:?}", SPARSE_FIELD, doc.id, value);
        }
    }

    println!("✅ Type consistency verified across all documents");
    println!("🎯 Level 3 type consistency test PASSED");
}
