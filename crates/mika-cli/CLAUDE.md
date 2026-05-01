# mika-cli — TUI CLI Binary

TUI CLI binary (`mika`): ratatui chat interface with clap subcommands.

## Subcommands

`status`, `memory`, `reminders`, `config`, `setup`, `mcp`, `skills`, `tasks`, `ask`, `doctor`, `dashboard`, `token`, `credential-helper`, `provider`, `model`, `agents`, `teams`, `webhook`, `kg`.

### Key Commands

- `mika agents list` — supports `--format text|json` (default `text`); `json` emits `[{"name":"...","active":true/false},...]` for scripting
- `mika agents validate [NAME]` — Validate agent config (provider/model pairing, API key, stale fields, max_tokens, soul.md, MCP, skill LLM overrides). Omit name to validate all agents. Supports `--format text|json`.
- `mika teams validate [NAME]` — Validate team config (team.toml, agent existence, orchestrator, flow settings). Omit name to validate all teams. Supports `--format text|json`.
- `mika provider` — List all providers with current marker. `mika provider <name>` switches provider (validates, persists, pre-fetches models). `mika provider set model|api-key|base-url <value>` sets a field (api-key always prompts interactively). Supports `--agent <name>` and `--format text|json`.
- `mika model` — List models for current provider. `mika model <name>` switches model (supports aliases like `sonnet`, `opus`, `gpt4o` and `provider/model` format for cross-provider switching). Supports `--agent <name>` and `--format text|json`.
- `mika token github` — Print a GitHub App installation token to stdout (lightweight: no tracing/DB). `--agent <name>` uses per-agent GitHub App credentials.
- `mika credential-helper get` — Git credential helper for HTTPS push with GitHub App tokens (used by git, not directly)
- `mika setup --mode compose` — Generate `.env` for docker-compose in current directory
- `mika setup --mode oauth` — Authorize Mika with Claude Pro/Max subscription via PKCE
- `mika teams log <name>` — shows full run UUIDs (copyable for `--run-id`), supports `--format text|json` and `-n/--limit` (default 10)

### `mika ask`

Sends a single non-interactive message; `--format text|json` (default `text`) controls output format — `json` emits `{"role":"assistant","content":"..."}` to stdout (includes optional `pending_tasks` array when background callback tasks were spawned during the session — omitted when empty for backward compatibility); after the agent loop, queries for pending callback tasks and prints a stderr notice if any exist (`[mika] N background task(s) started. Open TUI or start server to receive results.`).

Flags: `--task-id <uuid>` correlates the session with a task for observability and tags both user and assistant messages as `internal: true` (hidden from TUI inbox mode) — this is the relay session signal from claude-pilot; `--task-complete` (requires `--task-id`) marks the callback task complete and exits without running the agent (100KB result limit) — messages are NOT tagged internal; `--session-id <id>` reuses an existing session; `--parent-task-id <id>` sets `is_task_context: true` for task guard; `--model <model>` overrides the LLM model for this invocation only (not persisted); `--enable-skill <name>` forces named skill(s) to `always_on` for this invocation only (repeatable, not persisted — applied after DB overrides). Warns to stderr if the skill is not found or disabled; `--disable-skill <name>` transiently evicts named skill(s) from the registry for this invocation only (repeatable, not persisted — applied after DB overrides, before `--enable-skill`). Mutually exclusive per skill name with `--enable-skill`; `--verbose` emits runtime metadata alongside the response. **Orthogonal to `--format`** — the flag's semantics are the same regardless of encoding (mika#829 fixed mika#824's earlier text-mode-only scoping). In text mode: blank line followed by `session_id: <uuid>` on stdout after the response. In JSON mode: nested `metadata` object on the response envelope, e.g. `{"role":"assistant","content":"...","metadata":{"session_id":"<uuid>"}}`. Designed for cross-command integration (e.g., `/mika-groom-ticket` captures the session ID). Conflicts with `--team`. Downstream parsers should match by key name (`session_id:` in text, `metadata.session_id` in JSON), not by line position or top-level membership. 100KB result limit.

**Metadata envelope semantics (JSON mode):** the `metadata` object is **per-field gated**, not blanket-gated by `--verbose`. Today only `session_id` exists and it is `--verbose`-gated; future fields may ship gated or unconditional depending on use-case (e.g., a `trace_id` for ops observability could land without requiring the flag). The envelope shape supports both — `metadata` itself is omitted only when all its fields are absent. Consumers should treat individual keys as optional and not assume `metadata`'s presence implies `--verbose` was passed.

Scoped flags: `--agent <name>` (override active agent, most subcommands), `--team <name>` (team mode, chat and ask, mutually exclusive with `--agent` and `--model`). `mika ask --team <name> "goal"` runs the full team cycle non-interactively (progress to stderr, deliverable to stdout); `--format json` extends the schema with `team_run` metadata. `--run-id <uuid>` (requires `--team`) references a previous run's workspace as read-only context; `--last-run` (requires `--team`, conflicts with `--run-id`) resolves to the most recent finished team run automatically.

