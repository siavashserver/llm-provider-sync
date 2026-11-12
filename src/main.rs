//! `llm-provider-sync`: keep AI agents' global configs in sync with
//! OpenAI-compatible model providers.

mod agent;
mod cli;
mod commands;
mod config;
mod error;
mod interactive;
mod probe;
mod ui;
mod validate;

use std::{process::ExitCode, time::Duration};

use clap::Parser;

use crate::{
    agent::AgentKind,
    cli::{Cli, Command},
    commands::Ctx,
    error::Error,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let app_error = err.downcast_ref::<Error>();
            // The prompt library already reports a cancellation; stay quiet.
            if !matches!(app_error, Some(Error::Cancelled)) {
                eprintln!("error: {err:#}");
            }
            ExitCode::from(app_error.map_or(1, Error::exit_code))
        }
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let (agent, args) = match cli.command {
        Command::Agents => return commands::agents(),
        Command::Kilocode(args) => (AgentKind::Kilocode, args),
        Command::Crush(args) => (AgentKind::Crush, args),
    };

    let global = cli.global;
    let ctx = Ctx {
        agent,
        config: global.config,
        yes: global.yes,
        json: global.json,
        timeout: Duration::from_secs(global.timeout),
    };
    commands::dispatch(&ctx, args)
}
