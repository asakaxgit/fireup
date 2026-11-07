//! Every file in `.github/workflows/` has to be something GitHub can actually
//! read.
//!
//! This has now bitten the repository twice, both times the same way and both
//! times invisibly:
//!
//! 1. `run: cargo test --lib local::` — an unquoted scalar ending in `::`.
//!    YAML reads the trailing colon as the start of a mapping value, the
//!    document fails to parse, and GitHub rejects the whole file.
//! 2. GitHub's response to an unreadable workflow file is not "skip it". It
//!    manufactures a run for the *push* event even though the file declares
//!    only `workflow_dispatch`, fails it with zero jobs, and labels it by
//!    **path** instead of by `name:` — because it never got far enough to read
//!    `name:`. So `main` carried a permanent red X while the suite the file was
//!    supposed to guard never ran once.
//!
//! What made it survive so long is that a *manual* `workflow_dispatch` of the
//! same file is dispatched from the Actions list, which is populated from the
//! last version GitHub could parse — so the maintainer dispatching it by hand
//! sees green while every push sees red.
//!
//! Nothing in the Rust suite looked at these files, and the repository's CI is
//! manual-only, so no automatic check looked at them either. This test closes
//! that: it reads the workflow directory from disk, needs no server, no network
//! and no `gh`, and fails in `cargo test` in milliseconds.
//!
//! `actionlint` is the richer tool for this job — it validates against GitHub's
//! real schema, checks action references and expression syntax — and belongs in
//! CI once the repository has a workflow that runs automatically. This test is
//! the floor, not the ceiling: it catches the class of breakage that has
//! actually happened here, with no new tooling to install.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workflows_dir() -> PathBuf {
    repo_root().join(".github/workflows")
}

/// Every `.yml`/`.yaml` under `.github/workflows`, sorted so failures name the
/// same file in the same order on every machine.
fn workflow_files() -> Vec<PathBuf> {
    let dir = workflows_dir();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("reading {}: {error}", dir.display()));

    let mut files: Vec<PathBuf> = entries
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("yml") | Some("yaml")
            )
        })
        .collect();
    files.sort();
    files
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("reading {path:?}: {error}"))
}

fn label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<unnamed>")
        .to_string()
}

/// Look a key up in a workflow mapping.
///
/// `on` is the awkward one: under the YAML 1.1 resolution rules that GitHub's
/// own parser and PyYAML both use, a bare `on` is the boolean `true`, so the
/// key can arrive either spelling depending on the parser. Accept both rather
/// than depend on which one `serde_yaml` happens to apply.
fn get<'a>(map: &'a serde_yaml::Mapping, key: &str) -> Option<&'a serde_yaml::Value> {
    map.get(serde_yaml::Value::String(key.to_string()))
        .or_else(|| match key {
            "on" => map.get(serde_yaml::Value::Bool(true)),
            _ => None,
        })
}

fn parse(path: &Path) -> serde_yaml::Mapping {
    let document: serde_yaml::Value = serde_yaml::from_str(&read(path)).unwrap_or_else(|error| {
        panic!(
            "{} is not valid YAML, so GitHub will reject it and fail every push \
             with a zero-job run named by path: {error}",
            label(path)
        )
    });

    document
        .as_mapping()
        .unwrap_or_else(|| panic!("{} must be a YAML mapping at the top level", label(path)))
        .clone()
}

#[test]
fn the_workflow_directory_is_not_empty() {
    // Guards against the test quietly passing because it is looking at the
    // wrong directory, or because somebody moved the workflows elsewhere.
    let files = workflow_files();
    assert!(
        !files.is_empty(),
        "no workflow files found in {}",
        workflows_dir().display()
    );
}

#[test]
fn every_workflow_file_is_valid_yaml() {
    for path in workflow_files() {
        parse(&path);
    }
}

/// A workflow with no `name:` is the symptom that made the original bug hard to
/// read: GitHub falls back to the file path, so the Actions list and the run
/// titles stop matching what anybody calls the workflow.
#[test]
fn every_workflow_has_a_name() {
    for path in workflow_files() {
        let workflow = parse(&path);
        let name = get(&workflow, "name")
            .and_then(|value| value.as_str())
            .unwrap_or_else(|| {
                panic!(
                    "{} has no `name:`, so GitHub lists it by path instead",
                    label(&path)
                )
            });
        assert!(
            !name.trim().is_empty(),
            "{} has an empty `name:`",
            label(&path)
        );
    }
}

/// `on:` and `jobs:` are what make the file a workflow rather than a note. A
/// file missing either is accepted by a plain YAML parser and rejected by
/// GitHub, which is exactly the gap this suite exists to cover.
#[test]
fn every_workflow_declares_triggers_and_jobs() {
    for path in workflow_files() {
        let workflow = parse(&path);
        let name = label(&path);

        let triggers = get(&workflow, "on")
            .unwrap_or_else(|| panic!("{name} has no `on:` section, so nothing can trigger it"));
        assert!(
            !triggers.is_null(),
            "{name} has an empty `on:` section, so nothing can trigger it"
        );

        let jobs = get(&workflow, "jobs")
            .and_then(|value| value.as_mapping())
            .unwrap_or_else(|| panic!("{name} has no `jobs:` mapping"));
        assert!(!jobs.is_empty(), "{name} declares no jobs");

        for (id, job) in jobs {
            let id = id.as_str().unwrap_or("<non-string job id>");
            let job = job
                .as_mapping()
                .unwrap_or_else(|| panic!("{name}: job `{id}` is not a mapping"));

            // A job that reuses a workflow has `uses:` and no steps of its own;
            // anything else needs a runner and at least one step.
            if get(job, "uses").is_some() {
                continue;
            }

            assert!(
                get(job, "runs-on").is_some(),
                "{name}: job `{id}` has no `runs-on:`"
            );

            let steps = get(job, "steps")
                .and_then(|value| value.as_sequence())
                .unwrap_or_else(|| panic!("{name}: job `{id}` has no `steps:` list"));
            assert!(!steps.is_empty(), "{name}: job `{id}` has no steps");
        }
    }
}

/// The negative control, kept permanently rather than run once by hand.
///
/// This is the exact step that broke `local_firestore.yml`, verbatim. If a
/// future YAML parser accepts it, the tests above stop protecting anything and
/// this one fails to say so — which is the only way to notice, because the
/// workflow files on disk are (correctly) all parseable.
#[test]
fn an_unquoted_trailing_double_colon_still_fails_to_parse() {
    const BROKEN: &str = "\
jobs:
  rust-tests:
    runs-on: ubuntu-latest
    steps:
      - name: Data plane unit tests
        run: cargo test --lib local::
";

    let result = serde_yaml::from_str::<serde_yaml::Value>(BROKEN);
    assert!(
        result.is_err(),
        "`run: cargo test --lib local::` parsed cleanly — the trailing-colon \
         breakage that took down CI twice would no longer be caught. Quoting \
         the scalar is still correct, but these tests need a check that bites."
    );

    // And the quoted form — the fix — has to keep parsing, so the test is
    // asserting something about the colon and not about the whole snippet.
    const FIXED: &str = "\
jobs:
  rust-tests:
    runs-on: ubuntu-latest
    steps:
      - name: Data plane unit tests
        run: \"cargo test --lib local::\"
";
    serde_yaml::from_str::<serde_yaml::Value>(FIXED).expect("the quoted form must parse");
}
