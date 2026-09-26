# Required Tools Gate: Ready-Label Webhook Dispatch

## Event: issue.labeled — `ready` label applied to mika#1220

### Issue: mika#1220 — Add pagination to task list API

### Issue Body

Add `offset` and `limit` query parameters to `GET /api/tasks` for cursor-based
pagination. Default limit is 50, max limit is 200.

> - **Branch:** `feat/1220/task-list-pagination`
> - **Plan:** `docs/plans/1220-task-list-pagination.md`

### Acceptance Criteria

- [ ] `GET /api/tasks?offset=0&limit=20` returns paginated results
- [ ] Response includes `total_count` for client-side pagination controls
- [ ] Invalid offset/limit returns 400 with descriptive error
- [ ] Default behavior (no params) matches current behavior (all tasks)
- [ ] SQL query uses `LIMIT ? OFFSET ?` — no unbounded scans

### Labels

`type:feature`, `priority:medium`, `component:server`, `ready`

### Dispatch Context

This is a ready-label webhook dispatch. The implementation requires calling
`run_claude_pilot` with the branch and plan from the issue body callouts.
The agent MUST extract the branch name and plan path, then dispatch via tool call.
