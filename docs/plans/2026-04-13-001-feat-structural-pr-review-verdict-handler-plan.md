---
title: "feat: Structural pull_request_review.submitted verdict handler"
type: feat
status: active
date: 2026-04-13
issue: 524
---

# feat: Structural pull_request_review.submitted verdict handler

## Overview

Add a structural webhook handler in `mika-agent` that intercepts `pull_request_review.submitted` events **before** the LLM turn, parses the `VERDICT:` line from the review body, and for `pass` verdicts automatically initiates a PR merge via `gh pr merge`. This removes the merge decision from LLM improvisation and makes it a deterministic state-machine transition.

## Problem Frame

On 2026-04-11, mika-dev (qwen3-coder) received a `pull_request_review.submitted(approved)` webhook for PR #522 with `VERDICT: pass` in the review body. Instead of merging, mika-dev misclassified the event as `pull_request.opened`, fabricated a task_id, and re-dispatched `run_claude_pilot` for an unrelated issue. PR #522 sat unmerged for ~7 hours until manually merged.

Root cause: the verdict-to-merge decision was left to LLM interpretation of raw webhook text. The LLM never parsed `VERDICT: pass`, never checked PR state, and improvised a completely wrong action. This is a state-machine transition, not a judgement call.

## Requirements Trace

- R1. On `pull_request_review.submitted` with `state=approved` AND review body containing `VERDICT: pass`: look up work item, initiate merge, update status, notify user, pre-digest for LLM
- R2. Handler runs before the LLM turn — no dependency on LLM tool calls for the merge action
- R3. `block[*]` or `hold[*]` verdicts pass through to LLM as today
- R4. Missing `VERDICT:` line: log warning, pass through to LLM with verdict_missing flag
- R5. Non-`in_progress` work item status: log and skip, no double-merge
- R6. Integration test: `pass` verdict triggers merge, `run_claude_pilot` not called
- R7. Integration test: `block[ci]` verdict does NOT trigger auto-merge
- R8. Telemetry: `verdict_handled` audit event with `(verdict, action, work_item_id, pr_url)`

## Scope Boundaries

- Gateway routing is unchanged — `pull_request_review.submitted` still routes to mika-dev
- The `pr_merge_with_gate` LLM tool continues to exist for manual merge scenarios
- No new work item statuses are added to the schema (avoids v23 migration) — uses existing `in_progress` + metadata tracking instead
- No reviewer identity filtering — the handler keys off the `VERDICT:` line presence, which implicitly filters for mika-qa (the only agent emitting `VERDICT:` lines). Human reviews without `VERDICT:` lines pass through to the LLM.
- No gateway-side changes — the structural handling happens entirely in mika-agent

### Deferred to Separate Tasks

- Tool-level refusal for `run_claude_pilot` when work item is in terminal/merge states: mika#525
- QA review skill enforcement of `VERDICT:` format: companion ticket on mika-skills

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/server/handlers.rs` — `handle_message()` is the interception point. The spawned async task (line 185) acquires the agent lock, then passes `req.text` directly to `run_agent()`. The verdict handler inserts between lock acquisition and `run_agent()`.
- `crates/mika-agent/src/tools/pr_merge_with_gate.rs` — Contains `run_gh_checks()`, `classify_checks()`, and `run_gh_merge()` as private helpers. These need to be made `pub(crate)` for reuse by the verdict handler.
- `crates/mika-agent/src/db.rs:3123` — `find_active_work_item_by_ref_url(agent_id, reference_url)` queries the `tasks` table by `reference_url` column (indexed). Work items have `reference_url` set to the GitHub issue URL, while PR URLs are in `metadata.claude_pilot.pr_url`.
- `crates/mika-gateway/src/github.rs:241-261` — `format_event_text()` for `pull_request_review` produces structured text: `[GitHub] PR review ({state}) on {repo}#{number} ({title}) by @{reviewer}\n{review_url}\n\n{body}`. The `state` and `body` are extractable from this formatted text.
- `crates/mika-agent/src/server/types.rs:13` — `MessageRequest` has `text`, `chat_id`, `channel`, `request_id`, `agent`, `images`. No structured metadata field exists.
- `crates/mika-agent/src/async_db.rs:339` — Async wrapper for `find_active_work_item_by_ref_url`.
- `crates/mika-agent/src/tools/update_work_item_status.rs` — Status state machine: `pending` -> any; `in_progress` -> blocked/completed/cancelled; `blocked` -> in_progress/completed/cancelled. Terminal states: completed, cancelled.
- `crates/mika-agent/src/work_item_metadata.rs` — Two-level shallow merge for work item metadata JSON.

