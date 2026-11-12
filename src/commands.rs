//! Command implementations.
//!
//! [`add_provider`], [`set_provider`], [`sync_provider`] and [`remove`] hold the
//! behaviour and are shared by the non-interactive commands (this file) and the
//! interactive flows ([`crate::interactive`]). They never print; callers show the
//! returned outcomes, which implement `Display`.

use std::{collections::HashSet, fmt, io, iter, path::PathBuf, time::Duration};

use anyhow::Context as _;
use serde_json::json;

use crate::{
    agent::{AgentKind, Provider},
    cli::{AgentArgs, ProviderAction},
    config::{self, ConfigDoc},
    error::Error,
    interactive, probe, ui, validate,
};

/// Settings shared by every command, built from the global flags.
pub struct Ctx {
    pub agent: AgentKind,
    pub config: Option<PathBuf>,
    pub yes: bool,
    pub json: bool,
    pub timeout: Duration,
}

impl Ctx {
    pub fn load(&self) -> Result<ConfigDoc, Error> {
        ConfigDoc::load(self.agent, self.config.as_deref())
    }

    fn fetch_models(&self, base_url: &str, api_key: Option<&str>) -> Result<Vec<String>, Error> {
        Ok(probe::fetch_models(base_url, api_key, self.timeout)?)
    }
}

// ---- outcomes ------------------------------------------------------------------------------------

/// What a sync changed in a provider's model list.
#[derive(Debug)]
pub struct SyncReport {
    added: usize,
    kept: usize,
    removed: usize,
}

impl SyncReport {
    fn new(old: &[String], new: &[String]) -> Self {
        let old: HashSet<&str> = old.iter().map(String::as_str).collect();
        let kept = new
            .iter()
            .filter(|model| old.contains(model.as_str()))
            .count();
        Self {
            added: new.len() - kept,
            kept,
            removed: old.len().saturating_sub(kept),
        }
    }
}

impl fmt::Display for SyncReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} models (+{}, -{})",
            self.added + self.kept,
            self.added,
            self.removed
        )
    }
}

pub struct AddOutcome {
    id: String,
    report: Option<SyncReport>,
}

impl fmt::Display for AddOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Added provider `{}`", self.id)?;
        match &self.report {
            Some(report) => write!(f, ": {report}"),
            None => write!(f, " (models not synced)"),
        }
    }
}

pub enum KeyChange {
    Keep,
    Set(String),
    Clear,
}

pub struct SetChanges {
    pub new_id: Option<String>,
    pub base_url: Option<String>,
    pub api_key: KeyChange,
    /// Re-sync models after a base URL or API key change.
    pub sync: bool,
}

impl SetChanges {
    pub fn is_empty(&self) -> bool {
        self.new_id.is_none() && self.base_url.is_none() && matches!(self.api_key, KeyChange::Keep)
    }
}

pub struct SetOutcome {
    /// The provider's id after the change.
    pub id: String,
    refs_updated: usize,
    sync: Option<SyncReport>,
}

impl fmt::Display for SetOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Updated provider `{}`", self.id)?;
        if self.refs_updated > 0 {
            write!(f, ", updated {} model reference(s)", self.refs_updated)?;
        }
        if let Some(report) = &self.sync {
            write!(f, "; synced {report}")?;
        }
        Ok(())
    }
}

// ---- core operations -----------------------------------------------------------------------------

/// Validates, probes `/models`, then adds the provider (with its models unless `sync` is false).
pub fn add_provider(
    ctx: &Ctx,
    id: &str,
    base_url: &str,
    api_key: Option<String>,
    sync: bool,
) -> Result<AddOutcome, Error> {
    let id = validate::provider_id(id)?;
    let base_url = validate::base_url(base_url)?;
    let mut doc = ctx.load()?;
    if doc.contains(&id) {
        return Err(Error::ProviderExists(id));
    }

    let models = ctx.fetch_models(&base_url, api_key.as_deref())?;
    let provider = Provider {
        id,
        base_url,
        api_key,
        models: Vec::new(),
    };
    doc.upsert(&provider, sync.then_some(models.as_slice()));
    doc.save()?;

    Ok(AddOutcome {
        report: sync.then(|| SyncReport::new(&[], &models)),
        id: provider.id,
    })
}

