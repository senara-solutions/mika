---
title: "refactor(agent): rename work_item tools to task-centric vocabulary"
type: refactor
status: active
date: 2026-04-18
---

# refactor(agent): rename work_item tools to task-centric vocabulary

## Overview

Rename the four agent-facing work_item tools (`create_work_item`, `update_work_item_status`, `list_work_items`, `check_work_item`) and all internal references to use task-centric vocabulary that aligns with the actual domain model (DB table `tasks`, engine task IDs, CLI `--task-id`). The "work item" alias was introduced in `522e81f` but created a vocabulary split that contributed to the #595 UUID fabrication incident.

## Problem Frame

The agent sees tool names like `create_work_item` but the domain model is `tasks` everywhere else -- DB table, engine IDs, CLI flags, field names standardized in #601. The vocabulary split causes the LLM to hallucinate separate entities (`work_item_id` vs `task_id`) and fill them differently. Pre-1.0, so a clean break with no backward-compat aliases is appropriate.

## Requirements Trace

- R1. Rename all four agent-facing tools: `create_work_item` -> `create_task`, `update_work_item_status` -> `update_task_status`, `list_work_items` -> `list_tasks`, `check_work_item` -> `check_task`
- R2. Rename `validate_work_item()` -> `validate_task()` and remaining `work_item_id` variables -> `task_id`
- R3. Rename `work_item_metadata` module -> `task_metadata`
- R4. Update all DB helper method names that use `work_item` vocabulary
- R5. Update all bundled skill prompts (8 `system_prompt.md` + 3 `tools.json`)
- R6. Update prompt assembly, agent loop guards, rewind system, and server handlers
- R7. Update documentation (`crates/mika-agent/CLAUDE.md`, `docs/` files)
- R8. All tests pass after rename (`cargo test`)

## Scope Boundaries

- No schema migration -- DB table is already `tasks`
- No backward-compat aliases -- pre-1.0 clean break
- Agent identity files (`mika-dev/soul.md`) are agent-local, not in source -- note for post-deploy, not part of this PR
- Historical plan docs and brainstorm docs are not updated (they reflect the vocabulary at the time they were written)
- Solution docs in `docs/solutions/` ARE updated for accuracy since they serve as active reference

### Deferred to Separate Tasks

- Agent core memory audit post-deploy: verify no `self_model` entries reference old tool names
- Community skills in `mika-skills/` repo: separate PR if any reference work_item tool names

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/tools/` -- Tool trait pattern: `fn name() -> &str` + `ToolDefinition.name` (two string sites per tool)
- `crates/mika-agent/src/tools/mod.rs` -- `default_tools()` registers read tools, `management_tools_if_needed()` registers write tools (split registration pattern)
- `crates/mika-agent/src/agent.rs` -- Completion-claim guard matches on tool name strings
- `crates/mika-agent/src/rewind.rs` -- Match arms on tool name strings + `"work_item:{id}"` target_key prefix
- `crates/mika-agent/src/skills/index.rs` -- `inject_work_item_id_field()` injects `"work_item_id"` into long-running tool schemas
- `crates/mika-agent/src/work_item_metadata.rs` -- Shared `merge_metadata` helper

### Institutional Learnings

- `docs/solutions/architecture-patterns/config-key-rename-across-layers.md` -- Precedent for multi-layer rename: grep entire repo, categorize hits, work layer by layer. Rust struct field renames cause compiler errors that catch missed sites. Always run `cargo test` (test fixtures compile under `#[cfg(test)]` only).
- `docs/solutions/architecture-patterns/work-item-write-tools-orchestrator-restriction.md` -- Read tools in `default_tools()`, write tools in `management_tools_if_needed()`. Preserve split.
- `docs/solutions/architecture-patterns/completion-claim-guard-work-item-state-enforcement.md` -- Guard matches on tool name strings, not types. Must update string literals.
- `docs/solutions/best-practices/uuid-validation-at-tool-boundary.md` -- `validate_work_item()` helper called by multiple tools. Rename to `validate_task()`.
- `docs/solutions/logic-errors/builtin-skill-tool-name-shadowing.md` -- Verified: no existing skill registers `create_task`, `list_tasks`, `check_task`, or `update_task_status`.

