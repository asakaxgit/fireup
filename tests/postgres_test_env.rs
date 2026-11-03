//! Guards integration tests from touching the host Paperclip Postgres cluster.

fn postgres_url_user(url: &str) -> Option<&str> {
    let after_scheme = url.split("://").nth(1)?;
    let credentials = after_scheme.rsplit('@').next()?.rsplit('@').next()?;
    credentials.split(':').next()
}

fn postgres_url_database(url: &str) -> Option<&str> {
    let path = url.split("://").nth(1)?.split('@').last()?;
    let db = path.split('/').nth(1)?.split('?').next()?;
    Some(db)
}

/// Returns true when the URL uses the Paperclip application role or database.
pub fn is_forbidden_paperclip_postgres_url(url: &str) -> bool {
    if postgres_url_user(url)
        .is_some_and(|user| user.eq_ignore_ascii_case("paperclip"))
    {
        return true;
    }

    postgres_url_database(url)
        .is_some_and(|db| db.eq_ignore_ascii_case("paperclip"))
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

    if is_forbidden_paperclip_postgres_url(&url) {
        eprintln!(
            "Refusing Postgres integration tests: URL must not use the `paperclip` role or `paperclip` database."
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
        "Skipping {test_name}: set FIREUP_TEST_POSTGRES_URL or TEST_DATABASE_URL to a dedicated fireup test database (Docker compose on port 5433 is preferred). Do not use the Paperclip role or database."
    );
    true
}

#[cfg(test)]
mod tests {
    use super::is_forbidden_paperclip_postgres_url;

    #[test]
    fn rejects_paperclip_role_and_database() {
        assert!(is_forbidden_paperclip_postgres_url(
            "postgresql://paperclip:secret@127.0.0.1:5432/paperclip"
        ));
        assert!(is_forbidden_paperclip_postgres_url(
            "postgresql://fireup:secret@127.0.0.1:5433/paperclip"
        ));
        assert!(!is_forbidden_paperclip_postgres_url(
            "postgresql://fireup:fireup_dev_password@localhost:5433/fireup_test"
        ));
    }
}
