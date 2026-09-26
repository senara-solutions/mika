---
title: "GitHub API proxy endpoints for dashboard dev run detail"
category: dashboard-issues
date: 2026-04-04
tags: [dashboard, github-api, proxy, dev-run, react, axum]
issue: 438
---

# GitHub API Proxy Endpoints for Dashboard Dev Run Detail

## Problem

The Dev Run detail page only showed metadata (IDs, timestamps, costs) but couldn't answer "what was this about?" or "what changed?" because issue descriptions and PR details live in GitHub, and the dashboard frontend can't call GitHub directly (token is server-side only).

## Root Cause

No server-side proxy existed to forward GitHub API requests on behalf of the dashboard. The `MIKA_GITHUB_TOKEN` was available in `AppState` but only used in agent tool contexts, not exposed to dashboard consumers.

## Solution

### Backend: Two proxy endpoints in `dashboard_dev_runs.rs`

Added to the existing dashboard routes (under dashboard/internal token auth):

- `GET /api/v1/github/issues/{owner}/{repo}/{number}` — proxies to GitHub Issues API, returns title/body/labels/state
- `GET /api/v1/github/pulls/{owner}/{repo}/{number}` — proxies to GitHub Pulls API, fetches PR + reviews in parallel via `tokio::join!`

Key patterns:
- **Input validation**: `is_valid_github_name()` validates owner/repo against `[a-zA-Z0-9._-]+` to prevent SSRF-like probing
- **Graceful token absence**: Returns `503 Service Unavailable` with `{"error": "GitHub token not configured"}` — frontend shows "GitHub integration not available" instead of error red
- **Error mapping**: GitHub 404 → 404, GitHub 401/403 → 502, network errors → 502. Generic messages, no token leakage
- **Reviews are best-effort**: If review fetch fails, returns empty array (PR data still served)
- **Reviews capped**: `?per_page=20` on the reviews URL to prevent unbounded payloads

### Frontend: Rebuilt DevRunDetail.tsx

Replaced the 126-line metadata dump with a narrative page (~400 lines):

1. **Run header** with stats row (cost, duration, turns, files changed)
2. **Issue card** (collapsible) — markdown-rendered via `MarkdownContent`
3. **Pipeline timeline** — 4-step horizontal indicator (Plan > Work > PR > QA)
4. **PR summary card** — title, +/- stats, description
5. **Agent activity** — expandable sessions with lazy-loaded messages
6. **QA verdict** — extracted from PR reviews
7. **Claude Pilot metadata** (collapsed by default)

New components: `CollapsibleCard.tsx`, `PipelineTimeline.tsx`
New API: `useGitHubIssue`, `useGitHubPull` with `staleTime` caching
New utility: `parseGitHubUrl()` extracts owner/repo/number from GitHub URLs

### Key design decisions

- **Backend proxy over frontend direct calls**: Token stays server-side, CORS not needed, rate limiting is one concern not two
- **No backend caching**: React Query `staleTime` (2-5 min) is sufficient for a single-user dashboard. In-memory cache can be added later if needed
- **Separate internal/public API types in Rust**: `github_api` module deserializes full GitHub responses; public types (`GitHubIssueResponse`, `GitHubPullResponse`) expose only needed fields. This decouples us from GitHub API changes

## Prevention

- When adding dashboard features that need external API data, add a backend proxy rather than trying to call from the frontend. The auth token pattern is established in `dashboard_dev_runs.rs`
- Always validate path parameters that become part of outbound URLs (SSRF mitigation)
- Use `staleTime` on React Query hooks for data that doesn't change frequently

## Related

- [Adding RESTful detail pages pattern](add-restful-detail-pages-pattern.md) — established the dashboard detail page pattern
- [Task-session bidirectional linking](task-session-bidirectional-linking.md) — the `useTaskSessions` hook used here
