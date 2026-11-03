//! PostgreSQL connection URL parsing.
//!
//! The CLI accepts a libpq-style connection URI and every component of it has to
//! reach the connection pool. An earlier version of this code validated the URL
//! scheme and then returned a hardcoded `localhost/fireup_test` config, so
//! `--postgres-url` had no effect at all. Parsing is therefore strict: anything
//! this module cannot honour is a hard error rather than a silently dropped
//! setting, because silently connecting somewhere other than the requested
//! target is the worst possible failure mode for a migration tool.

use std::collections::HashMap;
use std::time::Duration;

use tracing::debug;

use crate::data_importer::importer::ConnectionConfig;
use crate::error::FireupError;

const DEFAULT_PORT: u16 = 5432;
const DEFAULT_HOST: &str = "localhost";

/// Query parameters this build understands. Anything else is rejected.
const SUPPORTED_PARAMS: &[&str] = &[
    "host",
    "port",
    "dbname",
    "database",
    "user",
    "username",
    "password",
    "connect_timeout",
    "max_connections",
    "retry_attempts",
    "retry_delay",
    "sslmode",
];

const URL_FORMAT_HINT: &str = "postgresql://user:password@host:port/database";

fn config_error(message: impl Into<String>, suggestions: Vec<String>) -> FireupError {
    FireupError::Configuration {
        message: message.into(),
        config_key: Some("postgres_url".to_string()),
        context: FireupError::new_context("parse_connection_url"),
        suggestions,
    }
}

/// Decode `%XX` escapes in a URI component.
fn percent_decode(component: &str, field: &str) -> Result<String, FireupError> {
    let bytes = component.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = component.get(index + 1..index + 3).ok_or_else(|| {
                config_error(
                    format!("Truncated percent-escape in the {field} component of the PostgreSQL URL"),
                    vec![format!("Percent-encode reserved characters as %XX, e.g. {URL_FORMAT_HINT}")],
                )
            })?;
            let byte = u8::from_str_radix(hex, 16).map_err(|_| {
                config_error(
                    format!("Invalid percent-escape '%{hex}' in the {field} component of the PostgreSQL URL"),
                    vec!["Use two hexadecimal digits after '%'".to_string()],
                )
            })?;
            out.push(byte);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8(out).map_err(|_| {
        config_error(
            format!("The {field} component of the PostgreSQL URL is not valid UTF-8 after decoding"),
            vec!["Percent-encode non-ASCII characters as UTF-8 bytes".to_string()],
        )
    })
}

/// Split `host:port`, tolerating bracketed IPv6 literals.
fn split_host_port(authority: &str) -> Result<(String, Option<&str>), FireupError> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or_else(|| {
            config_error(
                "Unterminated IPv6 literal in the PostgreSQL URL host".to_string(),
                vec!["Write IPv6 hosts as postgresql://user@[::1]:5432/database".to_string()],
            )
        })?;
        let port = match after {
            "" => None,
            other => Some(other.strip_prefix(':').ok_or_else(|| {
                config_error(
                    format!("Unexpected text '{other}' after the IPv6 host in the PostgreSQL URL"),
                    vec![format!("Expected format: {URL_FORMAT_HINT}")],
                )
            })?),
        };
        return Ok((host.to_string(), port));
    }

    match authority.split_once(':') {
        Some((host, port)) => Ok((host.to_string(), Some(port))),
        None => Ok((authority.to_string(), None)),
    }
}

fn parse_port(raw: &str) -> Result<u16, FireupError> {
    if raw.is_empty() {
        return Ok(DEFAULT_PORT);
    }
    raw.parse::<u16>().map_err(|_| {
        config_error(
            format!("Invalid PostgreSQL port '{raw}'"),
            vec!["The port must be an integer between 1 and 65535".to_string()],
        )
    })
}

fn parse_usize_param(key: &str, raw: &str) -> Result<usize, FireupError> {
    raw.parse::<usize>().map_err(|_| {
        config_error(
            format!("Invalid value '{raw}' for connection parameter '{key}'"),
            vec![format!("'{key}' must be a non-negative integer")],
        )
    })
}

fn parse_u64_param(key: &str, raw: &str) -> Result<u64, FireupError> {
    raw.parse::<u64>().map_err(|_| {
        config_error(
            format!("Invalid value '{raw}' for connection parameter '{key}'"),
            vec![format!("'{key}' must be a non-negative integer number of seconds")],
        )
    })
}

fn parse_query(query: &str) -> Result<HashMap<String, String>, FireupError> {
    let mut params = HashMap::new();

    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(raw_key, "query parameter name")?.to_ascii_lowercase();
        let value = percent_decode(raw_value, "query parameter value")?;

        if !SUPPORTED_PARAMS.contains(&key.as_str()) {
            return Err(config_error(
                format!("Unsupported PostgreSQL connection parameter '{key}'"),
                vec![format!("Supported parameters: {}", SUPPORTED_PARAMS.join(", "))],
            ));
        }

        params.insert(key, value);
    }

    Ok(params)
}

