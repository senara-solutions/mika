---
title: "Dashboard: add copy-to-clipboard and create-GitHub-issue actions"
type: feat
status: active
date: 2026-03-12
---

# Dashboard: add copy-to-clipboard and create-GitHub-issue actions

## Overview

Two quality-of-life improvements to the observability dashboard: (1) copy buttons on assistant messages and investigation responses, (2) a `create_github_issue` tool for the investigation agent.

## Problem Statement

- No way to copy assistant message content without manual text selection in `SessionDetail.tsx`
- No way to copy investigation responses in `InvestigationPanel.tsx`
- Investigation agent is purely read-only — users who discover bugs during investigation must manually create GitHub issues, losing context

## Proposed Solution

### Part 1: Copy to Clipboard

**Extract `CopyButton` to shared component:**
- Currently defined inline in `SessionDetail.tsx` (lines 91-122) — not exported
- Extract to `dashboard/src/components/CopyButton.tsx` so both `SessionDetail.tsx` and `InvestigationPanel.tsx` can import it
- Preserve existing behavior: `navigator.clipboard.writeText()`, Check icon for 2s, opacity transition, silent failure

**Add copy button to assistant message cards in `SessionDetail.tsx`:**
- In `renderRegularMessageCard` (line 605): add `CopyButton` to the message header row next to the existing investigate (Search) button, for `role === 'assistant'` messages
- In `renderTeamMessageCard` (line 538): add `CopyButton` to team agent response cards for consistency
- Text source: `msg.content`

**Add copy button to investigation responses in `InvestigationPanel.tsx`:**
- Add `CopyButton` to assistant message cards in the chat history (lines 219-252)
- Position: top-right corner of the assistant message card, matching the pattern from `SessionDetail.tsx`
- Show the button even during streaming — users understand partial copy. The button copies whatever `msg.content` currently holds

### Part 2: Investigation Agent — GitHub Issue Creation

**New env vars (optional, both required together):**
- `MIKA_GITHUB_TOKEN` — GitHub Personal Access Token (needs `repo` scope for private repos, or `public_repo` for public)
- `MIKA_GITHUB_REPO` — Target repository in `owner/repo` format (e.g., `senara-solutions/mika`)

**Settings updates (`crates/mika-common/src/config.rs`):**
- Add `github_token: Option<String>` and `github_repo: Option<String>` with `#[serde(default)]`
- Update manual `Debug` impl to redact `github_token`

**New tool: `CreateGithubIssueTool` (in `crates/mika-agent/src/server/investigate.rs`):**
- Struct fields: `http_client: reqwest::Client`, `github_token: String`, `github_repo: String`
- Input schema: `title` (required string), `body` (required string)
- The tool appends a context footer to the body:
  ```markdown
  ---
  **Investigation Context**
  - Session: {session_id}
  - Agent: {agent_id}
  - Trace: {trace_id}
  ```
  The investigation agent composes the title and body with relevant details from its tools; the tool adds metadata.
- Uses `reqwest` POST to `https://api.github.com/repos/{owner}/{repo}/issues` with `Authorization: Bearer {token}` and `User-Agent: mika-dashboard`
- Returns `"Created issue #{number}: {html_url}"` on success
- Error handling: map HTTP status codes to human-readable errors (401 → invalid token, 403 → insufficient scopes, 404 → repo not found, 422 → validation error, 429 → rate limited)
- `timeout_secs()`: `Some(10)` (matching other investigation tools)

**Tool registration (conditional):**
- In `build_investigation_tools()`: only register `CreateGithubIssueTool` when both `github_token` and `github_repo` are present in Settings
- Pass `http_client` from `AppState` (no new client)
- Thread `github_token` and `github_repo` through `AppState` or read from Settings at tool construction time

**Investigation context propagation:**
- The tool needs `session_id`, `agent_id`, and `trace_id` from the investigation context to append to the issue body
- Pass these through the `ToolContext` (already has `session_id` and `trace_id` as dummy values — populate them with the actual investigation context values)
- `agent_id` can come from the investigation request body (already present as `agent_id` field)

**System prompt update:**
- Current prompt (line 476): "You have read-only access to the database"
- Update to: "You have read-only access to the database. You can also create GitHub issues to track findings using the `create_github_issue` tool."
- Only include this sentence when the tool is registered (GitHub token configured)

**AppState changes (`crates/mika-agent/src/server/state.rs`):**
- Add `github_token: Option<String>` and `github_repo: Option<String>` fields
- Populated from `Settings` during server startup
- Passed to `build_investigation_tools()` for conditional tool construction
- The `investigation_tools` OnceCell pattern needs adjustment — currently lazily initialized without access to these config values. Either: (a) initialize eagerly during AppState construction, or (b) pass config into the lazy initializer. Option (a) is simpler.

## Technical Considerations

- **No new crates**: Uses existing `reqwest` (already a dependency) for GitHub API calls
- **Security**: `github_token` redacted in Debug impl, scrubbed from child processes by existing MIKA_* scrubbing
- **Dashboard auth scope**: The investigation endpoint accepts `MIKA_DASHBOARD_TOKEN` (read-only). Creating GitHub issues is an external mutation but does not modify Mika's state. This is acceptable — the dashboard token controls access to the investigation agent, and the GitHub token controls what the agent can do externally
- **Graceful degradation**: When GitHub token/repo is not configured, the tool is simply not registered. The investigation agent cannot create issues, and the system prompt does not mention the capability
- **Rate limiting**: The existing `MAX_INVESTIGATION_STEPS = 5` cap on tool calls per investigation provides natural guardrails against spam issue creation
- **GitHub API**: Hard-code `api.github.com` (no GitHub Enterprise support — YAGNI)

## Acceptance Criteria

- [x] `CopyButton` extracted to `dashboard/src/components/CopyButton.tsx` and imported in both consumers
- [x] Assistant messages in `SessionDetail.tsx` have a copy button (regular and team message cards)
- [x] Investigation panel responses in `InvestigationPanel.tsx` have a copy button
- [x] `MIKA_GITHUB_TOKEN` and `MIKA_GITHUB_REPO` env vars added to `Settings` with redacted Debug
- [x] `CreateGithubIssueTool` implemented following existing investigation tool pattern
- [x] Tool registered conditionally only when both GitHub env vars are set
- [x] Investigation system prompt updated when tool is available
- [x] Proper error handling for GitHub API failures (401, 403, 404, 422, 429)
- [x] `.env.example` updated with new env vars
- [x] `cargo clippy` passes
- [x] `cargo test` passes
- [x] No new crate dependencies

## Implementation Order

1. Extract `CopyButton` to shared component
2. Add copy buttons to `SessionDetail.tsx` assistant messages
3. Add copy buttons to `InvestigationPanel.tsx` responses
4. Add `github_token`/`github_repo` to `Settings` + Debug redaction
5. Implement `CreateGithubIssueTool`
6. Register tool conditionally in `build_investigation_tools()`
7. Update investigation system prompt
8. Update `AppState` and tool initialization
9. Update `.env.example`
10. Run `cargo clippy` + `cargo test`

## Sources & References

- Existing `CopyButton`: `dashboard/src/pages/SessionDetail.tsx:91-122`
- Investigation tools: `crates/mika-agent/src/server/investigate.rs`
- Settings: `crates/mika-common/src/config.rs`
- AppState: `crates/mika-agent/src/server/state.rs`
- Investigation panel: `dashboard/src/components/InvestigationPanel.tsx`
- GitHub Issues API: `POST /repos/{owner}/{repo}/issues`
- Related issue: #101
