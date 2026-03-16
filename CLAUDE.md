# Mika - AI Executive Assistant

## Project Overview

Mika is a conversation-first AI executive assistant with per-customer container isolation. Each customer gets their own agent container with SQLite storage. A shared gateway (`mika-gateway`) routes Telegram messages to the correct container.

**Current phase:** Phase 4 — Deployment infrastructure (Dockerfiles done, CI/CD done).

## Stack

- **Language:** Rust (edition 2024)
- **Agent engine:** Explicit Rust loop (no framework) — retrieve context → build prompt → LLM API → match stop_reason → execute tools or respond
- **LLM:** Multi-provider via `LlmProvider` trait — Anthropic (default, Claude Sonnet 4.6), OpenAI-compatible (OpenAI, Ollama, vLLM, Groq, Together). Provider selected by `llm_model` config with `provider/model` prefix (e.g., `openai/gpt-4o`, `ollama/llama3`). No prefix defaults to Anthropic.
- **Database:** SQLite via rusqlite (single DB per container at `~/.mika/data/mika.db`)
- **HTTP server:** Axum 0.8 (mika-server binary) with tower-http middleware
- **HTTP client:** reqwest 0.12 with rustls-tls (Claude API client with typed errors and retry)
- **Async runtime:** tokio
- **MCP client:** rmcp 0.17 (official Rust MCP SDK) — stdio and Streamable HTTP transports
- **Config:** config-rs with `MIKA_` env prefix + dotenvy for `~/.mika/.env` secrets
- **Logging:** tracing + tracing-subscriber (JSON for prod, pretty for dev) + optional OpenTelemetry export via `telemetry` feature flag
- **Telemetry:** opentelemetry 0.31 + tracing-opentelemetry 0.32, feature-gated OTLP HTTP export (Langfuse-compatible)
- **Dashboard:** React 19 + TypeScript + Vite + Tailwind CSS v4 + TanStack React Query + react-markdown/remark-gfm — observability dashboard SPA served separately, proxied to mika-server API

## Directory Structure

- `crates/mika-common/` — Shared library: config (config-rs with `MIKA_` prefix, `ConfigKeyInfo` registry with `ConfigBackend` enum, `get_effective_value`/`lookup_config_key` helpers), validation (`validation.rs` — API key format, file permissions, binary-in-PATH, config value validation), dotenv (`~/.mika/.env` load/read/write via dotenvy), Claude API client, logging, telemetry (feature-gated OTel export), home directory
- `crates/mika-agent/` — Agent container: SQLite DB, agent loop, tools, prompt assembly, HTTP server binary
- `crates/mika-gateway/` — Telegram webhook router: Postgres customer registry, message routing, pairing flow, outbound relay. `build.rs` forces recompilation when `migrations/` directory changes (prevents stale `sqlx::migrate!()` embeds from incremental compilation cache).
- `crates/mika-cli/` — TUI CLI binary (`mika`): ratatui chat interface, clap subcommands (status, memory, reminders, config, setup, mcp, skills, tasks, ask, doctor). `wizard.rs` — interactive dialoguer-based wizards for `agents create` and `teams create` with optional LLM-generated `soul.md`; `--no-interactive` flag skips wizard. `mika ask` sends a single non-interactive message; `--format text|json` (default `text`) controls output format — `json` emits `{"role":"assistant","content":"..."}` to stdout; `--task-id <uuid>` marks callback task complete and exits; `--session <id>` reuses an existing session; `--parent-task <id>` sets `is_task_context: true` for work item guard. 100KB result limit. Scoped flags: `--agent <name>` (override active agent, most subcommands), `--team <name>` (team mode, chat and ask, mutually exclusive with `--agent`). `mika ask --team <name> "goal"` runs the full team cycle non-interactively (progress to stderr, deliverable to stdout); `--format json` extends the schema with `team_run` metadata. `--run-id <uuid>` (requires `--team`) references a previous run's workspace as read-only context. Team mode streams `TeamEvent` callbacks, split-pane dashboard, `/verbose` toggles agent responses; team runs persisted to shared DB. Run-scoped workspace directories: each run creates `workspace/{run-uuid}/` with `.meta/` subdirectory for engine metadata (goal.md, assignments.md, critic_feedback.md, deliverable.md). TUI slash commands: `/think`, `/model`, `/agent`, `/undo`, `/rewind`. Shell-like Tab completion with context-aware argument completers. Multi-line input via Alt+Enter (primary) or Shift+Enter. Image paste (Ctrl+V), persistent per-agent input history, mouse scroll, click-drag text selection with clipboard copy, bracketed paste (100KB limit).
- `dashboard/` — React observability dashboard (Vite dev server on :5173, proxies `/api` to mika-server). Pages: Event Timeline, Agents, Sessions, Traces, Tasks, Team Runs. Auth via `VITE_MIKA_DASHBOARD_TOKEN` env var. Bearer token.
- `docs/` — Public documentation (architecture, configuration, deployment, runtime-structure, skills, slash-commands, getting-started) — **single source of truth** for all docs. See [docs/runtime-structure.md](docs/runtime-structure.md) for full `~/.mika` directory layout, DB schema, and log paths.
- `docs/adr/` — Architecture Decision Records (numbered)
- `docs/openapi/` — OpenAPI specs (mika-server.yaml, gateway.yaml)
- `scripts/` — Utility scripts (sync-agent-docs.sh for crates.io publish prep)
- `todos/` — Code review findings (tracked as markdown files)
- `.claude/commands/` — Claude Code slash commands (`/mika` — full dev workflow, `/mika-doc-audit` — standalone documentation audit, `/mika-issue` — create a single GitHub issue, `/mika-issues` — batch-create GitHub issues)

