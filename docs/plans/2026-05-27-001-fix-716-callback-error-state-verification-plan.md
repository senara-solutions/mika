# Plan: fix mika-dev fabricates state claims on callback errors (#716)

## Problem

When a claude-pilot callback handler returns an error/failure signal, mika-dev's LLM fabricates a narrative about downstream state (PR status, issue close reason, CI outcome) without calling any verification tool. The existing post-condition guards don't catch this because:

- **Guard #4 (completion-claim):** doesn't fire — the LLM claims *failure*, not completion
- **Guard #5 (fabricated-action-claim):** doesn't fire — the LLM calls tools (update_task_status, send_message) so `tools_called` is non-empty
- **Grounding rule (system prompt):** instructs but doesn't enforce during callback turns

The self-dev-callback skill prompt already handles the *success* path well (extracting metadata, discovering PRs via `run_gh`). But the *failure* paths (Step 4.5 and the "On failure" handler) don't mandate GitHub state verification before the LLM describes what happened. The LLM sees "callback failed" and rationalizes a story.

## Root Cause Analysis

The issue is in two layers:

1. **Engine layer (`format_callback_framing` / `build_callback_trigger_context`):** The callback framing tells the LLM the result is untrusted and not to extrapolate — but this is a prompt instruction, not structural enforcement. The LLM ignores it when rationalizing error results.

2. **Skill layer (`self-dev-callback/system_prompt.md`):** The failure paths instruct the agent to read logs, classify the error, and decide whether to retry. But they don't require the agent to first verify the *actual outcome* — whether a PR exists, whether the issue was closed, what the branch state is. The success path does this (Step 2: "If `pr_url` not in callback text, discover via `run_gh`") but the failure path skips it.

## Proposed Fix — Two Layers

### Layer 1: Structural — Callback-error verification post-condition guard (engine)

Add a new post-condition guard (position 4c, after completion-claim and before fabricated-action-claim) that fires on callback turns when the callback result indicates failure. This guard detects when the LLM makes GitHub state claims (PR status, issue close reason) on callback turns without having called `run_gh` or `check_task` to verify.

**Why a guard, not just prompt reinforcement:** Per `feedback_prompt_enforcement_fragile.md`, prompt-only enforcement degrades under load. The #716 incident proves the existing "untrusted" + "do not infer" instructions in `format_callback_framing` are insufficient. A structural guard catches the fabrication and forces a retry with verification instruction.

**Guard design:**

