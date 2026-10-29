# Tools

Moved verbatim out of `crates/mika-agent/CLAUDE.md` on 2026-10-13. That file is loaded whole into a pilot's prompt as soon as the pilot reads anything under `crates/mika-agent/`; at 594 KB it no longer fit, and `crates/mika-common/tests/claude_md_size.rs` now caps every `CLAUDE.md` in the repository. Headings keep their original level.


Each tool validates inputs. Control fields capped at `MAX_INPUT_LEN = 10_000` chars; payload fields capped at `MAX_PAYLOAD_BYTES = 200 * 1024` bytes (200 KB).

`ToolContext` contains `{ db, session_id, trace_id, home_dir, global_home_dir, core_memory_edit_count, is_onboarding, message_sender, embedding_client, brave_api_key, github_token, skills_dirty, is_reflection, is_task_context, is_callback_turn, provider_name, model_name, active_skill_paths, pr_review_posted, pr_reviews_posted, callback_task_id }`. `callback_task_id: Option<&str>` is `Some` for `SilentTrigger::Callback` and `SilentTrigger::DeferredDispatch` turns (carries the task ID for deferred dispatch registration and cycle detection, mika#1058), `None` otherwise. `global_home_dir: Option<&Path>` is `Some` for conversation mode, `None` for silent/team/delegate modes (blocks cross-agent file access). `active_skill_paths: &[SkillPathInfo]` lists skill prompt files already injected into the system prompt; populated in conversation mode, empty (`&[]`) in silent/team/investigate modes. `pr_reviews_posted: Option<&Arc<DashMap<String, HashSet<String>>>>` is `Some` in server mode (from `AppState`), `None` in CLI/test/silent/team modes — falls back to per-turn `pr_review_posted` AtomicBool (#821).

Tool trait uses `#[async_trait]` (Send futures). Per-tool timeout override via `timeout_secs()` default method (returns `None` -> uses 30s default). Shared `validate_and_resolve_path(path, base_dir, create_parents: bool)` helper in `tools/mod.rs` for path security (tilde expansion to base_dir, `~username` rejection, empty check, length limit, absolute rejection, traversal inspection, symlink check, canonicalize containment). Three-layer UUID validation chain in `tools/mod.rs` (#531, #596): (1) `validate_uuid(field_name, value)` — format-only via `Uuid::parse_str()`, returns `Result<Uuid, ToolOutput>` with structured JSON `{"error": "invalid_uuid", ...}`; (2) `validate_task_exists(db, field_name, value)` — format + DB existence + agent-scope, returns `Result<Task, ToolOutput>` with `{"error": "task_not_found", ...}` or `{"error": "db_error", ...}` (fail-closed); (3) `validate_task(db, task_id)` — calls `validate_task_exists` then layers trigger_type=manual + active status checks, returns `Option<String>`. Most task-accepting tools use layer 2 (`get_task`, `cancel_task`, `complete_task`, `update_task_status`, `check_task`); `delegate_task` and long-running dispatch use layer 3. `create_task` (`parent_task_id`) uses layer 1 only + `get_task_depth`.

**Live-pilot guard on `complete_task` (mika#2739):** after the layer-2 validation, after the `trigger_type=callback` refusal and after the status refusal — and **before any write** — `complete_task` asks whether a pilot is alive under the row. The question has a single reader, `live_pilot::live_pilot_for_task` (mika#2653 / mika#2670); the tool composes none of its terms itself. A proven live pilot and an **unreadable** verdict both refuse (fail-closed): the cost of completing a row whose pilot is in fact running is a whole dispatch thrown away — the `implement` slot is freed mid-flight and the pilot's own callback later lands on a terminal row, where its result is dropped (measured 2026-10-16 13:19Z on mika#2700: three pilots under a cap of 2). The refusal body carries the PID, the task label and the row bearing the pgid, names the two correct exits and whose the lift is, and names no command — `MIKA_DEV_IDENTITY` carries `shell-exec`. It also names the premise that produced the incident: a parent task in status `completed` is the normal state after a groom (the engine reuses that task to relaunch the pilot, mika#1614), not evidence of an orphaned callback. **Named cost of the fail-closed:** a callback row carrying a `process_id` with no readable `process_start_time` (the population mika#2335 counts under `unusable_child_count`) can no longer be completed by the LLM tool; bounded by `timeout_at` (6 h panic-fallback) and counted under its own audit value. The guard is **unconditional** — no turn-class term — because no reference site of the tool is an operator path and there is no `mika tasks complete`. (Its `cancel_task` sister carried one until mika#2763, which removed it: a silent `deferred_dispatch` turn posts `is_webhook_pr_event_turn: false`, so the guard did not arm, and on 2026-10-17 at 07:59:38Z such a turn cancelled a 4-minute-old live groom callback. Both guards are now unconditional, for the same reason: a refusal is reversible and a kill is not.) The nominal delivery path, `POST /tasks/{id}/complete` → `update_task_completed`, does not go through the tool and is deliberately left ungated: `dispatch-lib`'s EXIT trap delivers the callback from inside the still-running wrapper, so there the process *is* alive and the completion *is* legitimate. All three outcomes write an audit row under `tool_name = complete_task_pilot_guard`, fire-and-forget (residue `complete_task_pilot_guard_audit_failed`), with four `after_value`s shared by import with the `cancel_task` guard — `blocked_live_pilot`, `blocked_db_unreadable`, `blocked_start_time_unreadable`, `allowed_no_pilot`. The pass writes too: without that positive control, "zero refusals" and "the guard never ran" render identical bytes. `tool_name`, never the value, is what separates the two populations.

**Cross-agent file access:** `read_agent_file`, `write_agent_file`, and `list_agent_files` accept an optional `agent` parameter for orchestrator-only cross-agent file access. `resolve_agent_home(agent_param, ctx)` helper validates permissions.

**Core memory path guard (#645):** `read_agent_file` rejects paths targeting core_memory sections with a domain-specific error before reaching `validate_and_resolve_path`. `is_core_memory_path(path)` matches `core_memory/` and `core-memory/` prefixes, bare section names (with or without `.md`), tilde/dot-prefixed variants, and exact directory names. Uses `core_memory_section_names()` from `db.rs` as single source of truth. The system prompt's core_memory preamble and tool-usage section also warn against reading core_memory via file tools (defense-in-depth).

**Context-redundancy guards (#647):** Pre-tool checks that detect when read tools request data already in the agent's context. Three guards extend the #645 pattern: (1) `read_agent_file` rejects paths matching active skill prompt files (`is_active_skill_prompt()` checks `ToolContext.active_skill_paths`); (2) `search_memory` hard-redirects `category="core_memory"` since core memory is always in the system prompt; (3) `search_memory` appends a soft hint when `category="all"` and the query matches a `core_memory_section_names()` entry. All guards use `ToolOutput::error()` for definitive redirects and hints prepended to success results for soft nudges. Path normalization shared via `normalize_path_prefix()` helper. Guard ordering: core_memory path guard → skill prompt guard → normal execution.

### Post-Action Hooks (mika#772)

`src/tools/post_action_hooks.rs` — Registry-driven side-effect callbacks that fire after specific tool calls succeed. Architecturally distinct from the EndTurn post-condition guard chain (§ Post-Conditions): hooks run AFTER a tool call's side effects commit, BEFORE the tool result returns to the LLM. Failure is warn-and-continue — hooks never affect the tool's own result or the agent loop.

**Registered hooks:**

| Hook | Tool | Fires when | Action |
|------|------|-----------|--------|
| Task completion auto-fact | `update_task_status` | `status="completed"` on non-milestone, non-project tasks | Emits `store_fact(category="event")` via `db.add_event()` with structured description (repo#issue, PR URL, turns, cost, task ID) |

**Wiring:** `run_post_action_hooks()` is called from `tool_execution/dispatch.rs` after `execute_tool()` returns, before tool-call recording and summary building. Uses `AsyncDatabase` directly (same write path as the `store_fact` tool), NOT re-dispatch through `ToolRegistry` (avoids recursive dispatch + audit_events double-counting).

**Milestone/project exclusion:** Iteration 1 excludes `type="milestone"` and `type="project"` tasks — existing prompt directives handle those. **Graceful degradation:** Missing `claude_pilot` metadata fields (hand-completed tasks) use `—` placeholders.

### Management Tools

14 tools for multi-agent/team workflows (`create_agent`, `list_agents`, `create_team`, `delete_team`, `update_team`, `add_team_member`, `remove_team_member`, `delegate_task`, `list_teams`, `run_team`, `get_team_status`, `get_team_history`, `create_task`, `update_task_status`). `create_agent`, `list_agents`, `create_team` always registered; others added when `agents.len() > 1 || !teams.is_empty()`. Orchestrator guards: only default agent or team-listed orchestrators can delegate/run teams; self-delegation blocked. **Task guard:** `delegate_task` and long-running skills require `task_id` referencing an active manual task. Per-tool timeouts: `run_team` (300s), `delegate_task` (120s).

**Delegate session persistence:** `delegate_task` creates a `delegate-{uuid}` session with parent linkage, persists task and response as messages. `AgentParams` has `global_home_dir` distinct from per-agent `home_dir`. **Team conversation continuity:** injects previous run context into orchestrator's system prompt. **Coverage check (#286):** `decompose()` re-prompts once if the orchestrator silently omits team members; falls through with `warn!` log (`team_coverage_gap`) on second miss. `TeamRun.coverage_retry_fired` bool persists via checkpoint JSON.

**Local-member reach: name-addressed, not `a2a_call` (mika#1653).** `delegate_task` is the **conversation-mode** local-delegation tool (name-addressed). In a **team run** there is no per-member reach tool: the orchestrator reaches members by listing them in the decompose-JSON task-assignment array (`[{"agent","task","output_file"}]`); the engine spawns each assigned member's session by name (decompose→spawn→resume) and members exchange results via workspace files. `a2a_call` is **suppressed from the team-mode tool array** (`teams::engine::build_team_tool_registry` removes the `TEAM_SUPPRESSED_TOOLS` entries from the team's private `ToolRegistry`; regression-gated by `test_team_registry_suppresses_a2a_call`). `a2a_call` is **remote-only** — external, cross-container agents addressed by URL; it has no local fast-path and 503s on local siblings (no gateway route exists for `~/.mika/agents/<name>`). Its description and SSRF-rejection error redirect local targets to `delegate_task` (ADR-009). Composes with mika#1652 (containment): this prevents the 503-loop, the reaper catches residual stuck `team_runs`.

**A run's terminal disposition is one compile-checked decision (mika#1940).**
`RunStatus::disposition(&self) -> RunDisposition<'_>` (`teams/types.rs`) is **the**
exhaustive match over the enum, with no `_` arm; `is_terminal_failure()` and the
failure reason are *derived* from it, never written beside it, which makes "it is
a failure ⟺ it has a reason" true by construction rather than true by test. The
constant `NO_DELEGATION_REASON` gives the field-less `FailedNoDelegation` variant
a single write site for the **operator** register — `engine.rs`'s
`team_runs.failure_reason` column and the CLI's `stderr`; `notification.rs` keeps
its own, deliberately different, **user** register (mika#2290/#2292's rule).

**The split it closed is between forms, not between authors.** `FailedNoDelegation`
(mika#1676) and `FailedTransport` (mika#1671) were added after the CLI was
written. The three exhaustive matches in this crate — `Display`, the DB-column
match in `engine.rs`, `notification.rs` — followed **both**; the three
hand-written patterns in `mika-cli` (`matches!`, two `if let`) followed **neither**.
`matches!` and `if let` are exactly the two forms that keep compiling when a
variant appears, i.e. the class mika#2023 M2 had to name in writing about
`tier == AgentTier::Family`. The remedy was not six more arms but taking the right
to enumerate away from the call sites: `mika-cli` now classifies through
`commands/team_outcome.rs`, which names no variant, and
`mika1940_no_hand_written_run_status_predicate_in_the_cli` (a `ProductionScanner`
source scan) refuses a new one — **with an allowlist shipped empty**, since there
is nothing to exempt, so no slot in which to drop the next lapse (mika#2323).
Verified by hand at delivery: a seventh variant fails to compile at exactly four
sites — `disposition()`, `Display`, `engine.rs`'s column match, `notification.rs`
— and **zero** in `mika-cli`. Exit-code and `stdout` contract: `crates/mika-cli/CLAUDE.md`
§ `mika ask`.

### Task Tracking

4 tools: **Write** (orchestrator-only): `create_task`, `update_task_status`. **Read** (all agents): `list_tasks`, `check_task` (with optional GitHub PR/issue status enrichment). Tasks reuse the `tasks` table with `trigger_type='manual'` + `action_type='none'`.

**`list_tasks` output enrichment:** Response includes a `Summary:` line with status-count breakdown (e.g., `"50 items total — 2 blocked, 48 completed"`) computed in-memory from the result set. Fully unfiltered calls also include a `Note:` with filter guidance discouraging redundant re-filtering. Filtered calls (by status or source) get a scoped summary but no guidance note. Per-task lines include `dispatch_class` when not the default `"implement"` (shows `class:groom` for groom-class tasks, #1172 W2). `get_task` always shows `Dispatch class:` in output (#1172 W2). See #572.

**Status transition state machine:** `pending` -> any; `in_progress` -> blocked/completed/cancelled; `blocked` -> in_progress/completed/cancelled; `cancelled` -> in_progress (cancel-and-retry path, mika#856). `completed` is terminal — status cannot transition, but metadata can still be written (#617). Cancelled tasks can be reverted to in_progress, reusing the original row instead of creating a new task; while cancelled, metadata writes (e.g. via `cancelled_reason`) continue to work.

**Phantom retry guard (#579):** `update_task_status` rejects retry-semantic metadata writes (any top-level key containing "retry", case-insensitive) when the task has an active callback child task (`trigger_type="callback"` in `pending` or `in_progress` status). Returns structured JSON error `retry_metadata_rejected_active_dispatch`. Fail-open on `get_child_tasks` DB error — the dispatch readiness guard (#525) is the primary defense against re-dispatch. Non-retry metadata writes are unaffected.

**Idempotent creation:** Deduplicates on `reference_url` (DB partial unique index `idx_tasks_manual_active_ref_url`) and on label (case-insensitive pre-check). Five loop-prevention guards. Max 25 agent-created items per session (configurable via `max_agent_tasks_per_session`, user_request exempt).

**Note on renamed scheduled task tools:** The former scheduled-task tools `create_task` and `list_tasks` have been renamed to `create_scheduled_task` (in `create_scheduled_task.rs`) and `list_scheduled_tasks` (in `list_scheduled_tasks.rs`) to avoid name collisions with the task tracking tools above. `list_scheduled_tasks` supports an optional `label_contains` string parameter for substring filtering (e.g., `label_contains: ":deferred"` to find pending deferred dispatch wrappers, #1172 W1).

### PR Merge Gate

`pr_merge_with_gate` builtin tool — structural backstop against merging PRs with failing CI checks. Registered in `default_tools()` (all agents, including delegates). Returns a tagged-union `MergeGateResult` via `#[serde(tag = "action")]` — the LLM branches on the `action` field. Five variants: `merged`, `blocked` (with `BlockReason` sub-enum and backward-compat `failing_checks`), `already_merged`, `gate_errored` (with `GateErrorKind` sub-enum: `gh_cli_failure`, `credential_scope`, `network_error`, `parse_error`, `unknown`), `branch_updated`; plus the retired `auto_merge_enabled`, which no site produces since mika#2617 and which is kept as a wire format. Preflight `gh pr view` detects CONFLICTING/CLOSED/DRAFT before attempting merge (#794).

#### The gate reads every check, and no longer delegates "green" (mika#2617, phase A)

**The failure, measured 2026-10-01.** PR #2614 was merged **by the engine**
(`mergedBy: mika-platform-dev`, 02:54:59Z) with `Egress Uniqueness Lint` and
`Egress Manifest Lint` **already in FAILURE** on its head `8ccaabc8`. `main`
went red (`41a4a20e`) and every open PR inherited it — mika#2616, a p0. The
cause is one flag: `gh pr checks --required` (mika#485/#490), which restricts
the gate to the checks branch protection marks required. The two egress lints
are not among them, so for the gate that PR was green — while the orchestrator
recipe, which looks at every check, would never have merged it. **The engine
gate was laxer than the human recipe, and it is the one that merged.**

**U1 — one flag, four consumers.** `run_gh_checks_raw` is the single site of the
`gh pr checks` argv, now composed by the pure `gh_checks_args`. Four consumers
descend from it and **all four inherit for free, because they decide through
`classify_checks`**, which does not change by a line: the tool, `verdict_handler`,
`ci_success_handler`, and the mika#2455 CI↔verdict guard of qa-review. That last
one is AC6, and it is free by construction — mika#2455 already paid for the single
reader, writing why: *"a second definition of «required» is the class
`grooming_marker` (mika#2158) had to close after months of silent divergence."*

A **fifth** consumer does not decide, it **collects**:
`ci_failure_handler::fetch_failure_context` filters `fail|cancel` by hand to build
the repair context. Its population grew to every red check, which is two gains and
one cost — see its doc-comment, where the cost (a build failure behind several red
lints loses its log to `MAX_FAILING_JOBS`) is written rather than compensated.

**U2 — `--auto` was the last divergent definition of green, and AC1 implies it.**
On `HasPending`, two of the three merge paths armed `gh pr merge --auto`. GitHub
then merges as soon as the **required** checks pass — i.e. under the definition
U1 had just stopped using, which would have left the fix inert on that path.
`run_gh_merge` **lost its `auto` parameter**: the flag is inexpressible, not
discouraged, and the compiler forced the three remaining call sites (doctrine
mika#1991; a lexical scan over a positional `bool` would have been fragile). The
tool's pending arm became `BlockReason::ChecksPending` (wire name
`checks_pending`), `verdict_handler`'s became a `Passthrough` naming the running
checks.

**What that costs, named.** A PR whose CI is in flight is no longer merged by
GitHub behind us: it waits for a `check_suite.completed(success)`. If that webhook
is lost — mika#2334 measured the four places where it can be — the PR stays
**open**. The asymmetry decides: an open PR is visible and recoverable by hand, a
red `main` blocks the whole loop and is worth a p0. The re-entry point already
existed and already carried the right semantics: `ci_success_handler` requires a
strict `AllPassed` and never arms `--auto` (mika#571), so `--auto` was redundant
with it *and* the only one of the two using a foreign definition of green.

**The `mika#1211` supervisor write MOVED with the arm.** It existed to neutralise
the orphan reaper's `pr_url IS NULL` predicate (#871) for a supervisor whose child
waits on an open PR — the same state, now reached through `checks_pending`.
`write_pending_pr_url_to_supervisor` is the renamed call; dropping it with the arm
would have let a healthy supervisor be flipped to `failed` 600 s later.

**U4 — zero exemption list, and the detectors are structural because the class is
invisible to a behavioural test.** A list added tomorrow makes no decision wrong
the day it is written; it makes the gate laxer in silence.
`mika2617_no_check_exemption_list_exists` refuses a constant whose name joins
`CHECK` to a waiver word (`EXEMPT`/`ADVISORY`/`IGNORE`/`SKIP`/`ALLOW`/`WAIVE`) —
on the **name**, whatever the type, since `&[&str]` and `HashSet` say the same
thing. `mika2617_no_label_is_read_in_the_merge_decision` refuses a GitHub label
read in the four gate files (`.labels`, `"labels"`, `--add-label`,
`--remove-label` — deliberately distinct from `Task.label` /
`DEFERRED_DISPATCH_LABEL`, which populate those files legitimately): a label is
human-writable, and branching on it would turn an interface gesture into a merge
authorization, the refusal mika#2248 already had to make for `merge-ready`. Both
ship their **good-faith control** (a red fixture), without which a predicate gone
inert reads exactly like a clean tree (class mika#2103 / mika#2205). Both
allowlists ship **empty and are pinned empty** — when one fires, remove the site,
do not add a line (doctrine mika#2201). The behavioural half is
`mika2617_classify_checks_is_blind_to_the_check_name`, a property test over the 22
names of Vincent's ratified bridge plus `Docker Build`, `validate`,
`Pilot Egress Status Tap` and an arbitrary one.

**An unknown bucket is reported, and stays non-blocking.** `classify_checks`
refuses only `fail`/`cancel`, so any bucket `gh` adds tomorrow would be born
non-blocking — a fail-open on a merge gate. The policy is **deliberately
unchanged** (refusing on the unknown would break merging on another `gh` version,
i.e. trade a red `main` for a stopped loop); only its silence is.
`merge_gate_unknown_check_bucket` (WARN, fields `repo`, `pr`, `name`, `bucket`) is
emitted per unknown bucket, and nothing per nominal evaluation (mika#2131).

**The decision is a pure function, and that is what R12 imposes.** `execute` has
no seam — it calls `run_gh_checks` and `run_gh_merge` directly, on free functions,
with nothing substitutable — so `decide_merge_gate(checks) -> MergeGateDecision`
is **extracted** rather than substituted (house motif: `decide_content_net`
mika#2270, `classify_wrapper_activity` mika#2184, `screen_target_purges`
mika#2619). `execute` matches it with **no `_ =>` arm**. The cases live in-crate
rather than under `tests/eval/`: the function is `pub(crate)`, and widening a
production function's visibility for a test file's location is the worse trade
(precedent written by mika#2532 for `spawn_long_running_exec`).

**What that does NOT attest:** that `execute` *calls* the decision, and calls it
with the right arguments. No behavioural test can see it — an `execute` ignoring
the decision would leave every case green. What covers it is the argv scan (U1),
the cardinality assertion on `run_gh_merge`'s call sites (three after U2, so the
assertion is as much a measure of U2 as a guard), and, in phase C, the
`MergeClearance` type witness, which makes the bypass **non-compilable** rather
than detectable.

**Phase C is NOT in this change.** The MPC gate trace (AC5) ships separately;
the plan
`docs/plans/2026-10-02-001-fix-2617-merge-gate-lit-tous-les-checks-plan.md` is the
design reference for all three phases. Phase A is autonomous: with no rerun, a red
check blocks, which is the safe behaviour. **Credential-scope diagnostic (mika#1616):** when a `gh` call fails with a 403 / "Resource not accessible by integration" / forbidden response (GitHub App not installed on the target repo, or PAT missing write scope), `classify_credential_scope_error()` returns `GateErrorKind::CredentialScope { repo }` with an actionable detail naming the repo + remediation, instead of an opaque `gh_cli_failure`. Wired into all four `gh` failure sites (preflight, checks, auto-merge, immediate-merge). Mirrors the `classify_gh_error()` heuristic in `builtin_handlers.rs`. This stops the LLM from paraphrasing an opaque exit code into a fabricated cause (the reported symptom on mika-cloud PRs #135/#136). Decision matrix: CONFLICTING/DIRTY -> blocked[merge_conflict]; fail/cancel -> blocked[required_check_failed]; pending -> auto-merge; all pass -> immediate merge; already merged -> no-op; infra failure -> gate_errored. 60s timeout. Requires `ctx.github_token`. See #490, #794.

#### Rerun once, then block (mika#2617, phase B)

`crate::merge_gate_rerun` relaunches the failed jobs of a red run **once**, and
never twice. Prime, verbatim: *« si le re-run passe c'est vert pour de bon, s'il
échoue deux fois ce n'est pas flaky, c'est cassé, et ça doit bloquer. »*

**It decides nothing about the merge, and that is the first thing to know.** The
gate is already closed when it is called — phase A's
`MergeGateDecision::ChecksFailed` refuses, and its refusal is not conditioned on
anything that happens here. This module has a side effect (it relaunches) and
returns a `RerunOutcome` the caller **quotes** in the motive it exposes. One test
exists for exactly that property
(`mika2617_no_rerun_outcome_claims_the_gate_may_open`): a future editor who makes
one of those sentences say *"the merge may proceed"* would be moving the merge
decision into a module that has no business taking it.

**`--failed`, never `--job <id>` (plan R1).** The two flags are mutually
exclusive, so AC2's literal form is unexecutable — and the correction is a
**gain** rather than a fallback: N red lints of the same `ci.yml` run are covered
by **one** relaunch where `--job` would need N. The granularity of the rerun is
therefore the **run**, and the budget is counted per `(repo, PR, head, run)`.

**Named limit: one run, even on a PR red across several.** "One run is enough"
holds for N lints *within* a run; this repo ships five workflows, so a PR red on
both `ci.yml` and `pr-body-validation.yml` is red on **two** runs and only the
first is relaunched. The ledger key already carries the `run_id`, so handling
every red run in one pass would be budget-safe — what keeps it out of phase B is
that it widens the rerun's outcome from one value to a **set**, hence its single
reader and the three call sites. Follow-up named, precondition measured: PRs red
across several runs inside the `merge_gate_check_rerun` population.

**The link grammar has ONE reader, promoted rather than twinned (plan R14).**
`parse_check_link` moved out of `ci_failure_handler` into
[`crate::check_link`] verbatim, tests included; U3 derives its `run_id` from it
and adds only the `&str → u64` conversion plus the refusal of a non-numeric run.
Writing `extract_actions_run_id` beside it would have been the second reader the
same plan's R11 refuses for the head SHA — being inconsistent from one
rectification to the next would be worse than either choice.
`mika2617_run_id_has_a_single_link_reader` refuses its return, allowlist shipped
empty, with its anti-vacuity assertion. **Inherited behaviour, pinned as such:**
`parse_check_link` requires the `/job/` segment and returns `None` without it
(its own `parse_check_link_no_job_segment` since #594), so a run link without a
job reads as "no rerun" — the right default, and *inherited rather than decided*,
which is why it carries a test of its own instead of being left to be
rediscovered as an oversight.

**The ledger is durable, and the key carries the head.** An in-memory ledger does
not hold "never a second rerun" across a restart, so the budget lives in
`audit_events` under `merge_gate_check_rerun`, key
`rerun:{repo}#{pr}@{head_sha}:{run_id}`, read by
`count_recent_audit_events_for_target` by **exact equality** (motif mika#1869 /
mika#2347). Two properties follow: **a new sha reopens the budget by itself** —
new code, new chance, the idempotence obtained from the *shape* of the key rather
than from one more column — and mika#2347's `#234` ↔ `#2343` trap cannot open
here, the comparison being an equality rather than a `LIKE`. The `@` is written
anyway so a future prefix reader is safe by construction rather than by luck.

**`PrPreflight` gained `head_ref_oid`, and an empty value is UNREADABLE, never a
SHA.** `#[serde(default)]` renders an absent `headRefOid` as `""`; treating that
as a head would make the ledger key shared by every unreadable head — one budget
for all of them. The rerun refuses on empty (`LedgerUnreadable`). Zero network
call is added at the tool site and at `ci_success_handler` (which already holds
`pr.head_sha` from `find_open_pr`) — **this is where plan R11 needed correcting**:
`verdict_handler`'s `HasFailures` arm `return`s *before* the behind-main preflight,
so it resolves the head through `fetch_pr_head_sha` (mika#1563), one round trip on
a path that is already red and about to make a network call anyway. Hoisting it
above the arm would charge every branch, the nominal one included.

**And that one round trip is itself conditional**, through
`rerun_needs_the_head` — the two free terms (a derivable run, the kill-switch)
exposed as one predicate. Without it the module's cheapest-first ordering buys
nothing: the caller pays the most expensive term first, and pays it for nothing
on a disarmed process or on a PR whose red checks carry no Actions run. One
predicate rather than two reads at the call site, so "will the rerun look at the
head?" cannot drift from the real order of the terms.

**Fail-CLOSED on the ledger — the inverse of the rest of this work, and the
arbitration is local.** A false "already rerun" costs one lost relaunch on a PR
that is waiting for a human anyway; a false "never rerun" relaunches in a loop,
which AC2 forbids by name. So an unreadable database, an empty head, or a failed
ledger write all mean: no relaunch.

**The ledger row is written BEFORE the relaunch, and that ordering is the
mechanism.** `audit_events` is append-only: a row cannot be revised. Written
*after* the relaunch, a crash in between would reopen the budget and the relaunch
would replay — exactly what AC2 forbids. So it is written first and carries
`attempted`: at that instant the outcome is unknown, and naming it would be
inventing. The outcome lives on the **log line** (`outcome` field). Named
consequence: a relaunch refused with a 403 **spends** the budget for that head —
the letter of the plan's T8 ("aucune boucle") — and a new sha reopens it.

**`AlreadySpent` writes NO audit row, which revises the plan.** It announced "two
failures ⇒ `merge_gate_rerun_exhausted` (WARN + audit row)". The row is dropped,
and the reason is measurable: one push produces up to eight
`check_suite.completed` (mika#1869), so a row per *evaluation* of an
already-relaunched head would be up to eight rows per push over a population that
does not change — the churn doctrine mika#2131 bounds — **and** it would pollute
the count that holds the "never twice" invariant, since that predicate *is* a
count. The durable information ("this head had its relaunch") is already the
attempt row; exhaustion is its corollary, not a new fact. The WARN stays:
liveness is what the operator reads.

**One reader of the enum, and a scan that keeps it one.** `rerun_detail_suffix`
is the single `match` on `RerunOutcome`, exhaustive **with no `_ =>` arm**; the
three call sites quote it rather than matching themselves. Three copied `match`es
are three formulations free to diverge, which is the class `grooming_marker`
(mika#2158) had to close. `mika2617_rerun_outcome_has_no_wildcard_arm` carries two
terms — no wildcard in any function that *reads* a `RerunOutcome` (scoped by
`source_scan::fn_bodies`, so the legitimate `_ =>` over an **environment string**
in `merge_gate_rerun_is_enabled` is out of population, the unknown being exactly
what a kill-switch must catch there), and no second reader in the three consumer
files — plus a good-faith control on a fixture, because a predicate gone inert
reads exactly like a clean module (class mika#2103 / mika#2205).

**AC2 is shipped FAIL-SAFE and is NOT claimed held (plan R3).**
`gh run rerun --failed` needs the `actions: write` scope, which the platform
token's documented scopes do not carry. **AC2 may therefore be inert at deploy,
in 403**: the relaunch fails, the gate stays closed, the motive is named, and
that is the right state. Claiming it closed before an operator probe has observed
a successful relaunch would be the guarantee mika#2304 names. Kill-switch
`MIKA_MERGE_GATE_RERUN` (armed by default; polarity of
`MIKA_QA_CI_COHERENCE_GATE`): it gates the **relaunch**, never the reading nor the
refusal — the detection is unconditional (motif mika#2249 / mika#2272).

**Operator surfaces, expected regimes and the probes:** root `CLAUDE.md`
§ *Optional (relance une fois, puis blocage — mika#2617)*.

#### MPC gate trace at the merge path (mika#2617, phase C)

For `senara-solutions/claude-pilot` (and its pre-rename alias
`senara-solutions/claude-pilot-py`, which GitHub still redirects to the same PR),
an autonomous merge additionally requires a PR comment carrying
`<!-- mpc-gate: ok sha=<full head SHA> -->` — the orchestrator's ruling on **this**
head (AC5). Strict equality on the full SHA, so any later push invalidates the
marker by itself; a prefix is not an attestation. The pure half lives in
`mika_common::forge_identity` next to `merge_disposition`
(`MPC_GATE_REQUIRED_REPOS`, `extract_mpc_gate_shas`, `mpc_gate_verdict` →
`NotRequired | Attested | Missing | StaleSha`), because a second merge-policy list
is the drift this repo has paid twice. The marker is never read inside code —
fenced blocks or inline spans — since a comment *discussing* the format carries
the literal (mika#2050). Unreadable comments, a timeout (15 s) or an unknown head
are `Missing`: fail-closed, and it can only freeze the population's merges.

**AC5 cannot be bypassed by the two handlers, and that is a type, not a scan.**
`run_gh_merge` takes a `MergeClearance` by value; its field is private and its
only constructor, `MergeClearance::from_mpc_verdict`, returns `Some` only on
`NotRequired`/`Attested`. The tool, `verdict_handler` and `merge_ready_handler`
were forced by the compiler (plan R8).
`mika2617_merge_clearance_has_a_single_production_constructor_site` pins that the
witness is minted at one production site — `mpc_gate_clearance` — so no caller
can write `from_mpc_verdict(&MpcGateVerdict::NotRequired)` and skip the read.

**Zero added network call outside the population, by construction.**
`evaluate_mpc_gate` tests the population term **before** invoking its lazy reader
(`fetch_pr_comments`, `gh pr view <n> --json comments` — the only new read
capability of mika#2617, parsed as JSON so multi-line bodies stay whole). Pinned
by `mika2617_the_comment_reader_is_not_invoked_outside_the_population`, with its
positive control. The head SHA costs no resolution at the tool (preflight) nor at
`merge_ready_handler` (`signal.head_sha`); `verdict_handler` reuses its
behind-main preflight and resolves the head only for a population repo whose
preflight failed. In the tool the term runs last, inside the `AllChecksPassed`
arm, so a PR the gate refuses anyway never pays it.

Refusals: `blocked` / `mpc_gate_missing` | `mpc_gate_stale_sha` (tool, taught to
the three `self-dev*` prompts), `Passthrough` (verdict handler), `Handled` +
audit `merge_ready_mpc_gate_held` (merge actor). WARN `mpc_gate_refused` and
`mpc_gate_comments_unreadable` are distinct names on purpose — opposite remedies
(post the marker vs repair API access). Surfaces and the S6 probe: root
`CLAUDE.md` § *La trace de gate MPC, au chemin de merge (mika#2617, phase C)*.

**Supervisor pr_url write on `checks_pending` (mika#1211, arm moved by mika#2617):** On the pending-checks branch — `auto_merge_enabled` until 2026-10-02 — the tool writes `$.claude_pilot.pr_url = "https://github.com/<owner>/<repo>/pull/<n>"` to the supervisor task's metadata (resolved via `ToolContext.callback_task_id → parent`, gated by `trigger_type='manual' && source='self_dev'`). This neutralises the orphan reaper's `pr_url IS NULL` predicate (#871) and arms the parent-completer (mika#1162), so the supervisor stays `in_progress` until the dispatch callback ages past `REAPER_GRACE_SECONDS` and is then promoted to `completed`. Mirrors `dispatcher::try_extract_callback_metadata` (#376): two-level shallow merge via `task_metadata::merge_metadata`, fire-and-forget on error. Conversation-mode invocations (no `callback_task_id`) skip the write silently. See `docs/solutions/best-practices/pr-merge-with-gate-supervisor-metadata-2026-05-20.md`.

### Issue Dependency Resolution

`resolve_issue_order` builtin tool — resolves dependency-aware execution order for a set of GitHub issues using `blockedBy` GraphQL edges. Input: `{ repo: "owner/repo", issues: [1, 2, 3] }`. For each issue, queries GitHub GraphQL API (via shared `github_graphql` module) for blocked-by relationships, builds a DAG, runs Kahn's algorithm with issue-number-ascending tiebreaker, and returns `{ sorted, edges, external_blockers, cycle }`. Cycle detection: if the DAG has a cycle, `sorted` is cleared and `cycle` lists the cycle members. External blockers (issues outside the input list) are tracked separately and do not affect the sort order. Fail-open: returns input order with a warning when no GitHub token is configured. 60s timeout. Used by the self-dev milestone workflow (M2b step) to order issues before dispatch. See #714.

**Shared `github_graphql` module:** `fetch_open_blockers()` and `extract_open_blocker_numbers()` extracted from `skills/executor.rs` into `crate::github_graphql` for reuse by both the blocked-by dispatch guard (#713) and `resolve_issue_order`.

### GitHub Read-Only Handler

`gh_read` builtin handler (#811, #817) — read-only GitHub CLI operations for the mika-arch architect agent. Input: `{"op": "<operation>", "target": "<number>", "repo": "owner/repo"}`. Five allowed ops: `issue_view` (→ `gh issue view --json`), `pr_view` (→ `gh pr view --json`), `pr_diff` (→ `gh pr diff`), `issue_list` (→ `gh issue list --json`, optional target as milestone number or label filter), `file_view` (→ `gh api /repos/{owner}/{repo}/contents/{path}?ref={ref}`, base64 decode, returns `{content, ref, path, size_bytes}`). Structured error variants: `NotFound`, `AuthFailed`, `RateLimited`, `NetworkError`, `MalformedRequest`, `FileTooLarge` (#817). `FileTooLarge` fires when GitHub returns empty content + non-zero size for files > `FILE_VIEW_MAX_BYTES` (1 MiB, matching GitHub's contents API cap). Files > 100 MiB hit GitHub's 403 response, classified as `AuthFailed` via existing `classify_gh_error()` — this is a pre-existing GitHub boundary, not a regression. `file_view` input validation: `path` charset-restricted to `[A-Za-z0-9._/-]` (prevents URL-decoding attacks), no `..`, no leading `/` or `-`; `ref` optional (defaults to `main`), no leading `-`, max 256 chars. Error classification via `classify_gh_error()` on `spawn_and_collect` output content prefix (`"Exit code:"` detection for non-zero exits). Audit log line `gh_read_invocation` with `agent_id`, `op`, `resource` (`<ref>:<path>` for file_view, target number for other ops), `repo`, `latency_ms`, `status`, `blob_sha` (file_view only — resolved file content sha from GitHub response, cost-free) fields. Auth reuses `ToolContext.github_token` (shared GitHub App installation). Declared in skill `tools.json` with `"handler": {"type": "builtin", "function": "gh_read"}`. See #811, #817.

### `run_gh` — GitHub CLI Handler

General-purpose `gh` CLI handler. Four-tier validation: (1) global subcommand allowlist (`GH_ALLOWED_SUBCOMMANDS`: pr, issue, run, workflow, release, repo, search, label, api), (2) skill-scoped scope gate (`validate_qa_review_gh_scope` for qa-review, mika#1196), (3) `gh api` per-method gating (`validate_gh_api_scope`, mika#1167). **`gh api` per-method gating (#1167, evolved from #805 + #1153):** `gh api` is in the global subcommand allowlist but further restricted via a method+path allow matrix (`GH_API_ALLOW_MATRIX`). Each entry defines an HTTP method + API path regex + rule name. The matrix is deny-by-default: any combination not matching at least one entry is rejected by `validate_gh_api_scope()`. Initial entries: 4 GET (branch, branches-list, commit, milestone) + 1 PATCH (milestone-update). Matrix compiled once via `LazyLock`. Audit event `gh_api_invocation` includes `allowed_by_rule` for structured anomaly detection. Adding a new endpoint requires adding a `GhApiAllowEntry` to the matrix — no other code changes needed.

**Destructive-action grounding gate (mika#1646, tier 4).** `validate_destructive_action_grounding` refuses `gh pr close` / `gh issue close` that cannot show it rests on the target's current state, and refuses a *repeat* close that does not acknowledge the prior one. Predicates live in `evidence::guards` next to their siblings assert-grounded (mika#1331) and equivalence-claim (mika#1645); the enforcement is here, in the pre-subprocess chain, and that placement is the whole point.

**Why not an EndTurn guard.** Its two siblings read assistant *text* and re-prompt — the right shape when the defect is a sentence. mika#1646's defect is the call. `gh pr close 1644` leaves at step 3 of the tool loop, so by the time the EndTurn arm runs the PR is closed and a re-prompt can only annotate an accomplished fact. Hence the pre-subprocess placement, alongside mika#1196 / mika#1682 / mika#1167 (and consistent with the Layer 4 note in `tool_execution::dispatch`: builtins like `run_gh` have no `data_grade`, so their gate lives in the handler).

**Layer A (AC1) — grounded, and said so.** The turn must contain a read of the target (`gh pr view --json files`, `gh pr diff`, `gh issue view`, or a qa/arch read path — resolved from `tool_calls` by `trace_id`), *and* the close comment must cite what the read showed. Both halves are required: the founding incident's two close comments each paraphrased an upstream verdict and cited nothing checkable, so a read-without-citation is indistinguishable from no read at all. The satisfaction predicate excludes the close call itself, or `gh pr close 1644` would ground `gh pr close 1644`.

**Layer B (AC2) — a second execution knows it is a second execution.** If the same close ran against the same target within `MIKA_DEV_REPEAT_ACTION_WINDOW_SECS` (default 1800), the comment must acknowledge the prior one. Detection queries the persisted `tool_calls` table **scoped to the agent** — not the turn, not the session. This is load-bearing, not incidental: the founding incident's second close (11:15:49Z) came from a deferred webhook replay of the 11:12:11Z event, a context sharing no in-memory state with the first close at 11:08:54Z. A session-scoped or turn-scoped check finds nothing there and waves it through. Verifying state before acting is also not enough on its own — between the read and the write, someone can reopen. Only the record makes the repeat knowable.

**Which way it fails.** Detection is fail-open: an argv not recognized as a close is none of the gate's business, so `gh` at large is untouched and the surface stays bounded to `pr close` / `issue close`. Everything after recognition is fail-closed: once the call *is* a recognized destructive action, any inability to prove it founded — missing grounding, unreadable history, `MIKA_STORE_TOOL_CALLS` disabled, DB error — is a refusal. That inverts `assert_grounded`'s deliberately lean-narrow fail-open policy, and the inversion is deliberate. A ticket left open in error is visible and gets corrected; a ticket closed in error drops out of the count and nobody goes looking for it. Refusal bodies are structured JSON (`error`, `doctrine`, `target`, `reason`, `remedy`) and name the unblocking gesture.

**Audit (AC3).** Every decision — refusal *and* authorization — writes an `audit_events` row with `tool_name = "destructive_action_grounding"` and `target_key = "<pr|issue>:close:<n>"`. No migration was needed: `audit_events` has no `event_type` column, `tool_name` is free-form TEXT, and this follows `phantom_aged_out` (mika#1712) / `wip_rescue` (mika#1852). Query: `SELECT * FROM audit_events WHERE tool_name = 'destructive_action_grounding'`. Operator grep signal: `destructive_action_blocked` in `$MIKA_SPIRIT_LOG_FILE` — any hit is a close the engine stopped; sustained hits from one agent mean an upstream verdict source is producing ungrounded close recommendations (the mika#1645 class).

**Regression coverage.** Predicate units in `evidence::guards::tests`, repeat-detection SQL units in `db::tests` (cross-session, window edge, `#164` vs `#1644`, pr-vs-issue, cross-agent), and the AC4/AC5 calibration scenario `destructive_action_thread_reground` (mika-dev role suite) which replays the PR #1644 timeline and fails the model both for re-closing *and* for declining without engaging the contradiction.

**Verdict↔flag coherence gate (mika#2237, tier 5).** `validate_pr_review_flag_coherence` refuses a `gh pr review` whose flag contradicts the `VERDICT:` line in its own body. Same family as mika#1646 above, same reason for being pre-subprocess: the defect is the call, not a sentence — by the time an EndTurn arm ran, the review would be on GitHub. Placed immediately after `validate_review_depth_present`; depth is a condition of the body, the flag a condition of the act.

**The failure, 2026-09-08.** mika#2218 restored `--approve` by giving the reviewer a machine identity distinct from the author. On the first review after that deploy — mika#2236, body `VERDICT: pass ✅` — mika-qa posted `--comment` and **never tried** `--approve`: argv `["pr","review","2236","--comment",…]`, zero attempt. The skill mapped `pass → --approve`; what overrode it was the agent's own memory of the 137 pre-fix self-approval refusals. Re-measured since: the same agent posted **three `APPROVED` reviews later that day**, so the defensive memory was an *intermittent* arbitration, not a stable blocker — which is the measurement that rules out a memory-side remedy (a tag or a dated invalidation assumes persistent state to correct) in favour of a per-turn one.

**The mapping has one reader, derived from `Verdict`.** `required_review_flag` maps `Pass → --approve`, `Block(_) | Hold(_) → --comment`, `Missing → None`. Never a table transcribed from `qa-review/system_prompt.md`: the truth is the enum `verdict_handler` already consumes, which refuses to merge a `pass` whose `state != approved` — the same contract read at the other end. **Not a style preference but the condition of correctness**: a hand-rolled `contains("VERDICT: pass")` would fail open on `VERDICT: pass ✅`, i.e. on the literal body of the founding incident, and be indistinguishable from a guard that works. Going through `parse_verdict` inherits mika#1828's emphasis tolerance and mika#2239's trailing-decoration fallback **and their bounds** — `VERDICT: pass — but see findings` stays `Missing` (mika#1821), and leading decoration stays out of scope (mika#2239 D-D) and is deliberately **not** worked around here, which would create the second reader. Both bounds are pinned as decisions in `evidence::guards::tests::mika2237`.

**A third bound, on the guard's own side: long flag forms only.** `PR_REVIEW_FLAGS` carries `--approve` / `--comment` / `--request-changes` and not `-a` / `-c` / `-r`, so a short-form call falls open. That matches the body reader, which has read `--body` and not `-b` since mika#275, so the two halves of the recognition engage on the same population — the long-form shape qa-review's own table prescribes (`system_prompt.md:607-611`). **Widening the list alone would be worse than the gap**: with no canonicalization a correct `pr review N -a` on a `pass` body reads as posted `-a` ≠ required `--approve` and is refused, i.e. a legitimate review made impossible to post — the one outcome R8 forbids. Closing it means a canonicalizing map applied to the posted flag *and* the body reader, never a longer list here. Pinned by `short_flag_forms_are_out_of_the_population_and_that_is_pinned`.

**Gated on the BODY, never on the active skill.** Its neighbour `validate_review_depth_present` gates on `!ctx.required_tool_arg_suffixes.is_empty()`, a proxy for "qa-review is loaded". This one gates on its own subject, so a human or ad-hoc review with no `VERDICT:` line is never blocked, and the guard does not vanish in silence the day qa-review reorganizes its manifest.

**Both directions.** The measured defect is a degradation (`pass` → `--comment`), which is conservative; its inverse — `block[…]` posted as `--approve` — would merge a blocked PR. `--request-changes` is the mapping of no verdict and is refused on any classified one.

**The escape hatch is the fix, not its softening.** A guard that always refused `--comment` on `pass` would turn a real constraint into an inability to post the review at all — the turn loops and dies. So `--comment` on `pass` is allowed **iff** a `--approve` on the same PR was attempted in the same `trace_id` and failed (`!row.success`, which covers GitHub refusing, `gh` failing to spawn, and an upstream gate refusing — the three A2 populations). That is also the ticket's second ask made structural: a refusal means "was going to degrade without trying" (stale memory), the hatch means "tried and the door was shut" (a real constraint). Those two used to be separable only by reading argv by hand.

**Fail-OPEN on the hatch, the inverse of mika#1646.** The term the history carries is *the absence of an attempt*, and a term that cannot be read is never a satisfied term (mika#2277). Refusing on an unreadable or empty history would produce the loop `--approve` fails → `--comment` refused → `--approve` fails… Named cost: `MIKA_STORE_TOOL_CALLS=false` makes the history empty, so the guard is **inert** on the `pass → comment` direction (the `block → approve` direction consults no history and is unaffected). Same inertia shape as `MIKA_LOG_PILOT_TRANSCRIPTS` for the mika#2249 reaper, made visible the same way: the abstention grep.

**The refusal leaves an exit that is not a lie.** Naming only "post with `--approve`" would push a model held by its memory to rewrite its *verdict* (`pass → hold[review]`) instead of its flag — the same defect under another name, and the guard cannot adjudicate which verdict is right. The body names both correct ways out and ends on the sentence that **is** the ticket's fix (c) made operational: *"a failure recorded in your memory is not evidence about THIS pull request."* That workaround stays open, named, and is watched by halt 3 below and asserted by the `memory_vs_skill_no_verdict_degradation` calibration scenario.

**A defect found while wiring the tests, and load-bearing for the hatch.** The mika#821 session dedup registered its key on any `gh` invocation that merely spawned — and `spawn_and_collect` returns `ToolOutput::success` even on a non-zero exit. So a `--approve` **refused by GitHub** consumed the right to post, and the fallback `--comment` was rejected as `duplicate_pr_review`: the hatch was decorative. The ledger now registers only reviews that landed (`!is_error && !has_non_zero_exit_prefix`), the same predicate `tool_execution::dispatch` computes for every tool, so the dedup ledger and `tool_calls.success` agree on what a successful review is. Registering a review nobody posted was wrong independently of this ticket.

**Audit + operator greps.** Every non-nominal decision writes an `audit_events` row with `tool_name = "pr_review_flag_guard"`, `target_key = "pr_review:{repo}#{n}"` and `after_value` in `{refused, degraded_after_attempt, abstained}`; audit writes are warn-and-continue so losing the row never changes the verdict. Grep signals: `pr_review_flag_refused` (WARN — **expected non-empty**: each line is a degradation the memory still pushed and the engine stopped, i.e. the measure of the remanence nothing gave before; a plateau rather than a decay says the memory is being re-written, and *that* is when the tagging follow-up opens, with a count rather than an intuition), `pr_review_flag_degraded_after_attempt` (INFO), `pr_review_flag_guard_abstained` (WARN — expected zero; check `MIKA_STORE_TOOL_CALLS` before touching the predicate).

**Downstream half.** `verdict_handler`'s `Verdict::Pass` arm used to return `Passthrough` in silence when `state != "approved"` — no WARN, no audit row, no counter, which is where mika#2236 sat and what cost the defect eleven days of invisibility. It now emits `verdict_pass_without_approval` (WARN + audit row), **SOLE WRITER** pinned by a source scan with an allowlist shipped empty. Behaviour unchanged — the merge-safety gate still stands; what is added is attribution. Complementary population of mika#2239's `verdict_approved_but_unclassified`, and the two must stay countable apart. **Expected regime: zero**; any hit is a review that got past the pre-subprocess guard, and the halt is to establish *which path* posted (another agent, `run_gh_subprocess`, a binary predating the fix — class mika#2340) before widening anything.

**Regression coverage.** Predicate units in `evidence::guards::tests::mika2237` (both directions, the nine inherited body shapes with three negative controls, the hatch open and shut, URL ↔ bare-number matching); production path in `tests/eval/test_pr_review_flag_coherence_2237.rs` (`MockLlmProvider`, no network — plus three negative controls, without which "the guard decides" would be indistinguishable from "the guard blocks", and an audit-row assertion separating "accepted by the hatch" from "accepted by abstention"); U2 in `tests/eval/test_verdict_handler.rs` with its approved-pass negative control; and the two mika-qa calibration scenarios `memory_vs_skill_precedence` / `memory_vs_skill_no_verdict_degradation` (real-provider swap gate, mika#1190 — a text proxy, stated as such). Compound entry: `docs/solutions/best-practices/une-memoire-apprise-dun-echec-survit-au-fix-de-cet-echec-2026-09-19.md`.

### Un `block[pipeline]` cite une garde en échec, ou il ne part pas (mika#2687, vol 1)

**Le défaut, mesuré n=2 sur la même tête.** Sur mika#2685 (bump de lockfile
`quinn-proto` + exception `deny.toml` + doc dans `docs/solutions/`), mika-qa a
rendu `VERDICT: block[pipeline]` deux fois sur la tête `0a53b5f0` — 2026-10-13
11:54Z et 17:03Z — pendant que `scripts/verify-pipeline.sh` (check *Pipeline
Artifacts*) sortait **vert les deux fois**. Aucune des deux revues ne portait de
section `PIPELINE:` ni de ligne `NEGATIVE-TEST:`, et les deux raisons énonçaient
une règle que la garde ne porte pas (« plan manquant dans `docs/plans/` » ;
« label `pipeline-exempt` avec des fichiers non-docs ») — `verify-pipeline.sh`
compte `docs/solutions/` comme doc. Le prompt déployé l'interdisait déjà à ses
Step 2 et Step 2E. Même classe que mika#2172 et mika#2519, toutes deux fermées
par le prompt : **le correctif par le prompt seul ne tient pas**, le modèle
paraphrase une règle que le prompt a retirée. Le PR a été débloqué en ajoutant
un plan — on a satisfait la lecture erronée, pas corrigé la cause.

**La porte est la chaîne pré-subprocess de `run_gh`, pas `verdict_handler`.**
`validate_pipeline_block_grounding` refuse un `gh pr review` dont le corps porte
un `block[pipeline]` qui ne cite aucune garde en échec. Le critère d'acceptation
dit « n'est pas publié comme bloquant » : une rétrogradation dans
`verdict_handler` agirait **après la publication** — la revue serait sur GitHub,
lue par l'humain et par le dispatcher, et le handler ne corrigerait qu'une
conséquence. Même raison que mika#1646 et mika#2237 ci-dessus : le défaut est
l'appel.

**Position dans la chaîne.** Immédiatement **après** le garde verdict↔flag
(mika#2237), **avant** le garde CI↔verdict (mika#2455) et le garde
verdict↔dependabot (mika#2519/#2565). La chaîne va du plus local au plus
engageant : le nôtre est le dernier lien qui ne lit **ni le réseau ni la DB** —
il décide sur le corps seul. Conséquence sur une PR dependabot portant un
`block[pipeline]` sans section : c'est le nôtre qui répond, en amont de
mika#2519. Comme mika#2237, il est gardé **sur le corps** et non sur
`required_tool_arg_suffixes` : une revue humaine ou ad-hoc sans ligne
`VERDICT:` n'est jamais bloquée, et le garde ne disparaît pas en silence le jour
où qa-review réorganise son manifeste.

**Le prédicat, et sa population.** `classify_pipeline_block_grounding`
(`evidence::guards`) ne s'applique qu'à `parse_verdict(&body) == Verdict::Block(k)`
avec `k.eq_ignore_ascii_case("pipeline")` — hérité de `parse_verdict`, donc des
tolérances mika#1828/#2239 **et de leurs bornes**. Trois issues de ce côté-ci :
`RefusedSectionAbsent` (aucune ligne `PIPELINE:`), `RefusedNoFailingGuardCited`
(section présente, aucune citation d'échec), `Allowed`. Depuis mika#2961 la
fonction porte un second booléen et une quatrième issue, pour le membre `pass` —
§ suivant. La citation est une
**disjonction de trois motifs**, insensible à la casse et tolérante au préfixe
d'emphase markdown : (1) `exit` suivi d'un entier **non nul** (`PIPELINE_EXIT_CODE_RE`,
comparaison chiffre à chiffre, jamais un `parse`) ; (2) `REJECT:` ; (3)
`Verification FAILED`. Les trois tokens sont déclarés classe A dans
`scripts/canonical-tokens.tsv` (`PIPELINE`, `REJECT`, `FAILED`), donc un
renommage côté script casse la lint avant de casser le garde.
`PIPELINE: not-applicable (…)` — la forme Step 2D — est refusée délibérément :
un verdict qui bloque sur la pipeline tout en déclarant la pipeline non
applicable est exactement le défaut mesuré.

**Dans quel sens il échoue : fail-open.** La disjonction est volontairement
large. Un faux positif du refus rend une revue légitime **impossible à poster**
— le tour boucle et meurt (coût R8, cf. mika#2237) ; un faux négatif est le coût
déjà payé aujourd'hui. **Borne nommée :** un corps dont la seule mention
d'échec est un `exit 1` dans la prose d'une garde par ailleurs verte **passera**.
Lire la preuve d'exécution en DB — sur le patron B3 de mika#2565 — est le vol 2,
`Tracked in: senara-solutions/mika#2687`.

**Le gate, et son STOP.** `MIKA_PIPELINE_GROUNDING_GATE`, armé par défaut,
désarmé seulement par `0`/`false`/`off`/`no` ; toute autre valeur reste **armée**
avec un WARN citant la valeur d'origine trimée (mika#2220). Résolu une fois par
process (`OnceLock`), donc non commutable à chaud, et annoncé au démarrage par
`log_pipeline_grounding_gate_state()` quand il est désarmé (mika#2205).
**STOP opérateur : `MIKA_PIPELINE_GROUNDING_GATE=0` + redémarrage** — un
`block[pipeline]` non fondé repart alors, et c'est l'état déclaré. L'application
est scindée en un wrapper qui lit le gate à un seul site plus un corps
`…_gated(gate_enabled: bool, …)`, convention du dépôt (cf.
`classify_dependabot_verdict`) : le cache `OnceLock` rend `std::env::set_var`
intestable depuis un binaire de test partagé.

**Audit + greps opérateur.** Chaque décision — refus **et** autorisation —
écrit une ligne `audit_events` avec `tool_name = 'pipeline_grounding_guard'`,
`target_key = "pr_review:{repo}#{n}"` et `after_value` dans
`{refused_section_absent, refused_no_failing_guard_cited, allowed}`. La ligne
`allowed` est le **contrôle positif** de mika#2205 : une table vide dit que le
lien n'est pas atteint, pas qu'il ne refuse rien. Signal grep :
`pipeline_block_refused` (WARN, `reason` ∈ {`section_absent`,
`no_failing_guard_cited`}) — **attendu non vide**, chaque ligne est un
`block[pipeline]` que le moteur a retenu. Sonde, tables de lecture et les trois
haltes post-déploiement : `docs/operator/environment-variables.md` § *Un
`block[pipeline]` cite une garde en échec*.

**Couverture de régression.** Unités du prédicat dans
`evidence::guards::tests::mika2687` (15 cas : les deux corps mesurés du ticket,
les trois motifs un par un, le non-zéro contre `exit 0`, l'emphase markdown, le
`not-applicable`, et les contrôles négatifs `pass`/`hold`/`block[ac]`) ; câblage
dans `skills::builtin_handlers::mika2687_pipeline_grounding_tests` (6 cas sur le
chemin `run_gh` : le refus atteint l'appelant en `Err` avant tout subprocess, le
corps fondé passe, un `pass` passe, gate désarmé ⇒ même corps et il passe, plus
les lignes d'audit des deux côtés). Sans ce second Layer, **un prédicat qui
décide bien resterait compatible avec une garde que personne n'appelle**. Noter
que notre lien est en aval du `debug_assert!` de dedup mika#821 : un test au
niveau `run_gh` sur un `pr review` doit donc fournir `ctx.pr_reviews_posted` —
une map **vide**, car la peupler ferait répondre mika#821 avant nous.

### Et un `pass` dit ce que la garde a fait — la population passe à deux membres (mika#2961, vol 1/2)

**Le défaut, mesuré sur toute la population.** Le prompt de qa-review exige une
section `PIPELINE:` dans **chaque** verdict qui atteint Step 2E, et plafonne à
`hold[review]` celui qui en manque (l. 45). Sur 261 `pass` de
`mika-platform-qa` — 60 dernières PR de mika, claude-pilot, mika-platform et
control-monitor, relevé du 2026-10-29 — **zéro** porte une ligne `PIPELINE:`
ancrée. Aucun garde moteur ne tenait la règle sur cette population : mika#2687
ci-dessus a une population étroite et déclarée, `block[pipeline]` seul. Le cas
fondateur est `control-monitor#217` (14:26:22Z) : une approbation nue, postée une
seconde après une abstention du garde CI de mika#2455, sur un dépôt sans garde
exécutable et sans porte de merge.

**`contains` et un prédicat ancré ne comptent pas la même population**, et c'est
ce qui a corrigé la mesure du ticket. `pipeline_section_present` lit une ligne qui
**commence** par `PIPELINE:` (blancs de tête, course d'emphase markdown, casse
repliée) ; les quatre `pass` de mika#2691 que la mesure initiale comptait comme
conformes portent le mot dans leur **prose** — cette PR implémentait mika#2687 et
en parle — et zéro ligne ancrée. Régime réel : 426/426, pas 422/426. Un
`contains` compterait la prose d'une revue comme sa preuve, c'est-à-dire le
fail-open même que le ticket condamne. Le garde de cette doctrine est un test, pas
un commentaire : `mika2961_le_corps_de_2691_contient_le_mot_et_na_pas_la_section`
assère les **deux** faits sur le même corps mesuré, donc il rougit le jour où
quelqu'un réécrit le prédicat en `contains`.

**Un membre de plus, pas un garde de plus.** `classify_pipeline_block_grounding`
prend `verdict_is_pass: bool` en second argument — même convention que
`classify_dependabot_verdict` — et rend `RefusedPassSectionAbsent` sur un `pass`
sans section. Élargir plutôt que dupliquer, parce que la question est la même
(*ce corps porte-t-il la preuve de la garde ?*), que le prédicat de section est
le même, et que deux gardes voisins lisant le même corps auraient deux prédicats
à garder synchrones. Les deux booléens sont mutuellement exclusifs chez
l'appelant : un corps a une classe de verdict.

**Les deux autres refus de mika#2687 ne s'appliquent pas, et c'est un choix
argumenté.** `RefusedNoFailingGuardCited` serait un contresens sous un `pass` —
une approbation ne cite aucun échec par construction. `RefusedNoGuardExecuted`
lit le ledger d'outils : il reste réservé au membre `block`, donc **un `pass` ne
paie pas cette requête** et n'écrit jamais de ligne `abstained`.

**Polarité inverse, et c'est la seule de la famille.** `MIKA_QA_PASS_EVIDENCE_GATE`
est **désarmé par défaut** : `1`/`true`/`on`/`yes` arment, tout le reste laisse
désarmé (valeur non reconnue ⇒ WARN citant la valeur entre guillemets,
mika#2220). Ses quatre voisins sont armés par défaut, donc leur ligne de
démarrage signale une **dérogation** ; ici elle décrit le cas **nominal**, et
c'est le silence qui veut dire « armé ». Raison du défaut : armer d'emblée
changerait le régime de 261 verdicts d'un coup, et un refus tue le tour sans
aucune revue postée. L'ouverture est une décision de Vincent (AC4). Même
`OnceLock`, même scission wrapper/`…_gated`, pour la même raison qu'au § ci-dessus.

**Le remède ne cite pas la forme qu'il demande.** Un refus portant sa propre
ligne `PIPELINE:` **ancrée** satisferait ce garde-ci si le modèle le recollait
dans son corps suivant — le garde lirait alors son propre texte comme la preuve
du relecteur. Les deux sorties correctes (la section de Step 2E, la forme
*not-applicable* de Step 2D) sont donc **décrites** et jamais citées verbatim.
Pinné par `le_remedy_nomme_les_deux_sorties_sans_se_satisfaire_lui_meme`, qui
vérifie en plus qu'aucune ligne n'ouvre sur un libellé de verdict (`VERDICT_RE`
est ancré là).

**Audit et greps : noms d'événement distincts.** La ligne `audit_events` partage
`tool_name = 'pipeline_grounding_guard'` et son contrôle positif `allowed`, mais
le WARN est `pass_evidence_refused` (`reason = pass_section_absent`) et **non**
`pipeline_block_refused` : ce dernier est déjà un jeton de fil dans la doc
opérateur et dans les greps de `server.log`, et y classer un refus de `pass`
élargirait en silence une population publiée (mika#2131). La charge utile porte
`"error": "pass_evidence_absent"` et `"doctrine": "mika#2961"`, pour la même
raison. Sonde d'ouverture (AC5), tables de lecture et haltes :
`docs/operator/environment-variables.md` § *Un `pass` dit ce que la garde du
dépôt a fait*.

**Couverture de régression.** Layer A dans `evidence::guards::tests::mika2961`
(10 cas : les trois corps **mesurés** de cm#217, mika#2960 et mika#2691, le
contraste `contains`/ancrage, le `not-applicable` de Step 2D admis sous `pass` et
refusé sous `block` sur le **même** corps, le hors-population, la non-lecture du
ledger sur les deux valeurs du 4ᵉ argument, et la polarité de l'interrupteur) ;
Layer B dans `skills::builtin_handlers::mika2961_pass_evidence` (8 cas sur la
forme gatée). Ce module ne repasse pas par `run_gh` — l'interrupteur étant
désarmé par défaut derrière un `OnceLock`, un tour complet traverserait et irait
chercher `gh` ; les deux membres entrent dans le **même** appel, au même point de
chaîne, et le test `run_gh` de mika#2687 tient la position. Ce qu'aucun cas de
comportement ne pourrait voir — l'enveloppe non gatée figeant son second argument
à `false`, garde mort partout avec toutes les assertions vertes (classe
mika#2158) — est tenu par un scan de source à deux rôles : la décision et la
ligne de démarrage.

**Ce que le vol ne ferme pas.** La ligne `NEGATIVE-TEST:` (mika#2264) part au vol
2/2, avec son propre jeton canonique et son lint. L'abstention de mika#2455 reste
**fail-open**, inchangée : sur un dépôt qui ne merge pas par
`pr_merge_with_gate`, rien de mécanique ne tient le merge fermé — le vol ferme le
signal, pas la porte. **Aucun jeton canonique ajouté** :
`scripts/canonical-tokens.tsv` déclare déjà `PIPELINE` avec la tolérance
`ci+emphasis:line-anchored`.

### Per-Tool Skill Budget (mika#2276 M1)

**A skill tool runs under the budget of the skill that DEFINES it**, resolved once
per turn by `agent_loop::build_skill_tool_timeouts` into a `tool name → secs` map
threaded to `tool_execution::dispatch`. Sibling of `build_skill_tool_map` and
`build_skill_data_grades`: same `matched` slice, same order, same last-write-wins
collision rule — a tool dispatched to one skill's handler while cut at another
skill's budget is exactly the defect this closed. `max_skill_timeout` keeps its
maximum semantics and is now only the fallback for anything absent from the map.

**What it replaced.** That maximum used to be applied *uniformly to every tool
call*, so an outer skill loaded for an unrelated reason raised everyone's ceiling.
`shell-exec` declares 30 s and owns `run_shell`; `build-mika` declares 300 s and is
a declared dependency of `qa-review` — so a QA review turn ran `run_shell` under
300 s. Measured on PR #2275, trace `921f11f0`: two `cargo test --release` calls of
**237,9 s** and **231,1 s**, 469 s of a ~506 s envelope, then
`agent deadline exceeded` at `steps_completed=5` with no verdict written. And
`qa-review/system_prompt.md:194` said literally *"against `run_shell`'s 30s
budget"* — the doctrine reasoned on a floor the engine never held
(`feedback_prompt_enforcement_fragile`).

**Detail worth knowing before tuning any of these numbers.** Every skill with a
large declared budget (`dev-pilot`/`dev-groom`/`address-pr-comments`/
`resolve-pr-conflicts` at 600, `build-mika` at 300, `deploy-mika` at 120) exposes
**only `long_running` tools**, which return from `execute_skill_tool` *before* the
timeout is applied (detached spawn + callback). Those budgets therefore never
protected their own tools; their only observable effect was raising other skills'
ceiling. A value whose sole effect is on somebody else is a value nobody re-reads
— which is why `qa-review/skill.toml` now declares `timeout_secs = 30` explicitly
even though 30 is the manifest default (mika#2276 AC5).

**Fan-out:** `run_loop` has three callers (conversation `mod.rs` ~3382, silent
~4291, team ~4956) and all three build and thread the map. Compound entry:
`docs/solutions/best-practices/un-budget-declare-par-un-manifeste-doit-etre-celui-applique-2026-09-10.md`.

### Verdict Net (mika#2276 M2, generalised by mika#2368)

`server::deadline_verdict` — when a turn **owed a verdict on a PR and did not post
one**, the engine itself posts `VERDICT: hold[review]` on that PR.

**Five reasons since mika#2943, and the question the module answers changed at
the second.** It is no longer *"was the turn cut off?"* but *"did this turn owe a
verdict and fail to post one?"*:

| `VerdictReason` | ticket | the turn | `event_name()` | `cause()` |
|---|---|---|---|---|
| `CutOffByDeadline` | mika#2276 | webhook turn, never reached its conclusion | `qa_deadline_verdict` | *absent* |
| `CallbackConcludedWithoutVerdict` | mika#2368 | QA build callback, **concluded** cleanly, posting nothing | `qa_callback_verdict` | *absent* |
| `TurnFailed` | mika#2289 | died on a loop error (transport timeout, …) | `qa_deadline_verdict` | `error` |
| `CallbackCutOffWithoutVerdict` | mika#2515 | callback cut off by its deadline **or** its step budget | `qa_callback_verdict` | `cut_off_deadline` / `cut_off_max_steps` |
| `WebhookCutOffByMaxSteps` | mika#2943 | webhook turn cut off by its **step budget** | `qa_deadline_verdict` | `cut_off_max_steps` |

The reason decides the **body**, the **log line** and the **event name**; it
decides nothing else — target resolution, the anti-double-post registry, the 422
classification and the never-return-an-error discipline are shared.

The entry guard that used to read `overrun == None → NotApplicable("turn_completed")`
is gone, and its removal is the shape of the generalisation rather than a
relaxation: "the turn concluded" now describes *exactly* the second reason's
population, so it could not stay a refusal. It moved down into
`deadline_verdict_target`, the webhook call-site's entry decision, where it still
costs nothing on the nominal path (no token resolution, no log, no parse).

**The signal — two fields, one per bound.** `AgentOutput.deadline_exceeded:
Option<DeadlineOverrun>` (carrying `steps_completed`) is stamped in
`persist_deadline_fallback` — the one function all three deadline gates (prelude,
continuation-skip, `LoopResult::DeadlineExceeded`) return through, so the flag
cannot be set on two and forgotten on the third. Since mika#2943 its sibling
`max_steps_exceeded: Option<MaxStepsCutOff>` carries the **step** bound, stamped
only on the "continuation performed" exit of the `LoopResult::MaxStepsExceeded`
arm. The two are mutually exclusive by construction (each exit of `run_loop`
poses at most one), and `handlers.rs` reads them through the single **total**
converter `TurnConclusion::of_output(&AgentOutput)` — never field by field.
Architect Q3 chose a field over propagating `LoopResult`: `handlers.rs` already
consumes this struct, and widening the agent-core/orchestrator interface was too
much surface for a p1.

**Why `text` could not answer.** On deadline the loop persists a canned assistant
message — *"I'm sorry, that took too long"* — and returns it as `text` like any
other response, which `run_agent_for_message` then sends on the reply channel.
**That is the exact mechanism of the mika#2276 symptom:** Telegram notified on each
of four turns, PR silent, because the only path that posts a verdict is
`run_gh pr review` and the LLM never reached it. "The turn finished" and "the turn
concluded" were two different facts nothing in the code separated.

**`hold[review]`, not a new verdict (Q1).** The line already exists in
`qa-review/skill.toml` and `verdict_handler` already understands it (notify
operator, leave the task `in_progress`). A `block[timeout]` would have needed a new
branch in `verdict_handler.rs` — hence the CODEOWNERS gate — for a meaning
`hold[review]` already carries.

**Anti-double-post (AC3), two independent layers.** (1) The existing session-scoped
`pr_reviews_posted` registry, read for both key shapes `run_gh` can write
(`{repo}|{n}` and `__default__|{n}` when the call carried no `--repo`); the net also
registers its own post so a second pass in the same session cannot fire. (2) A POST
answering **422** is read as **idempotent success, never a verdict failure**
(architect Q4) — the in-memory registry does not survive a restart, and 422 is the
net for when it was lost. Classification is narrow on purpose: a bare `422` would be
too wide, and a genuine 403/404 must stay a failure or the outage goes invisible
again.

**Boundaries.** The POST is injected (`poster` closure) so the contract AC2 asks for
— *a verdict IS posted* — is assertable without touching GitHub; production wires
`run_gh_subprocess` with a PAT-first token (`Settings::resolve_github_token`, ADR-008
— posting a review is an operation whose author GitHub reads). Both call-sites use
that same canonical resolver, and deliberately **not** `resolve_periodic_scan_token`,
whose own doc-comment excludes in as many words the paths that require the machine
identity (PR review / merge). The net never returns an error: a net that fails the
webhook would replace a silence with an outage. It does not replace the
conversational fallback, which still goes out on the reply channel.

#### The second reason: a QA build callback that concluded mute (mika#2368)

**What it closes, written in the test mika#2355 itself left behind.**
`the_verdict_guard_fires_once_and_does_not_loop` says it verbatim: after the single
re-prompt, a second EndTurn with no review is **accepted** and `run_gh` was never
called — *"nothing was posted — the net's job"*. That is not a defect of the
`qa_build_callback_verdict` guard, it is its contract: every `intent_guard_retries`
guard has a one-shot budget, deliberately, because a guard that re-prompts for ever
turns a silence into a loop. Budget spent, what remains is an injunction the model
ignored twice and a mute PR — the exact shape
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` predicts.

**The signal travels up; it is not re-derived.** `run_loop` takes an out-param
(`qa_verdict_unmet: Option<&AtomicBool>`) posed on **both** EndTurn exit paths —
non-empty text and the empty-text mirror — under
`qa_build_callback::verdict_unmet_after_retry`, the exact complement of the guard
(same conjunction, budget term inverted). `run_silent_agent` returns it as
`SilentTurnOutcome`; `task_engine::dispatcher::post_callback_verdict_net` reads it
after a callback turn returns `Ok`, **before** the session's registry entry is
evicted. Not a `LoopResult` variant: that enum's exhaustiveness is a contract
forcing three external handlers to treat every *termination mode*, and "concluded
without posting" is not one — a turn can be `Done` **and** mute.

**Two exit sites, not one, and the second is the likelier.** The empty-text mirror's
own comment says why: *"a bare EndTurn is exactly the shape a turn that has nothing
to say takes, and it is the one the registry never sees."* A signal posed only on
the non-empty path would leave the net blind on the more probable half of its
population — and nothing would say so, because a blind net is silent, exactly like a
net with nothing to do. Hence one positive control **per site**
(`tests/eval/test_qa_callback_verdict_net_2368.rs`, T10a/T10b), verified red when
either site alone is unwired.

**The PR target is said, never derived.** `skills::executor::execute_long_running`
resolves it at spawn from `LongRunningContext.originating_message` through
`deadline_verdict::parse_pr_target` — the single reader of that grammar — and stamps
it on the callback row under `QA_REVIEW_PR_TARGET_KEY`, the same trajectory as
`metadata.dispatch_worktree_file` (mika#2249) and `metadata.pilot_transcript_expected`
(mika#2040). What is condemned is the **late** derivation: the one that would happen
at net time, when the failure is no longer recoverable and logs nowhere. Here the
resolution happens at spawn, its failure is logged on the spot
(`qa_review_pr_target_unresolved`), and **the net parses nothing** — it reads a
stamp. Fail-safe: no stamp, unreadable metadata, missing key, unparsable target →
zero POST and a line naming the abstention (`no_metadata`, `metadata_unreadable`,
`no_target_stamp`, `target_unreadable`). Four separate negative controls, because a
conjunction of fail-safe terms is not proven by neutralising all of them at once —
the mika#2277 lesson.

**The registry now reaches the silent path (AC7).** `agent_loop` posed
`pr_reviews_posted: None` with the comment *"Silent mode: no session-scoped dedup
needed"* while `skills::builtin_handlers` carried
`debug_assert!(ctx.pr_reviews_posted.is_some(), "…must be threaded for production pr
review calls")`. A QA callback that posts its review **is** a production pr review
call: the two statements had contradicted each other since the build callback became
a flow that posts reviews. AC7 corrects an inconsistency the source already
declared. The callback's session is fresh, so the registry carries exactly what that
turn posted.

**`hold[review]` and nothing else — a safety constraint, not a registry choice.**
`verdict_handler` routes `pass` to a **merge**. A net posting `pass` because the
build went green would merge a PR **no diff was ever reviewed on** — strictly worse
than the silence it replaces. Asserted on `DEADLINE_VERDICT_LINE` *and* by feeding
the produced body to `server::verdict::parse_verdict`: the constant alone does not
prove what the state machine will read.

**Kill-switch:** `MIKA_QA_CALLBACK_VERDICT_NET` (default armed; `0` disarms with no
redeploy). Not caution on principle — mika#2355's probe 2c prescribes *"disarm the
net before any diagnosis"* if a PR is ever merged unreviewed, and that prescription
is only executable if the lever exists. An unrecognised value is **said** and leaves
it armed: a disarm by typo on a safety net would be the silent failure this whole
ticket closes.

**Operator grep signals — two names, and that is what saves the probes.**
`qa_deadline_verdict` (reason `CutOffByDeadline`) and `qa_callback_verdict` (reason
`CallbackConcludedWithoutVerdict`), each **SOLE WRITER** of its own name in the log
and in `audit_events`, pinned by a source scan
(`mika2368_each_event_name_has_exactly_one_writer_in_production`) — a behavioural
test cannot see that class, since a second writer would make no decision wrong, only
the two populations inseparable. mika#2355's negative-control probe is literally
`grep qa_deadline_verdict … | jq 'select(.outcome == "posted")'` — *the mika#2276 net
must not start firing*; a shared name would merge two populations and make a `posted`
of the new reason read as a regression of the old. Same principle as
`phantom_aged_out` / `phantom_sweep_spared` (mika#2156) and `auto_pull_no_token` /
`wip_rescue_no_token` (mika#2205). Both carry an `outcome` field in
`{posted, already_reviewed, already_posted_upstream, post_failed, no_token, no_registry}`,
plus, for the callback reason, `disarmed` and the four abstention motives above.
For `qa_callback_verdict`, **near zero is the contract**: a net carrying nominal
traffic has replaced a silence with a systematic `hold[review]`, which means
mika#2355's B1/B2/guard did not take — and *that* is where to look, not in the
net's tuning. *This is a net, not a path*; if it carries the nominal traffic it
has also erased the signal that would show it.

**Two names are no longer two populations — `cause` is what subtracts them, and
reading the name alone now lies.** `qa_deadline_verdict` carried exactly one
reason when mika#2355 wrote its negative-control probe (*"the mika#2276 net must
not start firing"*, steady state zero lines); it carries three since mika#2943.
A `posted` under that name is therefore **not**, by itself, a regression of
mika#2276. The founding population is isolated by the **absence** of the field:

```sh
# mika#2276's founding population — the probe mika#2355 wrote, corrected.
grep qa_deadline_verdict /var/log/mika/server.log \
  | jq 'select(.cause == null) | select(.outcome == "posted")'   # expected: empty

# mika#2943's population — a webhook turn cut off at its step ceiling.
grep qa_deadline_verdict /var/log/mika/server.log \
  | jq 'select(.cause == "cut_off_max_steps")'
```

`cut_off_max_steps` is **one constant**
(`qa_build_callback::CAUSE_CUT_OFF_MAX_STEPS`), shared with the callback path
rather than retyped: one quantity, one name. The two paths stay separable
because they keep **different event names** while sharing the cause — so
`cause` alone answers *"which bound?"* and the name alone answers *"which
path?"*, and neither has to answer both.

#### The fifth reason: a webhook turn cut off by its step budget (mika#2943)

**The half of the net that was missing, and the reason nothing noticed.** The
mika#2276 net arms on `DeadlineExceeded` or `Failed`. A turn that exhausts its
**tool-step** budget arrives as `Concluded`, because the loop runs a continuation
turn and that turn produces text: `text` reads exactly like an answer. Six
webhook turns of mika-qa between 2026-10-26 and 2026-10-28 ended that way, each
leaving its PR unreviewed and **nothing on the PR saying so** — including
`b22e6ea0` on #2920 at the 30-step ceiling mika#2926 had just raised, so the
raise was not the fix. The callback path already had this net (mika#2515); the
webhook path did not.

**The partial converter is gone, and that is the fix's shape.**
`impl From<Option<DeadlineOverrun>> for TurnConclusion` saw one field, so a call
site could read the deadline alone **and compile** — which is precisely how this
defect shipped. It is replaced by `TurnConclusion::of_output(&AgentOutput)`,
total on both flags. Keeping the partial one beside it would have left the trap
armed for the next field. The constructively-impossible crossing (both `Some`)
is **decided** in favour of the deadline rather than `unreachable!()`: a future
mis-wired exit should cost a less precise verdict body, never a server crash.
Held structurally — `grep -rn 'TurnConclusion::from' crates/` returns nothing.

**One body, no second grammar (AC1).** The posted body is mika#2276's, with one
sentence changed, because the deadline body is frozen byte-for-byte by
mika#2368 AC5c: *"Le tour de revue QA a atteint la limite de son budget de steps
d'outil…"*. It says *budget de steps*, never *enveloppe de temps* — trace
`b22e6ea0` still had ~295 s of its envelope left, and reusing `CutOffByDeadline`
would have posted that falsehood. The step count is the **served** ceiling,
transported by `LoopResult::MaxStepsExceeded`, never re-read from a constant: a
mika-qa turn that consumed 30 must not be reported as 20.

**A build callback in flight is the one abstention (AC3).** The measured cases
`11699630` (#2920) and `517380e0` (#2932) were cut off at the ceiling **after**
dispatching a `long_running:build_mika` — and their review came back through the
callback, hence through *its* net. So when
`qa_build_callback::build_callback_dispatched_in_turn` holds, `run_loop` poses no
cut-off fact at all and says so once
(`event = "qa_max_steps_verdict_withheld"`, `cause = "build_callback_pending"`).
Posting here would double the `hold`. The abstention is **journalised** rather
than silent, because an abstention and a blind net return identical bytes.

**Probe (AC4).** At the next `max_steps_tool_census` of a mika-qa webhook turn on
a PR, the same `trace_id` carries either a posted `qa_deadline_verdict` with
`cause=cut_off_max_steps`, or a review the session posted itself, or a
`qa_max_steps_verdict_withheld`. Those three exhaust the dispositions; a fourth
outcome — the census line alone — is the defect still live. The instrument is not
live until `make deploy` (motif mika#2340).

### Structural Verdict Handler

`server::verdict_handler` — intercepts `pull_request_review.submitted` webhook events **before** the LLM turn in `handle_message`. Parses `VERDICT:` line from the review body (authoritative regardless of GH `review.state`). Full dispatch table: `pass` (state=approved only) → merge via `run_gh_checks` + `run_gh_merge`; `block[ac]` → dispatch claude-pilot with AC-fix prompt, bounded retry counter (max 3), escalate on limit; `block[ci]` → dispatch claude-pilot with CI-fix prompt, bounded retry counter (max 3), escalate on limit; `block[security]`/`block[pipeline]` → mark task blocked, notify operator, NO auto-dispatch; `hold[review]` → notify operator, leave task in_progress; missing/unparseable → safe-default hold[review] semantics + `verdict_classification_failed` structured log event. AC extraction from qa-review's `[❌] unsatisfied:` lines with 2000-char fallback. Pre-digests for all verdict classes avoid completion-claim guard trigger words. Shared helpers: `find_task_for_verdict` (task lookup + in_progress gate), `has_active_callback_child` (in-flight guard), `send_notification`, `truncate_body` (UTF-8 safe). Parser in `server::verdict` depends on gateway's `format_event_text()` output format. 60s timeout on subprocess calls. See #524, #889.

### The dispatcher identity never reviews (mika#2667)

The role separation at the merge gate (mika#2218, mika#2248) was held by the
prompt on the **write** side and checked nowhere on the **read** side. Three
gates now hold it, each with its own surface:

| side | site | rule | surface |
|---|---|---|---|
| write | `run_gh` → `validate_dispatcher_not_reviewer` (predicate `evidence::guards::detect_review_verdict_act`) | the dispatcher agent cannot post `gh pr review --approve`/`-a`, a body carrying a `VERDICT:` line (read by `verdict::verdict_raw_value`), or a body from `--body-file`/`-F` | `dispatcher_review_refused` (WARN + audit, motif in `after_value`) |
| read | `verdict_handler`, `pass` arm, before `handle_pass_verdict` | the merge path is reached only when the review author satisfies `forge_identity::is_reviewer_forge_login` | `verdict_pass_from_non_reviewer` (WARN + audit) |
| read | `ci_success_handler::select_pass_verdict` | a `pass` authored by any other login is never selected, so no merge-ready signal is emitted from it | — (the PR simply reads as having no pass verdict) |

**One reader of the reviewer login**, `mika_common::forge_identity::is_reviewer_forge_login`,
used by both halves of the merge path — never a copied literal.

**A `hold[review]` with no active task creates no work (AC2).** The passthrough
opens with `verdict_handler::HOLD_REVIEW_NO_TASK_MARKER`, notifies the operator
and writes `verdict_hold_review_no_task`. `webhook_dispatch::is_hold_review_without_task_turn`
is the single reader of that marker; `effective_disabled_tools` withholds
`create_task` on that turn and `validate_dispatch_readiness` refuses every
long-running dispatch with `hold_review_without_task_no_dispatch`. The message
starts with `[verdict_handler]`, so it stays outside the Webhook Fallthrough
domain and none of the mika#2517 consumers move.

**Test fixtures use the real reviewer login.** An approved `pass` in a test must
be authored by `mika-platform-qa`; `mika-qa` is an agent name, never a GitHub
login, and is refused by the read side like any other author.

### Structural CI Success Handler

`server::ci_success_handler` — intercepts `check_suite.completed(success)` webhook events **before** the LLM turn. Companion to `verdict_handler`: re-evaluates merge eligibility for PRs that have a pending `VERDICT: pass` but were blocked on CI at approval time. Queries GitHub API for open PR, QA pass review, stale-SHA gate (`review.commit_id == pr.head.sha`), and CI aggregation via `run_gh_checks` + `classify_checks`. Reuses `VerdictAction` return type and `pr_merge_with_gate` helpers. Order-independent with `verdict_handler` — each handler self-selects on event type. 60s timeout on subprocess calls. See #571.

**It evaluates; it does not merge (mika#2248).** On every gate cleared it emits a `MergeReadySignal` and returns; the merge belongs to `server::merge_ready_handler` (next section). The reason is the routing table, not style: `check_suite.completed(success)` routes to `mika-dev` **and** fans out to `mika-qa` (`secondary_targets`, mika#1711), both run this handler, and a `gh pr merge` issued here runs under the token of whichever agent won the race. Measured 2026-09-08 on mika#2244 — `mergedBy = mika-platform-qa`, the reviewer closing its own approval, with no handoff to the dispatcher that owns the cycle. The absence of a `run_gh_merge` callsite in this file is asserted by `tests/eval/test_ci_success_handler.rs`, and `tests/eval/test_merge_identity_2248.rs` asserts the gates still precede the signal. **Operator grep signal:** `ci_success_merge_ready` (INFO + `audit_events` row keyed `pr:{repo}#{n}@{head_sha}`). Note the prompt layer was already right here and could not help: `qa-review-webhook-success` tells mika-qa "never … merge tools", and mika-qa's allowlist carries neither `self-dev-webhook-ci` nor `self-dev-webhook-qa` — this handler runs before the LLM turn and is unaffected by either.

**Entry gate — only the dispatcher evaluates (mika#2260).** `try_handle_ci_success` refuses in its own head when `owns_merge_transition(db.agent_id())` is false: `Passthrough { enrichment: None }`, one `ci_success_handler_skipped_not_merge_actor` audit row, and nothing else. The placement is the deliverable — after the event-type selection, **before** the `evaluating` log line, the token requirement, the first `gh` call, the dedup write and every existing audit row.

**What it closes, and why the mika#2248 belt was not enough.** #2248 put identity at the **exit** (who merges) and left it absent from the **entry** (who this evaluation exists for). So the reviewer walked the whole evaluator — up to emitting the signal — and was only held at the actor. Measured 2026-09-09/10 on `audit_events`: mika-qa traversed the evaluator **81 times**, held the DECISION-CORE gate **6 times** (six duplicate operator notifications), emitted the signal once on mika#2259, then was stopped. A race resolved by a **late** guard depends on order; this one removes the race.

**The consequence the ticket feared without naming it, and it is a loop-stop class.** The in-memory dedup key one hop below is **process-global** and keyed `{repo}:{branch}:{head_sha}` with **no `agent_id`** (`check_suite_dedup::DEDUP_MAP`), so the reviewer entering first *consumed the dispatcher's slot*. On #2259, head `047e9eea`: mika-qa evaluated at 09:08:11 and registered the key; mika-dev's deferred replay at 09:08:26 came back `ci_success_dedup.skip`. Harmless there (checks still `pending`) — but at 09:12:36 every gate was green, and the dispatcher only evaluated because its copy landed **60.005 s** after the reviewer's registration against a 60 s window: **5 ms**. A deferral at 59.9 s instead, and mika-dev skips, no signal is emitted under an identity that can act, no further `check_suite` arrives on that head (all workflows are done), and **the PR stays green and open with nothing to close it.**

**Four other side effects the reviewer exercised before the belt:** five `gh` calls per event under its PAT (`find_open_pr`, `find_pass_verdict`, `run_gh_checks`, `run_gh_pr_view`, `is_behind_main`); a `PUT /repos/{repo}/pulls/{n}/update-branch` **write** on the forge at step 5c if the PR was behind main — posted by the agent that must never act on that PR; the duplicate DECISION-CORE operator notification; and the evaluator's pre-digest **replacing** `req.text` in mika-qa's turn, so the reviewer received "Merge-ready signal emitted…" in place of the raw event `qa-review-webhook-success` is written for.

**`Passthrough { enrichment: None }`, never `Handled`** — the reviewer's turn must see the **raw event**, which is that last effect. And no enrichment either: the reviewer's prompt already knows it does not merge, and telling it again only adds a second text to follow.

**One whitelist, two names, two questions.** `owns_merge_transition` is `merge_disposition(agent) == Act` and lives next to it in `mika_common::forge_identity`; adding an actor stays one explicit gesture there and the entry gate inherits it with no second table. `merge_disposition` answers *who merges* (an actor decision); this one answers *does my evaluation have a consumer* (an entry decision). `forge_identity`'s layer table now carries four rows, not three.

**The mika#2248 belt is untouched.** `merge_ready_handler::authorize_merge` (`WouldMergeAsReviewer` then `NotDispatcher`) and the `pr_merge_with_gate_reviewer_refused` tool refusal are unchanged. After this gate the reviewer's hold is unreachable in production; it remains the defence if a signal ever arrived by a path this evaluator did not post — which the module already says.

**Three refusals this ticket declined**, each with its own reason: putting the gate in `handlers.rs` around the evaluator+actor pair (it would keep the evaluator literally "identity-free", but `handle_message`'s body is not unit-testable, so the gate would only be pinnable by a source scan — here the per-agent self-selection sits where the per-type self-selection already does, and T1 exercises it without network); removing mika-qa from the gateway fan-out (the reviewer needs the raw event to start the review — mika#1711, 14 h of dead QA window before it; the gate is agent-side, not routing-side); and adding `agent_id` to the dedup key (once the gate is posted a single agent reaches the dedup, so a multi-agent key would only re-authorise N evaluations if a second actor were ever whitelisted — which the whitelist makes deliberately costly. **T3, the gate-before-dedup scan, is the protection — not the key.**)

**Operator grep signal:** `ci_success_handler_skipped_not_merge_actor` (INFO + an `audit_events` row of the same name — **SOLE WRITER**, pinned by `canonical_tokens::tests::mika2260_the_entry_gate_skip_name_has_a_single_writer` with an allowlist shipped empty, because AC6's measurement is only exact while one site writes it). `target_key = event:{repo}@{branch}` — the PR number is **not** known at that point and must not be: knowing it costs the first `gh` call, i.e. the side effect the gate removes; consequence worth knowing, the key carries no `head_sha`, so repeated refusals across distinct pushes on one branch are separable by `created_at` / `trace_id` rather than by `target_key`. Expected regime: **one line per non-dispatcher agent per event** — this is a counter, not an anomaly, and it is what AC6 measures. No dedup on it, deliberately.

**The halt that matters, and it is the inverse of the obvious one.** Because the expected regime is non-zero, the failure mode *"the dispatcher itself is being turned away, so nothing merges any more and nothing errors"* lands **inside** the nominal counter. The shape that separates them is a **conjunction**: a non-zero skip count together with **zero** `ci_success_merge_ready` over a window in which PRs did go green means the loop has stopped merging. Read the skip rows' `agent_id` (in `reasoning`) before anything else: `mika-qa` is nominal, `mika-dev` is the halt. **Do not widen the whitelist reflexively** — a dispatcher being refused means `DISPATCHER_AGENT` no longer names the agent the gateway routes `check_suite` to, and the remedy is the routing or the provisioning, not the predicate. Note the actor-side row this displaces, `merge_ready_hold_not_dispatcher`, had an expected regime of **zero** and was therefore loud on its own; that loudness is what the entry gate trades away for the removed side effects, so the conjunction above is what replaces it.

**Guards.** `tests/eval/test_ci_success_handler.rs`: T1 drives the three agents (`mika-qa`, `mika`, `mika-dev`) through one test — the positive control in the same breath, because a handler always returning `Passthrough { None }` would satisfy both negatives and be indistinguishable from the fix; T3 is a source scan pinning the gate's index strictly below `match github_token {`, `find_open_pr(`, `try_dedup_check_suite(` and `count_recent_audit_events_for_target(`, and it carries its **anti-vacuity** assertion (it fails when the symbol is absent from the body, because a scan aiming at a dead name reads exactly like a clean tree — class mika#2103/#2205). That scan is not decorative: rewriting the predicate inline as `merge_disposition(agent) != Act` leaves T1 and T2 **green** and only T3 red — verified by mutation. `tests/eval/test_ci_success_fanout_2260.rs`: T2 measures the attribution on `MultiAgentHarness` (mika#2265) in **both** patterns its doc-comment prescribes — deterministic order and `tokio::join!` race, *"#2248 is an order and #2260 is a race"* — with `cross_read_count` as the positive control of the mount itself, since two disjoint in-memory databases would make every assertion green while measuring nothing. **One rectification the implementation forced on the plan:** `audit_counts_by_agent` always returns one entry per mounted agent, zeros included, so the discrimination reads on the **values**, never on the map's cardinality; and with `token = None` nobody reaches `ci_success_handler_processed`, so that count is asserted as a non-regression control and explicitly **not** as a discriminant.

**Burst dedup (mika#1869):** a single push fans out to up to 8 workflows, each firing its own `check_suite.completed(success)` webhook → N redundant walks of the full handler path that saturated mika-qa's mailbox (41 `rate_limit_trip`/h baseline). Two `head_sha`-keyed layers, placed right after `find_open_pr` resolves `head_sha`, collapse the burst to one merge evaluation: (1) an in-memory precise gate (`server::check_suite_dedup::try_dedup_check_suite`, bounded 1000-entry DashMap, 10-min TTL, 60s window) and (2) an audit-durable gate (`ci_success_handler_processed` marker rows queried via `count_recent_audit_events_for_target`, keyed `pr:{repo}#{n}@{head_sha}`) that survives a process restart the in-memory map cannot. Both key on `(repo, branch, head_sha)`, so genuine distinct pushes (new `head_sha`) are never conflated; the audit read fails open. **Operator grep signals:** `ci_success_dedup.skip` (in-memory gate fired) and `ci_success_handler.dedup_skip` (audit gate fired) in `$MIKA_SPIRIT_LOG_FILE`.

### Merge Actor (mika#2248)

`server::merge_ready_handler` — consumes the `MergeReadySignal` emitted by `ci_success_handler` and performs the merge. Wired in `handlers.rs` **immediately after** that handler: the signal travels in `req.text` (the evaluator's pre-digest), so a handler inserted between the two would rewrite the text and carry the signal away — the ordering is asserted in `tests/eval/test_merge_identity_2248.rs`.

Four conditions, all necessary, in order: (1) a re-parseable signal (`mika_common::forge_identity::parse_merge_ready_signal`); (2) the running agent is not the reviewer named **in the signal** (`would_merge_as_reviewer` — AC1 enforced at runtime, not only in a routing table); (3) the running agent is the dispatcher (`merge_disposition`, an **allowlist** — an unknown agent holds, it does not merge); (4) the perimeter re-verified here, fail-closed and bounded at 60s, so a signal arriving by any other path still cannot close a DECISION-CORE PR. The merge then runs under the running agent's own token, which is what makes `mergedBy` deterministic.

The policy itself lives in `mika_common::forge_identity` — shared with `mika-gateway`, whose `QA_REVIEWER_LOGIN` is the same constant, because the login that decides who may *review* is the login that must not *merge*.

**Entry gate (mika#2260) — this belt is now the second line, not the first.** Since the evaluator refuses in its own head for any agent outside the merge whitelist (§ *Structural CI Success Handler* above), conditions (2) and (3) here are **unreachable in production**: no non-dispatcher gets far enough to emit a signal. They are kept because they guard a different population — a signal arriving by a path the evaluator did not post — and because the mika#2248 tests exercise them by calling the actor directly, which the gate does not touch. Read the two together as *the gate removes the race, the belt survives it*: a `merge_ready_hold_reviewer_is_not_merge_actor` row appearing after mika#2260 is deployed is no longer a nominal race outcome but a signal from an unaccounted-for producer, and worth reading as such.

**Why the signal is not a `merge-ready` label.** Two measured reasons. Permission: a label write under the resolved PAT fails (`Resource not accessible by personal access token (addLabelsToLabelable)`, 29 refusals measured 2026-09-07, mika#2228), so the signal would need a second token and therefore a second failure mode on the critical path. Authority: a label is human-writable, which would turn an interface gesture into a merge authorization able to bypass the DECISION-CORE gate.

**Operator grep signals** (`$MIKA_SPIRIT_LOG_FILE`, each with an `audit_events` row of the same name): `merge_ready_hold_reviewer_is_not_merge_actor` (the AC1 refusal — the shape mika#2244 measured), `merge_ready_hold_not_dispatcher` (any other agent holding), `merge_ready_human_gate_required` (perimeter hold at the actor), `merge_ready_merge_initiated` (the merge fired; the `ci_success_merge` audit row moved here from the evaluator). The tool-side companion refusal is `pr_merge_with_gate_reviewer_refused` — `pr_merge_with_gate` blocks the reviewer at step 0, before input validation and before any `gh` call, and returns `blocked` / `reason.reason = "reviewer_cannot_merge"` (the eighth `BlockReason`, carried in the three `self-dev*` prompts' taxonomy).

### QA-Review Reconciler (mika#2334)

`qa_review_reconcile` — the fourth recurring scan, next to `auto_pull_groomed`
and `wip_rescue` in `task_engine/dispatcher.rs`, carried by mika-dev, cron
`0 */15 * * * *`. It asks `mika-platform-qa` for a review on the loop's open PRs
that nothing came to review. **Outside the LLM, outside the pilot session,
triggered by time rather than by an event** — which is the whole point.

**Two measurements moved the diagnosis, and they are worth keeping.** The
founding ticket read two reviewerless PRs (2026-09-15) as a trailing
`gh pr edit --add-reviewer` the pilot died before reaching. (M1) **That step did
not exist anywhere in the repo** — the exhaustive search returns one hit and it
is a flag-arity table; `dispatch-lib.sh`'s eight `gh pr edit` carry only
`--add-label` / `--title` / `--body`. (M2) **The review never depended on the
pilot**: `mika-gateway/src/github.rs` routes `pull_request.opened` to mika-qa
with no draft filter, so creating the PR starts the cascade. The real defect is
that `opened` is a **single, non-replayable event** — droppable head-of-line by
the mika#1870 queue at saturation, then DLQ and `dead` behind the gateway
circuit breaker, then lost to a second empty turn that `webhook_zero_tools` only
opposes once — and **no path re-read an open PR without a review**. `auto_pull`
works on issues, `wip_rescue` only on `wip-rescue`-labelled drafts,
`curator_review` on skills; the mika#1711 `check_suite.completed(success)`
fan-out is the one existing catch-up and it requires `draft: false` **and** a
green CI.

**Why the letter of the fix was declined.** An *unconditional* reviewer posted at
PR creation double-reviews **every** PR of the loop: `opened` starts a session,
the gesture emits `review_requested` on exactly `REVIEWER_FORGE_LOGIN` — so
mika#1655's `is_suppressed_review_request` lets it through — and the second
session cannot see the first (`gh pr view` is outside `QA_REVIEW_GH_ALLOWED`, and
the first review is not posted yet anyway). The duplicate would be nominal, not
exceptional — the class #886 closed once already. Hence: the gesture must be
**conditional on the absence of a review**, and so it cannot live in
`dispatch-lib.sh`, whose tail delivers its callback and dies in seconds and
cannot observe an absence only measurable after a delay. The anchor there is real
(`_post_flight_recovery`, `PR_URL` resolved, `_stamp_pr_origin` already called)
and is declined for that reason alone.

**Shape.** `select_prs_needing_review(&[PrSnapshot], now, &ReconcileConfig)` is a
pure function carrying the entire decision; the `gh` execution is a thin caller.
Six conjunctive terms, each fail-safe — unreadable information takes a PR **out**
of the population, never into it (deleted author, unparseable `createdAt`, a
future timestamp clamped to age 0). **The pilot's termination is not a parameter
of that function**, which is the structural form of "independent of the pilot's
survival". Deserialization is part of the guard: `review_requests` and `reviews`
deliberately carry **no** `#[serde(default)]`, because a defaulted absence would
read as "no request, no review" and *admit* a PR on missing information.

**Sole writer** of the `qa_review_reconciled` audit `tool_name`, which is what
makes `SELECT … WHERE tool_name = 'qa_review_reconciled'` the exact list of PRs
the loop had to catch up on — and therefore the measure of the nominal path's
health. This is a **net, not a path**: if it carries nominal traffic, the upstream
loss is the subject and the net is now hiding the signal that would show it.

Token via `Settings::resolve_github_token` (PAT-first, App fallback — posting a
reviewer is not an operation whose author GitHub reads in the ADR-008 sense), and
the mika#2205 structural guard covers this fourth scan. `PeriodicScan` gained a
variant, so `mika2334_every_scan_variant_is_covered` makes the next one fail to
compile until the guards enumerate it. Config, kill-switch and the operator
grep signals: root `CLAUDE.md` § *Optional (QA-review reconciliation — mika#2334)*.

#### The ledger decides (mika#2347)

**What mika#2334 left open, and it is readable in the code alone.** Its only
idempotence terms were *no request* and *no review* for `REVIEWER_FORGE_LOGIN` —
two **GitHub states that exist only once the review has concluded**. A PR whose
review turn dies without posting anything therefore fell back into the population
on the next tick, identical to itself: with the `0 */15 * * * *` cron and the
then-default cap of 3, up to **96 re-requests per day per PR** across the seven
days of `MAX_AGE`. The `qa_review_reconciled` audit row was written and **never
read back** — the ledger existed and decided nothing.

**The key carries the head SHA.** `PrSnapshot` gained `head_ref_oid` (requested in
`--json`, **no `#[serde(default)]`**, empty value treated as unreadable and takes
the PR *out* of the population), and the audit key became
`pr:{repo}#{n}@{sha}` — the exact form `ci_success_handler` already uses for its
durable dedup (mika#1869). The operator query is unchanged and the `pr:{repo}#{n}`
prefix is preserved. **A new SHA reopens the budget by itself**: the key changes,
the count restarts — the "per (PR, SHA)" idempotence obtained from the shape of
the key rather than from one more column.

**Two bounds, read through `count_recent_audit_events_for_target` — no new DB
method, no migration.** `review_ledger_verdict` answers `Postable` / `Cooldown` /
`Abandoned{attempts}` / `Unreadable`: a pose inside `COOLDOWN_SECS` skips the PR
for that tick; `MAX_ATTEMPTS` poses inside the `MAX_AGE` window abandon that SHA
for good. **The budget is what separates this fix from a slowdown** — a cooldown
alone replays for ever, just less often. Direct precedent, cited in the code:
`MIKA_AUTO_PULL_MAX_REDRIVES` (mika#2020), born of mika#1901 receiving the `ready`
label sixteen times in nineteen hours. Legacy rows (`pr:{repo}#{n}`, no SHA) are
consulted by a **second exact query**, never a `LIKE` — which would match `#234`
against `#2343`; that query is dated in the code and can go once every pre-fix row
is older than `MAX_AGE`.

**Fail-closed, the inverse of `ci_success_handler`.** An unreadable
`audit_events` refuses the pose (`qa_review_reconcile_ledger_unreadable`, WARN,
**must stay empty**). Same choice as `wip_rescue` (mika#2199) and for the reason
written there: a false negative makes a PR wait, which is what it did before
mika#2334 anyway; a false positive replays a review, i.e. produces the very defect
this closes.

**The truncation moved, and that is a contract change.** `select_prs_needing_review`
no longer truncates to `max_per_tick`; the caller truncates **after** the cooldown
filter. Left upstream, `max_per_tick` PRs in cooldown would consume the whole tick
and a fourth, legitimately recoverable one would never be seen — a cap on *skips*
instead of a cap on *writes*. The tick budget is debited on **attempted poses**
only; the inverse rule already written (a *failed* pose does debit) is unchanged.
`MAX_PER_TICK` default drops **3 → 1**: 4 requests/hour against a measured
capacity of 6 reviews/hour (600 s envelope, serialized execution), the one setting
here whose effect is arithmetically demonstrable.

**Serialization (AC4) is an engine invariant, pinned rather than implemented.**
The scan triggers no turn: it posts a reviewer, and the resulting
`review_requested` falls into the mika#1870 bounded queue, drained by **one worker
per agent** taking `agent_lock`. Writing a concurrency lock inside
`qa_review_reconcile` would be a placebo — the scan holds no turn.
`test_qa_review_reconcile_2347.rs` pins it two ways: behaviourally (two
`review_requested` never coalesce and drain in order) and structurally (one
`spawn_webhook_drain_worker` definition, exactly two spawn sites, and
`run_agent_for_message` *receiving* the guard rather than taking it). A behavioural
test alone cannot see that class: a second consumer would make no decision wrong,
it would lift the invariant in silence.

**What this fix does NOT prove.** The ticket's hourly evidence is not compatible
with the reconciler as the sole source of the churn — two `hold[review]` eleven
minutes apart cannot come from a fifteen-minute scan, and the first `hold` **is a
posted review**, which takes the PR out of the population on the next tick.
`pull_request.synchronize`, the `check_suite` fan-out and a queue replay remain in
play and are out of scope. This reduces the number of reviews *requested*; it makes
no turn faster.

**What kept it inert for a day, and where that is fixed: mika#2337.** The scan
shipped complete — the registration literal and the match arm are two literals of
the same commit — and never ran. Its first fire failed on
`unknown run_skill trigger: qa_review_reconcile`, which only a binary *older* than
that commit can emit, and the death did the rest. See § *Unknown-Trigger Veto Lift*
below; the root cause (merged code ≠ running code) is not in this repo and is
tracked as follow-up.

### Hot STOP for `auto_pull` (mika#2329)

`crates/mika-agent/src/auto_pull_stop.rs` — **sole reader** of the sentinel file
`~/.mika/state/auto-pull-stop`. Its existence stops the feeder at the next tick
(≤ 10 min, `AUTO_PULL_CRON`); removing it resumes it. The content is never read.

**Why a file and not the env var the ticket proposed first.** `mika-spirit` calls
`load_dotenv` **once** at startup and nothing watches the file afterwards; a live
Linux process's environment is not mutable from outside, so **editing `~/.mika/.env`
changes nothing about what `std::env::var` returns**, even called every tick.
Re-reading the variable at tick time would have moved the defect one notch and made
it *harder* to see, since the code would then look like it re-reads. And the house
deliberately freezes its `MIKA_*` variables (four are documented "not
hot-swappable"): making one of them hot would create an invisible exception between
two variables nothing distinguishes — notably `MIKA_DEV_WIP_RESCUE`, the twin form
that does not re-read.

**The row is never touched, and that is the design.** The short-circuit sits at the
**head** of `dispatch_auto_pull_groomed`, before `resolve_periodic_scan_token` (two
token resolutions, one potentially a GitHub App exchange over the network) and
before the two `gh` fetches — same placement reasoning as mika#2279's gate 2c. The
recurring row keeps ticking; only its dispatch returns early. **Reversibility is
therefore not machinery but the absence of machinery**: no contact with the
mika#1742 anti-zombie guard, the mika#2271 config-cancel exemption, or
`RECURRING_ZOMBIE_GRACE_HOURS`. The boot-time knob, by contrast, *cancels* the row —
which is precisely what cost mika#2271 and its still-present repair machinery.

**The WARN is what actually closes the incident.** On the non-short-circuited path
(the predicate's population — if the process had booted with the knob, the row would
be cancelled and no tick would run at all), `stale_env_knob` reads the `.env`
**files** — per-agent then global, the mika#2218 order — and emits
`auto_pull_stop_stale_env_knob` when one carries `MIKA_DEV_AUTO_PULL=0`. Without it
the defect stays open in its most dangerous form: **a silently inoperative STOP reads
exactly like a STOP that works.** Inverse mirror of mika#2205.

**SOLE WRITER** of `tool_name = 'auto_pull_stop'`, one row per **transition**
(`armed` / `lifted`) — never per tick (mika#2131 doctrine). The INFO line, by
contrast, fires on every short-circuited tick: for a switch, liveness *is* the
information (Signal P precedent, mika#2156). Transition state is an `AtomicBool` on
the dispatcher and is **lost on restart, deliberately** — a fresh process
re-photographs what it finds.

**Fail-open, named:** `Path::exists()` returns `false` on any access error. Fail-closed
is not cleanly implementable (`exists()` cannot separate "absent" from "unreadable",
and `symlink_metadata()` would make the nominal "no `state/` dir" case a permanent
STOP). What makes that acceptable is the **visibility**, not the reasoning: the
operator sees the effect in ≤ 10 min. If the reader ever becomes fallible in a way
the operator cannot observe (DB, network), redo the trade-off rather than transport it.

**`AUTO_PULL_SCAN` is the loop's NEW-DISPATCH BRAKE, not a scan's private switch
(mika#2498).** Its name says a scan; its scope is wider, and reading the name as
the scope cost an incident. On 2026-09-23 the sentinel posted at 05:38 did
short-circuit the feeder tick — and an implement fired anyway at 06:11:39Z
(`ff694ec0`, 103 turns, 17.86 USD), because a converged groom chains straight
into its implementation through the **engine auto-fire**
(`try_dispatch_pilot_after_groom_success`, mika#1614), which goes through no
tick. The operator could only cancel it in reaction.

**Widened in its documented sense, never duplicated.** mika#2420's criterion is
*a distinct decision deserves a distinct file* — but the two routes have the
**same output** (a fresh dev-pilot dispatch) reached by two paths: the feeder
promotes `ready` then dispatches in-process (mika#2470), the auto-fire dispatches
directly. An operator who stops one and not the other has stopped nothing, which
is exactly the incident. A second sentinel (`groom-implement-stop`) was refused
for that reason: it splits one operator decision into two gestures, and the
gesture of muscle memory — posting the file of the incident — would then yield
today's behaviour while looking like a stop.

**The criterion for a future consumer**, and it is not "am I a scan?": *does it
start **new** pilot work?* If yes, read this sentinel, whatever the entry door.
If no, it needs its own scan name (mika#2420's criterion). That is what
deliberately leaves out `verdict_handler` (`block[ac]` / `block[ci]` repair an
open PR — a **continuation**, not new work) and a `ready` posted by hand during a
STOP (whether a file outranks the gesture an operator just made is a product
decision, not substrate).

**The refusal is convergent, never terminal**, which is what makes the widening
acceptable: it does not cancel the groom, does not lose the plan (committed and
pushed by `_push_branch`), creates no row, and leaves the parent `in_progress` /
`groom` — one more refusal in a groove the function's existing skip paths (no
token, tool absent from the registry, handler not long-running, flip impossible,
readiness refused, callback creation failed, handler script missing) had already
dug, which is what keeps the change small. On release,
the stuck-ready reconciler re-drives the ticket; and since no tick runs during
the STOP, **no point of the re-drive budget is spent** (mika#2020). Useful side
effect, and it answers the "groom-only" half of the ticket without introducing
any new state: under STOP the in-flight grooms converge, commit their plan, and
do not chain.

**Placement is the whole of the surface's meaning.** The guard sits after step 3
(the predicate establishing that a dispatch *will* happen — groom class,
`Outcome: PLAN_GROOMED`, parent with a parseable issue URL) and **before step 5c**
(the parent's `dispatch_class` flip groom→implement). At the head of the
function its line would have meant "a callback arrived during a STOP", a fact
with no associated conduct, dozens of times a day; after 5c it would leave a
parent labelled `implement` with no implement in flight and the next legitimate
dispatch counted on the wrong slot (#1001). It is also before step 4 (the token):
a STOP is an **operator decision**, a missing token an **environment fact** — the
operator must see the line of the gesture they made, including on a host with no
token. `dispatcher::tests::mika2498_le_refus_ne_bascule_pas_le_dispatch_class_du_parent`
exists for that placement and nothing else.

**Surfaces:** INFO `groom_pilot_autofire_stopped` (fields `parent_task_id`,
`callback_task_id`, `repo`, `issue`, `stop_file` — rendered by `stop_file_path`,
never a literal) and an `audit_events` row under the **same `tool_name` as the
nominal path** (`task_engine_groom_pilot_dispatcher`) with
`after_value = 'stopped_by_sentinel'` against the nominal `'implement_dispatched'`.
One name, the outcome in `after_value`, so a `GROUP BY after_value` yields both
counts in one query, subtractable — the `ready_label_outcome` motif (mika#2323),
and deliberately not the two-names motif of `phantom_aged_out` /
`phantom_sweep_spared`, which applies when each name carries its own cause. Named
and dated cost: a bare `SELECT count(*) WHERE tool_name = …` changes meaning at
this deploy; `WHERE after_value = 'implement_dispatched'` stays exact on both
sides and is the query to write. The three values are pinned as a wire format by
`mika2498_les_valeurs_daudit_sont_un_format_de_fil`, and the log name has a
single production writer (`mika2498_le_nom_du_refus_a_un_seul_ecrivain`,
allowlist shipped empty).

**Which scans get a file of their own: `auto_pull`, then `worktree_reap`
(mika#2420).** Distinct question from the paragraph above, which is about how far
*one* sentinel reaches. `is_stopped`/`stop_file_path`
are parameterized by scan name, and mika#2329 shipped that parameterization while
explicitly refusing to use it twice, for want of a measured need: *"stopping QA review
is not the same decision as stopping the feeder."* **A destructive operation is
precisely that need** — during an incident one wants to stop what *deletes* without
restarting mika-spirit, which is the thing one least wants to do with dispatches in
flight. `wip_rescue` and `qa_review_reconcile` still have **no** switch: stopping them
destroys nothing, and shipping gestures nobody asked for stays YAGNI.
Structural guard `mika2329_le_chemin_du_fichier_sentinelle_a_un_seul_lecteur` refuses
a second occurrence of **either** scan's path literal under `src/` — extended with the
second scan in the same commit, because a scan born outside the guard is a scan on
which the `grooming_marker` lesson replays. A copy would make no decision wrong the day
it is written, which is exactly why no behavioural test can see it. Note the guard is
deliberately literal: naming the sentinel path in a *doc comment* is a hit too, so both
call sites reference the constant and the operator-facing path lives in `CLAUDE.md`.
Operator surfaces, the exact gesture, and the post-deploy probe: root `CLAUDE.md`
§ *Optional (STOP global à chaud — mika#2329)* and § *Optional (terminal-worktree
reaper — mika#2420)*.

### Terminal-Worktree Reaper (mika#2420)

`worktree_reaper` — the **fifth** recurring scan, beside `auto_pull`, `wip_rescue` and
`qa_review_reconcile` in `task_engine/dispatcher.rs`, carried by mika-dev, cron
`0 */10 * * * *`. It removes the dispatch worktrees under `.claude/worktrees/` whose PR
is terminal — and their `target/` with them, which is the 25-to-44-Go consumer the
founding ticket measures. Outside the LLM, outside the pilot session, triggered by time.

**It is the only scan of the family whose action is destructive and irreversible**, and
three things follow from that, in this order in the body of `dispatch_worktree_reap`:
the STOP short-circuit is **first**, before any token resolution; the disposition is
gated separately from the detection (`MIKA_WORKTREE_REAP_DISPOSITION=observe`); and
every term of the predicate is fail-safe towards *keep*.

**Three measurements moved the diagnosis, and they are the first deliverable.** The
ticket reads prior art **#1694** as "closed but ineffective". (M1) It is a **dormeur**,
a live line of `docs/dormeurs.md` whose wake condition is now met — mika#2420 is its
*wake*, and U6 removes the line per the register's contract. (M2) Its logic **never
reached `main`**: commit `097cc66c` carries a real implementation saved as `wip()` by
the mika#1282 recovery and never promoted. (M3) Its own doc admits the defect in
writing — *"layers A/B clean up whatever slips through"* — where A and B are **manual**
and C is a `pull_request.closed` webhook handler, i.e. a **single non-replayable event**
losable in the four places mika#2334 already had to name. The gesture whose three-hour
half-life the ticket measures *is* layer B.

**Shape.** `screen_worktrees` (T1–T6) and `apply_work_states` (T7) are pure functions
carrying the whole decision; `select_worktrees_to_reap` composes them for tests while
production calls them in sequence so the two `git` subprocesses of T7 are only paid for
the survivors of T1–T6. The registry is read from `git worktree list --porcelain` —
**the worktree path is declared, never derived** (mika-platform#58) — and `prunable`
entries are dropped (they belong to `git worktree prune`, their directory is already
gone). One `gh pr list --state all` per repo per tick; the `owner/repo` is derived from
`git remote get-url origin` rather than declared in a second list to keep in sync.

**The cap is a cap on writes**, applied by the caller after the filter (mika#2347's
lesson transposed): applied upstream it would cap *skips*, and refused candidates would
consume the tick in place of treatable ones. Pinned by
`mika2420_un_refus_ne_consomme_pas_la_place_dun_traitable`.

**Token** via `resolve_periodic_scan_token` (PAT-first, App fallback), covered by the
mika#2205 structural guard, which was extended to this fifth scan in the same commit.
The scan writes **nothing** on the forge (`gh pr list` is its only call), so there is no
author for GitHub to read in the ADR-008 sense and no
`resolve_periodic_scan_label_token`. `PeriodicScan` gained a variant, so
`mika2334_every_scan_variant_is_covered` fails to compile until the guards enumerate it.

**SOLE WRITER** of both audit `tool_name`s — `worktree_reaped` (`armed`, an effective
removal) and `worktree_reap_would_dispose` (`observe`, the population that *would* be
removed — mika#2469) — pinned by the same two-needle source scan. That is what makes
`SELECT … WHERE tool_name = 'worktree_reaped'` the exact list of worktrees the loop
removed, i.e. the ticket's guard-rail 3: since mika#2469 the name is reserved to the
removal, and `outcome_for(disposition)` is the one site the log event and the audit
`tool_name` both read, so the two surfaces cannot diverge. Refusals are written under
`worktree_reap_skipped`, keyed `worktree:<path>@<motif>` and **deduplicated on 24 h**
(mika#2131): a dirty worktree of a merged PR would otherwise write a row every ten
minutes, and a motif *change* rewrites because it is a state change.

**The negative control is what makes the positive one mean anything, and it had to
assert the motif.** A vacuity-only assertion is **insensitive for T3 and T4**, both
absorbed downstream by T5 (no known PR ⇒ no `closedAt` ⇒ refused; an open PR carries
`closedAt: null` ⇒ refused). Verified by mutation, not by reasoning: removing T3 shifts
the motif from `pr_unknown` to `pr_closed_at_unreadable` while leaving a vacuity-only
test green. All seven terms were mutated one at a time and each was observed red.

**Config, the eleven motifs, the four halts and the named residual risk** (an absent
`refs/remotes/origin/<branch>` reads `Clean`, not `Unreadable`): root `CLAUDE.md`
§ *Optional (terminal-worktree reaper — mika#2420)*.

#### The `target/` of a live-but-idle worktree (mika#2497)

A **third arm of the same tick**, after the reaper, in the same module. Its
population is **exactly** the reaper's **eligible** refusals of that tick —
`pr_open` to begin with (the worktrees T4 turns away because their PR is open,
and whose `target/` therefore lives as long as the PR: 15 to 50 Go per pilot,
+90 Go in 8 h on the night of 2026-09-22, `/data` at 83 %), two motives since
mika#2482, **nine since mika#2619** (§ *Nine motives, N build dirs* below). It is
the *HALT-2 follow-up* mika#2420 names in its own body.

**The ordering is necessary, not tidy.** What the reaper has just removed no
longer exists; considering it for a purge would be a no-op at best and a race at
worst. Everything else is already paid for at that point — `git worktree list`,
the one `gh pr list` per repo, the `/proc` enumeration, the STOP sentinel, the
refusal dedup — so the arm costs no extra query, no `PeriodicScan` variant, no
recurring row, no migration.

**The asymmetry is INVERSE to the reaper's, and that is what licenses touching a
live worktree.** *The reaper removes potential work; this arm removes pure
derivative.* A `target/` carries none: it is wholly reconstructible by
`cargo build`, so a false positive costs **rebuild time, never a loss** — the
exact inverse of *"a false positive destroys hours of work, irreversibly"*. **What
the asymmetry does NOT license:** removing `target/` **during** a `cargo build`
breaks that build. The danger is not data loss, it is **concurrency**, and the
whole predicate is about that and nothing else.

**Five conjunctive terms, all fail-safe towards *keep*.** `screen_target_purges`
(P1 managed path, P2 `target/` exists and is a directory, P3 no live cwd inside,
P4 idle past the window) then `apply_lock_probes` (P5 the cargo build lock is
free) — the same two-stage idiom as `screen_worktrees` / `apply_work_states`, and
for the same reason: P5 costs an `open` + a `flock` per profile and is only paid
on the survivors (mika#2184's motif — *the proxy filters first, the direct
measure decides last*).

**P5 is what T4 used to supply.** P3 is **known to be holed** and mika#2420 says
so itself: a process can work in a worktree without its cwd being there. At the
reaper that hole was covered **by the conjunction** — such a process works on a
branch whose PR is open (excluded by T4) or leaves uncommitted changes (excluded
by T7). **Here that cover is gone: the PR is open by definition of the
population.** P5 answers "is a cargo working in this directory", independent of
cwd and of any delay.

**The lock file is discovered, never guessed**, and the two absences it can meet
have **opposite** dispositions — the point the words confuse most easily:

| what is missing | reading | disposition |
|---|---|---|
| no `.cargo-lock` under `target/` | cargo never built here | `LockProbe::Free` → purge allowed |
| the `flock` call (non-Linux) | we cannot look | `LockProbe::Unevaluable` → keep |

Treating the first as the second makes the arm **inert over a healthy
population** while reading exactly like a healthy disk (class mika#2205);
treating the second as the first purges blind. Hence three variants, never a
bool. The call follows `process_liveness::is_same_process_alive`: a
`#[cfg(target_os = "linux")]` block with an **explicit non-Linux fallback that
keeps**. **Nothing in the manifest forces that `#[cfg]`** — since mika#2900
`libc` is declared under `[target.'cfg(unix)'.dependencies]`, so the crate is
available for a darwin target. What forces it is semantic: `flock` semantics
differ across OSes, and the population of `<target>/<profile>/.cargo-lock` is
only verified under Linux. Rendering "free" there would purge on the one
platform where a running build cannot be seen.

The earlier wording presented the Cargo key as the cause of the `#[cfg]`, and
that is precisely the reading that produced mika#2900: the key was taken for a
platform constraint when it was only a too-narrow declaration, which an unguarded
call site brought down at the first release tag.

**P4 walks a bounded, declared set** — `target/`, its direct children, and their
children (depth 2) — and deliberately **not** `measure_tree_size`, whose 400 000
entry / 2 s budget a 40 Go `target/` exceeds: a truncated walk would report an
mtime **wrong in the dangerous direction** (under-estimating recency, i.e.
purging a live tree). Any unreadable entry yields `None`, which keeps.

**SOLE WRITER** of `target_purged` (armed) and `target_purge_would_dispose`
(observe), pinned by a source scan whose allowlist ships empty and which carries
its own **self-cleaning assertion** — it fails if the name is written *nowhere*,
because a scan aiming at a dead name verifies zero things and reads exactly like
a clean one. Refusals go under `target_purge_skipped` with `target:`-prefixed
keys, deliberately distinct from the reaper's `worktree:` so the two populations
stay subtractable. `ALL_PURGE_REFUSAL_REASONS` is likewise a **separate** wire
format from `ALL_REFUSAL_REASONS`.

**Every term was seen red by mutation, not by reasoning** — and one of those
mutations paid for itself: the symlink test was **decorative**, its link pointing
at a directory named otherwise, which guard 1 (`ends_with("/target")`) already
refused. It is now built on the only shape that traverses guards 1 and 2 — a link
to the `target/` of **another managed worktree**, which is exactly what guard 3
exists to close.

Config, the operator surfaces, the four probes (starting with *run in `observe`
first*) and the four halts: root `CLAUDE.md` § *Optional (purge du `target/` d'un
worktree vif — mika#2497)*.

#### Nine motives, N build dirs (mika#2619)

**The arm stopped reading *why* a worktree is kept.** The population was defined
by *keep motive* while the asymmetry that founds the arm is **indifferent** to it:
keeping the worktree protects the **work** (commits, working tree), it says
nothing about its build cache. Measured 2026-10-01, `/data` at 85 %, **64 Go**
freed by hand across three worktrees the arm could not see — `detached_head_pr_unknown`
(40 Go, a motive born of mika#2518 that nobody added to the list),
`unpushed_commits` (17 Go under `.pilot-scratch/ac6/`, failing on **three**
independent counts: the motive, the path, **and the vector**), `too_young` (8 Go,
legitimately refused by P4).

**`ALL_REFUSAL_REASONS` is now partitioned, and that is what closes the CLASS
rather than the occurrence.** Inverting the polarity to a denylist was refused —
the predicate is fail-closed on the unknown, and a denylist returns `true` for any
unlisted string — but the polarity was never the cause: a two-name list had simply
**fallen behind**. `PURGE_ELIGIBLE_REASONS` (9) and `PURGE_INELIGIBLE_REASONS` (3,
each carrying the reason for its exclusion in a doc-comment) partition the
reaper's vocabulary, and `mika2619_la_partition_des_motifs_est_exacte` requires
every motive to sit in **exactly one** of them, with a non-vacuity assertion
(without which two empty lists would satisfy the partition). A motive added
tomorrow reddens that test until somebody classifies it: the defect is no longer
"purgeable by oversight" nor "unpurgeable by oversight", it is **"no decision, no
build"**. The three exclusions are **redundancies** with P1/P3, never safety
reserves.

**Widening the list alone would have been INERT on three motives.** `dirty`,
`unpushed_commits` and `work_state_unreadable` come out of `apply_work_states`
(T7), whose output lived in a vector this arm **never received** —
`purge_stale_target_dirs` now takes **both** and concatenates caller-side, so the
pure functions keep their `&[ReapRefusal]` and every pre-existing test stays
valid. A second hole, unnamed by the ticket: at zero reaper budget T7 was **not
evaluated at all**, so the survivors of T1–T6 appeared in **no** vector and
escaped both arms. `t7_is_needed(reaper, purge, enabled)` is the named predicate —
*the arm that needs the computation pays for it* — and it is the **exact
complement** of `should_stop_repo_loop` on the same triple, asserted as such: a
divergence would pay `git` for a repo nobody looks at any more, or the reverse.
It is a pure predicate rather than an inline conjunction for `should_stop_repo_loop`'s
own reason — the site lives in a function that shells `git` and `gh`, so no
behavioural test reaches it.

**`discover_build_dirs` is the single site that composes a build-dir path**
(source scan, allowlist shipped empty): `<worktree>/target` if it exists — with
**no marker condition**, so today's population does not shrink, and returned even
when it is not a directory so the `target_not_a_dir` motive survives instead of
silently becoming `no_target_dir` — then a **bounded** walk under
`.pilot-scratch/` (depth 3, symlinks never followed — **the `.pilot-scratch` root
included**: `read_dir` follows a link, so a root symlinked to a neighbour's scratch
used to pull the neighbour's caches into a population whose P3 only asks about
*this* worktree's processes) keeping any directory that
carries a marker **without descending into it**: a build cache contains
subdirectories, and enumerating them would be an unbounded walk inside the very
tree just identified as a cache.

**The marker is a declaration of cache, and it completes the name without
replacing it.** `.rustc_info.json` **or** `CACHEDIR.TAG` at the root — the second
being the standard by which a tool declares *itself* a cache, exactly the
information the founding asymmetry asks for. So `build_dir_disposition` is **name
OR marker**: keeping `ends_with("/target")` avoids shrinking the current
population, adding the marker **replaces proof-by-name** for everything else — and
it is the stronger proof. **A marker is proof only as a regular file, and
`CACHEDIR.TAG` only with the spec's signature** (`CACHEDIR_TAG_SIGNATURE`, what
cargo writes): `is_file` followed links, and any file merely *named*
`CACHEDIR.TAG` made its whole directory disposable. `.rustc_info.json` content
is not validated (named residual). And the late guard now also requires
`build_dir_is_inside_worktree` — the canonical build dir strictly under the
canonical worktree P3 asked about, not merely under *a* managed worktree, which
also covers a parent component swapped for a symlink between discovery and
deletion. A directory carrying neither is **not recognised**, so it
enters no population and **produces neither candidate nor refusal**: that is what
keeps a text draft under `.pilot-scratch/` out of reach, and it is why the mika#2548
dispatch rule is not in tension with this arm (that rule addresses the **pilot**
mid-session; this is engine code, outside any session, on a directory idle for at
least four hours in a worktree **with no live process** — and a dispatched pilot has
`cwd == root`, so P3 removes its whole worktree from the population).

**Three keys moved to the build directory**, because the cardinality went from one
`target/` per worktree to N: `states` is keyed by build dir (and `build_dirs` says
*which* dirs a worktree carries — the two absences are both fail-safe and say
different things: no dir discovered ⇒ `no_target_dir`, a dir with no state ⇒
`mtime_unreadable`), `TargetPurgeCandidate.target_path` carries the build dir
(same field name — renaming it would break `jq '.target_path'`, a published
query), and `purge_refusal_audit_key` takes `TargetPurgeRefusal::audit_path()`.
That last one is load-bearing: with the worktree in the key, **two build dirs of
one worktree refused under the same motive deduplicated each other** and the
second refusal was lost in silence. `build_dir_path: Option<String>` is `None` for
the worktree-scoped refusals (P1, P3, no dir discovered), and **P3 is evaluated
before the per-dir loop** on purpose — it is a property of the worktree, and
refusing N times for one cause is the noise the ineligibility of `live_process`
already refuses.

**`keep_reason` is a field, never a second `tool_name`** (motif `RESOLUTION_BRANCH`
mika#2518, `ready_label_outcome` mika#2323): the removals are made by the same arm,
under the same five-term conjunction, with the same lethality — only the keep
motive differs, and a second `tool_name` would silently truncate
`SELECT … WHERE tool_name = 'target_purged'`, a published query. It rides the INFO
line and `reasoning`, which is what makes the widened population countable apart.

**`not_a_build_dir` is a TARDY refusal and no test reproduces it deterministically.**
It only fires if the marker disappears between the discovery (internal to
`purge_stale_target_dirs`) and the guard a few instructions later — a race. The
motive is covered at the decider by `mika2619_v8_la_garde_tardive_accepte_le_nom_ou_le_marqueur`;
claiming a production test for it would fabricate coverage, which is why
`mika2619_v8_un_marqueur_absent_sort_le_repertoire_de_la_population` asserts the
**honest** outcome instead (`no_target_dir`, worktree-scoped key, directory intact).
That is also what makes the root `CLAUDE.md`'s "expected regime: empty" readable.

Nine motives, the two datable surface changes, the operator probes and their five
halts: root `CLAUDE.md` § *Le bras cesse de lire pourquoi un worktree est
conservé*.

### Un hold explicite tient contre `wip_rescue` (mika#2597)

**Le défaut, mesuré le 2026-09-29.** `wip_rescue` a repris une PR maintenue en
**brouillon de hold**, l'a rebasée puis **sortie du brouillon** — deux fois en
35 minutes sur PR #2589, la seconde après une remise en draft explicite de
l'orchestrateur. La Fire-Disposition du plan interdisait la fusion (rebuild
+190 %, au-delà du seuil de renoncement de +50 %). **Le merge autonome n'a été
empêché que parce que le diff touchait des fichiers decision-core** : sur un
autre jeu de fichiers la PR était CLEAN et APPROVED. La défense en profondeur a
tenu par coïncidence de périmètre, pas par conception.

**Le discriminant que le démon n'avait pas.** Un *draft de rescue* est l'état de
**départ** — `dispatch-lib` fait `gh pr create --draft` — et le promouvoir est la
raison d'être de mika#1852. Un *draft de hold* est un état **remis après** le
travail. Les deux se lisent `isDraft: true` et rien ne les séparait.

**Le prédicat ne compare aucun instant, et c'est le cœur.** La timeline d'une PR
de rescue ne porte **aucun** `ConvertToDraftEvent` à la naissance ; les deux
seules transitions d'état draft sont `ConvertToDraftEvent` et
`ReadyForReviewEvent` ; et le listing filtre déjà `--draft`. Donc une PR qui
porte ≥ 1 `ConvertToDraftEvent` **et** qui est brouillon *maintenant* a
forcément eu son dernier basculement **vers** le brouillon : c'est un hold.
Aucune date, aucun ordre, aucune pagination. *Fragilité nommée* : retirer
`--draft` du listing casse l'équivalence — le sens de la casse est l'inertie, pas
la violation, et `mika2597_le_listing_filtre_toujours_draft` refuse l'édition.

**Les deux pistes du ticket sont réfutées par le code.** (a) *« un
`ConvertToDraftEvent` postérieur au dernier push »* s'auto-annule :
`prepare_branch` **pousse lui-même** en `--force-with-lease` à l'étape 4, avant
l'un-draft de l'étape 7, dans la même chaîne — le prédicat serait voidé par le
push du démon en train de violer le hold ; et un push ultérieur ne lève pas une
décision d'opérateur. (b) *un label porteur* exigerait que l'opérateur apprenne
un **nouveau** geste alors que celui qui a échoué deux fois est « je remets en
draft », et il faudrait le déclarer dans `.github/labels.yml` sous peine de
suppression silencieuse par `delete-other-labels: true` (classe à cinq
occurrences). `human-review-required` est refusé comme support : il porte **sa
propre cause** (« le démon a bailé »), et y router un hold d'opérateur
fusionnerait deux populations que le dépôt compte séparément.

**Le placement EST le livrable.** Le terme vit dans `select_eligible`, en
**troisième et dernière** exclusion (après le marqueur de bail mika#2199 et le
marqueur de park mika#2286), **jamais** dans `resume_chain`. Le scan traite au
plus **un brouillon par tick** (AC6 de mika#1852, cap = 1, le plus vieux
d'abord) : un hold placé dans la chaîne consommerait ce créneau unique à chaque
tick, indéfiniment — et un brouillon de hold est *vieux par nature*, donc il
reste le plus ancien candidat pour toujours. C'est la forme de livelock que
mika#2199 a mesurée (14 bails sur une PR en six heures) et que mika#2286 a dû
refermer une seconde fois. Coût : **un** appel GraphQL par tick en régime
nominal, grâce au court-circuit sur le premier candidat non exclu.

**GraphQL et pas la timeline REST**, qui a pourtant un lecteur maison
(`ready_label::TimelinePageFetcher`) : c'est mika#2315 qui l'écarte. Son défaut
B1 mesuré est que REST rend ses événements en ordre **ascendant**, donc un
événement récent vit en dernière page — d'où sa pagination explicite et son
plafond de 20 pages. Transposé ici : jusqu'à 20 `gh api` par candidat et par tick
(cron 5 min). Le `last: 1` retire d'un coup le piège de l'ordre, la pagination et
le plafond. Réutilisation : `github_graphql::fetch_convert_to_draft_events`, dans
la forme exacte de `fetch_open_blockers` (même client, même classification
401/403/429).

**Trois états, jamais un `bool` (`HoldVerdict`).** `NotHeld` (comportement
d'aujourd'hui, octet pour octet) / `Held { since, actor }` / `Unreadable`. Deux
des trois excluent et appellent des remèdes **opposés** : l'un dit que le
mécanisme marche, l'autre que `wip_rescue` est gelé par son propre fail-closed et
que la cause est le jeton ou l'API. Un booléen ferait lire le second comme le
premier. **Fail-closed vers « tenu »** — *un terme qu'on ne peut pas lire n'est
jamais un terme satisfait* (mika#2277), ici appliqué au terme « ce brouillon
n'est **pas** tenu ». C'est aussi la politique **uniforme** du module :
`has_bailed_marker`, `has_parked_marker`, `classify_route` et
`fresh_pipeline_verified` sont tous les quatre fail-closed. Coût nommé : une
panne durable de l'API GitHub gèle `wip_rescue` en entier, d'où le nom
d'événement distinct pour l'inertie.

**Pas de filtre d'acteur machine, et c'est une décision.** mika#2315 en porte un
parce que son propre `remove → add` se parquerait lui-même ; ici **aucun** chemin
n'écrit de `ConvertToDraftEvent`, donc le terme aurait une population vide — et
si une machine s'y mettait, lire son geste comme un hold est le sens **sûr**.
L'acteur et l'instant sont **rapportés** sur la ligne d'observabilité sans rien
décider, ce qui rend visible le jour où cette population cesse d'être vide.

**Sortie du hold : aucun geste à apprendre, aucun état à nettoyer.** L'opérateur
sort la PR du brouillon ; elle quitte le listing `--draft` et donc la population
entière. Le hold n'existe que tant que la PR *est* un brouillon — il se lève de
lui-même, comme le park de mika#2315. C'est ce qui distingue ce remède d'un
marqueur durable : il n'y a rien à effacer.

**Deux noms, comptables séparément.** `wip_rescue_hold_respected` (INFO + ligne
d'audit, **régime attendu non vide** — chaque ligne est un hold que le démon n'a
pas violé) et `wip_rescue_hold_unreadable` (WARN + ligne d'audit, **régime
attendu vide**). Motif `phantom_aged_out` / `phantom_sweep_spared` (mika#2156) :
les fondre ferait lire une panne comme un succès. Dédupliqués par `(PR, motif)`
sur 24 h — un hold est un **état**, pas un événement, et un brouillon tenu trois
semaines écrirait sinon 288 lignes/jour (doctrine mika#2131, forme exacte de
`worktree_reap_skipped`). La clé porte le motif après un `@`, ce qui rend un
`LIKE` de préfixe sûr (mika#2361 : `#234` ne doit pas apparier `#2343`). La ligne
INFO suit la **même** porte que la ligne d'audit, comme chez son voisin
`worktree_reaper::record_main_checkout_dirty` — coût nommé : une base illisible
est aussi un hold non rapporté, ce qui est un incident en soi et ne change
**jamais** la décision.

**Deux scans de source, allowlists livrées vides.**
`mika2597_un_seul_site_dundraft_en_production` refuse un second `gh pr ready` en
production — aucun test comportemental ne peut voir cette classe : un second site
ne rend **aucune** décision fausse le jour où il est écrit, toutes les assertions
restent vertes pendant qu'un chemin un-drafte sans consulter le hold. L'aiguille
est cherchée sur une source **normalisée en espaces**, donc rustfmt ne peut pas
la cacher ; `validate_pr_ready_undraft_scope` (mika#1682) compare
`verb == "ready"` à une variable et sort de la population **par sa forme**, pas
par une exemption. Quand il tire, **on route le nouveau site par la garde ; on
n'ajoute pas de ligne** (doctrine mika#2201).
`mika2597_le_listing_filtre_toujours_draft` épingle la prémisse ci-dessus.

**Le contrôle négatif V3 est porteur** : sans lui, « la garde décide » est
indistinguable de « la garde bloque tout », et le mécanisme mika#1852 pourrait
être mort avec tous les tests au vert. Chaque terme a été vu **rouge par
mutation**, un à la fois — une conjonction de termes fail-safe ne se prouve pas
en les neutralisant tous ensemble (leçon mika#2277).

**Ce que ce travail ne fait PAS.** Il ne rattrape pas l'incident fondateur :
mika#2589 a été sortie du brouillon deux fois et **rien ici ne réécrit une ligne
d'audit datée d'un hold qu'on n'a pas observé** — la sonde est la **prochaine**
occurrence. Il ne fermait pas le chemin `gh pr ready` d'un modèle sur une PR
**hors signature `wip-rescue`** — suivi nommé, dont la précondition (une mesure)
est tombée le 2026-10-01 et que mika#2624 ferme : § suivant. Il ne retire pas le
palliatif à trois couches, et il ne touche pas le bypass admin de l'identité du
pilote — les deux moitiés tombent séparément. Aucune valeur de réglage ne bouge,
aucune migration, aucune variable neuve.

Surfaces opérateur, régimes attendus, sondes et haltes : racine `CLAUDE.md`
§ Signal N.

### Le même hold tient contre `run_gh pr ready` (mika#2624)

**Le défaut, mesuré le 2026-10-01 (n=1).** samidarko remet PR #2621 en brouillon
à 14:00:41Z pour qu'une revue ait lieu avant le merge ; à 14:46:03Z mika-dev
appelle `run_gh ["pr","ready","2621"]` après avoir **lu** `isDraft=true`. Le hold
saute ; une revue par spawn trouvait au même moment un P1 de perte de données.
Le merge autonome n'a été évité que par calendrier. Même invariant que mika#2597,
**second site d'écriture** : le garde mika#1682 de `run_gh` ne connaissait que la
signature `wip-rescue`.

**Sept rectifications que la lecture du code impose, et c'est le premier
livrable.** (R1) le discriminant de mika#2597 est appelable, mais sa
**classification** était inline dans `hold_verdict`, privée et couplée à
`report_hold` : seule elle est extraite, en `classify_hold_verdict`, et le fetch
comme le report restent où ils sont. (R2) `detect_ready_promote_pr` couvre **deux**
formes, dont `pr edit <N> --title` qui ne touche pas l'état draft : le terme hold
a son propre détecteur, `detect_pr_ready_undraft`, strictement plus étroit, et un
renommage de titre d'une PR tenue n'est pas refusé. (R3) le handler stale-verdict
n'émet aucun prompt — il n'y avait pas de prescription à corriger ; mais **quatre**
prompts (`self-dev-webhook-{ci,qa,ready-label}`, `self-dev-callback`) interdisaient
`pr ready` *par délimitation* de la signature `wip-rescue`, donc le **permettaient
par contraste** — la lecture exacte du tour fautif. Ils deviennent topiques.
(R4) `gh api` est déjà une allowlist fermée (`GH_API_ALLOW_MATRIX`) : ni `graphql`
ni `/repos/*/pulls/*` n'y matchent, donc `markPullRequestReadyForReview` et
`PATCH draft=false` sont **exclus par mesure**, pas par un second discriminant.
(R5) `run_shell` refuse `gh` nommément et ses routes indirectes. (R6) le scan
`mika2597_un_seul_site_dundraft_en_production` exige **un** site `"pr","ready"`
en production : le terme hold compare des verbes à des variables et reste hors
population **par sa forme** — aucune constante `&["pr","ready",…]` en moitié
production. (R7) un pilote claude-pilot tape `gh` par Bash dans son bac à sable,
jamais par `run_gh` : **exclu et non couvert**, population mesurée vide, suivi.

**Un garde, deux termes, et l'ordre n'est pas arbitraire.** Les deux termes vivent
dans `validate_pr_ready_undraft_scope` parce qu'ils répondent à la même question —
*cette PR peut-elle sortir du brouillon ?* — et deux gardes au même site sont deux
sites qu'un éditeur peut faire diverger. Terme 1, mika#1682, inchangé et
**fail-open** sur sa lecture (`gh pr view`) ; terme 2, mika#2624, **fail-closed**
sur la sienne (un appel GraphQL, sur la seule forme `pr ready`) : un refus du
terme 1 économise l'appel du terme 2.

**Fail-closed, et l'asymétrie est locale.** `NotHeld` autorise octet pour octet
comme avant ; `Held` refuse sous `operator_hold` ; `Unreadable` refuse sous
`hold_unreadable` ; `ctx.github_token` absent refuse sous `hold_no_token` (`gh`
peut s'authentifier par `~/.config/gh/hosts.yml`, donc laisser passer garderait
ouverte exactement la population que le moteur ne peut pas mesurer). Un faux refus
coûte un `pr ready` visible et rattrapable ; un faux passage coûte le hold lui-même.
C'est l'**inverse** du faucheur mika#2420, et l'arbitrage ne se transporte pas.
Pas d'interrupteur d'environnement (précédent mika#2573) : le désarmement est un
revert.

**Résolution du dépôt, coût nommé.** Sans `repo` déclaré, repli sur
`wip_rescue::DEFAULT_REPO`. Une PR d'un autre dépôt interroge alors la mauvaise
timeline, l'extracteur rend `Err`, le refus tombe sous `hold_unreadable` — et son
corps **nomme le paramètre `repo`**. Le corps du refus nomme le hold, `since` et
`actor` quand GitHub les rend (jamais inventés, jamais `"null"`), dit que seul un
humain le lève, et **ne nomme aucun contournement** (doctrine mika#2520).

**Surfaces.** La ligne `tracing::info!(event = "pr_ready_undraft_blocked")` de
mika#1682 garde son nom, gagne `hold_since` / `hold_actor`, et gagne une ligne
`audit_events` sous le même nom — motif en `after_value`, **SOLE WRITER**
`PR_READY_UNDRAFT_AUDIT_TOOL` — avec `pr_ready_undraft_audit_failed` (WARN) quand
l'audit échoue. Un nom, quatre motifs (`operator_hold`, `hold_unreadable`,
`hold_no_token`, `wip_rescue_contract`), format de fil épinglé par test : motif
`ready_label_outcome` (mika#2323), pas deux noms, parce que les motifs
appartiennent au même site et à la même population. **Coût daté :** mika#1682
n'écrivait aucune ligne d'audit, donc un `count(*)` qui enjambe le déploiement
compare un vide à une population ; la requête juste groupe par `after_value`.

```sql
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'pr_ready_undraft_blocked' GROUP BY 1 ORDER BY 2 DESC;
```

| motif | régime attendu | lecture |
|---|---|---|
| `operator_hold` | non vide, faible | un hold que le moteur n'a pas levé |
| `hold_unreadable` | **vide** | lire d'abord si `repo` était passé, puis le jeton — **ne pas basculer en fail-open** |
| `hold_no_token` | **vide** | `run_gh` tourne sans jeton résolu (mika#2205) |
| `hold_unresolved_selector` | **vide** | `pr ready` sur une branche, `#N`, sans sélecteur, ou `repo` déclaré illisible : le terme ne peut pas adresser la timeline, il refuse (revue de PR #2628) |
| `wip_rescue_contract` | inchangé en sens | la population mika#1682, comptable séparément |

**Gardes structurelles**, allowlists livrées vides :
`canonical_tokens::tests::mika2624_le_predicat_de_hold_a_un_lecteur_unique` (tout
appel de production à `fetch_convert_to_draft_events` est l'argument immédiat de
`classify_hold_verdict` ; anti-vacuité ≥ 2 appels) et
`…::mika2624_le_nom_de_refus_a_un_ecrivain_unique`. Les deux lisent la production
**jusqu'au module de test**, pas jusqu'au premier `#[cfg(test)]` :
`builtin_handlers.rs` en porte un sur une constante, et l'énumérateur partagé les
rendait verts par population vide — leurs anti-vacuités l'ont dit.

**Sondes post-déploiement, et leurs haltes.** *S1* — sur une PR remise en
brouillon à la main, un `run_gh pr ready` doit être refusé `operator_hold`, sans
`ReadyForReviewEvent`. Halte : la PR sort du brouillon → lire d'abord
`grep -c 'run_gh invocation'` (le garde tourne-t-il ?), puis **par quelle porte**
l'appel est passé (`run_shell`, `gh api`, pilote — trois remèdes, seul le dernier
est ouvert). *S2* (7 j) — `wip_rescue_success` continue de croître ; s'il se fige,
la classification extraite a changé de sens : revert d'abord. *S3* (7 j) —
`hold_unreadable` et `hold_no_token` restent vides. *S4* (7 j) — aucun refus
`operator_hold` sur un `pr edit --title` (sinon le terme a été branché sur le
détecteur à deux formes : réparer le détecteur). *S5* — `cargo test -p mika-agent
mika2597_` reste vert ; s'il rougit sur la cardinalité, retirer le littéral, **ne
pas allowlister**. Zéro refus **et** zéro `run_gh` ne prouve rien (mika#2205).

**Ce que ça n'achète PAS.** L'incident de PR #2621 n'est pas rattrapé (rien ne
rétro-écrit un refus non observé). Un pilote dispatché peut toujours taper
`gh pr ready` (R7). Le modèle peut toujours *vouloir* sortir une PR du brouillon ;
il ne peut plus le faire sur une PR tenue. Le `Passthrough` muet du réveil
stale-verdict n'est pas enrichi (mika#1745 a tranché que ce chemin laisse le choix
au modèle). Et rien n'est surveillé : le silence des requêtes ci-dessus ne prouve
rien tant que personne ne les exécute.

### Unknown-Trigger Veto Lift (mika#2337)

**The failure this closes is a veto, not a missing wire.** A `run_skill` recurrence
whose trigger the running binary cannot route falls into `dispatch_run_skill`'s
catch-all and dies `failed`. There is **no spam**: re-enqueue lives only in
`fire_task`'s `Ok` arm, so a recurrence that fails is never rescheduled — the line
dies once and goes quiet, which is why it took a day to notice. What the death
leaves behind is the mika#1742 refuse-to-zombie veto, armed for
`RECURRING_ZOMBIE_GRACE_HOURS` (24 h), whose only exit
(`revert_config_cancel_recurring_task`) targets `status = 'cancelled'` and never
lifts a veto born of a `failed`. **Every restart inside that window — including one
carrying the correct binary — refuses to re-register.** The natural remedy is
exactly what the state neutralises.

**Why this class alone is exempt.** mika#1742 exists because a recurrence that
kills the system must not re-arm on every restart. Here the premise does not hold:
the binary that re-registers a trigger name is, by construction, the binary that
carries its arm — so the death predicts nothing about the next one, and the veto
only prolongs the outage it was meant to contain. mika#1742 is **not** disarmed in
general: any other cause of death still arms it
(`db::tests::mika2337_any_other_death_still_arms_the_veto`).

**Mechanism, three writes.** (1) `DispatchError::UnknownTrigger { trigger }` — the
class comes from the **variant**, never a substring on the rendered message; the
message text itself is unchanged, so operator greps and log history keep working.
(2) `fire_task` stamps `RECURRING_UNKNOWN_TRIGGER_PATH` on the row **before**
`update_task_failed` (never an unmarked corpse, not even between two writes), emits
the named `recurring_unknown_trigger` WARN instead of the generic
`task dispatch failed` — indistinguishable from a network failure — and writes an
`audit_events` row. The terminal state stays `failed`: what is corrected is its
*consequence* on re-registration. (3) `create_recurring_task_if_absent` skips a
marked dead sibling, and **spends** the lift in the same statement that honours it
(marker removed, `RECURRING_UNKNOWN_TRIGGER_LIFT_CONSUMED_PATH` written). A
**second** unknown-trigger death on the same label inside the window meets a fully
armed veto — the lift buys one restart, not immunity. The consumed marker rides on
the row it was spent on, so it ages out of the grace window with it and the
exemption re-arms rather than being lost for good.

**Not retroactive, and that is a stated bound.** A row that died *before* the
stamping code existed carries no marker, so it stays a `dead_sibling`. mika#2337
makes every **future** death repairable by restart; it does not retroactively
unblock the one that caused it. Reading a
`mika#1742: refusing to re-register` on `qa_review_reconcile` at the first
post-deploy startup is that pre-existing row, **not** a regression — wait out the
window, or clear the row by hand.

**Guards.** `tests/eval/test_recurring_trigger_wiring_2337.rs` — a class guard
asserting every **registered** trigger has an arm (population = the calls to
`task_engine::ensure_recurring_task`, anchored on the *caller* rather than on the
`{"trigger":"X"}` literal, which has three false members in the tree), plus a
firing probe driving the recurrence through `TaskEngine::tick` with no network.
`db::tests::mika2337_*` carry the veto behaviour. **Honest scope:** these close the
**intra-binary** divergence; none of them would have caught the 2026-09-16 incident,
which is a skew between merged and running code.

**Operator surfaces.** `SELECT * FROM audit_events WHERE tool_name =
'recurring_unknown_trigger'` — **must stay empty** in nominal operation; any row
names a registered trigger a running binary does not know, i.e. a version skew.
Grep `recurring_unknown_trigger` in `$MIKA_SPIRIT_LOG_FILE`. If
`unknown run_skill trigger` reappears after a deploy, **do not touch the
dispatcher** — establish the version of the running binary instead.

**Registry inspector (mika#2360).** `GET /api/v1/recurring-tasks` (dashboard or
internal token; `?agent_id=`, `?page=`, `?per_page=`) lists every
`trigger_type = 'recurring'` row — all statuses, sorted `label COLLATE NOCASE,
created_at` so a label's duplicates sit together — as a **closed projection**
(`db::RecurringRegistryRow`: `label, agent_id, trigger_type, action_type,
cron_expr, next_fire_at, status, created_at, updated_at, zombie_veto_active`).
No `action_config` / `result` / `input_context` / `metadata`: a `send_message`
recurrence carries the user's reminder text in `action_config`, which is why this
is not `GET /api/v1/tasks?trigger_type=recurring`. `zombie_veto_active` is the
mika#1742 guard evaluated per row in SQL (lift-spent term correlated on
`(agent_id, label COLLATE NOCASE)`, mika#2337); `db::tests::mika2360_zombie_veto_flag_*`
pin it to `create_recurring_task_if_absent`. Reached from outside the cluster
through the gateway's `GET /admin/tenants/{customer_id}/recurring-tasks`
(admin read token — see `crates/mika-gateway/CLAUDE.md`).

### Structural CI Failure Handler

`server::ci_failure_handler` — intercepts `check_suite.completed(failure|timed_out)` webhook events **before** the LLM turn. Failure-side companion to `ci_success_handler`.

**Surface-for-adoption on the no-task arm (mika#1745).** Step 4's `None` arm — a CI failure on an open PR that no mika task matches — used to return `Passthrough { enrichment: None }`, i.e. hand the model the raw webhook text under a prompt whose decision table says *"ignore: this CI failure is not from our work"*. `surface_for_adoption` replaces that silence, and it is written **here** rather than in the prompt for the reason mika#2120 measured (nine recurrences under prompt enforcement against zero when the fact is posed by code).

**Additive, and `Passthrough` never `Handled`** — `Handled` would tell the LLM an action was taken, and AC3 forbids automatic adoption: no task is created, no dispatch is triggered, no status is written, no process is signalled. Three surfaces, of which the first two hold whatever the model does: an `audit_events` row, an operator notification, and an `enrichment` the LLM reads (the intent half).

**The ownership list is `DISPATCHABLE_REPOS`, and that is a decision.** Two lists exist and they answer different questions: the gateway's `INTERNAL_REPOS` decides which repositories **route** to mika-dev (it carries `claude-pilot` and `wizzard`), `DISPATCHABLE_REPOS` decides where the loop may **dispatch**. AC1's list is the second, because the surface *proposes an action* — "create a task from this event and engage" — and that list is the one answering whether the proposal is executable. Surfacing `claude-pilot` would put a proposal in front of the operator that the mika#2046 gate refuses structurally, so that repository belongs to **AC4 (silent-stop)**. No third list: that would be the programmed divergence this repo has paid for twice (dispatch seats mika#2092, `DISPATCHABLE_REPOS` ↔ `labels.yml`).

**Every term is fail-safe towards today's silence**, and each was seen red by individual mutation — a conjunction of fail-safe terms is not proven by neutralising them all at once (the mika#2277 lesson). A repository that is not dispatchable, an unreadable `head_sha`, an unreadable `audit_events`: each falls back on the pre-mika#1745 behaviour. `is_already_surfaced` takes the `Result` rather than a count so the fail-open half is a property of a pure function instead of a branch no in-memory test can reach.

**Detection is unconditional; only the notification is gated** (`MIKA_SURFACE_FOR_ADOPTION`, default armed, motif mika#2249/#2272). **SOLE WRITER** of `surface_for_adoption` in the log and in `audit_events`, held by `canonical_tokens::tests::mika1745_the_surface_name_has_a_single_writer` with an allowlist shipped empty — that property is what makes the operator's `GROUP BY target_key` exact, and that count is the explicit precondition of the auto-adoption decision AC3 defers. **Named blind spot, inherited:** `production_sources()` truncates each file at the first textual `#[cfg(test)]` *wherever it is*, including inside a doc-comment, so a second writer planted in `webhook_dispatch.rs` (which cites one on line 110) is invisible to that scan — verified by mutation. The limit is shared by every scan in that module and is not mika#1745's perimeter; what makes it bearable is `mika1745_the_writer_predicate_sees_a_bare_literal`, which attests the **predicate** bites independently of what the enumerator feeds it.

Doctrine, the three verification cases with their rectified status (AC-R), operator surfaces and the four post-deploy halts: `docs/architecture/mika-dev-work-assignment.md`. Matches CI failures to open PRs and existing work items, fetches failing-job context (up to 3 jobs, 100 lines each), and constructs a pre-digest instructing the LLM to dispatch `run_claude_pilot` for an autonomous fix. Circuit breaker: `ci_fix_count >= 2` in task metadata triggers escalation instead of dispatch — the handler increments `ci_fix_count` deterministically (not reliant on LLM). Checks both task-level callback children and global dispatch guard, including results in the pre-digest. Reuses `VerdictAction`, `find_open_pr`, `run_gh_checks`/`classify_checks`, and `has_active_callback_child` from sibling modules. Also fixes `CHECK_SUITE_RE` regex in `webhook_queue.rs` to match actual gateway format (was `Check suite (failure)`, corrected to `Check suite failure`). Order-independent with other handlers. 30s timeout per subprocess call. See #594.

### Webhook Deferral Queue

`server::webhook_queue` — in-memory queue that holds inbound GitHub webhooks when the target task has an in-flight `run_claude_pilot` callback (#528). Prevents race conditions where a webhook (e.g. `pull_request_review.submitted`) arrives before the callback persists metadata (`pr_url`). Correlation: PR URL via `parse_pr_review_event()`, branch via check_suite regex, fallback to sole-inflight-callback heuristic. 60s per-webhook timeout with forced replay. Drain triggers: callback completion in `handle_task_complete` (Ok path only), or timeout expiry via `drain_expired()`. Emits `webhook_deferred` and `webhook_replayed` audit events. Queue is in-memory only (lost on restart; GitHub supports redelivery). See #528.

### Bounded Webhook Queue (v2)

`server::webhook_queue_v2` — per-agent bounded queue with backpressure + coalescing (mika#1870). **A different mechanism from the mika#528 deferral queue above** (which sequences webhooks against in-flight callbacks). This is the general-purpose ingestion queue that replaces `POST /message`'s legacy `try_lock_owned()` → 429-reject pattern. **It does NOT serve `/a2a/{agent}`, which has its own waiting form — see § Bounded A2A Wait Line below and do not unify the two without reading why.** **Uniform-queue model:** every inbound `POST /message` request enqueues; a single per-agent drain worker (`handlers::spawn_webhook_drain_worker`) is the sole consumer that acquires `agent_lock` and runs `run_agent_for_message`. The mika#528 deferral check runs **before** the enqueue and is unchanged. The accepted-case HTTP response is byte-identical to the legacy path (`status: "accepted"`) — the queue is invisible to the gateway; only the busy case changes (429 → queued).

**Classification + coalescing (AC2):** `classify_event(text)` maps gateway-formatted text to an exhaustive `WebhookEventKind` (`CheckSuite`/`PullRequestSync`/`Push`/`PrReview`/`IssueLabeled`/`ReadyLabel`/`Other`); `coalescing_key(&kind)` is an **exhaustive match with no wildcard arm** — a new variant fails to compile until a coalescing decision is made. Coalescing keys: `check_suite:{repo}:{branch}`, `pr_sync:{repo}:{pr}`, `push:{repo}:{branch}`, `labeled:{repo}:{issue}:{label}`; `PrReview`/`ReadyLabel`/`Other` return `None` (never coalesce — user input / dispatch trigger / order-preserving). `PR synchronize` → `PullRequestSync`; other PR actions (opened/closed/review_requested) → `Other`. Reuses `verdict::parse_pr_review_event` + sibling regexes. `Push` is reserved (the current gateway does not emit a branch-bearing push text; `classify_event` never produces it today).

**Enqueue algorithm (HYBRID: coalesce → block → drop-oldest):** (1) if a queued sibling shares the coalescing key → remove it, push the new to the back (newest-wins), return `Coalesced{replaced_event_id}`; (2) else if `depth < max_depth` → push, `Enqueued{depth}`; (3) else block up to `block_timeout` (default 100ms) on a `Notify` for a drain slot, retry once; (4) still full → drop the oldest (`pop_front`), push new, `Dropped` (dead-letter surface). `dequeue()` is cancel-safe (nothing popped until an item is present), awaits an item `Notify` when empty. Two `Notify` instances separate item-available (empty→wake) from slot-available (full→wake) to avoid mixed-semantics lost wakeups.

**`check_suite` SHA caveat (correctness-critical):** mika#1869 keys on `(repo, branch, head_sha)`; the gateway text carries repo+branch but **NOT** `head_sha`. v1 mitigates the same-branch-double-push missed-event hazard via **time-bounded coalescing** — only entries *still in the queue* coalesce, and the downstream `ci_success_handler` re-aggregates all required checks on every invocation, so a coalesced-away duplicate skips only a redundant walk, never a state transition. Threading `head_sha` end-to-end is a deferred cross-repo follow-up. Single FIFO priority class in v1; priority classes deferred.

**Drain worker (AC4):** one `tokio::spawn` per agent (boot loop in `run_server`; lazy-resolved agents get one in `resolve_agent`, mika#1399), `select!`-ing over a parent `CancellationToken` (`AppState.webhook_queue_shutdown`, cancelled at shutdown alongside `kg_shutdown_token`), `dequeue()`, and a 5s gauge heartbeat. Processing runs in a child task so a panic never crashes the loop (captured as `JoinError`). Blocking `lock_owned().await` serialises turns — modeled exactly on `replay_deferred_webhooks`.

**Audit events (AC5, throttled ≤1/sec/action/agent via `AppState.webhook_queue_audit_last`):** `webhook_queue_enqueued`, `webhook_queue_coalesced`, `webhook_queue_drop_oldest`, `webhook_queue_dequeued`, `webhook_queue_processing_error` (all `target_key = agent:{name}`). Best-effort 5s structured-log gauge `webhook_queue.gauge` (`depth`/`enqueued_total`/`coalesced_total`/`dropped_total`), emitted only when the queue is non-idle. **Config (AC6):** `MIKA_WEBHOOK_QUEUE_MAX_DEPTH` (64), `MIKA_WEBHOOK_QUEUE_BLOCK_TIMEOUT_MS` (100), `MIKA_WEBHOOK_QUEUE_ENABLED` (true; `false` = kill-switch AC9 → legacy 429-reject path verbatim, no redeploy). **Operator grep signals:** `webhook_queue_coalesced` (real coalescing volume), `webhook_queue_drop_oldest` (overflow/dead-letter — should be near-zero), `webhook_queue.gauge` (depth trend), and `rate_limit_trip` near-zero once enabled (baseline was 41/h on mika-qa). See mika#1870.

### Bounded A2A Wait Line (mika#2163)

`server::a2a_wait_queue` — the two `/a2a/{agent}` lock gates (`a2a.rs`
`handle_message_send`, `handle_message_stream`) **wait in a bounded line** for
`agent_lock` instead of refusing on collision. Before mika#2163 both did
`try_lock_owned()` and answered `-32603 "Agent is busy"` — a refusal, on a code
whose defined meaning is "the server failed", for what is ordinary contention.
`/a2a` is the path of every `mika ask` (`mika-cli/src/commands/ask.rs`) and
therefore of every pilot `canUseTool` callback, so the refusal was load-bearing:
the founding measurement is a grooming architect pass refused **five times in a
row** at 20 s intervals before the sixth attempt landed.

**Why this is not `webhook_queue_v2`, and must not be merged into it.** mika#2163
AC1 asked to reuse the mika#1870 mechanism; reading the code shows it does not
transpose. `WebhookQueue`'s producer has no return channel (`enqueue` answers
`202`, a drain worker runs the turn later) while `message/send` is synchronous and
its caller holds the connection open for the completed `Task`; its coalescing is
inert here (two `mika ask` calls never merge); and its saturation policy is
`drop_oldest`, which cannot apply to a request someone is waiting on. Generalising
it would mean a per-entry `oneshot`, a second saturation policy and a second drain
worker — putting `POST /message`, the autonomous loop's own dispatch path, into
this fix's blast radius. **Decision R-A, ruled by mika-prime 2026-09-05: take the
mika#1870 *form*, not its code.** The duplication of form between the two modules
is deliberate and load-bearing: the control contract differs (`/message` is
fire-and-forget, `/a2a` is synchronous). A future unification that skips this
paragraph reintroduces exactly what was rejected.

**Mechanism.** No second queue structure is introduced. The wait is the one
`tokio::sync::Mutex` already provides — documented FIFO-fair (`tokio-1.53.1`
`sync/mutex.rs:112-116`), so the bound bounds a line and not a scramble — and a
per-agent `Semaphore` (`AgentState.a2a_wait_slots`) only makes the depth of that
line explicit and refusable. **The permit is released at lock acquisition, never
at turn end**: holding it for the turn would make one wait and one execution count
against the same number, and the configured depth would stop meaning anything.

**The two gates differ, deliberately.** `message/send` waits **in the handler**
(it is synchronous; there is nothing to return early with). `message/stream` takes
its place in the handler **before** the `spawn` — take it inside and the spawn is
unbounded and the backpressure decorative — but waits **inside** the spawned task,
so the SSE stream opens immediately rather than staying silent until the lock
frees. Saturation is still answerable as a JSON-RPC error on both, because it is
detected before the stream opens.

**Caller abandonment (AC7) — the risk this fix creates rather than removes.** A
spawned task is not cancelled when the client disconnects, so a streaming caller
that hangs up mid-wait would otherwise still acquire the lock and run a turn
nobody reads, holding its place in front of everyone behind it. The spawned task
therefore races `wait_for_agent_lock` against `tx.closed()` (`broadcast.rs:919`,
completes when the last SSE receiver drops) with `biased`. The abandonment lands
**before** acquisition: the place in the mutex FIFO is given up, the permit
returns, the agent is never started, and the task row goes `canceled`. Detection
is bounded by the SSE `KeepAlive` interval, which is a transport property — the
in-crate test pins the mechanism and the accounting, and says explicitly that it
does not measure the over-the-wire delay.

**Error surface.** `AGENT_BUSY = -32000` (`mika-a2a/src/jsonrpc.rs`), **not**
`-32099`: the A2A spec claims the whole `-32001`..`-32099` band for itself
(`a2aproject/A2A@main` `docs/specification.md` §9.5; `-32001`..`-32009` assigned
today), so every code from `-32001` down is a number in someone else's namespace.
`-32000` is the one code of the JSON-RPC implementation-defined server-error band
A2A does not claim. `error.data` carries
`{reason: "queue_full"|"wait_timeout", retry_after_ms, queue_depth}` —
`retry_after_ms` is the **configured** bound, not a prediction of the running
turn's length, which the server does not know.

**Config (AC3, three tiers, same shape as mika#1870):**
`MIKA_A2A_QUEUE_MAX_DEPTH` (8; `0` → default + WARN),
`MIKA_A2A_QUEUE_WAIT_TIMEOUT_MS` (30000; `0` honoured as "do not wait"),
`MIKA_A2A_QUEUE_ENABLED` (true; `false` = kill-switch → `try_lock_owned()` →
`-32603 "Agent is busy"` **verbatim, error code included** — a rollback that
changed the code would not be a rollback). **The 30 s default is sized against the
tightest real caller budget, not the most generous one:** `A2aClient::DEFAULT_TIMEOUT`
is 300 s, but the claude-pilot `canUseTool` relay — the path this ticket was filed
about — is **120 s** (`.claude/claude-pilot.json`) and covers the wait *and* the turn.
A 120 s wait, reasoned from the 300 s figure alone, would spend a pilot's whole budget
waiting and let the relay kill `mika ask` at the exact instant the wait expired, so the
caller would never read the refusal it was owed. Pinned by the assertion in
`config::tests::a2a_queue_defaults`.

**Two costs this creates, named so they are not rediscovered.** (1) `returnImmediately`
is **exempt** from the line and must stay so: that branch returns a `submitted` Task
without running the agent loop, so it needs neither the lock nor a place — harmless as
a `try_lock`, not harmless as a wait. (2) A cross-agent call cycle (A's turn shells out
`mika ask --agent B` while B's shells out `mika ask --agent A`) now stalls for the wait
budget instead of failing in microseconds. Inherent to waiting rather than refusing; the
budget and the kill-switch bound the damage. If such cycles become a pattern rather than
an accident, the answer is a cycle detector, not a longer wait.

**Operator grep signals** (`$MIKA_SPIRIT_LOG_FILE`, throttled ≤1/sec/action/agent
via `AppState.a2a_queue_audit_last`): `a2a_queue_wait` (a wait that actually
happened, carries `wait_ms`), `a2a_queue_reject` (carries `reason` and the code),
`a2a_queue_abandoned` (a streaming caller hung up before acquiring — no orphan
turn ran). Audit-events SQL surface: `SELECT * FROM audit_events WHERE tool_name
IN ('a2a_queue_wait', 'a2a_queue_reject')`. Post-deploy expectation: `mika ask`
retry loops written by hand in caller shells stop firing.

**Relation to mika#2160.** Raising `MIKA_DISPATCH_MAX_CONCURRENT_IMPLEMENT` above
1 was gated on this ticket: two pilots whose permission escalations overlap would
have seen one refused mid-session. This is an *operational* prerequisite of N>1,
not a delivery one — mika#2160 ships its default of 1 without it.

### No Completed Turn Is Lost In Silence (mika#2270)

**The failure.** Eleven consecutive `mika ask` calls came back empty on
2026-09-09 — `.content` absent, **exit 0** — while the engine had produced the
full answer and written it to the log (trace `d5887aa7`, 43 108 input tokens,
28 s, `stop_reason=EndTurn`). mika#2266's architect verdict had to be harvested by
hand out of a 19 GB log file for the grooming to finish. The shape is what makes
it a loop-breaker rather than a slowdown: well-formed JSON, code zero, a key
simply missing — **nothing distinguished "the agent had nothing to say" from "the
answer was lost"**. A probe that lies without failing.

**M1 — `message/send` held the answer and threw it away; `message/stream` never
did.** `run_a2a_agent` returns `Result<Option<String>, String>`: the turn's text,
in memory. The stream port serves it directly. The send port filtered it out with
`Ok(_)` and then **rebuilt a Task from the database** via `a2a_build_task`. Same
loop, two ports, one of which kept the reply in hand. That is the literal shape of
"LLM cost paid and discarded".

**M2 — the renderer's "three tiers" were never three sources**, and its own doc
comment claimed defence in depth for months. `a2a_insert_artifact` has no
production caller, so tier 1 is dead on the spirit path; and `a2a_build_task`
derives `status.message` from the last agent-role message of `history`, which is
`a2a_get_messages`' result — so tiers 2 and 3 fail **together**, on one query.
Chasing the cause in the renderer, which is what the ticket's main lead proposed,
could not have converged: the renderer was the victim.

**What ships.** (a) `message/send` keeps the loop's text and guarantees that a
terminal Task it serves carries at least one non-empty slice —
`ensure_send_task_carries_text`, whose decision half `decide_content_net` is a
pure function with three outcomes (`Nominal` / `MuteTurn` / `Rescued`). (b) The
renderer returns `Result` instead of `String`, so an unreadable Task **fails
naming what was inspected** (artifact and history counts with their roles,
`status.message` presence, plus `task_id` and `context_id` — the two handles that
find the turn in `$MIKA_SPIRIT_LOG_FILE`). The signature change, not a parallel
function, is what makes the compiler force both call sites.

**The two halves share one predicate, and that is load-bearing.**
`mika_a2a::render::render_task_text` is the single definition of "does this Task
carry text", used by the server's net and by the CLI's renderer. A server-side
approximation could call a Task fine while the client found nothing readable —
the exact gap the net exists to close. It also means the fourth line of the
decision table is not a detail: a turn that produced **no** text is served with
the same literal `message/stream` serves (`COMPLETED_WITHOUT_TEXT`), which
**removes the whole "completed but empty" class from this port** and is what lets
the CLI treat every empty Task as a loss with no false positive.

**The net does not hide the defect.** A silent repair would turn the loop green
and make the fault permanently invisible — i.e. build the next occurrence. So the
rescue path emits `a2a_send_task_content_lost` carrying only the facts that settle
the cause: `task_id`, `context_id`, `session_id`, the `trace_id` handed to the
loop (equal to `task_id` today, reported rather than assumed), and the two counts
from `Database::a2a_message_census` — rows for this session, rows for this trace —
which separate "nothing was persisted" from "persisted under another trace id"
without an operator opening the database.

**SOLE WRITER:** `a2a_send_task_content_lost`, in the log and in `audit_events`.
Its **absence** under a symptom is therefore information: it says the loss is not
here. Pinned by `server::a2a::tests::the_loss_signal_has_exactly_one_writer_in_this_module`,
a lexical guard — a second writer would make no decision wrong, only
unattributable, which no behavioural test can see.

**What this does NOT explain: why `a2a_get_messages` returns empty.** M3 reduces
it to one predicate on one column written by one path (the agent loop's
`trace_id`), but deciding between "nothing persisted" and "persisted under another
trace id" needs the production database. Follow-up ticket **conditioned on the
first occurrence** of the WARN, which carries exactly the two counts that
depart it. Opening it before that line exists would be instructing without a
measurement.

**Operator surfaces.** Grep `a2a_send_task_content_lost` in
`$MIKA_SPIRIT_LOG_FILE` — **empty in nominal operation**; continuous firing means
the net is masking a persistence failure that deserves its own fix, so treat the
cause, not the threshold. SQL:
`SELECT COUNT(*) FROM audit_events WHERE tool_name = 'a2a_send_task_content_lost';`
Two adjacent names, deliberately distinct: `a2a_send_task_census_unreadable`
(ERROR — the counts could not be read, the loss is still reported) and
`a2a_send_task_content_lost_audit_failed` (the WARN landed, the audit row did
not).

**Halt conditions.** The symptom returns and the WARN stays silent → the loss is
not where this fix places it: do **not** widen the net or add a tier to the
renderer, reopen the investigation on the client or transport side with the POST
trace. A nominal reply whose bytes change → the net is biting where it must not;
halt before deploying, since a return-channel fix that alters healthy answers is
worse than the silence it replaces.

### Introspection Tools

5 read-only tools: `query_timeline`, `get_session_messages`, `list_audit_events`, `search_tool_history` (30-day retention, 500-char field truncation, 10KB output cap), `query_knowledge_graph`. Non-orchestrator agents scoped to their own agent_id/sessions.
