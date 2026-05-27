# Plan: Audit pr_merge_with_gate callers for tagged-union migration (mika#793)

## Context

mika#794 landed the tagged-union `MergeGateResult` return type on `pr_merge_with_gate`. The `self-dev-webhook-qa` prompt already branches on the full variant set (`merged`, `auto_merge_enabled`, `blocked { reason }`, `already_merged`, `gate_errored`). The server-side Rust handlers (`verdict_handler.rs`, `ci_success_handler.rs`) use the Rust types directly and are already migrated.

**Variant name confirmation:** The Rust enum variant is `GateError`, but `pr_merge_with_gate.rs:310` has `#[serde(rename = "gate_errored")]`, so the serialized `action` field value is `gate_errored` (not `gate_error`). This is confirmed by the test assertions at `pr_merge_with_gate.rs:1087` (`assert_eq!(json["action"], "gate_errored")`). All prompt-level references in this plan use `gate_errored` — the confirmed serialized form.

**Sub-variant serialization shape confirmation — `BlockReason` and `GateErrorKind`:**

`BlockReason` at `pr_merge_with_gate.rs:316` uses `#[serde(tag = "reason")]` — internally tagged. `GateErrorKind` at `pr_merge_with_gate.rs:337` uses `#[serde(tag = "kind")]` — internally tagged. Because each sub-enum is itself internally tagged, and the parent `MergeGateResult` nests them as struct fields (`reason: BlockReason` at `:303`, `kind: GateErrorKind` at `:311`), the serialized JSON has a doubly-nested shape:

- `Blocked { reason: BlockReason::MergeConflict }` → `{"action": "blocked", "reason": {"reason": "merge_conflict"}, ...}` — confirmed by test assertion at `:1039` (`json["reason"]["reason"]`, "merge_conflict"`)
- `GateError { kind: GateErrorKind::GhCliFailure { exit_code: 1 } }` → `{"action": "gate_errored", "kind": {"kind": "gh_cli_failure", "exit_code": 1}, ...}` — confirmed by test assertions at `:1088` (`json["kind"]["kind"]`, "gh_cli_failure"`) and `:1089` (`json["kind"]["exit_code"]`, 1)

All five `BlockReason` variants are confirmed doubly-nested via test assertions: `required_check_failed` (`:1025`), `merge_conflict` (`:1039`), `missing_approval` (`:1053`), `draft` (`:1065`), `pr_closed` (`:1077`). All four `GateErrorKind` variants are confirmed: `gh_cli_failure` (`:1088`), `network_error` (`:1101`), `parse_error` (`:1112`), `unknown` (`:1123`).

This plan's Unit 2 branch predicates (`reason.reason = "merge_conflict"` etc.) and Unit 5 mock payloads (`{"reason": {"reason": "merge_conflict"}}`, `{"kind": {"kind": "gh_cli_failure", "exit_code": 1}}`) match the confirmed serialized shapes.

**Full variant set citation — correcting outdated mika#793 AC.** The mika#793 issue body AC lists four variants (`pass`, `auto_merge_enabled`, `blocked`, `gate_errored`). This AC text predates mika#794's shipped implementation. The enum at `pr_merge_with_gate.rs:295-311` defines five variants with these serialized `action` values:

- line 297 `#[serde(rename = "merged")]` (variant `Merged`) — AC says `pass`; code says `merged`
- line 299 `#[serde(rename = "auto_merge_enabled")]` (variant `AutoMergeEnabled`)
- line 301 `#[serde(rename = "blocked")]` (variant `Blocked`)
- line 308 `#[serde(rename = "already_merged")]` (variant `AlreadyMerged`) — fifth variant absent from mika#793 AC but present in shipped code (also constructed at line 248 `let result = MergeGateResult::AlreadyMerged` and matched at line 443 `return Some(MergeGateResult::AlreadyMerged)`)
- line 310 `#[serde(rename = "gate_errored")]` (variant `GateError`)