## Key Technical Decisions

- **Compiler-driven rename**: Rename Rust structs, modules, and function names first. The compiler cascades errors to every missed call site. Run `cargo build` then `cargo test` (test fixtures only compile under `#[cfg(test)]`).
- **File renames for tool modules**: Rename the .rs files (not just contents) so module names match the new vocabulary.
- **Module rename for `work_item_metadata`**: Rename to `task_metadata` since the module is public and referenced by 4 other modules.
- **DB helper renames**: Rename `find_active_work_item_by_*`, `list_active_work_items`, `update_work_item_metadata`, `count_session_work_items`, struct field `active_work_items` -> `active_tasks`. These are internal API, no external consumers.
- **Rewind target_key prefix**: `"work_item:{id}"` -> `"task:{id}"`. Existing audit entries with the old prefix will become unmatchable for rewind lookups -- acceptable for pre-1.0 since rewind is best-effort and only looks at recent events.
- **XML tag in prompt**: `<pending-work-items>` in heartbeat injection renamed to `<pending-tasks>` for consistency.
- **Error codes**: Structured error codes like `"work_item_not_dispatchable"` -> `"task_not_dispatchable"`. These are in executor error responses consumed by the LLM, not persisted or parsed by external systems.

## Open Questions

### Resolved During Planning

- **Should we add backward-compat aliases?** No. Pre-1.0 project, clean break is appropriate per CLAUDE.md conventions and `docs/solutions/prompt-engineering/2026-04-09-tool-field-alias-for-llm-tokenization-quirks.md`.
- **Should we update historical plan/brainstorm docs?** No. They reflect vocabulary at time of writing. Solution docs ARE updated since they are active reference.
- **Should `delegate_task.rs` rename its `work_item_id` parameter?** Yes. The schema property `"work_item_id"` in `delegate_task` tool definition should become `"task_id"` for consistency. The executor's `inject_work_item_id_field()` should also become `inject_task_id_field()`.

### Deferred to Implementation

- Exact count of references in each solution doc -- will be discovered during the docs pass

## Implementation Units

- [ ] **Unit 1: Rename tool source files and update tool trait implementations**

**Goal:** Rename the four tool .rs files and update all internal names (struct, `fn name()`, `ToolDefinition.name`, function names, variable names).

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- Rename: `crates/mika-agent/src/tools/create_work_item.rs` -> `crates/mika-agent/src/tools/create_task.rs`
- Rename: `crates/mika-agent/src/tools/update_work_item_status.rs` -> `crates/mika-agent/src/tools/update_task_status.rs`
- Rename: `crates/mika-agent/src/tools/list_work_items.rs` -> `crates/mika-agent/src/tools/list_tasks.rs`
- Rename: `crates/mika-agent/src/tools/check_work_item.rs` -> `crates/mika-agent/src/tools/check_task.rs`

**Approach:**
- `git mv` each file to the new name
- In each file: rename struct (e.g., `CreateWorkItemTool` -> `CreateTaskTool`), update `fn name()` return string, update `ToolDefinition.name` string, rename local `work_item_id` variables to `task_id`, update tool description strings that cross-reference other tool names

**Patterns to follow:**
- Each tool has exactly 2 string sites for the tool name: `fn name()` and `ToolDefinition.name`
- Tool descriptions reference other tool names -- update cross-references

**Test scenarios:**
- Happy path: `cargo build` compiles with renamed files and no `work_item` in tool name strings
- Edge case: tool description strings that cross-reference other tools are updated

**Verification:**
- `rg 'create_work_item|update_work_item_status|list_work_items|check_work_item' crates/mika-agent/src/tools/` returns zero hits

- [ ] **Unit 2: Update tool registry and shared validation**

**Goal:** Update module declarations, imports, registration sites, and the shared `validate_work_item()` helper in `tools/mod.rs`.

