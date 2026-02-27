---
title: "Agent loop refactoring: Extract shared logic from three duplicated loop variants"
date: 2026-02-27
category: refactoring
module: "crates/mika-agent/src/agent.rs"
tags: [code-duplication, agent-loop, loopmode-enum, maintainability, tracing, multi-agent-review]
severity: medium
resolved: true
pr_number: 24
---

# Agent Loop Variant Extraction and Deduplication

## Problem

Three agent loop variants in `crates/mika-agent/src/agent.rs` duplicated approximately 70% of their code:

- **`run_agent_inner`** — Standard conversation loop (CLI and server message handling)
- **`run_silent_inner`** — Background tasks (heartbeat check-ins, reminder delivery)
- **`run_team_agent_inner`** — Sub-agent execution within a team context

Each independently implemented: soul/identity file loading, core memory retrieval, timezone config fetching, `ToolContext` construction, `MessagesRequest` building, and the entire tool-step dispatch loop (iterate up to `MAX_TOOL_STEPS`, match on `StopReason`, handle `ToolUse` vs `EndTurn`, inject follow-up prompts on empty responses).

**Impact:** Bug fixes to any shared concern required application in three separate locations, increasing divergence risk.

## Root Cause

Organic growth. Silent mode was added as a copy of the conversation loop with minor prompt/return changes. Team mode was then added as a copy of both. No shared abstraction existed because each variant was written independently as needs arose.

## Solution

Four new internal abstractions (all module-private), plus two review-driven fixes.

### 1. `AgentContext` + `load_agent_context()`

Replaces duplicated soul/identity/core_memory/timezone loading in all three `_inner` functions:

```rust
struct AgentContext {
    soul_content: String,
    identity: prompt::Identity,
    core_memory: Vec<crate::db::CoreMemoryEntry>,
    timezone: Option<String>,
}

async fn load_agent_context(db: &AsyncDatabase, home_dir: &Path) -> Result<AgentContext> {
    let soul_content = tokio::fs::read_to_string(home_dir.join("soul.md"))
        .await
        .unwrap_or_default();
    let identity = prompt::load_identity_async(home_dir).await;
    let core_memory = db.get_all_core_memory().await?;
    let timezone = db.get_customer_config("timezone").await?;
    Ok(AgentContext { soul_content, identity, core_memory, timezone })
}
```

### 2. `LoopMode` enum

Parameterizes the behavioral dimensions that differ between loop variants:

```rust
enum LoopMode<'a> {
    Conversation { channel_type: &'a str },
    Silent { channel_type: &'a str },
    Team,
}
```

#### Behavioral Matrix

| Dimension              | Conversation         | Silent               | Team            |
|------------------------|----------------------|----------------------|-----------------|
| `is_conversation()`    | **true**             | false                | false           |
| `follow_up_on_empty()` | **true**             | false                | **true**        |
| `channel_type()`       | `Some(channel_type)` | `Some(channel_type)` | `None`          |
| `label()`              | `"agent"`            | `"silent agent"`     | `"team agent"`  |

Effects within `run_loop`:
- **`is_conversation()`** — Controls usage tracking (`last_usage = Some(response.usage.clone())`) and thinking capture (`thinking_text = response.thinking()` on step 0).
- **`follow_up_on_empty()`** — Controls whether the loop injects `"[Briefly confirm what you just did.]"` after tool-use turns with no text. Silent skips this; Conversation and Team use it.
- **`channel_type()`** — When `Some`, assistant text is persisted via `db.save_message()`. When `None` (Team), no messages are saved.

### 3. `LoopResult` struct

Unified return type — callers inspect only the fields relevant to their mode:

```rust
struct LoopResult {
    text: Option<String>,
    thinking: Option<String>,
    usage: Option<mika_common::claude::Usage>,
    max_steps_exceeded: bool,
}
```

### 4. `run_loop()` — Shared tool-step loop

Single function replaces three independent `for step in 0..MAX_TOOL_STEPS` loops. Each `_inner` function became a thin dispatcher:

