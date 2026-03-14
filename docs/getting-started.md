# Getting Started with Mika

Mika is a conversation-first AI executive assistant that runs as a local CLI.
It remembers people, commitments, preferences, and events across sessions using
a local SQLite database, and communicates with Claude for intelligent responses.

This guide walks you through installation, first run, and everyday usage.

---

## 1. Prerequisites

- **Rust toolchain:** Mika requires Rust 1.91+ (the repository pins 1.93 via `rust-toolchain.toml`). Install via [rustup](https://rustup.rs/):

  ```sh
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  rustup install 1.93
  ```

  The repository includes a `rust-toolchain.toml` that pins the channel to `1.93`,
  so `cargo` will use it automatically.

- **Anthropic credential:** You need either an API key from [Anthropic](https://console.anthropic.com/)
  or a Claude subscription OAuth token (see [Setting up your credentials](#3-setting-up-your-credentials) below).

- **Platform:** Linux or macOS. File permissions (0700 dirs, 0600 files) are
  applied automatically on Unix systems.

- **jq:** Required by all bundled skill handler scripts (shell-exec, tmux, github, file-reader) for JSON input parsing. Install via your package manager (e.g., `apt install jq`, `brew install jq`, `emerge app-misc/jq`). The Docker agent image includes `jq` by default.

---

## 2. Installation

### Quick install (Linux / macOS)

Download and install the latest pre-built binary:

```sh
curl -fsSL https://raw.githubusercontent.com/senara-solutions/mika/main/install.sh | sh
```

To install a specific version:

```sh
curl -fsSL https://raw.githubusercontent.com/senara-solutions/mika/main/install.sh | sh -s -- v0.2.0
```

### Download from GitHub Releases

Pre-built binaries for Linux (x86_64, aarch64) and macOS (x86_64, Apple Silicon)
are available on the [Releases page](https://github.com/senara-solutions/mika/releases).
Download the archive for your platform, extract it, and place the `mika` binary
on your PATH.

### Install from crates.io

```sh
cargo install mika-cli
```

### Build from source

```sh
git clone https://github.com/senara-solutions/mika.git
cd mika
cargo build --release
cp target/release/mika ~/.local/bin/
```

---

## 3. Setting up your credentials

Mika supports two authentication methods. Both use the same environment variable --
Mika auto-detects which type you provided based on the token prefix.

### Option A: Anthropic API key (default)

Get an API key from [console.anthropic.com](https://console.anthropic.com/). Usage
is billed to your Anthropic account.

```sh
export MIKA_ANTHROPIC_API_KEY="sk-ant-api03-..."
```

### Option B: Claude subscription OAuth token

If you have a Claude Pro/Team/Enterprise subscription, you can use your subscription
quota instead of a paid API key. This requires the [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code).

1. Generate an OAuth token:

   ```sh
   claude setup-token
   ```

2. Set the token (it starts with `sk-ant-oat`):

   ```sh
   export MIKA_ANTHROPIC_API_KEY="sk-ant-oat01-..."
   ```

OAuth tokens expire periodically. When Mika reports an authentication error,
re-run `claude setup-token` to get a fresh token.

### Persisting your credential

The recommended way is to run `mika setup`, which writes secrets to `~/.mika/.env`
with echo suppression and 0600 file permissions.

Alternatively, set it as an environment variable in your shell profile
(`~/.bashrc`, `~/.zshrc`, etc.):

```sh
echo 'export MIKA_ANTHROPIC_API_KEY="sk-ant-..."' >> ~/.zshrc
source ~/.zshrc
```

Do NOT put credentials in `config.toml` -- use `~/.mika/.env` or shell environment variables.

### Verifying

Run `mika config` to confirm Mika sees your credential:

```
anthropic_api_key: OAuth token [REDACTED]   # subscription token
anthropic_api_key: API key [REDACTED]       # API key
```

For a more thorough check, run `mika doctor` to validate the entire
installation (directory permissions, database, config parsing, etc.).

---

## 4. First run

Just run `mika` with no arguments:

```sh
mika
```

On the first run, Mika detects that it has not been initialized and automatically
runs setup. This creates the `~/.mika/` directory with all default files and
launches a guided wizard to configure secrets and preferences:

```
  Mika Setup

  Anthropic API key: ••••••••••••
  Brave Search API key (optional, press Enter to skip):
  Enable telemetry? [y/N]: n
  Generated internal token for server mode.

  Secrets saved to ~/.mika/.env
  ✦ Mika initialized at ~/.mika/agents/main/
```

The wizard prompts for:
1. **Anthropic API key** (masked input)
2. **Brave Search API key** (optional, masked)
3. **GitHub token for investigation** (optional, masked — enables dashboard issue creation)
4. **GitHub repo** (optional, visible — `owner/repo` format for issue creation)
5. **Telemetry configuration** (OTLP endpoint and auth header if enabled)
6. **Internal token** (auto-generated 64-char hex token for server mode)

Each prompt is skipped if the value is already set (via environment variable or
`~/.mika/.env`). Secrets are written to `~/.mika/.env` (0600 permissions);
non-secret config goes to `~/.mika/config.toml`.

After setup completes, the interactive chat TUI opens immediately.

If you prefer to run setup explicitly without starting a chat session:

```sh
mika setup
```

Running `mika setup` again after initialization is safe -- it re-checks
configuration and only prompts for missing values.

**Setup modes:** The `--mode` flag selects different setup profiles:

| Mode | Command | Purpose |
|------|---------|---------|
| `cli` (default) | `mika setup` | Configure API keys, GitHub config, telemetry, internal token |
| `server` | `mika setup --mode server` | CLI config + routing URL, dashboard token |
| `compose` | `mika setup --mode compose` | Generate a `.env` for docker-compose in CWD |

**Non-interactive setup:** If stdin is not a terminal, `mika setup` requires all
secrets to be pre-set via environment variables. Pre-set all `MIKA_*` vars before
running to skip all prompts.

You can override the home directory by setting `MIKA_HOME`:

```sh
export MIKA_HOME=/path/to/custom/mika-home
mika
```

---

## 5. Your first conversation

Once the TUI chat opens, type a message and press Enter. Mika responds using
Claude (default model: `claude-sonnet-4-6`).

Try these to get started:

```
Hi, I'm Alex. I run a small engineering team at Acme Corp.
```

Mika will learn about you and store facts in its memory system. Ask it to
remember things:

```
Remember that Sarah Chen is my CTO and prefers async communication.
Remind me to review the Q3 budget next Monday at 9am.
```

Inside the chat TUI, slash commands are available. Key commands include `/help`
(list all commands), `/status` (system health), and `/memory` (inspect stored
memory). Slash commands autocomplete as you type -- press Tab or arrow keys to
navigate suggestions. See [Slash Commands Reference](slash-commands.md) for the
complete list.

---

## 6. Directory structure

After initialization, Mika creates its home directory at `~/.mika/` containing
configuration files, the SQLite database, skill definitions, and logs. See
[Directory Layout](configuration.md#directory-layout) for the full structure.

**Customizing Mika's personality:** Edit `~/.mika/soul.md` to change how Mika
communicates. You can also run `mika config soul` to print the current soul
definition, or `mika config edit` to open `identity.toml` in your `$EDITOR`.

**Telling Mika about yourself:** Edit `~/.mika/user.md` with your name, role,
and preferences. This seeds Mika's initial understanding on a fresh database.

---

## 7. CLI commands reference

The most commonly used commands:

| Command                      | Description                                      |
|------------------------------|--------------------------------------------------|
| `mika`                       | Auto-setup if needed, then open interactive chat  |
| `mika --agent <name>`        | Launch TUI with a specific agent                  |
| `mika --team <name>`         | Launch TUI in team mode (mutually exclusive with `--agent`) |
| `mika status`                | Show health info (messages, DB size, schema)      |
| `mika memory`                | Inspect stored core memory                        |
| `mika ask "<message>"`       | Send a message non-interactively (`--format json` for structured output) |
| `mika ask --team <name> "goal"` | Run a team workflow non-interactively (deliverable to stdout) |
| `mika ask --task-id <uuid> "<result>"` | Complete a callback task (TUI delivers result to conversation) |
| `mika tasks`                 | List scheduled tasks for the active agent         |
| `mika skills`                | List, install, validate, and manage skills          |
| `mika config get <key>`      | Get a configuration value                          |
| `mika config set <key> [val]`| Set a configuration value (prompts for secrets)    |
| `mika config list`           | List all configuration keys and values             |
| `mika doctor`                | Check installation health (add `--verify-api` for live API test, `--json` for machine output) |

Run `mika --help` for the complete list of commands and options.

---

## 8. Non-interactive mode (`mika ask`)

Use `mika ask` to send a single message and print the response without entering
the TUI. This is useful for scripts, pipelines, and quick one-off questions.

**Basic usage:**

```sh
mika ask "What commitments do I have this week?"
```

**Piping from stdin:**

Use `"-"` as the message to read from standard input:

```sh
echo "Summarize my pending tasks" | mika ask "-"
```

```sh
cat meeting-notes.txt | mika ask "-"
```

**JSON output for scripting:**

Use `--format json` to get structured output compatible with the OpenAI message
format. The response is a single JSON object on stdout:

```sh
mika ask --format json "What are my top priorities today?"
# Output: {"role":"assistant","content":"Your top priorities are..."}
```

This is useful for piping into `jq` or consuming from other tools:

```sh
mika ask --format json "Summarize my day" | jq -r '.content'
```

**In scripts:**

```sh
#!/bin/sh
response=$(mika ask "What are my top priorities today?")
echo "Mika says: $response"
```

Each `mika ask` invocation creates a fresh session. Mika still has access to all
stored memory (people, commitments, preferences, events) but does not carry over
conversation context from previous `ask` calls.

**Completing a callback task:**

Pass `--task-id <uuid>` to mark a background task as complete with a result.
The task is marked complete in the database and the command exits. Delivery to
the user happens automatically: the TUI polls for completed callback tasks
every ~5 seconds and injects the result into the conversation.

```sh
mika ask --agent main --task-id "550e8400-e29b-41d4-a716-446655440000" "Analysis complete: found 3 issues"
```

This is the entry point for background scripts that perform long-running work
and need to resume an agent with their findings. The referenced task must have
`trigger_type=callback` and be in `pending` or `in_progress` status.

**Running a team workflow:**

Use `--team <name>` to run a full team cycle (decompose → execute → review → deliver).
Progress is printed to stderr; the deliverable is printed to stdout.

```sh
mika ask --team research "Analyze Q1 customer churn patterns"
```

Use `--run-id <uuid>` to reference a previous run's workspace and context:

```sh
mika ask --team research --run-id "550e8400-..." "Refine the analysis with regional data"
```

With `--format json`, the response includes team run metadata:

```sh
mika ask --team research --format json "Summarize findings"
# Output: {"role":"assistant","content":"...","team_run":{"run_id":"...","status":"completed","iterations":2}}
```

---

## 9. Troubleshooting

If something isn't working, run the built-in diagnostic command:

```sh
mika doctor
```

This checks home directory permissions, API key format, database integrity,
`config.toml` parsing, optional keys (OpenAI, Brave), `jq` availability, MCP
configuration, and installed skills. Add `--verify-api` to make a live API
call to the configured LLM provider to confirm credentials work end-to-end. Use `--json` for
machine-readable output (e.g., in CI scripts).

Exit code is non-zero if any check fails.

---

## 10. Next steps

- **[Configuration](configuration.md)** -- Customize model, max tokens, log level,
  and environment variable overrides.
- **[Skills](skills.md)** -- Learn about the skill registry, builtin skills, and
  how to create custom skills.
- **[Slash Commands](slash-commands.md)** -- Full reference for in-chat slash commands.
- **[Architecture](architecture.md)** -- Deep dive into the agent loop, memory model,
  conversation compaction, and database schema.
