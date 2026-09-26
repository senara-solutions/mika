# Plan — feat(mika-arch): GROOMED gate must reject plans with TBDs (mika#1244)

## Phase 0 — Pin

**A. Current disposition contract** (`mika/skills/bundled/mika-arch-groom-ticket/system_prompt.md`):
```
Disposition semantics:
- READY — The plan is sound. Proceed to implementation.
- ITERATE — The plan has addressable concerns. Revise and re-submit.
- ESCALATE — The plan has concerns that require human judgment (Vincent).
```
No current rule about TBDs or unresolved decisions. The architect determines READY/ITERATE/ESCALATE on principle review (Single Responsibility / DRY / YAGNI / KISS / Orthogonality) — but a plan with TBDs can still pass these and reach READY.

**B. Sister skill** (`mika/skills/bundled/mika-arch-second-review/system_prompt.md`):
Second-pass verdict is `GROOMED` | `ESCALATE`. Same issue — semantic-TBD detection is not in the current gating rules.

**C. F-list emission contract**:
On ITERATE/ESCALATE, F-list with `F1: (BLOCKING|sharpening) ...` is mandatory (engine post-condition guard `required_finding_list_prefixes`).

**D. Calibration substrate (actual paths)**:
- `mika/crates/mika-agent/src/calibration/` — Rust code (roles dir, scenarios may need creating)
- `mika/crates/mika-agent/tests/eval/calibration_fixtures/mika-arch/` — fixtures
- `mika/Makefile:73` — `calibrate-mika-arch` target invokes `cargo run --bin calibrate --release -- --role mika-arch --model $MODEL --baseline docs/eval/calibration/baselines/latest.json`

**Ticket body says `crates/mika-agent/src/calibration/scenarios/` but that path doesn't exist literally.** Actual fixtures live in the tests directory. Plan will correct this.

**E. Review guide** (`mika/docs/architecture/review-guide.md`):
Existing principles reference. Doesn't currently enumerate "GROOMED contract" guarantees for downstream consumers. Ticket's AC4 asks for this addition.

## Hypothesis (committed)

**The TBD-detection gate is necessarily LLM-level** — there's no deterministic code that can semantically validate "this plan has unresolved decisions." It requires reading the plan content and understanding placeholder shapes (TBDs, "pick one", `<version>`, etc.).

