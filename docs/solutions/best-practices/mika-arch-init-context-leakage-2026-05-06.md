---
title: "mika-arch session-init context-channel leakage: core_memory and summary are system-prompt, not message-history — semantic misinterpretation, not structural misplacement"
date: 2026-05-06
category: best-practices
module: mika-arch
problem_type: semantic-leakage
component: prompt-assembly
severity: high
applies_when:
  - mika-arch produces degenerate responses referencing "prior turns" that do not exist in the session's message history
  - Degenerate language maps to core_memory blocks or conversation summary content
  - Agent operates in fresh-session mode (no --session-id reuse) yet behaves as if continuing a prior conversation
  - Silent/callback mode sessions with a single synthetic trigger message exhibit the pattern most severely
tags:
  - mika-arch
  - context-assembly
  - session-init
  - summary-injection
  - degenerate-response
  - core-memory
  - compaction
  - semantic-leakage
  - system-prompt
---

## Investigation Question

**Confirm whether `core_memory` and the most-recent session summary are merged into the LlmProvider's conversation context as system-prompt content (stable across turns; safer) vs. as message-history content (treated as prior turns by the LLM; explains the leakage).**

## Answer

**Both are system-prompt content. The placement is architecturally correct.** The leakage is semantic, not structural — the LLM misinterprets conversational-style content in the system prompt as prior conversation turns.

## Evidence: Code Path Pinning

### Core Memory Injection

**File:** `crates/mika-agent/src/prompt.rs` lines 353–387

`write_core_memory_section()` (line 353) writes all five core memory blocks (`self_model`, `user_summary`, `current_priorities`, `key_people`, `workflows`) into the system prompt string, wrapped in `<core-memory>` XML tags:

```
## Core Memory
These are your persistent memory blocks, auto-loaded into this prompt on every turn. [...]

<core-memory>
### self_model
[content]

### current_priorities
[content]
...
</core-memory>
```

Called from:
- `build_system_prompt()` at line 378 (conversation mode)
- `build_silent_prompt()` at line 747 (silent mode)

The resulting string goes into `LlmRequest.system: Option<String>` (`crates/mika-common/src/llm/types.rs`), NOT into `LlmRequest.messages: Vec<LlmMessage>`.

**Channel: system prompt. Confirmed safe.**

### Conversation Summary Injection

**File:** `crates/mika-agent/src/agent.rs` lines 2018–2024 (conversation mode) and lines 3044–3049 (silent mode)

After `build_system_prompt()` / `build_silent_prompt()` returns the system string, the summary is appended directly to it:

```rust
// agent.rs line 2018 (conversation) / line 3044 (silent)
if let Some(summary) = db.load_conversation_summary().await? {
    system.push_str("\n## Conversation Summary\n");
    system.push_str("<context type=\"summary\" trust=\"data\">\n");
    system.push_str(&summary.content);
    system.push_str("\n</context>\n");
}
```

The `system` variable is then placed in `LlmRequest.system: Some(system)` at line 2264 (conversation) and the equivalent silent-mode assembly point.

**Channel: system prompt. Confirmed safe.**

### Message History Construction (For Contrast)

**Conversation mode** (`agent.rs` line 2119): `db.load_recent_messages(20)` loads the 20 most recent non-summary messages from the DB. Summary messages (`role='summary'`) are excluded from this query — they travel through the system-prompt path above, never through the messages array.

**Silent mode** (`agent.rs` lines 3119–3122): A single synthetic trigger message is the entire messages array:

```rust
let messages = vec![LlmMessage {
    role: LlmRole::User,
    content: LlmContent::Text(user_msg), // e.g., "[callback: label]"
}];
```

No conversation history is loaded in silent mode. The LLM sees: (1) a large system prompt containing soul + identity + instructions + core_memory + skills + conversation summary, and (2) a single user message like `[callback: run_claude_pilot]`.

## Root Cause: Semantic Leakage

The structural placement is correct, but the LLM misinterprets the content. Three compounding factors create the leakage:

### Factor 1: Summary content uses conversational language

The compaction summarizer (`crates/mika-agent/src/compaction.rs` line 14) produces bullet-point summaries with phrasing like:

- "User discussed ticket #668 and agreed on disposition ITERATE"
- "Reviewed ticket #996 with mika-arch, received ESCALATE verdict"
- "store_fact housekeeping call completed"

This reads as a conversation transcript to the LLM. The `SUMMARIZATION_SYSTEM_PROMPT` instruction ("Preserve: key decisions, action items, commitments") produces content that mirrors conversational history by design.

### Factor 2: Core memory contains self-referential narrative

mika-arch's core memory blocks (populated via `update_core_memory` tool) accumulate narrative-style entries:

| Core memory block | Example degenerate-echo phrase | Why it reads as prior turns |
|---|---|---|
| `current_priorities` | "store_fact housekeeping call" | Sounds like a prior session action |
| `workflows` | "structural contract", "Disposition: ITERATE" | Sounds like a prior review conclusion |
| `self_model` | Session-reflective language about review patterns | Sounds like self-narration from a prior turn |

These entries are factually correct descriptions of the agent's state, but their phrasing is indistinguishable from a conversation log when the LLM encounters them in the system prompt.

