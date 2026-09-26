---
title: "test(eval): add Unit 1 eval test for mika-arch-groom-milestone skill"
type: test
status: active
date: 2026-04-29
ticket: senara-solutions/mika#879
branch: feat/879/skills-mika-arch-add-milestone-grooming
parent_plan: docs/plans/2026-04-29-001-feat-mika-arch-milestone-grooming-plan.md
---

# Add Unit 1 eval test for `mika-arch-groom-milestone` skill output contract

## Overview

PR mika#888 ships `mika-arch-groom-milestone` skill but omits the eval-harness test the parent plan explicitly required (Unit 1). mika-qa filed `VERDICT: block[ac]` (review id 4196247084 on commit `0a56d844`, posted 2026-04-29T10:54:47Z) with two unsatisfied ACs: Unit 1 eval test and Unit 3 mika-platform canonical command. **This plan only covers Unit 1.** Unit 3 is out of scope here — tracked separately as `mika-platform#65`.

This is a delta iteration on the open PR. The parent plan (`docs/plans/2026-04-29-001-feat-mika-arch-milestone-grooming-plan.md`) is the architect-validated contract; this delta plan completes the Unit 1 verification step the parent plan declared.

## Problem Frame

The parent plan's Unit 1 (`mika-arch-groom-milestone bundled skill scaffold`) declared the eval test in its `**Files:**` block (`crates/mika-agent/tests/eval/skills/mika_arch_groom_milestone.rs`) and `**Verification:**` step ("A representative agent eval run against a synthetic 3-sub-issue brief produces the trailer-line shape"). The implementation shipped the skill (`skill.toml`, `system_prompt.md`, `tools.json`) but did not ship the eval test. mika-qa correctly identified this as a delivery gap, not a design conflict.

The eval test validates **engine handling** of the milestone-shaped I/O contract — that the agent loop routes a milestone brief to the matched skill, the post-condition guards do not reject milestone-shaped output, and the literal final-line discipline (`Disposition: <KEYWORD>`) flows through unchanged. It does NOT validate that a real LLM would produce this output — that belongs to integration-tier real-provider tests (out of scope per parent plan §C2).

## Requirements Trace

- **R1.** Eval test file exists at the path the parent plan declared (`crates/mika-agent/tests/eval/skills/mika_arch_groom_milestone.rs`).
- **R2.** Test covers all four scenarios the parent plan's Unit 1 § Test scenarios enumerated:
  1. Happy path — synthetic 3-sub-issue brief → output contains `Scope: milestone` line + `Disposition: READY` as the literal final line.
  2. Single-sub-issue edge case — n=1 brief → milestone-shaped output (not per-ticket output).
  3. Conflicting-AC edge case — sub-issues with cross-cutting incompatibility → `Disposition: ITERATE`.
  4. Missing-sections error path — malformed brief without required sections → `Disposition: ESCALATE`.
- **R3.** Test uses `MockLlmProvider` via `EvalHarness::builder()`, matching the existing `tests/eval/` pattern.
- **R4.** Test registration matches the existing `tests/eval.rs` module wiring (new `pub mod skills;` + `pub mod mika_arch_groom_milestone;`).
- **R5.** `cargo test -p mika-agent --test eval` runs cleanly with the new test included; no regression in adjacent tests.

## Scope Boundaries

**In scope:**
- One new test file with four `#[tokio::test]` cases.
- One new module declaration site (`crates/mika-agent/tests/eval.rs` gains `pub mod skills;` + per-skill module).
- One new `mod.rs` for the `tests/eval/skills/` directory.

