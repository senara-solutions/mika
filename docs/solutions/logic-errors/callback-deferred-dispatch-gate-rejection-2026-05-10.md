---
module: skills-executor
date: 2026-05-10
last_updated: 2026-05-10
problem_type: logic_error
component: tooling
severity: high
symptoms:
  - "claude-pilot session exits Success after `mika ask --agent mika-arch` returns — verdict discarded, 0 commits"
  - "executor.rs gate rejects run_claude_pilot from callback turns with 'long-running tool invoked without long_running_ctx'"
  - "DeferredDispatch silent turns fail at INTENT_GUARD because run_claude_pilot is blocked by the same gate"
  - "Pipeline retry path on PIPELINE FAILURE callbacks cannot invoke run_claude_pilot"
  - "PIPELINE FAILURE marker fires in 3 of 4 dispatch-lib invocations (zero-commit class); 1 silent slip when staged plan was committed before exit"
root_cause: logic_error
resolution_type: code_fix
tags:
  - callback
  - deferred-dispatch
  - long-running
  - executor-gate
  - cycle-detection
  - pipeline-retry
  - bug-family-claude-pilot-exits-success
  - lineage-walk
  - callback-task-id-bridge
---

# Callback and DeferredDispatch Turns Cannot Call Long-Running Tools

## How this surfaced to the operator

Empirically observed across 5 dev-groom sessions over 24 hours (2026-05-09 → 2026-05-10), ~$4.61 of architect work discarded:

| Session | claude-pilot ID | Marker fired? | Cost / turns |
|---|---|---|---|
| mika#1051 pass-2 | n/a (manual orchestrator spawn — outside dispatch-lib) | n/a | $1.56 / 32t |
| mika-platform#96 groom | `582cc389` | **NO** (silent-slip class — HEAD changed because staged plan was committed before exit) | $0.89 / 21t |
| mika#1052 groom | `a9435e3d` | YES (zero commits) | $1.01 / 15t |
| mika-skills#159 groom | `239b6320` | YES (zero commits) | $1.15 / 29t |
| mika-skills#163 groom | `00e01562` | YES (zero commits) | $1.69 / 24t |

**Key empirical insight (drove the fix shape):** 3 of 4 dispatch-lib invocations hit the existing post-flight HEAD-diff PIPELINE FAILURE marker; **1 silently passed** because claude-pilot's first pass had committed the staged plan before the verdict-discard exit, leaving HEAD changed. Recovery first (this fix), detection-extension second (filed as follow-up after #1058 — value goes UP because the silent-slip class becomes a marker-fired class once recovery works).

## Problem

Two failure modes prevented pipeline retry and deferred dispatch from working:

1. **Callback gate rejection (Mode B):** When mika-dev's pipeline-retry path fires on a `PIPELINE FAILURE` callback, the retry attempts `run_claude_pilot` directly. This fails at `executor.rs:278-296` because callback turns have `long_running_ctx = None`. The gate is an `Option` check — not behavioral — and cannot be overcome by prompt changes.

