//! End-to-end import tests against a real PostgreSQL instance.
//!
//! `fireup import` used to report a successful migration while writing zero
//! rows, so these tests assert the one thing that defect could not fake: the
//! row counts actually present in PostgreSQL after an import.
//!
//! The tests need a reachable database and are skipped when
//! `FIREUP_TEST_POSTGRES_URL` is unset, for example:
//!
//! ```sh
//! FIREUP_TEST_POSTGRES_URL='postgresql://user:pw@localhost:5432/fireup_test?sslmode=disable' \
//!   cargo test --test postgres_import_tests
//! ```
//!
//! Each test uses table names suffixed with a random id and drops them
//! afterwards, so concurrent runs against the same database do not collide.

use std::io::Write;
use std::process::Command;

use fireup::data_importer::{
    parse_connection_url, DocumentTransformer, PostgreSQLImporter, TableImportSpec,
};
use fireup::leveldb_parser::parser::FirestoreDocumentParser;
use fireup::leveldb_parser::parser::LevelDBParser;
use fireup::monitoring::{initialize_monitoring, MonitoringConfig};
use fireup::schema_analyzer::{DDLGenerator, DocumentStructureAnalyzer, NormalizationEngine};

const SKIP_NOTICE: &str = "skipping: set FIREUP_TEST_POSTGRES_URL to run the PostgreSQL import tests";

/// Every table the managed-export fixture normalizes into.
///
/// The fixture's kind names are fixed, so these tables cannot be suffixed per
/// run; leaving the lookup table behind made a second run fail on its index
/// (`relation "idx_Orders_Users_lookup_Orders_key_path" already exists`).
const FIXTURE_TABLES: [&str; 3] = ["Users", "Orders", "Orders_Users_lookup"];

fn test_url() -> Option<String> {
    match std::env::var("FIREUP_TEST_POSTGRES_URL") {
        Ok(url) if !url.trim().is_empty() => Some(url),
        _ => {
            eprintln!("{SKIP_NOTICE}");
            None
        }
    }
}

/// A collection name unique to this test run, so tests can run concurrently.
fn unique_collection(prefix: &str) -> String {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}_{}", &suffix[..8])
}

/// Write a Firestore-style JSON Lines export with `count` documents.
fn write_backup(collection: &str, count: usize) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("create backup file");
    for index in 0..count {
        writeln!(
            file,
            r#"{{"name":"projects/p/databases/(default)/documents/{collection}/doc{index}","fields":{{"email":{{"stringValue":"user{index}@example.com"}},"rank":{{"integerValue":"{index}"}},"active":{{"booleanValue":{}}}}}}}"#,
            index % 2 == 0
        )
        .expect("write document");
    }
    file.flush().expect("flush backup file");
    file
}

async fn connect(url: &str) -> PostgreSQLImporter {
    let config = parse_connection_url(url, 4, 30).expect("parse FIREUP_TEST_POSTGRES_URL");
    PostgreSQLImporter::new(config)
        .await
        .expect("connect to the test database")
}

async fn row_count(importer: &PostgreSQLImporter, table: &str) -> i64 {
    let rows = importer
        .execute_query(&format!("SELECT COUNT(*) FROM \"{table}\""), &[])
        .await
        .unwrap_or_else(|e| panic!("count rows in {table}: {e}"));
    rows.first().expect("one count row").get(0)
}

async fn drop_table(importer: &PostgreSQLImporter, table: &str) {
    importer
        .execute_statement(&format!("DROP TABLE IF EXISTS \"{table}\" CASCADE"), &[])
        .await
        .ok();
}

/// The library pipeline must land exactly one row per source document.
#[tokio::test]
async fn import_writes_one_row_per_source_document() {
    let Some(url) = test_url() else { return };
    initialize_monitoring(MonitoringConfig::default());

    const DOCUMENT_COUNT: usize = 25;
    let collection = unique_collection("fireup_it_docs");
    let backup = write_backup(&collection, DOCUMENT_COUNT);

    let parser = FirestoreDocumentParser::new(backup.path().to_string_lossy().to_string());
    let parse_result = parser
        .parse_backup(&backup.path().to_string_lossy())
        .await
        .expect("parse backup");
    assert_eq!(
        parse_result.documents.len(),
        DOCUMENT_COUNT,
        "fixture must parse completely before the import is meaningful"
    );

    let analysis = DocumentStructureAnalyzer::new()
        .analyze_documents(&parse_result.documents)
        .await
        .expect("analyze documents");
    let schema = NormalizationEngine::new()
        .normalize_schema(&analysis)
        .expect("normalize schema");

    let transformation = DocumentTransformer::new()
        .transform_documents(&parse_result.documents, &schema)
        .expect("transform documents");

    let specs: Vec<TableImportSpec> = schema
        .tables
        .iter()
        .filter_map(|table| {
            let rows = transformation.table_data.get(&table.name)?;
            (!rows.is_empty()).then(|| TableImportSpec::from_table_rows(table, rows))
        })
        .collect();
    assert!(!specs.is_empty(), "transformation produced no rows to import");

    let importer = connect(&url).await;
    for table in &schema.tables {
        drop_table(&importer, &table.name).await;
    }

    let ddl_statements = DDLGenerator::new()
        .generate_ddl(&schema)
        .expect("generate DDL")
        .all_statements();

    let result = importer
        .execute_full_import(&ddl_statements, &specs)
        .await
        .expect("execute full import");

    let expected_rows: usize = specs.iter().map(|spec| spec.rows.len()).sum();
    assert_eq!(result.total_records_expected, expected_rows);
    assert_eq!(result.total_records_failed, 0, "warnings: {:?}", result.warnings);
    assert_eq!(
        result.total_records_imported, expected_rows,
        "reported imports must match the rows handed to the importer"
    );
    assert!(result.is_successful(), "summary: {}", result.summary());

    // The reported counts are only trustworthy if the database agrees.
    let mut counted = 0i64;
    for spec in &specs {
        let actual = row_count(&importer, &spec.table_name).await;
        assert_eq!(
            actual as usize,
            spec.rows.len(),
            "table {} row count mismatch",
            spec.table_name
        );
        counted += actual;
    }
    assert_eq!(
        counted as usize, DOCUMENT_COUNT,
        "every source document must be represented by exactly one row"
    );

    for table in &schema.tables {
        drop_table(&importer, &table.name).await;
    }
}

