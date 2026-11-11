//! The generated DDL must be accepted by PostgreSQL itself.
//!
//! Asserting on the DDL *string* cannot catch the failures that matter here:
//! the generator once emitted `user TEXT NOT NULL` for a Datastore property
//! called `user`, which PostgreSQL rejects outright (`syntax error at or near
//! "user"`) and which aborts every statement after it. So these tests hand the
//! generated script to `psql` with `ON_ERROR_STOP=1` and let the server be the
//! judge, then read the resulting catalog back to check the column types and
//! nullability the script actually produced.
//!
//! The tests need a reachable database and are skipped when
//! `FIREUP_TEST_POSTGRES_URL` is unset, for example:
//!
//! ```sh
//! FIREUP_TEST_POSTGRES_URL='postgresql:///fireup_test?host=/var/run/postgresql' \
//!   cargo test --test ddl_postgres_load_tests
//! ```
//!
//! Each test loads into its own randomly named PostgreSQL schema and drops it
//! afterwards, so concurrent runs against one database do not collide.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use fireup::leveldb_parser::parser::FirestoreDocumentParser;
use fireup::monitoring::{initialize_monitoring, MonitoringConfig};
use fireup::schema_analyzer::{
    DDLConfig, DDLGenerator, DocumentStructureAnalyzer, NormalizationEngine,
};
use fireup::types::{KEY_PATH_COLUMN, PARENT_KEY_PATH_COLUMN};

const SKIP_NOTICE: &str =
    "skipping: set FIREUP_TEST_POSTGRES_URL to run the generated-DDL load tests";

/// Both fixture trees carry the same logical dataset, so the generated DDL must
/// load for either payload format.
const FIXTURE_TREES: [&str; 2] = ["export-onestore", "export-v1-entity"];

/// The edge-case trees from `tests/fixtures/generate_edge_export.py`, split the
/// same way. See `tests/export_fidelity_tests.rs` for what each kind covers.
const EDGE_FIXTURE_TREES: [&str; 2] = ["export-edge-onestore", "export-edge-v1"];

fn test_url() -> Option<String> {
    match std::env::var("FIREUP_TEST_POSTGRES_URL") {
        Ok(url) if !url.trim().is_empty() => Some(url),
        _ => {
            eprintln!("{SKIP_NOTICE}");
            None
        }
    }
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// A PostgreSQL schema name unique to this test, so tests can run concurrently.
fn unique_schema(prefix: &str) -> String {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}_{}", &suffix[..8])
}

/// Run a SQL script through `psql`, aborting on the first error.
///
/// Returns `Err` with psql's diagnostics so a failing test names the offending
/// statement instead of just reporting a non-zero exit code.
fn psql_script(url: &str, sql: &str) -> Result<String, String> {
    let mut file = tempfile::NamedTempFile::new().expect("create SQL script file");
    file.write_all(sql.as_bytes()).expect("write SQL script");
    file.flush().expect("flush SQL script");

    let output = Command::new("psql")
        .arg(url)
        .args(["--no-psqlrc", "--quiet", "--variable=ON_ERROR_STOP=1", "--file"])
        .arg(file.path())
        .output()
        .expect("run psql — is the PostgreSQL client installed?");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        Err(format!(
            "psql exited with {:?}\n--- stderr ---\n{}\n--- stdout ---\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr),
            stdout
        ))
    }
}

