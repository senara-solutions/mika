# Agent-Native Architecture Review: Slash-Command System

**Branch:** `feat/slash-commands` (commit 09d8595)
**Date:** 2026-02-25
**Reviewer:** Claude Code (Agent-Native Specialist)

## Review Documents

This review consists of three documents:

### 1. [Full Review](./agent-native-slash-command-review.md) (8,500 words)
**Comprehensive analysis of agent-native compliance gaps**

Read this for:
- Detailed capability parity matrix (13 commands)
- 8 critical issues + warnings with code locations
- Architecture assessment and strength analysis
- Prioritized recommendations (P1-P6)
- Implementation guidance

**Key Finding:** 7 of 13 commands lack agent equivalents. Agent cannot check health, discover skills, or read configuration.

---

### 2. [Quick Reference](./slash-command-review-summary.md) (500 words)
**One-page summary for quick understanding**

Read this for:
- TL;DR of critical issues
- One-sentence verdict
- Priority action table
- Test examples of what's broken
- File changes needed at a glance

**Use Case:** Share with team leads, add to PR description, reference in standup.

---

### 3. [Agent-Native Design](./slash-command-agent-native-design.md) (2,000 words)
**Example of how to do it right**

Read this for:
- Before/after comparisons for 3 key features
- Unified command registry design pattern
- Concrete code examples for all three layers (TUI, CLI, Agent)
- System prompt documentation
- Migration path (4 phases)

**Use Case:** Guide for implementation, reference architecture, design discussions.

---

## Executive Summary

The slash-command system is **well-implemented TUI feature that violates agent-native principles** by:

1. Creating hidden user-only capabilities (agent cannot access `/status`, `/skills`, `/soul`)
2. Blocking agent self-introspection (no health checks, skill discovery, config reading)
3. Silos output in text-only format (no JSON, no automation)
4. Duplicates command registry (TUI and CLI separate)
5. Prevents silent-mode agent from understanding system state

### Verdict: NEEDS WORK (Agent-Native Score: 2.5/5)

| Principle | Score | Issue |
|-----------|-------|-------|
| Action Parity | 2/5 | 7 of 13 commands lack agent equivalents |
| Context Parity | 0/5 | Agent has no visibility into skills or health |
| Shared Workspace | 5/5 | ✅ All commands use same DB/filesystem |
| Primitives | 3/5 | Mostly primitives, but compact is workflow |
| Dynamic Context | 0/5 | System prompt doesn't document commands |

---

## Priority Actions

| P | Action | Impact | Files | Effort |
|---|--------|--------|-------|--------|
| 1 | Add 3 agent tools (status, skills, soul) | Enables agent self-awareness | `tools/*.rs`, `prompt.rs` | 150 LOC |
| 2 | Unify command registry | Eliminates duplication | `commands/registry.rs` | 200 LOC |
| 3 | Add JSON output support | Enables automation | `cli.rs`, `commands/*.rs` | 100 LOC |
| 4 | Agent compaction tool | Enables context management | `tools/request_compaction.rs` | 50 LOC |
| 5 | Fix TUI-only commands | Clarity | `tui/commands/mod.rs` | 10 LOC |

---

## Test These Are Broken Right Now

```bash
# These should work but don't:

# Agent can't check its own health
$ mika ask "How many messages am I storing?"
# Agent: "I don't have a tool to check message count."

# Agent can't discover skills
$ mika ask "What skills do I have?"
# Agent: "I don't know what skills are loaded."

# Users can't get machine-readable output
$ mika status --format json
# Error: unrecognized argument '--format'

# Agent can't read user configuration
$ mika ask "Tell me about my soul.md"
# Agent: "I can't read files directly."
```

---

## File Locations (This Repository)

**Review Documents:**
- `/data/workspace/senara-solutions/mika/docs/reviews/agent-native-slash-command-review.md` (full)
- `/data/workspace/senara-solutions/mika/docs/reviews/slash-command-review-summary.md` (quick ref)
- `/data/workspace/senara-solutions/mika/docs/reviews/slash-command-agent-native-design.md` (design)

**Codebase:**
- TUI slash commands: `crates/mika-cli/src/tui/commands/`
- CLI subcommands: `crates/mika-cli/src/commands/`
- Agent tools: `crates/mika-agent/src/tools/`
- Skills system: `crates/mika-agent/src/skills/`
- System prompt: `crates/mika-agent/src/prompt.rs`

---

## Recommendations for Next Steps

1. **Read the full review** (20 min read)
2. **Discuss P1 actions** (agent tools) in team sync
3. **Create focused PRs:**
   - `feat(agent): add system introspection tools` (get_system_status, list_skills, read_soul)
   - `refactor(cli): unify command registry`
   - `feat(cli): add JSON output format support`
4. **Update documentation:**
   - System prompt with new tools
   - README with new capabilities
   - Architecture doc with command design pattern
5. **Add tests:**
   - Agent can call new tools
   - Tools return correct JSON
   - TUI and CLI show same data

---

## Key Principles Applied

This review applies **agent-native architecture principles**:

- **Action Parity:** Every UI action should have an agent equivalent
- **Context Parity:** Agents should see the same data users see
- **Shared Workspace:** Agents and users work in the same data space
- **Primitives over Workflows:** Tools should be building blocks, not procedures
- **Dynamic Context:** System prompt should include runtime app state

Learn more: See "Agent-Native Architecture Reviewer" in `CLAUDE.md`.

---

## Questions?

**For detailed findings:** See full review (agent-native-slash-command-review.md)
**For implementation guidance:** See design document (slash-command-agent-native-design.md)
**For quick summary:** See quick reference (slash-command-review-summary.md)
