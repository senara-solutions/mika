---
title: "feat(dashboard): rebuild Dev Run detail page with narrative content"
type: feat
status: completed
date: 2026-04-04
issue: "#438"
---

# feat(dashboard): Rebuild Dev Run Detail Page with Narrative Content

## Overview

The Dev Run detail page currently shows only metadata (IDs, timestamps, costs) in two flat cards. It doesn't answer "What was this about? What changed? What decisions were made?" This plan rebuilds it into a narrative page with issue context, pipeline progress, PR summary, agent activity, and QA verdict.

## Problem Statement

The current `DevRunDetail.tsx` (126 lines) is a metadata dump: two `bg-bg-card` cards showing IDs, timestamps, and claude_pilot fields. A developer looking at a completed dev run can't understand:
- What issue/task was being worked on
- How far the pipeline got (plan/work/review/compound/PR/QA)
- What the PR contains (title, files changed, description)
- What happened during the agent sessions (tool calls, errors)
- Whether QA passed or failed

## Proposed Solution

### Backend: Two GitHub API proxy endpoints

Add two new endpoints to `crates/mika-agent/src/server/dashboard_dev_runs.rs`:

1. **`GET /api/v1/github/issues/{owner}/{repo}/{number}`** — Proxies to `https://api.github.com/repos/{owner}/{repo}/issues/{number}`. Returns: title, body, labels, state.

2. **`GET /api/v1/github/pulls/{owner}/{repo}/{number}`** — Proxies to `https://api.github.com/repos/{owner}/{repo}/pulls/{number}`. Returns: title, body, state, additions, deletions, changed_files, merged, reviews (via separate `/reviews` call).

Both endpoints:
- Use `AppState.github_token` for auth (already available)
- Protected by dashboard/internal token auth (same as other `/api/v1/*` routes)
- Return `503 Service Unavailable` with `{"error": "GitHub token not configured"}` when no token
- Return `502 Bad Gateway` with upstream error on GitHub API failures
- Use the shared `AppState.http_client` (reqwest)
- **Scope restriction:** Only allow `owner/repo` combinations that appear in existing task `reference_url` fields (defense-in-depth, not a hard security boundary since dashboard token already gates access)

### Frontend: Rebuild DevRunDetail.tsx

Replace the current two-card layout with seven sections:

1. **Run Header** — Title from task label (parsed: strip `[self_dev]` prefix), status badge, stats row (cost, duration, turns, files changed from PR)
2. **Issue Card** (collapsible, default open) — Markdown-rendered issue body via `MarkdownContent` from `@senara-solutions/ui`
3. **Pipeline Timeline** — Horizontal step indicator: Plan > Work > Review > Compound > PR > QA
4. **PR Summary Card** — Title, additions/deletions stats, description as markdown
5. **Agent Activity** — Expandable session list using existing `useTaskSessions()`, with inline message preview per session
6. **QA Verdict Card** — Extracted from PR reviews (latest review with APPROVED/CHANGES_REQUESTED/COMMENTED state)
7. **Claude Pilot Metadata** (collapsed by default) — Branch, session ID, tokens, raw JSON

### New shared component: CollapsibleCard

Extract to `dashboard/src/components/CollapsibleCard.tsx` — a `bg-bg-card` wrapper with chevron toggle. Keep it in the dashboard for now (not `packages/ui/`) since no other consumer exists yet.

## Technical Approach

### Phase 1: Backend — GitHub Proxy Endpoints

**Files to modify:**
- `crates/mika-agent/src/server/dashboard_dev_runs.rs` — Add handlers
- `crates/mika-agent/src/server/mod.rs` — Register routes

**New types in `dashboard_dev_runs.rs`:**

```rust
#[derive(Serialize)]
pub struct GitHubIssueResponse {
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub labels: Vec<GitHubLabel>,
}

#[derive(Serialize)]
pub struct GitHubLabel {
    pub name: String,
    pub color: String,
}

#[derive(Serialize)]
pub struct GitHubPullResponse {
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub additions: u32,
    pub deletions: u32,
    pub changed_files: u32,
    pub merged: bool,
    pub reviews: Vec<GitHubReview>,
}

#[derive(Serialize)]
pub struct GitHubReview {
    pub user: String,
    pub state: String, // APPROVED, CHANGES_REQUESTED, COMMENTED
    pub body: Option<String>,
    pub submitted_at: Option<String>,
}
```