### Factor 3: Silent-mode context ratio is dangerously imbalanced

In silent/callback mode, the messages array is a single synthetic trigger message (~30 chars). The system prompt is ~8,000–15,000 chars containing soul, identity, instructions, core_memory, skills, and conversation summary. The system-prompt-to-messages ratio is approximately **300:1**.

With no actual conversation history to anchor the LLM's sense of "what has been discussed in this session", the narrative-style content in the system prompt becomes the dominant context signal. The LLM treats the system prompt's narrative as the session's conversational history and attempts to "continue" it.

In conversation mode, this ratio is lower (~3:1 to ~5:1) because the messages array contains up to 20 actual conversation turns. The actual conversation history provides grounding that competes with the system-prompt narrative. This explains why the leakage manifests most severely in silent/callback mode.

### The inflection at 16:54Z

The sharp inflection (7 substantive responses before 16:54Z, 0 after) suggests a state change in the summary content. As the compaction summarizer accumulated more grooming session context through the day, the summary's narrative density crossed a threshold where it dominated the model's conditioning more than the actual task brief. This is consistent with Factor 3 — the summary grew while the messages array stayed at 1 message.

## Proposed Fix Shape

The fix is out of scope for this investigation ticket (#1008). The following axes should be explored in the implementation ticket:

### Axis 1: Anti-conversational summary framing

Replace the current summary injection framing:

```
## Conversation Summary
<context type="summary" trust="data">
[summary content]
</context>
```

with stronger anti-conversational anchoring:

```
## Prior Session Context (Read-Only Reference)
<context type="summary" trust="data" role="reference">
The following is a factual summary of prior sessions. It is NOT a conversation
you participated in during this session. Do not reference, continue, or respond
to any of these topics unless the current user message explicitly asks about them.
Treat this as background knowledge, not as conversation history.

[summary content]
</context>
```

This addresses the LLM's tendency to treat system-prompt narrative as conversation by explicitly framing it as non-conversational.

### Axis 2: Summary content format reform

Change the compaction summarizer's output format from conversational bullet points to factual state assertions. Replace the `SUMMARIZATION_SYSTEM_PROMPT` instruction to produce:

- **Current:** "Discussed ticket #668 and agreed on disposition ITERATE"
- **Proposed:** "Fact: Ticket #668 reviewed. Outcome: ITERATE."

The key change is removing agent-perspective verbs ("discussed", "agreed", "reviewed with") that trigger conversational-continuation behavior. Use declarative state assertions instead.

### Axis 3: Silent-mode summary budget cap

Cap or omit the conversation summary in silent/callback mode where the messages array is a single trigger message. Options:

- **Option A:** Omit the summary entirely in silent mode. The summary was designed for conversation continuity; silent mode has no conversation to continue.
- **Option B:** Cap the summary to N tokens (e.g., 200) in silent mode, with a recency bias (keep only the most recent items).
- **Option C:** Gate by trigger type — omit for `Heartbeat`/`Reflection`/`SkillRun`, keep (possibly capped) for `Callback`/`Reminder` where prior context may be relevant.

Option C is the most nuanced and likely correct. Callback turns genuinely benefit from knowing what task was in progress; heartbeat turns do not.

### Axis 4: Per-agent summary opt-out via identity.toml

Allow agents to opt out of summary injection entirely:

```toml
# identity.toml
[context]
inject_summary = false  # default: true
```

mika-arch operates in fresh-session mode (each grooming invocation is a new session with no `--session-id` reuse). For such agents, the conversation summary is a cross-session bleed vector with no upside. The opt-out makes the architectural intent explicit.

## Relationship to Prior Findings

| Document | Relationship |
|---|---|
| `docs/solutions/best-practices/mika-arch-pass-1-degenerate-recovery-2026-05-06.md` | Documents the same degenerate symptom. The retry-with-fresh-session workaround succeeds because the new session has an empty messages table, reducing Factor 3's ratio. This finding explains WHY the workaround works. |
| `docs/solutions/agent-quirks/mika-arch-persistence-meta-hallucination-2026-05-02-resolved.md` | Hypothesis 3 ("orchestration-shell-to-skill context handoff may inject memory-skill-adjacent state") is confirmed as directionally correct. The "memory-skill-adjacent state" is the conversation summary in the system prompt. |
| `docs/solutions/best-practices/citation-fabrication-prompt-anchoring-2026-05-02.md` | Cross-session parametric memory bleed is the same failure class. This finding identifies the specific channel (summary injection) and proposes structural fixes beyond prompt-level anchoring. |
| `docs/solutions/architecture-patterns/deterministic-skill-context-injection.md` | The pattern of moving LLM-dependent behavior to the engine layer applies here. The summary injection is engine-controlled but its content is LLM-generated (by the compaction summarizer). Reforming the content format (Axis 2) applies the same philosophy. |

## Related

- Issue: mika#1008
- Operationally blocked: senara-solutions/mika-platform#86 (grooming arc halted at architect first-pass)
- Sibling: `docs/solutions/best-practices/mika-arch-pass-1-degenerate-recovery-2026-05-06.md` (same symptom, operational workaround)
- Sibling: `docs/solutions/agent-quirks/mika-arch-persistence-meta-hallucination-2026-05-02-resolved.md` (same failure class, different trigger)
