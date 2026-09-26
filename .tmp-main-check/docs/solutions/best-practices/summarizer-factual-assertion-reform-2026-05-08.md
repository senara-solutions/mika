---
title: "Summarizer content reform: factual assertions over conversational language"
date: 2026-05-08
category: best-practices
module: mika-agent
problem_type: best_practice
component: assistant
severity: medium
applies_when:
  - The compaction summarizer produces text that the LLM misreads as prior conversation turns
  - Agents exhibit degenerate responses referencing "we discussed" or "as we agreed" when no such conversation occurred
  - Summary content survives Axis 3 truncation and Axis 4 is not engaged for the agent
tags:
  - compaction
  - summarizer
  - prompt-assembly
  - context-channel-leakage
  - factual-assertion
  - mika-1009
---

# Summarizer Content Reform: Factual Assertions Over Conversational Language

## Context

mika#1009 identified four fix axes for mika-arch session-init context-channel leakage. The root cause: the conversational summary uses bullet-point language ("Discussed ticket #668 and agreed on disposition ITERATE") that the LLM misreads as prior turns it participated in, producing degenerate responses.

Axis 4 (`[context.summary].inject = false`, mika#1019) prevents summary loading entirely for specific agents. Axis 3 (`[context.summary].max_tokens`, mika#1022) caps summary size on silent-mode turns. **Axis 2 (this reform) changes the content shape itself** so that surviving summary content reads as historical record, not conversation.

## Guidance

The `SUMMARIZATION_SYSTEM_PROMPT` constant in `crates/mika-agent/src/compaction.rs` produces factual state assertions using four named prefixes:

- `Fact:` for objective state (entities, references, timestamps, quantities)
- `Decision:` for choices made and disposition
- `Outcome:` for results and state transitions
- `Open:` for unresolved questions or pending work

The prompt explicitly forbids:
- First-person language (we, our, I) or second-person (you, your)
- Conversational verbs implying participation (discussed, agreed, decided together, wanted, asked)
- Process narration (then we, after that, next)

**Before (conversational shape):**
```
- Discussed migration plan for #668 and agreed on disposition ITERATE
- User wanted clarification on implementation steps
- We decided to defer the schema change to next sprint
```

**After (factual-assertion shape):**
```
- Fact: Ticket #668 reviewed for migration plan
- Decision: disposition ITERATE
- Outcome: implementation steps clarified
- Decision: schema change deferred to next sprint
```

The key structural insight: **named prefix prefixes are a forcing function**. Telling the LLM "use factual language" without a structural enforcer regresses under load. Naming the four prefixes gives the model a checklist; it must pick one per bullet, which structurally evicts narrative shape.

## Why This Matters

The summary block is consumed by the next session as system-prompt context. Conversational shape in that position causes the LLM to treat it as prior turns it participated in, producing:
- "As we discussed..." referencing non-existent conversations
- "Building on our agreement..." when no agreement was made this session
- Inherited context contaminating reasoning in fresh-session agents

This is compounding factor 1 from the mika#1009 finding. The anti-conversational anchor ("NOT a record OF a conversation. Future readers did not participate") directly contradicts the leak mechanism.

## When to Apply

- When modifying the compaction summarizer prompt in `compaction.rs`
- When designing new prompts that produce content injected into future system prompts
- When debugging degenerate "prior turns" references in agent responses
- When evaluating whether Axis 4 (load-prevention) is needed for a new agent: the reformed shape reduces the urgency but doesn't eliminate it for agents with high summary pollution

## Examples

The prompt constant at `compaction.rs:14` is the single point of control. Tests at `summarization_prompt_enforces_factual_shape` assert nine load-bearing invariants:

| Invariant class | Asserted substrings |
|---|---|
| Forcing-function prefixes | `` `Fact:` ``, `` `Decision:` ``, `` `Outcome:` ``, `` `Open:` `` |
| Anti-conversational anchors | `NOT a record OF a conversation`, `did not participate` |
| Negative list | `Do NOT use`, `First-person`, `Conversational verbs` |

These are architectural commitments. Changing them requires a plan update per mika#1024.

## Related

- mika#1009: Parent finding doc (`docs/solutions/best-practices/mika-arch-init-context-leakage-2026-05-06.md`)
- mika#1019: Axis 4 sibling — per-agent `[context.summary].inject` opt-out (`docs/solutions/best-practices/per-agent-context-injection-opt-out-2026-05-07.md`)
- mika#1022: Axis 3 sibling — silent-mode `[context.summary].max_tokens` budget cap (`docs/solutions/best-practices/silent-mode-summary-budget-cap-2026-05-08.md`)
- mika#1024: This reform's ticket and plan (`docs/plans/2026-05-08-001-feat-prompt-summarizer-content-reform-factual-plan.md`)
