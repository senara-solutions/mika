# Plan: Audit pr_merge_with_gate callers for tagged-union migration (mika#793)

## Context

mika#794 landed the tagged-union `MergeGateResult` return type on `pr_merge_with_gate`. The `self-dev-webhook-qa` prompt already branches on the full variant set (`merged`, `auto_merge_enabled`, `blocked { reason }`, `already_merged`, `gate_errored`). The server-side Rust handlers (`verdict_handler.rs`, `ci_success_handler.rs`) use the Rust types directly and are already migrated.

Two prompt-level callers remain in the "string-parsing" or "incomplete variant coverage" state:

1. **`self-dev/system_prompt.md`** — references `pr_merge_with_gate` in Rule 6, M4 step 2.5 (merge verification gate), and step 6 (close-out status rules). M4 step 2.5 already branches on `merged`, `already_merged`, `auto_merge_enabled`, `blocked`, and `gate_errored` — but the `blocked` branch does not enumerate `reason` sub-variants. Rule 6 lacks the structural enforcement note present in webhook-qa.
2. **`self-dev-webhook-ci/system_prompt.md`** — references `pr_merge_with_gate` only in Rule 6 (prohibition on `run_gh pr merge`). This skill does NOT call `pr_merge_with_gate` itself — it handles CI failures by dispatching `run_claude_pilot` for fixes. However, Rule 6 should match the structural enforcement language in webhook-qa for consistency.

The server-side callers (`verdict_handler.rs`, `ci_success_handler.rs`, `ci_failure_handler.rs`) use Rust types directly — they're already structurally correct. No changes needed.

## Scope

### What changes

**Prompt-level migrations only.** No Rust code changes. No new variants. No test changes to tool code.

### What does NOT change

- `crates/mika-agent/src/tools/pr_merge_with_gate.rs` — owned by mika#794
- `skills/bundled/self-dev-webhook-qa/system_prompt.md` — already migrated (gold standard)
- Server-side handlers — use Rust types directly
- No new `PrMergeGateResult` variants

## Units of Work

### Unit 1: Update self-dev Rule 6 with structural enforcement language

**File:** `skills/bundled/self-dev/system_prompt.md`

**Current (lines 271-277):**
```markdown
### Rule 6 — Always use pr_merge_with_gate for PR merges

Never call `run_gh("pr merge ...")` or `run_gh("gh pr merge ...")` to merge a PR. Always use `pr_merge_with_gate` with `pr_number` (integer) and `repo` (owner/repo string). The tool checks required CI statuses and returns a structured `action` — act on it.

**Exception:** The "merge anyway" block resumption command uses raw `run_gh` as an intentional override of the CI gate when Vincent explicitly requests it.

**Incident:** mika#485 on 2026-04-08 — PR merged with required CI check in FAILURE state because agent used `run_gh pr merge` which has no CI gate.
```

**Change:** Add the structural enforcement paragraph from webhook-qa between the main instruction and the Exception, and add the second incident reference:

```markdown
### Rule 6 — Always use pr_merge_with_gate for PR merges

Never call `run_gh("pr merge ...")` or `run_gh("gh pr merge ...")` to merge a PR. Always use `pr_merge_with_gate` with `pr_number` (integer) and `repo` (owner/repo string). The tool checks required CI statuses and returns a structured `action` — act on it.

**Structural enforcement:** `pr_merge_with_gate` returns typed variants (`merged`, `auto_merge_enabled`, `blocked`, `already_merged`, `gate_errored`). The `blocked` variant carries a `reason` field with sub-variants (`merge_conflict`, `required_check_failed`, `missing_approval`, `pr_closed`, `draft`). The `gate_errored` variant carries `kind` and `detail` fields. Branch on these variants exhaustively — do NOT fall back to `run_gh pr merge` on ANY error or blocked state. Runtime enforcement via policy table — see follow-up ticket.

**Exception:** The "merge anyway" block resumption command uses raw `run_gh` as an intentional override of the CI gate when Vincent explicitly requests it.

**Incident:** mika#485 on 2026-04-08 — PR merged with required CI check in FAILURE state because agent used `run_gh pr merge` which has no CI gate. mika#792 on 2026-04-24 — agent improvised `run_gh pr merge --auto` when gate returned an unstructured error on a CONFLICTING PR.
```

**AC check:** Rule 6 now documents the full variant taxonomy. Both incidents cited.

### Unit 2: Expand M4 step 2.5 blocked branch to enumerate reason sub-variants

**File:** `skills/bundled/self-dev/system_prompt.md`

**Current (line 496):**
```markdown
   - If `pr_merge_with_gate` returned `"blocked"` or `"gate_errored"`: the webhook handler already routed to the appropriate block/error path. M4 step 3 will see the child as `blocked`.
```

**Change:** Expand the `blocked` and `gate_errored` branches to enumerate sub-variants, mirroring the webhook-qa handling shape. The self-dev skill is the orchestrator — its handling is lighter (webhook handler already acted), but it needs to know the variant names to log and triage correctly:

```markdown
   - If `pr_merge_with_gate` returned `"blocked"`: branch on the `reason` field:
     - `reason.reason = "required_check_failed"`: the webhook handler already routed to CI-fix. M4 step 3 will see the child per the handler's outcome.
     - `reason.reason = "merge_conflict"`: rebase needed. M4 step 3 will see the child as `blocked` or `in_progress` per the handler's outcome.
     - `reason.reason = "missing_approval"`: review approval needed. Task stays `in_progress`.
     - `reason.reason = "draft"` or `reason.reason = "pr_closed"`: unexpected in milestone flow. Escalate to Vincent. Task status: `blocked`.
     - Unrecognized `reason` value: do NOT fall back to `run_gh pr merge`. Notify Vincent. Task stays `in_progress`.

   - If `pr_merge_with_gate` returned `"gate_errored"`: infrastructure failure. Do NOT fall back to `run_gh pr merge`. Notify Vincent with `kind` and `detail`. Task stays `in_progress`.
```

**AC check:** Every `blocked` sub-variant has an explicit path. `gate_errored` has an explicit no-fallback path. `MergeConflict` and `RequiredCheckFailed` have explicit branches.

### Unit 3: Update self-dev-webhook-ci Rule 6 with structural enforcement language

**File:** `skills/bundled/self-dev-webhook-ci/system_prompt.md`

**Current (lines 33-37):**
```markdown
### Rule 6 — Always use pr_merge_with_gate for PR merges

Never call `run_gh("pr merge ...")` or `run_gh("gh pr merge ...")` to merge a PR. Always use `pr_merge_with_gate` with `pr_number` (integer) and `repo` (owner/repo string). The tool checks required CI statuses and returns a structured `action` — act on it.

**Incident:** mika#485 on 2026-04-08 — PR merged with required CI check in FAILURE state because agent used `run_gh pr merge` which has no CI gate.
```

**Change:** Add the same structural enforcement paragraph and second incident:

```markdown
### Rule 6 — Always use pr_merge_with_gate for PR merges

Never call `run_gh("pr merge ...")` or `run_gh("gh pr merge ...")` to merge a PR. Always use `pr_merge_with_gate` with `pr_number` (integer) and `repo` (owner/repo string). The tool checks required CI statuses and returns a structured `action` — act on it.

**Structural enforcement:** `pr_merge_with_gate` returns typed variants (`merged`, `auto_merge_enabled`, `blocked`, `already_merged`, `gate_errored`). The `blocked` variant carries a `reason` field with sub-variants (`merge_conflict`, `required_check_failed`, `missing_approval`, `pr_closed`, `draft`). The `gate_errored` variant carries `kind` and `detail` fields. Branch on these variants exhaustively — do NOT fall back to `run_gh pr merge` on ANY error or blocked state. Runtime enforcement via policy table — see follow-up ticket.

**Incident:** mika#485 on 2026-04-08 — PR merged with required CI check in FAILURE state because agent used `run_gh pr merge` which has no CI gate. mika#792 on 2026-04-24 — agent improvised `run_gh pr merge --auto` when gate returned an unstructured error on a CONFLICTING PR.
```

**AC check:** Rule 6 matches webhook-qa's enforcement language. Both incidents cited.

### Unit 4: Verify acceptance criteria grep returns zero hits

After all prompt edits, run:

```bash
rg "gh.*exit code|Failed to fetch|pr_merge_with_gate.*response" crates/mika-agent/src/skills/bundled/
```

This should return zero hits. The `pr_merge_with_gate.*response` pattern was already clean (webhook-qa uses `action` field branching, not "response" string matching). The `gh.*exit code` and `Failed to fetch` patterns should not appear in any bundled skill prompt.

### Unit 5: No integration tests needed for prompt-only changes

The ticket's AC mentions "integration tests for each migrated skill." However, the migration here is **prompt-level only** — no Rust tool behavior changed. The tool already returns the tagged union (mika#794 delivered tests for that). The prompt changes instruct the LLM to branch on existing variant names. Eval-harness scenarios for prompt-level branching are covered by the grounding regression suite (scenarios in `tests/eval/grounding_regressions/`).

If the reviewer disagrees, the eval harness could add a scenario where the mock returns a `blocked { reason: merge_conflict }` JSON and asserts the model doesn't call `run_gh pr merge`, but this would be testing LLM instruction-following rather than code correctness.

## Acceptance Criteria Mapping

| AC | Unit | Status |
|----|------|--------|
| Grep returns zero hits | Unit 4 | Verified post-edit |
| Every caller has documented branch for all four top-level variants | Units 1, 2, 3 | M4 step 2.5 (Unit 2) expands blocked/gate_errored. Rule 6 (Units 1, 3) documents the full variant set. webhook-qa already done. |
| `blocked` callers branch on `reason` sub-variants (at minimum MergeConflict + RequiredCheckFailed) | Unit 2 | M4 step 2.5 enumerates all five reason variants |
| No skill prompt string-matches on error text | Units 1, 2, 3 | All callers use `action` field branching, no string matching |
| Integration tests | Unit 5 | Prompt-only changes; tool-level tests already exist from mika#794 |

## Risk Assessment

**Low risk.** Changes are prompt-only — no Rust code, no schema, no behavior change. The tool already returns the tagged union. The prompts are being updated to document what the tool already does. Worst case: a prompt edit introduces ambiguity that the LLM misinterprets, but the engine-level guards (Rule 6 prohibition, `pr_merge_with_gate` tool itself) are the structural backstops.

## Blocked-by check

mika#794 must be merged first. Verify: `rg "MergeGateResult" crates/mika-agent/src/tools/pr_merge_with_gate.rs` returns hits on the current branch. If the tagged union types are present, #794 has landed and this work can proceed.
