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

/// Run a query whose rows are a single text column.
async fn text_rows(importer: &PostgreSQLImporter, sql: &str) -> Vec<String> {
    importer
        .execute_query(sql, &[])
        .await
        .unwrap_or_else(|e| panic!("query failed: {sql}: {e}"))
        .iter()
        .map(|row| row.get::<_, String>(0))
        .collect()
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

    // SAK-195: blobs reach BYTEA as bytes, not as the ASCII of their base64.
    //
    // Asserting the column *type* is BYTEA cannot catch this: the pipeline
    // carries a blob as base64 text, and `$n::BYTEA` reads that text as a bytea
    // input literal, so `ada`'s 768-byte avatar used to arrive as 1024 bytes
    // whose first bytes spelled "AAECAwQFBgcI". The lengths below are the
    // fixture's real blob sizes, and `linus` is the 45 KB blob that has to be
    // reassembled from two LevelDB fragments first.
    for (key_path, expected_len) in [
        ("Users/ada", 768i64),
        ("Users/grace", 64),
        ("Users/4503599627370496", 0),
        ("Users/linus", 45_000),
        ("Users/unicode-ユーザー", 16),
    ] {
        let rows = importer
            .execute_query(
                "SELECT length(avatar) FROM \"Users\" WHERE key_path = $1",
                &[&key_path],
            )
            .await
            .unwrap_or_else(|e| panic!("read avatar for {key_path}: {e}"));
        let length: i32 = rows.first().expect("one row").get(0);
        assert_eq!(
            i64::from(length),
            expected_len,
            "{key_path}: avatar must be the blob's byte length, not its base64 length"
        );
    }

    let rows = importer
        .execute_query(
            "SELECT substring(avatar from 1 for 4) FROM \"Users\" WHERE key_path = 'Users/ada'",
            &[],
        )
        .await
        .expect("read avatar prefix");
    let prefix: Vec<u8> = rows.first().expect("one row").get(0);
    assert_eq!(
        prefix,
        vec![0u8, 1, 2, 3],
        "the fixture avatar starts with the bytes 00 01 02 03"
    );

    for table in FIXTURE_TABLES {
        drop_table(&importer, table).await;
    }
}

/// Every table the edge-case fixture normalizes into, for setup and teardown.
const EDGE_FIXTURE_TABLES: [&str; 6] =
    ["Awkward", "Fragments", "Idents", "Scoped", "Shards", "Values"];

