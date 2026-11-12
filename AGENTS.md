# AGENTS.md

Guidance for AI agents (and humans) working on this repository.

## Project introduction

`llm-provider-sync` is a small Rust CLI that manages **OpenAI-compatible model
providers** (for example a local server at `http://localhost:1234/v1`) inside
the **global config files of AI coding agents**. Today it supports **Kilo Code**
(`kilocode`) and **Crush** (`crush`).

It removes the chore of hand-editing those configs: add, change, rename or
delete a provider (`id`, `base URL`, `API key`) and **sync its model list** from
the provider's `GET {base_url}/models` endpoint.

Usage shape:

```
llm-provider-sync                                  # help
llm-provider-sync agents                           # supported agents + config paths
llm-provider-sync <agent>                          # interactive: pick or add a provider
llm-provider-sync <agent> list | sync-all
llm-provider-sync <agent> <id>                     # interactive for one provider
llm-provider-sync <agent> <id> show | add | set | sync | remove
```

Global flags: `--config <PATH>`, `-y/--yes`, `--timeout <SECS>`, `--json`.

Behaviour worth knowing before changing anything:

- A provider is identified by its **id only**; there are no display names.
- **Sync replaces the provider's model list** with what `/models` returns. It
  never merges and never keeps stale entries. An empty `/models` list is an
  error, so a sync cannot wipe a provider by accident.
- Only OpenAI-compatible providers are listed or touched. Other provider types
  and every unrelated key in the config are left alone.
- Config files are edited in place: comments, key order and formatting are
  preserved. Writes are atomic. There are no backups and no dry-run mode.
- If no config file exists, commands fail with the list of paths searched
  (nothing is created).

## Software stack

Rust, **edition 2024** (MSRV 1.85). Synchronous code only: there is no async
runtime.

| Crate                           | Used for                                                       |
| ------------------------------- | -------------------------------------------------------------- |
| `clap` (derive, env)            | CLI grammar, help, `--api-key` env fallback                    |
| `cliclack`                      | Interactive prompts, spinners, notes                           |
| `reqwest` (blocking, rustls)    | `GET {base_url}/models`                                        |
| `serde_json`                    | Reading config values, `/models` responses, JSON output        |
| `jsonc-parser` (`cst`, `serde`) | Parsing JSONC; editing it while preserving comments and layout |
| `url`                           | Base URL validation and normalization                          |
| `directories`                   | Home / local-app-data lookup                                   |
| `thiserror`                     | Typed errors (`Error`, `ProbeError`)                           |
| `anyhow`                        | Error plumbing in `main` and command handlers                  |

Do not add dependencies without a clear need. `serde` derive is intentionally
not used; JSON is handled through `serde_json::Value`.

## Build, test, lint

```
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

Tests are pure unit tests (validation, `/models` parsing, key masking, sync
diff) plus config round-trips against temp files. Nothing in the test suite
touches the network or the user's real configs.

## Architecture

```
src/
  main.rs         parse CLI, build Ctx, map errors to exit codes
  cli.rs          clap definitions only (no logic)
  commands.rs     shared operations + non-interactive commands + output formatting
  interactive.rs  cliclack flows; only collects input, calls commands.rs
  config.rs       ConfigDoc: locate/load/edit/save an agent config (agent-independent)
  agent/
    mod.rs        AgentKind, Agent trait, Provider type, path helpers
    crush.rs      Crush format
    kilocode.rs   Kilo Code format
  probe.rs        GET /models, response validation, API-key env references
  validate.rs     provider id / base URL validation
  error.rs        Error enum + exit codes
  ui.rs           TTY check, prompt-error mapping (Ctrl-C -> Cancelled)