/// Run a single query and return the rows as tab-separated, unaligned text.
fn psql_query(url: &str, sql: &str) -> Vec<String> {
    let output = Command::new("psql")
        .arg(url)
        .args([
            "--no-psqlrc",
            "--tuples-only",
            "--no-align",
            "--field-separator=\t",
            "--variable=ON_ERROR_STOP=1",
            "--command",
            sql,
        ])
        .output()
        .expect("run psql query");

    assert!(
        output.status.success(),
        "query failed: {}\n{}",
        sql,
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .filter(|line| !line.trim().is_empty())
        .collect()
}

/// Parse a fixture tree through fireup and generate DDL targeting `schema_name`.
async fn generate_ddl_for(tree: &str, schema_name: &str) -> Vec<String> {
    initialize_monitoring(MonitoringConfig::default());

    let root = fixtures_dir().join(tree);
    let parser = FirestoreDocumentParser::new(root.to_string_lossy().to_string());
    let parse_result = parser
        .parse_documents()
        .await
        .unwrap_or_else(|e| panic!("parse fixture tree {tree}: {e}"));
    assert!(
        !parse_result.documents.is_empty(),
        "fixture tree {tree} produced no documents, so the DDL would be vacuous"
    );

    let analysis = DocumentStructureAnalyzer::new()
        .analyze_documents(&parse_result.documents)
        .await
        .expect("analyze documents");
    let schema = NormalizationEngine::new()
        .normalize_schema(&analysis)
        .expect("normalize schema");

    let generator = DDLGenerator::with_config(DDLConfig {
        schema_name: Some(schema_name.to_string()),
        ..DDLConfig::default()
    });

    generator
        .generate_ddl(&schema)
        .expect("generate DDL")
        .all_statements()
}

/// A loaded schema that drops itself, even if the test panics part way through.
///
/// Without this the first failing assertion would leave its schema behind, and
/// a shared test database would silently fill up with them.
struct LoadedSchema {
    url: String,
    name: String,
}

impl Drop for LoadedSchema {
    fn drop(&mut self) {
        let _ = psql_script(
            &self.url,
            &format!("DROP SCHEMA IF EXISTS {} CASCADE;", quote_ident(&self.name)),
        );
    }
}

/// Load a fixture tree's generated DDL into a throwaway schema.
///
/// Panics with psql's diagnostics if PostgreSQL rejects any statement — which
/// is the primary assertion of this file.
async fn load_fixture_ddl(url: &str, tree: &str) -> LoadedSchema {
    let loaded = LoadedSchema {
        url: url.to_string(),
        name: unique_schema("fireup_ddl"),
    };
    let statements = generate_ddl_for(tree, &loaded.name).await;

    let script = format!(
        "CREATE SCHEMA {schema};\n{body}\n",
        schema = quote_ident(&loaded.name),
        body = statements.join("\n")
    );

    if let Err(diagnostics) = psql_script(url, &script) {
        panic!("generated DDL for {tree} was rejected by PostgreSQL:\n{diagnostics}");
    }

    loaded
}

fn quote_ident(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// `column -> data type` for one table, read back from the live catalog.
fn column_types(url: &str, schema_name: &str, table: &str) -> Vec<(String, String, bool)> {
    psql_query(
        url,
        &format!(
            "SELECT column_name, data_type, is_nullable \
             FROM information_schema.columns \
             WHERE table_schema = '{schema_name}' AND table_name = '{table}' \
             ORDER BY column_name"
        ),
    )
    .into_iter()
    .map(|line| {
        let mut parts = line.split('\t');
        let name = parts.next().unwrap_or_default().to_string();
        let data_type = parts.next().unwrap_or_default().to_string();
        let nullable = parts.next().unwrap_or_default() == "YES";
        (name, data_type, nullable)
    })
    .collect()
}

fn find_column<'a>(
    columns: &'a [(String, String, bool)],
    name: &str,
) -> &'a (String, String, bool) {
    columns
        .iter()
        .find(|(column, _, _)| column == name)
        .unwrap_or_else(|| {
            panic!(
                "column {name} is missing; table has {:?}",
                columns.iter().map(|(c, _, _)| c).collect::<Vec<_>>()
            )
        })
}

fn table_names(url: &str, schema_name: &str) -> Vec<String> {
    psql_query(
        url,
        &format!(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = '{schema_name}' ORDER BY table_name"
        ),
    )
}

// ---------------------------------------------------------------------------
// Acceptance tests
// ---------------------------------------------------------------------------

/// The headline criterion: the whole generated script loads, for every kind.
#[tokio::test]
async fn generated_ddl_for_every_fixture_kind_loads_in_postgres() {
    let Some(url) = test_url() else { return };

    for tree in FIXTURE_TREES {
        let loaded = load_fixture_ddl(&url, tree).await;
        let schema_name = &loaded.name;

        let tables = table_names(&url, &schema_name);
        // Identifiers are quoted, so the kind's own capitalisation survives
        // rather than silently folding to lower case.
        for kind in ["Users", "Orders"] {
            assert!(
                tables.iter().any(|table| table == kind),
                "tree {tree}: expected a table named {kind}, got {tables:?}"
            );
        }

    }
}