/// Import the whole edge-case fixture and check what actually landed.
///
/// Deliberately one test rather than six: the kind names are fixed, so parallel
/// tests importing the same tables would race each other.
///
/// Each assertion below corresponds to a defect this fixture found (SAK-195).
/// `tests/export_fidelity_tests.rs` covers the same ground at the parser and
/// schema level; only PostgreSQL can decide whether the *rows* survived.
#[tokio::test]
async fn cli_import_edge_fixture_preserves_every_entity() {
    let Some(url) = test_url() else { return };
    initialize_monitoring(MonitoringConfig::default());

    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/export-edge-onestore");
    assert!(
        fixture.is_dir(),
        "edge-case fixture missing at {}",
        fixture.display()
    );

    let importer = connect(&url).await;
    for table in EDGE_FIXTURE_TABLES {
        drop_table(&importer, table).await;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_fireup"))
        .args(["import", "-b"])
        .arg(&fixture)
        .args(["-p", &url, "--drop-existing"])
        .output()
        .expect("run fireup import against the edge-case export");

    assert!(
        output.status.success(),
        "edge-case import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Row counts first: a kind, shard, or namespace that stops being read shows
    // up here before any of the specific assertions.
    for (table, expected) in [
        ("Shards", 6i64),    // output-0, output-1, output-2 and output-10
        ("Fragments", 3),
        ("Idents", 6),       // includes the id/name pair that used to merge
        ("Values", 4),
        ("Awkward", 2),
        ("Scoped", 4),       // 2 key names x 2 namespaces
    ] {
        assert_eq!(
            row_count(&importer, table).await,
            expected,
            "{table} rows"
        );
    }

    // An int64 id and a same-looking string name are different entities, so
    // they must be two rows with two distinct primary keys.
    let idents = text_rows(
        &importer,
        "SELECT key_path || ' | ' || coalesce(key_name, '-') || ' | ' \
            || coalesce(key_id::text, '-') FROM \"Idents\" ORDER BY label",
    )
    .await;
    assert_eq!(
        idents,
        vec![
            "Idents/9223372036854775807 | - | 9223372036854775807",
            "Idents/-5 | - | -5",
            "Idents/1001 | - | 1001",
            "Idents/\"a/Idents/9\" | a/Idents/9 | -",
            "Idents/\"1001\" | 1001 | -",
            "Org/acme/Team/42/Idents/deep | deep | -",
        ],
        "key identity must survive to the row level"
    );

    // The same key name in two namespaces must not collide on the primary key.
    let scoped = text_rows(
        &importer,
        "SELECT key_path FROM \"Scoped\" ORDER BY key_path",
    )
    .await;
    assert_eq!(
        scoped,
        vec![
            "[tenant-a]Scoped/also-shared",
            "[tenant-a]Scoped/shared-name",
            "[tenant-b]Scoped/also-shared",
            "[tenant-b]Scoped/shared-name",
        ],
        "namespaced keys must stay distinct"
    );

    // NaN and ±Infinity are legal Datastore doubles and legal `numeric` input.
    // They used to fail the record outright on the v1 decoder and collapse to
    // NULL on the legacy one.
    let ratios = text_rows(
        &importer,
        "SELECT key_path || '=' || ratio::text FROM \"Values\" ORDER BY key_path",
    )
    .await;
    assert_eq!(
        ratios,
        vec![
            "Values/coercions=Infinity",
            "Values/infinities=-Infinity",
            "Values/limits=NaN",
            "Values/negative-infinity=NaN",
        ],
        "non-finite doubles must reach PostgreSQL"
    );

    // int64 limits must not round-trip through f64.
    let limits = text_rows(
        &importer,
        "SELECT big::text || ',' || small::text FROM \"Values\" WHERE key_path = 'Values/limits'",
    )
    .await;
    assert_eq!(
        limits,
        vec!["9223372036854775807,-9223372036854775808"],
        "int64 limits must be exact"
    );

    // A property name with a literal dot becomes the column `meta_source`;
    // looking the value up by that derived name finds nothing, which made the
    // NOT NULL column abort the entire import.
    let dotted = text_rows(
        &importer,
        "SELECT meta_source FROM \"Awkward\" ORDER BY key_path",
    )
    .await;
    assert_eq!(
        dotted,
        vec!["literal-dot", "literal-dot"],
        "a property name containing a dot must keep its value"
    );

    // The 69-character pair that agrees on its first 63 bytes must be two
    // columns holding two different values, not one column holding one.
    let long_columns = text_rows(
        &importer,
        "SELECT column_name FROM information_schema.columns \
         WHERE table_name = 'Awkward' \
           AND column_name LIKE 'shipping_address_normalization%' \
         ORDER BY column_name",
    )
    .await;
    assert_eq!(
        long_columns.len(),
        2,
        "expected two distinct long columns, got {long_columns:?}"
    );
    let long_values = text_rows(
        &importer,
        &format!(
            "SELECT \"{}\" || '/' || \"{}\" FROM \"Awkward\" ORDER BY key_path",
            long_columns[0], long_columns[1]
        ),
    )
    .await;
    for value in &long_values {
        let (left, right) = value.split_once('/').expect("two values");
        assert_ne!(
            left, right,
            "the two long columns must hold the two different source values"
        );
    }

    // The 80 KB blob spans three LevelDB blocks, so its bytes only come out
    // right if FIRST/MIDDLE/LAST reassembly and the BYTEA decode both work.
    let blob = text_rows(
        &importer,
        "SELECT length(payload)::text FROM \"Fragments\" \
         WHERE key_path = 'Fragments/three-block-blob'",
    )
    .await;
    assert_eq!(
        blob,
        vec!["80000"],
        "a blob reassembled from three blocks must arrive at its exact length"
    );

    for table in EDGE_FIXTURE_TABLES {
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
