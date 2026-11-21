# Contributing

Thank you so much for considering contributing to shell-history! We really appreciate it <3

## AI

While we are very happy for you to use AI to build your contribution, it is essential that any pull requests you open are reviewed, understood, and tested by you. Pull requests with giant, AI-written descriptions imply the opposite. Please take the time to write a PR body that outlines your intent, understanding and summary of the change.

## Development dependencies

1. A rust toolchain ([rustup](https://rustup.rs) recommended)

We commit to supporting the latest stable version of Rust - nothing more, nothing less, no nightly.

Before working on anything, we suggest taking a copy of your shell-history data directory (`~/.local/share/shell-history` on most \*nix platforms). If anything goes wrong, you can always restore it!

While data directory backups are always a good idea, you can instruct shell-history to use custom path using the following environment variables:

```shell
export SUBNET_SHELL_HISTORY_RECORD_STORE_PATH=/tmp/shell_history_records.db # path to primary record store
export SUBNET_SHELL_HISTORY_DB_PATH=/tmp/shell_history_dev.db               # path to materialized history database
export SUBNET_SHELL_HISTORY_KV__DB_PATH=/tmp/shell_history_kv.db            # path to key-value store
export SUBNET_SHELL_HISTORY_SCRIPTS__DB_PATH=/tmp/shell_history_scripts.db  # path to scripts database
export SUBNET_SHELL_HISTORY_AI__DB_PATH=/tmp/shell_history_ai_sessions.db   # path to AI sessions database
export SUBNET_SHELL_HISTORY_META__DB_PATH=/tmp/shell_history_meta.db        # path to meta database
```

It is also recommended to update your `$PATH` so that the pre-exec scripts would use the locally built version:

```shell
export PATH="./target/release:$PATH"
```

If you'd like to load a different configuration file, set `SUBNET_SHELL_HISTORY_CONFIG_DIR` to a folder that contains your `config.toml` file:

```shell
export SUBNET_SHELL_HISTORY_CONFIG_DIR=/tmp/shell-history-config/
```

These variable exports can be added in a local `.envrc` file, read by [direnv](https://direnv.net/).

## PRs

It can speed up the review cycle if you consent to maintainers pushing to your branch. This will only be in the case of small fixes or adjustments, and not anything large. If you feel OK with this, please check the box on the template!

## What to work on?

Any issues labeled "bug" or "help wanted" would be fantastic, just drop a comment and feel free to ask for help!

If there's anything you want to work on that isn't already an issue, either open a feature request or get in touch on the [forum](https://forum.atuin.sh)/Discord.

## Setup

```
git clone https://github.com/atuinsh/atuin
cd shell-history
cargo build
```

## Running

When iterating on a feature, it's useful to use `cargo run`

For example, if working on a search feature

```
cargo run -- search --a-new-flag
```

While iterating on the server, I find it helpful to run a new user on my system, with `sync_server` set to be `localhost`.

## Tests

Our test coverage is currently not the best, but we are working on it! Generally tests live in the file next to the functionality they are testing, and are executed just with `cargo test`.

## Logging and Debugging

### Log Files

shell-history writes logs to `~/.shell-history/logs` unless configured otherwise. Log files are rotated daily and retained for 4 days by default:

- `search.log.*` - Interactive search session logs
- `daemon.log.*` - Background daemon logs

### Log Levels

You can set the `SUBNET_SHELL_HISTORY_LOG` environment variable to override log verbosity from the config file:

```shell
SUBNET_SHELL_HISTORY_LOG=debug shell-history search  # Enable debug logging
SUBNET_SHELL_HISTORY_LOG=trace shell-history search  # Enable trace logging (very verbose)
```

### Span Timing (Performance Profiling)

For performance analysis, you can capture detailed span timing data as JSON:

```shell
SUBNET_SHELL_HISTORY_SPAN=spans.json shell-history search
```

This creates a JSON file with timing information for each instrumented span, including:
- `time.busy` - Time actively executing code
- `time.idle` - Time awaiting async operations (I/O, child tasks)

The `scripts/span-table.ts` script analyzes these logs:

```shell
# Summary view - shows all spans with timing stats
bun scripts/span-table.ts spans.json

# Detail view - shows individual calls for a specific span
bun scripts/span-table.ts spans.json --detail daemon_search

# Filter to specific spans
bun scripts/span-table.ts spans.json --filter "search|hydrate"
```

This is useful for comparing performance between different search implementations or identifying bottlenecks.

## Migrations

Be careful creating database migrations - once your database has migrated ahead of current stable, there is no going back

### Stickers

We try to ship anyone contributing to shell-history a sticker! Only contributors get a shiny one. Fill out [this form](https://noteforms.com/forms/contributors-stickers) if you'd like one.