## Versioning

- **Pre-1.0 breaking changes:** Until v1.0, breaking changes do not require backward compatibility. They are shipped as minor or patch releases (no major version bump). PRs that introduce breaking changes must document the required manual migration steps in the PR description.

## Conventions

- **Error handling:** `anyhow::Result` for application code, `thiserror` for library errors (e.g., `ClaudeApiError`)
- **Naming:** snake_case for functions/variables, PascalCase for types, SCREAMING_SNAKE for constants
- **Edition 2024:** `unsafe` blocks required for `std::env::set_var` etc.
- **Testing:** `#[cfg(test)] mod tests` inline in each module, `cargo test` to run
- **No framework:** The agent loop is a plain Rust async function, not a framework
- **Doc sync:** `docs/` is the single source of truth. `crates/mika-agent/build.rs` copies docs into `OUT_DIR` at build time via `include_str!(concat!(env!("OUT_DIR"), ...))`. Crate-local copies in `crates/mika-agent/docs/` are fallbacks for crates.io; sync them with `scripts/sync-agent-docs.sh` before publishing.
- **Proactive state checking:** The system prompt instructs the agent to check existing state (via `list_reminders`, `search_memory`) before any write operation (reminders, facts, people, events). This prevents duplicates after conversation compaction. New write tools should have a corresponding query tool. The system prompt also instructs the agent to check its own files (`list_agent_files`, `read_agent_file`) and documentation (`get_documentation`) before answering questions about its own configuration or internals — reinforced by the `self-knowledge` skill's always-on prompt.
- **Confirmation before action:** The system prompt includes a guardrail instructing the agent to answer informational questions directly without starting multi-step workflows. If follow-up action may be useful, the agent suggests it and waits for confirmation.
- **Database:** Case-insensitive COLLATE NOCASE on unique text columns.
- **Secrets:** `Settings` has manual `Debug` impl that redacts API key and OTLP auth header. API key errors are opaque. Exec handler executor scrubs all MIKA_* env vars from child processes (defense-in-depth). Shell-exec and github handlers additionally `unset` specific vars in their scripts. MCP child processes use `env_clear()` + allowlist. Git subprocesses scrub MIKA_* vars and set `GIT_TERMINAL_PROMPT=0`.
- **Tools:** Each tool validates inputs (empty check + 10,000 char max). `ToolContext` contains `{ db, session_id, trace_id, home_dir, core_memory_edit_count, is_onboarding, message_sender, embedding_client, brave_api_key, skills_dirty, is_reflection, is_task_context, is_callback_turn }`. Tool trait uses `#[async_trait]` (Send futures, required for `tokio::spawn` in server handlers). Per-tool timeout override via `timeout_secs()` default method (returns `None` → uses 30s default). Shared `validate_and_resolve_path(path, base_dir, create_parents: bool)` helper in `tools/mod.rs` for path security (tilde expansion to base_dir, `~username` rejection, empty check, length limit, absolute rejection, traversal inspection, symlink check, canonicalize containment) — `create_parents: true` for write tools (creates dirs), `false` for read-only tools — used by `write_agent_file`, `write_workspace`, `read_workspace`, `read_agent_file`, `list_agent_files`. `write_agent_file` tool writes files to the agent's home directory with overwrite confirmation flow: if the target exists, the current content is returned and the agent must re-call with `confirm: true`. File tools (`write_agent_file`, `write_workspace`, `read_workspace`, `read_agent_file`, `list_agent_files`) report resolved absolute paths in success/error messages so the agent can verify the actual file location. `PromptContext.home_dir` surfaces the agent's home directory absolute path in the system prompt so the agent knows write_agent_file's base path.
- **Labels:** `.github/labels.yml` is the canonical label taxonomy (type, priority, component). All issue-creation paths reference it.
- **Async DB:** `AsyncDatabase` wraps sync `Database` with dedicated OS thread + `sync_channel(512)` mpsc channel (closure-based dispatch). Clone-able, Send+Sync. `with_db` releases the mutex before calling `send()` to avoid deadlocks. Integrated into agent loop, tools, and task engine.