**Requirements:** R1, R2

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/tools/mod.rs`
- Modify: `crates/mika-agent/src/tools/delegate_task.rs`

**Approach:**
- Update `mod` declarations: `mod create_work_item` -> `mod create_task`, etc.
- Update `use` paths in `default_tools()` and `management_tools_if_needed()`
- Rename `validate_work_item()` -> `validate_task()`, update its `validate_uuid("work_item_id", ...)` -> `validate_uuid("task_id", ...)`
- In `delegate_task.rs`: update schema property `"work_item_id"` -> `"task_id"` in `required` array and `properties`, update `validate_work_item()` call -> `validate_task()`

**Patterns to follow:**
- Split registration: read tools in `default_tools()`, write tools in `management_tools_if_needed()`

**Test scenarios:**
- Happy path: `cargo build` succeeds with updated module paths and registration
- Error path: `validate_task()` still returns proper error when UUID is invalid

**Verification:**
- `rg 'validate_work_item|work_item_id' crates/mika-agent/src/tools/mod.rs crates/mika-agent/src/tools/delegate_task.rs` returns zero hits

- [ ] **Unit 3: Rename work_item_metadata module**

**Goal:** Rename the module file and update all import sites.

**Requirements:** R3

**Dependencies:** None (can run parallel with Unit 1)

**Files:**
- Rename: `crates/mika-agent/src/work_item_metadata.rs` -> `crates/mika-agent/src/task_metadata.rs`
- Modify: `crates/mika-agent/src/lib.rs` (`pub mod work_item_metadata` -> `pub mod task_metadata`)
- Modify: `crates/mika-agent/src/server/verdict_handler.rs` (import path)
- Modify: `crates/mika-agent/src/server/ci_success_handler.rs` (import path)
- Modify: `crates/mika-agent/src/task_engine/dispatcher.rs` (import path)

**Approach:**
- `git mv` the file
- Update `lib.rs` module declaration
- Update all `crate::work_item_metadata::` imports -> `crate::task_metadata::`

**Test scenarios:**
- Happy path: all 4 import sites compile after rename

**Verification:**
- `rg 'work_item_metadata' crates/mika-agent/src/` returns zero hits

- [ ] **Unit 4: Update DB helpers, struct fields, and async wrappers**

**Goal:** Rename all DB method names and struct fields that use `work_item` vocabulary.

**Requirements:** R4

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/db.rs`
- Modify: `crates/mika-agent/src/async_db.rs`

**Approach:**
- Rename struct field: `HealthSummary.active_work_items` -> `active_tasks`
- Rename DB methods: `list_active_work_items` -> `list_active_tasks`, `find_active_work_item_by_ref_url` -> `find_active_task_by_ref_url`, `find_active_work_item_by_pr_url` -> `find_active_task_by_pr_url`, `find_active_work_item_by_branch` -> `find_active_task_by_branch`, `find_active_work_item_by_label` -> `find_active_task_by_label`, `count_session_work_items` -> `count_session_tasks`, `update_work_item_metadata` -> `update_task_metadata`
- Mirror all renames in `async_db.rs` wrapper methods
- Update test function names

**Patterns to follow:**
- `AsyncDatabase` wraps sync `Database` method-for-method

**Test scenarios:**
- Happy path: `cargo build` cascades compiler errors to all callers of renamed methods
- Happy path: DB tests pass after rename

**Verification:**
- `rg 'work_item' crates/mika-agent/src/db.rs crates/mika-agent/src/async_db.rs` returns zero hits in function/field names

- [ ] **Unit 5: Update agent loop, prompt, rewind, and server handlers**

**Goal:** Update all string-literal tool name references and `work_item` vocabulary in the agent loop, prompt assembly, rewind system, and server handlers.

**Requirements:** R6

**Dependencies:** Unit 4 (needs renamed DB methods)

