# Plan: Post-condition guard — send_message to user forces EndTurn; forbid writes after

**Ticket:** mika issue#771
**Type:** feat
**Branch:** `feat/771/agent-post-condition-guard-send-message`

## Problem Summary

An agent can emit a user-facing `send_message` containing a question/choice, then answer its own question by executing state-changing tools in the same turn — without waiting for user input. The agent skips the authorization path. This is a structural bug: prompt-level enforcement is insufficient (system prompt is at capacity) and the failure class is agent-agnostic.

Concrete incident: mika-dev sent "Next up: deploy OR dispatch #744?" via `send_message`, then 4 seconds later dispatched `run_claude_pilot` for #744 unilaterally — $13.78 spent on unauthorized work.

## Proposed Architecture

Two orthogonal changes that compose:

1. **Tool read/write classification** — static data structure classifying every builtin tool
2. **Send-message turn boundary guard** — two enforcement points plus a PostConditionGuard registry extraction

### Key Design Decision: Two Enforcement Points

The send_message boundary cannot be a pure EndTurn post-condition like the completion-claim guard. By EndTurn, tool calls have already executed — you can't un-run `run_claude_pilot`. The guard needs **proactive gating**:

- **Intra-step gate** (in `process_tool_calls`): Within a single LLM response containing multiple tool_use blocks, skip write tools that appear after `send_message`.
- **Inter-step gate** (in agent loop): After a step that included `send_message`, force EndTurn on the next iteration — don't make another LLM call.

The PostConditionGuard registry holds the observability/logging side and the completion-claim migration. The enforcement itself is upstream of EndTurn.

## Implementation Steps

### Step 1: Tool read/write classification module

**File:** `crates/mika-agent/src/tools/classification.rs` (new)
**Wire:** `mod classification;` in `crates/mika-agent/src/tools/mod.rs`

Create a static classification of all builtin tools. The classification is a function `is_write_tool(name: &str) -> bool` backed by a `phf` or `match` on tool names.

**Write tools** (state-changing — forbidden after send_message):
- `create_task`, `update_task_status`, `cancel_task`, `complete_task`
- `create_reminder`, `cancel_reminder`
- `pr_merge_with_gate`
- `store_fact`, `update_fact`, `update_core_memory`
- `create_skill`, `delete_skill`, `toggle_skill`, `update_skill`
- `set_config`
- `write_agent_file`
- `create_agent`, `create_team`, `delete_team`, `update_team`, `add_team_member`, `remove_team_member`
- `delegate_task`, `run_team`
- `send_message` (second call treated as write per ticket contract)
- `a2a_call` (may mutate remote state)
- `create_scheduled_task`

**Read tools** (allowed after send_message):
- `list_tasks`, `check_task`, `get_task`
- `list_reminders`, `list_scheduled_tasks`
- `search_memory`
- `list_skills`
- `get_config`
- `read_agent_file`, `list_agent_files`
- `list_agents`, `list_teams`, `get_team_status`, `get_team_history`
- `query_timeline`, `get_session_messages`, `list_audit_events`, `search_tool_history`
- `query_knowledge_graph`
- `resolve_issue_order`

**Exec handler tools** (skill-provided, dynamic names): Default to **write** unless the skill handler is known read-only. Skill exec tools include `run_gh` (mixed — read subcommands like `view`/`list`/`diff` are read, write subcommands like `pr merge`/`issue create` are write), `run_claude_pilot`, `run_claude_pilot_groom`, `deploy_mika`, `build_mika`, etc.

**`run_gh` special case:** `run_gh` is mixed read/write. The guard should classify it based on `input_summary` content:
- Write if input contains write-indicative subcommands: `pr merge`, `pr review`, `pr close`, `issue create`, `issue close`, `issue edit`, `label create`, `release create`, `api` (with non-GET)
- Read otherwise (view, list, diff, checks, etc.)

Provide `is_write_tool_call(summary: &ToolCallSummary) -> bool` that uses the name-based classification plus the `run_gh` input inspection.

**MCP tools** (`mcp__*` prefix): Default to **write** (conservative — MCP tools can do anything). This is safe because MCP tools are excluded from silent/heartbeat mode already and are uncommon in the send_message violation scenario.

**Note on AC10 (internal channels — issue body):** The issue body lists "internal channels (non-user-dialog) don't trigger the guard (if applicable — some send_message variants may be internal)" as a possible AC. Source check at `crates/mika-agent/src/tools/send_message.rs:28-37` confirms the tool's `input_schema` exposes only a `text` property — there is no `channel` parameter. All `send_message` invocations target the configured `ctx.message_sender` (Telegram or HTTP gateway) by design. **AC10 is N/A** at the tool layer: no internal-channel variant exists to scope around, no test is required. If a channel-parameter variant lands in a future change, the classification module is the single place to update.

