---
title: "audit(skills): Fix fabrication guard predicate gating on tool-loaded"
type: fix
status: active
date: 2026-05-28
---

# audit(skills): Fix fabrication guard predicate gating on tool-loaded

## Overview

The dev-groom fabrication guard (#1133, position 5b in the post-condition chain) gates on `enabled_tool_names.contains("run_claude_pilot_groom")` — when the tool is not loaded (loader bug, identity allowlist denial, bundled skill exclusion), the guard silently bypasses. This is the exact scenario where fabrication risk is highest. The fix inverts the predicate from "fire only if dispatcher tool present" to "fire always unless agent is a known verdict-producer."

Additionally, audit all guards in `agent.rs` that use tool-presence predicates, classify each as (A) fire regardless, (B) stay gated with documented rationale, or (C) refactor to a more precise predicate, and produce an audit document.

## Problem Frame

The guard at `agent.rs:1464` uses tool presence as a **proxy for agent role discrimination** — is this agent a dispatcher (mika-dev with dev-groom skill, where verdict text is fabrication) or a producer (mika-arch-second-review, where verdict text is legitimate output)?

The proxy fails when the tool that identifies the dispatcher role is absent due to a bug (mika#1251 loader failure) or misconfiguration. The guard silently evaluates to false and the LLM can emit `Verdict: GROOMED`/`Verdict: ESCALATE` without any structural check. Confirmed during mika#1251 investigation.

Same anti-pattern class as gating a security check on the presence of an opt-in.

## Requirements Trace

- R1. Audit doc at `docs/audits/` listing every tool-presence-gated guard with decision (A/B/C) and rationale
- R2. Dev-groom guard (A/C): code change so it fires when tool is absent, not only when present
- R3. Regression tests covering fire-path (fabrication caught) and no-fire-path (legitimate output passes)
- R4. mika#1251-class regression test: LLM emits `Verdict: GROOMED` without successful `run_claude_pilot_groom` call AND tool not loaded → guard fires
- R5. (B) guards: inline comment at gate explaining correctness
- R6. Related: mika#940 premature-EndTurn detection — check whether it uses the same gating pattern

## Scope Boundaries

- Only guards in `crates/mika-agent/src/agent.rs` `run_loop` and its helper functions
- No changes to skill manifests, tool registration, or the skill loader
- No changes to the asserted-unavailability guard's core logic (it uses `enabled_tool_names` correctly)

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/agent.rs:1464-1500` — dev-groom fabrication guard (#1133, position 5b)
- `crates/mika-agent/src/agent.rs:1631-1683` — asserted-unavailability guard (#862, position 6c)
- `crates/mika-agent/src/agent.rs:1277-1338` — completion-claim guard (#483, position 4)
- `crates/mika-agent/src/agent.rs:1127-1133` — prose-style tool call detection (#569, position 2)
- `crates/mika-agent/src/agent.rs:2579-2580` — `enabled_tool_names` snapshot construction
- `crates/mika-agent/src/agent.rs:2770-2774` — skill name access via `params.skills.skills()`
- `tests/eval/grounding_regressions/dev_groom_fabricated_verdict_caught.rs` — existing test scenarios 30-33
- `tests/eval/grounding_regressions/dev_groom_dispatched_no_verdict.rs` — existing no-fire-path test

### Institutional Learnings

- `docs/solutions/agent-quirks/dev-groom-fabricated-verdict-2026-05-20.md` — Root cause: dispatcher/producer manifest shape mismatch. Three-part defense (F1 manifest, F2 prompt, F3 structural guard). This ticket strengthens F3.
- `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` — Three evasion shapes. Rule 2: "not callable" without an attempt is fabrication. The asserted-unavailability guard (#862) is the structural counterpart.

## Key Technical Decisions

- **Invert the predicate (Decision C for the dev-groom guard):** Replace tool-presence check with a verdict-producer exemption. The guard fires by default in conversation mode; agents with known verdict-producer skills (mika-arch-groom-ticket, mika-arch-second-review) are exempted. This means: if the dev-groom skill is loaded but its tool failed to register (mika#1251), the guard fires — correct. If no grooming skill is loaded at all (random agent), the guard fires — conservative but harmless (the verdict regex won't match normal text). If the agent IS a verdict producer (mika-arch), the guard skips — correct.

- **Use a pre-computed boolean, not skill names inside run_loop:** Compute `is_verdict_producer: bool` from the skill registry at each call site (conversation, silent, team) before calling `run_loop`. Thread it as a single `bool` parameter. This avoids threading the entire SkillRegistry into `run_loop` and keeps the interface minimal. The constant `VERDICT_PRODUCER_SKILLS` lives near the guard for locality.

- **Completion-claim guard stays gated (Decision B):** The `tools.get("update_task_status").is_some()` check on the completion-claim guard (#483) is a different concern — it gates a nudge (not a security boundary) for agents that legitimately lack the tool (delegates, team agents). Adding an inline comment explaining the correctness is sufficient.

- **Asserted-unavailability guard stays as-is (Decision B):** It uses `enabled_tool_names` as ground truth (the check's input, not its gate). The predicate is correct by construction.

- **Prose-style tool call detection stays as-is (Not applicable):** Uses tool names to eliminate false positives, not to gate a security check.

## Open Questions

### Resolved During Planning

- **Q: Should the guard also fire in silent/callback mode?** No. Callback turns legitimately carry Verdict lines from the inner session (claude-pilot callback results). The `mode.is_conversation()` gate is correct and unchanged.
- **Q: Does mika#940 use the same pattern?** Checked — #940 is about premature EndTurn detection via the text-based tool call detector (#569, position 1-2). It uses `available_tool_names` to avoid false positives, not as a security gate. No change needed.
- **Q: What if a future skill legitimately produces verdict lines?** Add it to `VERDICT_PRODUCER_SKILLS`. The constant is the single point of truth for role discrimination — a one-line change vs. the current implicit coupling to tool names.

### Deferred to Implementation

- **Exact parameter position in `run_loop` signature:** Insert after `enabled_tool_names` for locality with the guard it serves. The signature already has many parameters; one more bool is tolerable.

## Implementation Units

- [ ] **Unit 1: Audit document**

**Goal:** Produce the audit doc listing all tool-presence-gated guards with decisions

**Requirements:** R1

**Dependencies:** None

**Files:**
- Create: `docs/audits/2026-05-28-001-fabrication-guard-tool-gating-audit.md`

**Approach:**
- Document five guards found in audit: (1) dev-groom fabrication guard — Decision C, (2) asserted-unavailability guard — Decision B, (3) completion-claim guard — Decision B, (4) prose-style tool call detection — Not applicable (false-positive filter, not security gate), (5) milestone-close-claim guard — no tool-presence gating (already correct). For each: cite line numbers, explain the predicate, classify, state rationale.
- Include mika#940 check result (not affected).

**Test expectation:** none — documentation-only unit

**Verification:**
- File exists with all five guards documented, each with a classification and rationale

- [ ] **Unit 2: Add `VERDICT_PRODUCER_SKILLS` constant and `is_verdict_producer` parameter**

**Goal:** Thread a pre-computed verdict-producer flag into `run_loop` so the guard can use it instead of tool presence

**Requirements:** R2

**Dependencies:** Unit 1 (for rationale clarity, but no code dependency)

**Files:**
- Modify: `crates/mika-agent/src/agent.rs`

**Approach:**
- Add a `const VERDICT_PRODUCER_SKILLS: &[&str]` near the dev-groom guard (around line 1448) containing `["mika-arch-groom-ticket", "mika-arch-second-review"]`. These are the only skills whose LLM output legitimately contains `Verdict:` lines. Place near the guard for locality (same pattern as `ASSERTED_UNAVAILABILITY_LABEL`).
- At each of the three `run_loop` call sites (conversation mode ~line 2801, silent mode ~line 3907, team mode ~line 4311), compute `is_verdict_producer` by checking `params.skills.skills().iter().any(|s| VERDICT_PRODUCER_SKILLS.contains(&s.manifest.skill.name.as_str()))` (conversation) or equivalent for the silent/team entry points. For silent mode, always pass `false` (callback turns bypass the guard via `mode.is_conversation()` anyway). For team mode, compute from the team member's skill registry.
- Add `is_verdict_producer: bool` parameter to `run_loop` after `enabled_tool_names`.

**Patterns to follow:**
- The `enabled_tool_names` snapshot pattern: compute at call site, pass as parameter to `run_loop`
- The `ASSERTED_UNAVAILABILITY_LABEL` const pattern: label constant near the guard it serves

**Test scenarios:**
- Happy path: `VERDICT_PRODUCER_SKILLS` contains exactly the two known producer skills
- Edge case: ensure the constant is checked case-sensitively (skill names are lowercase by convention)

**Verification:**
- `run_loop` signature has the new parameter
- All three call sites pass the computed value
- Code compiles

- [ ] **Unit 3: Invert the dev-groom fabrication guard predicate**

**Goal:** Replace `enabled_tool_names.contains("run_claude_pilot_groom")` with `!is_verdict_producer`

**Requirements:** R2

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-agent/src/agent.rs`

**Approach:**
- Change the guard predicate at position 5b from:
  ```
  && enabled_tool_names.contains("run_claude_pilot_groom")
  ```
  to:
  ```
  && !is_verdict_producer
  ```
- Update the inline comment block (lines 1448-1463) to explain the new predicate: the guard fires for all agents in conversation mode except known verdict-producers. When a producer skill is active, verdict text is legitimate output, not fabrication.
- The rest of the guard body is unchanged — the `claims_verdict` regex check and `dispatched` tool-call check remain. If the LLM emits a verdict line and DID call `run_claude_pilot_groom` successfully, the guard still passes (correct). If the tool is absent AND the agent is not a producer, the guard fires on verdict text (the fix).

**Patterns to follow:**
- The existing guard body (claims_verdict + dispatched check) is unchanged
- Single-retry pattern via `dev_groom_fabrication_retry_done` is unchanged

**Test scenarios:**
- Happy path: Agent with dev-groom skill loaded (tool present) emits fabricated verdict → guard fires (existing test, unchanged behavior)
- Happy path: Agent with dev-groom skill loaded (tool present) dispatches then no verdict → guard does not fire (existing test, unchanged behavior)
- Integration: Agent with mika-arch-second-review skill emits Verdict: GROOMED → guard does not fire (producer exemption)

**Verification:**
- The guard predicate no longer references `enabled_tool_names.contains("run_claude_pilot_groom")`
- Existing tests pass (behavior unchanged for normal dev-groom scenario)

- [ ] **Unit 4: Add (B) guard inline comments**

**Goal:** Document correctness rationale for tool-presence gates that are staying as-is

**Requirements:** R5

**Dependencies:** Unit 1 (audit document provides the rationale)

**Files:**
- Modify: `crates/mika-agent/src/agent.rs`

**Approach:**
- At the completion-claim guard (~line 1282-1284), add a comment block explaining why the `tools.get("update_task_status").is_some()` gate is correct: delegates and team agents legitimately lack this tool; the guard is a nudge for task hygiene, not a security boundary; the failure mode (guard skips) is benign (no fabrication risk, just a missed nudge).
- At the asserted-unavailability guard (~line 1631-1640), the existing comment block already explains the pattern well. Add one sentence noting this was reviewed in the mika#1254 audit and classified as Decision B — `enabled_tool_names` is used as ground truth for the check, not as a gate on it.

**Test expectation:** none — comment-only changes

**Verification:**
- Both guards have inline comments explaining their tool-presence gating correctness

- [ ] **Unit 5: Regression test — tool not loaded, verdict fabricated (mika#1251 class)**

**Goal:** Add a regression test that catches the exact scenario from mika#1251: dev-groom skill's tool is NOT in `enabled_tool_names`, LLM emits `Verdict: GROOMED`, guard fires

**Requirements:** R3, R4

**Dependencies:** Unit 3

**Files:**
- Create: `crates/mika-agent/tests/eval/grounding_regressions/dev_groom_fabricated_verdict_tool_absent.rs`
- Modify: `crates/mika-agent/tests/eval/grounding_regressions/mod.rs` (register new module)

**Approach:**
- Create a new test scenario following the pattern of `dev_groom_fabricated_verdict_caught.rs` (scenarios 30-33) but with a critical difference: the SkillRegistry does NOT contain the dev-groom skill (and thus `run_claude_pilot_groom` is not in `enabled_tool_names`).
- However, the agent IS in conversation mode receiving a groom-related message and the LLM fabricates `Verdict: GROOMED`.
- Under the old code, the guard would silently skip. Under the fix, it fires because `!is_verdict_producer` is true (no producer skill loaded either).
- Use `EvalHarness::builder()` with an empty skill registry (or a registry containing only non-producer, non-dispatcher skills).
- Mock two LLM responses: Turn 1 fabricates verdict (guard fires, re-prompt); Turn 2 corrects.
- Assert: `llm_call_count > 1` (re-prompt occurred) and `assert_response_forbids(&trace, &["Verdict: GROOMED", "Verdict: ESCALATE"])`.

**Patterns to follow:**
- `dev_groom_fabricated_verdict_caught.rs` — test structure, `make_dev_groom_skill()` pattern (but omitted here), assertion helpers
- `dev_groom_dispatched_no_verdict.rs` — no-fire-path test structure

**Test scenarios:**
- Happy path: LLM emits `Verdict: GROOMED` with no grooming tool loaded, no producer skill loaded → guard fires, corrective re-prompt issued, final output has no verdict
- Edge case: LLM emits `Verdict: ESCALATE` variant → same behavior

**Verification:**
- Test passes with the fix from Unit 3 applied
- Test would FAIL if the old `enabled_tool_names.contains(...)` predicate were restored

- [ ] **Unit 6: Regression test — verdict producer exemption**

**Goal:** Add a regression test confirming that agents with verdict-producer skills can legitimately emit Verdict lines

**Requirements:** R3

**Dependencies:** Unit 3

**Files:**
- Create: `crates/mika-agent/tests/eval/grounding_regressions/dev_groom_verdict_producer_exempt.rs`
- Modify: `crates/mika-agent/tests/eval/grounding_regressions/mod.rs` (register new module)

**Approach:**
- Create a test scenario where the SkillRegistry contains the `mika-arch-second-review` skill (a known verdict producer).
- The LLM emits `Verdict: GROOMED` in its response.
- Assert: `llm_call_count == 1` (no re-prompt — the guard was exempted) and the response CONTAINS `Verdict: GROOMED` (not stripped).
- This confirms the producer exemption works correctly and prevents regression where the inverted predicate accidentally catches producer agents.

**Patterns to follow:**
- `dev_groom_dispatched_no_verdict.rs` — no-fire-path assertion structure
- `dev_groom_fabricated_verdict_caught.rs` — `make_*_skill()` helper pattern (adapt for mika-arch-second-review)

**Test scenarios:**
- Happy path: Agent with mika-arch-second-review skill emits `Verdict: GROOMED` → guard skips, response preserved
- Edge case: Agent with mika-arch-groom-ticket skill emits `Verdict: READY` → guard skips (READY is not in the claims_verdict regex anyway, but the exemption path is exercised)

**Verification:**
- Test passes confirming producer skills are exempted
- Test would FAIL if the producer exemption were removed

## System-Wide Impact

- **Interaction graph:** The change affects the post-condition chain in `run_loop` only. No callbacks, middleware, or external entry points are affected. The three call sites (conversation, silent, team) each compute the new boolean independently.
- **Error propagation:** No change — the guard's re-prompt mechanism is unchanged.
- **State lifecycle risks:** None — the guard is stateless within a turn (only `dev_groom_fabrication_retry_done` flag, unchanged).
- **API surface parity:** Silent mode and team mode call sites must pass the new parameter. Silent mode always passes `false` (the `mode.is_conversation()` gate handles it). Team mode computes from the team member's skills.
- **Integration coverage:** The new regression tests cover the critical integration path (skill registry → `is_verdict_producer` computation → guard predicate evaluation → re-prompt behavior).
- **Unchanged invariants:** The guard's body (verdict regex, dispatched-tool check, corrective re-prompt text, single-retry flag) is unchanged. The `mode.is_conversation()` gate is unchanged. The `required_suffix_lines` guard (#864) and `required_finding_list` guard (#901) are unaffected.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| New producer skill added without updating `VERDICT_PRODUCER_SKILLS` → guard incorrectly fires on legitimate verdict output | The constant is near the guard with a comment explaining its purpose. Adding a new producer skill is rare (two exist today, both from mika#811). The failure mode is a false-positive re-prompt, not a silent bypass — strictly safer than the current silent bypass. |
| `run_loop` signature grows by one parameter | Acceptable — the function already has 19 parameters. A struct refactor is out of scope. The new bool is grouped with `enabled_tool_names` for locality. |
| Existing tests break if skill name matching is case-sensitive | Skill names in `skill.toml` are lowercase by convention and enforced by `validate_skill()`. The `VERDICT_PRODUCER_SKILLS` constant uses lowercase. |

## Sources & References

- Related issues: mika#1254 (this ticket), mika#1251 (loader bug that surfaced the pattern), mika#1253 (sibling structural defense), mika#1133 (original dev-groom fabrication guard), mika#940 (premature EndTurn — confirmed not affected)
- Related code: `crates/mika-agent/src/agent.rs` (guard chain), `tests/eval/grounding_regressions/` (regression test suite)
- Related docs: `docs/solutions/agent-quirks/dev-groom-fabricated-verdict-2026-05-20.md`, `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`