**Out of scope:**
- Unit 3 (mika-platform canonical operator command) — tracked at `mika-platform#65`.
- Skill prompt changes (`skills/bundled/mika-arch-groom-milestone/system_prompt.md` is on the branch and architect-validated; do NOT edit).
- Any change to the production skill manifest, tools.json, allowlist, or LLM override.
- Real-provider integration tests (parent plan defers these to the gated `MIKA_EVAL_REAL_PROVIDERS` matrix; not part of Unit 1).
- Loading the production bundled skill directly. Tests use synthetic `SkillRegistry::from_test_entries` per the canonical pattern in `crates/mika-agent/tests/eval/grounding_regressions/required_suffix_line_caught.rs`.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/tests/eval/grounding_regressions/required_suffix_line_caught.rs` — canonical pattern for synthetic-skill + mock-LLM + trace-assertion eval tests. The `make_suffix_skill()` helper at lines 38-72 is the template; the four `#[tokio::test]` blocks demonstrate guard-fires/no-fires assertions and final-line shape.
- `crates/mika-agent/tests/eval/test_per_skill_provider_override.rs` — shorter reference for `EvalHarness::builder().responses(...).provider_name(...).build().await` shape.
- `crates/mika-agent/tests/eval/harness.rs` — `EvalHarness` builder (`responses`, `skills`, optional DI hooks).
- `crates/mika-agent/tests/eval/assertions.rs` — `assert_has_output` and other shared assertions.
- `crates/mika-agent/tests/eval.rs` — module entry; new `mod skills` block goes here.
- `crates/mika-agent/src/skills/manifest.rs` — `SkillManifest`, `SkillInfo`, `Triggers`, `Output` types used by synthetic `SkillEntry` construction.
- `skills/bundled/mika-arch-groom-milestone/system_prompt.md` — declares the milestone output contract (Scope: milestone + Disposition: READY/ITERATE/ESCALATE on literal final line). Reference for what the synthetic skill in the test should mimic.

### Institutional Learnings

- Parent plan §D2 — `Scope: milestone` is additive; the literal-final-line `Disposition:` discipline is preserved across all six existing parsers. The eval test asserts this final-line invariant directly.
- `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — origin of the literal-final-line requirement (Kimi paraphrased "Proceed" instead of `Disposition: READY` under load). The eval test is the structural counterpart for the milestone variant.
- Engine post-condition guard #8 (`required-suffix-line guard`, mika#864, see `crates/mika-agent/CLAUDE.md` § Post-Conditions) — `mika-arch-groom-milestone`'s `skill.toml` declares `[output] required_suffix_lines` so the guard fires on missing trailer lines. The eval test does NOT need to re-test the guard (already covered by `required_suffix_line_caught.rs` for the second-review skill); it tests the shape of valid outputs that satisfy the guard.

### External References

None — this is internal eval-harness work with strong local patterns.

## Key Technical Decisions

### TD1: Synthetic skill, not bundled-skill load

**Decision:** Use `SkillRegistry::from_test_entries` to inject a synthetic skill named `mika-arch-groom-milestone` with the same `[output] required_suffix_lines` declaration. Do NOT load the production bundled skill via `BUNDLED_SKILL_MANIFESTS`.

**Rationale:** This matches the canonical `tests/eval/grounding_regressions/required_suffix_line_caught.rs` pattern. The test validates **engine handling of milestone-shaped I/O**, not the production prompt's wording. Coupling the test to the production prompt's exact text would make every prompt edit a test-modifying change without adding regression value. The bundled skill is separately covered by `tests/bundled_skills_load.rs` (which gates oversized-prompt regressions).

### TD2: Suffix-line accept-set matches the production skill

**Decision:** The synthetic skill declares `required_suffix_lines = ["Disposition: READY", "Disposition: ITERATE", "Disposition: ESCALATE"]` — the exact set declared in the production `skills/bundled/mika-arch-groom-milestone/skill.toml`.

**Rationale:** The four scenarios produce all three disposition values; the accept-set must include all three so each scenario's mock response is considered valid by the post-condition guard.

### TD3: One test file, four `#[tokio::test]` blocks

**Decision:** Single file, four `#[tokio::test]` async fns (one per scenario). No shared parameterized helper beyond a small `make_milestone_skill()` constructor.

**Rationale:** Mirrors the existing eval test layout. Parameterized helpers add indirection at the cost of test readability — at four cases, four blocks read cleanly.

### TD4: Module location — `tests/eval/skills/` (new sub-directory)

**Decision:** Create new `crates/mika-agent/tests/eval/skills/mod.rs` and place the new test as `crates/mika-agent/tests/eval/skills/mika_arch_groom_milestone.rs`. Wire from `crates/mika-agent/tests/eval.rs` via `pub mod skills;`.

**Rationale:** This is the path the parent plan declared (Unit 1 `**Files:**`). Future per-skill eval tests (`mika-arch-groom-ticket`, `mika-arch-second-review`, `dev-groom`) can drop into the same directory without the existing flat-layout `test_*.rs` naming.

## Open Questions

### Resolved During Planning

- **Use bundled skill or synthetic?** Resolved per TD1 — synthetic.
- **Where does the test file live?** Resolved per TD4 — `tests/eval/skills/`.
- **Should we validate the agent loop's actual output text matches the mocked response?** Yes — `assert_has_output(&trace)` plus `trace.output.text.unwrap().ends_with("Disposition: READY")` for the literal-final-line invariant. The mock returns the canned response; the harness records what reached the user.

### Deferred to Implementation

- **Exact synthetic brief text per scenario:** Implementer chooses representative-but-minimal phrasing while writing the test (must include sub-issue numbers, plan paths, and dependency annotations for the happy path; must omit required sections for the error path).
- **Whether to add a README in `tests/eval/skills/`:** Defer until a second per-skill test arrives in the directory. One file does not need a README.

## Implementation Units

- [ ] **Unit 1: Add `tests/eval/skills/mika_arch_groom_milestone.rs` with four `#[tokio::test]` cases**

**Goal:** Ship the eval test the parent plan declared, satisfying mika-qa's R-Unit-1 AC.

**Requirements:** R1, R2, R3.

**Dependencies:** None (skill scaffold already on the branch).

**Files:**
- Create: `crates/mika-agent/tests/eval/skills/mod.rs` (single line: `pub mod mika_arch_groom_milestone;`).
- Create: `crates/mika-agent/tests/eval/skills/mika_arch_groom_milestone.rs` (4 tokio tests + a `make_milestone_skill()` helper modeled on `make_suffix_skill()`).
- Modify: `crates/mika-agent/tests/eval.rs` (add `pub mod skills;` inside the `mod eval { ... }` block, alphabetically sorted with the other `pub mod` lines near the top).

**Approach:**
- Mirror `tests/eval/grounding_regressions/required_suffix_line_caught.rs:36-72` for the helper. Skill name: `mika-arch-groom-milestone`. Keywords: include a milestone-trigger token (e.g., `"milestone-groom"`) so the harness's keyword matcher selects it. `[output] required_suffix_lines` = `["Disposition: READY", "Disposition: ITERATE", "Disposition: ESCALATE"]`.
- Each test: build a `SkillRegistry` via `from_test_entries`, build harness with one canned `text_response(...)` representing the LLM's milestone-shaped reply, run with a synthetic milestone brief as the user message, assert on `trace.output.text`.
- For the happy path (3 sub-issues), the mock response includes the per-sub-issue disposition summary, sequencing block, cross-cutting concerns, then a blank line, then `Scope: milestone`, then `Disposition: READY` as the literal final line.
- For the single-sub-issue case, the brief lists one sub-issue; the mock response still emits the milestone shape (with a one-entry per-sub-issue summary) — assert `Scope: milestone` is present.
- For the conflicting-AC case, the brief lists two sub-issues whose ACs are mutually incompatible; the mock response surfaces the conflict in the cross-cutting section and ends with `Disposition: ITERATE`.
- For the missing-sections error path, the brief omits per-sub-issue plan paths; the mock response cites the schema gap and ends with `Disposition: ESCALATE`.

**Patterns to follow:**
- `crates/mika-agent/tests/eval/grounding_regressions/required_suffix_line_caught.rs` (synthetic skill + mock + trace assertions).
- `crates/mika-agent/tests/eval/test_per_skill_provider_override.rs` (minimal harness builder usage).

**Test scenarios:**
- **Happy path (3 sub-issues, READY):** Mock returns milestone-shaped text ending with `Scope: milestone\nDisposition: READY`. Assert `assert_has_output(&trace)`, output contains `"Scope: milestone"`, output's last non-empty line equals `"Disposition: READY"`, `llm_call_count == 1` (no guard re-prompt).
- **Edge case (single-sub-issue, READY):** Mock returns milestone-shaped text with one entry in the per-sub-issue summary, ending in `Disposition: READY`. Assert output contains `"Scope: milestone"` AND `"#NNN:"` (sub-issue line). Confirms n=1 still emits milestone shape, not per-ticket shape.
- **Edge case (conflicting AC, ITERATE):** Mock returns text that names the cross-cutting conflict and ends with `Disposition: ITERATE`. Assert last non-empty line is `"Disposition: ITERATE"` and output contains a phrase like `"cross-cutting"` or `"conflicting"` (loose match — the test asserts presence of the explanatory section, not exact wording).
- **Error path (missing sections, ESCALATE):** Mock returns text citing the missing section and ends with `Disposition: ESCALATE`. Assert last non-empty line is `"Disposition: ESCALATE"` and output references a missing-section keyword (e.g., `"missing"`, `"Sub-issues"`, `"plan path"`).

**Verification:**
- `cargo test -p mika-agent --test eval -- --nocapture skills::mika_arch_groom_milestone` runs the four new tests and they all pass.
- `cargo test -p mika-agent --test eval` runs the full eval suite without new failures or panics.
- `cargo build -p mika-agent --tests` compiles with no warnings on the new file.

## System-Wide Impact

- **Interaction graph:** None outside `tests/eval/`. The new module declaration in `tests/eval.rs` is additive.
- **Error propagation:** N/A — test-only.
- **State lifecycle risks:** None — `EvalHarness` is fully self-contained per test.
- **API surface parity:** None.
- **Integration coverage:** The new test exercises `match_skills` → required-suffix-line guard path with a milestone-shape skill. The `required_suffix_line_caught.rs` test already covers guard-fires; this test covers guard-passes for the milestone variant.
- **Unchanged invariants:** Production skill prompt, manifest, tools.json, allowlist, LLM override — all untouched. mika-qa's other ACs (R1-R7) remain satisfied. Existing eval tests under `tests/eval/` are unmodified.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Synthetic skill drifts from production output contract | TD2 pins the accept-set to the production `skill.toml` value. If the production contract changes, both files update together — same PR-time review surface. |
| Future per-skill eval tests reuse this helper, leading to duplication | The `make_milestone_skill()` helper is local to one file by design (TD3). Promotion to a shared helper happens when the second per-skill eval ships, not preemptively. |
| Test passes but production skill ships invalid output | Out-of-scope for this plan — that risk is addressed by the parent plan's Unit 6 smoke test (`scripts/test-mika-groom-milestone.sh`) and the integration-tier real-provider eval (parent plan §C2.4). This Unit 1 test asserts engine *handling*, not LLM behavior. |

## Documentation / Operational Notes

- No CLAUDE.md updates required — `crates/mika-agent/CLAUDE.md` § Evaluation already documents the eval-harness pattern; the new test follows it.
- `/mika-doc-audit` will be run as part of the pipeline and will surface any doc gaps.

## Sources & References

- **Parent plan:** [`docs/plans/2026-04-29-001-feat-mika-arch-milestone-grooming-plan.md`](2026-04-29-001-feat-mika-arch-milestone-grooming-plan.md) (the architect-validated contract; this delta plan completes its Unit 1).
- **mika-qa verdict:** Review id 4196247084 on PR mika#888, commit `0a56d844`, posted 2026-04-29T10:54:47Z. `VERDICT: block[ac]` citing two unsatisfied ACs (Unit 1 + Unit 3); this plan addresses Unit 1 only.
- **Companion ticket:** `mika-platform#65` (Unit 3 canonical operator command — separate plan, separate PR, out of scope here).
- **Pattern reference:** `crates/mika-agent/tests/eval/grounding_regressions/required_suffix_line_caught.rs`.
- **Skill under test:** `skills/bundled/mika-arch-groom-milestone/system_prompt.md` (already on the branch).
