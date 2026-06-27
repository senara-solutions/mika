# Plan: Engine-Side Dispatch for Ready-Label Handler (mika#1572)

---

## Deepening addendum (2026-06-27, verified against code)

Resume/deepen pass verified the plan's flagged caveats against the actual `mika-agent` tree. Corrections and confirmations below are authoritative over the original step prose where they differ; the step structure is unchanged.

- **C1 — Tool resolution (corrects Step 1 + KTD-1).** There is no `load_tools()` / `find_tool_by_name()` method. `SkillRegistry` holds `skills: Vec<SkillEntry>` (field is `skills`, not `entries`); each `SkillEntry` has a **public** `skill_tools: Vec<ResolvedSkillTool>` field (`skills/index.rs:139`). `resolve_tool_by_name` is a simple `self.skills.iter().flat_map(|e| &e.skill_tools).find(|t| t.definition.name == name).cloned()`. Requires `ResolvedSkillTool: Clone` (verify at build; add derive if absent). `ResolvedSkillTool { definition: ToolDefinition, handler: ToolHandler, skill_dir: PathBuf }` (`skills/index.rs:52`).
- **C2 — Visibility (confirms Step 3b).** `validate_dispatch_readiness` (`skills/executor.rs:843`, `async fn`) and `spawn_long_running_exec` (`skills/executor.rs:2046`, `fn`) are both private → make `pub(crate)`. Confirmed.
- **C3 — Spawn replication (sharpens Step 3).** `execute_long_running` (`executor.rs:1777`) depends on `LongRunningContext` (`db`, `session_id`, `trace_id`, `agent_name`, per-turn `dispatch_count: AtomicU32`, `originating_message`) which the pre-LLM-turn handler does NOT have. The engine path replicates only what it needs: build callback `NewTask` (mirror `executor.rs:1923-1982`), `create_task`, `cmd_path = skill_tool.skill_dir.join(command)` existence check, enrich input with `__mika_task_id` + `__mika_agent`, then `spawn_long_running_exec`. **`command` is sourced by destructuring `skill_tool.handler` as `ToolHandler::Exec { command, .. }`** — not `handler.command()`. The per-turn `dispatch_count` cap is **omitted** in the engine path (it is a per-LLM-turn counter; pre-turn there is exactly one dispatch). The real double-dispatch protection is `validate_dispatch_readiness`'s `global_dispatch_active` per-class slot guard, which the engine path calls. `__mika_agent` needs the agent name — resolve from handler scope (the handler has `db.agent_id()`; confirm agent_name vs agent_id at impl, fall back to `db.agent_id()`).
- **C4 — Guard composition (VERIFIES Step 6's "no AgentParams threading needed", with stronger evidence).** The guard trigger `ready_label_dispatch_trigger` → `is_ready_label_dispatch_marker(msg)` uses **`msg.starts_with(READY_LABEL_DISPATCH_MARKER)`** (`webhook_dispatch.rs:51`), and `handlers.rs:901` passes `user_message: &req.text` — i.e., the guard sees the **post-handler** `req.text`. The **current `Handled` pre-digest already starts with `<ready_label_handler>`** (`ready_label_handler.rs:301`), so the marker `starts_with` is already false on the Handled path — **the guard does not fire when the handler succeeds today.** The ticket's "guard fires repeatedly / 0-5" evidence therefore comes from the **degraded Passthrough paths** (no github_token / body-fetch fail / parse fail / task-create fail), where `req.text` stays the raw marker. **Implication for #1572:** the `Dispatched` pre-digest (also `<ready_label_handler>`-prefixed) composes with the guard by construction — **no `AgentParams` threading, no `engine_dispatched` flag, no guard edit.** It also explains *why* #1571 is 0/5 even when it "succeeds": the rewritten prefix silently disables the guard's catch, so the LLM can no-op the dispatch with nothing to stop it. Engine-side spawn makes LLM compliance irrelevant — the correct fix.
- **C5 — `Dispatched` variant disposition (honors F4).** Per `is_ready_label_dispatch_marker`'s `starts_with` semantics, the F4 "guard inspects the pre-digest for the `engine_dispatched` marker" mechanism is **unnecessary** (the prefix already handles non-firing). Keep the `VerdictAction::Dispatched { pre_digest, task_id }` variant per F4's "new variant" option — it carries clarity (spawned vs prescriptive) and the task_id for logging — but implement guard non-firing **by construction (prefix)**, not by a marker-check. This *completes* F4's plain intent (guard must not fire on engine-dispatch); it does not overturn F4's design. Handle `Dispatched` at `handlers.rs:858` exactly like `Handled` (replace `req.text`); the other three handler match sites (`verdict_action`, `ci_success_action`, `ci_failure_action`) need the new arm for exhaustiveness, treated as `Passthrough { enrichment: None }` (they never construct it).

---

## Summary

Move the `run_claude_pilot` / `run_claude_pilot_groom` dispatch invocation from the LLM's tool-call decision into the engine itself. The ready-label handler (`server/ready_label_handler.rs`) currently builds a prescriptive pre-digest naming the exact tool + args + pre-created `task_id`, but the LLM still owns the final tool call — and empirical evidence (n≥6 stuck-dispatch incidents, 0/5 post-#1571 dispatches) confirms it doesn't comply. This ticket eliminates the LLM decision entirely: the handler spawns the subprocess directly, then informs the LLM post-facto.

## Acceptance Criteria

1. Engine-side dispatch invocation — handler calls `spawn_long_running_exec` directly
2. Pre-digest semantics shift — from "make this call" to "dispatch fired, acknowledge and EndTurn"
3. INTENT_GUARDS interaction — guard skips when engine already dispatched
4. Test coverage — integration test for the full flow
5. Empirical validation — stuck tickets dispatch successfully

## Architecture

### Key Insight

The current flow:
```
webhook → ready_label_handler (pre-digest) → LLM turn → LLM calls run_claude_pilot → executor
```

The new flow:
```
webhook → ready_label_handler → executor (direct spawn) → LLM turn (post-dispatch ack only)
```

The handler already resolves everything needed for dispatch: target tool name, skill name, dispatch class, task_id, issue URL. The missing piece is invoking `spawn_long_running_exec` directly instead of asking the LLM to do it.

### Fallback Path

Per the ticket's Failure-disposition section (Resolution F3): if engine-side spawn fails (tool not found, slot occupied, `validate_dispatch_readiness` rejection), the handler falls back to the #1571 prescriptive pre-digest path. The engine-side dispatch is an upgrade, not a replacement of the fallback.

## Implementation Steps

### Step 1: Add `resolve_tool_by_name` to SkillRegistry (~20 lines)

**File:** `crates/mika-agent/src/skills/mod.rs`

Per KTD-1 from the ticket, add a thin public method to `SkillRegistry`:

```rust
/// Resolve a skill tool by its tool name (e.g., `"run_claude_pilot"`).
/// Returns the first matching `ResolvedSkillTool` from the loaded entries.
pub fn resolve_tool_by_name(&self, name: &str) -> Option<ResolvedSkillTool> {
    for entry in &self.entries {
        if let Some(tools) = entry.load_tools() {
            for tool in tools {
                if tool.definition.name == name {
                    return Some(tool);
                }
            }
        }
    }
    None
}
```

This is the same resolution path the agent loop uses via `inject_skills_and_resolve_tools`, but exposed for direct engine-side use.

**Note:** `SkillEntry::load_tools()` returns `Option<Vec<ResolvedSkillTool>>`. Need to verify the exact method name — may be `tools()` or similar. The key is iterating the entry's tools and matching by `definition.name`.

### Step 2: Extend `VerdictAction` with `Dispatched` variant (~15 lines)

**File:** `crates/mika-agent/src/server/verdict_handler.rs`

Add a new variant per Resolution F4:

```rust
pub enum VerdictAction {
    Handled { pre_digest: String },
    Passthrough { enrichment: Option<String> },
    /// The handler spawned a dispatch subprocess directly.
    /// The pre-digest informs the LLM of the action taken.
    /// `engine_dispatched` signals the INTENT_GUARDS to skip.
    Dispatched { pre_digest: String, task_id: String },
}
```

The `Dispatched` variant carries both the post-dispatch pre-digest (for the LLM) and the `task_id` (for the intent guard to verify). This is structurally distinct from `Handled` to signal the engine-side dispatch semantics to `run_agent_for_message`.

### Step 3: Engine-side dispatch in `try_handle_ready_label_dispatch` (~100 lines)

**File:** `crates/mika-agent/src/server/ready_label_handler.rs`

The function signature gains the `SkillRegistry` parameter:

```rust
pub async fn try_handle_ready_label_dispatch(
    text: &str,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    _message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
    skills: &SkillRegistry,  // NEW
) -> VerdictAction
```

After step 7 (task pre-creation + audit event), insert the engine-side dispatch attempt:

```rust
// 8. Attempt engine-side dispatch (mika#1572).
// Resolve the target tool from the agent's loaded SkillRegistry.
let skill_tool = match skills.resolve_tool_by_name(target_tool) {
    Some(t) => t,
    None => {
        warn!(
            event = "ready_label_tool_not_found",
            target_tool,
            "ready_label_handler: target tool not in SkillRegistry — fallback to pre-digest"
        );
        // Fallback to prescriptive pre-digest (#1571 path)
        return VerdictAction::Handled {
            pre_digest: format_ready_label_pre_digest(
                &location, is_groomed, target_tool, target_skill, &task_id,
            ),
        };
    }
};

// Run validate_dispatch_readiness before spawning.
// Re-use the existing gate to check slot availability, grooming markers, etc.
let dispatch_input = serde_json::json!({
    "skill": target_skill,
    "prompt": format!("{}#{}", location.owner_repo(), location.number),
    "task_id": task_id,
});
if let Err(rejection) = validate_dispatch_readiness(
    db,
    &task_id,
    github_token,
    Some(&dispatch_input),
    Some(text),  // originating_message for unauthorized webhook check
).await {
    warn!(
        event = "ready_label_dispatch_readiness_failed",
        task_id = %task_id,
        rejection = %rejection,
        "ready_label_handler: dispatch readiness check failed — fallback to pre-digest"
    );
    // Fallback to prescriptive pre-digest (#1571 path)
    return VerdictAction::Handled {
        pre_digest: format_ready_label_pre_digest(
            &location, is_groomed, target_tool, target_skill, &task_id,
        ),
    };
}

// Create the callback child task (mirrors execute_long_running logic)
let callback_task = NewTask { /* ... same shape as execute_long_running ... */ };
let callback_task_id = match db.create_task(callback_task).await { ... };

// Auto-transition parent task to in_progress
let _ = db.update_task_status_if_pending(&task_id, "in_progress").await;

// Resolve command path and spawn
let command = skill_tool.handler.command().unwrap_or("handler.sh");
let cmd_path = skill_tool.skill_dir.join(command);
let enriched_input = /* add __mika_task_id, __mika_agent */;
spawn_long_running_exec(
    cmd_path,
    skill_tool.skill_dir.clone(),
    enriched_input,
    callback_task_id.clone(),
    db.clone(),
    github_token.map(|s| s.to_string()),
);

// Return Dispatched with post-dispatch pre-digest
VerdictAction::Dispatched {
    pre_digest: format_engine_dispatch_pre_digest(
        &location, is_groomed, target_tool, target_skill, &task_id,
    ),
    task_id: task_id.clone(),
}
```

**Critical details:**

- The callback task creation must mirror `execute_long_running` (lines 1923-1982 of executor.rs) — same fields: `trigger_type: "callback"`, `action_type: "resume_agent"`, `dispatch_class`, `timeout_at`, `parent_task_id`, `action_config` with input fields. This is extracted into a shared helper (Step 3b).
- `spawn_long_running_exec` must be made `pub(crate)` (currently private).
- `validate_dispatch_readiness` must be made `pub(crate)` (currently private).

### Step 3b: Extract callback task creation into a shared helper (~40 lines)

**File:** `crates/mika-agent/src/skills/executor.rs`

Extract the callback task creation logic (lines 1923-1982) into a `pub(crate)` function:

```rust
pub(crate) fn build_callback_task(
    agent_id: &str,
    parent_task_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    timeout_secs: u64,
    session_id: &str,
    trace_id: &str,
) -> NewTask { ... }
```

Both `execute_long_running` and `try_handle_ready_label_dispatch` call this helper. This avoids duplicating the callback task shape.

Also make `spawn_long_running_exec` and `validate_dispatch_readiness` `pub(crate)`:

```rust
pub(crate) fn spawn_long_running_exec(...) { ... }
pub(crate) async fn validate_dispatch_readiness(...) -> Result<String, String> { ... }
```

### Step 4: Add `format_engine_dispatch_pre_digest` (~30 lines)

**File:** `crates/mika-agent/src/server/ready_label_handler.rs`

New function for the post-dispatch pre-digest (AC2 semantics shift):

```rust
fn format_engine_dispatch_pre_digest(
    loc: &ReadyLabelLocation,
    is_groomed: bool,
    target_tool: &str,
    target_skill: &str,
    task_id: &str,
) -> String {
    format!(
        "<ready_label_handler>\n\
         [GitHub] Issue labeled ready on {}#{} — ENGINE-SIDE DISPATCH COMPLETE.\n\n\
         The engine has already spawned {} with skill={}, task_id={}.\n\
         The subprocess is running in the background.\n\n\
         You do NOT need to call {} — the dispatch is already running.\n\
         Optionally call `send_message` to acknowledge to the operator, then EndTurn.\n\
         </ready_label_handler>",
        loc.owner_repo(), loc.number,
        target_tool, target_skill, task_id,
        target_tool,
    )
}
```

### Step 5: Handle `Dispatched` in `run_agent_for_message` (~10 lines)

**File:** `crates/mika-agent/src/server/handlers.rs`

At the `ready_label_action` match site (lines 858-868), add the `Dispatched` arm:

```rust
match ready_label_action {
    VerdictAction::Handled { pre_digest } => {
        req.text = pre_digest;
    }
    VerdictAction::Dispatched { pre_digest, task_id } => {
        req.text = pre_digest;
        // Set flag for INTENT_GUARDS to recognize engine-side dispatch
        engine_dispatched_task_id = Some(task_id);
    }
    VerdictAction::Passthrough { enrichment: Some(e) } => {
        req.text = format!("{e}{}", req.text);
    }
    VerdictAction::Passthrough { enrichment: None } => {}
}
```

The `engine_dispatched_task_id` is threaded through `AgentParams` to the agent loop (Step 6).

**Also update:** The other handlers' match sites (`verdict_action`, `ci_success_action`, `ci_failure_action`) need exhaustive match updates for the new `Dispatched` variant — they treat it as `Passthrough { enrichment: None }` since they never return it.

### Step 6: Thread `engine_dispatched` through `AgentParams` to the guard (~15 lines)

**File:** `crates/mika-agent/src/agent.rs` (or wherever `AgentParams` is defined)

Add field:

```rust
pub struct AgentParams {
    // ... existing fields ...
    /// Set when the ready-label handler dispatched directly (mika#1572).
    /// The INTENT_GUARD `webhook_ready_label_dispatch` skips when this is set.
    pub engine_dispatched_task_id: Option<String>,
}
```

**File:** `crates/mika-agent/src/server/handlers.rs`

Pass `engine_dispatched_task_id` when constructing `AgentParams` for the `run_agent` call.

**File:** `crates/mika-agent/src/agent_loop/mod.rs`

In the `webhook_ready_label_dispatch` trigger check, add an early-return:

```rust
// In the INTENT_GUARDS evaluation loop:
if guard.label == "webhook_ready_label_dispatch"
    && params.engine_dispatched_task_id.is_some()
{
    // Engine already dispatched — guard is satisfied by construction
    continue;
}
```

**Alternative (simpler):** Instead of threading through AgentParams, detect engine dispatch from the pre-digest text itself. The pre-digest for `Dispatched` contains `"ENGINE-SIDE DISPATCH COMPLETE"` — the trigger predicate (`is_ready_label_dispatch_marker`) checks the _original_ user message, but after handler processing `req.text` is replaced with the pre-digest. Check: does the INTENT_GUARD's trigger function see the original or replaced text?

Looking at the code: the user message passed to `run_agent` is the modified `req.text` (post-handler). The INTENT_GUARD trigger checks `user_input_text` which is the `user_message` from params. So the trigger **will see the pre-digest**, not the original marker.

**Key finding:** The `ready_label_dispatch_trigger` checks `is_ready_label_dispatch_marker(msg)` which checks `msg.starts_with(READY_LABEL_DISPATCH_MARKER)` where `READY_LABEL_DISPATCH_MARKER = "[GitHub] Issue labeled ready on "`. The `Dispatched` pre-digest starts with `<ready_label_handler>` — **the trigger won't fire**.

This means: **no explicit engine_dispatched flag is needed**. The `Dispatched` variant's pre-digest naturally doesn't match the trigger predicate, so the guard won't fire. The INTENT_GUARD composition works by construction.

**However**, we need to verify this is safe. If the trigger doesn't fire, the LLM could EndTurn without any tool call, and that's correct — the dispatch already happened. The `webhook_zero_tools` guard (entry c) could fire though, since `[GitHub]` is in the message... but wait, the pre-digest starts with `<ready_label_handler>`, not `[GitHub]`. So `webhook_zero_tools` won't fire either.

**Conclusion:** The pre-digest format already handles AC3 naturally. No `AgentParams` threading needed. The pre-digest starts with `<ready_label_handler>`, which doesn't match any INTENT_GUARD trigger predicate. The LLM gets the post-dispatch message, can optionally `send_message`, and EndTurn cleanly.

**Revised Step 6:** Remove AgentParams threading. Instead, add a unit test verifying that the `Dispatched` pre-digest doesn't trigger any INTENT_GUARD.

### Step 7: Update callers of `try_handle_ready_label_dispatch` (~5 lines)

**File:** `crates/mika-agent/src/server/handlers.rs`

Pass `skills` to the handler:

```rust
let ready_label_action = super::ready_label_handler::try_handle_ready_label_dispatch(
    &req.text,
    &a.db,
    verdict_github_token.as_deref(),
    Some(&sender_arc),
    &session_id,
    &req.request_id,
    &skills,  // NEW
)
.await;
```

### Step 8: Tests (~80 lines)

**File:** `crates/mika-agent/src/server/ready_label_handler.rs` (unit tests)

1. **`engine_dispatch_pre_digest_does_not_trigger_intent_guard`** — Verify that the `Dispatched` pre-digest text does NOT match `is_ready_label_dispatch_marker()` or `is_unauthorized_webhook_dispatch()`. This is the structural guarantee for AC3.

2. **`engine_dispatch_pre_digest_names_task_id`** — Verify the post-dispatch pre-digest contains the task_id and "ENGINE-SIDE DISPATCH COMPLETE".

3. **`fallback_pre_digest_on_tool_not_found`** — Verify that when `resolve_tool_by_name` returns None, the handler falls back to the prescriptive pre-digest (same shape as #1571).

**File:** `crates/mika-agent/tests/eval/` (integration test, if feasible)

4. **Integration test with `MockLlmProvider`** — Inject a `[GitHub] Issue labeled ready on senara-solutions/mika#1234` message and assert:
   - The handler returns `VerdictAction::Dispatched`
   - The LLM turn's pre-digest reflects the post-dispatch shape
   - The INTENT_GUARD `webhook_ready_label_dispatch` does not fire

**Note:** Full integration test may be complex due to the need for a loaded SkillRegistry with dev-pilot/dev-groom tools. A unit test proving the pre-digest shape + guard non-triggering is sufficient for AC4's core assertion.

## Files Modified

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/mod.rs` | Add `resolve_tool_by_name` to `SkillRegistry` |
| `crates/mika-agent/src/server/verdict_handler.rs` | Add `Dispatched` variant to `VerdictAction` |
| `crates/mika-agent/src/server/ready_label_handler.rs` | Engine-side dispatch logic + `format_engine_dispatch_pre_digest` + tests |
| `crates/mika-agent/src/server/handlers.rs` | Handle `Dispatched` variant + pass `skills` to handler |
| `crates/mika-agent/src/skills/executor.rs` | Make `spawn_long_running_exec`, `validate_dispatch_readiness` pub(crate); extract callback task builder |

## Risks and Mitigations

1. **Callback task shape drift.** The engine-side dispatch must create callback tasks with the exact same shape as `execute_long_running`. Mitigated by extracting into a shared helper (Step 3b).

2. **validate_dispatch_readiness may reject legitimately.** The dispatch-readiness gate checks 7 conditions including slot availability and grooming markers. The handler already checked grooming markers (step 5 of the handler), but slot availability is only known at dispatch time. Fallback to #1571 pre-digest path on rejection (Resolution F3).

3. **Pre-digest shape changes break INTENT_GUARD composition.** The `Dispatched` pre-digest must NOT start with `[GitHub] Issue labeled ready on` — verified by unit test (Step 8.1).

## Out of Scope

- Changes to `validate_dispatch_readiness` itself
- Changes to `is_unauthorized_webhook_dispatch`
- Migration of other webhook-triggered dispatches (CI failure, verdict block)
- Changes to the fallback prescriptive pre-digest format
