# mika-cli — TUI CLI Binary

TUI CLI binary (`mika`): ratatui chat interface with clap subcommands.

## Crate layout

`src/main.rs` is the binary entry point and owns the full module tree under it (`init`, `commands`, `tui`, `wizard`). `src/lib.rs` is a minimal library surface (currently re-exporting `supervision` only) for types and primitives that need to be reachable from integration tests under `crates/mika-cli/tests/`. The binary tree and the library tree are independent — the binary accesses lib re-exports as `mika_cli::*`. Keep `lib.rs` minimal; do not bloat it to make arbitrary binary modules reachable from tests (most logic stays binary-private and is tested via inline `#[cfg(test)] mod tests`).

## Subcommands

`status`, `memory`, `reminders`, `config`, `setup`, `mcp`, `skills`, `tasks`, `ask`, `doctor`, `dashboard`, `token`, `credential-helper`, `provider`, `model`, `agents`, `teams`, `webhook`, `kg`, `logs`.

### Key Commands

- `mika agents list` — supports `--format text|json|yaml` (default `text`); `json` emits `[{"name":"...","active":true/false},...]` for scripting
- `mika agents validate [NAME]` — Validate agent config (provider/model pairing, API key, stale fields, max_tokens, soul.md, MCP, skill LLM overrides). Omit name to validate all agents. Supports `--format text|json|yaml`.
- `mika agents reset <name>` — Wipe all per-agent state (sessions, messages, memory, tasks, KG corpora, audit events, tool/LLM call history, skill overrides) without deleting the agent row or `identity.toml`. Flags: `--dry-run` (preview per-table row counts), `--force` (bypass active-task safety check), `--yes`/`-y` (skip typed-name confirmation). Active-task guard refuses reset if tasks are in `pending`/`in_progress`/`recurring_active` status. KG shared-corpus safety: shared tables are preserved when other agents reference the same `docs_root_hash`. FTS5 index is rebuilt post-reset. Custom skill overrides are cleared; bundled skills restore on next startup.
- `mika teams validate [NAME]` — Validate team config (team.toml, agent existence, orchestrator, flow settings). Omit name to validate all teams. Supports `--format text|json|yaml`.
- `mika provider` — List all providers with current marker. `mika provider <name>` switches provider (validates, persists, pre-fetches models). `mika provider set model|api-key|base-url <value>` sets a field (api-key always prompts interactively). Supports `--agent <name>` and `--format text|json|yaml`.
- `mika model` — List models for current provider. `mika model <name>` switches model (supports aliases like `sonnet`, `opus`, `gpt4o` and `provider/model` format for cross-provider switching). Supports `--agent <name>` and `--format text|json|yaml`.
- `mika token github` — Print a GitHub App installation token to stdout (lightweight: no tracing/DB). `--agent <name>` uses per-agent GitHub App credentials.
- `mika credential-helper get` — Git credential helper for HTTPS push with GitHub App tokens (used by git, not directly)
- `mika setup --mode compose` — Generate `.env` for docker-compose in current directory
- `mika setup --mode oauth` — Authorize Mika with Claude Pro/Max subscription via PKCE
- `mika teams log <name>` — shows full run UUIDs (copyable for `--run-id`), supports `--format text|json|yaml` and `-n/--limit` (default 10)
- `mika tasks list` — List active tasks (pending, in_progress, recurring_active). Bare `mika tasks` is an alias. Supports `--format text|json|yaml`. Text mode annotates **measured** pilot liveness (mika#2335): `[pilot alive, PID n]`, `[PID n — no live pilot]`, `[PID n — liveness unverifiable]`. On a **parent** tracking row this traverses `parent_task_id` to the dispatch child, which is where the pgid lives; before mika#2335 the row an operator consults carried no liveness signal at all, and the `[executing, PID n]` shown on callback rows was inferred from the PID column rather than measured.
- `mika tasks get <id>` — Show full details for a single task. Supports `--format text|json|yaml`. Text mode adds a `Dispatch pilot:` line — the measured answer (alive / not running / unverifiable / none), with the claude-pilot session-log age when alive. `status` and `fired_at` describe bookkeeping; this line describes the machine.
- `mika tasks cancel <id>` — Cancel a task and kill its process if running. When a pilot is still alive under the task, warns with its PID and log age and **asks for confirmation** (mika#2335 — the gesture that killed a working pilot on 2026-09-15 because its row read `pending` / `fired_at=null`). `--yes`/`-y` skips the prompt; a non-TTY without it is refused rather than answered for. It warns and asks rather than refusing: killing a live pilot is sometimes exactly the intent.
- `mika tasks promote-deferred <class>` — Force-promote the next pending deferred dispatch wrapper for a class (`implement` or `groom`). Fails if the per-class slot is occupied unless `--override` is set (cancel-then-promote).
- `mika reminders list` — List pending reminders. Bare `mika reminders` is an alias. Supports `--format text|json|yaml`.
- `mika reminders get <id>` — Show full details for a single reminder. Validates the task is a reminder (trigger_type: time/recurring). Supports `--format text|json|yaml`.
- `mika reminders cancel <id>` — Cancel a reminder by ID.

### `mika ask`

Sends a single non-interactive message; `--format text|json|yaml` (default `text`) controls output format — `json` emits `{"role":"assistant","content":"..."}` to stdout (includes optional `pending_tasks` array when background callback tasks were spawned during the session — omitted when empty for backward compatibility); after the agent loop, queries for pending callback tasks and prints a stderr notice if any exist (`[mika] N background task(s) started. Open TUI or start server to receive results.`).

Flags: `--task-id <uuid>` correlates the session with a task for observability and tags both user and assistant messages as `internal: true` (hidden from TUI inbox mode) — this is the relay session signal from claude-pilot; `--task-complete` (requires `--task-id`) marks the callback task complete and exits without running the agent (100KB result limit) — messages are NOT tagged internal; `--session-id <id>` reuses an existing session; `--parent-task-id <id>` sets `is_task_context: true` for task guard; `--model <model>` overrides the LLM **model id** for this invocation only (not persisted) — a model-id-only override: the request always routes through the agent's configured `llm_provider`; the model-name prefix never re-dispatches to a native provider (mika#1591). A `prefix/` is stripped only when it names the configured provider itself (e.g. `qwen/qwen3.7-max` under `llm_provider = "qwen"`); under any other provider (e.g. OpenRouter, whose ids are vendor-prefixed) the full id is preserved. When the configured provider has no API key, the override fails with a named error (`Provider '<name>' has no API key configured. Cannot route model '<id>'.`) rather than a bare downstream 401. **Since mika#2304 it reaches the execution surface, on BOTH paths** — the local mika-spirit one and `--remote` — travelling as the **raw** string in `message/send` request metadata under `mika.model_override` (`mika_a2a::params::MODEL_OVERRIDE_KEY`); the resolution above (alias, conditional prefix strip, API-key check) happens on the **executing** side, because under `--remote` the executing provider is not this machine's. See the "A model override is fail-closed" paragraph below for what the flag guarantees and what `--verbose` now reports; `--enable-skill <name>` forces named skill(s) to `always_on` for this invocation only (repeatable, not persisted — applied after DB overrides). Warns to stderr if the skill is not found or disabled; `--disable-skill <name>` transiently evicts named skill(s) from the registry for this invocation only (repeatable, not persisted — applied after DB overrides, before `--enable-skill`). Mutually exclusive per skill name with `--enable-skill`; `--only-skill <name>` restricts the invocation to the named skill(s), evicting every other one for this turn (repeatable, not persisted). **It is the one SELECTION flag that reaches the execution surface** — `--enable-skill` / `--disable-skill` configure the local registry, which since mika#1727 is no longer where the turn runs, so they are validated and then abandoned in transit. (`--model` used to be listed with them and no longer is: mika#2304 gave it its own key. The two halves of that channel are now `mika.only_skills` and `mika.model_override`, both in `mika_a2a::params`.) `--only-skill` travels to mika-spirit in `message/send` request metadata under `mika.only_skills` (`mika_a2a::params::ONLY_SKILLS_KEY`) and the server restricts that turn's registry. Strictly subtractive: it can evict a skill, never activate one — naming a skill that is not otherwise active keeps nothing, and naming one this agent does not carry keeps nothing at all (the turn runs with zero skills, which is visible as `active_skill_count = 0` rather than silently unrestricted). Mutually exclusive with `--enable-skill` and `--disable-skill` in the same invocation — three selection semantics on one turn is a composition nobody wants to debug; the refusal is a clap argument error, before any call. Ignored on the `--remote` path (a remote agent's skill names are not this machine's to guess) and refused with `--team`. Motivating measurement (mika#2363): mika-arch carries three `always_on` architect skills totalling 39 798 bytes of prompt and runs exactly one pass per turn, so `_arch_ask` declaring its pass removes 23.5–30.6 KB of every architect system prompt; `--isolated` runs this one turn with a session-scoped conversation window and no compaction summary, on **both** doors, under `mika.session_isolated` (`mika_a2a::params::SESSION_ISOLATED_KEY`) — see the "`--isolated`" paragraph below for the bool-not-scope safety constraint and what `--verbose` reports; `--verbose` emits runtime metadata alongside the response. **Orthogonal to `--format`** — the flag's semantics are the same regardless of encoding (mika#829 fixed mika#824's earlier text-mode-only scoping). In text mode: blank line followed by `key: value` lines on stdout (session_id, model, agent_id, latency_ms, tokens.input/output/cache_read/cache_write). In JSON mode: nested `metadata` object on the response envelope with the same fields. Designed for cross-command integration (e.g., `/mika-groom-ticket` captures the session ID). Conflicts with `--team`. Downstream parsers should match by key name (`session_id:` in text, `metadata.session_id` in JSON), not by line position or top-level membership. 100KB result limit.

**Metadata envelope semantics (JSON mode):** the `metadata` object is **per-field gated**, not blanket-gated by `--verbose`. Fields fall into two tiers:

- **Verbose-gated** (require `--verbose`): `session_id` (String), `model` (String, `provider/model` format), `isolated` (bool — the server's attestation, see `--isolated` below; **absent** means the server attested nothing, and is never to be read as `false`), `agent_id` (String), `latency_ms` (u64, wall-clock ms for the agent loop), `tokens` (object: `{input, output, cache_read, cache_write}` — all `Option<u64>`, mapped from `LlmUsage`; absent when the agent loop returns no usage).
- **Unconditional** (present whenever their CLI flag is provided, regardless of `--verbose`): `task_id` (String, from `--task-id`), `parent_task_id` (String, from `--parent-task-id`).

The envelope itself is omitted only when all its fields are absent — non-verbose invocations without `--task-id`/`--parent-task-id` produce byte-identical output to pre-mika#843. Consumers should treat individual keys as optional and not assume `metadata`'s presence implies `--verbose` was passed. Text mode mirrors the same fields as `key: value` lines (tokens use flat `tokens.input` / `tokens.output` keys for grep-friendliness).

Scoped flags: `--agent <name>` (override active agent, most subcommands), `--team <name>` (team mode, chat and ask, mutually exclusive with `--agent` and `--model`). `mika ask --team <name> "goal"` runs the full team cycle non-interactively (progress to stderr, deliverable to stdout); `--format json` extends the schema with `team_run` metadata. `--run-id <uuid>` (requires `--team`) references a previous run's workspace as read-only context; `--last-run` (requires `--team`, conflicts with `--run-id`) resolves to the most recent finished team run automatically.

**A lost reply is an error, not an empty answer (mika#2270).** On both paths — the
local mika-spirit thin-client path and `--remote` — a `Task` the renderer cannot
read makes `mika ask` exit **non-zero** with a message naming what was inspected
(artifact count, history count and roles, `status.message` presence) plus `task_id`
and `context_id`, the two handles that find the turn in `$MIKA_SPIRIT_LOG_FILE`.
It used to render the empty string and exit 0 with `.content` absent, which is how
eleven consecutive calls dropped a completed architect verdict in silence. The rule
holds identically for `text`, `json` and `yaml`: they share one gate,
`remote_ask::render`. Consumers of `--format json` should note that `content` can
no longer be `null` on this path — a missing answer is now a failure, not a field.
The background-task notice (`[mika] N background task(s) started.`) is still
emitted on that failure path: the work was genuinely started and the reply
channel's failure must not also cost the operator that fact.

**A model override is fail-closed, and `--verbose` no longer answers for the
server (mika#2304).** Two properties, and the second is what closed the defect.

*The override reaches the executing surface.* Since mika#1727 `mika ask` runs no
loop of its own, so `--model` was spent on a local provider nobody called —
**on both paths**: the local one dispatched it into `ctx.settings`, and the
`--remote` branch of `main.rs` did not pass it at all. It now travels raw under
`mika.model_override`; the server resolves it against **its own** `llm_provider`
(the mika#1591 semantics depend on the executing provider, which under `--remote`
is not this machine's) and runs the turn under it. An override it cannot serve —
today, a provider with no API key — **fails the request** with
`remote error: Provider '<name>' has no API key configured. Cannot route model
'<id>'.` It is never degraded to "no override": a skill restriction silently
dropped makes a turn wider, which is visible; a model silently dropped makes the
measurement wrong while producing a plausible answer.

*`--verbose` reports what the server attested.* The `model:` field used to be read
back from the local `Settings` the flag had just mutated — so it printed
`openrouter/moonshotai/kimi-k2.5`, with authority, while the turn ran under the
`config.toml` model. That is why the founding measurement had to inspect the A2A
body: the surface meant to say it was the one that lied. The field now carries
`mika.effective_model` off the returned `Task`, taken server-side on the provider
that **served** the turn (per-skill `[llm]` overrides included). **When the server
attests nothing, nothing is shown** — text mode prints
`model: (not attested by the server)`, JSON omits the key. A spirit older than
mika#2304, a remote agent on another version, `message/stream` and
`returnImmediately` all land there; it is exactly the population where printing a
local value would be a lie. Reading the CLI alone is not the probe — cross-check
`turn_usage` in `$MIKA_SPIRIT_LOG_FILE`.

*Unchanged:* `mika chat --model`, which runs in process and really does execute
against the provider it builds. The resolution itself moved to
`mika_common::llm::model_override` so both sides call one implementation; a second
copy in this crate is refused by
`init::tests::mika2304_the_cli_keeps_no_second_resolver`.

**`--isolated` — one turn, one session, and the CLI does not answer for the
server (mika#1951).** `mika ask --isolated` asks the executing agent to read this
turn's conversation window **session-scoped** and to inject **no compaction
summary**, whatever its `identity.toml` says. It is the per-call lever the bench
never had: the founding measurement ran ten `mika ask --session-id <fresh-uuid>`
and got a model answering « Six. Answer unchanged. » on a brand-new session,
because `HistoryScope::Agent` — the fleet default, and the mechanism that carries
conversational continuity on Telegram where each message mints its own session —
pulls the last 20 messages of the agent across all of them. The flag does not
change that default; it lets one call opt out of it.

*Strictly restrictive, by construction.* The wire value is a **bool**
(`mika.session_isolated`, `mika_a2a::params::SESSION_ISOLATED_KEY`), never a scope
name, so a caller on `/a2a/{agent}` cannot ask an agent configured `session` to
read `agent` — widening is inexpressible rather than refused by a predicate a
later editor could relax. Absent, `null` and `false` all mean "no restriction";
a key present with a **non-boolean** value fails the request rather than
degrading to `false`, because an isolation silently dropped makes a measurement
wrong while producing a plausible answer, which is the defect itself.

*It reaches both doors.* The key is posted at the single `build_send_params` site
that `mika ask` and `mika ask --remote` share — the split that cost mika#2304 a
follow-up is refused structurally by
`remote_ask::tests::mika1951_both_ask_doors_post_the_isolation_key_through_one_site`,
and the wire fact is asserted against a live exchange by
`both_ask_doors_carry_the_isolation_request_over_the_wire`.

*`--verbose` reports the server's attestation, never the flag.* The `isolated:`
line and the JSON `metadata.isolated` field carry
`mika.session_isolated_applied` off the returned `Task`, written server-side on
**every** synchronous `message/send` turn, isolated or not. **When the server
attests nothing, nothing is claimed** — text mode prints
`isolated: (not attested by the server)`, JSON omits the key. A spirit older than
mika#1951, a remote agent on another version, `message/stream` and
`returnImmediately` all land there, and it is exactly the population where
echoing the local flag would report an isolation that did not happen. Same rule,
same reason and same wording shape as the `model:` line above.

*What it does not close:* agent-scoped memory (`store_fact`, `update_fact`,
`search_memory`, `update_core_memory`) crosses sessions by design and is
untouched — a follow-up ticket, with a measurement as its precondition.

**Remote mode (R1 ascension architecture, 2026-06-09):** `--remote <URL>` or `MIKA_REMOTE_AGENT_URL` env (flag wins) puts `mika ask` in remote mode — the in-process agent loop is bypassed and the prompt is dispatched to a cloud Mika agent via the gateway's A2A proxy (e.g., `https://gw.example.com/a2a/{customer_id}/{agent}`). Auth uses `MIKA_INTERNAL_TOKEN` as a bearer header (existing gateway internal-token contract). The remote `Task` response is rendered to stdout: text parts emitted verbatim, file parts as `[file: <name>]`, data parts as `[data]`. Errors surface single-line prefixes by `A2aError` variant — `remote error:` (JSON-RPC error) and, for a transport failure, a sentence naming both what failed and what became of the work (mika#2036). The transport half distinguishes unreachable / timed out (naming the budget spent) / HTTP status / unreadable / interrupted; the second half reports whether the answer was reclaimed, is still being generated ("Retry"), was never started, or could not be looked up — and names the `context_id` to query with `tasks/get`. **A generated answer is no longer lost:** when the exchange fails after the request landed, `mika ask` re-reads the task by the `context_id` it minted before sending, and returns the response the server already produced. Conflicts with `--team`. JSON mode adds `metadata.remote_task_id` under `--verbose`; text mode appends a `remote_task_id: <id>` trailer under `--verbose`. Remote mode does not write to the local `~/.mika/data/mika.db` — session state lives on the cloud agent. Implementation: `src/remote_ask.rs` (lib-exposed for integration tests), wired in `src/main.rs` Ask branch.

## TUI Features

- **Slash commands:** `/clear`, `/model`, `/provider`, `/think`, `/agent`, `/undo`, `/rewind`, `/inbox`, `/restart`
- `/clear` ends the current session, creates a new one, notifies the agent worker, drains stale responses from `agent_rx`, and resets all transient state; user preferences (`thinking_level`, model, provider) are preserved; `active_background_task_count` is intentionally NOT reset (agent-scoped, not session-scoped)
- `/restart` tears down a crashed agent-worker tokio task and spawns a fresh worker for the same agent (mika#1149). Refuses on a healthy worker — `/clear` is the right tool for starting a new session. The lost in-flight prompt is NOT replayed; the operator must re-type. Background callback tasks survive the restart and continue delivering to the new worker
- `/provider` and `/model` pre-validate via `Settings::make_llm_provider()` before updating the UI. `/provider` switch persists default `{provider}_model` when none exists, warns about stale fields and max_tokens limits, and spawns a background `get_models()` to pre-warm the model list cache. `/model` lists available models from cache/API, supports aliases and direct `provider/model` format with cross-provider switching
- **Footer badges:** `[N tasks]` (Cyan) for pending reminders, `[N running]` (Yellow) for active background callback tasks (polled every ~5s), `[N hidden]` (DarkGray) for suppressed internal messages in inbox mode, and dashboard status indicator with clickable `[start]`/`[stop]` and `[open]` buttons
- **Inbox mode:** Default on — hides internal (agent-to-agent) messages from the chat view. `/inbox` toggles between inbox mode (filtered) and audit mode (all messages visible). Reloads message history from DB on toggle. `--inbox` flag on `mika chat` launches directly in audit mode (equivalent to typing `/inbox` after launch). `hidden_internal_count` tracks internal messages: seeded at startup with the count of internals filtered from the initial message load (mika#593), then incremented as new internal messages arrive during the session
- **Input:** Shell-like Tab completion with context-aware argument completers. Multi-line input via Alt+Enter (primary) or Shift+Enter. Image paste (Ctrl+V), persistent per-agent input history, mouse scroll, click-drag text selection with clipboard copy, bracketed paste (100KB limit)
- **Team mode:** Streams `TeamEvent` callbacks, split-pane dashboard, `/verbose` toggles agent responses; team runs persisted to shared DB. Run-scoped workspace directories: each run creates `workspace/{run-uuid}/` with `.meta/` subdirectory for engine metadata
- **Run context:** When `--run-id` or `--last-run` is used, the TUI displays a styled context block at the top of the chat area showing the referenced run's metadata

## Wizards

`wizard.rs` — interactive dialoguer-based wizards for `agents create` and `teams create` with optional LLM-generated `soul.md`; `--no-interactive` flag skips wizard.

## Other `--format text|json|yaml` Commands

`agents validate`, `teams list`, `teams status`, `teams validate`, `skills list`, `skills validate`, `status`, `config list`, `memory` (bare), `memory people`, `memory commitments`, `memory preferences`, `memory events`, `memory search`, `mcp list`, `provider`, `model`, `tasks list`, `tasks get`, `reminders list`, `reminders get`, `webhook list-dead`, `webhook replay`, `webhook replay-all`, `kg status`, `kg list-agents`, `kg purge`, `kg validate`.

## Webhook CLI

`mika webhook list-dead` — list DLQ entries (pending + dead). Optional `--status` filter, `--limit` cap.
`mika webhook replay <delivery_id>` — replay a single dead entry.
`mika webhook replay-all` — replay all dead entries.

Requires `MIKA_GATEWAY_URL` (default: `http://localhost:3001`) and `MIKA_INTERNAL_TOKEN` for gateway auth.

## Skills CLI

`mika skills install/uninstall/update/list/validate/info` — skill architecture details in `crates/mika-agent/CLAUDE.md`.

`mika skills list` supports property filters (mika#606): `--source <bundle|marketplace>` filters by origin, `--always-on <true|false>` filters by activation state. Both are optional, AND semantics. Invalid `--source` values produce a clear error. HTTP equivalent: `GET /api/v1/skills?source=bundle&always_on=true`.

## Knowledge Graph CLI

`mika kg status` — show KG state summary across all agents (entity counts, chunk counts, last extraction, enabled flag, corpus grouping by `docs_root_hash`). Multi-corpus agents (e.g., mika-arch) display one row per corpus with per-corpus resolution counts (#877); agent name and enabled flag are shown on the first row only. `--agent X` filters to one agent. Supports `--format text|json|yaml`.

`mika kg list-agents` — quick enumeration of agents with KG state (agent name, enabled flag, `docs_root_hash`, chunk count). Supports `--agent X` filter and `--format text|json|yaml`.

`mika kg purge --agent X` — delete an agent's per-agent KG state (resolutions, resolution log). Interactive typed-ID confirmation (operator types the exact agent ID). `--yes` bypasses confirmation for scripting. `--include-orphaned-corpus` also deletes shared-corpus rows if no other agent references the same `docs_root_hash`. Non-TTY contexts require `--yes`. Supports `--format text|json|yaml`.

`mika kg validate` — check for orphan FK rows across KG tables and NULL `source_doc_hash` entries. Each check produces `[OK]`, `[WARN]`, or `[FAIL]` output. Exit 0 when all checks pass (Warn is acceptable), exit 1 on any Fail. Supports `--format text|json|yaml`.

Exit codes: `status`, `list-agents` always 0. `purge` returns 0 on success, 1 on cancellation or error. `validate` returns 0 iff no Fail checks, 1 otherwise.

See `crates/mika-agent/CLAUDE.md` for KG architecture and schema details.

## Logs CLI

Two subcommands; bare `mika logs` defaults to `paths` for backward compatibility.

`mika logs` / `mika logs paths` — Show resolved log file paths for an agent. Prints both the server log path (`MIKA_SPIRIT_LOG_FILE` or `/var/log/mika/server.log` fallback) and the per-agent CLI log path (`~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD`). Includes file existence, size, and a ready-to-use `jq` filter command for querying the server log by agent_id. Supports `--agent <name>` and `--format text|json|yaml`.

`mika logs activity` — Query cross-surface activity from SQLite (messages, LLM calls, tool calls, tasks) and render as a chronologically-interleaved timeline. Supports:
- `--since <expr>` — Time window start: `30m`, `2h`, `1d`, `today`, or ISO 8601. Default: `1h`.
- `--until <expr>` — Time window end (same format). Default: now.
- `--include <surfaces>` — Comma-separated surfaces to query. Default: `messages,llm_calls,tool_calls,tasks`. `server_log` validates but returns "not yet implemented" error.
- `--session <prefix>` — Filter by session ID (prefix match).
- `--task <id>` — Filter by task ID (resolves to sessions via `sessions.task_id`).
- `--trace <id>` — Filter by trace ID.
- `--grep <pattern>` — Filter events by substring match on content fields.
- `-n / --limit <N>` — Max events to display. Default: 200.
- `--agent <name>` and `--format text|json|yaml`.

See `crates/mika-agent/CLAUDE.md` § Log Sinks for the architectural rationale behind the two-sink design.

## MCP CLI

`mika mcp add/remove/list/enable/disable`, `--header KEY=VALUE`.
