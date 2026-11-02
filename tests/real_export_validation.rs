//! Validation of fireup against realistic Firestore/Datastore managed-export fixtures.
//!
//! The fixtures under `tests/fixtures/` are produced by
//! `tests/fixtures/generate_datastore_export.py` and mimic the real managed-export
//! layout and binary format (32 KiB LevelDB log blocks, masked CRC32C record
//! checksums). See `tests/fixtures/README.md`.
//!
//! Two groups of tests live here:
//!
//! * **Fixture guards** (always run) — an independent masked-CRC32C log reader
//!   proves the fixtures are spec-correct, so a fireup failure against them is a
//!   fireup finding rather than a fixture artifact.
//! * **Pending acceptance tests** (`#[ignore]`) — these assert the behaviour
//!   fireup *should* have. They currently fail; each carries the reason and the
//!   gap it tracks. Remove the `#[ignore]` as each gap is fixed.

use std::path::{Path, PathBuf};

use fireup::leveldb_parser::parser::FirestoreDocumentParser;
use fireup::monitoring::{initialize_monitoring, MonitoringConfig};

/// `parse_documents()` calls `get_monitoring_system()`, which panics unless the
/// process has already initialised the global monitoring system. Every test that
/// touches the parser has to do this first.
fn init_monitoring() {
    initialize_monitoring(MonitoringConfig::default());
}

const BLOCK_SIZE: usize = 32768;
const HEADER_SIZE: usize = 7;

const EXPECTED_USERS: usize = 5;
const EXPECTED_ORDERS: usize = 7;
const EXPECTED_TOTAL: usize = EXPECTED_USERS + EXPECTED_ORDERS;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn kind_output(tree: &str, kind: &str) -> PathBuf {
    fixtures_dir()
        .join(tree)
        .join("all_namespaces")
        .join(format!("kind_{kind}"))
        .join("output-0")
}

// ---------------------------------------------------------------------------
// Reference LevelDB log reader (spec-correct, independent of src/)
// ---------------------------------------------------------------------------

/// CRC32C (Castagnoli), the polynomial LevelDB actually uses.
fn crc32c(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x82F6_3B78
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// LevelDB stores masked CRCs so a stored checksum is never a checksum of real data.
fn mask_crc(crc: u32) -> u32 {
    ((crc >> 15) | (crc << 17)).wrapping_add(0xA282_EAD8)
}

/// Unframe a LevelDB log file, verifying every record checksum.
fn read_log_records(path: &Path) -> Vec<Vec<u8>> {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut records = Vec::new();
    let mut pending: Vec<u8> = Vec::new();

    for block in raw.chunks(BLOCK_SIZE) {
        let mut offset = 0usize;
        while offset + HEADER_SIZE <= block.len() {
            let header = &block[offset..offset + HEADER_SIZE];
            let checksum = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
            let length = u16::from_le_bytes([header[4], header[5]]) as usize;
            let record_type = header[6];
            if checksum == 0 && length == 0 && record_type == 0 {
                break; // trailing padding
            }

            let start = offset + HEADER_SIZE;
            let body = &block[start..start + length];

            let mut hashed = Vec::with_capacity(length + 1);
            hashed.push(record_type);
            hashed.extend_from_slice(body);
            assert_eq!(
                mask_crc(crc32c(&hashed)),
                checksum,
                "{}: masked CRC32C mismatch at record offset {offset}",
                path.display()
            );

            match record_type {
                1 => {
                    assert!(pending.is_empty(), "FULL record inside an open fragment");
                    records.push(body.to_vec());
                }
                2 => {
                    assert!(pending.is_empty(), "FIRST record inside an open fragment");
                    pending = body.to_vec();
                }
                3 | 4 => {
                    assert!(!pending.is_empty(), "fragment without a preceding FIRST");
                    pending.extend_from_slice(body);
                    if record_type == 4 {
                        records.push(std::mem::take(&mut pending));
                    }
                }
                other => panic!("invalid LevelDB record type {other}"),
            }

            offset = start + length;
        }
    }

    assert!(pending.is_empty(), "{}: ends mid-fragment", path.display());
    records
}

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
// Pending acceptance tests — assert the behaviour fireup should have
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "gap: LevelDBReader::calculate_crc32 uses unmasked IEEE CRC-32 instead of \
            LevelDB's masked CRC32C, so every record in a real export is rejected"]
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
#[ignore = "gap: real exports carry legacy storage_onestore_v3.EntityProto payloads \
            (proto2 groups), which protobuf_entity.rs cannot decode"]
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
#[ignore = "gap: LevelDBReader::resolve_backup_path collapses an export tree to a single \
            output-0 file, so all kinds but one are silently dropped"]
async fn parser_reads_every_kind_in_an_export_tree() {
    init_monitoring();
    let root = fixtures_dir().join("export-onestore");
    init_monitoring();
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
#[ignore = "gap: document ids are dropped — the generated primary key is a random UUID \
            with no column holding the Datastore key name/id"]
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
