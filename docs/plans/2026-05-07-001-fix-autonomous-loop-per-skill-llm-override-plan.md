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
- **Layer 1 — sub-option (b) chosen.** Carve-out: `MatchReason::AlwaysOn` AND `entry.manifest.llm.from_db_override == true` qualifies for override resolution. Rationale pinned in Phase 0.1: the #463 concern was specifically about `skill.toml [llm]` (developer hardcode), not DB `skill_overrides` (operator intent). #463's underlying goal — *"agent config changes take effect immediately"* — is what the carve-out RESTORES for the autonomous-loop dispatch path. The carve-out is universal (any AlwaysOn skill with DB-sourced LLM override, not self-dev-specific) — see Phase 0.4 for scope justification. (a) skill.toml keyword addition reserved as fallback only if F1's #463 reading is overturned on second pass. (c) `MatchReason::SystemEvent` reserved for future if event markers proliferate beyond `[GitHub]` and `[callback:`.
- **Composition (two wake-up paths) — option γ chosen.** With (b) shipped, sonnet handles the webhook turn and (per design contract `agent.rs:4427-4430`) calls `send_message` on `global_dispatch_active`. Layer 2 wires an engine-side auto-fired deferred callback. **Both paths fire; existing `validate_dispatch_readiness()` arbitrates.** No prompt-layer awareness of deferred callbacks; no engine-layer awareness of `send_message` calls. If the operator manually re-applies `ready` while a deferred callback is pending, the dispatch-readiness check blocks the manual retry (or vice versa). Whichever fires first wins; the loser hits the existing dispatch-readiness guard.
  - **Why γ over α/β:** α requires prompt-level awareness of engine-internal state (engine→prompt layer leak); β requires engine awareness of LLM actions (prompt→engine layer leak). γ is the only option that preserves the engine/prompt layer separation. Per `mika/docs/architecture/review-guide.md` § Separation of Concerns.
