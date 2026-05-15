# mika-agent — Agent Engine

Agent container: SQLite DB, agent loop, tools, prompt assembly, A2A server endpoints, HTTP server binary (mika-server). This is the core crate where most development happens.

## Agent Loop

Max 20 tool steps (all modes: conversation, callback, reminder, team), 5-minute total deadline, 30s default per-tool timeout (overridable via `Tool::timeout_secs()`). `LoopMode::Silent { max_steps }` carries per-trigger step limits via `SilentTrigger::max_steps()`. Step-awareness nudge injected at step `max_steps - 2` for all modes to encourage wrapping up. Silent mode nudge text is tailored for `send_message` notification.

**Deadline enforcement (#848, #939):** the 5-min total budget is enforced via an `Instant`-based deadline checked at five points: (1) top of each `run_loop` step iteration, (2) end of prelude work in each inner function before entering `run_loop`, (3) before `attempt_continuation_turn` entry — skip when `now + CONTINUATION_TIMEOUT_SECS > deadline`, (4) inside `attempt_continuation_turn` itself, where the inner timeout is clamped to `min(60s, deadline - now)`, (5) inside the LLM transport retry loop — `send_message_with_deadline()` aborts the retry chain when remaining budget < `TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS` (120s), preventing doomed retries from consuming the deadline (#939). The provider's per-request 120s `reqwest` timeout (`crates/mika-common/src/claude.rs`) is the sole cancellation mechanism for in-flight HTTP calls — the outer agent deadline never drops a future mid-flight, so the `llm_calls` row is always persisted (success or transport-timeout). Worst-case turn duration is `300s + 120s = 420s`. `LoopResult` is a three-variant enum (`Done`/`MaxStepsExceeded`/`DeadlineExceeded`) without `#[non_exhaustive]` — the compiler's match-exhaustiveness check enforces that all three outer handlers (conversation, silent, team) handle every variant. CI lint guard `scripts/check-loop-select.sh` rejects `tokio::select!` inside `run_loop`'s body (would shadow the iteration-top deadline check).

On max-steps exceeded: continuation turn (tools disabled, deadline-clamped timeout, ceiling 60s) forces a text summary via shared `attempt_continuation_turn()` helper (used by Conversation, Team, and Silent modes); the helper persists an `llm_calls` row in all outcomes (success/error/timeout) so the continuation is never the silent-drop variant of the in-flight-cancel bug at smaller scale. If continuation fails or is skipped (deadline too close), structured fallback shows last 5 tool names with status. Silent mode continuation sends the summary via `message_sender` if available, prefixed with "[Background task exceeded tool step limit]".

**Test-only entry points (`run_*_with_deadline`):** publicly visible by naming convention, used only by `EvalHarness` to inject short deadlines for the deadline-during-LLM-call eval scenarios. Production callers should always use `run_agent`/`run_silent_agent`/`run_team_agent`. `AgentParams` carries no deadline knob.

Tool call summaries (name, truncated input/output, success, non_zero_exit) persisted in `messages.metadata` JSON column for cross-turn introspection (capped at `TOOL_METADATA_MAX = 4000` chars — tail entries dropped when exceeded, #744). The `tool_calls` DB table is the authoritative source; the dashboard's inline `ToolCallsTable` fetches from `GET /api/v1/traces/:trace_id/tool-calls` with metadata as fallback for pre-v15 messages. `MessageResponse` exposes `trace_id: Option<String>` to enable this lookup. `non_zero_exit` is set by heuristic detection of `Exit code:` / `Killed by signal:` prefixes from exec handlers; history builder tags these with `[NON-ZERO]` (distinct from `[FAILED]`). History builder appends `<context type="tool_history">` blocks to assistant messages.

Compaction includes tool names in summarization. Multi-modal tool results: `ToolOutput` carries optional `images: Vec<ImageData>` (base64-encoded), converted to multi-block `tool_result` content arrays for the Claude API. Prior-turn images are stripped before each API call to prevent unbounded memory growth.

**Per-turn tool_use dedup guard (#582):** `process_tool_calls()` deduplicates identical `(tool_name, arguments)` pairs emitted inside a single LLM response. The underlying tool runs once, the `tool_calls` DB row is saved once, one `ToolCallSummary` is emitted, and duplicate tool_use ids receive a `tool_result` built from the cached `ToolOutput` so the conversation/API history stays paired. Images on the cached result are stripped before reuse so the duplicate does not re-consume the shared `image_bytes_budget` (the LLM already received the images on the first duplicate's `tool_result`). Defends against provider-side duplication (observed with non-Anthropic providers). Logs `warn!` with `trace_id`, `tool`, `step`, and `cached_was_error` when it fires.

### Post-Conditions (EndTurn Chain)

Nine sequential post-conditions on assistant text responses, plus one early-accept:

1. **Text-based tool call detection:** `detect_text_based_tool_call()` catches XML-style patterns (`<function=...>`) that slip through `extract_xml_tool_calls()` in mika-common, re-prompts the LLM once.
2. **Prose-style tool call detection (#569):** `detect_prose_style_tool_call()` catches function-call-style prose patterns (`tool_name({"key": "val"})`) where the identifier matches a registered tool (builtins + skills + MCP). Gated against the tool set to avoid false positives on code examples. Single retry.
3. **Required-tools gate:** When keyword-matched skills declare `[constraints] required_tools`, the engine tracks tool calls across all steps; if required tools haven't been called, the response is rejected once. `filter_available_required_tools()` pre-filters against builtins + skill tools + MCP. Only `Keyword`-matched skills contribute constraints (#463). **Self-contained response instruction (#890):** The correction message includes a persistence-awareness clause instructing the LLM to restate the full content on its corrected response — not reference prior turns — because only the final `EndTurn` is persisted to `messages`. Defense-in-depth: mika-arch skill prompts (`mika-arch-groom-ticket`, `mika-arch-second-review`) reinforce the same contract in their `### Constraints` section. **Terminal failure bypass (#516):** `has_terminal_required_tool_failure()` checks `all_tool_summaries` for required tools that failed with known terminal errors (GitHub self-approval, HTTP 4xx, permission errors). When detected, the gate allows EndTurn without retry — the agent attempted the tool and hit an unrecoverable wall. `is_terminal_tool_error()` classifies output via `RETRYABLE_ERROR_PATTERNS` (checked first, takes priority) and `TERMINAL_ERROR_PATTERNS`. Unknown errors default to retryable (conservative).
3b. **PR review early-accept (#695, #821):** `has_successful_pr_review()` checks if `all_tool_summaries` contains a successful `run_gh` call with `"pr"` and `"review"` in the input. When true, guards #3 (required-tools, #821), #4–#8 are all skipped — the qa-review workflow's primary action completed and forced continuation would risk duplicate submissions. Defense-in-depth (two layers): (1) Session-scoped `pr_reviews_posted` map on `AppState` (`DashMap<String, HashSet<String>>`, #821) prevents duplicate reviews across turns within the same session — the primary defense, keyed by `(session_id, repo|pr_identifier)`. Entries evicted at `end_session()` callsites. (2) Per-turn `ToolContext.pr_review_posted` AtomicBool (#695) rejects duplicates within a single turn. Both guards reject `pr review` calls with structured `duplicate_pr_review` error. `make_pr_dedup_key()` derives the session-scope key from `gh pr review` arguments.
4. **Completion-claim guard (#483):** `detect_completion_claim()` detects completion-claim keywords (`merged`, `deployed`, `complete`/`completed`, `shipped`) in assistant text. If detected AND `update_task_status` is in the tool registry AND it was not called AND active tasks exist, the response is rejected once. Skips for delegates and team agents.
5. **Fabricated action-claim guard (#308):** `detect_fabricated_action_claim()` detects when the agent claims to have performed an action with a GitHub resource URL but made zero tool calls in the turn. Single retry.
6. **Intent-precondition registry (#702):** Registry-driven guard that generalizes the webhook zero-tools guard (#696). `INTENT_GUARDS` is a const array of `IntentPrecondition` entries, each with a trigger function, satisfaction check, and correction message. Retry tracking uses `HashSet<&'static str>` keyed by label (one retry per entry). Current entries: (a) `webhook_ready_label_dispatch` (#846, #907, #1089) — if user message matches the `[GitHub] Issue labeled ready on` marker, requires `run_claude_pilot` attempt (dispatch via dev-pilot, or auto-groom via dev-groom). Post-#1089: the `send_message` grooming-rejection path was removed — all legitimate paths call `run_claude_pilot`; (b) `webhook_no_unauthorized_dispatch` (#910) — if user message starts with `[GitHub]` but does NOT match the ready-label marker, rejects when `run_claude_pilot` was successfully called (only successful calls — failed attempts are already blocked by the dispatch-readiness guard in `executor.rs`). Engine-level fix for recurring unauthorized dispatch from comment events (#798, #838, #910) where prompt-level source-check rules drifted under load. Post-#933: this post-hoc EndTurn guard is **defense-in-depth** — the primary prevention is the pre-hoc tool-boundary gate in `validate_dispatch_readiness()` check (0) which rejects `run_claude_pilot` before the subprocess spawns. Post-#1102: the trigger predicate now delegates to `is_unauthorized_webhook_dispatch()` from `crate::webhook_dispatch` — the same positive-allowlist predicate used by the tool-boundary guard. PR review and check-suite events (qa/ci skill territory) no longer trip the guard. Shared predicates live in `crate::webhook_dispatch` module; (c) `webhook_zero_tools` — if user message starts with `[GitHub]` and zero successful tool calls, rejects once (unchanged #696 behavior); (d) `resume_reconcile` — if user message contains resume/continue verb + milestone/project reference and no successful `check_task` or `list_tasks` call was made, rejects once; (e) `callback_terminal_action` (#870) — if user message starts with `[callback:` (Silent mode callback trigger), requires BOTH `update_task_status` AND `send_message` before EndTurn. AND-shape: both tools must be attempted (success or failure). Also has an inline mirror guard in the empty-text exit path for Silent mode, where the INTENT_GUARDS registry is not evaluated (the registry only fires in the non-empty text branch). `CALLBACK_TERMINAL_ACTION_LABEL` and `CALLBACK_TERMINAL_ACTION_CORRECTION` shared consts keep both sites in sync.
6b. **Callback milestone advance guard (#991):** Inline guard (not in `INTENT_GUARDS` const array) that enforces queue advancement on milestone/project-context callback turns. Triggers on `[callback:` + `[milestone-parent: <id>]` markers in the user message (the marker is injected by `run_silent_agent` after a DB lookup of the parent task type). Satisfied by EITHER Path A: `run_claude_pilot` call (advance to next child), OR Path B: `update_task_status` targeting the parent task ID with status `blocked`/`completed` (halt or finish). Inline because the satisfied predicate needs the parent_task_id from the user message. Composes with `callback_terminal_action` (entry e) — a milestone-context callback must satisfy BOTH guards. Also has an empty-text exit mirror guard. Companion `SilentTrigger::PostCallbackAdvance` fires a second advance turn if the first callback turn did not advance; auto-blocks the milestone if the second turn also fails.
6c. **Asserted-unavailability guard (#862):** Inline guard (not in `INTENT_GUARDS` const array) that detects when assistant text claims a tool is unavailable ("X is not callable", "I don't have access to X", "X is skill-scoped", "cannot call X") while X is in the agent's turn-start enabled-tool set and no call to X was attempted in the turn. Five regex patterns with named `(?P<tool>...)` capture groups, normalized to lowercase for case-insensitive registry lookup. Two-layer false-positive filter: snake-case capture constraint + enabled-set lookup. Inline rather than in the registry because it checks *assistant* text (not user input) and needs the `enabled_tool_names` snapshot + dynamic `format!` correction message. Uses `intent_guard_retries` with label `"asserted_unavailability"` for single-retry semantics. `enabled_tool_names: HashSet<String>` is a turn-start snapshot of the LLM tool array (after identity denylist + skill overrides + MCP), threaded to `run_loop` from all three call sites (conversation, silent, team). Structural counterpart to Rule 2 of `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`.
7. **Persistence evaluation guard (#648):** `detect_informational_input()` checks user input for informational signals (FYI, diagnostic, correction, status update) and `detect_persistable_output()` checks assistant text for verdict-shaped patterns (root cause, confirmed, validated, lesson learned). If no persistence write tool (`store_fact`, `update_fact`, `update_core_memory`) was called and either detection matches, nudges the model once to consider calling `store_fact`. Conversation mode only. Nudge, not rejection — the model can decline. `PERSISTENCE_WRITE_TOOLS` constant defines the write-tool set.
8. **Required-suffix-line guard (#864):** Manifest-driven guard for skill-declared output contracts. Skills opt in via `[output] required_suffix_lines = ["Verdict: GROOMED", "Verdict: ESCALATE"]` in `skill.toml`. `collect_required_suffix_lines()` unions lines from `Keyword` and `AlwaysOn` matched skills (not `Dependency`). The guard scans the assistant's last 3 non-empty lines (after `trim()`) for an exact match to any entry; missing match rejects EndTurn once with a corrective re-prompt naming the accept-set. Standalone retry flag `required_suffix_line_retry_done`. Position: after persistence-eval — other guards' rejections take precedence. Silent mode passes empty `required_suffix_lines` (no enforcement). Currently opted-in: `mika-arch-second-review` (GROOMED/ESCALATE) and `mika-arch-groom-ticket` (READY/ITERATE/ESCALATE). Structural counterpart for the verdict-ghosting failure mode in mika#788.
9. **Required-finding-list guard (#901):** Manifest-driven guard for skill-declared F-list emission contracts. Skills opt in via `[output] required_finding_list_prefixes = ["F1:", "F2:", ..., "F10:"]` in `skill.toml`. `collect_required_finding_list_prefixes()` unions prefixes from `Keyword` and `AlwaysOn` matched skills (not `Dependency`). The guard fires only on terminal dispositions (ITERATE/ESCALATE/Verdict: ESCALATE) — `is_terminal_disposition()` checks the last 3 non-empty lines against both the skill's `required_suffix_lines` and a `TERMINAL_DISPOSITIONS` constant. Scan range: message start up to (exclusive of) the first line matching any `required_suffix_lines` entry. At least one line in the scan range must start with a declared prefix (via `starts_with`, no regex). Missing match rejects EndTurn once. Standalone retry flag `required_finding_list_retry_done`. Position: immediately after #864 suffix-line guard. Silent mode passes empty `required_finding_list_prefixes` (no enforcement). Currently opted-in: `mika-arch-groom-ticket` and `mika-arch-second-review`. Structural counterpart for the conditional-disclosure-evasion failure class (N=8 incidents, mika#901).

### Deterministic Context Injection

Skills with `[context.*]` sections have their data pre-fetched by the engine before the LLM turn. `resolve_contexts()` dispatches to engine-owned handlers by `context_type`, deduplicates across skills, and returns `ContextBlock`s. `apply_context_replacements()` performs single-pass `{{key}}` template substitution on skill prompts (injection-safe — replaced content is never re-scanned). If a `required = true` context fails, the declaring skill is excluded from the turn; if `required = false`, a sentinel message replaces the placeholder. Known types: `gh_pr_diff`. Module: `skills/context.rs`.

## Three-Layer Memory Model

- **Layer 1:** Core memory (always in system prompt, agent-editable via `update_core_memory` tool, 2500 token limit, 5 blocks: user_summary, self_model, current_priorities, key_people, workflows)
- **Layer 2:** Structured facts (People, Commitments, Preferences, Events — plaintext). Managed via `store_fact`, `update_fact`, `search_memory` tools.
- **Layer 3:** Hybrid search (FTS5 full-text + sqlite-vec cosine similarity via Reciprocal Rank Fusion). Optional OpenAI embeddings (text-embedding-3-small, 512 dims). Graceful degradation: hybrid -> FTS5-only -> LIKE fallback. Indexed on store_fact/update_fact, backfilled on startup.

**Per-agent override:** mika-arch sets `[context.summary] inject = false`, removing the *conversation summary* layer from its system prompt entirely (mika#1009 leak protection). New agents that disable summary injection should be listed here.

## Context Injection Configuration

`[context]` section in `identity.toml` controls prompt-assembly behavior for context blocks. Each context block has its own nested subsection.

### `[context.summary].inject` (bool, default: `true`)

When `true`, the conversational summary (from compaction) is loaded from the DB and injected into the system prompt as `<context type="summary" trust="data">`. When `false`, the summary is **load-prevented** — `db.load_conversation_summary()` is not called, the summary is not deserialized, and is not available to any downstream code path in the turn. This is strictly stronger than injection-prevention and is the correct shape for context-leakage protection.

Use case: agents where the conversational summary is a known context-channel leak source (mika#1009). mika-arch is provisioned with `[context.summary] inject = false` by default.

```toml
[context.summary]
inject = false
```

### `[context.summary].max_tokens` (usize, optional, default: `None`)

Mode-conditional token budget for summary injection (Axis 3 — mika#1021). The field name is mode-agnostic; the gate condition (`SilentTrigger.is_some()`) lives in code. Orthogonal with `inject`: when `inject = false` (Axis 4), Axis 4 wins — `load_gated_summary()` short-circuits before evaluating `max_tokens`.

When set and the in-code gate fires (currently: silent-mode turns — callback, webhook, heartbeat, etc.):

- `Some(0)` → **load-omit sentinel**: summary omitted entirely on silent-mode turns. NOT interpreted as "zero-token cap."
- `Some(n)` for n > 0 → summary truncated to approximately `n × CHARS_PER_TOKEN_ESTIMATE` (= 4) characters before injection. A truncation marker `[… summary truncated to fit silent-mode budget …]` is appended so the model knows content was elided.
- `None` → no cap (default; current behavior).

Non-silent turns (conversation mode, CLI) are never affected by `max_tokens` regardless of its value.

Token approximation uses `CHARS_PER_TOKEN_ESTIMATE = 4` (heuristic, conservative for English). Truncation cuts at a word boundary via `truncate_to_token_budget()` in `prompt.rs`.

```toml
# Cap summary to ~1000 tokens on silent-mode turns, keep full on interactive turns
[context.summary]
inject = true
max_tokens = 1000

# Omit summary entirely on silent-mode turns
[context.summary]
inject = true
max_tokens = 0
```

## Tools

Each tool validates inputs. Control fields capped at `MAX_INPUT_LEN = 10_000` chars; payload fields capped at `MAX_PAYLOAD_BYTES = 200 * 1024` bytes (200 KB).

`ToolContext` contains `{ db, session_id, trace_id, home_dir, global_home_dir, core_memory_edit_count, is_onboarding, message_sender, embedding_client, brave_api_key, github_token, skills_dirty, is_reflection, is_task_context, is_callback_turn, provider_name, model_name, active_skill_paths, pr_review_posted, pr_reviews_posted, callback_task_id }`. `callback_task_id: Option<&str>` is `Some` for `SilentTrigger::Callback` and `SilentTrigger::DeferredDispatch` turns (carries the task ID for deferred dispatch registration and cycle detection, mika#1058), `None` otherwise. `global_home_dir: Option<&Path>` is `Some` for conversation mode, `None` for silent/team/delegate modes (blocks cross-agent file access). `active_skill_paths: &[SkillPathInfo]` lists skill prompt files already injected into the system prompt; populated in conversation mode, empty (`&[]`) in silent/team/investigate modes. `pr_reviews_posted: Option<&Arc<DashMap<String, HashSet<String>>>>` is `Some` in server mode (from `AppState`), `None` in CLI/test/silent/team modes — falls back to per-turn `pr_review_posted` AtomicBool (#821).

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

`server::verdict_handler` — intercepts `pull_request_review.submitted` webhook events **before** the LLM turn in `handle_message`. Parses `VERDICT:` line from the review body (authoritative regardless of GH `review.state`). Full dispatch table: `pass` (state=approved only) → merge via `run_gh_checks` + `run_gh_merge`; `block[ac]` → dispatch claude-pilot with AC-fix prompt, bounded retry counter (max 3), escalate on limit; `block[ci]` → dispatch claude-pilot with CI-fix prompt, bounded retry counter (max 3), escalate on limit; `block[security]`/`block[pipeline]` → mark task blocked, notify operator, NO auto-dispatch; `hold[review]` → notify operator, leave task in_progress; missing/unparseable → safe-default hold[review] semantics + `verdict_classification_failed` structured log event. AC extraction from qa-review's `[❌] unsatisfied:` lines with 2000-char fallback. Pre-digests for all verdict classes avoid completion-claim guard trigger words. Shared helpers: `find_task_for_verdict` (task lookup + in_progress gate), `has_active_callback_child` (in-flight guard), `send_notification`, `truncate_body` (UTF-8 safe). Parser in `server::verdict` depends on gateway's `format_event_text()` output format. 60s timeout on subprocess calls. See #524, #889.

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

**Bundled skill sources (dual path, #598):** `seed_bundled_skills()` seeds two concatenated sources on startup — the hardcoded `BUNDLED_SKILLS` list (10 community-category skills: tmux, shell-exec, web-search, file-reader, self-knowledge, git-ops, google-workspace, github, mcp, browser-control — `include_str!`-embedded from `templates/skills/`) and the directory-sourced `ENTRIES` table generated at build time by `build.rs` walking `<workspace>/skills/bundled/*/`. `all_bundled_skills()` merges both with case-insensitive ENTRIES-wins-on-name-collision semantics. Production `skills/bundled/` contains 22 engine-coupled skills: self-dev, self-dev-callback (#1106), self-dev-iterate, self-dev-webhook-qa, self-dev-webhook-ci, self-dev-webhook-ready-label (#1106), qa-review, qa-review-build-callback, skill-review, dev-pilot, build-mika, deploy-mika, permission-policy, agents-teams, address-pr-comments, resolve-pr-conflicts, self-check, dev-groom (#845), dev-handsoff (#967), mika-arch-groom-ticket (#811), mika-arch-groom-milestone (#879), mika-arch-second-review (#811). Engine-coupled = correctness depends on staying in lockstep with Rust engine code (tool schemas, callback contracts, prompt-discipline rules encoded as Rust guards). `is_bundled_skill()` consults both sources. Trust-critical classification (`TRUST_CRITICAL_SKILLS`) stays hardcoded regardless of source. Shared build-time discovery helper lives at `build_support/bundled_skills_discover.rs` and is consumed by both `build.rs` and integration tests via `#[path = ...]` mod attributes. Directories prefixed with `_` (e.g., `_shared/`) are excluded from discovery — convention-reserved for shared support libraries. `_shared/dispatch-lib.sh` (#893, #932) provides the shared claude-pilot dispatch plumbing. dev-pilot is the host skill owning the `run_claude_pilot` tool with a union-enum `skill` parameter (`["dev-pilot", "dev-groom"]`); dev-groom is prompt-only (no `tools.json`, no handler). The lib derives the entry command from `$SKILL` via a `case` switch.

**Dependency resolution:** BFS with cycle detection (max depth 10), same-source sibling resolution. `--link` propagates to deps from same source. `find_orphaned_deps()` + `--remove-deps` for cleanup.

**Per-provider and per-model variants:** Two-level directory hierarchy: `{provider}/` and `{provider}/{model}/`. `resolve_prompt(provider, model)` returns `ResolvedPrompt` with four-step fallback: hand-authored model variant -> generated model variant -> generated canonical variant -> root `system_prompt.md`.

**Per-skill LLM override:** DB-only via `skill_overrides` table (schema v20). `[llm]` section no longer supported in `skill.toml` (#504). `resolve_skill_llm_override()` constructs per-skill `LlmProvider`. **AlwaysOn + DB-override carve-out (mika#1011):** `AlwaysOn` skills with DB-sourced LLM overrides (`LlmOverride.from_db_override = true`, set by `apply_overrides()`) also qualify for override resolution — this ensures operator intent via `mika skills llm set` fires on autonomous-loop webhook turns where no keyword match exists. The #463 protection against `skill.toml [llm]` hijacks is preserved (developer-time overrides have `from_db_override = false`).

**Identity-driven skill allowlist (#811, #815):** All four well-known agents declare `[skills].allowlist` in `identity.toml` to own their skill set via identity rather than `skill_overrides` DB rows. mika-arch was the first (#811); mika-dev, mika-qa, and mika-relay followed in #815 (D2 cross-cutting). New bundled skills are denied by default unless explicitly added to an agent's allowlist — see the root `CLAUDE.md` § "Adding a New Bundled Skill" for the checklist. `SkillsIdentityConfig` in `prompt.rs` deserializes the `[skills]` block with `allowlist: Option<Vec<String>>`. `SkillRegistry::apply_identity_allowlist()` runs as Phase -1 before `apply_overrides()` — evicts all skills NOT in the allowlist (case-insensitive), following the same `retain()` + `DisabledSkill` pattern as Phase 0. Warns on allowlisted names not found in the registry. Empty or absent allowlist = no-op (all skills active). Wired into all skill-loading paths: server init, hot-reload (handlers.rs, a2a.rs), team engine, delegate_task, list_skills tool, and CLI (chat.rs, ask.rs, skills.rs). A one-time data migration (`migrate_well_known_to_identity_allowlist`, guarded by `schema_meta` marker `well_known_d2_migration_v1`) deletes stale `skill_overrides` denylist rows for the three migrated agents on first startup after deploy.

**Identity-driven tool denylist (#811):** Mirror of the skill allowlist for built-in tools. Well-known agents declare `[tools].disabled` in `identity.toml` listing tools that must not appear in their LLM tool array. `ToolsIdentityConfig` in `prompt.rs` parses the section. `agent::apply_agent_tool_visibility()` is the named filter hook — applied inside `inject_skills_and_resolve_tools` at the LLM-tool-array assembly site, before the tool defs are converted to `LlmToolDefinition`. The model never sees disabled tools, cannot call them, cannot be prompt-injected into trying. The shared `Arc<ToolRegistry>` is unchanged — filtering is per-agent at the presentation layer. mika-arch denies platform-mutational tools (skill mutations, config writes, file writes, task mutations, PR merge, cross-agent invocation, agent/team mutations) while keeping `send_message` and memory writes (`update_core_memory`, `store_fact`, `update_fact`) allowed — both are agent-scoped self-state, constitutive of being an agent, not platform side-effects (see `docs/architecture/review-guide.md` § Orthogonality). Future migration: extend to support `[tools].allowlist` for the symmetric well-known-agent shape.

**Fail-closed identity for well-known agents (#811):** `prompt::load_identity()` distinguishes well-known from user-defined agents on parse failure. For well-known agents (detected by matching the home_dir's last component against `find_well_known_agent`), a malformed `identity.toml` returns a fail-closed `Identity` with a sentinel allowlist (`["__fail_closed_no_skills__"]`) that matches no real skill, plus the full `MIKA_ARCH_DISABLED_TOOLS` denylist. The agent is effectively neutered until the operator fixes the file. For user-defined agents, parse failure logs `error!` and falls back to `Identity::default()` (current behavior — no security contract to preserve). The discrimination happens inside `parse_identity_or_fail_closed()`, so callers don't need to know the agent's well-known status.

**Skill enabled state:** DB-backed via `skill_overrides.enabled` column (schema v24, #629). Tri-state: `NULL` = default (enabled), `0` = disabled, `1` = explicitly enabled. `apply_overrides()` evicts disabled skills from `SkillRegistry.entries` into `disabled: Vec<DisabledSkill>` before applying `always_on`/LLM overrides. `enabled=false` always wins over `always_on=true` and over identity allowlist (DB override at Phase 0 runs after identity allowlist at Phase -1). `toggle_skill` agent tool and CLI `mika skills enable/disable` write to DB. Legacy `.disabled` marker files are migrated to DB rows on startup via `migrate_disabled_markers()` (one-shot, idempotent, fail-open). No match-time filter — disabled skills are evicted before matching (#630).

**Transient enable/disable overrides (#682):** Two methods handle per-invocation skill overrides, called after `apply_overrides()` (disable first, enable second — matches Phase 0/1 pattern): (1) `SkillRegistry::apply_transient_disable(skill_names)` evicts named skills from the registry entirely for a single CLI invocation. Returns `TransientDisableResult` with `not_found` list. Used by `mika ask --disable-skill <name>` (repeatable). (2) `SkillRegistry::apply_transient_always_on(skill_names)` sets `always_on = true` on named skills. Returns `TransientOverrideResult` with separate `disabled` and `not_found` lists. Cannot resurrect disabled (evicted) or skipped skills. Used by `mika ask --enable-skill <name>` (repeatable). Neither is persisted. Conflict check: same skill name in both flags produces a hard error before any registry ops.

**Oversized prompt handling (#630):** Skills with prompts exceeding their size limit are hard-skipped at scan time (pushed to `ScanResult.skipped`) regardless of `always_on` status. This prevents zombie skills with tools but no prompt context. Tool-only skills (no `system_prompt.md`) are unaffected — they load with an empty prompt via `SnippetLoadResult::Empty`.

**Startup logging:** `SkillRegistry::log_summary()` emits a three-state `INFO` line (`loaded=N disabled=N skipped=N`) plus per-skip `WARN` lines. Call after both `apply_overrides()` and `validate_loaded()` for accurate counts.

**Validation:** `validate_skill()` checks name-in-keywords rejection (#510), markdown validation (#511), required_tools references, context types, and `{{key}}` placeholders. **Startup validation (#530):** `SkillRegistry::validate_loaded()` runs `validate_skill()` on every loaded skill after `apply_overrides()`. Decision matrix: missing handler/broken tools.json → skip skill entirely; deprecated `[llm]` section/name-in-keywords/invalid markdown → load with warning. Results stored in `validated_warnings` for TUI/CLI display. `is_skip_worthy_failure()` classifies Fail diagnostics.

**Required tools enforcement:** Optional `[constraints]` section with `required_tools`. `collect_required_tools()` computes union across keyword-matched skills only. One retry on EndTurn violation.

**Required suffix-line enforcement (#864):** Optional `[output]` section with `required_suffix_lines`. `collect_required_suffix_lines()` computes union across keyword-matched AND always-on skills (not dependency). One retry on EndTurn violation. `validate_skill()` warns on explicitly-empty lists and rejects empty/whitespace entries.

**Required finding-list enforcement (#901):** Optional `[output]` section with `required_finding_list_prefixes`. `collect_required_finding_list_prefixes()` computes union across keyword-matched AND always-on skills (not dependency). Enforced only on terminal dispositions (ITERATE/ESCALATE) — non-terminal (READY/GROOMED) are exempt. Scan range: message start up to the suffix-line landmark. One retry on EndTurn violation. `validate_skill()` warns on explicitly-empty lists and rejects empty/whitespace entries. Same Warn-not-Fail pattern as suffix-line validation.

**Match-reason conditioning (#463):** `match_skills()` returns `MatchedSkill` wrappers with `MatchReason` (`Keyword`, `AlwaysOn`, `Dependency`). `always_on` skills do not enforce constraints unless the user's message also triggered a keyword.

**Review-target exclusion (#513):** `review_filter::apply_review_filter()` runs after `match_message()` and before `resolve_contexts()` in both conversation and team mode. When `skill-review` is keyword-matched, any other keyword-matched skill whose name appears in the user message (case-insensitive) is excluded from the matched set. This prevents the reviewed skill's prompt from contaminating the review context. `AlwaysOn` and `Dependency` skills are never excluded. Silent mode is unaffected (no keyword matching).

## Exec Handlers

**Image protocol (`__mika_v1`):** Scripts return images via JSON envelope `{"__mika_v1": {"text": "...", "images": ["/path/to/img"]}}`. Executor validates files (5MB limit, magic-byte check for JPEG/PNG/GIF/WebP), base64-encodes, max 5 images per result.

**Long-running:** `long_running: true` + `estimated_duration_secs` in `skill.toml`. Conversation mode and `DeferredDispatch` silent mode (#1058). Creates callback task, injects `__mika_task_id` and `__mika_agent` env vars, spawns detached process. PID recorded for orphan cleanup. **Callback deferred dispatch (#1058):** When a callback or DeferredDispatch turn calls a long-running tool and `long_running_ctx` is `None`, the executor gate intercepts the call via `callback_task_id` on `ToolContext`. Instead of a hard error, it runs `check_lineage_cycle()` (lineage walk on `(repo, issue_number, skill)` tuple, max 4 hops, fail-open on extraction failure) and, if no cycle is detected, calls `register_deferred_callback()` to enqueue the dispatch. Returns `{"status": "deferred", "deferred": true}` so the LLM knows not to retry. The deferred callback fires as a `DeferredDispatch` silent turn which HAS `LongRunningContext` injected. Cycle detection rejects same-tuple re-dispatch (e.g., `groom-#159 → retry-groom-#159`) but allows cross-skill chains (e.g., `groom-#159 → pilot-#159`). **Dispatch-readiness guard (#525):** before spawning, `validate_dispatch_readiness()` enforces seven checks: (0) unauthorized webhook dispatch (#933) — if `originating_message` is present and matches the Webhook Fallthrough domain (`[GitHub]` prefix excluding ready-label, PR, and check-suite events), rejects with `unauthorized_webhook_dispatch` before any DB access. Pure string-prefix check, cheapest guard. Predicate shared via `crate::webhook_dispatch::is_unauthorized_webhook_dispatch()`. (1) task status must be `pending` or `in_progress` (rejects `blocked`/`completed`/`cancelled` with structured JSON error `task_not_dispatchable`), (2) no active callback child task may exist (rejects with `task_active_dispatch`), (3) no other task of the same dispatch class may have an active callback child — per-class slot guard (rejects with `global_dispatch_active`, scoped to `agent_id` + `dispatch_class`) (#583, #1001). `dispatch_class` is `'implement'` (dev-pilot, deploy_mika) or `'groom'` (dev-groom); pre-v34 NULL rows are treated as `'implement'` via SQL `COALESCE`. One implement + one groom dispatch may run concurrently per agent, (4) per-turn dispatch counter must be zero — only one long-running dispatch per agent turn (rejects with `dispatch_limit_exceeded`) (#583), (5) grooming-marker check (#919, #1108) — if the task's `reference_url` points to a GitHub issue AND the dispatch skill is `dev-pilot` AND `task.type == "issue"`, fetches the issue body via REST API and checks for three canonical grooming callouts: `> - **Branch:**`, `docs/plans/`, and a `second-pass` marker (canonical `(GROOMED)` or spec-tolerated `(READY, paraphrased GROOMED ...)`). Rejects with `dispatch_no_grooming_marker` (listing `missing_signals`) if any are absent. Bypass predicates: non-`dev-pilot` skill, non-issue task type, non-GitHub-issue reference_url, or `MIKA_DISPATCH_BYPASS_GROOMING_CHECK=1` env var (WARN-logged). Fail-open when no `github_token` configured; fail-closed on API errors. Coupled pair with `skills/bundled/self-dev/system_prompt.md:253` (defense-in-depth prompt-level check), (6) GitHub `blockedBy` check — if the task's `reference_url` points to a GitHub issue, queries the GraphQL API for open blockers and rejects with `dispatch_blocked_by` if any are still open (#713). Fail-open when no `github_token` configured (check skipped with warning); fail-closed on API errors. Uses GraphQL variables (not string interpolation) for injection safety. `extract_open_blocker_numbers()` parses the response. `LongRunningContext` carries `dispatch_count: AtomicU32` initialized to 0 per turn; incremented after task creation and path validation, right before subprocess spawn. `LongRunningContext` also carries `originating_message: Option<String>` (#933) — populated from the latest user-role message in conversation mode, `None` for silent triggers. Fail-closed on DB errors. Auto-transitions `pending` tasks to `in_progress` on successful dispatch. Stricter than the shared `validate_task()` which also allows `blocked` for `delegate_task`. **Dispatch-rejection observability (#1108):** All 7 rejection sites write the structured JSON error to `tasks.result` via `record_dispatch_rejection()` (fire-and-forget, warn on DB failure). This surfaces rejection reasons to operator-visible surfaces (`mika tasks list`, dashboard task detail) without requiring DB-level inspection. The `write_task_dispatch_rejection()` DB method is agent-unscoped (keyed by `task_id` + `trigger_type = 'manual'`) because the earliest rejection site (unauthorized webhook) fires before the task is fetched.

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

## Evaluation — Grounding Regressions (#741, #862, #863, #864, #890, #901, #1059)

`tests/eval/grounding_regressions/` — 28 fabrication-detection scenarios. Scenarios 1–5 from the KG milestone #14 retrospective (#741), scenarios 6–7 from the gate-evasion compound doc (#862), scenarios 8–10 from the quoted-resource pre-fetch guard (#863), scenarios 11–15 from the required-suffix-line verdict-ghosting guard (#864), scenarios 16–18 from the required-tools-gate transport-contract fix (#890), scenarios 19–20 from the qa-review per-AC enumeration fix (#1059, mika-skills#159), scenarios 21–28 from the required-finding-list conditional-disclosure-evasion guard (#901). Each tests a concrete fabrication class with hard assertions (forbidden-word, required-tool, contains-in-order, contains, per-element-enumeration, absence-grounding). No LLM-judge gating — each class has objectively checkable signals.

**Scenarios:** GraphQL field fabrication (#720), auto-merge vs merged (#727), core memory priority drift (#732), fabricated shell errors (feedback doc), KG result ignored (#740 D4), asserted unavailability caught (#862 — guard fires on fabricated unavailability claim), asserted unavailability genuine (#862 — guard does NOT fire on genuinely disabled tool), quoted resource pre-fetch caught/no-op/mixed (#863 — pre-fetch guard augments required_tools from brief content), required suffix line caught/pre-fix/position-3/position-4/unconstrained (#864 — verdict-ghosting guard fires on missing suffix line), required-tools retry thin-final-turn regression/post-fix/correction-message (#890 — transport-contract guard: final turn must be self-contained after required-tools retry), qa-review per-element enumeration/absence-claim grounding (#1059 — per-AC enumeration rule and absence-claim evidence rule from mika-skills#159), required finding list caught-on-iterate/no-op-on-ready/no-op-when-unset/position-inclusive/position-exclusive/position-at-message-start/caught-on-verdict-escalate/no-op-on-verdict-groomed (#901 — finding-list guard fires on thin F-list emission with terminal disposition).

**Assertion helpers:** `tests/eval/grounding_assertions/mod.rs` — `assert_response_forbids`, `assert_any_tool_called_from`, `assert_response_contains_in_order`, `assert_response_contains`, `assert_tool_called_before_response`, `assert_response_contains_question`, `assert_response_contains_per_element_enumeration`, `assert_absence_claim_grounded`.

**Frozen regression fixtures:** Each scenario has a `fixtures/{scenario}_pre_fix.json` file with the pre-fix response that demonstrates the failure class. Regression-reproduction tests prove assertions catch the failure.

**Tag vocabulary (`grounding:*`):** `fabricated-ref-suppressed`, `completion-claim-suppressed`, `source-cited-correctly`, `verification-before-claim`, `uncertainty-admitted`, `training-data-hallucination` (failure), `transport-contract-thin-final-turn` (failure), `transport-contract-self-contained`. Scope boundary with #740 `self-knowledge:*`: self-knowledge = query-invocation code paths; grounding = response-to-evidence paths.

See `tests/eval/grounding_regressions/README.md` for the full vocabulary, capability matrix, and how to add scenarios.

## Knowledge Graph — Domain Graph Builder

`src/kg/domain_builder.rs` — Deterministic startup-time builder that populates `kg_entities` and `kg_relationships` from five authoritative sources: `SkillRegistry`, `ToolRegistry`, `McpManager`, agent configs, and concept seeds (hardcoded). Runs once per server boot in `run_server()` after all agents are initialized. No LLM calls — pure code projection.

**Entity types:** Skill, Tool, Agent, ProblemType (5 seeds: `ci_failure`, `merge_conflict`, `duplicate_pr`, `stale_uuid`, `fabrication`), Concept (20 seeds: 7 `concept:cross-repo:*` + 13 `concept:infra:*`). **Relationship types:** `DEPENDS_ON` (Skill→Skill from `skill.toml` dependencies), `PROVIDES` (Skill→Tool from skill tools).

**Sole-writer contract:** This module is the sole writer of `skill:*`, `tool:*`, `agent:*`, `problem_type:*`, and `concept:*` entity_keys. No other code path writes these namespaces.

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

**Well-known agent KG topology (#800):** mika-arch is the sole KG consumer among well-known agents. mika-dev and mika-qa are provisioned with `[kg].enabled = false` — they have zero `query_knowledge_graph` usage (retrieval goes through `search_memory` over `memory_facts`). This eliminates the shared-corpus extractor race on the mika-docs corpus where multiple agents redundantly called the LLM for the same doc. Re-enable per-agent with one `identity.toml` edit (`enabled = true`) + restart if a dev/qa flow needs KG-backed retrieval.

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

**b. Array order no longer starves secondary corpora (#962).** Budget is distributed fairly across corpora using two-pass allocation (`kg::budget::allocate_fair_budget`) for both extraction and resolution. Each corpus with pending work receives a proportional share. Array order only affects tiebreaking when the budget cannot be evenly divided — not a starvation vector.

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

**Approved entity types:** `skill`, `tool`, `agent`, `problem_type`, `solution_path`, `failure_mode`, `pattern`, `concept`. **Approved relationship types:** `SOLVED_BY`, `USES`, `CALLS`, `INDICATES`, `PREVENTS`, `CAUSED_BY`, `MENTIONS` — each with from/to type constraints enforced in code.

**Sole-writer contract:** This module is the sole writer of `kg_subject_entities`, `kg_subject_relationships`, `kg_chunk_subjects`, `kg_chunk_subject_relationships`, and `kg_extractions` rows.

**Execution contexts (D2):** (1) Startup: background `tokio::spawn` per agent after lexical ingestion, non-blocking. (2) Compound hook: synchronous inline after doc write via `IngestionOrchestrator`, failure non-fatal (C2.3 log-and-skip).

**Extraction flow (D1, D4):** Read full doc from disk → insert `[CHUNK N]` markers at chunk boundaries → LLM call → parse/validate JSON output → UPSERT entities and relationships with provenance in single transaction.

**Re-extraction (D5):** Three-phase capture → reingest → reconcile. `IngestionOrchestrator` (`src/kg/ingestion_orchestrator.rs`) coordinates #689 and #690 — neither module calls the other directly. Scoped orphan sweep deletes entities/relationships that lost all provenance after doc change.

**Pending-doc detection (D7 + #757 hash check):** `kg_extractions` tracking table with `UNIQUE(agent_id, source_doc_path)` and a `source_doc_hash TEXT` column (nullable, added in v26). A doc is pending when it either has no `kg_extractions` row OR `kg_extractions.source_doc_hash != kg_chunks.source_doc_hash` — direct equality, no aggregation, because the lexical ingestor writes one identical per-doc hash across every chunk row. See `src/db/kg_schema.rs` for the full idempotency contract.

**Budget guard (#757):** `extract_pending(budget: u32)` caps per-batch LLM calls. `budget == 0` short-circuits with zero calls. On overflow, emits `kg_budget_exhausted` WARN with `scope="extraction"` and leaves remaining docs pending. Stats carry `aborted_budget: bool` + `llm_calls: u32`. Default budget: `MIKA_KG_BATCH_BUDGET` (500).

**LLM policy (C2):** Model from `MIKA_KG_EXTRACTION_MODEL` → `MIKA_KG_INGESTION_MODEL` fallback. Retry taxonomy per C2.2 (transport: 3 attempts with backoff; semantic: one retry with prompt reinforcement; config: no retry). Log-and-skip per C2.3. `llm_calls` rows per C2.4. Audit events per C3.3.

**Parse tolerance (#876):** `parse_extraction_json` tolerates reasoning prose before/after the JSON object — a common failure mode with haiku-class models (sibling of #768). Three-layer parsing: (1) strip markdown code fences, (2) direct `serde_json::from_str`, (3) `extract_first_json_object()` brace-matching fallback that locates the first balanced `{…}` in surrounding text with string-literal/escape-aware depth tracking. Schema validation stays strict — only surrounding-prose tolerance is added. When the slow path (layer 3) succeeds, emits `extraction_parse_slow_path` WARN for operator visibility. The extraction prompt also includes a JSON-only output instruction as defense-in-depth.

## Knowledge Graph — Entity Resolver

`src/kg/entity_resolver.rs` — Per-agent entity resolution that bridges subject graph entities to domain graph nodes (#691). Two-stage pipeline: exact match (case-insensitive) then LLM disambiguation for unresolved or ambiguous cases.

**Sole-writer contract:** This module is the sole writer of `kg_subject_resolutions` (subject → domain edges with confidence scores) and `kg_resolutions_log` (resolution tracking with outcome enum). No other code path writes these tables.

**Two-stage pipeline (D1):** Stage 1: case-insensitive exact match against `kg_entities.entity_key`. If match found and extraction confidence > 0.9, resolve immediately (confidence = extraction_confidence). Stage 2: LLM disambiguation with candidate list (max 50) and source chunk prose context. Combined confidence = min(extraction_confidence, llm_confidence). Discovered types (solution_path, failure_mode, pattern) skip resolution entirely — no domain counterpart exists.

**Execution contexts (D5):** (1) Startup: background `tokio::spawn` per agent after extraction tasks, non-blocking. (2) Compound hook: `IngestionOrchestrator` spawns async resolution after extraction commits. (3) Periodic tick (#906): `kg::resolver_tick::spawn_resolver_tick_task()` runs `resolve_pending(budget)` every 30 minutes per KG-enabled agent, decoupling drain rate from restart cadence. First fire skipped (startup spawn covers it). Fail-open (log-and-skip per C2.3). Lifecycle tied to tokio runtime drop (same pattern as `checkpoint_task`). Structured log events: `kg_resolver_tick.start`, `kg_resolver_tick.complete`, `kg_resolver_tick.error`.

**Pending-entity detection (D4):** `kg_resolutions_log` tracking table with `UNIQUE(agent_id, subject_entity_id)`. Pending query: subject entities with well-known types that have no log row, or whose `source_extraction_trace_id` differs from the latest `kg_chunk_subjects` extraction.

**Per-corpus fairness (#927):** `get_pending_entities(budget)` distributes the selection budget across all agent corpora via two-pass reallocation and round-robin interleaving. First pass assigns each corpus `min(pending_count, budget/N)`; second pass redistributes unused slots proportionally to hungry corpora. Per-corpus fetch limit uses 2× oversupply with a floor of 50. Results are interleaved `[A₀,B₀,C₀,A₁,B₁,...]` so no single large corpus starves smaller ones under the Stage-2 budget cap. Single-corpus agents take a fast path with no allocation overhead. `ResolutionStats.per_corpus_attempted: HashMap<String, u32>` tracks per-corpus attempt counts; emitted as JSON in the `kg_resolver_tick.complete` log event via `per_corpus_attempted` field.

**Budget guard (#757):** `resolve_pending(budget: u32)` caps per-batch **Stage-2** LLM disambiguation calls. Stage-1 exact matches cost no LLM calls and are NOT debited against the budget — even `budget=0` lets exact matches resolve (selection uses an effective minimum of 50 entities). On overflow, emits `kg_budget_exhausted` WARN with `scope="resolution"` and the remaining entities stay pending (no `kg_resolutions_log` row written). Stats carry `aborted_budget: bool` + `llm_calls: u32`. Default budget: `MIKA_KG_BATCH_BUDGET` (500).

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

**Task health awareness (heartbeat and callback):** `get_task_health_summary(agent_id)` detects 8 anomaly types and injects `<task-health>` block. Gated to `Heartbeat`, `Callback`, and `Reminder` triggers. Anomaly types: `stuck_callback` (completed but not delivered >10min), `failed_recurring`, `long_running` (in_progress >1h), `stale_blocked` (blocked >24h), `stale_pending` (pending >24h with no callback child — detects tasks created but never dispatched) (#583), `github_linked` (active items with GitHub PR URL), `dispatch_failures` (3+ recent `run_claude_pilot` failures in 2h window — wedged iteration loop detection, #980), `dispatch_stale` (in_progress task with no dispatch attempt in >1h — aging defense for wedges that stop retrying, #980).

## MessageSender Trait

`#[async_trait]` with `Send + Sync` bounds for `Arc<dyn MessageSender>`. Returns `Result<SendOutcome>` where `SendOutcome` is `Delivered` (gateway 2xx), `Failed { reason }` (non-2xx after retry, saved to `failed_sends`), or `NoChannel` (`chat_id == 0` sentinel — no reply channel available, e.g. GitHub webhook sessions). `Err` is reserved for infrastructure failures (chat_id resolution, DB errors). Text-only outbound. CLI prints to stdout. Server uses `GatewayMessageSender` (one retry after 2s, error classification: connection/timeout/HTTP status with body snippet). Team engine agents intentionally have `message_sender: None`. `NoopSender` (pub in `messaging.rs`) silently returns `Ok(Delivered)` — used to suppress user-facing notifications in team-child callback turns (#287) where the consolidated team-run notification handles delivery.

**`NoChannel` sentinel (#650, #1090):** `GatewayMessageSender::send()` detects `chat_id == 0` after `resolve_chat_id()` and returns `Ok(NoChannel)` before the HTTP POST — no retry, no `failed_sends` entry. `chat_id == 0` is the documented sentinel for sessions without a Telegram reply channel (GitHub webhooks, non-Telegram channels). The agent should use channel-appropriate tools (e.g., `run_gh`) instead of `send_message`. Two `error!`-level log lines are emitted on every `NoChannel` event for operator observability (#1090): Site A (`messaging.rs`) carries `agent_name`, `request_id`, and `message_text` (truncated to 500 chars via `truncate_for_log`); Site B (`send_message.rs`) carries `trace_id` and `session_id`. Both use the event name `send_message_nochannel` for grep-friendly filtering.

**Callsite handling policy:** The `send_message` tool surfaces `Failed` as `ToolOutput::error` so the LLM knows delivery failed; `NoChannel` returns `ToolOutput::success` with redirect guidance (prevents LLM retry loops). The task-engine dispatcher absorbs `Failed` and `NoChannel` with a warning (fire-and-forget for scheduled sends). Server handlers and notification paths (verdict, CI success) log warnings on `Failed` and `NoChannel` but continue. The `failed_sends` flush path increments retry count on `Failed`; deletes entries on `NoChannel` (permanent condition).

## Conversation Compaction & Rewind

**Compaction:** Threshold-based (50 messages). Keeps 20 most recent, summarizes older via Claude API. Summary injected into system prompt. `replace_with_summary` uses RAII `rusqlite::Transaction` (DEFERRED) — auto-rollback on error prevents stuck transactions that pin the WAL snapshot (#636).

**Summarizer output contract (#1024):** The compaction summarizer produces *factual state assertions*, not conversational summaries. Output bullets use one of four prefixes: `Fact:` (objective state), `Decision:` (choices and disposition), `Outcome:` (results and state transitions), `Open:` (unresolved questions). The prompt explicitly forbids first-person language, conversational verbs (discussed/agreed/decided), and process narration. This shape is per mika#1009 finding (Axis 2 — content reform): the summary block is consumed by the next session as system-prompt context, and conversational shape there causes the LLM to misread it as prior turns it participated in. The prompt is a single `const &str` at `compaction.rs:14`; tests at `compaction.rs` (`summarization_prompt_enforces_factual_shape`) assert prompt invariants.

**Rewind:** `rewind.rs` — two-phase flow: `preview_rewind()` then `execute_rewind()` with automatic reversal of memory/fact mutations via audit log. TUI: `/undo` (1 exchange), `/rewind [N | to <message_id>]`. Server: `POST /api/v1/rewind/{resolve,preview,execute}`.

## Unified Task Engine

`src/task_engine/` — single SQLite-backed scheduler. Min-heap + dedup set; 1-second tick loop; periodic DB scan (60 ticks). `TaskDispatcher` matches on `action_type`. `ensure_recurring_task()` idempotently registers heartbeat and reflection at startup.

**Callback/resume lifecycle:** agent creates callback task -> external process completes it -> server dispatches silent agent run with `SilentTrigger::Callback`. Loop prevention: callback turns cannot **directly** spawn new long-running tasks; the executor's `long_running_ctx == None` rejection is intercepted and re-routed through `DeferredDispatch` with `(repo, issue_number, skill)` lineage cycle detection (mika#1058). Direct spawn from callback context still hits the gate; deferred re-dispatch is the safe path.

**SilentTrigger variants:** `Heartbeat`, `Reflection`, `Callback`, `SkillRun`, `Reminder`, `PostCallbackAdvance` (#991), `DeferredDispatch` (mika#1011). Each produces correct system-prompt framing. `PostCallbackAdvance` is an engine-side structural backstop — fired by the dispatcher after a milestone/project-context callback turn completes without advancing the queue. `DeferredDispatch` is an engine-side auto-recovery for `global_dispatch_active` rejections — when `run_claude_pilot` is rejected because another dispatch is active, the engine registers a `pending` callback task with label `long_running:run_claude_pilot:deferred`. When the blocking dispatch completes, the deferred callback is promoted (status → `completed`) and dispatched on the next engine tick as a `DeferredDispatch` turn whose only required action is `run_claude_pilot` (enforced by the `deferred_dispatch_action` INTENT_GUARD). **Promotion paths (mika#1070):** (1) Inline — `dispatch_next_deferred_callback()` (`pub(crate)`) fires after any callback `mark_task_delivered`, including DeferredDispatch completions (chain promotion — the anti-cascade guard was removed; each promotion is a LIMIT 1 DB write + return, no call-stack cascade). (2) Periodic backstop — `promote_pending_deferred_if_idle()` runs every `DB_SCAN_INTERVAL_TICKS` (60 ticks), checks `has_any_active_callback()` (excludes deferred wrappers via `label NOT LIKE '%:deferred'`), and promotes when the dispatch slot is idle. Placed BEFORE `dispatch_undelivered_callbacks` for same-tick dispatch. Fail-closed on DB errors. **AgentBusy recovery (mika#1070):** when `dispatch_resume_agent` returns `AgentBusy` in `handle_task_complete`, the callback keeps `completed` status (not reset to `pending`) with `next_fire_at` set to now+30s for retry delay. `dispatch_undelivered_callbacks` has a `next_fire_at` guard that skips tasks whose retry delay has not expired. γ composition: the LLM's `send_message` notification and the engine's deferred callback are independent; `validate_dispatch_readiness()` arbitrates any race. Per-agent cap of 10 pending deferred callbacks prevents flood. `cancel_task()` cascades to callback children (both immediate and deferred).

**Engine-level callback metadata extraction (#376):** `try_extract_callback_metadata()` parses structured fields from callback results and persists to parent task. `extract_callback_fields()` extracts `session_id`, `turns`, `cost_usd`, `duration_ms`, and `pr_url` from claude-pilot output into the `claude_pilot` metadata object. The `pr_url` field (#871 R4) is parsed from `^PR:\s+<url>` lines emitted by `skills/bundled/_shared/dispatch-lib.sh` (shared handler for dev-pilot and dev-groom, #893); the reaper (below) keys off its presence/absence.

**Callback process liveness watchdog (#959):** `check_callback_process_liveness()` runs every 60-tick cycle and detects when a long-running subprocess (e.g., `run_claude_pilot`) has crashed without delivering its callback result. Detection: queries `in_progress` callback tasks with `process_id IS NOT NULL`, checks PID liveness via `kill(pid, 0)` + `/proc/<pid>/stat` field 22 (process start time) comparison to guard against PID reuse. On first detection of a dead process, records `first_dead_at` in task metadata; after `MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS` (default 120s) elapses, re-checks task status (race guard) then marks the task `failed` with `error_reason = "subprocess_exited_without_delivery"`. Process start time is stored in callback task metadata at spawn time by `spawn_long_running_exec()`. The watchdog detects death in ~60s (one tick) + 120s grace = ~3 minutes total, vs the previous 6-hour `timeout_at` fallback. Platform: Linux only (`/proc` filesystem). The existing `timeout_at` mechanism serves as a panic-fallback for edge cases where PID tracking fails.

**Orphaned parent reaper (#871):** `reap_orphaned_parent_tasks()` runs every 60-tick cycle (same cadence as other periodic scans) and detects parent self_dev tasks left `in_progress` after their callback subtask delivers without producing a PR. Detection query (`find_orphaned_parent_tasks`): parent `status='in_progress'`, `source='self_dev'`, `trigger_type='manual'`; child `trigger_type='callback'`, `action_type='resume_agent'`, `status='delivered'`; child `updated_at` older than `REAPER_GRACE_SECONDS` (600s); parent metadata has no `$.claude_pilot.pr_url`; `NOT EXISTS` active sibling guard (defers when #870's retry loop launched a new callback child). On match: transitions parent to `failed` via guarded `update_task_failed` (terminal-state check prevents TOCTOU race), emits `audit_events` row with `tool_name='task_engine_reaper'`. Pre-existing leaks (age > 24h) get a distinct log line for post-deploy backfill visibility.

**Team task tree:** parent `invoke_orchestrator` task + child `resume_agent` tasks per delegation. Suspend/resume on pending grandchild callbacks. **Team-run user notification (#287):** fired once at terminal status from two symmetric callsites (`run_team` tool for sync completion, `dispatch_invoke_orchestrator` for async resume), both routing through `teams::notification::build_run_completion_message`. Per-child `resume_agent` callbacks have their user-facing `send_message` suppressed via `NoopSender`; the silent turn still runs (updates memory, records `llm_calls`) — only the user channel is gated. Deliverable text is UTF-8-safe truncated at 4000 chars (below Telegram's 4096 limit).

## HTTP Server (mika-server)

Axum-based with two auth layers: mutation endpoints require `MIKA_INTERNAL_TOKEN` only; read-only dashboard API accepts either `MIKA_DASHBOARD_TOKEN` or `MIKA_INTERNAL_TOKEN` (superuser).

**Mutation endpoints:** `/message` (202 async, 10MB limit), `/tasks/{id}/complete` (200 sync, 100KB cap), `/tasks/{id}/cancel` (200 sync), `/api/v1/rewind/*`, `/a2a/*`.

**Dashboard API:** `/api/v1/*` — timeline, agents, sessions, messages, traces, investigate, tasks (+ detail/children/descendants/sessions), team-runs (+ summary), llm-calls (+ detail), tool-calls (+ detail), dev-runs (+ detail), github proxy endpoints. CORS scoped to `MIKA_CORS_ORIGIN`.

**Time-range filtering (#659):** All list endpoints (timeline, sessions, llm-calls, tool-calls, team-runs, tasks, dev-runs) accept `from`/`to` ISO 8601 string query params for server-side filtering against the surface's primary timestamp column (`created_at` or `started_at`). String comparison is correct because ISO 8601 ordering matches chronological ordering. Frontend emits via `<TimeRangeFilter />` from `@senara-solutions/ui`.

**Request logging:** `tower_http::trace::TraceLayer` middleware. `inject_request_meta` middleware copies method+path for top-level JSON fields. `/health` logged at DEBUG. Agent lock via `tokio::sync::Mutex<()>` with non-blocking `try_lock` (429 if busy).

**Failed sends flush:** Before each message processing, flushes up to 5 pending failed outbound sends from DB.

**WAL checkpoint (#636):** `server::checkpoint::spawn_dashboard_checkpoint_task()` runs `PRAGMA wal_checkpoint(PASSIVE)` on the dashboard DB connection every 60 seconds. Defense-in-depth against stale WAL snapshots: if a transaction leak pins the connection's read snapshot, the periodic checkpoint forces the connection to advance. Structured log events: `checkpoint.start`, `checkpoint.complete` (with `busy_pages`, `log_pages`, `checkpointed_pages`), `checkpoint.error`, `checkpoint.stopped`. Hard-coded 60s interval (future tunable: `MIKA_DASHBOARD_CHECKPOINT_INTERVAL_SECS`). If WAL grows despite PASSIVE checkpoints, escalate to RESTART mode per the operating-envelope trigger documented in `checkpoint.rs`.

## Observability

"Always instrument, optionally export" pattern. Two orthogonal correlation axes: `trace_id` (per-request/per-turn, 32-char hex) + `session_id`/`agent_id` (system-level). `unified_timeline` VIEW enables cross-subsystem queries.

**Span filtering:** Per-layer `filter::Targets` on the OTel layer exports only `target: "mika::otel"` spans (LLM calls, agent turns, server requests).

**LLM observability:** `llm_call` spans with `gen_ai.*` semantic convention attributes, feature-gated behind `#[cfg(feature = "telemetry")]`. When `log_llm_bodies=true`, request/response bodies are attached as `gen_ai.prompt`/`gen_ai.completion` span attributes for Langfuse Generation input/output (#671). Response bodies use `serialize_response_text()` from `mika-common::llm` (same format as `llm_calls.response_text`). Team engine emits `TeamEvent` variants for live dashboard updates.

**Session lifecycle:** Silent dispatcher variants call `end_session()` after completion. CLI commands call `end_session()` on all exit paths. `startup_recovery()` prunes old sessions via `prune_old_sessions()`.

### Log Sinks

Two distinct sinks exist for runtime log events. Both emit the same structured JSON with an `agent_id` field on every entry — the difference is the file target and the process that writes to it.

| Sink | Initializer | Process | Path | Rotation |
|------|-------------|---------|------|----------|
| **Server log** | `mika_common::logging::init()` (`crates/mika-common/src/logging.rs:208`) | `mika-server` (long-running daemon) | `MIKA_SERVER_LOG_FILE` (e.g. `/var/log/mika/server.log`) | None — single file via `tracing_appender::rolling::never` |
| **Per-agent CLI log** | `mika_common::logging::init_pretty()` (`crates/mika-common/src/logging.rs:314`) | `mika-cli` (`mika ask`, `mika chat`, `mika team run`, etc.) | `~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD` (daily) | Daily via `tracing_appender::rolling::daily` |

**Decision tree for operators:** If you want to read the running mika-server's runtime events (skill execution, task engine, callback lifecycle, autonomous-loop wedges), read `MIKA_SERVER_LOG_FILE` filtered by `agent_id`. If you want to read a specific `mika ask` or `mika chat` invocation's events, read `~/.mika/agents/<name>/logs/mika.log.<date>`. Both contain the same `agent_id` field on every entry, so cross-filtering by agent works in either sink.

**Single-sink rationale:** mika-server uses one file with `agent_id`-filtered queries instead of per-agent file appenders because: (a) per-agent appenders would double the disk-write rate per event, (b) they create a sync gap risk if the per-agent appender worker can't keep up, and (c) they duplicate data already correctly addressable via the JSON `agent_id` field. The per-agent CLI sink is correct for its purpose (discrete CLI invocations) and is not a substitute for the server log.

**Common mistake:** Audit tooling that reads `~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD` for server-mode events will find it nearly empty — only CLI invocations write there. Use `jq 'select(.agent_id == "<name>")' < $MIKA_SERVER_LOG_FILE` for server-mode queries. `mika logs --agent <name>` prints both resolved paths.

## Audit Log

`audit_events` table tracks all memory mutations per session. All writes include `trace_id`.

## Timestamps

All SQLite timestamp columns use ISO 8601 TEXT format (`%Y-%m-%dT%H:%M:%SZ`). The `crate::timestamp` module provides centralized helpers: `now()`, `format()`, `parse()`, `now_plus()`, `now_minus()`. Fixed-width UTC format ensures correct lexicographic ordering.

## Schema Version

**Current: v31.** Tables: sessions, messages (with `internal` flag for agent-to-agent visibility), team_workspace, audit_events, skill_overrides (with `enabled` column for DB-backed disable state), tasks (with manual/callback/a2a trigger types and a `type` column distinguishing `issue`/`milestone`/`project`), a2a_task_map, a2a_artifacts, a2a_push_notification_configs, llm_calls (with `response_text` and `reasoning` columns for LLM output persistence), tool_calls, team_runs, schema_meta (migration state tracking), kg_entities, kg_relationships. **Shared-corpus KG tables (keyed by `docs_root_hash`):** kg_chunks, kg_subject_entities, kg_subject_relationships, kg_chunk_subjects, kg_chunk_subject_relationships, kg_extractions (first-writer-wins via INSERT OR IGNORE). **Per-agent KG tables:** kg_subject_resolutions, kg_resolutions_log, agent_kg_corpora (agent_id to docs_root_hash mapping for multi-corpus fan-out). `unified_timeline` VIEW for cross-subsystem queries. Session-based message storage with FK. System sessions (`system-{agent_id}`) for compaction.

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
- v28->v29: Backfill migration (#908) — scrubs secret-shaped values from existing `tool_calls.input` and `tool_calls.output` rows using `secret_scrubber::scrub_secrets()`. Data-only, no DDL. `save_tool_call()` now applies `scrub_secrets()` to both `input` and `output` before INSERT. `ToolCallSummary` metadata (`input_summary`, `output_summary`) also scrubbed before serialization to `messages.metadata`.
- v29->v30: Expand `kg_resolutions_log.outcome` CHECK constraint to include `'matched_llm_db_fallback'` (#874). Table rebuild mirroring v26→v27 shape. Enables DB-fallback acceptance path for LLM matches outside the in-prompt candidate window.
- v30->v31: `llm_calls.response_text TEXT` and `llm_calls.reasoning TEXT` columns (#653). Stores serialized LLM response content (text blocks joined with newlines, tool-call summaries as `[Tool Call: name(args)]`, stripped of internal tags, capped at 50K chars) and extended thinking text. Additive ALTER TABLE with per-column `column_exists` guards for crash-recovery safety. `save_llm_call()` gains two new params. `get_llm_call_by_id()` uses `row_to_llm_call_detail` to read the new columns; list queries use `row_to_llm_call` which returns `None` for performance. New `get_tool_calls_by_llm_call_id()` query. New `GET /api/v1/llm-calls/{id}/tool-calls` endpoint.
- v26->v27: **Shared-corpus primary key** (#786 + #787). Six shared-layer KG tables (`kg_chunks`, `kg_subject_entities`, `kg_subject_relationships`, `kg_chunk_subjects`, `kg_chunk_subject_relationships`, `kg_extractions`) change primary-key scope from `agent_id` to `docs_root_hash` — a 16-hex-char SHA-256 prefix of `fs::canonicalize(docs_root)` computed by `kg::config::hash_docs_root`. Agents with the same `docs_root` now share a single corpus; extraction cost drops from N× to 1×. Per-agent tables (`kg_subject_resolutions`, `kg_resolutions_log`) FK-rewired but row-count preserved. `schema_meta` table added for migration state tracking. Two-phase migration: (1) DDL renames v26 tables to `*_v26_backup`, creates empty v27 tables (#786); (2) coalesce reads from backups, deduplicates via majority-vote (normalized `entity_key`, agent-count + mean-confidence + `MIN(id)` tiebreak), rewires FKs via temp lookup tables, drops backups, writes `v27_coalesce_complete` marker (#787). `v27_coalesce_sql()` is public for integration test access. `docs_root` resolved from `MIKA_KG_DOCS_ROOT` env var or CWD fallback at migration time. Startup guard refuses `Database::open()` when `schema_version == 27` and marker is absent. Recovery runbook: `docs/solutions/database-issues/kg-v27-stuck-migration-recovery-2026-04-24.md`.

- v33->v34: `tasks.dispatch_class TEXT` column (#1001) — nullable, CHECK constraint (`'implement'`/`'groom'`). Per-class dispatch slot split: the global single-session-at-a-time guard becomes per-class, allowing one implement + one groom dispatch concurrently per agent. Pre-v34 NULL rows treated as `'implement'` via `COALESCE` in the guard query. Partial index `idx_tasks_dispatch_class` on `(agent_id, dispatch_class, status)`. `update_task_dispatch_class()` method for mika#996 task-reuse pattern (flip class on groom→implement transition).

Full migration history: see `docs/runtime-structure.md`.
