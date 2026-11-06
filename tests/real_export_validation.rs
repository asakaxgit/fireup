//! Validation of fireup against realistic Firestore/Datastore managed-export fixtures.
//!
//! The fixtures under `tests/fixtures/` are produced by
//! `tests/fixtures/generate_datastore_export.py` and mimic the real managed-export
//! layout and binary format (32 KiB LevelDB log blocks, masked CRC32C record
//! checksums). See `tests/fixtures/README.md`.
//!
//! Two groups of tests live here:
//!
//! * **Fixture guards** — an independent masked-CRC32C log reader
//!   (`tests/support/export_log_reader.rs`) proves the fixtures are
//!   spec-correct, so a fireup failure against them is a fireup finding rather
//!   than a fixture artifact.
//! * **Acceptance tests** — the same fixtures read back through fireup itself.
//!
//! Edge cases beyond this happy-path dataset — output shards, namespaces, key
//! identity, numeric limits, awkward property names — live in
//! `tests/export_fidelity_tests.rs` against the `export-edge-*` trees.

use fireup::leveldb_parser::parser::FirestoreDocumentParser;
use fireup::monitoring::{initialize_monitoring, MonitoringConfig};

#[path = "support/export_log_reader.rs"]
mod export_log_reader;

use export_log_reader::{fixtures_dir, kind_output, read_log_records, BLOCK_SIZE};

/// `parse_documents()` calls `get_monitoring_system()`, which panics unless the
/// process has already initialised the global monitoring system. Every test that
/// touches the parser has to do this first.
fn init_monitoring() {
    initialize_monitoring(MonitoringConfig::default());
}

const EXPECTED_USERS: usize = 5;
const EXPECTED_ORDERS: usize = 7;
const EXPECTED_TOTAL: usize = EXPECTED_USERS + EXPECTED_ORDERS;

// ---------------------------------------------------------------------------
// Fixture guards — must always pass
// ---------------------------------------------------------------------------

#[test]
fn managed_export_layout_is_present() {
    for tree in ["export-onestore", "export-v1-entity"] {
        let root = fixtures_dir().join(tree);
        assert!(root.is_dir(), "missing fixture tree {}", root.display());

        let overall = std::fs::read_dir(&root)
            .expect("read fixture root")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".overall_export_metadata")
            })
            .count();
        assert_eq!(overall, 1, "{tree}: expected one .overall_export_metadata");

        for kind in ["Users", "Orders"] {
            let kind_dir = root.join("all_namespaces").join(format!("kind_{kind}"));
            assert!(
                kind_dir
                    .join(format!("all_namespaces_kind_{kind}.export_metadata"))
                    .is_file(),
                "{tree}/{kind}: missing per-kind .export_metadata"
            );
            assert!(
                kind_dir.join("output-0").is_file(),
                "{tree}/{kind}: missing output-0"
            );
        }
    }
}

#[test]
fn fixture_framing_matches_leveldb_log_spec() {
    for tree in ["export-onestore", "export-v1-entity"] {
        let users = read_log_records(&kind_output(tree, "Users"));
        let orders = read_log_records(&kind_output(tree, "Orders"));
        assert_eq!(users.len(), EXPECTED_USERS, "{tree}: Users record count");
        assert_eq!(orders.len(), EXPECTED_ORDERS, "{tree}: Orders record count");
        assert!(
            users.iter().chain(orders.iter()).all(|r| !r.is_empty()),
            "{tree}: empty record payload"
        );
    }
}

#[test]
fn users_fixture_exercises_cross_block_fragmentation() {
    // One user carries a ~45 KB blob so its record must be split across 32 KiB
    // blocks as FIRST/LAST. Without this the fragment-reassembly path is untested.
    let path = kind_output("export-onestore", "Users");
    assert!(
        std::fs::metadata(&path).expect("stat Users output-0").len() > BLOCK_SIZE as u64,
        "Users output-0 must exceed one block to exercise fragmentation"
    );
    let records = read_log_records(&path);
    assert!(
        records.iter().any(|r| r.len() > BLOCK_SIZE),
        "expected at least one record larger than a single block"
    );
}

// ---------------------------------------------------------------------------
// Acceptance tests — the fixtures read back through fireup itself
// ---------------------------------------------------------------------------

#[tokio::test]
async fn parser_reads_records_from_a_spec_correct_export() {
    init_monitoring();
    let parser = FirestoreDocumentParser::new(
        kind_output("export-v1-entity", "Orders").to_string_lossy().to_string(),
    );
    let result = parser.parse_documents().await.expect("parse Orders output-0");
    assert_eq!(
        result.metadata.records_processed, EXPECTED_ORDERS,
        "parser must read every framed record"
    );
}

#[tokio::test]
async fn parser_decodes_legacy_entity_proto_payloads() {
    init_monitoring();
    let parser = FirestoreDocumentParser::new(
        kind_output("export-onestore", "Orders").to_string_lossy().to_string(),
    );
    let result = parser.parse_documents().await.expect("parse Orders output-0");
    assert_eq!(result.documents.len(), EXPECTED_ORDERS);
    assert_eq!(result.collections, vec!["Orders".to_string()]);
}

#[tokio::test]
async fn parser_reads_every_kind_in_an_export_tree() {
    init_monitoring();
    let root = fixtures_dir().join("export-onestore");
    let parser = FirestoreDocumentParser::new(root.to_string_lossy().to_string());
    let result = parser.parse_documents().await.expect("parse export tree");
    assert_eq!(
        result.documents.len(),
        EXPECTED_TOTAL,
        "every kind under all_namespaces/ must be parsed"
    );
    let mut collections = result.collections.clone();
    collections.sort();
    assert_eq!(collections, vec!["Orders".to_string(), "Users".to_string()]);
}

#[tokio::test]
async fn parsed_documents_retain_their_datastore_key() {
    init_monitoring();
    let parser = FirestoreDocumentParser::new(
        kind_output("export-onestore", "Users").to_string_lossy().to_string(),
    );
    let result = parser.parse_documents().await.expect("parse Users output-0");
    let mut ids: Vec<&str> = result.documents.iter().map(|d| d.id.as_str()).collect();
    ids.sort();
    assert!(
        ids.contains(&"ada") && ids.contains(&"unicode-ユーザー"),
        "string key names must survive parsing, got {ids:?}"
    );
}
