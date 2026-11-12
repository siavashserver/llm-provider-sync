//! Static validation of user-supplied provider parameters.
//!
//! Reachability of the endpoint is checked separately by [`crate::probe`].

use url::Url;

use crate::error::Error;

const MAX_ID_LEN: usize = 64;

/// Words that clap would parse as commands instead of provider ids.
const RESERVED_IDS: &[&str] = &[
    "list", "ls", "sync-all", "agents", "help", "show", "info", "add", "set", "sync", "remove",
    "rm", "delete",
];

/// Validates a provider id and returns it trimmed.
///
/// Allowed: ASCII letters, digits, `.`, `_`, `-`. Notably no `/` or `#`, which
/// Kilo uses as separators in `provider/model` references.
pub fn provider_id(raw: &str) -> Result<String, Error> {
    let id = raw.trim();
    if id.is_empty() {
        return Err(Error::InvalidId("must not be empty".into()));
    }
    if id.len() > MAX_ID_LEN {
        return Err(Error::InvalidId(format!(
            "must be at most {MAX_ID_LEN} characters"
        )));
    }
    if let Some(bad) = id
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
    {
        return Err(Error::InvalidId(format!(
            "`{bad}` is not allowed; use letters, digits, `.`, `_` or `-`"
        )));
    }
    if RESERVED_IDS.contains(&id) {
        return Err(Error::InvalidId(format!("`{id}` is a reserved word")));
    }
    Ok(id.to_owned())
}

/// Validates a base URL and returns it normalized (no trailing slash).
pub fn base_url(raw: &str) -> Result<String, Error> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(Error::InvalidUrl("must not be empty".into()));
    }
    let url = Url::parse(raw).map_err(|e| Error::InvalidUrl(e.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::InvalidUrl("scheme must be http or https".into()));
    }
    if url.host_str().is_none() {
        return Err(Error::InvalidUrl("missing host".into()));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(Error::InvalidUrl(
            "must not contain a query or fragment".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::InvalidUrl("must not contain credentials".into()));
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_ids_and_trims() {
        assert_eq!(provider_id("  lm-studio.1 ").unwrap(), "lm-studio.1");
    }

    #[test]
    fn rejects_bad_ids() {
        for bad in [
            "",
            "  ",
            "a/b",
            "a#b",
            "has space",
            "list",
            "sync-all",
            "add",
        ] {
            assert!(provider_id(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(provider_id(&"x".repeat(65)).is_err());
    }

    #[test]
    fn normalizes_urls() {
        assert_eq!(
            base_url("http://localhost:1234/v1/").unwrap(),
            "http://localhost:1234/v1"
        );
        assert_eq!(
            base_url("https://example.com").unwrap(),
            "https://example.com"
        );
    }

    #[test]
    fn rejects_bad_urls() {
        for bad in [
            "",
            "localhost:1234/v1",
            "ftp://example.com/v1",
            "http://example.com/v1?x=1",
            "http://example.com/v1#frag",
            "http://user:pw@example.com/v1",
        ] {
            assert!(base_url(bad).is_err(), "{bad:?} should be rejected");
        }
    }
}
