---
title: "fix(core-memory): first-write exemption + cap raise for per-session update cap"
type: fix
status: active
date: 2026-08-20
issue: 1782
---

# fix(core-memory): first-write exemption + cap raise for per-session update cap

## WHY

On 2026-07-17 Vincent's cloud Mika (customer `57703ff5-…`, tier=family, provider=openrouter, model=z-ai/glm-5.2) hit `MAX_CORE_MEMORY_EDITS_PER_SESSION = 3` during a natural onboarding turn. The agent tried to seed all 5 core-memory blocks from their placeholder defaults; the 4th write was capped and the agent narrated the truncation to Vincent verbatim:

> "j'ai atteint la limite de mises à jour mémoire pour cette session, donc un bloc (self_model) n'a pas pu être mis à jour complètement. Pas critique — ce sera corrigé au prochain échange."

The post-onboarding DB state confirms: 4 of 5 blocks populated, `workflows` never written. The agent's assertion that "it will heal next exchange" is unverified (and structurally weak — the retry depends on the model spontaneously noticing the gap on a future turn without any prompt cue).

## Root cause (two contributing factors)

1. **`check_onboarding` is user_summary-only and one-shot.** `agent_loop::check_onboarding` at `crates/mika-agent/src/agent_loop/mod.rs:2489-2501` returns `true` iff `user_summary` still equals its default. Once `user_summary` is customized (typically write #1 of the onboarding turn), **any subsequent session** — even mid-bootstrap sessions where 4 of 5 blocks are still default — sees `is_onboarding=false`. If the onboarding turn fails mid-stream (mika#1781 parse error opened a new session in Vincent's case), the retry session loses the exemption despite still being in bootstrap-shape work.
2. **Cap of 3 is tighter than the natural onboarding footprint.** The system prompt (`prompt::onboarding_prompt`) explicitly instructs the agent to "seed all 5 blocks" during first contact. 5 > 3 — the cap contradicts the prompt directive it is meant to protect.

The safety the cap was protecting against remains valid: **runaway model writes that churn already-customized blocks on a long chat**. That protection should stay. What should NOT be protected against is **first-time writes to blocks still at their default** — those are bootstrap-shape work by definition and are naturally bounded to 5 (one per block).

## The fix

Two changes to `crates/mika-agent/src/tools/update_core_memory.rs`:

### Change 1 — First-write exemption

Fetch `existing` (the current block value) *before* the rate-limit check. Compute `is_first_write` = the current block value is `None` OR equals the section's canonical default. When `is_first_write == true`:
- Skip the rate-limit check.
- Do NOT increment `core_memory_edit_count` (first-writes don't consume the budget).

This means: **the cap counts only updates (writes to already-customized blocks). Writes that promote a default block to a customized value are always allowed.** The absolute per-session ceiling is now `MAX_CORE_MEMORY_EDITS_PER_SESSION` updates + up to 5 first-writes = 10 writes in the pathological case (only reachable if the agent resets a customized block mid-session, which itself would be cap-counted).

`default_self_model` is looked up per-agent (via `ctx.db.get_agent_display_name()`) because the default template is formatted with the agent's display name (`"I am {display_name}. No interaction history yet."`). Other section defaults come from the static `CORE_MEMORY_SECTIONS` array.

### Change 2 — Raise cap from 3 to 5

`MAX_CORE_MEMORY_EDITS_PER_SESSION: u32 = 3` → `5`. Rationale:

- Matches the block count — an agent can meaningfully update every block once in a single steady-state session.
- Matches `MAX_CORE_MEMORY_EDITS_REFLECTION`, aligning the two envelopes.
- With the first-write exemption already handling the bootstrap case, this change addresses the steady-state case (an agent that learns significant new information about the user and wants to refine multiple blocks in one turn).

### Change 3 — Update the cap-hit error message

The current message ("Core memory edit limit ({max_edits}) reached for this session. Focus on using your existing knowledge.") is misleading after the fix — the agent might still have first-write budget for default blocks. Update to:

> "Core memory edit limit ({max_edits}) reached for this session. You can still write to blocks that are at their default value (first-writes are exempt from the cap)."

This guides the LLM toward the correct recovery path.

### Change 4 — Update the tool-schema description

The current description ("You have {} blocks: {section_list}. Each block is limited to ~500 tokens.") is silent on the cap. Add one sentence naming the semantics so the model can reason about them proactively:

> "Rate limit: up to {MAX_CORE_MEMORY_EDITS_PER_SESSION} updates per session, plus one 'first-write' per block that is still at its default value (bootstrap writes are exempt from the cap)."

## Not changed

- **`check_onboarding` stays as-is.** Broadening it would flip the `is_onboarding` prompt injection ("## First Session — This is your first conversation with the user…") back on if the agent ever `reset`s user_summary, which is confusing tone drift. The first-session prompt is correctly one-shot; the cap exemption is what needs to be more forgiving.
- **`MAX_CORE_MEMORY_EDITS_REFLECTION = 5` stays at 5.** Reflection is a scheduled bulk-refinement turn; its cap is already well-sized. First-write exemption still applies to reflection turns for the same reason it applies to conversation.
- **Post-action hooks, audit events, reflection-evidence gate.** All unchanged.

## Test coverage

The existing `test_rate_limit_triggers` and `test_reflection_edit_cap_is_five` tests need to be updated because they intermix first-writes with updates (the first write of every test is to a still-default block, which is now exempt).

**Updates to existing tests:**

- `test_rate_limit_triggers` — reworked to demonstrate the new semantic: 1 first-write (exempt) + 5 updates all succeed, 6th update fails. Confirms cap raise (3→5) and first-write exemption together.
- `test_reflection_edit_cap_is_five` — reworked similarly for the reflection path (cap 5 unchanged, but first-write exemption applies there too).
- `test_rate_limit_exempt_during_onboarding` — unchanged (already exercises the onboarding path via `ctx_with_onboarding(true)`).

**New tests (bug regressions):**

- `test_bootstrap_five_blocks_from_default_succeeds` — Vincent's scenario reproduced. Non-onboarding session (is_onboarding=false), agent writes all 5 blocks from their defaults. All 5 succeed with the fix. On the pre-fix binary, only the first 3 succeed (cap fires on the 4th).
- `test_first_write_exempt_after_cap_hit` — After 5 updates (cap hit), a subsequent write to a still-default block succeeds. Proves the first-write exemption is orthogonal to update-cap exhaustion.
- `test_updates_still_capped_after_bootstrap` — Agent writes all 5 blocks (first-writes, exempt), then attempts a 6th write (update to any block). Update-cap enforces from write #1 onward for that block class; after 5 updates the 6th fails. Confirms runaway-update protection is preserved.
- `test_reset_then_update_counts_as_update` — Reset a customized block (cap-counted) then re-write it (before_value is now default → treated as first-write again → exempt). This behaviour is intentional; the test documents it so future readers understand the semantic.
- `test_reason_is_default_reports_first_write` — `default_self_model` is computed with the agent display name; the first-write check must use the same source of truth. Test seeds the agent with a non-`mika` display name and confirms first-write detection works for `self_model`.

**Injection-verified discipline (`feedback_verify_pipeline_passes_without_the_fix`):**
- All 7 new/updated regression tests FAIL on main (reverting `MAX_CORE_MEMORY_EDITS_PER_SESSION` back to 3 + removing the first-write exemption). PASSES after the two changes above are applied. Verified by temporarily reverting the impl in-place and re-running `cargo test -p mika-agent --lib update_core_memory` — all 7 fired with the expected "Core memory edit limit (3) reached for this session" message.

## Definition of Done

- All acceptance criteria pass.
- `cargo fmt`, `cargo clippy -p mika-agent --lib -- -D warnings`, `cargo test -p mika-agent --lib update_core_memory` all clean.
- Full multi-agent review completed with disposition on every finding.
- PR body includes `Closes #1782` + WHY-first framing (onboarding evidence → agent's own truncation report → the fix).

## Acceptance criteria

- [ ] `MAX_CORE_MEMORY_EDITS_PER_SESSION` is raised from 3 to 5.
- [ ] `update_core_memory` exempts first-writes (before_value is None or equals the section default) from the per-session cap AND does not increment the edit counter on first-writes.
- [ ] `default_self_model(ctx.db.get_agent_display_name())` is used as the default source for `self_model` first-write detection.
- [ ] The cap-hit error message names the first-write exemption explicitly (guides LLM recovery).
- [ ] The tool-schema description names the rate-limit semantic (updates + first-writes) so the model can reason about it proactively.
- [ ] Regression test `test_bootstrap_five_blocks_from_default_succeeds` reproduces Vincent's onboarding scenario (5 first-writes on a non-onboarding session) and passes.
- [ ] Regression test `test_first_write_exempt_after_cap_hit` proves first-write exemption is orthogonal to cap exhaustion.
- [ ] Regression test `test_updates_still_capped_after_bootstrap` proves runaway-update protection is preserved.
- [ ] Regression test `test_reset_then_update_counts_as_update` documents the reset-then-write semantic.
- [ ] Regression test `test_reason_is_default_reports_first_write` proves `self_model` first-write detection uses the per-agent default template.
- [ ] Existing tests updated where the new semantic changes expected outcomes: `test_rate_limit_triggers`, `test_reflection_edit_cap_is_five`.
- [ ] Unchanged behaviour asserted: onboarding path still exempts (`test_rate_limit_exempt_during_onboarding`), reflection cap unchanged at 5, evidence-required rules unchanged.

## Sources

- **Issue:** [#1782](https://github.com/senara-solutions/mika/issues/1782)
- **Target file:** `crates/mika-agent/src/tools/update_core_memory.rs` (constant at line 11, cap check around line 145-156 pre-fix)
- **`check_onboarding` reference:** `crates/mika-agent/src/agent_loop/mod.rs:2489-2501`
- **`default_self_model` reference:** `crates/mika-agent/src/db.rs:115-117`
- **Onboarding prompt:** `crates/mika-agent/src/prompt.rs:531-545` (the "seed all 5 blocks" directive)
- **Memory:** `feedback_verify_pipeline_passes_without_the_fix`, `feedback_prompt_enforcement_fragile` (any prompt-only "self-heal next exchange" claim is unreliable — must be structurally fixed).
- **Founding incident:** cloud Mika onboarding turn 2026-07-17 ~10:00 UTC (customer `57703ff5-…`).