2. **DeferredDispatch latent bug:** The existing `DeferredDispatch` mechanism (mika#1011) passes `None` for `long_running_ctx` to `run_loop` for ALL silent triggers (line 3339: `None, // long_running not supported in silent mode`). But the `deferred_dispatch_action` INTENT_GUARD requires the LLM to call `run_claude_pilot`. The LLM tries, gets blocked by the gate, correction fires, blocked again → max steps exceeded.

## What Didn't Work

- **Prompt-only enforcement on dev-groom** ("STOP — DO NOT EXIT" directive + Phase 5 verification gate of git/gh checks before emitting Verdict line): LLMs rationalize crossing prompt budgets; the gate is a structural `Option` check, not behavioral. Architect pass-1 review (session `d268e776`) returned ITERATE — load-bearing finding: claimed engine guard (mika#864 required-suffix-line) prevents claude-pilot exit, but actually the guard fires on mika-dev's host turn, NOT on claude-pilot's subprocess session. Per `feedback_prompt_enforcement_fragile` (auto memory [claude]): only structural constraints hold.
- **Watchdog re-spawn from dispatch-lib:** Parallel retry mechanism in bash, requires verdict-extraction-from-DB coupling, becomes tech debt the moment the engine fix lands. Per `feedback_loop_stability_beats_loop_speed` (auto memory [claude]): velocity is not a design driver; engine-level fix beats faster-to-ship bash bridge.
- **Existing recovery primitives don't apply** (session history): mika#959's process-liveness watchdog (`check_callback_process_liveness()`) detects dead subprocesses via `/proc/<pid>/stat` and respawns — but here the process IS gone cleanly (exit 0), just at the wrong workflow point. mika#991's `SilentTrigger::PostCallbackAdvance` fires after a callback turn completes to check queue advancement — but mika#1058's failure is that the callback child task is never *created* (the outer groom subprocess exits before reaching the dispatch). PostCallbackAdvance has no hook for a groom that self-terminates. Both adjacent fixes target different layers.
- **Detection-extension first** (extend dispatch-lib post-flight check beyond HEAD-SHA): wrong sequencing. Existing HEAD-marker already covers 75% of cases (3 of 4 above); pairing better detection with broken recovery wastes the leverage. Filed as follow-up — value rises *after* this fix lands. Per `feedback_evidence_before_diagnosis` (auto memory [claude]): the empirical 3-of-4-vs-1-of-4 split changed the fix shape.

## Solution

Two-part fix:

### Part 1: Inject `LongRunningContext` for DeferredDispatch triggers

In `run_silent_inner`, construct a `LongRunningContext` conditionally for `SilentTrigger::DeferredDispatch` and pass it to `run_loop` instead of unconditional `None`:

```rust
let long_running_ctx =
    if matches!(&params.trigger, SilentTrigger::DeferredDispatch { .. }) {
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
```

### Part 2: Gate-intercept for callback and DeferredDispatch turns

Added `callback_task_id: Option<&str>` to `ToolContext`, threaded from `SilentTrigger::Callback` and `SilentTrigger::DeferredDispatch`. When the executor gate rejects a long-running tool call and `callback_task_id` is `Some`:

1. Run `check_lineage_cycle()` — walks `parent_task_id` chain (max 4 hops), compares `(repo, issue_number, skill)` tuples
2. If no cycle: call `register_deferred_callback()` to enqueue the dispatch
3. Return `{"status": "deferred", "deferred": true}` — LLM knows not to retry

```rust
if let Some(task_id) = callback_task_id
    && let Some(db) = callback_db
{
    match check_lineage_cycle(db, task_id, &input).await {
        Ok(()) => {
            if register_deferred_callback(db, task_id, &input).await {
                return ToolOutput::success(json!({"status": "deferred", ...}));
            }
        }
        Err(cycle_msg) => {
            return ToolOutput::error(json!({"error": "deferred_dispatch_cycle_detected", ...}));
        }
    }
}
```

### Cycle detection design

Lineage walk on `(repo, issue_number, skill)` tuple using existing `parent_task_id` chain:
- ✅ Allows `groom-#159 → pilot-#159` (different skill)
- ✅ Catches `groom-#159 → retry-groom-#159` (same tuple)
- ✅ Catches A→B→A class via lineage walk
- Fail-open on extraction failure; `depth ≤ 3` schema CHECK is structural backstop

**Why lineage walk and not retry-budget-on-the-task** (session history): mika#1011's DeferredDispatch implementation accepted the infinite-chain risk as low-probability without bookkeeping. A retry counter on the dispatched task (alternative considered) doesn't catch A→B→A — neither task's counter increments past 1. Lineage walk catches A→B→A by design **and** is durable across service restarts because `parent_task_id` is in the DB (a budget counter would reset on restart, losing cycle context). Combined with the existing `tasks.depth ≤ 3` CHECK constraint as the structural ceiling, runaway recursion can't exceed 4 levels even if the lineage walk has bugs.

## Why This Works

The gate at `executor.rs` was intentional defensive code (commit `04ae084c`, "feat(tui): callback delivery polling and loop prevention") to prevent loop-like behavior in callback turns. The fix preserves the gate for non-deferred contexts (heartbeat, reflection, CLI test) while enabling the specific pattern the engine already supports: deferred dispatch through `register_deferred_callback()`.

DeferredDispatch turns are the engine's auto-recovery path for `global_dispatch_active` rejections — their sole purpose is to call `run_claude_pilot`. Blocking them from doing so via the gate was a latent bug from the original silent-mode `None` assignment.

**mika#1011 architectural parent** (session history): the gate at `executor.rs:284` was introduced as a deliberate feature *for* DeferredDispatch — when `global_dispatch_active` is true, reject and register a `SilentTrigger::DeferredDispatch` task that fires when the slot frees. Tested and working for the standard "queue advancement" case. The flaw mika#1058 surfaces is that `run_silent_inner` passed `None` for `long_running_ctx` to **all** silent triggers (including DeferredDispatch), so the very mechanism designed to recover from gate rejection ended up rejected by the same gate. The fix is conceptually small — construct `LongRunningContext` for DeferredDispatch — but only visible once a real consumer (mika-dev's pipeline-retry) exercised the path. The 24-hour bleed is the empirical cost of having a recovery primitive ship without an exercising consumer.

**Engine-level over watchdog (`feedback_loop_stability_beats_loop_speed`, auto memory [claude]):** the bash watchdog would have shipped faster but become tech debt the moment the engine fix landed; engine-level fix is the durable layer.

## Prevention

- When adding new `SilentTrigger` variants that need to execute long-running tools, construct `LongRunningContext` explicitly rather than inheriting the blanket `None` for silent mode
- The `callback_task_id` field on `ToolContext` is the signal — if a turn can legitimately defer a long-running dispatch, it must carry the task ID for cycle detection
- Test new dispatch paths with the executor gate: verify both the `long_running_ctx = Some` path (gate passes) and the `callback_task_id` intercept path (gate catches and defers)
- **Couple engine capabilities to their first consumer in the same PR.** PR #1061 ships both the executor change and the `self-dev/system_prompt.md` update teaching the LLM to recognize `{"status": "deferred"}`. Engine capabilities that ship without an exercising consumer become latent surface area no one tests — the `DeferredDispatch.long_running_ctx = None` latent bug existed precisely because the silent-trigger path had no consumer until pipeline-retry surfaced it.
- **When introducing engine guards, frame them as denylist not blanket-refusal.** Commit `04ae084c` refused all `long_running_ctx.is_none()` calls; the surgical version refuses only direct calls and cycle-creating callback calls. If the original commit had asked "what legitimate path needs `None` here?" the DeferredDispatch consumer would have surfaced earlier.
- **Look for existing-but-unexposed safe paths before inventing new primitives** — `DeferredDispatch` was already in production via queue advancement; it just wasn't exposed to the LLM as a callback-turn retry primitive. The "designed-but-unshipped protocol" pattern recurs in this codebase; before designing a new dispatch surface, grep `silent_trigger`, `callback_task_id`, `action_type` for existing safe paths.
- **When a defensive guard is hardened (mika#537 made silent fall-through an explicit error), use that moment to re-evaluate whether the underlying refusal is too broad** — hardening surfaces bad UX (cryptic → loud error) without questioning whether the rejection itself is correct. Treat hardening as a forcing function for policy review.

## Bug family — "claude-pilot exits Success without producing work"

mika#1058 is the third documented member of a recurring failure family. The shared symptom: claude-pilot subprocess returns exit code 0 with `[done] Success` log line, but the workflow's expected artifact (commit / PR / Plan callout / writeback) is missing.

| Member | Mechanism | Layer | Status |
|---|---|---|---|
| mika#537 | `execute_skill_tool` silently fell through long-running handler to sync exec on callback turns; sync path didn't inject `__mika_task_id`, handler exited 1 with cryptic message | Executor | CLOSED — gate hardened from silent fall-through to explicit error |
| mika#940 family (mika#1032 / mika#1033) | claude-pilot `/mika` pipeline exits before reaching `git push` + `gh pr create`; work stranded in worktree | claude-pilot session prompt + handler | OPEN — no structural fix yet; orchestrator-recovery recipe exists |
| **mika#1058 (this doc)** | Subprocess exits Success after `mika ask --agent mika-arch` returns; mika-dev's pipeline-retry from callback context blocked by `executor.rs:284` gate | Executor + dev-groom flow | **CLOSED 2026-05-10 — engine-level fix shipped** |

Pattern: each member is a different exit point in claude-pilot's session lifecycle that produces "Success" without the workflow's contractual artifact. Recovery layer differs per member (orchestrator-direct for #940, engine-level for #1058), but the *detection* layer (post-flight HEAD-diff PIPELINE FAILURE marker in `_shared/dispatch-lib.sh`) is shared and partial — it catches HEAD-unchanged cases but misses HEAD-changed-but-incomplete cases (the silent-slip class). Detection-extension is filed as a follow-up to mika#1058.

## Related

- `architecture-patterns/callback-task-loop-prevention.md` — the original "loop prevention" intent this fix narrows without unwinding. This fix preserves the defensive principle but makes it surgical.
- `architecture-patterns/webhook-deferral-queue-callback-sequencing.md` — the `DeferredDispatch` primitive being wrapped. Read for context on why DeferredDispatch is the right primitive to lean on (vs inventing new dispatch infrastructure).
- `logic-errors/long-running-handler-silent-fallthrough.md` — mika#537 (CLOSED). Same gate, different fall-through mode. **Stale-flag candidate** — the gate it celebrates is now structurally limited; reading without context risks reintroducing the same Mode B regression mika#1058 fixed.
- `logic-errors/dispatch-retry-parent-status-promotion-2026-05-07.md` — mika#958. Adjacent retry-semantics fix; same module.
- `dev-loop/dev-pilot-handler-silent-exit-0-pattern-2026-04-29.md` — mika#940 sibling family. Different exit point in the same family. No engine-level fix yet.
- `runtime-errors/silent-callback-max-steps-exhaustion.md` — directly referenced as the DeferredDispatch latent-bug failure mode (Mode A). The exhaustion would happen because the LLM hit the gate, gate fired correction, hit the gate again, until max steps.
- `best-practices/auto-groom-on-dispatch-2026-05-06.md` — mika#996, the dispatch path that triggers dev-groom for ungroomed `ready` tickets and exposed this bug at scale.
- `best-practices/required-suffix-line-guard-verdict-ghosting-structural-fix-2026-04-29.md` — mika#864 referent. Important to read alongside this doc: that guard fires on **mika-dev's host turn** (when mika-dev processes the claude-pilot callback), NOT on **claude-pilot's subprocess session**. This layering distinction is what made the prompt-only fix attempt fail (the verification gate would have been on the wrong layer).
- mika-platform#75 — dispatcher-bootstrapping has no designed escape hatch. Meta-pattern under which the bug-fix ticket itself was groomed via orchestrator-direct recovery.
