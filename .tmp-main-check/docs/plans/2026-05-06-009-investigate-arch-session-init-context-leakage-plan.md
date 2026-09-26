---
title: "investigate(arch): mika-arch session-init context-channel leakage"
type: fix
status: active
date: 2026-05-06
---

# investigate(arch): mika-arch session-init context-channel leakage

## Overview

Investigation ticket #1008. mika-arch produced four consecutive degenerate responses across fresh sessions (starting 2026-05-06T16:54Z), each referencing "prior turns" that do not exist in the database. The degenerate language maps directly to entries in mika-arch's `core_memory` blocks and the most-recent session summary. The investigation question is narrow: **confirm the channel through which `core_memory` and session summaries enter the LLM context, and document the structural fix shape.** No implementation in this ticket.

## Problem Frame

The symptom is that the LLM treats injected system-prompt content (core memory, conversation summary) as if it were prior conversation turns. This causes it to "continue" a conversation that never happened in the current session. The sharp inflection at 16:54Z (7 substantive responses before, 0 after) suggests a state change — likely the conversation summary crossing a threshold where the summary content dominates the system prompt, conditioning the LLM more strongly than the actual user message.

## Requirements Trace

- R1. Identify the exact code paths and line ranges where `core_memory` enters the LLM context
- R2. Identify the exact code paths and line ranges where conversation summaries enter the LLM context
- R3. Confirm whether each is placed in the `system` field (system prompt) or the `messages` array (conversation history)
- R4. Document the structural mechanism by which the LLM misinterprets the injected content as prior turns
- R5. Propose a fix shape (not implementation) that addresses the root cause
- R6. Produce a finding doc at `docs/solutions/best-practices/mika-arch-init-context-leakage-2026-05-06.md`

## Scope Boundaries

- No code changes to the agent engine
- No broader audit of memory/summary lifecycle policy
- No investigation of the three other open questions from the compound doc (store_fact invocation during reviews; why 16:54 specifically; architect state reset)
- No implementation of the proposed fix (separate ticket)

## Context & Research

### Finding: Both Channels Are System Prompt (Safe Placement)

**Core memory injection:**
- `crates/mika-agent/src/prompt.rs` lines 353–387: `write_core_memory_section()` writes core memory into the system prompt string wrapped in `<core-memory>` XML tags
- Called from `build_system_prompt()` (line 378) and `build_silent_prompt()` (line 747)
- The content goes into the `system: Option<String>` field of `LlmRequest`, NOT into the `messages: Vec<LlmMessage>` array
- **Verdict: Safe placement.** Core memory is purely system prompt content.

**Conversation summary injection:**
- `crates/mika-agent/src/agent.rs` lines 2018–2024 (conversation mode) and lines 3044–3049 (silent mode)
- Appended to the `system` string after `build_system_prompt()` / `build_silent_prompt()` returns
- Wrapped in `<context type="summary" trust="data">` XML tags under a `## Conversation Summary` heading
- **Verdict: Safe placement.** Summary is also purely system prompt content.

**Message history construction:**
- Conversation mode: `crates/mika-agent/src/agent.rs` line 2119: `db.load_recent_messages(20)` loads the 20 most recent non-summary messages. Summary messages (`role='summary'`) are excluded.
- Silent mode: `crates/mika-agent/src/agent.rs` lines 3119–3122: A single synthetic trigger message (e.g., `[callback: label]`). No conversation history loaded at all.

### Root Cause: Semantic Leakage, Not Structural Misplacement

The placement is architecturally correct — both go into the system prompt, not the messages array. The bug is **semantic**: the LLM interprets conversational-style content in the system prompt as if it were prior conversation turns.

Three compounding factors for mika-arch specifically:

1. **Summary content uses conversational language.** The `SUMMARIZATION_SYSTEM_PROMPT` (`crates/mika-agent/src/compaction.rs` line 14) produces bullet-point summaries with phrasing like "User discussed X", "Agreed on Y", "Reviewed ticket #Z with disposition ITERATE". This reads as conversational history to the LLM.

2. **Core memory contains self-referential narrative.** mika-arch's `self_model`, `current_priorities`, and `workflows` blocks contain phrases that sound like prior session notes (e.g., "store_fact housekeeping call", "structural contract", disposition language).

3. **Silent mode has ZERO actual conversation history.** The messages array contains a single synthetic trigger message. The system prompt (which includes core_memory + summary) is the overwhelmingly dominant source of context. With no actual conversation history to anchor the LLM's sense of "what has been discussed", it treats the system-prompt narrative as conversation context.

### Institutional Learnings

