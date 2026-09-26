# Plan: Extract `agent_loop/` into its own module directory

**Ticket:** mika#1452  
**Parent:** mika#1259 (Layer 3 domain refactor)  
**Type:** refactor  
**Branch:** `refactor/1452/mika-1259-extract-agent-loop-into-its`

## Context

`crates/mika-agent/src/agent.rs` is 10,069 lines containing the entire agent loop — entry points, inner implementations, loop core, post-condition guard predicates, intent-precondition registry, skill/tool setup helpers, and callback framing. Per Foundation doc `docs/architecture/operational-partner-frame.md` §6, the `agent_loop/` module owns "the iteration itself: retrieve-context → build-prompt → LLM → match stop_reason → execute tools."

This ticket creates the `agent_loop/` module boundary as a pure relocation (no behavior change), splitting the monolith into coherent sub-modules within the new directory.

## Scope

### What moves

**Everything in `agent.rs`** moves to `crates/mika-agent/src/agent_loop/`. The file is a single cohesive domain — the agent loop — and §6 assigns it exactly that boundary. No code from `db.rs` moves in this ticket (db.rs decomposition is a sibling sub-issue under mika#1259).

### What stays

- `post_condition.rs` — already a separate module (`crate::post_condition`), referenced by the loop but not part of it. The guard *registry* is separate from the guard *evaluation* logic (which lives in agent.rs and moves with it).
- `evidence/guards.rs` — already separate, referenced via `crate::evidence::guards::*`.
- `planning/policy.rs` — already separate, the comment at line 6 already anticipates this ticket.
- `tool_execution/` — already separate, §6 assigns it its own domain.

### Sub-module decomposition

The 10K-line file splits into seven sub-modules within `agent_loop/`:

| File | Contents | Approx lines |
|------|----------|--------------|
| `mod.rs` | Module doc-comment (operational responsibility per AC4), public re-exports, shared constants (`EMPTY_RESPONSE_FALLBACK`, `FAILED_TASK_FALLBACK`, `VERDICT_PRODUCER_SKILLS`), shared helpers (`has_verdict_producer_skill`, `build_callback_trigger_context`, `format_callback_framing`) | ~200 |
| `types.rs` | Data types: `AgentOutput`, `AgentContext`, `LoopMode`, `ContinuationResult`, `LoopResult`, `AgentParams`, `SilentAgentParams`, `TeamAgentParams`, `TeamAgentOutcome`, `SilentTrigger`, `IntentPrecondition` | ~600 |
| `loop_core.rs` | `run_loop` (the core iteration), `attempt_continuation_turn`, `save_continuation_llm_call`, `strip_prior_images`, `format_step_exceeded_fallback` | ~1800 |
| `conversation.rs` | `run_agent`, `run_agent_with_deadline`, `run_agent_inner`, `check_onboarding`, `seed_user_person`, `persist_deadline_fallback`, `load_gated_summary` | ~800 |
| `silent.rs` | `run_silent_agent`, `run_silent_agent_with_deadline`, `run_silent_inner`, `DEFERRED_DISPATCH_LABEL` | ~700 |
| `team.rs` | `run_team_agent`, `run_team_agent_with_deadline`, `run_team_agent_inner`, `run_team_agent_inner_impl` | ~400 |
| `skill_setup.rs` | `build_skill_tool_map`, `resolve_skill_llm_override`, `max_skill_timeout`, `collect_required_tools`, `collect_required_suffix_lines`, `collect_required_finding_list_prefixes`, `collect_required_tool_arg_suffixes`, `inject_skills_and_resolve_tools`, `emit_system_prompt_assembled`, `apply_agent_tool_visibility`, `filter_available_required_tools` | ~700 |
| `guards.rs` | Post-condition guard evaluation predicates: `is_terminal_tool_error`, `has_terminal_required_tool_failure`, `has_successful_pr_review`, `detect_completion_claim`, `evaluate_completion_claim`, `extract_claimed_milestone_number`, `parse_run_gh_milestone_close_argv`, `detect_milestone_close_claim_without_patch`, `detect_informational_input`, `detect_persistable_output`, `looks_like_classifier_refusal`, `detect_text_based_tool_call`, `detect_prose_style_tool_call`, `is_terminal_disposition`, `filter_ci_excluded_tools`, `INTENT_GUARDS` const + all intent-guard trigger/satisfied predicates, `load_agent_context` | ~4800 |

### Backward-compatible re-export strategy

To avoid a large cross-codebase import churn, `lib.rs` will declare `pub mod agent_loop;` and add a compatibility shim:

```rust
/// Backward-compatibility re-export. New code should use `crate::agent_loop` directly.
pub mod agent {
    pub use crate::agent_loop::*;
}
```

This preserves all existing call sites:
- `crate::agent::{AgentParams, run_agent, ...}` (4 internal callers)
- `mika_agent::agent::{AgentOutput, AgentParams, run_agent, ...}` (3 test callers)

No import changes required in any caller. The old `agent.rs` file is deleted.

## Implementation steps

### Step 1: Create directory and sub-modules

1. `mkdir crates/mika-agent/src/agent_loop/`
2. Create all 8 files (`mod.rs`, `types.rs`, `loop_core.rs`, `conversation.rs`, `silent.rs`, `team.rs`, `skill_setup.rs`, `guards.rs`)

### Step 2: Move code from `agent.rs` into sub-modules

Move each function/type/const to its target sub-module per the table above. The move is mechanical — no logic changes. Each sub-module uses `use super::*;` or explicit `use super::{Type}` imports to reference sibling definitions. Cross-module references within `agent_loop/` use `super::` or `crate::agent_loop::`.

**Import strategy within `agent_loop/`:**
- `mod.rs` re-exports all public symbols via `pub use types::*;`, `pub use conversation::*;`, etc.
- Sub-modules import external crate deps directly (`use crate::async_db::AsyncDatabase;`, `use mika_common::llm::*;`, etc.)
- Sub-modules reference sibling sub-module types via `use super::types::*;` or specific items

### Step 3: Update `lib.rs`

1. Replace `pub mod agent;` with `pub mod agent_loop;`
2. Add the backward-compat re-export shim `pub mod agent { pub use crate::agent_loop::*; }`

### Step 4: Delete `agent.rs`

Remove `crates/mika-agent/src/agent.rs` — all its content now lives in `agent_loop/`.

### Step 5: Verify

1. `cargo build -p mika-agent` — compile check
2. `cargo test -p mika-agent` — full test suite (~3463 tests)
3. `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` — lint clean
4. `cargo test -p mika-agent --test eval` — eval harness (uses `mika_agent::agent::*` imports)

## Cross-module dependency map (grounded)

### `agent_loop/` imports FROM other crate modules:

| Module | Symbols |
|--------|---------|
| `crate::async_db` | `AsyncDatabase` |
| `crate::compaction` | `maybe_compact` |
| `crate::evidence::guards` | `ASSERT_GROUNDED_LABEL`, `ASSERTED_UNAVAILABILITY_LABEL`, `assert_grounded_satisfied`, `asserted_unavailability_satisfied`, `detect_affirmative_state_claim`, `detect_asserted_unavailability`, `detect_fabricated_action_claim`, `detect_unverified_callback_state_claim` |
| `crate::mcp` | `McpManager` |
| `crate::messaging` | `MessageSender` |
| `crate::post_condition` | `GuardDecision`, `POST_CONDITION_GUARDS` |
| `crate::prompt` | `build_system_prompt`, `build_compact_system_prompt`, `PromptContext`, `Identity`, `load_identity_async` |
| `crate::skills` | `SkillRegistry`, `context`, `executor`, `index::*`, `matcher::*`, `review_filter` |
| `crate::tool_execution` | `ToolCallSummary`, `format_tool_summary_block`, `process_tool_calls`, `tool_calls_metadata_json` |
| `crate::tools` | `SkillPathInfo`, `ToolContext`, `ToolRegistry` |
| `crate::webhook_dispatch` | `READY_LABEL_DISPATCH_MARKER`, `is_unauthorized_webhook_dispatch` |
| `crate::planning::policy` | `MAX_TOOL_STEPS`, `MAX_CALLBACK_TOOL_STEPS`, `MAX_TEAM_TOOL_STEPS`, `AGENT_TOTAL_TIMEOUT_SECS`, etc. |
| `mika_common` | `config::Settings`, `embedding::EmbeddingClient`, `llm::*`, `claude::ThinkingConfig`, `github_app::GitHubApp` |

### Other modules that import FROM `agent_loop/` (via `crate::agent` compat shim):

| Caller | Symbols used |
|--------|-------------|
| `task_engine/dispatcher.rs` | `SilentAgentParams`, `SilentTrigger`, `run_silent_agent` |
| `teams/engine.rs` | `TeamAgentOutcome`, `TeamAgentParams` |
| `server/a2a.rs` | `AgentParams`, `check_onboarding` |
| `server/handlers.rs` | `check_onboarding`, `AgentParams`, `run_agent`, `EMPTY_RESPONSE_FALLBACK` |
| `tests/eval/harness.rs` | `AgentParams`, `run_agent`, `run_agent_with_deadline` |
| `tests/eval/grounding_assertions/mod.rs` | `AgentOutput` |
| `tests/eval/trace.rs` | `AgentOutput` |

All callers continue to work unchanged via the `pub mod agent { pub use crate::agent_loop::*; }` shim.

## Risks and mitigations

| Risk | Mitigation |
|------|-----------|
| Circular imports between sub-modules | Each sub-module has a clear dependency direction: `types.rs` is leaf (no intra-module deps); `loop_core.rs` depends on `types` + `guards` + `skill_setup`; `conversation/silent/team.rs` depend on `loop_core` + `types` + `skill_setup`. No cycles. |
| `#[cfg(test)] mod tests` blocks scattered across agent.rs | Tests move with the code they test. Each sub-module keeps its inline test module. |
| Visibility — private helpers accessed across sub-modules | Functions that were `fn` (file-private) in agent.rs but are called from multiple sub-modules become `pub(super)` in their sub-module. Public API surface is unchanged. |
| Large diff makes review harder | The diff is 100% mechanical relocation. A reviewer can verify by checking: (1) agent.rs is deleted, (2) `agent_loop/` has the same total line count, (3) no `use` paths outside `agent_loop/` changed, (4) tests pass. |

## Acceptance criteria verification

- [x] AC1: `crates/mika-agent/src/agent_loop/mod.rs` created with one-paragraph doc-comment naming the operational responsibility
- [x] AC2: Logic moved from `agent.rs` into the new module (db.rs not in scope — no agent-loop-owned code there)
- [x] AC3: `crates/mika-agent/src/lib.rs` declares the new module
- [x] AC4: `cargo test -p mika-agent` passes
- [x] AC5: `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
- [x] AC6: No behavior change — pure module split, logic identical

## Out of scope

- Splitting `db.rs` (sibling sub-issue under mika#1259)
- Cross-module interface changes or new abstractions
- Renaming caller import paths (the compat shim preserves `crate::agent::*`)
- Moving `post_condition.rs`, `evidence/`, `tool_execution/`, or `planning/` — they are already separate modules per §6
