//! Talking to an OpenAI-compatible provider: `GET {base_url}/models`.

use std::{borrow::Cow, env, error::Error as StdError, time::Duration};

use reqwest::{StatusCode, blocking::Client};
use serde_json::Value;
use thiserror::Error;

const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("API key references environment variable `{0}`, which is not set")]
    EnvVarUnset(String),
    #[error("cannot reach {url}: {reason}")]
    Unreachable { url: String, reason: String },
    #[error("request to {url} timed out after {secs}s")]
    Timeout { url: String, secs: u64 },
    #[error("{url} rejected the API key (HTTP {status})")]
    Unauthorized { url: String, status: u16 },
    #[error("{url} returned 404; check the base URL (it usually ends with /v1)")]
    NotFound { url: String },
    #[error("{url} returned HTTP {status}")]
    Status { url: String, status: u16 },
    #[error("{url} did not return valid JSON")]
    NotJson { url: String },
    #[error("unexpected response from {url}: expected a JSON object with a `data` array of models")]
    BadShape { url: String },
    #[error("{url} returned an empty model list")]
    Empty { url: String },
}

/// Fetches the model ids served by the provider, sorted and deduplicated.
///
/// `base_url` must already be normalized (see [`crate::validate::base_url`]).
/// An empty list is an error so a sync can never wipe a provider by accident.
pub fn fetch_models(
    base_url: &str,
    api_key: Option<&str>,
    timeout: Duration,
) -> Result<Vec<String>, ProbeError> {
    let url = format!("{base_url}/models");
    let key = api_key.map(resolve_api_key).transpose()?;

    let client = Client::builder()
        .timeout(timeout)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| classify(&url, &e, timeout))?;

    let mut request = client.get(&url);
    if let Some(key) = key {
        request = request.bearer_auth(key);
    }
    let response = request.send().map_err(|e| classify(&url, &e, timeout))?;

    let status = response.status();
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            return Err(ProbeError::Unauthorized {
                url,
                status: status.as_u16(),
            });
        }
        StatusCode::NOT_FOUND => return Err(ProbeError::NotFound { url }),
        s if !s.is_success() => {
            return Err(ProbeError::Status {
                url,
                status: s.as_u16(),
            });
        }
        _ => {}
    }

    let body = response.bytes().map_err(|e| classify(&url, &e, timeout))?;
    parse_models(&url, &body)
}

/// Extracts model ids from an OpenAI-style `{"data": [{"id": "..."}]}` body.
fn parse_models(url: &str, body: &[u8]) -> Result<Vec<String>, ProbeError> {
    let bad_shape = || ProbeError::BadShape {
        url: url.to_owned(),
    };

    let json: Value = serde_json::from_slice(body).map_err(|_| ProbeError::NotJson {
        url: url.to_owned(),
    })?;
    let data = json
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(bad_shape)?;

    let mut ids = data
        .iter()
        .map(|model| {
            model
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(bad_shape)?;

    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Err(ProbeError::Empty {
            url: url.to_owned(),
        });
    }
    Ok(ids)
}

/// Resolves environment-variable references used by the agents' config formats:
/// `$VAR` / `${VAR}` (Crush) and `{env:VAR}` (Kilo). Anything else is a literal key.
fn resolve_api_key(raw: &str) -> Result<Cow<'_, str>, ProbeError> {
    let var = raw
        .strip_prefix("{env:")
        .and_then(|s| s.strip_suffix('}'))
        .or_else(|| raw.strip_prefix("${").and_then(|s| s.strip_suffix('}')))
        .or_else(|| raw.strip_prefix('$'));

    match var {
        Some(name) => env::var(name)
            .map(Cow::Owned)
            .map_err(|_| ProbeError::EnvVarUnset(name.to_owned())),
        None => Ok(Cow::Borrowed(raw)),
    }
}

fn classify(url: &str, err: &reqwest::Error, timeout: Duration) -> ProbeError {
    if err.is_timeout() {
        ProbeError::Timeout {
            url: url.to_owned(),
            secs: timeout.as_secs(),
        }
    } else {
        ProbeError::Unreachable {
            url: url.to_owned(),
            reason: root_cause(err),
        }
    }
}

/// The innermost error message ("Connection refused" instead of "error sending request").
fn root_cause(err: &dyn StdError) -> String {
    let mut current = err;
    while let Some(source) = current.source() {
        current = source;
    }
    current.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://x/v1/models";

    #[test]
    fn parses_sorted_unique_ids() {
        let body = br#"{"object":"list","data":[{"id":"b"},{"id":"a"},{"id":"b"}]}"#;
        assert_eq!(parse_models(URL, body).unwrap(), ["a", "b"]);
    }

    #[test]
    fn rejects_bad_bodies() {
        assert!(matches!(
            parse_models(URL, b"nope"),
            Err(ProbeError::NotJson { .. })
        ));
        assert!(matches!(
            parse_models(URL, b"{}"),
            Err(ProbeError::BadShape { .. })
        ));
        assert!(matches!(
            parse_models(URL, br#"{"data":[{"id":1}]}"#),
            Err(ProbeError::BadShape { .. })
        ));
        assert!(matches!(
            parse_models(URL, br#"{"data":[]}"#),
            Err(ProbeError::Empty { .. })
        ));
    }

    #[test]
    fn literal_keys_pass_through() {
        assert_eq!(resolve_api_key("sk-123").unwrap(), "sk-123");
    }

    #[test]
    fn unset_env_reference_is_an_error() {
        let err = resolve_api_key("{env:LLM_PROVIDER_SYNC_SURELY_UNSET}").unwrap_err();
        assert!(
            matches!(err, ProbeError::EnvVarUnset(name) if name == "LLM_PROVIDER_SYNC_SURELY_UNSET")
        );
    }
}