/// Parse a PostgreSQL connection URI into a [`ConnectionConfig`].
///
/// `max_connections` and `timeout_secs` carry the CLI defaults; matching query
/// parameters in the URL take precedence over them. Credentials are never echoed
/// into error messages.
pub fn parse_connection_url(
    url: &str,
    max_connections: usize,
    timeout_secs: u64,
) -> Result<ConnectionConfig, FireupError> {
    let rest = url
        .strip_prefix("postgresql://")
        .or_else(|| url.strip_prefix("postgres://"))
        .ok_or_else(|| {
            config_error(
                format!("Invalid PostgreSQL URL format. Expected: {URL_FORMAT_HINT}"),
                vec![format!("Use format: {URL_FORMAT_HINT}")],
            )
        })?;

    let (location, query) = match rest.split_once('?') {
        Some((location, query)) => (location, query),
        None => (rest, ""),
    };

    let (authority, raw_database) = match location.split_once('/') {
        Some((authority, database)) => (authority, database),
        None => (location, ""),
    };

    // A password may legally contain an encoded '@', so split on the last one.
    let (raw_userinfo, raw_hostport) = match authority.rsplit_once('@') {
        Some((userinfo, hostport)) => (Some(userinfo), hostport),
        None => (None, authority),
    };

    let (mut user, mut password) = match raw_userinfo {
        Some(userinfo) => {
            let (raw_user, raw_password) = match userinfo.split_once(':') {
                Some((user, password)) => (user, Some(password)),
                None => (userinfo, None),
            };
            let user = percent_decode(raw_user, "user")?;
            let password = match raw_password {
                Some(password) => Some(percent_decode(password, "password")?),
                None => None,
            };
            (user, password)
        }
        None => (String::new(), None),
    };

    let (raw_host, raw_port) = split_host_port(raw_hostport)?;
    let mut host = percent_decode(&raw_host, "host")?;
    let mut port = parse_port(raw_port.unwrap_or(""))?;
    let mut database = percent_decode(raw_database, "database")?;

    let mut max_connections = max_connections;
    let mut connection_timeout = Duration::from_secs(timeout_secs);
    let mut retry_attempts = 3u32;
    let mut retry_delay = Duration::from_secs(1);

    let params = parse_query(query)?;

    for (key, value) in &params {
        match key.as_str() {
            "host" => host = value.clone(),
            "port" => port = parse_port(value)?,
            "dbname" | "database" => database = value.clone(),
            "user" | "username" => user = value.clone(),
            "password" => password = Some(value.clone()),
            "connect_timeout" => connection_timeout = Duration::from_secs(parse_u64_param(key, value)?),
            "max_connections" => max_connections = parse_usize_param(key, value)?,
            "retry_attempts" => retry_attempts = parse_usize_param(key, value)? as u32,
            "retry_delay" => retry_delay = Duration::from_secs(parse_u64_param(key, value)?),
            "sslmode" => {
                // The pool is built with NoTls, so honouring a TLS request is
                // impossible. Failing loudly beats connecting in the clear.
                if !matches!(value.to_ascii_lowercase().as_str(), "disable" | "prefer" | "allow") {
                    return Err(config_error(
                        format!("sslmode='{value}' is not supported by this build"),
                        vec![
                            "This build connects without TLS; use sslmode=disable".to_string(),
                            "Tunnel the connection (e.g. cloud-sql-proxy, stunnel) if TLS is required".to_string(),
                        ],
                    ));
                }
            }
            _ => unreachable!("parse_query rejects unsupported parameters"),
        }
    }

    if host.is_empty() {
        host = DEFAULT_HOST.to_string();
    }

    if user.is_empty() {
        user = std::env::var("PGUSER").unwrap_or_default();
    }
    if user.is_empty() {
        return Err(config_error(
            "The PostgreSQL URL does not specify a user".to_string(),
            vec![
                format!("Use format: {URL_FORMAT_HINT}"),
                "Or set the PGUSER environment variable".to_string(),
            ],
        ));
    }

    if database.is_empty() {
        return Err(config_error(
            "The PostgreSQL URL does not specify a database".to_string(),
            vec![format!("Use format: {URL_FORMAT_HINT}")],
        ));
    }

    if max_connections == 0 {
        return Err(config_error(
            "max_connections must be at least 1".to_string(),
            vec!["Pass --max-connections 1 or higher".to_string()],
        ));
    }

    // Password-less URLs fall back to PGPASSWORD so credentials never have to be
    // written into a command line or a source file.
    let password = match password {
        Some(password) => password,
        None => std::env::var("PGPASSWORD").unwrap_or_default(),
    };

    debug!(
        "Parsed PostgreSQL URL: host={}, port={}, database={}, user={}, max_connections={}",
        host, port, database, user, max_connections
    );

    Ok(ConnectionConfig {
        host,
        port,
        database,
        user,
        password,
        max_connections,
        connection_timeout,
        retry_attempts,
        retry_delay,
    })
}
