# Plan: Post-Condition Guard — send_message Turn Boundary (mika#771)

## Problem

An assistant turn can emit a user-facing `send_message` containing a question or choice, then answer its own question by executing state-changing tools in the same turn. The agent offered a choice and selected for the user unilaterally, bypassing the authorization path.

Concrete incident: mika-dev sent "Next up: deploy done → resume milestone#16 OR dispatch mika#744 now?", then 4 seconds later in the same turn dispatched `run_claude_pilot` for #744 without waiting for the user's answer. The dispatched pilot ran 47 min and spent $13.78.

Prompt-level enforcement cannot work: `system_prompt.md` is at 49,069 bytes with 83 bytes of headroom. The engine already has precedent for this class of guard (completion-claim guard #483, INTENT_GUARDS registry #702).

## Design

### Two-layer architecture

This feature has two distinct enforcement points:

1. **Mid-execution guard in `process_tool_calls`** — prevents state-changing tools from executing after a `send_message` to a user-dialog channel within a single LLM response
2. **Post-condition signal in the main loop** — forces EndTurn when the mid-execution guard fired, regardless of the LLM's `stop_reason`

### Read/Write tool classification

A const data structure classifying tools as state-changing (write) or read-only. This is the authoritative list checked by the guard.

```rust
/// Tools that are state-changing and forbidden after a user-dialog send_message
/// in the same turn. Any tool NOT in this list is implicitly read-only and allowed.
const STATE_CHANGING_TOOLS: &[&str] = &[
    "create_task",
    "update_task_status",
    "cancel_task",
    "complete_task",
    "pr_merge_with_gate",
    "run_claude_pilot",
    "run_claude_pilot_groom",
    "store_fact",
    "update_fact",
    "update_core_memory",
    "write_agent_file",
    "write_workspace",
    "create_agent",
    "create_team",
    "delete_team",
    "update_team",
    "add_team_member",
    "remove_team_member",
    "delegate_task",
    "run_team",
    "send_message",           // second send_message treated as write
    "create_scheduled_task",
    "deploy_mika",
];

/// run_gh is conditionally classified: write subcommands (pr merge, pr review,
/// issue create, issue close, issue edit, label create) are state-changing;
/// read subcommands (view, diff, list, search) are allowed.
fn is_run_gh_write(arguments: &serde_json::Value) -> bool { ... }
```

**Design choice — blocklist (write) not allowlist (read):** New tools default to "allowed after send_message" which is the safe direction. A new state-changing tool that's missing from the list will still be caught on the next review cycle. The alternative (allowlist of read-only tools) would silently block new legitimate read tools.

**`send_message` itself is in the write list.** A second `send_message` in the same turn after the first user-dialog one is treated as a write — either unnecessary (combine into one message) or a sign the agent is chaining statements where it should wait. The first `send_message` executes normally; any subsequent one is suppressed.

**`run_gh` gets special handling.** The tool accepts arbitrary `gh` subcommands, so a static classification is insufficient. The guard inspects the `arguments` JSON to detect write subcommands (`pr merge`, `pr review`, `pr close`, `issue create`, `issue close`, `issue edit`, `label create`, `api` with non-GET method). Read subcommands (`pr view`, `pr diff`, `pr list`, `issue view`, `issue list`, `run list`, `search`) are allowed.

### PostConditionGuard registry extraction

Extract the inline completion-claim guard (~line 1295) into a `PostConditionGuard` registry, co-located near `INTENT_GUARDS`. The existing 11-guard inline chain stays inline — only the completion-claim guard moves as proof-of-concept for the registry pattern. The send_message boundary guard is architecturally different (mid-execution, not post-execution) and does NOT go into this registry.

```rust
struct PostConditionGuard {
    label: &'static str,
    /// Check the completed turn. Returns the action to take.
    check: fn(&PostConditionContext) -> PostConditionAction,
}

struct PostConditionContext<'a> {
    text: &'a str,
    stop_reason: &'a LlmStopReason,
    tools_called: &'a HashSet<String>,
    all_tool_summaries: &'a [ToolCallSummary],
    // Additional fields as needed by specific guards
}

enum PostConditionAction {
    /// Guard passed — no action needed
    Pass,
    /// Reject EndTurn and re-prompt with correction message
    RejectEndTurn { correction: String },
}
```

**Why not put send_message guard in this registry:** The send_message guard must fire DURING tool execution (to prevent tools from running), not after. The `PostConditionGuard` pattern inspects the completed turn and re-prompts or accepts. The send_message guard suppresses tools before they execute and forces EndTurn — a fundamentally different lifecycle position.

### Mid-execution guard in process_tool_calls

Modify `process_tool_calls` to:

1. **Pre-scan** the `response_content` for `send_message` tool calls before executing any tools
2. If `send_message` is found, note its index
3. During execution, after `send_message` has been executed:
   - Check each subsequent tool against `STATE_CHANGING_TOOLS` (and `is_run_gh_write` for `run_gh`)
   - If state-changing: skip execution, emit a `tool_result` with `is_error: true` and content "Tool call suppressed: send_message turn boundary guard. State-changing tools cannot execute after a user-facing send_message in the same turn."
   - If read-only: execute normally
4. Return a `ProcessToolCallsResult` struct (not just `Vec<ToolCallSummary>`) that includes a `send_message_boundary_triggered: bool` flag and the list of suppressed tool names

**Channel discrimination:** The guard fires for ALL `send_message` calls, not just user-dialog ones. Rationale: the `send_message` tool doesn't have a channel parameter — it always targets the configured message sender (Telegram user channel in server mode). NoChannel sessions (chat_id=0, e.g. GitHub webhook origins) return success with redirect guidance but the message is still "sent" from the LLM's perspective. The guard fires on tool name alone.

**Internal channels are out of scope.** Agent-to-agent communication goes through `delegate_task` and `run_team`, not `send_message`. No channel filtering needed.

### Main loop integration

After `process_tool_calls` returns, check the `send_message_boundary_triggered` flag:

```rust
if result.send_message_boundary_triggered {
    warn!(
        step,
        label = mode.label(),
        suppressed_tools = ?result.suppressed_tool_names,
        "send_message_turn_boundary_violation — forcing EndTurn"
    );
    // Don't push tool results to request.messages (they're already added 
    // by process_tool_calls). Just break the loop to force EndTurn.
    break;
}
```

This is simpler than the existing post-condition pattern (no re-prompt, no correction injection, no retry flag). The suppressed tools never executed, so there's nothing to undo. The agent's `send_message` was delivered, and the turn ends cleanly.

### Retry semantics

**No retry.** Unlike other guards that re-prompt the model once, this guard silently drops the suppressed tools and forces EndTurn. Rationale from the ticket: "the agent's intent in a post-send_message write is usually 'I'll do this while waiting for the answer' which is exactly the bug. Re-prompting invites the same behavior."

The `send_message` was already delivered. The user's reply (next turn) either authorizes the suppressed action or doesn't.

## Implementation Steps

### Step 1: Define STATE_CHANGING_TOOLS classification

**File:** `crates/mika-agent/src/agent.rs` (near existing `PERSISTENCE_WRITE_TOOLS` at line ~5405)

- Add `STATE_CHANGING_TOOLS` const slice
- Add `is_run_gh_write(arguments: &Value) -> bool` helper
- Add `is_state_changing_tool(name: &str, arguments: &Value) -> bool` combining both

**Tests:**
- Unit test for `is_run_gh_write` with write subcommands → true
- Unit test for `is_run_gh_write` with read subcommands → false
- Unit test for `is_state_changing_tool` with each category

### Step 2: Extract PostConditionGuard registry

**File:** `crates/mika-agent/src/agent.rs`

- Define `PostConditionGuard`, `PostConditionContext`, `PostConditionAction` types
- Extract the completion-claim guard logic (lines ~1295–1362) into a `completion_claim_check` function
- Wire the registry into the main loop at the same position where the inline guard currently fires
- Preserve the existing `completion_claim_retry_done` boolean for single-retry semantics

**Tests:**
- Existing completion-claim tests in `tests/eval/grounding_regressions/` must still pass
- The behavior must be identical — this is a pure refactor

### Step 3: Implement send_message mid-execution guard in process_tool_calls

**File:** `crates/mika-agent/src/agent.rs` (in `process_tool_calls`)

- Change return type from `Vec<ToolCallSummary>` to `ProcessToolCallsResult`:
  ```rust
  struct ProcessToolCallsResult {
      summaries: Vec<ToolCallSummary>,
      send_message_boundary_triggered: bool,
      suppressed_tool_names: Vec<String>,
  }
  ```
- Pre-scan for `send_message` in the tool call list
- During execution, after `send_message` executes:
  - Check each subsequent tool with `is_state_changing_tool`
  - Suppress state-changing tools (emit error tool_result, push to suppressed list)
  - Allow read-only tools to execute
- Set `send_message_boundary_triggered = true` if any tools were suppressed

### Step 4: Wire the guard into the main loop

**File:** `crates/mika-agent/src/agent.rs` (main `run_loop` function)

- Update all `process_tool_calls` call sites to handle `ProcessToolCallsResult`
- After `process_tool_calls` returns with `send_message_boundary_triggered == true`:
  - Emit structured WARN log with `event=send_message_turn_boundary_violation`, `session_id`, `agent_id`, `suppressed_tool_calls`, `send_message_text` (first 200 chars from the send_message input)
  - Break the loop (force EndTurn)
- The suppressed tool calls' `ToolCallSummary` entries should have `success: false` so they don't count toward `tools_called`

### Step 5: Add eval harness tests

**File:** `crates/mika-agent/tests/eval/grounding_regressions/`

Add 6 new scenarios:

1. **send_message_then_read_only** — `send_message` followed by `list_tasks` → both execute, no suppression, turn continues normally (read-only post-message is fine)
2. **send_message_then_create_task** — `send_message` followed by `create_task` → `create_task` suppressed, EndTurn forced, WARN logged
3. **send_message_then_run_claude_pilot** — `send_message` followed by `run_claude_pilot` → `run_claude_pilot` suppressed (the incident case)
4. **double_send_message** — two consecutive `send_message` calls → second suppressed as write
5. **internal_channel_no_trigger** — `send_message` on a NoChannel session → guard DOES fire (channel discrimination is not at this layer; the guard fires on tool name)
6. **completion_claim_migration** — existing completion-claim behavior preserved after registry extraction

**File:** `crates/mika-agent/src/agent.rs` (unit tests)

- `test_state_changing_tools_classification` — verify each tool in the const is correctly classified
- `test_run_gh_write_detection` — verify write subcommand detection

### Step 6: Update CLAUDE.md documentation

**File:** `crates/mika-agent/CLAUDE.md`

- Add guard #10 (send_message turn boundary) to the Post-Conditions section
- Document `PostConditionGuard` registry
- Document `STATE_CHANGING_TOOLS` classification
- Note that the guard count is now 12 (11 existing + 1 new)

## Scope

### In scope
- `PostConditionGuard` registry with completion-claim as first entry
- `send_message_turn_boundary` mid-execution guard
- Explicit read/write tool classification (`STATE_CHANGING_TOOLS`)
- Guard applies globally (all agents)
- Eval tests for the new guard and migration of completion-claim

### Out of scope
- Model-level fix (prompt tightening) — prompt is full
- Retry/correction injection on trigger — dropping writes is intentional
- Hook-after-task-completion for auto-store_fact (separate ticket)
- Migrating other inline guards into the PostConditionGuard registry (future work)
- Channel-level discrimination (user-dialog vs internal) — `send_message` always targets user channel; no filtering needed

## Risks

1. **False positives on read-only tools:** A tool classified as read-only might have write side effects we missed. Mitigation: blocklist approach means new tools default to allowed; we can add them incrementally.
2. **`run_gh` write detection edge cases:** The `arguments` JSON parsing for `run_gh` may miss some write subcommand patterns. Mitigation: conservative — if the subcommand isn't recognizable, treat as write (fail-closed for `run_gh` specifically).
3. **Breaking existing flows that legitimately chain send_message + writes:** Some callback flows might send a message and then update task status. Mitigation: callback turns use `SilentTrigger::Callback` which runs `run_silent_agent` — the guard should fire in all modes (conversation, silent, team). If this breaks legitimate flows, we can add a per-mode gate. The ticket explicitly says the guard is global, and the failure class is agent-agnostic.
4. **process_tool_calls return type change:** Changing the return type from `Vec<ToolCallSummary>` to a struct requires updating all call sites. There are at least 3 (conversation, silent, team). Mechanical but high blast radius.

## Testing Strategy

- **Unit tests:** Tool classification (`STATE_CHANGING_TOOLS`, `is_run_gh_write`)
- **Eval harness tests:** 6 new grounding regression scenarios using `MockLlmProvider` with pre-programmed tool call sequences
- **Existing test preservation:** All existing completion-claim tests must pass unchanged after registry extraction
- **CI:** Existing `cargo test` covers all of the above
