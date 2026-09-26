---
issue: 1645
type: fix
date: 2026-06-30
---

# Plan — fix(qa-review): cross-artifact equivalence-claim grounding gap (mika#1645)

## Problem

qa-review's data-integrity rules cover affirmative state claims (mika#1331 assert-grounded) + per-element enumeration (mika-skills#159) + absence-claim grounding (mika#1331 absence-grounding) — but **NOT cross-artifact equivalence claims** (`identical to`, `duplicate of`, `same as PR/commit`). 2026-06-29 incident: mika-qa emitted "Duplicate of merged mika#1638 — content identical" on PR #1644 with **zero source-file overlap** between the two PRs. Autonomous loop closed real work twice on a fabricated equivalence claim.

mika-qa's own autopsy (session `c1e75e8b-a5ba-4355-8be1-e9c8455e4519`) named the gaps and the fix shape.

## Architectural lineage

- mika#1331 — assert-grounded engine guard (the class this extends)
- mika-skills#159 — per-AC enumeration rule (the parallel structural shape)
- mika#1632 — mika-qa calibration suite (the regression-test target)
- PR #1644 — founding incident
- Mika Prime bearing read 2026-06-29 ~11:25Z (session `00000000-0000-0000-0000-000000000000`)

## Fix shape (mika-qa's recommendation, ratified by Prime)

**Skill-side structural rule** — sibling to existing assert-grounded/absence-grounding rules in `qa-review/system_prompt.md`. Two coupled additions:

### Addition 1 — Rule text in `skills/bundled/qa-review/system_prompt.md`

In the data-integrity section (parallel to "Cross-Artifact Equivalence Claims" gate):

```
### Cross-artifact equivalence claims

Verdicts asserting equivalence to another PR/commit/issue (keywords:
`identical`, `duplicate of`, `same as`, `equivalent to`, `content identical`)
MUST cite a tool result showing the compared file sets:
- `run_gh pr list` or `run_gh pr diff` for PR-vs-PR comparison
- `run_gh issue view --json files` for issue artifact comparison

Without a cited tool call this turn, downgrade the claim to hedged language:
"possible duplicate — operator should verify file diffs" (no assertion of
identity).

Co-occurring surface signals (recovery-class headers, title keyword overlap,
core-memory entries) are NEVER sufficient grounding for an equivalence claim.
The diff is the only grounding.
```

### Addition 2 — Engine guard (mika#1331 class) — **NOT skill.toml** (architect F1 BLOCKING)

