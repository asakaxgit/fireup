//! Exit-code contract for the `fireup` CLI.
//!
//! A migration tool that exits 0 after losing every document is worse than one
//! that crashes: the output is indistinguishable from a completed run. These
//! tests need no database.

use std::io::Write;
use std::process::Command;

/// Non-empty input that the parser cannot turn into a single document.
fn write_unparseable_backup() -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("create backup file");
    file.write_all(&b"x".repeat(4096)).expect("write bytes");
    file.flush().expect("flush backup file");
    file
}

fn write_document_backup(collection: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("create backup file");
    writeln!(
        file,
        r#"{{"name":"projects/p/databases/(default)/documents/{collection}/doc0","fields":{{"email":{{"stringValue":"a@example.com"}}}}}}"#
    )
    .expect("write document");
    file.flush().expect("flush backup file");
    file
}

#[test]
fn analyze_fails_when_a_non_empty_source_yields_no_documents() {
    let backup = write_unparseable_backup();
    let output_path = tempfile::NamedTempFile::new().expect("create output file");

    let output = Command::new(env!("CARGO_BIN_EXE_fireup"))
        .args(["analyze", "-b"])
        .arg(backup.path())
        .arg("-o")
        .arg(output_path.path())
        .output()
        .expect("run fireup analyze");

    assert!(
        !output.status.success(),
        "total parse loss must exit non-zero, got {:?}",
        output.status.code()
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Parsed 0 documents"),
        "stderr should explain the parse loss, got: {stderr}"
    );
}

#[test]
fn import_fails_when_a_non_empty_source_yields_no_documents() {
    let backup = write_unparseable_backup();

    // The parse guard runs before any connection attempt, so this needs no
    // database: a reachable URL would not change the outcome.
    let output = Command::new(env!("CARGO_BIN_EXE_fireup"))
        .args(["import", "-b"])
        .arg(backup.path())
        .args(["-p", "postgresql://user:pw@127.0.0.1:1/absent"])
        .output()
        .expect("run fireup import");

    assert!(!output.status.success(), "total parse loss must exit non-zero");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Parsed 0 documents"),
        "stderr should explain the parse loss, got: {stderr}"
    );
}

#[test]
fn import_rejects_a_malformed_connection_url() {
    let backup = write_document_backup("cli_bad_url");

    let output = Command::new(env!("CARGO_BIN_EXE_fireup"))
        .args(["import", "-b"])
        .arg(backup.path())
        .args(["-p", "mysql://user:pw@localhost:3306/db"])
        .output()
        .expect("run fireup import");

    assert!(!output.status.success(), "a non-PostgreSQL URL must be rejected");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Invalid PostgreSQL URL format"),
        "stderr should name the URL problem, got: {stderr}"
    );
}

/// A URL with no database used to be accepted and silently replaced with a
/// hardcoded target.
#[test]
fn import_rejects_a_url_without_a_database() {
    let backup = write_document_backup("cli_no_db");

    let output = Command::new(env!("CARGO_BIN_EXE_fireup"))
        .args(["import", "-b"])
        .arg(backup.path())
        .args(["-p", "postgresql://user:pw@localhost:5432"])
        .output()
        .expect("run fireup import");

    assert!(!output.status.success(), "a URL without a database must be rejected");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not specify a database"),
        "stderr should name the missing database, got: {stderr}"
    );
}

#[test]
fn validate_check_quality_accepts_the_managed_export_fixture() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/export-onestore");

    let output = Command::new(env!("CARGO_BIN_EXE_fireup"))
        .args(["validate", "-b"])
        .arg(&fixture)
        .arg("--check-quality")
        .output()
        .expect("run fireup validate");

    assert!(
        output.status.success(),
        "managed export must validate: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Residual risk 9: framed LevelDB records that decode to zero documents must
/// not exit 0 under `validate --check-quality` (and must not claim integrity 1.0).
#[test]
fn validate_check_quality_fails_when_records_decode_to_zero_documents() {
    let backup = write_undecodable_leveldb_backup();

    let output = Command::new(env!("CARGO_BIN_EXE_fireup"))
        .args(["validate", "-b"])
        .arg(backup.path())
        .args(["--check-quality", "--detailed", "--format", "json"])
        .output()
        .expect("run fireup validate");

    assert!(
        !output.status.success(),
        "0 decoded documents from valid records must exit non-zero, got {:?}",
        output.status.code()
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");
    assert!(
        combined.contains("Decoded 0 Firestore documents")
            || combined.contains("validation failed")
            || combined.contains("\"is_valid\": false"),
        "output should explain total document loss, got: {combined}"
    );
    assert!(
        !combined.contains("\"overall_integrity_score\": 1.0")
            && !combined.contains("\"overall_integrity_score\":1.0"),
        "integrity score must not stay at 1.0, got: {combined}"
    );
}

/// Spec-correct LevelDB framing (masked CRC32C) whose payload is not an entity.
fn write_undecodable_leveldb_backup() -> tempfile::NamedTempFile {
    // Keep the CRC32C helper in-process so the test does not shell out to Python.
    fn crc32c(data: &[u8]) -> u32 {
        const POLY: u32 = 0x82F6_3B78;
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ POLY
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn mask(crc: u32) -> u32 {
        (((crc >> 15) | (crc << 17)).wrapping_add(0xA282_EAD8)) & 0xFFFF_FFFF
    }

    let payload = b"not-an-entity-payload";
    let record_type: u8 = 1; // FULL
    let mut type_and_payload = Vec::with_capacity(1 + payload.len());
    type_and_payload.push(record_type);
    type_and_payload.extend_from_slice(payload);
    let checksum = mask(crc32c(&type_and_payload));

    let mut block = Vec::with_capacity(32768);
    block.extend_from_slice(&checksum.to_le_bytes());
    block.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    block.push(record_type);
    block.extend_from_slice(payload);
    block.resize(32768, 0);

    let mut file = tempfile::NamedTempFile::new().expect("create backup file");
    file.write_all(&block).expect("write LevelDB block");
    file.flush().expect("flush backup file");
    file
}

/// `sslmode=require` cannot be honoured by a NoTls pool; connecting in the clear
/// instead would be a silent downgrade.
#[test]
fn import_rejects_an_unsupportable_sslmode() {
    let backup = write_document_backup("cli_sslmode");

    let output = Command::new(env!("CARGO_BIN_EXE_fireup"))
        .args(["import", "-b"])
        .arg(backup.path())
        .args(["-p", "postgresql://user:pw@localhost:5432/db?sslmode=require"])
        .output()
        .expect("run fireup import");

    assert!(!output.status.success(), "sslmode=require must be rejected");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not supported by this build"),
        "stderr should explain the TLS limitation, got: {stderr}"
    );
}
