# Plan: Add general `assert_grounded` EndTurn guard (mika#1331)

**Ticket:** senara-solutions/mika#1331
**Type:** fix (EndTurn guard — WC1 anchor)
**Estimated size:** ~500 lines (detection + satisfaction + wiring + unit tests + eval scenarios)
**Structural precedent:** `asserted_unavailability` guard (#862, `agent.rs:1631-1683` + `5811-5880`)

## Problem

The EndTurn guard chain catches specific fabrication shapes — negative unavailability claims (#862), zero-tool fabricated actions (#308), completion keywords without state writes (#483) — but lets through **affirmative, ungrounded state claims**: "I checked the issue and it's groomed," "I reviewed the PR, no issues," "the handler already closed the task." These claims assert verifiable state about a referenced resource (`#N`, task ID) without the corresponding grounding tool call (`run_gh`, `check_task`) in the turn's `all_tool_summaries`. This is the structural anchor of wedge-class WC1 (mika-dev hallucination/fabrication).

## Design decisions

### D1: Inline guard, not registry entry

Mirror `asserted_unavailability`'s inline pattern (not in `INTENT_GUARDS` const array). Reasons:
- Checks **assistant text** (not user message), which registry `trigger` functions don't access
- Needs dynamic correction message with the extracted resource reference
- Needs `all_tool_summaries` for the satisfaction predicate

### D2: Two-layer detection (claim patterns + resource-ref extraction)

**Layer 1 — Affirmative state-claim patterns.** A small set of high-precision regex patterns that capture the affirmative-claim shape. These are the mirror of `asserted_unavailability`'s negative-claim patterns:

```
Pattern 1: "I (checked|confirmed|verified|reviewed|inspected|looked at) (the )?(issue|PR|pull request|task|ticket) #<N>"
Pattern 2: "I (checked|confirmed|verified|reviewed|inspected|looked at) (the )?(issue|PR|pull request|task|ticket) and (it's|it is|they're|they are) <state>"
Pattern 3: "(issue|PR|pull request|task|ticket) #<N> (is|was|has been) (groomed|merged|closed|completed|ready|approved|reviewed|open|blocked)"
Pattern 4: "(the handler|the callback|the subprocess|the dispatch) (already )?(closed|completed|merged|finished|resolved) (the )?(issue|PR|pull request|task|ticket)"
```

**Layer 2 — Resource-ref extraction.** From the matched text region, extract the resource reference:
- GitHub issue/PR number: `#(\d+)` — the most common dispatch-loop reference shape
- Task UUID: standard UUID pattern from tool arguments

**Two-layer false-positive filter** (mirrors `asserted_unavailability`'s capture-group + enabled-set pattern):
1. The regex capture constrains to specific claim verb + resource-type noun pairs (filters out casual mentions)
2. The satisfaction predicate checks `all_tool_summaries` for a grounding call referencing the same resource

### D3: Satisfaction predicate

The guard is **satisfied** (does NOT fire) when ANY of:

1. **Grounding tool called with matching resource.** A successful `run_gh` or `check_task` call exists in `all_tool_summaries` where the `input_summary` contains the extracted resource reference (issue/PR number or task ID). For `run_gh`: `input_summary` contains the `#N` number. For `check_task`: `input_summary` contains the task UUID.
2. **No claim detected.** Detection function returns `None` (no affirmative state-claim pattern matched).

Note: unlike `asserted_unavailability` which accepts *any* call attempt (success or failure), this guard requires `s.success == true` for the grounding call. The purpose is different: `asserted_unavailability` forces an attempt (real errors are signals); `assert_grounded` requires actual verification (a failed `run_gh` doesn't ground the claim).

### D4: Guard chain position

Position: immediately after `asserted_unavailability`, before `dispatch_arg_match` (#1313). Same `skip_remaining_guards` gate — skipped when `has_successful_pr_review()` is true (a turn that posted `gh pr review` completed its primary action; the review body itself is grounded evidence).

**Phase 0 Pin (required before implementation):** Run `grep -n "dispatch_arg_match\|asserted_unavailability\|ASSERT_GROUNDED" crates/mika-agent/src/agent.rs` to confirm the exact line range between the `asserted_unavailability` block end and the `dispatch_arg_match` block start. Verify no intervening guard logic exists that would affect guard-chain semantics. The insertion point is the blank line immediately after the `asserted_unavailability` block's closing `}` and before the next guard's `if !skip_remaining_guards` line. If intervening logic is found, adjust the insertion point to preserve existing guard ordering semantics (review-guide.md § Orthogonality — guard ordering is load-bearing).

### D5: Single-retry budget

Uses `intent_guard_retries` with label `"assert_grounded"`. Same single-retry semantics as `asserted_unavailability`: fires once per `run_loop` invocation, exhausted after first corrective re-prompt.

### D6: Correction message

Dynamic format with extracted resource reference:

```
[mika-engine] The previous response claimed state about <resource_type> <resource_ref>
without a grounding tool call this turn. Verifiable state claims require evidence:
call `run_gh` (for issues/PRs) or `check_task` (for tasks) to verify the state,
then report what the tool returned — or remove the unverified claim from your response.
```

### D7: v1 scope — PR-state and task-state claims only

Per ticket § Open questions resolved during grooming:
- **In scope:** PR state claims ("I reviewed PR #N", "PR #N is merged"), issue state claims ("I checked issue #N", "issue #N is groomed"), task state claims ("the handler closed the task")
- **Out of scope:** Issue-body content claims ("the issue body says X") — defer to follow-up

### D8: Interaction with existing guards

- `completion_claim` (#483) — catches `merged/deployed/completed/shipped` keywords without `update_task_status`. Complementary: `completion_claim` gates on **state-write tool** absence; `assert_grounded` gates on **state-read tool** absence. Both can fire in the same turn (different retry flags).
- `fabricated_action_claim` (#308) — catches action verbs + GitHub URLs with zero tool calls. `assert_grounded` fires even when *some* tools were called (but not the grounding tool for the specific resource). Complementary.
- `asserted_unavailability` (#862) — catches negative claims. `assert_grounded` catches affirmative claims. Mirror pair, no overlap.

## Implementation steps

### Step 1: Detection function (~80 lines)

**File:** `crates/mika-agent/src/agent.rs` (near `detect_asserted_unavailability` at ~line 5848)

Add:
- `const ASSERT_GROUNDED_LABEL: &str = "assert_grounded";`
- `static AFFIRMATIVE_STATE_CLAIM_PATTERNS: LazyLock<Vec<Regex>>` — four regex patterns from D2 above, each with named capture groups `(?P<resource_type>...)` and `(?P<resource_ref>...)` where applicable
- `static RESOURCE_REF_RE: LazyLock<Regex>` — `#(\d+)` extractor for GitHub issue/PR numbers in the claim vicinity
- `fn detect_affirmative_state_claim(text: &str) -> Option<AffirmativeStateClaim>` — scans text for patterns, extracts resource reference. Returns `None` on no match. **Crucially, also returns `None` when a pattern matches but no `resource_ref` can be extracted** (e.g., Pattern 2 "I confirmed the PR and it's merged" without a `#N`). This is the lean-narrow design: no resource ref → no grounding check possible → no guard fire. Fail-open on absent resource ref is consistent with OQ1's "lean narrow — a small set of high-precision affirmative-claim patterns" resolution (review-guide.md § KISS).

```rust
struct AffirmativeStateClaim {
    resource_type: &'static str,  // "issue", "PR", "task"
    resource_ref: String,          // "#500", task UUID — never empty (None returned instead)
    claim_text: String,            // the matched claim for logging
}
```

**False-positive mitigation:** The regex patterns are deliberately narrow:
- Require first-person agent voice ("I checked", "I confirmed") OR passive with resource ("PR #N is merged", "the handler closed")
- Require specific claim verbs (checked/confirmed/verified/reviewed/inspected for agent-voice; is/was/has been for passive)
- Require resource-type noun (issue/PR/pull request/task/ticket) adjacent to the claim verb
- Do NOT match: "this relates to the #500 groom" (no claim verb), "we discussed #500 yesterday" (no state assertion), "see #500 for details" (reference, not claim)

### Step 2: Satisfaction predicate (~30 lines)

**File:** `crates/mika-agent/src/agent.rs` (near `asserted_unavailability_satisfied` at ~line 5874)

Add:
- `fn assert_grounded_satisfied(claim: &AffirmativeStateClaim, summaries: &[ToolCallSummary]) -> bool`

Logic:
1. Extract the resource reference from the claim (e.g., `"500"` from `"#500"`)
2. Search `summaries` for a **successful** call to a grounding tool:
   - `run_gh` where `input_summary` contains the resource reference number
   - `check_task` where `input_summary` contains a task reference
   - `gh_read` where `input_summary` contains the resource reference number (mika-arch's read-only GitHub tool)
3. Return `true` if found, `false` if not

**Same-turn ordering is irrelevant.** `all_tool_summaries` is accumulated over the full turn before the EndTurn guard chain runs. A grounding call appearing after the claim text but before EndTurn in the same turn satisfies the predicate identically to a call before the claim text. This is by design — the guard checks presence, not sequence — and is consistent with `asserted_unavailability`'s accumulation model (review-guide.md § Single Responsibility).

**Grounding tool set:** `const GROUNDING_TOOLS: &[&str] = &["run_gh", "check_task", "gh_read"];`

### Step 3: Inline guard wiring (~40 lines)

**File:** `crates/mika-agent/src/agent.rs` (after the `asserted_unavailability` guard block at ~line 1683)

Insert a new guard block mirroring the `asserted_unavailability` pattern:

```rust
// #1331 — Assert-grounded guard. Catches affirmative state claims
// about referenced resources (issue/PR/task) without a grounding
// tool call (run_gh, check_task, gh_read) in the turn's tool-call
// trace. Single retry via intent_guard_retries. Mirror of
// asserted_unavailability's negative-claim detector.
if !skip_remaining_guards
    && matches!(response.stop_reason, LlmStopReason::EndTurn)
    && !intent_guard_retries.contains(ASSERT_GROUNDED_LABEL)
    && let Some(claim) = detect_affirmative_state_claim(&text)
    && !assert_grounded_satisfied(&claim, &all_tool_summaries)
{
    intent_guard_retries.insert(ASSERT_GROUNDED_LABEL);
    warn!(
        step,
        label = mode.label(),
        resource_type = claim.resource_type,
        resource_ref = %claim.resource_ref,
        claim = %claim.claim_text,
        intent_guard = ASSERT_GROUNDED_LABEL,
        "Assert-grounded guard fired — re-prompting"
    );
    // Push assistant response + corrective re-prompt
    request.messages.push(LlmMessage { ... });
    request.messages.push(LlmMessage {
        role: LlmRole::User,
        content: LlmContent::Text(format!(
            "[mika-engine] The previous response claimed state about \
             {} {} without a grounding tool call this turn. \
             Verifiable state claims require evidence: call `run_gh` \
             (for issues/PRs) or `check_task` (for tasks) to verify \
             the state, then report what the tool returned — or \
             remove the unverified claim from your response.",
            claim.resource_type, claim.resource_ref,
        )),
    });
    continue;
}
```

### Step 4: Unit tests for detection function (~140 lines)

**File:** `crates/mika-agent/src/agent.rs` (in `mod tests`, near `test_detect_asserted_unavailability_*` at ~line 9927)

Tests:

1. **Pattern 1 fires:** "I checked the issue #500 and it's groomed" → `Some(AffirmativeStateClaim { resource_type: "issue", resource_ref: "#500", ... })`
2. **Pattern 1 fires (PR):** "I reviewed PR #123 — no issues found" → detects PR #123
3. **Pattern 2 fires:** "I confirmed the PR and it's merged" → detects PR claim
4. **Pattern 3 fires:** "Issue #500 is groomed and ready for dispatch" → detects issue #500
5. **Pattern 3 fires (passive):** "PR #123 has been merged" → detects PR #123
6. **Pattern 4 fires (with task UUID in context):** "The handler already closed the task abc-123-..." where a task UUID is extractable from the surrounding text → detects task claim with `resource_ref`
7. **No match — casual reference:** "This relates to the #500 groom we did" → `None`
8. **No match — discussion:** "See #500 for details on the approach" → `None`
9. **No match — question:** "Is issue #500 groomed yet?" → `None`
10. **No match — negation:** "I haven't checked issue #500 yet" → `None`
11. **No match — Pattern 2 without resource ref:** "I confirmed the PR and it's merged" (no `#N` extractable) → `None` (F1: lean-narrow fail-open on absent resource ref)
12. **No match — Pattern 4 without resource ref:** "The handler already closed the task" with no task UUID in context → `None` (F1: same fail-open behavior for Pattern 4)

### Step 5: Unit tests for satisfaction predicate (~70 lines)

Tests:

1. **Satisfied — run_gh with matching ref:** claim `#500`, summaries contain `run_gh` with `input_summary` containing `"500"`, success=true → `true`
2. **Not satisfied — run_gh with different ref:** claim `#500`, summaries contain `run_gh` with `input_summary` containing `"123"` → `false`
3. **Not satisfied — run_gh failed:** claim `#500`, summaries contain `run_gh` with `"500"` but `success=false` → `false`
4. **Satisfied — check_task:** claim about task, summaries contain `check_task` success → `true`
5. **Satisfied — gh_read:** claim `#500`, summaries contain `gh_read` with `"500"` success → `true`
6. **Not satisfied — empty summaries:** → `false`
7. **Not satisfied — unrelated tools only:** summaries contain `search_memory`, `store_fact` → `false`
8. **Satisfied — grounding call after claim text in same turn:** claim `#500`, summaries contain `run_gh` with `"500"` success=true appended AFTER the claim-text response in the same turn → `true` (F3: confirms same-turn ordering irrelevance)

### Step 6: Eval grounding regression scenarios (~150 lines)

**Directory:** `crates/mika-agent/tests/eval/grounding_regressions/`

Add four new scenario files (per ticket § Behavior-test contract):

**Scenario 34:** `assert_grounded_pr_state_caught.rs`
- Agent claims "I reviewed PR #123, no issues" with no `run_gh` call → guard fires, one retry
- MockLlmProvider sequence: (1) text with PR state claim → guard fires, (2) tool_call `run_gh` with PR args → succeeds, (3) text with grounded response
- Hard assertions: `llm_call_count > 1`, `assert_tools_include(&trace, &["run_gh"])`

**Scenario 35:** `assert_grounded_pr_state_satisfied.rs`
- Agent calls `run_gh pr view #123`, then claims "PR #123 looks good" → guard satisfied, no retry
- MockLlmProvider sequence: (1) tool_call `run_gh` → result, (2) text with PR state claim (grounded)
- Hard assertion: `llm_call_count == 2` (no extra re-prompt), `assert_tools_include(&trace, &["run_gh"])`

**Scenario 36:** `assert_grounded_false_positive_guard.rs`
- Agent discusses past work: "this relates to the #500 groom we did yesterday" → no guard fire
- MockLlmProvider sequence: (1) text with casual reference to #500
- Hard assertion: `llm_call_count == 1` (no re-prompt)

**Scenario 37:** `assert_grounded_task_state_caught.rs`
- Agent claims "the handler already closed the task" with no `check_task` call → guard fires
- MockLlmProvider sequence: (1) text with task state claim → guard fires, (2) tool_call `check_task` → result, (3) grounded text
- Hard assertions: `llm_call_count > 1`, `assert_tools_include(&trace, &["check_task"])`

**Registration:** Add `pub mod` entries in `grounding_regressions/mod.rs`.

### Step 7: CLAUDE.md updates (~10 lines)

**File:** `crates/mika-agent/CLAUDE.md`

Update the EndTurn guard chain documentation:
- Add entry between 6c and 7: `6d. **Assert-grounded guard (#1331):** Inline guard that detects affirmative state claims about referenced resources (issue/PR/task #N) without a grounding tool call (run_gh, check_task, gh_read) in the turn. Single retry. Mirror of asserted_unavailability (negative → affirmative). Skipped behind skip_remaining_guards.`
- Update grounding regression scenario count (35 → 39)
- Add scenario descriptions in the grounding regressions section

**File:** `CLAUDE.md` (root)
- Update "10 post-condition guards on EndTurn" → 11

## File change summary

| File | Change type | ~Lines |
|------|------------|--------|
| `crates/mika-agent/src/agent.rs` | Add detection fn, satisfaction fn, label const, regex patterns, inline guard wiring | ~150 |
| `crates/mika-agent/src/agent.rs` (tests) | Unit tests for detection + satisfaction (12 detection + 8 satisfaction) | ~210 |
| `crates/mika-agent/tests/eval/grounding_regressions/assert_grounded_pr_state_caught.rs` | New eval scenario | ~50 |
| `crates/mika-agent/tests/eval/grounding_regressions/assert_grounded_pr_state_satisfied.rs` | New eval scenario | ~40 |
| `crates/mika-agent/tests/eval/grounding_regressions/assert_grounded_false_positive_guard.rs` | New eval scenario | ~35 |
| `crates/mika-agent/tests/eval/grounding_regressions/assert_grounded_task_state_caught.rs` | New eval scenario | ~50 |
| `crates/mika-agent/tests/eval/grounding_regressions/mod.rs` | Register new scenarios | ~4 |
| `crates/mika-agent/CLAUDE.md` | Document guard 6d, update counts | ~10 |
| `CLAUDE.md` | Update guard count | ~2 |
| **Total** | | **~551** |

## Risks and mitigations

| Risk | Mitigation |
|------|-----------|
| **False positives on legitimate discussion** | Narrow regex patterns require claim-verb + resource-type noun pairs; casual references ("see #500", "relates to #500") don't match. Eval scenario 36 pins this. |
| **Truncated input_summary misses resource ref** | `input_summary` is capped at 200 chars. For `run_gh` calls, the issue/PR number appears early in the args and will survive truncation. For `check_task`, the task UUID is the primary argument. Low risk. |
| **Guard fires on honest pilot with grounded claim in a different turn** | By design — per-turn grounding is the contract. Cross-turn claims should reference the prior turn's result or re-verify. Single-retry budget means the pilot gets one correction opportunity. |
| **Interaction with `skip_remaining_guards`** | Guard sits behind `!skip_remaining_guards` — skipped when PR review posted. Correct: a PR review submission IS grounding evidence. |
| **Stale `callback_state_claim_unverified.rs`** | `callback_state_claim_unverified.rs` (from the retired #1322 regex brake approach) is removed by #716's implementation. If #1331 ships before #716, confirm the file is deleted or ignored — it is **not** a prior implementation of this guard. It uses a different detection strategy (retired regex brake) that was explicitly superseded by this state-based approach (review-guide.md § Single Responsibility; #1327/#1328 fabrication brake retirement). |

## Testing strategy

1. **Unit tests** (Step 4-5): Exhaustive pattern matching and satisfaction predicate coverage. Run via `cargo test -p mika-agent -- assert_grounded`.
2. **Eval scenarios** (Step 6): Four grounding regression scenarios with frozen pre-fix fixtures. Run via `cargo test -p mika-agent --test eval -- grounding_regressions::assert_grounded`.
3. **Full test suite**: `cargo test -p mika-agent` — verify no regressions in existing 35 grounding scenarios.
4. **Clippy**: `cargo clippy -p mika-agent` — no new warnings.

## Revision history

- rev 2 (2026-05-29): addressed F1 by specifying explicit fail-open (return `None`) when Pattern 2/4 matches but no `resource_ref` is extractable — added unit tests 11-12 pinning this behavior; addressed F2 by adding Phase 0 Pin grep requirement to D4 with exact instructions for confirming the insertion point and verifying no intervening guard logic (review-guide.md § Orthogonality); addressed F3 by documenting same-turn ordering irrelevance in Step 2 and adding satisfaction predicate unit test 8 for grounding-call-after-claim ordering; addressed F4 by adding `callback_state_claim_unverified.rs` cross-reference to the Risks table noting it is a stale artifact from the retired #1322 regex brake, not a prior implementation of this guard.