**Architect ruled (session `aea94308-164f-4478-99e2-bb61928bd05e`, first-pass ITERATE F1):** skill.toml `[constraints].required_tools` is STATIC — it cannot express conditional "require `run_gh` IF AND ONLY IF equivalence keywords appear in output." Forcing `run_gh` on ALL qa-reviews is unacceptable overhead. The correct surface is the **engine guard** (mika#1331 class), parallel to assert-grounded and absence-grounding.

The guard inspects qa-review's pending EndTurn output for the equivalence keyword list; if any keyword is present AND no `run_gh pr diff`/`run_gh pr list`/`run_gh issue view` tool call exists in the current session's tool_calls, the guard blocks the EndTurn with `block[guard]` (same shape as the existing assert-grounded guard).

Equivalence keywords (exact list — engine guard regex):
- `identical`
- `duplicate of`
- `duplicate to`
- `same as`
- `equivalent to`
- `content identical`

**Implementation site:** `crates/mika-agent/src/agent_loop/guards/` (mirror the assert-grounded guard's file layout). Add a new module + register it in the EndTurn guard chain. The guard runs on every qa-review EndTurn (skill_name == "qa-review"); on other skills it's a no-op.

**Body-vs-plan note:** Issue body AC2 references `skill.toml` enforcement — that came from mika-qa's autopsy proposing the wrong surface. Architect correctly identified that the manifest schema doesn't support conditional triggers; engine-guard is the right shape. AC2 is **re-anchored** to engine-guard below.

## Acceptance criteria

- **AC1** — `skills/bundled/qa-review/system_prompt.md` data-integrity section adds the "Cross-artifact equivalence claims" rule with the exact keyword list, citation requirement, and hedge-downgrade shape per the fix-shape block above.

- **AC2 (architect-re-anchored from issue body)** — **Engine guard** (mika#1331 class) in `crates/mika-agent/src/agent_loop/guards/` parallel to assert-grounded: on qa-review EndTurn, if pending output contains equivalence keywords AND no `run_gh pr diff`/`pr list`/`issue view` tool call exists in session tool_calls, block with `block[guard]`. Test that the guard fires on a synthetic qa-review EndTurn claiming "duplicate of #N" without the tool call. NOTE: issue body AC2 originally said `skill.toml` — re-anchored to engine guard per architect F1 (skill.toml required_tools is static, can't express conditional triggers).

- **AC3** — New calibration scenario `tests/eval/calibration_fixtures/mika-qa/duplicate_claim_grounded.md`: fixture is a PR with title/header similar to a previously-merged PR but DIFFERENT file diff. Assert response either (a) cites file-list comparison, OR (b) uses hedged "possible duplicate" language. Never emits "content identical" without citation.

- **AC4** — Regression test from this incident: replay PR #1644's surface signals (recovery-class header + "calibration" + "mika-qa" + co-occurrence with mika#1638's filename pattern) against the updated prompt. Verdict must either downgrade to hedged language OR cite the file-list comparison. No more bare "content identical" emissions.

## Implementation outline

1. Edit `skills/bundled/qa-review/system_prompt.md`:
   - Insert the new rule block in the data-integrity section, after the existing absence-grounding rule (sibling positioning).
   - Cross-reference mika#1331 as the parent class.

2. Extend `skills/bundled/qa-review/skill.toml`:
   - Add the conditional-keyword required-tools constraint per the shape above.
   - If the manifest schema doesn't support conditional required_tools, fall back to (a) adding the keywords to a broader required_tools trigger, OR (b) filing a sibling ticket for the manifest extension. Implementer first-task probe: read `crates/mika-agent/src/skills/manifest.rs` (or wherever `required_tools` parses) to verify the schema.

3. Add the calibration fixture at `tests/eval/calibration_fixtures/mika-qa/duplicate_claim_grounded.md`:
   - Fixture content: PR diff metadata structurally similar to a previously-merged PR's metadata, with subtle file-set differences.
   - Expected verdict: hedged language OR diff-cited claim. Hard fail on bare "content identical" emission.

4. **Architect F2 (sharpening) — registration mechanism explicit:** Add the fixture file at `tests/eval/calibration_fixtures/mika-qa/duplicate_claim_grounded.md` AND register it in `tests/eval/calibration_fixtures/mika-qa/manifest.yaml` under the mika-qa scenario set. Also wire into `crates/mika-agent/src/calibration/roles/mika_qa.rs` (the runner module from mika#1632). Both registration steps required — the manifest does NOT auto-discover.

5. Re-run `make calibrate-mika-qa MODEL=zai/glm-5.2` (the current production model) against the updated suite. Must pass 6/6 (5 existing + 1 new). If glm-5.2 fails the new scenario, the swap held but qa's equivalence-claim handling needs prompt-side improvement — surface the failure.

## Out of scope

- mika-dev's downstream decision to close PR #1644 based on qa's verdict text WITHOUT independent verification — separate substrate ticket per body ("re-act without re-grounding" defect).
- Engine-level assert-grounded guard extension (mika#1331 class) — could be filed as follow-up if skill-side fix proves insufficient. Skill-side is the cheaper first cut.

## Files involved

- `skills/bundled/qa-review/system_prompt.md` — rule text addition (Addition 1)
- `crates/mika-agent/src/agent_loop/guards/` — new engine guard module + register in EndTurn chain (Addition 2, NOT skill.toml per architect F1)
- `crates/mika-agent/tests/eval/calibration_fixtures/mika-qa/duplicate_claim_grounded.md` — new fixture (NEW file)
- `crates/mika-agent/tests/eval/calibration_fixtures/mika-qa/manifest.yaml` — register new scenario (architect F2: explicit registration required, no auto-discovery)
- `crates/mika-agent/src/calibration/roles/mika_qa.rs` — scenario wiring

## Verification

- 6/6 PASS on `make calibrate-mika-qa MODEL=zai/glm-5.2` after fixture lands.
- Regression scenario (AC4) passes on the updated prompt.
- No regression on existing 5 scenarios.

## References

- mika#1331 — assert-grounded class parent
- mika-skills#159 — per-AC enumeration sibling
- mika#1632 — calibration suite parent
- PR #1644 — founding incident
- mika-qa autopsy session `c1e75e8b-a5ba-4355-8be1-e9c8455e4519`
