# Required Tools Enforcement Gate

**Issue:** #270
**Category:** prompt-engineering, agent-core
**Date:** 2026-03-25

## Problem

The agent engine accepts any assistant response regardless of whether required tool calls were made. A skill can define a multi-step review process with mandatory tool calls (e.g., qa-review requiring `run_gh` to fetch PR data), but the engine has no mechanism to enforce that those calls actually happen. The model can shortcut the entire process by fabricating results — including fabricated `<context type="tool_history">` blocks.

This is distinct from the grounding guardrail (#266) which prevents hallucination *after* real tool results. This failure mode is the model never calling tools at all.

## Solution

### Skill-Declared Required Tools

Skills declare required tool calls in `skill.toml` via a new `[constraints]` section:

```toml
[constraints]
required_tools = ["run_gh"]
```

### Engine Enforcement in `run_loop()`

1. `collect_required_tools()` computes the union of all required_tools from matched skills
2. `run_loop()` tracks `tools_called: HashSet<String>` across all steps
3. On `EndTurn` with non-empty text, if any required tools haven't been called:
   - The response is rejected
   - A correction message is injected listing the missing tools
   - The agent is re-prompted (counts against max_steps)
   - Only one retry is allowed to prevent infinite loops

### Design Decisions

- **Only on `EndTurn`**: `MaxTokens` and `ContentFilter` are unrecoverable — re-prompting won't help if the context window is full or content was filtered.
- **Single retry**: One correction attempt. If the model still doesn't call the tools, the response is accepted (best-effort, not a guarantee). The retry counts against max_steps.
- **Union semantics**: Required tools from multiple matched skills are unioned. If skill A requires `run_gh` and skill B requires `run_lint`, both must be called.
- **Name-based tracking**: Tools are tracked by name, not by skill. A required tool can be satisfied by any call to that tool name, regardless of which skill declared it.
- **Silent mode excluded**: Silent mode (heartbeat/reflection) passes an empty required set since `safe_always_on_skills()` only loads builtin-handler skills which don't declare constraints.

## Key Files

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/manifest.rs` | `Constraints` struct, added to `SkillManifest` |
| `crates/mika-agent/src/agent.rs` | `collect_required_tools()`, `run_loop()` enforcement |
| `crates/mika-agent/src/skills/index.rs` | `validate_skill()` warns on unknown required tool names |

## Correction Message

When the enforcement triggers, the injected message is:

```
[Your response was rejected because you did not call the required tool(s): {missing_names}.
You MUST call these tools with real data before producing your response. Do not fabricate
or assume results — call the tools now.]
```

## Terminal Failure Bypass (#516)

When a required tool was called and failed with a terminal (unrecoverable) error, the gate allows EndTurn without retry. This prevents wasting LLM calls re-running a workflow that will hit the same wall.

Detection uses two pattern lists checked against `output_summary` (300 chars, case-insensitive):
1. `RETRYABLE_ERROR_PATTERNS` — checked first, takes priority (HTTP 429/5xx, rate limits, timeouts, connection errors)
2. `TERMINAL_ERROR_PATTERNS` — GitHub self-approval, HTTP 4xx, permission errors

`has_terminal_required_tool_failure()` scans `all_tool_summaries` for any required tool with `success == false` and a terminal output pattern. If found, the gate logs a warning and falls through to the next guard instead of re-prompting.

Unknown errors (matching neither list) default to retryable — the conservative path preserves existing retry behavior.

## Limitations

- **Best-effort**: If the model ignores the correction, the second response is accepted regardless. The feature is a guardrail, not a hard constraint.
- **Step budget**: The retry consumes a loop step. A skill requiring tools at step 9 of 10 might not have room.
- **No input validation**: The enforcement checks that the tool was *called*, not that the inputs or outputs are valid.
- **Terminal detection scope**: Pattern matching operates on the 300-char `output_summary`, not the full tool output. Long preambles before the error text may cause missed detection (conservative fallback: retry).
- **Cross-step name collisions**: A tool that failed terminally in an earlier step but succeeded later will still match. Bounded by the once-only `required_tools_retry_done` flag.
