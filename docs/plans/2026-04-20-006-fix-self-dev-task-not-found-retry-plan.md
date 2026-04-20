---
title: "fix: self-dev retry update_task_status via list_tasks lookup on task_not_found"
type: fix
status: active
date: 2026-04-20
---

# fix: self-dev retry update_task_status via list_tasks lookup on task_not_found

## Overview

Add a recovery rule to the self-dev close-out flow so that when `update_task_status` returns `task_not_found`, the agent looks up the correct task ID via `list_tasks` and retries — instead of ending the turn with inconsistent state.

## Problem Frame

When self-dev's close-out path (Step 6) calls `update_task_status` and the LLM has hallucinated or truncated the task UUID, the tool returns a structured `task_not_found` error. Currently, the agent ends the turn without recovering, leaving the child task stuck in `in_progress` even though the PR has already merged. This blocks milestone advancement and poisons downstream state (heartbeats see stale `in_progress` tasks with no work pending).

The hallucination pattern — correct prefix, wrong suffix — is a generic LLM characteristic, not a one-off bug. The recovery path must exist in the prompt because the agent needs its own context (which issue, which reference_url) to identify the correct task from `list_tasks` output.

**Incident:** Trace `7a9cb990-3cd8-11f1-9669-7eed18e94544` on 2026-04-20 (mika-dev, kimi-k2.5). `update_task_status` failed with `task_not_found` because the first 8 chars of the UUID matched but the suffix was hallucinated. The correct ID was visible in subsequent `list_tasks` output, but the turn ended without retrying.

## Requirements Trace

- R1. `self-dev/system_prompt.md` includes an explicit `task_not_found` recovery rule in the close-out flow (Step 6)
- R2. Eval harness test covers the scenario: `update_task_status` returns `task_not_found`, agent calls `list_tasks`, retries with the recovered ID
- R3. The `update_core_memory` missing-param case is either addressed or filed as a follow-up

## Scope Boundaries

- Prompt-level fix only for the primary recovery rule — no Rust engine changes
- The eval test exercises the tool-level `task_not_found` response and verifies the agent's mock-LLM retry sequence
- No changes to the UUID validation chain or `update_task_status` tool implementation

### Deferred to Separate Tasks

- Engine-level auto-resolve of `task_not_found` (rejected per issue — recovery needs agent context)
- Shorter task IDs in prompts (separate concern)
- Milestone resume mechanism (tracked separately)

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/self-dev/system_prompt.md` — Step 6 (lines 226-249) is the close-out flow where the recovery rule belongs
- `skills/bundled/self-dev/system_prompt.md` — Step 2 (lines 48-50) already demonstrates the pattern of calling `list_tasks` and matching by `reference_url`
- `crates/mika-agent/tools/mod.rs` (lines 317-325) — `task_not_found` structured error format: `{"error": "task_not_found", "field": "task_id", "task_id": "...", "reason": "..."}`
- `crates/mika-agent/tools/list_tasks.rs` — output format includes `ref:<url>` for each task
- `crates/mika-agent/tests/eval/test_phantom_retry_guard.rs` — closest existing eval test pattern (seeds tasks, registers `UpdateTaskStatusTool`, uses `clear_and_set` for dynamic IDs)
- `crates/mika-agent/tests/eval.rs` — test module registry

### Institutional Learnings

- **UUID validation at tool boundary** (`docs/solutions/best-practices/uuid-validation-at-tool-boundary.md`): Three-layer validation chain. The `task_not_found` error is structured JSON — the agent can parse it and act.
- **Terminal-state metadata fallback** (`docs/solutions/logic-errors/terminal-state-metadata-rejection-race.md`): If the task was already completed by another path, metadata writes still succeed. The retry path must account for this.
- **Phantom retry guard** (`docs/solutions/architecture-patterns/phantom-retry-guard-active-dispatch-metadata-validation.md`): Metadata keys containing "retry" are rejected when active callback children exist. The recovery rule's metadata naming must not conflict.
- **Engine guards vs prompt rules** (`docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`): This is a with-gradient behavior (agent already has the information, just needs explicit instruction to use it) — prompt rule is the right layer.

## Key Technical Decisions

- **Prompt-level fix, not engine guard:** The recovery needs agent context (current issue reference) to match from `list_tasks` output. An engine guard would need to duplicate this context, and the failure mode (hallucinated UUID) is recoverable with a single retry when the agent has explicit instructions.
- **Insert in Step 6 after the first paragraph:** The recovery rule goes right after line 228 which says "Call `update_task_status` based on the outcome." This is the natural point where `task_not_found` would be encountered.
- **Pattern mirrors Step 2:** Step 2 already teaches the agent to use `list_tasks` + `reference_url` matching. The recovery rule references the same pattern, making it consistent.
- **`update_core_memory` missing-param as follow-up:** The incident also showed `update_core_memory` called without `content` field. This is already covered by Rule 4 (tool input schema discipline) which lists `update_core_memory` requiring `"reasoning"`. The missing `content` field for `replace`/`append` actions is a distinct schema adherence issue — will be noted in the PR description as a follow-up.

## Open Questions

### Resolved During Planning

- **Where exactly in Step 6?** After line 228, before the status rules section. The recovery rule is a conditional branch on the `task_not_found` error from the `update_task_status` call that Step 6 mandates.
- **Should the rule use `check_task` or `list_tasks`?** `list_tasks` — it returns all tasks with their `reference_url`, allowing the agent to match by the current issue reference. `check_task` requires knowing the ID, which is the problem.

### Deferred to Implementation

- Exact wording of the prompt recovery rule will be refined during implementation to match the existing tone and structural gate patterns.

## Implementation Units

- [x] **Unit 1: Add task_not_found recovery rule to self-dev system_prompt.md**

**Goal:** Add an explicit recovery rule in Step 6 so the agent retries `update_task_status` after resolving the correct task ID via `list_tasks`.

**Requirements:** R1

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/self-dev/system_prompt.md`