### Institutional Learnings

- **Verdict misclassification compound doc** (`docs/solutions/agent-quality/2026-04-11-mika-dev-verdict-misclassification-pr-522.md`): "Verdict-to-action mapping is a state machine, not a prompt." Pre-digest the event as a fait accompli.
- **CI gate tool backstop** (`docs/solutions/architecture-patterns/ci-gate-tool-structural-backstop-for-pr-merges.md`): `pr_merge_with_gate` is an existing atomic check+merge tool. Reuse its internal logic.
- **Merge two-step contracts** (`docs/solutions/architecture-patterns/merge-two-step-llm-tool-contracts.md`): The verdict handler must be atomic — not split into "parse verdict" then "act on verdict" as two LLM-chained steps.
- **Completion-claim guard** (`docs/solutions/architecture-patterns/completion-claim-guard-work-item-state-enforcement.md`): The pre-digested message must avoid trigger words like "merged"/"completed" since the engine action happens outside the LLM's tool calls. Use "merge initiated" phrasing.
- **Engine-level callback metadata extraction** (`docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md`): Establishes the pattern of deterministic extraction before `run_silent_agent()`.
- **Silent callback max_steps exhaustion** (`docs/solutions/runtime-errors/silent-callback-max-steps-exhaustion.md`): Pre-digesting the verdict eliminates most LLM steps needed to handle it.

## Key Technical Decisions

- **Intercept in `handle_message` spawned task, not in a new endpoint:** The gateway already sends formatted text to `/message`. Adding a new endpoint would require gateway changes. Instead, parse the formatted text for verdict patterns inside the existing `handle_message` async task, after lock acquisition but before `run_agent()`. This is the minimal-change approach.
- **Parse from formatted text, not raw webhook payload:** The agent only receives formatted text from the gateway. Parsing `[GitHub] PR review (approved)` and extracting `VERDICT:` from the body is reliable because `format_event_text()` has a stable format. No gateway changes needed.
- **Reuse `run_gh_checks`/`classify_checks`/`run_gh_merge` from `pr_merge_with_gate`:** These are already battle-tested. Make them `pub(crate)` instead of duplicating logic. The verdict handler uses the same CI gate classification.
- **Work item lookup by metadata `pr_url`, not `reference_url`:** The `reference_url` column stores the GitHub issue URL (e.g., `https://github.com/senara-solutions/mika/issues/42`), not the PR URL. The PR URL is in `metadata.claude_pilot.pr_url`. Add a new `find_active_work_item_by_pr_url()` DB method that queries via `json_extract(metadata, '$.claude_pilot.pr_url')`.
- **No new work item statuses:** Adding `in_pr` and `merging` would require a schema migration (v23) with a full `tasks` table rebuild. Instead: gate on `in_progress` status + presence of `metadata.claude_pilot.pr_url`, and track merge-in-progress via `metadata.verdict_merge.state` field.
- **Pre-digest replaces, not augments, the user message:** When the verdict handler acts, the original webhook text is replaced with a pre-digest message that describes what the engine did. The LLM receives this as a fait accompli and cannot countermand it. For block/hold/missing verdicts, the original text passes through unmodified.
- **Merge failure falls through to LLM:** If `gh pr merge` fails, log the error, do NOT update work item metadata, and enrich the pre-digest with the failure reason so the LLM can decide the next action.

## Open Questions

### Resolved During Planning

- **Q: Should the handler filter by reviewer identity?** No. The `VERDICT:` line is the implicit filter — only mika-qa emits it. Human reviews without `VERDICT:` pass through normally.
- **Q: What if `gh pr merge --auto` succeeds but the PR never actually merges?** The handler uses `classify_checks()` from `pr_merge_with_gate` — if all checks pass, it merges immediately; if pending, it enables auto-merge. The pre-digest message distinguishes these cases ("merged" vs "auto-merge enabled") so the LLM and user know the actual state.
- **Q: Could the LLM duplicate the merge in its tool loop?** The pre-digest explicitly tells the LLM not to call `pr_merge_with_gate`. The existing `AlreadyMerged` detection in the tool provides defense-in-depth.
- **Q: What about multiple VERDICT lines?** First match wins. The QA review skill emits exactly one `VERDICT:` line per review.
- **Q: How to extract PR number and repo from the formatted text?** Parse from the `[GitHub] PR review ({state}) on {repo}#{number}` pattern using regex.

