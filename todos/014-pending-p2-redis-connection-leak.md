---
status: pending
priority: p2
issue_id: "014"
tags: [code-review, performance]
dependencies: []
---

# New Redis Connection Per Rate Limiter Call

## Problem Statement

The rate limiter creates a new Redis connection for every call instead of using a connection pool. Under load, this exhausts file descriptors and adds latency.

## Findings

- **Source:** Performance Oracle (CRITICAL-3)
- **Location:** Rate limiter module (creates `Redis()` instance per invocation)

## Proposed Solutions

### Option A: Use a shared Redis connection pool (Recommended)
- Create a `redis.asyncio.ConnectionPool` at module level
- Pass pool to Redis client instances
- **Effort:** Small | **Risk:** Low

## Acceptance Criteria

- [ ] Redis connections are pooled
- [ ] No new connection created per rate-limit check
- [ ] Connection pool has reasonable limits configured

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | |
