---
title: "fix(agent): required-tools-gate retry must produce a self-contained final response"
type: fix
status: active
date: 2026-04-29
---

# fix(agent): required-tools-gate retry must produce a self-contained final response

## Overview

Extend the engine's required-tools-gate correction message (and the mika-arch skill prompts as defense-in-depth) to instruct the LLM that its corrected response must restate the full content rather than referring to a prior turn. Today's failure mode: when the gate rejects an `EndTurn` and forces a tool call, the corrected response often becomes a thin pointer ("see above", "Disposition stands") because the LLM treats the prior reasoning as still in context — but only the final `EndTurn` turn is persisted to `messages`. Substantive review content is lost.

## Problem Frame

Per `crates/mika-agent/CLAUDE.md` — § Three-Layer Memory Model and § Conversation Compaction — the `messages` table holds one row per completed assistant turn. By design (see `architecture/architecture.md` § 6 and observability in `crates/mika-common/CLAUDE.md`), only `EndTurn` / `MaxTokens` / `StopSequence` stop reasons persist assistant text; `ToolUse` turns persist tool inputs/outputs but not the assistant's accompanying narration. `llm_calls` is metadata-only by explicit design — token counts, latency, stop reason, no response body (`MIKA_LOG_LLM_BODIES=false` in production).

The required-tools-gate retry path at `crates/mika-agent/src/agent.rs:1109-1129` pushes the rejected assistant response into `request.messages` (in-memory only — not persisted), injects a `User` correction message, and continues the loop. The next LLM step generates a NEW response with the prior turn visible in its in-memory context — but the LLM frequently produces a brief pointer-summary ("Disposition stands: ITERATE with the revised findings above") because the prior reasoning is "already in context" from the model's perspective. When that brief response lands on `EndTurn`, it becomes the only persisted assistant message. The substantive content is gone.

In-the-wild reproduction: `/mika-groom-ticket mika#886` first-pass on 2026-04-29 (mika-arch session `f2e6595d-c402-48c8-bcfb-db47b1ec420a`). Five LLM steps, ~10k output tokens of substantive review across steps 0 and 2. Final `EndTurn` (step 4): 111 tokens, 363 chars persisted. Two named architect concerns, zero citations, unactionable for `/mika-groom-ticket` Phase 4.

## Requirements Trace

