---
title: "fix(engine+skills): structural required-finding-list guard for mika-arch verbatim findings emission"
type: fix
status: active
date: 2026-04-30
---

# fix(engine+skills): structural required-finding-list guard for mika-arch verbatim findings emission

## Overview

mika-arch's two grooming skills (`mika-arch-groom-ticket`, `mika-arch-second-review`) drift toward thin acknowledgements that persist findings to memory via `store_fact` but emit only a terse summary in the final assistant message. The operator's `/mika-groom-ticket` Phase 4 step 11 ("Update plan to address each architect concern") depends on in-band findings; without them, the operator pays ~$1-2 of additional Opus 4.7 spend per ticket plus ~5-10 min of operator-side recovery via the disconfirmation procedure (operator-side `tool_calls.store_fact` query). This is **N=8 of the conditional-disclosure-evasion failure class** (per architect persisted memory current_priorities). Per `feedback_prompt_enforcement_fragile.md` and the `engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` precedent, three documented incidents establish the structural-ratchet condition; we're at eight. The fix is an engine-side `required_finding_list` post-condition guard analogous to mika#864's `required_suffix_lines`: skill declares the contract in `skill.toml [output]`; engine scans the final assistant text for an F-list pattern; rejects EndTurn once on miss with a corrective re-prompt. Emission is **unconditional** (every disposition emits an F-list, including READY which contains "verified principles" entries) per the discarded-review F1 finding — conditional-on-disposition phrasing is the exact rationalization vector the structural fix must close.

## Problem Frame

