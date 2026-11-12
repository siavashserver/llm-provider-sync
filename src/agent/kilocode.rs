//! Kilo Code (https://app.kilo.ai/config.json, an opencode-style config).
//!
//! Providers live under `provider.<id>` (the deprecated-but-documented block that
//! carries `options.baseURL` / `options.apiKey`). OpenAI-compatible ones use the
//! `@ai-sdk/openai-compatible` package. `models` is a *map* keyed by model id.
//! Entries under the newer `providers` block are not touched.

use std::path::PathBuf;

use jsonc_parser::cst::{CstInputValue, CstObject};
use serde_json::Value;

use super::{Agent, Provider, config_home, str_field};
use crate::config::{remove_field, set_field, text};

pub struct Kilocode;

const COMPAT_PACKAGE: &str = "@ai-sdk/openai-compatible";

/// Global config file names, in priority order.
const CONFIG_FILES: &[&str] = &["kilo.jsonc", "kilo.json", "opencode.jsonc", "opencode.json"];

impl Agent for Kilocode {
    fn name(&self) -> &'static str {
        "kilocode"
    }

    fn config_candidates(&self) -> Vec<PathBuf> {
        config_home()
            .map(|dir| {
                CONFIG_FILES
                    .iter()
                    .map(|file| dir.join("kilo").join(file))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn providers_key(&self) -> &'static str {
        "provider"
    }

    fn read_provider(&self, id: &str, entry: &Value) -> Option<Provider> {
        if str_field(entry, "npm")? != COMPAT_PACKAGE {
            return None;
        }
        let options = entry.get("options");
        Some(Provider {
            id: id.to_owned(),
            base_url: options
                .and_then(|o| str_field(o, "baseURL"))
                .unwrap_or_default()
                .to_owned(),
            api_key: options
                .and_then(|o| str_field(o, "apiKey"))
                .map(str::to_owned),
            models: entry
                .get("models")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|models| models.keys())
                .cloned()
                .collect(),
        })
    }

    fn write_provider(&self, entry: &CstObject, provider: &Provider, models: Option<&[String]>) {
        set_field(entry, "npm", text(COMPAT_PACKAGE));

        let options = entry.object_value_or_set("options");
        set_field(&options, "baseURL", text(&provider.base_url));
        match &provider.api_key {
            Some(key) => set_field(&options, "apiKey", text(key)),
            None => remove_field(&options, "apiKey"),
        }

        if let Some(ids) = models {
            let entries = ids
                .iter()
                .map(|id| {
                    (
                        id.clone(),
                        CstInputValue::Object(vec![("name".to_owned(), text(id))]),
                    )
                })
                .collect();
            set_field(entry, "models", CstInputValue::Object(entries));
        }
    }

    /// Model selections are `provider/model` strings: `model`, `small_model` and per-agent `model`.
    fn reference_paths(&self, config: &Value) -> Vec<Vec<String>> {
        let mut paths = vec![vec!["model".to_owned()], vec!["small_model".to_owned()]];
        paths.extend(
            config
                .get("agent")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|agents| agents.keys())
                .map(|key| vec!["agent".to_owned(), key.clone(), "model".to_owned()]),
        );
        paths
    }
}
