//! The fidelity matrix in `docs/local/README.md` and the machine-readable copy
//! in `tests/conformance/matrix.json` must say the same thing.
//!
//! The matrix is the promise `fireup local` makes about itself, and the
//! conformance suite in `tests/conformance/` is what proves it. Those two drift
//! in the obvious way: someone implements a feature, flips the README row to
//! "served", and nothing demands the check that proves it — or someone adds a
//! row and no probe ever runs against it.
//!
//! So this test compares them row by row. It needs no server and no network,
//! which is the point: it fails in `cargo test` in a second, long before
//! anybody waits on the SDK layer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const ALLOWED_STATUSES: [&str; 3] = ["served", "unimplemented", "documented-divergence"];
const ALLOWED_DRIVERS: [&str; 3] = ["node-sdk", "node-grpc", "auth-smoke"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("reading {path:?}: {error}"))
}

fn matrix() -> serde_json::Value {
    let path = repo_root().join("tests/conformance/matrix.json");
    serde_json::from_str(&read(&path)).unwrap_or_else(|error| panic!("parsing {path:?}: {error}"))
}

/// Markdown and prose both say the same thing in several ways; compare meaning,
/// not formatting. Backticks, emphasis, and runs of whitespace are decoration —
/// re-styling a row must not count as changing what it claims.
fn normalize_label(cell: &str) -> String {
    // Not underscores: `set_to_server_value` and `all_descendants` are names,
    // not emphasis, and flattening them could conflate two distinct rows.
    let stripped: String = cell.chars().filter(|c| *c != '`' && *c != '*').collect();
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A status cell may carry an explanation after the verdict — "served — the SDK
/// flips the order", "`UNIMPLEMENTED` — use `limit`/`offset`". The verdict is
/// what has to match.
fn normalize_status(cell: &str) -> String {
    let lowered = normalize_label(cell).to_lowercase();
    if lowered.starts_with("served") {
        "served".to_string()
    } else if lowered.contains("unimplemented") {
        "unimplemented".to_string()
    } else {
        lowered
    }
}

/// The rows of the first markdown table after `heading`, as `(label, status)`.
/// `status_column` is the 0-based index of the status cell, or `None` for a
/// table that has no status column — the Field transforms table puts it third,
/// after the SDK spelling.
fn table_rows(
    readme: &str,
    heading: &str,
    status_column: Option<usize>,
) -> Vec<(String, Option<String>)> {
    let mut lines = readme.lines();
    lines
        .by_ref()
        .find(|line| line.trim() == heading)
        .unwrap_or_else(|| panic!("docs/local/README.md has no heading {heading:?}"));

    let mut rows = Vec::new();
    let mut seen_header = false;
    for line in lines {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') {
            if seen_header && !rows.is_empty() {
                break; // the table ended
            }
            if trimmed.starts_with("## ") || trimmed.starts_with("### ") {
                panic!("no table found under {heading:?} before the next heading");
            }
            continue;
        }

        let cells: Vec<&str> = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if !seen_header {
            seen_header = true;
            continue; // the header row
        }
        if cells.iter().all(|cell| {
            cell.chars()
                .all(|c| c == '-' || c == ':' || c.is_whitespace())
        }) {
            continue; // the separator row
        }

        let label = normalize_label(cells[0]);
        let status = status_column.map(|column| {
            normalize_status(cells.get(column).copied().unwrap_or_else(|| {
                panic!("table {heading:?} row {label:?} has no column {column}")
            }))
        });
        rows.push((label, status));
    }

    assert!(
        !rows.is_empty(),
        "no table rows found under {heading:?} in docs/local/README.md"
    );
    rows
}

#[test]
fn matrix_json_is_well_formed() {
    let matrix = matrix();
    let features = matrix["features"]
        .as_array()
        .expect("matrix.json must have a `features` array");
    assert!(!features.is_empty());

    let mut seen = BTreeMap::new();
    for feature in features {
        let id = feature["id"].as_str().expect("every feature needs an id");
        assert!(
            seen.insert(id, ()).is_none(),
            "duplicate feature id {id:?} in matrix.json"
        );

        let status = feature["status"].as_str().unwrap_or_default();
        assert!(
            ALLOWED_STATUSES.contains(&status),
            "feature {id:?} has status {status:?}, expected one of {ALLOWED_STATUSES:?}"
        );

        let driver = feature["driver"].as_str().unwrap_or_default();
        assert!(
            ALLOWED_DRIVERS.contains(&driver),
            "feature {id:?} has driver {driver:?}, expected one of {ALLOWED_DRIVERS:?}"
        );

        assert!(
            feature["label"].as_str().is_some_and(|l| !l.is_empty()),
            "feature {id:?} needs a label"
        );
        assert!(
            feature.get("python").is_some_and(|p| p.is_boolean()),
            "feature {id:?} needs an explicit `python` flag, so nobody has to guess \
             whether the Python layer is meant to cover it"
        );

        // A table-bound row must name a table that exists.
        if let Some(table) = feature["docTable"].as_str() {
            assert!(
                matrix["docTables"].get(table).is_some(),
                "feature {id:?} names docTable {table:?}, which matrix.json does not describe"
            );
        }
    }
}

