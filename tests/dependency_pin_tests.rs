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
//!    makes it worth a test. The `deny` job would not notice: CI does not take
//!    the `--offline` path today — that job passes only `--locked` — so
//!    `cargo deny` stays green if the line goes away, and stays green the day
//!    someone adds `actions/cache` for `~/.cargo` or `--frozen` for
//!    reproducibility. The step that goes red is the one running this file.
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
//! 7. **`[advisories] yanked` stays at `"deny"`, and yank checking stays on.** A
//!    yanked crate in `Cargo.lock` is a version that was withdrawn from
//!    crates.io *after* we pinned it — usually because it was broken or unsound
//!    — and it is the one supply-chain fact `Cargo.lock` is actively working to
//!    preserve, since a pinned yanked version keeps resolving for as long as the
//!    lock file says so. Measured on this tree (SAK-231) with the lock
//!    downgraded to the yanked `async-trait` 0.1.90: the shipped config exits 1
//!    with `error[yanked]` and `advisories FAILED`, `"warn"` exits 0 with
//!    `warning[yanked]`, and deleting the line exits 0 the same way — the
//!    default is `warn`.
//!
//!    This one needs *two* assertions rather than one, which is the reason it is
//!    listed separately from (3) and (4). `[advisories]` also has
//!    `disable-yank-checking`, and setting it to `true` turns the check off
//!    without touching the `yanked` line at all: measured on the same tree,
//!    `yanked = "deny"` plus `disable-yank-checking = true` exits 0 and prints
//!    **no yanked diagnostic of any kind**, which is quieter than either
//!    weakening above. A guard that only read `yanked` would call that config
//!    correct. Where an enforcement key has a separate kill switch beside it,
//!    asserting the policy's value is not evidence that the policy runs.
//!
//!    (`disable-yank-checking = false` is accepted, and was measured rather than
//!    assumed: the explicit `false` exits 1 on the yanked lock exactly like the
//!    absent key.)
//!
//!    The control arm matters here more than elsewhere, because the mutation is
//!    a lock-file edit rather than a config edit: with the lock back at
//!    `async-trait` 0.1.92 the shipped config exits 0 with no diagnostics, so
//!    the red in the first arm is the setting meeting a yanked crate and not the
//!    downgrade by itself. The measurement deliberately uses `async-trait`
//!    0.1.90 and not the yanked `tokio` 1.54.0: tokio 1.54.0 also carries
//!    RUSTSEC-2026-0384, so it gives exit 1 for two reasons at once, while
//!    `advisory-db/crates/async-trait/` is empty.
//!
//! Reading `Cargo.lock` as text rather than running `cargo metadata` is
//! deliberate: it keeps the test free of a toml dependency and of a subprocess,
//! and the two fields it wants (`name`, `version`) are the first two lines of
//! every `[[package]]` block in the v3/v4 lock format.
//!
//! Not all of them are in `Cargo.lock` or `deny.toml` any more. Three now read
//! `.github/workflows/ci.yml`, because that is where the rest of this gate's
//! behaviour is written down: SAK-239 needs the `grep` the `deny` job runs,
//! SAK-240 needs the scope of the token every job in that workflow is handed,
//! and SAK-243 needs the gate command itself — the list of check names in it is
//! what decides which of the `deny.toml` settings above are in force at all,
//! and a name dropped from that line switches a whole check off with no
//! diagnostic and a zero exit. Those checks live here rather than in a workflow
//! test of their own for the reason the bottom of this header gives — this is
//! the one test target CI actually names.
//!
//! The checks are written as functions over parsed text, not as assertions over
//! the real file, so that the "and it really does fail" half can be committed
//! too — see the `negative_controls` module at the bottom. A hand-edited
//! `Cargo.lock` is not a usable mutation here: cargo re-resolves an inconsistent
//! lock file before it runs the test, so breaking the real file on disk proves
//! nothing.
//!
//! None of the above is a guard unless something runs it. This is an integration
//! test target: `cargo test --lib` does not reach it, `cargo build
//! --all-targets` only compiles it, and `coverage.yml` measures `--lib` scope
//! only. Every cargo invocation in the `test` job of `.github/workflows/ci.yml`
//! names its target with `--test`, and for the first five tickets' worth of
//! checks above none of them named this one — so the file ran nowhere but on the
//! machine of whoever typed a bare `cargo test`, and each of the one-line config
//! edits it exists to catch passed CI green. Measured in SAK-232, which added
//! the step. If a later tidy-up drops `--test dependency_pin_tests` from that
//! workflow, every check in this file switches off and CI gets quieter, not
//! louder.

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

/// `Err` unless `[advisories] yanked` is `"deny"` **and** nothing has switched
/// yank checking off beside it.
///
/// Two assertions, because the second one is the only thing that can report the
/// quietest version of this regression. `disable-yank-checking = true` leaves
/// `yanked = "deny"` untouched and still produces a tree with zero yanked
/// diagnostics, so the value of the policy key says nothing on its own about
/// whether the policy runs.
fn check_yanked_is_denied(deny: &str) -> Result<(), String> {
    match setting(deny, "yanked").as_deref() {
        Some("deny") => {}
        Some(level) => {
            return Err(format!(
                "deny.toml sets `[advisories] yanked = \"{level}\"`, not \"deny\". A yanked \
                 version in Cargo.lock was withdrawn from crates.io after we pinned it, and the \
                 lock file is what keeps it resolving. Measured on this tree (SAK-231) with the \
                 lock downgraded to the yanked async-trait 0.1.90: \"deny\" exits 1 with \
                 `error[yanked]` and `advisories FAILED`, while the same tree at this value \
                 exits 0 with only `warning[yanked]`."
            ));
        }
        None => {
            return Err(
                "deny.toml has no `[advisories] yanked` setting, which is the same as warning \
                 and quieter: cargo-deny's default is `warn`. Measured on this tree (SAK-231) \
                 with the lock downgraded to the yanked async-trait 0.1.90 and this line \
                 deleted, `cargo deny check advisories` printed `warning[yanked]`, reported \
                 `advisories ok` and exited 0. Set it to \"deny\"."
                    .into(),
            );
        }
    }

    match setting(deny, "disable-yank-checking").as_deref() {
        None | Some("false") => Ok(()),
        Some(value) => Err(format!(
            "deny.toml sets `[advisories] disable-yank-checking = {value}`, which switches the \
             yanked check off whatever `yanked` says. This is the regression that does not show \
             up in the `yanked` line: measured on this tree (SAK-231) with the lock downgraded \
             to the yanked async-trait 0.1.90, `yanked = \"deny\"` with this key set exits 0 and \
             prints no yanked diagnostic at all — not even the `warning[yanked]` that weakening \
             `yanked` itself would leave behind. Delete the key, or set it to false."
        )),
    }
}

/// `Err` unless `[licenses] include-dev` is on **and** nothing has dropped dev
/// dependencies out of the graph beside it.
///
/// Two assertions, and they report different regressions, so they carry
/// different evidence rather than one message with the key name substituted in.
/// `[graph] exclude-dev = true` removes dev dependencies before the licence
/// check runs, which reopens the hole with `include-dev = true` still sitting in
/// the file untouched — exactly the shape `disable-yank-checking` has against
/// `yanked` (SAK-231).
fn check_dev_licenses_are_checked(deny: &str) -> Result<(), String> {
    match setting(deny, "include-dev").as_deref() {
        Some("true") => {}
        Some(value) => {
            return Err(format!(
                "deny.toml sets `[licenses] include-dev = {value}`, so the licence allow list \
                 governs only what Fireup ships and says nothing about what its test suite \
                 pulls in. Measured on this tree (SAK-234) with the MPL-2.0 `smartstring` \
                 1.0.1 added to `[dev-dependencies]`, reached only as `(dev) fireup`: `true` \
                 exits 4 with `error[rejected]` and `licenses FAILED`, and the same tree at \
                 this value exits 0 with `licenses ok`. Set it to true."
            ));
        }
        None => {
            return Err(
                "deny.toml has no `[licenses] include-dev`, which is cargo-deny's default and \
                 means dev dependencies are not licence-checked at all. Measured on this tree \
                 (SAK-234) with the MPL-2.0 `smartstring` 1.0.1 added to `[dev-dependencies]`: \
                 with this line deleted, `cargo deny check licenses` reported `licenses ok` and \
                 exited 0 — a copyleft licence the allow list excludes passed the gate with no \
                 diagnostic. Set it to true."
                    .into(),
            );
        }
    }

    match setting(deny, "exclude-dev").as_deref() {
        None | Some("false") => Ok(()),
        Some(value) => Err(format!(
            "deny.toml sets `[graph] exclude-dev = {value}`, which drops dev dependencies from \
             the graph before the licence check ever sees them — so `include-dev` above is still \
             set and still means nothing. This is the regression that does not show up in the \
             `include-dev` line: measured on this tree (SAK-234) with the MPL-2.0 `smartstring` \
             1.0.1 in `[dev-dependencies]`, `include-dev = true` with this key set exits 0 and \
             prints no rejection at all, where `include-dev` alone exits 4. Delete the key, or \
             set it to false."
        )),
    }
}

/// `Err` unless `[sources] unused-allowed-source` is `"deny"`.
///
/// This is the one key that makes cargo-deny itself fail on a widened
/// `allow-registry`. `check_sources_are_crates_io_only` above already fails on
/// a second registry and is the stronger check of the two — it keeps failing
/// once a crate really comes from the new registry, which is the moment
/// `unmatched-source` stops firing. The overlap is deliberate: that one is a
/// test, and a test can be stopped from running (SAK-221), while this one is
/// read by the `deny` job itself.
fn check_unused_allowed_source_is_deny(deny: &str) -> Result<(), String> {
    match setting(deny, "unused-allowed-source").as_deref() {
        Some("deny") => Ok(()),
        Some(level) => Err(format!(
            "deny.toml sets `[sources] unused-allowed-source = \"{level}\"`, not \"deny\", so \
             cargo-deny does not fail on an `allow-registry` entry that nothing comes from. \
             Measured on this tree (SAK-234) with a second entry added to `allow-registry`: \
             \"deny\" exits 8 with `error[unmatched-source]` and `sources FAILED`, while the \
             same tree at this value exits 0 with only `warning[unmatched-source]`."
        )),
        None => Err(
            "deny.toml has no `[sources] unused-allowed-source`. cargo-deny's default is \
             `warn`, so widening `allow-registry` is exit 0 with only \
             `warning[unmatched-source]` and the `deny` job stays green — measured on this \
             tree (SAK-234). Without it the only thing reporting a widened source allow list \
             is `deny_toml_allows_crates_io_as_the_only_source` in this file, and a test can \
             be stopped from running by editing ci.yml (SAK-221) where this key cannot. Set \
             it to \"deny\"."
                .into(),
        ),
    }
}

/// `Err` unless `[graph] targets` leaves every target in the graph.
///
/// `deny.toml` has no `targets` list and the comment above `[graph]` says that
/// is deliberate. This is the assertion that makes the comment binding, because
/// adding one line there narrows the graph and makes the `deny` job *greener* —
/// the direction nothing watching CI ever reports.
///
/// Measured on the stack tip (SAK-235) by varying only this key, with the
/// checked set counted using `allow = []`, which rejects every crate the gate
/// actually looks at, rather than with `cargo deny list`:
///
///   shipping config (no key)                     217 crates
///   + `targets = ["x86_64-unknown-linux-gnu"]`   173 crates
///   + `targets = []`                             217 crates
///
/// The 44 that disappear are the `windows-*` family, `wasi`, `wasip2`,
/// `wasm-bindgen*`, `js-sys`, `web-sys`, `r-efi`, `objc2-*`,
/// `core-foundation-sys`, `redox_syscall`, `libredox`,
/// `android_system_properties`, `hermit-abi`, `bumpalo`, `shlex` and `cc`.
///
/// Unlike `include-dev`, this key hides vulnerabilities and not only licences,
/// which is why the error below leads with the advisory evidence. Both halves
/// were measured on a tree held fixed while only `[graph]` moved:
///
///   `shlex = "=1.2.0"` (RUSTSEC-2024-0006) as a `cfg(windows)` dependency
///     shipping config          exit 1   `RUSTSEC-2024-0006` / `advisories FAILED`
///     + `targets = [linux]`    exit 0   `advisories ok`          <- hole
///     + `targets = [windows]`  exit 1   control: the crate is still vulnerable
///
///   `smartstring = "=1.0.1"` (MPL-2.0) as a `cfg(windows)` dependency
///     shipping config          exit 4   `error[rejected]` MPL-2.0
///     + `targets = [linux]`    exit 0   `licenses ok`            <- hole
///     + `targets = [windows]`  exit 4   control: narrowing is not what did it
///     + `[linux]` + MPL allowed exit 0  control: green has other causes
///
/// The `[windows]` arms are the ones that matter: without them a disappearing
/// red could just as well mean that restricting the graph at all breaks the
/// check, rather than that *this* restriction hides *these* crates.
///
/// An empty list is accepted because it was measured to be the same as no list
/// (217 crates either way), which is cargo-deny's documented "all targets".
///
/// `[graph] exclude` narrows the graph too — `exclude = ["windows-sys"]` takes
/// the checked set from 217 to 206 — and is deliberately not forbidden here,
/// for the reason SAK-234 left it alone: it names the crates it excuses, so it
/// is visible in a diff as a decision, where `targets` excuses a whole platform
/// by naming a platform we do support. Requiring `exclude` to stay empty is a
/// new policy decision, not a guard on this one.
fn check_graph_covers_every_target(deny: &str) -> Result<(), String> {
    if setting(deny, "targets").is_none() {
        return Ok(());
    }

    match setting_array(deny, "targets") {
        Some(targets) if targets.is_empty() => Ok(()),
        Some(targets) => Err(format!(
            "deny.toml sets `[graph] targets = {targets:?}`, which drops every crate that only \
             builds for a target outside that list — their advisories and their licences stop \
             being checked and the `deny` job gets *greener* for it. This hides \
             vulnerabilities, not just licences: measured on this tree (SAK-235) with the \
             vulnerable `shlex` 1.2.0 (RUSTSEC-2024-0006) reached as a `cfg(windows)` \
             dependency, no list exits 1 and names the advisory, \
             `[\"x86_64-unknown-linux-gnu\"]` exits 0 with `advisories ok`, and \
             `[\"x86_64-pc-windows-msvc\"]` exits 1 again on the identical tree — so the red \
             is lost to which target was chosen, not to narrowing as such. Counted with \
             `allow = []`, the checked set goes from 217 crates to 173. Delete the key: \
             checking every target is the stricter reading and needs no maintenance. An empty \
             list is accepted too, and was measured to be the same 217 crates."
        )),
        None => Err(
            "deny.toml has a live `[graph] targets` line that is not an array, so this guard \
             cannot tell which targets it leaves out and must not report the graph as \
             unrestricted. cargo-deny rejects the scalar form by itself — measured on this \
             tree (SAK-235), `targets = \"x86_64-unknown-linux-gnu\"` exits 1 with \
             `error[wanted]: expected an array` — so today this branch is belt and braces. It \
             is here so that a form this reader does not understand can never be the thing \
             that makes the guard pass. Delete the key, or write it as an empty array."
                .into(),
        ),
    }
}

/// `Err` unless `[advisories] unsound` is set to `"all"`.
///
/// `unsound` is the sibling of `unmaintained` and takes the same scope enum,
/// not a lint level, so the same hole SAK-225 closed for one key was still open
/// for the other. The difference is that `unmaintained` was at least written
/// down: `unsound` was absent, and absence is `"workspace"`, so a transitive
/// unsound advisory was reported nowhere.
///
/// Measured (SAK-236) with `anstream` 0.6.7 — RUSTSEC-2024-0404, unsound,
/// affects `< 0.6.8` — made purely transitive by pinning `clap = "=4.5.0"`, so
/// `fireup -> clap 4.5.0 -> clap_builder 4.5.0 -> anstream 0.6.7` with no direct
/// edge. Tree fixed, only this key moved:
///
///   no key (what shipped)      exit 0   `advisories ok`                 <- hole
///   `unsound = "all"`          exit 1   `error[unsound]` RUSTSEC-2024-0404
///   `unsound = "workspace"`    exit 0   control: identical to no key
///   `unsound = "transitive"`   exit 1   control: the crate is reachable
///   `unsound = "none"`         exit 0   control: the kill switch
///
/// The `"workspace"` arm is what pins the default; the `"transitive"` arm is
/// what rules out "that crate was unreachable anyway". Separately, `const-cstr`
/// 0.3.0 (RUSTSEC-2023-0020) as a *direct* dependency exits 1 with the key still
/// unset, which rules out `"none"` as the default.
///
/// One assertion is enough here, unlike `yanked` (SAK-231) or `include-dev`
/// (SAK-234): there is no separate kill-switch key, so the value of this key
/// alone decides the scope. Absence is a failure, because absence is
/// `"workspace"`.
fn check_unsound_scope_is_all(deny: &str) -> Result<(), String> {
    match setting(deny, "unsound").as_deref() {
        Some("all") => Ok(()),
        Some(scope) => Err(format!(
            "deny.toml sets `[advisories] unsound = \"{scope}\"`, not \"all\". Only \"all\" \
             reports unsoundness in transitive dependencies, and every other value of this key \
             makes the `deny` job *greener* rather than redder, so nothing else will tell you it \
             changed. Measured on this tree (SAK-236) with RUSTSEC-2024-0404 (`anstream` 0.6.7) \
             reached only through `clap 4.5.0 -> clap_builder 4.5.0`: \"all\" and \"transitive\" \
             exit 1 and name the advisory, while \"workspace\" and \"none\" exit 0 with \
             `advisories ok` on the identical tree. Unsoundness is memory corruption, and a \
             transitive one is still a question about a direct dependency we chose."
        )),
        None => Err(
            "deny.toml has no `[advisories] unsound` setting, so it runs at the \
                     cargo-deny default of \"workspace\" and reports no unsound advisory in any \
                     transitive dependency. That is the same blind spot SAK-225 closed for \
                     `unmaintained`, one key to the left and never written down. Measured \
                     (SAK-236): with RUSTSEC-2024-0404 (`anstream` 0.6.7) purely transitive, no \
                     key exits 0 with `advisories ok` and is byte-identical to \
                     `unsound = \"workspace\"`, while `unsound = \"transitive\"` exits 1 on the \
                     same tree. Set it to \"all\" explicitly."
                .into(),
        ),
    }
}

/// The official RustSec advisory database, with the spellings that mean it.
const OFFICIAL_ADVISORY_DB: &str = "https://github.com/rustsec/advisory-db";

/// Reduce an advisory-database URL to a comparable form.
///
/// A trailing slash and a `.git` suffix both name the same repository — the
/// suffix form was measured to work (SAK-236: it exits 1 and names the same
/// advisory) — so neither may be the reason this guard fails.
fn same_advisory_db(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/').to_ascii_lowercase();
    trimmed
        .strip_suffix(".git")
        .unwrap_or(&trimmed)
        .trim_end_matches('/')
        .to_string()
}

/// `Err` unless the advisory database cargo-deny reads is still the official one.
///
/// Every other loosening this file guards against leaves the policy visible and
/// only changes how strictly it is applied. This one leaves every `deny` in
/// place and changes what the database is willing to mention, which makes it the
/// only edit here that is invisible both to the `deny` job and to a reader of
/// the diff — the URL of a fork looks exactly like the URL of the real thing.
///
/// SAK-228 established that failing to *fetch* a database fails closed, in three
/// different ways. What was unmeasured was a database that fetches cleanly and
/// has been edited. Measured now (SAK-236) against local forks of advisory-db
/// served over `git daemon`, tree fixed with `shlex = "=1.2.0"`
/// (RUSTSEC-2024-0006), only this key moving:
///
///   official URL (ships)              exit 1   `error[vulnerability]` named
///   fork with zero advisories         exit 1   fails to load -> fail closed
///   fork that HAS the advisory        exit 1   control: the fork is really used
///   same fork, that one file deleted  exit 0   `advisories ok`          <- hole
///   thinned fork + official URL       exit 1   control: URLs are a union
///   official URL + `.git`             exit 1   another spelling, still works
///   key deleted                       exit 1   the default is the official DB
///   `db-urls = []`                    exit 1   empty is the default too
///
/// The two forks differ by exactly one file in `git ls-tree`. The control arm is
/// what makes the hole legible: without it, the green could mean the fork was
/// never fetched at all rather than that it was fetched and was quiet.
///
/// Hence the three decisions below. Absence passes, because the default is this
/// database. An empty list passes, because that was measured to be the default
/// too. A non-empty list must *contain* the official URL rather than equal it,
/// because several URLs were measured to be a union, so an extra entry can only
/// add advisories and never remove one.
///
/// A live `db-urls` line this reader cannot parse is a failure, for the reason
/// SAK-235 gave for `targets`: cargo-deny rejects the scalar form by itself
/// (`error[wanted]: expected an array`, measured), so today this is belt and
/// braces — it is here so that a form this guard does not understand can never
/// be the thing that makes it pass.
fn check_advisory_db_is_the_official_one(deny: &str) -> Result<(), String> {
    if setting(deny, "db-urls").is_none() {
        return Ok(());
    }

    match setting_array(deny, "db-urls") {
        Some(urls) if urls.is_empty() => Ok(()),
        Some(urls)
            if urls
                .iter()
                .any(|url| same_advisory_db(url) == same_advisory_db(OFFICIAL_ADVISORY_DB)) =>
        {
            Ok(())
        }
        Some(urls) => Err(format!(
            "deny.toml sets `[advisories] db-urls = {urls:?}`, which does not include the \
             official RustSec database `{OFFICIAL_ADVISORY_DB}`. That one line decides everything \
             the `advisories` check is able to know, and pointing it elsewhere loosens no policy \
             at all — every `deny` in this file stays `deny` and the database simply stops \
             mentioning things. Measured on this tree (SAK-236) with the vulnerable `shlex` 1.2.0 \
             (RUSTSEC-2024-0006): a fork that still carries the advisory exits 1 and names it, \
             and the same fork with that one file deleted — a one-file difference in \
             `git ls-tree` — exits 0 with `advisories ok`. An empty database at least fails \
             closed; a thinned one is silent, and neither `maximum-db-staleness` nor \
             `-D advisory-not-detected` notices. Adding URLs alongside the official one is fine \
             and was measured to be a union; replacing it is not. Deleting the key is fine too, \
             because the default is this same database."
        )),
        None => Err(format!(
            "deny.toml has a live `[advisories] db-urls` line that is not an array, so this guard \
             cannot tell which advisory databases the gate reads and must not report the source \
             as official. cargo-deny rejects the scalar form by itself — measured on this tree \
             (SAK-236), `db-urls = \"{OFFICIAL_ADVISORY_DB}\"` exits 1 with \
             `error[wanted]: expected an array` and never runs the check — so today this branch \
             is belt and braces, for the reason SAK-235 gave for `[graph] targets`: a form this \
             reader does not understand must never be what makes it pass. Write it as an array, \
             or delete the key and take the default."
        )),
    }
}

/// `true` if `entry` has `key` as a key — `key` followed by `=`, not merely the
/// text `key` somewhere inside it.
///
/// The prefix check is what keeps `allow` from matching `allow-globs`: the text
/// is there, but what follows it is `-globs =` and not `=`.
fn has_key(entry: &str, key: &str) -> bool {
    count_key(entry, key) > 0
}

/// How many times `key` appears as a key in `entry`.
///
/// Counted rather than merely found, because one bypass entry may list several
/// `allow = [{ path, checksum }]` pairs and the guard below needs to know that
/// *every* path is checksummed, not that one of them is.
fn count_key(entry: &str, key: &str) -> usize {
    let bytes = entry.as_bytes();
    let mut count = 0;
    let mut from = 0;
    while let Some(at) = entry[from..].find(key) {
        let start = from + at;
        let end = start + key.len();
        let joined_to_a_longer_name = start > 0
            && matches!(bytes[start - 1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-');
        if !joined_to_a_longer_name && entry[end..].trim_start().starts_with('=') {
            count += 1;
        }
        from = end;
    }
    count
}

/// The inline-table entries of `[bans.build] bypass`, one `String` each.
///
/// `setting_array` is no use here: it splits the array on commas, and every
/// entry of this one is an inline table full of commas. This reader collects the
/// array's text the same way — comments stripped, newlines ignored until the
/// brackets balance — and then splits it into the `{ … }` groups at depth one.
///
/// `None` means there is a live `bypass` line whose value this reader cannot
/// read as an array of inline tables. The caller turns that into a failure
/// rather than a pass, for the reason SAK-235 gave for `[graph] targets`.
fn bypass_entries(deny: &str) -> Option<Vec<String>> {
    let mut collected: Option<String> = None;
    let mut body: Option<String> = None;
    for line in deny.lines() {
        let code = line.split('#').next().unwrap_or("");
        if let Some(open) = body.as_mut() {
            open.push(' ');
            open.push_str(code);
        } else if let Some((candidate, value)) = code.split_once('=') {
            if candidate.trim() == "bypass" {
                if !value.trim_start().starts_with('[') {
                    return None;
                }
                body = Some(value.to_string());
            }
        }

        let Some(open) = body.as_deref() else {
            continue;
        };
        if open.matches('[').count() != open.matches(']').count() {
            continue;
        }
        collected = Some(open.to_string());
        body = None;
    }
    // An array that never closes is not readable either.
    if body.is_some() {
        return None;
    }

    let inner = collected?
        .trim()
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .map(str::to_string)?;

    let mut entries = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for ch in inner.chars() {
        match ch {
            '{' => {
                depth += 1;
                if depth > 1 {
                    current.push(ch);
                }
            }
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    entries.push(std::mem::take(&mut current));
                } else {
                    current.push(ch);
                }
            }
            // Between entries only whitespace and separators belong. A bare
            // value at depth zero is a shape this reader does not know.
            _ if depth == 0 => {
                if !ch.is_whitespace() && ch != ',' {
                    return None;
                }
            }
            _ => current.push(ch),
        }
    }
    if depth != 0 {
        return None;
    }
    Some(entries)
}

/// `Err` unless `[bans.build]` actually scans the binaries the build runs, and
/// every exemption is pinned to a path and its bytes.
///
/// This table did not exist in `deny.toml` until SAK-237, which means cargo-deny
/// was looking at no build script and no bundled executable anywhere in the
/// tree. Everything else in that file reasons about source — its licence, its
/// advisories, its version, its registry — and none of that says anything about
/// an ELF binary inside a `.crate` tarball. `protoc-bin-vendored` is a direct
/// build-dependency of Fireup (`cargo tree --locked -i protoc-bin-vendored` →
/// `[build-dependencies] fireup v0.1.0`); its eight platform crates each ship
/// one precompiled `protoc`, and `build.rs` executes the linux-x86_64 one on
/// every build in CI and on every developer machine.
///
/// **Four assertions, because three of the four ways to lose this are green.**
/// All measured on the stack tip (SAK-237) by varying only this table:
///
///   no `[bans.build]` at all                      exit 0   —            <- hole
///   `executables = "deny"` alone                  exit 0   0 findings   <- inert
///   + `include-dependencies = true`               exit 2   8 findings
///   `executables = "allow"` + include             exit 0   0 findings   control
///
/// `include-dependencies` defaults to false, which scopes the scan to the
/// workspace, and Fireup bundles no binaries of its own — so `executables =
/// "deny"` on its own reads as enforcement and enforces nothing. Absence of
/// `include-dependencies` therefore has to fail rather than pass: the same
/// two-keys-one-fact shape as `yanked` / `disable-yank-checking` (SAK-231) and
/// `include-dev` / `exclude-dev` (SAK-234).
///
/// The bypass list is the third assertion, and it has two inert shapes of its
/// own. Measured against the eight-crate shipping shape:
///
///   `allow = [{ path, checksum }]` x8             exit 0   `bans ok`
///   one checksum, one hex digit flipped           exit 2   `error[checksum-mismatch]`
///   one path wrong, checksum right                exit 2   `error[detected-executable]`
///   `bypass = [{ crate = "…" }]` — name only      exit 2   8 findings   <- inert
///   `allow = [{ path }]`, no `checksum`           exit 0   0 findings   <- hole
///   `allow-globs = ["bin/*"]`, one entry          exit 2   7 findings
///   `allow-globs = ["bin/*"]`, all eight          exit 0   0 findings
///   same glob on an unrelated crate               exit 2   8 findings   control
///   `executables = "allow"`                       exit 0   0 findings   control
///
/// A bare crate name exempts nothing at all. `allow` without `checksum` exempts
/// everything at that path for ever, and `checksum` is optional in cargo-deny's
/// schema (`error[unexpected-keys]` lists `["path", "checksum"]`) — so dropping
/// it is a one-token edit that keeps the gate green while unpinning the bytes of
/// every binary in the list. `allow-globs` is the shape that *looks* right and
/// exempts whatever the crate ever puts in `bin/`, including a second binary
/// that does not exist yet. The two control arms are what make the glob green a
/// statement about these entries rather than about the check being switched off.
///
/// The fourth assertion is the fail-closed branch: a live `bypass` line this
/// reader cannot parse as an array of inline tables must not be reported as a
/// checksum-pinned list.
///
/// Also measured, and deliberately not asserted: `include-dependencies` is the
/// only kill switch this policy has. `[bans] skip` and `skip-tree` leave the
/// count at eight (they only touch duplicate-version analysis),
/// `[graph] exclude-dev` leaves it at eight (a build-dependency is not a dev
/// dependency), and `[graph] targets = ["x86_64-unknown-linux-gnu"]` leaves it
/// at eight too — so unlike the licence and advisory checks, this one does not
/// depend on SAK-235's decision. `[graph] exclude` does narrow it, 8 -> 7, and
/// is left alone for the reason SAK-234 and SAK-235 left it alone: it names the
/// crate it excuses.
///
/// Unlike most of this file, this guard is not the only thing watching: the
/// `deny` job runs the same config, so a developer who flips one checksum sees
/// `bans FAILED` from the tool. What the tool cannot report is the table going
/// away, `include-dependencies` going false, or a checksum being deleted —
/// every one of those makes it greener.
fn check_build_executables_are_pinned(deny: &str) -> Result<(), String> {
    match setting(deny, "executables").as_deref() {
        Some("deny") => {}
        Some(level) => {
            return Err(format!(
                "deny.toml sets `[bans.build] executables = \"{level}\"`, not \"deny\", so the \
                 only binary blob Fireup's build actually runs is not checked at all. \
                 `protoc-bin-vendored` is a direct build-dependency and its eight platform \
                 crates ship a precompiled `protoc` each, downloaded from crates.io; `build.rs` \
                 executes the linux-x86_64 one on every build. Measured on this tree (SAK-237): \
                 \"deny\" with `include-dependencies = true` exits 2 with eight \
                 `error[detected-executable]`, and the same tree at this value exits 0 with \
                 `bans ok`. Set it to \"deny\"."
            ));
        }
        None => {
            return Err(
                "deny.toml has no `[bans.build] executables` setting. Without it cargo-deny \
                 looks at no build script and no bundled executable anywhere in the tree — every \
                 other check in that file reasons about source, and none of them says anything \
                 about an ELF binary inside a `.crate` tarball. That is how this file shipped \
                 before SAK-237: `cargo deny check bans` exited 0 and reported nothing while \
                 `build.rs` ran a vendored `protoc` on every build. Set it to \"deny\" and add \
                 `include-dependencies = true` beside it."
                    .into(),
            );
        }
    }

    match setting(deny, "include-dependencies").as_deref() {
        Some("true") => {}
        Some(value) => {
            return Err(format!(
                "deny.toml sets `[bans.build] include-dependencies = {value}`, which scopes the \
                 executable scan to the workspace — and Fireup bundles no binaries of its own, \
                 so the scan finds nothing and reports success with `executables = \"deny\"` \
                 still sitting above it. Measured on this tree (SAK-237): `executables = \
                 \"deny\"` alone exits 0 with zero findings, and adding this key takes the \
                 identical tree to exit 2 with eight. Set it to true."
            ));
        }
        None => {
            return Err(
                "deny.toml has no `[bans.build] include-dependencies`, and absence is the \
                 cargo-deny default of false — the executable scan then covers the workspace \
                 only, which bundles no binaries, so `executables = \"deny\"` above it is inert. \
                 This is the trap this guard exists for: the key that says `deny` is not the key \
                 that decides whether anything is scanned. Measured on this tree (SAK-237), \
                 `executables = \"deny\"` with this line deleted exits 0 with `bans ok` and zero \
                 findings while eight vendored `protoc` binaries sit in the build graph. Set it \
                 to true."
                    .into(),
            );
        }
    }

    // No `bypass` list at all is the strictest configuration there is, and has
    // to pass — the same reason `check_graph_covers_every_target` returns early
    // on an absent `targets`. The two cases below are "a live line this reader
    // cannot parse", which is a different thing from absence.
    if setting(deny, "bypass").is_none() {
        return Ok(());
    }

    let Some(entries) = bypass_entries(deny) else {
        return Err(
            "deny.toml has a live `[bans.build] bypass` line that this guard cannot read as an \
             array of inline tables, so it cannot tell whether the exemptions are pinned to a \
             checksum and must not report them as pinned. Same reasoning as the `[graph] targets` \
             scalar branch (SAK-235): a form this reader does not understand must never be what \
             makes it pass. Write it as `bypass = [{ crate = \"…\", allow = [{ path = \"…\", \
             checksum = \"…\" }] }]`."
                .into(),
        );
    };

    for entry in &entries {
        if has_key(entry, "allow-globs") {
            return Err(format!(
                "deny.toml has a `[bans.build] bypass` entry using `allow-globs`:{entry}. A glob \
                 exempts whatever that crate ever puts in the matching directory, including a \
                 second binary that does not exist yet, so the exemption grows silently with \
                 upstream. Measured on this tree (SAK-237): `allow-globs = [\"bin/*\"]` on all \
                 eight crates is exit 0 and `bans ok`, exactly like the checksum form — the \
                 difference only shows up later. Use `allow = [{{ path, checksum }}]`, which \
                 fixes the path and the bytes: a flipped hex digit gives \
                 `error[checksum-mismatch]` and a wrong path gives \
                 `error[detected-executable]` (both measured)."
            ));
        }

        let paths = count_key(entry, "path");
        let checksums = count_key(entry, "checksum");

        if paths == 0 {
            return Err(format!(
                "deny.toml has a `[bans.build] bypass` entry that exempts no path:{entry}. A bare \
                 crate name is completely inert against `executables` — measured on this tree \
                 (SAK-237), `bypass = [{{ crate = \"protoc-bin-vendored-linux-x86_64\" }}]` \
                 leaves the finding count at eight, unchanged — so an entry in this shape reads \
                 as an exemption that was granted and was not. Either give it \
                 `allow = [{{ path, checksum }}]`, or delete it. (An entry carrying only \
                 `build-script` lands here too: that key does nothing while \
                 `allow-build-scripts` is unset, and adopting that key is a deliberate change to \
                 make here as well.)"
            ));
        }

        if checksums != paths {
            return Err(format!(
                "deny.toml has a `[bans.build] bypass` entry with {paths} exempted path(s) but \
                 {checksums} checksum(s):{entry}. `checksum` is optional in cargo-deny's schema, \
                 so leaving it out is a one-token edit that keeps the gate green while unpinning \
                 the bytes: measured on this tree (SAK-237), all eight entries written as \
                 `allow = [{{ path }}]` with no checksum exit 0 with `bans ok` and zero \
                 findings. The point of this list is that a new upstream binary has to be looked \
                 at, which is what the checksum forces — regenerate with `sha256sum \
                 ~/.cargo/registry/src/*/protoc-bin-vendored-*-<version>/bin/*`."
            ));
        }
    }

    Ok(())
}

