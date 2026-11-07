//! Supply-chain facts that `Cargo.toml` can only ask for, and that
//! `Cargo.lock` and `deny.toml` are the only places to actually check.
//!
//! Each one is the kind of thing a routine `cargo update`, a well-meant version
//! bump, or a one-word config edit can undo without anything going red.
//!
//! 1. **One prost.** `src/local/` (the gRPC data plane, from `tonic-build`) and
//!    the managed-export parser (from `prost-build`) exchange generated types.
//!    If the two ever resolve to different prost majors, a `prost::Message` from
//!    one is not a `prost::Message` to the other, and the failure arrives as a
//!    wall of trait-mismatch errors far away from the version that caused it.
//!    This is why tonic was pinned at 0.11 for as long as it was.
//!
//! 2. **h2 at or above the patched version.** `h2` below 0.4.16 accepts and
//!    queues unbounded empty DATA frames — RUSTSEC-2026-0258, which the repo
//!    carried as an accepted exception in `deny.toml` until the migration landed.
//!    CI's `cargo deny` job enforces this properly, but it needs the network and
//!    a pinned third-party binary. This test needs neither, so a local
//!    `cargo test` catches a reintroduction before the push.
//!
//!    (1) and (2) come out of SAK-220, the tonic 0.11 → 0.13 / prost 0.12 →
//!    0.13 migration.
//!
//! 3. **`[advisories] unmaintained` stays at `"all"`.** This one is not about a
//!    version, it is about the *scope* of the gate, and it is here because the
//!    narrower setting already cost us a real miss. SAK-218 wrote
//!    `unmaintained = "workspace"`, which only reports unmaintained advisories
//!    for direct dependencies; RUSTSEC-2024-0449 (`unsafe-libyaml`, archived
//!    read-only, reached through a dev dependency on `serde_yaml`) was
//!    transitive, so it fell outside the scope and `cargo deny` printed
//!    `advisories ok` for months. SAK-225 raised the scope and removed the
//!    crate.
//!
//!    A narrowed scope is the worst kind of regression for a gate: it makes CI
//!    *greener*, so nothing complains. Nobody would notice the word changing
//!    back. Note that `cargo deny` itself cannot catch this — the setting is
//!    its own input — which is why it needs a check outside the tool.
//!
//! 4. **`[advisories] maximum-db-staleness` stays at a week or less.** Also a
//!    scope question, and the same blind spot as (3) for the same reason.
//!    Without the setting cargo-deny has no age limit on the advisory database
//!    at all: given `--offline` or `--frozen` it falls back to whatever is
//!    cached in `$CARGO_HOME/advisory-dbs` and reports `advisories ok` with no
//!    warning, however old that copy is. Measured on this config before the
//!    line existed (SAK-228): a 30-day-old database, `--offline`, exit 0,
//!    silent. Every other way of not having a fresh database already fails
//!    closed, so a stale cache was the one route to a green gate that had not
//!    heard of last week's advisories.
//!
//!    Deleting the line reads as tidying up a redundant default, which is what
//!    makes it worth a test. CI does not take the `--offline` path today — the
//!    `deny` job passes only `--locked` — so nothing in CI would go red if the
//!    line went away, and nothing in CI would go red the day someone adds
//!    `actions/cache` for `~/.cargo` or `--frozen` for reproducibility either.
//!
//! Reading `Cargo.lock` as text rather than running `cargo metadata` is
//! deliberate: it keeps the test free of a toml dependency and of a subprocess,
//! and the two fields it wants (`name`, `version`) are the first two lines of
//! every `[[package]]` block in the v3/v4 lock format.
//!
//! The checks are written as functions over parsed text, not as assertions over
//! the real file, so that the "and it really does fail" half can be committed
//! too — see the `negative_controls` module at the bottom. A hand-edited
//! `Cargo.lock` is not a usable mutation here: cargo re-resolves an inconsistent
//! lock file before it runs the test, so breaking the real file on disk proves
//! nothing.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// `h2` below this accepts unbounded empty DATA frames (RUSTSEC-2026-0258).
const H2_PATCHED: (u64, u64, u64) = (0, 4, 16);

type Versions = BTreeMap<String, Vec<String>>;