**Weighing alternatives for `run_gh`:** The simpler alternative is **classify `run_gh` as write unconditionally**, eliminating the input-summary parsing entirely. The chosen approach (input-summary subcommand discrimination) trades ~30 lines of regex + classification logic for accurate read-vs-write distinction on the most-used skill exec tool. Reasons to keep input-summary parsing:

- Read-only `gh` subcommands (`view`, `list`, `diff`, `checks list`) are common in normal grooming/dispatch flows where the agent fetches context. Conservative-write here over-restricts useful context-fetch turns.
- The classification only matters POST-`send_message` in the same step. Pre-`send_message` reads are unaffected.
- Truncation fail-safe: when `input_summary` is truncated before the subcommand is parsable, the classification defaults to write (the safe direction). The "summary lies" failure mode degrades closed, not open.

Trade accepted explicitly — input-summary parsing is the chosen approach despite the complexity, because it preserves benign read-only `run_gh` calls. If empirical post-deploy data shows the read-only-after-send_message case never occurs in practice, simplify to write-unconditional in a follow-up.

**Tests:**
- Unit tests for every builtin tool name classification
- Unit test for `run_gh` read/write subcommand discrimination
- Unit test for unknown tool names defaulting to write
- Unit test for MCP tool names defaulting to write

### Step 2: Turn-level send_message tracking

**File:** `crates/mika-agent/src/agent.rs`

Add a turn-level flag alongside the existing turn state variables (near line ~890):

```rust
// Send-message turn boundary tracking (#771)
let mut send_message_boundary_active = false;
let mut suppressed_write_tools: Vec<String> = Vec::new();
```

The flag is set to `true` after any successful `send_message` tool call is recorded in `all_tool_summaries`. The `suppressed_write_tools` vec collects names of write tools that were skipped.

### Step 3: Intra-step write gating in tool processing

**File:** `crates/mika-agent/src/agent.rs` — in the tool_use processing block where individual tool calls from a single LLM response are executed

After each tool call is executed and its `ToolCallSummary` is built:
1. If the tool was `send_message` and succeeded, set `send_message_boundary_active = true`
2. Before executing any subsequent tool call in the same step, check:
   - If `send_message_boundary_active && is_write_tool_call(&pending_summary)`:
     - Do NOT execute the tool
     - Record a `ToolCallSummary` with `success: false` and output `"[mika-engine] Tool call suppressed: send_message turn boundary (#771). State-changing tools are not permitted after send_message in the same turn."`
     - Push tool name to `suppressed_write_tools`
     - Return the suppressed result as the `tool_result` to the LLM (so the conversation history stays paired)

**Key constraint:** The dedup guard (`#[582]`) runs before execution. The send_message boundary gate should run AFTER dedup but BEFORE execution, in the same tool-processing loop.