/// `Err` if `[licenses] include-build` has been set to anything but `true`.
///
/// This is the inverse of `check_dev_licenses_are_checked` above, and the
/// direction is the whole point. `include-dev` defaults to `false`, so the key
/// being absent is the hole and the guard demands the line. `include-build`
/// defaults to `true`, so the key being absent is *correct* and the guard has
/// nothing to demand — it only has to fail when someone writes the line down
/// with a `false` on it. Getting that backwards one way forces a harmless
/// redundant line into the file; getting it backwards the other way guards
/// nothing at all.
///
/// Measured on the stack tip (SAK-238) with the checked set counted using
/// `allow = []`, which rejects every crate the gate actually looks at, rather
/// than with `cargo deny list`:
///
///   no key (what ships)        217 crates
///   `include-build = true`     217 crates
///   `include-build = false`    196 crates
///
/// The 21 that disappear are `protoc-bin-vendored` and its eight
/// platform crates, `prost-build`, `tonic-build`, and the build-only tail they
/// pull (`autocfg`, `cc`, `find-msvc-tools`, `fixedbitset`, `multimap`,
/// `petgraph`, `prettyplease`, `regex`, `rustversion`, `shlex`). They are the
/// same eight protoc binaries `[bans.build]` pins by checksum one table down:
/// one line here and the crate that ships the `protoc` our `build.rs` executes
/// stops having its licence looked at.
///
/// The count is not the evidence, though. Measured with `smartstring = "=1.0.1"`
/// — MPL-2.0, a copyleft licence this allow list deliberately excludes —
/// *inserted into* the `[build-dependencies]` table (appending it to the end of
/// `Cargo.toml` lands it inside the last `[[test]]`, where cargo ignores it
/// without a word), confirmed a pure build dependency with
/// `cargo tree --locked -i smartstring` showing `[build-dependencies] └──
/// fireup` and no other edge. Tree held fixed, only this key moving:
///
///   no key (what ships)             exit 4   `error[rejected]` MPL-2.0
///   `include-build = false`         exit 0   `licenses ok`          <- hole
///   `include-build = true`          exit 4   control: the key existing is not
///                                            what makes it red
///   + `MPL-2.0` in `allow`          exit 0   control: the red is the licence
///                                            identifier, not the tree edit
///   + `[graph] exclude-dev = true`  exit 4   control: a build dependency is
///                                            not a dev dependency
///
/// The third arm is why the second one means anything, and the fifth is why
/// this guard needs one assertion where `include-dev` needed two. `include-dev`
/// has a kill switch a table away — `[graph] exclude-dev` empties the graph of
/// dev dependencies with `include-dev = true` still sitting in the file — and
/// `include-build` has no counterpart. `[graph]` takes seven keys (`targets`,
/// `exclude`, `features`, `all-features`, `no-default-features`, `exclude-dev`,
/// `exclude-unpublished`); there is no `exclude-build` among them, and
/// `exclude-dev` was measured above to leave the build dependency in place.
///
/// Nothing in the `deny` job reports this. Replaying the job's own step body on
/// the shipping tree, the shipping config, `include-build = false` and
/// `include-build = true` are all exit 0 with 0 errors and the same 6 warnings
/// — narrowing the gate makes it quieter, never louder, which is the direction
/// no one watching CI ever sees.
///
/// Why the key is deliberately *not* written out as `include-build = true`:
/// same reason `[graph] targets` is absent rather than listed (SAK-235). The
/// default is already the stricter reading, so the line would restate it and
/// add one more place a future edit can flip. The guard covers both shapes.
fn check_build_licenses_are_checked(deny: &str) -> Result<(), String> {
    match setting(deny, "include-build").as_deref() {
        // Absent is cargo-deny's default, and its default is the strict one.
        None | Some("true") => Ok(()),
        Some("false") => Err(
            "deny.toml sets `[licenses] include-build = false`, which drops build dependencies \
             out of the licence check — the allow list then says nothing about the crates \
             Fireup's `build.rs` compiles and runs. Counted with `allow = []`, the checked set \
             goes from 217 crates to 196; the 21 that vanish are `protoc-bin-vendored` and its \
             eight platform crates, `prost-build`, `tonic-build` and their build-only tail, so \
             the crate shipping the `protoc` binary the build executes stops being licence \
             checked while `[bans.build]` below is still pinning its checksum. Measured on this \
             tree (SAK-238) with the MPL-2.0 `smartstring` 1.0.1 inserted into \
             `[build-dependencies]`, a pure build dependency by `cargo tree -i`: with the key \
             absent and with `include-build = true` the check exits 4 with `error[rejected]`, \
             and the same tree at `false` exits 0 with `licenses ok`. The `deny` job reports \
             nothing either way — shipping, `false` and `true` are all exit 0 on the unmutated \
             tree. Delete the line: the default is true."
                .into(),
        ),
        Some(value) => Err(format!(
            "deny.toml has a live `[licenses] include-build = {value}`, a value this guard was \
             not measured at, so it must not report build dependencies as licence checked. \
             cargo-deny rejects a non-boolean here by itself — measured on this tree (SAK-238), \
             `\"false\"`, `\"maybe\"` and `0` each exit 1 with `error[wanted]` — so today this \
             branch is belt and braces. It is here so that a form this reader does not \
             understand can never be the thing that makes the guard pass. Delete the key: \
             cargo-deny's default is true, which is what this gate wants."
        )),
    }
}

/// The name of the step in `ci.yml`'s `deny` job that logs accepted advisories.
const LOG_STEP: &str = "Accepted advisory exceptions";

/// The literal that step greps `deny.toml` for, read out of `ci.yml`.
///
/// Read rather than written down a second time, because the string *is* the
/// contract. The guard below enforces a shape on `ignore` entries for one
/// reason only — that this `grep` can see them — so a test carrying its own
/// copy of the pattern would quietly stop describing CI the first time the step
/// changed, and would then demand a shape nobody looks for.
///
/// `Err` if the step is gone or its `run:` body is not the single `grep` over
/// `deny.toml` this reader knows. Either one voids the step's promise, which is
/// the thing the guard exists to keep true, so neither may read as a pass.
fn log_step_pattern(ci: &str) -> Result<String, String> {
    let lines: Vec<&str> = ci.lines().collect();
    let needle = format!("- name: {LOG_STEP}");
    let at = lines
        .iter()
        .position(|line| line.trim() == needle)
        .ok_or_else(|| {
            format!(
                "ci.yml has no `- name: {LOG_STEP}` step. That step is the only \
                 thing that puts an accepted advisory in a run's log — \
                 cargo-deny itself prints nothing at all for one — so without \
                 it there is no promise left for this guard to keep. If the \
                 step was removed on purpose, remove this test with it and say \
                 in deny.toml that accepted exceptions are invisible."
            )
        })?;

    // The body is the next `run:` in the step, before the next step begins.
    let body = lines[at + 1..]
        .iter()
        .take_while(|line| !line.trim().starts_with("- name:"))
        .find_map(|line| line.trim().strip_prefix("run:"))
        .ok_or_else(|| format!("the `{LOG_STEP}` step in ci.yml has no `run:` body"))?
        .trim();

    if !body.starts_with("grep ") || !body.contains("deny.toml") {
        return Err(format!(
            "the `{LOG_STEP}` step in ci.yml does not run a `grep` over \
             deny.toml any more; its body is `{body}`. This guard only knows \
             how to check that `ignore` entries are visible to a grep, so an \
             unfamiliar step has to fail rather than be assumed equivalent."
        ));
    }

    let (_, rest) = body.split_once('\'').ok_or_else(|| {
        format!("the `{LOG_STEP}` step's grep in ci.yml has no single-quoted pattern: `{body}`")
    })?;
    let (pattern, _) = rest.split_once('\'').ok_or_else(|| {
        format!("the `{LOG_STEP}` step's grep pattern in ci.yml is never closed: `{body}`")
    })?;
    if pattern.is_empty() {
        return Err(format!(
            "the `{LOG_STEP}` step in ci.yml greps for an empty pattern, which matches every line"
        ));
    }
    Ok(pattern.to_string())
}

/// A live `[advisories] ignore` entry and the source lines it occupies.
#[derive(Debug, PartialEq, Eq)]
struct IgnoreEntry {
    /// The entry's text, comments stripped and newlines flattened to spaces.
    text: String,
    /// The raw source lines the entry's text came from, in order.
    ///
    /// More than one means the entry is spread down the file, which is the
    /// shape the log step's line-oriented `grep` cannot report whole.
    lines: Vec<String>,
}

/// Every element of `[advisories] ignore`, one `IgnoreEntry` each.
///
/// `setting_array` is no use here for the same reason it was no use for
/// `[bans.build] bypass`: it splits on commas, and an entry of this array is an
/// inline table full of them. This reader also has to keep the *line* each
/// character came from, which no other reader in this file needs.
///
/// `Some(vec![])` means there are no accepted advisories — either `ignore = []`
/// or no key at all, which is cargo-deny's default and the same thing.
///
/// `None` means there is a live `ignore` line whose value this reader cannot
/// read as an array of entries. The caller turns that into a failure rather
/// than a pass, for the reason SAK-235 gave for `[graph] targets`. A `#` or a
/// backslash escape inside a reason string reaches this path — comments are
/// stripped per line, and escapes are not implemented. Failing closed on both
/// is the right answer anyway: the log step prints the whole line, so a reason
/// needing either one needs a reader smarter than `grep` too.
fn ignore_entries(deny: &str) -> Option<Vec<IgnoreEntry>> {
    let raw: Vec<&str> = deny.lines().collect();

    // Every character of the array, from its `[` to the matching `]`, tagged
    // with the line it came from.
    let mut tagged: Vec<(char, usize)> = Vec::new();
    let mut depth = 0usize;
    let mut open = false;
    let mut closed = false;
    for (number, line) in raw.iter().enumerate() {
        let code = line.split('#').next().unwrap_or("");
        let chunk = if open {
            // The line break is a separator. `bypass_entries` pushes a space
            // for the same reason: without it two lines' text is glued.
            tagged.push((' ', number));
            code
        } else {
            let Some((candidate, value)) = code.split_once('=') else {
                continue;
            };
            if candidate.trim() != "ignore" {
                continue;
            }
            if !value.trim_start().starts_with('[') {
                return None;
            }
            open = true;
            value
        };
        for ch in chunk.chars() {
            match ch {
                '[' => depth += 1,
                ']' => depth = depth.checked_sub(1)?,
                _ => {}
            }
            tagged.push((ch, number));
            if depth == 0 && ch == ']' {
                closed = true;
                break;
            }
        }
        if closed {
            break;
        }
    }

    if !open {
        return Some(Vec::new());
    }
    // An array that never closes is not readable either.
    if !closed {
        return None;
    }

    let first = tagged.iter().position(|&(ch, _)| ch == '[')?;
    let last = tagged.iter().rposition(|&(ch, _)| ch == ']')?;
    let inner = tagged.get(first + 1..last)?;

    /// Ends the entry being collected, if there is one.
    fn flush(
        entries: &mut Vec<IgnoreEntry>,
        raw: &[&str],
        text: &mut String,
        lines: &mut Vec<usize>,
    ) {
        if !text.trim().is_empty() {
            entries.push(IgnoreEntry {
                text: text.split_whitespace().collect::<Vec<_>>().join(" "),
                lines: lines
                    .iter()
                    .map(|&number| raw[number].to_string())
                    .collect(),
            });
        }
        text.clear();
        lines.clear();
    }

    let mut entries: Vec<IgnoreEntry> = Vec::new();
    let mut text = String::new();
    let mut lines: Vec<usize> = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for &(ch, number) in inner {
        // Whitespace does not put a line in the entry's span: the newline after
        // a one-line entry's comma belongs to no entry.
        if !ch.is_whitespace() && !lines.contains(&number) {
            lines.push(number);
        }
        // An escape would make the quote tracking below wrong, and this reader
        // does not implement TOML's escapes. Unreadable is the honest answer.
        if ch == '\\' {
            return None;
        }
        if let Some(opening) = quote {
            if ch == opening {
                quote = None;
            }
            text.push(ch);
            continue;
        }
        match ch {
            // Both TOML string forms, so that a `'…'` value cannot hide a
            // comma or a brace from the splitter.
            '"' | '\'' => {
                quote = Some(ch);
                text.push(ch);
            }
            '{' | '[' => {
                depth += 1;
                text.push(ch);
            }
            '}' | ']' => {
                depth = depth.checked_sub(1)?;
                text.push(ch);
            }
            ',' if depth == 0 => flush(&mut entries, &raw, &mut text, &mut lines),
            _ => text.push(ch),
        }
    }
    if quote.is_some() || depth != 0 {
        return None;
    }
    flush(&mut entries, &raw, &mut text, &mut lines);
    Some(entries)
}

/// The quoted value of `key` in `entry`, without its quotes.
///
/// `None` when the key is absent or its value is not a quoted string. The key
/// is matched with the same boundary rule as `count_key`, so `reason` cannot be
/// satisfied by some longer name ending in it.
fn string_value(entry: &str, key: &str) -> Option<String> {
    let bytes = entry.as_bytes();
    let mut from = 0;
    while let Some(at) = entry[from..].find(key) {
        let start = from + at;
        let end = start + key.len();
        let joined_to_a_longer_name = start > 0
            && matches!(bytes[start - 1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-');
        if !joined_to_a_longer_name {
            if let Some(value) = entry[end..].trim_start().strip_prefix('=') {
                let value = value.trim_start();
                let quote = value.chars().next()?;
                if quote != '"' && quote != '\'' {
                    return None;
                }
                let body = &value[quote.len_utf8()..];
                return body.find(quote).map(|close| body[..close].to_string());
            }
        }
        from = end;
    }
    None
}

/// `Some(date)` if `text` carries an ISO-8601 calendar date, `YYYY-MM-DD`.
///
/// An advisory id cannot be mistaken for one. `RUSTSEC-2026-0258` is four
/// digits, a dash and four more, so the scan runs out of shape where the day
/// part should start — which is why this looks inside the reason string rather
/// than at the whole line.
fn review_date(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    for start in 0..bytes.len() {
        if start > 0 && bytes[start - 1].is_ascii_digit() {
            continue;
        }
        let Some(window) = text.get(start..start + 10) else {
            continue;
        };
        let shaped = window.as_bytes().iter().enumerate().all(|(at, byte)| {
            if at == 4 || at == 7 {
                *byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        });
        if !shaped || bytes.get(start + 10).is_some_and(u8::is_ascii_digit) {
            continue;
        }
        let month: u32 = window[5..7].parse().ok()?;
        let day: u32 = window[8..10].parse().ok()?;
        if (1..=12).contains(&month) && (1..=31).contains(&day) {
            return Some(window.to_string());
        }
    }
    None
}

/// `Err` unless every live `[advisories] ignore` entry is one line that the
/// `deny` job's log step can `grep`, carrying its reason and reconsider date on
/// that same line.
///
/// cargo-deny prints **nothing at all** for an advisory it ignored — not the
/// id, not the reason, not a count. `ci.yml`'s `deny` job covers for that with
/// one step whose stated purpose is to leave the accepted exceptions and their
/// reconsider dates in every run's log rather than only in a config file, and
/// it does that with a plain `grep` for the pattern `log_step_pattern` reads.
///
/// That promise holds for exactly one way of writing an entry. Rewrite the same
/// entry in another valid TOML spelling and cargo-deny suppresses the advisory
/// just as before while the log goes quiet. Measured on the tree that had a
/// live entry (#77, h2 RUSTSEC-2026-0258), reading the gate and the log step
/// independently — one alone cannot tell "the entry works" from "there is no
/// advisory":
///
/// | `ignore` entry                  | `cargo deny check advisories` | log step          |
/// |---------------------------------|-------------------------------|-------------------|
/// | the shipped one-line table      | exit 0, suppressed            | id, reason, date  |
/// | split across two lines          | exit 0, suppressed            | id only           |
/// | `id` in single quotes           | exit 0, suppressed            | `none`            |
/// | `id="RUSTSEC-…"`, no spaces     | exit 0, suppressed            | `none`            |
/// | entry deleted (control)         | **exit 1**, advisory fires    | `none`            |
///
/// The control is what makes the middle rows mean anything: with no entry the
/// gate goes red, so the green in the single-quote and no-space rows is proof
/// the entry really is doing its job — and all three print the identical
/// `none`, so the log cannot distinguish "one advisory accepted" from "none".
/// The two-line row is quieter still: the id appears, so an exception is
/// visible, but the reason and the reconsider date — the step's whole reason
/// for existing — are on a line the grep never prints.
///
/// So the fix is not a cleverer `grep` in `ci.yml`. No test in this repository
/// reads a workflow's shell (SAK-221), so a smarter step would be unguarded;
/// and a `grep` is the right tool for a one-line entry. Instead the *shape*
/// the grep depends on becomes the invariant, checked here, and the step stays
/// simple.
///
/// Today this costs nothing: SAK-220's migration removed the h2 entry, so
/// `ignore` is empty and this guard changes no behaviour. It bites the next
/// person to accept an advisory, which is the moment the log is needed.
///
/// (This is deliberately unlike the judgement on `[licenses]
/// confidence-threshold`, which SAK-238 left unguarded as inert. That key's
/// value was not a contract. "A shape the log step can read" is one, and it
/// becomes load-bearing the instant `ignore` is used.)
fn check_ignore_entries_are_logged(deny: &str, pattern: &str) -> Result<(), String> {
    let Some(entries) = ignore_entries(deny) else {
        return Err(
            "deny.toml has a live `[advisories] ignore` line this test cannot read as an array \
             of entries, so it cannot report whether the `deny` job's log step can see them. \
             That is a failure and not a pass: the step is the only place an accepted advisory \
             is ever printed, because cargo-deny says nothing about one. Write the list as \
             one-line inline tables, and keep `#` out of the reason strings."
                .into(),
        );
    };

    for entry in &entries {
        if entry.lines.len() != 1 {
            return Err(format!(
                "an ignore entry spans more than one line, so the `deny` job's log step can \
                 never print it whole:\n{}\n\
                 The step greps `{pattern}` and prints the matching *line*. Split like this, \
                 the id is logged and the reason and reconsider date are not — and those are \
                 the step's whole purpose, since cargo-deny prints nothing at all for an \
                 advisory it ignored. Put the entry on one line.",
                entry
                    .lines
                    .iter()
                    .map(|line| format!("    {}", line.trim_end()))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        let line = entry.lines[0].trim();
        if !line.contains(pattern) {
            return Err(format!(
                "an ignore entry does not match the log step's grep, so accepting this advisory \
                 is invisible in every run's log:\n    {line}\n\
                 The step in ci.yml greps for the literal `{pattern}`. Valid TOML that misses \
                 it includes a single-quoted id (`id = 'RUSTSEC-…'`), no spaces around the \
                 `=` (`id=\"RUSTSEC-…\"`), and a bare string element (`\"RUSTSEC-…\"`). \
                 cargo-deny suppresses the advisory in all of them, so nothing else goes red: \
                 the gate stays green and the log prints `none`, exactly as it would with no \
                 entry at all. Write the id as `{pattern}…\"`."
            ));
        }

        let Some(reason) = string_value(&entry.text, "reason") else {
            return Err(format!(
                "an ignore entry has no `reason = \"…\"` string:\n    {line}\n\
                 The reason is what the `deny` job's log step is for. cargo-deny does not \
                 require it and prints nothing either way, so an entry without one accepts an \
                 advisory with no recorded argument and no reconsider date anywhere."
            ));
        };

        if review_date(&reason).is_none() {
            return Err(format!(
                "an ignore entry's reason carries no review date, so nothing says when the \
                 acceptance should be re-argued:\n    {line}\n\
                 Add an ISO-8601 date (`YYYY-MM-DD`) to the reason, on this same line — \
                 `reason = \"… Reconsider by 2027-02-06.\"`. cargo-deny has no expiry feature \
                 for `ignore`, so the date is a promise rather than a gate; the log step \
                 putting it in front of someone on every run is the whole mechanism, and a \
                 reason without one makes the acceptance permanent by default."
            ));
        }
    }
    Ok(())
}

/// A `permissions:` key in a workflow, and the block it owns.
#[derive(Debug, PartialEq, Eq)]
struct PermissionsKey {
    /// Columns of indentation before the key. `0` is the workflow top level;
    /// anything deeper is a job's own block, which overrides the top-level one
    /// for that job and is *not* capped by it.
    indent: usize,
    /// Whatever followed the colon on the same line, comment stripped. Empty
    /// when the scopes are a block mapping on the lines below.
    inline: String,
    /// The `scope: value` lines of a block mapping, comments stripped. Empty
    /// for an inline form — and also for a key with nothing under it at all,
    /// which is why `inline` has to be consulted first.
    entries: Vec<String>,
}

/// `line` with any `#` comment and trailing whitespace removed.
///
/// Good enough because a `permissions:` block is scopes and the words `read`,
/// `write` and `none`: there is nowhere for a `#` to appear inside a quoted
/// string and be data. A value this does mangle ends up unreadable rather than
/// read wrongly, and unreadable fails closed below.
fn without_comment(line: &str) -> &str {
    match line.split_once('#') {
        Some((before, _)) => before.trim_end(),
        None => line.trim_end(),
    }
}

/// Every `permissions:` key in a workflow, at any depth, in file order.
///
/// Line-oriented for the same reason the rest of this file is: it keeps the
/// test target free of a YAML dependency, and the grammar in question is two
/// levels of plain `key: value`.
///
/// A commented-out key is not a key. That is not a nicety — a workflow that
/// documents its own settings carries the word `permissions:` in prose, and a
/// reader that matched anywhere in a line would find the comment, read the
/// value beside it, and report on a setting that is not in effect.
fn permissions_keys(workflow: &str) -> Vec<PermissionsKey> {
    let lines: Vec<&str> = workflow.lines().collect();
    let mut keys = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        let bare = without_comment(line);
        let body = bare.trim_start();
        let Some(inline) = body.strip_prefix("permissions:") else {
            continue;
        };
        let indent = bare.len() - body.len();

        // The block is every deeper-indented line until the first line at this
        // depth or shallower. Blank and comment-only lines belong to neither,
        // so they are skipped rather than ending the block.
        let mut entries = Vec::new();
        for next in &lines[at + 1..] {
            let bare = without_comment(next);
            let body = bare.trim_start();
            if body.is_empty() {
                continue;
            }
            if bare.len() - body.len() <= indent {
                break;
            }
            entries.push(body.to_string());
        }

        keys.push(PermissionsKey {
            indent,
            inline: inline.trim().to_string(),
            entries,
        });
    }
    keys
}

/// `Ok` if `scope: value` grants no more than reading.
///
/// `read` and `none` are the only two values that pass. `write` is the point of
/// the check; anything else is a value this reader does not understand, and is
/// refused rather than guessed at — `contents: Read` and `contents: "read"` are
/// both real spellings that GitHub rejects or ignores, and neither may read as
/// a pass.
fn scope_is_read_only(pair: &str, which: &str) -> Result<(), String> {
    let Some((scope, value)) = pair.split_once(':') else {
        return Err(format!(
            "{which} has an entry this reader cannot parse as `scope: value`: `{pair}`. \
             Refused rather than assumed harmless: the whole job of this guard is to \
             read what the token can do, and a shape it does not understand is a shape \
             it cannot clear."
        ));
    };
    let (scope, value) = (scope.trim(), value.trim());
    match value {
        "read" | "none" => Ok(()),
        "write" => Err(format!(
            "{which} grants `{scope}: write`, so this workflow's GITHUB_TOKEN can change \
             the repository. The `deny` job fetches and executes a third-party binary with \
             a checkout whose `.git/config` holds that token (actions/checkout defaults to \
             `persist-credentials: true`), so a write scope here is a write scope handed to \
             downloaded code. If a job genuinely needs to write, give that one job its own \
             `permissions:` and say in a comment what it writes — do not widen the workflow."
        )),
        other => Err(format!(
            "{which} sets `{scope}` to `{other}`, which is not one of `read`, `write` or \
             `none`. This guard fails closed on a value it cannot read rather than passing \
             it: `Read` and `\"read\"` both look right and neither is the word GitHub \
             accepts. Write it as a bare lower-case `read` (or `none`)."
        )),
    }
}

/// `Ok` if this one `permissions:` key grants no more than reading.
fn permissions_key_is_read_only(key: &PermissionsKey) -> Result<(), String> {
    let which = if key.indent == 0 {
        "the top-level `permissions:` in ci.yml".to_string()
    } else {
        format!(
            "a job-level `permissions:` in ci.yml (indented {} columns)",
            key.indent
        )
    };

    if !key.inline.is_empty() {
        // `read-all`, `write-all`, or a flow mapping on the same line.
        if let Some(inner) = key
            .inline
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))
        {
            // `{}` is every scope at `none`, which is stricter than `read`.
            if inner.trim().is_empty() {
                return Ok(());
            }
            for pair in inner.split(',') {
                scope_is_read_only(pair.trim(), &which)?;
            }
            return Ok(());
        }
        return match key.inline.as_str() {
            "read-all" => Ok(()),
            "write-all" => Err(format!(
                "{which} is `write-all`, which grants every scope at write. That is the \
                 widest token GitHub will issue and it is handed to a workflow whose \
                 `deny` job runs a downloaded binary. Use `contents: read`."
            )),
            other => Err(format!(
                "{which} is `{other}`, which this reader does not understand. The forms it \
                 knows are `read-all`, `write-all`, `{{}}`, and a block of `scope: value` \
                 lines. Unreadable fails closed here, because the alternative is reporting \
                 on a token nobody has measured."
            )),
        };
    }

    if key.entries.is_empty() {
        return Err(format!(
            "{which} has no scopes under it. An empty key is not a least-privilege \
             declaration — it is a YAML null, which GitHub rejects when it loads the \
             workflow, so the run that was supposed to be gated does not happen at all. \
             Write `contents: read` under it, or `permissions: {{}}` if the intent really \
             is no scopes."
        ));
    }

    // Every scope, not just the first one that looks safe. A write hides
    // perfectly well as the second entry under a reassuring `contents: read`.
    for entry in &key.entries {
        scope_is_read_only(entry, &which)?;
    }
    Ok(())
}

/// `Ok` if `ci.yml` declares a top-level token that can only read.
///
/// `ci.yml` shipped with no `permissions:` at all, which does not mean the
/// token was wide — measured on SAK-240, `default_workflow_permissions` for
/// this repository is `read` — it means **the tree does not say**. The scope of
/// the token every job in this workflow gets was decided by a checkbox in the
/// repository's web settings, outside the tree, changeable with no commit, no
/// pull request and no review, and nothing in the repository would report it.
///
/// That matters here more than in a workflow that only compiles things. The
/// comment at the top of `ci.yml` names the `deny` job as the one job that
/// deliberately goes to the network, and what it does there is download a
/// third-party binary and run it — in a workspace where `actions/checkout`
/// has, by default, written the token into `.git/config`.
///
/// Three things this asserts that a laxer reading would miss:
///
/// * **The value, not the key.** A guard that only checked that a
///   `permissions:` line exists would accept `contents: write`. The arms below
///   pair every failing value with the passing one, because a guard whose
///   direction is wrong looks identical to a correct one until someone widens
///   the setting. (SAK-237's `include-dependencies` and SAK-238's
///   `include-build` default opposite ways; getting a direction backwards is
///   not hypothetical in this file.)
/// * **Every scope.** `contents: read` beside `packages: write` reads safe at a
///   glance and is not. Same shape as SAK-237's optional `checksum` sub-key:
///   the thing that unpins the gate is the token you did not look at.
/// * **Every block, not only the top-level one.** A job's `permissions:`
///   replaces the workflow's for that job and is *not* capped by it — only the
///   repository-wide maximum caps it — so a job-level `permissions: write-all`
///   would hand the `deny` job a write token with the reassuring
///   `contents: read` still sitting at the top of the file. That is the
///   kill-switch-beside-the-policy shape from SAK-231's
///   `disable-yank-checking`, and it is why this reads all of them.
///
/// Deliberately strict in one direction: the top-level key is *required*.
/// Putting `contents: read` on the `deny` job alone would be safe for that job
/// and this still fails it. The failure mode of being wrong about that is one
/// harmless line, against a default that lives where nobody reads it.
///
/// Note what this cannot do: it is a statement about the tree, not about the
/// repository setting. A top-level `contents: read` caps the token regardless
/// of what the web checkbox says, which is exactly why declaring it is the fix
/// — but if someone widens `default_workflow_permissions` and deletes these
/// lines in the same change, it is the deleted lines this reports, not the
/// checkbox.
fn check_workflow_token_is_read_only(ci: &str) -> Result<(), String> {
    let keys = permissions_keys(ci);

    let top_level = keys.iter().filter(|key| key.indent == 0).count();
    if top_level == 0 {
        return Err(
            "ci.yml declares no top-level `permissions:`, so the scope of every job's \
             GITHUB_TOKEN is whatever the repository's `default_workflow_permissions` web \
             setting happens to be. That setting is outside this tree: it can be widened to \
             `write` with no commit, no pull request and no review, and nothing in the \
             repository reports it. It reads `read` today (measured, SAK-240) — the problem \
             is that no file here says so. Add, above `jobs:`:\n\
             \n    permissions:\n      contents: read\n\n\
             which caps the token for the whole workflow in the tree, where a change to it \
             is a diff someone reviews."
                .to_string(),
        );
    }
    if top_level > 1 {
        return Err(format!(
            "ci.yml has {top_level} top-level `permissions:` keys. YAML duplicate keys are \
             resolved last-one-wins by some loaders and rejected by others, so which token \
             this workflow asks for is not something a reader can tell. Keep one."
        ));
    }

    for key in &keys {
        permissions_key_is_read_only(key)?;
    }
    Ok(())
}

/// The name of the step in `ci.yml`'s `deny` job that runs the gate itself.
const DENY_STEP: &str = "cargo deny check";

/// The checks that step names today, and that `deny.toml` configures.
///
/// Everything this file asserts about `deny.toml` is a setting for one of
/// these four. A name missing from the command is that whole column of
/// settings switched off.
const DENY_CHECKS: &[&str] = &["advisories", "licenses", "bans", "sources"];

/// Shell characters that end a word, so that `sources||true` is still two.
const SHELL_OPERATORS: &[char] = &['|', '&', ';', '(', ')', '<', '>'];

/// The `deny` job's gate command, as much of it as this guard reads.
#[derive(Debug, PartialEq, Eq)]
struct DenyStep {
    /// The command as one logical line: `\`-continuations joined, comment
    /// stripped. Kept as text because the `||` question is about an operator,
    /// not about a word.
    command: String,
    /// The words after the `check` subcommand, cut at whitespace *and* at the
    /// shell operators above. Not a parse of cargo-deny's command line — it
    /// deliberately does not try to tell a positional check name from a flag's
    /// value, because it does not have to: see `check_every_deny_check_runs`.
    arguments: Vec<String>,
    /// The words *before* the `check` subcommand, same cutting rules. Includes
    /// the binary name and `--locked`.
    ///
    /// A separate field because cargo-deny's command line has two halves and
    /// they hold different things. `-A`/`-W`/`-D` are `check`'s own options and
    /// are only accepted after it; the options that redirect *what* gets
    /// checked — `--config`, `--metadata-path`, `--target`, `--exclude`,
    /// `--exclude-dev` — are top-level and are only accepted before it.
    /// Measured on this tree (SAK-248): each of those six in the post-`check`
    /// position is `unexpected argument`, exit 2, nothing checked; each in the
    /// pre-`check` position runs. So a guard reading `arguments` alone cannot
    /// see any of them, which is exactly the state this file was in.
    options: Vec<String>,
    /// `true` if a `set … pipefail` line runs earlier in the same `run:` body.
    ///
    /// Read, not assumed. GitHub prints the shell it gave every step, and for
    /// this workflow -- which declares no `shell:` -- run 42362531653 says
    /// `shell: /usr/bin/bash -e {0}` on all ten of them. That is `-e` with no
    /// `-o pipefail`, so a pipe takes the status of its *last* stage and the
    /// gate's verdict is thrown away. This step's body has no `set` line at
    /// all; the `Install cargo-deny` step's has `set -euo pipefail`, which is
    /// where that protection does exist.
    pipefail: bool,
}

/// The `deny` job's `cargo-deny … check` command, read out of `ci.yml`.
///
/// Read rather than written down a second time for the same reason
/// `log_step_pattern` is: the command *is* the contract. `deny.toml` is a pile
/// of settings, and which of them are in force is decided here and nowhere
/// else.
///
/// `Err` if the step is gone, if there is not exactly one of it, or if its
/// `run:` body is not a single `cargo-deny … check` this reader understands.
/// Each of those voids the promise the guard below keeps, so none may read as a
/// pass — the same fail-closed call `log_step_pattern` and
/// `check_graph_covers_every_target` make.
fn deny_step_checks(ci: &str) -> Result<DenyStep, String> {
    let lines: Vec<&str> = ci.lines().collect();
    let needle = format!("- name: {DENY_STEP}");
    let named: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| without_comment(line).trim() == needle)
        .map(|(at, _)| at)
        .collect();
    let at = match named[..] {
        [at] => at,
        [] => {
            return Err(format!(
                "ci.yml has no `- name: {DENY_STEP}` step. That step is the only thing in \
                 this repository that runs cargo-deny, which makes every assertion in this \
                 file about deny.toml a statement about a tool nobody invokes. Note what \
                 this guard cannot do: deleting the step takes the gate away entirely, and \
                 all a test can do about that is refuse to pass (SAK-221). If the step was \
                 renamed, point DENY_STEP at its new name."
            ));
        }
        _ => {
            return Err(format!(
                "ci.yml has {} steps named `{DENY_STEP}`. Which one is the gate — and \
                 therefore which check list is in force — is not something a reader can \
                 tell, so this fails rather than reporting on whichever came first.",
                named.len()
            ));
        }
    };

    // The step is everything up to the next item in the `steps:` sequence.
    // `- name:` is not enough on its own: a `- uses:` step has no name.
    let step: Vec<&str> = lines[at + 1..]
        .iter()
        .copied()
        .take_while(|line| {
            let body = without_comment(line).trim();
            body != "-" && !body.starts_with("- ")
        })
        .collect();

    let run_at = step
        .iter()
        .position(|line| without_comment(line).trim_start().starts_with("run:"))
        .ok_or_else(|| format!("the `{DENY_STEP}` step in ci.yml has no `run:` body"))?;
    let run_line = without_comment(step[run_at]);
    let run_body = run_line.trim_start();
    let run_indent = run_line.len() - run_body.len();
    let inline = run_body.strip_prefix("run:").unwrap_or_default().trim();

    // Either `run: <command>` on the one line, or a block scalar whose body is
    // the deeper-indented lines under it — which is the form ci.yml uses, and
    // the reason this reader cannot be the one-liner `log_step_pattern` is.
    let mut raw: Vec<&str> = Vec::new();
    if inline.starts_with('|') || inline.starts_with('>') {
        for next in &step[run_at + 1..] {
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
            "the `{DENY_STEP}` step in ci.yml has an empty `run:` body, so the gate runs \
             nothing and the step exits 0."
        ));
    }

    // Join `\`-continuations: the shipped command is spread over three source
    // lines and is one command.
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

    let invocations: Vec<&String> = logical
        .iter()
        .filter(|line| line.contains("cargo-deny") || line.contains("cargo deny"))
        .collect();
    if invocations.len() != 1 {
        return Err(format!(
            "the `{DENY_STEP}` step in ci.yml does not run exactly one cargo-deny command \
             — this reader found {} in `{}`. An unfamiliar body fails rather than being \
             assumed equivalent: this guard's whole job is to read which checks that \
             command runs, and it cannot read a command it did not find.",
            invocations.len(),
            logical.join(" ; ")
        ));
    }
    let command = invocations[0].clone();

    let words: Vec<&str> = command
        .split(|c: char| c.is_whitespace() || SHELL_OPERATORS.contains(&c))
        .filter(|word| !word.is_empty())
        .collect();
    let check_at = words
        .iter()
        .position(|word| *word == "check")
        .ok_or_else(|| {
            format!(
                "the `{DENY_STEP}` step in ci.yml runs cargo-deny with no `check` \
                 subcommand: `{command}`. Without `check` there is no check list to read, \
                 and whatever that command does it is not running this repository's \
                 supply-chain policy."
            )
        })?;

    Ok(DenyStep {
        pipefail: logical
            .iter()
            .take_while(|line| line.as_str() != command.as_str())
            .any(|line| {
                let text = line.trim_start();
                text.starts_with("set ") && text.contains("pipefail")
            }),
        arguments: words[check_at + 1..]
            .iter()
            .map(|word| word.to_string())
            .collect(),
        options: words[..check_at]
            .iter()
            .map(|word| word.to_string())
            .collect(),
        command,
    })
}

/// The top-level options that change what the gate looks at, rather than how
/// loudly it complains about what it found.
///
/// `(short, long)`. `None` where cargo-deny gives no short form — note in
/// particular that `-c` is **`--color`**, measured, so a guard that treated it
/// as short for `--config` would redden a harmless colour flag.
const REDIRECTING_OPTIONS: &[(Option<&str>, &str)] = &[
    // Replaces the policy file outright. Every one of this file's `deny.toml`
    // assertions then describes a file cargo-deny does not read.
    (None, "--config"),
    // Supplies the crate graph from a file, so `cargo metadata` never runs —
    // which also makes `--locked` beside it inert.
    (None, "--metadata-path"),
    // cargo-deny's own help: "This option overrides the top-level `targets = []`
    // configuration value" — the key `check_graph_covers_every_target` (SAK-235)
    // asserts is absent from `deny.toml`. It is absent. This says it anyway.
    (Some("-t"), "--target"),
    // The CLI twin of `[graph] exclude`.
    (None, "--exclude"),
    // The CLI twin of `[graph] exclude-dev` — the kill switch that
    // `include-dev`'s guard (SAK-234) needs its second assertion for.
    (None, "--exclude-dev"),
];

/// The redirecting options present in the gate command, in the order found.
///
/// Matching is deliberately strict — exactly the flag, or the flag with `=`
/// glued to a value — and never a bare `starts_with`. Three measured reasons,
/// all on this tree:
///
/// * `--exclude-unpublished` is a prefix match for `--exclude` and is **not** a
///   hole: on a tree with a rejected licence it stays exit 5. A prefix reader
///   would redden it.
/// * `--all-features`, `--no-default-features`, `--workspace`, `--frozen` and
///   `--offline` all stay exit 5 on that same tree. None of them subtract.
/// * `--manifest-path` stays exit 4: it moves where `deny.toml` is looked up
///   too, so cargo-deny logs `unable to find a config path, falling back to
///   default config` and the default config allows no licence at all. It is
///   loud, not quiet, so it is not guarded here.
fn redirecting_options(step: &DenyStep) -> Vec<String> {
    let mut found = Vec::new();
    for word in &step.options {
        let hit = REDIRECTING_OPTIONS.iter().find(|(short, long)| {
            if word == long || word.starts_with(&format!("{long}=")) {
                return true;
            }
            match short {
                // `-t value` and `-tvalue` are both accepted (measured).
                Some(short) => {
                    word == short || (word.starts_with(short) && word.len() > short.len())
                }
                None => false,
            }
        });
        if let Some((_, long)) = hit {
            found.push((*long).to_string());
        }
    }
    found
}

/// `Ok` if the `deny` job's command runs every check `deny.toml` configures,
/// and if its verdict survives.
///
/// Drop one name from that command and that check's entire policy stops
/// running: `deny.toml` is unchanged so cargo-deny has nothing to say, there is
/// no diagnostic, and the step exits 0. Measured on this tree (SAK-243), each
/// against a `deny.toml`/`Cargo.toml` mutation the check in question catches:
///
/// | check | mutation | full list | list minus it |
/// |---|---|---|---|
/// | `licenses` | drop `"MIT"` from `allow` | exit 4, 48 errors | exit 0, 0 errors |
/// | `advisories` | an `ignore` entry matching nothing | exit 1, 1 error | exit 0 |
/// | `bans` | `tonic = "*"` | exit 2, 1 error | exit 0 |
/// | `sources` | `allow-registry = []` | exit 8, 216 errors | exit 0 |
///
/// The only trace in the log is one word missing from a summary line that
/// still reads `ok` (`advisories ok, bans ok, sources ok`). This is the same
/// shape as (3), (4) and (6) in the header: cargo-deny cannot report it,
/// because the thing that changed is its own invocation.
///
/// **Three assertions, and deliberately only three.**
///
/// 1. *Every* check in `DENY_CHECKS` is named, **or none is.** Given no check
///    names at all cargo-deny runs every check it has — measured, and it is
///    why "none" has to pass: omitting the list is *stricter* than the shipping
///    command, so a guard demanding the four names would be demanding the
///    longer spelling of a stronger gate.
/// 2. The command's exit is not sent to `||`.
/// 3. The command's exit is not sent to a **pipe**, unless the body turns
///    `pipefail` back on.
///
/// Deliberately **not** asserted: an unfamiliar extra check name. A check this
/// list has not heard of is the gate getting stricter, and a guard that
/// reddened on it would teach people to shorten the command — the exact
/// mistake SAK-241 undid in `the_permissions_reader_finds_exactly_the_live_keys`.
/// That is also why assertion 1 only looks for these four words among the
/// arguments and classifies nothing else: it never has to decide whether a
/// word is a positional check name or some flag's value.
///
/// Assertion 3 is a correction, not an addition (SAK-253). This doc used to
/// say `||` "and nothing else", on the strength of arms replayed under
/// `bash -e -o pipefail` — and attributed the pipe's safety to "the step's
/// `-o pipefail`", a line this step does not have. The runner's shell is
/// `bash -e` with **no** `pipefail`: measured from the `shell:` line GitHub
/// prints for every step of run 42362531653. Re-measured with `deny.toml`'s
/// licence allowlist broken, one variable being the shell:
///
/// | command tail | `bash -e` (the runner) | `bash -e -o pipefail` |
/// |---|---|---|
/// | *(none — shipped)* | 4 | 4 |
/// | `\| tee /dev/null` | **0** | 4 |
/// | `\| cat` | **0** | 4 |
/// | `\| tee out.log` | **0** | 4 |
/// | `> /dev/null` | 4 | 4 |
/// | `; true` | 4 | 4 |
/// | `\|\| true` | 0 | 0 |
///
/// With the shipped `deny.toml` every one of those is 0, so the red above is
/// the broken allowlist and the difference between the columns is the shell.
/// `;`, `&&` and a redirect are still allowed, still for the measured reason:
/// `-e` leaves before the `;`. And a pipe under a body that opens with
/// `set -o pipefail`, `set -eo pipefail` or `set -euo pipefail` is allowed,
/// because all three were measured to keep the 4 — the assertion has to be
/// satisfiable by something other than deleting the pipe, or it is a guard
/// teaching people that a working command is broken.
///
/// Also not asserted, and worth a ticket of its own rather than a guess here:
/// `continue-on-error: true` on this step would discard the verdict too. It is
/// a visible new key in a diff rather than a word going missing from a long
/// line, and it has not been measured on this tree.
fn check_every_deny_check_runs(step: &DenyStep) -> Result<(), String> {
    let named: Vec<&str> = DENY_CHECKS
        .iter()
        .copied()
        .filter(|check| step.arguments.iter().any(|argument| argument == check))
        .collect();

    if !named.is_empty() && named.len() != DENY_CHECKS.len() {
        let missing: Vec<&str> = DENY_CHECKS
            .iter()
            .copied()
            .filter(|check| !named.contains(check))
            .collect();
        return Err(format!(
            "the `{DENY_STEP}` step in ci.yml names {} but not {}: `{}`. A check \
             cargo-deny is not told to run is a policy that does not run — deny.toml \
             still configures it, cargo-deny still prints `ok` for the checks it did run, \
             and the step still exits 0. Measured (SAK-243): with `licenses` dropped from \
             that list and `\"MIT\"` dropped from deny.toml's allow list, the step exits 0 \
             with no errors, where the full list exits 4 with 48. Either name every check \
             in that command, or name none of them — cargo-deny with no check names runs \
             every check it has, which is stricter than the list above and passes here.",
            named.join(", "),
            missing.join(", "),
            step.command
        ));
    }

    if step.command.contains("||") {
        return Err(format!(
            "the `{DENY_STEP}` step in ci.yml sends the gate's exit status to `||`: `{}`. \
             That discards the verdict and the step exits 0 however the checks went, so \
             the job reports green on a tree cargo-deny has just failed. Measured \
             (SAK-243) with deny.toml's licence allowlist broken: the command alone exits \
             4, and with `|| true` after it exits 0. If something after the gate has to \
             run either way, use `;` — that keeps the failure, measured, because `-e` \
             leaves before the `;`. Not a pipe: this workflow declares no `shell:`, so \
             the runner gives it `bash -e` with no `-o pipefail` (run 42362531653), and \
             a pipe discards the exit too — the assertion below rejects one unless the \
             `run:` body opens with `set -o pipefail`.",
            step.command
        ));
    }

    if step.command.contains('|') && !step.pipefail {
        return Err(format!(
            "the `{DENY_STEP}` step in ci.yml pipes the gate's output: `{}`, and its \
             `run:` body does not turn `pipefail` on. The step then exits with the \
             *last* stage's status and cargo-deny's verdict is discarded, so the job \
             reports green on a tree the gate has just failed. This workflow declares no \
             `shell:`, so GitHub runs every step under `/usr/bin/bash -e {{0}}` — \
             measured from the `shell:` line of run 42362531653 — which is `-e` without \
             `-o pipefail`. Measured (SAK-253) with deny.toml's licence allowlist \
             broken: the command alone exits 4; `| tee /dev/null` exits 0; the same pipe \
             under a shell with `-o pipefail` exits 4. Put `set -o pipefail` above the \
             command in the same body — measured, and accepted here — or drop the pipe.",
            step.command
        ));
    }

    Ok(())
}

/// cargo-deny's two severity-*lowering* flags, each as `(short, long)`.
///
/// `-D`/`--deny` is deliberately absent. That is the strict direction and the
/// shipping command already uses it; a guard that reddened on it would be
/// reddening a stricter gate.
const SEVERITY_LOWERING_FLAGS: &[(&str, &str)] = &[("-A", "--allow"), ("-W", "--warn")];

/// The severity-lowering flags on the gate's command line, as written.
///
/// One entry per occurrence, with the diagnostic folded in so a failure can
/// quote the whole thing: `-A rejected` is two words, while `-Arejected` and
/// `--allow=rejected` are one. All four spellings were measured against
/// cargo-deny 0.20.2 and all four silence the policy, so all four are read.
///
/// The long form is matched only as exactly `--allow` or a `--allow=` prefix,
/// never as a bare prefix, so a future `--allow-<something-stricter>` is not
/// mistaken for this.
fn silenced_diagnostics(step: &DenyStep) -> Vec<String> {
    let mut silenced: Vec<String> = Vec::new();
    for (at, word) in step.arguments.iter().enumerate() {
        let separate = SEVERITY_LOWERING_FLAGS
            .iter()
            .any(|(short, long)| word == short || word == long);
        let attached = SEVERITY_LOWERING_FLAGS.iter().any(|(short, long)| {
            word.starts_with(&format!("{long}="))
                || (word.len() > short.len() && word.starts_with(short))
        });
        match (separate, attached) {
            // A flag written as its own word takes the next word as its value.
            (true, _) => match step.arguments.get(at + 1) {
                Some(value) => silenced.push(format!("{word} {value}")),
                None => silenced.push(word.clone()),
            },
            (false, true) => silenced.push(word.clone()),
            (false, false) => {}
        }
    }
    silenced
}

/// `Ok` if the `deny` job's command has not been told to stop failing on a
/// diagnostic.
///
/// `check_every_deny_check_runs` reads whether a whole check is named. One
/// level in from that: a check can be named, run, and still be unable to fail
/// the step. `-A <diagnostic>` drops the diagnostic to "allow" and `-W` to
/// "warn", and either overrides whatever `deny.toml` says — the config file is
/// not consulted about it. Measured on this tree, two checks so that neither
/// one's evidence is borrowed for the other:
///
/// | tree | command | step |
/// |---|---|---|
/// | `"MIT"` dropped from `allow` | shipped | exit 4, 48 errors, `licenses FAILED` |
/// | same | `+ -A rejected` | **exit 0, 0 errors, `licenses ok`** |
/// | same | `+ -W rejected` | **exit 0**, 54 warnings, `licenses ok` |
/// | same | `+ -A duplicate` (control) | exit 4, 48 errors — still red |
/// | `allow-registry = []` | shipped | exit 8, 216 errors, `sources FAILED` |
/// | same | `+ -A source-not-allowed` | **exit 0**, `sources ok` |
/// | same | `+ -A rejected` (control) | exit 8, 216 errors — still red |
///
/// The two control rows are what make the other four mean anything: silencing
/// the *wrong* diagnostic leaves the step red, so the green is the named
/// diagnostic going quiet and not the mere presence of a flag.
///
/// This is the same tool-invisible shape as the dropped check name. cargo-deny
/// cannot report it, because the thing that changed is its own invocation: the
/// step prints `advisories ok, bans ok, licenses ok, sources ok` and exits 0.
/// Measured on the real `ci.yml`: with `-A rejected` added to the gate command
/// this suite was still **165 passed**, and renaming the gate step in the same
/// file takes it to 7 failed — so the green was a missing assertion, not an
/// unread file.
///
/// Deliberately **not** asserted here: `--locked`. The `ci.yml` comment says it
/// is there to "check the exact versions this commit pins", and dropping it is
/// certainly a weaker promise, but two attempts to build a tree where the
/// committed lockfile is red and a fresh resolution is green both failed on
/// this graph — `cargo update -p shlex --precise 1.2.0` lands on 1.3.0, because
/// `cc` floors shlex at `^1.3`, so the vulnerable version is unreachable here.
/// With no measured fail-open, demanding the literal flag would also redden
/// `--frozen` and `--offline`, which are equivalent-or-stricter spellings — the
/// `set -euo pipefail`-as-a-string mistake SAK-246 avoided. It wants its own
/// ticket and its own measurement, not a guess in this one.
fn check_no_policy_is_silenced(step: &DenyStep) -> Result<(), String> {
    let silenced = silenced_diagnostics(step);
    if silenced.is_empty() {
        return Ok(());
    }
    Err(format!(
        "the `{DENY_STEP}` step in ci.yml lowers the severity of {} on its own command \
         line: `{}`. A diagnostic dropped to allow/warn cannot fail the step however \
         `deny.toml` is configured — the flag wins, and deny.toml is not consulted. \
         Measured (SAK-247): with `\"MIT\"` dropped from deny.toml's allow list the step \
         exits 4 with 48 errors; adding `-A rejected` to that same command takes it to \
         exit 0 with 0 errors and `licenses ok`, while `-A duplicate` on the same tree \
         stays at exit 4. Independently, with `allow-registry = []` the step exits 8 with \
         216 errors, and `-A source-not-allowed` takes it to exit 0. If a diagnostic \
         genuinely has to be tolerated, put it in `deny.toml` where the exception is \
         reviewable next to the policy it weakens and this file's other guards can see \
         it — not in the invocation, where cargo-deny prints `ok` and says nothing. \
         `-D`/`--deny` is the strict direction and passes this guard.",
        silenced.join(", "),
        step.command
    ))
}

/// `Ok` if the gate command does not redirect what cargo-deny looks at.
///
/// `check_no_policy_is_silenced` above reads the half of the command line after
/// `check` and asks how loudly a finding is reported. This reads the half
/// before it and asks whether the finding is ever produced. The two sets of
/// flags do not overlap and neither position accepts the other's (measured,
/// SAK-248): `-A rejected` after `check` takes a red tree to exit 0, before
/// `check` it is `unexpected argument`/exit 2; the five options here are the
/// mirror image.
///
/// Measured on this tree, one tree per option and an independent control for
/// each, so that no option borrows another's evidence. The red baseline is
/// `smartstring 1.0.1`, which is both MPL-2.0 (not in `allow`) and
/// `unmaintained` — so the shipped command reports two findings and exits 5,
/// and a hole has to hide both:
///
/// | option | hole | control |
/// |---|---|---|
/// | `--config <other.toml>` | `"MIT"` dropped from `allow`: exit 4/48 errors → **exit 0** | the same flag at a file that *also* lacks MIT: still exit 4 |
/// | `--metadata-path <clean.json>` | **exit 0** | the same flag at the *bad* tree's metadata: still exit 5 |
/// | `--target x86_64-unknown-linux-gnu` | windows-only dep: **exit 0** | `--target x86_64-pc-windows-msvc`: still exit 5 |
/// | `--exclude-dev` | dev-only dep: **exit 0** | the same flag with the dep non-dev: still exit 5 |
/// | `--exclude smartstring` | **exit 0** | `--exclude tokio-test`: still exit 5 |
///
/// The controls are the point. Each one keeps a *different* thing constant, so
/// the green is the named file/triple/crate and not the mere presence of a
/// flag — the correction this file's own history asked for when one error
/// template was spreading one key's measurement across several keys (#88).
///
/// `--target` is the sharpest of the five: with no `targets` key in `deny.toml`
/// at all, this flag drops the evaluated set from 217 crates to 173 — `cc`,
/// `shlex`, `bumpalo`, every `windows-*`, every `objc2-*`, `wasi`, `r-efi` —
/// and `check_graph_covers_every_target` passes throughout, because the key it
/// reads really is absent. The policy is in the file; the decision is on the
/// command line.
fn check_nothing_redirects_what_is_checked(step: &DenyStep) -> Result<(), String> {
    let redirecting = redirecting_options(step);
    if redirecting.is_empty() {
        return Ok(());
    }
    Err(format!(
        "the `{DENY_STEP}` step in ci.yml passes {} to cargo-deny: `{}`. That does not \
         weaken a policy, it changes which crates the policy is applied to — or, for \
         `--config` and `--metadata-path`, replaces the policy file or the crate graph \
         wholesale. Every `deny.toml` assertion in this file keeps passing either way, \
         because every one of them reads the file in the repository rather than the one \
         cargo-deny was pointed at. Measured (SAK-248) against a tree carrying \
         `smartstring 1.0.1` — MPL-2.0 and unmaintained, two findings, exit 5: each of \
         these five options takes that to exit 0 with `advisories ok, bans ok, licenses \
         ok, sources ok`, and in each case naming a different file, triple or crate \
         leaves it red. If the graph genuinely has to be narrowed, narrow it in \
         `deny.toml`'s `[graph]`, where the other guards here can see it — and expect \
         `check_graph_covers_every_target` to have an opinion, which is the point. \
         Options that do not subtract pass this guard: `--all-features`, \
         `--no-default-features`, `--workspace`, `--exclude-unpublished`, `--frozen`, \
         `--offline`, `--locked`, and `-c`/`--color`, all measured on the same red tree.",
        redirecting.join(", "),
        step.command
    ))
}

/// The id of the job in `ci.yml` that holds the gate — the mapping key under
/// `jobs:`, not the `name:` beside it.
const DENY_JOB: &str = "deny";

/// The two keys that decide whether the gate above runs and whether its verdict
/// is counted. Both are valid on a job and on a step, which is the four places
/// `deny_gate_conditions` looks.
const GATE_CONDITION_KEYS: &[&str] = &["if", "continue-on-error"];

/// The `if:` values that leave the gate running.
///
/// Not an attempt at an expression evaluator — the opposite. A text reader
/// cannot decide `github.event_name == 'push'`, so the only `if:` values that
/// may pass are the handful whose meaning is "run anyway" no matter what the
/// rest of the workflow did. Everything else, including every expression over
/// the `github` context, fails closed.
///
/// Both spellings of each, because `if: always()` and `if: ${{ always() }}` are
/// the same condition and GitHub accepts either.
const CONDITIONS_THAT_STILL_RUN: &[&str] = &[
    "always()",
    "${{ always() }}",
    "success()",
    "${{ success() }}",
    "!cancelled()",
    "${{ !cancelled() }}",
];

/// `value` with one matching pair of surrounding YAML quotes removed.
///
/// `if: !cancelled()` is not valid YAML — a leading `!` is a tag indicator, so
/// the workflow file itself does not parse:
///
/// ```text
/// $ python3 -c "import yaml; yaml.safe_load('if: !cancelled()')"
/// ConstructorError: could not determine a constructor for the tag '!cancelled()'
/// ```
///
/// That makes `if: '!cancelled()'` the only shippable spelling of that member of
/// `CONDITIONS_THAT_STILL_RUN`, and a reader comparing the raw text rejects it.
/// Measured on the real `ci.yml` while checking SAK-249's must-be-green arms: a
/// *stricter* edit to the gate went red, which is the direction that gets a
/// guard deleted rather than fixed (SAK-250).
///
/// Unquoting cannot open a hole. It can only turn a non-matching value into one
/// of `CONDITIONS_THAT_STILL_RUN`, every member of which means "run anyway"
/// however it is quoted. `if: 'false'` unquotes to `false`, still not a member.
/// And one pair comes off rather than all of them, so `if: "'false'"` stays
/// `'false'` and still fails closed.
///
/// Shared by both readers of these two keys — the `deny` gate's
/// `deny_gate_conditions` and `sibling_reporter_conditions` — because the gap is
/// in how the value is compared, not in either reader's search.
///
/// Note which of the two keys this makes *shippable*, measured against the
/// Actions workflow schema (`actions/runner`
/// `src/Sdk/DTPipelines/workflow-v1.0.json`, read 2026-11-07):
///
/// | key | schema type | a quoted scalar |
/// |---|---|---|
/// | `if` | `step-if`, declared `"string": {}` | valid — `if: '!cancelled()'` and `if: "always()"` are shippable workflows |
/// | `continue-on-error` | `boolean-steps-context`, declared `"boolean": {}` | **not** valid — `BooleanDefinition.IsMatch` accepts only a `BooleanToken`, and `'false'` is a `StringToken` |
///
/// So unquoting `continue-on-error: 'false'` is tolerance for a spelling nobody
/// can ship, not permission for one: that workflow fails to parse before the
/// gate runs, so letting it past here cannot hide a switched-off gate. Only
/// `${{ false }}` reaches the runner as an `ExpressionToken`, which is why
/// `check_nothing_switches_the_gate_off` names that spelling separately.
fn unquote_condition(value: &str) -> &str {
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

/// One live `if:` or `continue-on-error:` on the gate's path.
#[derive(Debug, PartialEq, Eq)]
struct GateCondition {
    /// Where it sits, in the words the failure message uses: the `deny` job or
    /// the gate step inside it. Which of the two matters to whoever reads the
    /// failure, and to nothing else — both switch the gate off.
    scope: String,
    /// `if` or `continue-on-error`, without the colon.
    key: String,
    /// The value as written, comment stripped, trimmed, and with one pair of
    /// surrounding YAML quotes removed (`unquote_condition`).
    value: String,
}

/// Every `if:` / `continue-on-error:` that governs the `deny` gate.
#[derive(Debug, PartialEq, Eq)]
struct GateConditions {
    /// In file order, job-level before step-level. Empty on the shipped tree —
    /// but see `the_gate_condition_reader_finds_the_live_gate` for why no test
    /// asserts that about the real file.
    keys: Vec<GateCondition>,
}

/// Indentation of `line`, comment and trailing whitespace already irrelevant.
fn indent_of(line: &str) -> usize {
    let bare = without_comment(line);
    bare.len() - bare.trim_start().len()
}

/// The lines strictly inside the block opened at `at`, by indentation.
///
/// Blank and comment-only lines belong to neither the block nor what follows,
/// so they are skipped rather than ending it — the same call `permissions_keys`
/// makes, and `ci.yml` has blank lines between every step.
fn block_under<'a>(lines: &[&'a str], at: usize) -> Vec<(usize, &'a str)> {
    let indent = indent_of(lines[at]);
    let mut body = Vec::new();
    for (offset, line) in lines[at + 1..].iter().enumerate() {
        let bare = without_comment(line);
        let inner = bare.trim_start();
        if inner.is_empty() {
            continue;
        }
        if bare.len() - inner.len() <= indent {
            break;
        }
        body.push((at + 1 + offset, *line));
    }
    body
}