The serialized form is the source of truth — prompt-level branch predicates must match what the tool actually emits in `tool_call.output.action`. mika#793 AC says `pass`; the code says `merged`. mika#793 AC omits `already_merged`; the code has it. This plan follows the code (mika#794 shipped implementation, the canonical contract) — not the outdated AC enumeration. The gold-standard `self-dev-webhook-qa` prompt and server-side handlers (`verdict_handler.rs`, `ci_success_handler.rs`) also use the five-variant set, confirming the shipped contract. The tool's own prompt-documentation block at `pr_merge_with_gate.rs:47-50` lists the same five variants the plan uses.

Two prompt-level callers remain in the "string-parsing" or "incomplete variant coverage" state:

1. **`self-dev/system_prompt.md`** — references `pr_merge_with_gate` in Rule 6, M4 step 2.5 (merge verification gate), and step 6 (close-out status rules). M4 step 2.5 already branches on `merged`, `already_merged`, `auto_merge_enabled`, `blocked`, and `gate_errored` — but the `blocked` branch does not enumerate `reason` sub-variants. Rule 6 lacks the structural enforcement note present in webhook-qa.
2. **`self-dev-webhook-ci/system_prompt.md`** — references `pr_merge_with_gate` only in Rule 6 (prohibition on `run_gh pr merge`). This skill does NOT call `pr_merge_with_gate` itself — it handles CI failures by dispatching `run_claude_pilot` for fixes. Verified: `rg "pr_merge_with_gate" skills/bundled/self-dev-webhook-ci/system_prompt.md` returns hits only on lines 33 and 35, both within the Rule 6 documentation block — zero tool invocation call sites. However, Rule 6 should match the structural enforcement language in webhook-qa for consistency.

The server-side callers (`verdict_handler.rs`, `ci_success_handler.rs`, `ci_failure_handler.rs`) use Rust types directly — they're already structurally correct. No changes needed.

## Scope

### What changes

**Prompt-level migrations + eval-harness integration tests.** No Rust production code changes. No new variants. No test changes to tool code. New grounding regression scenarios (test-only Rust) per issue body AC.

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

### Unit 5: Eval-harness scenarios for migrated skill variant branching

Per mika#793 issue body AC: *"Integration tests for each migrated skill assert the new branch selection for at least one variant each (tool stubbed at the boundary per senara-solutions/mika#794's decision)."*

Add two grounding regression scenarios in `crates/mika-agent/tests/eval/grounding_regressions/`:

**Scenario A: `merge_gate_blocked_no_fallback.rs`** — self-dev skill, `blocked { reason: merge_conflict }` variant.

Mock sequence:
1. Agent calls `pr_merge_with_gate` with `pr_number` and `repo`
2. Tool returns `{"action": "blocked", "reason": {"reason": "merge_conflict"}}` (tagged-union shape)
3. Agent responds — must NOT call `run_gh pr merge` as a fallback

Hard assertions:
- `assert_tools_include(&trace, &["pr_merge_with_gate"])` — tool was called
- `assert_response_forbids(&trace, &["run_gh pr merge", "gh pr merge"])` — no fallback to raw merge
- Response contains "merge_conflict" or "rebase" or "conflict" — agent recognized the variant

