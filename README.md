# llm-provider-sync

Stop hand-editing your AI agent's config every time you add an llm provider.

`llm-provider-sync` keeps your AI coding agents' model providers in sync with
the servers. Add a provider, tweak its base URL or API key, rename it, or pull
down its latest model list — all without touching JSON by hand.

It works with **Kilo Code** and **Crush** today.

## What it does, in plain English

Imagine you've got LM Studio running at `http://localhost:1234/v1` and you want
your agent to see its models. Instead of opening `kilo.jsonc` or `crush.json`,
finding the right nested key, and hoping you don't break the formatting, you
run:

```bash
llm-provider-sync crush lmstudio add --base-url http://localhost:1234/v1
```

The tool checks that the server is reachable, grabs the model list from `GET
/models`, and writes it into your agent's config. Comments, key order, and
everything else in the file stay exactly where they were.

Here's what you can do:

- **List** your providers and see how many models each has synced
- **Add** a provider — it probes `/models` first, so nothing gets saved if the
  server can't be reached
- **Change** a provider's id, base URL, or API key. Model references (like
  `provider/model`) get rewritten automatically
- **Sync** a provider to refresh its model list from `/models`
- **Remove** a provider, with a heads-up if any model settings still point at it

Only OpenAI-compatible providers are touched. The rest of your config is left
alone.

## Getting started

### Install

```bash
git clone https://github.com/Kilo-Org/llm-provider-sync.git
cd llm-provider-sync
cargo build --release
```

Copy `target/release/llm-provider-sync` somewhere on your `PATH`, or run `cargo
install --path .`.

### Your first provider

```bash
# See which agents are supported and where their configs live
llm-provider-sync agents

# Interactively pick or add a provider (needs a terminal)
llm-provider-sync crush

# Or add one non-interactively
llm-provider-sync kilocode lmstudio add --base-url http://localhost:1234/v1 --api-key-stdin

# List what you've got
llm-provider-sync crush list

# Sync every provider at once
llm-provider-sync kilocode sync-all
```

Run `llm-provider-sync --help` for the full picture, or `llm-provider-sync crush
--help` for agent-specific help.

## Commands at a glance

| Command                     | What it does                                      |
| --------------------------- | ------------------------------------------------- |
| `agents`                    | Show supported agents and their config file paths |
| `<agent> list` (alias `ls`) | List OpenAI-compatible providers                  |
| `<agent> sync-all`          | Sync every provider's model list                  |
| `<agent> <id>`              | Open the interactive menu for a provider          |
| `<agent> <id> show`         | Show a provider's settings and models             |
| `<agent> <id> add`          | Add a provider and sync its models                |
| `<agent> <id> set`          | Change id, base URL, or API key                   |
| `<agent> <id> sync`         | Refresh the model list from `/models`             |
| `<agent> <id> remove`       | Remove a provider                                 |

Global flags work everywhere:

- `--config <PATH>` — edit a specific file instead of the default
- `-y / --yes` — skip confirmation prompts
- `--timeout <SECS>` — HTTP timeout (default: 10 seconds)
- `--json` — JSON output for `list` and `show`

## Supported agents

| Agent                  | Where providers live | Config locations                                                                 |
| ---------------------- | -------------------- | -------------------------------------------------------------------------------- |
| Kilo Code (`kilocode`) | `providers.<id>`      | `~/.config/kilo/` — `kilo.jsonc`, `kilo.json`, `opencode.jsonc`, `opencode.json` |
| Crush (`crush`)        | `providers.<id>`     | `$XDG_CONFIG_HOME/crush/crush.json` (Windows: `%LOCALAPPDATA%\crush\crush.json`) |

Use `--config <PATH>` if your config lives somewhere unusual.

## How syncing works

When you add or sync a provider, the tool calls `GET {base_url}/models` and
replaces the model list with what comes back. An empty list is treated as an
error, so a sync can never accidentally wipe a provider. Model ids are sorted
and de-duplicated.

Renaming a provider also updates any selected-model references that pointed at
the old id, so nothing gets left dangling.

## Keys and secrets

API keys are stored as-is in your config. Environment references (`$VAR`,
`${VAR}`, `{env:VAR}`) are left untouched and only resolved when the tool
actually talks to the server — they're never printed unmasked unless you
explicitly pass `--show-key`.

For day-to-day use, prefer `--api-key-stdin` or the `LLM_PROVIDER_SYNC_API_KEY`
environment variable over `--api-key` on the command line, since command
arguments end up in your shell history.

## Under the hood

The full lives in [AGENTS.md](./AGENTS.md).

## License

MIT — see [LICENSE](./LICENSE).