- **Trigger:** `is_callback_turn == true` AND assistant text contains GitHub state claims (regex: downstream state keywords like "no PR", "manually closed", "without PR", "issue closed without", "no commits", "handler crashed" when combined with state assertions)
- **Satisfaction:** `tools_called` contains `run_gh` OR `check_task` (indicating the agent verified state)
- **Action:** Single retry with correction message instructing the agent to verify via `run_gh` before describing the outcome
- **Scope:** Silent mode callback turns only (conversation mode already has guards #4/#5)

**Detection regex patterns** (for `detect_unverified_callback_state_claim`):

```
"no PR"
"without PR"
"manually closed"
"closed without"
"no commits"
"handler crashed"
"no branch"
```

These patterns match the specific fabrication class observed in #716. The regex is conservative — it targets callback-specific fabrication vocabulary, not general state claims.

### Layer 2: Prompt reinforcement — Self-dev-callback failure path (skill)

Strengthen the self-dev-callback system prompt's failure handlers to mandate GitHub state verification *before* any diagnosis or reporting:

1. **"On failure" handler (Step 4.5 preamble):** Add a mandatory GitHub verification step before log reading:
   ```
   **MANDATORY STATE CHECK (before any failure diagnosis):**
   1. Call `run_gh("pr list --head <branch> --repo senara-solutions/<repo> --json url,number,state,reviewDecision")` 
   2. Call `run_gh("issue view <n> --repo senara-solutions/<repo> --json state,stateReason")`
   3. If a PR exists (especially if merged/approved), the run SUCCEEDED — do NOT enter failure diagnosis.
   ```

2. **"On pipeline failure" handler:** Same mandatory check before retry logic.

3. **Callback framing reinforcement (`build_callback_trigger_context`):** Add callback-error-specific instruction when `failed == true`:
   ```
   The callback reported a FAILURE. Before describing what happened, you MUST call run_gh 
   to verify the actual state of the issue and any PRs. Do not claim "no PR", "manually 
   closed", or "handler crashed" without tool verification. The callback error may not 
   reflect the actual outcome — the work may have succeeded despite the handler error.
   ```

## Implementation Steps

### Step 1: Add `detect_unverified_callback_state_claim` detection function

**File:** `crates/mika-agent/src/agent.rs` (near existing detection functions ~line 4947)

Add a new detection function:

```rust
/// Detects when callback-turn assistant text claims downstream GitHub state
/// (PR status, issue close reason) without verification. Returns the matched
/// claim fragment if found.
///
/// Only meaningful when checked against `tools_called` — the guard fires
/// when this returns `Some` AND neither `run_gh` nor `check_task` was called.
fn detect_unverified_callback_state_claim(text: &str) -> Option<&str> {
    // Fast path: lowercase scan for common fragments
    let lower = text.to_lowercase();
    
    static CALLBACK_STATE_CLAIM_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)\b(no\s+PR|without\s+PR|manually\s+closed|closed\s+without|no\s+commits?|handler\s+crashed|issue\s+was\s+.*closed|no\s+branch)\b"
        ).unwrap()
    });
    
    // Only fire if at least one fast-path fragment matches
    let has_candidate = lower.contains("no pr")
        || lower.contains("without pr")
        || lower.contains("manually closed")
        || lower.contains("closed without")
        || lower.contains("no commit")
        || lower.contains("handler crashed")
        || lower.contains("no branch");
    
    if !has_candidate {
        return None;
    }
    
    CALLBACK_STATE_CLAIM_RE
        .find(&text)
        .map(|m| m.as_str())
}
```

### Step 2: Wire the guard into the post-condition chain

**File:** `crates/mika-agent/src/agent.rs` (in the EndTurn post-condition chain, after guard #4 at ~line 1340, before guard #5 at ~line 1412)

Add guard 4c:

```rust
// #716 — Callback error state-claim guard (position 4c). Detects when a
// callback-turn response claims downstream GitHub state (PR status, issue
// close reason) without calling run_gh or check_task to verify. The LLM
// rationalizes error signals into fabricated narratives; this guard forces
// verification.
if !skip_remaining_guards
    && matches!(response.stop_reason, LlmStopReason::EndTurn)
    && !callback_state_claim_retry_done
    && params.is_callback_turn
    && let Some(claim) = detect_unverified_callback_state_claim(&text)
{
    let has_verification = tools_called.contains("run_gh")
        || tools_called.contains("check_task")
        || tools_called.contains("gh_read");
    
    if !has_verification {
        callback_state_claim_retry_done = true;
        warn!(
            step,
            claim,
            label = mode.label(),
            "Callback state claim detected without verification tool — re-prompting"
        );
        // Push assistant response + correction
        request.messages.push(LlmMessage {
            role: LlmRole::Assistant,
            content: LlmContent::Blocks(
                mika_common::llm::response_content_to_blocks(&response.content),
            ),
        });
        request.messages.push(LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(format!(
                "[mika-engine] The previous response claimed \"{claim}\" on a callback \
                 turn without calling run_gh or check_task to verify. Callback errors \
                 do not reliably reflect actual outcomes — the work may have succeeded \
                 despite the handler error. Before describing what happened, call \
                 run_gh to verify the actual state of the issue and any associated PRs. \
                 Then describe the VERIFIED state.",
            )),
        });
        continue;
    }
}
```

Add `callback_state_claim_retry_done: bool` to the retry-tracking flags at the top of `run_loop` (alongside `completion_claim_retry_done`, `fabricated_action_retry_done`, etc.).

### Step 3: Enhance `build_callback_trigger_context` for failed callbacks

**File:** `crates/mika-agent/src/agent.rs` (function at line 99)

When `failed == true`, append a verification mandate to the callback framing:

```rust
pub fn build_callback_trigger_context(
    label: &str,
    task_id: &str,
    parent_task_id: Option<&str>,
    result: &str,
    failed: bool,
) -> String {
    let base = format_callback_framing(label, task_id, parent_task_id, result, failed);
    
    let failure_verification = if failed {
        "\n\nIMPORTANT: This callback reported a FAILURE. Before describing what happened, \
         you MUST call run_gh to verify the actual state of the referenced issue and any \
         associated PRs. Do not claim 'no PR', 'manually closed', 'handler crashed', or \
         any other downstream state without tool verification. The callback error may not \
         reflect the actual outcome — work may have succeeded despite the handler error.\n"
    } else {
        ""
    };
    
    format!(
        "{base}\n\
         IMPORTANT: A successful result confirms only the specific action performed. \
         NEVER extrapolate to downstream states (PR status, CI health, deploy readiness) \
         that the result does not explicitly mention.\n{failure_verification}\n\
         Follow the workflow defined by your active skills for this callback type. \
         ..."  // rest unchanged
    )
}
```

### Step 4: Strengthen self-dev-callback failure path

**File:** `skills/bundled/self-dev-callback/system_prompt.md`

Add a mandatory state-verification preamble to the failure handlers:

**Before "On failure" (line ~99):**

```markdown
**On failure (non-zero exit, "FAILED", or "not structured JSON"):** Before blocking, **always check for a PR** via `run_gh("pr list --head <branch> --json url,number,state,reviewDecision")`. If a PR exists (especially if mika-qa-approved), the run succeeded — merge and close out normally. Only proceed to Step 4.5 if no PR exists.
```

This line already exists. The issue is Step 4.5 itself and the general error narrative. Add to the start of Step 4.5:

```markdown
> **MANDATORY STATE VERIFICATION (before ANY failure diagnosis or reporting):**
> Before reading logs, classifying errors, or describing what happened:
> 1. Extract `reference_url` from `check_task(task_id)` — parse `senara-solutions/<repo>/issues/<n>`.
> 2. Call `run_gh("issue view <n> --repo senara-solutions/<repo> --json state,stateReason,closedAt")`.
> 3. Call `run_gh("pr list --head <branch> --repo senara-solutions/<repo> --json url,number,state,mergedAt,reviewDecision")`.
> 4. If the issue is closed AND a merged PR exists → the work **succeeded**. Do NOT enter failure diagnosis. Route to success path.
> 5. Include the verified state in your notification to Vincent. Never claim "no PR" or "manually closed" — state what the tools returned.
```

### Step 5: Add grounding regression eval scenario

**File:** `tests/eval/grounding_regressions/` — new scenario file

Add a scenario that reproduces the #716 fabrication class:

- Mock a callback turn with `failed: true` and error text
- Assert that the agent calls `run_gh` before claiming state
- Assert that forbidden phrases ("no PR", "manually closed", "handler crashed") are absent from the response when tools show a merged PR

This follows the existing grounding regression pattern (scenarios 1-33).

### Step 6: Unit tests for detection function

**File:** `crates/mika-agent/src/agent.rs` (test module)

Add tests for `detect_unverified_callback_state_claim`:

```rust
#[test]
fn test_detect_callback_claim_no_pr() {
    assert!(detect_unverified_callback_state_claim("There was no PR created").is_some());
}

#[test]
fn test_detect_callback_claim_manually_closed() {
    assert!(detect_unverified_callback_state_claim("Issue was manually closed").is_some());
}

#[test]
fn test_detect_callback_claim_handler_crashed() {
    assert!(detect_unverified_callback_state_claim("The handler crashed").is_some());
}

#[test]
fn test_detect_callback_claim_no_match_normal() {
    assert!(detect_unverified_callback_state_claim("Task completed successfully").is_none());
}

#[test]
fn test_detect_callback_claim_no_match_verified() {
    // The detection function only finds the claim — the guard checks tools_called separately
    assert!(detect_unverified_callback_state_claim("no PR found").is_some());
}
```

## File Change Summary

| File | Change | Lines (est.) |
|------|--------|-------------|
| `crates/mika-agent/src/agent.rs` | Add `detect_unverified_callback_state_claim()` detection function | ~30 |
| `crates/mika-agent/src/agent.rs` | Add guard 4c in post-condition chain + retry flag | ~35 |
| `crates/mika-agent/src/agent.rs` | Enhance `build_callback_trigger_context` for failed callbacks | ~10 |
| `crates/mika-agent/src/agent.rs` | Unit tests for detection function | ~30 |
| `skills/bundled/self-dev-callback/system_prompt.md` | Mandatory state verification in failure path | ~15 |
| `tests/eval/grounding_regressions/` | New scenario for #716 fabrication class | ~80 |
| **Total** | | **~200** |

## Design Decisions

1. **Guard position 4c (after completion-claim, before fabricated-action):** The callback state-claim guard is conceptually between "claimed something done without marking it" (#4) and "fabricated an action with zero tools" (#5). Position 4c is logical.

2. **Applies to all callback turns, not just failed ones:** The guard detects fabricated *state claims* regardless of callback success/failure flag. A successful callback can still produce fabricated claims about downstream state (e.g., "PR is ready for review" when the callback only confirmed a build passed).

3. **`run_gh` OR `check_task` OR `gh_read` satisfies:** Any verification tool counts. The agent doesn't need to call `run_gh` specifically — `check_task` also surfaces GitHub-enriched state (it has optional PR/issue enrichment per CLAUDE.md).

4. **Two-layer defense (guard + prompt):** The prompt reinforcement in `build_callback_trigger_context` and `self-dev-callback` system prompt is defense-in-depth. The structural guard is the primary defense.

5. **Conservative regex:** The detection patterns are deliberately narrow — they target the specific fabrication vocabulary observed in #716 and similar incidents. False positives are more costly than false negatives (the prompt layer catches the rest).

## Risks

- **False positives on the detection regex:** The agent might legitimately report "no PR" after calling `run_gh` and confirming there is indeed no PR. The guard handles this — it only fires when `tools_called` does NOT contain a verification tool. If `run_gh` was called, the guard is satisfied.
- **Callback turn step budget:** The verification calls consume 1-2 of the callback step budget (`MAX_CALLBACK_TOOL_STEPS`). This is acceptable — verification before narration is more valuable than one extra retry.

## Test Plan

1. `cargo test -p mika-agent` — unit tests for detection function
2. `cargo test -p mika-agent --test eval -- grounding` — grounding regression scenarios
3. `cargo clippy` — lint
4. Manual verification: trigger a callback with `failed: true` and confirm the guard fires