- `docs/solutions/best-practices/mika-arch-pass-1-degenerate-recovery-2026-05-06.md` — Documents the same degenerate pattern; retry-with-fresh-session works but is a workaround
- `docs/solutions/agent-quirks/mika-arch-persistence-meta-hallucination-2026-05-02-resolved.md` — Hypothesis 3 matches exactly: "orchestration-shell-to-skill context handoff may inject memory-skill-adjacent state"
- `docs/solutions/best-practices/citation-fabrication-prompt-anchoring-2026-05-02.md` — Cross-session parametric memory bleed is a known failure class
- `docs/solutions/architecture-patterns/deterministic-skill-context-injection.md` — Documents the pre-LLM pipeline ordering; silent mode skips context resolution
- `docs/solutions/architecture-patterns/callback-turn-work-item-context-injection.md` — Different entry points have different context assembly paths; TUI vs server path asymmetry is intentional

## Key Technical Decisions

- **Investigation-only:** The finding doc documents the channel, root cause, and fix shape. The fix itself is a separate ticket.
- **Compound doc format:** Written as a `docs/solutions/best-practices/` entry with YAML frontmatter for institutional searchability.

## Implementation Units

- [x] **Unit 1: Write the finding doc**

**Goal:** Produce a comprehensive finding document that answers the investigation question, pins the exact code locations, explains the root cause mechanism, and proposes a fix shape.

**Requirements:** R1, R2, R3, R4, R5, R6

**Dependencies:** None

**Files:**
- Create: `docs/solutions/best-practices/mika-arch-init-context-leakage-2026-05-06.md`

**Approach:**
- Structure the doc with YAML frontmatter (`module: agent-core`, `tags: [mika-arch, context-assembly, session-init, summary-injection, degenerate-response]`, `problem_type: semantic-leakage`)
- Pin exact file paths and line ranges for both injection sites
- Document the three compounding factors (summary language, core memory narrative, silent-mode zero-history)
- Include the evidence table mapping degenerate phrases to their source (core_memory block or summary content)
- Propose fix shape covering these axes:
  - **Summary framing:** Wrap the summary injection with stronger anti-conversational framing (e.g., "The following is a factual summary of prior sessions. It is NOT a conversation you participated in. Do not reference or continue any of these topics unless the current user message explicitly asks about them.")
  - **Content format reform:** Change the compaction summarizer's output format from conversational bullet points to factual state assertions (e.g., "Fact: Ticket #668 was reviewed" rather than "Discussed ticket #668 and agreed on disposition ITERATE")
  - **Silent-mode context budget:** Cap or omit the summary in silent/callback mode where the messages array is a single trigger — the summary-to-message ratio is dangerously high
  - **Per-agent summary opt-out:** Allow agents like mika-arch (which operate in fresh-session mode) to opt out of summary injection entirely via `identity.toml`

**Patterns to follow:**
- `docs/solutions/best-practices/mika-arch-pass-1-degenerate-recovery-2026-05-06.md` — same module, same date, same agent
- `docs/solutions/architecture-patterns/deterministic-skill-context-injection.md` — code-pinning style

**Test scenarios:**
- Test expectation: none — this is a documentation-only unit

**Verification:**
- Finding doc exists at the expected path
- Doc contains YAML frontmatter with correct module and tags
- Doc pins exact file paths and line ranges for both core_memory and summary injection
- Doc answers the investigation question (system prompt, not messages array)
- Doc proposes at least three fix-shape axes
- Doc does NOT contain implementation code

## System-Wide Impact

- **Interaction graph:** The finding affects `prompt.rs` (system prompt assembly), `agent.rs` (summary injection in both conversation and silent modes), and `compaction.rs` (summary content format). All three files are touched by the fix proposal but NOT by this investigation ticket.
- **Error propagation:** N/A — investigation only
- **State lifecycle risks:** N/A — no state changes
- **API surface parity:** The summary injection pattern is duplicated between conversation mode (line 2018) and silent mode (line 3044) — any future fix must address both sites.
- **Unchanged invariants:** The `LlmRequest.system` / `LlmRequest.messages` separation is correct and should not change. The fix is about the *content* and *framing* within the system prompt, not the structural placement.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Finding doc may not fully explain why the inflection happened at 16:54Z specifically | Out of scope per issue #1008; note as a separate investigation if needed |
| Fix proposals may conflict with other agents' compaction needs | Fix shape is proposed, not implemented; conflicts resolved during fix-ticket planning |

## Sources & References

- Related issue: #1008
- Related code: `crates/mika-agent/src/prompt.rs` lines 353–387, 747–754
- Related code: `crates/mika-agent/src/agent.rs` lines 2018–2024, 3044–3049, 3119–3122
- Related code: `crates/mika-agent/src/compaction.rs` lines 14–19, 80–115
- Related code: `crates/mika-common/src/llm/types.rs` `LlmRequest` struct
- Compound docs: `docs/solutions/best-practices/mika-arch-pass-1-degenerate-recovery-2026-05-06.md`, `docs/solutions/agent-quirks/mika-arch-persistence-meta-hallucination-2026-05-02-resolved.md`
