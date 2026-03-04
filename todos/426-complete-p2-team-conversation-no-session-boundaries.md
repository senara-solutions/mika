---
status: complete
priority: p2
issue_id: 426
tags: [code-review, architecture, team-mode, persistence]
dependencies: []
---

# Team conversation persistence lacks session boundaries

## Problem Statement

Team mode saves user messages and deliverables to `~/.mika/teams/{name}/data/mika.db` with `channel_type = "team"`, but there is no session demarcation. When loading history via `load_recent_messages(20, Some(vec!["team".into()]))`, messages from different TUI sessions are mixed together without visual separation. A user who runs 10 team goals across 5 sessions sees the last 20 messages interleaved without context for which session they came from.

## Findings

- Source: architecture-strategist, learnings-researcher
- Agent mode uses `session_id` for grouping and compaction; team mode has no equivalent
- The `conversations` table has a `metadata` JSON column that could carry a session identifier
- The `conversations.created_at` timestamp exists but is not used for session grouping

## Proposed Solutions

### Option A: Insert session marker on TUI startup
- On TUI team mode startup, save a system message like "--- New team session ---" to the DB
- History display naturally shows the boundary
- **Pros:** Simple, no schema changes, backward-compatible
- **Effort:** Small (5 lines in `run_team()`)

### Option B: Use metadata column for session_id
- Generate a UUID session_id when team TUI starts
- Pass it via `save_message_with_metadata()` for all team messages
- Filter or group by session_id when loading history
- **Pros:** More structured, enables per-session history queries
- **Effort:** Medium (thread session_id through to save calls)

## Acceptance Criteria

- [ ] Multiple team TUI sessions are visually distinguishable in history
- [ ] Loading recent messages shows clear session boundaries
