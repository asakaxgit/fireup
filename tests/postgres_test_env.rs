//! Guards integration tests from touching a Postgres role or database the
//! machine owner has put off limits.
//!
//! `FIREUP_TEST_POSTGRES_DENY` holds a comma-separated list of names. A test URL
//! whose role or database matches one of them (case-insensitively) is refused.
//! Set it once for the machine, e.g. in `~/.cargo/config.toml`:
//!
//! ```toml
//! [env]
//! FIREUP_TEST_POSTGRES_DENY = "my_app,my_app_admin"
//! ```

use fireup::data_importer::parse_connection_url;

/// Env var naming the roles and databases integration tests must never use.
pub const DENY_ENV: &str = "FIREUP_TEST_POSTGRES_DENY";

/// Splits a comma-separated deny list, dropping blanks.
pub fn parse_deny_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// The deny list from [`DENY_ENV`]; empty when unset.
pub fn denied_postgres_names() -> Vec<String> {
    std::env::var(DENY_ENV)
        .map(|raw| parse_deny_list(&raw))
        .unwrap_or_default()
}

/// Returns the denied name when the URL would connect as a denied role or to a
/// denied database. Reads the URL with the same parser the stores connect with,
/// so query-parameter overrides and the `PGUSER` fallback are seen too. A URL
/// that parser rejects is refused outright.
pub fn denied_postgres_url_match(url: &str, denied: &[String]) -> Option<String> {
    if denied.is_empty() {
        return None;
    }
    let Ok(config) = parse_connection_url(url, 1, 10) else {
        return Some("<unparseable URL>".to_string());
    };
    denied
        .iter()
        .find(|name| {
            config.user.eq_ignore_ascii_case(name) || config.database.eq_ignore_ascii_case(name)
        })
        .cloned()
}

/// Dedicated fireup test URL from env. Never falls back to host Postgres.
pub fn resolve_integration_postgres_url() -> Option<String> {
    let url = std::env::var("FIREUP_TEST_POSTGRES_URL")
        .ok()
        .or_else(|| std::env::var("TEST_DATABASE_URL").ok())?
        .trim()
        .to_string();

    if url.is_empty() {
        return None;
    }

    if let Some(name) = denied_postgres_url_match(&url, &denied_postgres_names()) {
        eprintln!(
            "Refusing Postgres integration tests: the URL's role or database matches `{name}` from {DENY_ENV}."
        );
        return None;
    }

    Some(url)
}

pub fn integration_postgres_tests_enabled() -> bool {
    resolve_integration_postgres_url().is_some()
}

pub fn skip_postgres_integration_test(test_name: &str) -> bool {
    if integration_postgres_tests_enabled() {
        return false;
    }
    println!(
        "Skipping {test_name}: set FIREUP_TEST_POSTGRES_URL or TEST_DATABASE_URL to a dedicated fireup test database (Docker compose on port 5433 is preferred), and keep it clear of the names in {DENY_ENV}."
    );
    true
}

#[cfg(test)]
mod tests {
    use super::{denied_postgres_url_match, parse_deny_list};

    #[test]
    fn refuses_a_denied_role_or_database() {
        let denied = parse_deny_list(" host_app , ,host_admin");
        assert_eq!(denied, ["host_app", "host_admin"]);

        let refused = |url: &str| denied_postgres_url_match(url, &denied);
        assert_eq!(
            refused("postgresql://host_app:secret@127.0.0.1:5432/fireup_test").as_deref(),
            Some("host_app"),
            "denied role in the userinfo"
        );
        assert_eq!(
            refused("postgresql://fireup:secret@127.0.0.1:5433/HOST_APP").as_deref(),
            Some("host_app"),
            "denied database, case-insensitively"
        );
        assert_eq!(
            refused("postgresql://fireup:secret@127.0.0.1:5433/fireup_test?user=host_admin")
                .as_deref(),
            Some("host_admin"),
            "denied role via a query-parameter override"
        );
        assert_eq!(
            refused("postgresql://fireup:secret@127.0.0.1:5433/fireup_test?dbname=host_app")
                .as_deref(),
            Some("host_app"),
            "denied database via a query-parameter override"
        );
        assert!(
            refused("mysql://fireup@127.0.0.1/fireup_test").is_some(),
            "a URL the store parser rejects is refused"
        );
        for ambiguous in [
            "postgresql://fireup:secret@127.0.0.1:5433/fireup_test?dbname=fireup_test&database=host_app",
            "postgresql://fireup:secret@127.0.0.1:5433/fireup_test?user=fireup&username=host_app",
        ] {
            assert!(refused(ambiguous).is_some(), "two spellings of one setting: {ambiguous}");
        }
        assert_eq!(
            refused("postgresql://fireup:fireup_dev_password@localhost:5433/fireup_test"),
            None,
            "a dedicated test URL passes"
        );
    }

    #[test]
    fn an_empty_deny_list_refuses_nothing() {
        assert_eq!(parse_deny_list(""), Vec::<String>::new());
        assert_eq!(
            denied_postgres_url_match("postgresql://host_app:secret@127.0.0.1:5432/host_app", &[]),
            None
        );
    }
}
