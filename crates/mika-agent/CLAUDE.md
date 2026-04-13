# mika-agent — Agent Engine

Agent container: SQLite DB, agent loop, tools, prompt assembly, A2A server endpoints, HTTP server binary (mika-server). This is the core crate where most development happens.

## Agent Loop

Max 20 tool steps (all modes: conversation, callback, reminder, team), 5-minute total timeout, 30s default per-tool timeout (overridable via `Tool::timeout_secs()`). `LoopMode::Silent { max_steps }` carries per-trigger step limits via `SilentTrigger::max_steps()`. Step-awareness nudge injected at step `max_steps - 2` for all modes to encourage wrapping up. Silent mode nudge text is tailored for `send_message` notification.

On max-steps exceeded: continuation turn (tools disabled, 60s timeout) forces a text summary via shared `attempt_continuation_turn()` helper (used by Conversation, Team, and Silent modes); if that fails, structured fallback shows last 5 tool names with status. Silent mode continuation sends the summary via `message_sender` if available, prefixed with "[Background task exceeded tool step limit]".

Tool call summaries (name, truncated input/output, success, non_zero_exit) persisted in `messages.metadata` JSON column for cross-turn introspection. `non_zero_exit` is set by heuristic detection of `Exit code:` / `Killed by signal:` prefixes from exec handlers; history builder tags these with `[NON-ZERO]` (distinct from `[FAILED]`). History builder appends `<context type="tool_history">` blocks to assistant messages.

Compaction includes tool names in summarization. Multi-modal tool results: `ToolOutput` carries optional `images: Vec<ImageData>` (base64-encoded), converted to multi-block `tool_result` content arrays for the Claude API. Prior-turn images are stripped before each API call to prevent unbounded memory growth.

### Post-Conditions (EndTurn Chain)

Four sequential post-conditions on assistant text responses:

