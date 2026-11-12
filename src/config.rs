//! Reading and editing an agent's JSON/JSONC config file.
//!
//! Reads go through a plain `serde_json::Value`. Writes go through the
//! `jsonc_parser` CST, so comments, key order, formatting and every key we do
//! not own survive an edit.
//!
//! A [`ConfigDoc`] is meant to be used once: `load`, apply edits, `save`. The
//! read-side `value` is a snapshot taken at load time and is not refreshed by
//! edits; reload the document if you need to read again.

use std::{
    fmt::Display,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use jsonc_parser::{
    ParseOptions,
    cst::{CstInputValue, CstObject, CstRootNode},
    parse_to_serde_value,
};
use serde_json::{Map, Value};

use crate::{
    agent::{AgentKind, Provider},
    error::Error,
};

pub struct ConfigDoc {
    kind: AgentKind,
    path: PathBuf,
    /// Original file contents, used to skip the write when nothing changed.
    text: String,
    /// Read-side snapshot.
    value: Value,
    /// Write-side syntax tree.
    root: CstRootNode,
}

/// Finds the config file: `explicit` if given, otherwise the first existing
/// candidate of the agent. Symlinks are resolved so saving writes through them.
pub fn locate(kind: AgentKind, explicit: Option<&Path>) -> Result<PathBuf, Error> {
    let candidates = match explicit {
        Some(path) => vec![path.to_path_buf()],
        None => kind.adapter().config_candidates(),
    };
    let found = candidates
        .iter()
        .find(|path| path.is_file())
        .ok_or_else(|| Error::ConfigNotFound {
            agent: kind.name(),
            searched: candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        })?;
    Ok(resolve_symlink(found))
}

fn resolve_symlink(path: &Path) -> PathBuf {
    let is_link = fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink());
    if is_link {
        fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    } else {
        path.to_path_buf()
    }
}

fn parse_error(path: &Path, err: impl Display) -> Error {
    Error::Parse {
        path: path.to_path_buf(),
        message: err.to_string(),
    }
}

