# Mika Home Directory & Agent Core Systems

**Date:** 2026-02-23
**Status:** Brainstorm complete
**Scope:** `~/.mika/` directory structure, identity, soul, memory self-editing, heartbeat, user management, bootstrap experience

## What We're Building

A `~/.mika/` home directory that serves as the per-customer agent boundary. It contains human-editable configuration and persona files alongside an encrypted SQLite database for sensitive data. This directory powers six core agent systems: identity, soul, memory (with autonomous self-editing), heartbeat, user management, and bootstrap onboarding.

The design is CLI-first but container-ready — one `~/.mika/` = one customer. On K8s, each container gets its own directory.

## Why This Approach

**Hybrid encryption model:** Both OpenClaw and LettaBot store everything as plaintext files. Mika takes a better path — config and persona files stay human-editable (TOML/Markdown), while all PII and memory data lives encrypted in SQLite with AES-256-GCM. Best of both worlds.

**Full memory self-editing over append-only:** The MemGPT insight from LettaBot is correct — an assistant that can't update its own notes becomes stale. But the real concern is auditability, not immutability. We solve this with a MemoryEvent audit log that captures every mutation with before/after values, conversation context, and agent reasoning.

**Silent heartbeat with direct action:** Neither OpenClaw's HEARTBEAT_OK suppression protocol nor LettaBot's CLI-tooling requirement. Mika evaluates silently and uses `send_message` with urgency levels when action is warranted. No output = no action. Timezone-aware delivery.

**Opinionated soul, emergent personality:** Ship with a strong default personality (professional, proactive, concise exec assistant). Core memory self-editing means each user's Mika naturally diverges over time. The soul is the starting point, not the ceiling.

## Key Decisions

### 1. `~/.mika/` Directory Structure

```
~/.mika/
├── config.toml              # Main config (API keys, channels, settings)
├── identity.toml            # Agent name, avatar, emoji
├── soul.md                  # Agent personality, values, communication style
├── heartbeat.md             # Heartbeat checklist (what to check periodically)
├── user.md                  # High-level user context (human-editable)
├── data/
│   └── mika.db              # Encrypted SQLite (memory, facts, conversations, audit log)
└── logs/
    └── mika.log             # Application logs (tracing output)
```