**Interaction with existing guards:**
- The per-turn tool_use dedup guard (#582) is unaffected — it runs on `(name, arguments)` pairs before execution
- The `ToolContext.pr_review_posted` guard is unaffected — it's PR-review-specific
- The dispatch-readiness guard (#525) is unaffected — it runs inside the long-running executor

### Step 4: Inter-step EndTurn forcing in agent loop

**File:** `crates/mika-agent/src/agent.rs` — at the top of the agent loop or after tool processing

After processing all tool calls in a step, before the next LLM call:

```rust
// Send-message turn boundary: force EndTurn after a step that included send_message (#771)
if send_message_boundary_active {
    if !suppressed_write_tools.is_empty() {
        warn!(
            step,
            agent_id = %agent_id,
            session_id = %session_id,
            suppressed_tool_calls = ?suppressed_write_tools,
            send_message_text = %truncate_for_log(&send_message_text_capture, 200),
            "send_message_turn_boundary_violation: suppressed write tools after send_message"
        );
    }
    // Force EndTurn — do not make another LLM call
    break;
}
```

This sits in the `ToolUse` arm of the stop_reason match, after `process_tool_calls` returns but before the `continue` that loops back for another LLM call.

**Interaction with LoopMode:** This applies to all loop modes (Conversation, Silent, Team). In Silent mode, `send_message` is the primary output mechanism, so the guard firing there means the agent's notification was delivered and the turn should end. In callback turns where `callback_terminal_action` requires BOTH `update_task_status` AND `send_message`, the guard allows both because `update_task_status` typically fires BEFORE `send_message` (the callback pattern is: process result → update status → notify operator). If `send_message` fires first and `update_task_status` is suppressed, the `callback_terminal_action` guard will reject EndTurn and re-prompt — the two guards compose correctly because the re-prompt turn starts fresh (send_message_boundary_active resets).

**Edge case — callback_terminal_action composition:** If the callback pattern is inverted (send_message before update_task_status), the send_message boundary would suppress `update_task_status`, and then `callback_terminal_action` would reject EndTurn. On retry, the agent should call `update_task_status` first, then `send_message`. If it repeats the same order, the callback guard's single-retry fires and accepts EndTurn (fail-open after one retry). This is acceptable — the callback result metadata is still persisted via `try_extract_callback_metadata` at the dispatcher level.

### Step 5: PostConditionGuard registry extraction

**File:** `crates/mika-agent/src/post_condition.rs` (new module)
**Wire:** `mod post_condition;` in `crates/mika-agent/src/lib.rs` or `agent.rs`

**Spec-reconciliation note (vs issue body).** The issue body proposed a `PostConditionGuard` struct with a `trigger: fn(&TurnContext) -> GuardDecision` function-pointer field, plus a `TruncateAndEndTurn` GuardDecision variant intended to cover the send_message boundary. This plan diverges intentionally on both points:

1. **Function-pointer → label-dispatch.** The completion-claim evaluator requires `async` + DB access (lazy-resolve of active tasks via `&db`). Stable Rust has no `async fn` pointers without trait objects or `Pin<Box<dyn Future>>`; the registry would have to either drop the async/DB capability or introduce trait objects with associated-type juggling. Label-dispatched `match guard.label` against a static const-fn registry keeps the registry minimal while preserving the async capability inline. The label remains the source of truth for retry-tracking and structured logging.

2. **`TruncateAndEndTurn` variant → upstream enforcement (Steps 3+4).** The send_message boundary is *structurally* different from completion-claim. By EndTurn, post-send_message tool calls have already executed — the guard cannot "truncate" them retroactively. Enforcement must be proactive: intra-step gating during tool processing (Step 3) and inter-step EndTurn forcing in the agent loop (Step 4), both *upstream* of the post-condition guard sequence. The registry holds only the observability/logging surface for this case, not the enforcement logic.

Both divergences are architecturally justified by Rust language constraints (1) and the run-loop's execution order (2); this note makes the divergence visible for future readers and for the issue-body / plan reconciliation audit.

Extract the completion-claim guard into a registry. The registry is a stepping stone — it holds guards that fire at EndTurn time with the reject-and-reprompt pattern. The send_message boundary guard is NOT in this registry (different enforcement point per the spec-reconciliation note above), but its logging/observability fires here.

```rust
/// Post-condition guard evaluated at EndTurn.
pub struct PostConditionGuard {
    /// Unique label for retry-tracking and structured logging.
    pub label: &'static str,
    /// Human-readable description for debug output.
    pub description: &'static str,
}

/// Decision from a post-condition guard evaluation.
pub enum GuardDecision {
    /// No violation detected.
    Pass,
    /// Reject EndTurn and re-prompt with a correction message.
    RejectEndTurn { correction: String },
}

/// Context available to post-condition guard evaluators.
pub struct PostConditionContext<'a> {
    pub text: &'a str,
    pub all_tool_summaries: &'a [ToolCallSummary],
    pub tools_called: &'a HashSet<String>,
    pub tools: &'a ToolRegistry,
    pub stop_reason: &'a LlmStopReason,
}
```

**Migration scope:** Only the completion-claim guard moves into this registry in this PR. The other 10 guards remain inline — their shapes are diverse enough that premature extraction would add complexity without benefit. The registry establishes the pattern; future guards can opt in incrementally.

The completion-claim guard's `detect_completion_claim` function and its DB-dependent lazy-resolve of active tasks mean the evaluator function is `async`. The registry calls each guard in sequence (not parallel — guards are cheap).

```rust
pub const POST_CONDITION_GUARDS: &[PostConditionGuard] = &[
    PostConditionGuard {
        label: "completion_claim",
        description: "Reject EndTurn when agent claims completion without update_task_status",
    },
];
```

The actual evaluation logic stays in `agent.rs` as a match on `guard.label` (avoiding the need for async fn pointers or trait objects with async). The registry provides the label, description, and retry-tracking key. The guard logic dispatches on label:

```rust
for guard in POST_CONDITION_GUARDS {
    if post_condition_retries.contains(guard.label) {
        continue;
    }
    let decision = match guard.label {
        "completion_claim" => evaluate_completion_claim(&ctx, &db).await,
        _ => GuardDecision::Pass,
    };
    match decision {
        GuardDecision::Pass => {}
        GuardDecision::RejectEndTurn { correction } => {
            post_condition_retries.insert(guard.label);
            // inject correction, continue 'agent_loop
        }
    }
}
```

This approach keeps the async DB access pattern while providing registry-driven label management and retry tracking. The `completion_claim_retry_done` bool flag is replaced by the `post_condition_retries: HashSet<&'static str>`.

### Step 6: Structured logging and observability

**WARN event on violation** (emitted at Step 4's break point):

| Field | Type | Description |
|-------|------|-------------|
| `event` | `&str` | `"send_message_turn_boundary_violation"` |
| `session_id` | `&str` | Current session |
| `agent_id` | `&str` | Current agent |
| `step` | `u32` | Step number where violation detected |
| `suppressed_tool_calls` | `Vec<String>` | Names of write tools that were skipped |
| `send_message_text` | `String` | First 200 chars of the send_message content |
| `trace_id` | `&str` | Current trace |

**INFO event on clean boundary** (send_message called, no writes attempted after):

| Field | Type | Description |
|-------|------|-------------|
| `event` | `&str` | `"send_message_turn_boundary_enforced"` |
| `step` | `u32` | Step where EndTurn was forced |

No `audit_events` DB row — this is a per-turn engine decision, not a user-visible state change (per conventions C3.1).

### Step 7: Tests

All tests in `crates/mika-agent/tests/eval/` using `EvalHarness` + `MockLlmProvider`.

**Test 1: Read-only after send_message passes**
- Mock LLM returns: `send_message(text)` → `list_tasks()` → EndTurn
- Assert: both tools executed successfully, no suppression warning

**Test 2: Write after send_message truncated**
- Mock LLM returns: `send_message(text)` → `create_task(...)` → EndTurn
- Assert: `send_message` executed, `create_task` suppressed (success=false in summary), EndTurn forced, WARN logged

**Test 3: Dispatch after send_message truncated (incident case)**
- Mock LLM returns: `send_message(text)` → `run_claude_pilot(...)` in next step
- Assert: `send_message` executed, second LLM call never made (EndTurn forced after step 1), no `run_claude_pilot` in tool summaries

**Test 4: Two consecutive send_message calls — second treated as write**
- Mock LLM returns: `send_message("question")` → `send_message("more info")`
- Assert: first `send_message` executed, second suppressed

**Test 5: Completion-claim migration — existing behavior preserved**
- Same test shape as existing completion-claim tests
- Assert: guard fires when completion keyword detected without `update_task_status`
- Assert: guard uses registry label `"completion_claim"` for retry tracking

**Test 6: send_message in callback turn — composes with callback_terminal_action**
- Mock: callback trigger, LLM returns `update_task_status(completed)` → `send_message(text)` → EndTurn
- Assert: both tools executed (update_task_status is BEFORE send_message), EndTurn accepted
- This validates the correct ordering: status update first, then notification

**Test 7: Write tools before send_message are unaffected**
- Mock LLM returns: `create_task(...)` → `send_message(text)` → EndTurn
- Assert: `create_task` executed successfully, `send_message` executed, EndTurn forced (clean — no suppression)

### Step 8: Classification test coverage

**File:** `crates/mika-agent/src/tools/classification.rs`

- Exhaustive test: iterate `default_tools()` registry and assert every tool has an explicit classification (no silent defaults — catch new tools that weren't classified)
- Test: `run_gh` with `"gh pr view 123"` → read
- Test: `run_gh` with `"gh pr merge 123"` → write
- Test: `run_gh` with `"gh issue create"` → write
- Test: unknown/MCP tools → write (conservative default)

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/tools/classification.rs` | **New** — tool read/write classification |
| `crates/mika-agent/src/tools/mod.rs` | Add `pub mod classification;` |
| `crates/mika-agent/src/post_condition.rs` | **New** — PostConditionGuard types and registry |
| `crates/mika-agent/src/agent.rs` | Turn state vars, intra-step gating in tool processing, inter-step EndTurn forcing, completion-claim migration to registry dispatch |
| `crates/mika-agent/src/lib.rs` | Add `pub mod post_condition;` (if not inlined in agent.rs) |
| `tests/eval/` | 8 new test scenarios |

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Callback turns where send_message fires before update_task_status | callback_terminal_action guard catches the missing status update on EndTurn and re-prompts; the two guards compose (see Step 4 analysis) |
| New tools added without classification | Exhaustive test in Step 8 catches unclassified tools at test time; unknown tools default to write (fail-safe) |
| `run_gh` input parsing false positives | Conservative: if input parsing fails, classify as write |
| Completion-claim migration regression | Dedicated test (Test 5) verifies behavior preservation |
| Silent mode send_message suppression breaks notifications | Guard only suppresses writes AFTER send_message, not send_message itself; the notification is always delivered |

## Out of Scope

- Extracting all 11 post-condition guards into the registry (incremental — this PR establishes the pattern with completion-claim only)
- Retry/correction injection on send_message boundary violation (per ticket: dropping writes is intentional)
- Per-skill or per-agent opt-out of the guard (ticket specifies global scope)
- `run_gh` read-only restriction for post-send_message turns could use the existing `validate_gh_api_scope` pattern, but the `input_summary` approach is sufficient and simpler
