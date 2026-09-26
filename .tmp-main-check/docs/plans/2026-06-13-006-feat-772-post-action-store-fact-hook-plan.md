---
ticket: mika#772
branch: feat/772/post-action-store-fact-hook
status: active
date: 2026-06-13
origin: https://github.com/senara-solutions/mika/issues/772
execution: code
---

# Plan: post-action `store_fact` hook on `update_task_status(completed)` (mika#772)

## Problem frame

mika-qa stores facts after every PR review (prompt directive: *"After posting the verdict, call `store_fact`"*). mika-dev does NOT for single-issue completions because `self-dev/system_prompt.md` has `store_fact` directives only inside milestone-workflow branches. Single-issue dispatches — the majority of mika-dev's work — never traverse those directives. Audit questions like "which PRs did mika-dev complete this week?" require reconstructing state from `tasks` + `audit_events` + GitHub.

The body's "why not a prompt fix" argues: the self-dev prompt is at 49,069/49,152 bytes (83 byte headroom); adding a directive forces shrinking another; position-dependent fragility; observability shouldn't depend on agent diligence. **The structural fix is engine-level.**

## Layer taxonomy clarification (per first-pass architect guidance)

`crates/mika-agent/CLAUDE.md` lists 11 post-condition guards on EndTurn. These run AFTER the LLM produces a text response, checking whether to accept or reject EndTurn. They are **gating** guards (reject → retry).

This ticket adds a different layer:

- **Post-action hook** — runs AFTER a specific tool call succeeds, BEFORE the tool result returns to the LLM. Side-effect only (emits `store_fact`); not gating. Failure is warn-and-continue.

These are architecturally distinct surfaces. The hook does NOT extend INTENT_GUARDS or the EndTurn post-condition chain. It sits inside the tool dispatch path, scoped to specific tools.

## Scope boundaries

- New post-action hook layer in `crates/mika-agent/src/tools/post_action_hooks.rs` (or similar — implementer-choice for the module name).
- One hook registered: fires after successful `update_task_status(status="completed")` on single-issue / callback tasks.
- Hook emits `store_fact(category="event", description=<structured template>)`.
- Milestone parent tasks are EXCLUDED in iteration 1 (existing prompt directives handle them; deferred to follow-up if/when the hook is generalized).
- **Out of scope for iteration 1:** retiring the milestone-workflow `store_fact` directives from `self-dev/system_prompt.md` (separate concern — once the hook is stable, that retirement is a follow-up ticket); hooks on `delivered` / `failed` / `cancelled` / `blocked` transitions; hooks on tools other than `update_task_status`.

## Implementation Units

### U1 — Post-action hook layer

**Goal:** A registration surface for post-tool-call side-effect hooks.

**Files:**
- Create: `crates/mika-agent/src/tools/post_action_hooks.rs`

**Approach:**

```rust
//! Post-action hooks (mika#772) — side-effect-only callbacks that fire after
//! specific tool calls succeed. NOT gating (failure is warn-and-continue).
//! NOT part of the EndTurn post-condition guard chain — see CLAUDE.md.

pub struct PostActionHook {
    pub tool_name: &'static str,
    pub fires_on: fn(&Value, &ToolOutput) -> bool,
    pub action: fn(&Value, &ToolOutput, &ToolContext<'_>) -> BoxFuture<'static, ()>,
}

pub const POST_ACTION_HOOKS: &[PostActionHook] = &[
    PostActionHook {
        tool_name: "update_task_status",
        fires_on: is_task_completion_on_dispatchable_task,
        action: emit_auto_fact_on_task_completion,
    },
];

pub async fn run_post_action_hooks(
    tool_name: &str,
    input: &Value,
    output: &ToolOutput,
    ctx: &ToolContext<'_>,
) {
    for hook in POST_ACTION_HOOKS.iter().filter(|h| h.tool_name == tool_name) {
        if (hook.fires_on)(input, output) {
            (hook.action)(input, output, ctx).await;
        }
    }
}
```

The hook layer is registry-driven (matches INTENT_GUARDS pattern at `crate::agent`). Future hooks add a new `PostActionHook` entry; the dispatch site loops over them.

**Test scenarios:**
- **Hook registry compiles + lints clean.**
- **`run_post_action_hooks` is no-op when no hook matches the tool_name.**
- **Hook predicate is consulted before action runs.**