/// The one line in `lines` that is exactly `needle` once its comment is gone.
///
/// `Err` for none and `Err` for several, both for the reason `deny_step_checks`
/// gives: a reader that picked the first of two would be reporting on whichever
/// of them it happened to find.
fn sole_line(lines: &[&str], needle: &str, what: &str) -> Result<usize, String> {
    let found: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| without_comment(line).trim() == needle)
        .map(|(at, _)| at)
        .collect();
    match found[..] {
        [at] => Ok(at),
        [] => Err(format!(
            "ci.yml has no `{needle}` line, so this guard cannot find {what}. It fails \
             rather than passing: the gate it was written to protect is somewhere this \
             reader cannot see, and every promise below is about a gate it located. If \
             it was renamed, point DENY_JOB / DENY_STEP at the new name."
        )),
        _ => Err(format!(
            "ci.yml has {} `{needle}` lines, so which one is {what} is not something a \
             reader can tell.",
            found.len()
        )),
    }
}

/// The live `if:` / `continue-on-error:` keys on the `deny` gate's path,
/// read out of `ci.yml`.
///
/// There are exactly four places one can sit, and this reads all four: the
/// `deny` job's own keys, and the gate step's. A job-level `if:` stops the job
/// from being created at all; a step-level one skips the step; `continue-on-error`
/// at either level keeps the failure from reaching the job's conclusion. All
/// four end in the same place — the `deny` job concludes `success` with the
/// gate's verdict thrown away.
///
/// Read rather than asserted against a hard-coded expectation for the reason
/// `deny_step_checks` and `log_step_pattern` are: the file is the contract.
///
/// `Err` if the job or the step cannot be located. That is fail-closed, and it
/// has to be: a missing `deny:` job is the gate gone, which is worse than any
/// condition on it, and the only thing a test in this target can do about it is
/// refuse to pass (SAK-221).
fn deny_gate_conditions(ci: &str) -> Result<GateConditions, String> {
    let lines: Vec<&str> = ci.lines().collect();

    let job_at = sole_line(&lines, &format!("{DENY_JOB}:"), "the gate's job")?;
    let job_body = block_under(&lines, job_at);
    let Some(job_key_indent) = job_body.iter().map(|(_, line)| indent_of(line)).min() else {
        return Err(format!(
            "ci.yml's `{DENY_JOB}:` job has no body at all, so there is no gate under it \
             and nothing for this guard to report on."
        ));
    };

    let mut keys = Vec::new();
    let job_scope = format!("the `{DENY_JOB}` job");
    for (_, line) in &job_body {
        if indent_of(line) != job_key_indent {
            continue;
        }
        if let Some(found) = gate_condition(&job_scope, without_comment(line).trim()) {
            keys.push(found);
        }
    }

    // The step is looked for inside the job's own lines, so that a `deny` gate
    // and some other job's identically named step cannot be confused.
    let job_lines: Vec<&str> = job_body.iter().map(|(_, line)| *line).collect();
    let step_at = sole_line(
        &job_lines,
        &format!("- name: {DENY_STEP}"),
        "the gate step inside that job",
    )?;
    let step_key_indent = indent_of(job_lines[step_at]) + 2;
    let step_scope = format!("the `{DENY_STEP}` step");
    for (_, line) in &block_under(&job_lines, step_at) {
        // Exactly that depth, not "at least": `env:` under this step has its own
        // deeper keys, and a variable called `if` there governs nothing.
        if indent_of(line) != step_key_indent {
            continue;
        }
        if let Some(found) = gate_condition(&step_scope, without_comment(line).trim()) {
            keys.push(found);
        }
    }

    Ok(GateConditions { keys })
}

/// `body` as a gate condition, if that is what it is.
///
/// The value arrives unquoted: see `unquote_condition` for why a reader that
/// compared the raw text rejected `if: '!cancelled()'`, the one spelling of that
/// condition a workflow can actually carry.
fn gate_condition(scope: &str, body: &str) -> Option<GateCondition> {
    let (key, value) = body.split_once(':')?;
    let key = key.trim();
    if !GATE_CONDITION_KEYS.contains(&key) {
        return None;
    }
    Some(GateCondition {
        scope: scope.to_string(),
        key: key.to_string(),
        value: unquote_condition(value.trim()).to_string(),
    })
}

/// `Ok` if nothing on the gate's path stops it running or discards its verdict.
///
/// This is the other half of `check_every_deny_check_runs`. That one reads the
/// gate's command and asks whether it runs the whole policy; this one asks the
/// two questions one indent out — whether that command is reached, and whether
/// failing it costs anything. Measured on this tree (SAK-244), each against a
/// `deny.toml` mutation the gate otherwise catches:
///
/// | mutation | where | the step | the job | this file |
/// |---|---|---|---|---|
/// | `if: false` | gate step | skipped | `success` | 149 passed |
/// | `if: false` | `deny` job | never runs | no job to fail | 149 passed |
/// | `continue-on-error: true` | gate step | runs, fails | `success` | 149 passed |
///
/// Replaying the step the way the runner does (`bash --noprofile --norc -e -o
/// pipefail`) still exits 0, because none of the three changed the command.
/// cargo-deny has nothing to say about how it was invoked, so this is the same
/// tool-invisible shape as `check_every_deny_check_runs` and the `[graph]`
/// targets guard.
///
/// **Two assertions, and deliberately only two.**
///
/// 1. `continue-on-error` is absent or literally `false`.
/// 2. `if:` is absent or one of `CONDITIONS_THAT_STILL_RUN`.
///
/// An `if:` this reader cannot evaluate **fails**. That is not caution for its
/// own sake: `if: github.event_name == 'push'` is a condition no text reader can
/// decide, and on this repository it would skip the gate on exactly the event
/// that merges code, because every merge here arrives as a `pull_request`. A
/// guard that waved through what it could not read would wave that through.
///
/// What is asserted about the *runtime*, and on what evidence:
///
/// | claim | evidence |
/// |---|---|
/// | a false `if:` skips the step and the job still concludes `success` | run `42161681376` (`local_firestore.yml`): four jobs with a step skipped by `if: matrix.store == …`, all four `conclusion: success` |
/// | direction control — a step that really fails does fail its job | run `42153955324` (`CI`): `Build` `conclusion: failure`, job `conclusion: failure` |
/// | `if: always()` does not stop a step | run `42318904935` (`coverage.yml`): two `if: always()` steps, both `conclusion: success` |
///
/// `continue-on-error: true` is the one line here taken from GitHub's
/// documentation rather than measured on this repository: it says a step's
/// failure stops failing the job. Measuring it means pushing the mutation to a
/// branch of a public repository, which was not done. If it ever needs
/// measuring: put `continue-on-error: true` on the gate step, `gh workflow run
/// --ref` that branch, and read the `deny` job's `conclusion`.
///
/// Deliberately **out of scope**: an `if:` or `continue-on-error:` on the `test`
/// job's `Supply-chain pins and gate settings` step — the step that runs this
/// assertion. Switching that off takes the reporter away along with the report,
/// and no test inside the target it guards can see that happen (SAK-221).
fn check_nothing_switches_the_gate_off(conditions: &GateConditions) -> Result<(), String> {
    for found in &conditions.keys {
        let GateCondition { scope, key, value } = found;
        if key == "continue-on-error" {
            if value.eq_ignore_ascii_case("false") || value == "${{ false }}" {
                continue;
            }
            return Err(format!(
                "{scope} in ci.yml carries `continue-on-error: {value}`. That keeps the \
                 gate's failure out of the job's conclusion: cargo-deny runs, reports the \
                 problem, exits non-zero, and — per GitHub's documentation for this key, \
                 which is where this one line comes from rather than from a run of this \
                 repository — the `{DENY_JOB}` job still concludes `success`, so the one \
                 thing that reads the gate's verdict reads nothing. What *was* measured \
                 (SAK-244) is the half on this side: with that key on the gate step and \
                 deny.toml's licence allowlist broken, this whole file still passed 149 \
                 tests, so nothing here would have told you. Remove the key, or set it to \
                 `false` if something wants it written down explicitly."
            ));
        }

        if CONDITIONS_THAT_STILL_RUN.contains(&value.as_str()) {
            continue;
        }
        return Err(format!(
            "{scope} in ci.yml carries `if: {value}`, which this guard cannot clear. A \
             condition that is false does not fail anything — the step is `skipped` and \
             the job still concludes `success` (measured on this repository's own run \
             `42161681376`), so a tree with the supply-chain gate switched off is \
             indistinguishable from a tree that passed it. Measured (SAK-244) with \
             `if: false` on the gate step, this whole file still passed 149 tests.\n\n\
             Conditions that pass: {}. Everything else fails, including an expression \
             over the `github` context — a text reader cannot decide one, and \
             `github.event_name == 'push'` in particular would skip the gate on every \
             pull request, which is how code reaches `main` here. If the gate genuinely \
             has to be conditional, that is a decision to take in a ticket rather than \
             in a line this guard has to guess at.",
            CONDITIONS_THAT_STILL_RUN.join(", ")
        ));
    }
    Ok(())
}

/// The event that has to reach the gate, spelled as it appears under `on:`.
const GATE_EVENT: &str = "pull_request";

/// The keys that narrow which `pull_request` events start the workflow.
///
/// Every one of them subtracts: a workflow with any of these fires on a subset
/// of what it fired on before. None of them can make it fire on more. That
/// asymmetry is the whole reason this is a fixed list rather than a judgement
/// about values — a reader does not have to decide whether `[main]` or
/// `['src/**']` is the dangerous spelling, because narrowing at all is the
/// thing being reported.
const TRIGGER_NARROWING_KEYS: &[&str] = &[
    "branches",
    "branches-ignore",
    "paths",
    "paths-ignore",
    "types",
    "tags",
    "tags-ignore",
];

/// `ci.yml`'s `on:` block, as a text reader can see it.
#[derive(Debug, PartialEq, Eq)]
struct GateTrigger {
    /// The event names declared directly under `on:`, in file order.
    events: Vec<String>,
    /// The keys nested under `GATE_EVENT`, if that event is declared with a
    /// body at all. `Some(vec![])` is the shipped shape — declared, no body.
    gate_event_keys: Option<Vec<String>>,
}

/// What starts the workflow that holds the gate, read out of `ci.yml`.
///
/// One level further out than `deny_gate_conditions`. That one asks whether the
/// job and step conditions let the gate run; this asks whether the *workflow*
/// is started at all. A workflow that is never triggered has no jobs, so it has
/// no `deny` job, so there is nothing for any condition on that job to be read
/// off — and `GateConditions` would still come back empty and green.
///
/// `Err` if `on:` is missing or its body cannot be read, which is fail-closed
/// for the same reason the rest of this file is: a gate whose trigger this
/// reader cannot see is a gate none of the promises below are about.
fn deny_gate_trigger(ci: &str) -> Result<GateTrigger, String> {
    let lines: Vec<&str> = ci.lines().collect();

    let on_at = sole_line(&lines, "on:", "the workflow's trigger block")?;
    let body = block_under(&lines, on_at);
    let Some(event_indent) = body.iter().map(|(_, line)| indent_of(line)).min() else {
        return Err(
            "ci.yml's `on:` block has no body, so nothing starts this workflow and the \
             supply-chain gate inside it never runs. A workflow with no triggers is not a \
             gate; it is a file."
                .to_string(),
        );
    };

    let mut events = Vec::new();
    let mut gate_event_keys = None;
    for (offset, (_, line)) in body.iter().enumerate() {
        if indent_of(line) != event_indent {
            continue;
        }
        let Some((event, _)) = without_comment(line).trim().split_once(':') else {
            // A sequence entry (`on: [push, pull_request]` written as a block
            // list) or anything else this reader has no grammar for. Refusing
            // is the only honest answer: the events are right there in the file
            // and this guard cannot name them.
            return Err(format!(
                "ci.yml's `on:` block has the line `{}`, which this reader has no grammar \
                 for. It fails rather than passing: which events start the workflow is the \
                 single thing this guard is about, and it cannot read them. Write the \
                 triggers as `  <event>:` mapping keys, which is the form ci.yml uses.",
                without_comment(line).trim()
            ));
        };
        let event = event.trim().to_string();
        if event == GATE_EVENT {
            // Exactly that depth, not deeper: `types:` under `pull_request` is a
            // narrowing key, while `branches:` under a *different* event is that
            // event's business and not this one's.
            let nested = block_under(&lines, body[offset].0);
            let Some(key_indent) = nested.iter().map(|(_, l)| indent_of(l)).min() else {
                gate_event_keys = Some(Vec::new());
                events.push(event);
                continue;
            };
            let mut keys = Vec::new();
            for (_, nested_line) in &nested {
                if indent_of(nested_line) != key_indent {
                    continue;
                }
                let bare = without_comment(nested_line).trim();
                let key = bare.split_once(':').map_or(bare, |(key, _)| key).trim();
                keys.push(key.to_string());
            }
            gate_event_keys = Some(keys);
        }
        events.push(event);
    }

    Ok(GateTrigger {
        events,
        gate_event_keys,
    })
}