**Files:**
- Modify: `crates/mika-agent/src/agent.rs` (completion-claim guard)
- Modify: `crates/mika-agent/src/prompt.rs` (system prompt text, heartbeat XML tags)
- Modify: `crates/mika-agent/src/rewind.rs` (match arms, target_key prefix, display strings)
- Modify: `crates/mika-agent/src/skills/executor.rs` (`validate_work_item()` calls, error codes, variable names)
- Modify: `crates/mika-agent/src/skills/index.rs` (`inject_work_item_id_field()` -> `inject_task_id_field()`)
- Modify: `crates/mika-agent/src/server/handlers.rs` (variable names, method calls)
- Modify: `crates/mika-agent/src/server/webhook_queue.rs` (struct field, method names)
- Modify: `crates/mika-agent/src/server/verdict_handler.rs` (variable names)
- Modify: `crates/mika-agent/src/server/ci_success_handler.rs` (variable names)
- Modify: `crates/mika-agent/src/test_utils.rs` (`create_test_work_item()` -> `create_test_task()`)

**Approach:**
- `agent.rs`: update tool name string literals in completion-claim guard
- `prompt.rs`: update tool name strings, rename `<pending-work-items>` -> `<pending-tasks>` XML tag, update `active_work_items` -> `active_tasks`
- `rewind.rs`: update match arms and `"work_item:"` -> `"task:"` prefix
- `executor.rs`: update `validate_work_item()` -> `validate_task()` calls, error codes
- `index.rs`: `inject_work_item_id_field()` -> `inject_task_id_field()`
- Server handlers: update variable names and method call sites

**Test scenarios:**
- Happy path: completion-claim guard still triggers on fabricated completion claims
- Happy path: rewind correctly associates `"task:{id}"` target_key
- Edge case: `inject_task_id_field()` still injects correct field name

**Verification:**
- `rg 'work_item' crates/mika-agent/src/agent.rs crates/mika-agent/src/prompt.rs crates/mika-agent/src/rewind.rs crates/mika-agent/src/skills/` returns zero hits in code

- [ ] **Unit 6: Update LLM provider tests and eval integration tests**

**Goal:** Update all test files that reference old tool names or `work_item_id`.

**Requirements:** R8

**Dependencies:** Units 1-5

**Files:**
- Modify: `crates/mika-common/src/llm/mod.rs` (test strings)
- Modify: `crates/mika-common/src/llm/openai.rs` (XML tool call extraction tests)
- Modify: `crates/mika-agent/tests/eval/test_phantom_retry_guard.rs`
- Modify: `crates/mika-agent/tests/eval/test_completion_claim_guard.rs`
- Modify: `crates/mika-agent/tests/eval/test_verdict_handler.rs`
- Modify: `crates/mika-agent/tests/eval/test_webhook_queue.rs`

**Approach:**
- Update tool name strings in test fixtures and mock responses
- Rename helper functions
- Update `work_item_id` variable names in test code
- In `openai.rs`: update `<function=list_work_items>` -> `<function=list_tasks>` in XML extraction tests

**Test scenarios:**
- Happy path: `cargo test` passes all tests
- Integration: eval tests for completion-claim guard correctly match on `"update_task_status"`

**Verification:**
- `cargo test` passes with zero failures

- [ ] **Unit 7: Update bundled skill prompts and tools.json**

**Goal:** Update all bundled skill files that reference work_item vocabulary.

**Requirements:** R5

**Dependencies:** None (can run parallel with Rust units)

**Files:**
- Modify: `skills/bundled/self-dev/system_prompt.md` (~33 refs)
- Modify: `skills/bundled/self-dev-webhook-qa/system_prompt.md` (~5 refs)
- Modify: `skills/bundled/permission-policy/system_prompt.md` (~2 refs)
- Modify: `skills/bundled/address-pr-comments/system_prompt.md` (~1 ref)
- Modify: `skills/bundled/claude-pilot/system_prompt.md` (~1 ref)
- Modify: `skills/bundled/resolve-pr-conflicts/system_prompt.md` (~1 ref)
- Modify: `skills/bundled/self-dev-iterate/system_prompt.md` (~1 ref)
- Modify: `skills/bundled/self-dev-webhook-ci/system_prompt.md` (~1 ref)
- Modify: `skills/bundled/resolve-pr-conflicts/tools.json`
- Modify: `skills/bundled/address-pr-comments/tools.json`
- Modify: `skills/bundled/claude-pilot/tools.json`

