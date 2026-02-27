# Getting Started with Mika

Mika is a conversation-first AI executive assistant that runs as a local CLI.
It remembers people, commitments, preferences, and events across sessions using
a local SQLite database, and communicates with Claude for intelligent responses.

This guide walks you through installation, first run, and everyday usage.

---

## 1. Prerequisites

- **Rust toolchain:** Mika requires Rust 1.93 (stable). Install via [rustup](https://rustup.rs/):

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

---

## 2. Installation

Clone the repository and build the CLI binary:

```sh
git clone https://github.com/senara-solutions/mika.git
cd mika
cargo build --release
```

The binary is at `target/release/mika`. Add it to your PATH:

```sh
cp target/release/mika ~/.local/bin/
```

Or install directly:

```sh
cargo install --path crates/mika-cli
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

Do NOT put your credential in `config.toml` -- it must be set as an environment variable.
Add it to your shell profile (`~/.bashrc`, `~/.zshrc`, etc.) so it persists:

```sh
echo 'export MIKA_ANTHROPIC_API_KEY="sk-ant-..."' >> ~/.zshrc
source ~/.zshrc
```

### Verifying

Run `mika config` to confirm Mika sees your credential:

```
anthropic_api_key: OAuth token [REDACTED]   # subscription token
anthropic_api_key: API key [REDACTED]       # API key
```

---

## 4. First run

Just run `mika` with no arguments:

```sh
mika
```

On the first run, Mika detects that it has not been initialized and automatically
runs setup. This creates the `~/.mika/` directory with all default files:

```
Mika initialized at /home/youruser/.mika
```

After setup completes, the interactive chat TUI opens immediately.

If you prefer to run setup explicitly without starting a chat session:

```sh
mika setup
```

Running `mika setup` again after initialization is safe -- it prints a message
and exits without modifying anything.

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
| `mika status`                | Show health info (messages, DB size, schema)      |
| `mika memory`                | Inspect stored core memory                        |
| `mika ask "<message>"`       | Send a message non-interactively                  |

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

**In scripts:**

```sh
#!/bin/sh
response=$(mika ask "What's on my calendar today?")
echo "Mika says: $response"
```

Each `mika ask` invocation creates a fresh session. Mika still has access to all
stored memory (people, commitments, preferences, events) but does not carry over
conversation context from previous `ask` calls.

---

## 9. Next steps

- **[Configuration](configuration.md)** -- Customize model, max tokens, log level,
  and environment variable overrides.
- **[Skills](skills.md)** -- Learn about the skill registry, builtin skills, and
  how to create custom skills.
- **[Slash Commands](slash-commands.md)** -- Full reference for in-chat slash commands.
- **[Architecture](architecture.md)** -- Deep dive into the agent loop, memory model,
  conversation compaction, and database schema.