**Approach:**
- Insert a new paragraph after line 228 (after "Call `update_task_status` based on the outcome. **Always include the `metadata` parameter**...")
- The rule should: (1) detect `task_not_found` error from `update_task_status`, (2) call `list_tasks(status="in_progress")`, (3) match by `reference_url` containing the current issue reference (e.g., `mika#677`), (4) retry `update_task_status` with the recovered `task_id`, (5) explicitly state: do NOT end the turn on `task_not_found` — that leaves state inconsistent
- Follow the structural gate pattern used elsewhere in the prompt (explicit conditions, mandatory actions)
- Reference the incident (trace `7a9cb990`, 2026-04-20) to anchor the behavior
- Keep it to 5-8 prompt lines — concise but unambiguous

**Patterns to follow:**
- Step 2 (lines 48-50): `list_tasks` + `reference_url` matching pattern
- Calibration Rules format: incident citation with date and trace ID
- Structural gate pattern from milestone workflow (explicit GATE checks)

**Test scenarios:**
- Happy path: recovery rule text is present in the system_prompt.md, positioned within Step 6, references `task_not_found`, `list_tasks`, and `reference_url`
- Edge case: the rule also covers the case where `list_tasks` returns no matching task (agent should escalate, not loop)

**Verification:**
- The recovery rule is visible in Step 6 of `system_prompt.md`
- Build succeeds (`cargo build` picks up the prompt change via `build.rs`)

- [x] **Unit 2: Eval harness test for task_not_found retry sequence**

**Goal:** Add an integration test that verifies the agent calls `list_tasks` after receiving `task_not_found` and retries `update_task_status` with the correct ID.

**Requirements:** R2

**Dependencies:** Unit 1

**Files:**
- Create: `crates/mika-agent/tests/eval/test_task_not_found_retry.rs`
- Modify: `crates/mika-agent/tests/eval.rs` (add module declaration)

**Approach:**
- Follow the `test_phantom_retry_guard.rs` pattern: build harness with placeholder responses, seed task in DB, replace responses with real IDs via `clear_and_set`
- Register `UpdateTaskStatusTool` explicitly (not in `default_tools()` for non-multi-agent setups)
- Mock LLM response sequence: (1) call `update_task_status` with wrong ID -> tool returns `task_not_found`, (2) call `list_tasks` to find correct ID, (3) call `update_task_status` with correct ID -> succeeds, (4) text response confirming completion
- Seed two tasks: one with a known `reference_url` (the target), one without (noise). Set target to `in_progress`.
- Assert: `assert_tool_order(&trace, &["update_task_status", "list_tasks", "update_task_status"])` — the sequence must be: fail, lookup, retry
- Assert: first `update_task_status` output contains `task_not_found`
- Assert: second `update_task_status` output does NOT contain `task_not_found` (success)

**Patterns to follow:**
- `test_phantom_retry_guard.rs`: `EvalHarness::builder()`, `seed_task()`, `clear_and_set()`, assertion helpers
- `tools_with_update_task_status()` helper for tool registration

**Test scenarios:**
- Happy path: `update_task_status` returns `task_not_found` on first call, agent calls `list_tasks`, then retries `update_task_status` with recovered ID — second call succeeds
- Edge case: verify the tool order is strictly `update_task_status` -> `list_tasks` -> `update_task_status` (no extra calls between)

**Verification:**
- `cargo test -p mika-agent --test eval test_task_not_found_retry` passes
- The test exercises the full mock sequence without flakiness

## System-Wide Impact

- **Interaction graph:** The recovery rule adds a conditional branch in Step 6 that calls `list_tasks` (read-only) before retrying `update_task_status`. No new tool registrations or callback paths.
- **Error propagation:** If `list_tasks` returns no matching task, the agent should escalate to Vincent rather than looping. The prompt rule must include this fallback.
- **State lifecycle risks:** None — the retry path is idempotent. If the task was already completed by another path, the terminal-state metadata fallback (#617) handles it gracefully.
- **Unchanged invariants:** The `update_task_status` tool, UUID validation chain, and phantom retry guard are not modified. The recovery is purely at the prompt layer.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| LLM ignores the recovery rule under context pressure | The rule uses structural gate language and incident citation — harder for LLMs to skip than plain prose. If it fails repeatedly, escalate to engine guard per the engine-guards-vs-prompt-rules decision framework. |
| Recovery rule conflicts with phantom retry guard | The retry metadata naming is controlled by the existing Step 6 instructions, not the recovery rule. The recovery rule only changes the `task_id`, not the metadata shape. |
| Eval test flakiness from mock sequence sensitivity | The test uses deterministic `MockLlmProvider` sequences with `clear_and_set` — no network, no timing. Pattern proven stable in `test_phantom_retry_guard.rs`. |

## Sources & References

- Related issue: #693
- Related code: `skills/bundled/self-dev/system_prompt.md`, `crates/mika-agent/tools/mod.rs` (UUID validation), `crates/mika-agent/tests/eval/test_phantom_retry_guard.rs`
- Learnings: `docs/solutions/best-practices/uuid-validation-at-tool-boundary.md`, `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`
