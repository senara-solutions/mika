# Plan: engine(self-dev) `webhook_milestone_advance` INTENT_GUARD — mika#1218

type: engine
ticket: mika#1218
date: 2026-05-20
parent: mika#1208 (PR #1230 merged 2026-05-20 — prompt-only HOLD re-entry semantics with `⚠ ENGINE GUARD PENDING mika#1218` warnings this plan removes)
related: mika#991 (`callback_milestone_advance` guard + `PostCallbackAdvance` backstop — direct architectural precedent), mika#702 (`INTENT_GUARDS` registry), mika#933 (`is_unauthorized_webhook_dispatch` shared predicate), mika#1102 (predicate sharing across guard layers), mika#1207 (milestone-close-claim guard discrimination precedent — coexisting parent and child mutations in one turn)
base SHA: `3a39bd31e01fa9ee881d20899ca4e4bd39e988d6`

## Summary

Add a structural engine-side guard that enforces "advance OR halt" on webhook turns whose correlated task has a milestone/project parent — symmetric with the inline `callback_milestone_advance` guard (mika#991). Closes the prompt-only contract gap left by mika#1208 (`⚠ ENGINE GUARD PENDING mika#1218` warnings in both `self-dev/system_prompt.md` § M4 step 2.5 and `self-dev-webhook-qa/system_prompt.md` § Path A step 5.5).

**Shape:** the guard mirrors `callback_milestone_advance`'s **inline** shape (not the `INTENT_GUARDS` const array shape), because its satisfaction predicate needs dynamic context (the milestone parent task ID) from the user message. Continuity with mika#991 means the parent ID arrives via a `[milestone-parent: <id>]` marker, but the webhook path has no LLM-pre-emitter that injects this marker today (`run_silent_agent` does it for callbacks; webhooks are a conversation-mode entry path). So this plan adds a small server-side **milestone-context handler** that runs alongside `verdict_handler` / `ci_success_handler` / `ci_failure_handler` in `handlers.rs::process_message_inner`, correlates the PR-closed webhook to a task, checks the parent's `type`, and prepends the marker line as an `enrichment` (`VerdictAction::Passthrough { enrichment }`). The guard then parses the marker exactly as `callback_milestone_advance` does — no novel parsing surface.

**Why an inline guard, not an `INTENT_GUARDS` entry (AC1 wording divergence — open question Q1):** the ticket body says "New INTENT_GUARDS entry `webhook_milestone_advance`". The `INTENT_GUARDS` const array has `trigger: fn(&str) -> bool` and `satisfied: fn(&[ToolCallSummary]) -> bool` — pure functions over only the user message text and tool summaries. `callback_milestone_advance` is **inline** at `agent.rs:1565-1593` and `agent.rs:1937-1962` (mirror for empty-text exit) precisely because its `satisfied` needs the parent task ID. The webhook variant has the same need. Filing as an `INTENT_GUARDS` entry would require either (a) a registry-shape change to support dynamic context (a much bigger ticket), or (b) a parent-ID-free satisfaction predicate (loses precision — could accept `update_task_status` on an unrelated task). The cleanest shape is the inline mirror. I read AC1 as "structural engine guard" intent, not a literal "must be in the const array" requirement; surfacing for architect ratification.

## Phase 0 — Code pins (verbatim slices at base SHA `3a39bd31`)

Source pins are read-only; the implementer must compare these against `git show 3a39bd31:<path>` before editing to confirm no upstream drift.

### Pin A — `crates/mika-agent/src/agent.rs` lines 5524-5600 (the callback precedent this plan transplants)

```rust
// ---------------------------------------------------------------------------
// #991 — Callback milestone advance guard
// ---------------------------------------------------------------------------

/// Label used for `intent_guard_retries` tracking of the callback milestone
/// advance guard (#991). Inline guard (not in `INTENT_GUARDS` const array)
/// because the satisfied predicate needs the parent_task_id from the user
/// message to distinguish parent-targeting `update_task_status` calls from
/// child-targeting ones.
const CALLBACK_MILESTONE_ADVANCE_LABEL: &str = "callback_milestone_advance";

/// Marker prefix in the user message that signals a milestone/project-context
/// callback. Emitted by `run_silent_agent` when the callback's parent task
/// has `type='milestone'` or `type='project'`. Format:
/// `[callback: {label}] [milestone-parent: {parent_task_id}]`
const MILESTONE_PARENT_MARKER: &str = "[milestone-parent: ";

/// #991 — Returns `true` when the user message indicates a milestone/project-context
/// callback turn. Checks for both the callback prefix and the milestone-parent marker.
fn callback_milestone_advance_trigger(msg: &str) -> bool {
    msg.starts_with("[callback:") && msg.contains(MILESTONE_PARENT_MARKER)
}

/// #991 — Extracts the parent task ID from the milestone-parent marker in the
/// user message. Returns `None` if the marker is absent or malformed.
fn extract_milestone_parent_id(msg: &str) -> Option<&str> { /* ... */ }

/// #991 — Returns `true` when the milestone advance obligation is satisfied.
/// Two valid paths:
/// - **Path A (advance):** `run_claude_pilot` was called (any attempt, success or failure).
/// - **Path B (halt or finish):** `update_task_status` was called with the parent
///   task ID in the input AND a terminal status (`blocked` or `completed`).
fn callback_milestone_advance_satisfied(
    parent_task_id: &str,
    summaries: &[ToolCallSummary],
) -> bool {
    // Path A: any run_claude_pilot or run_claude_pilot_groom call advances the queue
    let has_advance = summaries
        .iter()
        .any(|s| s.name == "run_claude_pilot" || s.name == "run_claude_pilot_groom");
    if has_advance { return true; }
    // Path B: update_task_status targeting the parent with blocked/completed.
    summaries.iter().any(|s| {
        s.name == "update_task_status"
            && s.input_summary.contains(parent_task_id)
            && (s.input_summary.contains("blocked") || s.input_summary.contains("completed"))
    })
}

const CALLBACK_MILESTONE_ADVANCE_CORRECTION: &str = "[mika-engine] This is a callback turn for \
     a milestone/project child task. Per mika#991 the engine expects either: \
     (1) dispatch the next pending child via run_claude_pilot, OR \
     (2) mark the milestone/project parent as `blocked` (with a reason in the note field) \
     or `completed` via update_task_status. ...";
```

**Pinpoint claim being mirrored:** the inline guard at `agent.rs:1565-1593` (non-empty text branch) and `agent.rs:1937-1962` (empty-text branch). New constants/functions for webhook live next to the callback ones at `agent.rs:5524-5600`. New inline fire sites mirror the callback ones immediately after them in `run_loop`.

### Pin B — `crates/mika-agent/src/agent.rs` lines 1557-1593 (inline guard fire site — non-empty text branch)

```rust
// #991 — Callback milestone advance guard. For milestone/project-
// context callbacks, requires the agent to either advance the
// queue (run_claude_pilot) or explicitly halt/finish the milestone
// (update_task_status on parent with blocked/completed). Inline
// rather than in INTENT_GUARDS because the satisfied predicate
// needs the parent_task_id extracted from the user message.
// Composes with callback_terminal_action (entry e): a milestone-
// context callback must satisfy BOTH guards.
if !skip_remaining_guards
    && matches!(response.stop_reason, LlmStopReason::EndTurn)
    && !intent_guard_retries.contains(CALLBACK_MILESTONE_ADVANCE_LABEL)
    && callback_milestone_advance_trigger(&user_input_text)
    && let Some(parent_id) = extract_milestone_parent_id(&user_input_text)
    && !callback_milestone_advance_satisfied(parent_id, &all_tool_summaries)
{
    intent_guard_retries.insert(CALLBACK_MILESTONE_ADVANCE_LABEL);
    warn!(/* ... */);
    request.messages.push(/* ... assistant block ... */);
    request.messages.push(LlmMessage {
        role: LlmRole::User,
        content: LlmContent::Text(CALLBACK_MILESTONE_ADVANCE_CORRECTION.to_string()),
    });
    continue;
}
```

**Insertion site for the new webhook guard:** immediately AFTER this block in both fire sites (non-empty text and empty-text), preserving fire order callback → webhook (alphabetical within `intent_guard_retries` labels: `callback_milestone_advance` → `webhook_milestone_advance`). Composition with `callback_milestone_advance` is impossible by construction — the triggers are mutually exclusive on the user message's prefix (`[callback:` vs `[GitHub] PR closed:`).

### Pin C — `crates/mika-agent/src/server/handlers.rs` lines 770-850 (handler chain dispatch — webhook handler insertion site)

```rust
// Structural verdict handler: intercept pull_request_review.submitted webhooks (#524).
let action = try_handle_pr_review_verdict(/* ... */).await;
match action { /* ... */ }

// Structural CI success handler: intercept check_suite.completed(success) (#571).
let ci_action = ci_success_handler::try_handle_ci_success(/* ... */).await;
match ci_action { /* ... */ }

// Structural CI failure handler: intercept check_suite.completed(failure|timed_out) (#594).
let ci_failure_action = ci_failure_handler::try_handle_ci_failure(/* ... */).await;
match ci_failure_action {
    VerdictAction::Handled { pre_digest } => { req.text = pre_digest; }
    VerdictAction::Passthrough { enrichment: Some(e) } => {
        req.text = format!("{e}{}", req.text);
    }
    VerdictAction::Passthrough { enrichment: None } => {}
}
```

**Insertion site for the new milestone-context handler:** immediately after the CI failure handler block, before `agent::AgentParams` is constructed. Same `VerdictAction::Passthrough { enrichment }` shape — only the `Passthrough` arm is used; the new handler never short-circuits the LLM turn (the milestone-advance decision is the LLM's, the engine just supplies the parent ID).

### Pin D — `crates/mika-gateway/src/github.rs` line 386 (PR-closed message format — what the handler matches)

```rust
if action == "closed" {
    // emitted as: "[GitHub] PR closed: {repo}#{number} — {title} (branch: {branch})\n{url}"
}
```

**Pinpoint claim:** PR-closed webhooks arrive with prefix `[GitHub] PR closed:` and the PR URL on the second line. The new handler matches both, extracts the URL, and correlates via `db.find_active_task_by_pr_url(pr_url)` (existing query, used by `find_task_for_verdict` at `server/verdict_handler.rs:1046-1081`).

### Pin E — `crates/mika-agent/src/server/verdict_handler.rs:1046-1081` (task correlation helper to reuse)

```rust
async fn find_task_for_verdict(
    db: &AsyncDatabase,
    pr_url: &str,
    event: &PrReviewEvent,
) -> Option<crate::db::Task> {
    match db.find_active_task_by_pr_url(pr_url).await {
        Ok(Some(t)) if t.status == "in_progress" => Some(t),
        /* ... */
    }
}
```

**Pinpoint claim:** task correlation by PR URL is an existing, tested DB path. The new handler reuses `db.find_active_task_by_pr_url` directly (the `find_task_for_verdict` wrapper takes a `PrReviewEvent` shape, not a PR-closed event, so we lift the wrapper or call `find_active_task_by_pr_url` directly).

### Pin F — `crates/mika-agent/src/agent.rs` lines 5345-5400 (`run_silent_agent` marker emission — the symmetry source)

The callback `[milestone-parent: <id>]` marker is emitted by `run_silent_agent` (referenced in `crates/mika-agent/CLAUDE.md` § Post-Conditions step 6b) after a DB lookup of the parent task type. The webhook handler proposed here is the **webhook-side mirror** of that emission: same DB lookup (`get_task` on `parent_task_id`, check `type IN ('milestone', 'project')`), same marker format, different injection point (server handler chain vs silent-agent invocation).

### Pin G — `skills/bundled/self-dev/system_prompt.md` § M4 step 2.5 — current `⚠ ENGINE GUARD PENDING mika#1218` warning (to be removed per AC3)

From mika#1208 prompt diff (already merged on main):

```text
> ⚠ **ENGINE GUARD PENDING mika#1218** — the "advance OR halt" obligation in the webhook handler (Phase 2 below) is enforced by prompt prose only until mika#1218 lands a `webhook_milestone_advance` INTENT_GUARD. This is the same against-gradient-behavior class as `callback_milestone_advance` (mika#991): the LLM's trained default is "acknowledge and close the turn" rather than "advance the queue." See `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` for the doctrine. mika#1218's AC3 removes this warning when the engine guard lands.
```

**Locator:** the literal string `⚠ **ENGINE GUARD PENDING mika#1218**` appears twice on main — once in `skills/bundled/self-dev/system_prompt.md` (M4 step 2.5 `auto_merge_enabled` branch) and once in `skills/bundled/self-dev-webhook-qa/system_prompt.md` (Path A step 5.5 preamble). Both must be removed in the implementation PR (AC3).

### Pin H — `crates/mika-agent/CLAUDE.md` § Post-Conditions step 6b — current text (to be updated per AC4)

```text
6b. **Callback milestone advance guard (#991):** Inline guard (not in `INTENT_GUARDS` const array) that enforces queue advancement on milestone/project-context callback turns. [... existing description ...] **Webhook companion guard gap (mika#1218):** The webhook path (`self-dev-webhook-qa` Path A step 5.5) carries the same "advance OR halt" obligation via prompt prose only (mika#1208). The engine-layer `webhook_milestone_advance` INTENT_GUARD is filed as mika#1218; until it lands, the prompt-only contract carries the obligation. mika#1218's AC3 removes the warning prose added by mika#1208.
```

**Locator:** the literal heading `**Callback milestone advance guard (#991):**` introduces this paragraph. The "Webhook companion guard gap (mika#1218):" sub-paragraph at the end is replaced with a fully-fledged description of the new `webhook_milestone_advance` guard's trigger, satisfaction, and composition behavior.

## Scope

- **In scope:**
  - New server-side **milestone-context handler** (`server::milestone_context_handler` or extension of an existing handler module) that runs in `handlers.rs::process_message_inner` after `ci_failure_handler` and prepends a `[milestone-parent: <id>]` enrichment to PR-closed webhook turns that correlate to milestone/project-context tasks.
  - New constants and functions in `agent.rs` adjacent to the callback equivalents: `WEBHOOK_MILESTONE_ADVANCE_LABEL`, `webhook_milestone_advance_trigger`, `webhook_milestone_advance_satisfied`, `WEBHOOK_MILESTONE_ADVANCE_CORRECTION`. `extract_milestone_parent_id` is **reused** from the callback path (Pin A); same marker, same parser.
  - Two new inline guard fire sites mirroring the callback inline guard at `agent.rs:1557-1593` and `agent.rs:1935-1962`.
  - Eval tests at `tests/eval/test_callback_milestone_advance.rs` extended with a new webhook cohort (one cohort per AC2(a), (b), (c) plus mirror tests for the empty-text branch). Placement follows the mika#1208 plan's cohort-by-invariant ratification (architect session `8288311f`).
  - **Removal of mika#1208's `⚠ ENGINE GUARD PENDING mika#1218` warnings** in both prompt diffs (AC3 coupling).
  - **Update to `crates/mika-agent/CLAUDE.md` § Post-Conditions step 6b** describing both callback and webhook paths (AC4).
- **Out of scope:**
  - A `PostWebhookAdvance` SilentTrigger (the second-turn backstop equivalent to `PostCallbackAdvance`) — see open question Q3 below; folded into a separate ticket if architect ratifies the deferral.
  - A unified `MilestoneAdvance` SilentTrigger that fires from any context (callback / webhook / manual) — explicit non-goal from mika#1208 §Follow-ups; the smaller per-source guard ships first to establish symmetry before consolidating.
  - The R1.c "MilestoneCompletionCascade" SilentTrigger from mika#1208 §R1 — same family as the unified trigger above; deferred.
  - Engine-side recognition of the HOLD note format (`HOLD: auto-merge enabled ...`) to suppress `PostCallbackAdvance` while HOLD is active — orthogonal feature, see mika#1208 §Follow-ups "HOLD note canonicalization."
  - Verdict handler / CI handler refactor to a shared "milestone-context augmentation" helper — they don't need it today (they bypass the LLM via `Handled` rather than relying on guard enforcement). Considered in §Risks R4.

## Phase 1 — Server-side milestone-context handler

### Goal

When a `[GitHub] PR closed:` webhook arrives and correlates to a task whose parent has `type IN ('milestone', 'project')`, prepend a `[milestone-parent: <parent_id>]` marker line to the user message so the inline guard in `run_loop` can fire.

### Module placement

New module `crates/mika-agent/src/server/milestone_context_handler.rs`. Exports a single `pub(crate)` async function:

```rust
pub(crate) async fn try_handle_pr_closed_milestone_context(
    text: &str,
    db: &AsyncDatabase,
) -> VerdictAction;
```

Returns `VerdictAction::Passthrough { enrichment: Some(format!("[milestone-parent: {parent_id}]\n")) }` when the message correlates to a milestone/project-context task; otherwise `VerdictAction::Passthrough { enrichment: None }`. Never returns `VerdictAction::Handled` — this handler does not bypass the LLM, it only supplies context.

### Detection algorithm

1. Match prefix: `text.starts_with("[GitHub] PR closed:")`. If false → `Passthrough { enrichment: None }`.
2. Extract PR URL: scan `text` for the first line matching the URL regex `^https://github\.com/[^/]+/[^/]+/pull/\d+`. If absent → `Passthrough { enrichment: None }` + DEBUG log (malformed webhook; legitimate non-merge close events also have this shape).
3. Correlate task: `db.find_active_task_by_pr_url(&pr_url).await`. If `None` → `Passthrough { enrichment: None }` + INFO log. If `Some(t)` but `t.parent_task_id.is_none()` → `Passthrough { enrichment: None }`.
4. Fetch parent task: `db.get_task(parent_id).await`. If parent's `type` is `"milestone"` or `"project"` → emit the marker. Otherwise → `Passthrough { enrichment: None }`.
5. Emit: `Passthrough { enrichment: Some(format!("[milestone-parent: {parent_id}]\n")) }`. INFO log with `pr_url`, `task_id`, `parent_id`, `parent_type` for observability.

### Reused helpers

- `db.find_active_task_by_pr_url` — existing, used by `verdict_handler::find_task_for_verdict` (Pin E).
- `db.get_task` — existing, takes a `task_id`, returns full `Task` (parent's `type` is on the struct).

### Failure policy

Fail-open: any DB error returns `Passthrough { enrichment: None }` with a WARN log. The webhook handler skill's prompt prose still carries the milestone-advance obligation as a fallback (same shape mika#1208 already ships). The engine guard is **defense-in-depth**, not the sole enforcement layer.

### Handler chain insertion

`handlers.rs::process_message_inner` lines 829-849 — append a new block matching the same `match` shape as the CI handlers:

```rust
let milestone_action = milestone_context_handler::try_handle_pr_closed_milestone_context(
    &req.text,
    &a.db,
).await;
match milestone_action {
    VerdictAction::Passthrough { enrichment: Some(e) } => {
        req.text = format!("{e}{}", req.text);
    }
    VerdictAction::Passthrough { enrichment: None } => {}
    VerdictAction::Handled { .. } => unreachable!("milestone_context handler never handles"),
}
```

**Order rationale:** placed AFTER `ci_failure_handler` so CI-failure pre-digests (which already contain advance/halt prescriptions) compose cleanly with the milestone-parent marker. CI failure pre-digest replaces `req.text`; if the new milestone handler ran BEFORE it, the marker would be lost on `Handled`. Both must precede `agent::AgentParams` construction at line 852.

## Phase 2 — Inline `webhook_milestone_advance` guard in `agent.rs`

### Goal

Mirror the inline `callback_milestone_advance` guard at two sites, with trigger keyed on the webhook prefix rather than the callback prefix.

### New constants and functions at `agent.rs:5524-5600` (adjacent to the callback equivalents)

```rust
// ---------------------------------------------------------------------------
// #1218 — Webhook milestone advance guard
// ---------------------------------------------------------------------------

/// Label used for `intent_guard_retries` tracking of the webhook milestone
/// advance guard (#1218). Inline guard (not in `INTENT_GUARDS` const array)
/// because the satisfied predicate needs the parent_task_id from the user
/// message — identical shape to `callback_milestone_advance` (#991).
const WEBHOOK_MILESTONE_ADVANCE_LABEL: &str = "webhook_milestone_advance";

/// #1218 — Returns `true` when the user message indicates a milestone/project-
/// context PR-closed webhook turn. Reuses the `[milestone-parent: ...]`
/// marker from #991; the prefix check ensures we don't compose with the
/// callback guard (mutually exclusive triggers).
fn webhook_milestone_advance_trigger(msg: &str) -> bool {
    msg.starts_with("[milestone-parent: ")
        && msg.contains("[GitHub] PR closed:")
        || (msg.contains(MILESTONE_PARENT_MARKER) && msg.contains("[GitHub] PR closed:"))
}
```

**Marker-position note:** the server-side handler (Phase 1) prepends `[milestone-parent: <id>]\n` to the message — so after enrichment, the message starts with the marker and the `[GitHub] PR closed:` prefix is on a later line. The trigger checks both orderings (marker-first OR marker-anywhere-with-PR-closed-prefix) to remain robust to gateway message-format changes. The implementer should pick the simplest valid form once they verify the exact post-enrichment shape; this plan presents both for the architect to choose.

```rust
/// #1218 — Returns `true` when the webhook milestone advance obligation is satisfied.
/// Three valid paths (mirrors #991 plus deploy-hook path from mika#1208 plan §Phase 2 step 5.5.b):
/// - **Path A (advance):** `run_claude_pilot` or `run_claude_pilot_groom` was called.
/// - **Path B (halt/finish):** `update_task_status` targeting the parent task ID
///   with status `blocked` or `completed`.
/// - **Path C (deploy hook):** BOTH `deploy_mika` AND `send_message` were called
///   (deploy-hook ack to operator per the 5.5.b prompt contract).
///
/// **Race-case extension (RESOLVED in open question Q4 below):** Path D —
/// `update_task_status` on the CHILD task (any status) with note prefix `"HOLD: webhook arrived but PR state != MERGED"`
/// — see Q4 for the disposition.
fn webhook_milestone_advance_satisfied(
    parent_task_id: &str,
    summaries: &[ToolCallSummary],
) -> bool {
    // Path A — reuse the same predicate as callback (#991).
    let has_advance = summaries
        .iter()
        .any(|s| s.name == "run_claude_pilot" || s.name == "run_claude_pilot_groom");
    if has_advance { return true; }
    // Path B — parent-targeting update_task_status with terminal status.
    let has_halt = summaries.iter().any(|s| {
        s.name == "update_task_status"
            && s.input_summary.contains(parent_task_id)
            && (s.input_summary.contains("blocked") || s.input_summary.contains("completed"))
    });
    if has_halt { return true; }
    // Path C — deploy-hook ack: BOTH deploy_mika AND send_message.
    let has_deploy = summaries.iter().any(|s| s.name == "deploy_mika");
    let has_notify = summaries.iter().any(|s| s.name == "send_message");
    has_deploy && has_notify
}

const WEBHOOK_MILESTONE_ADVANCE_CORRECTION: &str = "[mika-engine] This is a `pull_request.closed(merged:true)` \
     webhook turn for a milestone/project child task. Per mika#1218 the engine expects exactly one of: \
     (1) dispatch the next pending child via run_claude_pilot, OR \
     (2) mark the milestone/project parent as `blocked` (with a reason) or `completed` via update_task_status, OR \
     (3) deploy_mika + send_message (the deploy-hook ack path from self-dev-webhook-qa step 5.5.b). \
     Posting a confirmation or summary without one of these three tool calls is the deliberation-stall \
     pattern documented in mika#991. Re-read the webhook event and either advance the queue, halt the \
     milestone explicitly, or trigger the deploy hook.";
```

### New inline guard fire sites

**Site 1 — non-empty text branch.** Inserted immediately AFTER the existing `callback_milestone_advance` block at `agent.rs:1557-1593`. Same shape:

```rust
// #1218 — Webhook milestone advance guard. Mirrors callback_milestone_advance
// for `pull_request.closed(merged:true)` webhook turns whose correlated task has
// a milestone/project parent. Inline rather than in INTENT_GUARDS because the
// satisfied predicate needs the parent_task_id (injected as a marker by
// server::milestone_context_handler).
if !skip_remaining_guards
    && matches!(response.stop_reason, LlmStopReason::EndTurn)
    && !intent_guard_retries.contains(WEBHOOK_MILESTONE_ADVANCE_LABEL)
    && webhook_milestone_advance_trigger(&user_input_text)
    && let Some(parent_id) = extract_milestone_parent_id(&user_input_text)
    && !webhook_milestone_advance_satisfied(parent_id, &all_tool_summaries)
{
    intent_guard_retries.insert(WEBHOOK_MILESTONE_ADVANCE_LABEL);
    warn!(
        step,
        label = mode.label(),
        parent_task_id = parent_id,
        intent_guard = WEBHOOK_MILESTONE_ADVANCE_LABEL,
        "Webhook milestone advance guard fired — re-prompting"
    );
    request.messages.push(LlmMessage {
        role: LlmRole::Assistant,
        content: LlmContent::Blocks(
            mika_common::llm::response_content_to_blocks(&response.content),
        ),
    });
    request.messages.push(LlmMessage {
        role: LlmRole::User,
        content: LlmContent::Text(WEBHOOK_MILESTONE_ADVANCE_CORRECTION.to_string()),
    });
    continue;
}
```

**Site 2 — empty-text branch.** Inserted immediately AFTER the empty-text mirror of `callback_milestone_advance` at `agent.rs:1937-1962`. Same shape, applies in webhook context only if the LLM returns empty text after diagnostic tool calls (rare — webhook turns are conversation-mode and `follow_up_on_empty()` returns true; the empty-text branch fires only in modes where `follow_up_on_empty()` returns false). Including it for symmetry with the callback mirror and to guard against future mode-flag changes.

### Composition with existing guards

- **Mutually exclusive triggers with `callback_milestone_advance`:** the user message starts with `[milestone-parent: ...]` followed by `[GitHub] PR closed:` for webhooks vs `[callback: ...] [milestone-parent: ...]` for callbacks. No single message satisfies both triggers.
- **Mutually exclusive triggers with `callback_terminal_action` (entry e of INTENT_GUARDS):** callback_terminal_action's trigger is `msg.starts_with("[callback:")` — false for webhook messages.
- **Composes with `webhook_no_unauthorized_dispatch` (entry b of INTENT_GUARDS):** this entry's trigger is `is_unauthorized_webhook_dispatch(msg)` which returns `false` for `[GitHub] PR closed:` (qa-territory allowlist per `webhook_dispatch.rs:38`). No interference.
- **Composes with `webhook_zero_tools` (entry c of INTENT_GUARDS):** zero-tools fires regardless of milestone context. If a webhook turn ends with zero tool calls, zero-tools fires first (registry guards iterate before inline guards in `run_loop`). After the zero-tools retry, if the LLM responds with text-only and still zero tools, zero-tools is exhausted (single-retry) and the new webhook_milestone_advance guard fires next. Acceptable composition: both nudge in the same direction (call a tool).

## Phase 3 — Tests

Extend `crates/mika-agent/tests/eval/test_callback_milestone_advance.rs` as a new cohort. Placement follows the mika#1208 plan's cohort-by-invariant decision (architect session `8288311f` NB2).

### New cohort: `webhook_milestone_advance` (one test per AC2 sub-clause plus mirrors)

1. **`webhook_milestone_advance_path_a_accepts_run_claude_pilot`** — webhook turn carries the `[milestone-parent: <id>]` marker; `run_claude_pilot` is called; assert guard does NOT fire, EndTurn accepted.
2. **`webhook_milestone_advance_path_a_accepts_run_claude_pilot_groom`** — same but `run_claude_pilot_groom` (auto-groom path).
3. **`webhook_milestone_advance_path_b_accepts_halt_blocked`** — `update_task_status(parent_id, status: "blocked")`. Assert guard does NOT fire.
4. **`webhook_milestone_advance_path_b_accepts_halt_completed`** — `update_task_status(parent_id, status: "completed")`. Assert guard does NOT fire.
5. **`webhook_milestone_advance_path_c_accepts_deploy_hook`** — BOTH `deploy_mika` AND `send_message` called. Assert guard does NOT fire.
6. **`webhook_milestone_advance_silent_text_rejection_then_retry_succeeds`** — webhook turn emits text-only (no tool calls). Assert guard fires once, re-prompt injected with `WEBHOOK_MILESTONE_ADVANCE_CORRECTION`; the second turn calls `run_claude_pilot`, EndTurn accepted.
7. **`webhook_milestone_advance_single_retry_semantics`** — webhook turn fails to satisfy even after retry. Assert guard fires exactly once (label tracked in `intent_guard_retries`), then EndTurn is accepted on the second violation with WARN log.
8. **`webhook_milestone_advance_no_marker_no_fire`** — webhook arrives but the milestone-context handler did NOT inject the marker (non-milestone task). Assert guard does NOT fire even on zero-tool EndTurn.
9. **`webhook_milestone_advance_callback_marker_does_not_trigger_webhook_guard`** — composition isolation: a callback turn with `[milestone-parent: <id>]` triggers `callback_milestone_advance` (existing behavior), NOT `webhook_milestone_advance`. Asserts on the label tracked in `intent_guard_retries`.

### Mock harness shape

The existing test file already uses `MockLlmProvider` from `mika-common::llm::mock` and the `EvalHarness` builder. New tests reuse the same fixtures with a different user message prefix (`[milestone-parent: <id>]\n[GitHub] PR closed: ...\nhttps://github.com/.../pull/N`) and assert on `intent_guard_retries` and the post-loop tool-call summaries.

### Server-side handler tests

Separate unit tests in `crates/mika-agent/src/server/milestone_context_handler.rs` `#[cfg(test)] mod tests`:
1. Non-PR-closed message → `Passthrough { enrichment: None }`.
2. PR-closed message, no PR URL parse → `Passthrough { enrichment: None }`.
3. PR-closed message, URL parses, no task found → `Passthrough { enrichment: None }`.
4. PR-closed, task found, parent absent → `Passthrough { enrichment: None }`.
5. PR-closed, task found, parent type `"issue"` → `Passthrough { enrichment: None }`.
6. PR-closed, task found, parent type `"milestone"` → `Passthrough { enrichment: Some("[milestone-parent: <id>]\n") }`.
7. PR-closed, task found, parent type `"project"` → `Passthrough { enrichment: Some(...) }`.

Use an in-memory `AsyncDatabase` (existing test pattern at `db::tests` / `task_engine::tests`).

## Phase 4 — Documentation

1. **Remove `⚠ ENGINE GUARD PENDING mika#1218` warnings** in two places (AC3):
   - `skills/bundled/self-dev/system_prompt.md` § M4 step 2.5 `auto_merge_enabled` branch — remove the blockquote per Pin G.
   - `skills/bundled/self-dev-webhook-qa/system_prompt.md` § Path A step 5.5 preamble — remove the blockquote per Pin G.

   The surrounding prompt prose stays (the HOLD semantics, the "advance OR halt" obligation, the step 5.5.a/b/c paths). Only the engine-guard-pending warnings are removed.

2. **Update `crates/mika-agent/CLAUDE.md` § Post-Conditions step 6b** (AC4) — replace the closing "Webhook companion guard gap" sub-paragraph with a description of the new guard. Proposed replacement text:

   > **Webhook companion guard (#1218, paired with #991):** Sibling inline guard for `pull_request.closed(merged:true)` webhook turns. Triggers on the `[milestone-parent: <id>]` marker prepended by `server::milestone_context_handler` when the PR-closed event correlates to a task with a `milestone`/`project` parent. Satisfaction has three valid paths: Path A (`run_claude_pilot` or `run_claude_pilot_groom` — advance), Path B (`update_task_status` on parent with `blocked`/`completed` — halt), Path C (`deploy_mika` + `send_message` — deploy-hook ack per self-dev-webhook-qa step 5.5.b). Mutually exclusive triggers with #991: the callback prefix `[callback:` and the webhook prefix `[GitHub] PR closed:` cannot both appear on a single user message. The marker parser `extract_milestone_parent_id` and the constant `MILESTONE_PARENT_MARKER` are shared with #991.

3. **Add an entry to `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`** — single line in the "Gradient data points" section noting that mika#1218 establishes per-event-source symmetry for milestone-advance, and that a unified `MilestoneAdvance` SilentTrigger (R1.c from mika#1208) remains the next-step consolidation. Out-of-scope sentence count: ~2-3 lines. Frontmatter tags unchanged.

## Risks

1. **R1 — Marker-format coupling between server handler and inline guard.** The Phase 1 handler emits `[milestone-parent: <id>]\n`; the Phase 2 guard parses it via the shared `MILESTONE_PARENT_MARKER = "[milestone-parent: "` constant and `extract_milestone_parent_id`. Single source of truth on the marker shape (Pin A). **Mitigation:** A new static assertion test (`#[test] fn marker_constant_matches_handler_emission()`) in the handler module imports `MILESTONE_PARENT_MARKER` and asserts the emission string starts with it. Defense against future drift.

2. **R2 — Trigger ordering ambiguity in `webhook_milestone_advance_trigger`.** The plan presents two trigger shapes (marker-first vs marker-anywhere). The implementer must pick one based on the EXACT post-enrichment message shape. **Mitigation:** Phase 1 step 5 emits the marker as a PREFIX (`format!("{enrichment}{original_text}")` — see Pin C `match` arm); the simpler trigger `msg.starts_with(MILESTONE_PARENT_MARKER) && msg.contains("[GitHub] PR closed:")` is correct given that order. The two-shape presentation in the plan is for architect ratification; the implementation ships the prefix-form.

3. **R3 — TOCTOU on parent task type.** The handler reads `parent.type` at message-ingest time, before the LLM turn runs. If the LLM mutates the parent task's `type` mid-turn (e.g., via an `update_task` tool), the marker becomes stale. **Mitigation:** `type` is set at task-creation time (`create_task`) and never mutated on existing tasks (per `crates/mika-agent/CLAUDE.md` schema v23 description: `type TEXT NOT NULL DEFAULT 'issue' CHECK ...`). No tool today rewrites `tasks.type`. Risk is hypothetical, not active. If a future ticket adds a mutation path, this guard must be revisited.

4. **R4 — Verdict handler and CI handlers do NOT inject the milestone-parent marker today.** A PR-review or check-suite webhook against a milestone-context task does not surface the marker, so this guard does NOT enforce milestone-advance on those event classes. **Disposition:** intentional and correct. `verdict_handler` and `ci_success_handler` already BYPASS the LLM via `VerdictAction::Handled` for the merge-eligibility decision — the milestone-advance decision in those paths is engine-deterministic, not LLM-driven, so the guard is not needed there. `ci_failure_handler` enriches but does NOT decide; it would benefit from the marker (the LLM still owns the iterate-vs-escalate decision). **Open follow-up:** consider adding the milestone-context handler call to the CI failure flow in a separate ticket once this one settles. Not in scope here because it expands the test matrix and the milestone-advance semantics on CI failure are different (the LLM dispatches `run_claude_pilot` for a CI fix, NOT for the next pending child — the guard's Path A would over-accept).

5. **R5 — `PostWebhookAdvance` SilentTrigger absence.** The callback path has `PostCallbackAdvance` (mika#991) which fires a second silent turn if the first callback turn did not advance, auto-blocking the milestone after a second miss. The webhook path has no equivalent today. **Disposition:** ticket open question Q3 (below). If the architect ratifies deferral, file a follow-up; if not, scope expands here.

6. **R6 — Path C false positives (`deploy_mika` + `send_message` in unrelated turns).** The Phase 2 `webhook_milestone_advance_satisfied` Path C admits any turn that calls BOTH tools. A webhook turn that legitimately deploys for a NON-milestone-context reason and notifies in the same turn would pass. **Likelihood:** low (the trigger already restricts to milestone-context webhook turns, so any `deploy_mika` call in such a turn is the legitimate 5.5.b deploy-hook ack). **Mitigation:** acceptable. The guard's purpose is to enforce SOME action, not to police the action's correctness; the prompt prose owns the "deploy_mika in the context of the milestone parent" semantic.

## Open questions

1. **Q1 — `INTENT_GUARDS` const array entry vs inline guard.** Ticket AC1 says "New INTENT_GUARDS entry"; this plan ships the inline shape (mirror of #991). The choice is forced by the satisfaction predicate needing `parent_task_id` (dynamic context the registry doesn't pass). Resolution options: (a) ratify inline (recommended — matches #991), (b) push back and require an `INTENT_GUARDS` shape extension (much larger ticket, registry redesign), (c) accept a parent-ID-free satisfaction predicate that fires on ANY `run_claude_pilot`/`update_task_status`/`deploy_mika` call (precision loss — false negatives if the LLM updates an unrelated task). **My lean:** (a). The ticket's AC1 wording is taxonomic, not architectural — "INTENT_GUARDS" reads as "post-condition intent-precondition family," and the inline mirror IS in that family.

2. **Q2 — Marker-injection location.** This plan adds a new server handler module (`milestone_context_handler`). Alternatives: (i) extend `verdict_handler.rs` with a sibling `try_handle_pr_closed_milestone_context` function (already handles PR-related webhooks, but the existing module is named for review events specifically), (ii) inject the marker inside `agent::run_agent` before the LLM turn (couples engine to webhook semantics — anti-pattern), (iii) inject in the gateway crate (`mika-gateway::github::format_event_text` — gateway has no DB access today, would need to add). **My lean:** new sibling module (Phase 1 as proposed). Keeps the handler chain shape clean and lets `verdict_handler` stay scoped to review verdicts.

3. **Q3 — `PostWebhookAdvance` SilentTrigger.** The ticket's "Backstop trigger" paragraph notes "fire a `PostWebhookAdvance` SilentTrigger (or extend `PostCallbackAdvance` to cover both — symmetry argument)." Options: (a) ship this ticket without a backstop (guard fires + retries once; if that fails, EndTurn is accepted with WARN log — same as #991 single-retry semantics WITHOUT the second-turn backstop layer); (b) extend `PostCallbackAdvance` to fire from both callback and webhook contexts (rename to `PostMilestoneAdvance`?); (c) add a separate `PostWebhookAdvance` SilentTrigger. **My lean:** (a) for this ticket. The ticket's "Single-retry semantics like other INTENT_GUARDS entries" line in the proposed contract argues for (a). Defer (b) or (c) to a follow-up once we have observability data on first-turn-success vs second-turn-success rates on the webhook path. File as a follow-up ticket regardless of which the architect picks.

4. **Q4 — Race-case Path D (the 5.5.a re-set HOLD scenario).** mika#1208 plan §Phase 2 step 5.5.a defines a race scenario: webhook arrives but `gh pr view` shows `state != MERGED`. The prompt prescribes `update_task_status(child_task_id, status="in_progress", note="HOLD: webhook arrived but PR state != MERGED")` + `send_message` + end the turn. The ticket's satisfaction list does NOT include this path. Options: (a) add Path D — `update_task_status` on the CHILD (not parent) with note-prefix match — to the satisfaction predicate (precision: matches the prompt contract exactly); (b) accept the guard fires once on this rare race, retries, and either succeeds (if the LLM retries the same path) or accepts EndTurn after the WARN (single-retry semantics); (c) include `send_message` alone as a relief path (operator-notify acts as the safety valve — but this is too permissive). **My lean:** (a). The Path D condition is concrete and the prompt mandates it; the guard should match. Implementation: add a fourth arm to `webhook_milestone_advance_satisfied` that checks for `update_task_status` on the child task ID (which we'd need to inject as a second marker — adding complexity) OR matches on a note-prefix string. Note-prefix is fragile (LLM may paraphrase). **Acceptable workaround:** Q4(b) — accept the rare false-positive guard fire on the race case; the WARN log surfaces it; if real-world frequency is non-trivial, file a follow-up with Q4(a) implementation. **I lean Q4(b) for v1**, file the follow-up under "operational signals show a need" gating per mika#1208 §Follow-ups style.

5. **Q5 — Test placement: extend existing file vs new file.** mika#1208 plan ratified "cohort by invariant" — extend `test_callback_milestone_advance.rs`. This plan inherits that decision. Architect can override on second pass; defaulting to inheritance saves a discussion round.

## Acceptance criteria (mapped to ticket ACs)

- **AC1** (ticket: "New INTENT_GUARDS entry `webhook_milestone_advance` added per the contract above"): NEW constants/functions in `agent.rs` (`WEBHOOK_MILESTONE_ADVANCE_LABEL`, `webhook_milestone_advance_trigger`, `webhook_milestone_advance_satisfied`, `WEBHOOK_MILESTONE_ADVANCE_CORRECTION`) + TWO new inline guard fire sites in `run_loop` mirroring the callback equivalents. Single-retry semantics via `intent_guard_retries`. **Wording note:** AC1 satisfied "by family, not by literal array entry" — see Q1.
- **AC2** (ticket: "Eval test at `tests/eval/test_webhook_milestone_advance.rs` (or extension of `test_callback_milestone_advance.rs`) covering: (a) advance-via-`run_claude_pilot` satisfies the guard, (b) halt-via-`update_task_status(blocked)` satisfies, (c) silent-text rejection + single retry"): 9 new tests in the existing file (cohort placement per Q5), per Phase 3 above. Three explicitly map to AC2(a)/(b)/(c); the other six cover Path C, single-retry exhaustion, no-marker no-fire, callback-vs-webhook trigger isolation, and the empty-text branch.
- **AC3** (ticket: "mika#1208 prompt diff's `⚠ ENGINE GUARD PENDING mika#<this>` warning is **removed** in the same PR (coupling)"): Phase 4 step 1 — remove both warnings (M4 step 2.5 in self-dev + Path A step 5.5 in self-dev-webhook-qa). Coupling enforced by file-level grep in the PR's manual verification step.
- **AC4** (ticket: "`crates/mika-agent/CLAUDE.md` § Post-Conditions step 6b updated to describe both callback and webhook paths"): Phase 4 step 2 — replace the closing "Webhook companion guard gap" sub-paragraph with the full description of the new guard.
- **AC5** (NEW, not in ticket): server-side `milestone_context_handler` module with the seven unit tests in Phase 3 sub-section "Server-side handler tests." **Justification:** the engine guard depends on marker injection; without the handler module the guard never fires on webhook paths. The ticket's Surface section names `crates/mika-agent/src/agent.rs` only; this plan expands to `crates/mika-agent/src/server/milestone_context_handler.rs` as the structural prerequisite. Surfaced to architect for ratification or split into a sibling ticket.

## Follow-ups (ticket-able after merge)

- **`PostWebhookAdvance` or unified `PostMilestoneAdvance` SilentTrigger** (Q3 disposition).
- **Race-case Path D precision** (Q4 disposition) — if the rare-race guard-fire shows up in operational logs.
- **CI failure handler milestone-context augmentation** (R4) — if the CI-fix path needs the same guard semantics on milestone-context children.
- **Unified `MilestoneAdvance` SilentTrigger** (R1.c from mika#1208) — the larger cross-source consolidation.
- **HOLD note canonicalization** (mika#1208 §Follow-ups) — engine recognizes the HOLD note and suppresses backstop turns while HOLD is active.

## Out of scope (explicitly)

- The `PostWebhookAdvance` second-turn backstop (Q3 disposition).
- A unified `MilestoneAdvance` SilentTrigger across all event sources (mika#1208 §R1.c).
- HOLD-note engine recognition (mika#1208 follow-up).
- Verdict handler / CI handler refactor to share milestone-context augmentation (R4).
- Provider-specific prompt variants (none exist on `self-dev`, `self-dev-webhook-qa`, or `self-dev-callback`; per `feedback_no_provider_prompts`).
- Project P4 vs milestone M4 — P4 inherits M4 by reference per `self-dev/system_prompt.md:660`; no separate handling needed because both expose the same `[milestone-parent: <id>]` marker shape (parent task `type` is `"project"` instead of `"milestone"`, but both pass the Phase 1 step 4 condition).