#[test]
fn every_readme_matrix_row_has_a_matrix_json_entry() {
    let matrix = matrix();
    let readme = read(&repo_root().join(matrix["docFile"].as_str().unwrap()));

    for (table_id, table) in matrix["docTables"].as_object().unwrap() {
        let heading = table["heading"].as_str().unwrap();
        let status_column = table["statusColumn"].as_u64().map(|column| column as usize);
        let rows = table_rows(&readme, heading, status_column);

        let json_rows: BTreeMap<String, &serde_json::Value> = matrix["features"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|feature| feature["docTable"].as_str() == Some(table_id.as_str()))
            .map(|feature| (normalize_label(feature["label"].as_str().unwrap()), feature))
            .collect();

        for (label, status) in &rows {
            let feature = json_rows.get(label).unwrap_or_else(|| {
                panic!(
                    "docs/local/README.md, table {heading:?}, has row {label:?} with no entry in \
                     tests/conformance/matrix.json. Add it, and a probe for it in \
                     tests/conformance/ — a documented row with no check is a promise nobody \
                     verifies. Known rows: {:?}",
                    json_rows.keys().collect::<Vec<_>>()
                )
            });

            if let Some(status) = status {
                let claimed = feature["status"].as_str().unwrap();
                assert_eq!(
                    status, claimed,
                    "docs/local/README.md says {label:?} is {status:?}, but \
                     tests/conformance/matrix.json says {claimed:?}. Flip both at once: the \
                     matrix entry is what decides whether CI demands the feature works or \
                     demands it fails clearly."
                );
            }
        }

        // The other direction: an entry claiming a table row that is gone.
        let readme_labels: Vec<&String> = rows.iter().map(|(label, _)| label).collect();
        for (label, feature) in &json_rows {
            assert!(
                readme_labels.contains(&label),
                "tests/conformance/matrix.json entry {:?} claims to be row {label:?} of table \
                 {heading:?}, but docs/local/README.md has no such row. Rows there: {readme_labels:?}",
                feature["id"].as_str().unwrap()
            );
        }
    }
}

#[test]
fn the_firestore_rpc_table_covers_the_whole_service() {
    // The README says "All 17 RPCs of google.firestore.v1.Firestore". If the
    // vendored proto grows an RPC, the matrix has to grow a row — otherwise a
    // new RPC ships with no documented status and no check.
    let proto = read(&repo_root().join("proto/google/firestore/v1/firestore.proto"));
    let rpcs: Vec<String> = proto
        .lines()
        .filter_map(|line| line.trim().strip_prefix("rpc "))
        .map(|rest| {
            rest.split(['(', ' '])
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .filter(|name| !name.is_empty())
        .collect();
    assert!(
        rpcs.len() >= 17,
        "expected at least 17 RPCs, found {rpcs:?}"
    );

    let matrix = matrix();
    let documented: Vec<String> = matrix["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|feature| feature["docTable"].as_str() == Some("firestore-rpcs"))
        .map(|feature| {
            // "Write (streaming)" is the Write RPC with a note attached.
            normalize_label(feature["label"].as_str().unwrap())
                .split(' ')
                .next()
                .unwrap()
                .to_string()
        })
        .collect();

    for rpc in &rpcs {
        assert!(
            documented.contains(rpc),
            "google.firestore.v1.Firestore has an RPC {rpc:?} with no row in the fidelity \
             matrix. Every RPC is either served or an explicit refusal; say which."
        );
    }
}

#[test]
fn the_datastore_rpc_table_covers_the_whole_service() {
    let proto = read(&repo_root().join("proto/google/datastore/v1/datastore.proto"));
    let rpcs: Vec<String> = proto
        .lines()
        .filter_map(|line| line.trim().strip_prefix("rpc "))
        .map(|rest| {
            rest.split(['(', ' '])
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .filter(|name| !name.is_empty())
        .collect();
    assert!(!rpcs.is_empty());

    let matrix = matrix();
    // "BeginTransaction / Rollback" is one row covering two RPCs.
    let documented: Vec<String> = matrix["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|feature| feature["docTable"].as_str() == Some("datastore-rpcs"))
        .flat_map(|feature| {
            normalize_label(feature["label"].as_str().unwrap())
                .split(" / ")
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect();

    for rpc in &rpcs {
        assert!(
            documented.contains(rpc),
            "google.datastore.v1.Datastore has an RPC {rpc:?} with no row in the fidelity matrix"
        );
    }
}

#[test]
fn every_matrix_entry_is_claimed_by_a_conformance_probe() {
    // Cheap static check of what the Node runner enforces at runtime, so a
    // forgotten probe fails in `cargo test` instead of only in the SDK job.
    // It greps for the probe id rather than parsing JavaScript: a probe is
    // registered as `id: 'the.feature.id'`.
    let root = repo_root().join("tests/conformance");
    let mut sources = String::new();
    for name in [
        "node/probes-firestore.mjs",
        "node/probes-queries.mjs",
        "node/probes-datastore.mjs",
        "node/probes-transforms.mjs",
    ] {
        sources.push_str(&read(&root.join(name)));
    }
    let python = read(&root.join("python/conformance.py"));

    for feature in matrix()["features"].as_array().unwrap() {
        let id = feature["id"].as_str().unwrap();
        let driver = feature["driver"].as_str().unwrap();

        if driver == "node-sdk" || driver == "node-grpc" {
            assert!(
                sources.contains(&format!("id: '{id}'")),
                "no Node probe registers {id:?}. Add one in tests/conformance/node/, or move \
                 the row to a driver that does cover it."
            );
        }

        if feature["python"].as_bool() == Some(true) {
            assert!(
                python.contains(&format!("\"{id}\"")),
                "matrix.json marks {id:?} as covered by the Python layer, but \
                 tests/conformance/python/conformance.py has no probe for it."
            );
        }
    }
}