/// Every `(name, version)` pair in a lock file, grouped by name.
///
/// A crate with two entries is a crate resolved to two versions, which is what
/// the prost check below is about.
fn parse_lock(lock: &str) -> Versions {
    let mut versions: Versions = BTreeMap::new();
    for block in lock.split("[[package]]").skip(1) {
        let mut name = None;
        let mut version = None;
        for line in block.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().trim_matches('"').to_string();
            match key.trim() {
                // First occurrence only: a `[[package]]` block can carry a
                // `dependencies` list whose entries are not `key = value`, but
                // a `[metadata]` tail could still contain either word.
                "name" if name.is_none() => name = Some(value),
                "version" if version.is_none() => version = Some(value),
                _ => {}
            }
        }
        if let (Some(name), Some(version)) = (name, version) {
            versions.entry(name).or_default().push(version);
        }
    }
    versions
}

/// `major.minor` of a semver string — the part that has to agree across the
/// prost family, since prost makes breaking changes on the minor.
fn minor_line(version: &str) -> String {
    let mut parts = version.split('.');
    let major = parts.next().unwrap_or_default();
    let minor = parts.next().unwrap_or_default();
    format!("{major}.{minor}")
}

/// `(major, minor, patch)`, ignoring any pre-release or build suffix, so
/// `0.4.16-rc.1` compares as 0.4.16 for "is the fix in here".
fn triple(version: &str) -> (u64, u64, u64) {
    let parts: Vec<u64> = version
        .split('.')
        .map(|part| {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().unwrap_or(0)
        })
        .collect();
    (
        parts.first().copied().unwrap_or(0),
        parts.get(1).copied().unwrap_or(0),
        parts.get(2).copied().unwrap_or(0),
    )
}

/// `Err` with the message a developer needs if the prost family has split.
fn check_prost_is_one_version(versions: &Versions) -> Result<(), String> {
    let Some(prost) = versions.get("prost") else {
        return Err("prost is a direct dependency but is not in the lock file".into());
    };
    if prost.len() != 1 {
        return Err(format!(
            "prost resolved to {prost:?}. The gRPC data plane (tonic-build) and \
             the managed-export parser (prost-build) exchange generated types, \
             so they have to share one prost. Whatever was just bumped needs to \
             move tonic, tonic-build, prost, prost-types and prost-build \
             together — see the comment on `tonic` in Cargo.toml."
        ));
    }

    for crate_name in ["prost-types", "prost-build"] {
        let Some(resolved) = versions.get(crate_name) else {
            return Err(format!("{crate_name} is not in the lock file"));
        };
        if resolved.len() != 1 {
            return Err(format!(
                "{crate_name} resolved to {resolved:?}; it has to track prost exactly"
            ));
        }
        if minor_line(&resolved[0]) != minor_line(&prost[0]) {
            return Err(format!(
                "{crate_name} {} does not match prost {}. prost breaks its API \
                 on the minor version, so these move as a set.",
                resolved[0], prost[0]
            ));
        }
    }
    Ok(())
}

/// `Err` if any resolved `h2` is below the version that patched the advisory.
fn check_h2_is_patched(versions: &Versions) -> Result<(), String> {
    let Some(resolved) = versions.get("h2") else {
        return Err("h2 reaches the tree via tonic -> hyper but is not in the lock file".into());
    };
    for version in resolved {
        if triple(version) < H2_PATCHED {
            return Err(format!(
                "h2 {version} is below 0.4.16, which reintroduces \
                 RUSTSEC-2026-0258 (unbounded empty DATA frames). h2 0.4.x needs \
                 hyper 1.x, which needs tonic >= 0.12 — a downgrade here means \
                 the tonic pin moved back."
            ));
        }
    }
    Ok(())
}

/// `Err` if `deny.toml` carries a live `ignore` entry for the h2 advisory.
///
/// The prose above `ignore` names the advisory on purpose, as history, so only
/// an entry outside a comment counts as a live acceptance.
fn check_advisory_not_reaccepted(deny: &str) -> Result<(), String> {
    for line in deny.lines() {
        let code = line.split('#').next().unwrap_or("");
        if code.contains("RUSTSEC-2026-0258") {
            return Err(format!(
                "deny.toml accepts RUSTSEC-2026-0258 again: {}\n\
                 The tonic 0.13 / prost 0.13 migration (SAK-220) fixed it. If \
                 the advisory is back, fix the dependency rather than \
                 re-accepting it.",
                line.trim()
            ));
        }
    }
    Ok(())
}

