---
title: fix(autonomous-loop): per-skill LLM override doesn't fire on ready-label webhook turns
type: fix
status: active
date: 2026-05-07
origin: mika#1011 (ticket body is the requirements doc)
---

# fix(autonomous-loop): per-skill LLM override doesn't fire on ready-label webhook turns

## Overview

mika-dev's per-skill LLM override (`skill_overrides` row pinning `self-dev` to `anthropic/claude-sonnet-4-6`) silently fails to apply on autonomous-loop dispatch turns. When a `[GitHub] Issue labeled ready on …` webhook fires, the agent turn runs on mika-dev's base model (`moonshotai/kimi-k2.5`), not the override target. Combined with kimi's documented fabrication-on-tool-failure pattern, this leaves mika-dev posting fabricated success messages on `run_claude_pilot` rejection paths while the parent task is silently stranded with no callback wired to wake it up.

This plan addresses both halves of the problem as **two layers of one fix**:
- **Layer 1 — Config-resolution:** make the override actually fire on these turns. (Three sub-options; architect-evaluation requested.)
- **Layer 2 — Engine-semantic:** auto-register a deferred-dispatch callback on `global_dispatch_active` so the failure mode self-heals regardless of LLM grounding behavior.

Both layers ship in one PR. Layer 1 alone leaves the engine fragile to future LLM-grounding regressions. Layer 2 alone leaves the override mechanism un-utilized on the load-bearing turns.

## Problem Frame

See mika#1011 body for the canonical evidence (mika-platform#85 retry pair: original session `fedfb369-…` fabricated, retry session `f5a721d6-…` succeeded only because the dispatch slot happened to be free). Code-read root cause confirmed:

1. `resolve_skill_llm_override()` (`crates/mika-agent/src/agent.rs:3761`) IS reached from conversation-mode (`agent.rs:2057`) and team-mode (`agent.rs:3508`) main loops.
2. `apply_overrides()` (`crates/mika-agent/src/skills/mod.rs:481-489`) DOES write DB `skill_overrides.llm_provider/llm_model` into `entry.manifest.llm`.
3. **The actual gate:** `MatchReason::Keyword`-only filter at `agent.rs:3773-3774` (per #463 matching #265 `collect_required_tools()` precedent). Webhook user message `[GitHub] Issue labeled ready on …` contains no self-dev trigger keyword (`add feature`, `implement`, `develop yourself`, `build`, `add yourself`, `improve yourself`, `new feature`, `resume`, `continue` — `skills/bundled/self-dev/skill.toml:21-30`). Self-dev lands as `MatchReason::AlwaysOn`. Override filtered out.
4. Engine returns `global_dispatch_active` as a failed tool result with no deferred-callback registration (`executor.rs:785-814`); the contract at `agent.rs:4427-4430` delegates failure-mode recovery to the LLM.

## Requirements Trace

- **R1 (AC1):** A mika-platform#85-class workflow (ready-label fired while another dispatch is active) reaches PR-merge without operator intervention or fabrication.
- **R2 (AC2):** `llm_calls.model` for the ready-label dispatch turn shows the self-dev override target (`anthropic/claude-sonnet-4-6`) on the very first attempt — no manual re-trigger required.
- **R3 (AC3):** When `run_claude_pilot` returns `global_dispatch_active`, a `pending` callback task with `parent_task_id` matching the requesting task is created. When the blocking task completes, the deferred callback fires within one engine tick.
- **R4 (AC4):** Existing single-session-at-a-time invariant (mika#583) is preserved — never more than one `in_progress` callback at a time.

## Scope Boundaries

- Not changing mika-dev's base model (stays on `moonshotai/kimi-k2.5`).
- Not removing or modifying the `agent.rs:4427-4430` design contract for OTHER terminal failures (`task_not_dispatchable`, `dispatch_blocked_by`, `dispatch_check_failed`) — those continue to delegate to the LLM via `send_message`. Only `global_dispatch_active` gets the deferred-callback treatment, because only it has a clear queue-and-retry semantic (the slot will free).
- Not extending deferred-dispatch to A2A or team-engine paths — Layer 2 scope is the long-running `run_claude_pilot` dispatch only.
- Not changing the `webhook_ready_label_dispatch` or `webhook_no_unauthorized_dispatch` INTENT_GUARDS (`agent.rs:4431/4464`) — they continue to enforce dispatch-shape on webhook turns.
- Not modifying mika-arch, mika-qa, or mika-relay skill_overrides — Layer 1 affects only the override resolution path; mika-arch's existing skill-only invocations continue to work.

### Deferred to Separate Tasks

- The broader question of whether `MatchReason::AlwaysOn` skills with DB overrides should universally take precedence over keyword-matched skills (vs. just on autonomous-loop turns) — Layer 1 sub-option (b) addresses this; if rejected, file a follow-up for design discussion.
- Renaming or restructuring `skill_overrides` table (out of scope; v20 schema stays).

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/agent.rs:3761-3865` — `resolve_skill_llm_override()` definition. Filter at line 3773 is the gate.
- `crates/mika-agent/src/agent.rs:2057, 3508` — production callsites (conversation, team).
- `crates/mika-agent/src/skills/mod.rs:436-508` — `apply_overrides()` writes DB rows into manifest.
- `crates/mika-agent/src/skills/matcher.rs:10-24` — `MatchReason` enum (`Keyword` / `AlwaysOn` / `Dependency`).
- `crates/mika-agent/src/skills/matcher.rs:39-68` — `match_message()` precedence (Keyword wins over AlwaysOn).
- `crates/mika-agent/src/skills/executor.rs:785-814` — `global_dispatch_active` rejection path.
- `crates/mika-agent/src/agent.rs:4427-4430, 4548-4551` — design contract delegating failure-mode recovery to LLM.
- `crates/mika-agent/src/agent.rs:4431, 4464` — `webhook_ready_label_dispatch`, `webhook_no_unauthorized_dispatch` INTENT_GUARDS (Layer 1 must not break these).
- `crates/mika-agent/src/task_engine/dispatcher.rs` — callback dispatch / `handle_task_complete` (Layer 2 deferred-callback fire site).
- `crates/mika-agent/src/db.rs` — `tasks` table, `get_child_tasks`, callback task creation patterns (Layer 2 model).
- `skills/bundled/self-dev/skill.toml:5, 21-30` — `always_on = true`, trigger keywords.

### Institutional Learnings

- `docs/solutions/best-practices/autonomous-agent-operational-discipline-2026-04-23.md` — kimi fabrication A/B test; sonnet recommended on grounding-sensitive paths. The 2026-05-06 cost swap was supposed to apply this; this fix makes it actually apply.
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — *"against-gradient behaviors must bind structurally."* Layer 2 is the structural binding for the rejection-recovery path; Layer 1 is the configuration binding for the model selection path.
- mika#583 — single-session-at-a-time invariant (Layer 2 must compose).
- mika#871 — orphaned parent reaper (Layer 2's deferred callback must not look like an orphan).
- mika#959 — callback subprocess liveness watchdog (same family).
- mika#910 — `webhook_no_unauthorized_dispatch` (Layer 1 must not weaken this).

### External References

Not applicable — codebase has strong local patterns for both layers.

## Key Technical Decisions

- **One PR, both layers.** Per ticket body framing. Layer 1 fixes intent; Layer 2 fixes mechanism. Either alone leaves a hazard.
- **`global_dispatch_active` is the only deferred-callback case.** Other terminal errors (`task_not_dispatchable`, `dispatch_blocked_by`) genuinely require operator/LLM action. `global_dispatch_active` is the only one with deterministic queue-and-retry semantics.
- **Deferred-callback model: spawn-on-rejection.** When `executor.rs` returns `global_dispatch_active`, it ALSO creates a `pending` callback task with `parent_task_id = requesting_task_id` and `trigger_type = 'callback'`. The blocking task's existing callback fires first (resumes its own session); on `handle_task_complete` for the blocking task, the dispatcher promotes the next pending deferred callback to `in_progress` and dispatches it.
- **Layer 1 sub-option choice deferred to architect.** Three candidates with different blast radii:
  - **(a) skill.toml keyword addition** — 1-line change, fully reversible, fully scoped to self-dev.
  - **(b) lift `Keyword`-only filter for `AlwaysOn` + DB-override skills** — small code change, broader semantic change, touches the #463/#265 precedent.
  - **(c) introduce `MatchReason::SystemEvent`** — architectural addition, largest surface, most flexible.
- **Author lean: (b)** as the structurally correct answer — operator DB intent (an explicit `skill_overrides` row) should outrank keyword inference. The load-bearing question is whether #463/#265's rationale permits an additive carve-out for `AlwaysOn` skills with non-empty `entry.manifest.llm` (i.e., explicit DB override). Architect to re-read #463/#265 issue bodies and decide. **(a) is the fallback** only if (b) violates that precedent. (c) reserved as a future architectural option if event markers proliferate beyond `[GitHub]` and `[callback:`.
- **Two wake-up paths now coexist (composition risk).** With (b) shipped, sonnet handles webhook turns and the existing design contract at `agent.rs:4427-4430` directs it to call `send_message` on rejection (operator escalation path). Layer 2 ALSO wires an engine-side auto-fired deferred callback. On `global_dispatch_active`, both paths now activate: sonnet → `send_message` notifies the operator; the engine queues the deferred callback to auto-retry. Architect must pick the precedence:
  - **Option α — deferred callback suppresses operator escalation:** when a deferred callback is registered, the LLM is instructed (via prompt context injection or guard correction) to skip `send_message` for `global_dispatch_active` specifically. Operator only sees the autonomous-loop result.
  - **Option β — operator escalation suppresses pending deferred:** when `send_message` notifies the operator, the pending deferred callback is cancelled. Operator manually drives the retry (apply `ready` label again).
  - **Option γ — both fire, one no-ops by guard:** operator gets notified AND the deferred callback runs. If the operator re-applies `ready` (cancelling the deferred callback's effect via the existing in-flight check), the deferred callback's promotion is skipped because there's already an active dispatch. Inverse: if the deferred callback fires first, the operator's notification is informational-only.

## Open Questions

### Resolved During Planning

- **Q: Does `apply_overrides()` actually write the DB row into the manifest?** Yes — confirmed at `skills/mod.rs:481-489`. The DB read path is correct; the keyword-filter is the gate.
- **Q: Does `resolve_skill_llm_override()` get called on autonomous-loop turns?** Yes — at `agent.rs:2057` (conversation mode), which is the path webhook events take.
- **Q: Why didn't the 2026-04-26 architect dogfood catch this?** The mika-arch override is on `mika-arch-groom-ticket` skill, which is keyword-matched (`groom`, `groom ticket`). mika-arch's primary use cases trigger keyword match; mika-dev's primary use case (webhook events) does not.

### Architect ratification needed (proposed shape, not yet committed)

- **Race window between rejection-return and pending-callback insertion — proposed shape:** insert the deferred callback row FIRST (before any rejection-decision logic returns), then check whether the blocking task is still active; if it has already completed in the interim, immediately promote the just-inserted deferred callback to `in_progress` and dispatch. This closes the TOCTOU window where a blocking task could finish between "in-flight check" and "deferred callback insert," leaving the deferred callback orphan-pending forever (the blocking task's `handle_task_complete` already drained). Architect to specify the exact transaction boundary — single transaction with row-level lock vs. two transactions with idempotent retry.
- **Silent trigger variant — proposed shape:** introduce a new `SilentTrigger::DeferredDispatch` variant carrying `[callback:deferred-dispatch]` framing, with its own INTENT_GUARDS entry distinct from `callback_terminal_action` (`agent.rs:4507-4512`). Conflating with `Silent::Callback` will mis-fire `callback_terminal_action`, which requires `update_task_status` AND `send_message` — a deferred-dispatch retry doesn't have a completed sub-run to report. New variant also makes the audit trail clearer (operator can grep `kg_deferred_dispatch_fire` style events). Architect to ratify or propose alternative.
- **Reaper/watchdog regression check — verify side-by-side, do not infer.** I claim no regression because (i) deferred-pending callbacks have no `process_id` so mika#959's watchdog skips, (ii) mika#871's reaper looks for `delivered` children so `pending` deferred callbacks don't match. Both claims need direct code-read verification, not CLAUDE.md-description inference. Implementer must read `dispatcher.rs::reap_orphaned_parent_tasks()` and `check_callback_process_liveness()` queries side-by-side with the new deferred-callback shape before merging.

### Deferred to Implementation

- **Concrete callback metadata shape for the deferred-dispatch task.** `action_config` JSON content — needs to encode "retry the original `run_claude_pilot` call with original arguments." Settled when implementer reads the existing callback task creation pattern in `executor.rs::spawn_long_running_exec()`.
- **Sub-option (a) keyword choice — `"[GitHub]"` vs `"labeled ready"` vs both.** Settled only if architect rules (b) violates precedent and (a) is the fallback; then implementer reads the gateway's `format_event_text()` output to confirm the literal prefix.

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification.*

```
WEBHOOK ARRIVES                                  RUN_CLAUDE_PILOT REJECTED
─────────────────                                ─────────────────────────
[GitHub] Issue labeled ready on R#N              executor: global_dispatch_active
        │                                                │
        ▼                                                ▼
match_message(...) → MatchedSkill[]              Layer 2: register deferred callback
        │                                        ── parent_task_id = requesting task
        │  (Layer 1 fix)                         ── trigger_type = 'callback'
        │  Keyword filter relaxed (a/b/c)        ── status = 'pending'
        ▼                                                │
resolve_skill_llm_override(matched)                      ▼
        │                                        executor returns rejection JSON
        ▼  ── DB override now applies                    │
effective_llm = sonnet-4.6                               ▼
        │                                        LLM sees rejection
        ▼                                        (kimi or sonnet — Layer 1 outcome)
LLM turn runs on sonnet                                  │
        │                                                ▼
        ▼                                        LLM may fabricate or send_message
run_claude_pilot called                          (no longer load-bearing)
        │                                                │
        ▼                                                │
(succeeds OR rejected with G_D_A) ───────────────────────┤
                                                         ▼
                                                 BLOCKING TASK COMPLETES
                                                         │
                                                         ▼
                                                 dispatcher::handle_task_complete
                                                 promotes next pending deferred callback
                                                         │
                                                         ▼
                                                 Silent::Callback fires
                                                 mika-dev resumes requesting session
                                                 retries run_claude_pilot
                                                 (slot now free → succeeds)
```

## Implementation Units

### Unit 1: Layer 1 — pick and apply override-scope fix

- [ ] **Unit 1: Override-scope fix (architect-chosen sub-option)**

**Goal:** Make `resolve_skill_llm_override()` fire on `[GitHub] Issue labeled ready on …` webhook turns, so mika-dev's `self-dev → anthropic/claude-sonnet-4-6` DB override actually applies. Architect picks one of (a)/(b)/(c).

**Requirements:** R2

**Dependencies:** None (independent of Unit 2 in code; ships together for behavior).

**Files** (per sub-option):
- **(a) skill.toml keyword addition:**
  - Modify: `skills/bundled/self-dev/skill.toml` (add to `[triggers].keywords`)
  - Test: `crates/mika-agent/src/skills/matcher.rs` test module — add a webhook-message keyword-match test
- **(b) lift Keyword-only filter for AlwaysOn + DB-override:**
  - Modify: `crates/mika-agent/src/agent.rs` (`resolve_skill_llm_override()` filter at line 3773)
  - Modify: comment in `crates/mika-agent/src/agent.rs` near line 3766-3768 (rationale for filter relaxation)
  - Test: `crates/mika-agent/src/agent.rs` test module — add an `always_on_with_db_override_applies` test
- **(c) introduce `MatchReason::SystemEvent`:**
  - Modify: `crates/mika-agent/src/skills/matcher.rs` (extend enum, extend `match_message()` precedence, extend skill.toml schema parser)
  - Modify: `crates/mika-agent/src/skills/manifest.rs` (or wherever `[triggers]` parses) — add `system_events: Vec<String>` field
  - Modify: `skills/bundled/self-dev/skill.toml` (add `[triggers].system_events`)
  - Modify: `crates/mika-agent/src/agent.rs:3773` filter to honor `SystemEvent`
  - Test: `crates/mika-agent/src/skills/matcher.rs` — add `system_event_match_precedence` test

**Approach:**

| Sub-option | Mechanism | Reversibility | Blast radius | Author lean |
|---|---|---|---|---|
| (a) | Add `"[GitHub]"` (or narrower fragment) to self-dev keywords; webhook event becomes a Keyword match | High — single-line skill.toml revert | Low — only self-dev affected | **Fallback only** if (b) violates the #463/#265 precedent |
| (b) | Relax filter: any `AlwaysOn` skill with non-empty `entry.manifest.llm` (i.e., explicit DB override) qualifies for override resolution | Medium — code change, but localized | Medium — affects every always_on skill with a DB override; today only self-dev qualifies, but precedent established | **Lean** — operator DB intent (an explicit `skill_overrides` row) is structurally a higher-authority signal than keyword inference. The #463/#265 rationale needs re-read to confirm the carve-out is permitted |
| (c) | New match reason, new skill.toml field, new dispatch precedence | Low — schema change, multi-file | Higher — new mental model for skill matching | Reserved for future if event markers proliferate beyond `[GitHub]` and `[callback:` |

**Patterns to follow:**
- For (a): `skills/bundled/self-dev/skill.toml [triggers].keywords` shape — array of strings, case-insensitive matched via `to_lowercase()` (`matcher.rs:39, 124`).
- For (b): existing tests `test_resolve_skill_llm_override_*` in `agent.rs:6716-6870`. The test `test_resolve_skill_llm_override_keyword_match_on_always_on_skill_applies` at line 6817 is the closest neighbor.
- For (c): `MatchReason` enum extension pattern; `match_message()` ordered precedence at `matcher.rs:50-68`.

**Test scenarios:**
- *Happy path (all sub-options):* webhook user message `[GitHub] Issue labeled ready on senara-solutions/mika-platform#85 — feat(slash): /mika-onboarding` triggers self-dev → `resolve_skill_llm_override()` returns `Some(sonnet)` when the DB row is set. `effective_llm.model_name()` reports `claude-sonnet-4-6`.
- *Edge case:* multiple agents with different overrides — assert this fix is per-agent (each agent's matched skills only see their own DB rows).
- *Edge case:* `webhook_no_unauthorized_dispatch` guard (`agent.rs:4464`) still blocks unauthorized dispatch on non-ready-label `[GitHub]` events. The Layer 1 fix must not enable mika-dev to dispatch on PR-comment or check-suite events.
- *Edge case:* user message `"build the new feature"` (action-verb keyword) still triggers the override via the existing Keyword path (no regression on non-webhook flows).
- *Error path (sub-option b only):* skill has `always_on=true` but empty `entry.manifest.llm` (no DB override row) — must NOT trigger override (filter skips empty manifest LLM at line 3777).
- *Integration:* webhook-triggered dispatch turn produces `llm_calls.model = anthropic/claude-sonnet-4-6` end-to-end.

**Verification:**
- `llm_calls` rows for a webhook-triggered mika-dev dispatch turn show the override target model on every step.
- `prompt_variant.self-dev` value reflects the resolved variant for sonnet (per provider/model variant resolution).
- No regression on `webhook_no_unauthorized_dispatch` — non-ready-label `[GitHub]` events still get fall-through behavior.
- No regression on existing `test_resolve_skill_llm_override_*` tests (or they're updated coherently).

---

### Unit 2: Layer 2 — deferred-dispatch callback registration

- [ ] **Unit 2: Auto-register deferred callback on `global_dispatch_active`**

**Goal:** When `run_claude_pilot` is rejected with `global_dispatch_active`, the engine creates a `pending` callback task linked to the requesting parent task. When the blocking task completes, the deferred callback fires and resumes the requesting session.

**Requirements:** R1, R3, R4

**Dependencies:** None in code (independent of Unit 1).

**Files:**
- Modify: `crates/mika-agent/src/skills/executor.rs` (the `global_dispatch_active` rejection path at lines 785-814)
- Modify: `crates/mika-agent/src/task_engine/dispatcher.rs` (or `handle_task_complete` callsite — wherever the blocking task's completion fires its callback) to ALSO promote the next pending deferred callback
- Modify: `crates/mika-agent/src/agent.rs:4427-4430` and `4548-4551` — update the design-contract comment to reflect the new `global_dispatch_active` semantics (auto-recovers; LLM no longer needs to handle via `send_message` for this specific terminal error)
- Test: `crates/mika-agent/src/skills/executor.rs` test module — add `global_dispatch_active_registers_deferred_callback`
- Test: `crates/mika-agent/src/task_engine/dispatcher.rs` test module — add `deferred_callback_fires_on_blocking_completion`
- Test: `crates/mika-agent/tests/eval/` — end-to-end scenario covering "two ready-label webhooks fire; first dispatches, second is deferred; deferred resumes after first completes"

**Approach:**

When `executor.rs::validate_dispatch_readiness()` (or equivalent) detects `global_dispatch_active`:

1. **Before** returning the rejection JSON, insert a `pending` callback row:
   - `parent_task_id` = the task being rejected (NOT the blocking task)
   - `trigger_type` = `'callback'`
   - `action_type` = `'resume_agent'`
   - `agent_id` = the requesting agent
   - `status` = `'pending'`
   - `label` = `long_running:run_claude_pilot:deferred`
   - `action_config` = JSON encoding the original `run_claude_pilot` call args (prompt, skill, task_id) for replay
2. Return the rejection JSON to the LLM (unchanged shape — keeps INTENT_GUARDS happy and avoids breaking other callers).
3. In `dispatcher.rs::handle_task_complete()` for the blocking task, AFTER firing the blocking task's existing callback, query for the next-oldest `pending` deferred callback (`label LIKE 'long_running:run_claude_pilot:deferred'`, `status='pending'`, ORDER BY `created_at ASC`, LIMIT 1) and promote it to `in_progress`.
4. The deferred callback fires via the standard `Silent::Callback` path. The agent loop sees a `[callback:` framing user message containing the original `action_config` payload. The LLM resumes its session, sees the deferred-dispatch context, and re-invokes `run_claude_pilot` (slot now free, succeeds).

**Composition with mika#583 invariant:** never more than one `in_progress` callback. The promotion in step 3 enforces this — only one deferred callback promotes per `handle_task_complete` cycle.

**Composition with mika#871 reaper:** the reaper looks for orphaned parent self_dev tasks. Deferred callbacks have a clear parent and a clear pending state — the reaper's `find_orphaned_parent_tasks` query (`status='in_progress'`, `source='self_dev'`, child `status='delivered'`) doesn't false-positive on deferred-callback-pending parents (child is `pending`, not `delivered`).

**Composition with mika#959 watchdog:** the watchdog only checks `in_progress` callbacks with `process_id IS NOT NULL`. Deferred-pending callbacks have no `process_id` and are skipped.

**Patterns to follow:**
- Existing callback task creation in `executor.rs::spawn_long_running_exec()` — `action_config` JSON shape, parent linkage.
- `dispatcher.rs::handle_task_complete()` Ok-path drain pattern (already drains webhook deferral queue per mika#528 — same mechanic).
- `task_engine::dispatcher::dispatch_resume_agent` — Silent::Callback dispatch site.

**Test scenarios:**
- *Happy path:* Task A dispatches `run_claude_pilot`; while in flight, Task B attempts `run_claude_pilot` and is rejected with `global_dispatch_active`. A pending callback row exists with `parent_task_id = B.id`. When Task A's callback delivers, B's deferred callback promotes to `in_progress` within one engine tick and fires.
- *Edge case:* Three tasks queue (A in flight, B and C deferred). A completes → B promotes, C stays pending. B completes → C promotes. FIFO order preserved.
- *Edge case:* Task B is cancelled while its deferred callback is `pending`. Cancellation cascades — deferred callback transitions to `cancelled` (not promoted on A's completion).
- *Edge case:* Task A's claude-pilot crashes (subprocess dies; mika#959 watchdog marks A's callback `failed`). B's deferred callback still promotes — slot is free either way.
- *Error path:* `db.rs` insert of pending callback row fails (e.g., FK constraint, disk full). Original `run_claude_pilot` rejection still returns; LLM falls back to existing `send_message` path. Defense-in-depth — Layer 1 keeps the LLM grounded enough to handle this.
- *Error path:* Two blocking tasks complete simultaneously (rare). Each `handle_task_complete` tries to promote the next deferred. Transaction / row-level lock prevents both promoting the same row.
- *Integration:* mika#583 single-session-at-a-time invariant holds — at no point are two callbacks `in_progress` simultaneously.
- *Integration:* mika-platform#85-class regression test — webhook A fires while webhook B holds the slot; webhook A reaches PR-merge end-to-end without operator intervention (this is R1).

**Verification:**
- `tasks` table query: deferred callback row exists with correct `parent_task_id`, `status='pending'`, then transitions to `in_progress` → `delivered`.
- Audit log shows `deferred_dispatch_registered` and `deferred_dispatch_promoted` events (or equivalent).
- `agent.rs:4427-4430` comment reflects the new semantics; the LLM-handles-via-send_message contract is now scoped to the OTHER terminal errors only.
- mika#583 invariant assertion in tests holds across the deferred-callback flow.

---

### Unit 3: Doc + memory updates

- [ ] **Unit 3: Update CLAUDE.md and contract comments**

**Goal:** Keep the codebase's self-documentation in sync with the new behavior, so future contributors don't recreate the gap.

**Requirements:** None directly (post-correctness hygiene).

**Dependencies:** Unit 1 + Unit 2.

**Files:**
- Modify: `crates/mika-agent/CLAUDE.md` § Skills System — update the "Per-skill LLM override" paragraph to flag the keyword-vs-always-on filter behavior and (post-Layer 1) the resolved scope.
- Modify: `crates/mika-agent/CLAUDE.md` § Unified Task Engine — add a paragraph on deferred-dispatch callback semantics (Layer 2).
- Modify: `crates/mika-agent/src/agent.rs:4427-4430, 4548-4551` — update inline comments to reflect that `global_dispatch_active` is now engine-handled, not LLM-handled.
- Test expectation: none — pure documentation.

**Patterns to follow:**
- Existing `crates/mika-agent/CLAUDE.md` paragraph style — concise, version-pinned, file:line cited.
- Inline comment style at `agent.rs:4427-4430` — references issue numbers + design intent.

## System-Wide Impact

- **Interaction graph:**
  - Layer 1 affects `match_message()` → `resolve_skill_llm_override()` → main agent loop. Touches conversation mode primarily; team mode unchanged in shape (mika-arch and team flows already keyword-match).
  - Layer 2 affects `executor.rs` (rejection path) → `task_engine/dispatcher.rs::handle_task_complete()` (drain-on-completion). New callback task class; reuses existing `Silent::Callback` infrastructure.
- **Error propagation:** Layer 2's deferred callback failure (insert error, promotion error) falls back to existing LLM-handles-it path — Layer 1 ensures the LLM is sonnet on these turns, so fallback is grounded.
- **State lifecycle risks:** Deferred callback promotion ordering — FIFO via `ORDER BY created_at ASC` is the design choice. If multiple agents share the global dispatch slot (e.g., mika-dev + mika-qa), each agent's deferred callbacks queue independently per `parent_task_id`. Single-session-at-a-time invariant is global, not per-agent.
- **API surface parity:** No public API changes. `run_claude_pilot` tool's input/output shape unchanged; the rejection JSON still has the `global_dispatch_active` key for INTENT_GUARDS / `webhook_ready_label_dispatch` compatibility.
- **Integration coverage:**
  - Cross-layer: mika-platform#85-class regression test (Layer 2 happy path) is the core integration scenario.
  - Cross-layer: webhook-triggered turn produces `model = sonnet` end-to-end (Layer 1 happy path).
- **Unchanged invariants:**
  - mika#583 single-session-at-a-time — preserved (only one `in_progress` callback at a time; `pending` callbacks don't count).
  - mika#910 `webhook_no_unauthorized_dispatch` — preserved (Layer 1 doesn't relax the source-check; just makes the override fire when the source-check passes).
  - mika#871 reaper — preserved (looks for `delivered` children; `pending` deferred callbacks don't trigger).
  - mika#959 watchdog — preserved (only checks `in_progress` callbacks with `process_id`).

## Risks & Dependencies

| Risk | Mitigation |
|---|---|
| Layer 1 sub-option (a) keyword `"[GitHub]"` fires on non-ready-label webhooks (PR comments, check-suite events), causing self-dev to match where it shouldn't | `webhook_no_unauthorized_dispatch` (`agent.rs:4464`) blocks unauthorized dispatch via the engine guard. The skill being matched doesn't mean it dispatches; the prompt's source-check + the engine guard gate dispatch. Worst case: extra prompt content loaded into webhook-fall-through turns (cost). Sub-option choice is reversible. |
| Layer 1 sub-option (b) changes #463/#265 precedent and may surface unexpected behavior in mika-arch's keyword-only flows | mika-arch's overrides are on keyword-matched skills (`groom-ticket`, `second-review`); sub-option (b) doesn't change behavior for keyword-matched paths. Affects only `AlwaysOn`-only matched skills with explicit DB overrides — currently only self-dev qualifies on mika-dev. Add explicit test coverage for mika-arch's flow to confirm no regression. |
| Layer 2 race: blocking task completes between rejection-decision and pending-callback-insert | Sequence the operations transactionally — insert first, return rejection second. Or use the existing `db.rs` transaction wrapper pattern. |
| Layer 2 deferred-callback flood: malicious or buggy agent fires many `run_claude_pilot` calls; queue grows unbounded | Add a per-agent cap on pending deferred callbacks (e.g., 10). Existing `max_agent_tasks_per_session` (`db.rs`) is the precedent. |
| Layer 2 stale deferred callbacks: parent task is cancelled but deferred callback orphans | Cascade delete or status-cascade — mark deferred callback `cancelled` when parent is `cancelled`/`failed`. Add to the existing `update_task_status` cascade logic. |
| The 2026-04-26 #463 precedent change cited in sub-option (b) commentary may have rationale we don't fully understand | Architect to evaluate and re-read #463/#265. If sub-option (b) is rejected, fall back to (a) — still solves R2. |
| **Composition: two wake-up paths now coexist.** With Layer 1 (b) shipped, sonnet handles the rejection turn and (per design contract `agent.rs:4427-4430`) calls `send_message` to notify the operator. Layer 2 simultaneously wires an engine-side auto-fired deferred callback. Both activate on `global_dispatch_active`. Risk: operator gets paged AND the autonomous loop self-recovers; or the operator manually re-applies `ready` while the deferred callback is also pending, causing a double-dispatch attempt | Architect picks one of three precedence options (α/β/γ — see Key Technical Decisions). Whichever is chosen must compose with the existing in-flight check in `validate_dispatch_readiness()` so a double-dispatch can't slip through |

## Documentation / Operational Notes

- After merge, update memory `project_skill_override_scope_gap.md` to mark the gap closed and cite the merge commit.
- Update memory `project_mika_dev_model_switch.md` to remove the ⚠ warnings about the override being inert.
- Operator-side: no config changes required. The fix is automatic on next `make deploy` / `mika-server` restart.
- Validation post-merge: re-trigger a ready-label webhook on a queued issue while another dispatch is in flight; confirm `llm_calls.model = sonnet` and confirm the deferred callback fires when the slot frees.

## Sources & References

- Origin: mika#1011 ticket body (this plan was filed against that ticket).
- Evidence pair: mika.db sessions `fedfb369-f4e7-4c2e-bb91-dc03f0ebdebd` (original) and `f5a721d6-25c2-49b6-9583-4b5c38e85ae6` (retry).
- Related: mika#583, mika#841, mika#847, mika#871, mika#910, mika#959.
- Related memories: `project_skill_override_scope_gap`, `project_mika_dev_model_switch`, `feedback_sonnet_over_kimi_for_grounding`, `feedback_mika_dev_llm_fabricates_tool_errors`.
