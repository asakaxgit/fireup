use std::path::PathBuf;
use std::collections::HashMap;
use serde_json::json;
use fireup::types::{FirestoreDocument, DocumentMetadata};
use fireup::leveldb_parser::parser::FirestoreDocumentParser;

/// Test data directory path
pub fn test_data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(".firestore-data")
}

/// Get path to the sample Firestore export
pub fn sample_export_path() -> PathBuf {
    test_data_dir()
        .join("firestore_export")
        .join("all_namespaces")
        .join("all_kinds")
        .join("output-0")
}

/// Get test data path as a String for APIs expecting &str
pub fn get_test_data_path() -> String {
    sample_export_path().to_string_lossy().to_string()
}

/// Ensure global monitoring is initialized for tests
pub fn ensure_monitoring_initialized() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        fireup::monitoring::initialize_monitoring(fireup::monitoring::MonitoringConfig::default());
    });
}

/// Create a test Firestore document
pub fn create_test_document(id: &str, collection: &str) -> FirestoreDocument {
    let mut data = HashMap::new();
    data.insert("name".to_string(), json!("Test Document"));
    data.insert("value".to_string(), json!(42));
    data.insert("active".to_string(), json!(true));
    
    FirestoreDocument {
        id: id.to_string(),
        collection: collection.to_string(),
        data,
        subcollections: vec![],
        metadata: DocumentMetadata {
            created_at: Some(chrono::Utc::now()),
            updated_at: Some(chrono::Utc::now()),
            path: format!("projects/test/databases/(default)/documents/{}/{}", collection, id),
            size_bytes: Some(256),
        },
    }
}

/// Create a parser instance for testing
pub fn create_test_parser() -> FirestoreDocumentParser {
    let export_path = sample_export_path();
    FirestoreDocumentParser::new(export_path.to_string_lossy().to_string())
}

/// Check if test data exists
pub fn test_data_exists() -> bool {
    sample_export_path().exists()
}

/// Create sample JSON document data for testing
pub fn sample_json_document() -> serde_json::Value {
    json!({
        "name": "projects/test-project/databases/(default)/documents/users/user123",
        "fields": {
            "email": {
                "stringValue": "test@example.com"
            },
            "age": {
                "integerValue": "25"
            },
            "active": {
                "booleanValue": true
            },
            "profile": {
                "mapValue": {
                    "fields": {
                        "firstName": {
                            "stringValue": "John"
                        },
                        "lastName": {
                            "stringValue": "Doe"
                        }
                    }
                }
            },
            "tags": {
                "arrayValue": {
                    "values": [
                        {"stringValue": "developer"},
                        {"stringValue": "rust"}
                    ]
                }
            }
        },
        "createTime": "2023-01-01T00:00:00Z",
        "updateTime": "2023-01-02T00:00:00Z"
    })
}

/// Create sample complex nested document
pub fn sample_complex_document() -> serde_json::Value {
    json!({
        "name": "projects/test-project/databases/(default)/documents/orders/order456",
        "fields": {
            "orderId": {
                "stringValue": "ORD-2023-001"
            },
            "customer": {
                "mapValue": {
                    "fields": {
                        "id": {"stringValue": "cust123"},
                        "name": {"stringValue": "Jane Smith"},
                        "email": {"stringValue": "jane@example.com"}
                    }
                }
            },
            "items": {
                "arrayValue": {
                    "values": [
                        {
                            "mapValue": {
                                "fields": {
                                    "productId": {"stringValue": "prod001"},
                                    "name": {"stringValue": "Widget A"},
                                    "price": {"doubleValue": "19.99"},
                                    "quantity": {"integerValue": "2"}
                                }
                            }
                        },
                        {
                            "mapValue": {
                                "fields": {
                                    "productId": {"stringValue": "prod002"},
                                    "name": {"stringValue": "Widget B"},
                                    "price": {"doubleValue": "29.99"},
                                    "quantity": {"integerValue": "1"}
                                }
                            }
                        }
                    ]
                }
            },
            "total": {
                "doubleValue": "69.97"
            },
            "status": {
                "stringValue": "pending"
            },
            "metadata": {
                "mapValue": {
                    "fields": {
                        "source": {"stringValue": "web"},
                        "campaign": {"stringValue": "summer2023"}
                    }
                }
            }
        },
        "createTime": "2023-06-15T10:30:00Z",
        "updateTime": "2023-06-15T10:30:00Z"
    })
}

/// Assert that a document has expected basic structure
pub fn assert_document_structure(doc: &FirestoreDocument) {
    assert!(!doc.id.is_empty(), "Document ID should not be empty");
    assert!(!doc.collection.is_empty(), "Collection name should not be empty");
    assert!(!doc.metadata.path.is_empty(), "Document path should not be empty");
}

/// Assert that document data contains expected fields
pub fn assert_document_data(doc: &FirestoreDocument, expected_fields: &[&str]) {
    for field in expected_fields {
        assert!(
            doc.data.contains_key(*field),
            "Document should contain field: {}",
            field
        );
    }
}

/// Extract number value from a JSON value and convert to string
pub fn extract_number_value(v: &serde_json::Value) -> Option<String> {
    if v.is_i64() {
        Some(v.as_i64().unwrap().to_string())
    } else if v.is_u64() {
        Some(v.as_u64().unwrap().to_string())
    } else if v.is_f64() {
        Some(v.as_f64().unwrap().to_string())
    } else if v.is_number() {
        Some(v.as_number().unwrap().to_string())
    } else {
        None
    }
}