/// Applies changes to an existing provider. A changed base URL or API key is
/// re-validated against `/models` and, unless `changes.sync` is false, the model
/// list is replaced.
pub fn set_provider(ctx: &Ctx, id: &str, changes: SetChanges) -> Result<SetOutcome, Error> {
    let mut doc = ctx.load()?;
    let mut provider = doc.provider(id)?;
    let old_models = provider.models.clone();

    let mut connection_changed = false;
    if let Some(url) = changes.base_url.as_deref() {
        provider.base_url = validate::base_url(url)?;
        connection_changed = true;
    }
    match changes.api_key {
        KeyChange::Keep => {}
        KeyChange::Set(key) => {
            provider.api_key = Some(key);
            connection_changed = true;
        }
        KeyChange::Clear => {
            provider.api_key = None;
            connection_changed = true;
        }
    }
    let new_id = changes
        .new_id
        .as_deref()
        .map(validate::provider_id)
        .transpose()?
        .filter(|new| new != id);

    // Probe before touching the file so a bad URL or key changes nothing.
    let fetched = if connection_changed {
        Some(ctx.fetch_models(&provider.base_url, provider.api_key.as_deref())?)
    } else {
        None
    };
    let models = fetched.as_deref().filter(|_| changes.sync);

    let refs_updated = match &new_id {
        Some(new) => {
            let updated = doc.rename(id, new)?;
            provider.id.clone_from(new);
            updated
        }
        None => 0,
    };
    doc.upsert(&provider, models);
    doc.save()?;

    Ok(SetOutcome {
        sync: models.map(|models| SyncReport::new(&old_models, models)),
        refs_updated,
        id: provider.id,
    })
}

/// Replaces the provider's model list with what its `/models` endpoint returns.
pub fn sync_provider(ctx: &Ctx, id: &str) -> Result<SyncReport, Error> {
    let mut doc = ctx.load()?;
    let provider = doc.provider(id)?;
    let base_url = validate::base_url(&provider.base_url)?;

    let models = ctx.fetch_models(&base_url, provider.api_key.as_deref())?;
    let report = SyncReport::new(&provider.models, &models);
    doc.upsert(&provider, Some(&models));
    doc.save()?;
    Ok(report)
}

/// Removes a provider after confirmation. Returns `false` if the user declined.
pub fn remove(ctx: &Ctx, id: &str) -> anyhow::Result<bool> {
    let mut doc = ctx.load()?;
    doc.provider(id)?;
    let question = match doc.reference_count(id) {
        0 => format!("Remove provider `{id}`?"),
        n => format!(
            "Remove provider `{id}`? {n} model setting(s) reference it and will be left dangling."
        ),
    };
    if !confirm(ctx, &question)? {
        return Ok(false);
    }
    doc.remove(id)?;
    doc.save()?;
    Ok(true)
}

fn confirm(ctx: &Ctx, question: &str) -> anyhow::Result<bool> {
    if ctx.yes {
        return Ok(true);
    }
    if !ui::is_interactive() {
        return Err(Error::Usage("confirmation required; re-run with --yes".into()).into());
    }
    Ok(ui::prompt(
        cliclack::confirm(question).initial_value(false).interact(),
    )?)
}

// ---- command dispatch ----------------------------------------------------------------------------

pub fn dispatch(ctx: &Ctx, args: AgentArgs) -> anyhow::Result<()> {
    match (args.target.as_deref(), args.action) {
        (None, None) => interactive::run_agent(ctx),
        (None, Some(_)) => Err(Error::Usage("missing provider id".into()).into()),
        (Some("list" | "ls"), None) => list(ctx),
        (Some("sync-all"), None) => sync_all(ctx),
        (Some("list" | "ls" | "sync-all"), Some(_)) => {
            Err(Error::Usage("`list` and `sync-all` take no further command".into()).into())
        }
        (Some(id), None) => interactive::run_provider(ctx, id),
        (Some(id), Some(action)) => provider_action(ctx, id, action),
    }
}

fn provider_action(ctx: &Ctx, id: &str, action: ProviderAction) -> anyhow::Result<()> {
    match action {
        ProviderAction::Show { show_key } => show(ctx, id, show_key),
        ProviderAction::Add(args) => {
            let api_key = read_key(args.api_key, args.api_key_stdin)?;
            println!(
                "{}",
                add_provider(ctx, id, &args.base_url, api_key, !args.no_sync)?
            );
            Ok(())
        }
        ProviderAction::Set(args) => {
            let api_key = if args.clear_api_key {
                KeyChange::Clear
            } else {
                read_key(args.api_key, args.api_key_stdin)?.map_or(KeyChange::Keep, KeyChange::Set)
            };
            let changes = SetChanges {
                new_id: args.new_id,
                base_url: args.base_url,
                api_key,
                sync: !args.no_sync,
            };
            if changes.is_empty() {
                return Err(Error::Usage(
                    "nothing to change; pass --id, --base-url, --api-key, --api-key-stdin or --clear-api-key".into(),
                )
                .into());
            }
            println!("{}", set_provider(ctx, id, changes)?);
            Ok(())
        }
        ProviderAction::Sync => {
            println!("Synced `{id}`: {}", sync_provider(ctx, id)?);
            Ok(())
        }
        ProviderAction::Remove => {
            if remove(ctx, id)? {
                println!("Removed provider `{id}`");
            } else {
                println!("Aborted");
            }
            Ok(())
        }
    }
}

