# ADR-004: Multi-Agent Teams Orchestration (Hub-and-Spoke)

**Date:** 2026-02-25
**Status:** Accepted
**Component:** mika-agent/teams

## Context

Mika supported multiple independent agents (each with their own DB, memory, skills,
and personality), but had no mechanism for agents to collaborate on complex goals.
There was no way to decompose a goal across specialists, share intermediate work
products, review quality, or iterate toward a deliverable.

## Decision

Implement a hub-and-spoke orchestration system using TOML-based team definitions
and filesystem-based inter-agent communication.

### Architecture

```
Goal → Orchestrator → Specialists → Workspace → Critic → Deliver/Iterate
```

1. **Decompose** — Orchestrator agent produces JSON task assignments
2. **Execute** — Each specialist runs with single-turn context + workspace tools
3. **Review** — Critic evaluates all workspace outputs
4. **Iterate** — If rejected, feed critique back and re-execute (up to `max_iterations`)
5. **Deliver** — Produce final synthesized deliverable

### Team Definitions

```toml
# ~/.mika/teams/dev-team/team.toml
[team]
name = "dev-team"
orchestrator = "planner"

[[agents]]
name = "planner"
role = "orchestrator"
mandate = "Decompose goals into actionable tasks"

[flow]
max_iterations = 3
```

### Workspace Tools

Three tools enable inter-agent communication through shared files:
- `read_workspace` — read files from shared workspace
- `write_workspace` — write files to shared workspace
- `list_workspace` — list workspace files with sizes

All three enforce defense-in-depth path security: component inspection for `..`,
absolute path rejection, symlink detection before canonicalize, containment
verification after resolution.

### Key Design Choices

1. **Filesystem communication** (not messages) — maintains clean separation between agents
2. **TOML definitions** — no database schema changes, filesystem-portable
3. **Explicit Rust** — no framework, consistent with Mika's agent loop philosophy
4. **Fail-closed critic** — auto-rejects on JSON parse failure
5. **Agent name validation** — cross-referenced against team definition to catch hallucinations
6. **Single-turn execution** — team agents run without conversation history

## Consequences

- Agents preserve autonomy (own personality, memory, skills) during team execution
- Team workflows are declarative — add teams without code changes
- Workspace files are the only inter-agent data channel
- Tool registry is built once and shared across all agent invocations
- `AsyncDatabase` instances need explicit `shutdown()` for thread cleanup
- Run history persisted as TOML files in `{team_dir}/history/`

### CLI Commands

```bash
mika teams list | create | run | status | log | delete
```

TUI slash commands: `/teams` (list) and `/team <name> "<goal>"` (execute).