**Verification:** unit tests in the new module.

### U2 — Trigger predicate: `is_task_completion_on_dispatchable_task`

**Goal:** Returns `true` when `update_task_status` succeeded with `status="completed"` on a single-issue or callback task (NOT a milestone parent).

**Files:**
- Modify: `crates/mika-agent/src/tools/post_action_hooks.rs`

**Approach:**

```rust
fn is_task_completion_on_dispatchable_task(input: &Value, output: &ToolOutput) -> bool {
    // (1) tool must have succeeded
    if !output.is_success() { return false; }
    // (2) status field must be "completed"
    if input.get("status").and_then(|v| v.as_str()) != Some("completed") { return false; }
    // (3) task must exist (validated by the tool — by here, the row was updated)
    // The hook's `action` will re-query for full task metadata; here we only filter
    // by what's available in the tool input.
    true
}
```

The milestone-parent exclusion happens inside the `action` (U3) where the full task row is available. Keeping the predicate cheap is the right shape — expensive DB calls live in the action.

**Test scenarios:**
- **`status="completed"` → true**
- **`status="blocked"` → false**
- **Tool output error → false**

**Verification:** unit tests.

### U3 — Action: `emit_auto_fact_on_task_completion`

**Goal:** Emits `store_fact` with the structured description from the issue body.

**Files:**
- Modify: `crates/mika-agent/src/tools/post_action_hooks.rs`

**Approach:**

```rust
async fn emit_auto_fact_on_task_completion(
    input: &Value,
    _output: &ToolOutput,
    ctx: &ToolContext<'_>,
) {
    let task_id = match input.get("task_id").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return,
    };

    // Fetch full task metadata to extract repo#issue, PR number, etc.
    let task = match ctx.db.get_task(task_id).await {
        Ok(Some(t)) => t,
        _ => return,
    };

    // Milestone parent exclusion (iteration 1 scope)
    if task.r#type == "milestone" || task.r#type == "project" {
        return;
    }

    let description = build_completion_fact_description(&task);

    // Emit via store_fact internal helper (NOT by calling the tool — that would
    // require re-dispatching through ToolRegistry. Use the same DB write path
    // store_fact tool uses internally).
    if let Err(e) = ctx.db.store_fact_event(&description).await {
        tracing::warn!(
            event = "auto_fact_emission_failed",
            task_id = %task_id,
            error = %e,
            "post-action auto-fact emission failed (non-blocking)"
        );
    }
}

fn build_completion_fact_description(task: &Task) -> String {
    let repo_issue = task.reference_url.as_deref().unwrap_or("(no ref)");
    let title = &task.label;
    let pr = task.metadata.pointer("/claude_pilot/pr_url")
        .and_then(|v| v.as_str())
        .unwrap_or("—");
    let turns = task.metadata.pointer("/claude_pilot/turns")
        .and_then(|v| v.as_u64())
        .map(|n| n.to_string())
        .unwrap_or_else(|| "—".to_string());
    let cost = task.metadata.pointer("/claude_pilot/cost_usd")
        .and_then(|v| v.as_f64())
        .map(|n| format!("${:.4}", n))
        .unwrap_or_else(|| "—".to_string());
    let verdict = derive_verdict(task); // merged | closed | abandoned | —
    let task_id_short = &task.id[..8.min(task.id.len())];

    format!(
        "Completed {repo_issue}: {title}. PR: {pr}. Turns: {turns}. Cost: {cost}. Verdict: {verdict}. Task: {task_id_short}."
    )
}
```

