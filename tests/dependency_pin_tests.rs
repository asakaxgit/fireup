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
//! 5. **`[bans] wildcards` stays at `"deny"`.** A `*` version requirement is a
//!    dependency on whatever cargo resolves at build time, which is the thing
//!    `Cargo.lock` exists to stop, and it is how a major version nobody read
//!    arrives on an unrelated `cargo update`. Measured on this tree (SAK-230):
//!    with `tonic = "*"` in `Cargo.toml`, the shipped config exits 2 with
//!    `error[wildcard]` and `bans FAILED`; the same tree under
//!    `wildcards = "allow"` exits 0, so it is the setting and not the edit that
//!    makes it red. `Cargo.lock` does not change in either arm — `*` is
//!    satisfied by the version already pinned — so the red is not `--locked`
//!    complaining.
//!
//!    Deleting the line is the same as allowing it, only quieter: cargo-deny's
//!    default is `allow`, measured as exit 0 and `bans ok` with the wildcard
//!    still sitting in `Cargo.toml`.
//!
//! 6. **`[sources]` keeps every crate on crates.io.** `unknown-registry` and
//!    `unknown-git` are both `"deny"`, and `allow-registry` names crates.io
//!    alone. Together they are what stops an unreviewed, unpinned third-party
//!    repository from entering the build. Measured on this tree (SAK-230) with
//!    a local `file://` git dependency added: the shipped config exits 8 with
//!    `error[source-not-allowed]` and `sources FAILED`, and the same tree under
//!    `unknown-git = "allow"` exits 0.
//!
//!    Deleting either line is worse than it looks. The default is `warn`, so
//!    the same git dependency that fails the gate instead prints
//!    `warning[source-not-allowed]`, the check reports `sources ok`, and the
//!    step exits 0.
//!
//!    `allow-registry` is the part cargo-deny cannot check at all. A second
//!    registry added to that list exits 0 with only
//!    `warning[unmatched-source]` ("allowed source was not encountered") while
//!    nothing uses it — and the day a crate does come from it, there is no
//!    diagnostic left to print. This test is the only thing that can report
//!    that edit, which is the same argument as (3) and (4): the setting is the
//!    tool's own input.
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

/// The live value of a `key = value` line in `deny.toml`, comments stripped.
///
/// Last occurrence wins, which is how a TOML parser treats a redefinition, and
/// comments are stripped before matching for the reason the three checks above
/// strip them: the prose in `deny.toml` quotes the settings it explains, and
/// that has to stay writable.
///
/// Flat matching with no notion of tables is enough — every key this file
/// checks is unique across cargo-deny's whole schema. (The three checks above
/// predate this helper and inline the same loop. They are left as they are so
/// this change is only the two guards it adds; folding them in is a tidy-up for
/// once the layers below this one have landed.)
fn setting(deny: &str, key: &str) -> Option<String> {
    let mut found = None;
    for line in deny.lines() {
        let code = line.split('#').next().unwrap_or("");
        if let Some((candidate, value)) = code.split_once('=') {
            if candidate.trim() == key {
                found = Some(value.trim().trim_matches('"').to_string());
            }
        }
    }
    found
}

/// The entries of a `key = [...]` array in `deny.toml`, comments stripped.
///
/// Reads across newlines until the brackets balance. Matching a single line
/// would be enough for the file as it ships and wrong for the edit this
/// guards: a second registry would most naturally arrive as a reformatted
/// multi-line array, and a reader that only looked at the `key = [` line would
/// find no entries on it and pass.
fn setting_array(deny: &str, key: &str) -> Option<Vec<String>> {
    let mut entries = None;
    let mut body: Option<String> = None;
    for line in deny.lines() {
        let code = line.split('#').next().unwrap_or("");
        if let Some(open) = body.as_mut() {
            open.push_str(code);
        } else if let Some((candidate, value)) = code.split_once('=') {
            if candidate.trim() == key && value.trim_start().starts_with('[') {
                body = Some(value.to_string());
            }
        }

        let Some(collected) = body.as_deref() else {
            continue;
        };
        if collected.matches('[').count() != collected.matches(']').count() {
            continue;
        }
        let inner = collected
            .trim()
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .unwrap_or_default();
        entries = Some(
            inner
                .split(',')
                .map(|entry| entry.trim().trim_matches('"').to_string())
                .filter(|entry| !entry.is_empty())
                .collect(),
        );
        body = None;
    }
    entries
}