/// `Ok` if every pull request starts the workflow that holds the gate.
///
/// The third of three questions about the same gate, and the one left over
/// after the other two. `check_every_deny_check_runs` reads what the gate's
/// command names; `check_nothing_switches_the_gate_off` reads whether that
/// command is reached and whether failing it costs anything. Both of those read
/// keys *inside* a job that exists. This one reads whether the workflow runs at
/// all, which is a key one indent further out, in a block neither of them
/// opens.
///
/// Measured on this tree (SAK-218), each arm the real `ci.yml` with its `on:`
/// block edited and nothing else:
///
/// | mutation | what still runs on a pull request | this file |
/// |---|---|---|
/// | `pull_request: branches: [main]` | only the bottom layer of a stack | 154 passed |
/// | `pull_request: paths: ['src/**']` | nothing, for a `deny.toml`-only PR | 154 passed |
/// | `pull_request:` removed | nothing | 154 passed |
/// | `pull_request: types: [labeled]` | nothing, until someone labels it | 154 passed |
/// | whole block replaced by `workflow_dispatch:` | nothing, ever | 154 passed |
///
/// The control that says the green is a missing assertion rather than a file
/// this target cannot see: renaming the gate step in the same file turns seven
/// of these tests red.
///
/// **One assertion.** `pull_request` is declared, and carries none of
/// `TRIGGER_NARROWING_KEYS`.
///
/// Deliberately nothing about the rest of `on:`. Another event, a
/// `workflow_dispatch`, a `schedule` — each is one more way the gate runs, so a
/// guard that reddened on them would be failing a tree stricter than the one
/// shipped. That is the same direction `check_every_deny_check_runs` settled on
/// when it had to pass a command that named no checks at all, and the mistake
/// SAK-241 undid in the `permissions` reader. A narrowing key under a *different*
/// event is likewise not this guard's business: narrowing `push` costs the
/// post-merge run on `main`, not the pull-request run that gates the merge.
///
/// What is asserted about the *runtime*, and on what evidence. `actionlint.yml`
/// on `main` carries the identical `on:` block to this one, which makes its run
/// history a one-variable measurement of these two filters in the same file
/// shape:
///
/// | claim | evidence |
/// |---|---|
/// | a `branches:` filter suppresses the event on every non-matching ref | `actionlint`'s `push` runs (`branches: [main]`): 3, every one of them on `main` |
/// | the same workflow's unfiltered event does fire on feature branches | `actionlint`'s `pull_request` runs: 4, on 4 distinct `sak-*` branches |
/// | direction control — pushes to feature branches do generate runs when nothing filters them out | `fireup local`: 56 `push` runs across 18 branches, feature branches included |
///
/// The direction control is what makes the first row mean anything. Without it,
/// "zero `push` runs on feature branches" reads just as well as "nobody pushed".
///
/// `paths:` has no precedent in this repository — no workflow here has ever
/// carried one — so that row of the first table is the tree-side measurement
/// only, and GitHub's documentation for the runtime half. Said plainly here
/// rather than left to look measured.
///
/// Why this one matters more here than the filter looks: every change reaches
/// `main` through a pull request, and overlapping work in this repository is
/// stacked — 21 layers deep while this was written. `ci.yml`'s own comment
/// argues for exactly this ("a `branches: [main]` filter would skip exactly the
/// PRs that most need checking — every layer of a stack but the bottom one").
/// That argument was prose, and prose is not a reporter: the five mutations
/// above left this file at 154 passed.
fn check_every_pull_request_runs_the_gate(trigger: &GateTrigger) -> Result<(), String> {
    let Some(keys) = &trigger.gate_event_keys else {
        return Err(format!(
            "ci.yml's `on:` block declares {:?} but not `{GATE_EVENT}`, so no pull request \
             starts this workflow. Nothing then creates the `{DENY_JOB}` job, so the \
             supply-chain gate has no verdict to give on the change being merged — and \
             every change reaches `main` here through a pull request. Measured (SAK-218): \
             with `{GATE_EVENT}` removed, this whole file still passed 154 tests, so \
             nothing here would have told you.",
            trigger.events
        ));
    };

    let narrowing: Vec<&String> = keys
        .iter()
        .filter(|key| TRIGGER_NARROWING_KEYS.contains(&key.as_str()))
        .collect();
    if narrowing.is_empty() {
        return Ok(());
    }

    Err(format!(
        "ci.yml narrows its `{GATE_EVENT}` trigger with {narrowing:?}, which subtracts from \
         the pull requests that start this workflow. The ones it subtracts are not a random \
         sample. Overlapping work in this repository is stacked — one pull request based on \
         another rather than on `main` — so `branches: [main]` keeps the gate off every \
         layer but the bottom one, and a `paths:` filter keeps it off exactly the change \
         that is only `deny.toml` and `Cargo.lock`. A workflow that does not start has no \
         `{DENY_JOB}` job, so `check_nothing_switches_the_gate_off` has no condition to \
         read and comes back green.\n\n\
         Measured on this repository's own history: `actionlint.yml` on `main` carries the \
         identical `on:` block to this one, and its `push` event — the one with \
         `branches: [main]` on it — has run 3 times, every time on `main`, while its \
         unfiltered `pull_request` event has run on 4 separate `sak-*` branches. The \
         direction control is `fireup local`, with 56 `push` runs across 18 branches \
         including feature branches: pushes there do make runs when no filter removes \
         them.\n\n\
         Adding an event, or a `workflow_dispatch`, is a tree this guard passes — more ways \
         to run is not a hole. Only narrowing `{GATE_EVENT}` fails. If the gate genuinely \
         has to be conditional, that is a decision for a ticket rather than a line this \
         guard has to guess at."
    ))
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

#[test]
fn deny_toml_refuses_a_yanked_crate_and_leaves_the_check_switched_on() {
    if let Err(reason) = check_yanked_is_denied(&repo_file("deny.toml")) {
        panic!("{reason}");
    }
}

#[test]
fn deny_toml_licence_checks_dev_dependencies_and_leaves_them_in_the_graph() {
    if let Err(reason) = check_dev_licenses_are_checked(&repo_file("deny.toml")) {
        panic!("{reason}");
    }
}

#[test]
fn deny_toml_fails_on_an_allowed_source_that_nothing_comes_from() {
    if let Err(reason) = check_unused_allowed_source_is_deny(&repo_file("deny.toml")) {
        panic!("{reason}");
    }
}

#[test]
fn deny_toml_leaves_every_build_target_in_the_graph() {
    if let Err(reason) = check_graph_covers_every_target(&repo_file("deny.toml")) {
        panic!("{reason}");
    }
}

#[test]
fn deny_toml_reports_unsound_advisories_for_the_whole_graph() {
    if let Err(reason) = check_unsound_scope_is_all(&repo_file("deny.toml")) {
        panic!("{reason}");
    }
}

#[test]
fn deny_toml_reads_the_official_advisory_database() {
    if let Err(reason) = check_advisory_db_is_the_official_one(&repo_file("deny.toml")) {
        panic!("{reason}");
    }
}

#[test]
fn deny_toml_scans_the_binaries_the_build_runs_and_pins_every_exemption() {
    if let Err(reason) = check_build_executables_are_pinned(&repo_file("deny.toml")) {
        panic!("{reason}");
    }
}

#[test]
fn deny_toml_licence_checks_build_dependencies() {
    if let Err(reason) = check_build_licenses_are_checked(&repo_file("deny.toml")) {
        panic!("{reason}");
    }
}

#[test]
fn every_accepted_advisory_is_visible_in_the_deny_jobs_log() {
    let pattern = match log_step_pattern(&repo_file(".github/workflows/ci.yml")) {
        Ok(pattern) => pattern,
        Err(reason) => panic!("{reason}"),
    };
    if let Err(reason) = check_ignore_entries_are_logged(&repo_file("deny.toml"), &pattern) {
        panic!("{reason}");
    }
}

#[test]
fn ci_yml_declares_a_read_only_token_for_every_job() {
    if let Err(reason) = check_workflow_token_is_read_only(&repo_file(".github/workflows/ci.yml")) {
        panic!("{reason}");
    }
}

#[test]
fn the_deny_job_runs_every_check_deny_toml_configures() {
    let step = match deny_step_checks(&repo_file(".github/workflows/ci.yml")) {
        Ok(step) => step,
        Err(reason) => panic!("{reason}"),
    };
    if let Err(reason) = check_every_deny_check_runs(&step) {
        panic!("{reason}");
    }
}

#[test]
fn the_deny_jobs_command_silences_no_diagnostic() {
    let step = match deny_step_checks(&repo_file(".github/workflows/ci.yml")) {
        Ok(step) => step,
        Err(reason) => panic!("{reason}"),
    };
    if let Err(reason) = check_no_policy_is_silenced(&step) {
        panic!("{reason}");
    }
}

#[test]
fn the_deny_jobs_command_redirects_nothing() {
    let step = match deny_step_checks(&repo_file(".github/workflows/ci.yml")) {
        Ok(step) => step,
        Err(reason) => panic!("{reason}"),
    };
    if let Err(reason) = check_nothing_redirects_what_is_checked(&step) {
        panic!("{reason}");
    }
}

#[test]
fn nothing_in_ci_yml_switches_the_deny_gate_off() {
    let conditions = match deny_gate_conditions(&repo_file(".github/workflows/ci.yml")) {
        Ok(conditions) => conditions,
        Err(reason) => panic!("{reason}"),
    };
    if let Err(reason) = check_nothing_switches_the_gate_off(&conditions) {
        panic!("{reason}");
    }
}

#[test]
fn every_pull_request_starts_the_workflow_holding_the_deny_gate() {
    let trigger = match deny_gate_trigger(&repo_file(".github/workflows/ci.yml")) {
        Ok(trigger) => trigger,
        Err(reason) => panic!("{reason}"),
    };
    if let Err(reason) = check_every_pull_request_runs_the_gate(&trigger) {
        panic!("{reason}");
    }
}

/// The mutations, committed rather than run once by hand.
///
/// Each one is the regression this file exists to catch, fed to the same
/// function the tests above call. Without these, a refactor that made
/// `check_*` return `Ok(())` unconditionally would leave every test above green.
/// The name of the step in `ci.yml`'s `deny` job that installs the gate's own
/// binary.
///
/// Every other guard in this file reads something the gate *uses* — the policy
/// in `deny.toml`, the check list in the gate's command, the conditions on the
/// step, the triggers on the workflow. This one reads where the gate's own
/// executable comes from. The `deny` job downloads a release tarball from a
/// third party and runs it; the only thing standing between "the bytes this
/// commit pinned" and "whatever that URL served today" is one line of shell.
const INSTALL_STEP: &str = "Install cargo-deny";

/// The digest-tool invocations this reader recognises as *comparing* bytes.
///
/// `sha256sum <file>` prints a digest and exits 0 whatever the bytes are; only
/// the `-c` form compares one. Measured on the real file: swapping the shipped
/// line for `sha256sum cargo-deny.tar.gz` leaves the step at exit 0, installs
/// whatever was downloaded, and changes nothing else in the log.
///
/// A line that names one of these tools in a form not on this list is not
/// assumed equivalent — it fails closed, the same call `deny_step_checks` makes
/// about a command body it cannot read. Extend the list rather than widening
/// the match.
const DIGEST_COMPARISONS: [&str; 4] = [
    "sha256sum -c",
    "sha256sum --check",
    "shasum -a 256 -c",
    "shasum -a 256 --check",
];

/// The `Install cargo-deny` step's `env:` pins and `run:` body, read out of
/// `ci.yml`.
struct InstallStep {
    /// The step's `env:` entries as `KEY: value`, comments stripped. The pins
    /// live here: `CARGO_DENY_VERSION` picks the release, `CARGO_DENY_SHA256`
    /// says which bytes that release is allowed to be.
    env: Vec<String>,
    /// The step's `run:` body, one entry per source line, comments stripped,
    /// blank and comment-only lines dropped.
    ///
    /// Comments are stripped for a reason that matters here more than
    /// elsewhere: a commented-out verification line is not a verification line,
    /// and a reader that counted it would pass the one tree this guard exists
    /// to fail.
    body: Vec<String>,
}

/// Whether `line` turns errexit back off.
///
/// `set +e`, `set +eu`, `set +o errexit` — any of them, and a later failing
/// command no longer ends the step. Measured: with `set +e` added after the
/// shipped `set -euo pipefail`, a mismatched checksum prints
/// `cargo-deny.tar.gz: FAILED`, the step carries on, the unverified binary
/// lands on `PATH`, and the step exits **0**.
fn clears_errexit(line: &str) -> bool {
    let mut words = line.split_whitespace();
    if words.next() != Some("set") {
        return false;
    }
    let words: Vec<&str> = words.collect();
    words.windows(2).any(|pair| pair == ["+o", "errexit"])
        || words
            .iter()
            .any(|word| word.starts_with('+') && word[1..].contains('e'))
}

/// The shell variables `line` reads, by name.
fn shell_vars(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    let bytes: Vec<char> = line.chars().collect();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == '$' {
            let mut cursor = at + 1;
            if cursor < bytes.len() && bytes[cursor] == '{' {
                cursor += 1;
            }
            let start = cursor;
            while cursor < bytes.len()
                && (bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == '_')
            {
                cursor += 1;
            }
            if cursor > start {
                found.push(bytes[start..cursor].iter().collect());
            }
            at = cursor;
            continue;
        }
        at += 1;
    }
    found
}