/// A Datastore property named `user` is legal; it must not break the script.
#[tokio::test]
async fn reserved_word_column_survives_as_a_quoted_identifier() {
    let Some(url) = test_url() else { return };

    let loaded = load_fixture_ddl(&url, "export-onestore").await;
    let schema_name = &loaded.name;

    let orders = column_types(&url, &schema_name, "Orders");
    // `Orders.user` is the key reference that used to emit `user TEXT NOT NULL`
    // and abort the load at that line.
    let (_, data_type, _) = find_column(&orders, "user");
    assert_eq!(
        data_type, "text",
        "a key reference should land in a text column"
    );

}

/// Datastore identity must be preserved, and must be the table's primary key.
#[tokio::test]
async fn datastore_key_is_preserved_and_is_the_primary_key() {
    let Some(url) = test_url() else { return };

    let loaded = load_fixture_ddl(&url, "export-onestore").await;
    let schema_name = &loaded.name;

    let users = column_types(&url, &schema_name, "Users");
    assert_eq!(
        find_column(&users, KEY_PATH_COLUMN).1,
        "text",
        "the natural key column must be text"
    );
    // Auto-allocated ids such as 4503599627370496 overflow int32.
    assert_eq!(find_column(&users, "key_id").1, "bigint");
    assert_eq!(find_column(&users, "key_kind").1, "text");
    assert_eq!(find_column(&users, "key_name").1, "text");
    assert_eq!(find_column(&users, "key_namespace").1, "text");

    // No surrogate UUID key is generated any more.
    assert!(
        !users.iter().any(|(column, _, _)| column == "id"),
        "a random surrogate key would make re-imports non-idempotent"
    );

    let pk_columns = psql_query(
        &url,
        &format!(
            "SELECT a.attname \
             FROM pg_index i \
             JOIN pg_class c ON c.oid = i.indrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey) \
             WHERE n.nspname = '{schema_name}' AND c.relname = 'Users' AND i.indisprimary \
             ORDER BY a.attname"
        ),
    );
    assert_eq!(
        pk_columns,
        vec![KEY_PATH_COLUMN.to_string()],
        "the Datastore key path must be the primary key"
    );

    // Child entities keep their parent path, so the hierarchy is reconstructable.
    let orders = column_types(&url, &schema_name, "Orders");
    assert_eq!(find_column(&orders, PARENT_KEY_PATH_COLUMN).1, "text");

}

/// Sparse fields must be nullable, or the import cannot insert the rows at all.
#[tokio::test]
async fn sparse_field_is_nullable_in_the_loaded_schema() {
    let Some(url) = test_url() else { return };

    let loaded = load_fixture_ddl(&url, "export-onestore").await;
    let schema_name = &loaded.name;

    let users = column_types(&url, &schema_name, "Users");
    // `nickname` is null on 4 of the 5 fixture users.
    let (_, _, nullable) = find_column(&users, "nickname");
    assert!(
        *nullable,
        "a field absent from some documents must be nullable"
    );

}

/// Types that JSON cannot represent must not be flattened into TEXT or JSONB.
#[tokio::test]
async fn datastore_types_map_to_faithful_postgres_types() {
    let Some(url) = test_url() else { return };

    let loaded = load_fixture_ddl(&url, "export-onestore").await;
    let schema_name = &loaded.name;

    let users = column_types(&url, &schema_name, "Users");

    assert_eq!(
        find_column(&users, "created_at").1,
        "timestamp with time zone",
        "a timestamp stored as text loses its instant semantics"
    );
    assert_eq!(
        find_column(&users, "avatar").1,
        "bytea",
        "non-UTF-8 blob bytes cannot round-trip through JSONB"
    );
    assert_eq!(
        find_column(&users, "age").1,
        "bigint",
        "Datastore integers are int64"
    );
    assert_eq!(
        find_column(&users, "location").1,
        "point",
        "a geo point must not be dropped from the schema"
    );
    // PR #62: `numeric` is decimal arbitrary precision. It has no signed zero
    // and does not round-trip every binary64 value through its decimal text
    // form, which is what a Datastore double actually needs.
    assert_eq!(
        find_column(&users, "score").1,
        "double precision",
        "a Datastore double is IEEE 754 binary64, not a decimal"
    );

}