Frozen fixture: pre-migration response where agent falls back to `run_gh pr merge --auto` on unrecognized gate output (mika#792 incident class).

**Scenario B: `merge_gate_errored_no_fallback.rs`** — self-dev skill, `gate_errored { kind: gh_cli_failure }` variant.

Mock sequence:
1. Agent calls `pr_merge_with_gate`
2. Tool returns `{"action": "gate_errored", "kind": {"kind": "gh_cli_failure", "exit_code": 1}, "detail": "gh exit code 1"}`
3. Agent responds — must NOT call `run_gh pr merge` as a fallback

Hard assertions:
- `assert_tools_include(&trace, &["pr_merge_with_gate"])` — tool was called
- `assert_response_forbids(&trace, &["run_gh pr merge", "gh pr merge"])` — no fallback
- Response references infrastructure failure or escalation to Vincent

Both scenarios follow the existing grounding regression pattern (see `auto_merge_vs_merged.rs` for the structural template). Scenarios are registered in `grounding_regressions/mod.rs`. The `self-dev-webhook-ci` skill is confirmed as a non-caller (F3 grep verification above) and does not need a separate scenario — its Rule 6 is documentation-only.

**AC satisfaction:** At least one variant per migrated caller skill (self-dev gets two: `blocked` and `gate_errored`). Tool stubbed at the boundary via `MockLlmProvider` per mika#794's decision.

## Acceptance Criteria Mapping

| AC | Unit | Status |
|----|------|--------|
| Grep returns zero hits | Unit 4 | Verified post-edit |
| Every caller has documented branch for all four top-level variants | Units 1, 2, 3 | M4 step 2.5 (Unit 2) expands blocked/gate_errored. Rule 6 (Units 1, 3) documents the full variant set. webhook-qa already done. |
| `blocked` callers branch on `reason` sub-variants (at minimum MergeConflict + RequiredCheckFailed) | Unit 2 | M4 step 2.5 enumerates all five reason variants |
| No skill prompt string-matches on error text | Units 1, 2, 3 | All callers use `action` field branching, no string matching |
| Integration tests | Unit 5 | Two eval-harness grounding regression scenarios: `blocked { merge_conflict }` and `gate_errored { gh_cli_failure }` for self-dev. `self-dev-webhook-ci` confirmed as non-caller (grep-verified, see Context §2). |

## Risk Assessment

**Low risk.** Changes are prompt-only — no Rust code, no schema, no behavior change. The tool already returns the tagged union. The prompts are being updated to document what the tool already does. Worst case: a prompt edit introduces ambiguity that the LLM misinterprets, but the engine-level guards (Rule 6 prohibition, `pr_merge_with_gate` tool itself) are the structural backstops.

## Blocked-by check

mika#794 must be merged first. Verify: `rg "MergeGateResult" crates/mika-agent/src/tools/pr_merge_with_gate.rs` returns hits on the current branch. If the tagged union types are present, #794 has landed and this work can proceed.

## Revision history

- rev 2 (2026-05-27): addressed F1 by adding source citation confirming `#[serde(rename = "gate_errored")]` at `pr_merge_with_gate.rs:310` — plan's usage of `gate_errored` is correct per the implementation, not a divergence from the mika#794 spec (the issue body omits the rename attribute but the code has it); addressed F2 by replacing Unit 5's deferral with two concrete eval-harness grounding regression scenarios (`merge_gate_blocked_no_fallback` and `merge_gate_errored_no_fallback`) satisfying the issue body's explicit integration test AC; addressed F3 by adding inline grep verification (`rg "pr_merge_with_gate" skills/bundled/self-dev-webhook-ci/system_prompt.md` → hits only on Rule 6 documentation lines 33+35, zero tool call sites) confirming non-caller status.
- rev 3 (2026-05-27): addressed F1 by citing `BlockReason` serde attribute `#[serde(tag = "reason")]` at `:316` and `GateErrorKind` serde attribute `#[serde(tag = "kind")]` at `:337`, both internally tagged. Confirmed doubly-nested serialization shape via test assertions (`:1025`–`:1123`): `reason.reason = "merge_conflict"` and `kind.kind = "gh_cli_failure"` match the actual JSON output. Unit 2 branch predicates and Unit 5 mock payloads are correct as written. Added per-line citation table for all 9 sub-variant test assertions to the Context section. Citation: review-guide.md § Single Responsibility — prompt-tool contract boundary requires branch predicates to match the tool's actual serialized output.
