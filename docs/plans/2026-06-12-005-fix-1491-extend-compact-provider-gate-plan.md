# Plan: Extend ProviderKind::MikaModel compact gate to skills + tools array

**Ticket:** mika#1491
**Type:** fix (bug — compact gate incomplete, prompt still 167K post-merge of mika#1398)
**Branch:** `fix/1491/llm-extend-providerkind-mikamodel`

## Problem Summary

mika#1398 shipped the compact base system prompt for `ProviderKind::MikaModel` (≤5KB from `build_compact_system_prompt()`). But the gate only swaps the BASE prompt — the agent loop still appends all active skill prompts (~96KB for mika-dev) and the full 58-tool catalog unfiltered. Empirical verification shows the system prompt is still 167K chars and 58 tools post-#1398.

## Design Decisions

### D1: Skill prompt filtering strategy — skip all skills for compact providers

When `is_compact_provider == true`, **zero skills** are concatenated to the system prompt. Rationale: MikaModel's small context window cannot accommodate any skill prompts. The compact prompt builder's docstring already states ≤5KB total; even one skill (self-dev = 54KB) would blow the budget. If a future MikaModel version handles more context, we can revisit with a skill allowlist.

### D2: Tool filtering strategy — static core-tool allowlist

When `is_compact_provider == true`, the tools array is filtered to a static allowlist of ≤10 core tools. The allowlist is defined as a `const` slice in `agent.rs`:

```rust
const COMPACT_PROVIDER_CORE_TOOLS: &[&str] = &[
    "send_message",
    "update_core_memory",
    "store_fact",
    "search_memory",
    "update_fact",
    "create_reminder",
    "list_reminders",
    "read_agent_file",
    "write_agent_file",
    "list_agent_files",
];
```

These are the fundamental agent capabilities: communication, memory, facts, reminders, and file access. No management tools, no task tools, no skill-defined tools, no KG tools, no PR tools. This matches the ticket's out-of-scope proposal.

Skill-defined tools are also excluded since skills are not loaded (D1).

### D3: Threading approach — pass `is_compact_provider` into `inject_skills_and_resolve_tools`

Add `is_compact_provider: bool` parameter to `inject_skills_and_resolve_tools()`. When `true`:
- Skip the skill-prompt-concatenation loop entirely (no `write!` to `system`)
- Filter `tool_defs` to only names in `COMPACT_PROVIDER_CORE_TOOLS`
- Skip skill-tool collection (no skill tools added)
- `per_skill_bytes` map stays empty (correct for observability)

This is cleaner than filtering at each of the three request-construction sites because the function already owns both skill injection and tool assembly.

### D4: Observability — add `is_compact_provider` to `system_prompt_assembled` log event

Thread `is_compact_provider: bool` into `emit_system_prompt_assembled()` and add it as a field on the structured log event. Also add `tool_count: usize` for the tools-array dimension. This satisfies AC5.

## Implementation Steps

### Step 1: Add `COMPACT_PROVIDER_CORE_TOOLS` constant

**File:** `crates/mika-agent/src/agent.rs`
**Location:** Near existing constants (e.g., near `apply_agent_tool_visibility`)

Add a `const COMPACT_PROVIDER_CORE_TOOLS: &[&str]` with the 10 core tool names from D2.

### Step 2: Thread `is_compact_provider` into `inject_skills_and_resolve_tools`

**File:** `crates/mika-agent/src/agent.rs:4671`

Add `is_compact_provider: bool` as the last parameter. When `true`:
1. After `apply_agent_tool_visibility`, filter `tool_defs` to retain only names in `COMPACT_PROVIDER_CORE_TOOLS` (case-insensitive, matching existing convention in `apply_agent_tool_visibility`)
2. Skip the `for entry in matched { ... }` loop body — no skill prompts appended, no skill tools added
3. Return early with the filtered tool_defs, `None` prompt_variant, and empty `per_skill_bytes`

When `false`: no behavior change (current code path).

### Step 3: Update all three call sites of `inject_skills_and_resolve_tools`

Pass the already-computed `is_compact_provider` local variable:

- **Conversation mode** (`agent.rs:~2553`): `inject_skills_and_resolve_tools(..., is_compact_provider)`
- **Silent mode** (`agent.rs:~3407`): pass `false` — silent mode is engine-driven (heartbeat, callback, etc.) and MikaModel agents won't receive silent triggers in current architecture. Keeping it `false` avoids surprising behavior and satisfies AC3.
- **Team mode** (search for third call site): pass `false` for the same reason.

### Step 4: Add `is_compact_provider` and `tool_count` to `emit_system_prompt_assembled`

**File:** `crates/mika-agent/src/agent.rs:4749`

1. Add `is_compact_provider: bool` and `tool_count: usize` parameters
2. Add `is_compact_provider = is_compact_provider` and `tool_count = tool_count` fields to the `info!()` macro call
3. Update all three emission call sites to pass the new arguments:
   - Conversation mode: pass actual `is_compact_provider` and `skill_tool_defs.len()`
   - Silent mode: pass `false` and actual tool count
   - Team mode: pass `false` and actual tool count

### Step 5: Unit tests

**File:** `crates/mika-agent/src/agent.rs` (inline `#[cfg(test)] mod tests`)

Add tests:

1. **`test_inject_skills_compact_provider_filters_tools`**: Build a `ToolRegistry` with a mix of core and non-core tools. Call `inject_skills_and_resolve_tools` with `is_compact_provider = true`, matched skills with prompts. Assert:
   - `system` string has no `<context type="skill"` blocks
   - Returned tool_defs only contain names from `COMPACT_PROVIDER_CORE_TOOLS`
   - `per_skill_bytes` is empty

2. **`test_inject_skills_non_compact_provider_unchanged`**: Same setup with `is_compact_provider = false`. Assert behavior matches current: skills appended, all tools present.

3. **`test_compact_provider_core_tools_are_valid`**: Assert every name in `COMPACT_PROVIDER_CORE_TOOLS` exists in the default `ToolRegistry` from `default_tools()`. Prevents drift.

### Step 6: Integration test (eval harness)

**File:** `crates/mika-agent/tests/eval/` — new scenario or extend existing

Add an `EvalHarness` test using `MockLlmProvider` that:
1. Configures the agent with `ProviderKind::MikaModel`
2. Sends a simple message
3. Captures the LLM request via the mock
4. Asserts: system prompt ≤5KB, tools array ≤10 entries (AC1, AC2)

This satisfies AC4 as a programmatic regression gate (the ticket mentions `MIKA_OLLAMA_DUMP_PAYLOAD` but that's a manual verification tool; the eval harness provides CI-gated assertion).

## Affected Files

| File | Change |
|------|--------|
| `crates/mika-agent/src/agent.rs` | `COMPACT_PROVIDER_CORE_TOOLS` const, `inject_skills_and_resolve_tools` signature + compact gate, `emit_system_prompt_assembled` new fields, 3 call-site updates for each function, unit tests |
| `crates/mika-agent/tests/eval/` | Integration test for compact provider request shape |

## AC Traceability

| AC | Step | Mechanism |
|----|------|-----------|
| AC1: system ≤5KB | Steps 2-3 | `build_compact_system_prompt` (≤5KB base) + zero skills appended |
| AC2: tools ≤10 | Steps 1-3 | `COMPACT_PROVIDER_CORE_TOOLS` allowlist filter in `inject_skills_and_resolve_tools` |
| AC3: non-MikaModel byte-identical | Steps 2-3 | `is_compact_provider = false` → no behavior change; silent/team modes always pass `false` |
| AC4: regression test | Steps 5-6 | Unit test on tool filtering + eval harness on full request shape |
| AC5: observability field | Step 4 | `is_compact_provider` field on `system_prompt_assembled` log event |

## Risks

- **Tool name drift:** If a tool in `COMPACT_PROVIDER_CORE_TOOLS` is renamed or removed, the allowlist silently shrinks. Mitigated by Step 5 test 3 (`test_compact_provider_core_tools_are_valid`).
- **Silent/team mode hardcoded `false`:** If MikaModel is ever used for silent triggers, the full prompt + tools will be sent. Acceptable for now — MikaModel is a general-agent provider, not a dispatch target. Can be extended when needed.
