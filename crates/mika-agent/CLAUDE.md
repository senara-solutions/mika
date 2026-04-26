# mika-agent — Agent Engine

Agent container: SQLite DB, agent loop, tools, prompt assembly, A2A server endpoints, HTTP server binary (mika-server). This is the core crate where most development happens.

## Agent Loop

Max 20 tool steps (all modes: conversation, callback, reminder, team), 5-minute total timeout, 30s default per-tool timeout (overridable via `Tool::timeout_secs()`). `LoopMode::Silent { max_steps }` carries per-trigger step limits via `SilentTrigger::max_steps()`. Step-awareness nudge injected at step `max_steps - 2` for all modes to encourage wrapping up. Silent mode nudge text is tailored for `send_message` notification.

On max-steps exceeded: continuation turn (tools disabled, 60s timeout) forces a text summary via shared `attempt_continuation_turn()` helper (used by Conversation, Team, and Silent modes); if that fails, structured fallback shows last 5 tool names with status. Silent mode continuation sends the summary via `message_sender` if available, prefixed with "[Background task exceeded tool step limit]".

Tool call summaries (name, truncated input/output, success, non_zero_exit) persisted in `messages.metadata` JSON column for cross-turn introspection (capped at `TOOL_METADATA_MAX = 4000` chars — tail entries dropped when exceeded, #744). The `tool_calls` DB table is the authoritative source; the dashboard's inline `ToolCallsTable` fetches from `GET /api/v1/traces/:trace_id/tool-calls` with metadata as fallback for pre-v15 messages. `MessageResponse` exposes `trace_id: Option<String>` to enable this lookup. `non_zero_exit` is set by heuristic detection of `Exit code:` / `Killed by signal:` prefixes from exec handlers; history builder tags these with `[NON-ZERO]` (distinct from `[FAILED]`). History builder appends `<context type="tool_history">` blocks to assistant messages.

Compaction includes tool names in summarization. Multi-modal tool results: `ToolOutput` carries optional `images: Vec<ImageData>` (base64-encoded), converted to multi-block `tool_result` content arrays for the Claude API. Prior-turn images are stripped before each API call to prevent unbounded memory growth.

**Per-turn tool_use dedup guard (#582):** `process_tool_calls()` deduplicates identical `(tool_name, arguments)` pairs emitted inside a single LLM response. The underlying tool runs once, the `tool_calls` DB row is saved once, one `ToolCallSummary` is emitted, and duplicate tool_use ids receive a `tool_result` built from the cached `ToolOutput` so the conversation/API history stays paired. Images on the cached result are stripped before reuse so the duplicate does not re-consume the shared `image_bytes_budget` (the LLM already received the images on the first duplicate's `tool_result`). Defends against provider-side duplication (observed with non-Anthropic providers). Logs `warn!` with `trace_id`, `tool`, `step`, and `cached_was_error` when it fires.

### Post-Conditions (EndTurn Chain)

Seven sequential post-conditions on assistant text responses, plus one early-accept:

1. **Text-based tool call detection:** `detect_text_based_tool_call()` catches XML-style patterns (`<function=...>`) that slip through `extract_xml_tool_calls()` in mika-common, re-prompts the LLM once.
2. **Prose-style tool call detection (#569):** `detect_prose_style_tool_call()` catches function-call-style prose patterns (`tool_name({"key": "val"})`) where the identifier matches a registered tool (builtins + skills + MCP). Gated against the tool set to avoid false positives on code examples. Single retry.
3. **Required-tools gate:** When keyword-matched skills declare `[constraints] required_tools`, the engine tracks tool calls across all steps; if required tools haven't been called, the response is rejected once. `filter_available_required_tools()` pre-filters against builtins + skill tools + MCP. Only `Keyword`-matched skills contribute constraints (#265). **Terminal failure bypass (#516):** `has_terminal_required_tool_failure()` checks `all_tool_summaries` for required tools that failed with known terminal errors (GitHub self-approval, HTTP 4xx, permission errors). When detected, the gate allows EndTurn without retry — the agent attempted the tool and hit an unrecoverable wall. `is_terminal_tool_error()` classifies output via `RETRYABLE_ERROR_PATTERNS` (checked first, takes priority) and `TERMINAL_ERROR_PATTERNS`. Unknown errors default to retryable (conservative).
3b. **PR review early-accept (#695, #821):** `has_successful_pr_review()` checks if `all_tool_summaries` contains a successful `run_gh` call with `"pr"` and `"review"` in the input. When true, guards #3 (required-tools, #821), #4–#7 are all skipped — the qa-review workflow's primary action completed and forced continuation would risk duplicate submissions. Defense-in-depth (two layers): (1) Session-scoped `pr_reviews_posted` map on `AppState` (`DashMap<String, HashSet<String>>`, #821) prevents duplicate reviews across turns within the same session — the primary defense, keyed by `(session_id, repo|pr_identifier)`. Entries evicted at `end_session()` callsites. (2) Per-turn `ToolContext.pr_review_posted` AtomicBool (#695) rejects duplicates within a single turn. Both guards reject `pr review` calls with structured `duplicate_pr_review` error. `make_pr_dedup_key()` derives the session-scope key from `gh pr review` arguments.
4. **Completion-claim guard (#483):** `detect_completion_claim()` detects completion-claim keywords (`merged`, `deployed`, `complete`/`completed`, `shipped`) in assistant text. If detected AND `update_task_status` is in the tool registry AND it was not called AND active tasks exist, the response is rejected once. Skips for delegates and team agents.
5. **Fabricated action-claim guard (#308):** `detect_fabricated_action_claim()` detects when the agent claims to have performed an action with a GitHub resource URL but made zero tool calls in the turn. Single retry.
6. **Intent-precondition registry (#702):** Registry-driven guard that generalizes the webhook zero-tools guard (#696). `INTENT_GUARDS` is a const array of `IntentPrecondition` entries, each with a trigger function, satisfaction check, and correction message. Retry tracking uses `HashSet<&'static str>` keyed by label (one retry per entry). Current entries: (a) `webhook_zero_tools` — if user message starts with `[GitHub]` and zero successful tool calls, rejects once (unchanged #696 behavior); (b) `resume_reconcile` — if user message contains resume/continue verb + milestone/project reference and no successful `check_task` or `list_tasks` call was made, rejects once. `detect_resume_intent()` requires both a verb (`resume`/`continue`) AND a process ref (`milestone#`/`project#`) to avoid false positives.
7. **Persistence evaluation guard (#648):** `detect_informational_input()` checks user input for informational signals (FYI, diagnostic, correction, status update) and `detect_persistable_output()` checks assistant text for verdict-shaped patterns (root cause, confirmed, validated, lesson learned). If no persistence write tool (`store_fact`, `update_fact`, `update_core_memory`) was called and either detection matches, nudges the model once to consider calling `store_fact`. Conversation mode only. Nudge, not rejection — the model can decline. `PERSISTENCE_WRITE_TOOLS` constant defines the write-tool set.

### Deterministic Context Injection

Skills with `[context.*]` sections have their data pre-fetched by the engine before the LLM turn. `resolve_contexts()` dispatches to engine-owned handlers by `context_type`, deduplicates across skills, and returns `ContextBlock`s. `apply_context_replacements()` performs single-pass `{{key}}` template substitution on skill prompts (injection-safe — replaced content is never re-scanned). If a `required = true` context fails, the declaring skill is excluded from the turn; if `required = false`, a sentinel message replaces the placeholder. Known types: `gh_pr_diff`. Module: `skills/context.rs`.

## Three-Layer Memory Model

- **Layer 1:** Core memory (always in system prompt, agent-editable via `update_core_memory` tool, 2500 token limit, 5 blocks: user_summary, self_model, current_priorities, key_people, workflows)
- **Layer 2:** Structured facts (People, Commitments, Preferences, Events — plaintext). Managed via `store_fact`, `update_fact`, `search_memory` tools.
- **Layer 3:** Hybrid search (FTS5 full-text + sqlite-vec cosine similarity via Reciprocal Rank Fusion). Optional OpenAI embeddings (text-embedding-3-small, 512 dims). Graceful degradation: hybrid -> FTS5-only -> LIKE fallback. Indexed on store_fact/update_fact, backfilled on startup.

## Tools

Each tool validates inputs. Control fields capped at `MAX_INPUT_LEN = 10_000` chars; payload fields capped at `MAX_PAYLOAD_BYTES = 200 * 1024` bytes (200 KB).

`ToolContext` contains `{ db, session_id, trace_id, home_dir, global_home_dir, core_memory_edit_count, is_onboarding, message_sender, embedding_client, brave_api_key, github_token, skills_dirty, is_reflection, is_task_context, is_callback_turn, provider_name, model_name, active_skill_paths, pr_review_posted, pr_reviews_posted }`. `global_home_dir: Option<&Path>` is `Some` for conversation mode, `None` for silent/team/delegate modes (blocks cross-agent file access). `active_skill_paths: &[SkillPathInfo]` lists skill prompt files already injected into the system prompt; populated in conversation mode, empty (`&[]`) in silent/team/investigate modes. `pr_reviews_posted: Option<&Arc<DashMap<String, HashSet<String>>>>` is `Some` in server mode (from `AppState`), `None` in CLI/test/silent/team modes — falls back to per-turn `pr_review_posted` AtomicBool (#821).

Tool trait uses `#[async_trait]` (Send futures). Per-tool timeout override via `timeout_secs()` default method (returns `None` -> uses 30s default). Shared `validate_and_resolve_path(path, base_dir, create_parents: bool)` helper in `tools/mod.rs` for path security (tilde expansion to base_dir, `~username` rejection, empty check, length limit, absolute rejection, traversal inspection, symlink check, canonicalize containment). Three-layer UUID validation chain in `tools/mod.rs` (#531, #596): (1) `validate_uuid(field_name, value)` — format-only via `Uuid::parse_str()`, returns `Result<Uuid, ToolOutput>` with structured JSON `{"error": "invalid_uuid", ...}`; (2) `validate_task_exists(db, field_name, value)` — format + DB existence + agent-scope, returns `Result<Task, ToolOutput>` with `{"error": "task_not_found", ...}` or `{"error": "db_error", ...}` (fail-closed); (3) `validate_task(db, task_id)` — calls `validate_task_exists` then layers trigger_type=manual + active status checks, returns `Option<String>`. Most task-accepting tools use layer 2 (`get_task`, `cancel_task`, `complete_task`, `update_task_status`, `check_task`); `delegate_task` and long-running dispatch use layer 3. `create_task` (`parent_task_id`) uses layer 1 only + `get_task_depth`.

**Cross-agent file access:** `read_agent_file`, `write_agent_file`, and `list_agent_files` accept an optional `agent` parameter for orchestrator-only cross-agent file access. `resolve_agent_home(agent_param, ctx)` helper validates permissions.

**Core memory path guard (#645):** `read_agent_file` rejects paths targeting core_memory sections with a domain-specific error before reaching `validate_and_resolve_path`. `is_core_memory_path(path)` matches `core_memory/` and `core-memory/` prefixes, bare section names (with or without `.md`), tilde/dot-prefixed variants, and exact directory names. Uses `core_memory_section_names()` from `db.rs` as single source of truth. The system prompt's core_memory preamble and tool-usage section also warn against reading core_memory via file tools (defense-in-depth).

**Context-redundancy guards (#647):** Pre-tool checks that detect when read tools request data already in the agent's context. Three guards extend the #645 pattern: (1) `read_agent_file` rejects paths matching active skill prompt files (`is_active_skill_prompt()` checks `ToolContext.active_skill_paths`); (2) `search_memory` hard-redirects `category="core_memory"` since core memory is always in the system prompt; (3) `search_memory` appends a soft hint when `category="all"` and the query matches a `core_memory_section_names()` entry. All guards use `ToolOutput::error()` for definitive redirects and hints prepended to success results for soft nudges. Path normalization shared via `normalize_path_prefix()` helper. Guard ordering: core_memory path guard → skill prompt guard → normal execution.

### Management Tools

14 tools for multi-agent/team workflows (`create_agent`, `list_agents`, `create_team`, `delete_team`, `update_team`, `add_team_member`, `remove_team_member`, `delegate_task`, `list_teams`, `run_team`, `get_team_status`, `get_team_history`, `create_task`, `update_task_status`). `create_agent`, `list_agents`, `create_team` always registered; others added when `agents.len() > 1 || !teams.is_empty()`. Orchestrator guards: only default agent or team-listed orchestrators can delegate/run teams; self-delegation blocked. **Task guard:** `delegate_task` and long-running skills require `task_id` referencing an active manual task. Per-tool timeouts: `run_team` (300s), `delegate_task` (120s).

**Delegate session persistence:** `delegate_task` creates a `delegate-{uuid}` session with parent linkage, persists task and response as messages. `AgentParams` has `global_home_dir` distinct from per-agent `home_dir`. **Team conversation continuity:** injects previous run context into orchestrator's system prompt. **Coverage check (#286):** `decompose()` re-prompts once if the orchestrator silently omits team members; falls through with `warn!` log (`team_coverage_gap`) on second miss. `TeamRun.coverage_retry_fired` bool persists via checkpoint JSON.

### Task Tracking

4 tools: **Write** (orchestrator-only): `create_task`, `update_task_status`. **Read** (all agents): `list_tasks`, `check_task` (with optional GitHub PR/issue status enrichment). Tasks reuse the `tasks` table with `trigger_type='manual'` + `action_type='none'`.

**`list_tasks` output enrichment:** Response includes a `Summary:` line with status-count breakdown (e.g., `"50 items total — 2 blocked, 48 completed"`) computed in-memory from the result set. Fully unfiltered calls also include a `Note:` with filter guidance discouraging redundant re-filtering. Filtered calls (by status or source) get a scoped summary but no guidance note. See #572.

**Status transition state machine:** `pending` -> any; `in_progress` -> blocked/completed/cancelled; `blocked` -> in_progress/completed/cancelled. Terminal states (`completed`, `cancelled`) cannot transition to a new status, but metadata can still be written (#617) — the tool applies metadata and returns success without changing status.

**Phantom retry guard (#579):** `update_task_status` rejects retry-semantic metadata writes (any top-level key containing "retry", case-insensitive) when the task has an active callback child task (`trigger_type="callback"` in `pending` or `in_progress` status). Returns structured JSON error `retry_metadata_rejected_active_dispatch`. Fail-open on `get_child_tasks` DB error — the dispatch readiness guard (#525) is the primary defense against re-dispatch. Non-retry metadata writes are unaffected.

**Idempotent creation:** Deduplicates on `reference_url` (DB partial unique index `idx_tasks_manual_active_ref_url`) and on label (case-insensitive pre-check). Five loop-prevention guards. Max 25 agent-created items per session (configurable via `max_agent_tasks_per_session`, user_request exempt).

**Note on renamed scheduled task tools:** The former scheduled-task tools `create_task` and `list_tasks` have been renamed to `create_scheduled_task` (in `create_scheduled_task.rs`) and `list_scheduled_tasks` (in `list_scheduled_tasks.rs`) to avoid name collisions with the task tracking tools above.

### PR Merge Gate

`pr_merge_with_gate` builtin tool — structural backstop against merging PRs with failing required CI checks. Registered in `default_tools()` (all agents, including delegates). Decision matrix: fail/cancel -> blocked; pending -> auto-merge; all pass -> immediate merge; already merged -> no-op. 60s timeout. Requires `ctx.github_token`. See #490.

### Issue Dependency Resolution

`resolve_issue_order` builtin tool — resolves dependency-aware execution order for a set of GitHub issues using `blockedBy` GraphQL edges. Input: `{ repo: "owner/repo", issues: [1, 2, 3] }`. For each issue, queries GitHub GraphQL API (via shared `github_graphql` module) for blocked-by relationships, builds a DAG, runs Kahn's algorithm with issue-number-ascending tiebreaker, and returns `{ sorted, edges, external_blockers, cycle }`. Cycle detection: if the DAG has a cycle, `sorted` is cleared and `cycle` lists the cycle members. External blockers (issues outside the input list) are tracked separately and do not affect the sort order. Fail-open: returns input order with a warning when no GitHub token is configured. 60s timeout. Used by the self-dev milestone workflow (M2b step) to order issues before dispatch. See #714.

**Shared `github_graphql` module:** `fetch_open_blockers()` and `extract_open_blocker_numbers()` extracted from `skills/executor.rs` into `crate::github_graphql` for reuse by both the blocked-by dispatch guard (#713) and `resolve_issue_order`.

### GitHub Read-Only Handler

`gh_read` builtin handler (#811, #817) — read-only GitHub CLI operations for the mika-arch architect agent. Input: `{"op": "<operation>", "target": "<number>", "repo": "owner/repo"}`. Five allowed ops: `issue_view` (→ `gh issue view --json`), `pr_view` (→ `gh pr view --json`), `pr_diff` (→ `gh pr diff`), `issue_list` (→ `gh issue list --json`, optional target as milestone number or label filter), `file_view` (→ `gh api /repos/{owner}/{repo}/contents/{path}?ref={ref}`, base64 decode, returns `{content, ref, path, size_bytes}`). Structured error variants: `NotFound`, `AuthFailed`, `RateLimited`, `NetworkError`, `MalformedRequest`, `FileTooLarge` (#817). `FileTooLarge` fires when GitHub returns empty content + non-zero size for files > `FILE_VIEW_MAX_BYTES` (1 MiB, matching GitHub's contents API cap). Files > 100 MiB hit GitHub's 403 response, classified as `AuthFailed` via existing `classify_gh_error()` — this is a pre-existing GitHub boundary, not a regression. `file_view` input validation: `path` charset-restricted to `[A-Za-z0-9._/-]` (prevents URL-decoding attacks), no `..`, no leading `/` or `-`; `ref` optional (defaults to `main`), no leading `-`, max 256 chars. Error classification via `classify_gh_error()` on `spawn_and_collect` output content prefix (`"Exit code:"` detection for non-zero exits). Audit log line `gh_read_invocation` with `agent_id`, `op`, `resource` (`<ref>:<path>` for file_view, target number for other ops), `repo`, `latency_ms`, `status`, `blob_sha` (file_view only — resolved file content sha from GitHub response, cost-free) fields. Auth reuses `ToolContext.github_token` (shared GitHub App installation). Declared in skill `tools.json` with `"handler": {"type": "builtin", "function": "gh_read"}`. See #811, #817.

### Structural Verdict Handler

`server::verdict_handler` — intercepts `pull_request_review.submitted` webhook events **before** the LLM turn in `handle_message`. Parses `VERDICT:` line from the review body. For `pass` verdicts with matching `in_progress` tasks: initiates merge via `run_gh_checks` + `run_gh_merge` (reused from `pr_merge_with_gate`), updates task metadata, logs `verdict_handled` audit event, sends notification. For `block[*]`/`hold[*]` verdicts: passes through to LLM. For missing `VERDICT:` line: passes through with `verdict_missing=true` enrichment. Parser in `server::verdict` depends on gateway's `format_event_text()` output format. 60s timeout on subprocess calls. See #524.

### Structural CI Success Handler

`server::ci_success_handler` — intercepts `check_suite.completed(success)` webhook events **before** the LLM turn. Companion to `verdict_handler`: re-evaluates merge eligibility for PRs that have a pending `VERDICT: pass` but were blocked on CI at approval time. Queries GitHub API for open PR, QA pass review, stale-SHA gate (`review.commit_id == pr.head.sha`), and CI aggregation via `run_gh_checks` + `classify_checks`. Reuses `VerdictAction` return type and `pr_merge_with_gate` helpers. Order-independent with `verdict_handler` — each handler self-selects on event type. 60s timeout on subprocess calls. See #571.

### Structural CI Failure Handler

`server::ci_failure_handler` — intercepts `check_suite.completed(failure|timed_out)` webhook events **before** the LLM turn. Failure-side companion to `ci_success_handler`. Matches CI failures to open PRs and existing work items, fetches failing-job context (up to 3 jobs, 100 lines each), and constructs a pre-digest instructing the LLM to dispatch `run_claude_pilot` for an autonomous fix. Circuit breaker: `ci_fix_count >= 2` in task metadata triggers escalation instead of dispatch — the handler increments `ci_fix_count` deterministically (not reliant on LLM). Checks both task-level callback children and global dispatch guard, including results in the pre-digest. Reuses `VerdictAction`, `find_open_pr`, `run_gh_checks`/`classify_checks`, and `has_active_callback_child` from sibling modules. Also fixes `CHECK_SUITE_RE` regex in `webhook_queue.rs` to match actual gateway format (was `Check suite (failure)`, corrected to `Check suite failure`). Order-independent with other handlers. 30s timeout per subprocess call. See #594.

### Webhook Deferral Queue

`server::webhook_queue` — in-memory queue that holds inbound GitHub webhooks when the target task has an in-flight `run_claude_pilot` callback (#528). Prevents race conditions where a webhook (e.g. `pull_request_review.submitted`) arrives before the callback persists metadata (`pr_url`). Correlation: PR URL via `parse_pr_review_event()`, branch via check_suite regex, fallback to sole-inflight-callback heuristic. 60s per-webhook timeout with forced replay. Drain triggers: callback completion in `handle_task_complete` (Ok path only), or timeout expiry via `drain_expired()`. Emits `webhook_deferred` and `webhook_replayed` audit events. Queue is in-memory only (lost on restart; GitHub supports redelivery). See #528.

### Introspection Tools

5 read-only tools: `query_timeline`, `get_session_messages`, `list_audit_events`, `search_tool_history` (30-day retention, 500-char field truncation, 10KB output cap), `query_knowledge_graph`. Non-orchestrator agents scoped to their own agent_id/sessions.

## Skills System

Git-based and local skill distribution via `mika skills install/uninstall/update`. Sources: git URLs, GitHub shorthand, local paths, `file://` URIs. Optional `--link` flag creates absolute symlinks. Four-tier origin: `[built-in]`, `[marketplace]`, `[marketplace/linked]`, `[custom]`. Tracks in `marketplace.lock` (TOML).

**Bundled skill sources (dual path, #598):** `seed_bundled_skills()` seeds two concatenated sources on startup — the hardcoded `BUNDLED_SKILLS` list (10 community-category skills: tmux, shell-exec, web-search, file-reader, self-knowledge, git-ops, google-workspace, github, mcp, browser-control — `include_str!`-embedded from `templates/skills/`) and the directory-sourced `ENTRIES` table generated at build time by `build.rs` walking `<workspace>/skills/bundled/*/`. `all_bundled_skills()` merges both with case-insensitive ENTRIES-wins-on-name-collision semantics. Production `skills/bundled/` contains 13 engine-coupled skills: self-dev, self-dev-webhook-qa, self-dev-webhook-ci, self-dev-sprint, qa-review, skill-review, claude-pilot, build-mika, deploy-mika, permission-policy, agents-teams, mika-arch-groom-ticket (#811), mika-arch-second-review (#811). Engine-coupled = correctness depends on staying in lockstep with Rust engine code (tool schemas, callback contracts, prompt-discipline rules encoded as Rust guards). `is_bundled_skill()` consults both sources. Trust-critical classification (`TRUST_CRITICAL_SKILLS`) stays hardcoded regardless of source. Shared build-time discovery helper lives at `build_support/bundled_skills_discover.rs` and is consumed by both `build.rs` and integration tests via `#[path = ...]` mod attributes.

**Dependency resolution:** BFS with cycle detection (max depth 10), same-source sibling resolution. `--link` propagates to deps from same source. `find_orphaned_deps()` + `--remove-deps` for cleanup.

**Per-provider and per-model variants:** Two-level directory hierarchy: `{provider}/` and `{provider}/{model}/`. `resolve_prompt(provider, model)` returns `ResolvedPrompt` with four-step fallback: hand-authored model variant -> generated model variant -> generated canonical variant -> root `system_prompt.md`.

**Per-skill LLM override:** DB-only via `skill_overrides` table (schema v20). `[llm]` section no longer supported in `skill.toml` (#504). `resolve_skill_llm_override()` constructs per-skill `LlmProvider`.

**Identity-driven skill allowlist (#811):** Well-known agents (mika-arch) can declare `[skills].allowlist` in `identity.toml` to own their skill set via identity rather than `skill_overrides` DB rows. `SkillsIdentityConfig` in `prompt.rs` deserializes the `[skills]` block with `allowlist: Option<Vec<String>>`. `SkillRegistry::apply_identity_allowlist()` runs as Phase -1 before `apply_overrides()` — evicts all skills NOT in the allowlist (case-insensitive), following the same `retain()` + `DisabledSkill` pattern as Phase 0. Warns on allowlisted names not found in the registry. Empty or absent allowlist = no-op (all skills active). Wired into all skill-loading paths: server init, hot-reload (handlers.rs, a2a.rs), team engine, delegate_task, list_skills tool, and CLI (chat.rs, ask.rs, skills.rs).

**Identity-driven tool denylist (#811):** Mirror of the skill allowlist for built-in tools. Well-known agents declare `[tools].disabled` in `identity.toml` listing tools that must not appear in their LLM tool array. `ToolsIdentityConfig` in `prompt.rs` parses the section. `agent::apply_agent_tool_visibility()` is the named filter hook — applied inside `inject_skills_and_resolve_tools` at the LLM-tool-array assembly site, before the tool defs are converted to `LlmToolDefinition`. The model never sees disabled tools, cannot call them, cannot be prompt-injected into trying. The shared `Arc<ToolRegistry>` is unchanged — filtering is per-agent at the presentation layer. mika-arch denies platform-mutational tools (skill mutations, config writes, file writes, task mutations, PR merge, cross-agent invocation, agent/team mutations) while keeping `send_message` and memory writes (`update_core_memory`, `store_fact`, `update_fact`) allowed — both are agent-scoped self-state, constitutive of being an agent, not platform side-effects (see `docs/architecture/review-guide.md` § Orthogonality). Future migration: extend to support `[tools].allowlist` for the symmetric well-known-agent shape.

**Fail-closed identity for well-known agents (#811):** `prompt::load_identity()` distinguishes well-known from user-defined agents on parse failure. For well-known agents (detected by matching the home_dir's last component against `find_well_known_agent`), a malformed `identity.toml` returns a fail-closed `Identity` with a sentinel allowlist (`["__fail_closed_no_skills__"]`) that matches no real skill, plus the full `MIKA_ARCH_DISABLED_TOOLS` denylist. The agent is effectively neutered until the operator fixes the file. For user-defined agents, parse failure logs `error!` and falls back to `Identity::default()` (current behavior — no security contract to preserve). The discrimination happens inside `parse_identity_or_fail_closed()`, so callers don't need to know the agent's well-known status.

**Skill enabled state:** DB-backed via `skill_overrides.enabled` column (schema v24, #629). Tri-state: `NULL` = default (enabled), `0` = disabled, `1` = explicitly enabled. `apply_overrides()` evicts disabled skills from `SkillRegistry.entries` into `disabled: Vec<DisabledSkill>` before applying `always_on`/LLM overrides. `enabled=false` always wins over `always_on=true` and over identity allowlist (DB override at Phase 0 runs after identity allowlist at Phase -1). `toggle_skill` agent tool and CLI `mika skills enable/disable` write to DB. Legacy `.disabled` marker files are migrated to DB rows on startup via `migrate_disabled_markers()` (one-shot, idempotent, fail-open). No match-time filter — disabled skills are evicted before matching (#630).

**Transient enable/disable overrides (#682):** Two methods handle per-invocation skill overrides, called after `apply_overrides()` (disable first, enable second — matches Phase 0/1 pattern): (1) `SkillRegistry::apply_transient_disable(skill_names)` evicts named skills from the registry entirely for a single CLI invocation. Returns `TransientDisableResult` with `not_found` list. Used by `mika ask --disable-skill <name>` (repeatable). (2) `SkillRegistry::apply_transient_always_on(skill_names)` sets `always_on = true` on named skills. Returns `TransientOverrideResult` with separate `disabled` and `not_found` lists. Cannot resurrect disabled (evicted) or skipped skills. Used by `mika ask --enable-skill <name>` (repeatable). Neither is persisted. Conflict check: same skill name in both flags produces a hard error before any registry ops.

**Oversized prompt handling (#630):** Skills with prompts exceeding their size limit are hard-skipped at scan time (pushed to `ScanResult.skipped`) regardless of `always_on` status. This prevents zombie skills with tools but no prompt context. Tool-only skills (no `system_prompt.md`) are unaffected — they load with an empty prompt via `SnippetLoadResult::Empty`.

**Startup logging:** `SkillRegistry::log_summary()` emits a three-state `INFO` line (`loaded=N disabled=N skipped=N`) plus per-skip `WARN` lines. Call after both `apply_overrides()` and `validate_loaded()` for accurate counts.

**Validation:** `validate_skill()` checks name-in-keywords rejection (#510), markdown validation (#511), required_tools references, context types, and `{{key}}` placeholders. **Startup validation (#530):** `SkillRegistry::validate_loaded()` runs `validate_skill()` on every loaded skill after `apply_overrides()`. Decision matrix: missing handler/broken tools.json → skip skill entirely; deprecated `[llm]` section/name-in-keywords/invalid markdown → load with warning. Results stored in `validated_warnings` for TUI/CLI display. `is_skip_worthy_failure()` classifies Fail diagnostics.

**Required tools enforcement:** Optional `[constraints]` section with `required_tools`. `collect_required_tools()` computes union across keyword-matched skills only. One retry on EndTurn violation.

**Match-reason conditioning (#265):** `match_skills()` returns `MatchedSkill` wrappers with `MatchReason` (`Keyword`, `AlwaysOn`, `Dependency`). `always_on` skills do not enforce constraints unless the user's message also triggered a keyword.

**Review-target exclusion (#513):** `review_filter::apply_review_filter()` runs after `match_message()` and before `resolve_contexts()` in both conversation and team mode. When `skill-review` is keyword-matched, any other keyword-matched skill whose name appears in the user message (case-insensitive) is excluded from the matched set. This prevents the reviewed skill's prompt from contaminating the review context. `AlwaysOn` and `Dependency` skills are never excluded. Silent mode is unaffected (no keyword matching).

## Exec Handlers

**Image protocol (`__mika_v1`):** Scripts return images via JSON envelope `{"__mika_v1": {"text": "...", "images": ["/path/to/img"]}}`. Executor validates files (5MB limit, magic-byte check for JPEG/PNG/GIF/WebP), base64-encodes, max 5 images per result.

**Long-running:** `long_running: true` + `estimated_duration_secs` in `skill.toml`. Conversation mode only. Creates callback task, injects `__mika_task_id` and `__mika_agent` env vars, spawns detached process. PID recorded for orphan cleanup. **Dispatch-readiness guard (#525):** before spawning, `validate_dispatch_readiness()` enforces five checks: (1) task status must be `pending` or `in_progress` (rejects `blocked`/`completed`/`cancelled` with structured JSON error `task_not_dispatchable`), (2) no active callback child task may exist (rejects with `task_active_dispatch`), (3) no other task may have an active callback child — global single-session-at-a-time guard (rejects with `global_dispatch_active`, scoped to `agent_id`) (#583), (4) per-turn dispatch counter must be zero — only one long-running dispatch per agent turn (rejects with `dispatch_limit_exceeded`) (#583), (5) GitHub `blockedBy` check — if the task's `reference_url` points to a GitHub issue, queries the GraphQL API for open blockers and rejects with `dispatch_blocked_by` if any are still open (#713). Fail-open when no `github_token` configured (check skipped with warning); fail-closed on API errors. Uses GraphQL variables (not string interpolation) for injection safety. `extract_open_blocker_numbers()` parses the response. `LongRunningContext` carries `dispatch_count: AtomicU32` initialized to 0 per turn; incremented after task creation and path validation, right before subprocess spawn. Fail-closed on DB errors. Auto-transitions `pending` tasks to `in_progress` on successful dispatch. Stricter than the shared `validate_task()` which also allows `blocked` for `delegate_task`.

## MCP (Model Context Protocol) Client

Connects to external MCP servers at startup via `McpManager`. Configured in `{agent_home}/mcp.json`. Supports stdio and Streamable HTTP transports. Tools namespaced as `mcp__{server}__{tool}`. Dispatch chain: builtins -> skills -> MCP -> unknown error. MCP tools excluded from silent/heartbeat mode. Child processes use `env_clear()` + allowlist.

## Evaluation — Golden Dataset (#339)

`tests/eval/golden/` — 25 curated end-to-end quality scenarios across 4 capability classes: Memory (8), Tool Selection (8), Conversation Quality (5), Skill-Specific (4). Each scenario has hard assertions (regression-gating) and optional soft tags (`quality:*` namespace, observability-only). Sibling tickets own their namespaces: #740 `self-knowledge:*`, #741 `grounding:*`.

**Three-tier execution model (D6):**
1. **Unit** — `MockLlmProvider`, runs on every CI push: `cargo test -p mika-agent --test eval golden`
2. **Integration** — real providers via `MIKA_EVAL_REAL_PROVIDERS` + `--ignored`
3. **Calibration** — integration + `MIKA_EVAL_CALIBRATE=1` artifact capture for weekly drift detection (#742)

**Scenario registration (D7):** Each scenario calls `register()` on `GoldenRegistry`. `HashMap::insert` uniqueness guard panics on duplicate names at test binary load time — protects against copy-paste-without-rename.

**Scoring (D4):** `GoldenOutcome` carries `hard_assertions: Vec<HardAssertion>` (pass/fail) + `soft_tags: Vec<SoftTag>` (LLM-judged quality signals). Judge model pinned to `claude-sonnet-4-6` with `MIKA_EVAL_JUDGE_MODEL` override. Tag-based judging — no free-form 0-10 scores. Judge-deprecation-as-reset protocol: when pinned model is EOL'd, baseline resets via explicit PR.

**Ticket-namespaced vocabulary (D4):** #339 owns `quality:concise`, `quality:verbose`, `quality:uncertain`, `quality:actionable`, `quality:off-topic`. Each sibling ticket defines its own namespace.

See `tests/eval/golden/README.md` for author-facing guidance (fixture patterns, assertion style, how to add scenarios).

## Evaluation — KG Provider Comparison (#762)

`tests/eval/kg_provider_eval/` — reproducible harness comparing LLM providers for the two KG call types (entity extraction and entity resolution). Uses direct LLM calls with the *production* KG prompts (not the full agent loop) so results reflect prompt-level provider behavior.

**Gating:** `#[ignore]` + `MIKA_EVAL_KG_PROVIDERS` env var (separate from `MIKA_EVAL_REAL_PROVIDERS` to avoid accidentally running during the basic provider matrix). Format: comma-separated `provider/model` strings (e.g. `anthropic/claude-haiku-4-5-20251001,openrouter/deepseek/deepseek-v3`) or `default` for the four-provider minimum set (Anthropic Haiku + Sonnet, OpenRouter DeepSeek + Kimi). Each referenced provider must have its API key set.

**Fixtures:** 15 sample docs (`docs/solutions/kg/eval-fixtures-2026-04-24/extraction_sample_docs.toml`) and 30 hand-labeled resolution ground-truth cases (`resolution_ground_truth.toml`). Scoring: extraction uses entity-set F1 + triple-set F1 against annotated expectations; resolution uses exact-match accuracy against the labeled correct candidate.

**Outputs:** per-run report with per-provider quality/cost/latency tables. Decision matrix lives at `docs/solutions/kg/kg-provider-evaluation-2026-04-24.md` (populated by running the eval with API keys). Compound pattern doc at `docs/solutions/best-practices/kg-provider-eval-harness-reproducible-comparison-2026-04-24.md`.

**Run:** `MIKA_EVAL_KG_PROVIDERS=default cargo test -p mika-agent --test eval -- --ignored --nocapture kg_provider_eval`

## Evaluation — Grounding Regressions (#741)

`tests/eval/grounding_regressions/` — 5 fabrication-detection scenarios from the KG milestone #14 retrospective. Each tests a concrete fabrication class with hard assertions (forbidden-word, required-tool, contains-in-order). No LLM-judge gating — each class has objectively checkable signals.

**Scenarios:** GraphQL field fabrication (#720), auto-merge vs merged (#727), core memory priority drift (#732), fabricated shell errors (feedback doc), KG result ignored (#740 D4).

**Assertion helpers:** `tests/eval/grounding_assertions/mod.rs` — `assert_response_forbids`, `assert_any_tool_called_from`, `assert_response_contains_in_order`, `assert_response_contains`, `assert_tool_called_before_response`, `assert_response_contains_question`.

**Frozen regression fixtures:** Each scenario has a `fixtures/{scenario}_pre_fix.json` file with the pre-fix response that demonstrates the failure class. Regression-reproduction tests prove assertions catch the failure.

**Tag vocabulary (`grounding:*`):** `fabricated-ref-suppressed`, `completion-claim-suppressed`, `source-cited-correctly`, `verification-before-claim`, `uncertainty-admitted`, `training-data-hallucination` (failure). Scope boundary with #740 `self-knowledge:*`: self-knowledge = query-invocation code paths; grounding = response-to-evidence paths.

See `tests/eval/grounding_regressions/README.md` for the full vocabulary, capability matrix, and how to add scenarios.

## Knowledge Graph — Domain Graph Builder

`src/kg/domain_builder.rs` — Deterministic startup-time builder that populates `kg_entities` and `kg_relationships` from four authoritative sources: `SkillRegistry`, `ToolRegistry`, `McpManager`, and agent configs. Runs once per server boot in `run_server()` after all agents are initialized. No LLM calls — pure code projection.

**Entity types:** Skill, Tool, Agent, ProblemType (5 seeds: `ci_failure`, `merge_conflict`, `duplicate_pr`, `stale_uuid`, `fabrication`). **Relationship types:** `DEPENDS_ON` (Skill→Skill from `skill.toml` dependencies), `PROVIDES` (Skill→Tool from skill tools).

**Sole-writer contract:** This module is the sole writer of `skill:*`, `tool:*`, `agent:*`, and `problem_type:*` entity_keys. No other code path writes these namespaces.

**Idempotency:** Entity UPSERT via `INSERT ... ON CONFLICT(entity_key) DO UPDATE` preserves rowids (protects `kg_subject_resolutions.domain_entity_id` FK references). Relationships are DELETE-all-then-INSERT per rebuild. Stale entities are pruned with a type-scoped DELETE that only touches `KG_DOMAIN_ENTITY_TYPES`.

**Failure policy:** Rebuild failures log `warn!` — the server continues to boot. KG queries return stale or empty results until the next successful rebuild. **Staleness contract:** Domain graph reflects registry state as of the last server boot.

**Observability:** Single `trace_id` per rebuild invocation, INFO-level structured logs (`domain_rebuild_start`, `domain_rebuild_entities`, `domain_rebuild_edges`, `domain_rebuild_complete`). No `audit_events` rows (per conventions C3.1).

**Cross-cutting conventions:** See `docs/architecture/kg-implementation-conventions.md` (C1–C3) and `docs/architecture/kg-id-convention.md` for the `<type>:<name>` entity key format.

## Knowledge Graph — v27 Migration Recovery

If a database restart between #786 and #787 deployments leaves the DB stuck at v27 with empty tables and the `v27_coalesce_complete` marker absent, see `docs/solutions/database-issues/kg-v27-stuck-migration-recovery-2026-04-24.md` for the operator recovery procedure.

## Knowledge Graph — Docs Root Configuration

`src/kg/config.rs` — Resolution chain for the docs root path the lexical ingestor reads (#738, #778). Two levels of resolution:

**Global fallback** (#738): `resolve_kg_docs_root(&Settings) -> (PathBuf, PathSource)`. Resolution order (first hit wins): `MIKA_KG_DOCS_ROOT` env var > `kg_docs_root` config.toml field > `<CWD>/docs/solutions` (container-native default).

**Per-agent resolution** (#778): `resolve_per_agent_docs_root(&Identity, &Settings) -> Result<KgAgentConfig, KgConfigError>`. Each agent's `identity.toml` gains a `[kg]` section:

```toml
[kg]
enabled = true                    # default: true
docs_root = "/absolute/path"      # optional; falls back to global chain above
```

**Behavior matrix:**

| `enabled` | `docs_root` set | Behavior |
|-----------|-----------------|----------|
| `true` (default) | set | Validate path exists as directory; hard-error if not; else use it with computed `docs_root_hash`. |
| `true` | unset | Fall back to global resolver. Hard-error on explicit env/config source if missing; warn-and-skip on CWD default. |
| `false` | any | Skip KG entirely (no `LexicalIngestor`, no `SubjectExtractor`, no `SubjectEntityResolver`). Existing shared-corpus rows preserved. |

**Types:** `KgAgentConfig` enum (`Disabled` / `Enabled { docs_root, docs_root_hash }`), `KgConfigError` via `thiserror`. Resolved at `init_agent` time, cached on `AgentState.kg_config`. Per-agent failure isolation: a single agent's KG misconfiguration skips that agent; others start normally.

**Hard-error policy:** Explicit paths (per-agent or global config/env) that don't exist fail loud. CWD-based default uses warn-and-skip per #738's policy. `enabled=false` does NOT delete existing rows — cleanup via #779's CLI.

For OpenRC hosts where the service starts with CWD ≠ repo root, set `MIKA_KG_DOCS_ROOT=/path/to/mika-repo/docs/solutions` in the service config, or use the existing `--chdir` init-script workaround.

### Multi-Corpus Support (#798)

Agents that need to reason across multiple repositories can specify an array of docs roots. The `mika-arch` agent is the canonical example — it indexes all six platform repos' `docs/solutions/` directories.

**Per-agent identity.toml recipe (mika-arch):**

```toml
name = "mika-arch"
emoji = "A"

[kg]
enabled = true
docs_roots = [
  "/data/workspace/mika-platform/mika/docs/solutions",
  "/data/workspace/mika-platform/mika-cloud/docs/solutions",
  "/data/workspace/mika-platform/mika-skills/docs/solutions",
  "/data/workspace/mika-platform/claude-pilot-py/docs/solutions",
  "/data/workspace/mika-platform/openclaw/docs/solutions",
  "/data/workspace/mika-platform/lettabot/docs/solutions",
]
```

**Precedence:** Per-agent `[kg].docs_roots` overrides the global `MIKA_KG_DOCS_ROOTS` env var, which overrides the singular `MIKA_KG_DOCS_ROOT` / `kg_docs_root` chain. If both `docs_root` (singular) and `docs_roots` (plural) are set in the same `[kg]` section, the plural form wins.

**Policy callouts:**

**a. Singular vs plural validation asymmetry:**

| Field | Missing path | Empty value |
|-------|-------------|-------------|
| `docs_root` (singular) | Hard-error if explicit; warn-and-skip if CWD default | Skips ingestion with distinct warn |
| `docs_roots` (plural) | Each path validated independently; missing paths logged as WARN and skipped; agent starts if at least one path is valid | Empty array treated as "not set" (falls back to singular chain) |

**b. Array order matters under budget pressure.** When `MIKA_KG_BATCH_BUDGET` is constrained, corpora are ingested in array order. If the budget is exhausted mid-array, remaining corpora are deferred to the next restart. Place the most important corpus first.

**c. Resolution chain priority (seven tiers, first match wins):**

| Priority | Source | Field |
|----------|--------|-------|
| 1 | Per-agent identity.toml | `[kg].docs_roots` (plural) |
| 2 | Per-agent identity.toml | `[kg].docs_root` (singular) |
| 3 | Environment variable | `MIKA_KG_DOCS_ROOTS` (colon-separated) |
| 4 | Environment variable | `MIKA_KG_DOCS_ROOT` |
| 5 | Config file | `kg_docs_roots` in config.toml (plural) |
| 6 | Config file | `kg_docs_root` in config.toml |
| 7 | CWD default | `<CWD>/docs/solutions` |

## Knowledge Graph — Subject Extractor

`src/kg/subject_extractor.rs` — Per-agent LLM-based extraction of named entities and fact triples from previously-ingested documents (#690). Uses constrained NER with approved entity/relationship types and structural validation (not just prompt-based).

**Approved entity types:** `skill`, `tool`, `agent`, `problem_type`, `solution_path`, `failure_mode`, `pattern`. **Approved relationship types:** `SOLVED_BY`, `USES`, `CALLS`, `INDICATES`, `PREVENTS`, `CAUSED_BY`, `MENTIONS` — each with from/to type constraints enforced in code.

**Sole-writer contract:** This module is the sole writer of `kg_subject_entities`, `kg_subject_relationships`, `kg_chunk_subjects`, `kg_chunk_subject_relationships`, and `kg_extractions` rows.

**Execution contexts (D2):** (1) Startup: background `tokio::spawn` per agent after lexical ingestion, non-blocking. (2) Compound hook: synchronous inline after doc write via `IngestionOrchestrator`, failure non-fatal (C2.3 log-and-skip).

**Extraction flow (D1, D4):** Read full doc from disk → insert `[CHUNK N]` markers at chunk boundaries → LLM call → parse/validate JSON output → UPSERT entities and relationships with provenance in single transaction.

**Re-extraction (D5):** Three-phase capture → reingest → reconcile. `IngestionOrchestrator` (`src/kg/ingestion_orchestrator.rs`) coordinates #689 and #690 — neither module calls the other directly. Scoped orphan sweep deletes entities/relationships that lost all provenance after doc change.

**Pending-doc detection (D7 + #757 hash check):** `kg_extractions` tracking table with `UNIQUE(agent_id, source_doc_path)` and a `source_doc_hash TEXT` column (nullable, added in v26). A doc is pending when it either has no `kg_extractions` row OR `kg_extractions.source_doc_hash != kg_chunks.source_doc_hash` — direct equality, no aggregation, because the lexical ingestor writes one identical per-doc hash across every chunk row. See `src/db/kg_schema.rs` for the full idempotency contract.

**Budget guard (#757):** `extract_pending(budget: u32)` caps per-batch LLM calls. `budget == 0` short-circuits with zero calls. On overflow, emits `kg_budget_exhausted` WARN with `scope="extraction"` and leaves remaining docs pending. Stats carry `aborted_budget: bool` + `llm_calls: u32`. Default budget: `MIKA_KG_BATCH_BUDGET` (500).

**LLM policy (C2):** Model from `MIKA_KG_EXTRACTION_MODEL` → `MIKA_KG_INGESTION_MODEL` fallback. Retry taxonomy per C2.2 (transport: 3 attempts with backoff; semantic: one retry with prompt reinforcement; config: no retry). Log-and-skip per C2.3. `llm_calls` rows per C2.4. Audit events per C3.3.

## Knowledge Graph — Entity Resolver

`src/kg/entity_resolver.rs` — Per-agent entity resolution that bridges subject graph entities to domain graph nodes (#691). Two-stage pipeline: exact match (case-insensitive) then LLM disambiguation for unresolved or ambiguous cases.

**Sole-writer contract:** This module is the sole writer of `kg_subject_resolutions` (subject → domain edges with confidence scores) and `kg_resolutions_log` (resolution tracking with outcome enum). No other code path writes these tables.

**Two-stage pipeline (D1):** Stage 1: case-insensitive exact match against `kg_entities.entity_key`. If match found and extraction confidence > 0.9, resolve immediately (confidence = extraction_confidence). Stage 2: LLM disambiguation with candidate list (max 50) and source chunk prose context. Combined confidence = min(extraction_confidence, llm_confidence). Discovered types (solution_path, failure_mode, pattern) skip resolution entirely — no domain counterpart exists.

**Execution contexts (D5):** (1) Startup: background `tokio::spawn` per agent after extraction tasks, non-blocking. (2) Compound hook: `IngestionOrchestrator` spawns async resolution after extraction commits. (3) `resolve_pending()` catches entities missed by either path.

**Pending-entity detection (D4):** `kg_resolutions_log` tracking table with `UNIQUE(agent_id, subject_entity_id)`. Pending query: subject entities with well-known types that have no log row, or whose `source_extraction_trace_id` differs from the latest `kg_chunk_subjects` extraction.

**Budget guard (#757):** `resolve_pending(budget: u32)` caps per-batch **Stage-2** LLM disambiguation calls. Stage-1 exact matches cost no LLM calls and are NOT debited against the budget — even `budget=0` lets exact matches resolve. On overflow, emits `kg_budget_exhausted` WARN with `scope="resolution"` and the remaining entities stay pending (no `kg_resolutions_log` row written). Stats carry `aborted_budget: bool` + `llm_calls: u32`. Default budget: `MIKA_KG_BATCH_BUDGET` (500).

**LLM policy (C2):** Model from `MIKA_KG_RESOLUTION_MODEL` → `MIKA_KG_INGESTION_MODEL` fallback. Mid-tier model recommended. Same C2.2 retry taxonomy as extraction. `no_match` is a first-class LLM response (not an error). `llm_calls` rows per C2.4. Per-batch audit events per C3.3.

**Failure policy:** Resolution failures are log-and-skip per C2.3. Failed entities stay pending for next startup's `resolve_pending`. No resolution model configured → exact-match-only mode with `outcome = 'skipped_no_llm'`.

## Knowledge Graph — Query Tool

`src/kg/query.rs` + `src/tools/query_knowledge_graph.rs` — Read-only graph traversal tool that lets agents discover capabilities, find solution paths, and reason about their environment (#688). Registered in `default_tools()`.

**Query modes:** Exactly one of `question` (free-text → entry path resolution) or `traversal.start` (known `entity_key` → direct traversal). `agent_id` enables agent-scoped context enrichment. `include_context` returns chunk prose text. `result_limit` caps output (max 20 entities, 30 edges, 10 chunks).

**Hybrid entry paths (D1):** Three parallel strategies find starting entities from a free-text question:
- **Path A** — Direct domain entity match: case-insensitive LIKE on `kg_entities.name`/`entity_key`. Confidence: 1.0 (exact) / 0.8 (LIKE).
- **Path B** — Subject entity match (agent-scoped): same against `kg_subject_entities`. Confidence scaled by extraction confidence.
- **Path C** — Semantic search via chunks: `hybrid_search(source_type="kg_chunk")` → `kg_chunk_subjects` → subject entities → resolve to domain via `kg_subject_resolutions` (substitutes domain entity when resolved, keeps subject when unresolved).
Results merged, deduped by `(layer, entity_id)` keeping highest confidence, top-K (5) as traversal starting points.

**Graph traversal (D2):** Recursive CTE over `kg_relationships` (domain) and `kg_subject_relationships` (subject). Delimiter-bounded cycle detection (`INSTR(',' || path || ',', ...)`). Default depth 2, cap 4. Default edge types: `SOLVED_BY`, `PROVIDES`, `DEPENDS_ON`, `USES`, `CALLS` (overridable via `follow`).

**Ranking (D3):** Lexicographic sort on `(hop ASC, cumulative_confidence DESC)`. Distance always dominates.

**Agent context (D5):** Annotate, don't filter. Skill entities enriched with `agent_context.enabled` (tri-state from `skill_overrides.enabled` via `COALESCE`). Non-skill entities have no context.

**Status values (D4):** `ok` (results found), `starting_entity_missing` (all entry paths failed), `traversal_empty` (entity found, no edges). Status enables #692 self-knowledge skill fallback logic.

## Knowledge Graph — Eval Fixture Seeding (#740)

`tests/eval/kg_fixtures/mod.rs` — crate-shared helpers for seeding a known KG state into a test `AsyncDatabase`. Used by `#740` (self-knowledge scenarios), `#741` (grounding scenarios), and `#787` (v27 migration invariant tests). Schema pin lives in the module (currently v27) with an actionable assertion message on drift. v26 fixture helpers (`V26_KG_DDL`, `V27_KG_DDL`, `DriftProfile`, `open_v26_for_coalesce()`, `build_v26_synthetic_db()`) support migration testing with realistic extraction drift simulation.

**Spec-struct pattern.** Each seed helper takes a `*Spec` struct and returns the inserted row ID: `seed_domain_entity`, `seed_domain_relationship`, `seed_subject_entity`, `seed_chunk`, `seed_chunk_subject`, `seed_resolution`, `disable_skill`. Query helpers (`get_resolution_log`, `get_resolution`) read rows back for assertions.

**FTS/vec parity.** `seed_chunk` writes to `kg_chunks` and calls `Database::index_content(agent_id, KG_CHUNK_SOURCE_TYPE, Some(chunk_id), text)` so `search_content` and `fts_search` stay in parity — Path C semantic retrieval depends on FTS5 being populated. A raw insert into `search_content` silently breaks Path C.

**Where scenarios live.** `tests/eval/kg_self_knowledge/` holds the seven #740 scenarios (one file per scenario, named `{class}_{shape}_{descriptor}.rs`). New scenarios: add the file, register it as `pub mod <name>;` in `kg_self_knowledge/mod.rs`, import `kg_fixtures::*`, call `assert_schema_version(&db).await` in setup, and update the capability-×-status matrix in the README. See `tests/eval/kg_self_knowledge/README.md` for the fixture table, tag vocabulary, and baseline scores.

## Silent Mode Agent Loop

Background tasks (heartbeat, reminders) where text output is NOT delivered. Agent must use `send_message` tool explicitly. Separate `run_silent_agent` function with `SilentPromptContext`.

**Trigger-aware skill selection:** `Heartbeat`, `Reflection`, `Reminder`, and `SkillRun` modes use `safe_always_on_skills()` which filters out exec/http-handler skills and does NOT resolve dependencies for security (autonomous triggers must not execute arbitrary commands). `Callback` mode uses `callback_safe_skills()` which preserves exec/http handlers AND resolves transitive skill dependencies via BFS (same algorithm as `match_skills()` in `matcher.rs`) — callback turns continue a tool call the agent already authorized in conversation mode, so retry/continuation workflows must have access to the same tool set (#567, #578). Loop-prevention guards in the long-running dispatch path prevent callbacks from spawning new unrelated long-running tasks.

**Task health awareness (heartbeat and callback):** `get_task_health_summary(agent_id)` detects 6 anomaly types and injects `<task-health>` block. Gated to `Heartbeat`, `Callback`, and `Reminder` triggers. Anomaly types: `stuck_callback` (completed but not delivered >10min), `failed_recurring`, `long_running` (in_progress >1h), `stale_blocked` (blocked >24h), `stale_pending` (pending >24h with no callback child — detects tasks created but never dispatched) (#583), `github_linked` (active items with GitHub PR URL).

## MessageSender Trait

`#[async_trait]` with `Send + Sync` bounds for `Arc<dyn MessageSender>`. Returns `Result<SendOutcome>` where `SendOutcome` is `Delivered` (gateway 2xx), `Failed { reason }` (non-2xx after retry, saved to `failed_sends`), or `NoChannel` (`chat_id == 0` sentinel — no reply channel available, e.g. GitHub webhook sessions). `Err` is reserved for infrastructure failures (chat_id resolution, DB errors). Text-only outbound. CLI prints to stdout. Server uses `GatewayMessageSender` (one retry after 2s, error classification: connection/timeout/HTTP status with body snippet). Team engine agents intentionally have `message_sender: None`. `NoopSender` (pub in `messaging.rs`) silently returns `Ok(Delivered)` — used to suppress user-facing notifications in team-child callback turns (#287) where the consolidated team-run notification handles delivery.

**`NoChannel` sentinel (#650):** `GatewayMessageSender::send()` detects `chat_id == 0` after `resolve_chat_id()` and returns `Ok(NoChannel)` before the HTTP POST — no retry, no `failed_sends` entry. `chat_id == 0` is the documented sentinel for sessions without a Telegram reply channel (GitHub webhooks, non-Telegram channels). The agent should use channel-appropriate tools (e.g., `run_gh`) instead of `send_message`.

**Callsite handling policy:** The `send_message` tool surfaces `Failed` as `ToolOutput::error` so the LLM knows delivery failed; `NoChannel` returns `ToolOutput::success` with redirect guidance (prevents LLM retry loops). The task-engine dispatcher absorbs `Failed` and `NoChannel` with a warning (fire-and-forget for scheduled sends). Server handlers and notification paths (verdict, CI success) log warnings on `Failed` and `NoChannel` but continue. The `failed_sends` flush path increments retry count on `Failed`; deletes entries on `NoChannel` (permanent condition).

## Conversation Compaction & Rewind

**Compaction:** Threshold-based (50 messages). Keeps 20 most recent, summarizes older via Claude API. Summary injected into system prompt.

**Rewind:** `rewind.rs` — two-phase flow: `preview_rewind()` then `execute_rewind()` with automatic reversal of memory/fact mutations via audit log. TUI: `/undo` (1 exchange), `/rewind [N | to <message_id>]`. Server: `POST /api/v1/rewind/{resolve,preview,execute}`.

## Unified Task Engine

`src/task_engine/` — single SQLite-backed scheduler. Min-heap + dedup set; 1-second tick loop; periodic DB scan (60 ticks). `TaskDispatcher` matches on `action_type`. `ensure_recurring_task()` idempotently registers heartbeat and reflection at startup.

**Callback/resume lifecycle:** agent creates callback task -> external process completes it -> server dispatches silent agent run with `SilentTrigger::Callback`. Loop prevention: callback turns cannot spawn new long-running tasks.

**SilentTrigger variants:** `Heartbeat`, `Reflection`, `Callback`, `SkillRun`, `Reminder`. Each produces correct system-prompt framing.

**Engine-level callback metadata extraction (#376):** `try_extract_callback_metadata()` parses structured fields from callback results and persists to parent task.

**Team task tree:** parent `invoke_orchestrator` task + child `resume_agent` tasks per delegation. Suspend/resume on pending grandchild callbacks. **Team-run user notification (#287):** fired once at terminal status from two symmetric callsites (`run_team` tool for sync completion, `dispatch_invoke_orchestrator` for async resume), both routing through `teams::notification::build_run_completion_message`. Per-child `resume_agent` callbacks have their user-facing `send_message` suppressed via `NoopSender`; the silent turn still runs (updates memory, records `llm_calls`) — only the user channel is gated. Deliverable text is UTF-8-safe truncated at 4000 chars (below Telegram's 4096 limit).

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

**Current: v28.** Tables: sessions, messages (with `internal` flag for agent-to-agent visibility), team_workspace, audit_events, skill_overrides (with `enabled` column for DB-backed disable state), tasks (with manual/callback/a2a trigger types and a `type` column distinguishing `issue`/`milestone`/`project`), a2a_task_map, a2a_artifacts, a2a_push_notification_configs, llm_calls, tool_calls, team_runs, schema_meta (migration state tracking), kg_entities, kg_relationships. **Shared-corpus KG tables (keyed by `docs_root_hash`):** kg_chunks, kg_subject_entities, kg_subject_relationships, kg_chunk_subjects, kg_chunk_subject_relationships, kg_extractions (first-writer-wins via INSERT OR IGNORE). **Per-agent KG tables:** kg_subject_resolutions, kg_resolutions_log, agent_kg_corpora (agent_id to docs_root_hash mapping for multi-corpus fan-out). `unified_timeline` VIEW for cross-subsystem queries. Session-based message storage with FK. System sessions (`system-{agent_id}`) for compaction.

Recent migrations:
- v18->v19: `sessions.task_id` column for reverse session->task lookups. `get_sessions_for_task_tree()`.
- v19->v20: `skill_overrides.llm_provider` and `skill_overrides.llm_model` for per-skill LLM override.
- v20->v21: `llm_calls.prompt_variant` for skill prompt variant recording.
- v21->v22: `messages.internal` column (`INTEGER NOT NULL DEFAULT 0`) for agent-to-agent message visibility. TUI inbox mode filters internal messages at the DB level. Set by `delegate_task` tool and by `mika ask --task-id` relay sessions (without `--task-complete`). `AgentParams.internal` threads the flag through `run_loop` to all message save paths.
- v22->v23: `tasks.type` column (`TEXT NOT NULL DEFAULT 'issue' CHECK (type IN ('issue', 'milestone', 'project'))`). Foundational for milestone/project dispatch (mika#595): `create_task` accepts an optional `type` parameter; `list_tasks` and `check_task` surface it. mika core stays a dumb task store — orchestration logic lives in self-dev (mika-skills#149). `NewTask.r#type: Option<String>` defaults to `'issue'` via SQL DEFAULT when `None`. Constants: `TASK_TYPE_ISSUE`/`TASK_TYPE_MILESTONE`/`TASK_TYPE_PROJECT`/`VALID_TASK_TYPES` in `db.rs`.
- v23->v24: `skill_overrides.enabled` column (`INTEGER`, nullable tri-state). `NULL` = default (enabled), `0` = disabled, `1` = explicitly enabled. Replaces `.disabled` marker files (#629). `SkillOverride.enabled: Option<bool>`. `set_skill_enabled()` with default-equals-delete (row deleted when all columns are NULL). `apply_overrides()` evicts disabled skills from `SkillRegistry.entries` into `disabled: Vec<DisabledSkill>`. One-shot `migrate_disabled_markers()` converts legacy `.disabled` marker files to DB rows at startup (fail-open on marker removal).
- v24->v25: Knowledge graph tables. Domain layer: `kg_entities`, `kg_relationships`. Lexical layer: `kg_chunks` + `search_content` integration. Subject layer: `kg_subject_entities`, `kg_subject_relationships`, provenance tables (`kg_chunk_subjects`, `kg_chunk_subject_relationships`), `kg_extractions` tracking. Resolution layer (#691): `kg_subject_resolutions` (subject → domain edges with confidence, UNIQUE on agent_id+subject_entity_id+domain_entity_id), `kg_resolutions_log` (resolution tracking with outcome CHECK constraint, UNIQUE on agent_id+subject_entity_id).
- v25->v26: `kg_extractions.source_doc_hash TEXT` (nullable) for #757 extraction idempotency. Pending-doc query now compares the stored hash against `kg_chunks.source_doc_hash` directly; pre-v26 rows get NULL and re-extract once under the budget before populating. Additive-nullable; no backfill needed. See `src/db/kg_schema.rs` → **Idempotency key** for the full contract.
- v27->v28: `agent_kg_corpora` table (#798) — maps `agent_id` to `docs_root_hash` for multi-corpus query fan-out. Populated by startup lexical ingest. Enables agents with `[kg].docs_roots` (plural) to query across multiple corpora.
- v26->v27: **Shared-corpus primary key** (#786 + #787). Six shared-layer KG tables (`kg_chunks`, `kg_subject_entities`, `kg_subject_relationships`, `kg_chunk_subjects`, `kg_chunk_subject_relationships`, `kg_extractions`) change primary-key scope from `agent_id` to `docs_root_hash` — a 16-hex-char SHA-256 prefix of `fs::canonicalize(docs_root)` computed by `kg::config::hash_docs_root`. Agents with the same `docs_root` now share a single corpus; extraction cost drops from N× to 1×. Per-agent tables (`kg_subject_resolutions`, `kg_resolutions_log`) FK-rewired but row-count preserved. `schema_meta` table added for migration state tracking. Two-phase migration: (1) DDL renames v26 tables to `*_v26_backup`, creates empty v27 tables (#786); (2) coalesce reads from backups, deduplicates via majority-vote (normalized `entity_key`, agent-count + mean-confidence + `MIN(id)` tiebreak), rewires FKs via temp lookup tables, drops backups, writes `v27_coalesce_complete` marker (#787). `v27_coalesce_sql()` is public for integration test access. `docs_root` resolved from `MIKA_KG_DOCS_ROOT` env var or CWD fallback at migration time. Startup guard refuses `Database::open()` when `schema_version == 27` and marker is absent. Recovery runbook: `docs/solutions/database-issues/kg-v27-stuck-migration-recovery-2026-04-24.md`.

Full migration history: see `docs/runtime-structure.md`.
