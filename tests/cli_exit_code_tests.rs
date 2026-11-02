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