/// Whether `line` carries a 64-character hex run — a sha256 written out.
fn has_sha256_literal(line: &str) -> bool {
    let mut run = 0;
    for ch in line.chars() {
        if ch.is_ascii_hexdigit() {
            run += 1;
            if run >= 64 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// The value `name` is given in the install step's `env:`, if it is given one.
///
/// `env` entries arrive as `KEY: value` with comments already stripped, so the
/// split is on the first colon only: a value may itself contain one.
fn env_value<'a>(env: &'a [String], name: &str) -> Option<&'a str> {
    env.iter().find_map(|entry| {
        let (key, value) = entry.split_once(':')?;
        (key.trim() == name).then_some(value.trim())
    })
}

/// Whether `value` is a digest *this commit* fixes.
///
/// Two conditions, and the second is the one the first does not cover. A
/// 64-character hex run makes it a digest; the absence of `${{` makes it one
/// the repository holds. `${{ vars.CARGO_DENY_SHA256 }}` satisfies neither,
/// and `${{ vars.X || '<64 hex>' }}` satisfies only the first while still
/// letting a value set outside the tree win -- so a fallback default is
/// refused rather than read as a pin.
fn is_a_pinned_digest(value: &str) -> bool {
    has_sha256_literal(value) && !value.contains("${{")
}

/// The `Install cargo-deny` step, read out of `ci.yml`.
///
/// Read rather than written down a second time for the same reason
/// `deny_step_checks` and `log_step_pattern` are: the file is the contract.
///
/// `Err` if the step is gone, if there is not exactly one of it, or if its
/// `run:` body is not something this reader can see. Each of those voids the
/// promise below, so none may read as a pass. Note what this guard cannot do,
/// the same note `deny_step_checks` carries: deleting the step outright takes
/// the install away, and all a test can do about that is refuse to pass
/// (SAK-221). Deleting it is also the one edit here that is *fail-safe* rather
/// than fail-open — with no binary on `PATH` the gate step cannot run at all,
/// which is why SAK-244's guard deliberately permits `if: false` on this very
/// step.
fn install_step(ci: &str) -> Result<InstallStep, String> {
    let lines: Vec<&str> = ci.lines().collect();
    let at = sole_line(
        &lines,
        &format!("- name: {INSTALL_STEP}"),
        "the step that installs the gate's binary",
    )?;

    // Everything up to the next item in the `steps:` sequence. `- name:` alone
    // is not the boundary: a `- uses:` step has no name.
    let step: Vec<&str> = lines[at + 1..]
        .iter()
        .copied()
        .take_while(|line| {
            let body = without_comment(line).trim();
            body != "-" && !body.starts_with("- ")
        })
        .collect();

    let key_indent = indent_of(lines[at]) + 2;

    let mut env = Vec::new();
    if let Some(env_at) = step.iter().position(|line| {
        let bare = without_comment(line);
        bare.trim() == "env:" && indent_of(bare) == key_indent
    }) {
        for (_, line) in block_under(&step, env_at) {
            env.push(without_comment(line).trim().to_string());
        }
    }

    let run_at = step
        .iter()
        .position(|line| {
            let bare = without_comment(line);
            indent_of(bare) == key_indent && bare.trim_start().starts_with("run:")
        })
        .ok_or_else(|| {
            format!(
                "the `{INSTALL_STEP}` step in ci.yml has no `run:` body. This guard reads \
                 that body to check the download is verified before it is executed, and it \
                 cannot read a body it did not find."
            )
        })?;

    let run_line = without_comment(step[run_at]);
    let inline = run_line
        .trim_start()
        .strip_prefix("run:")
        .unwrap_or_default()
        .trim();

    let mut body: Vec<String> = Vec::new();
    if inline.starts_with('|') || inline.starts_with('>') || inline.is_empty() {
        for (_, line) in block_under(&step, run_at) {
            let text = without_comment(line).trim();
            if !text.is_empty() {
                body.push(text.to_string());
            }
        }
    } else {
        body.push(inline.to_string());
    }

    if body.is_empty() {
        return Err(format!(
            "the `{INSTALL_STEP}` step in ci.yml has an empty `run:` body, so it installs \
             nothing and exits 0."
        ));
    }

    Ok(InstallStep { env, body })
}

/// `Ok` if the gate's own binary is checked against a digest this commit pins
/// before it is run.
///
/// ## Why this is a guard and not a comment
///
/// `ci.yml` already says, in prose, "Bump CARGO_DENY_VERSION and
/// CARGO_DENY_SHA256 together, deliberately." Prose is not a reporter. The
/// pin is load-bearing today — measured on the real file, replaying the step
/// body the way the runner does (`bash --noprofile --norc -e -o pipefail`):
///
/// ```text
///   shipped                                       step exit 0   cargo-deny 0.20.2 installed
///   one hex digit of the digest flipped           step exit 1   nothing installed
///   version moved to 0.19.9, digest left alone    step exit 1   nothing installed
///   CARGO_DENY_SHA256 deleted (`set -u`)          step exit 1   nothing installed
/// ```
///
/// And it is switched off by one line, with no reporter anywhere. Same
/// measurement, same harness, each arm paired with the version bump the pin
/// exists to catch:
///
/// ```text
///   delete the `sha256sum -c` line     + 0.19.9   step exit 0   cargo-deny 0.19.9 installed
///   `… | sha256sum -c - || true`       + 0.19.9   step exit 0   cargo-deny 0.19.9 installed
///   `sha256sum cargo-deny.tar.gz`      + 0.19.9   step exit 0   cargo-deny 0.19.9 installed
///   `set +e` after `set -euo pipefail` + 0.19.9   step exit 0   cargo-deny 0.19.9 installed
/// ```
///
/// In all four the `deny` job goes on to print `advisories ok, bans ok,
/// licenses ok, sources ok` and exit 0 — run with a binary nothing checked.
/// `cargo test --test dependency_pin_tests` was **159 passed in every one of
/// them**, and that green is an absent assertion rather than an unread file:
/// renaming the gate step in the same file takes the suite to 7 failed.
///
/// ## What this does not assert, and why
///
/// Not that the body sets `-e`. It does not have to: the runner invokes
/// `bash -e {0}`, so errexit is on before the first line of the body. Measured
/// — replacing the shipped `set -euo pipefail` with `set -uo pipefail` and
/// moving the version to 0.19.9 is still **step exit 1, nothing installed**.
/// That corrected the author's first hypothesis, which was that dropping the
/// `e` was the hole; only `set +e` actually takes errexit away, which is why
/// that is what is checked. A guard demanding the literal `set -euo pipefail`
/// would have reddened `set -uo pipefail` plus an explicit `|| exit 1`, which
/// is equally fail-closed. Stricter trees have to stay green or the cheapest
/// way back to green is to undo the strictness — the mistake SAK-241 undid in
/// `the_permissions_reader_finds_exactly_the_live_keys` and SAK-244 avoided in
/// `the_gate_condition_reader_finds_the_live_gate`.
///
/// Not the digest's *value*, and not the URL's shape. Both are fail-closed: a
/// wrong digest is the second table above, and pointing the URL at
/// `releases/latest/download/…` was step exit 22 (`curl -f` on a 404) in the
/// same harness. A guard is only worth writing where the failure direction is
/// fail-open — the rule `without_comment`'s two call sites produced.
///
/// Not that the installed tool is a particular version. The hole is unverified
/// bytes being executed, not a specific release being weaker: run against the
/// shipped `deny.toml` with the `[bans.build]` bypass removed, 0.19.9 and
/// 0.20.2 both report the same 8 `detected-executable` findings and both exit
/// 2. The stand-in version enforced the same policy; that is a reason to state
/// the finding as "unverified", not as "downgraded".
fn check_install_verifies_its_download(step: &InstallStep) -> Result<(), String> {
    let mut verifications: Vec<(usize, &String)> = Vec::new();
    let mut unclassified: Vec<&String> = Vec::new();
    for (at, line) in step.body.iter().enumerate() {
        if !(line.contains("sha256sum") || line.contains("shasum")) {
            continue;
        }
        if DIGEST_COMPARISONS.iter().any(|form| line.contains(form)) {
            verifications.push((at, line));
        } else {
            unclassified.push(line);
        }
    }

    if verifications.is_empty() {
        if let Some(line) = unclassified.first() {
            return Err(format!(
                "the `{INSTALL_STEP}` step in ci.yml names a digest tool but never compares \
                 one: `{line}`. `sha256sum <file>` prints a digest and exits 0 whatever the \
                 bytes are. Measured: with this line in place of the shipped one the step \
                 is exit 0 and installs whatever the URL served. Use one of {DIGEST_COMPARISONS:?}."
            ));
        }
        return Err(format!(
            "nothing in the `{INSTALL_STEP}` step in ci.yml checks the downloaded tarball \
             against a digest. The `deny` job downloads a release binary from a third party \
             and runs it; with no comparison the bytes it runs are whatever that URL served, \
             the step is exit 0, the gate prints `advisories ok, bans ok, licenses ok, \
             sources ok`, and no test in this repository says a word. The pin itself is still \
             in `env:` — a digest nothing compares against is decoration. Body read: {:?}",
            step.body
        ));
    }

    let declared: Vec<&str> = step
        .env
        .iter()
        .filter_map(|entry| entry.split_once(':').map(|(key, _)| key.trim()))
        .collect();

    for (at, line) in &verifications {
        // The expected digest has to be fixed by this commit. A checksum file
        // fetched from the same release alongside the tarball is signed by
        // whoever served the tarball, so it attests to nothing.
        let read = shell_vars(line);
        let from_env: Vec<&String> = read
            .iter()
            .filter(|name| declared.contains(&name.as_str()))
            .collect();
        let pinned_by_env = !from_env.is_empty();
        if !pinned_by_env && !has_sha256_literal(line) && !read.is_empty() {
            return Err(format!(
                "the `{INSTALL_STEP}` step in ci.yml compares against {read:?}, and this step \
                 declares {declared:?}. At runtime an undeclared name is fail-closed — the \
                 body's `set -u` makes the step exit 1 with nothing installed (measured) — so \
                 this is not a hole. It fails anyway because the reader can no longer tell a \
                 digest this commit pinned from one fetched at run time, and that is the whole \
                 question it was written to answer."
            ));
        }
        if !pinned_by_env && !has_sha256_literal(line) {
            return Err(format!(
                "the `{INSTALL_STEP}` step in ci.yml compares a digest that this commit does \
                 not pin: `{line}`. The expected digest has to come from the repository — an \
                 `env:` key of this step (declared here: {declared:?}) or a 64-character hex \
                 literal on the line. A `SHA256SUMS` fetched next to the tarball is served by \
                 the same host as the tarball and attests to nothing. If the digest is in a \
                 checked-in file, this reader cannot tell that from a downloaded one, so it \
                 fails closed: say so in a ticket and widen it deliberately."
            ));
        }

        // The name being declared here is not the same question as the value
        // being fixed by this commit. `CARGO_DENY_SHA256: ${{ vars.X }}` is a
        // declared name whose value lives in the repository's web settings --
        // the same place SAK-240 found `default_workflow_permissions` and
        // SAK-242 found the ruleset, changeable with no commit, no diff and no
        // review.
        if pinned_by_env
            && !from_env
                .iter()
                .any(|name| env_value(&step.env, name).is_some_and(is_a_pinned_digest))
        {
            let values: Vec<(&str, &str)> = from_env
                .iter()
                .map(|name| (name.as_str(), env_value(&step.env, name).unwrap_or("")))
                .collect();
            return Err(format!(
                "the `{INSTALL_STEP}` step in ci.yml compares against an `env:` name this \
                 step declares but whose value this commit does not fix: {values:?}. \
                 Measured by replaying the step body the way the runner does, with the \
                 step's own `env:` map applied and the `${{{{ }}}}` filled from outside: \
                 against the shipped literal pin the tree decides the bytes -- an outside \
                 value of 0.19.9 plus its digest still installs 0.20.2 at step exit 0 -- \
                 while with `CARGO_DENY_SHA256: ${{{{ vars.S }}}}` the same tree installs \
                 0.20.2 or 0.19.9 at step exit 0 depending on a value held outside the \
                 repository. The comparison is not off: an outside digest that does not \
                 match is step exit 1 with nothing installed, and an unset one is the \
                 same. What moved is who chooses the number. Write the digest out as a \
                 64-character hex literal, in this `env:` or on the comparison line. Only \
                 the digest needs it: `CARGO_DENY_VERSION: ${{{{ vars.V }}}}` with the \
                 digest left literal gives an outside value no new freedom -- measured, it \
                 can only pick the version the digest already pins (0.20.2 exit 0, 0.19.9 \
                 exit 1, unset exit 1)."
            ));
        }

        if let Some((_, tail)) = line.split_once("||") {
            let tail = tail.trim();
            let aborts = tail
                .strip_prefix("exit")
                .map(|rest| {
                    let code = rest.trim();
                    !code.is_empty() && code != "0"
                })
                .unwrap_or(false);
            if !aborts {
                return Err(format!(
                    "the `{INSTALL_STEP}` step in ci.yml sends the digest comparison's exit \
                     status to `||`: `{line}`. Measured with `|| true`: the comparison prints \
                     `cargo-deny.tar.gz: FAILED`, the step carries on, the unverified binary \
                     is installed, and the step exits 0. `|| exit 1` is fine; anything that \
                     swallows the status is the hole. Only `||` does this — `; true` and a \
                     trailing pipe both still abort, because the runner runs this body under \
                     `bash -e -o pipefail`."
                ));
            }
        }

        for earlier in step.body.iter().take(*at) {
            if clears_errexit(earlier) {
                return Err(format!(
                    "the `{INSTALL_STEP}` step in ci.yml runs `{earlier}` before its digest \
                     comparison, which takes errexit back off, and a failed comparison then \
                     no longer ends the step. Measured: `set +e` after the shipped \
                     `set -euo pipefail`, with the version moved to 0.19.9 — \
                     `cargo-deny.tar.gz: FAILED` in the log, the wrong binary installed, step \
                     exit 0. Note the asymmetry: the body cannot *add* errexit-off by writing \
                     `set -uo pipefail`, because the runner already invoked `bash -e`; only \
                     `set +e` does it."
                ));
            }
        }
    }

    Ok(())
}

/// The gate's own binary is checked against a digest this commit pins before it
/// is executed.
///
/// What is asked of the real file and what is not: only that the install step is
/// **locatable** and that its download is verified. Nothing here pins the
/// shipped text, the shipped version, or the shipped digest. A tree that
/// verified twice, used `shasum -a 256 -c`, added `--status`, or replaced
/// `set -euo pipefail` with `set -uo pipefail` plus `|| exit 1` is as safe or
/// safer, and every one of those is a green arm below.
#[test]
fn ci_yml_verifies_the_gate_binary_before_running_it() {
    let step = install_step(&repo_file(".github/workflows/ci.yml")).expect(
        "the step that installs cargo-deny must be locatable, or every arm below is failing \
         on the reader rather than on what it put in the body",
    );
    if let Err(problem) = check_install_verifies_its_download(&step) {
        panic!("{problem}");
    }
}

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

    #[test]
    fn the_yanked_policy_the_repository_ships_passes() {
        assert!(check_yanked_is_denied("yanked = \"deny\"\n").is_ok());
    }

    /// The regression itself. Measured on the stack tip with `Cargo.lock`
    /// downgraded to the yanked `async-trait` 0.1.90: `"deny"` exits 1 with
    /// `error[yanked]`, and this value exits 0 on the identical tree.
    #[test]
    fn warning_on_a_yanked_crate_fails() {
        let error = check_yanked_is_denied("yanked = \"warn\"\n").expect_err("\"warn\" must fail");
        assert!(error.contains("error[yanked]"), "{error}");
    }

    /// Deleting the line is the quieter version of the same change, and the one
    /// a tidy-up produces: cargo-deny's default is `warn`, so the yanked crate
    /// is still reported and the check still exits 0.
    #[test]
    fn omitting_the_yanked_setting_fails() {
        let deny = "[advisories]\nunmaintained = \"all\"\nignore = []\n";
        let error = check_yanked_is_denied(deny).expect_err("a missing setting must fail");
        assert!(error.contains("default is `warn`"), "{error}");
    }

    /// The reason this key needs two assertions. `yanked = "deny"` is untouched
    /// and the check is off: measured on the stack tip, zero yanked diagnostics
    /// and exit 0, where weakening `yanked` itself at least still prints
    /// `warning[yanked]`.
    #[test]
    fn disabling_yank_checking_beside_a_denied_yanked_fails() {
        let deny = "[advisories]\nyanked = \"deny\"\ndisable-yank-checking = true\n";
        let error = check_yanked_is_denied(deny)
            .expect_err("a kill switch beside the policy key must fail");
        assert!(error.contains("disable-yank-checking"), "{error}");
        assert!(error.contains("no yanked diagnostic at all"), "{error}");
    }

    /// The allowed half of that key, measured rather than assumed: an explicit
    /// `false` exits 1 on the yanked lock exactly like the absent key, so it
    /// must not fail the guard.
    #[test]
    fn an_explicit_false_disable_yank_checking_passes() {
        let deny = "[advisories]\nyanked = \"deny\"\ndisable-yank-checking = false\n";
        assert!(check_yanked_is_denied(deny).is_ok());
    }

    /// The guard fails closed on anything it does not recognise, rather than
    /// ignoring the value and reporting the config as correct. Only a literal
    /// `false` (or no key at all) was measured as still enforcing.
    #[test]
    fn an_unrecognised_disable_yank_checking_value_fails() {
        // (A quoted `"false"` is not in this list: the reader strips quotes, so
        // it reads as `false` and passes. That is the right side to err on —
        // cargo-deny rejects a string for a boolean key by itself, with
        // `error[wanted]: expected a boolean`, so the config fails closed.)
        for value in ["1", "off", "true # really", ""] {
            let deny = format!("yanked = \"deny\"\ndisable-yank-checking = {value}\n");
            assert!(
                check_yanked_is_denied(&deny).is_err(),
                "`disable-yank-checking = {value}` must not pass"
            );
        }
    }

    #[test]
    fn the_dev_licence_policy_the_repository_ships_passes() {
        assert!(check_dev_licenses_are_checked("include-dev = true\n").is_ok());
    }

    /// The regression itself, and the one the repository shipped until SAK-234:
    /// the key simply absent. cargo-deny's default is to skip dev
    /// dependencies, so the allow list reads as if it governed the whole tree
    /// while governing only what Fireup ships. Measured on the stack tip with
    /// the MPL-2.0 `smartstring` 1.0.1 in `[dev-dependencies]`: exit 0 and
    /// `licenses ok`, where `include-dev = true` exits 4.
    #[test]
    fn omitting_the_dev_licence_setting_fails() {
        let deny = "[licenses]\nallow = [\"MIT\"]\nconfidence-threshold = 0.93\n";
        let error = check_dev_licenses_are_checked(deny).expect_err("a missing setting must fail");
        assert!(error.contains("not licence-checked at all"), "{error}");
    }

    /// Turning it off explicitly is the same hole, written down.
    #[test]
    fn switching_the_dev_licence_check_off_fails() {
        let error =
            check_dev_licenses_are_checked("include-dev = false\n").expect_err("`false` must fail");
        assert!(error.contains("error[rejected]"), "{error}");
    }

    /// The reason this key needs two assertions, and the arm that fails without
    /// the second one. `include-dev = true` is untouched and the dev
    /// dependencies are gone from the graph before the check sees them:
    /// measured on the stack tip, exit 0 with no rejection printed at all.
    #[test]
    fn excluding_dev_dependencies_from_the_graph_beside_include_dev_fails() {
        let deny = "[graph]\nexclude-dev = true\n\n[licenses]\ninclude-dev = true\n";
        let error = check_dev_licenses_are_checked(deny)
            .expect_err("a kill switch beside the policy key must fail");
        assert!(error.contains("exclude-dev"), "{error}");
        assert!(error.contains("prints no rejection at all"), "{error}");
    }

    /// The allowed half of that key, measured rather than assumed: an explicit
    /// `false` exits 4 on the MPL-2.0 dev dependency exactly like the absent
    /// key, so it must not fail the guard. This is the control for the arm
    /// above — without it, that test would pass on the key merely being
    /// present.
    #[test]
    fn an_explicit_false_exclude_dev_passes() {
        let deny = "[graph]\nexclude-dev = false\n\n[licenses]\ninclude-dev = true\n";
        assert!(check_dev_licenses_are_checked(deny).is_ok());
    }

    /// Both halves fail closed on a value neither was measured at, rather than
    /// ignoring it and reporting the config as correct.
    ///
    /// (A quoted `"true"` is not in the `include-dev` list and a quoted
    /// `"false"` is not in the `exclude-dev` one: the reader strips quotes, so
    /// each reads as its bare keyword. That is the right side to err on —
    /// cargo-deny rejects a string for a boolean key by itself, so the config
    /// fails closed without this guard's help.)
    #[test]
    fn an_unrecognised_dev_graph_value_fails() {
        for value in ["1", "on", "yes", ""] {
            let deny = format!("include-dev = {value}\n");
            assert!(
                check_dev_licenses_are_checked(&deny).is_err(),
                "`include-dev = {value}` must not pass"
            );
        }
        for value in ["1", "true # really", "on", ""] {
            let deny = format!("include-dev = true\nexclude-dev = {value}\n");
            assert!(
                check_dev_licenses_are_checked(&deny).is_err(),
                "`exclude-dev = {value}` must not pass"
            );
        }
    }

    /// `deny.toml` quotes both of these keys in the prose that explains them —
    /// the `include-dev` block names `exclude-dev` four times, including in a
    /// measured arm written as `+ [graph] exclude-dev = true`. None of that may
    /// be read as the setting, or deleting the live line would still pass.
    #[test]
    fn the_comments_about_the_dev_graph_are_not_read_as_the_settings() {
        let deny = "\
# `include-dev = true` is what puts dev dependencies in the licence check, and
# `[graph] exclude-dev = true` would take them back out without touching it.
confidence-threshold = 0.93
";
        assert!(check_dev_licenses_are_checked(deny).is_err());

        // A commented-out good value must not rescue a live weakened one.
        let deny = "\
# include-dev = true
include-dev = false
";
        assert!(check_dev_licenses_are_checked(deny).is_err());

        // And a commented-out kill switch is not a live one.
        let deny = "\
include-dev = true
#   + `[graph] exclude-dev = true`          exit 0   <- hole
";
        assert!(check_dev_licenses_are_checked(deny).is_ok());
    }

    #[test]
    fn the_unused_allowed_source_level_the_repository_ships_passes() {
        assert!(check_unused_allowed_source_is_deny("unused-allowed-source = \"deny\"\n").is_ok());
    }

    /// The regression itself, and the quiet one: `"warn"` is the cargo-deny
    /// default, so this is what the file said before SAK-234 and what deleting
    /// the line goes back to. Measured on the stack tip with a second entry in
    /// `allow-registry`: `"deny"` exits 8 with `error[unmatched-source]`, and
    /// the identical tree without the key exits 0 with only
    /// `warning[unmatched-source]`.
    #[test]
    fn omitting_the_unused_allowed_source_level_fails() {
        let deny = "[sources]\nunknown-registry = \"deny\"\nunknown-git = \"deny\"\n";
        let error =
            check_unused_allowed_source_is_deny(deny).expect_err("a missing level must fail");
        assert!(error.contains("default is"), "{error}");
        assert!(error.contains("SAK-221"), "{error}");
    }

    #[test]
    fn warning_on_an_unused_allowed_source_fails() {
        let error = check_unused_allowed_source_is_deny("unused-allowed-source = \"warn\"\n")
            .expect_err("\"warn\" must fail");
        assert!(error.contains("error[unmatched-source]"), "{error}");
    }

    /// `"allow"` is the third level cargo-deny accepts, and the one that drops
    /// the diagnostic entirely.
    #[test]
    fn allowing_an_unused_allowed_source_fails() {
        assert!(
            check_unused_allowed_source_is_deny("unused-allowed-source = \"allow\"\n").is_err()
        );
    }

    /// The `[sources]` prose names this key in a measured arm written as
    /// `+ unused-allowed-source = "deny"`, and names the licence-side twin it
    /// deliberately does not set. Neither may be read as the setting.
    #[test]
    fn the_comment_about_the_unused_allowed_source_level_is_not_read_as_the_level() {
        let deny = "\
#   + `unused-allowed-source = \"deny\"`      exit 8   `error[unmatched-source]`
unknown-registry = \"deny\"
";
        assert!(check_unused_allowed_source_is_deny(deny).is_err());

        // And a commented-out good value must not rescue a live weakened one.
        let deny = "\
# unused-allowed-source = \"deny\"
unused-allowed-source = \"warn\"
";
        assert!(check_unused_allowed_source_is_deny(deny).is_err());

        // The licence-side twin is named in the same block and is a different
        // key, so it must not be read as this one.
        let deny = "unused-allowed-license = \"deny\"\n";
        assert!(check_unused_allowed_source_is_deny(deny).is_err());
    }

    /// `deny.toml` explains both of these settings in prose above them, so a
    /// naive `contains` finds the comment rather than the setting. Neither
    /// direction may be confused.
    #[test]
    fn the_comments_about_yanked_are_not_read_as_the_settings() {
        let deny = "\
# `yanked = \"deny\"` makes a withdrawn version an error rather than a warning,
# and `disable-yank-checking = true` would switch it off without touching it.
unmaintained = \"all\"
";
        assert!(check_yanked_is_denied(deny).is_err());

        // A commented-out good value must not rescue a live weakened one.
        let deny = "\
# yanked = \"deny\"
yanked = \"warn\"
";
        assert!(check_yanked_is_denied(deny).is_err());

        // And a commented-out kill switch is not a live one.
        let deny = "\
yanked = \"deny\"
# disable-yank-checking = true
";
        assert!(check_yanked_is_denied(deny).is_ok());
    }

    #[test]
    fn the_graph_the_repository_ships_passes() {
        assert!(check_graph_covers_every_target("[graph]\nall-features = true\n").is_ok());
    }

    /// The regression itself: one line, and the whole platform-only half of the
    /// tree stops being checked. Measured on the stack tip with the vulnerable
    /// `shlex` 1.2.0 as a `cfg(windows)` dependency — no list exits 1 and names
    /// RUSTSEC-2024-0006, this list exits 0 with `advisories ok`.
    #[test]
    fn restricting_the_graph_to_one_target_fails() {
        let deny = "[graph]\ntargets = [\"x86_64-unknown-linux-gnu\"]\nall-features = true\n";
        let error = check_graph_covers_every_target(deny).expect_err("a target list must not pass");
        assert!(error.contains("RUSTSEC-2024-0006"), "{error}");
        assert!(error.contains("217 crates to 173"), "{error}");
    }

    /// A list long enough to look thorough still is not every target. These two
    /// keep the `windows-*` family and drop `wasi`, `wasm-bindgen` and the
    /// `objc2-*` tree, so the hole is narrower and still open.
    #[test]
    fn a_longer_target_list_that_still_leaves_platforms_out_fails() {
        let deny = "\
targets = [
  \"x86_64-unknown-linux-gnu\",
  \"x86_64-pc-windows-msvc\",
]
";
        assert!(check_graph_covers_every_target(deny).is_err());
    }

    /// cargo-deny's other spelling for the same key, and it is not cosmetic:
    /// measured on the stack tip with the MPL-2.0 `smartstring` as a
    /// `cfg(windows)` dependency, `targets = [{ triple = "…linux-gnu" }]` exits
    /// 0 with `licenses ok` exactly like the string form. A reader that only
    /// understood the string form would call this unrestricted.
    #[test]
    fn the_table_form_of_a_target_fails_too() {
        let deny = "targets = [{ triple = \"x86_64-unknown-linux-gnu\" }]\n";
        let error = check_graph_covers_every_target(deny)
            .expect_err("the table form narrows the graph the same way");
        assert!(error.contains("x86_64-unknown-linux-gnu"), "{error}");
    }

    /// The allowed half, measured rather than assumed: an empty list leaves the
    /// checked set at 217 crates, the same as no list at all, so it must not
    /// fail the guard. Without this the test above would pass on the key merely
    /// being present.
    #[test]
    fn an_empty_target_list_passes() {
        assert!(check_graph_covers_every_target("targets = []\n").is_ok());
        assert!(check_graph_covers_every_target("targets = [\n]\n").is_ok());
    }

    /// A live `targets` line this reader cannot parse fails rather than being
    /// reported as no restriction at all. cargo-deny also rejects this form
    /// today (`error[wanted]: expected an array`, exit 1, measured), so the
    /// branch is belt and braces — but a shape the reader does not understand
    /// must never be the reason it passes.
    #[test]
    fn a_target_line_that_is_not_an_array_fails() {
        let error = check_graph_covers_every_target("targets = \"x86_64-unknown-linux-gnu\"\n")
            .expect_err("a scalar must not be read as no restriction");
        assert!(error.contains("not an array"), "{error}");
    }

    /// `deny.toml` explains in prose why this key is absent, and this change
    /// adds a measured arms table above `[graph]` that writes the setting out
    /// four times — including as `+ targets = ["x86_64-unknown-linux-gnu"]`.
    /// None of that may be read as the setting, or deleting the comment would
    /// "fix" a file that has the live line in it.
    #[test]
    fn the_comments_about_targets_are_not_read_as_the_setting() {
        let deny = "\
# No `targets` list on purpose. Restricting the graph to Linux would hide
# advisories and licences on the Windows/macOS/wasi crates.
#   + `targets = [\"x86_64-unknown-linux-gnu\"]`   173 crates
all-features = true
";
        assert!(check_graph_covers_every_target(deny).is_ok());

        // And a commented-out key must not rescue a live one beside it.
        let deny = "\
# targets = []
targets = [\"x86_64-unknown-linux-gnu\"]
";
        assert!(check_graph_covers_every_target(deny).is_err());

        // Nor may a trailing comment hide a live entry.
        let deny = "targets = [\"x86_64-unknown-linux-gnu\"]  # just for CI speed\n";
        assert!(check_graph_covers_every_target(deny).is_err());
    }

    #[test]
    fn the_unsound_scope_the_repository_ships_passes() {
        assert!(check_unsound_scope_is_all("unsound = \"all\"\n").is_ok());
    }

    /// The regression, and the value the file ran at before SAK-236 without
    /// ever saying so. Measured: identical to the key being absent.
    #[test]
    fn narrowing_the_unsound_scope_to_the_workspace_fails() {
        let error = check_unsound_scope_is_all("unsound = \"workspace\"\n")
            .expect_err("\"workspace\" must fail");
        assert!(
            error.contains("RUSTSEC-2024-0404") && error.contains("workspace"),
            "{error}"
        );
    }

    /// `"transitive"` is red on the measured tree, but it is still a narrowing:
    /// it drops the workspace's own crates. The guard wants "all", not "any
    /// value that happened to be red in one experiment".
    #[test]
    fn narrowing_the_unsound_scope_to_transitive_only_fails() {
        assert!(check_unsound_scope_is_all("unsound = \"transitive\"\n").is_err());
    }

    /// The kill switch. One word and unsound advisories stop being reported at
    /// all — measured with `const-cstr` 0.3.0 as a direct dependency, where it
    /// takes the check from exit 1 to `advisories ok`.
    #[test]
    fn disabling_the_unsound_check_fails() {
        assert!(check_unsound_scope_is_all("unsound = \"none\"\n").is_err());
    }

    /// The quietest form of the regression and the one that actually shipped:
    /// the key is simply not there, cargo-deny defaults to `"workspace"`, and
    /// the file reads as though the question was never asked. This is the arm
    /// that differs from `unmaintained`, which was at least written down.
    #[test]
    fn omitting_the_unsound_setting_entirely_fails() {
        let deny = "[advisories]\nyanked = \"deny\"\nunmaintained = \"all\"\nignore = []\n";
        let error = check_unsound_scope_is_all(deny).expect_err("an absent key must fail");
        assert!(error.contains("workspace"), "{error}");
    }

    /// `deny.toml` now writes this key out five times in the arms table above
    /// it, including `unsound = "workspace"` and `unsound = "none"`. None of
    /// those may be read as the setting, or deleting a comment would "fix" a
    /// file whose live line is wrong — and documenting the measurement would
    /// break the guard that the measurement justified.
    #[test]
    fn the_comments_about_the_unsound_scope_are_not_read_as_the_scope() {
        let deny = "\
#   no key (what shipped)      exit 0   `advisories ok`
#   `unsound = \"all\"`          exit 1   RUSTSEC-2024-0404
#   `unsound = \"workspace\"`    exit 0   control: identical to no key
#   `unsound = \"none\"`         exit 0   control: the kill switch
unsound = \"all\"
";
        assert!(check_unsound_scope_is_all(deny).is_ok());

        // A commented-out good value must not rescue a live bad one.
        let deny = "# unsound = \"all\"\nunsound = \"workspace\"\n";
        assert!(check_unsound_scope_is_all(deny).is_err());

        // Nor may a trailing comment hide the live value.
        let deny = "unsound = \"none\"  # back to all once the backlog is clear\n";
        assert!(check_unsound_scope_is_all(deny).is_err());

        // And prose alone is not the setting.
        let deny = "# `unsound` is the sibling scope enum of `unmaintained`.\n";
        assert!(check_unsound_scope_is_all(deny).is_err());
    }

    #[test]
    fn the_advisory_database_the_repository_ships_passes() {
        let deny = "db-urls = [\"https://github.com/rustsec/advisory-db\"]\n";
        assert!(check_advisory_db_is_the_official_one(deny).is_ok());
    }

    /// The hole, as measured: a fork of advisory-db with one advisory file
    /// deleted takes the check from exit 1 to `advisories ok` while every
    /// `deny` in the file stays exactly where it was.
    #[test]
    fn a_thinned_fork_of_the_advisory_database_fails() {
        let deny = "db-urls = [\"https://github.com/acme/advisory-db\"]\n";
        let error = check_advisory_db_is_the_official_one(deny)
            .expect_err("a fork must not be accepted as the advisory source");
        assert!(
            error.contains("RUSTSEC-2024-0006") && error.contains("rustsec/advisory-db"),
            "{error}"
        );
    }

    /// Absence passes, and this is measured rather than assumed: with the key
    /// deleted the check still exits 1 and names RUSTSEC-2024-0006, because the
    /// cargo-deny default is this same database. Failing on absence would be
    /// wrong *and* would make the guard a nuisance.
    #[test]
    fn omitting_the_database_url_passes() {
        let deny = "[advisories]\nyanked = \"deny\"\nunsound = \"all\"\nignore = []\n";
        assert!(check_advisory_db_is_the_official_one(deny).is_ok());
    }

    /// An empty list is the default too (measured: exit 1, advisory named), so
    /// it must not fail. Without this, the guard would be passing on the key
    /// merely containing something.
    #[test]
    fn an_empty_database_url_list_passes() {
        assert!(check_advisory_db_is_the_official_one("db-urls = []\n").is_ok());
        assert!(check_advisory_db_is_the_official_one("db-urls = [\n]\n").is_ok());
    }

    /// Several URLs are a union, measured: the thinned fork beside the official
    /// one still exits 1 and names the advisory. So an extra entry can only add
    /// advisories, and requiring `== [official]` would forbid a harmless thing.
    #[test]
    fn an_extra_database_url_beside_the_official_one_passes() {
        let deny = "db-urls = [\"https://github.com/acme/advisory-db\", \
                    \"https://github.com/rustsec/advisory-db\"]\n";
        assert!(check_advisory_db_is_the_official_one(deny).is_ok());

        // Including when the array is reformatted across lines, which is how a
        // second entry would most naturally arrive.
        let deny = "db-urls = [\n  \"https://github.com/acme/advisory-db\",\n  \
                    \"https://github.com/rustsec/advisory-db\",\n]\n";
        assert!(check_advisory_db_is_the_official_one(deny).is_ok());
    }

    /// The spellings that name the same repository. The `.git` suffix was
    /// measured to work (exit 1, same advisory), so none of these may be the
    /// reason the guard fails.
    #[test]
    fn the_other_spellings_of_the_official_database_pass() {
        for url in [
            "https://github.com/rustsec/advisory-db.git",
            "https://github.com/rustsec/advisory-db/",
            "https://github.com/rustsec/advisory-db.git/",
            "https://github.com/RustSec/advisory-db",
        ] {
            let deny = format!("db-urls = [\"{url}\"]\n");
            assert!(
                check_advisory_db_is_the_official_one(&deny).is_ok(),
                "{url} names the official database"
            );
        }
    }

    /// A live line this reader cannot parse fails closed rather than being
    /// reported as the official source. cargo-deny rejects the scalar form
    /// itself today (`error[wanted]: expected an array`, measured), so this is
    /// belt and braces — same reasoning as the `targets` scalar branch.
    #[test]
    fn a_database_url_line_that_is_not_an_array_fails() {
        let deny = "db-urls = \"https://github.com/rustsec/advisory-db\"\n";
        let error = check_advisory_db_is_the_official_one(deny)
            .expect_err("a scalar must not be read as the official database");
        assert!(error.contains("not an array"), "{error}");
    }

    /// `deny.toml` quotes this URL in prose above the live line, and the arms
    /// table describes forks of it. None of that may stand in for the setting,
    /// in either direction.
    #[test]
    fn the_comments_about_the_database_are_not_read_as_the_setting() {
        // Prose mentioning the official URL must not rescue a live fork.
        let deny = "\
# Where the advisories come from: https://github.com/rustsec/advisory-db
#   official URL (ships)    exit 1   `error[vulnerability]` named
db-urls = [\"https://github.com/acme/advisory-db\"]
";
        assert!(check_advisory_db_is_the_official_one(deny).is_err());

        // A commented-out key must not count as the key being present either —
        // absence is legal, so this has to pass for the right reason.
        let deny = "# db-urls = [\"https://github.com/acme/advisory-db\"]\nignore = []\n";
        assert!(check_advisory_db_is_the_official_one(deny).is_ok());

        // Nor may a trailing comment hide a live fork.
        let deny = "db-urls = [\"https://github.com/acme/advisory-db\"]  # mirror, faster\n";
        assert!(check_advisory_db_is_the_official_one(deny).is_err());
    }

    /// The `[bans.build]` table in the shape `deny.toml` ships it, with the
    /// eight `protoc` crates collapsed to two and the checksums truncated. The
    /// shape is what is under test; the real file is checked by
    /// `deny_toml_scans_the_binaries_the_build_runs_and_pins_every_exemption`
    /// above, and the checksum *values* are checked by cargo-deny itself.
    fn build_table(bypass: &str) -> String {
        format!("[bans.build]\nexecutables = \"deny\"\ninclude-dependencies = true\n{bypass}")
    }

    const SHIPPED_BYPASS: &str = "\
bypass = [
  { crate = \"protoc-bin-vendored-linux-x86_64\", allow = [{ path = \"bin/protoc\", checksum = \"f6ee22d6\" }] },
  { crate = \"protoc-bin-vendored-win32\", allow = [{ path = \"bin/protoc.exe\", checksum = \"ab254642\" }] },
]
";

    #[test]
    fn the_build_scan_the_repository_ships_passes() {
        assert!(check_build_executables_are_pinned(&build_table(SHIPPED_BYPASS)).is_ok());
    }

    /// The hole as it actually shipped: no table at all, so cargo-deny looked at
    /// no build script and no bundled executable anywhere in the tree and
    /// reported `bans ok`.
    #[test]
    fn omitting_the_build_table_entirely_fails() {
        let deny = "[bans]\nmultiple-versions = \"warn\"\nwildcards = \"deny\"\ndeny = []\n";
        let error = check_build_executables_are_pinned(deny)
            .expect_err("a missing [bans.build] must not pass");
        assert!(error.contains("no `[bans.build] executables`"), "{error}");
    }

    #[test]
    fn allowing_executables_fails() {
        for level in ["allow", "warn"] {
            let deny = build_table(SHIPPED_BYPASS).replace(
                "executables = \"deny\"",
                &format!("executables = \"{level}\""),
            );
            assert!(
                check_build_executables_are_pinned(&deny).is_err(),
                "executables = {level:?} must fail"
            );
        }
    }

    /// Trap 1, and the reason this guard needs two value assertions rather than
    /// one. `executables = "deny"` on its own is **inert**: measured on this tree
    /// (SAK-237) it exits 0 with zero findings, because `include-dependencies`
    /// defaults to false and Fireup's own workspace bundles no binaries. A guard
    /// that read only the first key would call that config correct.
    #[test]
    fn omitting_the_include_dependencies_key_fails() {
        let deny = format!("[bans.build]\nexecutables = \"deny\"\n{SHIPPED_BYPASS}");
        let error = check_build_executables_are_pinned(&deny)
            .expect_err("a workspace-only scan must not pass as enforcement");
        assert!(
            error.contains("no `[bans.build] include-dependencies`"),
            "{error}"
        );
    }

    /// The same hole written out loud. `false` is the default, so this and the
    /// test above are the same regression in two spellings — and the explicit
    /// one is the more plausible edit, because it looks like a decision.
    #[test]
    fn an_explicit_false_include_dependencies_fails() {
        let deny = build_table(SHIPPED_BYPASS).replace(
            "include-dependencies = true",
            "include-dependencies = false",
        );
        let error = check_build_executables_are_pinned(&deny)
            .expect_err("include-dependencies = false must not pass");
        assert!(
            error.contains("scopes the executable scan to the workspace"),
            "{error}"
        );
    }

    /// Trap 2. A `bypass` entry that is only a crate name exempts *nothing*:
    /// measured on this tree (SAK-237) the finding count stays at eight. It is
    /// the shape that most looks like an exemption without being one, so it has
    /// to fail rather than be quietly tolerated as harmless.
    #[test]
    fn a_bypass_entry_with_only_a_crate_name_fails() {
        let deny = build_table("bypass = [{ crate = \"protoc-bin-vendored-linux-x86_64\" }]\n");
        let error = check_build_executables_are_pinned(&deny)
            .expect_err("a bare crate name must not pass as an exemption");
        assert!(error.contains("exempts no path"), "{error}");
    }

    /// The shape that is green *today* and wrong *later*: `allow-globs` exempts
    /// whatever the crate ever puts in `bin/`. Measured on all eight crates it
    /// is exit 0 and `bans ok`, indistinguishable from the checksum form until a
    /// second binary appears upstream.
    #[test]
    fn a_bypass_entry_with_a_glob_fails() {
        let deny = build_table(
            "bypass = [{ crate = \"protoc-bin-vendored-linux-x86_64\", \
             allow-globs = [\"bin/*\"] }]\n",
        );
        let error = check_build_executables_are_pinned(&deny)
            .expect_err("a glob must not pass as a pinned exemption");
        assert!(error.contains("allow-globs"), "{error}");

        // Including beside a properly pinned path in the same entry: the glob
        // still exempts everything else in that directory.
        let deny = build_table(
            "bypass = [{ crate = \"protoc-bin-vendored-linux-x86_64\", \
             allow = [{ path = \"bin/protoc\", checksum = \"f6ee22d6\" }], \
             allow-globs = [\"bin/*\"] }]\n",
        );
        assert!(check_build_executables_are_pinned(&deny).is_err());
    }

    /// Trap 3, found while auditing the keys this change does *not* set, and the
    /// quietest of the three: `checksum` is optional in cargo-deny's schema, so
    /// `allow = [{ path }]` is a one-token deletion that exits 0 with `bans ok`
    /// and zero findings while the bytes at that path are no longer pinned at
    /// all.
    #[test]
    fn a_bypass_entry_with_a_path_and_no_checksum_fails() {
        let deny = build_table(
            "bypass = [{ crate = \"protoc-bin-vendored-linux-x86_64\", \
             allow = [{ path = \"bin/protoc\" }] }]\n",
        );
        let error =
            check_build_executables_are_pinned(&deny).expect_err("an unpinned path must not pass");
        assert!(
            error.contains("1 exempted path(s) but 0 checksum(s)"),
            "{error}"
        );
    }

    /// And partially: one entry may list several paths, so finding *a* checksum
    /// is not the same as every path having one.
    #[test]
    fn a_bypass_entry_with_one_checksum_for_two_paths_fails() {
        let deny = build_table(
            "bypass = [{ crate = \"protoc-bin-vendored-linux-x86_64\", allow = [\
             { path = \"bin/protoc\", checksum = \"f6ee22d6\" }, \
             { path = \"bin/protoc-gen-go\" }] }]\n",
        );
        let error = check_build_executables_are_pinned(&deny)
            .expect_err("a second unpinned path must not pass");
        assert!(
            error.contains("2 exempted path(s) but 1 checksum(s)"),
            "{error}"
        );
    }

    /// No bypass list at all is the strictest configuration there is, so it has
    /// to pass. (It would make *this* tree red, because the eight `protoc`
    /// binaries are really there — but that is cargo-deny's job to report, and
    /// a guard that failed on it would be forbidding the strict case.)
    #[test]
    fn an_absent_or_empty_bypass_list_passes() {
        assert!(check_build_executables_are_pinned(&build_table("")).is_ok());
        assert!(check_build_executables_are_pinned(&build_table("bypass = []\n")).is_ok());
        assert!(check_build_executables_are_pinned(&build_table("bypass = [\n]\n")).is_ok());
    }

    /// Fail closed on a shape this reader does not understand, for the reason
    /// SAK-235 gave for `[graph] targets`: a form the guard cannot parse must
    /// never be the thing that makes it pass.
    #[test]
    fn a_bypass_line_this_reader_cannot_parse_fails() {
        for bypass in [
            // Not an array.
            "bypass = \"protoc-bin-vendored-linux-x86_64\"\n",
            // An array that never closes.
            "bypass = [\n  { crate = \"protoc-bin-vendored-linux-x86_64\",\n",
            // A bare value where an inline table belongs.
            "bypass = [\"protoc-bin-vendored-linux-x86_64\"]\n",
            // Braces that do not balance.
            "bypass = [ } ]\n",
        ] {
            let deny = build_table(bypass);
            let error = check_build_executables_are_pinned(&deny)
                .expect_err("an unreadable bypass line must not be read as a pinned list");
            assert!(
                error.contains("cannot read as an array"),
                "{bypass:?}: {error}"
            );
        }
    }

    /// `deny.toml` quotes every one of these keys in the prose that explains
    /// them, including the values that must fail, and the arms table above the
    /// live lines spells out `executables = "allow"` and `allow = [{ path }]`.
    /// None of that may stand in for — or rescue — the live setting.
    #[test]
    fn the_comments_about_the_build_scan_are_not_read_as_the_settings() {
        // Prose quoting the good value must not rescue a live bad one.
        let deny = "\
[bans.build]
#   `executables = \"deny\"` alone          exit 0   0 findings
#   + `include-dependencies = true`       exit 2   8 findings
executables = \"allow\"
include-dependencies = true
";
        assert!(check_build_executables_are_pinned(deny).is_err());

        // A commented-out key is not the key being present.
        let deny = "\
[bans.build]
executables = \"deny\"
# include-dependencies = true
";
        assert!(check_build_executables_are_pinned(deny).is_err());

        // Nor may a trailing comment hide the live value.
        let deny = "\
[bans.build]
executables = \"deny\"
include-dependencies = false  # workspace is enough for now
";
        assert!(check_build_executables_are_pinned(deny).is_err());

        // A commented-out bypass list is not the list, and absence is legal —
        // so this has to pass for the right reason.
        let deny = "\
[bans.build]
executables = \"deny\"
include-dependencies = true
# bypass = [{ crate = \"protoc-bin-vendored-linux-x86_64\", allow-globs = [\"bin/*\"] }]
";
        assert!(check_build_executables_are_pinned(deny).is_ok());

        // And a trailing comment on a live entry must not hide its shape.
        let deny = build_table(
            "bypass = [{ crate = \"protoc-bin-vendored-linux-x86_64\", \
             allow-globs = [\"bin/*\"] }]  # pending real checksums\n",
        );
        assert!(check_build_executables_are_pinned(&deny).is_err());
    }

    /// The readers this guard is built on, exercised directly. Without this the
    /// `allow` / `allow-globs` prefix rule and the entry splitter are only
    /// tested through whole configs, where a wrong answer can be masked by
    /// another branch reaching the same verdict.
    #[test]
    fn the_entry_reader_reads_the_shapes_it_claims_to() {
        // `allow` must not match `allow-globs`, and `path` must not match a
        // crate whose name happens to end in `path`.
        assert!(has_key("{ allow-globs = [\"bin/*\"] }", "allow-globs"));
        assert!(!has_key("{ allow-globs = [\"bin/*\"] }", "allow"));
        assert!(has_key("{ allow = [{ path = \"a\" }] }", "allow"));
        assert!(!has_key("{ crate = \"camino-path\" }", "path"));
        assert_eq!(count_key("{ path = \"a\" }, { path = \"b\" }", "path"), 2);
        assert_eq!(count_key("{ crate = \"x\" }", "checksum"), 0);

        // Entries split at depth one, with the nested `allow` array kept inside
        // the entry it belongs to.
        let deny =
            "bypass = [\n  { crate = \"a\", allow = [{ path = \"p\", checksum = \"c\" }] },\n  \
                    { crate = \"b\", allow-globs = [\"bin/*\"] },\n]\n";
        let entries = bypass_entries(deny).expect("a two-entry array is readable");
        assert_eq!(entries.len(), 2);
        assert!(entries[0].contains("crate = \"a\"") && entries[0].contains("checksum"));
        assert!(entries[1].contains("allow-globs"));

        // A single-line array reads the same way.
        let entries = bypass_entries("bypass = [{ crate = \"a\" }, { crate = \"b\" }]\n")
            .expect("a one-line array is readable");
        assert_eq!(entries.len(), 2);

        // No key at all is "no list", which is distinct from an unreadable one.
        assert_eq!(bypass_entries("executables = \"deny\"\n"), None);
        assert_eq!(bypass_entries("bypass = []\n"), Some(Vec::new()));
        assert_eq!(bypass_entries("bypass = \"nope\"\n"), None);
    }

    /// The shipping shape is the key being *absent*, which is the half of this
    /// guard that reads backwards next to every other check in this file.
    /// cargo-deny's default for `include-build` is `true`, so there is no line
    /// to require and nothing for a reader to find.
    #[test]
    fn the_build_licence_policy_the_repository_ships_passes() {
        let deny =
            "[licenses]\nallow = [\"MIT\"]\nconfidence-threshold = 0.93\ninclude-dev = true\n";
        assert!(check_build_licenses_are_checked(deny).is_ok());
    }

    /// The regression itself. One line, and the 21 crates Fireup's build
    /// compiles and executes — `protoc-bin-vendored` and its eight platform
    /// binaries, `prost-build`, `tonic-build`, and their build-only tail — stop
    /// having their licences checked. Measured on the stack tip with the
    /// MPL-2.0 `smartstring` 1.0.1 inserted into `[build-dependencies]`: exit 0
    /// and `licenses ok`, where the absent key exits 4.
    #[test]
    fn switching_the_build_licence_check_off_fails() {
        let error = check_build_licenses_are_checked("include-build = false\n")
            .expect_err("`false` must fail");
        assert!(error.contains("217 crates to 196"), "{error}");
        assert!(error.contains("error[rejected]"), "{error}");
    }

    /// The control for the arm above, measured rather than assumed: writing the
    /// default out by hand exits 4 on the MPL-2.0 build dependency exactly like
    /// the absent key, so it must not fail the guard. Without this test, the
    /// one above would pass on the key merely being present — which is the
    /// mistake that would make this guard demand a redundant line instead of
    /// forbidding a harmful one.
    #[test]
    fn an_explicit_true_include_build_passes() {
        assert!(check_build_licenses_are_checked("include-build = true\n").is_ok());
    }

    /// Fails closed on a value it was not measured at, rather than reading an
    /// unfamiliar form as the permissive default and reporting the config as
    /// correct. cargo-deny rejects every one of these by itself (`error[wanted]`,
    /// exit 1), so this is belt and braces — but the guard must not be the thing
    /// that waves them through.
    ///
    /// (A quoted `"false"` is not in this list: the reader strips quotes, so it
    /// reads as the bare keyword and lands on the measured `false` arm. Failing
    /// is the right answer either way.)
    #[test]
    fn an_unrecognised_include_build_value_fails() {
        for value in ["1", "0", "off", "no", "false # really", ""] {
            let deny = format!("include-build = {value}\n");
            assert!(
                check_build_licenses_are_checked(&deny).is_err(),
                "`include-build = {value}` must not pass"
            );
        }
    }

    /// The inverted twin of `the_comments_about_the_dev_graph_are_not_read_as_the_settings`,
    /// and the one this key actually needs: `deny.toml` writes `include-build =
    /// false` out in prose, inside the measured table that explains why the key
    /// is absent. For `include-dev` the danger was a commented-out *good* value
    /// rescuing a live bad one. Here it is the opposite — a commented-out *bad*
    /// value must not be mistaken for a live one, or this guard would fail on
    /// the documentation of itself.
    #[test]
    fn the_comment_about_build_licences_is_not_read_as_the_setting() {
        let deny = "\
# Measured (SAK-238): no key is 217 crates, `include-build = false` is 196.
#   include-build = false
confidence-threshold = 0.93
";
        assert!(check_build_licenses_are_checked(deny).is_ok());

        // And a commented-out default must not rescue a live weakened value.
        let deny = "\
# include-build = true
include-build = false
";
        assert!(check_build_licenses_are_checked(deny).is_err());
    }

    /// The pattern the real log step greps for.
    ///
    /// The arms below measure CI's actual `grep`, not a copy of it kept in this
    /// file. If someone edits the step so that the entry shape this repository
    /// documents stops matching, `the_entry_shape_the_log_step_can_read_passes`
    /// is where it shows up.
    fn grep() -> String {
        log_step_pattern(&repo_file(".github/workflows/ci.yml"))
            .expect("ci.yml must carry the log step this guard is built on")
    }

    /// `[advisories]` with `body` as the contents of the `ignore` array.
    fn advisories_with(body: &str) -> String {
        format!("[advisories]\nunmaintained = \"all\"\nignore = [\n{body}]\n\nyanked = \"deny\"\n")
    }

    /// The entry #77 actually shipped, copied character for character.
    const SHIPPED_ENTRY: &str = "  { id = \"RUSTSEC-2026-0258\", reason = \"h2 0.3.27 via pinned \
                                 tonic 0.11; local-only gRPC surface. Fix needs the tonic \
                                 0.12/prost 0.13 migration (SAK-220). Reconsider by 2027-02-06.\" \
                                 },\n";

    /// K1. The shape the repository ships and the log step was written for: one
    /// inline table on one line, with the id in the spelling the grep looks for
    /// and the reason — reconsider date included — beside it. Measured on #77:
    /// `cargo deny check advisories` exits 0 with the advisory suppressed, and
    /// the log step prints the id, the reason and the date.
    #[test]
    fn the_entry_shape_the_log_step_can_read_passes() {
        let deny = advisories_with(SHIPPED_ENTRY);
        assert!(
            check_ignore_entries_are_logged(&deny, &grep()).is_ok(),
            "the shipped entry shape must pass, or this guard forbids the only \
             shape CI can report"
        );
    }

    /// K2. The quietest break, and the one no reader would call a mistake:
    /// valid TOML, cargo-deny suppresses the advisory exactly as before, and
    /// the grep still prints a line — so an exception is still visible. What is
    /// lost is the reason and the reconsider date, which is the step's entire
    /// purpose.
    #[test]
    fn an_entry_split_across_lines_fails() {
        let deny = advisories_with(
            "  { id = \"RUSTSEC-2026-0258\",\n    \
             reason = \"h2 via pinned tonic 0.11. Reconsider by 2027-02-06.\" },\n",
        );
        let error = check_ignore_entries_are_logged(&deny, &grep())
            .expect_err("a two-line entry must fail");
        assert!(error.contains("spans more than one line"), "{error}");

        // The brace-on-its-own-line form is the same failure.
        let deny = advisories_with(
            "  {\n    id = \"RUSTSEC-2026-0258\",\n    \
             reason = \"h2 via pinned tonic 0.11. Reconsider by 2027-02-06.\",\n  },\n",
        );
        let error = check_ignore_entries_are_logged(&deny, &grep())
            .expect_err("a three-line entry must fail");
        assert!(error.contains("spans more than one line"), "{error}");
    }

    /// K3. A TOML literal string for the id. Still an id, still matched against
    /// the advisory database, still suppresses it — measured on #77 with
    /// `-D advisory-not-detected`, which is what rules out "the advisory simply
    /// is not there". The grep looks for a double quote, so the log says `none`.
    #[test]
    fn a_single_quoted_id_fails() {
        let deny = advisories_with(
            "  { id = 'RUSTSEC-2026-0258', \
             reason = \"h2 via pinned tonic 0.11. Reconsider by 2027-02-06.\" },\n",
        );
        let error = check_ignore_entries_are_logged(&deny, &grep())
            .expect_err("a single-quoted id must fail");
        assert!(
            error.contains("does not match the log step's grep"),
            "{error}"
        );
    }

    /// K4. The same silence from the smallest edit in the set: two spaces
    /// removed. TOML does not care, cargo-deny does not care — measured on #77
    /// with `-D advisory-not-detected`, exit 0 and suppressed — and the grep
    /// pattern has the spaces in it, so the log says `none`.
    #[test]
    fn an_id_without_spaces_around_the_equals_fails() {
        let deny = advisories_with(
            "  { id=\"RUSTSEC-2026-0258\", \
             reason = \"h2 via pinned tonic 0.11. Reconsider by 2027-02-06.\" },\n",
        );
        let error =
            check_ignore_entries_are_logged(&deny, &grep()).expect_err("`id=\"…\"` must fail");
        assert!(
            error.contains("does not match the log step's grep"),
            "{error}"
        );
    }

    /// K5. The control for K3 and K4, and the arm that makes their green mean
    /// something. With no entry at all the log step also prints `none` — so the
    /// log cannot tell "one advisory accepted" from "none accepted", which is
    /// the finding. Here the guard must pass: an empty list is the shipping
    /// state after SAK-220, and a guard that failed on it would be demanding an
    /// exception rather than a shape.
    #[test]
    fn no_entries_at_all_passes() {
        for deny in [
            advisories_with(""),
            "[advisories]\nignore = []\n".to_string(),
            // No key either: cargo-deny's default is the empty list.
            "[advisories]\nunmaintained = \"all\"\n".to_string(),
        ] {
            assert!(
                check_ignore_entries_are_logged(&deny, &grep()).is_ok(),
                "an empty ignore list must pass:\n{deny}"
            );
        }
    }

    /// K6. Without this the guard would only be checking a house style. The
    /// log step exists to put the *reconsider date* in front of someone on
    /// every run — cargo-deny has no expiry feature, so the date is the only
    /// thing that stops an acceptance from being permanent. An entry the grep
    /// finds but with no date in its reason has to fail.
    #[test]
    fn a_grep_visible_entry_with_no_review_date_fails() {
        let deny = advisories_with(
            "  { id = \"RUSTSEC-2026-0258\", \
             reason = \"h2 0.3.27 via pinned tonic 0.11; local-only gRPC surface.\" },\n",
        );
        let error = check_ignore_entries_are_logged(&deny, &grep())
            .expect_err("a reason with no date must fail");
        assert!(error.contains("no review date"), "{error}");

        // An id is not a date, which is why the scan reads the reason and not
        // the line: `RUSTSEC-2026-0258` is four digits, a dash and four more.
        assert_eq!(review_date("RUSTSEC-2026-0258"), None);
        assert_eq!(review_date("RUSTSEC-2024-0449"), None);
        assert_eq!(
            review_date("Reconsider by 2027-02-06."),
            Some("2027-02-06".to_string())
        );
        // Not a date-shaped number that happens to be ten characters long.
        assert_eq!(review_date("2027-13-06"), None);
        assert_eq!(review_date("2027-02-00"), None);
        assert_eq!(review_date("12027-02-06"), None);
        assert_eq!(review_date("2027-02-061"), None);
    }

    /// K7. Without this arm the guard could be pinning one advisory number
    /// rather than a shape. A second, differently-numbered entry written
    /// correctly has to pass — and its reason carries a comma, so the entry
    /// splitter is exercised on the thing that would make it split in the
    /// wrong place.
    #[test]
    fn a_second_correctly_written_entry_passes() {
        let deny = advisories_with(&format!(
            "{SHIPPED_ENTRY}  {{ id = \"RUSTSEC-2024-0449\", reason = \"unsafe-libyaml, reached \
             through a dev dependency on serde_yaml; archived, no code path in the shipped \
             binary. Reconsider by 2026-12-31.\" }},\n"
        ));
        assert!(
            check_ignore_entries_are_logged(&deny, &grep()).is_ok(),
            "a correctly written second entry must pass, or this guard is \
             pinning an advisory id instead of a shape"
        );
    }

    /// The other two valid spellings the grep cannot see. A bare string element
    /// is the shape cargo-deny's own documentation shows first, and it carries
    /// no reason at all; an entry with an `id` and nothing else is grep-visible
    /// but has nothing to log.
    #[test]
    fn an_entry_with_no_reason_fails() {
        let deny = advisories_with("  \"RUSTSEC-2026-0258\",\n");
        let error = check_ignore_entries_are_logged(&deny, &grep())
            .expect_err("a bare string element must fail");
        assert!(
            error.contains("does not match the log step's grep"),
            "{error}"
        );

        let deny = advisories_with("  { id = \"RUSTSEC-2026-0258\" },\n");
        let error = check_ignore_entries_are_logged(&deny, &grep())
            .expect_err("an entry with no reason must fail");
        assert!(error.contains("no `reason"), "{error}");
    }

    /// A commented-out entry is not a live one. `deny.toml` quotes advisory ids
    /// and whole entries in its prose as history — that is where the h2
    /// acceptance is now recorded — so a guard that read comments would fail on
    /// the documentation of itself. (The log step's `grep` does print such a
    /// line, which over-reports; this check is about what cargo-deny acts on.)
    #[test]
    fn a_commented_out_entry_is_not_read_as_live() {
        let deny = advisories_with(&format!("#{SHIPPED_ENTRY}"));
        assert!(check_ignore_entries_are_logged(&deny, &grep()).is_ok());

        // And a commented-out good entry must not rescue a live bad one.
        let deny = advisories_with(&format!(
            "#{SHIPPED_ENTRY}  {{ id='RUSTSEC-2026-0258', reason = \"x 2027-02-06\" }},\n"
        ));
        assert!(check_ignore_entries_are_logged(&deny, &grep()).is_err());
    }

    /// A live `ignore` whose value this reader cannot parse fails rather than
    /// passing vacuously — the same call SAK-235 made for `[graph] targets`.
    #[test]
    fn an_unreadable_ignore_list_fails() {
        for value in [
            // Not an array at all.
            "\"RUSTSEC-2026-0258\"",
            // An array that never closes.
            "[ { id = \"RUSTSEC-2026-0258\" ",
            // An escape this reader does not implement.
            "[ { id = \"RUSTSEC-2026-0258\", reason = \"needs a \\\"quote\\\" 2027-02-06\" } ]",
            // An *unbalanced* bracket inside a reason. The first pass counts
            // brackets before it knows it is inside a string, so the array
            // closes early and the cut falls mid-string — which lands here,
            // rather than parsing into something wrong. (A balanced pair reads
            // fine; see `the_ignore_reader_reads_the_shapes_it_claims_to`.)
            "[ { id = \"RUSTSEC-2026-0258\", reason = \"see RUSTSEC-1] 2027-02-06\" } ]",
            "[ { id = \"RUSTSEC-2026-0258\", reason = \"opens a [ 2027-02-06\" } ]",
        ] {
            let deny = format!("[advisories]\nignore = {value}\n");
            assert!(
                check_ignore_entries_are_logged(&deny, &grep()).is_err(),
                "`ignore = {value}` must not read as an empty, compliant list"
            );
        }
    }

    /// The readers this guard is built on, exercised directly, so that a wrong
    /// answer cannot be masked by another branch reaching the same verdict.
    #[test]
    fn the_ignore_reader_reads_the_shapes_it_claims_to() {
        // One entry, one line, and the line kept verbatim for the grep check.
        let entries = ignore_entries(&advisories_with(SHIPPED_ENTRY)).expect("readable");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].lines.len(), 1);
        assert!(entries[0].lines[0].contains("RUSTSEC-2026-0258"));
        assert!(entries[0].text.starts_with('{') && entries[0].text.ends_with('}'));

        // A comma inside a quoted reason does not split the entry.
        let entries = ignore_entries(&advisories_with(
            "  { id = \"RUSTSEC-1\", reason = \"a, b, c 2027-02-06\" },\n",
        ))
        .expect("readable");
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(
            string_value(&entries[0].text, "reason").as_deref(),
            Some("a, b, c 2027-02-06")
        );

        // Nor does a brace, in either string form.
        let entries = ignore_entries(&advisories_with(
            "  { id = \"RUSTSEC-1\", reason = 'closes with } 2027-02-06' },\n",
        ))
        .expect("readable");
        assert_eq!(entries.len(), 1, "{entries:?}");

        // A balanced bracket pair inside a reason reads fine: the first pass
        // counts brackets without knowing about strings, and a pair nets to
        // zero. Only an unbalanced one is unreadable — see
        // `an_unreadable_ignore_list_fails`.
        let entries = ignore_entries(&advisories_with(
            "  { id = \"RUSTSEC-1\", reason = \"see [1]. Reconsider by 2027-02-06.\" },\n",
        ))
        .expect("readable");
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert!(check_ignore_entries_are_logged(
            &advisories_with(
                "  { id = \"RUSTSEC-2026-0258\", \
                 reason = \"see [1]. Reconsider by 2027-02-06.\" },\n"
            ),
            &grep()
        )
        .is_ok());

        // Two entries on one line split; a multi-line entry keeps its lines.
        let entries = ignore_entries(&advisories_with(
            "  { id = \"RUSTSEC-1\" }, { id = \"RUSTSEC-2\" },\n",
        ))
        .expect("readable");
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| entry.lines.len() == 1));
        let entries = ignore_entries(&advisories_with(
            "  { id = \"RUSTSEC-1\",\n    reason = \"x\" },\n",
        ))
        .expect("readable");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].lines.len(), 2);

        // Absent and empty are both "no accepted advisories"; unreadable is not.
        assert_eq!(
            ignore_entries("[advisories]\nyanked = \"deny\"\n"),
            Some(Vec::new())
        );
        assert_eq!(ignore_entries("ignore = []\n"), Some(Vec::new()));
        assert_eq!(ignore_entries("ignore = \"RUSTSEC-1\"\n"), None);
        assert_eq!(
            ignore_entries("ignore = [\n  { id = \"RUSTSEC-1\" },\n"),
            None
        );

        // `string_value` keeps the boundary rule `count_key` uses.
        assert_eq!(
            string_value("{ reason = \"why\" }", "reason").as_deref(),
            Some("why")
        );
        assert_eq!(string_value("{ my-reason = \"why\" }", "reason"), None);
        assert_eq!(string_value("{ id = \"RUSTSEC-1\" }", "reason"), None);
        assert_eq!(string_value("{ reason = 0 }", "reason"), None);
    }

    /// The pattern reader, and the half of this guard that points at `ci.yml`.
    /// The real step is the measurement: whatever it greps for is what the
    /// shipped entry shape has to match, so the two cannot drift apart silently.
    #[test]
    fn the_log_steps_pattern_is_read_out_of_the_workflow() {
        assert_eq!(grep(), "id = \"RUSTSEC");

        let step = format!("      - name: {LOG_STEP}\n");
        assert_eq!(
            log_step_pattern(&format!(
                "{step}        run: grep -n 'id = \"RUSTSEC' deny.toml\n"
            ))
            .expect("the shipped step body is readable"),
            "id = \"RUSTSEC"
        );

        // No step means no promise to keep, which is a failure and not a pass.
        assert!(
            log_step_pattern("jobs:\n  deny:\n    steps:\n      - run: cargo deny check\n")
                .is_err()
        );
        // A step whose body is no longer that grep is not assumed equivalent.
        assert!(log_step_pattern(&format!("{step}        run: cat deny.toml\n")).is_err());
        assert!(
            log_step_pattern(&format!("{step}        run: grep -n RUSTSEC deny.toml\n")).is_err()
        );
        assert!(log_step_pattern(&format!("{step}        run: grep -n '' deny.toml\n")).is_err());
        assert!(log_step_pattern(&step).is_err());
        // And the `run:` has to belong to this step, not to the next one.
        assert!(log_step_pattern(&format!(
            "{step}      - name: Something else\n        run: grep -n 'x' deny.toml\n"
        ))
        .is_err());
    }

    /// A workflow with `permissions` spliced in where `ci.yml` carries it:
    /// top level, between `env:` and `jobs:`.
    ///
    /// The argument is literal source text rather than a parsed value, so an
    /// arm can hand this a shape no author would choose on purpose — an empty
    /// key, a capitalised value, a key that is only a comment. Those are the
    /// arms that decide whether the guard fails closed.
    fn workflow_with(permissions: &str) -> String {
        format!(
            "name: CI\n\non:\n  push:\n    branches: [main]\n  pull_request:\n\n\
             env:\n  CARGO_TERM_COLOR: always\n\n\
             {permissions}jobs:\n  deny:\n    name: supply chain\n    \
             runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      \
             - run: cargo deny check\n"
        )
    }

    /// D1. The shipped state before this ticket: no `permissions:` anywhere, so
    /// the token's scope is the repository's web setting and the tree is silent
    /// about it. This is the arm that is red on the tree as it stands, which is
    /// why the two lines in `ci.yml` and this guard had to land together.
    #[test]
    fn a_workflow_that_declares_no_permissions_fails() {
        let error = check_workflow_token_is_read_only(&workflow_with(""))
            .expect_err("a workflow with no permissions block must fail");
        assert!(error.contains("default_workflow_permissions"), "{error}");
    }

    /// D2 / D3. The direction control, and the pair that matters most.
    ///
    /// A guard that checked for the *presence* of a `permissions:` key would
    /// pass both of these, and would have been written, committed and reviewed
    /// looking exactly like this one. Only the fact that `write` fails where
    /// `read` passes says the guard reads the value.
    #[test]
    fn a_write_scope_fails_where_the_same_scope_at_read_passes() {
        let error = check_workflow_token_is_read_only(&workflow_with(
            "permissions:\n  contents: write\n\n",
        ))
        .expect_err("contents: write must fail");
        assert!(error.contains("contents: write"), "{error}");

        assert!(
            check_workflow_token_is_read_only(&workflow_with("permissions:\n  contents: read\n\n"))
                .is_ok(),
            "contents: read is the declaration this ticket asks for and must pass, or the \
             guard is one that can never be satisfied"
        );
    }

    /// D4 / D5. The two blanket forms. `write-all` is the widest token GitHub
    /// issues; `read-all` is a legitimate way to say the same thing the block
    /// form says, so refusing it would be refusing a correct declaration.
    #[test]
    fn write_all_fails_where_read_all_passes() {
        let error = check_workflow_token_is_read_only(&workflow_with("permissions: write-all\n\n"))
            .expect_err("write-all must fail");
        assert!(error.contains("every scope at write"), "{error}");

        assert!(
            check_workflow_token_is_read_only(&workflow_with("permissions: read-all\n\n")).is_ok(),
            "read-all grants nothing but reading and must pass"
        );
    }

    /// D6. The one a careless reader ships: the write is in the *second* scope.
    /// Stop at `contents` and the block reads safe. Same shape as the optional
    /// `checksum` sub-key in SAK-237 — what unpins the gate is the token nobody
    /// looked at.
    #[test]
    fn a_write_scope_hiding_behind_a_read_one_fails() {
        let error = check_workflow_token_is_read_only(&workflow_with(
            "permissions:\n  contents: read\n  packages: write\n\n",
        ))
        .expect_err("a second scope at write must fail");
        assert!(error.contains("packages: write"), "{error}");
    }

    /// D7 / D8. Two things that look alike in a diff and are not the same at
    /// all. `permissions: {}` is every scope at `none` — stricter than this
    /// guard demands. A key with nothing under it is a YAML null that GitHub
    /// refuses to load, so the gated run simply never happens; passing it would
    /// be passing a workflow that does not run.
    #[test]
    fn an_empty_mapping_passes_where_an_empty_key_fails() {
        assert!(
            check_workflow_token_is_read_only(&workflow_with("permissions: {}\n\n")).is_ok(),
            "`permissions: {{}}` is no scopes at all, which is stricter than contents: read"
        );

        let error = check_workflow_token_is_read_only(&workflow_with("permissions:\n\n"))
            .expect_err("an empty permissions key must fail");
        assert!(error.contains("no scopes under it"), "{error}");
    }

    /// D9. Fails closed on a value it cannot read, rather than guessing. Both
    /// spellings here are ones a careful author might type; GitHub accepts
    /// neither, and a guard that shrugged at them would be reporting on a token
    /// nobody has measured. Same call SAK-235 made for `[graph] targets`.
    #[test]
    fn a_value_this_reader_cannot_parse_fails() {
        for value in ["Read", "READ", "\"read\"", "true"] {
            let Err(error) = check_workflow_token_is_read_only(&workflow_with(&format!(
                "permissions:\n  contents: {value}\n\n"
            ))) else {
                panic!("`contents: {value}` is not a value GitHub accepts and must fail");
            };
            assert!(error.contains("not one of"), "{error}");
        }

        // The control: the one spelling that is the word GitHub accepts.
        assert!(
            check_workflow_token_is_read_only(&workflow_with("permissions:\n  contents: read\n\n"))
                .is_ok(),
            "the bare lower-case value must pass, or the arms above are failing on \
             something other than the spelling"
        );
    }

    /// D10. Comments inside the block are not entries. `ci.yml` explains every
    /// setting it carries, and this block is no exception, so a reader that
    /// counted comment lines as scopes would fail the file it was written for.
    #[test]
    fn comments_inside_the_block_do_not_change_the_reading() {
        assert!(
            check_workflow_token_is_read_only(&workflow_with(
                "permissions:\n  # Least privilege for every job.\n  contents: read\n\n"
            ))
            .is_ok(),
            "a comment inside the block must not be read as a scope"
        );

        // Including a trailing one on the live line.
        assert!(check_workflow_token_is_read_only(&workflow_with(
            "permissions:\n  contents: read  # nothing here writes\n\n"
        ))
        .is_ok());
    }

    /// A commented-out declaration is not a declaration. `ci.yml` documents its
    /// own settings in prose, so the word `permissions:` appears in the file
    /// whether or not the key does — and a substring match would find the
    /// comment, read the value beside it, and call an unset workflow safe.
    #[test]
    fn a_commented_out_permissions_block_is_not_the_setting() {
        let error = check_workflow_token_is_read_only(&workflow_with(
            "# permissions:\n#   contents: read\n\n",
        ))
        .expect_err("a commented-out block must not count as a declaration");
        assert!(error.contains("default_workflow_permissions"), "{error}");

        // And the mirror: a commented-out `write` beside a live `read` is prose
        // about history, not a setting, and must not fail the file.
        assert!(
            check_workflow_token_is_read_only(&workflow_with(
                "permissions:\n  # was: contents: write, until SAK-240\n  contents: read\n\n"
            ))
            .is_ok(),
            "a commented-out write must not fail a workflow whose live value is read"
        );
    }

    /// The scope the ticket left out, and the reason this guard reads every
    /// block rather than only the first.
    ///
    /// A job's `permissions:` *replaces* the workflow's for that job — it is
    /// not capped by it; only the repository-wide maximum is a cap — so this
    /// workflow would hand the `deny` job a write token with `contents: read`
    /// still sitting reassuringly at the top of the file. Policy at the top,
    /// kill switch further down: the shape SAK-231 found in
    /// `disable-yank-checking`.
    #[test]
    fn a_job_level_block_that_widens_the_token_fails() {
        let widened = "name: CI\n\non:\n  pull_request:\n\npermissions:\n  contents: read\n\n\
                       jobs:\n  deny:\n    runs-on: ubuntu-latest\n    \
                       permissions: write-all\n    steps:\n      - run: cargo deny check\n";
        let error = check_workflow_token_is_read_only(widened)
            .expect_err("a job-level write-all must fail even under a read-only top level");
        assert!(error.contains("job-level"), "{error}");

        // A job-level block that only narrows is fine, and must stay fine:
        // giving the `deny` job its own `contents: read` is good practice, and
        // a guard that failed it would be teaching people to delete it.
        let narrowed = "name: CI\n\non:\n  pull_request:\n\npermissions:\n  contents: read\n\n\
                        jobs:\n  deny:\n    runs-on: ubuntu-latest\n    \
                        permissions:\n      contents: read\n    steps:\n      \
                        - run: cargo deny check\n";
        assert!(check_workflow_token_is_read_only(narrowed).is_ok());
    }

    /// The reader itself, on the real file and on the shapes above.
    ///
    /// Here because every arm in this section is only as good as this parse: if
    /// `permissions_keys` found nothing in any input, most of the failing arms
    /// would still fail — for the wrong reason — and the passing ones would be
    /// the only thing left telling the difference.
    ///
    /// Note how little is asked of the real file: only that the reader can see
    /// its top-level key. That is deliberate, and re-tightening it would be a
    /// regression (SAK-241). `check_workflow_token_is_read_only` requires that
    /// nothing grants more than `read`, which means it *passes* declarations
    /// stricter than the one shipped today — `permissions: read-all`,
    /// `permissions: {}`, an extra `packages: none`, and a narrowing job-level
    /// block on `deny` are all fine by the policy. An assertion here that fixed
    /// the live block to its exact current text failed every one of those, so
    /// tightening a workflow turned CI red with a message that said nothing
    /// about tokens (`left: "read-all" right: ""`), and the cheapest way back to
    /// green was to undo the tightening — which is the thing
    /// `a_job_level_block_that_widens_the_token_fails` warns about in so many
    /// words where it checks the narrowing case: "a guard that failed it would
    /// be teaching people to delete it."
    ///
    /// Nothing is lost by asking less, because the grammar this reader
    /// implements — indentation, comments, prefix matching, where a block ends —
    /// is pinned exactly by the synthetic inputs below, where the expected text
    /// is fixed because the input is. Only the live-file arm has to tolerate a
    /// file that is allowed to change.
    #[test]
    fn the_permissions_reader_finds_exactly_the_live_keys() {
        let ci = repo_file(".github/workflows/ci.yml");
        let keys = permissions_keys(&ci);
        assert!(
            keys.iter().any(|key| key.indent == 0),
            "the reader must find ci.yml's top-level `permissions:` key, found {keys:?}"
        );

        // Depth is what separates a workflow-wide key from a job's own.
        let keys = permissions_keys(
            "permissions:\n  contents: read\njobs:\n  deny:\n    permissions: write-all\n",
        );
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].indent, 0);
        assert_eq!(keys[1].indent, 4);
        assert_eq!(keys[1].inline, "write-all");

        // A comment is not a key, and a longer key that merely starts with the
        // word is not this key either.
        assert_eq!(permissions_keys("# permissions: write-all\n"), Vec::new());
        assert_eq!(
            permissions_keys("permissions-note: write-all\n"),
            Vec::new()
        );

        // The block ends at the next key at the same depth, and a blank line in
        // the middle of it does not end it early.
        let keys = permissions_keys(
            "permissions:\n  contents: read\n\n  packages: none\njobs:\n  deny:\n    \
             runs-on: ubuntu-latest\n",
        );
        assert_eq!(
            keys[0].entries,
            vec!["contents: read".to_string(), "packages: none".to_string()]
        );
    }

    /// The command `ci.yml` ships, written the way it is written there: one
    /// `cargo-deny` spread over three lines by `\`-continuations.
    const SHIPPED_DENY_COMMAND: &str = "cargo-deny --locked check \\\n  \
                                        -D advisory-not-detected \\\n  \
                                        advisories licenses bans sources";

    /// A `deny` job whose gate step runs `command`, in the shape `ci.yml` uses:
    /// the last step of the job, a `run: |` block, with another named step
    /// ahead of it so that an arm can tell "found the gate" from "found a step".
    ///
    /// `command` is literal shell source, so an arm can hand this a body no
    /// author would choose on purpose. Those are the arms that decide whether
    /// the reader fails closed.
    fn deny_job_with(command: &str) -> String {
        let body: String = command
            .lines()
            .map(|line| format!("          {line}\n"))
            .collect();
        format!(
            "name: CI\n\non:\n  pull_request:\n\npermissions:\n  contents: read\n\n\
             jobs:\n  deny:\n    runs-on: ubuntu-latest\n    steps:\n      \
             - uses: actions/checkout@v4\n      - name: {LOG_STEP}\n        \
             run: grep -n 'id = \"RUSTSEC' deny.toml\n      - name: {DENY_STEP}\n        \
             run: |\n{body}"
        )
    }

    /// The reader itself, before anything is asserted about what it read.
    ///
    /// Here for the reason `the_permissions_reader_finds_exactly_the_live_keys`
    /// gives: every failing arm below is only as good as this parse. A reader
    /// that found no check names in any input would still fail the dropped-name
    /// arms — for entirely the wrong reason — and the passing arms would be the
    /// only thing left able to tell the difference.
    ///
    /// Note how little is asked of the real file, deliberately (SAK-241): only
    /// that the reader can see its gate command at all. Nothing here says which
    /// checks that command names — `check_every_deny_check_runs` is the one
    /// place that decides, and it has to stay the one place. Measured while
    /// writing this: an arm that also looped over `DENY_CHECKS` against the live
    /// file made every dropped-name arm fail *twice*, the second time with
    /// "the live command must name `licenses`" — a worse message than the real
    /// one — and, worse, it failed the all-names-dropped tree, which the policy
    /// passes on purpose because omitting the list runs every check. That is
    /// the guard demanding redundancy, the thing this ticket set out not to
    /// build. The grammar — block scalars, `\`-continuations, where a step ends
    /// — is pinned exactly by the synthetic inputs below instead, where the
    /// expected text is fixed because the input is.
    #[test]
    fn the_deny_step_reader_finds_the_live_gate_command() {
        let live = deny_step_checks(&repo_file(".github/workflows/ci.yml"))
            .expect("the shipped deny step must be readable");
        assert!(
            live.command.contains("cargo-deny") && live.command.contains("check"),
            "{live:?}"
        );

        // The synthetic shape the arms below use, pinned exactly: three source
        // lines joined into one logical command, nothing before `check` kept,
        // and the flag's value kept as a word like any other — the reader does
        // not try to tell a flag's value from a positional, and does not need
        // to, because `advisory-not-detected` is not one of the four names.
        let step = deny_step_checks(&deny_job_with(SHIPPED_DENY_COMMAND)).expect("readable");
        assert_eq!(
            step.command,
            "cargo-deny --locked check -D advisory-not-detected advisories licenses bans sources"
        );
        assert_eq!(
            step.arguments,
            vec![
                "-D".to_string(),
                "advisory-not-detected".to_string(),
                "advisories".to_string(),
                "licenses".to_string(),
                "bans".to_string(),
                "sources".to_string(),
            ]
        );

        // A one-line `run:` is read too, and the gate is the step named
        // DENY_STEP rather than whichever step came first.
        let step = deny_step_checks(
            "jobs:\n  deny:\n    steps:\n      - name: Accepted advisory exceptions\n        \
             run: grep -n 'id = \"RUSTSEC' deny.toml\n      - name: cargo deny check\n        \
             run: cargo-deny check advisories licenses bans sources\n",
        )
        .expect("a one-line run: body is readable");
        assert_eq!(step.arguments.len(), 4);
    }

    /// The hole this ticket is about: one name gone, and the whole of that
    /// check's policy with it. Four arms, because a guard that happened to read
    /// only the first or the last name would pass three of them.
    #[test]
    fn dropping_one_check_name_fails_and_names_the_check() {
        for dropped in DENY_CHECKS {
            let kept: Vec<&str> = DENY_CHECKS
                .iter()
                .copied()
                .filter(|check| check != dropped)
                .collect();
            let command = format!(
                "cargo-deny --locked check \\\n  -D advisory-not-detected \\\n  {}",
                kept.join(" ")
            );
            let step = deny_step_checks(&deny_job_with(&command)).expect("readable");
            let Err(error) = check_every_deny_check_runs(&step) else {
                panic!("a command that no longer names `{dropped}` must fail");
            };
            assert!(
                error.contains(dropped),
                "the failure has to name the dropped check, got: {error}"
            );
        }

        // The control, and the arm that says the guard is satisfiable at all:
        // the command as shipped passes.
        let step = deny_step_checks(&deny_job_with(SHIPPED_DENY_COMMAND)).expect("readable");
        assert!(
            check_every_deny_check_runs(&step).is_ok(),
            "the shipped command must pass, or the arms above are failing on something \
             other than the missing name"
        );
    }

    /// The arm that keeps this guard from demanding redundancy.
    ///
    /// Given no check names cargo-deny runs **every** check it has — measured,
    /// and every control in the ticket's first table agrees with the full list.
    /// So omitting the names is the *stronger* gate, and failing it would be
    /// failing a tree that is safer than the one shipped.
    #[test]
    fn naming_no_checks_at_all_passes_because_that_runs_them_all() {
        let step = deny_step_checks(&deny_job_with(
            "cargo-deny --locked check \\\n  -D advisory-not-detected",
        ))
        .expect("readable");
        assert!(
            step.arguments
                .iter()
                .all(|argument| !DENY_CHECKS.contains(&argument.as_str())),
            "{step:?}"
        );
        assert!(
            check_every_deny_check_runs(&step).is_ok(),
            "a command with no check names runs every check cargo-deny has, which is \
             stricter than the shipped list and must not be reported as a hole"
        );
    }

    /// The second lever in the same step. `|| true` turns exit 4 into exit 0.
    #[test]
    fn sending_the_gates_exit_to_or_fails() {
        let error = check_every_deny_check_runs(
            &deny_step_checks(&deny_job_with(&format!("{SHIPPED_DENY_COMMAND} || true")))
                .expect("readable"),
        )
        .expect_err("`|| true` after the gate must fail");
        assert!(error.contains("||"), "{error}");

        // Including the form that does not look like a bypass at all.
        let error = check_every_deny_check_runs(
            &deny_step_checks(&deny_job_with(&format!(
                "{SHIPPED_DENY_COMMAND} || echo 'supply chain check skipped'"
            )))
            .expect("readable"),
        )
        .expect_err("any `||` after the gate must fail");
        assert!(error.contains("discards the verdict"), "{error}");

        // The *advice* in that message is why SAK-253 had to reopen this guard:
        // the old one read "as does a pipe (`-o pipefail`)", crediting a `set`
        // line this step does not have. No test read the prose, so nothing
        // reported it. These two do — one for the wrong recommendation, one for
        // the sentence finishing at all.
        assert!(
            !error.contains("as does a pipe"),
            "the `||` message must not offer a bare pipe as a safe alternative: {error}"
        );
        assert!(
            error.contains("set -o pipefail"),
            "the `||` message must name the one spelling that does keep the gate's exit \
             through a pipe: {error}"
        );
    }

    /// The direction control for the arm above: the trailing constructs that
    /// really do keep the gate's exit.
    ///
    /// With `deny.toml`'s licence allowlist broken the step exits 4 under each
    /// of these on the runner's own shell (`bash -e`, run 42362531653), so an
    /// assertion rejecting them would be red on a command that works -- the
    /// failure mode this whole ticket is trying not to introduce.
    ///
    /// A pipe used to be on this list. It is not any more, and the arm below
    /// is where it went: that entry was measured under a replay harness with
    /// `-o pipefail`, which the runner does not give this workflow (SAK-253).
    #[test]
    fn the_trailing_constructs_that_keep_the_exit_status_pass() {
        for trailing in [
            "; true",
            "&& echo done",
            "&& echo 'supply chain ok'",
            "> /dev/null",
        ] {
            let step = deny_step_checks(&deny_job_with(&format!(
                "{SHIPPED_DENY_COMMAND} {trailing}"
            )))
            .expect("readable");
            assert!(
                check_every_deny_check_runs(&step).is_ok(),
                "`{trailing}` leaves the gate's exit status intact (measured) and must not \
                 be reported as a bypass"
            );
        }

        // And the word boundary the arms rely on: an operator ends a word, so
        // a check name cannot hide against one.
        let step = deny_step_checks(&deny_job_with(
            "cargo-deny --locked check advisories licenses bans sources;echo done",
        ))
        .expect("readable");
        assert!(check_every_deny_check_runs(&step).is_ok(), "{step:?}");
    }

    /// A pipe discards the gate's verdict on the shell the runner actually
    /// gives this workflow, and the only thing that brings it back is a `set`
    /// line in the same body.
    ///
    /// Three arms, because two of them are what make the first one a finding
    /// rather than a preference. The pipe is rejected; the same pipe under
    /// `set -o pipefail` is accepted, so the assertion is satisfiable without
    /// deleting it; and a redirect -- which is not a pipe and never discarded
    /// anything -- stays accepted, so this is about the operator and not about
    /// punctuation in general.
    #[test]
    fn piping_the_gates_output_without_pipefail_is_caught() {
        for trailing in [
            "| tee /dev/null",
            "| cat",
            "|tee out.log",
            "| tee out.log | cat",
        ] {
            let step = deny_step_checks(&deny_job_with(&format!(
                "{SHIPPED_DENY_COMMAND} {trailing}"
            )))
            .expect("readable");
            let error = check_every_deny_check_runs(&step)
                .expect_err("a pipe with no pipefail discards the verdict");
            assert!(error.contains("does not turn `pipefail` on"), "{error}");
        }

        for body in ["set -o pipefail", "set -eo pipefail", "set -euo pipefail"] {
            let step = deny_step_checks(&deny_job_with(&format!(
                "{body}\n{SHIPPED_DENY_COMMAND} | tee /dev/null"
            )))
            .expect("readable");
            assert!(
                step.pipefail,
                "`{body}` should be read as turning pipefail on"
            );
            assert!(
                check_every_deny_check_runs(&step).is_ok(),
                "`{body}` keeps the gate's exit through a pipe (measured) and must pass"
            );
        }
    }

    /// Every spelling of `-A`/`-W` that cargo-deny 0.20.2 accepts, and each one
    /// measured to silence the licence policy at exit 0 before being listed
    /// here. The guard reads all four shapes because the parser does.
    #[test]
    fn lowering_a_diagnostics_severity_on_the_command_line_fails() {
        for flag in [
            "-A rejected",
            "-Arejected",
            "--allow rejected",
            "--allow=rejected",
            "-W rejected",
            "-Wrejected",
            "--warn rejected",
            "--warn=rejected",
        ] {
            let step = deny_step_checks(&deny_job_with(&format!(
                "cargo-deny --locked check {flag} \\\n  -D advisory-not-detected \\\n  \
                 advisories licenses bans sources"
            )))
            .expect("readable");
            let Err(error) = check_no_policy_is_silenced(&step) else {
                panic!("`{flag}` stops the gate failing on `rejected` and must be reported");
            };
            assert!(error.contains("lowers the severity"), "`{flag}`: {error}");
        }
    }

    /// The direction control, and the arm that keeps this guard off a stricter
    /// tree.
    ///
    /// `-D`/`--deny` is the same flag family pointing the other way — the
    /// shipping command is built on it — so every spelling of it has to pass.
    /// Without this arm a guard that merely looked for a `-` followed by a
    /// capital letter would read as working while reddening the gate for
    /// getting stricter, which is the mistake SAK-241 undid in
    /// `the_permissions_reader_finds_exactly_the_live_keys`.
    #[test]
    fn raising_a_diagnostics_severity_or_leaving_it_alone_passes() {
        for command in [
            // The shipped command, which already carries a `-D`.
            SHIPPED_DENY_COMMAND,
            // Stricter still, in each spelling of the strict flag.
            "cargo-deny --locked check -D advisory-not-detected -D rejected advisories \
             licenses bans sources",
            "cargo-deny --locked check --deny rejected advisories licenses bans sources",
            "cargo-deny --locked check --deny=rejected advisories licenses bans sources",
            "cargo-deny --locked check -Drejected advisories licenses bans sources",
            // No check names at all: stricter, and passes assertion 1 too.
            "cargo-deny --locked check -D advisory-not-detected",
            // Flags that are not about severity at all, including the two
            // lock-related ones this guard deliberately says nothing about.
            "cargo-deny --frozen check -D advisory-not-detected advisories licenses bans \
             sources",
            "cargo-deny --locked --offline check advisories licenses bans sources",
            "cargo-deny --locked check --all-features advisories licenses bans sources",
            "cargo-deny check advisories licenses bans sources",
            // The strict-prefix arms. `silenced_diagnostics` matches the long
            // form as exactly `--allow`/`--warn` or an `=`-suffixed prefix of
            // one, and nothing else. These pin that shape, so a later rewrite
            // to a bare `starts_with("--allow")` reddens a tree in which no
            // diagnostic was lowered at all. Neither flag exists in cargo-deny
            // 0.20.2 — the shape of the match is the whole point, and it is the
            // only thing standing between this guard and a future
            // `--allow-<something stricter>`.
            "cargo-deny --locked check --allow-registry-only advisories licenses bans sources",
            "cargo-deny --locked check --warn-as-error advisories licenses bans sources",
        ] {
            let step = deny_step_checks(&deny_job_with(command)).expect("readable");
            assert!(
                check_no_policy_is_silenced(&step).is_ok(),
                "`{command}` does not lower any diagnostic's severity and must not be \
                 reported: {:?}",
                silenced_diagnostics(&step)
            );
        }
    }

    /// Every spelling of the five redirecting options, each measured to take a
    /// red tree to exit 0 before being listed.
    ///
    /// `-t` has two accepted shapes and each long form has two, all measured on
    /// cargo-deny 0.20.2. A guard that caught one spelling would leave the
    /// others open, which is the counting mistake SAK-247 had to avoid for
    /// `-A`/`-W`.
    #[test]
    fn redirecting_what_the_gate_checks_fails() {
        for (flag, expected) in [
            ("--config other.toml", "--config"),
            ("--config=other.toml", "--config"),
            ("--metadata-path graph.json", "--metadata-path"),
            ("--metadata-path=graph.json", "--metadata-path"),
            ("--target x86_64-unknown-linux-gnu", "--target"),
            ("--target=x86_64-unknown-linux-gnu", "--target"),
            ("-t x86_64-unknown-linux-gnu", "--target"),
            ("-tx86_64-unknown-linux-gnu", "--target"),
            ("--exclude smartstring", "--exclude"),
            ("--exclude=smartstring", "--exclude"),
            ("--exclude-dev", "--exclude-dev"),
        ] {
            let step = deny_step_checks(&deny_job_with(&format!(
                "cargo-deny --locked {flag} check \\\n  -D advisory-not-detected \\\n  \
                 advisories licenses bans sources"
            )))
            .expect("readable");
            let Err(error) = check_nothing_redirects_what_is_checked(&step) else {
                panic!("`{flag}` changes what the gate checks and must be reported");
            };
            assert!(error.contains(expected), "`{flag}`: {error}");
        }
    }

    /// The arms that keep this guard off a tree that did not narrow anything.
    ///
    /// Two groups, and both are load-bearing. The first is the options measured
    /// on the red tree to leave it red — if any of these were reported the
    /// guard would be reddening a gate for doing nothing wrong, and in
    /// `--exclude-unpublished`'s case for an option that is a literal prefix
    /// match of one that *is* a hole. The second is the strict-prefix and
    /// short-flag arms: `-c` is `--color`, not `--config`, and
    /// `--targets-are-fine` is not `--target`. Both would be caught by a bare
    /// `starts_with`, and neither narrows anything.
    #[test]
    fn leaving_the_graph_and_the_policy_file_alone_passes() {
        for command in [
            // The shipped command.
            SHIPPED_DENY_COMMAND,
            // Measured on the red tree as still exit 4 or 5 — none subtract.
            "cargo-deny --locked --all-features check advisories licenses bans sources",
            "cargo-deny --locked --no-default-features check advisories licenses bans sources",
            "cargo-deny --locked --workspace check advisories licenses bans sources",
            "cargo-deny --locked --exclude-unpublished check advisories licenses bans sources",
            "cargo-deny --frozen check advisories licenses bans sources",
            "cargo-deny --locked --offline check advisories licenses bans sources",
            // `-c` is `--color`. Measured: `-c never` on the red tree is exit 4.
            "cargo-deny --locked -c never check advisories licenses bans sources",
            "cargo-deny --locked --color=never check advisories licenses bans sources",
            // Not short for anything here, and not `--target`.
            "cargo-deny -L off --locked check advisories licenses bans sources",
            // Strict-prefix arms. None of these exist in cargo-deny 0.20.2; the
            // shape of the match is what they pin, so a later rewrite to a bare
            // `starts_with` reddens a tree that narrowed nothing.
            "cargo-deny --locked --config-check-only check advisories licenses bans sources",
            "cargo-deny --locked --targets-are-fine check advisories licenses bans sources",
            "cargo-deny --locked --excludes-nothing check advisories licenses bans sources",
            // `--manifest-path` is measured and deliberately not guarded: it
            // moves where `deny.toml` is looked up too, so cargo-deny falls
            // back to the default config, which allows no licence at all. Exit
            // 4 on an otherwise-clean tree. Loud, so not this guard's business.
            "cargo-deny --locked --manifest-path ./Cargo.toml check advisories licenses bans \
             sources",
        ] {
            let step = deny_step_checks(&deny_job_with(command)).expect("readable");
            assert!(
                check_nothing_redirects_what_is_checked(&step).is_ok(),
                "`{command}` narrows nothing and must not be reported: {:?}",
                redirecting_options(&step)
            );
        }
    }

    /// Fails closed, rather than passing, on anything it cannot read.
    ///
    /// Every arm here is a tree with no working gate in it. A reader that
    /// returned "no names found" for these would hand them straight to
    /// assertion 1, which passes an empty set by design — so the fail-closed
    /// behaviour has to live in the reader, and this is the arm that says it
    /// does.
    #[test]
    fn a_step_this_reader_cannot_understand_fails_closed() {
        // The gate deleted outright. Nothing a test can do but refuse to pass.
        assert!(
            deny_step_checks(&deny_job_with(SHIPPED_DENY_COMMAND).replace(
                &format!("- name: {DENY_STEP}"),
                "- name: something else entirely"
            ))
            .is_err()
        );

        // Two steps by that name: which list is in force is unreadable.
        let twice = format!(
            "{}      - name: {DENY_STEP}\n        run: |\n          {SHIPPED_DENY_COMMAND}\n",
            deny_job_with(SHIPPED_DENY_COMMAND)
        );
        assert!(deny_step_checks(&twice).is_err());

        // A body that is not a cargo-deny invocation, an empty one, two of
        // them, and one with no `check` subcommand.
        for body in [
            "echo 'supply chain: trust me'",
            "cargo-deny --version",
            "cargo-deny check advisories\ncargo-deny check licenses",
        ] {
            assert!(
                deny_step_checks(&deny_job_with(body)).is_err(),
                "an unfamiliar body must fail closed: {body}"
            );
        }
        assert!(deny_step_checks(
            "jobs:\n  deny:\n    steps:\n      - name: cargo deny check\n        run: |\n"
        )
        .is_err());
        assert!(deny_step_checks(
            "jobs:\n  deny:\n    steps:\n      - name: cargo deny check\n        \
             runs-on: ubuntu-latest\n"
        )
        .is_err());

        // A commented-out gate is not a gate. `ci.yml` carries the words
        // `cargo deny check` in prose above this very step, so a reader that
        // matched anywhere in a line would find the comment and report on a
        // command that does not run.
        assert!(
            deny_step_checks(&deny_job_with(SHIPPED_DENY_COMMAND).replace(
                &format!("- name: {DENY_STEP}"),
                &format!("# - name: {DENY_STEP}")
            ))
            .is_err()
        );
    }

    /// The three lines in the real `ci.yml` the SAK-244 arms anchor to, each
    /// verified to be exactly one live line before any of them is edited.
    const DENY_JOB_LINE: &str = "  deny:";
    const GATE_STEP_LINE: &str = "      - name: cargo deny check";
    const INSTALL_STEP_LINE: &str = "      - name: Install cargo-deny";

    /// The real `ci.yml` with the single live line `anchor` replaced.
    ///
    /// Against the real file rather than a synthetic workflow, because the
    /// question these arms ask is about the file that actually runs — a
    /// synthetic `deny` job could differ from it in the one structural way that
    /// mattered and nothing would say so.
    ///
    /// Whole-line, exact, and it asserts there is exactly **one** live match
    /// before editing. `ci.yml` is a file that documents its own settings in
    /// prose: a substring edit, or one that tolerated two matches, could land
    /// on a comment and produce a tree where the mutation is not in force,
    /// which reads as "the guard did not catch it".
    fn with_sole_line(text: &str, anchor: &str, replacement: &str) -> String {
        let live = text.lines().filter(|line| *line == anchor).count();
        assert_eq!(
            live, 1,
            "`{anchor}` is {live} live lines, not one. These arms edit by whole-line \
             anchor, so an anchor that has moved means they are no longer mutating the \
             gate they claim to be mutating."
        );
        text.lines()
            .map(|line| {
                if line == anchor {
                    format!("{replacement}\n")
                } else {
                    format!("{line}\n")
                }
            })
            .collect()
    }

    /// The real `ci.yml` with every `if:` / `continue-on-error:` line removed.
    ///
    /// The base every arm below mutates, and a no-op on the shipped tree, which
    /// carries neither key anywhere (that is a fact about today, which is why no
    /// assertion here states it — see the note on
    /// `the_gate_condition_reader_finds_the_live_gate`).
    ///
    /// Normalising first is what keeps each arm measuring the key *it* added.
    /// With the raw file as the base, putting `if: false` on the real gate step
    /// would redden the green arms too — they would be reading a file with both
    /// their `if: always()` and the mutation on it — and a suite where one hole
    /// fails five tests says less, and says it worse, than one where it fails
    /// the single test whose message is about that hole. The test that reads the
    /// shipped file unnormalised, and so is the one that goes red when a
    /// condition really lands there, is
    /// `nothing_in_ci_yml_switches_the_deny_gate_off`.
    fn ci_yml_base() -> String {
        repo_file(".github/workflows/ci.yml")
            .lines()
            .filter(|line| {
                let body = without_comment(line).trim();
                !GATE_CONDITION_KEYS
                    .iter()
                    .any(|key| body.starts_with(&format!("{key}:")))
            })
            .map(|line| format!("{line}\n"))
            .collect()
    }

    fn ci_yml_with(anchor: &str, replacement: &str) -> String {
        with_sole_line(&ci_yml_base(), anchor, replacement)
    }

    /// The real `ci.yml` with `inserted` added directly under `anchor`, as the
    /// first key of that job or step.
    fn ci_yml_with_key_under(anchor: &str, inserted: &str) -> String {
        ci_yml_with(anchor, &format!("{anchor}\n{inserted}"))
    }

    /// The reader itself, before anything is asserted about what it read.
    ///
    /// Note what is asked of the real file and what is not: only that the gate
    /// is **locatable**. Nothing here says the live file has no conditions on
    /// it, even though today it has none.
    ///
    /// That restraint is the whole lesson of this ticket. The first draft of
    /// this test asserted `assert_eq!(live.keys, Vec::new())`, which is true of
    /// the shipped tree — and went **red** the moment `if: always()` was added
    /// to the gate step. `if: always()` does not stop a step (measured: run
    /// `42318904935`), so that tree is *safer* than the one shipped, and the
    /// guard was failing it. That is precisely the mistake SAK-241 undid in
    /// `the_permissions_reader_finds_exactly_the_live_keys`, reproduced here by
    /// the author who had raised it against #96.
    ///
    /// The grammar is pinned on synthetic input instead, where `assert_eq!` is
    /// honest because the input is fixed.
    #[test]
    fn the_gate_condition_reader_finds_the_live_gate() {
        deny_gate_conditions(&repo_file(".github/workflows/ci.yml")).expect(
            "the shipped deny gate must be locatable, or every arm below is failing on \
             the reader rather than on the condition it put there",
        );

        // A job-level key, read with the scope that names the job.
        let read = deny_gate_conditions(&ci_yml_with_key_under(DENY_JOB_LINE, "    if: always()"))
            .expect("readable");
        assert_eq!(
            read.keys,
            vec![GateCondition {
                scope: format!("the `{DENY_JOB}` job"),
                key: "if".to_string(),
                value: "always()".to_string(),
            }]
        );

        // A step-level key, and the value kept verbatim including the `${{ }}`.
        let read = deny_gate_conditions(&ci_yml_with_key_under(
            GATE_STEP_LINE,
            "        continue-on-error: ${{ false }}",
        ))
        .expect("readable");
        assert_eq!(
            read.keys,
            vec![GateCondition {
                scope: format!("the `{DENY_STEP}` step"),
                key: "continue-on-error".to_string(),
                value: "${{ false }}".to_string(),
            }]
        );

        // Both at once, job before step, with a trailing comment stripped off
        // the value rather than read as part of it.
        let read = deny_gate_conditions(&with_sole_line(
            &ci_yml_with_key_under(GATE_STEP_LINE, "        if: success()  # keeps running"),
            DENY_JOB_LINE,
            &format!("{DENY_JOB_LINE}\n    continue-on-error: false"),
        ))
        .expect("readable");
        assert_eq!(
            read.keys
                .iter()
                .map(|found| (found.key.as_str(), found.value.as_str()))
                .collect::<Vec<_>>(),
            vec![("continue-on-error", "false"), ("if", "success()")]
        );

        // Depth is exact, not "at least": a variable called `if` in the gate
        // step's own `env:` block governs nothing, and neither does an `if:` on
        // a different step of the same job.
        let read = deny_gate_conditions(&ci_yml_with_key_under(
            GATE_STEP_LINE,
            "        env:\n          if: false\n          continue-on-error: true",
        ))
        .expect("readable");
        assert_eq!(read.keys, Vec::new());
        let read = deny_gate_conditions(&ci_yml_with_key_under(
            INSTALL_STEP_LINE,
            "        if: false",
        ))
        .expect("readable");
        assert_eq!(read.keys, Vec::new());
    }

    /// The reader hands back the value with one pair of YAML quotes off.
    ///
    /// Pinned with `assert_eq!` on synthetic input, where that is honest,
    /// rather than on the shipped file, which carries none of these keys.
    /// `conditions_that_still_run_the_gate_pass` and
    /// `a_condition_that_switches_the_gate_off_fails` carry the arms that say
    /// what the *verdict* is either way; this one says what was read.
    #[test]
    fn the_gate_condition_reader_strips_one_pair_of_yaml_quotes() {
        let value_of = |inserted: &str| {
            let read = deny_gate_conditions(&ci_yml_with_key_under(GATE_STEP_LINE, inserted))
                .expect("readable");
            match &read.keys[..] {
                [found] => found.value.clone(),
                other => panic!("expected exactly one condition, read {other:?}"),
            }
        };

        // The reason this function exists: `if: !cancelled()` is not valid YAML,
        // so this is the only spelling of it a workflow can carry.
        assert_eq!(value_of("        if: '!cancelled()'"), "!cancelled()");
        assert_eq!(value_of("        if: \"always()\""), "always()");
        assert_eq!(value_of("        if: '${{ always() }}'"), "${{ always() }}");
        assert_eq!(value_of("        continue-on-error: 'false'"), "false");

        // One pair, not all of them.
        assert_eq!(value_of("        if: \"'false'\""), "'false'");

        // Unquoted and unbalanced values come back untouched. Tidying a
        // mismatched pair would be inventing a value the file does not have.
        assert_eq!(value_of("        if: always()"), "always()");
        assert_eq!(value_of("        if: ${{ false }}"), "${{ false }}");
        assert_eq!(value_of("        if: 'always()\""), "'always()\"");
        assert_eq!(value_of("        if: 'always()"), "'always()");
    }

    /// The hole this ticket is about: the gate is never reached, or its failure
    /// is never counted. Four arms, because the two keys are each valid in two
    /// places and a guard that read only one place would pass half of them.
    #[test]
    fn a_condition_that_switches_the_gate_off_fails() {
        for (anchor, inserted, expected) in [
            (DENY_JOB_LINE, "    if: false", "if: false"),
            (GATE_STEP_LINE, "        if: false", "if: false"),
            (
                DENY_JOB_LINE,
                "    continue-on-error: true",
                "continue-on-error: true",
            ),
            (
                GATE_STEP_LINE,
                "        continue-on-error: true",
                "continue-on-error: true",
            ),
            // The spellings that do not read as a switch at a glance.
            (
                GATE_STEP_LINE,
                "        if: ${{ false }}",
                "if: ${{ false }}",
            ),
            (
                GATE_STEP_LINE,
                "        continue-on-error: ${{ github.event_name == 'pull_request' }}",
                "continue-on-error: ${{ github.event_name == 'pull_request' }}",
            ),
            // The direction control for `unquote_condition` (SAK-250): a pair of
            // quotes makes a switch readable as YAML, not harmless. The expected
            // text is the *unquoted* value, which is also how these arms say the
            // unquoting happened at all rather than the match failing by luck.
            (DENY_JOB_LINE, "    if: 'false'", "if: false"),
            (GATE_STEP_LINE, "        if: 'false'", "if: false"),
            (GATE_STEP_LINE, "        if: \"false\"", "if: false"),
            (
                DENY_JOB_LINE,
                "    continue-on-error: \"true\"",
                "continue-on-error: true",
            ),
            (
                GATE_STEP_LINE,
                "        continue-on-error: \"true\"",
                "continue-on-error: true",
            ),
            (
                GATE_STEP_LINE,
                "        continue-on-error: 'true'",
                "continue-on-error: true",
            ),
            // One pair comes off, not all of them, so a doubly quoted switch is
            // still not a member of `CONDITIONS_THAT_STILL_RUN`.
            (GATE_STEP_LINE, "        if: \"'false'\"", "if: 'false'"),
            // Unbalanced quotes are left alone and fail closed rather than
            // being tidied into something that passes.
            (GATE_STEP_LINE, "        if: 'always()\"", "if: 'always()\""),
        ] {
            let conditions =
                deny_gate_conditions(&ci_yml_with_key_under(anchor, inserted)).expect("readable");
            let Err(error) = check_nothing_switches_the_gate_off(&conditions) else {
                panic!("`{inserted}` under `{anchor}` must fail: it switches the gate off");
            };
            assert!(
                error.contains(expected),
                "the failure has to quote the key it found, got: {error}"
            );
        }

        // The control, and the arm that says the guard is satisfiable at all:
        // the same base with nothing added to it passes. Deliberately the base
        // and not the raw shipped file — that assertion is
        // `nothing_in_ci_yml_switches_the_deny_gate_off`, and duplicating it
        // here would just make a real hole fail two tests.
        let conditions = deny_gate_conditions(&ci_yml_base()).expect("readable");
        assert!(
            check_nothing_switches_the_gate_off(&conditions).is_ok(),
            "the base must pass, or the arms above are failing on something other than \
             the condition they added"
        );
    }

    /// Fails closed on an `if:` it cannot evaluate, and on a gate it cannot
    /// find.
    ///
    /// `github.event_name == 'push'` is the arm that matters. A text reader
    /// cannot decide it, and on this repository it is not a harmless guess
    /// either way: every change reaches `main` through a `pull_request`, so that
    /// condition skips the supply-chain gate on exactly the event that merges
    /// code. A guard that passed what it could not read would pass that.
    #[test]
    fn a_condition_this_reader_cannot_evaluate_fails_closed() {
        for (anchor, inserted) in [
            (DENY_JOB_LINE, "    if: github.event_name == 'push'"),
            (
                GATE_STEP_LINE,
                "        if: ${{ github.event_name == 'push' }}",
            ),
            (
                GATE_STEP_LINE,
                "        if: github.ref == 'refs/heads/main'",
            ),
            (GATE_STEP_LINE, "        if: cancelled()"),
        ] {
            let conditions =
                deny_gate_conditions(&ci_yml_with_key_under(anchor, inserted)).expect("readable");
            assert!(
                check_nothing_switches_the_gate_off(&conditions).is_err(),
                "`{inserted}` is a condition this reader cannot evaluate and must fail \
                 closed rather than be assumed harmless"
            );
        }

        // The gate somewhere this reader cannot see. Renaming either the job or
        // the step is a tree whose gate this guard has not located, and the
        // only thing it can do about that is refuse to pass (SAK-221).
        assert!(deny_gate_conditions(&ci_yml_with(DENY_JOB_LINE, "  supply-chain:")).is_err());
        assert!(deny_gate_conditions(&ci_yml_with(
            GATE_STEP_LINE,
            "      - name: dependency audit"
        ))
        .is_err());

        // A commented-out job header is not a job header.
        assert!(deny_gate_conditions(&ci_yml_with(DENY_JOB_LINE, "#  deny:")).is_err());

        // Two jobs by that name: which one carries the gate is unreadable.
        assert!(deny_gate_conditions(&ci_yml_with(
            DENY_JOB_LINE,
            "  deny:\n    runs-on: ubuntu-latest\n  deny:"
        ))
        .is_err());

        // A job header with nothing under it.
        assert!(deny_gate_conditions("jobs:\n  deny:\n").is_err());
    }

    /// The direction control, and the arm without which this guard would be
    /// rejecting trees safer than the one shipped.
    ///
    /// Every value in `CONDITIONS_THAT_STILL_RUN` leaves the gate running —
    /// `if: always()` was measured on this repository's own run `42318904935`,
    /// where two `if: always()` steps both ran to `conclusion: success`. So is
    /// `continue-on-error: false`, which is the key written down with the value
    /// it already has by default.
    ///
    /// And `if: false` on the *Install* step has to pass. Skipping the install
    /// leaves no `cargo-deny` on `PATH`, so the gate step fails — that mutation
    /// is fail-*safe*, and it is the arm that rules out a predicate merely
    /// reacting to "an `if:` somewhere in the job".
    #[test]
    fn conditions_that_still_run_the_gate_pass() {
        for value in CONDITIONS_THAT_STILL_RUN {
            // All three spellings, because quoting an `if:` value changes
            // nothing about what it means and `'!cancelled()'` is the *only*
            // way to write that member at all — a bare `!` opens a YAML tag and
            // the file stops parsing (SAK-250).
            for spelling in [
                value.to_string(),
                format!("'{value}'"),
                format!("\"{value}\""),
            ] {
                for (anchor, indent) in [(DENY_JOB_LINE, "    "), (GATE_STEP_LINE, "        ")] {
                    let conditions = deny_gate_conditions(&ci_yml_with_key_under(
                        anchor,
                        &format!("{indent}if: {spelling}"),
                    ))
                    .expect("readable");
                    assert!(
                        check_nothing_switches_the_gate_off(&conditions).is_ok(),
                        "`if: {spelling}` leaves the gate running and must not be reported \
                         as a hole — a guard that reddened on it would be failing a tree \
                         that is safer than the one shipped"
                    );
                }
            }
        }

        for (anchor, inserted) in [
            (DENY_JOB_LINE, "    continue-on-error: false"),
            (GATE_STEP_LINE, "        continue-on-error: false"),
            (GATE_STEP_LINE, "        continue-on-error: False"),
            // Quoted, which no runner accepts for this key — the schema wants a
            // `BooleanToken`. Green here all the same: that tree fails to parse
            // before the gate runs, so a guard reddening on it is reporting a
            // hole that is not one. See `unquote_condition`.
            (DENY_JOB_LINE, "    continue-on-error: 'false'"),
            (GATE_STEP_LINE, "        continue-on-error: 'false'"),
            (GATE_STEP_LINE, "        continue-on-error: \"false\""),
            // Fail-safe, not fail-open: no install means no binary means the
            // gate step fails.
            (INSTALL_STEP_LINE, "        if: false"),
        ] {
            let conditions =
                deny_gate_conditions(&ci_yml_with_key_under(anchor, inserted)).expect("readable");
            assert!(
                check_nothing_switches_the_gate_off(&conditions).is_ok(),
                "`{inserted}` under `{anchor}` does not switch the gate off and must pass"
            );
        }
    }

    /// The one live `pull_request:` line in the real `ci.yml`.
    const GATE_EVENT_LINE: &str = "  pull_request:";

    /// The one live `push:` line, for the arm about a filter on a *different*
    /// event.
    const PUSH_EVENT_LINE: &str = "  push:";

    /// The real `ci.yml` with every `TRIGGER_NARROWING_KEYS` line removed.
    ///
    /// The base every arm below mutates. On the shipped tree it removes exactly
    /// one line — `push:`'s `branches: [main]` — and nothing under
    /// `pull_request`, which has no body.
    ///
    /// Normalising first is what keeps each arm measuring the key *it* added,
    /// and it is the lesson of the first draft of this guard rather than a
    /// precaution. Built on the raw file, the arms below did redden on a real
    /// mutation: putting `branches: [main]` on the live `pull_request:` failed
    /// **four** tests, because three of these controls read the shipped file
    /// and every one of them had an opinion about it. A suite where one hole
    /// fails four tests says less, and says it worse, than one where it fails
    /// the single test whose message is about that hole. That is the same trap
    /// `ci_yml_base` was written for one ticket earlier.
    ///
    /// The test that reads the shipped file unnormalised — and so is the one
    /// that goes red when a narrowing key really lands there — is
    /// `every_pull_request_starts_the_workflow_holding_the_deny_gate`.
    ///
    /// Normalising means two things, not one: the narrowing keys come out, and
    /// every line the arms anchor to goes back in if it is not there. Stripping
    /// alone was not enough, and the second half is a measured correction
    /// rather than a precaution. With only the filter, the arms that *delete* a
    /// trigger line still failed three and four tests each — the helpers below
    /// anchor to those lines and cannot find them. A normalised base has to be
    /// normal in both directions: free of what an arm adds, and in possession
    /// of what an arm removes.
    fn ci_yml_trigger_base() -> String {
        let mut lines: Vec<String> = repo_file(".github/workflows/ci.yml")
            .lines()
            .filter(|line| {
                let body = without_comment(line).trim();
                !TRIGGER_NARROWING_KEYS
                    .iter()
                    .any(|key| body.starts_with(&format!("{key}:")))
            })
            .map(str::to_string)
            .collect();

        let on_at = lines
            .iter()
            .position(|line| without_comment(line).trim() == "on:")
            .expect("the base is built from a ci.yml that has an `on:` block");
        for anchor in [GATE_EVENT_LINE, PUSH_EVENT_LINE] {
            if !lines.iter().any(|line| line == anchor) {
                lines.insert(on_at + 1, anchor.to_string());
            }
        }

        lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>()
    }

    /// The base with `nested` added as the body of `anchor`'s event.
    ///
    /// Built from the real file rather than a synthetic workflow, for the same
    /// reason the SAK-244 arms are: a synthetic `on:` block could differ from
    /// the shipped one in the single structural way that mattered — the comment
    /// block sitting between `push:` and `pull_request:`, for one — and nothing
    /// would say so.
    ///
    /// `with_sole_line` asserts one live match before editing, which is what
    /// keeps this off the prose mentions of `pull_request` elsewhere in the file.
    fn ci_yml_event_body(anchor: &str, nested: &str) -> String {
        with_sole_line(
            &ci_yml_trigger_base(),
            anchor,
            &format!("{anchor}\n{nested}"),
        )
    }

    fn ci_yml_gate_event_body(nested: &str) -> String {
        ci_yml_event_body(GATE_EVENT_LINE, nested)
    }

    /// The reader, before anything is asserted about what it read.
    ///
    /// Note what is asked of the real file and what is not: only that its
    /// trigger block is **readable**. Nothing here says the shipped
    /// `pull_request:` has no body, even though today it has none, and nothing
    /// pins the shipped event list — adding an event is a tree this guard has
    /// to pass. An `assert_eq!` on either would be the SAK-241 mistake, which
    /// SAK-244 reproduced one ticket ago and caught only on its
    /// stricter-direction control.
    ///
    /// The grammar is pinned on the normalised base instead, where the input is
    /// fixed and `assert_eq!` is honest.
    #[test]
    fn the_gate_trigger_reader_finds_the_live_triggers() {
        deny_gate_trigger(&repo_file(".github/workflows/ci.yml")).expect(
            "the shipped trigger block must be readable, or every arm below is failing on \
             the reader rather than on the trigger it put there",
        );

        // The base: `pull_request` declared with no body at all.
        let base = deny_gate_trigger(&ci_yml_trigger_base()).expect("readable");
        assert_eq!(base.gate_event_keys, Some(Vec::new()));
        assert!(base.events.iter().any(|event| event == GATE_EVENT));

        // A narrowing key is read with its own name, and the value is not
        // needed for anything: `[main]` and `['src/**']` both narrow.
        let read = deny_gate_trigger(&ci_yml_gate_event_body("    branches: [main]")).expect("ok");
        assert_eq!(read.gate_event_keys, Some(vec!["branches".to_string()]));

        // Several, with a comment on one of them stripped rather than read as
        // part of the key, and the nested sequence items not read as keys.
        let read = deny_gate_trigger(&ci_yml_gate_event_body(
            "    branches: [main]\n    paths:  # only the sources\n      - 'src/**'",
        ))
        .expect("ok");
        assert_eq!(
            read.gate_event_keys,
            Some(vec!["branches".to_string(), "paths".to_string()])
        );

        // Depth is exact, not "at least": a `branches:` under a *different*
        // event belongs to that event. This is the shipped file's own shape —
        // `push:` carries one — and the reader must not attribute it here.
        let read = deny_gate_trigger(&ci_yml_event_body(PUSH_EVENT_LINE, "    branches: [main]"))
            .expect("ok");
        assert_eq!(read.gate_event_keys, Some(Vec::new()));
    }

    /// The hole this closes: the workflow does not start, so the gate inside it
    /// has no verdict and nothing reports the absence.
    #[test]
    fn a_trigger_that_keeps_pull_requests_from_the_gate_fails() {
        for (nested, expected) in [
            ("    branches: [main]", "branches"),
            ("    branches-ignore: ['sak-*']", "branches-ignore"),
            ("    paths: ['src/**']", "paths"),
            ("    paths-ignore: ['deny.toml']", "paths-ignore"),
            ("    types: [labeled]", "types"),
        ] {
            let trigger = deny_gate_trigger(&ci_yml_gate_event_body(nested)).expect("readable");
            let Err(error) = check_every_pull_request_runs_the_gate(&trigger) else {
                panic!(
                    "`{nested}` must fail: it subtracts from the pull requests that run the gate"
                );
            };
            assert!(
                error.contains(expected),
                "the failure has to name the narrowing key it found, got: {error}"
            );
        }

        // The event gone altogether, which is every pull request rather than
        // some of them. Removing the line is the mutation; the `push:` and
        // `workflow_dispatch:` triggers stay, so the workflow still exists and
        // the arm is about this event only.
        let without_event: String = ci_yml_trigger_base()
            .lines()
            .filter(|line| *line != GATE_EVENT_LINE)
            .map(|line| format!("{line}\n"))
            .collect();
        let trigger = deny_gate_trigger(&without_event).expect("readable");
        let Err(error) = check_every_pull_request_runs_the_gate(&trigger) else {
            panic!("a ci.yml that does not trigger on `{GATE_EVENT}` must fail");
        };
        assert!(
            error.contains("not `pull_request`"),
            "the failure has to say which event is missing, got: {error}"
        );

        // The control, and the arm that says the guard is satisfiable at all:
        // the same base with nothing added to it passes. Deliberately the base
        // and not the raw shipped file — that assertion is
        // `every_pull_request_starts_the_workflow_holding_the_deny_gate`, and
        // duplicating it here would make a real hole fail two tests.
        let trigger = deny_gate_trigger(&ci_yml_trigger_base()).expect("readable");
        assert!(
            check_every_pull_request_runs_the_gate(&trigger).is_ok(),
            "the base must pass, or the arms above are failing on something other than \
             the key they added"
        );
    }

    /// The direction control, and the arm without which this guard would be
    /// rejecting trees that run the gate *more* often than the one shipped.
    ///
    /// Adding an event is not a hole. Neither is narrowing an event that is not
    /// the one gating the merge: `push: branches: [main]` is in the shipped
    /// file and costs the post-merge run on `main`, not the pull-request run.
    #[test]
    fn triggers_that_still_reach_the_gate_pass() {
        for nested in [
            // A key under `pull_request` that is not a narrowing key.
            "    # a comment is not a filter",
        ] {
            let trigger = deny_gate_trigger(&ci_yml_gate_event_body(nested)).expect("readable");
            assert!(
                check_every_pull_request_runs_the_gate(&trigger).is_ok(),
                "`{nested}` does not subtract any pull request and must pass"
            );
        }

        // More ways to run, each of which this guard must pass.
        for extra in [
            "  schedule:\n    - cron: '0 3 * * *'",
            "  pull_request_target:\n    branches: [main]",
            "  merge_group:",
        ] {
            let widened = ci_yml_event_body(GATE_EVENT_LINE, extra);
            let trigger = deny_gate_trigger(&widened).expect("readable");
            assert!(
                check_every_pull_request_runs_the_gate(&trigger).is_ok(),
                "`{extra}` is one more way the gate runs, not a hole — a guard that \
                 reddened on it would be failing a tree stricter than the one shipped"
            );
        }

        // A narrowing key on a *different* event. This is the shipped file's
        // own shape, restored onto the base: narrowing `push` costs the
        // post-merge run on `main`, not the pull-request run that gates the
        // merge, so it has to pass. Without this arm, a predicate that merely
        // reacted to "a narrowing key somewhere under `on:`" would look correct.
        let trigger =
            deny_gate_trigger(&ci_yml_event_body(PUSH_EVENT_LINE, "    branches: [main]"))
                .expect("readable");
        assert!(
            check_every_pull_request_runs_the_gate(&trigger).is_ok(),
            "`push: branches: [main]` is the shipped tree and must pass"
        );
    }

    /// Fails closed on a trigger block it cannot read.
    #[test]
    fn a_trigger_block_this_reader_cannot_read_fails_closed() {
        // No `on:` at all.
        assert!(deny_gate_trigger("name: CI\njobs:\n  deny:\n    runs-on: x\n").is_err());

        // `on:` with nothing under it.
        assert!(deny_gate_trigger("name: CI\non:\njobs:\n  deny:\n").is_err());

        // The flow-sequence form, which this reader has no grammar for. It
        // refuses rather than reading zero events out of it and passing.
        assert!(deny_gate_trigger("name: CI\non:\n  - push\n  - pull_request\n").is_err());

        // Two `on:` blocks: which one governs is not something a reader can
        // tell, and YAML duplicate keys are not an error everything agrees on.
        assert!(deny_gate_trigger("on:\n  push:\non:\n  pull_request:\n").is_err());

        // A commented-out trigger block is not a trigger block.
        let commented: String = ci_yml_trigger_base()
            .lines()
            .map(|line| {
                if line == "on:" {
                    "# on:\n".to_string()
                } else {
                    format!("{line}\n")
                }
            })
            .collect();
        assert!(deny_gate_trigger(&commented).is_err());
    }

    /// The `Install cargo-deny` step the SAK-246 arms mutate, spliced into the
    /// real `ci.yml` in place of the shipped one.
    ///
    /// ## Why a canonical step and not the shipped bytes
    ///
    /// The arms below anchor to whole lines of the install step and edit them.
    /// Run against the real file that is wrong twice over, and the first draft
    /// of this block was wrong in both ways:
    ///
    /// * A real hole landing in the shipped step — someone actually deleting
    ///   the verification — reddened **four** tests rather than one, because
    ///   three of them read the same file and were reading it with the hole on
    ///   it. One hole failing one test, whose message is about that hole, says
    ///   more and says it better.
    /// * A real tree that was *stricter* — `set -o errexit` spelled out, or the
    ///   comparison done twice — made the anchors stop matching, and
    ///   `with_sole_line` fails on an anchor it cannot find. Six safe trees
    ///   came back red for a reason that had nothing to do with their safety.
    ///
    /// So the base has to lack what an arm adds (`set +e`) and possess what an
    /// arm removes or rewrites (the `set` line, the comparison, the pin) — both
    /// halves, which is the shape SAK-245 arrived at the hard way.
    ///
    /// Still spliced into the real file rather than built as a synthetic
    /// workflow, for the reason `with_sole_line` gives: the job, the step
    /// sequence and the indentation around it stay the ones that actually run.
    /// The version and digest here are deliberately not the shipped ones —
    /// nothing below is a statement about which release this commit pins.
    const CANONICAL_INSTALL_STEP: &str = concat!(
        "      - name: Install cargo-deny\n",
        "        env:\n",
        "          CARGO_DENY_VERSION: 0.0.0\n",
        "          CARGO_DENY_SHA256: ",
        "0000000000000000000000000000000000000000000000000000000000000000\n",
        "        run: |\n",
        "          set -euo pipefail\n",
        "          curl -sSfL -o cargo-deny.tar.gz \"$CARGO_DENY_URL\"\n",
        "          echo \"${CARGO_DENY_SHA256}  cargo-deny.tar.gz\" | sha256sum -c -\n",
        "          tar xzf cargo-deny.tar.gz\n",
        "          install -m 0755 cargo-deny \"${RUNNER_TEMP}/bin/cargo-deny\"\n",
    );

    /// The three lines of `CANONICAL_INSTALL_STEP` the arms anchor to.
    const VERIFY_LINE: &str =
        r#"          echo "${CARGO_DENY_SHA256}  cargo-deny.tar.gz" | sha256sum -c -"#;
    const SET_LINE: &str = "          set -euo pipefail";
    const SHA_PIN_LINE: &str =
        "          CARGO_DENY_SHA256: 0000000000000000000000000000000000000000000000000000000000000000";

    /// The real `ci.yml` with its install step replaced by the canonical one.
    fn install_yml_base() -> String {
        let ci = repo_file(".github/workflows/ci.yml");
        let lines: Vec<&str> = ci.lines().collect();
        let at = sole_line(
            &lines,
            &format!("- name: {INSTALL_STEP}"),
            "the step this base replaces",
        )
        .expect("the shipped install step must be locatable for these arms to have a base");

        let mut out = String::new();
        for line in &lines[..at] {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(CANONICAL_INSTALL_STEP);
        // Everything from the next item in the `steps:` sequence onwards.
        let rest = lines[at + 1..].iter().skip_while(|line| {
            let body = without_comment(line).trim();
            body != "-" && !body.starts_with("- ")
        });
        for line in rest {
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    fn install_arm(anchor: &str, replacement: &str) -> Result<(), String> {
        check_install_verifies_its_download(
            &install_step(&with_sole_line(&install_yml_base(), anchor, replacement))
                .expect("the install step stays locatable in every arm that only edits its body"),
        )
    }

    /// The base is a tree this guard passes, so a red arm below is the arm's
    /// mutation and not the splice.
    #[test]
    fn the_canonical_install_step_is_a_passing_base() {
        let base = install_yml_base();
        assert!(check_install_verifies_its_download(
            &install_step(&base).expect("the canonical step is locatable")
        )
        .is_ok());
        // It is a base the arms can anchor into: three lines, one match each,
        // and `set +e` is not already on it.
        for anchor in [VERIFY_LINE, SET_LINE, SHA_PIN_LINE] {
            assert_eq!(base.lines().filter(|line| *line == anchor).count(), 1);
        }
        assert!(!base.contains("set +e"));
        // And it is still the real workflow around that step.
        assert!(base.contains(&format!("- name: {DENY_STEP}")));
        assert!(base.contains(&format!("- name: {LOG_STEP}")));
    }

    /// The five ways the digest comparison stops comparing, each measured
    /// against the real file.
    ///
    /// Every one is step exit 0 with an unverified binary installed, and
    /// `cargo test --test dependency_pin_tests` was 159 passed on all of them
    /// before this guard existed.
    #[test]
    fn a_download_nothing_compares_fails() {
        // 1. The line is gone. Paired with the version moved to 0.19.9, the
        //    step installs 0.19.9 and exits 0.
        assert!(install_arm(VERIFY_LINE, "          # nothing checks the tarball").is_err());

        // 2. The status goes to `||`. The comparison still *runs* and still
        //    prints `cargo-deny.tar.gz: FAILED`; the step exits 0 anyway.
        assert!(install_arm(VERIFY_LINE, &format!("{VERIFY_LINE} || true")).is_err());
        assert!(install_arm(VERIFY_LINE, &format!("{VERIFY_LINE} || echo mismatch")).is_err());
        assert!(install_arm(VERIFY_LINE, &format!("{VERIFY_LINE} || exit 0")).is_err());

        // 3. A digest that is printed rather than compared.
        assert!(install_arm(VERIFY_LINE, "          sha256sum cargo-deny.tar.gz").is_err());

        // 4. errexit taken back off before the comparison runs.
        assert!(install_arm(SET_LINE, &format!("{SET_LINE}\n          set +e")).is_err());
        assert!(install_arm(SET_LINE, &format!("{SET_LINE}\n          set +eu")).is_err());
        assert!(install_arm(SET_LINE, &format!("{SET_LINE}\n          set +o errexit")).is_err());

        // 5. A digest fetched from the same host as the tarball it vouches for.
        assert!(install_arm(
            VERIFY_LINE,
            "          curl -sSfL -o SHA256SUMS \"$BASE/SHA256SUMS\"\n          \
             sha256sum -c SHA256SUMS"
        )
        .is_err());
    }

    /// Trees that are as safe as the shipped one, or safer, stay green.
    ///
    /// Without these the cheapest way back to green from a red build is to undo
    /// the strictness, which is the lesson SAK-241 paid for. Six of these came
    /// back red against an un-spliced base; see `CANONICAL_INSTALL_STEP`.
    #[test]
    fn a_stricter_install_step_passes() {
        // The shipped file is deliberately *not* read here. The one test that
        // reads it unnormalised is `ci_yml_verifies_the_gate_binary_before_running_it`,
        // so that a hole landing in `ci.yml` fails that one test and no other —
        // two tests reporting one hole is how the first draft of this block
        // read, and the second message was the worse of the two.

        // `set -o errexit` spelled out. Measured on the real file: step exit 0,
        // 0.20.2 installed.
        assert!(install_arm(SET_LINE, "          set -o errexit -o nounset -o pipefail").is_ok());

        // The body's own `set` without `e`, with and without an explicit abort.
        // Measured: the runner invokes `bash -e`, so `set -uo pipefail` plus a
        // version bump is still step exit 1 with nothing installed. Dropping
        // the `e` is not the hole; only `set +e` is.
        assert!(install_arm(VERIFY_LINE, &format!("{VERIFY_LINE} || exit 1")).is_ok());
        assert!(install_arm(SET_LINE, "          set -uo pipefail").is_ok());

        // Quieter, still comparing. Measured: no `: OK` line in the log, step
        // exit 0, right binary — which is also why the log line is not what
        // this guard reads.
        assert!(install_arm(
            VERIFY_LINE,
            &VERIFY_LINE.replace("sha256sum -c -", "sha256sum -c --status -")
        )
        .is_ok());

        // A different tool, and the same thing twice.
        assert!(install_arm(
            VERIFY_LINE,
            &VERIFY_LINE.replace("sha256sum -c -", "shasum -a 256 -c -")
        )
        .is_ok());
        assert!(install_arm(VERIFY_LINE, &format!("{VERIFY_LINE}\n{VERIFY_LINE}")).is_ok());

        // The digest written out instead of referenced. Equally pinned by this
        // commit, so equally fine — which is why the guard asks where the digest
        // comes from rather than demanding a particular `env:` key.
        assert!(install_arm(
            VERIFY_LINE,
            "          echo \"9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f  \
             cargo-deny.tar.gz\" | sha256sum -c -"
        )
        .is_ok());

        // A `set +e` after the comparison has already run and been acted on.
        // Harmless, so not a hole — the arms above put it *before*.
        assert!(install_arm(VERIFY_LINE, &format!("{VERIFY_LINE}\n          set +e")).is_ok());
    }

    /// The digest the comparison reads has to be a number this commit holds.
    ///
    /// The name being declared in this step's `env:` was the whole of the
    /// earlier question, and a declared name's *value* can still live outside
    /// the repository. Measured by replaying the step body the way the runner
    /// does, with the step's own `env:` map applied and `${{ }}` filled from
    /// outside:
    ///
    /// ```text
    ///   tree                  outside says            step exit  installed
    ///   shipped literal       nothing                     0      0.20.2
    ///   shipped literal       0.19.9 + its digest          0      0.20.2   <- the tree decides
    ///   `${{ vars.* }}`       0.20.2 + its digest          0      0.20.2
    ///   `${{ vars.* }}`       0.19.9 + its digest          0      0.19.9   <- outside decides
    ///   `${{ vars.* }}`       0.19.9 + wrong digest        1      none
    ///   `${{ vars.* }}`       nothing                      1      none
    /// ```
    ///
    /// Row two is the control that makes row four a finding: with the literal
    /// in place an outside value changes nothing at all. Rows five and six are
    /// the other control — the comparison still runs and still fails closed on
    /// a mismatch, so what the fourth row shows is not verification being off
    /// but the number being chosen somewhere this repository cannot see. Each
    /// arm was `181 passed` and `actionlint` rc 0 before this assertion
    /// existed.
    #[test]
    fn a_digest_supplied_from_outside_the_repository_fails() {
        for value in [
            "${{ vars.CARGO_DENY_SHA256 }}",
            "${{ secrets.CARGO_DENY_SHA256 }}",
            "${{ env.CARGO_DENY_SHA256 }}",
            "${{ github.event.inputs.sha }}",
            // A literal is present, and an outside value still wins over it.
            // This is why `is_a_pinned_digest` asks two things rather than one.
            concat!(
                "${{ vars.CARGO_DENY_SHA256 || '",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "' }}"
            ),
            // Not a digest at all. Fail-closed at run time — an empty or
            // malformed checksum line is `sha256sum` exit 1 with nothing
            // installed — but unreadable to this guard either way, and the
            // rule everywhere else in this file is that an unreadable pin
            // fails rather than passes.
            "\"\"",
            "latest",
        ] {
            let why = install_arm(
                SHA_PIN_LINE,
                &format!("          CARGO_DENY_SHA256: {value}"),
            )
            .expect_err(
                "a digest whose value this commit does not hold is not a pin, and the \
                 earlier `declared` check passes it: the name is right there in `env:`",
            );
            assert!(
                why.contains("does not fix"),
                "the failure must say the value is not fixed by this commit, said: {why}"
            );
            assert!(
                why.contains("CARGO_DENY_SHA256"),
                "the failure must name the env key whose value moved, said: {why}"
            );
        }
    }

    /// And a digest this commit does hold stays green, including the shapes
    /// that are stricter than the one shipped.
    ///
    /// Without these the cheapest way back to green would be to delete the
    /// assertion above — the mistake SAK-241 undid in
    /// `the_permissions_reader_finds_exactly_the_live_keys`.
    #[test]
    fn a_digest_this_commit_holds_passes() {
        // The base's own literal, quoted and unquoted.
        assert!(install_arm(SHA_PIN_LINE, SHA_PIN_LINE).is_ok());
        assert!(install_arm(
            SHA_PIN_LINE,
            &format!("          CARGO_DENY_SHA256: \"0000000000000000000000000000000000000000000000000000000000000000\"")
        )
        .is_ok());
        // Under a different name, still written out here.
        assert!(install_arm(
            SHA_PIN_LINE,
            &format!("          CARGO_DENY_SHA256: 0000000000000000000000000000000000000000000000000000000000000000\n          GATE_DIGEST: 0000000000000000000000000000000000000000000000000000000000000000")
        )
        .is_ok());
        // The *version* from outside, with the digest left literal. Measured:
        // this gives an outside value no new freedom — it can only pick the
        // version the digest already pins (0.20.2 exit 0, 0.19.9 exit 1, unset
        // exit 1). Only the digest is load-bearing for which bytes run, so
        // only the digest is asked for, and this arm is what keeps the
        // assertion from over-reaching to both pins.
        assert!(install_arm(
            "          CARGO_DENY_VERSION: 0.0.0",
            "          CARGO_DENY_VERSION: ${{ vars.CARGO_DENY_VERSION }}"
        )
        .is_ok());
        // A comparison line that reads two declared names, only one of which
        // is a digest. This is why the check asks whether *any* name it reads
        // is a pinned digest rather than whether all of them are: the version
        // is a declared name and is not a digest, and a tree that builds the
        // filename out of it is pinned just the same.
        assert!(install_arm(
            VERIFY_LINE,
            "          echo \"${CARGO_DENY_SHA256}  cargo-deny-${CARGO_DENY_VERSION}.tar.gz\" \
             | sha256sum -c -"
        )
        .is_ok());
    }

    /// Shapes the reader cannot read fail rather than passing.
    #[test]
    fn an_unreadable_install_step_fails() {
        let base = install_yml_base();

        // The step renamed, so this guard would be reading about a step that is
        // not the one installing the binary.
        assert!(install_step(&base.replace(
            &format!("- name: {INSTALL_STEP}"),
            &format!("- name: {INSTALL_STEP} RENAMED")
        ))
        .is_err());

        // Commented out.
        assert!(install_step(&base.replace(
            &format!("- name: {INSTALL_STEP}"),
            &format!("# - name: {INSTALL_STEP}")
        ))
        .is_err());

        // Two of them: which one installs the binary the gate runs is not
        // something a reader can tell.
        assert!(install_step(&format!(
            "jobs:\n  deny:\n    steps:\n      - name: {INSTALL_STEP}\n        run: true\n      \
             - name: {INSTALL_STEP}\n        run: true\n"
        ))
        .is_err());

        // No `run:` at all, and a `run:` with nothing under it.
        assert!(install_step(&format!(
            "    steps:\n      - name: {INSTALL_STEP}\n        uses: some/action@v1\n"
        ))
        .is_err());
        assert!(install_step(&format!(
            "    steps:\n      - name: {INSTALL_STEP}\n        run: |\n      - name: next\n"
        ))
        .is_err());

        // A one-line `run:` is read as a body rather than skipped — and a
        // one-liner that installs without comparing is the hole, not an
        // exemption from it.
        let inline = install_step(&format!(
            "    steps:\n      - name: {INSTALL_STEP}\n        run: curl -sSfL -o d.tgz \"$URL\" \
             && tar xzf d.tgz\n"
        ))
        .expect("an inline body is still a body");
        assert!(check_install_verifies_its_download(&inline).is_err());

        // The pin deleted from `env:` while the body still reads it. At runtime
        // that is fail-closed — `set -u` makes the step exit 1 with nothing
        // installed (measured) — so this arm is not reporting a hole. It is here
        // because the *reader* can no longer tell a pinned digest from a fetched
        // one, and an unreadable comparison fails rather than passing.
        assert!(install_arm(SHA_PIN_LINE, "          # pin removed").is_err());
    }

    /// The two helpers the predicate leans on, pinned on synthetic input where
    /// `assert_eq!` is honest because the input is fixed.
    #[test]
    fn the_install_step_readers_grammar() {
        assert!(clears_errexit("set +e"));
        assert!(clears_errexit("set +eu"));
        assert!(clears_errexit("set +uo pipefail +e"));
        assert!(clears_errexit("set +o errexit"));
        assert!(!clears_errexit("set -euo pipefail"));
        assert!(!clears_errexit("set +u"));
        assert!(!clears_errexit("set +o pipefail"));
        // Not a `set` line at all, however much it looks like one.
        assert!(!clears_errexit("echo 'set +e'"));

        assert_eq!(
            shell_vars("echo \"${A}  $B\" | x -c -"),
            vec!["A".to_string(), "B".to_string()]
        );
        assert_eq!(shell_vars("sha256sum -c SHA256SUMS"), Vec::<String>::new());

        assert!(has_sha256_literal(
            "9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f  f"
        ));
        assert!(!has_sha256_literal("9f12ed4c  f"));

        // The body of the canonical step as this reader sees it: comments gone,
        // the comparison present exactly once. Asserted against the base rather
        // than the shipped file, which may legitimately verify twice.
        let step = install_step(&install_yml_base()).expect("locatable");
        assert_eq!(
            step.body
                .iter()
                .filter(|line| DIGEST_COMPARISONS.iter().any(|form| line.contains(form)))
                .count(),
            1
        );
        assert!(step.body.iter().all(|line| !line.starts_with('#')));
        assert_eq!(
            step.env,
            vec![
                "CARGO_DENY_VERSION: 0.0.0".to_string(),
                "CARGO_DENY_SHA256: \
                 0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            ]
        );
    }
}

/// The sibling test target that holds this file's own reporter guard.
///
/// The guard in question is in `fidelity_matrix_tests`, and it is there because
/// a guard on the step that runs *this* target cannot live in this target: the
/// edit it reports on is the edit that stops it running. That arrangement only
/// holds while both steps are intact, which is what this guard is for. Switch
/// off this file's step and `fidelity_matrix_tests` says so; switch off
/// `fidelity_matrix_tests`' step and this says so. Opening the hole takes both
/// edits rather than either one.
///
/// The same two-independent-keys shape as `unused-ignored-advisory` beside
/// `-D advisory-not-detected`: neither is redundant, because the hole needs
/// both of them gone (SAK-229, SAK-249).
const SIBLING_REPORTER_TARGET: &str = "fidelity_matrix_tests";

/// The live `if:` / `continue-on-error:` on the `ci.yml` step that runs `target`.
///
/// Anchored on the command, not on the step's `name:` -- renaming a step does
/// not stop it running, so a reader keyed on the name would redden on a
/// harmless edit while missing the ones that matter.
///
/// `Err` for no such step and `Err` for more than one, for the reason
/// `deny_step_checks` gives: the reporter gone is worse than any condition on
/// it, and a reader that picked the first of two would be reporting on
/// whichever it happened to find.
fn sibling_reporter_conditions(ci: &str, target: &str) -> Result<Vec<GateCondition>, String> {
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
                "ci.yml runs no `{needle}`. That target holds this file's own reporter guard \
             (SAK-249), so with its step gone, switching off the step that runs \
             dependency_pin_tests would be reported by nothing."
            ))
        }
        _ => {
            return Err(format!(
                "ci.yml has {} steps running `{needle}`; which is the reporter is not \
             something this reader can tell.",
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
            "ci.yml's `{needle}` is not inside a step -- no `- ` line opens a block above \
             it, and this reader only understands the step list form."
        ));
    };

    let step_indent = indent_of(lines[start]);
    let scope = format!("the `{target}` step");
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
        if indent_of(line) == step_indent + 2 {
            keyed.push(inner);
        }
    }

    let mut keys = Vec::new();
    for text in keyed {
        for key in GATE_CONDITION_KEYS {
            if let Some(value) = text.strip_prefix(&format!("{key}:")) {
                keys.push(GateCondition {
                    scope: scope.clone(),
                    key: (*key).to_string(),
                    value: unquote_condition(value.trim()).to_string(),
                });
            }
        }
    }
    Ok(keys)
}

