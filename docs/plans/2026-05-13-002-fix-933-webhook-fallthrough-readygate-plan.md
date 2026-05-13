---
ticket: mika#933
type: fix
status: GROOMED (pending)
date: 2026-05-13
branch: fix/933/self-dev-webhook-fallthrough-does-not
related:
  - mika#583  # prior fallthrough incident (2026-04-15)
  - mika#841  # ready-label positive-consent contract
  - mika#847  # webhook_ready_label_dispatch guard
  - mika#910  # webhook_no_unauthorized_dispatch guard (post-hoc EndTurn)
  - mika#932  # the dispatched-without-ready-label incident
  - mika-platform#74  # companion: closing-comment template fix
---

# fix(self-dev): webhook fallthrough must reject dispatch at the tool boundary, not post-hoc

## Problem

`webhook_no_unauthorized_dispatch` (mika#910, agent.rs:4743) is an **intent-precondition guard** that fires at `EndTurn`. It rejects the assistant's response and re-prompts when `[GitHub]`-prefixed webhook turns that are NOT `[GitHub] Issue labeled ready on` successfully called `run_claude_pilot`. The rejection is post-hoc:

```text
[GitHub] webhook arrives → LLM responds with create_task + run_claude_pilot → 
  → executor spawns claude-pilot subprocess → LLM EndTurns → 
  → INTENT_GUARDS evaluates → #910 fires → correction sent for NEXT turn
```

At the point #910 fires, the dispatch has **already executed**: the callback task exists, the subprocess is running, claude-pilot is doing real work. The correction is too late — it teaches the model for the next turn but does not undo the side effects.

The live incident at mika#932 (2026-05-02T10:55:47) reproduces this gap. mika#910 shipped 2026-04-30 (commit `01f89474`) and was deployed when the incident occurred. The guard fired (recorded in tool-call traces) but did not prevent the dispatch.

The companion prompt-side rule at `skills/bundled/self-dev/system_prompt.md` lines 309–322 (Webhook Fallthrough section) and Rule 9 (lines 497–503) is prose-only — keyword-rich operator comments (e.g., `/mika-groom-ticket` Phase 5 step 20 closing text) defeat the prose rule by activating the model's dispatch heuristic before it reaches the SCOPE RULE.

## Goal

Convert the webhook → no-dispatch rule from a **post-hoc EndTurn correction** into a **tool-boundary block** that rejects `run_claude_pilot` synchronously, before the subprocess spawns or the callback task is created. The prompt-side rule becomes belt-and-suspenders (observable in tool-call logs) rather than load-bearing.

## Non-goals

- Changing mika#841's ready-label contract or its label-vs-event semantics.
- Touching `self-dev-webhook-qa` / `self-dev-webhook-ci` dispatch paths — those are keyword-matched handler skills with their own entry points and are out of scope.
- Operator-facing closing-comment template — that's mika-platform#74.
- Replacing the post-hoc #910 INTENT_GUARD — it stays as a redundant safety net. Removing it would weaken defense in depth without justification.

## Approach

Two complementary tactics, both shipped in this PR:

1. **Engine-side tool-boundary gate** (load-bearing). Add a webhook-source check to `validate_dispatch_readiness` in `crates/mika-agent/src/skills/executor.rs`. The check rejects with `error: "unauthorized_webhook_dispatch"` when the originating user message starts with `[GitHub]` and does NOT start with the `READY_LABEL_DISPATCH_MARKER` prefix. This requires plumbing the originating user-message text into `LongRunningContext`.

2. **Prompt-side hardening** (belt-and-suspenders). Replace the prose-only SCOPE RULE in `skills/bundled/self-dev/system_prompt.md` Webhook Fallthrough section with a **structural pre-call check**: before any `run_claude_pilot` on a webhook turn, the agent must call `run_gh issue view <n> --json labels --repo <repo>` and confirm `ready` is in the label list. Falsifiable in tool-call logs.

The engine-side gate is the durable structural fix. The prompt-side change closes the gap on any future event class the engine prefix-check might miss (e.g., a hypothetical new gateway-formatted prefix that isn't `[GitHub]`).

## Design — engine-side gate

### Where the check lives

`crates/mika-agent/src/skills/executor.rs::validate_dispatch_readiness` (line 775) is the existing tool-boundary gate for `run_claude_pilot`. It runs **before** subprocess spawn (line 1210 in `execute_long_running`) and returns a structured JSON error that propagates as the tool's error output.

The new check is the **cheapest** of all dispatch-readiness checks (pure string-prefix match, no DB read, no API call). It runs **first**, before `validate_task` / status / blocked-by / global-dispatch-slot.

### Plumbing originating message into `LongRunningContext`

`LongRunningContext` (`crates/mika-agent/src/skills/executor.rs:93`) currently carries `db`, `agent_name`, `session_id`, `trace_id`, `dispatch_count`. Add one field:

```rust
pub struct LongRunningContext {
    // ... existing fields ...
    /// Originating user-message text for this turn, when available.
    ///
    /// Populated in conversation-mode turns (the actual user/webhook input).
    /// `None` for silent triggers (`SilentTrigger::DeferredDispatch`, callback
    /// continuation turns) where there is no fresh user input — those paths
    /// have already passed an upstream gate.
    pub originating_message: Option<String>,
}
```

Construction sites in `crates/mika-agent/src/agent.rs`:

- **Conversation mode (~line 2353)**: populate from the latest user-role message in `request.messages` using the same extraction logic that already produces `user_input_text` at line 841 (which today runs *inside* `run_loop`). We extract it once at the construction site and pass the same string into both `LongRunningContext` and (via the existing `user_input_text` definition inside `run_loop`) the INTENT_GUARDS path. To avoid duplication, hoist the extraction into a small helper `latest_user_text(&request.messages) -> String` in `agent.rs`.

- **Silent `DeferredDispatch` mode (~line 3431)**: keep `originating_message: None`. Deferred-dispatch retries are engine-initiated and have no `[GitHub]`-prefixed user turn. The agent.rs `deferred_dispatch_trigger` already gates them; the executor doesn't need a second check.

- **Test sites (`make_lr_ctx` at line 2733 and call sites in `agent.rs:2357` / `agent.rs:3431`)**: default to `None` in `make_lr_ctx`. Add a `with_originating_message` builder helper to keep test ergonomics light. Existing tests that don't care about the new field keep working unchanged.

### Predicate sharing between agent.rs and executor.rs

Both modules need:
- `READY_LABEL_DISPATCH_MARKER` (currently `agent.rs:4694`)
- A predicate that decides "is this an unauthorized webhook dispatch turn?"

Refactor: extract a new module `crates/mika-agent/src/webhook_dispatch.rs` exposing:

```rust
/// Marker prefix emitted by `mika_gateway::github::format_event_text` for
/// `issues.labeled` events where the label name is `ready`.  See
/// `crates/mika-gateway/src/github.rs::format_event_text` and mika#842.
pub(crate) const READY_LABEL_DISPATCH_MARKER: &str = "[GitHub] Issue labeled ready on ";

/// True when the message is a `[GitHub]` webhook event that is NOT a
/// ready-label dispatch — i.e., a turn that MUST NOT call `run_claude_pilot`.
/// Mutually exclusive with `is_ready_label_dispatch_marker` on the
/// `[GitHub]` domain (mika#910).
pub(crate) fn is_unauthorized_webhook_dispatch(msg: &str) -> bool {
    msg.starts_with("[GitHub]") && !msg.starts_with(READY_LABEL_DISPATCH_MARKER)
}

pub(crate) fn is_ready_label_dispatch_marker(msg: &str) -> bool {
    msg.starts_with(READY_LABEL_DISPATCH_MARKER)
}
```

`agent.rs::webhook_no_unauthorized_dispatch_trigger` and `ready_label_dispatch_trigger` become thin wrappers around these. `agent.rs::READY_LABEL_DISPATCH_MARKER` is removed in favor of the re-export. **No behavior change** in the post-hoc INTENT_GUARD — only the source location moves.

### The tool-boundary check

In `validate_dispatch_readiness`, BEFORE `validate_task`'s re-fetch:

```rust
if let Some(msg) = ctx_originating_message
    && crate::webhook_dispatch::is_unauthorized_webhook_dispatch(msg)
{
    return Err(serde_json::json!({
        "error": "unauthorized_webhook_dispatch",
        "task_id": task_id,
        "reason": "This turn was initiated by a [GitHub] webhook event that is \
                   NOT the ready-label dispatch marker. Only `[GitHub] Issue \
                   labeled ready on` webhooks may dispatch claude-pilot (mika#841 \
                   positive-consent contract). All other webhook events (issue \
                   comments, label changes other than `ready`, PR comments) must \
                   use Webhook Fallthrough: acknowledge without dispatching."
    })
    .to_string());
}
```

`ctx_originating_message` threads in as a new parameter to `validate_dispatch_readiness` so the test sites can pass `None` and the production sites pass `ctx.originating_message.as_deref()`.

Function signature change:

```rust
async fn validate_dispatch_readiness(
    db: &AsyncDatabase,
    task_id: &str,
    github_token: Option<&str>,
    tool_input: Option<&serde_json::Value>,
    originating_message: Option<&str>,  // NEW
) -> Result<String, String>
```

All three call sites updated:
- `execute_long_running` (line 1210) — passes `ctx.originating_message.as_deref()`.
- `test_dispatch_guard_*` (lines 3783, 3799, 3815) — pass `None` since they test unrelated guards.

### Order of checks in `validate_dispatch_readiness`

1. **unauthorized_webhook_dispatch** (new — cheapest, no DB)
2. task re-fetch + task_not_found
3. task_not_dispatchable (status check)
4. task_active_dispatch (child callback check)
5. global_dispatch_active (cross-task slot check)
6. dispatch_blocked_by (GitHub API call — most expensive)

This order is preserved because the new check is pure string match. The decision was deliberate during #910's design — see the comment in agent.rs:4730 — and we extend the same ordering rationale.

### Error code naming

`unauthorized_webhook_dispatch` (not `webhook_no_unauthorized_dispatch`) — the executor-side errors use noun-phrase tense; the agent.rs guards use predicate-phrase tense. Matches the existing pattern (`task_not_dispatchable`, `global_dispatch_active`).

## Design — prompt-side hardening

### Section to edit

`skills/bundled/self-dev/system_prompt.md` — the **SCOPE RULE** callout in the Webhook Fallthrough section (currently lines 311–322) plus Rule 9 (lines 497–503).

### New prose

Replace the prose-only "Do NOT call run_claude_pilot" line with a **structural check** the model must execute:

```
> **SCOPE RULE (HARD GATE)** — This turn handles ONLY the webhook event. The
> permitted-actions list below is exhaustive; nothing else is allowed.
>
> **Pre-dispatch label gate** — If the message references an issue and you
> believe `run_claude_pilot` is appropriate, you MUST first run
> `run_gh("issue view <n> --json labels", repo="senara-solutions/<repo>")` and
> verify `ready` appears in the returned label list. If `ready` is absent,
> abort the dispatch and post `send_message` to Vincent describing the event.
> The engine enforces the same gate at the tool boundary
> (`unauthorized_webhook_dispatch`); the label query is your evidence that
> you considered the gate before acting.
>
> Permitted actions:
> 1. Acknowledge the event
> 2. If the event correlates to an existing active task ... [unchanged]
> 3. If the event requires Vincent's attention ... [unchanged]
> 4. Stop — do NOT proceed to the generic Workflow section above
```

Rule 9 (line 497) gets a single new sentence at the end:

```
**Engine backstop:** The engine rejects `run_claude_pilot` at the tool
boundary with `error: unauthorized_webhook_dispatch` when this rule is
violated (mika#933). Treat this as a hard contract, not a soft preference.
```

The "incident" footnote gets one new line appended:

```
**Incident:** mika#583 ... [unchanged]
mika#932 on 2026-05-02 — `issue_comment.created` webhook with dispatch-class
keywords in body bypassed the prose rule; mika#910 post-hoc EndTurn guard
fired but could not undo the dispatch.
```

## Tests

### Unit tests (executor.rs)

Append three tests to the existing `dispatch_guard` block in `crates/mika-agent/src/skills/executor.rs`:

1. `test_dispatch_guard_rejects_unauthorized_webhook` — `originating_message = Some("[GitHub] Comment on senara-solutions/mika#933: ...")`, expect `is_error: true` and parsed `error == "unauthorized_webhook_dispatch"`. Asserts the new check runs **before** any DB-level check by using a task that would otherwise pass.

2. `test_dispatch_guard_allows_ready_label_webhook` — `originating_message = Some("[GitHub] Issue labeled ready on senara-solutions/mika#933")`, expect dispatch proceeds (no `unauthorized_webhook_dispatch` error). Continues to subsequent checks normally.

3. `test_dispatch_guard_allows_no_originating_message` — `originating_message = None` (callback continuation / silent trigger), expect dispatch proceeds. Confirms the new guard is opt-in by context, not a blanket block.

Add a fourth helper test asserting the predicate directly:

4. `test_is_unauthorized_webhook_dispatch_predicate` in `crates/mika-agent/src/webhook_dispatch.rs` — exhaustive matrix:
   - `"[GitHub] Comment on ..."` → true
   - `"[GitHub] Issue labeled bug on ..."` → true
   - `"[GitHub] Issue labeled ready on senara-solutions/mika#933"` → false
   - `"[GitHub] PR review on ..."` → true (these have keyword-matched handler skills with their own dispatch logic; the engine guard fires on those skills' calls only if they ALSO call run_claude_pilot outside their handler scope, which is a separate failure mode and out of scope)
   - `"[claude-pilot] ..."` → false (not a `[GitHub]` prefix)
   - `""` → false
   - `"Implement mika#933"` (direct `mika ask` prompt) → false

### Integration test (eval harness)

Add `crates/mika-agent/tests/eval/test_unauthorized_webhook_dispatch_tool_boundary.rs` modeled on the existing `test_webhook_no_unauthorized_dispatch_guard.rs`. The key difference: the existing eval covers the post-hoc INTENT_GUARD; this new eval drives the agent through a real `[GitHub] Comment ...` turn where the LLM is mocked to call `create_task` + `run_claude_pilot`, and asserts:

1. `run_claude_pilot`'s `ToolOutput.is_error` is `true`.
2. The error JSON parses with `error == "unauthorized_webhook_dispatch"`.
3. No callback task was created in the DB (the spawn never started).
4. `tool_calls` row exists with `success = 0` and the error message visible in the `output` column — matches the ticket's verification criterion ("the row exists with success=0 and an error message naming the missing-label gate").

### Regression coverage for ready-label happy path

Add a parallel positive test in the same eval file confirming that on `[GitHub] Issue labeled ready on senara-solutions/mika#933` the dispatch succeeds (no new error introduced for the canonical path).

### Existing eval `test_webhook_no_unauthorized_dispatch_guard.rs`

No changes — it tests the INTENT_GUARD's post-hoc behavior, which is unchanged. The new eval tests the tool-boundary block. Both are valid layers of defense.

## Risks & mitigations

| Risk | Mitigation |
|------|------------|
| False positive: legitimate non-`[GitHub]` webhook channels accidentally caught | The predicate matches `[GitHub]` exactly. `[claude-pilot]`, `[Telegram]`, `[Slack]` etc. are unaffected. |
| False positive: synthetic `[GitHub]`-prefixed test messages | The prefix is the gateway's load-bearing format; tests outside the eval harness construct their own `LongRunningContext` with `originating_message = None`. |
| Field rename breaks existing tests | The new field is `Option<String>` defaulting to `None`; all existing test contexts compile unchanged with the default. |
| Predicate drift between agent.rs and executor.rs | Single source of truth in `webhook_dispatch.rs`; both consumers go through it. |
| Performance: per-dispatch string-prefix check overhead | Two `starts_with` calls; negligible relative to even the cheapest existing check. |
| Engine backstop allows the prompt-side rule to atrophy | The prompt-side `run_gh issue view` step makes the agent's reasoning **observable in tool-call logs** even when the engine doesn't reject — operators can audit and tighten. |

## Acceptance criteria

- [ ] `crates/mika-agent/src/webhook_dispatch.rs` exists with `READY_LABEL_DISPATCH_MARKER`, `is_unauthorized_webhook_dispatch`, `is_ready_label_dispatch_marker`, and unit tests covering the predicate matrix.
- [ ] `agent.rs::READY_LABEL_DISPATCH_MARKER` is replaced by the re-export; `webhook_no_unauthorized_dispatch_trigger` and `ready_label_dispatch_trigger` delegate to the new module. INTENT_GUARDS behavior unchanged.
- [ ] `LongRunningContext` gains `originating_message: Option<String>`. Conversation-mode construction site populates it from the latest user-role message; `DeferredDispatch` construction passes `None`.
- [ ] `validate_dispatch_readiness` accepts `originating_message: Option<&str>` and checks it first. New error `unauthorized_webhook_dispatch` returned with structured JSON.
- [ ] Three new unit tests in executor.rs pass (`rejects_unauthorized_webhook`, `allows_ready_label_webhook`, `allows_no_originating_message`).
- [ ] New eval `test_unauthorized_webhook_dispatch_tool_boundary.rs` passes; asserts no callback task was created and `tool_calls` row has `success=0` with the error in `output`.
- [ ] `skills/bundled/self-dev/system_prompt.md` Webhook Fallthrough section has the SCOPE RULE replaced with the pre-dispatch label gate; Rule 9 has the engine-backstop sentence and the mika#932 incident line.
- [ ] `cargo build --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` green.
- [ ] CHANGELOG.md entry under `## [Unreleased]` → `### Fixed`: "Webhook fallthrough enforced at the dispatch tool boundary; `run_claude_pilot` on non-ready-label `[GitHub]` webhook turns is rejected with `unauthorized_webhook_dispatch` before the subprocess spawns (mika#933)."

## Compound

After implementation, file a `docs/solutions/logic-errors/` entry documenting the **post-hoc vs tool-boundary** distinction in guard design. Key claim: when a guard's failure mode is "the side effect already shipped," EndTurn intent-precondition guards are insufficient — the gate must move to the tool-call boundary. mika#910 vs mika#933 is the citation pair. This is a transferable design rule, not a one-off lesson.

## Out of scope (explicit non-changes)

- `self-dev-webhook-qa`, `self-dev-webhook-ci` — these activate via keyword and have their own dispatch logic; the new gate does not affect them because their handler skills don't go through `run_claude_pilot` on the comment payload (they dispatch on PR-review / check-suite events that use different markers).
- mika#841's contract surface, mika#847's positive case, mika#910's post-hoc safety net — all preserved.
- The mika-platform#74 closing-comment template fix — companion ticket, separate PR.
- Any change to the `mika ask --agent mika-dev "implement <ref>"` direct path — that arrives without a `[GitHub]` prefix and is not affected.
