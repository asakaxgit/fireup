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
///
/// Compared against the value with any surrounding YAML quotes removed, which
/// `!cancelled()` needs: bare, a leading `!` is a YAML tag indicator and the
/// file does not parse, so the only spelling of that entry anyone can actually
/// ship is `if: '!cancelled()'`.
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

/// `value` with one matching pair of surrounding YAML quotes removed.
///
/// Not cosmetic. `if: !cancelled()` is not valid YAML -- a leading `!` is a tag
/// indicator -- so the only shippable spelling of that allowed value is
/// `if: '!cancelled()'`, and a reader comparing the raw text would reject the
/// one form anybody can write. Measured: it reddened the `!cancelled()` arm.
///
/// Unquoting cannot open a hole. It can only turn a non-matching value into one
/// of `CONDITIONS_THAT_STILL_RUN`, and every value on that list means "run
/// anyway" however it is quoted. `if: 'false'` unquotes to `false`, which is
/// still not on the list.
fn unquote(value: &str) -> &str {
    for quote in ['\'', '"'] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|v| v.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

/// One live `if:` or `continue-on-error:` on a step.
#[derive(Debug, PartialEq, Eq)]
struct StepCondition {
    /// `if` or `continue-on-error`, without the colon.
    key: String,
    /// The value as written, comment stripped, trimmed and unquoted.
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
    let needle = format!("--test {target}");
    let missing = format!(
        "ci.yml runs no `{needle}`, so nothing in CI reports the {target} \
         assertions. Whatever they promise, they promise it to a file nobody \
         executes. Restore the step, or point GUARDED_TARGET at wherever the \
         target moved."
    );
    step_conditions(ci, &needle, &missing).map(|(_, conditions)| conditions)
}

/// The live `if:` / `continue-on-error:` on the one step whose command contains
/// `needle`, with the line that opens that step. `missing` is the error when no
/// step runs `needle`; the other refusals are the ones `reporting_step`
/// documents.
fn step_conditions(
    ci: &str,
    needle: &str,
    missing: &str,
) -> Result<(usize, Vec<StepCondition>), String> {
    let lines: Vec<&str> = ci.lines().collect();

    let found: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| without_comment(line).contains(needle))
        .map(|(at, _)| at)
        .collect();
    let at = match found[..] {
        [at] => at,
        [] => return Err(missing.to_string()),
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
                    value: unquote(value.trim()).to_string(),
                });
            }
        }
    }

    Ok((start, conditions))
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
    fn a_quoted_condition_that_still_runs_passes() {
        // `!cancelled()` has to be quoted to be valid YAML at all, so a reader
        // that compared the raw text would reject the only spelling of it
        // anyone can ship. Both quote styles, because YAML takes either.
        for ok in ["'!cancelled()'", "\"always()\"", "'success()'"] {
            let conditions =
                reporting_step(&ci_with(&format!("        if: {ok}\n")), GUARDED_TARGET)
                    .expect("located");
            assert_eq!(conditions.len(), 1, "{ok}");
            assert!(
                CONDITIONS_THAT_STILL_RUN.contains(&conditions[0].value.as_str()),
                "{ok} is a condition that still runs the step, only quoted"
            );
        }
    }

    #[test]
    fn a_quoted_hole_is_still_caught() {
        // The direction control for unquoting: it must not let a value through
        // just because it had quotes on it. Each key is checked against the
        // predicate the live test applies to that key.
        let switched_off =
            reporting_step(&ci_with("        if: 'false'\n"), GUARDED_TARGET).expect("located");
        assert_eq!(switched_off.len(), 1);
        assert!(
            !CONDITIONS_THAT_STILL_RUN.contains(&switched_off[0].value.as_str()),
            "`if: 'false'` should still read as switched off, not as a condition that runs"
        );

        let discarded = reporting_step(
            &ci_with("        continue-on-error: \"true\"\n"),
            GUARDED_TARGET,
        )
        .expect("located");
        assert_eq!(discarded.len(), 1);
        assert_ne!(
            discarded[0].value, "false",
            "`continue-on-error: \"true\"` should still read as discarding the verdict"
        );
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

/// The libtest arguments that narrow which tests in a target actually run.
///
/// Each was measured on this tree against a `deny.toml` widened so that one
/// assertion in `dependency_pin_tests` fails. The shipped command exits 101
/// and reports it; each of these leaves the step at **exit 0** (SAK-253):
///
/// | command tail | step exit | test result line |
/// |---|---|---|
/// | *(none — shipped)* | **101** | `FAILED. 182 passed; 1 failed` |
/// | `-- --skip deny_` | 0 | `ok. 162 passed; 0 failed; 21 filtered out` |
/// | `-- --exact no_such_test` | 0 | `ok. 0 passed; 0 failed; 183 filtered out` |
/// | `-- --ignored` | 0 | `ok. 0 passed; 0 failed; 183 filtered out` |
/// | `-- --list` | 0 | *(no test result line at all)* |
///
/// `--exact` is on the list even though it needs a name beside it to be
/// useful, because the useless spelling is the dangerous one: `--exact` with a
/// name no test has selects **nothing** and still reads `ok`.
const TEST_FILTERING_ARGS: &[&str] = &["--skip", "--exact", "--ignored", "--list"];

/// libtest arguments whose value is the *next* word rather than glued on with
/// `=`, so that a reader looking for a bare positional filter does not mistake
/// the `1` in `--test-threads 1` for one.
///
/// Both spellings of that flag were measured and both keep the step at exit
/// 101, so both have to stay green.
const LIBTEST_ARGS_WITH_A_VALUE: &[&str] = &[
    "--test-threads",
    "--logfile",
    "--color",
    "--format",
    "--shuffle-seed",
    "-Z",
];

/// The command a reporter step hands to the shell, plus whether that body
/// turns `pipefail` on above it.
#[derive(Debug, PartialEq, Eq)]
struct ReporterCommand {
    /// The one logical command that runs the target: `\`-continuations joined,
    /// comment stripped. Kept as text because two of the questions below are
    /// about shell operators rather than about words.
    command: String,
    /// `true` if a `set … pipefail` line runs earlier in the same `run:` body.
    ///
    /// Read, not assumed. The runner gives this workflow `/usr/bin/bash -e
    /// {0}` — measured from the `shell:` line GitHub prints for every step of
    /// run 42362531653 — and that is `-e` **without** `-o pipefail`, so a pipe
    /// discards the command's exit status unless the body asks for it back.
    pipefail: bool,
}

/// The command on the `ci.yml` step that runs `target`.
///
/// Anchored on the command for the reason `reporting_step` gives: renaming a
/// step does not stop it running. `Err` in every case where the reader cannot
/// say which command runs the target, for the reason it gives too — the
/// reporter gone is worse than anything written on it.
fn reporter_command(ci: &str, target: &str) -> Result<ReporterCommand, String> {
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
                 assertions."
            ))
        }
        _ => {
            return Err(format!(
                "ci.yml has {} steps running `{needle}`; which of them is the reporter \
                 is not something this reader can tell.",
                found.len()
            ))
        }
    };

    let mut start = None;
    for above in (0..=at).rev() {
        if without_comment(lines[above]).trim_start().starts_with("- ") {
            start = Some(above);
            break;
        }
    }
    let Some(start) = start else {
        return Err(format!(
            "ci.yml's `{needle}` is not inside a step -- no `- ` line opens a block \
             above it, and this reader only understands the step list form."
        ));
    };

    // The `run:` that holds it: either the needle's own line, or a block
    // scalar opened above it inside the same step.
    let run_at = (start..=at)
        .rev()
        .find(|above| {
            without_comment(lines[*above])
                .trim_start()
                .starts_with("run:")
        })
        .ok_or_else(|| {
            format!("ci.yml's `{needle}` is not under a `run:` key this reader can find.")
        })?;

    let run_line = without_comment(lines[run_at]);
    let run_body = run_line.trim_start();
    let run_indent = run_line.len() - run_body.len();
    let inline = run_body.strip_prefix("run:").unwrap_or_default().trim();

    let mut raw: Vec<&str> = Vec::new();
    if inline.starts_with('|') || inline.starts_with('>') {
        for next in &lines[run_at + 1..] {
            let bare = without_comment(next);
            let body = bare.trim_start();
            if body.is_empty() {
                continue;
            }
            if bare.len() - body.len() <= run_indent {
                break;
            }
            raw.push(body);
        }
    } else if !inline.is_empty() {
        raw.push(inline);
    }
    if raw.is_empty() {
        return Err(format!(
            "the step running `{needle}` in ci.yml has an empty `run:` body."
        ));
    }

    // Join `\`-continuations: one logical command per entry.
    let mut logical: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in raw {
        match line.strip_suffix('\\') {
            Some(head) => {
                current.push_str(head.trim_end());
                current.push(' ');
            }
            None => {
                current.push_str(line);
                logical.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() {
        logical.push(current);
    }

    let running: Vec<usize> = logical
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains(&needle))
        .map(|(at, _)| at)
        .collect();
    // No control covers the `_` arm below, and that is deliberate rather than
    // an oversight: `found` above counts the *lines* carrying the needle and
    // already refuses more than one, so by here the body holds exactly one.
    // The arm stays because it fails closed if that ever stops being true --
    // measured by disabling it (SAK-253), and every control stayed green,
    // which is how we know it is unreachable rather than unguarded.
    let command_at = match running[..] {
        [one] => one,
        _ => {
            return Err(format!(
                "the step's `run:` body has {} commands running `{needle}`; this reader \
                 reports on one or on none.",
                running.len()
            ))
        }
    };

    Ok(ReporterCommand {
        pipefail: logical[..command_at].iter().any(|line| {
            let text = line.trim_start();
            text.starts_with("set ") && text.contains("pipefail")
        }),
        command: logical[command_at].clone(),
    })
}

/// `Ok` if that command runs the whole target and its verdict survives.
///
/// Three assertions, each for a measured fail-open — the step stays at exit 0
/// while an assertion in the target is failing:
///
/// 1. The exit is not handed to `||`.
/// 2. The exit is not handed to a **pipe**, unless the body turns `pipefail`
///    back on. See `ReporterCommand::pipefail`: the runner's shell is `bash -e`
///    with no `pipefail`, so `| tee` exits 0 on a failing command.
/// 3. Nothing narrows the set of tests that run: no `--no-run`, none of
///    `TEST_FILTERING_ARGS`, and no bare positional filter after `--`.
///
/// Deliberately **not** asserted, all measured on the widened tree and all
/// still exit 101, so a guard reddening them would be reddening a command that
/// works: `--all-features`, `--frozen`, `--no-fail-fast`, `; true`,
/// `> /dev/null`, `-- --nocapture`, `-- --test-threads=1`, `-- --test-threads 1`,
/// `-- --include-ignored` and an unfamiliar `-- --format=terse`. An unknown
/// libtest flag passes on purpose: it is as likely to widen as to narrow, and
/// the three families above are every documented way libtest subtracts.
fn check_the_reporter_runs_every_test(
    reporter: &ReporterCommand,
    target: &str,
) -> Result<(), String> {
    let command = &reporter.command;

    if command.contains("||") {
        return Err(format!(
            "ci.yml's `{target}` step sends that target's exit status to `||`: \
             `{command}`. The tests still run and still print `FAILED`, and the step \
             exits 0 anyway. Measured (SAK-253) with one assertion in the target \
             failing: the command alone exits 101, and with `|| true` after it exits 0 \
             while the log still says `182 passed; 1 failed`. Use `;` if something has \
             to run either way -- that keeps the failure, measured."
        ));
    }

    if command.contains('|') && !reporter.pipefail {
        return Err(format!(
            "ci.yml's `{target}` step pipes that target's output: `{command}`, and the \
             body does not turn `pipefail` on. This workflow declares no `shell:`, so \
             GitHub runs every step under `/usr/bin/bash -e {{0}}` -- measured from the \
             `shell:` line of run 42362531653 -- and that is `-e` without `-o pipefail`, \
             so the step takes the *last* command's status and the test failure is \
             discarded. Measured (SAK-253) with one assertion failing: the command alone \
             exits 101; `| tee /dev/null` exits 0; the same pipe under a shell with \
             `-o pipefail` exits 101. Add `set -o pipefail` above it in the same `run:` \
             body -- measured, and accepted here -- or drop the pipe."
        ));
    }

    let words: Vec<&str> = command
        .split_whitespace()
        .filter(|word| *word != "\\")
        .collect();

    if words.iter().any(|word| *word == "--no-run") {
        return Err(format!(
            "ci.yml's `{target}` step runs cargo test with `--no-run`: `{command}`. That \
             compiles the target and runs none of it, and the step exits 0 with no test \
             result line at all. Measured (SAK-253): with one assertion in the target \
             failing, the shipped command exits 101 and this exits 0."
        ));
    }

    let Some(separator) = words.iter().position(|word| *word == "--") else {
        return Ok(());
    };
    let mut at = separator + 1;
    while at < words.len() {
        let word = words[at];
        let name = word.split('=').next().unwrap_or(word);

        if TEST_FILTERING_ARGS.contains(&name) {
            return Err(format!(
                "ci.yml's `{target}` step narrows which tests run with `{word}`: \
                 `{command}`. The tests it filters out cannot report anything, and the \
                 step exits 0 reading `ok`. Measured (SAK-253) with one assertion in the \
                 target failing: `-- --skip deny_` exits 0 at `162 passed; 21 filtered \
                 out`, `-- --exact no_such_test` and `-- --ignored` exit 0 at `0 passed; \
                 183 filtered out`. Narrowing arguments are {TEST_FILTERING_ARGS:?}."
            ));
        }

        if !word.starts_with('-') {
            return Err(format!(
                "ci.yml's `{target}` step passes `{word}` as a positional test filter: \
                 `{command}`. Only tests whose name contains it then run. Measured \
                 (SAK-253) with one assertion failing: `-- the_committed_lock` exits 0 \
                 at `2 passed; 181 filtered out`, and `-- negative_controls` exits 0 at \
                 `150 passed; 33 filtered out` -- 150 green tests and the one that had \
                 something to say filtered away."
            ));
        }

        if LIBTEST_ARGS_WITH_A_VALUE.contains(&name) && !word.contains('=') {
            at += 1;
        }
        at += 1;
    }

    Ok(())
}

#[test]
fn nothing_in_ci_yml_narrows_the_supply_chain_reporter() {
    let ci = read(&repo_root().join(".github/workflows/ci.yml"));
    let reporter = match reporter_command(&ci, GUARDED_TARGET) {
        Ok(reporter) => reporter,
        Err(why) => panic!("{why}"),
    };
    if let Err(why) = check_the_reporter_runs_every_test(&reporter, GUARDED_TARGET) {
        panic!("{why}");
    }
}

#[cfg(test)]
mod reporter_command_controls {
    use super::*;

    /// A `ci.yml` whose reporter step runs `tail`, reduced to the parts this
    /// reader looks at.
    ///
    /// Synthetic rather than the real file so that one mutation reddens one
    /// test: an arm built by editing the real file reddens every control that
    /// also reads it.
    fn ci_running(tail: &str) -> String {
        format!(
            "name: CI\non:\n  pull_request:\njobs:\n  test:\n    runs-on: ubuntu-latest\n    \
             steps:\n      - uses: actions/checkout@v4\n\n      - name: Unit tests\n        \
             run: cargo test --locked --lib\n\n      - name: Supply-chain pins\n        \
             run: cargo test --locked --test dependency_pin_tests{tail}\n"
        )
    }

    fn verdict(tail: &str) -> Result<(), String> {
        let ci = ci_running(tail);
        let reporter = reporter_command(&ci, GUARDED_TARGET).expect("located");
        check_the_reporter_runs_every_test(&reporter, GUARDED_TARGET)
    }

    #[test]
    fn the_shipped_reporter_command_is_read_and_passes() {
        let ci = read(&repo_root().join(".github/workflows/ci.yml"));
        let reporter = reporter_command(&ci, GUARDED_TARGET).expect("the real ci.yml");
        assert!(
            reporter.command.contains("cargo test"),
            "the reader found `{}` rather than a cargo test command, so the assertions \
             below describe a command that is not the one CI runs",
            reporter.command
        );
        assert_eq!(verdict(""), Ok(()));
    }

    #[test]
    fn swallowing_the_verdict_is_caught() {
        assert!(verdict(" || true").is_err());
        assert!(verdict(" ||true").is_err());
        assert!(verdict(" | tee /dev/null").is_err());
        assert!(verdict(" | cat").is_err());
    }

    #[test]
    fn a_pipe_under_a_body_that_restores_pipefail_passes() {
        // The one spelling that measurably keeps the failure on the runner's
        // own shell. Without this arm the pipe assertion would have no way to
        // be satisfied except by deleting the pipe, which is a guard telling
        // people a working command is broken.
        let ci = "name: CI\njobs:\n  test:\n    steps:\n      - name: Supply-chain pins\n        \
                  run: |\n          set -o pipefail\n          cargo test --locked --test \
                  dependency_pin_tests | tee out.log\n";
        let reporter = reporter_command(ci, GUARDED_TARGET).expect("located");
        assert!(reporter.pipefail);
        assert_eq!(
            check_the_reporter_runs_every_test(&reporter, GUARDED_TARGET),
            Ok(())
        );
    }

    #[test]
    fn a_pipefail_body_does_not_excuse_a_double_pipe() {
        // The control that makes the `||` assertion its own. `|| true` contains
        // a `|`, so with a bare pipe already rejected the pipe assertion covers
        // it -- until the body turns `pipefail` on, at which point the pipe
        // assertion passes it and only the `||` assertion is left. Measured:
        // `-o pipefail` does nothing for `||` at all (exit 0 under both
        // shells), so this has to be rejected whatever the body says.
        let ci = "name: CI\njobs:\n  test:\n    steps:\n      - name: Supply-chain pins\n        \
                  run: |\n          set -o pipefail\n          cargo test --locked --test \
                  TARGET || true\n";
        let ci = ci.replace("TARGET", GUARDED_TARGET);
        let reporter = reporter_command(&ci, GUARDED_TARGET).expect("located");
        assert!(reporter.pipefail);
        let error = check_the_reporter_runs_every_test(&reporter, GUARDED_TARGET)
            .expect_err("`|| true` discards the verdict with or without pipefail");
        assert!(
            error.contains("sends that target's exit status to `||`"),
            "{error}"
        );
    }

    #[test]
    fn running_none_of_the_target_is_caught() {
        assert!(verdict(" --no-run").is_err());
    }

    #[test]
    fn narrowing_which_tests_run_is_caught() {
        assert!(verdict(" -- --skip deny_").is_err());
        assert!(verdict(" -- --exact no_such_test").is_err());
        assert!(verdict(" -- --ignored").is_err());
        assert!(verdict(" -- --list").is_err());
        assert!(verdict(" -- the_committed_lock").is_err());
        assert!(verdict(" -- --skip=deny_").is_err());
    }

    #[test]
    fn commands_that_still_run_everything_pass() {
        // Every one of these was measured on a widened tree and still exits
        // 101. A guard reddening any of them would be reddening a command that
        // reports exactly what it is supposed to report.
        for tail in [
            " --all-features",
            " --no-fail-fast",
            " ; true",
            " > /dev/null",
            " -- --nocapture",
            " -- --test-threads=1",
            " -- --test-threads 1",
            " -- --include-ignored",
            " -- --format=terse",
            " -- --color always",
        ] {
            assert_eq!(verdict(tail), Ok(()), "tail `{tail}` should pass");
        }
    }

    #[test]
    fn a_continued_command_is_read_as_one() {
        let ci = "name: CI\njobs:\n  test:\n    steps:\n      - name: Supply-chain pins\n        \
                  run: |\n          cargo test --locked --test dependency_pin_tests \\\n          \
                  || true\n";
        let reporter = reporter_command(ci, GUARDED_TARGET).expect("located");
        assert!(check_the_reporter_runs_every_test(&reporter, GUARDED_TARGET).is_err());
    }

    #[test]
    fn an_unlocatable_command_fails_closed() {
        let gone = "jobs:\n  test:\n    steps:\n      - name: Unit tests\n        run: cargo \
                    test --locked --lib\n";
        assert!(reporter_command(gone, GUARDED_TARGET).is_err());

        let twice = format!(
            "{}      - name: Again\n        run: cargo test --locked --test \
             dependency_pin_tests\n",
            ci_running("")
        );
        assert!(reporter_command(&twice, GUARDED_TARGET).is_err());

        let commented = "jobs:\n  test:\n    steps:\n      - name: Pins\n        # run: cargo \
                         test --locked --test dependency_pin_tests\n        run: cargo test \
                         --locked --lib\n";
        assert!(reporter_command(commented, GUARDED_TARGET).is_err());
    }
}

// ---------------------------------------------------------------------------
// The job that runs the fidelity regression gate
// ---------------------------------------------------------------------------
//
// `scripts/fidelity-smoke-gate.sh` fails a pull request that breaks an SDK
// smoke check passing on `main`, and its job in `ci.yml` is a required status
// check. Being required does not cover the two cheapest ways to switch it off.
// `continue-on-error: true` on the gate step lets the step fail and the job
// conclude success, and an `if:` that skips the job reports Success to a
// required check. Both are one line in `ci.yml`, and a gate cannot report its
// own removal, so the guard lives here, in the `test` job.

/// What makes a step the fidelity gate: the command, not the step's `name:`.
const FIDELITY_GATE_COMMAND: &str = "scripts/fidelity-smoke-gate.sh";

/// The live `if:` / `continue-on-error:` on the job containing the step that
/// opens at line `step_start`.
///
/// Walks up from the step to its `steps:` key, takes that key's indentation as
/// the job's key indentation, and reads the job's own keys from the line that
/// names the job. `Err` when that shape is not there, rather than reporting an
/// empty, passing list for a layout this reader does not understand.
fn job_conditions(ci: &str, step_start: usize) -> Result<Vec<StepCondition>, String> {
    let lines: Vec<&str> = ci.lines().collect();
    let step_indent = indent_of(lines[step_start]);

    let steps_at = (0..step_start)
        .rev()
        .find(|&at| {
            let bare = without_comment(lines[at]);
            !bare.trim().is_empty() && indent_of(lines[at]) < step_indent
        })
        .filter(|&at| without_comment(lines[at]).trim() == "steps:")
        .ok_or("the gate step is not directly under a `steps:` key")?;
    let key_indent = indent_of(lines[steps_at]);

    let job_at = (0..steps_at)
        .rev()
        .find(|&at| {
            let bare = without_comment(lines[at]);
            !bare.trim().is_empty() && indent_of(lines[at]) < key_indent
        })
        .ok_or("the gate step's `steps:` key belongs to no job")?;
    let job_indent = indent_of(lines[job_at]);

    let mut conditions = Vec::new();
    for line in &lines[job_at + 1..] {
        let bare = without_comment(line);
        if bare.trim().is_empty() {
            continue;
        }
        if indent_of(line) <= job_indent {
            break;
        }
        if indent_of(line) != key_indent {
            continue;
        }
        for key in STEP_CONDITION_KEYS {
            if let Some(value) = bare.trim_start().strip_prefix(&format!("{key}:")) {
                conditions.push(StepCondition {
                    key: (*key).to_string(),
                    value: unquote(value.trim()).to_string(),
                });
            }
        }
    }
    Ok(conditions)
}

/// `Err` naming the first condition that can switch the fidelity gate off.
fn fidelity_gate_is_unconditional(ci: &str) -> Result<(), String> {
    let missing = format!(
        "ci.yml runs no `{FIDELITY_GATE_COMMAND}`, so nothing fails a pull request \
         that breaks an SDK check passing on main. Restore the `fidelity` job's gate \
         step, or point FIDELITY_GATE_COMMAND at wherever it moved."
    );
    let (start, on_step) = step_conditions(ci, FIDELITY_GATE_COMMAND, &missing)?;
    let on_job = job_conditions(ci, start)?;

    for (place, conditions) in [("step", on_step), ("job", on_job)] {
        for StepCondition { key, value } in conditions {
            let holds = if key == "continue-on-error" {
                value == "false"
            } else {
                CONDITIONS_THAT_STILL_RUN.contains(&value.as_str())
            };
            if !holds {
                return Err(format!(
                    "the fidelity gate's {place} carries `{key}: {value}`. A skipped job \
                     satisfies a required check and a discarded step failure concludes the \
                     job success, so either one lets a fidelity regression merge green. \
                     Allowed: `continue-on-error: false`, or `if:` one of {CONDITIONS_THAT_STILL_RUN:?}."
                ));
            }
        }
    }
    Ok(())
}

#[test]
fn the_fidelity_gate_cannot_be_switched_off_in_ci_yml() {
    let ci = read(&repo_root().join(".github/workflows/ci.yml"));
    if let Err(why) = fidelity_gate_is_unconditional(&ci) {
        panic!("{why}");
    }
}

#[cfg(test)]
mod fidelity_gate_controls {
    use super::*;

    /// A job in the shape `ci.yml` uses, with `job` added to the job's keys and
    /// `step` to the gate step's.
    fn ci_with(job: &str, step: &str) -> String {
        format!(
            "jobs:\n  fidelity:\n    name: gate\n{job}    runs-on: ubuntu-latest\n    steps:\n      \
             - uses: actions/checkout@v4\n      - name: Fidelity regression gate\n{step}        \
             run: scripts/fidelity-smoke-gate.sh\n  other:\n    if: false\n    steps:\n      \
             - run: true\n"
        )
    }

    #[test]
    fn an_unconditional_job_passes() {
        // The `if: false` on the neighbouring job must not be read as this one's.
        assert_eq!(fidelity_gate_is_unconditional(&ci_with("", "")), Ok(()));
    }

    #[test]
    fn skipping_the_job_is_caught() {
        for off in ["    if: false\n", "    if: github.event_name == 'push'\n"] {
            let verdict = fidelity_gate_is_unconditional(&ci_with(off, ""));
            assert!(
                verdict.is_err_and(|why| why.contains("job carries `if:")),
                "{off:?} passed"
            );
        }
    }

    #[test]
    fn discarding_the_verdict_is_caught_on_the_job_and_the_step() {
        let on_job = fidelity_gate_is_unconditional(&ci_with("    continue-on-error: true\n", ""));
        assert!(on_job.is_err_and(|why| why.contains("job carries `continue-on-error: true`")));
        let on_step =
            fidelity_gate_is_unconditional(&ci_with("", "        continue-on-error: true\n"));
        assert!(on_step.is_err_and(|why| why.contains("step carries `continue-on-error: true`")));
    }

    #[test]
    fn skipping_the_step_is_caught() {
        let verdict = fidelity_gate_is_unconditional(&ci_with("", "        if: false\n"));
        assert!(verdict.is_err_and(|why| why.contains("step carries `if: false`")));
    }

    #[test]
    fn conditions_that_still_run_pass() {
        assert_eq!(
            fidelity_gate_is_unconditional(&ci_with("    if: always()\n", "")),
            Ok(())
        );
        let strict = ci_with(
            "    continue-on-error: false\n",
            "        continue-on-error: false\n",
        );
        assert_eq!(fidelity_gate_is_unconditional(&strict), Ok(()));
    }

    #[test]
    fn a_missing_gate_step_fails_closed() {
        let ci = ci_with("", "").replace(FIDELITY_GATE_COMMAND, "true");
        assert!(fidelity_gate_is_unconditional(&ci).is_err());
    }
}