/// A nested map is either one JSONB column or flat columns — never both.
#[tokio::test]
async fn nested_map_is_not_emitted_twice() {
    let Some(url) = test_url() else { return };

    let loaded = load_fixture_ddl(&url, "export-onestore").await;
    let schema_name = &loaded.name;

    let users = column_types(&url, &schema_name, "Users");
    let names: Vec<&String> = users.iter().map(|(column, _, _)| column).collect();

    let whole_map = names.iter().any(|name| *name == "address");
    let flattened: Vec<&&String> = names
        .iter()
        .filter(|name| name.starts_with("address_"))
        .collect();

    assert!(
        whole_map || !flattened.is_empty(),
        "the nested address map was dropped entirely, got {names:?}"
    );
    assert!(
        !(whole_map && !flattened.is_empty()),
        "address is emitted both as a JSONB column and flattened into {flattened:?}"
    );

}

/// PostgreSQL already indexes a primary key; a second index is pure overhead.
#[tokio::test]
async fn primary_key_is_not_indexed_twice() {
    let Some(url) = test_url() else { return };

    let loaded = load_fixture_ddl(&url, "export-onestore").await;
    let schema_name = &loaded.name;

    // Count indexes per table that cover exactly the primary key's columns.
    let duplicates = psql_query(
        &url,
        &format!(
            "SELECT c.relname, count(*) \
             FROM pg_index i \
             JOIN pg_class c ON c.oid = i.indrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = '{schema_name}' \
               AND i.indkey = (SELECT pi.indkey FROM pg_index pi \
                               WHERE pi.indrelid = c.oid AND pi.indisprimary) \
             GROUP BY c.relname HAVING count(*) > 1 \
             ORDER BY c.relname"
        ),
    );
    assert!(
        duplicates.is_empty(),
        "these tables index their primary key more than once: {duplicates:?}"
    );

}

// ---------------------------------------------------------------------------
// Edge-case fixtures (#60)
//
// `tests/export_fidelity_tests.rs` asserts these properties against the
// in-memory schema. These two tests hand the same schema to PostgreSQL, which
// is the only authority on whether an identifier is distinct: the server
// truncates anything past 63 bytes without an error, so a length bug is
// invisible until the catalog is read back.
// ---------------------------------------------------------------------------

/// Both edge trees, every kind, loaded by the server.
#[tokio::test]
async fn generated_ddl_for_the_edge_fixture_loads_in_postgres() {
    let Some(url) = test_url() else { return };

    for tree in EDGE_FIXTURE_TREES {
        let loaded = load_fixture_ddl(&url, tree).await;
        let tables = table_names(&url, &loaded.name);
        for kind in ["Awkward", "Fragments", "Idents", "Scoped", "Shards", "Values"] {
            assert!(
                tables.iter().any(|table| table == kind),
                "tree {tree}: expected a table named {kind}, got {tables:?}"
            );
        }
    }
}

/// Two property names that agree on their first 63 bytes must stay two columns.
///
/// `kind_Awkward` carries a 69-character pair differing only at byte 64.
/// PostgreSQL truncates both to the same 63 bytes, so emitting them verbatim
/// either aborts the load with a duplicate column or drops one field — and
/// reading `information_schema` back is the only way to see which.
#[tokio::test]
async fn long_property_names_load_as_two_distinct_columns() {
    let Some(url) = test_url() else { return };

    for tree in EDGE_FIXTURE_TREES {
        let loaded = load_fixture_ddl(&url, tree).await;
        let columns = column_types(&url, &loaded.name, "Awkward");

        let long: Vec<&String> = columns
            .iter()
            .map(|(name, _, _)| name)
            .filter(|name| name.starts_with("shipping_address_normalization"))
            .collect();
        assert_eq!(
            long.len(),
            2,
            "tree {tree}: the two long property names must survive as two columns, got {long:?} \
             out of {:?}",
            columns.iter().map(|(c, _, _)| c).collect::<Vec<_>>()
        );
        for name in &long {
            assert!(
                name.len() <= 63,
                "tree {tree}: PostgreSQL stored '{name}' ({} bytes); it would have truncated \
                 anything longer",
                name.len()
            );
        }
    }
}
