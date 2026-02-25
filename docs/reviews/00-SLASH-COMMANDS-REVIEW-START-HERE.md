# Agent-Native Architecture Review: Slash-Command System

## START HERE

This is your entry point to a comprehensive agent-native architecture review of the `feat/slash-commands` branch (commit 09d8595).

**Time Investment:**
- Quick overview: 5 minutes (this document)
- Executive summary: 10 minutes (Quick Reference)
- Full understanding: 45 minutes (Full Review)
- Implementation: 6 hours total (3 PRs)

---

## One-Sentence Summary

**The slash-command feature is well-built UX that violates agent-native principles by creating hidden user-only capabilities that block the agent from self-awareness and system introspection.**

---

## The Core Problem

Users can access critical system information via slash commands:
```
/status          ← See how many messages, memory usage, DB size
/skills          ← See what skills are loaded
/soul            ← Read personal configuration
/config          ← View settings
```

**But the agent cannot.** The agent has no tools to:
- Check its own health
- Discover available skills
- Read user configuration
- Understand its own capabilities

This violates the **agent-native principle: action parity** — every user capability should have an agent equivalent.

---

## Visual: What's Broken

```
┌─────────────────────────────────────────┐
│         MIKA SLASH-COMMAND SYSTEM       │
├──────────────────┬──────────────────────┤
│   USER (TUI)     │   AGENT (Thinking)   │
├──────────────────┼──────────────────────┤
│ /status          │ ❌ No tool           │
│ /skills          │ ❌ No tool           │
│ /soul            │ ❌ No tool           │
│ /config          │ ❌ No tool           │
│ /memory search   │ ✅ search_memory()   │
│ /reminders       │ ✅ list_reminders()  │
└──────────────────┴──────────────────────┘

Result: Agent is blind to system state
        User has capabilities agent can't access
```

---

## Quick Answer: What Do I Need to Do?

**Priority 1 (Critical - Blocks Agent Self-Awareness):**
Create 3 new agent tools:
- `get_system_status()` — Check health, memory usage, message count
- `list_skills()` — Discover available skills at runtime
- `read_soul()` — Read user's core values from soul.md

**Priority 2 (High - Reduces Duplication):**
Unify the command registry so TUI and CLI use the same definitions

**Priority 3 (High - Enables Automation):**
Add `--format json` support to CLI commands

---

## The Four Review Documents

### 1. 📋 [Quick Reference](./slash-command-review-summary.md) (5 min read)
**For:** Team leads, PR descriptions, quick briefings

Contains:
- TL;DR of issues
- One-sentence verdict
- Priority action table
- Examples of what's broken
- File changes at a glance

**Best for:** Sharing with stakeholders, adding to PR

---

### 2. 📖 [Full Review](./agent-native-slash-command-review.md) (45 min read)
**For:** Architects, senior engineers, thorough understanding

Contains:
- Detailed capability parity matrix (13 commands, 3 pages)
- 5 critical issues with code locations
- 3 warnings with design implications
- Architecture strength assessment
- 6 prioritized recommendations with rationale
- Code quality analysis

**Best for:** Understanding the full context, design decisions, tradeoffs

---

### 3. 🛠️ [Implementation Guide](./slash-command-implementation-guide.md) (reference guide)
**For:** Developers building the fix

Contains:
- Step-by-step instructions for all 3 priorities
- Exact code locations and snippets
- Testing checklist
- Effort estimates (6 hours total)
- PR structure recommendation (3 PRs)
- Validation criteria

**Best for:** Actually implementing the changes

---

### 4. 🎯 [Agent-Native Design](./slash-command-agent-native-design.md) (20 min read)
**For:** Design discussions, architecture patterns

Contains:
- Before/after examples for 3 key features
- Unified command registry design pattern
- Concrete code examples (TUI, CLI, Agent layers)
- System prompt documentation
- 4-phase migration path

**Best for:** Understanding correct architecture, reference design

---

## Findings Summary

### Critical Issues (Must Fix)

| # | Issue | Impact | Effort |
|---|-------|--------|--------|
| 1 | No agent tools for `/status`, `/skills`, `/soul`, `/model` | Agent can't check health or discover capabilities | 2.5h |
| 2 | Skills system completely inaccessible to agent | Agent doesn't know what it can do | Blocked by #1 |
| 3 | Command output not programmatically consumable | Can't automate, no JSON, not pipe-friendly | 1.5h |
| 4 | Command registry isolated from CLI | Maintenance burden, risk of divergence | 2h |
| 5 | Silent-mode agent is completely blind | Background tasks can't check health | Blocked by #1 |

### Warnings (Should Fix)

| # | Issue | Severity |
|---|-------|----------|
| 6 | `/clear` is TUI state, not system feature | Medium |
| 7 | `/export` doesn't support output format/location | Low |
| 8 | `/compact` has no agent trigger mechanism | Medium |

### Strengths

✅ Client-side isolation is correct (no agent pollution)
✅ Autocomplete implementation is solid
✅ Handler organization is clean
✅ All commands use shared DB/filesystem

---

## Verdict: Agent-Native Compliance Score

**Overall: 2.5 / 5 ⚠️ NEEDS WORK**

