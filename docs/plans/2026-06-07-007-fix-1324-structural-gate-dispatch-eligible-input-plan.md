# Plan — fix(engine): structural tool-filter for CI webhook events (mika#1324)

## Phase 0 — Pin

**A. dispatch_arg_match intent guard** (`crates/mika-agent/src/agent.rs:1838-1860`):
```rust
if !skip_remaining_guards
    && matches!(response.stop_reason, LlmStopReason::EndTurn)
    && !intent_guard_retries.contains("dispatch_arg_match")
    && ready_label_dispatch_trigger(&user_input_text)
    && let Some(ref expected_location) = expected_hash_n
    && let Some(mismatched) = all_tool_summaries.iter().find(|s| {
        (s.name == "run_claude_pilot" || s.name == "run_claude_pilot_groom")
            && !s.input_summary.contains(expected_location.as_str())
    })
```
This guard fires AFTER an LLM tool call. It's post-hoc detection, not prevention.

**B. ready_label_dispatch_trigger** (`agent.rs:5932`):
```rust
fn ready_label_dispatch_trigger(msg: &str) -> bool {
    crate::webhook_dispatch::is_ready_label_dispatch_marker(msg)
}
```
Only true when message starts with `READY_LABEL_DISPATCH_MARKER`. CI webhook text does NOT start with that marker.

**C. CI handler Passthrough → raw text reaches LLM** (`server/handlers.rs:794-816`):
```rust
let ci_action = ci_success_handler::try_handle_ci_success(&req.text, ...).await;
match ci_action {
    VerdictAction::Handled { pre_digest } => { req.text = pre_digest; }
    VerdictAction::Passthrough { enrichment: Some(e) } => { req.text = format!("{e}{}", req.text); }
    VerdictAction::Passthrough { enrichment: None } => {}  // ← raw text → LLM
}
```
When the CI handler self-selects but conditions aren't met (no GH token, no open PR, no QA verdict, stale SHA), it returns `Passthrough { enrichment: None }` and the raw `check_suite.completed(...)` text reaches the LLM unmodified.

**D. parse_check_suite_success identifies CI events** (`server/ci_success_handler.rs`):
```rust
let event = match parse_check_suite_success(text) {
    Some(e) => e,
    None => return VerdictAction::Passthrough { enrichment: None },
};
```
The parser is deterministic — given any `text`, it returns `Some(event)` iff `text` matches the check_suite format. This is the discrimination point for "is this a CI event."

**E. Tool-filter infrastructure** (`agent.rs:4977`):
```rust
fn filter_available_required_tools(required_tools, tools, skill_tool_map, mcp_manager)
```
Existing mechanism for filtering tools per-turn. The fix extends this layer.

## Hypothesis (committed)

**The LLM (mika-dev) receives raw `check_suite.completed(...)` webhook text via the Passthrough { enrichment: None } path. It infers from the branch name substring "dev-groom" that it should call `run_claude_pilot_groom` on the underlying issue. The `dispatch_arg_match` intent guard's audit-event-log entry refers to the LLM's tool-call attempt being caught at the EndTurn boundary — but as Pin A shows, the guard's `ready_label_dispatch_trigger` precondition would NOT fire on CI text (it requires marker-prefix). So the audit_events note `dispatch_arg_match guard matching branch name substring` is the ticket-author's mis-description of the actual mechanism — the LLM did the substring inference, not the guard's deterministic code.**

This hypothesis fits the observed behavior:
- 4 dispatches attempted (LLM tool calls)
- All rejected (orchestrator-Claude judgment, not deterministic guard)
- Audit-event entry stored as a `store_fact` note FROM mika-dev — i.e., mika-dev's own LLM-generated retrospective, which may have mis-attributed root cause

## Fix shape (committed)

**Filter the LLM's available tools when the input is a CI webhook event.** Structurally remove `run_claude_pilot` and `run_claude_pilot_groom` from the per-request tool set when `req.text` matches `parse_check_suite_success(text).is_some() || parse_check_suite_failure(text).is_some()`.

This is structural because:
1. The LLM cannot call a tool that's not in its tool list — no prompt enforcement involved
2. The discrimination is deterministic (parser, not LLM)
3. The filter applies at the request-assembly layer, before LLM invocation

### Implementation site

Two viable locations; choose the narrower:

**Option 1 (narrower):** At the request-assembly layer in `agent.rs` around where `tools` is wired into `request`. Add a CI-detection branch that filters dispatch tools out.

**Option 2 (broader):** At the webhook-handler layer in `handlers.rs` after CI handlers run. Set a per-request flag (`req.is_ci_webhook = true`) that the agent loop reads when assembling tools.