```

How it fits together:

1. `main` parses the CLI. The agent is a subcommand (`kilocode`/`crush`); the
   provider id is a positional followed by an optional action. `list` and
   `sync-all` arrive as the "id" and are matched in `commands::dispatch`. That
   is why they, and the action names, are **reserved provider ids**
   (`validate::RESERVED_IDS`). Keep that list in sync with `cli.rs` when adding
   commands.
2. `commands::dispatch` routes to either the interactive flows or a provider
   action.
3. The core operations (`add_provider`, `set_provider`, `sync_provider`,
   `remove`) validate input, probe `/models` **before** modifying anything, then
   edit and save through `ConfigDoc`. They return outcome values that implement
   `Display`; they never print. Both the CLI and the interactive flows reuse
   them, so behaviour is identical in both modes.
4. `ConfigDoc` is load -> edit -> save, used once. Reads come from a
   `serde_json::Value` snapshot; writes go through the `jsonc_parser` CST.
   Reload after saving if you need to read again (the interactive loop does).
5. An `Agent` adapter only describes its format: candidate config paths, the
   providers key, how to read a provider, how to write its managed fields, and
   which settings reference providers. `ConfigDoc` owns everything generic
   (upsert, remove, rename-with-reference-rewrite, atomic save).

### Config formats

|                                                 | Crush                                                                                             | Kilo Code                                                                                          |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Schema                                          | https://charm.land/crush.json                                                                     | https://app.kilo.ai/config.json                                                                    |
| Global file                                     | `$XDG_CONFIG_HOME` or `~/.config` + `crush/crush.json`; Windows `%LOCALAPPDATA%\crush\crush.json` | `~/.config/kilo/` + first existing of `kilo.jsonc`, `kilo.json`, `opencode.jsonc`, `opencode.json` |
| Provider block                                  | `providers.<id>`                                                                                  | `provider.<id>`                                                                                    |
| OpenAI-compatible when                          | `type == "openai-compat"`                                                                         | `npm == "@ai-sdk/openai-compatible"`                                                               |
| URL / key                                       | `base_url`, `api_key`                                                                             | `options.baseURL`, `options.apiKey`                                                                |
| Models                                          | array of objects (many required fields, filled with defaults)                                     | map keyed by model id: `{ "<id>": { "name": "<id>" } }`                                            |
| Selected-model references (rewritten on rename) | `models.*.provider`                                                                               | `model`, `small_model`, `agent.*.model` as `provider/model`                                        |

API keys are stored verbatim. Environment references (`$VAR`, `${VAR}`,
`{env:VAR}`) are kept as written and resolved only when probing.

Kilo note: entries in the newer `providers` block are ignored; this tool targets
the `provider` block, which documents `options.baseURL` and `options.apiKey`.

## Conventions and best practices

- **Errors**: library-style modules return `Result<_, Error>` (thiserror).
  Handlers and `main` use `anyhow`. Never `unwrap`/`expect` outside tests.
  User-facing messages are lowercase, say what failed and what to try. Every
  `Error` variant maps to an exit code in `Error::exit_code`: 2 usage, 3 config
  problems, 4 validation/probe/sync failures, 5 provider not found, 130
  cancelled, 1 anything else.
- **Validate, then probe, then write.** Nothing is written if validation or the
  `/models` probe fails.
- **Keep the file safe**: edit through the CST only (never re-serialize the
  whole file from `Value`), write atomically, keep the original permissions (the
  file holds API keys), skip the write when content is unchanged, resolve
  symlinks so dotfile setups keep working.
- **Secrets**: never print a key unmasked unless `show --show-key` is passed;
  never log request headers; prefer `--api-key-stdin` or
  `LLM_PROVIDER_SYNC_API_KEY` over `--api-key` in docs and examples.
- **Separation of concerns**: `cli.rs` has no logic; `interactive.rs` has no
  business rules; agent adapters know formats but not workflows.
- **Keep it simple**: plain functions and small structs; one trait (`Agent`)
  justified by two real implementations. Prefer borrowing (`&str`, `&[String]`)
  over cloning in new code.
- **Style**: `cargo fmt`, clippy clean (`-D warnings`), doc comments (`//!`,
  `///`) explain _why_ and invariants, not what the code already says. Public
  surface is `pub` only where another module needs it.
- **Tests**: add a unit test with every new validation rule, parser branch or
  format quirk. Use temp files for config tests and clean them up.
- **Output**: human output on stdout, errors and per-item failures on stderr.
  `--json` output must stay stable and contain no prose.

### Adding a new agent

1. Add `src/agent/<name>.rs` implementing `Agent` (see `crush.rs` for the
   smallest example).
2. Add a variant to `AgentKind`, extend `AgentKind::ALL` and `adapter()`.
3. Add a variant to `cli::Command` and a match arm in `main::run`.
4. Document its format in the table above and add a round-trip test in
   `config.rs`.

## Known unknowns

- Config file locations and Crush model defaults (`DEFAULT_CONTEXT_WINDOW`,
  `DEFAULT_MAX_TOKENS`) were written from the public schemas and docs; verify
  them against the installed agents. `--config` always overrides the location.
- Crate versions in `Cargo.toml` are major-version pins; bump if a newer major
  is needed.
- The `jsonc_parser::cst` calls in `config.rs` and `agent/*.rs` are the most
  likely place to need small API adjustments between crate versions; the tests
  in `config.rs` cover the behaviour that matters (comment preservation,
  replace, rename).