#[test]
fn nothing_in_ci_yml_switches_off_this_files_own_reporter_guard() {
    let conditions = match sibling_reporter_conditions(
        &repo_file(".github/workflows/ci.yml"),
        SIBLING_REPORTER_TARGET,
    ) {
        Ok(conditions) => conditions,
        Err(why) => panic!("{why}"),
    };

    for condition in &conditions {
        if condition.key == "continue-on-error" {
            assert_eq!(
                condition.value, "false",
                "{} carries `continue-on-error: {}`. That step runs the guard protecting \
                 this file's own step in CI, so discarding its verdict leaves the \
                 supply-chain assertions reportable by nothing. Measured: a step that \
                 really fails takes its job to `failure` (run 42153955324).",
                condition.scope, condition.value
            );
        } else {
            assert!(
                CONDITIONS_THAT_STILL_RUN.contains(&condition.value.as_str()),
                "{} carries `if: {}`, so the guard protecting this file's own step need \
                 not run. Measured: a skipped step leaves its job concluding success (run \
                 42161681376). Allowed values are {:?}; anything a text reader cannot \
                 evaluate fails closed.",
                condition.scope,
                condition.value,
                CONDITIONS_THAT_STILL_RUN
            );
        }
    }
}

/// The action every job in `ci.yml` uses to put the tree on disk.
const CHECKOUT_ACTION: &str = "actions/checkout";