/// The CLI path is what SAK-142 was filed against: it must insert rows and exit 0.
#[tokio::test]
async fn cli_import_populates_postgres_and_exits_zero() {
    let Some(url) = test_url() else { return };
    initialize_monitoring(MonitoringConfig::default());

    const DOCUMENT_COUNT: usize = 12;
    let collection = unique_collection("fireup_it_cli");
    let backup = write_backup(&collection, DOCUMENT_COUNT);

    let importer = connect(&url).await;
    drop_table(&importer, &collection).await;

    let output = Command::new(env!("CARGO_BIN_EXE_fireup"))
        .args(["import", "-b"])
        .arg(backup.path())
        .args(["-p", &url, "--batch-size", "5"])
        .output()
        .expect("run fireup import");

    assert!(
        output.status.success(),
        "fireup import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        row_count(&importer, &collection).await as usize,
        DOCUMENT_COUNT,
        "CLI import must insert one row per document"
    );

    // Re-importing with --drop-existing must replace the rows, not duplicate them.
    let output = Command::new(env!("CARGO_BIN_EXE_fireup"))
        .args(["import", "-b"])
        .arg(backup.path())
        .args(["-p", &url, "--drop-existing"])
        .output()
        .expect("re-run fireup import");

    assert!(
        output.status.success(),
        "fireup import --drop-existing failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        row_count(&importer, &collection).await as usize,
        DOCUMENT_COUNT,
        "--drop-existing must drop the previous rows"
    );

    drop_table(&importer, &collection).await;
}

/// `--continue-on-error` must import the rows it can and still report the
/// failures: a partial import is not a completed migration.
#[tokio::test]
async fn continue_on_error_imports_the_remaining_rows() {
    let Some(url) = test_url() else { return };
    initialize_monitoring(MonitoringConfig::default());

    // Two distinct documents plus a repeat of the first. The transformer derives
    // the primary key from the document id, so the repeat collides on the PK.
    let collection = unique_collection("fireup_it_partial");
    let mut backup = tempfile::NamedTempFile::new().expect("create backup file");
    for doc_id in ["d0", "d1", "d0"] {
        writeln!(
            backup,
            r#"{{"name":"projects/p/databases/(default)/documents/{collection}/{doc_id}","fields":{{"email":{{"stringValue":"{doc_id}@example.com"}}}}}}"#
        )
        .expect("write document");
    }
    backup.flush().expect("flush backup file");

    let importer = connect(&url).await;
    drop_table(&importer, &collection).await;

    // Without the flag the import aborts and leaves nothing behind.
    let output = Command::new(env!("CARGO_BIN_EXE_fireup"))
        .args(["import", "-b"])
        .arg(backup.path())
        .args(["-p", &url])
        .output()
        .expect("run fireup import");
    assert!(!output.status.success(), "a failing row must abort the import");
    assert_eq!(
        row_count(&importer, &collection).await,
        0,
        "an aborted import must not leave rows behind"
    );

    // With the flag the importable rows land, and the exit code still fails.
    let output = Command::new(env!("CARGO_BIN_EXE_fireup"))
        .args(["import", "-b"])
        .arg(backup.path())
        .args(["-p", &url, "--continue-on-error", "--drop-existing"])
        .output()
        .expect("run fireup import --continue-on-error");
    assert!(
        !output.status.success(),
        "a partial import must not report overall success"
    );
    assert_eq!(
        row_count(&importer, &collection).await,
        2,
        "--continue-on-error must import the rows that are importable"
    );

    drop_table(&importer, &collection).await;
}

