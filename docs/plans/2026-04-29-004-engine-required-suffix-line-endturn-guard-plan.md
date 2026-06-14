---
title: "engine: required-suffix-line EndTurn guard for skill-declared output contracts (verdict ghosting)"
type: engine
status: active
date: 2026-04-29
ticket: senara-solutions/mika#864
branch: engine/864/required-suffix-line-endturn-guard-for
origin: docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md (companion + cross-ref to pending prompt-level-output-discipline-fails-under-load.md)
related: senara-solutions/mika#862 (asserted-unavailability — sibling), senara-solutions/mika#863 (quoted-resource pre-fetch — sibling), senara-solutions/mika#870/#871 (callback-flow guards — adjacent)
---

# engine: required-suffix-line EndTurn guard for skill-declared output contracts (verdict ghosting)

## Overview

mika#864 is the structural counterpart to the verdict-ghosting failure mode documented in mika#788 (trace `03d3ec38-0839-47b6-9226-111b38d8b52b`). The architect's `mika-arch-second-review` skill prompt at `mika/skills/bundled/mika-arch-second-review/system_prompt.md:39-53` declares output MUST end with `Verdict: GROOMED` or `Verdict: ESCALATE`. Under cognitive load, the architect end-turned with no verdict line at all — a meta-acknowledgment that read like a verdict but contained no parseable verdict keyword. The skill spec's "MUST" language is prompt-level; nothing structural enforced it, and downstream consumers (`/mika-groom-ticket`'s pass-1/pass-2 disposition parser) silently fell through to defaults.

This plan adds a manifest-driven post-condition guard on the EndTurn chain. Skills that need to enforce a verdict-line contract opt in via a new `[output] required_suffix_lines = [...]` section in `skill.toml` listing the literal exhaustive accept-set. The guard scans the assistant's last 3 non-empty lines (after whitespace trim) for an exact match to any list entry; missing match rejects the EndTurn and re-prompts.

**Architectural distinction from siblings:** mika#862 (asserted-unavailability) lives in `INTENT_GUARDS` registry. mika#863 (quoted-resource pre-fetch) lives in the skills pipeline at turn-start. mika#864 is a **standalone post-condition** at the EndTurn chain, following the existing single-retry-via-`_retry_done` pattern (text-based detection at `agent.rs:785`, prose-style at 794, completion-claim at 788, fabricated-action at 791, persistence-eval at 798).

## Problem Frame

### Observed failure (from mika#788 trace)

The mika#788 grooming run, second-pass: the architect read its own `current_priorities` core-memory entry cataloguing the prior occurrence (verdict-ghost recurrence-1), proceeded to write a structurally-shaped second-pass response, and end-turned without emitting `Verdict: GROOMED` or `Verdict: ESCALATE`. The compound doc captures the fingerprint: prompt-level "MUST" enforcement does not bind under load, even when the agent has the failure pattern actively in its system prompt.

Trace `03d3ec38-0839-47b6-9226-111b38d8b52b` is the canonical pre-fix evidence. The downstream parser at `/mika-groom-ticket` Phase 4 step 15 expects `Verdict: GROOMED` | `Verdict: ESCALATE` and falls through silently when neither is present, defaulting to a soft "treat as continue" path — exactly the silent-failure shape this guard closes.

### Root cause

Output-format contracts declared in `skill.toml` system prompts are prompt-only constraints. The agent loop has no awareness of "this skill's output must end with a verdict line" — that awareness lives only in the prose of the skill's `system_prompt.md`. Per `feedback_prompt_enforcement_fragile.md`, prompt-level enforcement rationalizes away under cognitive load. The structural counterpart is a manifest-declared accept-set + EndTurn check.

### Why exhaustive literal list, not regex

The issue body's design rationale: closed alphabets compose better with `validate_skill()` and avoid the regex-author footgun (forgotten anchors, unescaped colons, greedy capture). For mika-arch's two skills, the accept-sets are tiny:

- `mika-arch-second-review`: `["Verdict: GROOMED", "Verdict: ESCALATE"]`
- `mika-arch-groom-ticket`: `["Disposition: READY", "Disposition: ITERATE", "Disposition: ESCALATE"]`

Both are explicitly closed alphabets defined by skill design. Regex would make the contract less inspectable for `validate_skill()` and would silently fail to fire on author errors (a missing `^` or `$` anchor producing a pattern that matches mid-line text). Literal list is the safer and more grep-able shape.

## Requirements Trace

- **R1.** New struct `Output` and field `output: Output` added to `SkillManifest` in `crates/mika-agent/src/skills/manifest.rs:24+`. The struct holds:
  ```rust
  #[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
  pub struct Output {
      /// Exhaustive list of literal lines, one of which must appear as the assistant's
      /// last non-empty line (after trimming whitespace) in any turn that ends with
      /// EndTurn while this skill is active. Empty list = no constraint.
      #[serde(default)]
      pub required_suffix_lines: Vec<String>,
  }
  ```
  Skills opt in via `[output] required_suffix_lines = ["...", "..."]` in `skill.toml`. Defaults to empty list (no constraint).
- **R2.** `validate_skill` (`crates/mika-agent/src/skills/index.rs:637`) extended to reject malformed `[output]` entries at skill-load time per the existing #510/#511 validation pattern. Specifically: each entry must be a non-empty string; whole-list-must-be-non-empty-or-omitted. **F5 resolution — empty-list severity is `Warn`, not `Fail`:** an explicit `required_suffix_lines = []` is suspicious (likely author oversight — they meant to add entries but didn't) but is NOT a correctness violation (the engine's no-op for empty list is well-defined: no constraint applied). Hard-rejection on author-uncertainty would create a load-failure footgun for skill development workflows where the field is being added incrementally. Warn-level diagnostic surfaces the issue in `mika skills list`'s warning section without blocking the skill's load. Returns a `SkillDiagnostic` per existing helper functions.
- **R3.** New post-condition guard in `crates/mika-agent/src/agent.rs` at the existing chain site (~lines 955-1333). **Position: END of the chain (per F4)** — after the persistence-evaluation guard at the chain's tail, before the EndTurn return path. Rationale: other guards' rejections (text-tool-call, prose-style, completion-claim, fabricated-action, intent-precondition, persistence-eval) take precedence so a turn that would already be rejected for a more fundamental reason doesn't waste a suffix-line check. **Trigger:** at least one matched skill (Keyword OR AlwaysOn) declares non-empty `output.required_suffix_lines`. **Satisfaction check (read (b) per F7):** assistant's last 3 non-empty lines (after `.trim_end()` per line + skipping fully-blank lines) contain AT LEAST ONE line that exactly equals an entry in the union of required-suffix-lines from all matched skills. The "any-of-last-3" read is chosen over the stricter "last-non-empty-line-only" read because skill outputs commonly end with a code-fence close, a blank trailing line, or a footer that isn't the verdict — strict-last-line would false-negative on standard markdown formatting. The looser read accepts these formatting tails while keeping the verdict near the end. Test scenarios MUST match this read (not the strict alternative). **On violation:** reject EndTurn, inject corrective system message naming the skill path + the accept-set + instruction to re-emit with the verdict appended. Single-retry tracked via new flag `required_suffix_line_retry_done` initialized at `agent.rs:~798` alongside the existing 6 retry flags.
- **R4.** **Last-3-lines scan rationale:** assistant text often ends with markdown trailing whitespace, an empty line, or a code-fence close. Scanning the last 3 non-empty lines (rather than strictly the last line) accommodates the mika-arch output convention without over-relaxing — verdict lines that appear earlier in the response (e.g., in the body of a section) don't satisfy. The window is small enough that the verdict still has to be near the end; large enough to tolerate formatting.
- **R5.** **Corrective system message** (action-first, context-second per pass-2 sharpening — match `webhook_zero_tools`' template shape):
  ```
  [Your response must end with one of these literal lines (any of the last 3
   non-empty lines, after whitespace trim, will satisfy):
     - "Verdict: GROOMED"
     - "Verdict: ESCALATE"
   Re-emit the same response with one of the required lines appended verbatim
   on its own line at the end. Do not paraphrase — the suffix is a structural
   contract parsed by downstream consumers (e.g., /mika-groom-ticket Phase 4
   disposition parser).

   (Required by skill: <skill_path>, [output].required_suffix_lines.
   See feedback_prompt_enforcement_fragile.md for why prompt-level "MUST"
   doesn't bind here.)]
  ```
  Accept-set listed FIRST so the LLM reads the actionable constraint before the
  provenance context. Skill path + meta-doc citation deferred to the parenthetical
  trailer. Same action-first/context-second principle as `webhook_zero_tools`.
- **R6.** mika-arch's two skills opt in. Files:
  - `mika/skills/bundled/mika-arch-second-review/skill.toml` — add `[output] required_suffix_lines = ["Verdict: GROOMED", "Verdict: ESCALATE"]`.
  - `mika/skills/bundled/mika-arch-groom-ticket/skill.toml` — add `[output] required_suffix_lines = ["Disposition: READY", "Disposition: ITERATE", "Disposition: ESCALATE"]`.
- **R7.** Eval scenarios in `crates/mika-agent/tests/eval/grounding_regressions/`:
  - **`required_suffix_line_caught.rs`** — `MockLlmProvider` first turn emits a verdict-shaped response with no trailing `Verdict: GROOMED` / `Verdict: ESCALATE` line. Skill manifest declares the accept-set. Assert: guard fires once; corrective re-prompt issued; turn 2 emits the response with `Verdict: GROOMED` appended; loop exits cleanly. Frozen pre-fix fixture `fixtures/required_suffix_line_caught_pre_fix.json` reproduces the mika#788 trace shape.
  - **`required_suffix_line_unconstrained.rs`** — `MockLlmProvider` first turn emits free-form text. Skill manifest does NOT declare `[output] required_suffix_lines`. Assert: guard does NOT fire; loop exits cleanly with one assistant message. Verifies the false-positive case for non-opt-in skills.
- **R8.** Compound doc updates in `mika/docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`. **F1 resolution — explicit forward-pointer replacement, not just an appended section.** The compound doc contains "once that lands" / "tracked as a separate ship" phrases referencing the structural fix for the verdict-ghosting failure mode. Each occurrence MUST be updated to cite this guard's resolution: `mika#864 required_suffix_line guard (manifest-driven `[output] required_suffix_lines` opt-in, EndTurn post-condition with single-retry, last-3-non-empty-lines scan, structural counterpart for trace 03d3ec38-0839-47b6-9226-111b38d8b52b)`. Implementation step: `grep -n "once that lands\|tracked as a separate ship" docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` to enumerate occurrences before the plan-on-branch commit, then update each in-place. Same shape as mika#841's second-pass on Resolution-section deliverable.
- **R9.** No new DB columns or schema migrations. The required state (matched skills' manifest, assistant text) is already available at the post-condition chain site.

## Proposed Fix

### Primary: standalone post-condition guard

**Where (manifest):** `crates/mika-agent/src/skills/manifest.rs:24+` — add the `Output` struct and `output: Output` field to `SkillManifest` adjacent to the existing `constraints: Constraints` field.

**Where (validation):** `crates/mika-agent/src/skills/index.rs:637` — extend `validate_skill` to inspect `manifest.output.required_suffix_lines` per R2.

**Where (guard chain):** `crates/mika-agent/src/agent.rs:~798` — declare new flag `let mut required_suffix_line_retry_done = false;` adjacent to the existing 6 retry flags. Add the new guard check in the EndTurn post-condition chain at the appropriate sequence point (suggest: after the prose-style tool call detection at the existing chain site ~line 998, before the completion-claim guard, since suffix-line check is a fast string operation that should run before more expensive checks). Pseudocode:

```rust
// #864 — Required-suffix-line guard. Skills can declare an exhaustive accept-set
// for their final line; missing match rejects EndTurn once.
if !required_suffix_line_retry_done {
    let required_set: Vec<&String> = matched_skills.iter()
        .flat_map(|m| m.skill.output.required_suffix_lines.iter())
        .collect();
    if !required_set.is_empty() {
        let last_3_non_empty: Vec<&str> = assistant_text.lines()
            .map(|l| l.trim_end())
            .filter(|l| !l.is_empty())
            .rev()
            .take(3)
            .collect();
        let satisfied = last_3_non_empty.iter()
            .any(|line| required_set.iter().any(|req| line == req.as_str()));
        if !satisfied {
            // Find the skill that declared the constraint for the corrective message
            let declaring_skill = matched_skills.iter()
                .find(|m| !m.skill.output.required_suffix_lines.is_empty())
                .expect("required_set non-empty implies at least one declaring skill");
            let corrective = format_required_suffix_line_corrective(
                &declaring_skill.skill.path,
                &required_set,
            );
            required_suffix_line_retry_done = true;
            // ... inject corrective via the existing rejection path, re-enter loop
            continue;
        }
    }
}
```

The check is a pure string operation on already-available state — no new DB calls, no new context fields beyond the existing `matched_skills` already in scope at the post-condition chain site.

**Where (skill manifests):** Two `skill.toml` edits per R6 (mika-arch's two skills).

### Tests

**File:** `crates/mika-agent/tests/eval/grounding_regressions/required_suffix_line_caught.rs` and `required_suffix_line_unconstrained.rs` (new), modelled on the existing `grounding_regressions/` scaffold.

Scenario 1 — **`required_suffix_line_caught`:**
- `EvalHarness` configures a skill with `[output] required_suffix_lines = ["Verdict: GROOMED", "Verdict: ESCALATE"]`.
- `MockLlmProvider` turn 1: emits multi-paragraph verdict-shaped text ending with a meta-acknowledgment line (no `Verdict:` prefix anywhere in the last 3 lines).
- Assert: `required_suffix_line_retry_done` flips to true; corrective system message injected naming both required lines; turn 2 emitted.
- Turn 2: `MockLlmProvider` returns the response with `\n\nVerdict: GROOMED` appended. Assert: loop exits cleanly; one assistant message persisted.
- Frozen pre-fix fixture: `fixtures/required_suffix_line_caught_pre_fix.json` reproduces the mika#788 trace shape (multi-paragraph response, no verdict line).

Scenario 2 — **`required_suffix_line_unconstrained`:**
- `EvalHarness` configures a skill WITHOUT `[output] required_suffix_lines`.
- `MockLlmProvider` turn 1: emits free-form text with no verdict line.
- Assert: `required_suffix_line_retry_done` never flips; loop exits cleanly after turn 1; one assistant message persisted.
- Verifies false-positive avoidance for non-opt-in skills.

**Scenario 3 — `required_suffix_line_position_3_boundary`** (F2-S — positive boundary case for last-3 window):
- Same skill config as scenario 1.
- `MockLlmProvider` turn 1: emits a response where `Verdict: GROOMED` is at position 3 from the end (i.e., line N-2 with two trailing non-empty lines after it that are NOT verdict lines). Wait — under the any-of-last-3 read, this should still satisfy.
- Assert: guard does NOT fire (verdict line is within last 3 non-empty); loop exits cleanly. Verifies the inclusive bound at position 3.

**Scenario 4 — `required_suffix_line_position_4_violation`** (F2-S — negative boundary case):
- Same skill config.
- `MockLlmProvider` turn 1: emits a response where `Verdict: GROOMED` is at position 4 from the end (i.e., line N-3 with three trailing non-empty lines that are NOT verdict lines).
- Assert: guard fires (verdict line is OUTSIDE last 3 non-empty); corrective re-prompt issued. Verifies the exclusive bound at position 4 — locks down the off-by-one regression class.

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/manifest.rs` | Add `Output` struct (line 24+); add `output: Output` field to `SkillManifest`; add unit test mirroring the existing `test_parse_constraints_required_tools` for the new field. |
| `crates/mika-agent/src/skills/index.rs` | Extend `validate_skill` (line 637+) to inspect `manifest.output.required_suffix_lines` per R2 (warn on empty-but-declared list, reject malformed entries). |
| `crates/mika-agent/src/agent.rs` | Declare `required_suffix_line_retry_done` flag (~line 798); add the guard check in the EndTurn chain (~line 998 or appropriate sequence point); add `format_required_suffix_line_corrective` helper. |
| `mika/skills/bundled/mika-arch-second-review/skill.toml` | Add `[output] required_suffix_lines = ["Verdict: GROOMED", "Verdict: ESCALATE"]`. |
| `mika/skills/bundled/mika-arch-groom-ticket/skill.toml` | Add `[output] required_suffix_lines = ["Disposition: READY", "Disposition: ITERATE", "Disposition: ESCALATE"]`. |
| `crates/mika-agent/tests/eval/grounding_regressions/required_suffix_line_caught.rs` | New file — scenario 1. |
| `crates/mika-agent/tests/eval/grounding_regressions/required_suffix_line_unconstrained.rs` | New file — scenario 2. |
| `crates/mika-agent/tests/eval/grounding_regressions/fixtures/required_suffix_line_caught_pre_fix.json` | New fixture — frozen pre-fix trace. |
| `crates/mika-agent/tests/eval/grounding_regressions/mod.rs` | Register new scenarios. |
| `crates/mika-agent/tests/eval/grounding_regressions/README.md` | Add new scenarios to capability matrix; add tags `verdict-suffix-required-but-ghosted` (failure) / `verdict-suffix-emitted` (success) to `grounding:*` namespace. |
| `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` | Append note citing mika#864 as the structural counterpart for verdict-ghosting; resolve any "once that lands" forward-pointers. |
| `CHANGELOG.md` | Add entry under "Added" — "Engine now enforces skill-declared output suffix lines via a manifest-driven EndTurn guard. Closes #864." |

No schema changes. No new dependencies. No new env vars.

## Verification

### Unit / integration

```bash
cd /data/workspace/mika-platform/.claude/worktrees/engine-864-required-suffix-line-endturn-guard-for/mika
cargo test -p mika-agent --test eval grounding_regressions::required_suffix_line
cargo test -p mika-agent skills::manifest  # manifest parsing tests
cargo test -p mika-agent skills::index     # validate_skill tests
cargo test -p mika-agent  # full suite
cargo clippy -- -D warnings
cargo fmt --check
```

### Manual reproduction (post-merge)

mika#788's pre-fix trace `03d3ec38-0839-47b6-9226-111b38d8b52b` is the fingerprint. After deploy:

1. Restart mika-spirit.
2. Run a mika-arch second-review CLI ask that historically triggered the verdict ghost (multi-section response with cognitive load on `current_priorities`).
3. Inspect the resulting session in `~/.mika/data/mika.db`:
   ```sql
   SELECT trace_id FROM messages
     WHERE agent_id = 'mika-arch' AND role = 'assistant'
       AND content NOT LIKE '%Verdict: GROOMED%'
       AND content NOT LIKE '%Verdict: ESCALATE%'
     ORDER BY created_at DESC LIMIT 1;
   ```
4. For that `trace_id`: confirm the next assistant turn emits the missing verdict line. The pre-fix shape — verdict-shaped response with no verdict-keyword line, then loop-exit — must not appear.

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Last-3-lines scan misses verdict lines that appear at line 4 from the end (e.g., assistant adds a footer). | Window is intentionally narrow to enforce verdict-at-end discipline. If real briefs need a wider window, extend to last 5 lines as a follow-up — but the discipline argument is stronger with last-3. |
| Skill author writes malformed entries (typos in `Verdict: GROMED`). | `validate_skill` rejects malformed entries at load time per R2. Typo would mean the skill doesn't load until fixed — fail-closed. |
| Multiple matched skills declare conflicting accept-sets. | Union semantics: any line matching any entry from any matched skill satisfies. Conflict-free by construction. If two skills want mutually-exclusive verdicts (rare), they shouldn't both match in the same turn — that's a skill-design issue, not a guard issue. |
| Corrective re-prompt produces verdict line but in wrong format ("Verdict GROOMED" without colon). | Single-retry semantics: guard fires once, then dormant. Second wrong attempt exits the loop with last assistant message saved. Not catastrophic — operator can manually trigger another turn. Future hardening: extend retry to N (per #870's `intent_guard_retries` shape) if observed pressure emerges. |
| `Output` struct deserialization breaks existing skill manifests that don't have an `[output]` section. | `#[serde(default)]` on the field + `Default` impl on the struct. Existing manifests parse unchanged. Verified pattern from existing `Constraints` field (also defaulted). |
| Conflict with mika#862/#863/#870/#871 on shared agent.rs files. | All four siblings + #864 touch `agent.rs`. Different functions / non-overlapping line ranges. Rebase order doesn't matter; final commit can be in any order. |

## Out of Scope

- **Regex-based suffix patterns.** Issue body explicitly excludes regex per the friend's design feedback. Exhaustive literal list is sufficient for closed-alphabet verdict surfaces. The compound doc captures the rationale.
- **`prompt-level-output-discipline-fails-under-load.md` compound doc.** Separate ship that cites this guard as its structural floor. Issue body explicitly carves it out.
- **Auto-detection of skills that should opt in.** Manifest opt-in is correct: skills that legitimately have free-form output (most of them) shouldn't have a guard. Per-skill justification required for each opt-in.
- **Multi-retry semantics.** Single-retry matches the existing 6 standalone-guard pattern. Migration to N-retry via `intent_guard_retries` HashSet (mika#870 pattern) is deferred until observed pressure emerges (e.g., a skill where the corrective re-prompt regularly produces the wrong format).
- **N=3 vs N=5 last-lines window expansion (F3 sentinel).** N=3 chosen per issue body. Recalibration trigger: if real responses regularly have ≥3 trailing non-verdict lines AND the boundary-case scenarios start producing false-positives, expand to N=5. No need to revisit absent observed pressure.
- **`[output]` vs `[constraints]` namespace (F6 sentinel).** Plan uses `[output]` per issue body verbatim. If a third output-side constraint emerges (e.g., `required_prefix_lines`, `max_response_length`), revisit whether `[output]` namespace is the right home or whether the entries should fold into `[constraints]` for unification. Sentinel filed; not designed now.
- **Shared-helper extraction across the four guards (#862/#863/#864/#870).** Per mika#870/#862's plans: revisit when the second EndTurn-family guard ships. This guard is the third in the family but uses a different retry pattern (standalone vs registry); shared-helper question is even less applicable.
- **Pattern-addition protocol for new skills opting in.** Each skill opt-in is a per-skill design decision (not a generic pattern that needs cross-cutting institutional record). The opt-in IS the documentation; no compound-doc protocol like #862/#863's.

## Open Questions for mika-arch

1. **Last-N-lines window size.** I picked N=3. Issue body says "last 3 lines are scanned to allow for trailing blank lines." If real responses regularly have more than 2 trailing-blank-line equivalents, N=5. Defer-to-architect.
2. **Guard sequence position.** R3 says "after prose-style tool call detection (~line 998), before completion-claim guard." String-comparison is fast; placing it earlier in the chain than expensive checks is correct. But if there's a rationale for placing it last (e.g., other guards' rejections shouldn't waste a suffix-line check), defer-to-architect.
3. **Validation severity.** R2 treats explicitly-empty-list `required_suffix_lines = []` as a `SkillDiagnostic::Warn` (skill loads). Alternative: hard-reject (skill refuses to load). My read: warn is correct — the field declared-but-empty is suspicious but not a security/correctness issue. Defer-to-architect.
4. **Optional 3rd test scenario** (mid-text verdict-line that's not at the end). Worth adding for last-3-lines scope lockdown? Probably yes — small marginal cost. Defer-to-architect.

---

## Architect first-pass concerns (resolved in this revision)

This revision applies the seven findings from mika-arch's first-pass review (session `fcf8d04b-86b3-4f27-8139-4340c3875b16`).

### F1 — Compound-doc forward-pointer update specificity (BLOCKING, resolved)

R8 now states the explicit forward-pointer update protocol: each occurrence of "once that lands" / "tracked as a separate ship" in `mika/docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` MUST be updated in-place to cite mika#864's resolution with the canonical reference text (manifest-driven `[output] required_suffix_lines` opt-in, EndTurn post-condition with single-retry, last-3-non-empty-lines scan, structural counterpart for trace `03d3ec38-…`). Implementation step: `grep -n "once that lands\|tracked as a separate ship"` to enumerate occurrences before commit. Same shape as mika#841's Resolution-section deliverable.

### F2-S — Boundary-case test scenarios added (sharpening, applied)

Tests section now includes scenario 3 (`required_suffix_line_position_3_boundary` — verdict at position 3-from-end → satisfies, inclusive bound verified) and scenario 4 (`required_suffix_line_position_4_violation` — verdict at position 4-from-end → fires, exclusive bound verified). Locks down the off-by-one regression class that the issue-body-mandated two scenarios alone wouldn't catch.

### F3 — N=3/N=5 recalibration sentinel (sharpening, applied)

Out of Scope: N=3 chosen per issue body. Recalibration trigger named: real responses regularly having ≥3 trailing non-verdict lines AND boundary-case scenarios producing false-positives. No revisit absent observed pressure.

### F4 — Guard sequence position pinned to end-of-chain (sharpening, applied)

R3 now states: position is END of the chain (after persistence-evaluation, before EndTurn return). Other guards' rejections take precedence so a turn rejected for a more fundamental reason doesn't waste a suffix-line check.

### F5 — Validation severity rationale documented (sharpening, applied)

R2 now states: empty-list severity is `Warn`, not `Fail`. Rationale: declared-but-empty is suspicious (author oversight) but not a correctness violation (no-op is well-defined). Hard-rejection would create a load-failure footgun during incremental skill development.

### F6 — `[output]` namespace conformity + sentinel (sharpening, applied)

Plan uses `[output]` per issue body verbatim. Out of Scope filed sentinel: if a third output-side constraint emerges (e.g., `required_prefix_lines`, `max_response_length`), revisit whether `[output]` is the right home or whether entries should fold into `[constraints]` for unification.

### F7 — Trim semantics pinned to read (b) (sharpening, applied)

R3 now explicitly chooses the "any-of-last-3" read (b) over the strict "last-non-empty-line-only" read (a). Rationale: skill outputs commonly end with code-fence close, blank trailing line, or footer that isn't the verdict; strict-last-line would false-negative on standard markdown formatting. Test scenarios 3 and 4 are calibrated against read (b).

---

## Architect verdict

- **First-pass (mika-arch session `fcf8d04b-86b3-4f27-8139-4340c3875b16`):** ITERATE. One blocker (F1 compound-doc forward-pointer specificity) + six sharpenings (F2-S, F3-F7). All resolved in this revision.
- **Second-pass (same session, continuity preserved):** GROOMED. All seven findings resolved. Two remaining uncertainties dispositioned (AlwaysOn-inclusion is architecturally correct; multi-skill multi-naming YAGNI deferred). One residual: corrective message in R5 reordered to action-first / context-second per `webhook_zero_tools`' template shape — accept-set values listed before skill path + meta-doc citation, so the LLM reads the actionable constraint before the provenance.