## TUI Features

- **Slash commands:** `/clear`, `/model`, `/provider`, `/think`, `/agent`, `/undo`, `/rewind`, `/inbox`
- `/clear` ends the current session, creates a new one, notifies the agent worker, drains stale responses from `agent_rx`, and resets all transient state; user preferences (`thinking_level`, model, provider) are preserved; `active_background_task_count` is intentionally NOT reset (agent-scoped, not session-scoped)
- `/provider` and `/model` pre-validate via `Settings::make_llm_provider()` before updating the UI. `/provider` switch persists default `{provider}_model` when none exists, warns about stale fields and max_tokens limits, and spawns a background `get_models()` to pre-warm the model list cache. `/model` lists available models from cache/API, supports aliases and direct `provider/model` format with cross-provider switching
- **Footer badges:** `[N tasks]` (Cyan) for pending reminders, `[N running]` (Yellow) for active background callback tasks (polled every ~5s), `[N hidden]` (DarkGray) for suppressed internal messages in inbox mode, and dashboard status indicator with clickable `[start]`/`[stop]` and `[open]` buttons
- **Inbox mode:** Default on — hides internal (agent-to-agent) messages from the chat view. `/inbox` toggles between inbox mode (filtered) and audit mode (all messages visible). Reloads message history from DB on toggle. `hidden_internal_count` tracks new internal messages arriving during the session
- **Input:** Shell-like Tab completion with context-aware argument completers. Multi-line input via Alt+Enter (primary) or Shift+Enter. Image paste (Ctrl+V), persistent per-agent input history, mouse scroll, click-drag text selection with clipboard copy, bracketed paste (100KB limit)
- **Team mode:** Streams `TeamEvent` callbacks, split-pane dashboard, `/verbose` toggles agent responses; team runs persisted to shared DB. Run-scoped workspace directories: each run creates `workspace/{run-uuid}/` with `.meta/` subdirectory for engine metadata
- **Run context:** When `--run-id` or `--last-run` is used, the TUI displays a styled context block at the top of the chat area showing the referenced run's metadata

## Wizards

`wizard.rs` — interactive dialoguer-based wizards for `agents create` and `teams create` with optional LLM-generated `soul.md`; `--no-interactive` flag skips wizard.

## Other `--format text|json` Commands

`agents validate`, `teams list`, `teams status`, `teams validate`, `skills list`, `skills validate`, `status`, `config list`, `memory search`, `provider`, `model`, `webhook list-dead`, `webhook replay`, `webhook replay-all`, `kg status`, `kg list-agents`, `kg purge`, `kg validate`.

## Webhook CLI

`mika webhook list-dead` — list DLQ entries (pending + dead). Optional `--status` filter, `--limit` cap.
`mika webhook replay <delivery_id>` — replay a single dead entry.
`mika webhook replay-all` — replay all dead entries.

Requires `MIKA_GATEWAY_URL` (default: `http://localhost:3001`) and `MIKA_INTERNAL_TOKEN` for gateway auth.

## Skills CLI

`mika skills install/uninstall/update/list/validate/info` — skill architecture details in `crates/mika-agent/CLAUDE.md`.

## Knowledge Graph CLI

`mika kg status` — show KG state summary across all agents (entity counts, chunk counts, last extraction, enabled flag, corpus grouping by `docs_root_hash`). Multi-corpus agents (e.g., mika-arch) display one row per corpus with per-corpus resolution counts (#877); agent name and enabled flag are shown on the first row only. `--agent X` filters to one agent. Supports `--format text|json`.

`mika kg list-agents` — quick enumeration of agents with KG state (agent name, enabled flag, `docs_root_hash`, chunk count). Supports `--agent X` filter and `--format text|json`.

`mika kg purge --agent X` — delete an agent's per-agent KG state (resolutions, resolution log). Interactive typed-ID confirmation (operator types the exact agent ID). `--yes` bypasses confirmation for scripting. `--include-orphaned-corpus` also deletes shared-corpus rows if no other agent references the same `docs_root_hash`. Non-TTY contexts require `--yes`. Supports `--format text|json`.

`mika kg validate` — check for orphan FK rows across KG tables and NULL `source_doc_hash` entries. Each check produces `[OK]`, `[WARN]`, or `[FAIL]` output. Exit 0 when all checks pass (Warn is acceptable), exit 1 on any Fail. Supports `--format text|json`.

Exit codes: `status`, `list-agents` always 0. `purge` returns 0 on success, 1 on cancellation or error. `validate` returns 0 iff no Fail checks, 1 otherwise.

See `crates/mika-agent/CLAUDE.md` for KG architecture and schema details.

## MCP CLI

`mika mcp add/remove/list/enable/disable`, `--header KEY=VALUE`.
