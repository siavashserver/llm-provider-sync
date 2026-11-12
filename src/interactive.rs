//! Interactive flows built on `cliclack`.
//!
//! These only collect input and present results; the actual work is done by the
//! shared operations in [`crate::commands`]. Every action reloads the config
//! from disk, so the menus always reflect the file's current state.
//!
//! Flow: `browse` (provider picker) -> `manage` (one provider's menu). Removing a
//! provider returns to the picker; only "Exit" leaves the app.

use std::fmt::Display;

use crate::{
    agent::Provider,
    commands::{self, Ctx, KeyChange, SetChanges},
    error::Error,
    ui::{self, prompt},
    validate,
};

/// What to do after leaving a provider's menu.
enum Flow {
    /// Go back to the provider picker.
    Back,
    /// Leave the app.
    Quit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Sync,
    Edit,
    Models,
    Remove,
    Exit,
}

#[derive(Clone, PartialEq, Eq)]
enum Choice {
    Provider(String),
    Add,
    Exit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyAction {
    Keep,
    Change,
    Clear,
}

/// `llm-provider-sync <agent>`: pick a provider (or add one) and manage it.
pub fn run_agent(ctx: &Ctx) -> anyhow::Result<()> {
    ui::require_interactive()?;
    prompt(cliclack::intro(title(ctx)))?;
    browse(ctx)?;
    prompt(cliclack::outro("Done"))?;
    Ok(())
}

/// `llm-provider-sync <agent> <id>`: manage the provider, or offer to create it.
pub fn run_provider(ctx: &Ctx, id: &str) -> anyhow::Result<()> {
    ui::require_interactive()?;
    prompt(cliclack::intro(title(ctx)))?;

    if ctx.load()?.contains(id) {
        if let Flow::Back = manage(ctx, id)? {
            browse(ctx)?;
        }
    } else {
        let create_it = prompt(
            cliclack::confirm(format!("Provider `{id}` not found. Create it?"))
                .initial_value(true)
                .interact(),
        )?;
        if !create_it {
            prompt(cliclack::outro_cancel("Nothing was changed"))?;
            return Ok(());
        }
        create(ctx, id)?;
    }
    prompt(cliclack::outro("Done"))?;
    Ok(())
}

fn title(ctx: &Ctx) -> String {
    format!("llm-provider-sync · {}", ctx.agent.name())
}

/// Provider picker. Loops until the user chooses "Exit" (or quits from a provider's menu).
fn browse(ctx: &Ctx) -> anyhow::Result<()> {
    loop {
        let providers = ctx.load()?.providers();
        let mut menu = cliclack::select::<Choice>("Provider");
        for provider in &providers {
            menu = menu.item(
                Choice::Provider(provider.id.clone()),
                &provider.id,
                &provider.base_url,
            );
        }
        let choice = prompt(
            menu.item(Choice::Add, "+ Add new provider", "")
                .item(Choice::Exit, "Exit", "")
                .interact(),
        )?;

        match choice {
            Choice::Provider(id) => {
                if let Flow::Quit = manage(ctx, &id)? {
                    return Ok(());
                }
            }
            Choice::Add => {
                let id: String = prompt(
                    cliclack::input("New provider id")
                        .validate(|value: &String| validate::provider_id(value).map(|_| ()))
                        .interact(),
                )?;
                create(ctx, &id)?;
            }
            Choice::Exit => return Ok(()),
        }
    }
}

/// Menu loop for an existing provider.
fn manage(ctx: &Ctx, id: &str) -> anyhow::Result<Flow> {
    let mut id = id.to_owned();
    loop {
        let provider = ctx.load()?.provider(&id)?;
        prompt(cliclack::note(&id, summary(&provider)))?;

        let action = prompt(
            cliclack::select("What next?")
                .item(
                    Action::Sync,
                    "Sync models",
                    format!("{} in config", provider.models.len()),
                )
                .item(Action::Edit, "Edit provider", "")
                .item(Action::Models, "Show models", "")
                .item(Action::Remove, "Remove provider", "")
                .item(Action::Exit, "Exit", "")
                .interact(),
        )?;

        match action {
            Action::Sync => {
                run_step("Syncing models…", || commands::sync_provider(ctx, &id));
            }
            Action::Edit => {
                if let Some(new_id) = edit(ctx, &provider)? {
                    id = new_id;
                }
            }
            Action::Models => {
                let models = if provider.models.is_empty() {
                    "(none synced)".to_owned()
                } else {
                    provider.models.join("\n")
                };
                prompt(cliclack::note("Models", models))?;
            }
            Action::Remove => {
                if commands::remove(ctx, &id)? {
                    prompt(cliclack::log::success(format!("Removed `{id}`")))?;
                    return Ok(Flow::Back);
                }
            }
            Action::Exit => return Ok(Flow::Quit),
        }
    }
}

/// Prompts for new values (prefilled with the current ones). Returns the provider's id afterwards,
/// or `None` if nothing changed or the update failed.
fn edit(ctx: &Ctx, current: &Provider) -> anyhow::Result<Option<String>> {
    let id: String = prompt(
        cliclack::input("Provider id")
            .default_input(&current.id)
            .validate(|value: &String| validate::provider_id(value).map(|_| ()))
            .interact(),
    )?;
    let base_url: String = prompt(
        cliclack::input("Base URL")
            .default_input(&current.base_url)
            .validate(|value: &String| validate::base_url(value).map(|_| ()))
            .interact(),
    )?;

    // The key is optional: keep it, replace it, or (if one is set) remove it.
    let has_key = current.api_key.is_some();
    let mut menu = cliclack::select("API key")
        .item(
            KeyAction::Keep,
            "Keep as is",
            if has_key { "key set" } else { "no key" },
        )
        .item(
            KeyAction::Change,
            if has_key { "Replace" } else { "Set a key" },
            "",
        );
    if has_key {
        menu = menu.item(KeyAction::Clear, "Remove", "");
    }
    let api_key = match prompt(menu.interact())? {
        KeyAction::Keep => KeyChange::Keep,
        KeyAction::Change => KeyChange::Set(ask_key()?),
        KeyAction::Clear => KeyChange::Clear,
    };

    let changes = SetChanges {
        new_id: (id != current.id).then_some(id),
        base_url: (validate::base_url(&base_url)? != current.base_url).then_some(base_url),
        api_key,
        sync: true,
    };
    if changes.is_empty() {
        prompt(cliclack::log::info("No changes"))?;
        return Ok(None);
    }

    let outcome = run_step("Applying changes…", || {
        commands::set_provider(ctx, &current.id, changes)
    });
    Ok(outcome.map(|outcome| outcome.id))
}

/// Collects a base URL and an optional API key, then adds the provider. Offers a retry on failure.
fn create(ctx: &Ctx, id: &str) -> anyhow::Result<()> {
    let id = validate::provider_id(id)?;
    loop {
        let base_url: String = prompt(
            cliclack::input("Base URL")
                .placeholder("http://localhost:1234/v1")
                .validate(|value: &String| validate::base_url(value).map(|_| ()))
                .interact(),
        )?;
        let uses_key = prompt(
            cliclack::confirm("Does it need an API key?")
                .initial_value(false)
                .interact(),
        )?;
        let api_key = if uses_key { Some(ask_key()?) } else { None };

        if run_step("Checking /models…", || {
            commands::add_provider(ctx, &id, &base_url, api_key.clone(), true)
        })
        .is_some()
        {
            prompt(cliclack::log::success(format!("Provider `{id}` is ready")))?;
            return Ok(());
        }
        let retry = prompt(
            cliclack::confirm("Try again?")
                .initial_value(true)
                .interact(),
        )?;
        if !retry {
            prompt(cliclack::log::info("Nothing was changed"))?;
            return Ok(());
        }
    }
}

/// Masked key prompt. Only asked when the user has said they want a key, so it may be required.
fn ask_key() -> anyhow::Result<String> {
    Ok(prompt(cliclack::password("API key").mask('•').interact())?)
}

/// Runs a fallible step behind a spinner. Failures are shown and turned into `None`
/// so the flow can continue.
fn run_step<T: Display>(label: &str, step: impl FnOnce() -> Result<T, Error>) -> Option<T> {
    let spinner = cliclack::spinner();
    spinner.start(label);
    match step() {
        Ok(value) => {
            spinner.stop(&value);
            Some(value)
        }
        Err(err) => {
            spinner.error(&err);
            None
        }
    }
}

fn summary(provider: &Provider) -> String {
    format!(
        "base URL: {}\napi key:  {}\nmodels:   {}",
        provider.base_url,
        provider
            .api_key
            .as_deref()
            .map_or_else(|| "(none)".to_owned(), commands::mask_key),
        provider.models.len(),
    )
}