impl ConfigDoc {
    pub fn load(kind: AgentKind, explicit: Option<&Path>) -> Result<Self, Error> {
        let path = locate(kind, explicit)?;
        let text = fs::read_to_string(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;

        let options = ParseOptions::default(); // comments and trailing commas allowed
        let mut value =
            parse_to_serde_value::<Value>(&text, &options).map_err(|e| parse_error(&path, e))?;
        if value.is_null() {
            value = Value::Object(Map::new()); // empty file
        }
        if !value.is_object() {
            return Err(Error::NotObject(path));
        }
        let root = CstRootNode::parse(&text, &options).map_err(|e| parse_error(&path, e))?;

        Ok(Self {
            kind,
            path,
            text,
            value,
            root,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    // ---- reads --------------------------------------------------------------------------------

    fn providers_map(&self) -> Option<&Map<String, Value>> {
        self.value
            .get(self.kind.adapter().providers_key())?
            .as_object()
    }

    /// All OpenAI-compatible providers, sorted by id. Other provider types are ignored.
    pub fn providers(&self) -> Vec<Provider> {
        let agent = self.kind.adapter();
        let mut providers: Vec<_> = self
            .providers_map()
            .into_iter()
            .flat_map(|map| map.iter())
            .filter_map(|(id, entry)| agent.read_provider(id, entry))
            .collect();
        providers.sort_by(|a, b| a.id.cmp(&b.id));
        providers
    }

    /// Whether *any* provider entry (compatible or not) uses this id.
    pub fn contains(&self, id: &str) -> bool {
        self.providers_map().is_some_and(|map| map.contains_key(id))
    }

    pub fn provider(&self, id: &str) -> Result<Provider, Error> {
        let entry = self
            .providers_map()
            .and_then(|map| map.get(id))
            .ok_or_else(|| Error::ProviderNotFound(id.to_owned()))?;
        self.kind
            .adapter()
            .read_provider(id, entry)
            .ok_or_else(|| Error::NotCompatible(id.to_owned()))
    }

    /// Number of settings (selected models, agent overrides) that point at this provider.
    pub fn reference_count(&self, id: &str) -> usize {
        self.reference_updates(id, id).len()
    }

    /// `(path, new value)` for every string setting that references `old`.
    fn reference_updates(&self, old: &str, new: &str) -> Vec<(Vec<String>, String)> {
        self.kind
            .adapter()
            .reference_paths(&self.value)
            .into_iter()
            .filter_map(|path| {
                let current = string_at(&self.value, &path)?;
                retarget(current, old, new).map(|updated| (path, updated))
            })
            .collect()
    }

    // ---- writes -------------------------------------------------------------------------------

    fn providers_object(&self) -> CstObject {
        self.root
            .object_value_or_set()
            .object_value_or_set(self.kind.adapter().providers_key())
    }

    /// Creates or updates a provider. `models`: `Some` replaces the model list, `None` leaves it alone.
    pub fn upsert(&mut self, provider: &Provider, models: Option<&[String]>) {
        let entry = self.providers_object().object_value_or_set(&provider.id);
        self.kind.adapter().write_provider(&entry, provider, models);
    }

    pub fn remove(&mut self, id: &str) -> Result<(), Error> {
        self.provider(id)?;
        if let Some(prop) = self.providers_object().get(id) {
            prop.remove();
        }
        Ok(())
    }

    /// Renames a provider in place, keeping every field we don't manage, and
    /// rewrites settings that reference it. Returns the number of references updated.
    pub fn rename(&mut self, old: &str, new: &str) -> Result<usize, Error> {
        let mut provider = self.provider(old)?;
        if self.contains(new) {
            return Err(Error::ProviderExists(new.to_owned()));
        }
        let raw = self
            .providers_map()
            .and_then(|map| map.get(old))
            .cloned()
            .unwrap_or(Value::Null);
        let updates = self.reference_updates(old, new);

        let providers = self.providers_object();
        if let Some(prop) = providers.get(old) {
            prop.remove();
        }
        providers.append(new, to_input(&raw));

        // Refresh managed fields that embed the id (e.g. Crush's `id`).
        provider.id = new.to_owned();
        self.upsert(&provider, None);

        for (path, value) in &updates {
            set_string_at(&self.root, path, value);
        }
        Ok(updates.len())
    }

    /// Writes the file atomically. Returns `false` (and touches nothing) if the content is unchanged.
    pub fn save(self) -> Result<bool, Error> {
        let new_text = self.root.to_string();
        if new_text == self.text {
            return Ok(false);
        }
        write_atomic(&self.path, &new_text)?;
        Ok(true)
    }
}

// ---- CST helpers shared with the agent adapters -------------------------------------------------

pub(crate) fn text(value: &str) -> CstInputValue {
    CstInputValue::String(value.to_owned())
}

/// Sets `name` on `object`, replacing the value if the property already exists.
pub(crate) fn set_field(object: &CstObject, name: &str, value: CstInputValue) {
    match object.get(name) {
        Some(prop) => {
            prop.set_value(value);
        }
        None => {
            object.append(name, value);
        }
    }
}

pub(crate) fn remove_field(object: &CstObject, name: &str) {
    if let Some(prop) = object.get(name) {
        prop.remove();
    }
}

fn to_input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(b) => CstInputValue::Bool(*b),
        Value::Number(n) => CstInputValue::Number(n.to_string()),
        Value::String(s) => CstInputValue::String(s.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(to_input).collect()),
        Value::Object(map) => {
            CstInputValue::Object(map.iter().map(|(k, v)| (k.clone(), to_input(v))).collect())
        }
    }
}

// ---- reference rewriting -----------------------------------------------------------------------

/// Maps a reference to provider `old` onto `new`. Handles both a bare provider
/// id (Crush) and a `provider/model` string (Kilo).
fn retarget(current: &str, old: &str, new: &str) -> Option<String> {
    if current == old {
        return Some(new.to_owned());
    }
    let model = current.strip_prefix(old)?.strip_prefix('/')?;
    Some(format!("{new}/{model}"))
}

fn string_at<'a>(value: &'a Value, path: &[String]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |v, key| v.get(key.as_str()))?
        .as_str()
}