/// `actions/checkout` inputs that change *which tree* a job then checks.
///
/// Each of these leaves a workflow that lints clean and runs to a green
/// conclusion while the gate -- and this file's own guards -- read a tree
/// nobody proposed. Measured on the real `ci.yml` (SAK-251):
///
/// | input | actionlint | dependency_pin_tests | the gate |
/// |---|---|---|---|
/// | none (shipped) | ok | ok 173 | exit 0 |
/// | `ref: 5d015dc` on the `deny` job | **ok** | **ok 173** | reads another tree |
/// | `repository: someone/else` on the `deny` job | **ok** | **ok 173** | reads another repo |
/// | `github-server-url: https://ghe.example.com` | **ok** | **ok 180** | reads another server |
/// | `bogus-input: x` | **rc 1** | -- | -- |
///
/// The `bogus-input` row is the control: actionlint does validate this block,
/// so the clean rows are a shippable workflow rather than one that fails to
/// parse. Compare `continue-on-error: 'false'`, which actionlint rejects and
/// `unquote_condition` therefore tolerates.
///
/// `github-server-url` was found by asking actionlint for the list rather than
/// by reading the ones this hole was reported with: rejecting `bogus-input`
/// prints every input `actions/checkout@v4` accepts, and that enumeration is
/// what the three rows above and the fifteen in the exclusion list below are
/// measured against. It is the same key as `repository:` one level out -- the
/// repository path stays `${{ github.repository }}` and the *server* it is
/// fetched from moves -- and the `ok 180` row is against the guard as first
/// committed, which read only `ref` and `repository`.
///
/// What makes the verdict follow the tree, same binary and same pins, only the
/// checkout differing:
///
/// | tree | the gate's step |
/// |---|---|
/// | the proposed one, `"MIT"` dropped from `allow` | exit 4, 48 `rejected` |
/// | the one a `ref:` points at | **exit 0, `licenses ok`** |
///
/// and the same one variable against this file's own guards, with
/// `unsound = "all"` dropped: **1 failed** in the proposed tree, **ok 173** in
/// the tree the `ref:` points at.
///
/// Deliberately *not* listed. The other fifteen inputs in actionlint's list,
/// each measured on the real `ci.yml` rather than assumed -- all fifteen are
/// `actionlint` ok and `180 passed`, so what separates them from the three
/// above is the direction, not the silence:
///
/// - `fetch-depth`, `fetch-tags`, `filter`, `lfs`, `clean`, `show-progress`,
///   `set-safe-directory`, `sparse-checkout-cone-mode`, `submodules`,
///   `persist-credentials` -- neutral or stricter. `persist-credentials: false`
///   narrows the token this job hands to the third-party binary it runs, so a
///   guard reddening on it would be failing a tree safer than the one shipped,
///   which is the mistake SAK-241 undid in the `permissions` reader. (`clean`
///   is neutral because both jobs are `runs-on: ubuntu-latest`: there is no
///   workspace to leave dirty. On a self-hosted runner it would need its own
///   look, which is why the reason is written down rather than the verdict.)
/// - `token`, `ssh-key`, `ssh-user`, `ssh-known-hosts`, `ssh-strict` -- the
///   credential and transport, not the tree. They change how the same
///   `repository` at the same `ref` is fetched. `token` is the one worth
///   naming: handing the checkout a different credential is a real question,
///   but it is SAK-240's question about what the job can *do*, not this one's
///   about which bytes it reads.
/// - `path` and `sparse-checkout` -- fail-closed, not silent. Both leave files
///   the gate needs missing: `path` moves the tree into a subdirectory while
///   the step's working directory stays the workspace root, which is the
///   measured "no manifest" case -- with `deny.toml` simply absent the gate
///   exits 4 with 216 errors, and with `.github/` absent this target reports 37
///   failures. A direction that is loud needs no guard (SAK-247's `--locked`).
///
/// So the list is the inputs whose failure direction is **fail-open**, and only
/// those. Three fail-open, two loud, fifteen neutral is every input the action
/// has; the exclusions are enumerated rather than summarised so that a new
/// input in `actions/checkout@v5` is visibly not yet classified.
const CHECKOUT_REDIRECTING_INPUTS: &[&str] = &["ref", "repository", "github-server-url"];

