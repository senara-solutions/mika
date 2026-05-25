---
name: mika-ask
description: Send a message to a Mika agent via `mika ask` (defaults to agent `mika`)
argument-hint: "[--agent <name>] [--session-id <id>] [--task-id <uuid>] [--model <m>] [--verbose] [--format text|json] <message-or-@path>"
---

Send a message to a Mika agent via the `mika ask` CLI and print the reply. Do NOT run `mika ask --help` — the flag surface is documented below.

## Input

`$ARGUMENTS` is a flag-prefixed message. Parse it as follows:

- `--agent <name>` — target agent (default: `mika`)
- `--session-id <id>` — reuse an existing session (creates it if missing)
- `--task-id <uuid>` — correlate with a task for observability
- `--task-complete` — mark the `--task-id` task completed (requires `--task-id`)
- `--parent-task-id <uuid>` — thread this message under a parent task
- `--model <m>` — one-shot model override (e.g. `sonnet`, `opus`, `haiku`, `openai/gpt-4o`)
- `--team <team>` — run a full team cycle instead of single-agent ask
- `--run-id <id>` — continue a previous team run (requires `--team`)
- `--last-run` — use the most recent team run as context (requires `--team`)
- `--format text|json` — output format (default `text`)
- `--verbose` — append a metadata trailer (session_id, trace_id, agent_id, provider, model, timestamps, token counts) after the response. Metadata renders as a `---`-separated key-value trailer in text mode and as a nested `metadata` object in JSON mode. For programmatic consumers capturing structured fields (`session_id`, `trace_id`, etc.), JSON metadata (`--format json --verbose`) is the canonical channel; the text trailer is a user-facing rendering only.

Everything after the recognized flags is the message.

### `@<path>` file-body reading

If the message starts with `@`, treat the remainder as a file path. Read the file content and use it as the message body. Tilde expansion is supported (`@~/path/to/file.md`). This is useful for multi-section briefs or long prompts that are awkward to pass inline.

If no flags are present, the entire `$ARGUMENTS` string is the message and the target agent is `mika`.

## Execution

1. **Resolve message body:**
   - If the message starts with `@`, strip the `@` prefix, expand `~` to `$HOME`, read the file at that path using the Read tool, and use the file content as the message body.
   - Otherwise, use the literal text as the message.

2. **Build and run the command:**
   ```bash
   mika ask [parsed flags] "<message>"
   ```
   If `--agent` was not supplied, omit the flag — `mika ask` uses the active agent (currently `mika`).

Print the agent's reply verbatim. Do not summarize or editorialize unless the user asks for it.

## Examples

- `/mika-ask what's on your plate today?`
  → `mika ask "what's on your plate today?"`

- `/mika-ask --agent mika-dev status of #608`
  → `mika ask --agent mika-dev "status of #608"`

- `/mika-ask --agent mika-dev --session-id 2026-04-18-salvage continuing from earlier — did QA pass?`
  → `mika ask --agent mika-dev --session-id 2026-04-18-salvage "continuing from earlier — did QA pass?"`

- `/mika-ask --team research what do we know about X?`
  → `mika ask --team research "what do we know about X?"`

- `/mika-ask --verbose --agent mika-arch review this plan`
  → `mika ask --verbose --agent mika-arch "review this plan"`

- `/mika-ask @~/docs/briefs/review-brief.md`
  → Reads `~/docs/briefs/review-brief.md`, then `mika ask "<file content>"`

- `/mika-ask --verbose --format json @/tmp/groom-brief.md`
  → Reads `/tmp/groom-brief.md`, then `mika ask --verbose --format json "<file content>"`

$ARGUMENTS