/// `Err` unless `[advisories] unmaintained` is set to `"all"`.
///
/// Narrowing this back to `"workspace"` is a one-word edit that makes the
/// `cargo deny` job report *less* and therefore look healthier, so it is the
/// regression least likely to be noticed by anyone watching CI.
///
/// Comments are stripped before matching, for the same reason as
/// `check_advisory_not_reaccepted`: the prose above the setting quotes the old
/// value as history, and that has to stay writable.
fn check_unmaintained_scope_is_all(deny: &str) -> Result<(), String> {
    let mut found = None;
    for line in deny.lines() {
        let code = line.split('#').next().unwrap_or("");
        let Some((key, value)) = code.split_once('=') else {
            continue;
        };
        if key.trim() == "unmaintained" {
            found = Some(value.trim().trim_matches('"').to_string());
        }
    }

    match found.as_deref() {
        Some("all") => Ok(()),
        Some(scope) => Err(format!(
            "deny.toml sets `unmaintained = \"{scope}\"`, not \"all\". Only \
             \"all\" covers transitive dependencies, and a transitive crate is \
             how RUSTSEC-2024-0449 (unsafe-libyaml, archived) sat in the tree \
             unreported under \"workspace\" — see SAK-225. Narrowing the scope \
             makes the cargo deny job greener rather than redder, so nothing \
             else will tell you this changed."
        )),
        None => Err("deny.toml has no `[advisories] unmaintained` setting. The \
                     cargo-deny default reports unmaintained advisories for the \
                     workspace only, which is the blind spot SAK-225 closed; \
                     set it to \"all\" explicitly."
            .into()),
    }
}

/// Seconds in an ISO-8601 period, for the subset that can express a week.
///
/// `Err` on a year or a date-part month on purpose. Either one is longer than
/// the bound below, so rejecting them outright is the same answer the length
/// comparison would give, and it keeps the parser out of the ISO-8601 trap
/// where `M` means months before the `T` and minutes after it.
fn staleness_seconds(period: &str) -> Result<u64, String> {
    let body = period
        .strip_prefix('P')
        .ok_or_else(|| format!("`{period}` is not an ISO-8601 period; it must start with `P`"))?;
    let (date, time) = match body.split_once('T') {
        Some((date, time)) => (date, time),
        None => (body, ""),
    };

    let mut seconds: u64 = 0;
    for (part, units) in [
        (date, [('W', 604_800), ('D', 86_400)].as_slice()),
        (time, &[('H', 3_600), ('M', 60), ('S', 1)]),
    ] {
        let mut digits = String::new();
        for character in part.chars() {
            if character.is_ascii_digit() {
                digits.push(character);
                continue;
            }
            let Some((_, unit)) = units.iter().find(|(name, _)| *name == character) else {
                return Err(format!(
                    "`{period}` uses `{character}`, which this check does not read. A year or a \
                     month is longer than the bound either way; use days, weeks, or hours."
                ));
            };
            let count: u64 = digits
                .parse()
                .map_err(|_| format!("`{period}` has a `{character}` with no number before it"))?;
            digits.clear();
            seconds += count * unit;
        }
        if !digits.is_empty() {
            return Err(format!("`{period}` has a trailing `{digits}` with no unit"));
        }
    }
    Ok(seconds)
}

/// The longest advisory database age this repository accepts.
const MAX_DB_STALENESS: u64 = 7 * 86_400;

/// `Err` unless `[advisories] maximum-db-staleness` is set to a week or less.
///
/// Comments are stripped before matching, for the same reason as
/// `check_unmaintained_scope_is_all`.
fn check_db_staleness_is_bounded(deny: &str) -> Result<(), String> {
    let mut found = None;
    for line in deny.lines() {
        let code = line.split('#').next().unwrap_or("");
        let Some((key, value)) = code.split_once('=') else {
            continue;
        };
        if key.trim() == "maximum-db-staleness" {
            found = Some(value.trim().trim_matches('"').to_string());
        }
    }

    let Some(period) = found else {
        return Err(
            "deny.toml has no `[advisories] maximum-db-staleness`. Without it \
                    cargo-deny accepts a cached advisory database of any age, so \
                    `cargo deny --offline check` reports `advisories ok` off a database \
                    from last month without printing a warning — see SAK-228. Set it to \
                    \"P7D\"."
                .into(),
        );
    };

    let seconds = staleness_seconds(&period)?;
    if seconds > MAX_DB_STALENESS {
        return Err(format!(
            "deny.toml allows an advisory database up to `{period}` old, which is more \
             than the week this repository accepts. The point of the bound is that a \
             cached database is the only way this gate goes green without knowing about \
             current advisories; widening it makes the gate quieter, so nothing else \
             will tell you this changed."
        ));
    }
    Ok(())
}