- R1. After this fix, a session that triggers the required-tools-gate retry produces a final assistant message that is **self-contained** — operationally measured by the marker-based assertion in Unit 3 (the rejected step's deterministic markers reappear in the corrected final turn, in order). NOT a length-floor heuristic; see § Key Technical Decisions for the rationale.
- R1a (chain-composition invariant — verified pre-commit via `crates/mika-agent/src/agent.rs:1109-1249`): guards #3 (required-tools), #4 (completion-claim, lines 1191-1213), and #5 (fabricated-action-claim, lines 1237+) all use the same retry shape — push the rejected assistant response onto `request.messages`, then push the `User` correction, then `continue`. They do NOT reset the messages queue between guards. The new self-contained instruction injected by guard #3 therefore persists across any subsequent guard retry: on a #3-then-#4 retry chain, the LLM sees the original user prompt AND #3's self-contained correction AND #4's correction. Verified by reading the cited line ranges; pin in plan as the chain-composition invariant.
- R2. The fix benefits every skill that opts into `[constraints] required_tools` — not just mika-arch. The engine-side change is the primary defense; the skill-prompt change is reinforcement. **Blast-radius survey (verified 2026-04-29):** five bundled skills opt in — `mika-arch-groom-ticket`, `mika-arch-second-review`, `qa-review`, `self-dev`, `skill-review`. **Zero** community skills in `mika-skills/` opt in. All five bundled consumers produce final-artifact assistant text (review verdicts, plans, completion summaries) — none are pipeline skills that pass tool outputs to a downstream stage. Broad sweep is safe; no scope-narrowing required. Verified via `grep -rln "required_tools" skills/bundled/*/skill.toml mika-skills/` (only 5 hits, all in `skills/bundled/`).
- R3. No change to the `messages` table persistence contract. One row per completed turn stays. No new column on `llm_calls`. No mid-loop `ToolUse` text persistence.
- R4. The grooming pipeline (`/mika-ask-arch` consumer, `/mika-groom-ticket` Phase 3 step 10) parses by line-content, not by session-replay. After this fix, `metadata.session_id` extraction continues unchanged; only the captured `.content` becomes self-contained.
- R5. A frozen, **hand-crafted, model-agnostic** regression fixture proves the failure mode reproduces without the fix and is suppressed with the fix. Fixture is NOT captured from real runs — it represents the *contract shape* (rejected step contains markers; final turn must repeat them), so the test is stable across LLM updates.

## Scope Boundaries

- The fix is in `mika` only. There is no `mika-skills` change — both mika-arch skills are engine-coupled bundled skills at `skills/bundled/mika-arch-{groom-ticket,second-review}/` (verified by directory listing).
- We do NOT add a structural guard ("reject if final turn ≤ N% of rejected step content"). Rationale below; the fix is contract-level (LLM instruction) before structural-level (engine-enforced length floor). If the fix lands and the failure mode recurs, that's N=2 territory and warrants a separate ticket.
- We do NOT touch the persistence contract. The one-row-per-turn invariant is load-bearing for `load_recent_messages(20)`, compaction thresholds, and the A2A single-writer contract.
- We do NOT add an `assistant_intermediate_text` column on `llm_calls`. That's the N=2 escalation path if the contract-level fix proves insufficient.
- We do NOT modify the disposition-keyword paraphrase tolerance in `/mika-groom-ticket` — that's a separate drift family tracked in `mika/docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md`.

### Deferred to Separate Tasks

- **Structural length-floor guard** on required-tools-gate retry final turn: deferred. Add only if contract-level fix proves insufficient.
- **Persistence-side fix** (per-LLM-step content storage): deferred. Bigger blast radius; not warranted yet.
- **Architect evasion taxonomy update** (today's batch of 7 sessions where mika-arch fabricated tool unavailability): separate batch-update to `mika/docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`. Different fix surface.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/agent.rs:1099-1130` — required-tools-gate rejection branch. Lines 1118-1128 hold the correction message that this plan extends. The retry pushes the rejected assistant response into `request.messages` (in-memory only), injects the User correction, and `continue`s.
- `crates/mika-agent/src/agent.rs:1080-1098` — terminal-failure-bypass branch (#516). Reference for how the gate decides retry vs. accept; the new instruction lives only in the retry branch.
- `skills/bundled/mika-arch-groom-ticket/system_prompt.md` (59 lines, sections: Operating Discipline / Process / Output / Constraints).
- `skills/bundled/mika-arch-second-review/system_prompt.md` (61 lines, same section structure).
- `crates/mika-agent/CLAUDE.md` § Post-Conditions (EndTurn Chain) item #3 — the gate's documented contract; this plan extends the contract to include "self-contained corrected response."
- `crates/mika-agent/tests/eval/test_required_tools_gate.rs` — existing integration test surface for the gate. Pattern: `MockLlmProvider` with sequence-of-responses, `EvalHarness` builder, asserts on final response and tool-call ordering.
- `crates/mika-agent/tests/eval/grounding_regressions/` — canonical home for fabrication-detection scenarios. Pattern (per `README.md`): one file per scenario named `{class}_{shape}_{descriptor}.rs`, frozen pre-fix JSON fixture under `fixtures/`, hard assertions via `tests/eval/grounding_assertions/mod.rs` helpers (`assert_response_contains`, `assert_response_forbids`, `assert_tool_called_before_response`).

### Institutional Learnings

- `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` — sibling doc tracking architect evasion of the gate. Today's bug is a *transport/contract* failure (the gate fired correctly, but the post-correction turn was thin); the doc tracks *evasion* failures (the gate didn't fire because the LLM rationalized around its trigger). Different families. This plan adds a brief cross-reference in the predecessor doc but does NOT extend the evasion taxonomy here.
- `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — known disposition-keyword paraphrase drift (e.g., "Proceed" instead of "READY"). Tolerated by `/mika-groom-ticket`. Different family from this bug.
- `docs/solutions/architecture-patterns/per-turn-tool-use-dedup-guard.md` — the original of the post-condition-guard family. The discipline "scope the defense to the smallest unit that preserves intent" applies here: extending the correction message benefits every skill that opts into `required_tools`, not just mika-arch.
- `docs/architecture/review-guide.md` § 6 (citation-or-silence) — the principle the corrected response must satisfy. Self-contained-by-default is the precondition for citation-bearing output.

### External References

None. The fix is purely internal — instruction text in two places, a fixture, a test scenario.

## Key Technical Decisions

- **Two fix surfaces, both inside `mika`.** Engine correction message (broad) + mika-arch skill prompts (narrow reinforcement). Both land in this plan; the engine change goes first in commit order so the engine-level test exercises it before the skill text.
- **Contract-level fix before structural-level.** A length-floor structural guard (reject EndTurn if `len(final) < 0.5 * len(rejected_step_response)`) was considered and rejected for v1. Rationale: structural guards work when there's a clean signal; here the signal is fuzzy (some legitimate corrections genuinely shorten — e.g., the rejected response contained fabricated content that the corrected one rightly drops). Land the contract first; promote to structural only if contract proves insufficient. `feedback_prompt_enforcement_fragile.md` warns against prompt-level budgets — but this isn't a budget, it's a coordination rule between the engine's persistence contract and the LLM's output behavior. The latter is the right layer for this *specific* fix.
- **Same rationale rejects a length-floor TEST assertion in favor of marker-based assertions.** The original ticket Acceptance #2 prescribed a `≥ 80%` length heuristic. Same fuzziness applies at the test layer: a corrected response that legitimately drops fabricated content from the rejected step would fail a length-floor assertion despite being correctly self-contained. Marker-based assertions (deterministic `[KEY-FINDING-N]` markers in the rejected step's content; the corrected final turn must contain the same markers in order) test the *contract* (the LLM repeated the named findings) without depending on response volume. Issue body Acceptance #2 was updated to reflect this on 2026-04-29 with an edit-notice comment citing this plan as the trigger.
- **Engine instruction includes the model (the *why*), not just the rule.** Proposed exact text: *"When you produce your corrected response, restate the full content — do not reference your prior turn. Only the final response is persisted to the conversation log; prior turns exist only in the in-memory loop context."* The cost is ~15 tokens; the gain is rationale-clarity that reduces the LLM's latitude to rationalize around the rule. The existing correction message is already infrastructure-detailed (it names the rejection cause, the gate's mechanism, and the fabrication prohibition); one more clause about persistence is consistent with the message's tone.
- **Skill prompt instruction names the failure mode explicitly.** "Only the final response is persisted." gives the LLM the model of *why* the rule exists; reasoning models are likelier to comply when they understand the contract.
- **Fixture-driven regression test, hand-crafted, model-agnostic.** Frozen pre-fix JSON fixture under `tests/eval/grounding_regressions/fixtures/required_tools_retry_thin_final_turn_pre_fix.json`. The fixture represents the *contract shape* (rejected step contains markers; final turn is thin pointer-summary without markers), not a captured-from-real-run snapshot. Hand-crafted is correct here because real-run capture would tie the test to one model's phrasing patterns; hand-crafted is model-agnostic and stable across LLM updates. Two scenarios: regression (proves the assertion catches the failure on the pre-fix fixture) + post-fix happy path (proves a self-contained final turn passes).
- **Test home: `grounding_regressions/`.** Per its README, the directory is for fabrication-detection scenarios with hard assertions and frozen fixtures. The "thin final turn" failure is a fabrication-adjacent contract violation (the LLM is fabricating that the prior content is "still there"). Right home; same pattern as `asserted_unavailability_caught` and `required_suffix_line_caught`. New tag `grounding:transport-contract` registered in the README's vocabulary alongside the existing fabrication tags — the categorical distinction (what the LLM said vs. what got persisted) is real but the test pattern is identical to siblings.
- **Explicit non-goal: don't touch `/mika-ask-arch` or `/mika-groom-ticket`.** The grooming-pipeline consumer parses `metadata.session_id` and `.content` from the JSON envelope; both fields stay unchanged. The fix lands upstream of the consumer.
- **The "see above" anti-phrase is illustrative, not exhaustive.** The skill prompt names "see above" as the canonical anti-phrase but the broader rule is "do not refer to prior turns." Keep the rule prose short, the anti-phrase as one example.
- **Predecessor-doc cross-reference shape (Unit 4):** one-paragraph addition at the END of `required-tools-gate-evasion-patterns-2026-04-28.md`'s tail. **Do NOT** extend the evasion taxonomy with a new entry — would dilute the doc's focus. The pointer says "this doc tracks evasion (gate didn't fire). For transport-contract failures (gate fired but post-correction turn was thin), see [mika#890 compound doc / this plan]."

## Open Questions

### Resolved During Planning

- **Q: mika-skills change or mika change?** mika. Both mika-arch skills (`mika-arch-groom-ticket`, `mika-arch-second-review`) are engine-coupled bundled skills inside the mika repo at `skills/bundled/`. Verified by directory listing. The friend's brief said "mika-skills" — directionally right (it's the mika-arch skill prompts), repo-wise wrong.
- **Q: Should the fix be one ticket or two?** One. Both surfaces are in mika; the engine change is the primary defense and the skill change reinforces it. Splitting introduces sequencing tax with no parallelism benefit.
- **Q: Add a metric for "thin final turn after retry"?** Out of scope for v1. The grounding-regression test fixture is the dev-time signal; if the failure recurs in production, file an observability ticket separately.
- **Q: What's the regression-suppress assertion?** A two-arm assertion: (a) the rejected step's content includes a marker citation (`[KEY-FINDING-1]`, `[KEY-FINDING-2]`) and the final EndTurn must contain those same markers; (b) the final EndTurn length must exceed a threshold proportional to the rejected step (≥ 0.5×). The fixture controls both arms.
- **Q: Is the "feedback_architect_criterion_change_loopback.md" family the same?** No. That family is *semantic criterion drift* (architect critique replaces the issue's decision criterion). This is *transport/contract* (the gate's retry produces non-self-contained output). Different bug families per peer review. Do not promote to compound on this third instance.

### Deferred to Implementation

- Exact phrasing of the engine correction-message addendum — "When you produce your corrected response, restate the full content — do not reference your prior turn." is the proposed text from peer review, but the implementer may tune (e.g., adding "since only the final response is persisted") if the test fixture demonstrates the fuller phrasing improves recovery.
- Exact location of the skill-prompt addendum — at the end of `### Constraints` is the natural slot in both files; implementer to confirm placement preserves the existing flow.
- Whether the regression test belongs alongside `required_suffix_line_caught` (sibling guard) or in a sub-directory grouping required-tools failures together — directory-organization detail; implementer's call.

## Implementation Units

- [ ] **Unit 1: Extend the engine correction message**

**Goal:** Make the required-tools-gate retry path instruct the LLM that the corrected response must be self-contained.

**Requirements:** R1, R2, R4

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/agent.rs` (the format string at lines 1122-1126)

**Approach:**
- Extend the format string used for the User correction message at lines 1119-1128 to include the self-contained-response instruction with the model. The new text is appended to the existing `[Your response was rejected because you did not call the required tool(s)...]` body, before the closing `]`. Proposed exact wording (per Key Technical Decisions § engine-instruction-includes-the-model): *"When you produce your corrected response, restate the full content — do not reference your prior turn. Only the final response is persisted to the conversation log; prior turns exist only in the in-memory loop context."*
- No control-flow changes. The retry mechanics (push assistant response, inject User correction, `continue`) stay identical. Only the User correction text grows.
- **Chain-composition invariant** (R1a, verified at `crates/mika-agent/src/agent.rs:1109-1249` pre-commit): the new instruction persists across subsequent guard retries because guards #3, #4, #5 all append to `request.messages` rather than reset it. No additional code is required to make the new instruction visible to the LLM on second-stage retry.

**Patterns to follow:**
- Existing format-string composition style at `crates/mika-agent/src/agent.rs:1122-1126`.
- The terminal-failure-bypass branch (lines 1080-1098) and the early-accept guards (per CLAUDE.md § Post-Conditions item 3b) for understanding which branches the new text DOES NOT touch.

**Test scenarios:**
- Happy path (Unit-test): a `MockLlmProvider` sequence where step 0 emits `EndTurn` without calling required tools, gate forces retry, step 1 calls the required tool, step 2 emits a final `EndTurn` containing the required content. Assert: the final-turn assistant message contains substantive content (≥ length floor as defined by the fixture markers).
- Regression assertion shape: the User correction message in `request.messages` must contain the new self-contained instruction. Inspect via the test harness's request-capture hook (see `EvalHarness.captured_messages()` or equivalent in `MockLlmProvider`'s assertion API).
- Edge case: if step 0 already produced an `EndTurn` that DOES call the required tool (no retry triggered), the new instruction is NOT injected. Assert no User correction message is appended.

**Verification:**
- Existing `crates/mika-agent/tests/eval/test_required_tools_gate.rs` continues to pass.
- The new unit-level test in that file (or a sibling) passes both pre-fix and post-fix assertion arms.

---

- [ ] **Unit 2: Reinforce the contract in mika-arch skill prompts**

**Goal:** Add a "self-contained final response" behavior clause to both mika-arch skill prompts so the model has the contract internalized at skill-prompt time, not only at retry time.

**Requirements:** R1, R2 (defense-in-depth)

**Dependencies:** None (independent of Unit 1)

**Files:**
- Modify: `skills/bundled/mika-arch-groom-ticket/system_prompt.md`
- Modify: `skills/bundled/mika-arch-second-review/system_prompt.md`

**Approach:**
- Append a single short paragraph to each file's `### Constraints` section. Proposed wording (exact text deferred to implementation):
  > Your final response must be self-contained. If a prior turn was rejected (e.g., by the required-tools gate) and you re-issued the review after fetching ground truth, restate the full annotated findings in your final response — do not refer to prior turns with phrases like "see above." Only the final response is persisted.
- Both files get the same paragraph (verbatim) — keeps the contract identical between first-pass and second-pass paths.

**Patterns to follow:**
- Existing `### Constraints` section structure in both files.
- Citation-or-silence framing already used in the prompts (see `### Output` sections).

**Test scenarios:**
- Test expectation: none — pure prompt text. Coverage is via Unit 3's grounding-regression scenario, which exercises both prompts under the failure mode.

**Verification:**
- Manual diff review: each file gains exactly one short paragraph in `### Constraints`. No restructuring, no removal of existing content.
- Lint: both files still parse as valid markdown; `validate_skill()` (per CLAUDE.md § Skills System) does not warn.

---

- [ ] **Unit 3: Add grounding-regression scenario for thin-final-turn failure**

**Goal:** Frozen regression fixture + scenario that proves the failure mode reproduces without the fix and is suppressed with it.

**Requirements:** R1, R5

**Dependencies:** Unit 1 (engine fix). Unit 2 is independently verified by reading the diff.

**Files:**
- Create: `crates/mika-agent/tests/eval/grounding_regressions/required_tools_retry_thin_final_turn.rs`
- Create: `crates/mika-agent/tests/eval/grounding_regressions/fixtures/required_tools_retry_thin_final_turn_pre_fix.json`
- Modify: `crates/mika-agent/tests/eval/grounding_regressions/mod.rs` (register the new scenario)
- Modify: `crates/mika-agent/tests/eval/grounding_regressions/README.md` (add scenario to the capability matrix and tag vocabulary)

**Approach:**
- Pre-fix fixture content (**hand-crafted, model-agnostic** — see Key Technical Decisions § fixture-driven-regression-test for rationale): a JSON document representing the LLM call sequence. Step 0 produces a substantive review with explicit citation markers (e.g., `[KEY-FINDING-1] Concern: key shape ...`, `[KEY-FINDING-2] Concern: record-at-dispatch failure mode ...`) but does NOT call the required tool; step 1 forces the tool call (e.g., `gh_read`); step 2 is the rejected-style thin pointer (`"Disposition stands: ITERATE with the revised findings above"`); step 3 is `EndTurn` with the same thin content. Do NOT capture the fixture from a real run — that would tie the test to one model's phrasing patterns.
- **Marker-based assertion (NOT length-floor)** — see Key Technical Decisions § same-rationale-rejects-a-length-floor for rationale. The assertion shape is `assert_response_contains_in_order(&["[KEY-FINDING-1]", "[KEY-FINDING-2]"])`. The fixture controls the markers; LLM phrasing isn't tested.
- Test scenario uses `EvalHarness` + `MockLlmProvider` to replay the fixture. Two arms:
  - **Regression-reproduction** (no engine fix): the captured final turn fails the marker assertion. This proves the assertion has teeth.
  - **Post-fix happy path** (with engine fix): a sibling fixture (or programmatic mock) where step 2 emits a self-contained corrected response containing both markers. Assertion passes.
- Register the new tag `grounding:transport-contract` in `tests/eval/grounding_regressions/README.md`'s vocabulary alongside the existing fabrication tags. The categorical distinction (what the LLM said vs. what got persisted) is real, but the test pattern (frozen pre-fix fixture, marker assertions, two arms) matches sibling scenarios exactly.

**Execution note:** Test-first. Land the fixture and the failing-assertion regression-reproduction test BEFORE Unit 1's engine change. After Unit 1, the post-fix assertion arm should pass. This locks the contract behaviorally, not just textually.

**Patterns to follow:**
- `crates/mika-agent/tests/eval/grounding_regressions/asserted_unavailability_caught.rs` (sibling guard, same shape).
- `crates/mika-agent/tests/eval/grounding_regressions/required_suffix_line_caught.rs` (manifest-driven post-condition, same shape).
- `crates/mika-agent/tests/eval/grounding_assertions/mod.rs` for assertion helpers — use existing helpers when they fit, add new ones only if a sibling helper doesn't already cover the assertion shape.

**Test scenarios:**
- **Regression-reproduction** (Pre-fix fixture): fixture's final turn = `"Disposition stands: ITERATE with the revised findings above"`. Assertion: `assert_response_contains_in_order(&["[KEY-FINDING-1]", "[KEY-FINDING-2]"])` FAILS. The test catches the failure class. Tag: `grounding:transport-contract` (failure tag).
- **Post-fix happy path**: fixture's final turn restates both KEY-FINDING markers. Assertion passes. Tag: `grounding:transport-contract` (success tag).
- **Edge case (no retry triggered)**: fixture where step 0 calls the required tool on first try and emits a self-contained `EndTurn`. Assertion passes; the new gate instruction was not injected (separate verification at Unit 1's level — this scenario lives there).
- **Integration with Unit 1**: with the Unit 1 fix in place, the regression-reproduction fixture's `User` correction message is observed to contain the self-contained instruction (assertion via test harness's request-capture hook).

**Verification:**
- `cargo test -p mika-agent --test eval grounding_regressions::required_tools_retry_thin_final_turn` passes both arms.
- The full grounding-regressions suite continues to pass.
- README's capability matrix and tag vocabulary include the new entry.

---

- [ ] **Unit 4: Cross-reference in the required-tools-gate evasion patterns doc**

**Goal:** Make the predecessor doc (which named the *evasion* failure mode) point at this fix as the *transport* failure mode that's adjacent but distinct.

**Requirements:** R3 (documentation parity — the persistence contract is unchanged; this fix is contract-level, not persistence-level)

**Dependencies:** Units 1–3 merged

**Files:**
- Modify: `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` (one short subsection at the bottom of the doc)

**Approach:**
- One short paragraph appended at the **END** of the predecessor doc's tail (after Resolution / Scope sections, before any closing references). Names the bug class ("transport contract: gate fires correctly but post-correction turn is thin") and points at this fix's commit / PR / compound doc.
- **Do NOT extend the evasion taxonomy with a new entry.** The predecessor doc tracks evasion (gate didn't fire); today's bug is a sibling family (gate fired, post-correction turn was thin). Adding a new evasion-taxonomy entry would dilute the doc's focus. The cross-reference is a pointer, not a taxonomy extension.

**Test scenarios:**
- Test expectation: none — documentation update.

**Verification:**
- Diff review: the doc gains exactly one short subsection. The existing taxonomy is unchanged.

## System-Wide Impact

- **Interaction graph:** The fix touches the agent loop's required-tools-gate retry branch. It affects every skill that opts into `[constraints] required_tools` (per CLAUDE.md § Skills System). Today that includes mika-arch skills, but the contract is generic.
- **Error propagation:** No change. The gate's retry semantics, terminal-failure-bypass, and PR-review-early-accept paths are untouched.
- **State lifecycle risks:** None. No new persistent state. No new in-memory state. Pure instruction text.
- **API surface parity:** No external API change. `mika ask --format json --verbose` envelope unchanged. `metadata.session_id` extraction in `/mika-ask-arch` unchanged.
- **Integration coverage:** Unit 3's grounding-regression scenario covers the full retry path end-to-end. The existing `crates/mika-agent/tests/eval/test_required_tools_gate.rs` continues to cover the gate's own rejection-and-retry mechanics.
- **Unchanged invariants:** `messages` table one-row-per-turn contract; `llm_calls` metadata-only contract; `MIKA_LOG_LLM_BODIES` default `false`; `EndTurn` / `MaxTokens` / `StopSequence` as the only persistence-eligible stop reasons; the post-condition guard chain composition (1 → 2 → 3 → 3b → 4 → 5 → 6 → 6b → 7 → 8) — none of these change.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| LLMs sometimes treat in-context content as "already said" even with explicit instructions — the fix may not fully suppress the failure mode | Defense-in-depth: engine-side instruction (every skill) + skill-prompt instruction (mika-arch). Frozen fixture proves the assertion has teeth. If the failure recurs after both fixes land, escalate to the structural-guard / persistence-side conversation per the deferred-tasks list. |
| Engine instruction text becomes too long and crowds out the existing correction body | Keep the addendum to one sentence. Avoid lecturing about persistence model — that lives in the skill prompt. |
| Skill-prompt addendum is the wrong layer (prompt-enforcement-fragile per `feedback_prompt_enforcement_fragile.md`) | Engine-side change is the primary defense; skill-prompt is reinforcement. The engine is the authoritative layer. |
| Test fixture is too rigid (e.g., relies on exact LLM phrasing) and breaks on reasonable model variation | Use marker-based assertions (`[KEY-FINDING-1]` etc.) embedded in the fixture's pre-fix step-0 content. Markers are deterministic; LLM phrasing isn't tested. |
| The fix interacts unexpectedly with the asserted-unavailability guard (#862, post-condition #6b) | Both guards live in the same chain but operate on different signals (#3 = required tools missing; #6b = assistant claims tool unavailable). They don't share state. The new instruction is in the User correction at the #3 retry — it won't be re-emitted by #6b. Verified by reading the chain composition. |
| The fix conflicts with terminal-failure-bypass (#516, lines 1080-1098) | The bypass branch sets `required_tools_retry_done = true` and falls through; it never reaches the new instruction text. Verified by reading the code. |

## Documentation / Operational Notes

- No env-var changes, no migration, no rollout sequencing. The fix is engine code + skill prompts + tests, all in one PR.
- Rollback: `git revert` of the PR. No data to clean up.
- The compound doc that `/ce:compound` produces for this fix should explicitly distinguish *transport-contract* drift from *evasion* drift; the next time the gate is touched, the distinction prevents lumping.

## Grooming History

- 2026-04-29 — `/ce:plan` initial draft (this file).
- 2026-04-29 — mika-arch first-pass review (session `cb16ca68-5212-4fdf-aabe-9a8bf57062d6`): **Disposition: ITERATE**. Two blockers (F1 issue body Fix 2 path correction; F2 Acceptance #2 length-floor vs marker-based assertion). Six sharpenings (F3 chain composition trace; F4 blast radius grep; F5 grounding-regressions tag README entry; F6 hand-crafted fixture pinning; F7 one-sentence-plus-model recommendation; F8 predecessor-doc cross-reference shape). All eight applied. Issue body updated 2026-04-29 with edit-notice comment citing this plan.

## Sources & References

- Origin ticket: senara-solutions/mika#890
- Predecessor: senara-solutions/mika#270 — required-tools gate post-condition origin
- Predecessor: senara-solutions/mika#272 — observability doc establishing `llm_calls` as metadata-only
- In-the-wild reproduction: mika-arch session `f2e6595d-c402-48c8-bcfb-db47b1ec420a` during `/mika-groom-ticket mika#886` first-pass on 2026-04-29
- Sibling: senara-solutions/mika#886 — the cross-session duplicate-review fix whose grooming surfaced this bug; resumes once mika#890 lands
- Sibling: `mika/docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` — adjacent-but-distinct failure family (evasion vs. transport-contract)
- Persistence contract: `mika/docs/architecture/architecture.md` § 6 (one row per completed turn; only `EndTurn` / `MaxTokens` / `StopSequence` save text)
- Test-pattern source: `crates/mika-agent/tests/eval/grounding_regressions/README.md` (frozen-fixture + hard-assertion convention)