/// An explicit `--api-key` or the first line of stdin (which wins when requested).
fn read_key(inline: Option<String>, from_stdin: bool) -> anyhow::Result<Option<String>> {
    if from_stdin {
        let mut line = String::new();
        io::stdin()
            .read_line(&mut line)
            .context("failed to read the API key from stdin")?;
        let key = line.trim();
        if key.is_empty() {
            return Err(Error::Usage("no API key received on stdin".into()).into());
        }
        return Ok(Some(key.to_owned()));
    }
    Ok(inline
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty()))
}

// ---- read-only commands --------------------------------------------------------------------------

pub fn agents() -> anyhow::Result<()> {
    for kind in AgentKind::ALL {
        let location = config::locate(kind, None).map_or_else(
            |_| "no config file found".to_owned(),
            |path| path.display().to_string(),
        );
        println!("{:<10} {location}", kind.name());
    }
    Ok(())
}

fn list(ctx: &Ctx) -> anyhow::Result<()> {
    let doc = ctx.load()?;
    let providers = doc.providers();

    if ctx.json {
        let items: Vec<_> = providers
            .iter()
            .map(|p| {
                json!({
                    "id": p.id,
                    "base_url": p.base_url,
                    "api_key": p.api_key.as_deref().map(mask_key),
                    "models": p.models.len(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    if providers.is_empty() {
        println!("No OpenAI-compatible providers in {}", doc.path().display());
        return Ok(());
    }
    let rows: Vec<Vec<String>> = providers
        .iter()
        .map(|p| {
            vec![
                p.id.clone(),
                p.base_url.clone(),
                p.models.len().to_string(),
                p.api_key
                    .as_deref()
                    .map_or_else(|| "-".to_owned(), mask_key),
            ]
        })
        .collect();
    print_table(&["ID", "BASE URL", "MODELS", "API KEY"], &rows);
    Ok(())
}

fn show(ctx: &Ctx, id: &str, show_key: bool) -> anyhow::Result<()> {
    let provider = ctx.load()?.provider(id)?;
    let key = provider.api_key.as_deref().map(|key| {
        if show_key {
            key.to_owned()
        } else {
            mask_key(key)
        }
    });

    if ctx.json {
        let value = json!({
            "id": provider.id,
            "base_url": provider.base_url,
            "api_key": key,
            "models": provider.models,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    println!("id:       {}", provider.id);
    println!("base URL: {}", provider.base_url);
    println!("api key:  {}", key.as_deref().unwrap_or("(none)"));
    println!("models:   {}", provider.models.len());
    for model in &provider.models {
        println!("  - {model}");
    }
    Ok(())
}

/// Syncs every provider, continuing past failures.
fn sync_all(ctx: &Ctx) -> anyhow::Result<()> {
    let ids: Vec<String> = ctx.load()?.providers().into_iter().map(|p| p.id).collect();
    if ids.is_empty() {
        println!("No OpenAI-compatible providers to sync");
        return Ok(());
    }

    let mut failed = 0;
    for id in &ids {
        match sync_provider(ctx, id) {
            Ok(report) => println!("✓ {id}: {report}"),
            Err(err) => {
                failed += 1;
                eprintln!("✗ {id}: {err}");
            }
        }
    }
    if failed > 0 {
        return Err(Error::SyncFailed(failed).into());
    }
    Ok(())
}

// ---- formatting ----------------------------------------------------------------------------------

/// Hides all but the last four characters of a key. Environment references are not secret and stay as-is.
pub fn mask_key(key: &str) -> String {
    if key.starts_with('$') || key.starts_with("{env:") {
        return key.to_owned();
    }
    let len = key.chars().count();
    if len < 12 {
        "***".to_owned()
    } else {
        let tail: String = key.chars().skip(len - 4).collect();
        format!("***{tail}")
    }
}

fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    let widths: Vec<usize> = headers
        .iter()
        .enumerate()
        .map(|(i, header)| {
            rows.iter()
                .map(|row| row[i].chars().count())
                .chain(iter::once(header.chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();

    let line = |cells: Vec<&str>| {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|(cell, &width)| format!("{cell:<width$}"))
            .collect();
        println!("{}", padded.join("  ").trim_end());
    };
    line(headers.to_vec());
    for row in rows {
        line(row.iter().map(String::as_str).collect());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_literal_keys_but_not_env_references() {
        assert_eq!(mask_key("short"), "***");
        assert_eq!(mask_key("sk-abcdefghijklmnop"), "***mnop");
        assert_eq!(mask_key("$OPENAI_API_KEY"), "$OPENAI_API_KEY");
        assert_eq!(mask_key("{env:KEY}"), "{env:KEY}");
    }

    #[test]
    fn sync_report_counts_changes() {
        let old = ["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let new = [
            "b".to_owned(),
            "c".to_owned(),
            "d".to_owned(),
            "e".to_owned(),
        ];
        assert_eq!(SyncReport::new(&old, &new).to_string(), "4 models (+2, -1)");
    }
}