- **Layer 2 race window — single transaction with `BEGIN IMMEDIATE`.** Insert deferred callback row → check blocking task status → if completed, promote → COMMIT. Atomic in SQLite WAL mode (the `IMMEDIATE` write lock prevents `handle_task_complete` from committing its own callback drain mid-transaction). Two-transaction-with-retry shape rejected as KISS violation — adds complexity without correctness benefit in WAL mode. Implementation site: in `executor.rs` near the existing `validate_dispatch_readiness()` call where `global_dispatch_active` is detected. Use the existing `with_db` transaction wrapper pattern from `db.rs` (the same shape used for transactional tool-call writes per #636).
- **Layer 2 SilentTrigger variant — `DeferredDispatch` with required-action set `{run_claude_pilot}`.** New variant distinct from `Silent::Callback`. Framing: `[callback:deferred-dispatch]`. INTENT_GUARDS entry (sibling to `callback_terminal_action` at `agent.rs:4507-4512`) requires only that the agent attempt `run_claude_pilot` — no `update_task_status` or `send_message` requirement, because a deferred-dispatch retry has no completed sub-run to report. The agent's only job on the deferred-dispatch turn is to re-fire the original dispatch.

## Open Questions

### Resolved During Planning

- **Q: Does `apply_overrides()` actually write the DB row into the manifest?** Yes — confirmed at `skills/mod.rs:481-489`. The DB read path is correct; the keyword-filter is the gate.
- **Q: Does `resolve_skill_llm_override()` get called on autonomous-loop turns?** Yes — at `agent.rs:2057` (conversation mode), which is the path webhook events take.
- **Q: Why didn't the 2026-04-26 architect dogfood catch this?** The mika-arch override is on `mika-arch-groom-ticket` skill, which is keyword-matched (`groom`, `groom ticket`). mika-arch's primary use cases trigger keyword match; mika-dev's primary use case (webhook events) does not.

### Resolved on first-pass review (mika-arch session `8a315f0a-...`, ITERATE)

All questions previously deferred to architect are now pinned in Phase 0 and Key Technical Decisions:
- Layer 1 sub-option (b) chosen (Phase 0.1, §0.4 carve-out scope).
- Race window: single transaction with `BEGIN IMMEDIATE` (Key Technical Decisions).
- Silent variant: new `SilentTrigger::DeferredDispatch` with required-action set `{run_claude_pilot}` (Key Technical Decisions).
- Composition (α/β/γ): γ chosen (Key Technical Decisions).
- Reaper/watchdog regression: verified in Phase 0.5 (no inference).

### Deferred to Implementation

- **Concrete callback metadata shape for the deferred-dispatch task.** `action_config` JSON content — needs to encode "retry the original `run_claude_pilot` call with original arguments." Settled when implementer reads the existing callback task creation pattern in `executor.rs::spawn_long_running_exec()`.
- **Sub-option (a) keyword choice — `"[GitHub]"` vs `"labeled ready"` vs both.** Only relevant if architect on second-pass overturns Phase 0.1's #463-rationale reading and falls back to (a); then implementer reads the gateway's `format_event_text()` output to confirm the literal prefix.

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

## Phase 0 — Verification (read source material before any code change)

This section was added per architect first-pass review (mika-arch session `8a315f0a-fa6e-4d6e-947b-a78f08c9f662`, ITERATE on F1/F2/F3/NF3/NF4). It pins the source material the plan depends on, so future readers and second-pass review can evaluate without leaving the document.

### 0.1 — mika#463 rationale (the precedent for the Keyword-only filter)

mika#463 verbatim (closed 2026-04-06):

> **Problem:** `resolve_skill_llm_override()` in `agent.rs` iterates ALL matched skills (including `always_on`) when collecting `[llm]` overrides. This means an `always_on` skill with a hardcoded `[llm]` section hijacks the LLM provider for **every turn**, regardless of whether the user's message triggered that skill's keywords.
>
> **Observed behavior:** mika-dev's `self-dev` skill is `always_on = true` with `[llm] provider = "openrouter", model = "qwen/qwen3-coder-plus"`. When the agent config was changed to `x-ai/grok-4.1-fast`, every turn still used `qwen3-coder-plus` because `self-dev` was always in the matched set.
>
> **Impact:** Agent config changes (`config.toml` or `mika model`/`mika provider` commands) are silently overridden by always_on skills with `[llm]` sections.
>
> **Fix:** Change `resolve_skill_llm_override()` to accept `&[MatchedSkill]` instead of `&[&SkillEntry]`, and filter to `MatchReason::Keyword` only — matching the `collect_required_tools` precedent.

**Decisive observation for sub-option (b):** The #463 rationale was specifically about `always_on` skills with **hardcoded `[llm]` sections in `skill.toml`** (a developer-time artifact baked into the skill manifest). The 2026-05-07 issue is about **`skill_overrides` DB rows** (an operator-time artifact, explicit per-agent intent). These are different sources of `entry.manifest.llm`:

- skill.toml `[llm]` is now deprecated per #504 (`crates/mika-agent/CLAUDE.md` § Skills System: *"`[llm]` section no longer supported in `skill.toml` (#504)"*). The original #463 concern is largely moot today because the load-bearing source is gone.
- DB `skill_overrides` is the only path that populates `entry.manifest.llm` today, via `apply_overrides()` at `skills/mod.rs:481-489`.

**Conclusion:** The #463 rationale does not bind for DB-sourced overrides. An additive carve-out for `MatchReason::AlwaysOn` AND DB-source LLM is structurally consistent with #463's intent — operator config changes (the very thing #463 protected) become EFFECTIVE when shipped via DB, not silently overridden.

### 0.2 — mika#265 disambiguation (the cited "precedent" appears stale)

mika#265 verbatim (closed 2026-03-26): about `mika ask` vs TUI claude-pilot execution — *"Mika announces implementation but doesn't execute via `mika ask`."* This issue is unrelated to skill matching or `MatchReason`.

`crates/mika-agent/CLAUDE.md` § Skills System cites `#265` for "Match-reason conditioning." `crates/mika-agent/src/agent.rs:3768` cites `(#265, #463)` as the precedent. **Both citations appear stale or miscited** — the actual mika#265 issue does not establish the `MatchReason::Keyword`-only filter for `collect_required_tools()`. Possible explanations: a private/earlier issue tracker, a re-numbered issue, or a documentation drift.

**Decision:** This does not change the architectural call (the #463 rationale stands on its own). Note in Unit 3 (doc updates) that the stale `#265` citation in `crates/mika-agent/CLAUDE.md` and `agent.rs:3768` should be removed or corrected to "match-reason filter pattern" without a specific issue reference.

### 0.3 — Existing test surface for `resolve_skill_llm_override`

Read `crates/mika-agent/src/agent.rs:6716-6864` (5 tests). Verbatim assertions:

| Test (line) | Setup | Assertion |
|---|---|---|
| `_returns_none_for_empty_matched` (6733) | empty `matched` vec | `is_none()` — no overrides at all |
| `_ignores_always_on_skills` (6744) | self-dev with `LlmOverride { qwen }`, `MatchReason::AlwaysOn` | `is_none()` — *"always_on skills should not impose [llm] override"* |
| `_ignores_dependency_skills` (6767) | dev-pilot with sonnet, `MatchReason::Dependency` | `is_none()` — *"dependency skills should not impose [llm] override"* |
| `_mixed_reasons_only_keyword_considered` (6786) | self-dev AlwaysOn + skill-review Keyword (no LLM) | `is_none()` — *"only keyword-matched skills with [llm] should produce an override"* |
| `_keyword_match_on_always_on_skill_applies` (6817) | self-dev keyword-matched | gets PAST `overrides.is_empty()` check (returns None due to Settings=None, but proves Keyword-matched always_on works) |
| `_same_provider_short_circuit` (6844) | qa-review keyword-matched, same provider as base | `is_none()` — short-circuit |

**Test fixture:** `make_skill_entry_with_llm()` at line 6718 directly populates `entry.manifest.llm = LlmOverride { provider, model }`. This simulates the **skill.toml `[llm]` source path**, NOT the DB-override source path. There is no existing fixture for "skill.manifest.llm came from `apply_overrides()`."

**Changes required under sub-option (b) carve-out:**
- `_ignores_always_on_skills` (6744): assertion stays valid — the test fixture sets `entry.manifest.llm` without setting any DB-origin marker, so under the carve-out it still represents a non-DB override and is correctly ignored. **No change to this test.** Add a sibling test below.
- New test `_always_on_skill_with_db_override_applies`: create skill entry with `entry.manifest.llm` populated, marked as DB-sourced (via the new `from_db_override` flag — see § 0.4). `MatchReason::AlwaysOn`. Assert override IS imposed (gets past `overrides.is_empty()` check; with Settings, returns Some).
- `_mixed_reasons_only_keyword_considered` (6786): assertion stays valid — same reason as above (no DB marker on the test fixture).
- Other tests: unchanged.

### 0.4 — Code path: `MatchReason::AlwaysOn` reaches `resolve_skill_llm_override()` today

Read `agent.rs:2057` (conversation-mode call site): `let mut matched = params.skills.match_message(params.user_message);` returns `Vec<MatchedSkill<'_>>` containing all match reasons. `resolve_skill_llm_override(&matched, ...)` receives the whole vec.

Inside the function, the filter at `agent.rs:3773-3774`:
```rust
if ms.reason != MatchReason::Keyword {
    continue;
}
```

skips both `AlwaysOn` and `Dependency` entries. So `MatchReason::AlwaysOn` DOES reach the function but is filtered out at iteration. The carve-out fix shape: relax the filter for AlwaysOn entries that ALSO have a DB-origin LLM override.

**Carve-out scope: universal, not self-dev-specific.** Any `AlwaysOn` skill with a DB-sourced LLM override qualifies. Today only mika-dev's self-dev qualifies in production, but the precedent permits any future agent's always_on skill with `mika skills llm set` to get the same behavior. This is intentional — the operator's `mika skills llm set` is the canonical statement of intent.

**Implementation: introduce `LlmOverride::from_db_override: bool`.** Add a bool field to `LlmOverride` struct (in `crates/mika-agent/src/skills/manifest.rs` or wherever `LlmOverride` is defined). Default `false`. `apply_overrides()` at `skills/mod.rs:481-489` sets `entry.manifest.llm.from_db_override = true` when populating `provider`/`model` from DB. The filter at `agent.rs:3773-3774` becomes:

```rust
if ms.reason == MatchReason::Keyword {
    // existing path
} else if ms.reason == MatchReason::AlwaysOn && ms.entry.manifest.llm.from_db_override {
    // new path: AlwaysOn + DB override qualifies
} else {
    continue;
}
```

Pseudocode only — directional, not implementation specification.

### 0.5 — Reaper / watchdog regression check (verified, no inference)

`crates/mika-agent/src/db.rs:4929-4970` — `find_orphaned_parent_tasks()` SQL verbatim filter:
```
AND child.status = 'delivered'
```
Pending deferred callbacks have `status='pending'`, not `'delivered'`. **Reaper does not match.** No regression.

The reaper also has a `NOT EXISTS (... sibling.status IN ('pending', 'in_progress'))` clause that defers reaping if any sibling task is active. This actually HELPS: if a parent has both a `delivered` claude-pilot child AND a `pending` deferred-dispatch sibling, the reaper waits. Correct behavior.

`crates/mika-agent/src/task_engine/engine.rs:411-412` — `check_callback_process_liveness()` calls `get_active_callback_tasks_with_pid()`. Pending deferred callbacks have NO `process_id` (they haven't dispatched a subprocess yet). **Watchdog does not match.** No regression.

### 0.6 — `apply_overrides()` write of DB row into manifest

`crates/mika-agent/src/skills/mod.rs:481-489` verbatim:
```rust
if ov.llm_provider.is_some() || ov.llm_model.is_some() {
    if let Some(p) = &ov.llm_provider {
        entry.manifest.llm.provider = Some(p.clone());
    }
    if let Some(m) = &ov.llm_model {
        entry.manifest.llm.model = Some(m.clone());
    }
    entry.has_override = true;
}
```

`entry.has_override` is a generic flag set on ANY DB override (always_on, llm, enabled). Not specific enough to distinguish DB-LLM-sourced vs other DB overrides. Hence the new `LlmOverride::from_db_override` field per § 0.4.

## Implementation Units

### Unit 1: Layer 1 — relax `resolve_skill_llm_override()` for AlwaysOn + DB-sourced overrides (sub-option b)

- [ ] **Unit 1: Override-scope carve-out**

**Goal:** Extend `resolve_skill_llm_override()` to honor DB-sourced LLM overrides on `MatchReason::AlwaysOn` skills, so mika-dev's `self-dev → anthropic/claude-sonnet-4-6` DB override applies on `[GitHub] Issue labeled ready on …` webhook turns. Universal carve-out (any AlwaysOn skill with DB override), not self-dev-specific.

**Requirements:** R2

**Dependencies:** None in code (independent of Unit 2). Phase 0.1, 0.3, 0.4, 0.6 verification must complete before this unit.

**Files:**
- Modify: `crates/mika-agent/src/skills/manifest.rs` — add `from_db_override: bool` field to `LlmOverride` struct (default `false`)
- Modify: `crates/mika-agent/src/skills/mod.rs:481-489` — `apply_overrides()` sets `entry.manifest.llm.from_db_override = true` after writing DB-sourced provider/model
- Modify: `crates/mika-agent/src/agent.rs:3766-3784` — `resolve_skill_llm_override()` filter relaxation; update the `(#265, #463)` comment per Phase 0.2 (remove stale #265 reference, replace with concrete description of the DB-override carve-out and link to mika#1011)
- Test: `crates/mika-agent/src/agent.rs` test module (~line 6716+) — add `test_resolve_skill_llm_override_always_on_with_db_override_applies` (sibling to `_keyword_match_on_always_on_skill_applies` at 6817). Use a new test fixture `make_skill_entry_with_db_llm()` that sets both `LlmOverride { provider, model }` AND `from_db_override = true`.

**Approach:**

The filter relaxation is targeted: `MatchReason::AlwaysOn` qualifies for override resolution ONLY when `entry.manifest.llm.from_db_override == true`. This preserves the #463 protection against `skill.toml [llm]` hijacks (the deprecated path) while allowing operator DB intent to apply on autonomous-loop turns.

Pseudocode (directional — not implementation specification):

```rust
for ms in matched {
    let qualifies = match ms.reason {
        MatchReason::Keyword => true,
        MatchReason::AlwaysOn => ms.entry.manifest.llm.from_db_override,
        MatchReason::Dependency => false,
    };
    if !qualifies { continue; }
    // existing override collection logic unchanged
}
```

The deduplication / conflict-detection / same-provider short-circuit logic at `agent.rs:3790-3865` is unchanged. If two qualifying skills disagree on provider/model, the existing `falling back to default provider` warn path fires.

**Patterns to follow:**
- `LlmOverride` struct shape — currently has `provider: Option<String>, model: Option<String>`; adding `from_db_override: bool` follows the existing field pattern.
- `apply_overrides()` already sets `entry.has_override = true` (line 488) — `from_db_override` is symmetric but more specific.
- Existing test `_keyword_match_on_always_on_skill_applies` at `agent.rs:6817` is the closest neighbor; the new sibling test mirrors its structure.

**Test scenarios:**
- *Happy path:* webhook user message `[GitHub] Issue labeled ready on senara-solutions/mika-platform#85 — feat(slash): /mika-onboarding` matches self-dev as `AlwaysOn` with `from_db_override = true`. `resolve_skill_llm_override()` returns `Some(provider=anthropic, model=claude-sonnet-4-6)`. `effective_llm.model_name() == "claude-sonnet-4-6"`.
- *Edge case (regression guard for #463 concern):* skill with `always_on = true` and `LlmOverride` populated from skill.toml `[llm]` (so `from_db_override = false`) — must NOT trigger override. The existing test `_ignores_always_on_skills` at `agent.rs:6744` covers this and its assertion still holds.
- *Edge case:* skill matched both as `Keyword` (user typed an action verb) AND has DB override — Keyword path wins (existing behavior preserved), override applies. No regression on existing `_keyword_match_on_always_on_skill_applies` test at `agent.rs:6817`.
- *Edge case:* skill matched as `Dependency` with DB override — must NOT trigger override (Dependency reason still filtered).
- *Edge case:* `webhook_no_unauthorized_dispatch` guard (`agent.rs:4464`) still blocks unauthorized dispatch on non-ready-label `[GitHub]` events. Unit 1 only affects model selection, not dispatch authorization — guards untouched.
- *Edge case:* multiple agents with different DB overrides on the same skill name — fix is per-agent (each agent's `apply_overrides()` populates only its own manifest copy). Concurrent `mika ask --agent X` and `mika ask --agent Y` see their respective overrides.
- *Integration:* webhook-triggered dispatch turn produces `llm_calls.model = anthropic/claude-sonnet-4-6` end-to-end on a fresh `~/.mika/data/mika.db` with the `skill_overrides` row set via `mika skills llm set`.

**Verification:**
- `llm_calls` rows for a webhook-triggered mika-dev dispatch turn show `model = anthropic/claude-sonnet-4-6` on every step.
- `prompt_variant.self-dev` reflects the resolved variant for sonnet (per provider/model variant resolution at `inject_skills_and_resolve_tools()`).
- New test `_always_on_with_db_override_applies` passes.
- All existing `test_resolve_skill_llm_override_*` tests (`agent.rs:6716-6864`) still pass without modification.
- No regression on `webhook_no_unauthorized_dispatch` (PR-comment / check-suite webhook events fall through correctly per existing guard).

---

### Unit 2: Layer 2 — deferred-dispatch callback registration

- [ ] **Unit 2: Auto-register deferred callback on `global_dispatch_active`**

**Goal:** When `run_claude_pilot` is rejected with `global_dispatch_active`, the engine creates a `pending` callback task linked to the requesting parent task. When the blocking task completes, the deferred callback fires and resumes the requesting session.

**Requirements:** R1, R3, R4

**Dependencies:** None in code (independent of Unit 1).

**Files:**
- Modify: `crates/mika-agent/src/skills/executor.rs:785-814` — `global_dispatch_active` rejection path inserts a deferred callback inside a `BEGIN IMMEDIATE` transaction that also re-checks blocking-task status and immediate-promotes if drained
- Modify: `crates/mika-agent/src/task_engine/dispatcher.rs` — `handle_task_complete()` Ok-path: after firing blocking task's existing callback, drain next pending deferred callback (FIFO via `created_at ASC`) under same transaction discipline
- Modify: `crates/mika-agent/src/task_engine/mod.rs` (or wherever `SilentTrigger` enum lives) — add `DeferredDispatch` variant. Wire it through `run_silent_agent` framing logic so `[callback:deferred-dispatch]` prefix is injected
- Modify: `crates/mika-agent/src/agent.rs` `INTENT_GUARDS` array (~line 4405-4513) — add `deferred_dispatch_action` entry per Key Technical Decisions (required-action set: `{run_claude_pilot}`)
- Modify: `crates/mika-agent/src/agent.rs:4427-4430` and `4548-4551` — update the design-contract comment scope: `global_dispatch_active` still surfaces to the LLM (γ composition — LLM may call `send_message` in parallel) but now ALSO has an engine-side deferred-callback auto-recovery path. Other terminal errors (`task_not_dispatchable`, `dispatch_blocked_by`, `dispatch_check_failed`) still delegate to LLM via `send_message`, unchanged
- Test: `crates/mika-agent/src/skills/executor.rs` test module — add `global_dispatch_active_registers_deferred_callback` (asserts insert + status under `BEGIN IMMEDIATE`)
- Test: `crates/mika-agent/src/skills/executor.rs` test module — add `global_dispatch_active_immediate_promote_when_blocking_already_drained` (TOCTOU close)
- Test: `crates/mika-agent/src/task_engine/dispatcher.rs` test module — add `deferred_callback_fires_on_blocking_completion` (FIFO drain)
- Test: `crates/mika-agent/src/task_engine/dispatcher.rs` test module — add `mika583_invariant_preserved_with_deferred_queue` (never two `in_progress` callbacks at once)
- Test: `crates/mika-agent/src/agent.rs` test module — add `intent_guard_deferred_dispatch_requires_run_claude_pilot` (NF1 verification)
- Test: `crates/mika-agent/tests/eval/` — end-to-end scenario covering "two ready-label webhooks fire; first dispatches, second is deferred; deferred resumes after first completes; both produce PRs"

**Approach:**

When `executor.rs::validate_dispatch_readiness()` (or equivalent) detects `global_dispatch_active`, the rejection path acquires a write transaction, inserts the deferred callback, checks blocking-task status, and (if the blocking task already completed) promotes the just-inserted deferred callback to `in_progress` for immediate dispatch — all atomically.

**Single transaction with `BEGIN IMMEDIATE`:** the SQLite WAL-mode write lock prevents `handle_task_complete` from committing its own callback drain mid-transaction. This closes the TOCTOU window where the blocking task could finish between "in-flight check" and "deferred callback insert," leaving the deferred callback orphan-pending forever.

Pseudocode (directional — not implementation specification):

```rust
// Inside executor.rs, after detecting global_dispatch_active:
db.with_db_transaction(|tx| {
    // 1. Insert deferred callback
    let deferred = tx.insert_task(&NewTask {
        parent_task_id: Some(requesting_task_id),
        agent_id: requesting_agent_id,
        trigger_type: "callback",
        action_type: "resume_agent",
        status: "pending",
        label: "long_running:run_claude_pilot:deferred",
        action_config: json!({
            "trigger_kind": "deferred_dispatch",
            "original_call": { "prompt": ..., "skill": ..., "task_id": ... }
        }),
        ..
    })?;
    // 2. Re-check blocking task status under the same lock
    let blocking_still_active = tx.has_active_callback_tasks_excluding(requesting_task_id)?;
    // 3. If blocking already drained, promote immediately
    if blocking_still_active.is_none() {
        tx.update_task_status(deferred.id, "in_progress")?;
        // Caller will dispatch the Silent::DeferredDispatch turn after txn commits
    }
    Ok(())
})?;
return Err(global_dispatch_active_json); // rejection JSON unchanged
```

In `dispatcher.rs::handle_task_complete()` for the blocking task, AFTER firing the blocking task's existing callback, query for the next-oldest `pending` deferred callback and promote it. Same transaction discipline (`BEGIN IMMEDIATE`):

```rust
// FIFO promotion under write lock
let next = tx.query_one(
    "SELECT id, agent_id, action_config FROM tasks
     WHERE label = 'long_running:run_claude_pilot:deferred'
       AND status = 'pending'
     ORDER BY created_at ASC LIMIT 1"
)?;
if let Some(t) = next {
    tx.update_task_status(t.id, "in_progress")?;
    // Caller will dispatch Silent::DeferredDispatch turn after txn commits
}
```

The deferred callback fires via the new `SilentTrigger::DeferredDispatch` path (NOT `Silent::Callback`). Framing: `[callback:deferred-dispatch] <action_config payload>`. INTENT_GUARDS entry distinct from `callback_terminal_action`:

```rust
// In INTENT_GUARDS array (agent.rs near 4505):
IntentPrecondition {
    label: "deferred_dispatch_action",
    trigger: |msg| msg.starts_with("[callback:deferred-dispatch]"),
    satisfied: |summaries| summaries.iter().any(|s| s.name == "run_claude_pilot"),
    correction_message: "[Your response was rejected. This is a deferred-dispatch \
         retry — the prior run_claude_pilot was rejected with global_dispatch_active. \
         The dispatch slot is now free. You MUST re-invoke run_claude_pilot with the \
         original arguments to complete the deferred dispatch. Do not call \
         update_task_status, send_message, or any other tool first.]",
},
```

The agent's only required action on a deferred-dispatch turn is `run_claude_pilot`. No `update_task_status` (the parent task is still `in_progress`, no terminal transition). No `send_message` (operator is informed via the original sonnet rejection-handling turn under the γ composition).

**Composition with mika#583 invariant:** never more than one `in_progress` callback. The promotion logic enforces this — only one deferred callback promotes per `handle_task_complete` cycle, and the inline immediate-promote case at insert time only fires when there's no active callback.

**Composition with mika#871 reaper:** verified in Phase 0.5 — `find_orphaned_parent_tasks` requires `child.status = 'delivered'`. Pending deferred callbacks have `status='pending'`. No match.

**Composition with mika#959 watchdog:** verified in Phase 0.5 — `check_callback_process_liveness` calls `get_active_callback_tasks_with_pid()`. Pending deferred callbacks have no `process_id` (they haven't dispatched a subprocess). No match.

**Patterns to follow:**
- Existing callback task creation in `executor.rs::spawn_long_running_exec()` — `action_config` JSON shape, parent linkage.
- `dispatcher.rs::handle_task_complete()` Ok-path drain pattern (already drains webhook deferral queue per mika#528 — same mechanic).
- `task_engine::dispatcher::dispatch_resume_agent` — Silent dispatch site (extend to handle `SilentTrigger::DeferredDispatch`).
- Existing INTENT_GUARDS entries in `agent.rs:4405-4513` — pattern for `label`, `trigger`, `satisfied`, `correction_message`.
- Existing `with_db` transaction wrapper pattern (`crates/mika-agent/src/async_db.rs`) — RAII `rusqlite::Transaction` (DEFERRED by default; use `BEGIN IMMEDIATE` explicitly for the write-lock semantics needed here, per mika#636 transaction discipline).

**Test scenarios:**
- *Happy path:* Task A dispatches `run_claude_pilot`; while in flight, Task B attempts `run_claude_pilot` and is rejected with `global_dispatch_active`. A pending callback row exists with `parent_task_id = B.id`. When Task A's callback delivers, B's deferred callback promotes to `in_progress` within one engine tick and fires under `SilentTrigger::DeferredDispatch`.
- *Happy path (TOCTOU close):* Task A's blocking callback completes between Task B's rejection-decision and pending-callback insert. Under `BEGIN IMMEDIATE`, the transaction sees the post-completion state on the re-check; B's deferred callback is inserted AND immediately promoted to `in_progress` in the same transaction. No orphan-pending state.
- *Edge case:* Three tasks queue (A in flight, B and C deferred). A completes → B promotes, C stays pending. B completes → C promotes. FIFO order preserved (`ORDER BY created_at ASC`).
- *Edge case:* Task B is cancelled while its deferred callback is `pending`. Cancellation cascades — deferred callback transitions to `cancelled` (not promoted on A's completion).
- *Edge case:* Task A's claude-pilot crashes (subprocess dies; mika#959 watchdog marks A's callback `failed`). B's deferred callback still promotes — slot is free either way.
- *Edge case (γ composition):* Task B's rejection-handling LLM turn calls `send_message` to notify the operator AND the engine registers the deferred callback. Operator receives notification; deferred callback promotes when slot frees. Operator manually re-applies `ready` while deferred callback is pending → manual retry hits `validate_dispatch_readiness()` and fails with `task_active_dispatch` (for the pending sibling) — operator's retry no-ops correctly.
- *Edge case (INTENT_GUARDS — NF1):* Deferred-dispatch turn LLM responds with text only and no tool calls → `intent_guard_deferred_dispatch_requires_run_claude_pilot` rejects; correction message instructs re-invoke of `run_claude_pilot`.
- *Edge case (INTENT_GUARDS isolation):* Deferred-dispatch turn does NOT trigger `callback_terminal_action` guard — that guard's `[callback:` prefix-match must NOT include `[callback:deferred-dispatch]`. Verify with a test that asserts the guard's trigger predicate distinguishes the two prefixes.
- *Error path:* `db.rs` insert of pending callback row fails (e.g., FK constraint, disk full). Original `run_claude_pilot` rejection still returns; LLM falls back to existing `send_message` path. Defense-in-depth — Layer 1 keeps the LLM grounded enough to handle this.
- *Error path:* Two blocking tasks complete simultaneously (rare). Each `handle_task_complete` tries to promote the next deferred. `BEGIN IMMEDIATE` write lock serializes them; the second one finds either the next-oldest deferred callback or an empty queue.
- *Integration:* mika#583 single-session-at-a-time invariant holds — at no point are two callbacks `in_progress` simultaneously. Asserted via a 3-task scenario where promotions happen serially.
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
- Modify: `crates/mika-agent/CLAUDE.md` § Skills System — update the "Per-skill LLM override" paragraph to flag the carve-out behavior (`AlwaysOn` skills with DB-sourced overrides via `from_db_override` flag also qualify; original #463 protection against `skill.toml [llm]` hijacks preserved); also fix the stale `#265` reference per Phase 0.2 (remove or correct to a non-issue-number citation like "match-reason filter pattern")
- Modify: `crates/mika-agent/CLAUDE.md` § Unified Task Engine — add a paragraph on deferred-dispatch callback semantics: registration on `global_dispatch_active`, FIFO drain in `handle_task_complete`, `SilentTrigger::DeferredDispatch` framing, γ composition with operator-notification path
- Modify: `crates/mika-agent/src/agent.rs:3766-3768` — update the `(#265, #463)` comment per Phase 0.2 (remove stale `#265` reference; re-state the precedent as "AlwaysOn skills with developer-time `[llm]` sections do not impose LLM override; AlwaysOn skills with operator-time DB overrides DO impose")
- Modify: `crates/mika-agent/src/agent.rs:4427-4430, 4548-4551` — update inline comments to reflect the γ composition: `global_dispatch_active` STILL surfaces to the LLM (which may call `send_message`) AND has an engine-side deferred-callback auto-recovery path; the two paths are independent and `validate_dispatch_readiness()` arbitrates any race. Other terminal errors unchanged.
- Test expectation: none — pure documentation.

**Patterns to follow:**
- Existing `crates/mika-agent/CLAUDE.md` paragraph style — concise, version-pinned, file:line cited.
- Inline comment style at `agent.rs:4427-4430` — references issue numbers + design intent. Cite `mika#1011` for the new behavior.

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
| Layer 1 carve-out for `AlwaysOn` + `from_db_override` may surface unexpected behavior in mika-arch's existing flows | mika-arch's overrides are on keyword-matched skills (`mika-arch-groom-ticket`, `mika-arch-second-review`); sub-option (b) doesn't change behavior for keyword-matched paths. Affects only AlwaysOn-only matched skills with explicit DB overrides — today only self-dev on mika-dev qualifies. Add explicit test coverage that mika-arch's groom-ticket flow continues to use sonnet on keyword-match (regression unchanged). |
| Layer 2 deferred-callback flood: malicious or buggy agent fires many `run_claude_pilot` calls; queue grows unbounded | Add a per-agent cap on pending deferred callbacks (e.g., 10). Existing `max_agent_tasks_per_session` (`db.rs`) is the precedent. Implementer settles the exact cap based on operational signals; default 10 is a safe starting point. |
| Layer 2 stale deferred callbacks: parent task is cancelled but deferred callback orphans | Cascade delete or status-cascade — mark deferred callback `cancelled` when parent is `cancelled`/`failed`. Add to the existing `update_task_status` cascade logic in `db.rs`. |
| Layer 2 immediate-promote path (TOCTOU close) needs operator visibility: when `global_dispatch_active` rejection IMMEDIATELY promotes the just-inserted deferred callback, the rejection JSON still surfaces to the LLM, which may call `send_message` saying the dispatch was rejected — even though the engine already auto-recovered. Operator could see a misleading notification | Acceptable under γ — the LLM's message is informational; the deferred callback's actual dispatch result will land on its own callback turn, providing the authoritative outcome. Optional: add a one-line annotation to the rejection JSON when immediate-promote fires (`"deferred_dispatched_immediately": true`) so the LLM can phrase the notification appropriately. Implementer judgment. |
| Phase 0.2's stale `#265` citation indicates documentation drift: the actual mika#265 GitHub issue is unrelated to skill matching. Other code/doc references to `#265` for "match-reason precedent" may exist that this plan doesn't catch | Implementer must `grep -rn "#265" crates/mika-agent/src crates/mika-agent/CLAUDE.md` during Unit 3 doc updates. Each occurrence: verify it cites the actual issue or remove/replace per Phase 0.2's correction. Out of scope: tracking down the original issue tracker reference (if any). |

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