Per [[feedback-prompt-enforcement-fragile]] + [[feedback-structural-enforcement-layer-for-tool-requirements]]: LLM-prompt rules don't reliably bind alone. The right composite is:
1. **Prompt-level rule** (the architect IS the LLM doing the gating; the rule MUST be in its system prompt — there's no structural alternative for semantic validation)
2. **Calibration test** (verifies the rule binds against fixture plans with deliberate TBDs)
3. **Documentation** (review-guide.md "GROOMED contract" section names what the verdict guarantees, so downstream consumers can assume it)

The prompt rule is necessary-but-not-sufficient. The calibration test is the structural binding — CI fails if the architect skill produces READY/GROOMED on a TBD-containing plan.

## Approach (committed)

### A. Tighten mika-arch-groom-ticket system prompt

Add a new "Unresolved-Decision Gate" section before the disposition-emission contract:

```markdown
### Unresolved-Decision Gate (mika#1244)

**A plan with ANY unresolved decision MUST return ITERATE (with the unresolved items enumerated in the F-list) — NOT READY.**

Unresolved decisions include (non-exhaustive):
- Literal `TBD` / `tbd` tokens in the plan
- "Pick one" / "Choose between" / "Either ... or ..." without committing to one
- Unspecified version pins (`<tag>`, `<version>`, "TBD version")
- Placeholder paths (`<path>`, `path/to/...`, "TBD path")
- "Operator decides" / "Decision deferred" / "Awaiting input"
- Phrasing that defers a load-bearing design choice to the implementer
- Any "we'll decide at implementation time" hedging on a design surface

**Decision tree:**
1. If plan has unresolved decisions AND the architect can rule on them with principle citations: return `ITERATE` with the decisions enumerated as findings (BLOCKING).
2. If plan has unresolved decisions AND they genuinely require operator judgment outside architect authority: return `ESCALATE` naming the operator-decision (BLOCKING).
3. If plan has no unresolved decisions AND passes principle review: return `READY`.

**The contract downstream consumers depend on:** GROOMED means *the plan is implementable as-written without further operator input on design decisions*. /ce:work's Phase 1 "Read Plan and Clarify" clause must be correct-but-unreachable on a GROOMED plan — the implementer should never need to ask.
```

### B. Tighten mika-arch-second-review system prompt

Mirror the gate. Second-pass GROOMED has the same contract — if the revised plan still contains unresolved decisions, return ESCALATE (not GROOMED).

### C. Calibration fixtures (correct path)

Add fixture at `mika/crates/mika-agent/tests/eval/calibration_fixtures/mika-arch/`:
- `groomed_no_tbds_passes.yaml` — clean plan, expected disposition: READY
- `groomed_with_tbd_rejected.yaml` — plan with `TBD: pick a port number`, expected disposition: ITERATE with finding naming the TBD
- `groomed_with_placeholder_path_rejected.yaml` — plan with `<path>` placeholder, expected disposition: ITERATE

Verify against existing `calibrate-mika-arch` Makefile target. If new fixtures format doesn't match existing infrastructure, follow the shape of fixtures already in the directory.

### D. Update review-guide.md

Add a new "GROOMED contract" section:

```markdown
## GROOMED contract — what the verdict guarantees

When mika-arch's second-pass returns `Verdict: GROOMED`, the plan is guaranteed:

1. **Implementable as-written** — no TBDs, no "pick one" choices, no `<version>` or `<path>` placeholders.
2. **No load-bearing decisions deferred to the implementer** — the architect ruled on or escalated everything material.
3. **Architecturally sound** per Single Responsibility / DRY / YAGNI / KISS / Orthogonality.

The implementer can `/ce:work` the plan headlessly without surfacing clarifying questions. If they discover an ambiguity at execution time that grooming couldn't have anticipated, that's an orthogonal operator-question-relay concern (see ticket separate-followups).
```

### E. Regression scenario

The calibration test (C above) is the regression. Documented in the calibration baseline; new failure mode = regression caught.

## Acceptance Criteria (concrete)

1. **AC1:** `mika/skills/bundled/mika-arch-groom-ticket/system_prompt.md` has the "Unresolved-Decision Gate" section before the disposition contract. Verified by grep for "Unresolved-Decision Gate" in the file.

2. **AC2:** `mika/skills/bundled/mika-arch-second-review/system_prompt.md` mirrors the gate. Verified by grep.

3. **AC3:** New calibration fixtures at `mika/crates/mika-agent/tests/eval/calibration_fixtures/mika-arch/`:
   - `groomed_no_tbds_passes.{yaml|md|json — match existing fixture format}` 
   - `groomed_with_tbd_rejected.<ext>` — expected disposition includes TBD-naming finding
   - At least one of the rejected-fixtures' expected disposition is `ITERATE` (not READY/GROOMED).

4. **AC4:** `mika/docs/architecture/review-guide.md` has a "GROOMED contract" section enumerating the 3 guarantees.

5. **AC5:** `make calibrate-mika-arch MODEL=<canonical-model>` succeeds against the baseline, including the new fixtures. Documented exit/output shape in the PR description.

6. **AC6:** `cargo test -p mika-agent --lib` + `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean.

## Files to change

- `mika/skills/bundled/mika-arch-groom-ticket/system_prompt.md` — new section
- `mika/skills/bundled/mika-arch-second-review/system_prompt.md` — mirror
- `mika/crates/mika-agent/tests/eval/calibration_fixtures/mika-arch/groomed_no_tbds_passes.<ext>` — new
- `mika/crates/mika-agent/tests/eval/calibration_fixtures/mika-arch/groomed_with_tbd_rejected.<ext>` — new
- `mika/crates/mika-agent/tests/eval/calibration_fixtures/mika-arch/groomed_with_placeholder_path_rejected.<ext>` — new (optional, sharpening)
- `mika/docs/architecture/review-guide.md` — new GROOMED contract section

**Note:** the actual fixture file extension + schema follows existing fixtures in the directory. Implementer reads existing fixtures first to match format.

## Out of scope (per ticket)

- Operator-question relay for runtime-discovered ambiguities (separate ticket; this gate prevents grooming-time ambiguities, not execution-time discoveries)
- `/ce:work` Phase 1 clarify-clause override in dev-pilot (workaround; this gate makes it correct-but-unreachable)
- Tightening of mika-arch's principle review beyond the TBD gate (different concern)

## Risk

Medium-low.
- **LLM-prompt-enforcement fragility** ([[feedback-prompt-enforcement-fragile]]): LLMs may still slip GROOMED through on edge-case plans. Mitigated by the calibration test (structural binding) — CI fails if regression observed against baseline.
- **False positives on legitimate prose-phrasing**: phrasing like "decision: option A (chosen over B)" mentions option B but isn't a TBD. The gate must be careful to distinguish "deferred decision" from "documented-but-decided decision." Mitigated by explicit decision-tree wording in the prompt section.
- **Calibration baseline drift**: every prompt change shifts the baseline. Mitigated by re-baselining as part of the PR.

## Implementation order

1. Read existing calibration fixtures in `tests/eval/calibration_fixtures/mika-arch/` — confirm format.
2. Draft prompt section for `mika-arch-groom-ticket` and `mika-arch-second-review`.
3. Add fixtures matching existing format.
4. Run `make calibrate-mika-arch MODEL=<model>` to capture baseline shift; commit new baseline.
5. Add review-guide.md "GROOMED contract" section.
6. Run `cargo test -p mika-agent --lib` + clippy.
7. Manual sanity check: feed a known-TBD plan through `/mika-ask-arch --enable-skill mika-arch-groom-ticket` and verify ITERATE disposition with TBD-naming finding.

## Test plan

1. Unit: calibration fixtures (3 above) pass against baseline.
2. Manual: feed a plan-with-TBD to the architect, observe ITERATE disposition.
3. Regression: existing mika-arch calibration baseline still passes.
