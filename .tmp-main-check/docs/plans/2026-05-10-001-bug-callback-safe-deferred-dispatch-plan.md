# Plan: Callback-Safe Long-Running Dispatch Path (mika#1058)

type: bug
issue: mika#1058
branch: bug/1058/claude-pilot-session-exits-success-after

## Problem

Two failure modes prevent pipeline retry from working:

**Mode A — Model early exit (behavioral).** Claude-pilot session exits Success after `mika ask --agent mika-arch` returns without retrying the groom. The model doesn't attempt `run_claude_pilot` at all. This is a prompt/behavioral issue — out of scope for this ticket.

**Mode B — Callback gate rejection (mechanical).** When mika-dev's pipeline-retry path fires on a `PIPELINE FAILURE` callback, the retry attempts `run_claude_pilot` directly. This fails at `executor.rs:278-296` because callback turns have `long_running_ctx = None`. The gate is an `Option` check — not a behavioral flag — and cannot be overcome by prompt changes.

**Session classification:**

| Session | claude-pilot ID | Mode | Evidence |
|---|---|---|---|
| mika-platform#96 | `582cc389` | A (silent slip) | HEAD changed; no marker fired, no gate rejection in logs |
| mika#1052 | `a9435e3d` | B (gate rejection) | Marker fired → retry attempted → gate blocked → manual recovery |
| mika-skills#159 | `239b6320` | B (gate rejection) | Marker fired → retry attempted → gate blocked → manual recovery |
| mika-skills#163 | `00e01562` | B (gate rejection) | Marker fired → retry attempted → gate blocked → manual recovery |

**This plan fixes Mode B** (3/4 sessions, the dominant failure). Mode A requires separate prompt-level mitigation — filed as follow-up if recurrence warrants it.