fn set_string_at(root: &CstRootNode, path: &[String], value: &str) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let Some(mut object) = root.object_value() else {
        return;
    };
    for key in parents {
        match object.object_value(key) {
            Some(next) => object = next,
            None => return,
        }
    }
    if let Some(prop) = object.get(last) {
        prop.set_value(text(value));
    }
}

// ---- file I/O ------------------------------------------------------------------------------------

/// Writes via a temp file in the same directory and renames it into place.
/// The original file's permissions are applied *before* any content is written,
/// because config files hold API keys.
fn write_atomic(path: &Path, contents: &str) -> Result<(), Error> {
    let io_err = |source: io::Error| Error::Io {
        path: path.to_path_buf(),
        source,
    };

    let file_name = path
        .file_name()
        .map_or_else(|| "config".into(), |n| n.to_string_lossy());
    let tmp = path.with_file_name(format!(".{file_name}.tmp"));

    let write = || -> io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        if let Ok(meta) = fs::metadata(path) {
            file.set_permissions(meta.permissions())?;
        }
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    };
    write().map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io_err(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(name: &str, contents: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("llm-provider-sync-{}-{name}", std::process::id()));
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn retarget_handles_ids_and_model_refs() {
        assert_eq!(retarget("lm", "lm", "x").as_deref(), Some("x"));
        assert_eq!(retarget("lm/qwen", "lm", "x").as_deref(), Some("x/qwen"));
        assert_eq!(retarget("lmx/qwen", "lm", "x"), None);
        assert_eq!(retarget("other", "lm", "x"), None);
    }

    #[test]
    fn upsert_replaces_models_and_keeps_comments() {
        let path = temp_config(
            "crush.json",
            "// keep me\n{\"options\": {\"debug\": true}}\n",
        );
        let mut doc = ConfigDoc::load(AgentKind::Crush, Some(&path)).unwrap();
        let provider = Provider {
            id: "lm".into(),
            base_url: "http://localhost:1234/v1".into(),
            api_key: None,
            models: vec![],
        };
        doc.upsert(&provider, Some(&["a".to_owned(), "b".to_owned()]));
        assert!(doc.save().unwrap());

        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("// keep me"));
        let doc = ConfigDoc::load(AgentKind::Crush, Some(&path)).unwrap();
        assert_eq!(doc.provider("lm").unwrap().models, ["a", "b"]);
        assert_eq!(doc.value["options"]["debug"], true);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rename_updates_references() {
        let path = temp_config(
            "kilo.jsonc",
            r#"{
              // selected model
              "model": "lm/a",
              "providers": {
                "lm": {
                  "package": "@ai-sdk/openai-compatible",
                  "settings": { "baseURL": "http://localhost:1234/v1" },
                  "models": { "a": {} }
                }
              }
            }"#,
        );
        let mut doc = ConfigDoc::load(AgentKind::Kilocode, Some(&path)).unwrap();
        assert_eq!(doc.rename("lm", "local").unwrap(), 1);
        assert!(doc.save().unwrap());

        let doc = ConfigDoc::load(AgentKind::Kilocode, Some(&path)).unwrap();
        assert!(doc.contains("local") && !doc.contains("lm"));
        assert_eq!(doc.value["model"], "local/a");
        assert_eq!(doc.provider("local").unwrap().models, ["a"]);
        fs::remove_file(path).unwrap();
    }
}