**Approach:**
- Systematic find-and-replace: `create_work_item` -> `create_task`, `update_work_item_status` -> `update_task_status`, `list_work_items` -> `list_tasks`, `check_work_item` -> `check_task`, `work_item_id` -> `task_id`, `work item` -> `task`
- Validate JSON files after edits

**Test scenarios:**
- Happy path: `rg 'work_item' skills/bundled/` returns zero hits
- Edge case: JSON files remain valid after edits

**Verification:**
- `rg 'work_item' skills/bundled/` returns zero hits

- [ ] **Unit 8: Update documentation**

**Goal:** Update developer docs and solution docs to reflect new vocabulary.

**Requirements:** R7

**Dependencies:** None (can run parallel)

**Files:**
- Modify: `crates/mika-agent/CLAUDE.md` (~8 refs)
- Modify: `docs/task-system.md`
- Modify: `docs/architecture.md`
- Modify: Solution docs in `docs/solutions/` that reference work_item tool names (~10 files)

**Approach:**
- Update tool names, function names, and file paths in all documentation
- Run `scripts/sync-agent-docs.sh` to sync crate-local doc copies
- Do NOT update historical plan docs or brainstorm docs

**Test scenarios:**
Test expectation: none -- documentation-only changes

**Verification:**
- `rg 'create_work_item|update_work_item_status|list_work_items|check_work_item|validate_work_item' docs/ crates/mika-agent/CLAUDE.md` returns zero hits in active docs

- [ ] **Unit 9: Build verification and final sweep**

**Goal:** Full build + test + comprehensive grep to catch any missed references.

**Requirements:** R8

**Dependencies:** Units 1-8

**Approach:**
- `cargo build` -- verify clean compilation
- `cargo test` -- verify all tests pass
- `cargo clippy` -- verify no new warnings
- Comprehensive `rg 'work_item'` sweep across Rust source, skills, and active docs
- Fix any remaining hits

**Test scenarios:**
- Happy path: `cargo test` passes all tests with zero failures
- Happy path: comprehensive grep sweep returns zero hits in active code and docs

**Verification:**
- Clean `cargo build && cargo test && cargo clippy`
- Zero `work_item` hits in Rust source, bundled skills, and active docs

## System-Wide Impact

- **Interaction graph:** Tool dispatch chain resolves by name string. All string-literal sites must be updated: completion-claim guard, rewind match arms, prompt assembly, executor validation, skill prompt references.
- **Error propagation:** Structured error codes change (`work_item_not_dispatchable` -> `task_not_dispatchable`). Not persisted or parsed by external systems.
- **State lifecycle risks:** Existing `audit_events` rows with `"work_item:{id}"` target_key become unmatchable for rewind. Acceptable pre-1.0.
- **API surface parity:** Dashboard already uses task vocabulary. No dashboard changes needed.
- **Integration coverage:** Eval tests verify full agent loop path with renamed tools.
- **Unchanged invariants:** DB schema, task engine, CLI, and dashboard DTOs already use task vocabulary.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Missed string-literal reference causes silent tool dispatch failure | Compiler catches struct/module renames; comprehensive `rg` sweep catches string literals |
| Rewind breaks for existing audit events | Acceptable pre-1.0; rewind is best-effort |
| Community skills reference old tool names | Separate PR; bundled skills are atomic |
| Agent core memory references old tool names | Post-deploy audit |

## Documentation / Operational Notes

- Post-deploy: audit agent identity files and `self_model` core memory for old tool name references
- Post-deploy: update community skills in `mika-skills/` (separate PR)
- Run `scripts/sync-agent-docs.sh` before merging to sync crate-local doc copies

## Sources & References

- Related issue: #608
- Related PRs: #595 (tasks.type column), #601 (executor task_id standardization)
- Learnings: `docs/solutions/architecture-patterns/config-key-rename-across-layers.md`
- Learnings: `docs/solutions/architecture-patterns/work-item-write-tools-orchestrator-restriction.md`
- Learnings: `docs/solutions/architecture-patterns/completion-claim-guard-work-item-state-enforcement.md`
- Learnings: `docs/solutions/best-practices/uuid-validation-at-tool-boundary.md`
