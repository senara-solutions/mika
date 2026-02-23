# Letta & OpenClaw Evaluation Brainstorm

**Date:** 2026-02-23
**Status:** Resolved
**Participants:** User, Claude

## What We Were Exploring

Whether to adopt Letta (formerly MemGPT) or OpenClaw as a foundation for Mika instead of the current LangGraph + Neo4j + custom adapters stack.

**Motivation:** General landscape exploration before committing further. The user already uses OpenClaw personally and wanted to evaluate whether building on an existing agent platform would accelerate time to market.

## Product Context

Mika's roadmap (established during this brainstorm):

- **Phase 1 (0-3mo):** SaaS for execs, 20-30 paying users, prove retention
- **Phase 2 (3-6mo):** Multi-user, team features, first B2B pilot
- **Phase 3 (6-12mo):** White-label offering for firms/companies
- **Phase 4 (12mo+):** Platform/API if B2B demand justifies

**Core thesis:** Memory + personalization engine is the moat. Every interaction makes the product smarter per user. EAs cost 60-100k EUR/year in France; execs will pay 200-500 EUR/month for an AI that genuinely knows them. B2B contracts (50-200k EUR/year) are the real prize.

## Platforms Evaluated

### Letta (formerly MemGPT)

- **What:** Memory-first agent platform. Python, Apache-2.0, 21k GitHub stars.
- **Strongest feature:** Tiered memory (core/archival/recall) with autonomous self-management. Agent decides what to remember, forget, and retrieve.
- **Weaknesses for Mika:** Not a workflow engine (wouldn't replace LangGraph). No channels, no proactive scheduling. Would only replace the Neo4j memory layer.
- **Verdict:** Interesting memory patterns to study, but doesn't deliver enough to justify a dependency.

### OpenClaw

- **What:** Fully built personal AI agent. TypeScript, MIT, 208k GitHub stars.
- **Strongest features:** 15+ messaging channels, 100+ skills, massive community, skills marketplace.
- **Weaknesses for Mika:** Single-user architecture (no multi-tenancy), TypeScript (not Python), 512 security vulnerabilities in audit, 12% malicious skills in marketplace, flat-file memory (downgrade from Neo4j). Creator hired by OpenAI.
- **User experience:** Already uses OpenClaw personally. Values the channel coverage, community momentum, speed to market, and skills ecosystem.
- **Verdict:** Compelling as a product to use, but building Mika *on top of* OpenClaw would mean inheriting its architecture constraints (single-user, flat memory) while still needing to build the core differentiator (deep memory/personalization) from scratch.

## Key Decision

**Neither integrate nor depend on either platform. Instead, study both codebases and cherry-pick the best patterns.**

The turning point: when evaluating how to build the memory moat on top of OpenClaw's flat files, it became clear that the things that make Mika a *product* (deep personalization, proactive intelligence, executive workflows) are custom by nature. No existing platform delivers them.

## What to Learn From Each

### From OpenClaw (cloned for study)
- Channel adapter architecture (hub-and-spoke, gateway pattern)
- Skills system design (Markdown/YAML skill definitions, self-extending agent)
- UX patterns for multi-channel messaging
- Community/marketplace model (ClawHub)

### From Letta (cloned for study)
- Memory hierarchy design (core/archival/recall — MemGPT paper)
- Autonomous memory self-editing via tool calls
- Agent state persistence patterns
- Memory management without unbounded context growth

### For Mika's Stack
- Stay Python 3.12+ with current stack (LangGraph + Neo4j + Celery + FastAPI)
- Incorporate Letta's memory patterns into the Neo4j memory layer
- Study OpenClaw's channel architecture for improving Telegram/WhatsApp adapters
- The custom stack *is* the moat — own every layer

## Open Questions

None — both repos cloned for reverse engineering. Patterns will be extracted and applied incrementally.

## Next Steps

1. Reverse-engineer OpenClaw and Letta codebases (in progress)
2. Extract applicable patterns for memory, channels, and skills
3. Continue resolving the 24 code review todos (P1 items first)
4. Get to Phase 1 MVP with 20-30 paying exec users