/// `Err` unless `[advisories] unused-ignored-advisory` is set to `"deny"`.
///
/// This setting is the config-side form of the `-D advisory-not-detected` flag
/// in the `deny` job, and the reason it is duplicated here is the reason this
/// check exists: until SAK-229 that flag was the *only* thing making the
/// `ignore` list unable to rot, it lives in exactly one line of
/// `.github/workflows/ci.yml`, and no test in this repository reads that file.
/// Deleting it there left a stale `ignore` entry printing
/// `warning[advisory-not-detected]` and the step exiting 0.
///
/// Comments are stripped before matching, for the same reason as
/// `check_unmaintained_scope_is_all`: the prose in `deny.toml` quotes the
/// setting while explaining it.
fn check_unused_ignored_advisory_is_deny(deny: &str) -> Result<(), String> {
    let mut found = None;
    for line in deny.lines() {
        let code = line.split('#').next().unwrap_or("");
        let Some((key, value)) = code.split_once('=') else {
            continue;
        };
        if key.trim() == "unused-ignored-advisory" {
            found = Some(value.trim().trim_matches('"').to_string());
        }
    }

    match found.as_deref() {
        Some("deny") => Ok(()),
        Some(level) => Err(format!(
            "deny.toml sets `unused-ignored-advisory = \"{level}\"`, not \"deny\". At \
             anything but \"deny\", an `ignore` entry that no longer matches an advisory \
             is a warning and `cargo deny check` exits 0, so an accepted vulnerability \
             outlives its fix with nothing going red — see SAK-229."
        )),
        None => Err(
            "deny.toml has no `[advisories] unused-ignored-advisory`. Without it \
                     cargo-deny only warns about an `ignore` entry that matches nothing, \
                     and the `-D advisory-not-detected` flag in ci.yml is then the only \
                     enforcement of it — in a file no test here reads. Set it to \"deny\"."
                .into(),
        ),
    }
}

fn repo_file(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be committed: {error}", path.display()))
}

fn committed_lock() -> Versions {
    let versions = parse_lock(&repo_file("Cargo.lock"));
    assert!(
        versions.contains_key("fireup"),
        "parsed {} packages out of Cargo.lock but not fireup itself, so the lock \
         format changed under this test and its assertions check nothing",
        versions.len()
    );
    versions
}

#[test]
fn the_committed_lock_has_exactly_one_prost() {
    if let Err(reason) = check_prost_is_one_version(&committed_lock()) {
        panic!("{reason}");
    }
}

#[test]
fn the_committed_lock_has_h2_at_or_above_the_patched_version() {
    if let Err(reason) = check_h2_is_patched(&committed_lock()) {
        panic!("{reason}");
    }
}

#[test]
fn deny_toml_no_longer_accepts_the_h2_advisory() {
    if let Err(reason) = check_advisory_not_reaccepted(&repo_file("deny.toml")) {
        panic!("{reason}");
    }
}

#[test]
fn deny_toml_reports_unmaintained_advisories_for_the_whole_graph() {
    if let Err(reason) = check_unmaintained_scope_is_all(&repo_file("deny.toml")) {
        panic!("{reason}");
    }
}

#[test]
fn deny_toml_refuses_an_advisory_database_older_than_a_week() {
    if let Err(reason) = check_db_staleness_is_bounded(&repo_file("deny.toml")) {
        panic!("{reason}");
    }
}

#[test]
fn deny_toml_fails_on_an_ignore_entry_that_matches_no_advisory() {
    if let Err(reason) = check_unused_ignored_advisory_is_deny(&repo_file("deny.toml")) {
        panic!("{reason}");
    }
}