/// One live `actions/checkout` step in `ci.yml`.
#[derive(Debug, PartialEq, Eq)]
struct CheckoutStep {
    /// The job it belongs to, in the words the failure message uses.
    job: String,
    /// Its `with:` input names, comments stripped. Every one of them, not just
    /// the redirecting ones, so a reader that silently found nothing is
    /// distinguishable from a step that really declares nothing.
    inputs: Vec<String>,
}

/// Every live `actions/checkout` step in `ci.yml`, with the inputs it declares.
///
/// Reads both spellings of a step, because a reader that knew only the shipped
/// one would be fail-open on the other:
///
/// ```yaml
/// - uses: actions/checkout@v4          # what ci.yml ships
///
/// - name: Checkout                     # equally valid, and `with:` sits in
///   uses: actions/checkout@v4          # exactly the same place
///   with:
///     ref: whatever
/// ```
///
/// so the step is located by walking back from the `uses:` line to the `- `
/// that opens it, and the inputs are read as siblings of that line rather than
/// at a depth guessed from the shipped file's layout.
///
/// `Err` when `jobs:` cannot be found, when a `uses:` line is not inside any
/// step, or when the file declares no checkout at all. Fail-closed for the
/// reason `deny_gate_conditions` is: every promise below is about checkouts
/// this reader located, so a reader that located none must refuse to pass
/// rather than report a clean file.
fn checkout_steps(ci: &str) -> Result<Vec<CheckoutStep>, String> {
    let lines: Vec<&str> = ci.lines().collect();
    let jobs_at = sole_line(&lines, "jobs:", "the workflow's jobs")?;
    let jobs_body = block_under(&lines, jobs_at);
    let Some(job_indent) = jobs_body.iter().map(|(_, line)| indent_of(line)).min() else {
        return Err(
            "ci.yml's `jobs:` has no body, so there are no checkouts to read and \
                    nothing below is about the gate as it ships."
                .to_string(),
        );
    };

    // Which job each line belongs to, by the nearest job key above it.
    let mut job_of: Vec<(usize, String)> = Vec::new();
    for (at, line) in &jobs_body {
        if indent_of(line) == job_indent {
            let name = without_comment(line)
                .trim()
                .trim_end_matches(':')
                .to_string();
            job_of.push((*at, name));
        }
    }

    let mut steps = Vec::new();
    for (at, line) in &jobs_body {
        if !without_comment(line).contains(CHECKOUT_ACTION) {
            continue;
        }
        // Walk back to the `- ` that opens this step. `uses:` may be its first
        // key or a later one.
        let opens_step = |at: usize| without_comment(lines[at]).trim_start().starts_with("- ");
        let mut root = *at;
        while root > jobs_at && !opens_step(root) {
            root -= 1;
        }
        if !opens_step(root) {
            return Err(format!(
                "ci.yml line {} uses `{CHECKOUT_ACTION}` but is not inside a step this \
                 reader can find the start of, so it cannot tell which inputs belong to \
                 it. It fails rather than passing: an unread checkout is one that could \
                 be pointing the gate at another tree.",
                at + 1
            ));
        }

        let key_indent = indent_of(lines[root]) + 2;
        let mut inputs = Vec::new();
        for (with_at, with_line) in block_under(&lines, root) {
            if indent_of(with_line) != key_indent {
                continue;
            }
            if without_comment(with_line).trim() != "with:" {
                continue;
            }
            for (_, input_line) in block_under(&lines, with_at) {
                let body = without_comment(input_line).trim();
                if let Some((key, _)) = body.split_once(':') {
                    inputs.push(key.trim().to_string());
                }
            }
        }

        let job = job_of
            .iter()
            .rev()
            .find(|(job_at, _)| *job_at <= root)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| "an unnamed job".to_string());
        steps.push(CheckoutStep { job, inputs });
    }

    if steps.is_empty() {
        return Err(format!(
            "ci.yml declares no `{CHECKOUT_ACTION}` step. Every assertion in this file \
             is about files read out of a checkout, so with none to read this guard \
             reports nothing rather than reporting a clean file."
        ));
    }
    Ok(steps)
}

/// No checkout in `ci.yml` redirects its job to a different tree.
fn check_no_checkout_redirects_the_tree(steps: &[CheckoutStep]) -> Result<(), String> {
    for step in steps {
        for input in &step.inputs {
            if !CHECKOUT_REDIRECTING_INPUTS.contains(&input.as_str()) {
                continue;
            }
            // The evidence is per input, not one sentence reused for all
            // three: the `ref:` measurement is a statement about `ref:`, and
            // quoting it under `github-server-url:` would be reporting a
            // number nobody took.
            let measured = match input.as_str() {
                "ref" => {
                    "same command, same binary, same pins, only the tree differing: with \
                     the proposed tree's `allow` missing `\"MIT\"` the gate's step exits 4 \
                     with 48 `rejected`, and against the tree a `ref:` points at it exits \
                     0 with `licenses ok`"
                }
                "repository" => {
                    "`repository: someone/else` on this job is `actionlint` ok and leaves \
                     this target green, and the tree the gate then reads is whatever that \
                     other repository's default branch holds"
                }
                "github-server-url" => {
                    "`github-server-url: https://ghe.example.com` is `actionlint` ok and \
                     left this target at `180 passed` before it was listed here; the \
                     repository path stays `${{ github.repository }}` and the server the \
                     whole tree is cloned from moves"
                }
                _ => "measured as fail-open, see `CHECKOUT_REDIRECTING_INPUTS`",
            };
            return Err(format!(
                "the `{}` job's checkout declares `{input}:`, which chooses the tree that \
                 job then reads. Measured: {measured}. Nothing else reports this -- \
                 actionlint accepts it, and the gate reads whatever is on disk. If a \
                 redirect is genuinely wanted, say which of \
                 {CHECKOUT_REDIRECTING_INPUTS:?} and why in the same diff; the other \
                 fifteen inputs, such as `fetch-depth` and `persist-credentials`, are \
                 not reported.",
                step.job
            ));
        }
    }
    Ok(())
}

/// The gate, and this file's own guards, read the tree that was proposed.
///
/// Scope, stated exactly, because half of this is not closeable from here.
///
/// The `deny` job's checkout **is**: it is a different job, so when it is
/// redirected this target still runs against the real tree and sees the
/// `ref:`. That half is what this test reports.
///
/// The `test` job's checkout is **not**, and no guard in this repository can
/// be. Redirect it and this target compiles from the tree the `ref:` points
/// at, reads that tree's `ci.yml`, finds no `ref:` in it, and reports
/// `ok 173` -- the same self-reference that put the reporter guard in
/// `fidelity_matrix_tests` (SAK-249), except that moving it there does not help:
/// both reporters run in the `test` job, so one `ref:` takes them together.
///
/// Note it is also not closed by a required status check, which is the fix
/// SAK-242 is weighing for the neighbouring `if:`-on-the-job hole. A skipped
/// job and a redirected one differ here: the redirected job really runs and
/// really succeeds, so a required check is *satisfied*. What would close it is
/// a reader of this file hosted in a job with its own checkout -- the SAK-249
/// mutual arrangement one level up, at job rather than target granularity.
/// Left to SAK-242, which already owns the `test` job's other job-level hole,
/// rather than done here: it adds a step to `ci.yml`, which is a change to the
/// gate's own file and not to this file's reader.
#[test]
fn no_checkout_in_ci_yml_points_a_job_at_another_tree() {
    let steps = checkout_steps(&repo_file(".github/workflows/ci.yml"))
        .unwrap_or_else(|why| panic!("{why}"));
    if let Err(why) = check_no_checkout_redirects_the_tree(&steps) {
        panic!("{why}");
    }
}

#[cfg(test)]
mod checkout_scope_controls {
    use super::*;

    /// The real `ci.yml` with every checkout's `with:` block removed.
    ///
    /// Normalising first is what keeps each arm measuring the inputs *it*
    /// added, for the reason `ci_yml_base` gives: with the raw file as the
    /// base, a real `ref:` on the shipped checkout reddened **five** tests here
    /// instead of one, and the four extra failures were these controls reading
    /// a file carrying both their own inputs and the mutation. The test that
    /// reads the shipped file unnormalised, and so is the one that goes red
    /// when a redirect really lands there, is
    /// `no_checkout_in_ci_yml_points_a_job_at_another_tree`.
    ///
    /// The base has to lack what the arms add -- a `with:` block -- and these
    /// arms remove nothing, so stripping is the whole of it. SAK-245 needed
    /// both directions; this one does not.
    fn ci_yml_checkout_base() -> String {
        let ci = repo_file(".github/workflows/ci.yml");
        let lines: Vec<&str> = ci.lines().collect();
        let mut drop: Vec<usize> = Vec::new();
        for (at, line) in lines.iter().enumerate() {
            if !without_comment(line).contains(CHECKOUT_ACTION) {
                continue;
            }
            let mut root = at;
            while root > 0 && !without_comment(lines[root]).trim_start().starts_with("- ") {
                root -= 1;
            }
            let key_indent = indent_of(lines[root]) + 2;
            for (with_at, with_line) in block_under(&lines, root) {
                if indent_of(with_line) != key_indent
                    || without_comment(with_line).trim() != "with:"
                {
                    continue;
                }
                drop.push(with_at);
                drop.extend(block_under(&lines, with_at).iter().map(|(at, _)| *at));
            }
        }
        lines
            .iter()
            .enumerate()
            .filter(|(at, _)| !drop.contains(at))
            .map(|(_, line)| format!("{line}\n"))
            .collect()
    }

    /// `ci_yml_checkout_base()` with `inputs` given to the checkout of `job`.
    fn ci_yml_with_checkout_inputs(job: &str, inputs: &[&str]) -> String {
        let ci = ci_yml_checkout_base();
        let lines: Vec<&str> = ci.lines().collect();
        let job_at = lines
            .iter()
            .position(|line| without_comment(line).trim() == format!("{job}:"))
            .unwrap_or_else(|| panic!("ci.yml has no `{job}:` job"));
        let uses_at = (job_at..lines.len())
            .find(|at| without_comment(lines[*at]).contains(CHECKOUT_ACTION))
            .unwrap_or_else(|| panic!("the `{job}` job has no checkout step"));

        let mut out: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
        let mut block = vec!["        with:".to_string()];
        for input in inputs {
            block.push(format!("          {input}"));
        }
        out.splice(uses_at + 1..uses_at + 1, block);
        out.join("\n")
    }

    /// The reader itself, before anything is asserted about what it read.
    ///
    /// Only that the checkouts are **locatable**, and that it reads the inputs
    /// it is handed. Nothing here pins the shipped file's input list, for the
    /// reason `the_gate_condition_reader_finds_the_live_gate` gives: today
    /// every checkout declares no inputs at all, and a guard asserting that
    /// would redden on `persist-credentials: false`, a stricter tree.
    #[test]
    fn the_checkout_reader_finds_every_live_checkout() {
        let steps = checkout_steps(&repo_file(".github/workflows/ci.yml")).expect("readable");
        let jobs: Vec<&str> = steps.iter().map(|step| step.job.as_str()).collect();
        assert!(
            jobs.contains(&"deny") && jobs.contains(&"test"),
            "both jobs' checkouts must be found, read {jobs:?}"
        );

        let read = checkout_steps(&ci_yml_with_checkout_inputs(
            "deny",
            &["ref: 5d015dc", "fetch-depth: 0"],
        ))
        .expect("readable");
        let deny = read
            .iter()
            .find(|step| step.job == "deny")
            .expect("deny job");
        assert_eq!(
            deny.inputs,
            vec!["ref".to_string(), "fetch-depth".to_string()]
        );
    }

    /// The hole: a one-line input that leaves everything green while the gate
    /// and the guards read a tree nobody proposed.
    #[test]
    fn a_checkout_that_redirects_the_tree_fails() {
        for (job, inputs) in [
            ("deny", vec!["ref: 5d015dc"]),
            ("test", vec!["ref: 5d015dc"]),
            ("deny", vec!["repository: someone/else"]),
            ("test", vec!["repository: someone/else"]),
            // The same redirect one level out: the repository path is left
            // alone and the server it is fetched from moves.
            ("deny", vec!["github-server-url: https://ghe.example.com"]),
            ("test", vec!["github-server-url: https://ghe.example.com"]),
            // Quoting changes nothing: it is the key that selects the tree.
            ("deny", vec!["ref: 'main'"]),
            ("deny", vec!["ref: \"main\""]),
            // Hidden among inputs that are themselves fine.
            (
                "deny",
                vec![
                    "fetch-depth: 0",
                    "ref: 5d015dc",
                    "persist-credentials: false",
                ],
            ),
        ] {
            let steps =
                checkout_steps(&ci_yml_with_checkout_inputs(job, &inputs)).expect("readable");
            let why = check_no_checkout_redirects_the_tree(&steps)
                .expect_err(&format!("{inputs:?} on the `{job}` job redirects its tree"));
            assert!(
                why.contains(job),
                "the failure must name the job whose tree moved, said: {why}"
            );
            // And the evidence it quotes has to be the evidence for *this*
            // input. One sentence reused across the three would read as a
            // measurement of a key nobody measured.
            let redirect = inputs
                .iter()
                .find_map(|input| {
                    let key = input.split_once(':')?.0.trim();
                    CHECKOUT_REDIRECTING_INPUTS
                        .iter()
                        .find(|listed| **listed == key)
                })
                .expect("each arm carries one redirecting input");
            assert!(
                why.contains(*redirect),
                "the failure must name `{redirect}`, the input that moved the tree, \
                 said: {why}"
            );
        }
    }

    /// Each listed input is reported with its own measurement.
    ///
    /// The control for a template, rather than for the predicate: one
    /// `{input}` hole in one sentence would make the `ref:` numbers read as
    /// evidence for `repository:` and `github-server-url:` as well, and the
    /// reader of the failure would have no way to tell. So the three failures
    /// must differ by more than the key's name.
    #[test]
    fn each_redirecting_input_is_reported_with_its_own_evidence() {
        let mut said: Vec<String> = Vec::new();
        for input in CHECKOUT_REDIRECTING_INPUTS {
            let steps = checkout_steps(&ci_yml_with_checkout_inputs(
                "deny",
                &[&format!("{input}: whatever")],
            ))
            .expect("readable");
            let why = check_no_checkout_redirects_the_tree(&steps)
                .expect_err("a listed input redirects the tree");
            // Strip every key's name, so what is left is only the evidence.
            let evidence = CHECKOUT_REDIRECTING_INPUTS
                .iter()
                .fold(why.clone(), |acc, key| acc.replace(key, ""));
            assert!(
                !said.contains(&evidence),
                "`{input}:` is reported with evidence already used for another input, \
                 which would be quoting a measurement nobody took for it: {why}"
            );
            said.push(evidence);
        }
    }

    /// Inputs that are neutral, stricter, or fail-closed must pass.
    ///
    /// Without these arms the guard could be spelled "a checkout declares no
    /// `with:` at all", which would redden on `persist-credentials: false` --
    /// a tree safer than the one shipped, and the direction that teaches
    /// people to delete a guard rather than fix it.
    ///
    /// Every input `actions/checkout@v4` accepts and
    /// `CHECKOUT_REDIRECTING_INPUTS` does not list appears here, taken from
    /// actionlint's own rejection message rather than from the three the hole
    /// was reported with. That is how `github-server-url` turned out to belong
    /// on the other list, so the enumeration earns its length: an input missing
    /// from both lists is an input nobody decided about.
    #[test]
    fn inputs_that_do_not_move_the_tree_pass() {
        for inputs in [
            // Neutral or stricter.
            vec!["fetch-depth: 0"],
            vec!["fetch-tags: true"],
            vec!["filter: blob:none"],
            vec!["lfs: true"],
            vec!["clean: false"],
            vec!["show-progress: false"],
            vec!["set-safe-directory: false"],
            vec!["sparse-checkout-cone-mode: false"],
            vec!["submodules: false"],
            vec!["persist-credentials: false"],
            // The credential and the transport, not the tree: the same
            // repository at the same ref, fetched differently.
            vec!["token: ${{ secrets.OTHER }}"],
            vec!["ssh-key: ${{ secrets.KEY }}"],
            vec!["ssh-user: git"],
            vec!["ssh-known-hosts: example.com ssh-ed25519 AAAA"],
            vec!["ssh-strict: false"],
            // Fail-closed, not silent: both leave files the gate needs
            // missing. `path` puts the tree in a subdirectory while the step's
            // working directory does not move, which is the measured
            // no-manifest case (gate exit 4).
            vec!["path: sub"],
            vec!["sparse-checkout: src"],
            vec!["fetch-depth: 0", "persist-credentials: false"],
        ] {
            for job in ["deny", "test"] {
                let steps =
                    checkout_steps(&ci_yml_with_checkout_inputs(job, &inputs)).expect("readable");
                assert!(
                    check_no_checkout_redirects_the_tree(&steps).is_ok(),
                    "`{inputs:?}` on the `{job}` job does not choose a different tree and \
                     must not be reported as a hole"
                );
            }
        }
    }

    /// A commented-out redirect is not a redirect, and a trailing comment does
    /// not hide a live one.
    ///
    /// Which code this actually covers, because the first draft of this test
    /// claimed more than it tested. Deleting `checkout_steps`' own
    /// `without_comment` call on an input line left all 180 tests green, and
    /// the reason is that `block_under` has already dropped comment-only lines
    /// before the input loop sees them. So the comment-only arm below is a
    /// statement about `block_under`, not about this reader, and this reader's
    /// `without_comment` call only trims a comment *trailing* a real input --
    /// cosmetic, since the key sits before the first colon either way. That
    /// line is therefore deliberately left unguarded: its failure direction is
    /// to report a spurious extra input name, never to hide a real one, and a
    /// guard is worth writing only where the direction is fail-open (SAK-241's
    /// `without_comment` call sites, same question and same answer).
    #[test]
    fn a_commented_out_redirect_passes() {
        let steps = checkout_steps(&ci_yml_with_checkout_inputs(
            "deny",
            &["# ref: 5d015dc", "fetch-depth: 0"],
        ))
        .expect("readable");
        let deny = steps
            .iter()
            .find(|step| step.job == "deny")
            .expect("deny job");
        assert_eq!(deny.inputs, vec!["fetch-depth".to_string()]);
        assert!(check_no_checkout_redirects_the_tree(&steps).is_ok());

        // The other direction: a live redirect wearing a comment is still one.
        let dressed = checkout_steps(&ci_yml_with_checkout_inputs(
            "deny",
            &["ref: 5d015dc  # pinned deliberately"],
        ))
        .expect("readable");
        check_no_checkout_redirects_the_tree(&dressed)
            .expect_err("a trailing comment does not stop `ref:` choosing the tree");
    }

    /// The other spelling of a step, which a reader keyed on `- uses:` would
    /// have read as declaring nothing.
    #[test]
    fn a_named_checkout_step_is_read_too() {
        // Synthetic rather than a rewrite of the shipped file: this is a
        // statement about the reader, and an arm built by editing `ci.yml`
        // measured the edit as well as the reader -- it reddened a second time
        // when a real `ref:` landed on the shipped checkout.
        let ci = "\
name: CI
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

  deny:
    runs-on: ubuntu-latest
    steps:
      - name: Checkout
        uses: actions/checkout@v4
        with:
          ref: 5d015dc

      - name: cargo deny check
        run: cargo-deny --locked check advisories licenses bans sources
";
        let steps = checkout_steps(ci).expect("readable");
        let named = steps
            .iter()
            .find(|step| step.job == "deny")
            .expect("the `- name:` spelling must still be found as a checkout");
        assert_eq!(
            named.inputs,
            vec!["ref".to_string()],
            "`with:` sits in the same place in both spellings; a reader keyed on \
             `- uses:` read this step as declaring nothing"
        );
        let plain = steps
            .iter()
            .find(|step| step.job == "test")
            .expect("test job");
        assert_eq!(plain.inputs, Vec::<String>::new());
        check_no_checkout_redirects_the_tree(&steps)
            .expect_err("a redirect in the other step spelling is still a redirect");
    }

    /// Shapes this reader cannot evaluate fail closed.
    #[test]
    fn a_checkout_this_reader_cannot_find_fails_closed() {
        let ci = repo_file(".github/workflows/ci.yml");
        for (what, broken) in [
            (
                "no checkout at all",
                ci.replace(CHECKOUT_ACTION, "actions/nothing"),
            ),
            ("no jobs:", ci.replace("\njobs:\n", "\nnot-jobs:\n")),
        ] {
            checkout_steps(&broken)
                .expect_err(&format!("`{what}` must fail closed, not read as clean"));
        }
    }
}

/// The libtest arguments that narrow which tests in a target actually run.
///
/// Each was measured on this tree against a `deny.toml` widened so that one
/// assertion in *this* target fails -- these are cargo's and libtest's own
/// semantics, so they do the same to the sibling target this guard watches.
/// The shipped command exits 101 and reports it; each of these leaves the step
/// at **exit 0** (SAK-253):
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
/// Anchored on the command for the reason `sibling_reporter_conditions` gives:
/// renaming a
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
fn nothing_in_ci_yml_narrows_this_files_own_reporter_guard() {
    let ci = repo_file(".github/workflows/ci.yml");
    let reporter = match reporter_command(&ci, SIBLING_REPORTER_TARGET) {
        Ok(reporter) => reporter,
        Err(why) => panic!("{why}"),
    };
    if let Err(why) = check_the_reporter_runs_every_test(&reporter, SIBLING_REPORTER_TARGET) {
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
             run: cargo test --locked --test fidelity_matrix_tests{tail}\n"
        )
    }

    fn verdict(tail: &str) -> Result<(), String> {
        let ci = ci_running(tail);
        let reporter = reporter_command(&ci, SIBLING_REPORTER_TARGET).expect("located");
        check_the_reporter_runs_every_test(&reporter, SIBLING_REPORTER_TARGET)
    }

    #[test]
    fn the_shipped_reporter_command_is_read_and_passes() {
        let ci = repo_file(".github/workflows/ci.yml");
        let reporter = reporter_command(&ci, SIBLING_REPORTER_TARGET).expect("the real ci.yml");
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
                  fidelity_matrix_tests | tee out.log\n";
        let reporter = reporter_command(ci, SIBLING_REPORTER_TARGET).expect("located");
        assert!(reporter.pipefail);
        assert_eq!(
            check_the_reporter_runs_every_test(&reporter, SIBLING_REPORTER_TARGET),
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
        let ci = ci.replace("TARGET", SIBLING_REPORTER_TARGET);
        let reporter = reporter_command(&ci, SIBLING_REPORTER_TARGET).expect("located");
        assert!(reporter.pipefail);
        let error = check_the_reporter_runs_every_test(&reporter, SIBLING_REPORTER_TARGET)
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
                  run: |\n          cargo test --locked --test fidelity_matrix_tests \\\n          \
                  || true\n";
        let reporter = reporter_command(ci, SIBLING_REPORTER_TARGET).expect("located");
        assert!(check_the_reporter_runs_every_test(&reporter, SIBLING_REPORTER_TARGET).is_err());
    }

    #[test]
    fn an_unlocatable_command_fails_closed() {
        let gone = "jobs:\n  test:\n    steps:\n      - name: Unit tests\n        run: cargo \
                    test --locked --lib\n";
        assert!(reporter_command(gone, SIBLING_REPORTER_TARGET).is_err());

        let twice = format!(
            "{}      - name: Again\n        run: cargo test --locked --test \
             fidelity_matrix_tests\n",
            ci_running("")
        );
        assert!(reporter_command(&twice, SIBLING_REPORTER_TARGET).is_err());

        let commented = "jobs:\n  test:\n    steps:\n      - name: Pins\n        # run: cargo \
                         test --locked --test fidelity_matrix_tests\n        run: cargo test \
                         --locked --lib\n";
        assert!(reporter_command(commented, SIBLING_REPORTER_TARGET).is_err());
    }
}
