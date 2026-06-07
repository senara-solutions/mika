# Plan — fix(engine): structural gate for dispatch-eligible input (mika#1324)

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
{
    intent_guard_retries.insert("dispatch_arg_match");
    warn!(... "Dispatch-arg-fabrication guard fired — re-prompting (#1313)");
```

**B. ready_label_dispatch_trigger predicate** (`crates/mika-agent/src/agent.rs:5932`):
```rust
fn ready_label_dispatch_trigger(msg: &str) -> bool {
    crate::webhook_dispatch::is_ready_label_dispatch_marker(msg)
}
```

**C. is_ready_label_dispatch_marker** (`crates/mika-agent/src/webhook_dispatch.rs:50`):
```rust
pub(crate) fn is_ready_label_dispatch_marker(msg: &str) -> bool {
    msg.starts_with(READY_LABEL_DISPATCH_MARKER)
}
```

**D. Structural CI handlers** (`crates/mika-agent/src/server/handlers.rs:794, 818`):
- `ci_success_handler::try_handle_ci_success` — intercepts check_suite.completed(success)
- `ci_failure_handler::try_handle_ci_failure` — intercepts check_suite.completed(failure|timed_out)
- Both return `VerdictAction::Handled | Passthrough` — passthrough cases STILL reach the LLM via `req.text`.

## Investigation gap (named honestly)

The ticket body attributes the bug to `dispatch_arg_match guard matching branch name substring "dev-groom"`. **Code reading shows the guard does not do branch-name substring matching.** The guard compares `mismatched.input_summary.contains(expected_location)` where `expected_location` is the issue `#N` extracted from the user input — and that guard is gated on `ready_label_dispatch_trigger(msg)` which requires the message to start with `READY_LABEL_DISPATCH_MARKER`.

CI webhook events would NOT start with that marker, so the guard as documented should NOT fire. Yet the audit-event log shows it firing 4× on CI webhook events.

**Hypotheses to verify with architect:**

1. **LLM-internal inference (most likely):** The LLM receives CI webhook text containing `branch: fix/1318/dev-groom-no-force-push`, infers from "dev-groom" substring that it should call `run_claude_pilot_groom`, makes the fabricated tool call. The guard then catches the fabrication post-hoc. *In this case, the bug is "LLM treats CI webhook as dispatch trigger" — the guard works correctly, but the LLM shouldn't have attempted dispatch at all.*

2. **Webhook handler enriches CI text with ready-label-marker prefix:** Some code path prepends a marker that makes `ready_label_dispatch_trigger` return true on CI events. Unlikely given the prefix-match shape, but possible if a handler is over-eager.

3. **Different guard / code path entirely:** Maybe a separate intent guard or webhook dispatcher in another file is matching branch-name substrings. Needs broader investigation.

## Approach (assumes Hypothesis 1, subject to architect verification)

If LLM-internal inference: the structural fix is at the **input-classification** layer, NOT in `dispatch_arg_match` itself.

Add structural filter:
- CI webhook events (check_suite.*, pull_request_review.*) MUST never carry user_input shape that the LLM would interpret as a dispatch trigger.
- The LLM should receive CI webhook text with an explicit "do not dispatch" marker AND/OR with the branch name stripped/sanitized.

Simpler structural alternative:
- After `ci_success_handler` / `ci_failure_handler` Passthrough, prepend an instruction to `req.text` that says "[CI webhook — not a dispatch trigger. Do not call run_claude_pilot* tools.]"
- The LLM then has explicit instruction NOT to dispatch.

## Acceptance criteria (subject to architect calibration)

1. **AC1:** CI webhook events on a PR whose branch name contains `dev-groom` / `dev-pilot` / `groom` MUST NOT trigger LLM dispatches of `run_claude_pilot*` tools.
2. **AC2:** Audit log shows zero `dispatch_arg_match` guard fires for CI webhook events (because LLM doesn't attempt the dispatch in the first place).
3. **AC3:** Regression test: simulate a `check_suite.completed` webhook on a branch named `fix/<N>/dev-groom-anything-here` with no `ready` label — verify zero `run_claude_pilot*` tool calls made by the LLM.
4. **AC4:** Existing ready-label dispatch flow continues to work — `issues.labeled` with `ready` still triggers dispatch.

## Files

- `crates/mika-agent/src/server/handlers.rs` — likely site for structural CI-webhook instruction-prepend
- `crates/mika-agent/src/server/ci_success_handler.rs` / `ci_failure_handler.rs` — possibly the right home for the instruction
- `crates/mika-agent/src/agent.rs:1838-1860` — `dispatch_arg_match` guard stays as defense-in-depth

## Out of scope

- Restructuring webhook event routing entirely (separate concern)
- Removing `dispatch_arg_match` guard (it's defense-in-depth)
- Other intent guards (different surfaces)

## Risk

Medium. The fix touches webhook input flow that affects the autonomous loop's dispatch behavior. A too-aggressive structural filter could prevent legitimate dispatches. Architect canvass needed to:
- Verify Hypothesis 1 (the actual root cause)
- Calibrate the instruction-prepend approach vs alternatives (event-type sentinel, request-class enum)
- Identify additional code paths that might be involved

## Investigation tasks (architect deliverable)

1. Confirm Hypothesis 1 or rule it out via additional code reading.
2. Identify all webhook handlers that emit `req.text` reaching the agent loop.
3. Verify whether `ci_success_handler` / `ci_failure_handler` Passthrough cases are the relevant exposure surface.
4. Specify the exact AC2 audit-event shape.
5. Specify whether the instruction-prepend approach is sufficient or if a structural request-class enum is preferred.

## Test plan (subject to architect)

1. Unit test: simulate CI webhook text → verify LLM tool-call set excludes `run_claude_pilot*`.
2. Integration test: spawn agent with simulated webhook event, observe tool-calls.
3. Regression: existing ready-label dispatch still works.
