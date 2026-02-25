# Slash-Command Review — Quick Reference

**Full Review:** `/data/workspace/senara-solutions/mika/docs/reviews/agent-native-slash-command-review.md`

## TL;DR

The slash-command system is a well-engineered TUI feature that **blocks the agent from accessing critical system capabilities**. The agent cannot:
- Check its own health or status
- Discover which skills are loaded
- Read configuration or soul.md
- List or introspect available features

## Quick Findings

### Critical Issues

1. **No agent tools for `/status`, `/skills`, `/skill`, `/model`**
   - Users can type `/status` but agent cannot check health
   - Agent has no visibility into which skills are available
   - Agent can't report its own capabilities

2. **7 of 13 commands lack agent equivalents**
   - `/clear`, `/exit`, `/help`, `/export` are TUI-only
   - `/memory`, `/soul`, `/config` don't have agent tools
   - `/compact` user-only, no agent trigger mechanism

3. **Command output not programmatically consumable**
   - All handlers return formatted text only
   - No `--format json` option
   - Cannot pipe/automate: `mika status | jq '.core_memory_tokens'`

4. **Command registry isolated from CLI structure**
   - Slash commands defined separately from CLI subcommands
   - Two separate match statements, two separate docs
   - Maintenance burden and risk of divergence

5. **Silent-mode agent is completely blind**
   - Background agent (reminders, heartbeats) can't check system health
   - Can't discover skills before making recommendations
   - Can't read config/soul to inform decisions

### Design Strengths

- Client-side isolation is correct (no agent loop pollution)
- Autocomplete implementation is solid
- Handler organization is clean
- Metadata-rich registry supports discovery

## One-Sentence Verdict

**Good UX + good implementation + wrong scope = agent lockout**

The feature should extend agent capabilities, not replace them with user-only operations.

## Priority Actions

| Priority | Action | Impact |
|----------|--------|--------|
| P1 | Create agent tools: `get_system_status()`, `list_skills()`, `read_soul()` | Enables agent self-awareness |
| P2 | Unify command registry (shared by TUI, CLI, agent docs) | Eliminates duplication, enables consistency |
| P3 | Add `--format json` to all info commands | Enables automation, scripting |
| P4 | Create `request_compaction()` agent tool | Enables agent context management |
| P5 | Document which commands are TUI-only (clear, exit) | Reduces user confusion |

## Test What's Missing

```bash
# These should work but don't:
mika ask "How many messages are in my conversation?"
# Agent: (no tool to check message count)

mika ask "What skills do I have loaded?"
# Agent: (no tool to list skills)

# This should be programmatic but isn't:
mika status --format json
# Error: unrecognized argument '--format'
```

## File Changes Needed

**New files (agent tools):**
- `crates/mika-agent/src/tools/get_system_status.rs`
- `crates/mika-agent/src/tools/list_skills.rs`
- `crates/mika-agent/src/tools/read_soul.rs`

**Updated files:**
- `crates/mika-agent/src/tools/mod.rs` — Register new tools
- `crates/mika-agent/src/prompt.rs` — Document new tools
- `crates/mika-cli/src/tui/commands/handlers.rs` — Add JSON output
- `crates/mika-cli/src/cli.rs` — Add `--format` flags
- `crates/mika-cli/src/commands/*.rs` — JSON output support

**Refactoring (medium complexity):**
- `crates/mika-cli/src/commands/registry.rs` — NEW unified command registry
- `crates/mika-cli/src/tui/commands/mod.rs` — Use registry
- `crates/mika-cli/src/main.rs` — Use registry

## Key Quote from Full Review

> "Users can type `/status` and see agent health, core memory usage, schema version, message count, DB size. The agent should have equivalent capability via a tool. This is foundational to agent self-awareness."

See **Critical Issue #1** in full review for system health architecture.