**Verified evidence (architect persisted memory + this session's grooming experience):**

- mika-arch persisted memory tracks the failure class as `n_8_conditional_disclosure_evasion_pattern` per `current_priorities` core memory updates today.
- Today's grooming sessions on mika#876 produced thin emission on **both** first-pass AND second-pass turns — verbatim findings recovered operator-side from `tool_calls.store_fact` rows. Direct cost: ~10 min operator-side recovery on this single groom + delayed Phase 4 iteration.
- Today's grooming session on mika#904 produced **clean verbatim findings** on both passes — the architect demonstrated the discipline this fix formalizes. So the discipline is achievable; it just doesn't hold deterministically without structural enforcement.
- Earlier discarded review (mika-arch retroactive on commit `1d5a420c`, session `1e148899-d577-4cb7-be7d-4e25d016c708`) produced 3 BLOCKING findings + 4 sharpenings. The findings persist in mika#901's most-recent operator comment (2026-04-30T10:39:24Z) and are reused as the F1-F7 input for this re-groom.

**The discarded review's load-bearing critique (F1):** the original `1d5a420c` plan used **conditional-on-disposition phrasing** ("When your disposition is ITERATE or ESCALATE, the final assistant message MUST contain every finding"). F1 named two failure modes:
1. Disposition-first reasoning gap: agent composes terse final message and reasons "disposition gets named at the end."
2. Self-rationalization: agent emits "Disposition stands: ITERATE" with persisted-findings reference and reasons "the persistence covers the contract."

The structurally robust shape per F1: **every first-pass review session emits an F-list as a structural constant of the final message** — like the `Disposition:` line itself. On READY, list contains "verified principles" entries (e.g., `F1: Verified — orthogonality holds; Change 2 propagates within the shared lib only.`). On ITERATE/ESCALATE, list contains blockers and sharpenings.

This fix implements that unconditional shape via an engine guard, eliminating the rationalization vector entirely.

**The discarded review's other findings (F2-F3):**
- **F2:** Detector / counting surface / threshold for structural escalation. The structural fix IS the detector (engine guard fires per-turn). Counting surface = log events. Threshold = N=1 single-retry per turn (matches mika#864's pattern). F2 was BLOCKING for the prompt-only fix; resolved by switching to structural.
- **F3:** Unit 3's "Self-contained final response" parity addition to `mika-arch-groom-milestone` was scope creep. Resolution: drop from this plan. mika-arch-groom-milestone parity = separate ticket if needed.

## Requirements Trace

- **R1 (revised per first-pass F3 — honor operator spec).** Final assistant message in `mika-arch-groom-ticket` and `mika-arch-second-review` skills MUST contain an F-list **CONDITIONALLY on disposition** — when disposition is ITERATE or ESCALATE (or verdict ESCALATE for second-review), the F-list is required; on READY (or verdict GROOMED), the message may stay short per mika#901 issue body's operator-authored acceptance criterion: *"READY/GROOMED can stay short since no iteration is needed."* The discarded-F1 reasoning (unconditional emission) was sound on its own logic but conflicts with operator spec; per issue-as-versioned-contract pattern, architect cannot ratify spec divergence unilaterally.
- **R2 (revised per first-pass F2 — compose with mika#864 landmark).** Engine post-condition guard scans from message start UP TO (exclusive of) the line matching the skill's `required_suffix_lines` landmark for any line matching a declared F-list prefix. NO parallel scan-window parameter — the suffix-line landmark is the terminator. Single source of truth; reuses mika#864's existing per-skill suffix-line declaration.
- **R3 (revised per first-pass F1 — closed-alphabet `Vec<String>` matching mika#864 shape).** Skill declares the contract in `skill.toml [output]` section as: `required_finding_list_prefixes = ["F1:", "F2:", "F3:", "F4:", "F5:", "F6:", "F7:", "F8:", "F9:", "F10:"]`. Closed-alphabet bounded list. Bounded at F10 because no observed grooming session has emitted >10 findings (mika#904 second-pass: 7 findings; today's first-pass: 5 findings). If a future session needs F11+, that's a discoverable sentinel — bump the list. Composes with mika#864's existing `Vec<String>` machinery; `validate_skill()` already rejects malformed entries; Warn-not-Fail on empty already in place. Mirrors mika#864's anti-regex precedent: *"regex is a footgun (silent failure to fire when pattern is malformed)."*
- **R4.** Threshold = N=1 single-retry per turn (matches mika#864). The retry boundary is per-turn, not per-ticket: any final assistant message in a `mika-arch-groom-*` skill turn that lacks an F-list (when guard fires conditionally per R1) gets one retry, then accepts whatever the model produces.
- **R5.** Per `feedback_make_deploy_wipes_editable.md`: skill prompt updates land alongside skill.toml updates so the deployed skill manifest matches the prompt's documented contract.
- **R6.** Operator `/mika-groom-ticket` Phase 4 step 11 ("Update plan to address each architect concern") works without operator-side `tool_calls.store_fact` recovery procedure post-deploy.
- **R7 (revised per first-pass F5).** **First production test of the fix is the next mika-arch grooming session post-deploy** — NOT qa-review on this PR. qa-review uses its own `required_suffix_lines` (mika#864's mechanism), a DIFFERENT field; the new `required_finding_list_prefixes` guard applies only to opted-in mika-arch skills via skill.toml. Friend's chicken-and-egg framing dissolves trivially because the guard is mika-arch-only — qa-review on this PR runs under its own existing guard, unaffected by this fix.
- **R8.** No regression on existing skill output contracts (mika-arch-groom-ticket and mika-arch-second-review already declare `required_suffix_lines` per agent.rs:744 docstring referencing skills "currently opted-in: `mika-arch-second-review` (GROOMED/ESCALATE) and `mika-arch-groom-ticket` (READY/ITERATE/ESCALATE)"). The new `required_finding_list` is additive.
- **R9.** **mika-arch-groom-milestone parity = OUT of scope** per discarded-F3. Separate ticket if/when its emission discipline drifts (single-skill-at-a-time discipline per `feedback_implementation_scope_bundling.md`).

## Scope Boundaries

- **In scope:**
  - Engine post-condition guard in `crates/mika-agent/src/agent.rs` analogous to `required_suffix_lines` (Unit 1).
  - `[output] required_finding_list = true` field added to `skill.toml` for `mika-arch-groom-ticket` and `mika-arch-second-review` (Unit 2).
  - Skill prompt updates documenting the unconditional F-list contract + worked examples for READY/ITERATE/ESCALATE shapes (Unit 3).
  - Behavioral test fixtures replaying the thin-emission failure shapes from today's mika#876 grooming as red tests against the pre-guard build, green after Unit 1 lands (Unit 4).
- **Out of scope:**
  - **mika-arch-groom-milestone parity** (per discarded-F3 + R9). Separate ticket. Drift can be observed independently.
  - **Memory-key namespacing between mika-arch skills** (mika#904 surfaced this as separate ticket). Orthogonal to emission discipline.
  - **Counting-surface persistence in DB** (per discarded-F2). The engine guard is the detector; log events are the counting surface; no new DB table needed.
  - **N=2 escalation threshold** (per discarded-F2). N=1 matches mika#864's ratchet; the structural fix doesn't need a soft-launch threshold.
  - **Backwards-compatible run on existing groomed plans.** Plans groomed under the old (thin-emission) regime retain their format. Going forward, all new grooming sessions use the F-list contract.

## Phase 0 Pins (load-bearing source verification)

### Pin 1: mika#864 `required_suffix_lines` guard — pattern reference

`crates/mika-agent/src/agent.rs:744-2232` documents the existing pattern:

```rust
/// `required_suffix_lines` specifies literal lines (from matched skills' `[output]` sections)
/// that the assistant text must end with...
fn check_required_suffix_lines(
    text: &str,
    required_suffix_lines: &[String],
    // ... single-retry tracking
) -> CheckResult { /* ... */ }
```

Call site at `agent.rs:1418-1460`:
```rust
let lines: Vec<&str> = text.lines().rev().filter(|l| !l.trim().is_empty()).take(3).collect();
let satisfied = lines.iter().any(|line| required_suffix_lines.iter().any(|req| *line == req));
if !satisfied {
    let lines_display: Vec<String> = required_suffix_lines.iter().map(/* format */).collect();
    // inject correction message:
    // "(Required by skill [output].required_suffix_lines. ..."
}
```

Aggregation at `agent.rs:1982`:
```rust
let required_suffix_lines = collect_required_suffix_lines(&matched);
```

`collect_required_suffix_lines` (defined elsewhere in skills loading) unions `[output].required_suffix_lines` arrays from matched skills. Currently opted-in: `mika-arch-second-review` (GROOMED/ESCALATE) and `mika-arch-groom-ticket` (READY/ITERATE/ESCALATE).

**Implication for Unit 1:** mirror this shape exactly. New function `check_required_finding_list`, new aggregation `collect_required_finding_list`, new boolean `required_finding_list: bool` field on the Output config struct. Single-retry semantics; same scan window (last 3 non-empty lines or larger — see Key Technical Decision below).

### Pin 2: skill.toml `[output]` section structure

mika-arch-groom-ticket's current skill.toml:
- `[skill]` section
- `[output]` section with `required_suffix_lines = ["..."]`

Add field: `required_finding_list = true` (boolean — simpler than literal-line array because the contract is "regex `^F\d+:` matches at least one line in the scan window," not "literal line match").

### Pin 3: F-list scan window and pattern

The required pattern is `^F\d+:` (literal `F` + digits + colon, at start of line). Scan window: **last 30 non-empty lines** (vs. mika#864's 3 — F-lists can be long; an architect emitting F1-F12 with citations and change-required sub-bullets needs a wider window). The 30-line cap is a heuristic; alternative shapes considered: (a) entire body scan (false positives if the model writes `F1:` in an example or quote — risk acceptable given F-list is the dominant pattern); (b) last 50 lines (over-generous; doesn't cost much).

Lean: 30 lines as the v1 cap. Tunable via const if empirical drift surfaces.

### Pin 4: Discarded-review F1-F7 findings as input

The earlier `1d5a420c` retroactive review (session `1e148899-d577-4cb7-be7d-4e25d016c708`) produced findings preserved in mika#901's 2026-04-30T10:39:24Z comment. F1-F7 inform this plan:
- **F1 (BLOCKING) — applied:** unconditional emission via structural guard.
- **F2 (BLOCKING) — applied via switch to structural:** detector = engine guard; counting surface = log events; threshold = N=1.
- **F3 (BLOCKING) — applied via scope:** mika-arch-groom-milestone parity OUT (R9 + Out of scope).
- **F4 (sharpening):** broaden citation surface in skill prompt's F-list example. Apply in Unit 3.
- **F5 (sharpening):** end-of-list anchor placement. Doesn't apply to a structural guard (engine scans the final-message body, position-agnostic within scan window). N/A.
- **F6 (sharpening):** R4 invariant memory persistence as defense-in-depth. Preserve `store_fact` calls in skill prompts (Unit 3 explicitly does NOT remove them).
- **F7 (sharpening):** verification path is empirical re-run. Apply via R7 (qa-review on this PR is the first production test).

## Context & Research

### Relevant Code

- **Pattern reference:** `crates/mika-agent/src/agent.rs:744+` — `required_suffix_lines` guard family
- **Skill manifest parser:** `crates/mika-agent/src/skills/` — likely `skill.toml` deserialization site for the `[output]` section
- **Match-aggregation:** `agent.rs:1982` `collect_required_suffix_lines(&matched)` — same shape needed for `collect_required_finding_list`
- **mika-arch skills:** `skills/bundled/mika-arch-groom-ticket/{skill.toml, system_prompt.md}`, `skills/bundled/mika-arch-second-review/{skill.toml, system_prompt.md}`

### Institutional Learnings

- **mika#864** — engine-guard analog this fix mirrors. Same retry shape, same skill-declaration mechanism.
- **`engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`** — informs the prompt-to-structural ratchet at N=8 (vs N=3 codification threshold).
- **`feedback_prompt_enforcement_fragile.md`** — prompt-only fixes drift; structural enforcement is the durable answer.
- **architect persisted memory `n_8_conditional_disclosure_evasion_pattern`** (current_priorities) — the empirical evidence base.

## Key Technical Decisions

- **Conditional F-list emission on terminal disposition** (per first-pass F3 — REVERSES discarded-F1 to honor operator spec). F-list required on ITERATE/ESCALATE per mika#901 issue body's acceptance criterion: *"READY/GROOMED can stay short since no iteration is needed."* The discarded-F1 reasoning (unconditional emission eliminates rationalization) was sound on its own logic, but per issue-as-versioned-contract pattern the architect cannot ratify spec divergence unilaterally. Bounded narrowly: rationalization can only fire on READY/GROOMED, where the failure mode is "false negative" (architect emits clean READY when should have emitted ITERATE) — that failure mode predates this plan and isn't what mika#901 is filed against. mika#901 defends specifically against thin emission on ITERATE.

- **Closed-alphabet `Vec<String>` matching mika#864 anti-regex precedent** (per first-pass F1). Skill declares `required_finding_list_prefixes = ["F1:", "F2:", ..., "F10:"]` as a literal-string starts-with match list. NO regex (per mika#864 grooming-history verbatim: *"regex is a footgun (silent failure to fire when pattern is malformed)"*). Bounded at F10; future bump is a discoverable sentinel via Warn-not-Fail.

- **Compose with mika#864's `required_suffix_lines` landmark, not parallel scan window** (per first-pass F2). F-lists are message body, not message tail. Scan range = message start UP TO (exclusive of) the suffix-line landmark. Single source of truth; no parallel parameter.

- **Single-retry on miss** (R4, mirrors mika#864). One retry per turn; structural fix doesn't need a soft-launch threshold.

- **Drop `mika-arch-groom-milestone` parity** (discarded-F3, R9, first-pass F6). Single-skill-at-a-time discipline. milestone-skill drift can be observed independently; if the same N=8 pattern recurs there, file a separate parity ticket.

- **Skill prompt updates retain `store_fact` persistence** (discarded-F6 R4 invariant). Memory persistence is defense-in-depth; the structural guard is the primary surface but persistence remains valuable for cross-session lookup.

- **First production test = next mika-arch grooming session post-deploy** (per first-pass F5 — REVERSES R7's prior framing). qa-review on this PR runs under its own existing `required_suffix_lines` guard (mika#864's mechanism), DIFFERENT field. The new `required_finding_list_prefixes` guard applies only to opted-in mika-arch skills via skill.toml. Friend's chicken-and-egg framing dissolves trivially — guard is mika-arch-only.

- **`validate_skill()` Warn-not-Fail inheritance** (per first-pass F7). mika#864's existing `validate_skill()` extension handles the new field symmetrically — malformed entries rejected, declared-but-empty list emits Warn following #510/#511 validation pattern. No new validation code required.

- **No backwards-compat for existing groomed plans** (Out of scope). Plans groomed pre-fix retain their format; new grooms use the F-list contract. Avoids migration scope.

## Implementation Units

- [ ] **Unit 1: Engine post-condition guard `check_required_finding_list`**

  **Goal:** Add the structural guard that scans the final assistant message for the F-list pattern.

  **Requirements:** R1, R2, R4

  **Dependencies:** None (additive engine code).

  **Files:**
  - Modify: `crates/mika-agent/src/agent.rs` — add `check_required_finding_list` function near `check_required_suffix_lines`; wire into the EndTurn post-condition chain.
  - Modify: `crates/mika-agent/src/skills/` — extend `[output]` config struct with `required_finding_list: bool` field; extend `collect_*` aggregators.

  **Approach (per first-pass F1 + F2 + F3 + F7):**
  - **New function signature** mirrors mika#864's exactly — closed-alphabet `Vec<String>` shape, NO regex (per F1's anti-regex precedent verbatim from mika#864 grooming-history: *"regex is a footgun (silent failure to fire when pattern is malformed)"*):
    ```rust
    fn check_required_finding_list(
        text: &str,
        required_finding_list_prefixes: &[String],
        required_suffix_lines: &[String],  // F2: suffix-line is the F-list scan terminator
        disposition_is_terminal: bool,     // F3: only enforce on ITERATE/ESCALATE per operator spec
        retry_done: &mut bool,
    ) -> CheckResult {
        // F1 (revised): empty list → contract not declared → no-op.
        if required_finding_list_prefixes.is_empty() { return CheckResult::Ok; }
        // F3 (revised): operator spec exempts READY/GROOMED — short message permitted.
        if !disposition_is_terminal { return CheckResult::Ok; }
        if *retry_done { return CheckResult::Ok; }  // single-retry semantics

        // F2 (revised): scan from message start UP TO (exclusive of) the line matching
        // any required_suffix_line. F-list lives in body, not tail. Compose with mika#864
        // landmark; no parallel scan-window parameter.
        let lines: Vec<&str> = text.lines().collect();
        let suffix_line_idx = lines.iter().position(|line| {
            let trimmed = line.trim();
            required_suffix_lines.iter().any(|req| trimmed == req.as_str())
        });
        let scan_range_end = suffix_line_idx.unwrap_or(lines.len()); // if no suffix, scan whole body
        let scan_lines = &lines[..scan_range_end];

        // F1: literal-string starts-with match against any declared prefix.
        let satisfied = scan_lines.iter().any(|line| {
            let trimmed = line.trim_start();
            required_finding_list_prefixes.iter().any(|prefix| trimmed.starts_with(prefix.as_str()))
        });

        if satisfied {
            CheckResult::Ok
        } else {
            *retry_done = true;
            CheckResult::Reject(/* correction message — see below */)
        }
    }
    ```

  - **`disposition_is_terminal` semantics:** caller infers from skill output — for `mika-arch-groom-ticket`, ITERATE and ESCALATE are terminal; READY is non-terminal. For `mika-arch-second-review`, ESCALATE is terminal; GROOMED is non-terminal. The detection mechanism: scan the assistant text for `Disposition: ITERATE`, `Disposition: ESCALATE`, or `Verdict: ESCALATE` (literal-string match against existing `required_suffix_lines` declarations from mika#864). Any non-terminal disposition (READY/GROOMED) bypasses the F-list check per F3 — this composes naturally with the existing `required_suffix_lines` enforcement (suffix line MUST be present per mika#864; once present, its content determines whether the F-list guard fires).
  - **Correction message:**
    ```
    [Your response was rejected because it does not contain the required F-list emission per the skill's `[output] required_finding_list = true` contract.

    Every grooming review session — first-pass and second-pass alike — MUST emit findings as `F1:`, `F2:`, etc., in the final assistant message, regardless of disposition. The F-list is a structural constant of the response shape, like the `Disposition:` or `Verdict:` line itself.

    On disposition READY (or verdict GROOMED), the F-list contains "verified principles" entries — e.g., `F1: Verified — orthogonality holds; Change 2 propagates within the shared lib only [citation].` Empty list is acceptable only when there are zero principles to verify, which is rare.

    On ITERATE / ESCALATE, the F-list contains blockers and sharpenings, each with: (a) **Concern** — the concrete issue, (b) **Change required** — what the plan must address, (c) **Citation** — the source grounding the concern (review-guide.md section, ADR number, compound doc path, or specific codebase convention with file:line reference).

    Persisting findings to memory (`store_fact` / `update_core_memory`) is encouraged as defense-in-depth, but the in-band emission is the contract the operator depends on. Re-emit the response with the F-list before EndTurn.]
    ```
  - **Wiring:** add to the EndTurn post-condition chain at the same site as `check_required_suffix_lines`. Order doesn't matter functionally (both guards are independent), but for diagnostic clarity put `required_finding_list` immediately after `required_suffix_lines`.
  - **Aggregation:** add `collect_required_finding_list(&matched) -> bool` (logical OR across matched skills' `[output] required_finding_list` declarations). Mirrors `collect_required_suffix_lines`.
  - **Single-retry tracking:** new flag in the same retry-tracking struct as `required_suffix_lines`'s retry flag.

  **Patterns to follow:**
  - `check_required_suffix_lines` and surrounding code at `agent.rs:744+`.
  - The existing skill.toml `[output]` parsing in `crates/mika-agent/src/skills/`.

  **Test expectation:**
  - Unit test: text with `F1: ...` in scan window → `CheckResult::Ok`.
  - Unit test: text without F-list → `CheckResult::Reject` (first call), `CheckResult::Ok` (subsequent call after retry_done flipped).
  - Unit test: scan window respects 30-line cap (text with `F1:` at line 31 from end → not detected; text with `F1:` at line 30 → detected).
  - Integration test in `tests/eval/grounding_regressions/` mirroring mika#864's regression-fixture pattern: replay a thin-emission case from today's mika#876 grooming, assert the guard fires, retry produces a clean F-list response, second call passes.

  **Verification:**
  - `cargo test -p mika-agent` — all new + existing tests pass.
  - `grep -n "check_required_finding_list" crates/mika-agent/src/agent.rs` — function present, wired.

- [ ] **Unit 2: Skill.toml `[output] required_finding_list_prefixes` for mika-arch skills**

  **Goal:** Declare the closed-alphabet F-list prefix list in mika-arch-groom-ticket and mika-arch-second-review skill manifests.

  **Requirements:** R3 (revised), R5, R8

  **Dependencies:** Unit 1 (engine consumes the field).

  **Files:**
  - Modify: `skills/bundled/mika-arch-groom-ticket/skill.toml` — add `required_finding_list_prefixes = ["F1:", "F2:", ..., "F10:"]` to `[output]`.
  - Modify: `skills/bundled/mika-arch-second-review/skill.toml` — same.
  - **NOT modified:** `skills/bundled/mika-arch-groom-milestone/skill.toml` (R9, out-of-scope per discarded-F3).

  **Approach (per first-pass F1 + F7):**
  - Open each skill.toml. Locate `[output]` section (existing, contains `required_suffix_lines = [...]`).
  - Add adjacent:
    ```toml
    required_finding_list_prefixes = ["F1:", "F2:", "F3:", "F4:", "F5:", "F6:", "F7:", "F8:", "F9:", "F10:"]
    ```
  - No removal of existing fields. The `Vec<String>` shape mirrors mika#864's `required_suffix_lines` precedent exactly. F1-F10 bound is observably sufficient (mika#904 second-pass: 7 findings; today's first-pass: 5 findings). Future bump to F11+ is a discoverable sentinel — `validate_skill()` Warn-not-Fail emits a warning and the operator can extend the list.
  - **`validate_skill()` Warn-not-Fail inheritance (per F7):** mika#864's existing `validate_skill()` extension handles this automatically — malformed entries rejected, declared-but-empty list emits Warn following #510/#511 validation pattern. The engine deserializer treats `required_finding_list_prefixes: Option<Vec<String>>` symmetrically with `required_suffix_lines`. No new validation code required.

  **Patterns to follow:**
  - Existing `[output]` declarations using `required_suffix_lines`.

  **Test expectation:** None at unit level — covered by Unit 1's tests.

  **Verification:**
  - `grep -n "required_finding_list_prefixes" skills/bundled/mika-arch-*/skill.toml` returns the new field in both target skills, NOT in groom-milestone.
  - mika-spirit startup logs no skill-validation warnings about the new field beyond expected Warn-not-Fail patterns.

- [ ] **Unit 3: Skill prompt updates documenting the unconditional F-list contract**

  **Goal:** Skill prompts explicitly state the F-list emission as a structural constant and provide worked examples for READY / ITERATE / ESCALATE shapes.

  **Requirements:** R1, R5, R6

  **Dependencies:** None (orthogonal to Unit 1+2; ships in the same PR for atomicity).

  **Files:**
  - Modify: `skills/bundled/mika-arch-groom-ticket/system_prompt.md` — add F-list contract section with worked examples.
  - Modify: `skills/bundled/mika-arch-second-review/system_prompt.md` — same.

  **Approach (per first-pass F3 — honor operator spec, conditional-on-disposition):**
  - Add a new constraint paragraph in `### Constraints`:
    *"**F-list emission on terminal disposition.** When disposition is ITERATE or ESCALATE (or verdict ESCALATE for second-review), the final assistant message MUST contain an F-list — one or more lines starting with `F1:`, `F2:`, ..., up through `F10:`. The F-list is enforced by the engine's `required_finding_list` post-condition guard (mika#901) — missing F-list on terminal disposition rejects EndTurn once with a corrective re-prompt. Each finding has three sub-fields: (a) **Concern** — the concrete issue, (b) **Change required** — what the plan must address, (c) **Citation** — the source grounding the concern (review-guide.md section, ADR number, compound doc path, or specific codebase convention with file:line reference). Persisting findings to memory (`store_fact` / `update_core_memory`) is encouraged as defense-in-depth, but the in-band emission is the contract the downstream operator depends on. **On READY / GROOMED, the F-list is NOT required — the message may stay short** since no iteration is needed (per mika#901 issue body's operator-authored acceptance criterion)."*

  - Add two worked-example F-list shapes (ITERATE, ESCALATE) in the prompt's existing examples section. READY/GROOMED examples remain short per operator spec.
    ```
    ### Disposition: ITERATE example (F-list required)
    F1 (BLOCKING): Concrete issue X.
       Concern: ...
       Change required: ...
       Citation: review-guide.md § Y + crates/foo.rs:42
    F2 (sharpening): ...
       ...

    Disposition: ITERATE
    ```
    ```
    ### Disposition: READY example (F-list optional, brief acceptable)
    Plan-on-branch ratifies the architect's first-pass review. No remaining concerns.

    Disposition: READY
    ```

  - Preserve all existing `store_fact` / `update_core_memory` instructions (discarded-F6 R4 invariant). The structural guard is the primary surface; memory persistence remains defense-in-depth.

  **Patterns to follow:**
  - Existing `### Constraints` style in the same skill prompts.
  - The discarded `1d5a420c` plan's worked examples (preserved as drafting reference; do not commit any of `1d5a420c`'s code — that branch was discarded — but the example-shape language is operator-authored and reusable).

  **Test expectation:** None at prompt level (prompts not behaviorally testable in unit scope; verification via Unit 4's integration test).

  **Verification:**
  - `grep -n "Unconditional F-list emission\|required_finding_list" skills/bundled/mika-arch-*/system_prompt.md` — present in both targets.
  - Prompt size check post-edit — qa-review and self-dev prompts are at high cap utilization (per mika#852's 95%-cap concern). mika-arch-groom-ticket is currently ~5KB and has plenty of headroom; new constraint paragraph adds ~500 bytes. Confirm: `wc -c skills/bundled/mika-arch-groom-ticket/system_prompt.md` < 57344 bytes.

- [ ] **Unit 4: Behavioral test fixtures replaying thin-emission failure shapes**

  **Goal:** Pin the failure class as red tests against pre-guard build; green after Unit 1. Prevents regression.

  **Requirements:** R1, R2, R7, R8

  **Dependencies:** Units 1 + 2 + 3.

  **Files:**
  - Add: `crates/mika-agent/tests/eval/grounding_regressions/fixtures/required_finding_list_caught.json` (pre-fix response demonstrating thin emission).
  - Add: `crates/mika-agent/tests/eval/grounding_regressions/required_finding_list_*.rs` (regression-reproduction tests).

  **Approach (per first-pass F4 — boundary-test discipline):**

  Mirror mika#864's eval suite shape — caught + unconstrained + boundary-inclusive + boundary-exclusive.

  - **Test 1 — `required_finding_list_caught_on_iterate`:** Replay today's mika#876 first-pass thin emission ("Both patterns persisted... Disposition: ITERATE", no F-list). Assert guard fires on thin response when skill declares `required_finding_list_prefixes` AND disposition is ITERATE; assert retry response containing `F1:` passes.
  - **Test 2 — `required_finding_list_no_op_on_ready`:** Seed thin response with disposition READY (no F-list). Assert guard does NOT fire — operator-spec exemption per R1 revised. Critical: confirms conditional-on-disposition contract.
  - **Test 3 — `required_finding_list_no_op_when_unset`:** Seed thin response with skill that does NOT declare `required_finding_list_prefixes` (e.g., qa-review). Assert guard does not fire. Validates per-skill opt-in.
  - **Test 4 — `required_finding_list_position_inclusive`:** F-list line position immediately before the `required_suffix_lines` landmark. Assert satisfies (inclusive of pre-suffix range — the suffix line is the EXCLUSIVE terminator). Mirrors mika#864's "position-3 boundary (inclusive, satisfies)" discipline.
  - **Test 5 — `required_finding_list_position_exclusive`:** F-list line position AFTER the `required_suffix_lines` landmark. Assert does NOT satisfy. Mirrors mika#864's "position-4 boundary (exclusive, fires)."
  - **Test 6 — `required_finding_list_position_at_message_start`:** F-list line at message start (line 0); suffix line at message end. Assert satisfies (no lower bound on scan range).
  - **Test 7 — `required_finding_list_code_fence_false_positive_documented`:** F-list pattern appears inside a code-fence example (`\`F1: example\``) but NOT in prose. Per F4's "fourth case": engine guard accepts this as false positive — grooming output legitimately won't contain F-list-shaped examples in code fences (the documented behavior is "accept; rely on architect emission discipline plus structural enforcement composing reasonably"). Test asserts current behavior so future tightening is intentional, not accidental. Alternative: scan skips fenced ranges — defer to follow-up if false-positive rate becomes empirically observable.

  **Patterns to follow:**
  - Existing fixtures at `crates/mika-agent/tests/eval/grounding_regressions/fixtures/` and surrounding test modules.
  - mika#864's regression fixtures (likely at the same path).

  **Test expectation:** All three new tests pass post-Unit-1.

  **Verification:** `cargo test -p mika-agent --test eval grounding_regressions::required_finding_list` — all pass.

## System-Wide Impact

- **Interaction graph:** New guard joins the existing post-condition chain (text-based tool-call detection, prose-style tool-call detection, required-tools gate, etc.). All guards independent; new guard checks F-list-pattern presence, no overlap with existing guards.
- **Error propagation:** Single-retry on miss; if model emits thin response twice, the second emission is accepted (engine doesn't loop). Same shape as mika#864.
- **State lifecycle risks:** None — guard is per-turn state in the existing retry-tracking struct.
- **API surface parity:** No new public APIs.
- **Burst invariant / unchanged invariants:** Existing skill output contracts (mika-arch-groom-ticket and mika-arch-second-review's `required_suffix_lines` declarations per agent.rs:744 docstring) preserved. New `required_finding_list` is additive opt-in.
- **Operational observability:** Engine emits structured log event when guard fires (`required_finding_list_rejected` or similar, mirrors `required_suffix_lines_rejected`). Counting surface = log scan; matches discarded-F2's "log events" framing.
- **mika-arch-groom-milestone unchanged:** R9 keeps the parity gap explicit.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| The `^F\d+:` regex matches something other than an architect F-list (false positive: e.g., model writes "F1: 2026" as a date). | Acceptable — guard fires when pattern is PRESENT; false positive means guard PASSES on a non-finding. The failure mode the guard prevents is the F-list being ABSENT. False positives don't cause regression. |
| Model emits F-list with non-incremental numbering (e.g., `F1:` `F3:` skipping `F2:`). Guard accepts; downstream operator may be confused. | Acceptable v1. Guard only checks pattern presence, not enumeration consistency. If empirical drift surfaces, tighten regex to require sequential. |
| 30-line scan window is too narrow (very long F-lists with detailed citations cross 30 lines and the F-list spans into the truncatable tail). | Tunable const. If empirical evidence shows 30 is too small, raise to 50. The lower bound matters more than the upper — too-small misses real F-lists; too-large accepts false positives but doesn't reject valid ones. |
| Skill prompt cap (mika#852 at 95% on self-dev). mika-arch-groom-ticket and mika-arch-second-review are currently <5KB; adding ~500 bytes per skill stays well under cap. | Verify via `wc -c` post-edit per Unit 3 verification. Headroom check is mechanical. |
| Regression on existing `required_suffix_lines` guard (the GROOMED/ESCALATE / READY/ITERATE/ESCALATE suffix discipline). | New guard is additive. Existing field unchanged. Existing tests continue to run. |
| qa-review on this PR (R7) emits thin verdict block, blocking merge. | This is the **first verification step**, not a problem to route around. If qa-review emits thin, fix is incomplete and operator inspects the residual case. The verdict block should pass the existing `required_suffix_lines` guard (qa-review skill declares it), so qa-review's discipline is structurally enforced via a different guard path. The new `required_finding_list` guard does NOT apply to qa-review skill — it's opt-in via skill.toml, and qa-review doesn't add the field. |

## Acceptance

Per ticket AC + R7 (friend's framing):

- ✅ Unit 1's engine guard fires on thin emission and accepts F-list emission. Three behavioral tests pass.
- ✅ Unit 2's skill.toml additions deserialize correctly; `required_finding_list = true` declared on both mika-arch-groom-ticket and mika-arch-second-review.
- ✅ Unit 3's skill prompt updates document the unconditional contract with worked examples for READY / ITERATE / ESCALATE.
- ✅ Unit 4's regression fixtures pass.
- ✅ **R7 first production test:** qa-review on this PR emits a clean verdict block (not thin). If qa-review emits thin, fix is incomplete; do NOT merge.
- ✅ Post-deploy: re-run today's mika#876 first-pass shape; verbatim F1-F7 emitted in-band; operator does NOT need `tool_calls.store_fact` recovery.

**Verification path:**
1. **Unit-test gate (CI):** `cargo test -p mika-agent` — all tests pass.
2. **Build verification:** Engine builds cleanly with the new guard wired.
3. **R7 first production test (this PR):** mika-qa runs against this PR; verdict block emerges clean. If thin, halt and inspect.
4. **Empirical signal (post-deploy, 24h):** zero `required_finding_list_rejected` log events on healthy mika-arch grooming sessions; correlate with operator-side recovery procedure no longer needed (tracked via mika-platform/docs/logs/ entries).

## Future Work

- **mika-arch-groom-milestone parity** — separate ticket if the skill's emission discipline drifts. Apply same `required_finding_list = true` once confirmed.
- **Sequential-numbering regex tightening** — if false-positive non-sequential F-lists surface empirically.
- **Memory-key namespacing between mika-arch skills** (mika#904's deferred follow-up) — orthogonal to this fix but composes.

## Sources & References

- Related issue: mika#901 (this ticket)
- Pattern reference: mika#864 — engine-side `required_suffix_lines` guard (analog this fix mirrors)
- Earlier discarded review: session `1e148899-d577-4cb7-be7d-4e25d016c708`, F1-F7 findings preserved in mika#901 comment 2026-04-30T10:39:24Z
- Sibling failure-class: mika#876 (today's grooming demonstrated the thin-emission bug twice)
- Sibling structurally-related: mika#904 (review-guide.md §7 sharpening — orthogonal but shares the "discipline must be structurally enforced" reasoning)
- Code references:
  - `crates/mika-agent/src/agent.rs:744+` — `required_suffix_lines` guard family (Pin 1)
  - `crates/mika-agent/src/agent.rs:1418-1460` — guard scan + correction injection
  - `crates/mika-agent/src/agent.rs:1982` — `collect_required_suffix_lines` aggregator
- Documentation:
  - `crates/mika-agent/CLAUDE.md` § EndTurn post-condition chain
  - `mika/CLAUDE.md` § Conventions (skill output contracts)
- Institutional learnings:
  - `mika/docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` (the ratchet precedent — N=3 codification, N=8 here means structural is overdue)
  - `feedback_prompt_enforcement_fragile.md` (memory)
  - architect persisted memory `n_8_conditional_disclosure_evasion_pattern` (current_priorities)