/// The mutations, committed rather than run once by hand.
///
/// Each one is the regression this file exists to catch, fed to the same
/// function the tests above call. Without these, a refactor that made
/// `check_*` return `Ok(())` unconditionally would leave every test above green.
mod negative_controls {
    use super::*;

    /// A lock file with just the packages these checks look at.
    fn lock_with(packages: &[(&str, &str)]) -> String {
        let mut out = String::from("version = 4\n");
        for (name, version) in packages {
            out.push_str(&format!(
                "\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n\
                 source = \"registry+https://github.com/rust-lang/crates.io-index\"\n"
            ));
        }
        out
    }

    const GOOD: &[(&str, &str)] = &[
        ("fireup", "0.1.0"),
        ("h2", "0.4.20"),
        ("prost", "0.13.5"),
        ("prost-build", "0.13.5"),
        ("prost-types", "0.13.5"),
    ];

    #[test]
    fn the_parser_reads_a_lock_file_at_all() {
        let versions = parse_lock(&lock_with(GOOD));
        assert_eq!(versions["prost"], vec!["0.13.5"]);
        assert_eq!(versions["h2"], vec!["0.4.20"]);
        assert!(check_prost_is_one_version(&versions).is_ok());
        assert!(check_h2_is_patched(&versions).is_ok());
    }

    #[test]
    fn two_prost_versions_fail() {
        let mut packages = GOOD.to_vec();
        packages.push(("prost", "0.12.6"));
        let error = check_prost_is_one_version(&parse_lock(&lock_with(&packages)))
            .expect_err("two prost versions must fail");
        assert!(
            error.contains("0.12.6") && error.contains("0.13.5"),
            "{error}"
        );
    }

    #[test]
    fn a_prost_family_member_left_behind_fails() {
        let mut packages: Vec<_> = GOOD
            .iter()
            .copied()
            .filter(|(name, _)| *name != "prost-build")
            .collect();
        packages.push(("prost-build", "0.12.6"));
        let error = check_prost_is_one_version(&parse_lock(&lock_with(&packages)))
            .expect_err("prost-build on a different minor must fail");
        assert!(error.contains("prost-build 0.12.6"), "{error}");
    }

    #[test]
    fn a_missing_prost_fails_rather_than_passing_vacuously() {
        let packages: Vec<_> = GOOD
            .iter()
            .copied()
            .filter(|(name, _)| *name != "prost")
            .collect();
        assert!(check_prost_is_one_version(&parse_lock(&lock_with(&packages))).is_err());
    }

    #[test]
    fn h2_one_patch_below_the_fix_fails() {
        // 0.4.15 is the real last vulnerable release, not a made-up version.
        let packages: Vec<_> = GOOD
            .iter()
            .copied()
            .map(|(name, version)| {
                if name == "h2" {
                    (name, "0.4.15")
                } else {
                    (name, version)
                }
            })
            .collect();
        let error = check_h2_is_patched(&parse_lock(&lock_with(&packages)))
            .expect_err("h2 0.4.15 must fail");
        assert!(error.contains("RUSTSEC-2026-0258"), "{error}");
    }

    #[test]
    fn the_old_h2_line_fails() {
        // What the tonic 0.11 pin used to resolve to.
        let packages: Vec<_> = GOOD
            .iter()
            .copied()
            .map(|(name, version)| {
                if name == "h2" {
                    (name, "0.3.27")
                } else {
                    (name, version)
                }
            })
            .collect();
        assert!(check_h2_is_patched(&parse_lock(&lock_with(&packages))).is_err());
    }

    #[test]
    fn a_second_h2_that_is_vulnerable_fails_even_beside_a_patched_one() {
        let mut packages = GOOD.to_vec();
        packages.push(("h2", "0.3.27"));
        assert!(
            check_h2_is_patched(&parse_lock(&lock_with(&packages))).is_err(),
            "every resolved h2 has to be patched, not just the newest"
        );
    }

    #[test]
    fn a_missing_h2_fails_rather_than_passing_vacuously() {
        let packages: Vec<_> = GOOD
            .iter()
            .copied()
            .filter(|(name, _)| *name != "h2")
            .collect();
        assert!(check_h2_is_patched(&parse_lock(&lock_with(&packages))).is_err());
    }