### Deferred to Implementation

- Exact regex patterns for parsing the formatted webhook text — will be refined against real gateway output
- Whether `json_extract` on the `metadata` column needs a partial index for performance — likely not needed given low work item volume per agent

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
handle_message() spawned task:
  ├─ acquire agent lock
  ├─ [NEW] try_handle_pr_review_verdict(&req, &agent_state, &app_state)
  │   ├─ parse_pr_review_event(text) -> Option<PrReviewEvent>
  │   │   └─ regex: "[GitHub] PR review ({state}) on {repo}#{number}" + VERDICT: line
  │   ├─ if no match -> return None (passthrough)
  │   ├─ parse_verdict(body) -> Verdict { Pass, Block(sub), Hold(sub), Missing }
  │   ├─ if Block/Hold/Missing -> return VerdictAction::Passthrough { enrichment }
  │   ├─ if Pass:
  │   │   ├─ find_work_item_by_pr_url(pr_url)
  │   │   ├─ if not found or status != in_progress -> return Passthrough
  │   │   ├─ run_gh_checks + classify_checks + run_gh_merge (reused from pr_merge_with_gate)
  │   │   ├─ on success: update work item metadata, log audit event, send notification
  │   │   └─ return VerdictAction::Handled { pre_digest_text }
  │   └─ on merge failure: return VerdictAction::Handled { pre_digest_with_error }
  ├─ match verdict_action:
  │   ├─ Handled -> replace req.text with pre_digest_text
  │   └─ Passthrough -> optionally enrich req.text with verdict_missing flag
  └─ run_agent(params) as today
```

## Implementation Units

- [x] **Unit 1: Extract merge helpers from `pr_merge_with_gate` to `pub(crate)`**

**Goal:** Make `run_gh_checks`, `classify_checks`, `run_gh_merge`, `run_gh_subprocess`, and the supporting types (`GhCheck`, `CheckClassification`, `CheckInfo`, `MergeGateResult`) reusable by the verdict handler.

**Requirements:** R1 (merge reuse)

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/tools/pr_merge_with_gate.rs`
- Test: existing tests in same file should continue passing

**Approach:**
- Change visibility of `run_gh_checks`, `classify_checks`, `run_gh_merge`, `run_gh_subprocess` from private to `pub(crate)`
- Change visibility of `GhCheck`, `CheckClassification`, `CheckInfo` types to `pub(crate)`
- No behavioral changes — purely a visibility refactor

**Patterns to follow:**
- Existing `pub(crate)` patterns in the codebase (e.g., `crate::skills::executor::scrub_mika_env_vars`)

**Test scenarios:**
- Happy path: All existing `pr_merge_with_gate` tests pass unchanged (no regressions from visibility change)

**Verification:**
- `cargo test -p mika-agent` passes
- `cargo clippy -p mika-agent` passes

- [x] **Unit 2: Add `find_work_items_by_pr_url` DB method**

**Goal:** Enable lookup of active work items by the PR URL stored in `metadata.claude_pilot.pr_url`.

**Requirements:** R1 (work item lookup)

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/db.rs`
- Modify: `crates/mika-agent/src/async_db.rs`
- Test: `crates/mika-agent/src/db.rs` (inline tests)

**Approach:**
- Add `find_active_work_item_by_pr_url(agent_id, pr_url)` to `Database` that queries `WHERE json_extract(metadata, '$.claude_pilot.pr_url') = ?2 AND trigger_type = 'manual' AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')`
- Add corresponding async wrapper in `AsyncDatabase`
- Return `Option<Task>` matching the same pattern as `find_active_work_item_by_ref_url`

**Patterns to follow:**
- `find_active_work_item_by_ref_url` in `db.rs:3123` — same query structure with column-based WHERE
- Async wrapper pattern in `async_db.rs`

**Test scenarios:**
- Happy path: Create work item with `metadata.claude_pilot.pr_url` set, look up by that URL, find it
- Edge case: No matching work item returns `None`
- Edge case: Work item exists but is in terminal status (`completed`) — not returned
- Edge case: Work item exists but `pr_url` is in a different metadata path — not returned
- Edge case: Multiple work items with same `pr_url` — returns one (LIMIT 1)

**Verification:**
- New unit tests pass
- `cargo test -p mika-agent` passes

- [x] **Unit 3: Implement verdict parser module**

**Goal:** Parse `pull_request_review.submitted` formatted text into structured verdict data, extracting review state, repo, PR number, reviewer, PR URL, and the verdict classification.

**Requirements:** R1, R3, R4

**Dependencies:** None

**Files:**
- Create: `crates/mika-agent/src/server/verdict.rs`
- Modify: `crates/mika-agent/src/server/mod.rs`
- Test: `crates/mika-agent/src/server/verdict.rs` (inline tests)

**Approach:**
- Define `PrReviewEvent` struct: `{ state, repo, pr_number, reviewer, review_url, body }`
- Define `Verdict` enum: `Pass`, `Block(String)`, `Hold(String)`, `Missing`
- `parse_pr_review_event(text: &str) -> Option<PrReviewEvent>` — regex match on `[GitHub] PR review ({state}) on {owner/repo}#{number} ({title}) by @{reviewer}` first line, extract review URL from second line, body from remainder
- `parse_verdict(body: &str) -> Verdict` — case-insensitive scan for `VERDICT:` at start of line. First match wins. Parse value: `pass`, `block[...]`, `hold[...]`. If no match, return `Missing`.
- Construct PR URL from repo + number: `https://github.com/{repo}/pull/{number}`

**Patterns to follow:**
- `parse_github_ref` in `crates/mika-agent/src/tools/check_work_item.rs` for URL parsing patterns
- `LazyLock<Regex>` pattern from `pr_merge_with_gate.rs`

**Test scenarios:**
- Happy path: Parse well-formed `[GitHub] PR review (approved)` text with `VERDICT: pass` -> `PrReviewEvent` + `Verdict::Pass`
- Happy path: Parse `VERDICT: block[ci]` -> `Verdict::Block("ci")`
- Happy path: Parse `VERDICT: hold[review]` -> `Verdict::Hold("review")`
- Edge case: Review text with no `VERDICT:` line -> `Verdict::Missing`
- Edge case: `VERDICT: PASS` (uppercase) -> `Verdict::Pass` (case-insensitive)
- Edge case: `verdict: pass` (lowercase prefix) -> `Verdict::Pass`
- Edge case: Multiple `VERDICT:` lines -> first match wins
- Edge case: `VERDICT:pass` (no space after colon) -> `Verdict::Pass`
- Edge case: Non-review GitHub event text (issue, PR opened) -> `parse_pr_review_event` returns `None`
- Edge case: Review with state `changes_requested` -> parsed but state is not `approved`
- Edge case: Truncated body (`[truncated]` suffix) with no `VERDICT:` line -> `Verdict::Missing` with truncation noted
- Error path: Malformed first line (missing repo/number) -> returns `None`

**Verification:**
- All unit tests pass covering the verdict parsing matrix
- `cargo clippy` clean

- [x] **Unit 4: Implement verdict handler logic**

**Goal:** The core handler that coordinates verdict parsing, work item lookup, merge execution, metadata update, audit event, and notification. Returns either a pre-digest message (for pass verdicts) or a passthrough signal (for other cases).

**Requirements:** R1, R2, R3, R4, R5, R8

**Dependencies:** Unit 1, Unit 2, Unit 3

**Files:**
- Create: `crates/mika-agent/src/server/verdict_handler.rs`
- Modify: `crates/mika-agent/src/server/mod.rs`
- Test: `crates/mika-agent/src/server/verdict_handler.rs` (inline tests)

**Approach:**
- Define `VerdictAction` enum: `Handled { pre_digest: String }`, `Passthrough { enrichment: Option<String> }`
- `try_handle_pr_review_verdict(text, db, github_token, message_sender, agent_id, session_id, trace_id) -> VerdictAction`:
  1. Parse `PrReviewEvent` from text — return `Passthrough` if not a PR review
  2. Check `state == "approved"` — return `Passthrough` if not approved
  3. Parse `Verdict` from body
  4. For `Block`/`Hold`: return `Passthrough` (LLM handles)
  5. For `Missing`: log warning, return `Passthrough` with `verdict_missing=true` enrichment
  6. For `Pass`:
     a. Construct PR URL, find work item by `pr_url`
     b. If no work item or status != `in_progress`: log, return `Passthrough`
     c. Call `run_gh_checks` + `classify_checks`
     d. Based on classification: `AllPassed` -> `run_gh_merge` (immediate), `HasPending` -> `run_gh_merge` with `--auto`, `HasFailures` -> return `Passthrough` with enrichment
     e. On merge success: update work item metadata (set `verdict_merge.state`, `verdict_merge.merged_at`, `verdict_merge.pr_number`), log `verdict_handled` audit event, send notification
     f. On merge failure: log error, return `Handled` with error pre-digest
  7. Build pre-digest message with clear "do not call pr_merge_with_gate" instruction

