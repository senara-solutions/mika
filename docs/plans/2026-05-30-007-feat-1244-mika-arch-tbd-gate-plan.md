# Plan: feat(mika-arch) — GROOMED plans must contain zero unresolved TBDs

**Ticket:** mika#1244
**Type:** enhancement
**Branch:** `feat/1244/mika-arch-groomed-plans-must-contain`

## Problem

The mika-arch grooming pipeline can return `Disposition: READY` (first pass) or `Verdict: GROOMED` (second pass) on plans that still contain unresolved design decisions — TBDs, placeholders, "pick one" phrasing, unspecified version pins, etc. When such plans reach the autonomous pilot, `/ce:work` Phase 1 asks clarifying questions, claude-pilot treats the SDK-success-without-PR as `pipeline_incomplete`, and the dispatch burns cost with zero edits (observed: mika#1243, 3 attempts, $1.50).

The fix is upstream at grooming: the architect skill prompts must gate disposition/verdict on zero unresolved decisions.

## Scope

Four deliverables across two skill prompts, one calibration scenario, one review-guide section, and one grounding regression test.

## Units of Work

### Unit 1: Prompt rule — `mika-arch-groom-ticket/system_prompt.md` (AC1)

**File:** `skills/bundled/mika-arch-groom-ticket/system_prompt.md`

Add a new `### No-TBD Gate` section between `### Process` (step 4) and `### Output`. This section defines a mandatory pre-disposition check:

```markdown
### No-TBD Gate (Disposition Pre-Check)

Before emitting any disposition, scan the plan for unresolved decisions. A plan with ANY of the following CANNOT receive `Disposition: READY`:

- Explicit `TBD` or `tbd` tokens
- "Pick one" / "choose between" / "either X or Y" phrasing that defers a load-bearing design choice
- Unspecified version pins: `<tag>`, `<version>`, `latest` without a pinned value
- Placeholder paths: `<path>`, `path/to/...`, `<file>`, `<dir>`
- "Operator decides" / "decision deferred" / "to be determined" / "needs discussion"
- Any phrasing that defers a load-bearing design choice to the implementer

If ANY unresolved decision is found:
- Return `Disposition: ITERATE` with each unresolved decision enumerated as an F-list finding
- Each finding's "(b) Change required" must state what concrete decision the plan author needs to make
- Citation: this section (`review-guide.md § GROOMED contract`)

Exception: informational TBDs in "Out of scope" or "Future work" sections that explicitly mark items as NOT part of this plan are not blockers.
```

**Placement rationale:** The gate runs after step 4 (review against principles) and before output — it's a structural pre-check on the plan text, not a principles-level review.

### Unit 2: Prompt rule — `mika-arch-second-review/system_prompt.md` (AC2)

**File:** `skills/bundled/mika-arch-second-review/system_prompt.md`

Add a matching `### No-TBD Gate (Verdict Pre-Check)` section between `### Process` (step 5) and `### Output`. Same rule set as Unit 1 but adapted for second-pass semantics:

```markdown
### No-TBD Gate (Verdict Pre-Check)

Before emitting any verdict, scan the revised plan for unresolved decisions. A plan with ANY of the following CANNOT receive `Verdict: GROOMED`:

[Same enumeration as Unit 1]

If ANY unresolved decision is found:
- Return `Verdict: ESCALATE` (not ITERATE — second pass has no ITERATE)
- Enumerate each unresolved decision as an F-list finding
- Citation: `review-guide.md § GROOMED contract`

Exception: informational TBDs in "Out of scope" or "Future work" sections that explicitly mark items as NOT part of this plan are not blockers.
```

**Why ESCALATE, not ITERATE:** The second-pass skill cannot return ITERATE (hard architectural constraint, line 9 of the current prompt). If TBDs survive both the first-pass ITERATE and the plan revision, ESCALATE is the correct disposition — human judgment is needed to resolve what the plan author and first-pass reviewer couldn't.

### Unit 3: Calibration scenario — `tbd_gate_rejects_ready` (AC3)

**New fixture:** `crates/mika-agent/tests/eval/calibration_fixtures/mika-arch/tbd_gate_rejects_ready.md`

Fixture content: a plan for a fictional feature (e.g., "Add webhook retry queue") that is structurally sound but contains three deliberate TBDs:
1. An explicit `TBD` token: "Retry backoff strategy: TBD"
2. A placeholder path: "Config file at `<path>/retry.toml`"
3. A "pick one" choice: "Storage: either SQLite table or in-memory queue — pick one based on durability needs"

**New scenario in `calibration/roles/mika_arch.rs`:**

Add a 6th scenario entry to `SCENARIOS`:

```rust
RoleScenario {
    id: "tbd_gate_rejects_ready",
    description: "Plan with unresolved TBDs must NOT receive READY/GROOMED disposition",
    tags: &["contract", "tbd-gate"],
    flaky: false,
    weight: 2.0,
    expected_failure_classes_absent: &["ContractViolation"],
},
```

**Scenario function `run_tbd_gate_rejects_ready`:**

- System prompt: mika-arch first-pass reviewer prompt (include the No-TBD Gate section)
- Input: the fixture with deliberate TBDs
- Assertions:
  1. Response does NOT contain `Verdict: READY` or `Disposition: READY` (the plan has TBDs)
  2. Response contains `Disposition: ITERATE` or `Verdict: ESCALATE` (either is acceptable — the key invariant is NOT READY/GROOMED)
  3. Response contains at least one `F` finding prefix (`F1:`)
- Failure class: `ContractViolation` if READY/GROOMED is returned despite TBDs

**Manifest entry in `manifest.yaml`:**

```yaml
  - id: tbd_gate_rejects_ready
    fixture: tbd_gate_rejects_ready.md
    tags: [grooming, tbd-gate, contract]
    flaky: false
    weight: 2.0
    description: >
      Plan with deliberate TBDs (explicit TBD token, placeholder path, pick-one
      choice). Model must NOT produce READY/GROOMED — must ITERATE with findings
      naming each unresolved decision.
    expected_failure_classes_absent:
      - false_approval
      - missing_findings
```

### Unit 4: Review guide — GROOMED contract section (AC4)

**File:** `docs/architecture/review-guide.md`

Add a new `## 8. GROOMED Contract` section after `## 7. Self-review boundary`:

```markdown
## 8. GROOMED Contract

### What it means here

The `GROOMED` verdict (second-pass) and the `READY` disposition (first-pass) are a guarantee to downstream consumers — specifically, the autonomous pilot running `/ce:work` — that the plan is implementable as-written with zero operator intervention. The implementer can proceed without asking questions.

### What this guarantees

A GROOMED/READY plan has:
- **Zero unresolved decisions.** No TBDs, no "pick one", no placeholder paths, no unspecified version pins, no deferred design choices.
- **All acceptance criteria are addressable from the plan alone.** The implementer does not need to consult the operator to satisfy any AC.
- **File paths, module locations, and integration points are concrete.** "Add a module at `crates/mika-agent/src/kg/budget.rs`" is concrete; "add a module somewhere in the KG layer" is not.

### What to flag

- **A plan returning READY/GROOMED with any unresolved decision** — explicit `TBD`/`tbd` tokens, "pick one"/"choose between" phrasing, unspecified version pins (`<tag>`, `<version>`), placeholder paths (`<path>`, `path/to/...`), "operator decides"/"decision deferred", or any phrasing that defers a load-bearing design choice to the implementer.
- **Acceptance criteria that require operator input.** "AC3: Deploy to the environment Vincent specifies" defers a load-bearing decision. "AC3: Deploy to staging via `make deploy`" does not.

### What not to flag

- **Informational TBDs in "Out of scope" or "Future work" sections.** These explicitly mark items as NOT part of this plan — they are not decisions the implementer needs to make.
- **Design choices that are explicitly made in the plan** even if the reviewer would have chosen differently. The GROOMED contract is about completeness, not optimality.

### Evidence base

mika#1243 (2026-05-22): pilot asked 3 clarifying questions about unresolved items in a GROOMED plan. 3 dispatch attempts, $1.50 burned, zero edits. Root cause: mika-arch returned GROOMED on a plan with unspecified decisions (ollama checksum derivation, emerge-webrsync timing, Phase 4 verification scope).
```

**Maintenance note:** Update the `## Maintenance` section reference count ("Eight sections, not seven") — or just leave it implicit since it says "Updates land via normal PR."

### Unit 5: Grounding regression test (AC5)

**New fixture:** `crates/mika-agent/tests/eval/grounding_regressions/fixtures/tbd_gate_pre_fix.json`

This fixture captures the pre-fix failure: a mock LLM response that returns `Disposition: READY` on a plan containing TBDs. The fixture exercises the calibration scenario's assertion logic as a frozen regression.

**New test file:** `crates/mika-agent/tests/eval/grounding_regressions/tbd_gate_false_approval.rs`

Test structure (following existing patterns in `dev_groom_fabricated_verdict_caught.rs`):

1. **Setup:** Create `EvalHarness` with `MockLlmProvider` containing a sequence:
   - Response 1: Assistant text that includes a review of the plan and ends with `Disposition: READY` — despite the plan containing TBDs
2. **System prompt:** Include the updated `mika-arch-groom-ticket` system prompt with the No-TBD Gate
3. **User message:** The fixture plan with deliberate TBDs (same content as Unit 3 fixture)
4. **Assertions:**
   - The mock response contains `Disposition: READY` (confirming the pre-fix shape)
   - The response text does NOT contain findings naming TBD items (confirming the pre-fix gap)
   - Use `assert_response_contains` to verify the TBD patterns are in the input plan

**Registration:** Add `pub mod tbd_gate_false_approval;` to `grounding_regressions/mod.rs` and register in the eval runner.

However, this test as described is a frozen-fixture regression test — it proves the assertion catches the failure shape, not that the live model behaves correctly. The live-model behavior is tested by Unit 3 (calibration scenario). This is consistent with how grounding regressions work: they freeze a pre-fix response and assert the detection logic catches it.

**Alternative (simpler, matches AC5 more precisely):** Since AC5 asks for "a regression test that takes a known-bad plan through the architect skill and asserts ITERATE disposition with findings naming each TBD," and the calibration scenario (Unit 3) already does this with a real LLM, Unit 5 provides the mock-based regression backstop. Together they cover both the structural assertion and the live-model behavior.

## File Change Summary

| File | Action | Size |
|------|--------|------|
| `skills/bundled/mika-arch-groom-ticket/system_prompt.md` | Edit | +20 lines |
| `skills/bundled/mika-arch-second-review/system_prompt.md` | Edit | +20 lines |
| `crates/mika-agent/src/calibration/roles/mika_arch.rs` | Edit | +60 lines |
| `crates/mika-agent/tests/eval/calibration_fixtures/mika-arch/tbd_gate_rejects_ready.md` | New | ~30 lines |
| `crates/mika-agent/tests/eval/calibration_fixtures/mika-arch/manifest.yaml` | Edit | +12 lines |
| `docs/architecture/review-guide.md` | Edit | +30 lines |
| `crates/mika-agent/tests/eval/grounding_regressions/tbd_gate_false_approval.rs` | New | ~80 lines |
| `crates/mika-agent/tests/eval/grounding_regressions/fixtures/tbd_gate_pre_fix.json` | New | ~30 lines |
| `crates/mika-agent/tests/eval/grounding_regressions/mod.rs` | Edit | +1 line |

## Sequencing

Units 1 and 2 are independent (prompt changes to different files). Unit 3 depends on Unit 1 (the calibration scenario includes the updated prompt). Unit 4 is independent (review-guide section). Unit 5 depends on Units 1 and 3 (regression test references the updated prompt and fixture pattern).

Recommended order: Units 1+2 → Unit 4 → Unit 3 → Unit 5. All can be done in a single PR since they're tightly coupled.

## Verification

1. `cargo build` — confirms calibration scenario compiles
2. `cargo test -p mika-agent` — confirms existing tests pass with prompt changes
3. `make calibrate-mika-arch MODEL=anthropic/claude-sonnet-4-6` — runs the new `tbd_gate_rejects_ready` scenario against a real model
4. Manual review: read the updated prompts in both skill files and verify the No-TBD Gate section is present and consistent

## Self-review boundary (§7)

This plan modifies `mika-arch-groom-ticket` and `mika-arch-second-review` skill prompts — mika-arch's own operational surface. Per `docs/architecture/review-guide.md` § 7, the self-review boundary applies. The change is purely additive (new gating rule, no deprecation or behavioral reduction), so first-pass review by mika-arch is appropriate. If the first pass introduces reviewer-driven reshaping (provenance test), second pass must route externally.

## Risks

- **False positives:** The TBD patterns could match legitimate uses (e.g., code examples that contain `<path>` as a placeholder in documentation). The "Out of scope"/"Future work" exception mitigates the most common case. The pattern list is intentionally specific (8 concrete shapes) rather than open-ended.
- **Prompt length:** Adding ~20 lines to each skill prompt is within budget. The mika-arch prompts are relatively short (100 lines each).
- **LLM compliance:** The gate is prompt-based, not engine-enforced. The LLM might still return READY despite the instruction. The calibration scenario (Unit 3) will catch this during model swaps. Engine-level enforcement (scanning plan text for TBD patterns before accepting READY) is a potential future hardening but is out of scope — prompt-level is the right first step.