**Latent bug discovered during planning:** The existing `DeferredDispatch` mechanism (mika#1011) has the same gate issue. `DeferredDispatch` turns fire as silent mode with `long_running_ctx = None` (agent.rs line 3339: `None, // long_running not supported in silent mode`), but the `deferred_dispatch_action` INTENT_GUARD requires the LLM to call `run_claude_pilot`. The LLM tries, gets blocked by the gate, correction fires, blocked again → max steps exceeded. This plan fixes both the callback retry case AND the existing DeferredDispatch path.

## Pinned Source (Phase 0)

### 1. Executor gate (`executor.rs:274-296`)

```rust
// Lines 274-296 — Refuse long-running tools when no long-running context is available
// (callback turns, silent mode, CLI test). The sync exec path does not
// inject __mika_task_id/__mika_agent, so the handler would crash with
// a cryptic error. Return an explicit error instead (#537).
if matches!(
    &skill_tool.handler,
    ToolHandler::Exec {
        long_running: true,
        ..
    }
) && long_running_ctx.is_none()
{
    warn!(
        tool = %skill_tool.definition.name,
        "long-running tool invoked without long_running_ctx"
    );
    return ToolOutput::error(format!(
        "Tool '{}' is declared long_running but cannot run in the current context \
         (callback turn, silent mode, or CLI test). Long-running tools require a \
         conversation-mode turn with an active task engine.",
        skill_tool.definition.name
    ));
}
```

The gate checks `long_running_ctx.is_none()`. `LongRunningContext` is a struct carrying DB, agent name, session/trace IDs, and a dispatch counter.

### 2. `LongRunningContext` struct (`executor.rs:93-101`)

```rust
pub struct LongRunningContext {
    pub db: AsyncDatabase,
    pub agent_name: String,
    pub session_id: String,
    pub trace_id: String,
    /// Per-turn dispatch counter (#583). Only one long-running dispatch is
    /// permitted per agent turn. Atomic for interior mutability through `&self`.
    pub dispatch_count: AtomicU32,
}
```

### 3. Silent mode `long_running_ctx` (agent.rs line 3339)

```rust
let result = run_loop(
    llm,
    tools,
    &skill_tool_map,
    skill_timeout,
    &tool_ctx,
    &mut request,
    &mode,
    params.session_id,
    db,
    None, // MCP tools excluded from silent mode
    None, // long_running not supported in silent mode     <--- THE BUG
    &no_required_tools,
    // ...
```

ALL silent triggers — including `DeferredDispatch` — pass `None` here. This means the existing `DeferredDispatch` mechanism cannot execute `run_claude_pilot`.

### 4. `ToolContext` struct (`tools/mod.rs:94-142`)

```rust
pub struct ToolContext<'a> {
    pub db: &'a AsyncDatabase,
    pub session_id: &'a str,
    pub trace_id: &'a str,
    pub home_dir: &'a Path,
    pub global_home_dir: Option<&'a Path>,
    pub core_memory_edit_count: &'a AtomicU32,
    pub is_onboarding: bool,
    pub message_sender: Option<Arc<dyn MessageSender>>,
    pub embedding_client: Option<&'a EmbeddingClient>,
    pub brave_api_key: Option<&'a str>,
    pub github_token: Option<&'a str>,
    pub skills_dirty: &'a AtomicBool,
    pub is_reflection: bool,
    pub is_task_context: bool,
    pub is_callback_turn: bool,          // <--- exists, set to true for Callback/PostCallbackAdvance/DeferredDispatch
    pub provider_name: &'a str,
    pub model_name: &'a str,
    pub active_skill_paths: &'a [SkillPathInfo],
    pub max_tasks_per_session: i64,
    pub pr_review_posted: &'a AtomicBool,
    pub pr_reviews_posted: Option<&'a Arc<DashMap<String, HashSet<String>>>>,
}
```

`is_callback_turn` already exists but carries no task_id information. Decision: **add `callback_task_id: Option<&'a str>`** rather than re-derive from `is_callback_turn`. Rationale: the cycle detection walk needs the parent task ID to start from, and `is_callback_turn` doesn't carry it. A separate DB lookup to find "the callback task for this session" would be fragile — the `SilentTrigger` already has the `task_id`. Threading it through is 5 lines of code.

### 5. `register_deferred_callback()` (`executor.rs:893-960`)

```rust
async fn register_deferred_callback(
    db: &AsyncDatabase,
    task_id: &str,
    input: &serde_json::Value,
) -> bool {
    // Flood-cap check: reject without insert if at capacity (MAX_PENDING_DEFERRED_CALLBACKS = 10)
    // ...
    let action_config = serde_json::json!({
        "trigger_kind": "deferred_dispatch",
        "original_call": input,
    }).to_string();

    let task = NewTask {
        agent_id: db.agent_id().to_string(),
        parent_task_id: Some(task_id.to_string()),
        depth: 0,
        label: DEFERRED_DISPATCH_LABEL.to_string(),    // "long_running:run_claude_pilot:deferred"
        trigger_type: "callback",
        action_type: "resume_agent",
        // ...
    };
    db.create_task(task).await  // returns true on success, false on error
}
```

Takes `task_id` (parent) and `input` (the `run_claude_pilot` arguments). Currently called only from `validate_dispatch_readiness` when `global_dispatch_active` fires.

### 6. DeferredDispatch promotion (`dispatcher.rs:464-474, 954-966`)

```rust
// Lines 464-474 — after mark_task_delivered:
if task.label != crate::agent::DEFERRED_DISPATCH_LABEL {
    // Only drain from non-deferred callback completions to prevent
    // cascading deferred dispatches from draining the whole queue
    // in a single call stack (each deferred turn drains one more).
    self.dispatch_next_deferred_callback().await;
}

// Lines 954-966 — promote_next_deferred_callback():
// UPDATE tasks SET status='completed', next_fire_at=now
// WHERE id = (SELECT id FROM tasks WHERE ... status='pending'
//   AND label='long_running:run_claude_pilot:deferred' ORDER BY created_at ASC LIMIT 1)
```

**Promotion timing resolution (F3):** Promotion fires at line 464-474 when ANY non-deferred callback completes. In the pipeline-retry case:
1. claude-pilot fails → delivers callback result to parent
2. `dispatch_resume_agent` fires for the callback task
3. `run_silent_agent` runs the callback turn (this is where the LLM retries)
4. During callback turn, LLM calls `run_claude_pilot` → gate fires → `register_deferred_callback` enqueues
5. Callback turn ends
6. Back in `dispatch_resume_agent`: `mark_task_delivered` (line 454)
7. Line 469: `task.label != DEFERRED_DISPATCH_LABEL` → true (this is a regular callback) → enters block
8. Line 473: `dispatch_next_deferred_callback()` → promotes the just-registered deferred callback

**Conclusion:** Promotion fires correctly. The current callback is not a deferred callback, so it passes the label check and drains the deferred queue. No timing issue.

## Design

### Two-part fix

**Part 1: Inject `long_running_ctx` for DeferredDispatch silent turns** — Fix the existing latent bug. When the trigger is `DeferredDispatch`, construct a `LongRunningContext` and pass it to `run_loop` instead of `None`. The DeferredDispatch turn already has all the context needed: `db`, `agent_name`, `session_id`, `trace_id`.

**Part 2: Gate-intercept for callback turns** — When the executor gate rejects a long-running tool from a callback turn, detect callback context via `callback_task_id` on `ToolContext`, run cycle detection, and call `register_deferred_callback()`. Return structured success (not error) so the LLM knows the retry was enqueued.

### Surface choice: Gate-intercept for callback turns (no new tool)

The ticket says surface choice is for mika-arch to settle. Three options:

| Option | Pros | Cons |
|--------|------|------|
| New `enqueue_deferred_dispatch` tool | Explicit LLM control | New tool in registry, prompt space cost, callback turns need tool registered |
| Flag on `run_claude_pilot` | No new tool | Union-enum `skill` param already complex; flag semantics depend on context |
| **Gate-intercept (chosen)** | Zero new tools; engine handles transparently; wraps existing DeferredDispatch | LLM must understand the deferred response shape |

Gate-intercept wraps `DeferredDispatch` (requirement 1), fires at the callback detection point (requirement 2), and can enforce cycle detection before enqueue (requirement 3).

### Cycle Detection

Lineage walk on `(repo, issue_number, skill)` tuple, bounded by `depth ≤ 3` (max 4 hops).

**Fail-open on extraction:** If `(repo, issue_number, skill)` can't be extracted from an ancestor task's metadata/action_config, that ancestor is skipped. This is theoretical — `register_deferred_callback` always stores `original_call` with the full tool input including `skill` and `prompt` (repo#number). Parent manual tasks from `create_task` use `reference_url` (GitHub URL) and `source` ("self_dev") — different fields but parseable. Worst case if extraction fails: one extra dispatch that hits the `depth ≤ 3` schema CHECK constraint, which is a hard structural limit producing an INSERT rejection.

## Changes

### 1. `crates/mika-agent/src/agent.rs` — Inject `long_running_ctx` for DeferredDispatch

**Line 3339:** Replace unconditional `None` with conditional construction:

```rust
// Construct LongRunningContext for DeferredDispatch triggers only.
// DeferredDispatch turns MUST be able to call run_claude_pilot — that's their
// sole purpose. All other silent triggers (Heartbeat, Callback, etc.) keep None.
let long_running_ctx = if matches!(&params.trigger, SilentTrigger::DeferredDispatch { .. }) {
    Some(executor::LongRunningContext {
        db: db.clone(),
        agent_name: db.agent_id().to_string(),
        session_id: params.session_id.to_string(),
        trace_id: trace_id.clone(),
        dispatch_count: AtomicU32::new(0),
    })
} else {
    None
};

// In run_loop call:
long_running_ctx.as_ref(), // was: None
```

### 2. `crates/mika-agent/src/tools/mod.rs` — Add `callback_task_id` to `ToolContext`

```rust
pub struct ToolContext<'a> {
    // ... existing fields ...
    /// The callback task ID, if this turn is processing a callback result.
    /// Used by the executor gate to register deferred dispatches from callback context.
    /// Set from SilentTrigger::Callback { task_id, .. }. None for non-callback contexts.
    pub callback_task_id: Option<&'a str>,
}
```

### 3. `crates/mika-agent/src/agent.rs` — Thread `callback_task_id`

In `run_silent_inner` (around line 3226), extract task_id from the trigger:

```rust
let callback_task_id = match &params.trigger {
    SilentTrigger::Callback { task_id, .. } => Some(task_id.as_str()),
    _ => None,
};

let tool_ctx = ToolContext {
    // ... existing fields ...
    callback_task_id,
};
```

In `run_agent_inner` (conversation mode) and `run_team_agent` — pass `callback_task_id: None`.

### 4. `crates/mika-agent/src/skills/executor.rs` — Gate enhancement

Replace lines 274-296 with:

```rust
if matches!(
    &skill_tool.handler,
    ToolHandler::Exec { long_running: true, .. }
) && long_running_ctx.is_none()
{
    // Callback turns: attempt deferred dispatch registration instead of hard error.
    // The deferred callback fires as a DeferredDispatch silent turn (Part 1)
    // which HAS long_running_ctx injected.
    if let Some(task_id) = ctx.callback_task_id {
        // Cycle detection: walk parent_task_id chain, compare (repo, issue_number, skill).
        match check_lineage_cycle(&ctx.db, task_id, &input).await {
            Ok(()) => {
                if register_deferred_callback(ctx.db, task_id, &input).await {
                    info!(
                        tool = %skill_tool.definition.name,
                        task_id,
                        "callback_deferred_dispatch_registered"
                    );
                    return ToolOutput::success(serde_json::json!({
                        "status": "deferred",
                        "message": "Long-running dispatch registered as deferred callback. \
                                    It will fire automatically when the current dispatch \
                                    slot is free. Do not retry.",
                        "deferred": true
                    }).to_string());
                }
                // Fall through to error if registration failed (cap exceeded / DB error)
            }
            Err(cycle_msg) => {
                warn!(
                    tool = %skill_tool.definition.name,
                    task_id,
                    "deferred_dispatch_cycle_detected"
                );
                return ToolOutput::error(serde_json::json!({
                    "error": "deferred_dispatch_cycle_detected",
                    "message": cycle_msg,
                }).to_string());
            }
        }
    }

    // Original error for non-callback contexts (heartbeat, reflection, CLI test)
    warn!(
        tool = %skill_tool.definition.name,
        "long-running tool invoked without long_running_ctx"
    );
    return ToolOutput::error(format!(
        "Tool '{}' is declared long_running but cannot run in the current context \
         (callback turn, silent mode, or CLI test). Long-running tools require a \
         conversation-mode turn with an active task engine.",
        skill_tool.definition.name
    ));
}
```

### 5. `crates/mika-agent/src/skills/executor.rs` — Cycle detection

```rust
/// Check for cycles in the task lineage before enqueuing a deferred dispatch.
///
/// Walks the parent_task_id chain (max 4 hops, bounded by depth ≤ 3 schema CHECK).
/// Extracts (repo, issue_number, skill) from each ancestor's metadata and compares
/// against the proposed dispatch. Returns Ok(()) if safe, Err(message) if cycle detected.
///
/// Fail-open: if metadata extraction fails for an ancestor, that ancestor is skipped.
/// The depth cap is the structural backstop.
async fn check_lineage_cycle(
    db: &AsyncDatabase,
    parent_task_id: &str,
    proposed_input: &serde_json::Value,
) -> Result<(), String> {
    let proposed_skill = proposed_input.get("skill").and_then(|v| v.as_str());
    let proposed_prompt = proposed_input.get("prompt").and_then(|v| v.as_str());
    // Parse "repo#number" from prompt
    let (proposed_repo, proposed_issue) = parse_repo_issue(proposed_prompt);

    let mut current_id = parent_task_id.to_string();
    for _depth in 0..4 {
        let task = match db.get_task_unscoped(&current_id).await {
            Ok(Some(t)) => t,
            _ => break, // task not found or DB error → stop walking (fail-open)
        };

        // Extract (repo, issue, skill) from this ancestor
        if let Some((ancestor_repo, ancestor_issue, ancestor_skill)) = extract_dispatch_tuple(&task) {
            if proposed_skill == Some(ancestor_skill.as_str())
                && proposed_repo == Some(ancestor_repo.as_str())
                && proposed_issue == Some(ancestor_issue)
            {
                return Err(format!(
                    "Cycle detected: ancestor task {} has same dispatch tuple \
                     ({}, #{}, skill={}). Refusing to enqueue.",
                    task.id, ancestor_repo, ancestor_issue, ancestor_skill
                ));
            }
        }

        // Walk up
        match task.parent_task_id {
            Some(pid) => current_id = pid,
            None => break,
        }
    }
    Ok(())
}

/// Parse "repo#number" format from a prompt string.
fn parse_repo_issue(prompt: Option<&str>) -> (Option<&str>, Option<i64>) {
    // ... parse "mika#159" → (Some("mika"), Some(159))
}

/// Extract (repo, issue_number, skill) tuple from a task's metadata/action_config.
fn extract_dispatch_tuple(task: &Task) -> Option<(String, i64, String)> {
    // Try action_config.original_call first (deferred callbacks store this)
    // Then try task metadata fields
    // Then try reference_url parsing (manual tasks)
}
```

### 6. `skills/bundled/self-dev/system_prompt.md` — Pipeline retry update

Update the "On pipeline failure" section (line 184-185):

**Current:**
> Then call `run_claude_pilot` with the same `repo#number` and `task_id` (handler reuses existing worktree). Wait for callback and re-enter this entry point.

**New:**
> Then call `run_claude_pilot` with the same `repo#number` and `task_id`. If the call returns `{"status": "deferred", "deferred": true}`, the retry has been automatically enqueued and will fire as a fresh session — do NOT retry again. Proceed to Step 6 with status `in_progress` and note "pipeline retry deferred — engine will auto-dispatch when dispatch slot is free."

**Byte budget:** 64,732 / 65,536 bytes. Net change: +120 bytes. Remaining: ~684 bytes. Tight — follow-up prompt trim recommended but not blocking for this PR.

### 7. Tests

**7a. Unit test: Cycle detection rejects same-tuple lineage**
- Create task chain: parent with metadata containing `(mika, 159, dev-groom)` → child callback
- Call `check_lineage_cycle` with proposed `(mika, 159, dev-groom)`
- Assert: returns `Err` with cycle message

**7b. Unit test: Cycle detection allows cross-skill chains**
- Create task chain: parent with `(mika, 159, dev-groom)` → child callback
- Call `check_lineage_cycle` with proposed `(mika, 159, dev-pilot)`
- Assert: returns `Ok(())`

**7c. Unit test: Existing gate preserved for non-callback contexts**
- Call `execute_skill_tool` with `long_running_ctx = None` and no `callback_task_id`
- Assert: original error message (not deferred)

**7d. Unit test: DeferredDispatch turn with `long_running_ctx` injected**
- Construct `LongRunningContext` for a DeferredDispatch trigger
- Verify `long_running_ctx.is_some()` passes the gate

**7e. Unit test: Depth cap interaction**
- Create 3-deep lineage (depth=3), attempt enqueue at depth 4
- Assert: schema CHECK rejects regardless of lineage walk result

## Sequence Diagrams

### Callback retry (new — fixes Mode B)

```
PIPELINE FAILURE callback arrives
  → self-dev prompt: "retries remain, call run_claude_pilot"
  → LLM calls run_claude_pilot
  → executor.rs gate fires (long_running_ctx is None, callback turn)
    → callback_task_id is Some(task_id)
    → check_lineage_cycle → no cycle → Ok(())
    → register_deferred_callback(task_id, input) → pending callback created
    → Return: {"status": "deferred", "deferred": true}
  → LLM receives deferred confirmation, proceeds to EndTurn
  → Callback turn completes → mark_task_delivered
  → dispatch_next_deferred_callback() promotes the deferred callback (line 473)
  → Engine fires SilentTrigger::DeferredDispatch
  → Part 1: long_running_ctx = Some(LongRunningContext{...})  <-- NEW
  → run_claude_pilot executes normally (gate passes)
  → Fresh claude-pilot session → completion callback delivered
```

### DeferredDispatch turn (fixes existing latent bug)

```
global_dispatch_active fires in conversation mode
  → register_deferred_callback (existing mika#1011 path)
  → Blocking dispatch completes → promotes deferred callback
  → Engine fires SilentTrigger::DeferredDispatch
  → run_silent_inner:
      BEFORE: long_running_ctx = None → gate blocks → INTENT_GUARD retries → max steps
      AFTER:  long_running_ctx = Some(...) → gate passes → run_claude_pilot executes
```

## Out of Scope

Per ticket:
- `dispatch-lib.sh` post-flight check extensions
- Watchdog re-spawn from `dispatch-lib` (explicitly rejected)
- Verdict-text extraction from `messages` DB
- Lifting the gate for non-`DeferredDispatch` direct tool calls
- Mode A (model early exit) — separate prompt-level concern

## Risks

1. **`LongRunningContext` for DeferredDispatch turns:** Constructing this enables subprocess spawning from silent mode — previously blocked by design. Mitigated by: (a) only constructed for `DeferredDispatch` triggers, not all silent triggers; (b) `validate_dispatch_readiness` still enforces all 5 guards (task status, active callback, global dispatch, per-turn limit, blockedBy check); (c) the deferred callback is always a child of an existing task, so depth guards apply.

2. **Prompt budget:** 684 bytes remaining after this change. Not blocking but a follow-up prompt trim is recommended.

## Verification

Per acceptance criteria:
1. End-to-end: PIPELINE FAILURE callback → retry enqueued → original session closes → deferred dispatch fires → fresh claude-pilot session → callback delivered.
2. Cycle detection unit test with same-tuple lineage → rejection.
3. Existing gate preserved for non-callback, non-DeferredDispatch contexts.
4. Existing DeferredDispatch consumers work end-to-end (previously broken).
5. Self-dev prompt byte count under 65,536.
