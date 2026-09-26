# Plan: chore(docs) cleanup grounding_regressions README — drop duplicate plan block, add #901 scenarios to capability matrix

**Ticket:** mika issue#1179
**Type:** chore (docs-only)
**Risk:** None — no code changes

## Context

Two cosmetic doc gaps surfaced during the mika#894 code review:

1. **Plan doc duplicate block** — `docs/plans/2026-05-13-002-fix-894-elided-copula-asserted-unavailability-plan.md` lines 168–182: the "Canonical fixture locations" heading + bullet list appears verbatim twice (architect-signed content, duplicated in the original plan).

2. **README capability matrix missing #901 scenarios** — `crates/mika-agent/tests/eval/grounding_regressions/README.md` lists 30 scenarios but is missing the 8 `required_finding_list` scenarios from mika#901 that exist in `mod.rs` and have test functions in `required_finding_list.rs`. The README header says "Thirty scenarios" and all three matrices (capability, three-tier execution, frozen fixtures) are missing these entries.

Additionally, during investigation a third gap was found:

3. **README missing #1024 scenarios** — 2 `summary_conversational_recall` scenarios from mika#1024 exist in `mod.rs` and have a frozen fixture (`summary_conversational_recall_pre_fix.json`) but are absent from the README.

## Changes

### 1. Strip duplicate "Canonical fixture locations" block in mika#894 plan doc

**File:** `docs/plans/2026-05-13-002-fix-894-elided-copula-asserted-unavailability-plan.md`

Delete lines 176–182 (the second copy of the "Canonical fixture locations" block). The first copy at lines 168–175 remains.

Before (showing the duplicate):
```
   **Canonical fixture locations** (cite these in Rule 4 ...):    ← line 168 (KEEP)
   - `crates/mika-agent/tests/eval/grounding_regressions/...`
   - ...
   A new contributor adding a recurrence ...                      ← line 175

   **Canonical fixture locations** (cite these in Rule 4 ...):    ← line 176 (DELETE)
   - `crates/mika-agent/tests/eval/grounding_regressions/...`
   - ...
   A new contributor adding a recurrence ...                      ← line 182
```

After: only the first block (lines 168–175) remains.

### 2. Add #901 required_finding_list scenarios to README capability matrix

**File:** `crates/mika-agent/tests/eval/grounding_regressions/README.md`

#### 2a. Update the header line

The README currently says "Thirty scenarios." After adding 8 #901 scenarios + 2 #1024 scenarios = 40 total. Update the header text and the citation list.

Current header (line 1):
```
# Grounding + Fabrication Regression Scenarios (#741, #793, #797, #862, #863, #864, #894, #1059, #1133, #1221)
```

Updated header:
```
# Grounding + Fabrication Regression Scenarios (#741, #793, #797, #862, #863, #864, #894, #901, #1024, #1059, #1133, #1221)
```

Current scenario description (line 3):
```
Thirty scenarios testing concrete fabrication classes. Scenarios 1–5 ...
```

Updated to: "Forty scenarios..." with additional citation for the #901 and #1024 scenario groups.

#### 2b. Add grounding tags for #901 and #1024 to the Tag Vocabulary table

Add these tags:

| Tag | Trigger condition | Type |
|-----|-------------------|------|
| `grounding:finding-list-emission-required` | Guard correctly required F-list on terminal disposition (ITERATE/ESCALATE) | Success |
| `grounding:thin-emission-evasion` | Agent emitted terminal disposition without required finding-list entries | **Failure** |
| `grounding:conversational-recall-suppressed` | Reformed summary did not trigger conversational-recall patterns | Success |
| `grounding:conversational-recall-triggered` | Conversational summary caused the LLM to produce first-person recall | **Failure** |

#### 2c. Add rows to Capability × Status Matrix

Insert 8 rows for required_finding_list (numbered 31–38) and 2 rows for summary_conversational_recall (numbered 39–40) after the current scenario 30:

| Scenario | Forbidden-word | Required-tool | Contains-in-order | Contains | Tags |
|----------|:-:|:-:|:-:|:-:|------|
| 31. required_finding_list_caught_on_iterate | | | | V | `finding-list-emission-required` |
| 32. required_finding_list_no_op_on_ready | | | | | `finding-list-emission-required` |
| 33. required_finding_list_no_op_when_unset | | | | | `finding-list-emission-required` |
| 34. required_finding_list_position_inclusive | | | | V | `finding-list-emission-required` |
| 35. required_finding_list_position_exclusive | | | | | `thin-emission-evasion` (failure) |
| 36. required_finding_list_position_at_message_start | | | | V | `finding-list-emission-required` |
| 37. required_finding_list_caught_on_verdict_escalate | | | | V | `finding-list-emission-required` |
| 38. required_finding_list_no_op_on_verdict_groomed | | | | | `finding-list-emission-required` |
| 39. summary_conversational_recall (reformed) | V | | | | `conversational-recall-suppressed` |
| 40. summary_conversational_recall (regression) | | | | V | `conversational-recall-triggered` (failure) |

Exact assertion shapes will be verified by reading each test function at implementation time.

#### 2d. Add rows to Three-Tier Execution matrix

| Scenario | Unit (mock) | Integration (real) | Calibration |
|----------|:-:|:-:|:-:|
| 31-38. required_finding_list | V | - | - |
| 39-40. summary_conversational_recall | V | - | - |

(All are MockLlmProvider-based unit tests with no real-provider or calibration variants.)

#### 2e. Add rows to Frozen Regression Fixtures table (where fixtures exist)

Check: no `required_finding_list_*_pre_fix.json` fixture exists in the fixtures directory. The #901 scenarios do not have frozen fixtures — their regression-reproduction tests operate on constructed responses, not pre-fix captures.

`summary_conversational_recall_pre_fix.json` does exist:

| Scenario | Fixture | Incident |
|----------|---------|----------|
| 40 | `summary_conversational_recall_pre_fix.json` | mika#1024 (Axis 2 — conversational summary shape) |

## What this plan does NOT do

- Does not modify any Rust code — test files, assertions, or the agent loop.
- Does not modify `crates/mika-agent/CLAUDE.md` — the CLAUDE.md already references these scenarios correctly (it says "35 fabrication-detection scenarios" which is also outdated; updating CLAUDE.md scenario counts is a separate concern and not requested in the ticket).
- Does not add new test scenarios — only documents existing ones.
- Does not renumber existing scenarios 22–30 to make room for #901 — the new scenarios are appended as 31–40 to avoid breaking any existing external references to scenario numbers.

## Acceptance criteria (mapped to ticket body)

- [ ] Duplicate "Canonical fixture locations" block stripped from `docs/plans/2026-05-13-002-fix-894-elided-copula-asserted-unavailability-plan.md`
- [ ] README capability matrix extended with rows 31–38 for the required_finding_list scenarios (8 tests from #901) and rows 39–40 for summary_conversational_recall (#1024)
- [ ] README header count updated from "Thirty" to "Forty"
- [ ] Three-tier execution matrix extended with corresponding rows
- [ ] Frozen-regression-fixtures table extended with `summary_conversational_recall_pre_fix.json` entry