```rust
// Conversation
let mode = LoopMode::Conversation { channel_type };
let result = run_loop(claude, tools, &skill_tool_map, skill_timeout,
                      &tool_ctx, &mut request, &mode, db).await?;
if result.max_steps_exceeded { /* save fallback to DB */ }
Ok(AgentOutput { text: result.text, thinking: result.thinking, usage: result.usage })

// Silent
let mode = LoopMode::Silent { channel_type };
run_loop(claude, tools, &skill_tool_map, skill_timeout,
         &tool_ctx, &mut request, &mode, db).await?;
Ok(())

// Team
let mode = LoopMode::Team;
let result = run_loop(claude, tools, &skill_tool_map, skill_timeout,
                      &tool_ctx, &mut request, &mode, params.db).await?;
if result.max_steps_exceeded { return Ok(Some("Agent exceeded maximum tool steps.".into())); }
Ok(result.text)
```

### 5. Review-driven fixes

A 7-agent code review (architecture, patterns, performance, security, simplicity, agent-native, learnings) caught two issues:

1. **Merged duplicate methods:** `capture_thinking()` and `track_usage()` had identical implementations (`matches!(self, Self::Conversation { .. })`). Merged into single `is_conversation()` method.

2. **Restored `channel_type` in tracing:** The original silent loop logged `info!(step, channel_type, "silent agent done")`. The refactored `run_loop` initially lost this field. Fixed by extracting `channel_type` once at the top of the loop and threading it through all tracing macros:

```rust
let channel_type = mode.channel_type();
debug!(step, label = mode.label(), channel_type, messages_len = request.messages.len(), "agent loop step");
```

## Verification

- All 475 tests pass
- Clippy clean on agent.rs
- Public API unchanged: `run_agent`, `run_silent_agent`, `run_team_agent` signatures and return types identical
- No caller modifications needed (CLI, server, scheduler, teams/engine.rs)

## Prevention Strategies

### Extending with new agent modes

When adding a fourth variant (e.g., `Batch`), extend the `LoopMode` enum — never copy `run_loop`:

1. Add the variant to the enum
2. Update each method in `impl LoopMode<'_>` for the new variant
3. Create a thin `run_batch_inner` dispatcher that calls `run_loop`

### The "3+ call sites" calibration rule

From prior code review decisions (documented in `multi-agent-pr-review-v3-synthesis.md`):
- **<3 call sites:** Accept duplication — abstraction cost outweighs benefit
- **3+ call sites:** Extract — duplication has proven itself across multiple contexts

This refactoring had exactly 3 call sites for both `load_agent_context` and `run_loop`, meeting the threshold.

### Audit structured logging during refactoring

Before extracting a shared function, list all `tracing` macros and their structured fields. After refactoring, verify fields survive by checking log output. The `channel_type` field is critical for filtering heartbeat vs. reminder vs. CLI logs in production.

## Lessons Learned

1. **Identical method bodies are a code smell even in newly-written code.** The simplicity reviewer caught `capture_thinking()`/`track_usage()` duplication that the author missed.

2. **Structured logging fields carry production observability value.** The architecture reviewer caught the `channel_type` regression that would have degraded log filtering in production.

3. **Multi-agent review catches complementary issues.** Neither reviewer would have caught both findings alone — the simplicity reviewer focused on code redundancy, the architecture reviewer on observability. Running them in parallel with different mandates yields broader coverage.

4. **Line count is not the metric.** The file grew from 842 to ~910 lines (+72 net). The value is in deduplication of logic — one place to fix bugs instead of three — not raw line reduction.

## Related Documentation

- [PR #24: refactor: extract shared logic from duplicated agent loops](https://github.com/senara-solutions/mika/pull/24)
- [Todo 078: Extract shared logic from duplicated agent loops](../../todos/078-complete-p2-agent-loop-duplication.md) — Status: complete
- [Todo 086: Code quality improvements from review](../../todos/086-ready-p3-code-quality-improvements.md) — Status: ready (includes item #3: no agent loop integration tests)
- [AsyncDatabase wrapper pattern](../architecture/async-database-wrapper-pattern.md) — Documents the closure-based dispatch pattern used in ToolContext
- [Strip field-level encryption refactor](./strip-field-level-encryption-refactor.md) — Prior large-scale refactoring (27 call sites) with similar methodology
- [Multi-agent teams orchestration](../architecture-decisions/multi-agent-teams-orchestration-hub-spoke.md) — Documents the team agent variant addressed by LoopMode::Team
- [Multi-agent PR review v3 synthesis](../code-review-workflow/multi-agent-pr-review-v3-synthesis.md) — The "3+ call sites" calibration rule origin
