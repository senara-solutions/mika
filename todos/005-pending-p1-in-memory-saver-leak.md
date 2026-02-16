---
status: pending
priority: p1
issue_id: "005"
tags: [code-review, performance, architecture]
dependencies: []
---

# InMemorySaver Checkpointer is Unbounded Memory Leak

## Problem Statement

`app/agent/graph.py` uses LangGraph's `InMemorySaver` as the checkpointer. This stores all conversation state in process memory with no eviction. Under production load, memory will grow unboundedly until the process is OOM-killed.

**Why it matters:** Production service will crash under sustained usage.

## Findings

- **Source:** Performance Oracle (CRITICAL-6), Architecture Strategist (R4), Code Simplicity
- **Location:** `app/agent/graph.py` — `memory = InMemorySaver()`
- **Evidence:** No TTL, no size limit, no eviction policy

## Proposed Solutions

### Option A: Switch to PostgresSaver (Recommended)
- Use LangGraph's `PostgresSaver` backed by existing PostgreSQL database
- Provides persistence across restarts and bounded memory usage
- **Pros:** Production-ready; uses existing infra; conversation state survives restarts
- **Cons:** Slightly higher latency per checkpoint; needs connection pool config
- **Effort:** Medium
- **Risk:** Low

### Option B: Switch to RedisSaver
- Use LangGraph's Redis-backed checkpointer
- **Pros:** Fast; Redis already in stack
- **Cons:** May require additional Redis memory; less durable than Postgres
- **Effort:** Medium
- **Risk:** Low

### Option C: Add TTL wrapper around InMemorySaver
- Create a wrapper that evicts entries older than N hours
- **Pros:** Quick fix
- **Cons:** Still in-memory; still loses state on restart; not production-grade
- **Effort:** Small
- **Risk:** High

## Recommended Action
<!-- Filled during triage -->

## Technical Details

**Affected files:**
- `app/agent/graph.py`
- Potentially `app/config.py` for checkpointer config

## Acceptance Criteria

- [ ] Checkpointer uses persistent storage (Postgres or Redis)
- [ ] Memory usage is bounded
- [ ] Conversation state survives process restarts
- [ ] Existing agent tests still pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | Identified by 3 agents |

## Resources

- LangGraph Persistence: PostgresSaver documentation