/// Managed LevelDB export → parse → analyze → import in one CLI run.
///
/// SAK-141 closed the read path and SAK-142/143 closed import/DDL; this is the
/// remaining combination called out under "Not yet assessed" in PROJECT_STATUS.
#[tokio::test]
async fn cli_import_managed_export_fixture_into_postgres() {
    let Some(url) = test_url() else { return };
    initialize_monitoring(MonitoringConfig::default());

    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/export-onestore");
    assert!(
        fixture.is_dir(),
        "managed-export fixture missing at {}",
        fixture.display()
    );

    // Drop fixture tables if a prior run left them behind.
    let importer = connect(&url).await;
    for table in FIXTURE_TABLES {
        drop_table(&importer, table).await;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_fireup"))
        .args(["import", "-b"])
        .arg(&fixture)
        .args(["-p", &url, "--drop-existing"])
        .output()
        .expect("run fireup import against managed export");

    assert!(
        output.status.success(),
        "managed-export import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Fixture has 5 Users + 7 Orders (see tests/fixtures/README.md).
    assert_eq!(row_count(&importer, "Users").await, 5, "Users rows");
    assert_eq!(row_count(&importer, "Orders").await, 7, "Orders rows");

    for table in FIXTURE_TABLES {
        drop_table(&importer, table).await;
    }
}

/// Reserved words and mixed-case identifiers must survive the whole pipeline.
///
/// SAK-170: the DDL quoted every identifier but the INSERT builder did not, so a
/// Firestore field called `user` produced `syntax error at or near "user"` and a
/// kind called `Orders` produced an INSERT against the folded `orders`, which
/// the `"Orders"` table creation had never created.
#[tokio::test]
async fn cli_import_handles_reserved_words_and_mixed_case_identifiers() {
    let Some(url) = test_url() else { return };
    initialize_monitoring(MonitoringConfig::default());

    const DOCUMENT_COUNT: usize = 6;
    // Mixed case on purpose: unquoted, PostgreSQL folds this to lower case.
    let collection = format!(
        "Orders_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );

    let mut backup = tempfile::NamedTempFile::new().expect("create backup file");
    for index in 0..DOCUMENT_COUNT {
        // Every field name here is a PostgreSQL reserved word, plus one
        // mixed-case field name.
        writeln!(
            backup,
            r#"{{"name":"projects/p/databases/(default)/documents/{collection}/doc{index}","fields":{{"user":{{"stringValue":"u{index}"}},"order":{{"integerValue":"{index}"}},"group":{{"stringValue":"g{index}"}},"default":{{"booleanValue":{}}},"select":{{"stringValue":"s{index}"}},"table":{{"stringValue":"t{index}"}},"TotalCents":{{"integerValue":"{}"}}}}}}"#,
            index % 2 == 0,
            index * 100
        )
        .expect("write document");
    }
    backup.flush().expect("flush backup file");

    let importer = connect(&url).await;
    drop_table(&importer, &collection).await;

    let output = Command::new(env!("CARGO_BIN_EXE_fireup"))
        .args(["import", "-b"])
        .arg(backup.path())
        .args(["-p", &url, "--drop-existing"])
        .output()
        .expect("run fireup import");

    assert!(
        output.status.success(),
        "import with reserved-word columns failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        row_count(&importer, &collection).await as usize,
        DOCUMENT_COUNT,
        "every document must land in the mixed-case table"
    );

    // The reserved-word and mixed-case columns must hold the values, not NULL:
    // a column that silently went missing would still satisfy the row count.
    let rows = importer
        .execute_query(
            &format!(
                "SELECT COUNT(*) FROM \"{collection}\" \
                 WHERE \"user\" IS NOT NULL AND \"order\" IS NOT NULL \
                   AND \"group\" IS NOT NULL AND \"select\" IS NOT NULL \
                   AND \"table\" IS NOT NULL AND \"TotalCents\" IS NOT NULL"
            ),
            &[],
        )
        .await
        .expect("query reserved-word columns");
    let populated: i64 = rows.first().expect("one count row").get(0);
    assert_eq!(
        populated as usize, DOCUMENT_COUNT,
        "reserved-word and mixed-case columns must carry their values"
    );

    drop_table(&importer, &collection).await;
}

/// A connection string that cannot be honoured must fail, not fall back to some
/// other target. Before SAK-142 the URL was discarded entirely.
#[tokio::test]
async fn cli_import_honours_the_requested_database() {
    let Some(url) = test_url() else { return };

    let config = parse_connection_url(&url, 4, 30).expect("parse test URL");
    let unreachable = format!(
        "postgresql://{}:{}@{}:{}/fireup_absent_{}",
        config.user,
        config.password,
        config.host,
        config.port,
        uuid::Uuid::new_v4().simple()
    );

    let collection = unique_collection("fireup_it_target");
    let backup = write_backup(&collection, 3);

    let output = Command::new(env!("CARGO_BIN_EXE_fireup"))
        .args(["import", "-b"])
        .arg(backup.path())
        .args(["-p", &unreachable])
        .output()
        .expect("run fireup import");

    assert!(
        !output.status.success(),
        "importing into a non-existent database must fail"
    );
}