| Principle | Score | Status |
|-----------|-------|--------|
| Action Parity | 2/5 | 7 of 13 commands lack agent equivalents |
| Context Parity | 0/5 | Agent has zero visibility into skills or health |
| Shared Workspace | 5/5 | ✅ Good — all commands use same data |
| Primitives over Workflows | 3/5 | Mostly good, but `/compact` is workflow |
| Dynamic Context Injection | 0/5 | System prompt doesn't document commands |

---

## Test These Are Actually Broken

```bash
# These should work but don't demonstrate the problem:

# Agent can't check its own status
$ mika ask "How many messages am I storing?"
# Agent: "I don't have a tool to check message count"

# Agent can't discover skills
$ mika ask "What capabilities do I have?"
# Agent: "I'm not aware of what skills are loaded"

# Users can't get structured output
$ mika status --format json
# Error: unrecognized argument '--format'

# Agent can't read configuration
$ mika ask "Remind me what my soul.md says"
# Agent: "I can't read files directly"
```

---

## Quick Navigation

**I want to...**

| Goal | Go to | Time |
|------|-------|------|
| Get the headlines | [Quick Ref](./slash-command-review-summary.md) | 5 min |
| Understand the full context | [Full Review](./agent-native-slash-command-review.md) | 45 min |
| Implement the fixes | [Impl Guide](./slash-command-implementation-guide.md) | Ref |
| Learn the right way | [Design Doc](./slash-command-agent-native-design.md) | 20 min |
| Find the PR template | [Implementation Guide §PR Structure](#) | - |
| See the code locations | [Full Review §Critical Issues](#) | - |
| Check effort estimates | [Implementation Guide §Effort](#) | - |

---

## For Different Audiences

### For Product Managers / Leads
1. Read this document (5 min)
2. Read [Quick Reference](./slash-command-review-summary.md) (5 min)
3. Ask dev team: "When can we fit the P1 work (3 tools)?"

### For Architects / Senior Engineers
1. Read this document (5 min)
2. Read [Full Review](./agent-native-slash-command-review.md) (45 min)
3. Review [Design Doc](./slash-command-agent-native-design.md) (20 min)
4. Discuss tradeoffs in design review

### For Developers Implementing the Fix
1. Read [Implementation Guide](./slash-command-implementation-guide.md)
2. Follow step-by-step instructions
3. Use code snippets provided
4. Follow testing checklist
5. Create 3 PRs as recommended

### For Future Architecture Review
Reference this review when:
- Adding new slash commands
- Designing agent tools
- Building CLI subcommands
- Discussing command registry architecture

---

## Why This Matters

Agent-native principles aren't theoretical. They directly impact:

1. **Agent Self-Awareness:** Agent should know its own constraints, capabilities, and health
2. **Feature Discoverability:** New skills loaded? Agent should know and offer them
3. **System Reliability:** Agent can't take action on stale assumptions about system state
4. **User Experience:** User shouldn't have capabilities the agent lacks
5. **Maintainability:** One command registry beats two separate ones

---

## Key Principles Applied

This review uses **agent-native architecture principles**:

- **Action Parity:** Every UI action should have an agent equivalent
- **Context Parity:** Agents should see the same data users see
- **Shared Workspace:** Agents and users work in the same data space
- **Primitives over Workflows:** Tools should be building blocks, not procedures
- **Dynamic Context Injection:** System prompt should include runtime app state

Learn more: See `CLAUDE.md` in the repo root.

---

## File Locations

All review documents are in `/data/workspace/senara-solutions/mika/docs/reviews/`:

```
00-SLASH-COMMANDS-REVIEW-START-HERE.md    ← You are here
REVIEW_INDEX.md                            ← Master index
agent-native-slash-command-review.md       ← Full technical review
slash-command-review-summary.md            ← Quick reference
slash-command-agent-native-design.md       ← Design patterns
slash-command-implementation-guide.md      ← Step-by-step guide
```

---

## Recommendations for Next Steps

### This Week
1. [ ] Read [Quick Reference](./slash-command-review-summary.md) (team sync)
2. [ ] Assign P1 work (create 3 agent tools)
3. [ ] Schedule design review for unified registry

### Next Week
1. [ ] Start implementing P1 (3 tools, 2.5 hours)
2. [ ] Create PR: "feat(agent): add system introspection tools"
3. [ ] Review and merge P1

### Later
1. [ ] Implement P3 (JSON output, 1.5 hours)
2. [ ] Implement P2 (unified registry, 2 hours)
3. [ ] Update system prompt and documentation

---

## Questions?

**Quick questions:** See [Quick Reference](./slash-command-review-summary.md)
**Detailed questions:** See [Full Review](./agent-native-slash-command-review.md)
**How to fix it:** See [Implementation Guide](./slash-command-implementation-guide.md)
**Design discussion:** See [Agent-Native Design](./slash-command-agent-native-design.md)

---

## Document Metadata

- **Branch:** `feat/slash-commands`
- **Commit:** `09d8595` — "feat(cli): add slash-command system with autocomplete popup to TUI"
- **Review Date:** 2026-02-25
- **Reviewer:** Claude Code (Agent-Native Architecture Specialist)
- **Total Words:** ~11,000 across all documents
- **Implementation Effort:** 6 hours
- **Scope:** TUI commands, CLI subcommands, agent tools, system prompt

---

**Ready to dig in? Start with [Quick Reference](./slash-command-review-summary.md)** (5 minutes) or jump straight to [Implementation Guide](./slash-command-implementation-guide.md) if you're building the fix.