**Handler pattern:**
```rust
pub async fn handle_github_issue(
    State(state): State<AppState>,
    Path((owner, repo, number)): Path<(String, String, u32)>,
) -> impl IntoResponse {
    let Some(token) = &state.github_token else {
        return (StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "GitHub token not configured"}))).into_response();
    };
    // ... reqwest GET with Bearer token, deserialize, map to response type
}
```

**Route registration in `mod.rs`:**
```rust
// Inside dashboard_routes:
.route("/github/issues/{owner}/{repo}/{number}", get(dashboard_dev_runs::handle_github_issue))
.route("/github/pulls/{owner}/{repo}/{number}", get(dashboard_dev_runs::handle_github_pull))
```

**Error handling:**
- GitHub 404 → return 404 with `{"error": "Issue not found"}`
- GitHub 403 (rate limit) → return 502 with `{"error": "GitHub rate limit exceeded"}`
- GitHub 401 (bad token) → return 502 with `{"error": "GitHub authentication failed"}`
- Network errors → return 502 with `{"error": "GitHub API unreachable"}`

### Phase 2: Frontend API Layer

**New file: `dashboard/src/api/github.ts`**

```typescript
export interface GitHubIssue {
  title: string
  body: string | null
  state: string
  labels: Array<{ name: string; color: string }>
}

export interface GitHubReview {
  user: string
  state: string
  body: string | null
  submitted_at: string | null
}

export interface GitHubPull {
  title: string
  body: string | null
  state: string
  additions: number
  deletions: number
  changed_files: number
  merged: boolean
  reviews: GitHubReview[]
}

export function useGitHubIssue(owner: string | null, repo: string | null, number: number | null) {
  return useQuery<GitHubIssue>({
    queryKey: ['github-issue', owner, repo, number],
    queryFn: () => apiFetch(`/github/issues/${owner}/${repo}/${number}`),
    enabled: !!owner && !!repo && !!number,
    staleTime: 5 * 60 * 1000, // 5 min — issue body doesn't change often
  })
}

export function useGitHubPull(owner: string | null, repo: string | null, number: number | null) {
  return useQuery<GitHubPull>({
    queryKey: ['github-pull', owner, repo, number],
    queryFn: () => apiFetch(`/github/pulls/${owner}/${repo}/${number}`),
    enabled: !!owner && !!repo && !!number,
    staleTime: 2 * 60 * 1000, // 2 min — PR stats change more often
  })
}
```

**Helper: `parseGitHubUrl(url: string)`** — extracts `{owner, repo, number, type: 'issue'|'pull'}` from GitHub URLs like `https://github.com/senara-solutions/mika/issues/438`. Add to `dashboard/src/utils/github.ts`.

### Phase 3: Frontend Components

**New file: `dashboard/src/components/CollapsibleCard.tsx`**

```typescript
interface CollapsibleCardProps {
  title: string
  defaultOpen?: boolean
  badge?: React.ReactNode
  children: React.ReactNode
}
```

Uses `useState` for open/closed. Chevron icon rotates on toggle. Standard `bg-bg-card border border-white/[0.05] rounded-2xl` styling.

**New file: `dashboard/src/components/PipelineTimeline.tsx`**

Horizontal step indicator showing: Plan > Work > Review > Compound > PR > QA. Each step has three states: completed (green circle), active (blue pulse), pending (gray). Steps are derived from task metadata and child task labels:

- **Plan** — completed if task has any child session
- **Work** — completed if task has a branch in metadata
- **Review** — completed if task has a PR URL in metadata
- **Compound** — completed if task status is `completed`
- **PR** — completed if PR exists (pr_url set)
- **QA** — completed if PR has reviews; state determined by review verdict

### Phase 4: Rebuild DevRunDetail.tsx

Replace the current 126-line component with the new narrative layout. Data flow:

1. `useDevRun(taskId)` — existing hook, provides metadata
2. `parseGitHubUrl(run.reference_url)` — extract issue owner/repo/number
3. `useGitHubIssue(owner, repo, issueNumber)` — fetch issue body
4. `parseGitHubUrl(run.pr_url)` or derive from `run.repo` + `run.pr_number` — extract PR owner/repo/number
5. `useGitHubPull(owner, repo, prNumber)` — fetch PR details + reviews
6. `useTaskSessions(taskId)` — existing hook from #437
7. `useTaskChildren(taskId)` — existing hook, for child task activity