/// `Err` unless `[bans] wildcards` is set to `"deny"`.
fn check_wildcards_are_denied(deny: &str) -> Result<(), String> {
    match setting(deny, "wildcards").as_deref() {
        Some("deny") => Ok(()),
        Some(level) => Err(format!(
            "deny.toml sets `[bans] wildcards = \"{level}\"`, not \"deny\". A `*` version \
             requirement resolves to whatever is newest at build time, so a major version \
             nobody reviewed arrives on an unrelated `cargo update`. Measured on this tree \
             (SAK-230): with `tonic = \"*\"` in Cargo.toml, \"deny\" exits 2 with \
             `error[wildcard]` and `bans FAILED`, while the same tree at this value exits 0."
        )),
        None => Err(
            "deny.toml has no `[bans] wildcards` setting, which is the same as allowing \
                     wildcards and quieter: cargo-deny's default is `allow`. Measured on this \
                     tree (SAK-230) with `tonic = \"*\"` in Cargo.toml and this line deleted, \
                     `cargo deny check bans` printed no wildcard diagnostic at all and exited \
                     0. Set it to \"deny\"."
                .into(),
        ),
    }
}

/// The spellings of the crates.io index that count as crates.io.
///
/// `Cargo.lock` in this repository records the git form, which is what
/// `allow-registry` carries. The sparse form is what newer cargo writes for the
/// same registry, so both are accepted rather than failing a cosmetic change;
/// anything else is a different registry.
const CRATES_IO_INDEX: &[&str] = &[
    "https://github.com/rust-lang/crates.io-index",
    "sparse+https://index.crates.io/",
];

