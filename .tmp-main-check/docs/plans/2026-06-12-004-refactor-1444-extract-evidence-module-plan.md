# Plan: Extract `evidence/` into its own module (mika#1444)

**Parent:** mika#1259 — Layer 3 domain refactor (operational-partner project)
**Type:** refactor (pure module split, no behavior change)
**Foundation ref:** `docs/architecture/operational-partner-frame.md` §6

## Context

`agent.rs` is 11,740 lines. Per Foundation §6, the `evidence/` module owns:
- Grounding-rule enforcement
- Fabrication-guard predicates
- Tool-call audit trail

These predicates are scattered across `agent.rs` lines ~5300–6600 (detection functions) and ~1450–1830 (guard integration in the EndTurn chain). The detection logic is pure — it takes text and tool-call summaries, returns verdicts. The guard *integration* (retry state, loop re-prompt) is agent-loop machinery and stays in `agent.rs`.

## Scope decision: predicates move, loop integration stays

The evidence module owns the **detection predicates** — the functions that answer "is this text making an ungrounded claim?" The agent loop owns the **enforcement** — what to do when a predicate fires (retry tracking, re-prompt injection, guard ordering). This split matches Foundation §6's language: "Owns the predicates from mika#1254."

`ToolCallSummary` is used by the agent loop for many purposes beyond evidence (history building, metadata serialization, step-exceeded fallback). It stays in `agent.rs` and the evidence module imports it. This avoids a circular dependency where agent.rs depends on evidence/ for a core loop type.

## Module structure

```
crates/mika-agent/src/evidence/
├── mod.rs              # Module root — doc comment, re-exports
├── grounding.rs        # Assert-grounded predicates (#1331)
├── fabrication.rs      # Fabrication-guard predicates (#308, #1133, #862, #894, #1207, #1313)
└── audit.rs            # Tool-call metadata serialization
```

## Step 1: Create `evidence/mod.rs`

Create `crates/mika-agent/src/evidence/mod.rs` with:
- One-paragraph doc-comment naming the operational responsibility (per parent AC4)
- `pub mod grounding;`
- `pub mod fabrication;`
- `pub mod audit;`

## Step 2: Create `evidence/grounding.rs`

Move from `agent.rs` (lines ~6426–6565):

| Item | Current location | Type |
|------|-----------------|------|
| `ASSERT_GROUNDED_LABEL` | agent.rs:6426 | `pub(crate) const` |
| `GROUNDING_TOOLS` | agent.rs:6429 | `pub(crate) const` |
| `AffirmativeStateClaim` | agent.rs:6432–6436 | `pub(crate) struct` |
| `AFFIRMATIVE_STATE_CLAIM_PATTERNS` | agent.rs:6440–6465 | `static LazyLock` |
| `detect_affirmative_state_claim()` | agent.rs:6483–6549 | `pub(crate) fn` |
| `assert_grounded_satisfied()` | agent.rs:6558–6565 | `pub(crate) fn` |

**Imports needed:** `regex`, `std::sync::LazyLock`, `crate::agent::ToolCallSummary`.

**Tests to move** (lines ~11306–11403): All `test_detect_affirmative_state_claim_*` tests move to `#[cfg(test)] mod tests` inside `grounding.rs`.

## Step 3: Create `evidence/fabrication.rs`

Move from `agent.rs`:

| Item | Current location | Type |
|------|-----------------|------|
| `VERDICT_PRODUCER_SKILLS` | agent.rs:44 | `pub(crate) const` |
| `has_verdict_producer_skill()` | agent.rs:47–51 | `pub(crate) fn` |
| `COMPLETION_CLAIM_RE` | agent.rs:~5330 | `static LazyLock` |
| `detect_completion_claim()` | agent.rs:5347–5358 | `pub(crate) fn` |
| `MILESTONE_CLOSE_CLAIM_RE` | agent.rs:~5470 | `static LazyLock` |
| `MILESTONE_API_PATH_RE` | agent.rs:~5510 | `static LazyLock` |
| `extract_claimed_milestone_number()` | agent.rs:~5480 | `fn` (private) |
| `parse_run_gh_milestone_close_argv()` | agent.rs:~5520 | `fn` (private) |
| `detect_milestone_close_claim_without_patch()` | agent.rs:5551–5620 | `pub(crate) fn` |
| `GITHUB_RESOURCE_URL_RE` | agent.rs:5624 | `static LazyLock` |
| `ACTION_CLAIM_RE` | agent.rs:5636 | `static LazyLock` |
| `detect_fabricated_action_claim()` | agent.rs:5646–5654 | `pub(crate) fn` |
| `CALLBACK_STATE_CLAIM_RE` | agent.rs:5659 | `static LazyLock` |
| callback state claim detection fn | agent.rs:~5668 | `pub(crate) fn` |
| `ASSERTED_UNAVAILABILITY_PATTERNS` | agent.rs:6353–6393 | `static LazyLock` |
| `asserted_unavailability_satisfied()` | agent.rs:6411–6417 | `pub(crate) fn` |
| `detect_prose_style_tool_call()` | agent.rs:5810 | `pub(crate) fn` |
| Dispatch-arg fabrication detection | agent.rs:~1786 area helpers | `pub(crate) fn` if extractable |

**Note on `evaluate_completion_claim()`:** This async fn (agent.rs:~5364) takes `AsyncDatabase` + `ToolRegistry` — it's an *evaluator* that queries active tasks, not a pure predicate. It straddles the boundary. Two options:

- **Option A (recommended):** Move it to `evidence/fabrication.rs`. The DB query is read-only and the function's purpose is claim-validation — evidence domain. The agent loop calls it like any other predicate. Import `AsyncDatabase` and `ToolRegistry` as dependencies.
- **Option B:** Keep it in `agent.rs` or `post_condition.rs`. The DB dependency makes it "not pure." But this fragments the completion-claim logic across modules.

Decision: **Option A.** The function's *purpose* is evidence (is this claim fabricated?), even though it needs DB context. Purity is not the boundary criterion; operational responsibility is.

**Imports needed:** `regex`, `std::sync::LazyLock`, `std::collections::HashSet`, `crate::agent::ToolCallSummary`, `crate::skills::index::SkillEntry`, `crate::async_db::AsyncDatabase`, `crate::tools::ToolRegistry`.

**Tests to move:** All `test_detect_fabricated_action_claim_*`, `test_detect_completion_claim_*`, `test_detect_prose_style_tool_call_*`, and asserted-unavailability tests move to `#[cfg(test)] mod tests` inside `fabrication.rs`.

## Step 4: Create `evidence/audit.rs`

Move from `agent.rs` (lines ~263–350):

| Item | Current location | Type |
|------|-----------------|------|
| `has_non_zero_exit_prefix()` | agent.rs:278–285 | `pub(crate) fn` |
| `truncate_summary()` | agent.rs:289–301 | `pub(crate) fn` |
| `tool_calls_metadata_json()` | agent.rs:309–350+ | `pub fn` (currently pub) |
| `format_step_exceeded_fallback()` | agent.rs:417+ | `pub(crate) fn` |

**Note:** `tool_calls_metadata_json` is `pub` — it's used by `post_condition.rs` via `crate::agent::tool_calls_metadata_json`. After the move, the import path changes to `crate::evidence::audit::tool_calls_metadata_json`. Add a re-export in `evidence/mod.rs` for convenience.

**Note:** `ToolCallSummary` itself does NOT move — it stays in `agent.rs`. The audit functions take `&[ToolCallSummary]` as parameters. This keeps the type close to its primary construction site (the agent loop's `process_tool_calls`).

**Imports needed:** `tracing::warn`, `serde_json`, `crate::agent::ToolCallSummary`, `crate::planning::policy::TOOL_METADATA_MAX`.

**Tests to move:** Any tests for `truncate_summary`, `has_non_zero_exit_prefix`, `tool_calls_metadata_json` move to `audit.rs`.

## Step 5: Update `lib.rs`

Add `pub mod evidence;` to `crates/mika-agent/src/lib.rs` (alphabetically between `db` and `github_graphql`).

## Step 6: Update import paths in `agent.rs`

Replace all moved items with imports from the new module:

```rust
use crate::evidence::grounding::{
    detect_affirmative_state_claim, assert_grounded_satisfied,
    ASSERT_GROUNDED_LABEL, GROUNDING_TOOLS,
};
use crate::evidence::fabrication::{
    detect_fabricated_action_claim, detect_completion_claim,
    detect_milestone_close_claim_without_patch, asserted_unavailability_satisfied,
    has_verdict_producer_skill, detect_prose_style_tool_call,
    evaluate_completion_claim, VERDICT_PRODUCER_SKILLS,
};
use crate::evidence::audit::{
    tool_calls_metadata_json, has_non_zero_exit_prefix,
    truncate_summary, format_step_exceeded_fallback,
};
```

## Step 7: Update import paths in `post_condition.rs`

`post_condition.rs` imports `crate::agent::ToolCallSummary` — this stays unchanged (ToolCallSummary doesn't move).

If `post_condition.rs` references any moved functions, update those imports.

## Step 8: Update import paths in test files

`tests/eval/grounding_regressions/` tests import from `mika_agent::agent::*`. Update any imports that reference moved items to use `mika_agent::evidence::*`.

Check `tests/eval/grounding_assertions/mod.rs` for any direct references to moved functions.

## Step 9: Verify

- `cargo test -p mika-agent` passes (AC2)
- `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean (AC5)
- No behavior change — same functions, new locations (AC6)

## What does NOT move

| Item | Why it stays |
|------|-------------|
| `ToolCallSummary` struct | Core loop type used everywhere — moving creates awkward dependency direction |
| Guard retry state flags (`fabricated_action_retry_done`, etc.) | Agent-loop enforcement state, not detection logic |
| Guard integration code in EndTurn chain (lines ~1450–1830) | Loop machinery — calls predicates but owns retry/re-prompt |
| `process_tool_calls()` fn | Tool execution orchestration — evidence module doesn't own execution |
| `save_tool_call()` in db.rs | `Database` impl block method — splitting impl blocks across modules is invasive; the DB method is the persistence layer, not the evidence layer |
| `EvidenceRef` / `EvidenceRefKind` in `operational/types.rs` | Already properly placed in the operational module per Foundation §6's write-path inventory |
| `IntentPrecondition` / `INTENT_GUARDS` registry | Guard framework, not individual predicates |

## Risk assessment

**Low risk.** This is a pure module split — functions move verbatim, imports update. No logic changes, no new abstractions, no signature changes.

**One subtlety:** Some moved functions are `fn` (private) — they become `pub(crate)` when crossing module boundaries. This is intentional visibility widening required by the split. Verify no accidental `pub` (externally visible) items are created.

## Estimated scope

- ~900 lines of detection logic + ~300 lines of tests move out of `agent.rs`
- ~50 lines of new module scaffolding (`mod.rs` + file headers)
- ~20 lines of import updates in `agent.rs`, `post_condition.rs`, and test files
- Net effect: `agent.rs` shrinks by ~1,200 lines (from 11,740 to ~10,540)
