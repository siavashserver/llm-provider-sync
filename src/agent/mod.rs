//! Per-agent knowledge: where the config lives and how a provider is laid out in it.
//!
//! Everything agent-independent (loading, editing, saving, rename and reference
//! handling) lives in [`crate::config`]; adapters only describe their own format.

mod crush;
mod kilocode;

use std::{env, path::PathBuf};

use directories::BaseDirs;
use jsonc_parser::cst::CstObject;
use serde_json::Value;

/// An OpenAI-compatible provider as stored in an agent's config.
#[derive(Debug, Clone)]
pub struct Provider {
    pub id: String,
    pub base_url: String,
    /// Stored verbatim; may be an environment reference such as `$KEY` or `{env:KEY}`.
    pub api_key: Option<String>,
    /// Model ids currently in the config.
    pub models: Vec<String>,
}

/// The agents we can manage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    Kilocode,
    Crush,
}

impl AgentKind {
    pub const ALL: [Self; 2] = [Self::Kilocode, Self::Crush];

    pub fn adapter(self) -> &'static dyn Agent {
        match self {
            Self::Kilocode => &kilocode::Kilocode,
            Self::Crush => &crush::Crush,
        }
    }

    pub fn name(self) -> &'static str {
        self.adapter().name()
    }
}

/// Describes one agent's config format.
pub trait Agent {
    fn name(&self) -> &'static str;

    /// Possible global config locations, in priority order.
    fn config_candidates(&self) -> Vec<PathBuf>;

    /// Top-level key holding the provider map.
    fn providers_key(&self) -> &'static str;

    /// Parses a provider entry. `None` if it is not an OpenAI-compatible provider.
    fn read_provider(&self, id: &str, entry: &Value) -> Option<Provider>;

    /// Writes the provider's managed fields into `entry`, leaving all other fields alone.
    /// `models`: `Some` replaces the model list, `None` leaves it untouched.
    fn write_provider(&self, entry: &CstObject, provider: &Provider, models: Option<&[String]>);

    /// Paths of string settings that may reference a provider (by id or `id/model`).
    fn reference_paths(&self, config: &Value) -> Vec<Vec<String>>;
}

/// `$XDG_CONFIG_HOME` or `~/.config`. These tools use this location on every OS
/// except where an adapter says otherwise.
fn config_home() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| BaseDirs::new().map(|dirs| dirs.home_dir().join(".config")))
}

fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}