The `ctx.db.store_fact_event(...)` helper either already exists (used by the `store_fact` tool's internals) or needs a thin wrapper added — implementer verifies. The hook MUST NOT re-dispatch through `ToolRegistry::dispatch(...)` because that would risk recursive tool dispatch + audit_events double-counting.

**Test scenarios:**
- **Happy path — single-issue completion with claude_pilot metadata:** fact description includes repo#issue, PR URL, turns, cost.
- **Single-issue completion without claude_pilot metadata (hand-completed):** placeholders `—` populate cleanly.
- **Milestone parent task:** action exits silently; no fact emitted.
- **`store_fact_event` failure:** WARN log line; no panic; tool result unchanged.

**Verification:** unit tests + integration smoke (complete a task in a test harness, verify a fact row appears in `memory_facts`).

### U4 — Wire post-action hooks into tool dispatch

**Goal:** After every successful tool execution, the dispatch path calls `run_post_action_hooks`.

**Files:**
- Modify: `crates/mika-agent/src/tools/mod.rs` (the tool dispatch site, around the existing `ToolRegistry::dispatch`)

**Approach:** Find the tool dispatch site where a tool's `.execute(...)` returns its `ToolOutput`. AFTER the result is materialized but BEFORE returning it to the caller (i.e., before constructing the LLM-facing message), call:

```rust
post_action_hooks::run_post_action_hooks(tool_name, &input, &output, ctx).await;
```

This is one line at one site. The semantic guarantee per the issue body:
- Hook runs AFTER the tool's side effects committed (status update transaction is closed before the hook runs).
- Hook runs BEFORE the tool result is returned to the LLM (so the LLM cannot read state mid-hook; not strictly necessary for correctness here, but cleaner ordering).
- Hook failure does NOT propagate — even a panic in a hook would not corrupt the tool's success path (the hook's `action` is `async` and uses `tracing::warn!` on error).

**Test scenarios:**
- **Hook fires after tool success:** integration test confirms the auto-fact appears after `update_task_status(completed)`.
- **Hook does NOT fire on tool failure:** when `update_task_status` rejects with an invalid transition, no fact emitted.
- **Hook does NOT fire on other tools:** completing a `read_agent_file` call (irrelevant tool) doesn't trigger the registry.

**Verification:** integration tests + manual smoke.

### U5 — Docs note

**Goal:** Document the new layer in CLAUDE.md.

**Files:**
- Modify: `crates/mika-agent/CLAUDE.md` § Tools — add a subsection "Post-Action Hooks (mika#772)"

**Approach:** Short addition explaining: layer purpose, distinction from EndTurn guards, the registered hook for `update_task_status(completed)` → auto-fact, failure semantics, milestone-parent exclusion.

**Verification:** manual read.

## Dependencies / sequencing

- U1 → U2 → U3 → U4 (each builds on the prior); can interleave commits but final PR has all four
- U5 ships in same PR; last

## Patterns to follow (cross-cutting)

- `INTENT_GUARDS` pattern at `crate::agent` — registry-of-static-entries style
- Existing `store_fact` tool's DB write path — for the `store_fact_event` helper
- `tracing::warn!` with structured fields — for the failure log line

## Verification (top-level)

- `cargo test -p mika-agent` passes (existing + new tests)
- `cargo clippy --workspace` clean
- `cargo fmt --all -- --check` clean
- Manual smoke: complete a task via `update_task_status(completed)` in a CLI test session; verify `mika memory search` shows the new auto-fact event

## Risk / known unknowns

- **`store_fact_event` helper existence.** If the internal write path used by the `store_fact` tool isn't a clean callable helper, the implementer extracts it. The plan should not re-dispatch through ToolRegistry (recursive risk + double-audit-events).
- **Task metadata extraction robustness.** Claude-pilot's metadata (`pr_url`, `turns`, `cost_usd`) is written by `try_extract_callback_metadata` at callback delivery. If the task was manually completed by the operator (no callback), those fields are missing. The plan uses `—` placeholders for graceful degradation.
- **Race with audit-events.** `update_task_status` already emits an `audit_events` row for the transition. The auto-fact is a separate `memory_facts` row in a different table — no conflict, two-tier observability (transactional audit + semantic memory).
- **Milestone retirement iteration 2.** Once the hook is stable, the milestone-workflow `store_fact` directives in `self-dev/system_prompt.md` can be retired (reclaiming the 83-byte headroom + more). Explicitly out of scope here.

## Out-of-scope (explicit)

- Retiring milestone-workflow `store_fact` directives in `self-dev/system_prompt.md` (iteration 2).
- Hooks on `delivered` / `failed` / `cancelled` / `blocked` transitions (separate semantics; emit facts for them only if/when prompts direct it).
- Hooks on tools other than `update_task_status` (the layer supports it; the registry just doesn't list other entries today).
- Generalizing the hook to milestone parents (iteration 2 / 3).
- Migrating existing prompt-level fact emissions to engine-level (separate concern).
