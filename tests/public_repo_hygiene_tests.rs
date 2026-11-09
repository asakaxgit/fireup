//! This repository is public. Our internal issue tracker is not.
//!
//! For a year the two were written as if they were one thing: 467 references to
//! internal ticket ids across 41 tracked files, and ten markdown links whose
//! target was a path on the tracker's own host. To anyone outside the company
//! those links 404 and those ids name nothing — they are a dead pointer sitting
//! where a reader expects provenance. The same change always has a public
//! identity, the pull request that landed it, and that is what belongs in a
//! file anybody can clone.
//!
//! So the references were replaced rather than deleted: the ticket id became
//! the pull request number, and the tracker links became `github.com` pull
//! request URLs. Provenance survives; the tracker stops being named.
//!
//! Nothing stops it coming back. A tracker id in a commit message, a branch
//! name or a PR title is habit, and habit is what put 467 of them in the tree.
//! Prose in a contributing guide does not fail a build, so this target does.
//!
//! # Why this file holds no literal of the thing it looks for
//!
//! The scan reads every tracked file, and this file is tracked. A literal
//! sample written here would be found by the scan and reported as a finding in
//! this file — and then there would be no way to tell a real finding from the
//! guard's own source. Both needles are therefore assembled at run time from
//! fragments, and the negative controls build their samples the same way. The
//! alternative — exempting this file from its own scan — is a hole: it would
//! also exempt a careless edit.
//!
//! # The two exemptions, and when they go
//!
//! `tests/dependency_pin_tests.rs` and `tests/fidelity_matrix_tests.rs` still
//! carry 148 references between them. Both are under an open three-layer pull
//! request stack (#106 → #107 → #108) that adds roughly 1,400 lines to them.
//! Sweeping them here would conflict with every layer, so they are exempt by
//! name until that stack lands.
//!
//! The exemption is exact rather than permissive:
//! `every_exempt_file_still_needs_its_exemption` fails when an exempt file
//! stops containing a reference, so the entry cannot outlive the cleanup and
//! sit there silently widening the gate. That test is also the live positive
//! control for the scanner — it proves the needle matches real bytes in the
//! real tree, so "no findings" cannot come from a detector that matches
//! nothing.
//!
//! # What this cannot reach
//!
//! Commit messages and branch names. 149 of the 298 commits on `main` carry a
//! ticket id in the subject line, and seven remote heads are named after one.
//! Rewriting published history would break every clone and fork for a
//! cosmetic gain, so those stay. This target guards the tree, which is what a
//! reader actually opens.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Files still allowed to name the internal tracker, with the reason in the
/// module documentation above.
///
/// Adding an entry widens the gate, so the count is pinned below: a new
/// exemption has to be written in two places, which is enough to make it a
/// decision rather than a reflex.
const EXEMPT: [&str; 2] = [
    "tests/dependency_pin_tests.rs",
    "tests/fidelity_matrix_tests.rs",
];

/// The internal ticket prefix, assembled rather than written out.
fn ticket_prefix() -> String {
    format!("{}{}", "SAK", "-")
}

/// The tracker's own issue path, as it appeared in the markdown links this
/// target exists to keep out. Assembled for the same reason.
fn tracker_issue_path() -> String {
    format!("/{}/issues/", "SAK")
}

/// Every file git tracks, as a repository-relative path.
///
/// `git ls-files` rather than a directory walk on purpose. A walk needs a list
/// of directories to skip — `target/`, `.git/`, anything vendored later — and
/// that list is a hole: the day somebody adds a directory to it, the scan stops
/// looking there and nothing says so. The tracked set is also exactly what we
/// publish, which is the question being asked.
///
/// If git is unavailable this panics rather than returning an empty list. An
/// empty list would make every assertion below pass.
fn tracked_files() -> Vec<String> {
    let root = repo_root();
    let output = Command::new("git")
        .arg("ls-files")
        .arg("-z")
        .current_dir(&root)
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "running `git ls-files` in {}: {error}. This target needs git to \
                 know which files are published; it will not fall back to a \
                 directory walk, because a walk that silently skips a directory \
                 passes for the wrong reason.",
                root.display()
            )
        });

    assert!(
        output.status.success(),
        "`git ls-files` failed in {}: {}",
        root.display(),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect()
}

/// Read a tracked file as text. A few fixtures are binary export payloads;
/// lossy decoding keeps them in the scan instead of skipping them, because a
/// skip is a place a reference could hide.
fn read_lossy(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|error| panic!("reading {path:?}: {error}"));
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Every internal ticket reference in `text`: the prefix followed by at least
/// one digit.
///
/// The digit requirement is what separates a reference from the company's own
/// name appearing in a hyphenated word, and from the prefix used as a bare
/// word in prose about this rule.
fn ticket_references(text: &str) -> Vec<String> {
    let needle = ticket_prefix();
    let mut found = Vec::new();

    for (start, _) in text.match_indices(&needle) {
        let rest = &text[start + needle.len()..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            found.push(format!("{needle}{digits}"));
        }
    }

    found
}

/// Every link into the internal tracker in `text`.
fn tracker_links(text: &str) -> Vec<String> {
    let needle = tracker_issue_path();
    text.match_indices(&needle)
        .map(|(start, _)| {
            let end = (start + needle.len() + 16).min(text.len());
            text[start..end].to_string()
        })
        .collect()
}

#[test]
fn the_tracked_file_list_is_not_empty() {
    // Anti-vacuity. Every assertion below iterates this list, so an empty one
    // reads as a clean repository.
    let files = tracked_files();
    assert!(
        files.len() > 100,
        "`git ls-files` returned {} files in {}, which is too few to be this \
         repository — the scan below would pass by looking at nothing",
        files.len(),
        repo_root().display()
    );
}