**Session messages:** Use `useSessionMessages(sessionId)` (already exists at `dashboard/src/api/sessions.ts`) inside expandable session rows. Lazy-load on expand (enabled only when expanded).

**Section layout:**

```
[← Back to Dev Runs]

# Task Label (cleaned)
[status badge] [cost] [duration] [turns] [files changed]

┌─ Issue ────────────────────────────┐
│ (collapsible, open by default)     │
│ Markdown-rendered issue body       │
└────────────────────────────────────┘

┌─ Pipeline ─────────────────────────┐
│ [Plan]→[Work]→[Review]→[PR]→[QA]  │
└────────────────────────────────────┘

┌─ Pull Request ─────────────────────┐
│ PR title, +additions/-deletions    │
│ PR description (markdown)          │
└────────────────────────────────────┘

┌─ Agent Activity ───────────────────┐
│ Session 1 (expandable) → messages  │
│ Session 2 (expandable) → messages  │
└────────────────────────────────────┘

┌─ QA Verdict ───────────────────────┐
│ Review state + reviewer + body     │
└────────────────────────────────────┘

┌─ Claude Pilot Metadata ───────────┐
│ (collapsed by default)             │
│ Branch, repo, session ID, etc.     │
└────────────────────────────────────┘
```

## Edge Cases & Error Handling

- **No `reference_url`:** Issue card shows "No linked issue" empty state, pipeline still renders
- **No `pr_url` / `pr_number`:** PR card shows "No PR yet" — run may still be in progress
- **GitHub token not configured:** Issue and PR cards show "GitHub integration not available" with subtle info styling (not error red)
- **GitHub API errors (rate limit, 404):** Individual cards show error with retry button, other sections remain functional
- **Partial `claude_pilot` metadata:** All fields are optional — each section degrades gracefully using `?? '—'` pattern already established
- **In-progress runs:** Pipeline shows current stage as active (pulsing blue), PR/QA sections show "Pending" state
- **Legacy tasks (pre-metadata schema):** Falls back to current metadata-only view
- **Session messages too large:** Session expand shows first 50 messages with "Load more" pattern, not full dump

## Acceptance Criteria

- [x] `GET /api/v1/github/issues/:owner/:repo/:number` returns issue title/body/labels/state
- [x] `GET /api/v1/github/pulls/:owner/:repo/:number` returns PR title/body/stats/reviews
- [x] Both endpoints return 503 when `MIKA_GITHUB_TOKEN` is not configured
- [x] Dev Run detail page shows issue description as markdown (via `MarkdownContent`)
- [x] Pipeline timeline shows 5 stages with completion status derived from task metadata
- [x] PR summary card shows title, +/- stats, description as markdown
- [x] Session messages visible inline (expandable per-session, lazy-loaded)
- [x] QA verdict displayed from PR reviews (APPROVED/CHANGES_REQUESTED/COMMENTED)
- [x] CollapsibleCard component works for issue, QA, and metadata sections
- [x] All existing tests pass (`cargo test`)
- [x] Dashboard builds without errors (`npm run build -w dashboard`)
- [x] Graceful degradation: page usable when GitHub token is not configured

## Implementation Order

1. Backend: GitHub proxy handlers + route registration + tests
2. Frontend: `github.ts` API hooks + `parseGitHubUrl` utility
3. Frontend: `CollapsibleCard.tsx` + `PipelineTimeline.tsx` components
4. Frontend: Rebuild `DevRunDetail.tsx` with all sections
5. Integration test: verify full page renders with mock data

## Sources & References

- Existing pattern: `dashboard/src/pages/TaskDetail.tsx` (282 lines) — stacked cards, MetadataRow, sessions, children
- Existing pattern: `dashboard/src/pages/DevRunDetail.tsx` (126 lines) — current implementation to replace
- Backend pattern: `crates/mika-agent/src/server/dashboard_dev_runs.rs` — DevRunResponse type, claude_pilot metadata extraction
- Shared UI: `packages/ui/` — MarkdownContent, TaskStatusBadge, CopyButton, formatRelativeTime
- API client: `dashboard/src/api/client.ts` — apiFetch pattern with auth token
- Sessions API: `dashboard/src/api/tasks.ts` — useTaskSessions hook (#437)
- GitHub issue: #438