- Audit event follows existing pattern: `log_audit_event(agent_id, session_id, "verdict_handled", target_key, before_value, after_value, reasoning, trace_id)`
- Notification uses `message_sender.send()` directly

**Patterns to follow:**
- `try_extract_callback_metadata()` in `task_engine/dispatcher.rs` — engine-level extraction before LLM turn
- `format_callback_framing()` in `agent.rs` — pre-digest XML wrapping pattern
- Audit event pattern in `update_work_item_status.rs`

**Test scenarios:**
- Happy path: Approved review with `VERDICT: pass`, work item in `in_progress`, all CI checks pass -> `Handled` with merge pre-digest, audit event logged
- Happy path: `VERDICT: pass` with pending CI checks -> `Handled` with auto-merge pre-digest
- Happy path: `VERDICT: block[ci]` -> `Passthrough` (no merge action)
- Happy path: `VERDICT: hold[review]` -> `Passthrough`
- Edge case: Missing `VERDICT:` line -> `Passthrough` with warning logged
- Edge case: Approved review but no matching work item -> `Passthrough` with log
- Edge case: Work item found but status is `completed` (terminal) -> `Passthrough` with log
- Edge case: Work item found but status is `pending` (not `in_progress`) -> `Passthrough` with log
- Edge case: CI checks have failures -> `Passthrough` with enrichment about failing checks
- Error path: `gh pr merge` subprocess fails -> `Handled` with error pre-digest, work item metadata NOT updated
- Error path: `gh pr checks` fails -> `Handled` with error pre-digest
- Edge case: Review state is `commented` (not `approved`) -> `Passthrough`
- Edge case: `github_token` is None -> `Passthrough` with warning (cannot merge without token)
- Integration: Audit event contains correct `(verdict, action, work_item_id, pr_url)` fields

**Verification:**
- All unit tests pass (mock DB, mock subprocess where needed)
- Pre-digest message avoids completion-claim guard trigger words
- `cargo clippy` clean

- [x] **Unit 5: Wire verdict handler into `handle_message`**

**Goal:** Insert the verdict handler call into the `handle_message` async task, between agent lock acquisition and `run_agent()`, so verdicts are handled structurally before the LLM sees the message.

**Requirements:** R2

**Dependencies:** Unit 4

**Files:**
- Modify: `crates/mika-agent/src/server/handlers.rs`
- Test: `crates/mika-agent/tests/eval/` (integration tests in Unit 6)

**Approach:**
- After lock acquisition and before `run_agent()` call (around line 187-224 in `handlers.rs`), add:
  1. Check if `req.channel == "github"` (skip for non-GitHub messages)
  2. Call `try_handle_pr_review_verdict()` with the request text and agent state
  3. Match on `VerdictAction`:
     - `Handled { pre_digest }` -> replace `req.text` with `pre_digest`
     - `Passthrough { enrichment: Some(e) }` -> prepend enrichment to `req.text`
     - `Passthrough { enrichment: None }` -> no change
  4. Continue to `run_agent()` with the (potentially modified) text

- The handler needs: `&a.db`, `s.github_token`, `sender_arc`, `a.db.agent_id()`, `&session_id`, `req.request_id` (as trace_id)

**Patterns to follow:**
- The existing skill hot-reload block (lines 189-202) as an example of pre-agent-loop logic inside the spawned task
- `is_callback_turn` flag pattern for signaling special turn context to `AgentParams`

**Test scenarios:**
- Test expectation: none — integration coverage provided by Unit 6. This unit is wiring only.

**Verification:**
- `cargo build -p mika-agent` compiles
- `cargo clippy -p mika-agent` clean
- Existing handler tests still pass

- [x] **Unit 6: Integration tests**

**Goal:** End-to-end tests that verify the structural verdict handler works correctly for both pass and block scenarios, matching the acceptance criteria.

**Requirements:** R6, R7

**Dependencies:** Unit 5

**Files:**
- Create: `crates/mika-agent/tests/eval/verdict_handler.rs`
- Modify: `crates/mika-agent/tests/eval/mod.rs` (if module registration needed)
- Test: `crates/mika-agent/tests/eval/verdict_handler.rs`

