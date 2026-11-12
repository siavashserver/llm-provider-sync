//! Crush (https://charm.land/crush.json).
//!
//! Providers live under `providers.<id>`; OpenAI-compatible ones have
//! `"type": "openai-compat"`. `models` is an *array* of objects with many
//! required fields, but `/models` only returns ids, so new entries get the
//! defaults below.

use std::path::PathBuf;

use jsonc_parser::cst::{CstInputValue, CstObject};
use serde_json::Value;

use super::{Agent, Provider, str_field};
use crate::config::{remove_field, set_field, text};

pub struct Crush;

const COMPAT_TYPE: &str = "openai-compat";

/// `/models` does not report these, so they are fixed placeholders users can edit afterwards.
const DEFAULT_CONTEXT_WINDOW: u64 = 128_000;
const DEFAULT_MAX_TOKENS: u64 = 4_096;

impl Agent for Crush {
    fn name(&self) -> &'static str {
        "crush"
    }

    fn config_candidates(&self) -> Vec<PathBuf> {
        config_dir()
            .map(|dir| vec![dir.join("crush.json")])
            .unwrap_or_default()
    }

    fn providers_key(&self) -> &'static str {
        "providers"
    }

    fn read_provider(&self, id: &str, entry: &Value) -> Option<Provider> {
        if str_field(entry, "type")? != COMPAT_TYPE {
            return None;
        }
        Some(Provider {
            id: id.to_owned(),
            base_url: str_field(entry, "base_url").unwrap_or_default().to_owned(),
            api_key: str_field(entry, "api_key").map(str::to_owned),
            models: entry
                .get("models")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|model| str_field(model, "id"))
                .map(str::to_owned)
                .collect(),
        })
    }

    fn write_provider(&self, entry: &CstObject, provider: &Provider, models: Option<&[String]>) {
        set_field(entry, "id", text(&provider.id));
        set_field(entry, "type", text(COMPAT_TYPE));
        set_field(entry, "base_url", text(&provider.base_url));
        match &provider.api_key {
            Some(key) => set_field(entry, "api_key", text(key)),
            None => remove_field(entry, "api_key"),
        }
        if let Some(ids) = models {
            set_field(
                entry,
                "models",
                CstInputValue::Array(ids.iter().map(|id| model(id)).collect()),
            );
        }
    }

    /// `models.<large|small|...>.provider` holds the selected provider id.
    fn reference_paths(&self, config: &Value) -> Vec<Vec<String>> {
        config
            .get("models")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|selected| selected.keys())
            .map(|key| vec!["models".to_owned(), key.clone(), "provider".to_owned()])
            .collect()
    }
}

/// Crush's global config directory: `%LOCALAPPDATA%\crush` on Windows, `$XDG_CONFIG_HOME/crush` elsewhere.
#[cfg(windows)]
fn config_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.data_local_dir().join("crush"))
}

#[cfg(not(windows))]
fn config_dir() -> Option<PathBuf> {
    super::config_home().map(|dir| dir.join("crush"))
}

fn model(id: &str) -> CstInputValue {
    let number = |n: u64| CstInputValue::Number(n.to_string());
    CstInputValue::Object(vec![
        ("id".to_owned(), text(id)),
        ("name".to_owned(), text(id)),
        ("cost_per_1m_in".to_owned(), number(0)),
        ("cost_per_1m_out".to_owned(), number(0)),
        ("cost_per_1m_in_cached".to_owned(), number(0)),
        ("cost_per_1m_out_cached".to_owned(), number(0)),
        ("context_window".to_owned(), number(DEFAULT_CONTEXT_WINDOW)),
        ("default_max_tokens".to_owned(), number(DEFAULT_MAX_TOKENS)),
        ("can_reason".to_owned(), CstInputValue::Bool(false)),
        (
            "supports_attachments".to_owned(),
            CstInputValue::Bool(false),
        ),
    ])
}