1. **Text-based tool call detection:** `detect_text_based_tool_call()` catches patterns that slip through `extract_xml_tool_calls()` in mika-common, re-prompts the LLM once.
2. **Required-tools gate:** When keyword-matched skills declare `[constraints] required_tools`, the engine tracks tool calls across all steps; if required tools haven't been called, the response is rejected once. `filter_available_required_tools()` pre-filters against builtins + skill tools + MCP. Only `Keyword`-matched skills contribute constraints (#265).
3. **Completion-claim guard (#483):** `detect_completion_claim()` detects completion-claim keywords (`merged`, `deployed`, `complete`/`completed`, `shipped`) in assistant text. If detected AND `update_work_item_status` is in the tool registry AND it was not called AND active work items exist, the response is rejected once. Skips for delegates and team agents.
4. **Fabricated action-claim guard (#308):** `detect_fabricated_action_claim()` detects when the agent claims to have performed an action with a GitHub resource URL but made zero tool calls in the turn. Single retry.

### Deterministic Context Injection

Skills with `[context.*]` sections have their data pre-fetched by the engine before the LLM turn. `resolve_contexts()` dispatches to engine-owned handlers by `context_type`, deduplicates across skills, and returns `ContextBlock`s. `apply_context_replacements()` performs single-pass `{{key}}` template substitution on skill prompts (injection-safe — replaced content is never re-scanned). If a `required = true` context fails, the declaring skill is excluded from the turn; if `required = false`, a sentinel message replaces the placeholder. Known types: `gh_pr_diff`. Module: `skills/context.rs`.

## Three-Layer Memory Model

- **Layer 1:** Core memory (always in system prompt, agent-editable via `update_core_memory` tool, 2500 token limit, 5 blocks: user_summary, self_model, current_priorities, key_people, workflows)
- **Layer 2:** Structured facts (People, Commitments, Preferences, Events — plaintext). Managed via `store_fact`, `update_fact`, `search_memory` tools.
- **Layer 3:** Hybrid search (FTS5 full-text + sqlite-vec cosine similarity via Reciprocal Rank Fusion). Optional OpenAI embeddings (text-embedding-3-small, 512 dims). Graceful degradation: hybrid -> FTS5-only -> LIKE fallback. Indexed on store_fact/update_fact, backfilled on startup.

## Tools

Each tool validates inputs. Control fields capped at `MAX_INPUT_LEN = 10_000` chars; payload fields capped at `MAX_PAYLOAD_BYTES = 200 * 1024` bytes (200 KB).

`ToolContext` contains `{ db, session_id, trace_id, home_dir, global_home_dir, core_memory_edit_count, is_onboarding, message_sender, embedding_client, brave_api_key, github_token, skills_dirty, is_reflection, is_task_context, is_callback_turn, provider_name, model_name }`. `global_home_dir: Option<&Path>` is `Some` for conversation mode, `None` for silent/team/delegate modes (blocks cross-agent file access).

Tool trait uses `#[async_trait]` (Send futures). Per-tool timeout override via `timeout_secs()` default method (returns `None` -> uses 30s default). Shared `validate_and_resolve_path(path, base_dir, create_parents: bool)` helper in `tools/mod.rs` for path security (tilde expansion to base_dir, `~username` rejection, empty check, length limit, absolute rejection, traversal inspection, symlink check, canonicalize containment). Shared `validate_uuid(field_name, value)` helper validates UUID format via `Uuid::parse_str()` at the tool boundary, returning structured JSON errors (`{"error": "invalid_uuid", "field": ..., "received": ..., "reason": ...}`) to catch hallucinated/fabricated IDs before DB lookups. Applied to all tools accepting UUID-typed arguments: `get_task`, `cancel_task`, `complete_task`, `update_work_item_status`, `check_work_item`, `create_work_item` (`parent_task_id`), and `validate_work_item` (used by `delegate_task`). See #531.

**Cross-agent file access:** `read_agent_file`, `write_agent_file`, and `list_agent_files` accept an optional `agent` parameter for orchestrator-only cross-agent file access. `resolve_agent_home(agent_param, ctx)` helper validates permissions.

### Management Tools

12 tools for multi-agent/team workflows (`create_agent`, `list_agents`, `create_team`, `delete_team`, `update_team`, `delegate_task`, `list_teams`, `run_team`, `get_team_status`, `get_team_history`, `create_work_item`, `update_work_item_status`). `create_agent`, `list_agents`, `create_team` always registered; others added when `agents.len() > 1 || !teams.is_empty()`. Orchestrator guards: only default agent or team-listed orchestrators can delegate/run teams; self-delegation blocked. **Work item guard:** `delegate_task` and long-running skills require `work_item_id` referencing an active manual work item. Per-tool timeouts: `run_team` (300s), `delegate_task` (120s).

**Delegate session persistence:** `delegate_task` creates a `delegate-{uuid}` session with parent linkage, persists task and response as messages. `AgentParams` has `global_home_dir` distinct from per-agent `home_dir`. **Team conversation continuity:** injects previous run context into orchestrator's system prompt.

### Work Item Tracking

4 tools: **Write** (orchestrator-only): `create_work_item`, `update_work_item_status`. **Read** (all agents): `list_work_items`, `check_work_item` (with optional GitHub PR/issue status enrichment). Work items reuse `tasks` table with `trigger_type='manual'` + `action_type='none'`.

**Status transition state machine:** `pending` -> any; `in_progress` -> blocked/completed/cancelled; `blocked` -> in_progress/completed/cancelled. Terminal states cannot transition.

**Idempotent creation:** Deduplicates on `reference_url` (DB partial unique index `idx_tasks_manual_active_ref_url`) and on label (case-insensitive pre-check). Five loop-prevention guards. Max 5 agent-created items per session (user_request exempt).

### PR Merge Gate

`pr_merge_with_gate` builtin tool — structural backstop against merging PRs with failing required CI checks. Registered in `default_tools()` (all agents, including delegates). Decision matrix: fail/cancel -> blocked; pending -> auto-merge; all pass -> immediate merge; already merged -> no-op. 60s timeout. Requires `ctx.github_token`. See #490.

### Structural Verdict Handler

`server::verdict_handler` — intercepts `pull_request_review.submitted` webhook events **before** the LLM turn in `handle_message`. Parses `VERDICT:` line from the review body. For `pass` verdicts with matching `in_progress` work items: initiates merge via `run_gh_checks` + `run_gh_merge` (reused from `pr_merge_with_gate`), updates work item metadata, logs `verdict_handled` audit event, sends notification. For `block[*]`/`hold[*]` verdicts: passes through to LLM. For missing `VERDICT:` line: passes through with `verdict_missing=true` enrichment. Parser in `server::verdict` depends on gateway's `format_event_text()` output format. 60s timeout on subprocess calls. See #524.

### Webhook Deferral Queue

`server::webhook_queue` — in-memory queue that holds inbound GitHub webhooks when the target work item has an in-flight `run_claude_pilot` callback (#528). Prevents race conditions where a webhook (e.g. `pull_request_review.submitted`) arrives before the callback persists metadata (`pr_url`). Correlation: PR URL via `parse_pr_review_event()`, branch via check_suite regex, fallback to sole-inflight-callback heuristic. 60s per-webhook timeout with forced replay. Drain triggers: callback completion in `handle_task_complete` (Ok path only), or timeout expiry via `drain_expired()`. Emits `webhook_deferred` and `webhook_replayed` audit events. Queue is in-memory only (lost on restart; GitHub supports redelivery). See #528.

### Introspection Tools

4 read-only tools: `query_timeline`, `get_session_messages`, `list_audit_events`, `search_tool_history` (30-day retention, 500-char field truncation, 10KB output cap). Non-orchestrator agents scoped to their own agent_id/sessions.

## Skills System

Git-based and local skill distribution via `mika skills install/uninstall/update`. Sources: git URLs, GitHub shorthand, local paths, `file://` URIs. Optional `--link` flag creates absolute symlinks. Four-tier origin: `[built-in]`, `[marketplace]`, `[marketplace/linked]`, `[custom]`. Tracks in `marketplace.lock` (TOML).

**Dependency resolution:** BFS with cycle detection (max depth 10), same-source sibling resolution. `--link` propagates to deps from same source. `find_orphaned_deps()` + `--remove-deps` for cleanup.

**Per-provider and per-model variants:** Two-level directory hierarchy: `{provider}/` and `{provider}/{model}/`. `resolve_prompt(provider, model)` returns `ResolvedPrompt` with four-step fallback: hand-authored model variant -> generated model variant -> generated canonical variant -> root `system_prompt.md`.

**Per-skill LLM override:** DB-only via `skill_overrides` table (schema v20). `[llm]` section no longer supported in `skill.toml` (#504). `resolve_skill_llm_override()` constructs per-skill `LlmProvider`.

**Validation:** `validate_skill()` checks name-in-keywords rejection (#510), markdown validation (#511), required_tools references, context types, and `{{key}}` placeholders. **Startup validation (#530):** `SkillRegistry::validate_loaded()` runs `validate_skill()` on every loaded skill after `apply_overrides()`. Decision matrix: missing handler/broken tools.json → skip skill entirely; deprecated `[llm]` section/name-in-keywords/invalid markdown → load with warning. Results stored in `validated_warnings` for TUI/CLI display. `is_skip_worthy_failure()` classifies Fail diagnostics.

**Required tools enforcement:** Optional `[constraints]` section with `required_tools`. `collect_required_tools()` computes union across keyword-matched skills only. One retry on EndTurn violation.

**Match-reason conditioning (#265):** `match_skills()` returns `MatchedSkill` wrappers with `MatchReason` (`Keyword`, `AlwaysOn`, `Dependency`). `always_on` skills do not enforce constraints unless the user's message also triggered a keyword.

## Exec Handlers

**Image protocol (`__mika_v1`):** Scripts return images via JSON envelope `{"__mika_v1": {"text": "...", "images": ["/path/to/img"]}}`. Executor validates files (5MB limit, magic-byte check for JPEG/PNG/GIF/WebP), base64-encodes, max 5 images per result.

**Long-running:** `long_running: true` + `estimated_duration_secs` in `skill.toml`. Conversation mode only. Creates callback task, injects `__mika_task_id` and `__mika_agent` env vars, spawns detached process. PID recorded for orphan cleanup. **Dispatch-readiness guard (#525):** before spawning, `validate_dispatch_readiness()` enforces two checks: (1) work item status must be `pending` or `in_progress` (rejects `blocked`/`completed`/`cancelled` with structured JSON error `work_item_not_dispatchable`), (2) no active callback child task may exist (rejects with `work_item_active_dispatch`). Fail-closed on DB errors. Auto-transitions `pending` work items to `in_progress` on successful dispatch. Stricter than the shared `validate_work_item()` which also allows `blocked` for `delegate_task`.

## MCP (Model Context Protocol) Client

Connects to external MCP servers at startup via `McpManager`. Configured in `{agent_home}/mcp.json`. Supports stdio and Streamable HTTP transports. Tools namespaced as `mcp__{server}__{tool}`. Dispatch chain: builtins -> skills -> MCP -> unknown error. MCP tools excluded from silent/heartbeat mode. Child processes use `env_clear()` + allowlist.

## Silent Mode Agent Loop

Background tasks (heartbeat, reminders) where text output is NOT delivered. Agent must use `send_message` tool explicitly. Separate `run_silent_agent` function with `SilentPromptContext`. Heartbeat mode uses `safe_always_on_skills()` which filters out exec/http-handler skills for security.

**Task health awareness (heartbeat and callback):** `get_task_health_summary(agent_id)` detects 5 anomaly types and injects `<task-health>` block. Gated to `Heartbeat`, `Callback`, and `Reminder` triggers.

## MessageSender Trait

`#[async_trait]` with `Send + Sync` bounds for `Arc<dyn MessageSender>`. Text-only outbound. CLI prints to stdout. Server uses `GatewayMessageSender`. Team engine agents intentionally have `message_sender: None`.

## Conversation Compaction & Rewind

**Compaction:** Threshold-based (50 messages). Keeps 20 most recent, summarizes older via Claude API. Summary injected into system prompt.

**Rewind:** `rewind.rs` — two-phase flow: `preview_rewind()` then `execute_rewind()` with automatic reversal of memory/fact mutations via audit log. TUI: `/undo` (1 exchange), `/rewind [N | to <message_id>]`. Server: `POST /api/v1/rewind/{resolve,preview,execute}`.

## Unified Task Engine

`src/task_engine/` — single SQLite-backed scheduler. Min-heap + dedup set; 1-second tick loop; periodic DB scan (60 ticks). `TaskDispatcher` matches on `action_type`. `ensure_recurring_task()` idempotently registers heartbeat and reflection at startup.

**Callback/resume lifecycle:** agent creates callback task -> external process completes it -> server dispatches silent agent run with `SilentTrigger::Callback`. Loop prevention: callback turns cannot spawn new long-running tasks.

**SilentTrigger variants:** `Heartbeat`, `Reflection`, `Callback`, `SkillRun`, `Reminder`. Each produces correct system-prompt framing.

**Engine-level callback metadata extraction (#376):** `try_extract_callback_metadata()` parses structured fields from callback results and persists to parent work item.

**Team task tree:** parent `invoke_orchestrator` task + child `resume_agent` tasks per delegation. Suspend/resume on pending grandchild callbacks.

## HTTP Server (mika-server)

Axum-based with two auth layers: mutation endpoints require `MIKA_INTERNAL_TOKEN` only; read-only dashboard API accepts either `MIKA_DASHBOARD_TOKEN` or `MIKA_INTERNAL_TOKEN` (superuser).

**Mutation endpoints:** `/message` (202 async, 10MB limit), `/tasks/{id}/complete` (200 sync, 100KB cap), `/tasks/{id}/cancel` (200 sync), `/api/v1/rewind/*`, `/a2a/*`.

**Dashboard API:** `/api/v1/*` — timeline, agents, sessions, messages, traces, investigate, tasks (+ detail/children/sessions), team-runs (+ summary), llm-calls (+ detail), tool-calls (+ detail), github proxy endpoints. CORS scoped to `MIKA_CORS_ORIGIN`.

**Request logging:** `tower_http::trace::TraceLayer` middleware. `inject_request_meta` middleware copies method+path for top-level JSON fields. `/health` logged at DEBUG. Agent lock via `tokio::sync::Mutex<()>` with non-blocking `try_lock` (429 if busy).

**Failed sends flush:** Before each message processing, flushes up to 5 pending failed outbound sends from DB.

## Observability

"Always instrument, optionally export" pattern. Two orthogonal correlation axes: `trace_id` (per-request/per-turn, 32-char hex) + `session_id`/`agent_id` (system-level). `unified_timeline` VIEW enables cross-subsystem queries.

**Span filtering:** Per-layer `filter::Targets` on the OTel layer exports only `target: "mika::otel"` spans (LLM calls, agent turns, server requests).

**LLM observability:** `llm_call` spans with `gen_ai.*` semantic convention attributes, feature-gated behind `#[cfg(feature = "telemetry")]`. Team engine emits `TeamEvent` variants for live dashboard updates.

**Session lifecycle:** Silent dispatcher variants call `end_session()` after completion. CLI commands call `end_session()` on all exit paths. `startup_recovery()` prunes old sessions via `prune_old_sessions()`.

## Audit Log

`audit_events` table tracks all memory mutations per session. All writes include `trace_id`.

## Timestamps

All SQLite timestamp columns use ISO 8601 TEXT format (`%Y-%m-%dT%H:%M:%SZ`). The `crate::timestamp` module provides centralized helpers: `now()`, `format()`, `parse()`, `now_plus()`, `now_minus()`. Fixed-width UTC format ensures correct lexicographic ordering.

## Schema Version

**Current: v22.** Tables: sessions, messages (with `internal` flag for agent-to-agent visibility), team_workspace, audit_events, skill_overrides, tasks (with manual/callback/a2a trigger types), a2a_task_map, a2a_artifacts, a2a_push_notification_configs, llm_calls, tool_calls, team_runs. `unified_timeline` VIEW for cross-subsystem queries. Session-based message storage with FK. System sessions (`system-{agent_id}`) for compaction.

Recent migrations:
- v18->v19: `sessions.task_id` column for reverse session->task lookups. `get_sessions_for_task_tree()`.
- v19->v20: `skill_overrides.llm_provider` and `skill_overrides.llm_model` for per-skill LLM override.
- v20->v21: `llm_calls.prompt_variant` for skill prompt variant recording.
- v21->v22: `messages.internal` column (`INTEGER NOT NULL DEFAULT 0`) for agent-to-agent message visibility. TUI inbox mode filters internal messages at the DB level.

Full migration history: see `docs/runtime-structure.md`.