**Approach:**
- Use `EvalHarness` with `MockLlmProvider` to exercise the full `run_agent()` path
- Create test work items with `metadata.claude_pilot.pr_url` set to match test PR URLs
- Simulate the exact formatted webhook text that the gateway produces for `pull_request_review.submitted`
- For the merge subprocess: the tests should verify the handler's decision logic (verdict parsing, work item lookup, action selection) rather than actually calling `gh`. Mock or stub `run_gh_merge`/`run_gh_checks` at the function level.

**Execution note:** Study existing eval tests in `crates/mika-agent/tests/eval/` for harness patterns before implementing.

**Patterns to follow:**
- Existing eval tests in `crates/mika-agent/tests/eval/` — `EvalHarness` builder, `MockLlmProvider` sequence
- Test data setup for work items in `update_work_item_status.rs` tests

**Test scenarios:**
- Integration: VERDICT: pass with matching `in_progress` work item -> handler produces pre-digest text, `run_agent()` receives modified message, LLM does NOT invoke `pr_merge_with_gate` or `run_claude_pilot`
- Integration: VERDICT: block[ci] -> handler does NOT merge, `run_agent()` receives original text, LLM sees the review for decision-making
- Integration: VERDICT: pass but no matching work item -> passthrough to LLM
- Integration: Non-PR-review GitHub event (e.g., issue assigned) -> passthrough, no verdict handling attempted
- Integration: VERDICT: pass with work item in `completed` status -> passthrough, no merge

**Verification:**
- `cargo test -p mika-agent --test eval` passes with new verdict handler tests
- Tests are deterministic (no network calls, mock subprocess)

## System-Wide Impact

- **Interaction graph:** The verdict handler runs in `handle_message` before `run_agent()`. It calls into `pr_merge_with_gate`'s extracted helpers (subprocess), `AsyncDatabase` (work item lookup + metadata update + audit), and `MessageSender` (notification). The LLM's `run_agent()` receives either the original text or a pre-digest.
- **Error propagation:** Merge subprocess failures produce a pre-digest error message for the LLM rather than bubbling up as HTTP errors. DB failures in work item lookup are logged and treated as passthrough (graceful degradation).
- **State lifecycle risks:** The handler updates `metadata.verdict_merge` but does NOT transition work item status — that's left to the LLM after confirming the merge. This avoids the completion-claim guard false-positive. If the handler crashes mid-execution (after `gh pr merge` but before metadata update), the PR may be merged without metadata tracking — acceptable since `AlreadyMerged` detection handles the idempotency concern.
- **API surface parity:** The `POST /message` endpoint behavior changes — it now has a side-effect path for GitHub PR review messages. No external API contract changes (same request/response shapes).
- **Integration coverage:** Unit 6 covers the full handler-to-agent-loop path. The `pr_merge_with_gate` tool's existing tests cover the merge subprocess logic.
- **Unchanged invariants:** The gateway's `format_event_text()` format is the contract the verdict parser relies on. Any change to the `pull_request_review` text format would break parsing. The `MessageRequest` schema is unchanged. The work item status state machine is unchanged.
- **Post-condition guard interaction:** The pre-digest message for successful merges must avoid the completion-claim guard trigger words ("merged", "completed"). Use phrasing like "merge initiated" or "auto-merge enabled". The fabricated-action-claim guard should not fire because the engine action is not a tool call visible to the guard.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Gateway changes `format_event_text()` format, breaking verdict parser | Add a comment in `format_event_text()` noting the agent-side parser dependency. Parser tests catch regressions. |
| `json_extract` on metadata column is slow for agents with many work items | Low risk — work item volume per agent is small (tens, not thousands). Can add index later if needed. |
| Agent lock held during `gh pr merge` subprocess (up to 60s) blocks other messages | Existing behavior for all messages — the agent already holds the lock during `run_agent()` which can run for minutes. The merge subprocess is faster. |
| Pre-digest wording triggers completion-claim guard | Carefully avoid trigger words. Test the exact pre-digest text against `detect_completion_claim()`. |
| `gh` CLI not installed in agent container | Existing risk for `pr_merge_with_gate` — the verdict handler uses the same subprocess path and fails gracefully. |

## Sources & References

- Related issue: #524
- Related issue: #525 (companion: tool-level refusal)
- Compound doc: `docs/solutions/agent-quality/2026-04-11-mika-dev-verdict-misclassification-pr-522.md`
- Stuck PR: senara-solutions/mika#522
- Existing merge tool: `crates/mika-agent/src/tools/pr_merge_with_gate.rs`
- CI gate backstop learning: `docs/solutions/architecture-patterns/ci-gate-tool-structural-backstop-for-pr-merges.md`
