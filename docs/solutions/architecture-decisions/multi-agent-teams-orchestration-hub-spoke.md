---
title: "Multi-Agent Teams Orchestration System"
date: 2026-02-25
category: architecture-decisions
severity: feature
component: mika-agent/teams
related_modules:
  - mika-agent/teams/engine
  - mika-agent/teams/prompt
  - mika-agent/teams/types
  - mika-agent/teams/history
  - mika-agent/tools/read_workspace
  - mika-agent/tools/write_workspace
  - mika-agent/tools/list_workspace
  - mika-common/team
  - mika-cli/commands/teams
tags:
  - architecture
  - multi-agent
  - orchestration
  - hub-spoke
  - workspace-tools
  - team-execution
related_issues:
  - PR #13 (feat/teams-system)
  - PR #12 (feat/multi-agent-support)
  - PR #11 (feat/layer3-vector-search)
  - 18 code review findings (#248-#265)
---

# Multi-Agent Teams Orchestration System

## Problem

Mika supported multiple independent agents (each with their own DB, memory, skills, and personality), but had no mechanism for agents to collaborate on complex goals. There was no way to decompose a goal across specialists, share intermediate work products, review quality, or iterate toward a deliverable. Each agent operated in isolation.

## Investigation Steps

1. Studied existing multi-agent patterns in reference repositories (OpenClaw's hub-and-spoke gateway, LettaBot's memory hierarchy).
2. Evaluated framework approaches (LangGraph, CrewAI) vs. explicit Rust implementation. Chose explicit Rust for consistency with Mika's "no framework" philosophy.
3. Prototyped TOML-based team definitions to avoid database schema changes and keep teams filesystem-portable.
4. Designed workspace tools as the inter-agent communication channel (files, not messages) to maintain clean separation between agents.
5. Implemented the full orchestration engine and ran 18-finding code review to harden security and quality.

## Root Cause / Design Rationale

The fundamental gap was the absence of any coordination layer between agents. Individual agents were powerful but isolated. The design needed to:

- Preserve agent autonomy (each retains its own personality, memory, and skills)
- Add coordination without coupling (hub-and-spoke, not mesh)
- Use filesystem-based communication (workspace files) rather than direct agent-to-agent messaging
- Support iterative refinement (critic review loop)
- Keep configuration declarative (TOML, no code changes to add a team)

## Solution

### Architecture: Hub-and-Spoke Orchestration

```
Goal
  │
  ▼
┌─────────────┐
│ Orchestrator │ ─── Decomposes goal into tasks
└──────┬──────┘
       │
  ┌────┴────┐
  ▼         ▼
┌──────┐ ┌──────┐
│Agent1│ │Agent2│ ─── Specialists execute tasks
└──┬───┘ └──┬───┘
   │        │
   ▼        ▼
┌──────────────┐
│  Workspace   │ ─── Shared filesystem (read/write/list tools)
└──────┬───────┘
       │
       ▼
┌──────────┐
│  Critic  │ ─── Reviews all outputs, approve/reject
└──────┬───┘
       │
  ┌────┴────┐
  │ Approved?│
  ├── Yes ──→ Deliver final result
  └── No ───→ Feed back critique, re-execute (up to max_iterations)
```

### Key Components

**1. Team Definition (`mika-common/team.rs`)**

```toml
# ~/.mika/teams/dev-team/team.toml
[team]
name = "dev-team"
orchestrator = "planner"

[[agents]]
name = "planner"
role = "orchestrator"
mandate = "Decompose goals into actionable tasks"

[[agents]]
name = "researcher"
role = "specialist"
mandate = "Research topics thoroughly"

[[agents]]
name = "critic"
role = "qa"
mandate = "Review quality and completeness"

[flow]
max_iterations = 3
```

**2. Orchestration Engine (`mika-agent/teams/engine.rs`)**

The `TeamEngine` implements a state machine with 5 phases:
1. **Decompose** - Orchestrator agent produces JSON task assignments
2. **Execute** - Each specialist runs with single-turn context + workspace tools
3. **Review** - Critic evaluates all workspace outputs
4. **Iterate** - If rejected, feed critique back and re-execute (up to `max_iterations`)
5. **Deliver** - Produce final synthesized deliverable

Key design decisions:
- Tool registry built once and shared across all agent invocations (not recreated per task)
- `AsyncDatabase` instances get explicit `shutdown()` calls for thread cleanup
- Progress callback (`ProgressCallback = Box<dyn Fn(&str) + Send + Sync>`) for real-time status updates
- Fail-closed security: critic auto-rejects on JSON parse failure

**3. Workspace Tools (`mika-agent/tools/`)**

Three tools enable inter-agent communication through shared files:

| Tool | Purpose | Security |
|------|---------|----------|
| `read_workspace` | Read files from shared workspace | Path traversal prevention, symlink detection, length validation |
| `write_workspace` | Write files to shared workspace | Canonicalize + starts_with containment, parent dir symlink check |
| `list_workspace` | List workspace files with sizes | Depth limit (10), file count limit (500), symlink filtering |

**Defense-in-depth path security:**
- `Path::components()` inspection rejects `..` segments
- `Path::is_absolute()` rejects absolute paths
- `symlink_metadata()` detects symlinks before `canonicalize()`
- `canonicalize()` + `starts_with()` verifies containment after resolution
- Per-write content limit (`MAX_INPUT_LEN = 10,000` chars)

**4. Team Agent Execution (`mika-agent/agent.rs`)**

`run_team_agent()` is a simplified variant of the main agent loop:
- Single-turn context (no conversation history)
- Team-specific system prompt injected after agent identity
- Standard tool loop (max 10 steps, 30s per-tool timeout, 5-min total timeout)
- Does NOT save messages to DB or run compaction
- Agent retains its personality, memory, and skills

**5. Prompt Builders (`mika-agent/teams/prompt.rs`)**

Four prompt builders inject team context into agent system prompts:
- `build_orchestrator_context()` - Team members, workspace state, task assignment format
- `build_specialist_context()` - Role, mandate, specific task, output file
- `build_critic_context()` - Goal, completed tasks, review criteria, approve/reject format
- `build_deliverable_context()` - Goal, synthesis instructions

All use `format!()` with multi-line string literals (not `writeln!` chains).

**6. CLI Commands (`mika-cli/commands/teams.rs`)**

```bash
mika teams list              # List all teams
mika teams create <name>     # Interactive team creation
mika teams run <name> <goal> # Execute team workflow
mika teams status <name>     # Show team + latest run
mika teams log <name>        # Show run history
mika teams delete <name>     # Delete team (with --force)
```

TUI slash commands `/teams` and `/team <name> "<goal>"` delegate to CLI for long-running operations.

### JSON Parsing from LLM Responses

The engine extracts structured JSON from free-form LLM text using smart start patterns:

```rust
fn extract_json(text: &str, open: char, close: char) -> Option<&str> {
    // For arrays, look for "[{" pattern (not bare "[")
    // For objects, look for '{"' pattern (not bare "{")
    let start_pattern = if open == '[' { "[{" } else { "{\"" };
    let start = text.find(start_pattern)?;
    // ... bracket matching from start
}
```

This handles LLM responses that include preamble text before JSON, Markdown code fences, or trailing commentary.

### Run History

Runs are persisted as TOML files in `{team_dir}/history/`:
```
~/.mika/teams/dev-team/history/run-2026-02-25-abc123.toml
```

Each file captures: run_id, goal, status, iteration count, task assignments with statuses, timestamps, and optional deliverable text.

## Code Review Hardening

The initial implementation received 18 code review findings. 16 were resolved:

| Category | Findings | Key Changes |
|----------|----------|-------------|
| Security (P1) | #248, #249, #250 | Path canonicalization, symlink detection, component-based traversal checks |
| Robustness (P1) | #251, #252 | Fail-closed critic defaults, agent name validation against team definition |
| Quality (P2) | #253-#258 | Tool registry reuse, TUI blocking prevention, DB shutdown, JSON extraction, dead code removal |
| Simplification (P3) | #259-#265 | Removed unused fields, format! conversion, workspace listing DRY, module visibility, depth/count limits |

2 findings remain as future work:
- #253: Parallelize specialist task execution with `tokio::JoinSet`
- #258: Agent-native parity (add `run_team`/`list_teams` agent tools)

## Prevention Strategies

### For Multi-Agent Systems

1. **Workspace isolation**: Always validate paths at the tool level, never trust agent-provided paths
2. **Fail-closed parsing**: When parsing structured output from LLMs, default to safe values (e.g., `approved: false` for critic)
3. **Agent name validation**: Cross-reference task assignments against the team definition to catch hallucinated agent names
4. **Resource limits**: Cap workspace file count, directory depth, and per-file size to prevent unbounded growth
5. **Explicit cleanup**: Call `shutdown()` on background threads (AsyncDatabase) to prevent resource leaks

### For Filesystem-Based Communication

1. **Defense-in-depth**: Layer multiple path security checks (components, symlinks, canonicalize, containment)
2. **Symlink detection before canonicalize**: Check `symlink_metadata()` first, since `canonicalize()` follows symlinks silently
3. **Containment verification**: After canonicalization, verify the resolved path still starts with the workspace root

### Security Checklist for New Tools

- [ ] Validate input length against `MAX_INPUT_LEN`
- [ ] Check for empty input
- [ ] Reject absolute paths
- [ ] Inspect `Path::components()` for `..` segments
- [ ] Check `symlink_metadata()` before following paths
- [ ] Use `canonicalize()` + `starts_with()` for containment
- [ ] Reject paths outside the designated directory

## Test Coverage

340 tests pass across all crates. Key test areas:
- Team name validation and normalization (reuses agent naming rules)
- TOML parsing of team definitions
- Workspace tools with path traversal, symlink, and boundary testing
- Orchestration engine task parsing and review parsing
- Prompt builder output verification
- Run history save/load serialization
- CLI command integration

## Related Documentation

- [Layer 3 Vector Search](./layer3-vector-search-sqlite-vec-implementation.md) - Hybrid search architecture (PR #11)
- [Multi-Agent Support](../architecture/multi-agent-support.md) - Per-agent isolation (PR #12)
- [Filesystem Skill Registry](./filesystem-skill-registry-implementation.md) - Similar filesystem-based pattern

## References

- PR #13: https://github.com/senara-solutions/mika/pull/13
- PR #12: https://github.com/senara-solutions/mika/pull/12
- Code review findings: todos/248-265 (16 resolved, 2 pending)
