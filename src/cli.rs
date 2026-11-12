//! Command-line interface definition.
//!
//! Grammar: `llm-provider-sync <AGENT> [list | sync-all | <PROVIDER> [ACTION]]`.
//!
//! The provider id is a positional argument followed by an optional subcommand,
//! so `list` and `sync-all` are parsed as the "id" and handled in
//! [`crate::commands::dispatch`]. That keeps a single clap parse (global flags
//! work anywhere) at the price of reserving a few words as provider ids; see
//! [`crate::validate::provider_id`].

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

const EXAMPLES: &str = "\
Examples:
  llm-provider-sync crush list
  llm-provider-sync crush lmstudio add --base-url http://localhost:1234/v1
  llm-provider-sync kilocode lmstudio sync
  llm-provider-sync crush lmstudio            (interactive)";

const AGENT_HELP: &str = "\
Without a command the agent runs interactively (needs a terminal).

Agent-level commands (use in place of PROVIDER):
  list        List OpenAI-compatible providers (alias: ls)
  sync-all    Sync every provider";

/// Manage OpenAI-compatible model providers and sync their models into AI agent configs.
#[derive(Debug, Parser)]
#[command(version, arg_required_else_help = true, after_help = EXAMPLES)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Args)]
pub struct GlobalArgs {
    /// Config file to edit instead of the agent's global config
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Skip confirmation prompts
    #[arg(short, long, global = true)]
    pub yes: bool,

    /// HTTP timeout for provider requests, in seconds
    #[arg(long, global = true, value_name = "SECS", default_value_t = 10)]
    pub timeout: u64,

    /// JSON output (for `list` and `show`)
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show supported agents and their config files
    Agents,
    /// Manage Kilo Code providers
    Kilocode(AgentArgs),
    /// Manage Crush providers
    Crush(AgentArgs),
}

#[derive(Debug, Args)]
#[command(after_help = AGENT_HELP)]
pub struct AgentArgs {
    /// Provider id (or `list` / `sync-all`); omit for interactive mode
    #[arg(value_name = "PROVIDER")]
    pub target: Option<String>,

    #[command(subcommand)]
    pub action: Option<ProviderAction>,
}

#[derive(Debug, Subcommand)]
pub enum ProviderAction {
    /// Show a provider's settings and models
    #[command(visible_alias = "info")]
    Show {
        /// Print the API key unmasked
        #[arg(long)]
        show_key: bool,
    },
    /// Add a provider and sync its models
    Add(AddArgs),
    /// Change a provider's id, base URL or API key
    Set(SetArgs),
    /// Replace the provider's model list with what its /models endpoint returns
    Sync,
    /// Remove a provider
    #[command(visible_aliases = ["rm", "delete"])]
    Remove,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Base URL of the OpenAI-compatible API, e.g. http://localhost:1234/v1
    #[arg(long, value_name = "URL")]
    pub base_url: String,

    /// API key. Arguments end up in shell history; prefer --api-key-stdin or the env var
    #[arg(long, env = "LLM_PROVIDER_SYNC_API_KEY", hide_env_values = true)]
    pub api_key: Option<String>,

    /// Read the API key from the first line of stdin
    #[arg(long)]
    pub api_key_stdin: bool,

    /// Add the provider without syncing its models
    #[arg(long)]
    pub no_sync: bool,
}

#[derive(Debug, Args)]
pub struct SetArgs {
    /// New provider id (model references to the old id are updated too)
    #[arg(long = "id", value_name = "ID")]
    pub new_id: Option<String>,

    /// New base URL
    #[arg(long, value_name = "URL")]
    pub base_url: Option<String>,

    /// New API key
    #[arg(long, conflicts_with_all = ["api_key_stdin", "clear_api_key"])]
    pub api_key: Option<String>,

    /// Read the new API key from the first line of stdin
    #[arg(long, conflicts_with = "clear_api_key")]
    pub api_key_stdin: bool,

    /// Remove the stored API key
    #[arg(long)]
    pub clear_api_key: bool,

    /// Don't re-sync models after changing the base URL or API key
    #[arg(long)]
    pub no_sync: bool,
}