## Commands

- `cargo build` — Build all crates
- `cargo test` — Run all tests (~957 tests)
- `cargo run --bin mika` — Run TUI CLI (default: chat, or `mika status`, `mika memory`, etc.)
- `cargo run --bin mika-server` — Run HTTP server (requires `MIKA_ROUTING_URL` and `MIKA_INTERNAL_TOKEN`)
- `VITE_MIKA_DASHBOARD_TOKEN=<token> npm run dev --prefix dashboard` — Run dashboard dev server (requires mika-server on :8080)
- `npm run build --prefix dashboard` — Build dashboard for production
- `cargo clippy` — Lint
- `cargo fmt` — Format
- `docker build -f Dockerfile.agent -t mika-agent:dev .` — Build agent container image
- `docker build -f Dockerfile.gateway -t mika-gateway:dev .` — Build gateway container image
- `docker compose up` — Run agent + gateway (add `--profile db` for local Postgres)
- `mika setup --mode compose` — Generate `.env` for docker-compose in current directory

## Architecture

- **One container per customer**
- **Three-layer memory model:**
  - Layer 1: Core memory (always in system prompt, agent-editable via `update_core_memory` tool, 2500 token limit, 5 blocks: user_summary, self_model, current_priorities, key_people, workflows)
  - Layer 2: Structured facts (People, Commitments, Preferences, Events — plaintext). Managed via `store_fact`, `update_fact`, `search_memory` tools.
  - Layer 3: Hybrid search (FTS5 full-text + sqlite-vec cosine similarity via Reciprocal Rank Fusion). Optional OpenAI embeddings (text-embedding-3-small, 512 dims). Graceful degradation: hybrid → FTS5-only → LIKE fallback. Indexed on store_fact/update_fact, backfilled on startup.
