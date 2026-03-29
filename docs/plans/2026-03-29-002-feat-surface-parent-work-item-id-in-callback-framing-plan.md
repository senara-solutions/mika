---
title: "feat: surface parent work_item_id in callback framing"
type: feat
status: completed
date: 2026-03-29
issue: "#313"
---

# Surface parent work_item_id in Callback Framing

## Overview

After a long-running callback completes (e.g., claude-pilot finishes a dev task), the agent receives framing text with the callback's own task ID but NOT the parent work item ID. This forces the agent to remember the work item across async gaps or query `list_work_items` — both unreliable after conversation compaction. Additionally, `build_callback_trigger_context()` hardcodes claude-pilot-specific workflow instructions that compete with the self-dev skill's prompt, creating two sources of truth.

## Problem Statement

**Root cause 1 — Missing parent_task_id:** `format_callback_framing()` emits `Task: '{label}' (ID: {task_id})` showing only the callback task's own UUID. The `parent_task_id` field exists on the `Task` struct at both dispatch points (server dispatcher and TUI poller) but is dropped when constructing `SilentTrigger::Callback` and `AgentRequest::CallbackResult`.

**Root cause 2 — Competing instruction sets:** `build_callback_trigger_context()` has three branches: claude-pilot success (5-step workflow), claude-pilot failure (escalation), and generic. The engine's 5-step workflow competes with the self-dev skill's 450-line prompt. Weaker models follow neither fully — having two sources of truth is an architectural anti-pattern.

## Proposed Solution

### A. Simplify `build_callback_trigger_context()` to generic framing

Replace the 3-branch `if/else if/else` with a single generic path:
- Remove `CLAUDE_PILOT_CALLBACK_LABEL` constant
- All callbacks get the same generic framing
- Add "Follow the workflow defined by your active skills for this callback type" — delegates to the correct skill
- Self-dev skill prompt becomes the single source of truth for claude-pilot workflow

### B. Surface `parent_task_id` in callback framing

Thread `parent_task_id: Option<String>` through both callback paths and include it in `format_callback_framing()` output when present.

## Technical Approach

### Files and Changes

#### 1. `crates/mika-agent/src/agent.rs` — Core framing functions

**`CLAUDE_PILOT_CALLBACK_LABEL` (line 63):** Delete the constant.

**`build_callback_trigger_context()` (lines 74-126):** New signature and single-path body:

```rust
pub fn build_callback_trigger_context(
    label: &str,
    task_id: &str,
    parent_task_id: Option<&str>,  // NEW
    result: &str,
    failed: bool,
) -> String {
    let base = format_callback_framing(label, task_id, parent_task_id, result, failed);
    format!(
        "{base}\n\
         IMPORTANT: A successful result confirms only the specific action performed. \
         NEVER extrapolate to downstream states (PR status, CI health, deploy readiness) \
         that the result does not explicitly mention.\n\n\
         Follow the workflow defined by your active skills for this callback type. \
         If no skill-specific workflow applies, use send_message to notify the user \
         with a clear, concise summary of the key findings and any recommended actions."
    )
}
```

**`format_callback_framing()` (lines 137-172):** Add `parent_task_id: Option<&str>` parameter. When `Some(id)`, emit `Parent work item: {id}` line after the `Task:` line:

```rust
pub fn format_callback_framing(
    label: &str,
    task_id: &str,
    parent_task_id: Option<&str>,  // NEW
    result: &str,
    failed: bool,
) -> String {
    // ... truncation logic unchanged ...
    let parent_line = parent_task_id
        .map(|id| format!("\nParent work item: {id}"))
        .unwrap_or_default();
    format!(
        "A background task has {status}.\n\
         Task: '{label}' (ID: {task_id}){parent_line}\n\n\
         <callback_result trust=\"untrusted\">\n{result}\n</callback_result>\n\n\
         {grounding}"
    )
}
```

#### 2. `crates/mika-agent/src/agent.rs` — `SilentTrigger::Callback` variant (line ~1590)

Add `parent_task_id: Option<String>` field:

```rust
Callback {
    task_id: String,
    label: String,
    result: String,
    failed: bool,
    parent_task_id: Option<String>,  // NEW
},
```

Update the match arm in `run_silent_inner()` (~line 1738) to destructure and pass `parent_task_id`.

#### 3. `crates/mika-agent/src/task_engine/dispatcher.rs` — Server callback path

In `dispatch_resume_agent()` (~line 319), thread `task.parent_task_id.clone()` into `SilentTrigger::Callback` construction.

