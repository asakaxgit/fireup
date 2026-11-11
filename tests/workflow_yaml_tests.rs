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
//! Nothing in the Rust suite looked at these files when this was written, so
//! nothing caught it. This test closes that: it reads the workflow directory
//! from disk, needs no server, no network and no `gh`, and fails in
//! `cargo test` in milliseconds.
//!
//! `actionlint` is the richer tool for this job — it validates against GitHub's
//! real schema, checks action references and expression syntax — and PR #78
//! now runs it from `actionlint.yml`, the one workflow in this repository that
//! triggers automatically. This test remains the floor under it: it runs
//! locally before the push, with no network and no third-party binary, and it
//! keeps a permanent negative control for the exact breakage that happened
//! here.
//!
//! # Why `saphyr` and not `serde_yaml`
//!
//! `serde_yaml` is deprecated (`0.9.34+deprecated` is its last release) and
//! reaches `unsafe-libyaml` — libyaml's C machine-translated through c2rust —
//! whose repository was archived read-only in March 2024. That is
//! RUSTSEC-2024-0449, an unmaintained advisory with no patched version, and a
//! bug found in machine-translated `unsafe` Rust has no upstream to fix it.
//!
//! Depending on it for a *test* also had a second cost the first version of
//! this file did not anticipate: it put the one advisory in the tree that
//! `deny.toml` was configured not to report (#83), because the crate
//! arrives transitively. `saphyr` is the maintained YAML 1.2 parser, pure safe
//! Rust, and it rejects the trailing-`::` document exactly the way libyaml did
//! — which the negative control at the bottom of this file asserts rather than
//! assumes.
//!
//! One deliberate consequence of the parser swap: `saphyr` applies the YAML 1.2
//! core schema, where a bare `on` is the string `"on"`. GitHub's parser applies
//! YAML 1.1, where it is the boolean `true`. `get` below accepts both spellings
//! so the suite does not depend on which rules the parser underneath it uses.

use saphyr::{LoadableYamlNode, Mapping, Scalar, Yaml};
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
/// own parser and PyYAML both use, a bare `on` is the boolean `true`, while
/// `saphyr` follows the YAML 1.2 core schema and keeps it the string `"on"`.
/// Accept both rather than depend on which rules the parser applies.
fn get<'a, 'text>(map: &'a Mapping<'text>, key: &'text str) -> Option<&'a Yaml<'text>> {
    map.get(&Yaml::value_from_str(key)).or_else(|| match key {
        "on" => map.get(&Yaml::Value(Scalar::Boolean(true))),
        _ => None,
    })
}

/// The workflow's top-level mapping, borrowed from `text`.
///
/// `saphyr` nodes borrow their scalars out of the input, so the caller holds
/// the file contents for as long as it reads the parsed document. That is also
/// why this takes the text rather than the path: there is nowhere for a
/// self-contained `parse(&path)` to keep the string.
fn parse<'text>(text: &'text str, path: &Path) -> Mapping<'text> {
    let documents = Yaml::load_from_str(text).unwrap_or_else(|error| {
        panic!(
            "{} is not valid YAML, so GitHub will reject it and fail every push \
             with a zero-job run named by path: {error}",
            label(path)
        )
    });

    // GitHub reads one workflow per file. A stream of several documents parses
    // cleanly here but is not a workflow, and an empty file parses to no
    // documents at all — the old `serde_yaml::from_str` rejected both for us,
    // so say so explicitly now that the loader returns every document.
    assert_eq!(
        documents.len(),
        1,
        "{} holds {} YAML documents; GitHub reads exactly one workflow per file",
        label(path),
        documents.len()
    );

    match documents.into_iter().next() {
        Some(Yaml::Mapping(map)) => map,
        _ => panic!("{} must be a YAML mapping at the top level", label(path)),
    }
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
        let text = read(&path);
        parse(&text, &path);
    }
}

/// A workflow with no `name:` is the symptom that made the original bug hard to
/// read: GitHub falls back to the file path, so the Actions list and the run
/// titles stop matching what anybody calls the workflow.
#[test]
fn every_workflow_has_a_name() {
    for path in workflow_files() {
        let text = read(&path);
        let workflow = parse(&text, &path);
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
        let text = read(&path);
        let workflow = parse(&text, &path);
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

        for (id, job) in jobs.iter() {
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
/// This is the exact step that broke `local_firestore.yml`, verbatim. If the
/// YAML parser underneath this suite ever accepts it, the tests above stop
/// protecting anything and this one fails to say so — which is the only way to
/// notice, because the workflow files on disk are (correctly) all parseable.
///
/// It is also what made the `serde_yaml` → `saphyr` swap checkable rather than
/// hopeful: a replacement parser that shrugged at this document would have been
/// the wrong replacement, however well maintained.
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

    let result = Yaml::load_from_str(BROKEN);
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
    Yaml::load_from_str(FIXED).expect("the quoted form must parse");
}

/// `on` arriving as a string under YAML 1.2 and as a boolean under YAML 1.1 is
/// a real divergence between this suite's parser and GitHub's, so `get` handles
/// both spellings. Without this, the day a parser bump changes which one it
/// resolves to, `every_workflow_declares_triggers_and_jobs` would stop finding
/// `on:` at all — and would fail with "nothing can trigger it" on a workflow
/// that is perfectly fine.
#[test]
fn the_trigger_key_is_found_under_either_yaml_version_spelling() {
    const BARE: &str = "on:\n  push:\n    branches: [main]\n";
    let bare = Yaml::load_from_str(BARE).expect("parses");
    let bare = bare[0].as_mapping().expect("mapping");
    assert!(
        get(bare, "on").is_some(),
        "bare `on:` not found; saphyr resolved it to something `get` does not try"
    );

    // Quoting it is the other legal spelling, and some repositories write it
    // that way precisely because of the 1.1 boolean rule.
    const QUOTED: &str = "\"on\":\n  push:\n    branches: [main]\n";
    let quoted = Yaml::load_from_str(QUOTED).expect("parses");
    let quoted = quoted[0].as_mapping().expect("mapping");
    assert!(get(quoted, "on").is_some(), "quoted `\"on\":` not found");
}

/// The other half of a mutual pair with `public_repo_hygiene_tests`.
///
/// That target keeps the internal issue tracker out of a public tree: no ticket
/// ids, no links to the tracker's host. Like every target in the `test` job of
/// `ci.yml` it only runs because a step names it, and a guard that never runs
/// reports nothing while still looking like a guard — so it cannot be the thing
/// that checks it is still named.
///
/// It asserts `workflow_yaml_tests` is named here; this asserts the reverse.
/// Deleting either step leaves the other one running to say so.
#[test]
fn ci_still_names_the_public_repo_hygiene_target() {
    let ci = read(&workflows_dir().join("ci.yml"));
    assert!(
        ci.contains("--test public_repo_hygiene_tests"),
        "ci.yml no longer names `public_repo_hygiene_tests`, so nothing checks \
         that the published tree stays free of internal tracker ids and links. \
         Restore the step in the `test` job."
    );
}