Going with **Option 1**. Rationale: narrower scope; no new request-level fields; tool-filter is already an existing pattern in `agent.rs`.

### Specific code

In `crates/mika-agent/src/agent.rs`, near where `tools` is assigned into `request.tools` (probably around line 850-1000):

```rust
// mika#1324: CI webhook events MUST NOT trigger dispatch.
// Filter out dispatch tools when input is a check_suite.completed event.
// Structural prevention — the LLM cannot call a tool that isn't in its list.
const CI_EXCLUDED_TOOLS: &[&str] = &["run_claude_pilot", "run_claude_pilot_groom"];
let is_ci_webhook =
    crate::server::ci_success_handler::parse_check_suite_success(user_input_text).is_some()
    || crate::server::ci_failure_handler::parse_check_suite_failure(user_input_text).is_some();
if is_ci_webhook {
    tools.retain(|t| !CI_EXCLUDED_TOOLS.contains(&t.name.as_str()));
}
```

### Why not gate at `try_handle_ci_*`

Both handlers consume `text` (read-only) and can't mutate the eventual tool set. Cleaner to make the agent-layer (which already owns tool-filtering) read the CI-detection from the parsers.

### Why not remove the intent guard

The `dispatch_arg_match` intent guard (Pin A) stays as defense-in-depth. The structural filter prevents the LLM from making the wrong tool call; the guard catches any case where structural filtering doesn't apply (e.g., new dispatch-trigger paths added later that don't yet have the CI exclusion).

## Acceptance Criteria (concrete)

1. **AC1:** When `req.text` matches the check_suite.completed pattern (success OR failure variant), the per-request tool set excludes `run_claude_pilot` and `run_claude_pilot_groom`. Verified via unit test on the agent-loop's tool-assembly path.

2. **AC2:** Integration regression test in `crates/mika-agent/tests/`:
   - Simulate a `check_suite.completed(success)` webhook on branch `fix/1318/dev-groom-no-force-push` (matches the original incident)
   - Inject the resulting `req.text` into the agent loop
   - Assert the resulting `request.tools` (sent to LLM) does NOT contain `run_claude_pilot` or `run_claude_pilot_groom`

3. **AC3:** Existing ready-label dispatch flow continues to work:
   - Simulate `issues.labeled (ready)` event
   - Assert `request.tools` DOES contain `run_claude_pilot` and `run_claude_pilot_groom` (when otherwise available to the agent)

4. **AC4:** Existing `dispatch_arg_match` intent guard stays in place as defense-in-depth (no removal). Verified by reading agent.rs:1838-1860 unchanged.

5. **AC5:** `cargo test -p mika-agent --lib` + `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean.

## Files to change

- `crates/mika-agent/src/agent.rs` — add CI-detection branch in tool-assembly path (~line 850-1000)
- `crates/mika-agent/src/server/ci_success_handler.rs` — ensure `parse_check_suite_success` is `pub` so agent.rs can call it
- `crates/mika-agent/src/server/ci_failure_handler.rs` — ensure `parse_check_suite_failure` is `pub`
- `crates/mika-agent/tests/` — new integration test for AC2 + AC3

## Out of scope

- Restructuring webhook event routing entirely
- Removing the `dispatch_arg_match` intent guard (it's defense-in-depth)
- Adding event-type filtering for pull_request_review or other non-CI event types (separate concern; this ticket scopes to check_suite per the incident)

## Risk

Medium. The filter touches the agent loop's tool-assembly. Risks:
- Over-filtering: if the discrimination misfires, legitimate ready-label dispatches could lose tools. Mitigated by AC3 (positive regression test) and by using the SAME deterministic parsers the CI handlers use (single source of truth for "is this a CI event").
- Under-filtering: if a CI event reaches the LLM through a path that doesn't go through the standard `req.text` (e.g., a future handler that injects raw webhook text differently), the filter wouldn't apply. Mitigated by the defense-in-depth `dispatch_arg_match` guard.

## Test plan

1. Unit: `is_ci_webhook(text)` returns true on real check_suite.completed text samples (success + failure conclusions).
2. Unit: tool-assembly filters dispatch tools when `is_ci_webhook` is true.
3. Integration: full agent-loop turn on CI webhook input — verify zero dispatch tool calls.
4. Regression: same agent-loop on ready-label dispatch input — verify dispatch tools available.

## Implementation order

1. Promote `parse_check_suite_success` and `parse_check_suite_failure` to `pub`.
2. Add CI-detection branch in agent.rs tool-assembly path with the `CI_EXCLUDED_TOOLS` const.
3. Unit tests for the filter.
4. Integration test for AC2.
5. Regression test for AC3.