/// `Err` unless `[sources]` confines every dependency to crates.io.
///
/// Three facts, checked together because they only mean anything together and
/// weakening any one of them is enough: `unknown-registry` and `unknown-git`
/// decide whether a source nobody listed is an error, and `allow-registry`
/// decides what counts as listed.
fn check_sources_are_crates_io_only(deny: &str) -> Result<(), String> {
    for key in ["unknown-registry", "unknown-git"] {
        match setting(deny, key).as_deref() {
            Some("deny") => {}
            Some(level) => {
                return Err(format!(
                    "deny.toml sets `[sources] {key} = \"{level}\"`, not \"deny\". Anything but \
                     \"deny\" lets a crate come from a source nobody listed, which is an \
                     unreviewed and unpinned third party in the build. Measured on this tree \
                     (SAK-230) with a local git dependency added: \"deny\" exits 8 with \
                     `error[source-not-allowed]` and `sources FAILED`, and the same tree at \
                     `unknown-git = \"allow\"` exits 0."
                ));
            }
            None => {
                return Err(format!(
                    "deny.toml has no `[sources] {key}`. That is as permissive as \"allow\" and \
                     much quieter: cargo-deny's default is `warn`, so the dependency that \
                     should fail the gate instead prints `warning[source-not-allowed]`, the \
                     check reports `sources ok`, and the step exits 0 — measured on this tree \
                     (SAK-230) with a local git dependency added. Set it to \"deny\"."
                ));
            }
        }
    }

    let Some(registries) = setting_array(deny, "allow-registry") else {
        return Err(
            "deny.toml has no `[sources] allow-registry`. This one is not a hole today — \
                     cargo-deny's default already allows crates.io, measured on this tree \
                     (SAK-230) as `sources ok` and exit 0 with the line deleted. It is pinned \
                     because that default is cargo-deny's to change and the version is bumped \
                     by hand in ci.yml, so the registry Fireup accepts belongs in Fireup's own \
                     config. Set it to [\"https://github.com/rust-lang/crates.io-index\"]."
                .into(),
        );
    };

    if registries.is_empty() {
        return Err(
            "deny.toml sets `[sources] allow-registry` to an empty list, so no registry is \
                     allowed at all and nothing resolves. cargo-deny reports that loudly by \
                     itself, but passing it here would be a vacuous green."
                .into(),
        );
    }

    for registry in &registries {
        if !CRATES_IO_INDEX.contains(&registry.as_str()) {
            return Err(format!(
                "deny.toml allows the registry `{registry}`, which is not crates.io. Fireup \
                 takes dependencies from crates.io only. This is the one fact in this file \
                 that cargo-deny cannot report for itself: measured on this tree (SAK-230), a \
                 second entry in `allow-registry` exits 0 with only \
                 `warning[unmatched-source]` while nothing uses it, and once a crate does come \
                 from it there is no diagnostic at all."
            ));
        }
    }
    Ok(())
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

#[test]
fn deny_toml_refuses_a_wildcard_dependency() {
    if let Err(reason) = check_wildcards_are_denied(&repo_file("deny.toml")) {
        panic!("{reason}");
    }
}

#[test]
fn deny_toml_allows_crates_io_as_the_only_source() {
    if let Err(reason) = check_sources_are_crates_io_only(&repo_file("deny.toml")) {
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

    #[test]
    fn the_wildcards_level_the_repository_ships_passes() {
        assert!(check_wildcards_are_denied("wildcards = \"deny\"\n").is_ok());
    }

    /// The regression itself. Measured on the stack tip: with `tonic = "*"` in
    /// `Cargo.toml`, `"deny"` exits 2 with `error[wildcard]` and this value
    /// exits 0 on the identical tree.
    #[test]
    fn allowing_wildcard_dependencies_fails() {
        let error =
            check_wildcards_are_denied("wildcards = \"allow\"\n").expect_err("\"allow\" must fail");
        assert!(error.contains("error[wildcard]"), "{error}");
    }

    /// `"warn"` is the third level cargo-deny accepts: the diagnostic is still
    /// printed, and the check still exits 0.
    #[test]
    fn warning_on_a_wildcard_dependency_fails() {
        assert!(check_wildcards_are_denied("wildcards = \"warn\"\n").is_err());
    }

    /// Deleting the line is the quiet version, and the one a tidy-up would
    /// produce: cargo-deny's default is `allow`, so the file then reads as if
    /// wildcards were never considered.
    #[test]
    fn omitting_the_wildcards_setting_fails() {
        let deny = "[bans]\nmultiple-versions = \"warn\"\ndeny = []\n";
        let error = check_wildcards_are_denied(deny).expect_err("a missing setting must fail");
        assert!(error.contains("default is `allow`"), "{error}");
    }

    /// `deny.toml` is mostly comments, so a naive `contains` finds the prose
    /// rather than the setting. Neither direction may be confused.
    #[test]
    fn the_comment_about_wildcards_is_not_read_as_the_setting() {
        let deny = "\
# `wildcards = \"deny\"` is what stops a `*` requirement reaching the lock file.
multiple-versions = \"warn\"
";
        assert!(check_wildcards_are_denied(deny).is_err());

        // And a commented-out good value must not rescue a live weakened one.
        let deny = "\
# wildcards = \"deny\"
wildcards = \"allow\"
";
        assert!(check_wildcards_are_denied(deny).is_err());
    }

    /// `[sources]` as the repository ships it.
    const SOURCES: &str = r#"
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
"#;

    #[test]
    fn the_sources_policy_the_repository_ships_passes() {
        assert!(check_sources_are_crates_io_only(SOURCES).is_ok());
    }

    /// The regression for the git half. Measured on the stack tip with a local
    /// `file://` git dependency added: `"deny"` exits 8 with
    /// `error[source-not-allowed]`, this value exits 0 on the identical tree.
    #[test]
    fn allowing_an_unknown_git_source_fails() {
        let deny = SOURCES.replace("unknown-git = \"deny\"", "unknown-git = \"allow\"");
        let error =
            check_sources_are_crates_io_only(&deny).expect_err("an allowed git source must fail");
        assert!(error.contains("unknown-git"), "{error}");
    }

    #[test]
    fn allowing_an_unknown_registry_fails() {
        let deny = SOURCES.replace(
            "unknown-registry = \"deny\"",
            "unknown-registry = \"allow\"",
        );
        let error = check_sources_are_crates_io_only(&deny)
            .expect_err("an allowed unknown registry must fail");
        assert!(error.contains("unknown-registry"), "{error}");
    }

    /// `"warn"` is the cargo-deny default for both keys, which is why deleting
    /// a line and weakening it to `"warn"` are the same regression: the
    /// dependency is reported and the check still exits 0.
    #[test]
    fn warning_on_an_unlisted_source_fails() {
        for key in ["unknown-registry", "unknown-git"] {
            let deny = SOURCES.replace(&format!("{key} = \"deny\""), &format!("{key} = \"warn\""));
            assert!(
                check_sources_are_crates_io_only(&deny).is_err(),
                "`{key} = \"warn\"` must fail"
            );
        }
    }

    /// Deleting either line, which is the quiet form of the same thing.
    #[test]
    fn omitting_an_unlisted_source_setting_fails() {
        for key in ["unknown-registry", "unknown-git"] {
            let deny = SOURCES.replace(&format!("{key} = \"deny\"\n"), "");
            let error =
                check_sources_are_crates_io_only(&deny).expect_err("a missing setting must fail");
            assert!(error.contains("exits 0"), "{error}");
        }
    }

    /// The regression cargo-deny itself cannot report: a second registry in the
    /// allow list is exit 0 with only `warning[unmatched-source]`.
    #[test]
    fn a_second_registry_in_the_allow_list_fails() {
        let deny = SOURCES.replace(
            r#"allow-registry = ["https://github.com/rust-lang/crates.io-index"]"#,
            r#"allow-registry = ["https://github.com/rust-lang/crates.io-index", "https://example.invalid/index"]"#,
        );
        let error =
            check_sources_are_crates_io_only(&deny).expect_err("a second registry must fail");
        assert!(error.contains("https://example.invalid/index"), "{error}");
    }

    /// Replacing crates.io outright, rather than adding alongside it.
    #[test]
    fn a_substituted_registry_fails() {
        let deny = SOURCES.replace(
            "https://github.com/rust-lang/crates.io-index",
            "https://example.invalid/index",
        );
        assert!(check_sources_are_crates_io_only(&deny).is_err());
    }

    /// The multi-line form of the same edit. This is the shape a second
    /// registry would most likely arrive in, and the shape a single-line reader
    /// would pass over without seeing any entry at all.
    #[test]
    fn a_multi_line_allow_registry_is_read_across_lines() {
        let good = r#"
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = [
  "https://github.com/rust-lang/crates.io-index", # crates.io
]
"#;
        assert!(check_sources_are_crates_io_only(good).is_ok());

        let widened = r#"
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = [
  "https://github.com/rust-lang/crates.io-index",
  "https://example.invalid/index",
]
"#;
        let error = check_sources_are_crates_io_only(widened)
            .expect_err("a multi-line second registry must fail");
        assert!(error.contains("https://example.invalid/index"), "{error}");
    }

    /// Not a hole on its own — cargo-deny allows crates.io by default — but the
    /// default is the tool's to change, so the list is pinned here.
    #[test]
    fn omitting_the_allow_registry_list_fails() {
        let deny = SOURCES.replace(
            r#"allow-registry = ["https://github.com/rust-lang/crates.io-index"]"#,
            "",
        );
        let error = check_sources_are_crates_io_only(&deny).expect_err("a missing list must fail");
        assert!(error.contains("not a hole today"), "{error}");
    }

    /// An empty list breaks the build loudly rather than quietly, so it is not
    /// the regression this guards — but reporting `Ok` for it would be a
    /// vacuous pass, which is worse than a duplicated complaint.
    #[test]
    fn an_empty_allow_registry_fails_rather_than_passing_vacuously() {
        let deny = SOURCES.replace(r#"["https://github.com/rust-lang/crates.io-index"]"#, "[]");
        let error = check_sources_are_crates_io_only(&deny).expect_err("an empty list must fail");
        assert!(error.contains("vacuous"), "{error}");
    }

    /// The sparse protocol is the same registry under a different spelling, so
    /// it must not be reported as a foreign one.
    #[test]
    fn the_sparse_spelling_of_crates_io_passes() {
        let deny = SOURCES.replace(
            "https://github.com/rust-lang/crates.io-index",
            "sparse+https://index.crates.io/",
        );
        assert!(check_sources_are_crates_io_only(&deny).is_ok());
    }

    /// The `[sources]` prose in `deny.toml` is three lines of comment above
    /// three lines of config, and names both settings while explaining them.
    #[test]
    fn the_comment_about_the_sources_policy_is_not_read_as_the_policy() {
        let deny = r#"
# Everything must come from crates.io: `unknown-registry = "deny"` and
# `unknown-git = "deny"`, with `allow-registry` naming crates.io alone.
multiple-versions = "warn"
"#;
        assert!(check_sources_are_crates_io_only(deny).is_err());

        // A commented-out good value must not rescue a live weakened one.
        let deny = r#"
unknown-registry = "deny"
# unknown-git = "deny"
unknown-git = "allow"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
"#;
        assert!(check_sources_are_crates_io_only(deny).is_err());

        // Nor may a commented-out registry list rescue a widened live one, or
        // count as the list when the live one is gone.
        let deny = r#"
unknown-registry = "deny"
unknown-git = "deny"
# allow-registry = ["https://github.com/rust-lang/crates.io-index"]
allow-registry = ["https://example.invalid/index"]
"#;
        assert!(check_sources_are_crates_io_only(deny).is_err());

        let deny = r#"
unknown-registry = "deny"
unknown-git = "deny"
# allow-registry = ["https://github.com/rust-lang/crates.io-index"]
"#;
        let error = check_sources_are_crates_io_only(deny)
            .expect_err("a commented-out list is not the list");
        assert!(error.contains("not a hole today"), "{error}");
    }

    /// A commented-out multi-line array must not be read as the live one
    /// either — the bracket-balancing reader is the part at risk here.
    #[test]
    fn a_commented_out_multi_line_allow_registry_is_not_the_list() {
        let deny = r#"
unknown-registry = "deny"
unknown-git = "deny"
# allow-registry = [
#   "https://example.invalid/index",
# ]
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
"#;
        assert!(check_sources_are_crates_io_only(deny).is_ok());
    }

    #[test]
    fn the_array_reader_reads_the_shapes_it_claims_to() {
        assert_eq!(
            setting_array(r#"allow-registry = ["a", "b"]"#, "allow-registry"),
            Some(vec!["a".to_string(), "b".to_string()])
        );
        assert_eq!(
            setting_array(
                "allow-registry = [\n  \"a\",\n  \"b\",\n]\n",
                "allow-registry"
            ),
            Some(vec!["a".to_string(), "b".to_string()])
        );
        assert_eq!(
            setting_array("allow-registry = []", "allow-registry"),
            Some(vec![])
        );
        assert_eq!(setting_array("deny = []", "allow-registry"), None);
        // A plain scalar is not an array, and must not be reported as an empty
        // one — that would turn a typo into a silent pass.
        assert_eq!(setting_array(r#"wildcards = "deny""#, "wildcards"), None);
    }
}
