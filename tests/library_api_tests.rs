//! Library-API guarantees for consumers that are not the `fireup` CLI.
//!
//! This file deliberately holds a single test and never calls
//! `initialize_monitoring`: the global monitoring system is process-wide, so any
//! other test in the same binary would initialise it and mask the regression
//! being guarded here.

use std::io::Write;

use fireup::leveldb_parser::parser::FirestoreDocumentParser;

/// `parse_documents()` used to panic with "Monitoring system not initialized"
/// for every consumer that had not called `initialize_monitoring()` first. The
/// CLI only avoided it because `main` initialises before parsing.
#[tokio::test]
async fn parse_documents_does_not_panic_without_monitoring_initialised() {
    let mut backup = tempfile::NamedTempFile::new().expect("create backup file");
    writeln!(
        backup,
        r#"{{"name":"projects/p/databases/(default)/documents/users/doc0","fields":{{"email":{{"stringValue":"a@example.com"}}}}}}"#
    )
    .expect("write document");
    backup.flush().expect("flush backup file");

    let parser = FirestoreDocumentParser::new(backup.path().to_string_lossy().to_string());
    let result = parser
        .parse_documents()
        .await
        .expect("parse_documents must work without initialize_monitoring");

    assert_eq!(result.documents.len(), 1);
    assert_eq!(result.collections, vec!["users".to_string()]);
}