- **Agent loop:** Max 10 tool steps, 5-minute total timeout, 30s default per-tool timeout (overridable via `Tool::timeout_secs()`). Step-awareness nudge injected at step 8 (conversation mode only) to encourage wrapping up. On max-steps exceeded: continuation turn (tools disabled, 60s timeout) forces a text summary; if that fails, structured fallback shows last 5 tool names with status. Tool call summaries (name, truncated input/output, success, non_zero_exit) persisted in `messages.metadata` JSON column for cross-turn introspection. `non_zero_exit` is set by heuristic detection of `Exit code:` / `Killed by signal:` prefixes from exec handlers; history builder tags these with `[NON-ZERO]` (distinct from `[FAILED]`). History builder appends `<context type="tool_history">` blocks to assistant messages. Compaction includes tool names in summarization. Multi-modal tool results: `ToolOutput` carries optional `images: Vec<ImageData>` (base64-encoded), converted to multi-block `tool_result` content arrays for the Claude API. Prior-turn images are stripped before each API call to prevent unbounded memory growth.
- **Exec handler image protocol (`__mika_v1`):** Scripts return images via JSON envelope `{"__mika_v1": {"text": "...", "images": ["/path/to/img"]}}`. Executor validates files (5MB limit, magic-byte check for JPEG/PNG/GIF/WebP), base64-encodes, max 5 images per result. Used by file-reader skill.
- **Long-running exec handlers:** `long_running: true` + `estimated_duration_secs` in `skill.toml`. Conversation mode only (not silent/team). Creates a callback task, injects `__mika_task_id` and `__mika_agent` env vars (MIKA_* scrubbed), spawns detached process, returns immediately. Background monitor marks task failed on non-zero exit. PID recorded for orphan cleanup via `kill_orphan_processes()`. Team engine detects pending grandchild callbacks and suspends/resumes the run.
- **MCP (Model Context Protocol) client:** Connects to external MCP servers at startup via `McpManager`. Configured in `{agent_home}/mcp.json` (Claude Desktop convention). Supports stdio and Streamable HTTP transports (with custom headers for auth). Tools namespaced as `mcp__{server}__{tool}`. Dispatch chain: builtins → skills → MCP → unknown error. MCP tools excluded from silent/heartbeat mode. Child processes use `env_clear()` + allowlist + server-specific env, blocking MIKA_* overrides. Available in CLI ask/chat and server modes. Team agents do not yet support MCP. CLI: `mika mcp add/remove/list/enable/disable`, `--header KEY=VALUE`.
- **Management tools:** 10 tools for multi-agent/team workflows (`create_agent`, `list_agents`, `create_team`, `delete_team`, `update_team`, `delegate_task`, `list_teams`, `run_team`, `get_team_status`, `get_team_history`). `create_agent`, `list_agents`, `create_team` always registered; others added when `agents.len() > 1 || !teams.is_empty()`. Orchestrator guards: only default agent or team-listed orchestrators can delegate/run teams; self-delegation blocked. **Work item guard:** `delegate_task` and long-running skills require `work_item_id` referencing an active manual work item. Delegated agents get `default_tools()` only (no management tools). Per-tool timeouts: `run_team` (300s), `delegate_task` (120s). `AgentParams` has `global_home_dir` (e.g. `~/.mika/`) distinct from per-agent `home_dir`. **Team conversation continuity:** injects previous run context (goal, agent results, deliverable, critic feedback) into orchestrator's system prompt; skipped on first run.
- **Work item tracking:** 3 tools for tracking work: `create_work_item` (create trackable items with optional parent/source/reference_url), `update_work_item_status` (transition status: pending → in_progress → blocked → completed/cancelled), `list_work_items` (list with status/source filters). Work items reuse the `tasks` table with `trigger_type='manual'` + `action_type='none'`. Five loop-prevention guards: (1) no top-level creation from task context, (2) depth cap of 3, (3) callback turns block all creation, (4) deferred self_dev source, (5) max 5 agent-created items per session (user_request exempt). Active work items injected into heartbeat prompt with label sanitization (200 char truncation, `<>`stripping). Partial index `idx_tasks_manual_active` optimizes heartbeat queries.
- **Introspection tools:** 3 read-only tools for agent self-awareness: `query_timeline` (query unified timeline VIEW across messages/audit_events/tasks), `get_session_messages` (retrieve messages from past sessions), `list_audit_events` (list memory mutation audit events). All registered in `default_tools()`. Non-orchestrator agents are scoped to their own agent_id/sessions.
- **Skills marketplace:** Git-based skill distribution via `mika skills install/uninstall/update`. Scans for `skill.toml` (depth <= 2), tracks in `marketplace.lock` (TOML). GitHub shorthand (`user/repo`). Three-tier origin: `[built-in]`, `[marketplace]`, `[custom]`. ADR-006. Built-in `always_on` overrides stored in `skill_overrides` DB table (schema v7); `apply_overrides()` after `scan_skills_dir()`. One-level dependency resolution in `match_skills()`. `safe_always_on_skills()` does NOT resolve dependencies — prevents exec/http handlers in autonomous background contexts. `web-search` is `always_on = true` by default. CLI: `mika skills validate [--name <skill>]` for diagnostics.
- **Observability:** "Always instrument, optionally export" pattern. Two orthogonal correlation axes: `trace_id` (per-request/per-turn, 32-char hex) + `session_id`/`agent_id` (system-level). `trace_id` threaded through `ToolContext`, `LongRunningContext`, `TeamEngine`, and all DB write paths. HTTP server `request_id` flows into agent `trace_id` via `AgentParams.trace_id: Option<String>` — same `unwrap_or_else(generate_trace_id)` pattern as `SilentAgentParams` and `TeamAgentParams`. `unified_timeline` VIEW enables cross-subsystem queries by trace_id. `tracing` spans on agent loop, team engine, and server handlers. Feature-gated OTLP HTTP export; `TelemetryGuard` flushes on drop. Team engine emits `TeamEvent::PhaseChanged` and `TeamEvent::AgentStarted` for live dashboard updates. Team engine creates per-agent sessions (`team-{run_id}-{agent_name}`) with `INSERT OR IGNORE` for idempotency on resumed runs; sessions are ended when agent tasks complete. Silent dispatcher variants (heartbeat, reflection, callback, skill_run) call `end_session()` after completion. CLI commands (`ask`, `chat`) call `end_session()` on all exit paths (including agent errors and agent switches) so the dashboard shows session duration instead of perpetual "ongoing". `startup_recovery()` prunes ended system/silent sessions older than 7 days via `prune_old_sessions()` (cascade-deletes messages).
- **Typed Claude API errors:** `ClaudeApiError` enum with HTTP status-code retry (429/500/529)
- **Audit log:** `audit_events` table tracks all memory mutations per session (renamed from `memory_events` in schema v5). All audit log writes include `trace_id` for cross-subsystem correlation.
- **Conversation compaction:** Threshold-based (50 messages). Keeps 20 most recent, summarizes older via Claude API. Summary injected into system prompt (not message history). Runs inline post-turn in CLI.
- **Conversation rewind:** `crates/mika-agent/src/rewind.rs` — two-phase flow: `preview_rewind()` then `execute_rewind()` with automatic reversal of memory/fact mutations via audit log. Supports exchange count or message ID targeting, cross-session rewinds. Context marker injected post-rewind to prevent confabulation (accumulation guard removes prior markers). Audit events tagged with `rewound_by_trace_id`. TUI: `/undo` (1 exchange), `/rewind [N | to <message_id>]`. Server: `POST /api/v1/rewind/{resolve,preview,execute}` (internal token auth).
- **Silent mode agent loop:** Background tasks (heartbeat, reminders) where text output is NOT delivered. Agent must use `send_message` tool explicitly. Separate `run_silent_agent` function with `SilentPromptContext`. Heartbeat mode uses `safe_always_on_skills()` which filters out exec/http-handler skills (e.g., tmux, shell-exec) for security — only builtin-handler skills are available in autonomous background runs. Silent prompt conditionally includes `send_message` guidance only when a message sender is configured.
- **MessageSender trait:** `#[async_trait]` with `Send + Sync` bounds for `Arc<dyn MessageSender>`. Text-only outbound: `send(&self, text: &str)` — tool-produced images are consumed by the LLM for visual analysis (via `tool_result` content blocks) and are never forwarded to the end user. CLI prints to stdout. Server uses `GatewayMessageSender` (POST to gateway `/send` with retry + failed_sends fallback). `GatewayMessageSender` carries `agent_name: Option<String>` for multi-agent identification; the gateway prepends `[agent_name]` to outbound Telegram messages. `delegate_task` creates a delegate-specific sender with the delegate's agent name (not the orchestrator's) for correct attribution and reply routing. Team engine agents intentionally have `message_sender: None` — they communicate through the orchestrator pipeline, not directly to users.
- **Unified task engine:** `crates/mika-agent/src/task_engine/` — single SQLite-backed scheduler. Min-heap + dedup set; 1-second tick loop; periodic DB scan (60 ticks) picks up tool-created tasks (excludes callback and manual); periodic expiry + orphan process cleanup. `TaskDispatcher` matches on `action_type` ("send_message", "run_skill", "inject_context", "resume_agent", "invoke_orchestrator"). `ensure_recurring_task()` idempotently registers heartbeat (`0 0 * * * *`) and reflection (`0 0 2 * * *`) at startup. `startup_recovery()` expires timed-out tasks, kills orphans, marks orphaned in_progress as failed (skipping manual work items). Agent-busy retry with 30s delay and timeout guard. **Callback/resume lifecycle:** agent creates `trigger_type=callback` + `action_type=resume_agent` task → external process POSTs to `/tasks/{id}/complete` (server) or runs `mika ask --task-id <uuid>` (CLI) → server dispatches silent agent run with `SilentTrigger::Callback`; TUI polls every ~5s, injects result as `role='tool_result'`, runs agent with `is_callback_turn: true`. Loop prevention: callback turns cannot spawn new long-running tasks. Task lifecycle: `pending → completed → delivered`. Results wrapped in `<callback_result trust="untrusted">`. **SilentTrigger variants:** `Heartbeat`, `Reflection`, `Callback`, `SkillRun` (each produces correct system-prompt framing). **Sibling completion:** `try_complete_parent_on_sibling_done()` checks all children in terminal states and claims parent; uses `parent_task_id` linkage only (supports mixed agent_ids in team trees). **Team task tree:** parent `invoke_orchestrator` task + child `resume_agent` tasks per delegation. **Team suspend/resume:** pending grandchild callbacks cause suspension with versioned checkpoint; on completion, `dispatch_invoke_orchestrator` fires → `resume_team_run()` continues from next phase. `review_and_iterate()` handles critic rejection on all paths.
- **HTTP server (mika-server):** Axum-based with two auth layers: mutation endpoints (`/message`, `/tasks/{id}/complete`, `/api/v1/rewind/*`) require `MIKA_INTERNAL_TOKEN` only (gateway-to-agent traffic); read-only dashboard API (`/api/v1/*` — timeline, agents, sessions, messages, traces, investigate, tasks, team-runs, team-runs/:id/summary) accepts either `MIKA_DASHBOARD_TOKEN` or `MIKA_INTERNAL_TOKEN` (superuser). `/message` has 202 async + 10MB body limit; `/tasks/{id}/complete` has 200 sync + 100KB result cap. Dashboard routes have CORS scoped to `MIKA_CORS_ORIGIN` (default `http://localhost:5173`) with GET+POST+OPTIONS (POST for investigate). `/health` is unauthenticated for probes. `AppState` is Clone via Arc-wrapped deps. Agent lock (`tokio::sync::Mutex<()>`) serializes agent loops with non-blocking `try_lock` (429 if busy).
- **Failed sends flush:** Before each message processing, flushes up to 5 pending failed outbound sends from DB.
- **Schema version:** 11 — sessions + messages two-table pattern, `team_workspace`, `audit_events` (renamed from `memory_events`), `skill_overrides`, work item support in `tasks`. `unified_timeline` VIEW for cross-subsystem queries (includes `team_workspace` union; task leg uses `COALESCE(execution_trace_id, created_trace_id)` for accurate trace correlation). `team_runs.trace_id` for resume continuity. Session-based message storage with FK to sessions. System sessions (`system-{agent_id}`) for compaction. v1→v3 and v2→v3 are clean-slate migrations. v9→v10: `ALTER TABLE team_runs ADD COLUMN trace_id TEXT`, recreate `unified_timeline` VIEW with `team_workspace` UNION ALL. v10→v11: `ALTER TABLE tasks ADD COLUMN execution_trace_id TEXT` (with partial index `idx_tasks_execution_trace`), `ALTER TABLE sessions ADD COLUMN parent_session_id TEXT` (with partial index `idx_sessions_parent`), recreate `unified_timeline` VIEW with `COALESCE(execution_trace_id, created_trace_id)` for the task leg. See [docs/runtime-structure.md](docs/runtime-structure.md) for full schema and migration history.
- **mika-gateway** (`crates/mika-gateway/` in this repo)**:** Telegram webhook router with Postgres customer registry. Handles text messages and images. Endpoints: `/webhook/telegram` (inbound), `/send` (outbound relay with `agent_name` identification), `/health` + `/readyz` + `/livez` (health probes). Env-var-only config. **Agent identification:** outbound messages carry `agent_name` in the `/send` payload; gateway prepends `[agent_name]` to Telegram text and stores `(telegram_message_id, chat_id, agent_name)` in `outbound_messages` Postgres table. **Reply routing:** parses `reply_to_message` from Telegram updates; looks up the originating agent via `outbound_messages` and forwards the inbound message with `"agent": "<name>"` to the correct agent in the container. **Periodic cleanup:** purges `outbound_messages` older than 7 days (batched, every ~100 webhooks). Gateway migration 002 creates the `outbound_messages` table. `build.rs` ensures `cargo::rerun-if-changed=migrations` so new migration files invalidate the incremental compilation cache (SQLx `migrate!()` is a compile-time proc macro).
- **Docker images:** Multi-stage builds with BuildKit `--mount=type=cache` for cargo registry and target dir caching. Builder: `rust:1.93-slim`. `Dockerfile.agent` (95MB) for per-customer containers (runtime deps: ca-certificates, wget, file, jq, gh). `Dockerfile.gateway` for the stateless gateway (leaner: ca-certificates + wget only, no home dir). Both use rustls (no OpenSSL build deps). Both run as non-root user `mika` (UID 1000). Release profile: LTO + strip. `docker-compose.yml` defines agent, gateway, and postgres services (`db` profile for postgres). **Host dependency:** `jq` is required by all skill handler scripts (shell-exec, tmux, github, file-reader) for JSON input parsing; handlers fail with a clear error if `jq` is not found.
- **CI/CD:** Three GitHub Actions workflows: `ci.yml` (PR checks: fmt, clippy, test, test with `--features telemetry`), `release-plz.yml` (automated versioning, changelog, crates.io publishing, git tagging via conventional commits), `release.yml` (cross-platform binary builds on tag push: x86_64/aarch64 Linux + macOS). All actions pinned to commit SHAs. Binaries published to GitHub Releases with SHA256 checksums. Installer script: `install.sh`.