    #[test]
    fn a_reaccepted_advisory_fails_but_the_comment_about_it_does_not() {
        let live = r#"
ignore = [
  { id = "RUSTSEC-2026-0258", reason = "it came back" },
]
"#;
        assert!(check_advisory_not_reaccepted(live).is_err());

        // The committed file explains the history in prose. That must stay legal,
        // or the next person cannot write down why the entry is gone.
        let history = r#"
# There was one entry here: RUSTSEC-2026-0258 ("h2 unbounded empty DATA frames").
ignore = []
"#;
        assert!(check_advisory_not_reaccepted(history).is_ok());

        // Including when the comment trails a line of real config.
        let trailing = r#"ignore = []  # was RUSTSEC-2026-0258, fixed by SAK-220"#;
        assert!(check_advisory_not_reaccepted(trailing).is_ok());
    }

    #[test]
    fn the_scope_the_repository_ships_passes() {
        assert!(check_unmaintained_scope_is_all("unmaintained = \"all\"\n").is_ok());
    }

    /// The regression itself: the value SAK-218 shipped and SAK-225 had to
    /// raise.
    #[test]
    fn narrowing_the_unmaintained_scope_to_the_workspace_fails() {
        let error = check_unmaintained_scope_is_all("unmaintained = \"workspace\"\n")
            .expect_err("\"workspace\" must fail");
        assert!(
            error.contains("RUSTSEC-2024-0449") && error.contains("workspace"),
            "{error}"
        );
    }

    /// `"none"` is the other narrowing cargo-deny accepts, and the one that
    /// turns the check off outright.
    #[test]
    fn disabling_the_unmaintained_check_fails() {
        assert!(check_unmaintained_scope_is_all("unmaintained = \"none\"\n").is_err());
    }

    /// Deleting the line is not the same as setting it, and it is the quietest
    /// version of the regression: cargo-deny falls back to a workspace-scoped
    /// default, so the file reads as if nothing was decided.
    #[test]
    fn omitting_the_setting_entirely_fails() {
        let deny = "[advisories]\nyanked = \"deny\"\nignore = []\n";
        assert!(check_unmaintained_scope_is_all(deny).is_err());
    }

    /// The prose above the setting quotes the old value on purpose, as history.
    /// That must not be mistaken for the setting, in either direction.
    #[test]
    fn the_comment_about_the_old_scope_is_not_read_as_the_scope() {
        let deny = "\
# This said `unmaintained = \"workspace\"` first, which hid RUSTSEC-2024-0449.
unmaintained = \"all\"
";
        assert!(check_unmaintained_scope_is_all(deny).is_ok());

        // And a commented-out `\"all\"` above a live `\"workspace\"` must not
        // rescue it.
        let deny = "\
# unmaintained = \"all\"
unmaintained = \"workspace\"
";
        assert!(check_unmaintained_scope_is_all(deny).is_err());
    }

    #[test]
    fn the_staleness_bound_the_repository_ships_passes() {
        assert!(check_db_staleness_is_bounded("maximum-db-staleness = \"P7D\"\n").is_ok());
    }

    /// The regression itself, and the quiet one: before SAK-228 the line was
    /// simply absent, and `cargo deny --offline check` was green off a
    /// 30-day-old database. Deleting it again looks like removing a redundant
    /// default.
    #[test]
    fn omitting_the_staleness_bound_fails() {
        let deny = "[advisories]\nyanked = \"deny\"\nunmaintained = \"all\"\n";
        let error = check_db_staleness_is_bounded(deny).expect_err("a missing bound must fail");
        assert!(error.contains("any age"), "{error}");
    }

    /// Widening is the other half: the setting can stay in the file and still
    /// stop meaning anything.
    #[test]
    fn a_staleness_bound_longer_than_a_week_fails() {
        assert!(check_db_staleness_is_bounded("maximum-db-staleness = \"P8D\"\n").is_err());
        assert!(check_db_staleness_is_bounded("maximum-db-staleness = \"P2W\"\n").is_err());
        assert!(check_db_staleness_is_bounded("maximum-db-staleness = \"P1Y\"\n").is_err());
        // A week and an hour is over the line too — the check is a comparison,
        // not a match against the one string we ship.
        assert!(check_db_staleness_is_bounded("maximum-db-staleness = \"P7DT1H\"\n").is_err());
    }

