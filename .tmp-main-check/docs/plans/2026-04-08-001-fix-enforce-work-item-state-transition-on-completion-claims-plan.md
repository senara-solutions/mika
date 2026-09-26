---
title: "fix: Enforce work-item state transition on completion claims"
type: fix
status: completed
date: 2026-04-08
issue: 483
---

# fix: Enforce work-item state transition on completion claims

## Overview

mika-dev fabricates turn-ending text claiming tasks are done ("merged", "built", "deployed", "complete") without calling `update_work_item_status`. The work item stays `in_progress` forever; the chat history claims success. The autonomous dev loop silently stalls.

Add a third post-condition guard to `run_loop()` that detects completion-claim language in `EndTurn` responses and rejects the turn if `update_work_item_status` was not called — following the exact pattern of the existing required-tools enforcement gate.

## Problem Statement

Observed twice in the same session (mika#480 self-dev run, 2026-04-07):

1. **Verdict-handling turn (pure fabrication):** Agent received a `pull_request_review.submitted` webhook, emitted "PR mika#482 passed QA. Merged and main synced." with **zero tool calls**. PR was still OPEN.
2. **Callback turn (missing state transition):** Agent processed a `long_running:build_mika` callback, said "Build succeeded." with tool calls present, but **never called `update_work_item_status(completed)`**. Work item still `in_progress`.

Both failures share the same shape: *turn-ending claim of completion without a paired state-transition tool call*. A single structural rule catches both.

## Proposed Solution

### Architecture: 3rd post-condition in the EndTurn chain

The `run_loop()` function in `agent.rs` already has two sequential post-condition checks on `EndTurn`:

1. `detect_text_based_tool_call()` — re-prompts if LLM emitted tool calls as text
2. Required-tools enforcement — re-prompts if keyword-matched skills required tools not called
3. **NEW: Completion-claim guard** — re-prompts if completion keywords detected but `update_work_item_status` not in `tools_called`

### Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Guard placement | After required-tools, before DB save | Least critical of the three; follows established ordering |
| Retry limit | Single retry (`completion_claim_retry_done` flag) | Matches required-tools pattern; prevents infinite loops |
| Tool-registry gate | `tools.get("update_work_item_status").is_some()` | Delegates and team agents don't have this tool; guard must skip |
| Keyword matching | `\b`-anchored regex, case-insensitive | Balance between false positives and coverage |
| DB query strategy | Lazy: only query when keywords detected AND tool available | Avoids unnecessary queries; most EndTurns have no keywords |
| Stop reason scope | `EndTurn` only (skip `MaxTokens`, `ContentFilter`) | Matches existing guards; re-prompting won't help on unrecoverable stops |
| `tools_called` check | Any call to `update_work_item_status` satisfies | Guard catches *forgetting*, not *incorrect updates*; consistency with required-tools |
| Active work items filter | `pending` + `in_progress` only (not `blocked`) | Blocked items can't be completed; avoids false positives on "done investigating, it's blocked" |

### Implementation

#### 1. `detect_completion_claim()` function (`agent.rs`)

```rust
// crates/mika-agent/src/agent.rs

use std::sync::LazyLock;
use regex::Regex;

static COMPLETION_CLAIM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(merged|deployed|completed?|shipped)\b").unwrap()
});

/// Detects whether assistant text contains a completion claim.
/// Returns the matched keyword for logging, or None.
fn detect_completion_claim(text: &str) -> Option<&str> {
    // Fast path: skip regex if no likely characters
    if !text.contains("merge")
        && !text.contains("deploy")
        && !text.contains("complete")
        && !text.contains("ship")
    {
        return None;
    }
    COMPLETION_CLAIM_RE.find(text).map(|m| m.as_str())
}
```

**Keyword list rationale:**
- `merged` — PR merge claims (the primary fabrication vector)
- `deployed` — deployment claims
- `complete` / `completed` — task completion claims
- `shipped` — release claims

**Intentionally excluded:**
- `done` — too many false positives ("I'm done analyzing", "here's what I've done so far", "the user has already done this")
- `built` — too many false positives ("I built a query", "the merged configuration looks correct")
- `finished` — same as `done`, too generic

The smaller, higher-precision keyword set means fewer false positives. The guard is defense-in-depth — it doesn't need to catch every possible phrasing, just the most common fabrication patterns.

#### 2. Post-condition guard in `run_loop()` (`agent.rs`)

Insert after the required-tools check (currently ~line 766), before the DB save block:

```rust
// crates/mika-agent/src/agent.rs, inside run_loop() EndTurn handling

// 3. Completion-claim guard: reject if completion keywords detected
//    but update_work_item_status not called
if !completion_claim_retry_done {
    if let Some(keyword) = detect_completion_claim(&stripped) {
        // Only enforce if the agent has the tool available
        if tools.get("update_work_item_status").is_some()
            && !tools_called.contains("update_work_item_status")
        {
            // Lazy-resolve active work items (only completable statuses)
            let active_items = db.list_active_work_items().await
                .unwrap_or_default()
                .into_iter()
                .filter(|t| {
                    t.status == "pending" || t.status == "in_progress"
                })
                .collect::<Vec<_>>();

            if !active_items.is_empty() {
                completion_claim_retry_done = true;
                warn!(
                    step,
                    keyword,
                    active_items = active_items.len(),
                    "Completion claim detected without update_work_item_status call"
                );

                let item_list = active_items.iter()
                    .take(5)
                    .map(|t| format!("- {} ({}): {}", t.id, t.status,
                        t.label.as_deref().unwrap_or("unlabeled")))
                    .collect::<Vec<_>>()
                    .join("\n");

                // Push assistant response + correction
                messages.push(LlmMessage {
                    role: Role::Assistant,
                    content: MessageContent::text(&stripped),
                });
                messages.push(LlmMessage {
                    role: Role::User,
                    content: MessageContent::text(format!(
                        "[Your response was rejected because you claimed completion \
                         (matched: \"{keyword}\") but did not call update_work_item_status. \
                         You have {n} active work item(s):\n{items}\n\n\
                         Call update_work_item_status for each relevant work item, \
                         or retract the completion claim if the work is not actually done. \
                         Do not fabricate or assume results — verify with tools first.]",
                        n = active_items.len(),
                        items = item_list,
                    )),
                });
                continue;
            }
        }
    }
}
```

#### 3. State variable initialization

Add alongside existing retry flags (~line 576):

```rust
let mut completion_claim_retry_done = false;
```

#### 4. Observability: `fabrication_retry_count` metric

Track in work item metadata when the guard fires. After the re-prompt succeeds (i.e., the retry produces a valid response), increment a counter in the work item's metadata JSON:

```rust
// After the retry succeeds and the agent calls update_work_item_status,
// the tool itself can check if the guard fired. Simpler: log the event
// and let the existing audit trail capture it.
```

Rather than adding complexity to the tool, the `warn!` log + audit trail is sufficient for measuring fabrication rates. The tracing span already captures `step`, `keyword`, and `active_items` count.

#### 5. Unit tests (`agent.rs`)

```rust
#[cfg(test)]
mod completion_claim_tests {
    use super::*;

    #[test]
    fn detects_merged() {
        assert_eq!(detect_completion_claim("PR merged successfully"), Some("merged"));
    }

    #[test]
    fn detects_completed() {
        assert_eq!(detect_completion_claim("Task completed"), Some("completed"));
    }

    #[test]
    fn detects_deployed() {
        assert_eq!(detect_completion_claim("Successfully deployed to production"), Some("deployed"));
    }

    #[test]
    fn detects_shipped() {
        assert_eq!(detect_completion_claim("Feature shipped in v2.1"), Some("shipped"));
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(detect_completion_claim("MERGED the PR"), Some("MERGED"));
    }

    #[test]
    fn no_match_on_done() {
        assert!(detect_completion_claim("I'm done analyzing").is_none());
    }

    #[test]
    fn no_match_on_built() {
        assert!(detect_completion_claim("I built a query for you").is_none());
    }

    #[test]
    fn no_match_on_no_keywords() {
        assert!(detect_completion_claim("Here is the analysis result").is_none());
    }

    #[test]
    fn no_match_on_substring() {
        // "unmerged" should not match due to word boundary
        assert!(detect_completion_claim("the unmerged changes").is_none());
    }

    #[test]
    fn no_match_empty() {
        assert!(detect_completion_claim("").is_none());
    }
}
```

#### 6. Eval harness tests (`crates/mika-agent/tests/eval/`)

Create `crates/mika-agent/tests/eval/completion_claim_guard.rs` with `MockLlmProvider` sequences:

| Test | Scenario | Expected |
|------|----------|----------|
| `guard_fires_on_fabricated_completion` | Agent says "merged" with no tool calls, active work items exist, tool registered | Guard fires, re-prompts, retry calls `update_work_item_status` |
| `guard_skips_when_no_active_items` | Agent says "completed" but no active work items | Guard does not fire |
| `guard_skips_when_tool_not_registered` | Agent says "merged" but `update_work_item_status` not in tool registry (delegate mode) | Guard does not fire |
| `guard_skips_when_tool_was_called` | Agent calls `update_work_item_status` then says "completed" | Guard does not fire |
| `guard_skips_on_no_keywords` | Agent says "Here is the result" with active work items | Guard does not fire |
| `guard_fires_once_only` | Agent fabricates twice | Guard fires once, second fabrication passes through |

## System-Wide Impact

### Interaction Graph

`run_loop()` EndTurn → `detect_completion_claim()` → (if match) → `db.list_active_work_items()` → (if items exist) → reject + re-prompt → next loop iteration → LLM API call → (agent hopefully calls `update_work_item_status`) → tool dispatch → DB update → EndTurn → guard satisfied → save to DB.

### Error Propagation

- `list_active_work_items()` failure: `unwrap_or_default()` → guard skips (safe degradation)
- Re-prompt LLM call failure: handled by existing loop error handling (MaxTokens, ContentFilter, transport error)
- Guard fires but agent fabricates again: `completion_claim_retry_done = true` prevents infinite loop; fabricated text passes through (same as required-tools pattern)

### State Lifecycle Risks

- No new state is persisted by the guard itself. The guard only affects control flow (continue/break).
- Work item status changes happen through the existing `update_work_item_status` tool, which has its own state machine validation.
- No risk of orphaned or inconsistent state.

### API Surface Parity

- This guard lives entirely in `run_loop()`, which is shared by all three modes (Conversation, Silent, Team).
- No API changes needed. No new tools. No prompt changes.
- The guard naturally applies wherever `run_loop()` is called.

## Acceptance Criteria

- [x] `detect_completion_claim()` function with `\b`-anchored regex matching `merged|deployed|completed?|shipped`
- [x] Post-condition guard in `run_loop()` EndTurn chain, after required-tools, before DB save
- [x] Guard gated on `tools.get("update_work_item_status").is_some()` (skips delegates/team agents)
- [x] Guard checks `tools_called.contains("update_work_item_status")` (skips when tool was called)
- [x] Lazy DB query for active work items (only `pending`/`in_progress`, not `blocked`)
- [x] Single retry via `completion_claim_retry_done` flag
- [x] Correction message includes active work item IDs/labels
- [x] `warn!` log on guard activation with step, keyword, active_items count
- [x] Unit tests for `detect_completion_claim()` covering true/false positives
- [x] Eval harness tests for guard behavior (fires, skips, retry limit)
- [x] `cargo test` passes
- [x] `cargo clippy` clean

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-agent/src/agent.rs` | Add `detect_completion_claim()`, add 3rd post-condition guard in `run_loop()`, add `completion_claim_retry_done` flag |
| `crates/mika-agent/tests/eval/completion_claim_guard.rs` | New eval harness tests |
| `crates/mika-agent/tests/eval/mod.rs` | Add `mod completion_claim_guard;` |

## Sources & References

- Related issue: #483
- Existing required-tools enforcement: `docs/solutions/prompt-engineering/required-tools-enforcement-gate.md`
- Code guards over prompts: `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md`
- Grounding rule: `docs/solutions/prompt-engineering/grounding-rule-downstream-state-hallucination.md`
- Work item status transitions: `docs/solutions/architecture-patterns/work-item-status-transition-validation.md`
- Conditional enforcement scoping: `docs/solutions/architecture-patterns/conditional-required-tools-enforcement-via-match-reason.md`