## Environment Variables

See `.env.example` for the full list. Required:
- `MIKA_ANTHROPIC_API_KEY` — Anthropic API key or OAuth subscription token. Auto-detected from prefix: `sk-ant-oat*` → OAuth bearer auth, otherwise → standard API key auth.

Optional (Layer 3 vector search):
- `MIKA_OPENAI_API_KEY` — OpenAI API key for embedding generation (enables vector similarity in hybrid search)

Optional (web search):
- `MIKA_BRAVE_API_KEY` — Brave Search API key for `web_search` builtin skill (get free key at https://brave.com/search/api/)

Optional (investigation panel — GitHub issue creation):
- `MIKA_INVESTIGATE_GITHUB_TOKEN` — GitHub Personal Access Token for investigation panel issue creation (needs `repo` scope for private repos, `public_repo` for public)
- `MIKA_GITHUB_REPO` — Target repository in `owner/repo` format (e.g. `senara-solutions/mika`). Both must be set to enable the `create_github_issue` investigation tool.

Server mode additionally requires:
- `MIKA_ROUTING_URL` — Gateway URL for outbound message delivery
- `MIKA_INTERNAL_TOKEN` — Shared secret for Bearer auth between gateway and agent

Optional (startup behavior):
- `MIKA_DISABLE_BUNDLED_SKILLS` — Skip bundled skill re-sync on startup (default: false). WARNING: do not enable in production — prevents security updates to handler scripts.

Optional (telemetry — requires `--features telemetry` build):
- `MIKA_TELEMETRY_ENABLED` — Enable OpenTelemetry trace export (default: false)
- `MIKA_OTLP_ENDPOINT` — OTLP HTTP endpoint URL with `/v1/traces` path (e.g. `https://cloud.langfuse.com/api/public/otel/v1/traces` or `http://localhost:4318/v1/traces`)
- `MIKA_OTLP_AUTH_HEADER` — OTLP auth header value (Base64-encoded credentials for Langfuse)

Optional (dashboard):
- `MIKA_CORS_ORIGIN` — Allowed origin for dashboard CORS (default: `http://localhost:5173`). Only applies to `/api/v1/*` dashboard routes.
- `MIKA_DASHBOARD_TOKEN` — Separate bearer token for read-only dashboard API routes (`/api/v1/*`). If unset, dashboard routes accept `MIKA_INTERNAL_TOKEN` (backwards compatible). `MIKA_INTERNAL_TOKEN` is always accepted on all routes (superuser). Dashboard frontend uses `VITE_MIKA_DASHBOARD_TOKEN` env var.

Optional (log format and files):
- `MIKA_LOG_FORMAT` — Stdout log format for mika-server and mika-gateway: `json` (default) or `pretty` (human-readable, for local dev). CLI always uses pretty.
- `MIKA_SERVER_LOG_FILE` — File path for mika-server log output (always JSON regardless of `MIKA_LOG_FORMAT`)

Gateway mode (`mika-gateway` binary) additionally requires:
- `MIKA_DATABASE_URL` — Postgres connection string
- `MIKA_TELEGRAM_BOT_TOKEN` — Telegram Bot API token
- `MIKA_TELEGRAM_WEBHOOK_SECRET` — 64-char hex secret for webhook validation
- `MIKA_TELEGRAM_WEBHOOK_URL` — Public HTTPS URL for Telegram webhook delivery
- `MIKA_INTERNAL_TOKEN` — Shared 64-char hex bearer token (same as server mode)

## Pending Work

- **Deployment:** Production deployment guide, Docker image CI, Kubernetes/cloud manifests
- **Future features:** WhatsApp channel adapter, morning briefings, admin API

## Reference Repositories

Local clones of agent platforms to study for patterns and inspiration. Read freely when designing Mika features.

- **OpenClaw** — `../openclaw/`
  TypeScript monorepo. Study for: channel adapter architecture (hub-and-spoke gateway), skills system (Markdown/YAML definitions), multi-channel UX patterns, community marketplace model.

- **LettaBot** — `../lettabot/`
  TypeScript. Study for: memory hierarchy patterns (core/archival/recall from MemGPT), autonomous memory self-editing via tool calls, agent state persistence, channel integrations built on top of Letta's memory API.
