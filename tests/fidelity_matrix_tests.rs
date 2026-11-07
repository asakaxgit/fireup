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
/// after the SDK spelling. `doc` names the file, for the panic messages.
fn table_rows_in(
    doc: &str,
    text: &str,
    heading: &str,
    status_column: Option<usize>,
) -> Vec<(String, Option<String>)> {
    let mut lines = text.lines();
    lines
        .by_ref()
        .find(|line| line.trim() == heading)
        .unwrap_or_else(|| panic!("{doc} has no heading {heading:?}"));

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
        "no table rows found under {heading:?} in {doc}"
    );
    rows
}

/// `table_rows_in` against the fidelity matrix's own document.
fn table_rows(
    readme: &str,
    heading: &str,
    status_column: Option<usize>,
) -> Vec<(String, Option<String>)> {
    table_rows_in("docs/local/README.md", readme, heading, status_column)
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

// ---------------------------------------------------------------------------
// docs/local/COMPARISON.md — the same rule, applied to the comparison tables.
//
// The comparison puts Fireup beside Google's emulator and the production
// service. Those two columns are a reading of someone else's documentation and
// nothing here can check them. Fireup's column is different:
// it makes the same claims as the fidelity matrix, in a second place, which is
// exactly how a document starts lying. So every comparison row whose label
// matches a matrix row is compared against `matrix.json`, and the number of
// rows that match has a floor — otherwise the linkage could be removed by
// rewording the labels, and the test would still pass while checking nothing.
// ---------------------------------------------------------------------------

const COMPARISON_DOC: &str = "docs/local/COMPARISON.md";

/// Section headings in the comparison whose second column is Fireup's verdict.
const COMPARISON_TABLES: [&str; 7] = [
    "## Firestore RPCs",
    "## Queries",
    "## Field transforms",
    "## Where the local implementations diverge from production",
    "## Datastore",
    "### Datastore property masks",
    "## Firebase Auth / Identity Toolkit",
];

/// Below this many cross-checked rows, assume the comparison drifted away from
/// the matrix rather than that the matrix shrank. Raise it when rows are added.
const COMPARISON_CROSS_CHECKED_FLOOR: usize = 57;

/// A comparison cell is a verdict, sometimes followed by an em-dash
/// explanation: "documented-divergence — read the link from …". The verdict is
/// what has to agree with the matrix.
fn comparison_verdict(cell: &str) -> String {
    let verdict = cell.split('—').next().unwrap_or(cell);
    normalize_status(verdict)
}

#[test]
fn the_comparison_doc_agrees_with_matrix_json_about_fireup() {
    let comparison = read(&repo_root().join(COMPARISON_DOC));
    let matrix = matrix();

    let by_label: BTreeMap<String, &serde_json::Value> = matrix["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|feature| (normalize_label(feature["label"].as_str().unwrap()), feature))
        .collect();

    let mut cross_checked = 0usize;
    for heading in COMPARISON_TABLES {
        // Column 1 is Fireup: the tables are
        // | subject | fireup | Firebase emulator | GCP |.
        for (label, verdict) in table_rows_in(COMPARISON_DOC, &comparison, heading, Some(1)) {
            let Some(feature) = by_label.get(&label) else {
                continue; // a product-level row with no matrix entry, by design
            };
            let verdict = comparison_verdict(verdict.as_deref().unwrap());
            let claimed = feature["status"].as_str().unwrap();
            assert_eq!(
                verdict, claimed,
                "{COMPARISON_DOC}, table {heading:?}, says Fireup's {label:?} is {verdict:?}, \
                 but tests/conformance/matrix.json says {claimed:?}. The matrix is the one with \
                 a probe behind it; fix the comparison."
            );
            cross_checked += 1;
        }
    }

    assert!(
        cross_checked >= COMPARISON_CROSS_CHECKED_FLOOR,
        "only {cross_checked} rows of {COMPARISON_DOC} matched a matrix.json label, expected at \
         least {COMPARISON_CROSS_CHECKED_FLOOR}. Comparison rows are tied to the matrix by their \
         label, so rewording one silently stops it being checked — keep the label verbatim from \
         the fidelity matrix."
    );
}

#[test]
fn the_readme_fidelity_badge_counts_what_the_matrix_says() {
    // The badge in README.md is a number about the matrix, sitting far away
    // from the matrix. Pin it, or it rots: a feature landing flips a status in
    // matrix.json and nobody thinks about a shields.io URL.
    let readme = read(&repo_root().join("README.md"));
    let features = matrix()["features"].as_array().unwrap().clone();
    let served = features
        .iter()
        .filter(|feature| feature["status"].as_str() == Some("served"))
        .count();
    let expected = format!("fidelity%20matrix-{served}%2F{}%20served", features.len());

    assert!(
        readme.contains(&expected),
        "README.md has no fidelity-matrix badge reading {expected:?}. \
         tests/conformance/matrix.json now holds {} rows, {served} of them served; \
         update the badge URL to match.",
        features.len()
    );
}

#[test]
fn the_coverage_floor_is_the_same_number_everywhere() {
    // The floor is written in three places that cannot see each other: the
    // script that enforces it, the workflow that runs the script, and the badge
    // that advertises it. Two of them agreeing is not enough — a badge claiming
    // a floor nobody enforces is worse than no badge.
    let script = read(&repo_root().join("scripts/coverage.sh"));
    let workflow = read(&repo_root().join(".github/workflows/coverage.yml"));
    let readme = read(&repo_root().join("README.md"));

    let after = |text: &str, marker: &str| -> String {
        let start = text
            .find(marker)
            .unwrap_or_else(|| panic!("no {marker:?} found; the coverage floor moved"))
            + marker.len();
        text[start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect()
    };

    let script_floor = after(&script, "FIREUP_COVERAGE_MIN:-");
    let workflow_floor = after(&workflow, "FIREUP_COVERAGE_MIN: ");
    assert_eq!(
        script_floor, workflow_floor,
        "scripts/coverage.sh defaults to a {script_floor}% floor but \
         .github/workflows/coverage.yml sets {workflow_floor}%"
    );

    // %E2%89%A5 is "≥", URL-escaped for shields.io.
    let badge = format!("line%20coverage-%E2%89%A5{script_floor}%25");
    assert!(
        readme.contains(&badge),
        "README.md's coverage badge does not advertise the {script_floor}% floor that \
         scripts/coverage.sh enforces; it should contain {badge:?}"
    );
    assert!(
        readme.contains(&format!("fails under {script_floor}% line coverage")),
        "README.md should say in words that CI fails under {script_floor}% line coverage"
    );
}

// ---------------------------------------------------------------------------
// The step that reports on the supply-chain gate (SAK-249)
// ---------------------------------------------------------------------------
//
// Everything below is about `ci.yml`, not about the fidelity matrix, and it
// lives in this file for one structural reason.
//
// `dependency_pin_tests` holds every assertion about the supply-chain gate --
// `deny.toml`'s policy, the gate's command line, the version and checksum pins,
// the workflow's triggers and token. All of them are reported by exactly one
// step of `ci.yml`. A guard on that step cannot be written inside
// `dependency_pin_tests`: switch the step off and the guard switches off with
// it, which is the one shape of guard that reports nothing at the moment it is
// needed. It has to run in a step other than the one it guards, and this target
// is the only other automatically-triggered target in the repository that
// already reads a workflow file.
//
// Measured, not assumed. Eight edits to `ci.yml` -- deleting that step, `if:
// false` or `continue-on-error: true` on the step or on the `test` job, and
// renaming the step -- each leave `dependency_pin_tests` at 171 passed and this
// target at 8 passed. Renaming the gate step instead reddens
// `dependency_pin_tests` by 9 tests, so the file is read and those greens were
// absence of an assertion.
//
// Deliberately out of scope: the two job-level keys. `if:` on the `test` job
// stops this target from running too, and `continue-on-error:` on it discards
// this target's failure along with the one it was reporting -- so no test in
// this repository can report either, because every test target runs in that one
// job. Closing that half needs the gate made a required status check, which is
// a repository setting rather than a file (SAK-242).

/// The target whose step this file guards. See the note above for why the guard
/// is not in that target.
const GUARDED_TARGET: &str = "dependency_pin_tests";

/// The `if:` values that leave a step running whatever else happened.
///
/// Deliberately not an expression evaluator -- the opposite. A text reader
/// cannot decide `github.event_name == 'push'`, so the only values allowed
/// through are the handful that mean "run anyway"; everything else fails
/// closed. Same list and same reasoning as `dependency_pin_tests`'
/// `CONDITIONS_THAT_STILL_RUN`.
const CONDITIONS_THAT_STILL_RUN: &[&str] = &[
    "always()",
    "${{ always() }}",
    "success()",
    "${{ success() }}",
    "!cancelled()",
    "${{ !cancelled() }}",
];

/// The two keys that decide whether a step runs and whether its verdict counts.
const STEP_CONDITION_KEYS: &[&str] = &["if", "continue-on-error"];

/// `line` with any comment removed.
///
/// A commented-out key is not a key, and `ci.yml` documents its own settings in
/// prose -- a reader that matched anywhere in the line would find the comment
/// and report on a setting that is not in effect.
fn without_comment(line: &str) -> &str {
    match line.split_once('#') {
        Some((before, _)) => before.trim_end(),
        None => line.trim_end(),
    }
}

/// Indentation of `line`, ignoring any comment on it.
fn indent_of(line: &str) -> usize {
    let bare = without_comment(line);
    bare.len() - bare.trim_start().len()
}

/// One live `if:` or `continue-on-error:` on a step.
#[derive(Debug, PartialEq, Eq)]
struct StepCondition {
    /// `if` or `continue-on-error`, without the colon.
    key: String,
    /// The value as written, comment stripped and trimmed.
    value: String,
}

/// The live `if:` / `continue-on-error:` on the `ci.yml` step that runs `target`.
///
/// Anchored on the command rather than on the step's `name:`. That is the
/// deliberate choice: renaming a step does not stop it running, so a reader
/// keyed on the name would fail on a harmless edit while still missing the
/// edits that matter. The command is what makes the step the reporter.
///
/// `Err` when the step cannot be located -- no such command, or more than one.
/// Fail-closed in both directions: no step at all is the reporter gone, which
/// is worse than any condition on it, and a reader that picked the first of two
/// would be reporting on whichever it happened to find.
fn reporting_step(ci: &str, target: &str) -> Result<Vec<StepCondition>, String> {
    let lines: Vec<&str> = ci.lines().collect();
    let needle = format!("--test {target}");

    let found: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| without_comment(line).contains(&needle))
        .map(|(at, _)| at)
        .collect();
    let at = match found[..] {
        [at] => at,
        [] => {
            return Err(format!(
                "ci.yml runs no `{needle}`, so nothing in CI reports the {target} \
                 assertions. Whatever they promise, they promise it to a file nobody \
                 executes. Restore the step, or point GUARDED_TARGET at wherever the \
                 target moved."
            ))
        }
        _ => {
            return Err(format!(
                "ci.yml has {} steps running `{needle}`; which of them is the reporter \
                 is not something this reader can tell, so it refuses to report on \
                 either.",
                found.len()
            ))
        }
    };

    // Walk up to the `- ` that opens the step holding that command.
    let mut start = None;
    for above in (0..=at).rev() {
        let bare = without_comment(lines[above]);
        if bare.trim_start().starts_with("- ") {
            start = Some(above);
            break;
        }
    }
    let Some(start) = start else {
        return Err(format!(
            "ci.yml's `{needle}` is not inside a step -- no `- ` line opens a block \
             above it. This reader only understands the step list form, so it refuses \
             rather than guessing which keys govern that command."
        ));
    };

    let step_indent = indent_of(lines[start]);
    let key_indent = step_indent + 2;
    let mut conditions = Vec::new();

    // A step's first key sits on the `- ` line itself, so read that one too:
    // `- if: false` followed by `name:` is the same step with the same effect.
    let inline = without_comment(lines[start]).trim_start();
    let mut keyed: Vec<&str> = vec![inline.trim_start_matches("- ")];

    for line in &lines[start + 1..] {
        let bare = without_comment(line);
        let inner = bare.trim_start();
        if inner.is_empty() {
            continue;
        }
        if bare.len() - inner.len() <= step_indent {
            break;
        }
        if indent_of(line) == key_indent {
            keyed.push(inner);
        }
    }

    for text in keyed {
        for key in STEP_CONDITION_KEYS {
            if let Some(value) = text.strip_prefix(&format!("{key}:")) {
                conditions.push(StepCondition {
                    key: (*key).to_string(),
                    value: value.trim().to_string(),
                });
            }
        }
    }

    Ok(conditions)
}

#[test]
fn ci_yml_still_counts_the_supply_chain_guards() {
    let ci = read(&repo_root().join(".github/workflows/ci.yml"));
    let conditions = match reporting_step(&ci, GUARDED_TARGET) {
        Ok(conditions) => conditions,
        Err(why) => panic!("{why}"),
    };

    for condition in &conditions {
        if condition.key == "continue-on-error" {
            assert_eq!(
                condition.value, "false",
                "ci.yml's `{GUARDED_TARGET}` step carries `continue-on-error: {}`, so \
                 every assertion in that target can fail and the job still concludes \
                 success. A step whose failure is discarded is not a gate. Measured: a \
                 step that really fails takes its job to `failure` (run 42153955324), \
                 which is what this key turns off.",
                condition.value
            );
        } else {
            assert!(
                CONDITIONS_THAT_STILL_RUN.contains(&condition.value.as_str()),
                "ci.yml's `{GUARDED_TARGET}` step carries `if: {}`, so the supply-chain \
                 assertions need not run at all. Measured: a skipped step leaves its job \
                 concluding success (run 42161681376), so this reads as green rather \
                 than as nothing having been checked. Allowed values are {:?}; anything \
                 a text reader cannot evaluate fails closed.",
                condition.value,
                CONDITIONS_THAT_STILL_RUN
            );
        }
    }
}

#[cfg(test)]
mod supply_chain_reporter_controls {
    use super::*;

    /// A `ci.yml` whose reporter step carries `extra` -- the shape the real file
    /// uses, reduced to the parts this reader looks at.
    ///
    /// Synthetic rather than the real file for the reason the arms below need:
    /// one mutation has to redden one test. An arm built by editing the real
    /// file reddens every control that also reads it.
    fn ci_with(extra: &str) -> String {
        format!(
            "name: CI\non:\n  pull_request:\njobs:\n  test:\n    runs-on: ubuntu-latest\n    \
             steps:\n      - uses: actions/checkout@v4\n\n      - name: Unit tests\n        \
             run: cargo test --locked --lib\n\n      - name: Supply-chain pins\n{extra}        \
             run: cargo test --locked --test dependency_pin_tests\n"
        )
    }

    #[test]
    fn the_shipped_reporter_step_is_locatable() {
        // Deliberately not an assertion about what the live keys are. The
        // shipped step has none, but a guard that demanded that would fail on a
        // *stricter* `ci.yml` -- someone adding `if: always()` -- and teach
        // whoever hit it to delete the guard rather than keep the step.
        let ci = read(&repo_root().join(".github/workflows/ci.yml"));
        assert!(
            reporting_step(&ci, GUARDED_TARGET).is_ok(),
            "the reader cannot find the {GUARDED_TARGET} step in the real ci.yml, so \
             every promise above is about a step it never located"
        );
    }

    #[test]
    fn a_step_with_no_conditions_passes() {
        assert_eq!(reporting_step(&ci_with(""), GUARDED_TARGET), Ok(Vec::new()));
    }

    #[test]
    fn switching_the_reporter_step_off_is_caught() {
        for off in ["        if: false\n", "        if: ${{ false }}\n"] {
            let conditions = reporting_step(&ci_with(off), GUARDED_TARGET)
                .expect("the step is still there, only switched off");
            assert!(
                !conditions
                    .iter()
                    .all(|c| CONDITIONS_THAT_STILL_RUN.contains(&c.value.as_str())),
                "{off:?} should not read as a condition that still runs"
            );
        }
    }

    #[test]
    fn a_condition_this_reader_cannot_evaluate_fails_closed() {
        let conditions = reporting_step(
            &ci_with("        if: github.event_name == 'push'\n"),
            GUARDED_TARGET,
        )
        .expect("located");
        assert_eq!(conditions.len(), 1);
        assert!(!CONDITIONS_THAT_STILL_RUN.contains(&conditions[0].value.as_str()));
    }

    #[test]
    fn conditions_that_still_run_the_step_pass() {
        // The arm that keeps this guard from rejecting a safer ci.yml.
        for ok in CONDITIONS_THAT_STILL_RUN {
            let conditions =
                reporting_step(&ci_with(&format!("        if: {ok}\n")), GUARDED_TARGET)
                    .expect("located");
            assert_eq!(conditions.len(), 1, "{ok}");
            assert!(
                CONDITIONS_THAT_STILL_RUN.contains(&conditions[0].value.as_str()),
                "{ok} should pass"
            );
        }
    }

    #[test]
    fn discarding_the_reporter_steps_verdict_is_caught() {
        for bad in [
            "        continue-on-error: true\n",
            "        continue-on-error: ${{ github.event_name == 'pull_request' }}\n",
        ] {
            let conditions = reporting_step(&ci_with(bad), GUARDED_TARGET).expect("located");
            assert_eq!(conditions.len(), 1, "{bad:?}");
            assert_ne!(
                conditions[0].value, "false",
                "{bad:?} should not read as false"
            );
        }
    }

    #[test]
    fn continue_on_error_false_passes() {
        // The sibling control: the key being present is not the finding, its
        // value is. Guarding the wrong one of these forces a redundant line or
        // protects nothing.
        let conditions = reporting_step(
            &ci_with("        continue-on-error: false\n"),
            GUARDED_TARGET,
        )
        .expect("located");
        assert_eq!(conditions[0].value, "false");
    }

    #[test]
    fn a_condition_on_the_dash_line_is_caught() {
        // `- if: false` then `name:` is the same step switched off, one column
        // further left than the arms above.
        let ci = "jobs:\n  test:\n    steps:\n      - if: false\n        name: Supply-chain \
                  pins\n        run: cargo test --locked --test dependency_pin_tests\n";
        let conditions = reporting_step(ci, GUARDED_TARGET).expect("located");
        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].value, "false");
    }

    #[test]
    fn a_multi_line_run_block_is_located() {
        // The other shape ci.yml uses for steps that run several targets.
        let ci = "jobs:\n  test:\n    steps:\n      - name: Supply-chain pins\n        \
                  continue-on-error: true\n        run: |\n          cargo test --locked \
                  --test dependency_pin_tests\n";
        let conditions = reporting_step(ci, GUARDED_TARGET).expect("located");
        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].value, "true");
    }

    #[test]
    fn a_missing_reporter_step_fails_closed() {
        let ci = "jobs:\n  test:\n    steps:\n      - name: Unit tests\n        run: cargo \
                  test --locked --lib\n";
        assert!(reporting_step(ci, GUARDED_TARGET).is_err());
    }

    #[test]
    fn a_commented_out_reporter_step_fails_closed() {
        // The edit that looks like the step is still there.
        let ci = "jobs:\n  test:\n    steps:\n      - name: Unit tests\n        run: cargo \
                  test --locked --lib\n      # run: cargo test --locked --test \
                  dependency_pin_tests\n";
        assert!(reporting_step(ci, GUARDED_TARGET).is_err());
    }

    #[test]
    fn two_reporter_steps_fail_closed() {
        let ci = format!(
            "{}      - name: Again\n        run: cargo test --locked --test \
             dependency_pin_tests\n",
            ci_with("")
        );
        assert!(reporting_step(&ci, GUARDED_TARGET).is_err());
    }
}