    /// Shorter is not the regression this guards, so it must not be reported as
    /// one. `P0D` is unusable in practice — it calls a database stale seconds
    /// after the fetch — but that failure is loud and self-inflicted, unlike a
    /// widened bound.
    #[test]
    fn a_shorter_staleness_bound_passes() {
        assert!(check_db_staleness_is_bounded("maximum-db-staleness = \"P1D\"\n").is_ok());
        assert!(check_db_staleness_is_bounded("maximum-db-staleness = \"PT12H\"\n").is_ok());
        assert!(check_db_staleness_is_bounded("maximum-db-staleness = \"P0D\"\n").is_ok());
    }

    /// The prose above the setting quotes `P7D` and `P0D` on purpose, to explain
    /// why the bound is not zero. Neither may be read as the setting.
    #[test]
    fn the_comment_about_the_staleness_bound_is_not_read_as_the_bound() {
        let deny = "\
# `P7D` and not `P0D`: the limit is measured against the cached fetch.
maximum-db-staleness = \"P7D\"
";
        assert!(check_db_staleness_is_bounded(deny).is_ok());

        // And a commented-out good value must not rescue a widened live one.
        let deny = "\
# maximum-db-staleness = \"P7D\"
maximum-db-staleness = \"P90D\"
";
        assert!(check_db_staleness_is_bounded(deny).is_err());
    }

    #[test]
    fn the_period_parser_reads_the_units_it_claims_to() {
        assert_eq!(staleness_seconds("P7D"), Ok(7 * 86_400));
        assert_eq!(staleness_seconds("P1W"), Ok(7 * 86_400));
        assert_eq!(
            staleness_seconds("P1DT2H3M4S"),
            Ok(86_400 + 7_200 + 180 + 4)
        );
        assert!(staleness_seconds("7D").is_err());
        assert!(staleness_seconds("P1M").is_err());
        assert!(staleness_seconds("P7").is_err());
    }

    #[test]
    fn the_unused_ignored_advisory_level_the_repository_ships_passes() {
        assert!(
            check_unused_ignored_advisory_is_deny("unused-ignored-advisory = \"deny\"\n").is_ok()
        );
    }

    /// The regression itself, and the quiet one: `"warn"` is the cargo-deny
    /// default, so this is what the file said before SAK-229 and what deleting
    /// the line goes back to. Measured on the stack tip: with a stale `ignore`
    /// entry and the `ci.yml` flag removed, `cargo deny check` printed
    /// `warning[advisory-not-detected]` and the step exited 0.
    #[test]
    fn warning_on_an_unused_ignore_entry_fails() {
        let error = check_unused_ignored_advisory_is_deny("unused-ignored-advisory = \"warn\"\n")
            .expect_err("\"warn\" must fail");
        assert!(error.contains("exits 0"), "{error}");
    }

    /// `"allow"` is the other level cargo-deny accepts, and the one that drops
    /// the diagnostic entirely.
    #[test]
    fn allowing_an_unused_ignore_entry_fails() {
        assert!(
            check_unused_ignored_advisory_is_deny("unused-ignored-advisory = \"allow\"\n").is_err()
        );
    }

    #[test]
    fn omitting_the_unused_ignored_advisory_level_fails() {
        let deny = "[advisories]\nyanked = \"deny\"\nmaximum-db-staleness = \"P7D\"\n";
        let error =
            check_unused_ignored_advisory_is_deny(deny).expect_err("a missing level must fail");
        assert!(error.contains("only warns"), "{error}");
    }

    /// The prose in `deny.toml` quotes this setting while explaining it, in the
    /// header as well as above the line itself. None of that may be read as the
    /// setting — otherwise deleting the live line would still pass.
    #[test]
    fn the_comment_about_the_level_is_not_read_as_the_level() {
        let deny = "\
# The same enforcement is set below as `unused-ignored-advisory = \"deny\"`.
yanked = \"deny\"
";
        assert!(check_unused_ignored_advisory_is_deny(deny).is_err());

        // And a commented-out good value must not rescue a live weakened one.
        let deny = "\
# unused-ignored-advisory = \"deny\"
unused-ignored-advisory = \"warn\"
";
        assert!(check_unused_ignored_advisory_is_deny(deny).is_err());
    }
}