#### 4. `crates/mika-cli/src/tui/app.rs` — TUI callback path

**`AgentRequest::CallbackResult` variant (~line 250):** Add `parent_task_id: Option<String>` field.

**`poll_callback_tasks()` (~line 1491):** Forward `task.parent_task_id.clone()` when constructing `AgentRequest::CallbackResult`.

**Callback metadata JSON:** Include `parent_task_id` in the metadata for audit trail:
```json
{"callback_task_id": "...", "label": "...", "parent_task_id": "..."}
```

#### 5. `crates/mika-cli/src/commands/chat.rs` — TUI handler (~line 324)

Destructure `parent_task_id` from `AgentRequest::CallbackResult` and pass to `build_callback_trigger_context()`.

#### 6. Tests in `agent.rs` (~lines 3632-3713)

- **Delete:** `test_callback_trigger_claude_pilot_success_injects_workflow_continuation`, `test_callback_trigger_claude_pilot_failure_injects_escalation`, `test_callback_trigger_label_must_be_exact_match`
- **Update:** `test_callback_trigger_generic_retains_analyze_instruction` and `test_callback_trigger_generic_failed_retains_analyze_instruction` — update assertions to match new generic text ("Follow the workflow defined by your active skills")
- **Add:** `test_callback_framing_with_parent_task_id` — verifies "Parent work item: {uuid}" appears when `Some`
- **Add:** `test_callback_framing_without_parent_task_id` — verifies no parent line when `None`
- **Add:** `test_callback_framing_parent_task_id_with_failed` — parent line present even when failed=true
- **Update:** All `SilentTrigger::Callback` construction sites in tests to include `parent_task_id` field

#### 7. CLAUDE.md updates

Update references to "workflow-aware callback triggers" and `CLAUDE_PILOT_CALLBACK_LABEL` in the architecture documentation. The callback section should describe the new generic framing approach.

## System-Wide Impact

- **Interaction graph:** `build_callback_trigger_context()` is called from exactly 2 runtime paths (silent dispatcher, TUI chat handler) and 5 test sites. No callbacks, middleware, or observers involved.
- **Error propagation:** `parent_task_id` is an `Option<String>` passed through — no error paths introduced. If the parent task no longer exists, the agent sees the UUID and may get "not found" from `check_work_item`, which is normal agent-level recovery.
- **State lifecycle risks:** None. This is a read-only data threading change — no new writes, no state mutations, no DB changes.
- **API surface parity:** Both callback delivery paths (server/silent and TUI) will use the same `build_callback_trigger_context()` with the same parameters. The `mika ask --task-id` path is separate (it uses `--parent-task-id` flag for work item guard, a different mechanism).
- **Dual-path consistency:** Per learnings from `tui-callback-skips-mika-qa-delegation.md`, both paths call the same public function. This change maintains that invariant.

## Acceptance Criteria

- [x] `CLAUDE_PILOT_CALLBACK_LABEL` constant removed from `agent.rs`
- [x] `build_callback_trigger_context()` uses single generic framing for all callback types
- [x] `format_callback_framing()` includes `Parent work item: {uuid}` when `parent_task_id` is `Some`
- [x] No parent line when `parent_task_id` is `None` (backward compatible)
- [x] `SilentTrigger::Callback` variant carries `parent_task_id: Option<String>`
- [x] `AgentRequest::CallbackResult` variant carries `parent_task_id: Option<String>`
- [x] Server path threads `task.parent_task_id` through dispatcher → trigger → framing
- [x] TUI path threads `task.parent_task_id` through poller → request → chat handler → framing
- [x] Claude-pilot-specific tests removed; generic tests updated with new assertion text
- [x] New tests cover `parent_task_id` Some/None/failed permutations
- [x] CLAUDE.md updated to reflect generic framing (remove workflow-aware routing references)
- [x] `cargo test` passes
- [x] `cargo clippy` clean

## Sources & References

- Issue: #313
- Companion: #314 (callback turn work item context injection — already merged)
- Learning: `docs/solutions/logic-errors/tui-callback-skips-mika-qa-delegation.md` — dual-path consistency pattern
- Learning: `docs/solutions/architecture-patterns/trace-id-structural-linkage-delegate-silent-callback.md` — `Option<String>` field propagation pattern
- Learning: `docs/solutions/architecture-patterns/callback-task-loop-prevention.md` — callback safety constraints
- Learning: `docs/solutions/architecture-patterns/callback-turn-work-item-context-injection.md` — SilentTrigger guard patterns