**Design rationale:**
- `config.toml` — uses existing config-rs infrastructure with `MIKA_` env prefix. Replaces `config/default.toml` as the primary config source when running from `~/.mika/`
- `identity.toml` — agent name and avatar. Read-only to the agent, user-editable only
- `soul.md` — static personality baseline. Read-only to the agent. The agent adapts its behavior via the `persona` core memory block (in SQLite), not by editing this file. `soul.md` is the foundation; `persona` is the evolution
- `user.md` — human-editable seed for user context. Loaded into the `user_summary` core memory block on first run (or when the block is empty). Once the agent starts editing `user_summary`, the SQLite block is the live version. `user.md` serves as a reset mechanism — delete the block and it re-seeds from the file
- `heartbeat.md` — editable checklist for what the agent checks during heartbeats. Read by the agent during heartbeat evaluation, editable by the user
- `data/mika.db` — all encrypted data: core memory blocks, structured facts, conversation history, memory audit log
- No daily log files (unlike OpenClaw's `memory/YYYY-MM-DD.md`) — conversation history lives in SQLite

### 2. Core Memory Blocks (Layer 1 — Always in System Prompt)

Four blocks, ~500 tokens each (~2000 token total budget):

| Block | Purpose | Agent-editable |
|---|---|---|
| `persona` | How Mika behaves with this specific user | Yes |
| `user_summary` | Who the user is, communication style, preferences | Yes |
| `current_priorities` | Active goals, projects, what they care about now | Yes |
| `key_people` | Inner circle, relationships, context on frequent names | Yes |

**Storage:** All four blocks live as encrypted rows in `data/mika.db`, not as files. They are loaded into the system prompt on every turn. The `soul.md` and `user.md` files are static seeds — the agent never writes to files, only to SQLite blocks via tools.

**Why these four:** `persona` and `user_summary` are standard (OpenClaw/LettaBot both have equivalents). `current_priorities` gives Mika context for proactive behaviors. `key_people` was chosen over `active_commitments` because commitments change daily (they belong in Layer 2 structured facts), while knowing "Sarah is the COO and prefers Slack" shapes every interaction.

**Budget discipline:** 500 tokens per block forces the agent to compress and prioritize. When a block fills up, the agent must decide what to drop — that compression IS understanding.

### 3. Memory Self-Editing Tools

```
update_core_memory { section, action: replace|append|remove_line, content }
store_fact { category: person|commitment|preference|event, content }
update_fact { fact_id, update (includes marking resolved/obsolete) }
search_memory { query, filters }
```

**Audit log (MemoryEvent table in SQLite):**
- `timestamp` — when the mutation happened
- `tool_call` — which tool was invoked
- `before` — previous value (null for new entries)
- `after` — new value
- `conversation_id` — which conversation triggered it
- `reasoning` — agent's stated reason for the change

**Guardrail:** Rate-limit core memory updates to 2-3 per conversation. If the agent is rewriting core memory every turn, the prompt needs fixing.

### 4. Heartbeat System

**Model:** Silent evaluation with direct action (no HEARTBEAT_OK protocol, no CLI tooling requirement).

**How it works:**
1. Heartbeat fires on configured interval (default: 30 min)
2. Agent receives heartbeat system prompt with: user's local time, pending commitments, last heartbeat summary
3. Agent evaluates silently — reviews context, todos, upcoming events
4. If action warranted: calls `send_message { channel, urgency: low|normal|high, message }`
5. If nothing warranted: responds with `NO_ACTION` (not delivered anywhere)

**Timezone awareness:** Never wake the user at midnight for a non-urgent follow-up. Inject user's local time and active hours into heartbeat context.

**Urgency-based delivery (Phase 2 — requires background process and channel routing):**
- `high` — deliver immediately regardless of time
- `normal` — deliver during active hours, queue otherwise
- `low` — batch into next morning briefing

**Phase 1 scope:** Define the heartbeat data model, tools, and prompt. The actual scheduler and delivery routing are Phase 2 (requires async runtime beyond CLI). In Phase 1, heartbeats can be triggered manually via CLI command for testing.

**heartbeat.md example:**
```markdown
# Heartbeat Checklist

- Review active commitments approaching deadline
- Check if any meetings are coming up in the next 2 hours
- Look for stale priorities (no updates in 3+ days)
- Surface patterns worth mentioning
```

### 5. Soul & Identity

**soul.md (shipped default):**
- Professional, calm, confident, concise
- Anticipates needs rather than waits for instructions
- Protects the user's time fiercely
- Leads with the answer, then context
- Matches the user's energy
- Pushes back respectfully when something doesn't make sense
- Never pretends to have done something it hasn't

**identity.toml:**
```toml
name = "Mika"
emoji = "✦"
```

**Evolution:** The default soul.md is the starting point. Over time, `update_core_memory(section: "persona")` causes Mika to adapt to each user's communication style naturally. After a month, two users' Mikas feel meaningfully different.

### 6. Bootstrap / First-Run Experience

**When `~/.mika/` doesn't exist:**
1. Create directory structure with defaults (soul.md, identity.toml, config.toml, empty DB)
2. Print `✦ Mika initialized at ~/.mika/`
3. Inject `ONBOARDING_PROMPT` into system prompt for first session
4. Start a natural onboarding conversation (2-3 exchanges max):
   - "Hi, I'm Mika — your executive assistant. What should I call you, and what do you do?"
   - Follow-up based on response: current priorities, key people
5. Agent uses `update_core_memory` aggressively to seed all blocks from responses
6. Transition to normal mode — every subsequent interaction already feels personal

**Implementation:** `if db.is_first_run() { system_prompt.prepend(ONBOARDING_PROMPT); }`

## Design Lineage

**From OpenClaw:** Markdown workspace files for human-editable config (soul, identity, heartbeat checklist). Hybrid memory search direction (sqlite-vec + FTS5 for Layer 3). Channel adapter architecture concept (Phase 2).

**From LettaBot:** MemGPT-style autonomous memory self-editing via tool calls. Persona/human split in core memory concept (adapted to 4 blocks). Silent heartbeat evaluation model (improved with direct action + urgency).

**Mika's own contributions:** MemoryEvent audit log (auditability over immutability). Urgency-based heartbeat delivery with timezone awareness. Field-level AES-256-GCM encryption with HMAC lookups. 500-token budget discipline per core memory block. Rate-limited core memory edits (2-3 per conversation guardrail).

## Open Questions

*None — all key design questions resolved during brainstorming.*

## Next Steps

This brainstorm covers the WHAT. Run `/workflows:plan` to design the HOW — implementation order, file changes, migration from current codebase to `~/.mika/` directory model.