#[test]
fn the_exemption_list_has_not_grown() {
    assert_eq!(
        EXEMPT.len(),
        2,
        "the exemption list is the one way to widen this gate. Changing its \
         length means a tracked file is now allowed to name the internal \
         tracker; say which file and why in the module documentation, and \
         update this count deliberately."
    );
}

#[test]
fn no_tracked_file_references_an_internal_ticket() {
    let root = repo_root();
    let mut findings: Vec<String> = Vec::new();

    for relative in tracked_files() {
        if EXEMPT.contains(&relative.as_str()) {
            continue;
        }

        let text = read_lossy(&root.join(&relative));
        for reference in ticket_references(&text) {
            findings.push(format!("{relative}: {reference}"));
        }
    }

    assert!(
        findings.is_empty(),
        "{} internal tracker reference(s) in the published tree:\n  {}\n\n\
         This repository is public and the tracker is not, so an id here names \
         nothing a reader can open. Use the pull request number instead — that \
         is the same change's public identity. If a reference genuinely has no \
         pull request, drop it and let the sentence stand on the measurement it \
         already describes.",
        findings.len(),
        findings.join("\n  ")
    );
}

#[test]
fn no_tracked_file_links_to_the_internal_tracker() {
    let root = repo_root();
    let mut findings: Vec<String> = Vec::new();

    // No exemptions on this one. A link is worse than a bare id: it renders as
    // a working link and resolves to nothing outside the company.
    for relative in tracked_files() {
        let text = read_lossy(&root.join(&relative));
        for link in tracker_links(&text) {
            findings.push(format!("{relative}: {link}"));
        }
    }

    assert!(
        findings.is_empty(),
        "{} link(s) into the internal tracker in the published tree:\n  {}\n\n\
         Link to the pull request on github.com instead.",
        findings.len(),
        findings.join("\n  ")
    );
}

/// The exemptions are exact, not permissive: an entry that is no longer needed
/// has to go, or it sits there allowing what it was never meant to allow.
///
/// This is also the scanner's live positive control. It asserts the needle
/// matches real bytes in a real tracked file, which is the only thing standing
/// between "no findings" and a detector that matches nothing at all.
#[test]
fn every_exempt_file_still_needs_its_exemption() {
    let root = repo_root();
    let tracked = tracked_files();

    for exempt in EXEMPT {
        assert!(
            tracked.contains(&exempt.to_string()),
            "{exempt} is exempt from the tracker-reference scan but is not a \
             tracked file; delete the entry"
        );

        let text = read_lossy(&root.join(exempt));
        let references = ticket_references(&text);
        assert!(
            !references.is_empty(),
            "{exempt} is exempt from the tracker-reference scan but no longer \
             contains one. Delete the entry from EXEMPT and lower the count in \
             `the_exemption_list_has_not_grown` — an exemption nobody needs is \
             a gate nobody is watching."
        );
    }
}

/// The negative controls. Both needles are assembled here the same way the
/// detectors assemble them, for the reason in the module documentation.
#[test]
fn the_detectors_find_a_reference_and_ignore_everything_else() {
    let prefix = ticket_prefix();

    // Found.
    let reference = format!("{prefix}257");
    assert_eq!(
        ticket_references(&format!("see {reference} for the decision")),
        vec![reference.clone()],
        "the scan has to find a reference in the middle of a sentence"
    );
    assert_eq!(
        ticket_references(&format!("({reference})")).len(),
        1,
        "parenthesised is the commonest shape in this repository"
    );

    // Not found: the prefix with no number is how this rule is written about,
    // including in the documentation above.
    assert!(
        ticket_references(&format!("the {} prefix followed by a number", "SAK")).is_empty(),
        "the prefix as a bare word is prose about the rule, not a reference"
    );
    assert!(
        ticket_references(&format!("{prefix} 257")).is_empty(),
        "a space between the prefix and the number is not the id format"
    );
    assert!(
        ticket_references("a pull request number like #257 is the public form").is_empty(),
        "the replacement form must not be reported as a finding"
    );

    // The link detector.
    let link = format!("https://tracker.example{}257", tracker_issue_path());
    assert_eq!(
        tracker_links(&format!("[#257]({link})")).len(),
        1,
        "the scan has to find a tracker path inside a markdown link target"
    );
    assert!(
        tracker_links("https://github.com/asakaxgit/fireup/pull/257").is_empty(),
        "a github.com pull request URL is the replacement, not a finding"
    );
}

/// Half of a mutual pair with `workflow_yaml_tests`.
///
/// Every cargo invocation in the `test` job of `ci.yml` names its target, so a
/// test target no step lists never runs — and a guard that never runs reports
/// nothing while looking exactly like a guard. This target cannot assert that
/// `ci.yml` still names *itself*: if the step is deleted, the assertion goes
/// with it. So each of the pair asserts the other is named, and deleting either
/// step is caught by the one still running.
///
/// The residual hole, stated rather than implied: deleting *both* steps in one
/// edit is caught by neither. That edit removes two named targets from the
/// workflow at once, which is a much louder diff than dropping one line.
#[test]
fn ci_still_names_the_workflow_yaml_target() {
    let ci = read_lossy(&repo_root().join(".github/workflows/ci.yml"));
    assert!(
        ci.contains("--test workflow_yaml_tests"),
        "ci.yml no longer names `workflow_yaml_tests`, so the workflow guards \
         stopped running — including the one asserting that *this* target is \
         still named. Restore the step in the `test` job."
    );
}
