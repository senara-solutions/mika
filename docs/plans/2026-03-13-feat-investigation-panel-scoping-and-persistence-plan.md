---
title: "Investigation Panel Scoping & History Persistence"
type: feat
status: completed
date: 2026-03-13
---

# Investigation Panel Scoping & History Persistence

Closes #128 and #130.

## Overview

Two improvements to the dashboard investigation panel:

1. **Scope indicator (#128):** Make the investigation scope visible in the panel header — show whether we're investigating a specific message, a tool call, or the full session. Add a "session-level" investigate button so users can investigate without targeting a specific message.
2. **History persistence (#130):** Persist investigation conversations to localStorage so they survive panel close/reopen. Key by investigation scope, prune automatically, add clear-history controls.

Both are frontend-only changes in `dashboard/`. No Rust backend changes.

## Technical Considerations

### API Constraint

The backend `POST /api/v1/investigate` requires `message_id: i64`. For "full session" investigation, the frontend picks the **last assistant message** in the session as a proxy — the server's context window (5 before + target + 3 after) will capture recent conversation naturally. No API changes needed.

### localStorage Schema

```typescript
// Top-level: one localStorage key holds an index of all stored investigations
// Key: "mika:investigations"
// Value: InvestigationStore (JSON)

interface StoredInvestigation {
  scopeKey: string           // e.g. "sess:abc:agent:main:msg:42" or "sess:abc:agent:main:session"
  messages: ChatMessage[]
  lastUpdatedAt: number      // Unix timestamp (ms) for pruning
  scope: InvestigationScope  // For display purposes
}

interface InvestigationScope {
  type: 'message' | 'tool_call' | 'session'
  sessionId: string
  agentId: string
  messageId?: number
  toolCallIndex?: number
  toolName?: string
}

interface InvestigationStore {
  version: 1
  entries: StoredInvestigation[]
}
```

**Key format:** `sess:{sessionId}:agent:{agentId}:msg:{messageId}[:tool:{toolCallIndex}]` for message/tool scopes, `sess:{sessionId}:agent:{agentId}:session` for full-session scope.

### Pruning Strategy

On every write (after SSE `done` event):
1. Remove entries older than 14 days
2. If still > 20 entries, keep only the 20 most recently updated
3. Wrap `localStorage.setItem` in try/catch for `QuotaExceededError`

### Scope Change Behavior

When user clicks "Investigate" on a different message while the panel is open:
- Current component unmounts (abort controller cancels SSE stream)
- New component mounts, loads history for the new scope from localStorage
- If server returns 429 (lock still held from previous investigation), show friendly message: "Previous investigation is still winding down, please try again."

## Acceptance Criteria

- [x] Panel header shows scope badge: "Message #42", "Tool: run_shell (step 3)", or "Full Session"
- [x] Session-level "Investigate" button available in SessionDetail page header
- [x] Session-level investigate picks last assistant message as proxy for API call
- [x] Investigation messages persist to localStorage after each complete assistant response
- [x] History restored on panel reopen for the same scope
- [x] "Restored from previous investigation" divider shown when history is loaded
- [x] History keyed per scope (different messages = different histories)
- [x] Auto-prune: max 20 scopes, max 14 days old
- [x] "Clear" button clears current scope's history
- [x] "Clear all" option clears all investigation history
- [x] `QuotaExceededError` handled gracefully (silent catch, no crash)
- [x] Schema version field (`version: 1`) for future migration support

## Implementation

### Files to modify

1. **`dashboard/src/components/InvestigationPanel.tsx`** — Main changes:
   - Accept `InvestigationScope` instead of current `InvestigationContext`
   - Add scope badge in panel header (message/tool/session indicator)
   - Load history from localStorage on mount
   - Save history to localStorage after each SSE `done` event
   - Add "Clear history" and "Clear all" buttons
   - Show "Restored from previous investigation" divider
   - Handle 429 errors with user-friendly message

2. **`dashboard/src/lib/investigationStorage.ts`** — New file:
   - `loadInvestigation(scopeKey: string): ChatMessage[] | null`
   - `saveInvestigation(scope: InvestigationScope, messages: ChatMessage[]): void`
   - `clearInvestigation(scopeKey: string): void`
   - `clearAllInvestigations(): void`
   - `buildScopeKey(scope: InvestigationScope): string`
   - `pruneInvestigations(store: InvestigationStore): InvestigationStore`
   - All localStorage access wrapped in try/catch

3. **`dashboard/src/pages/SessionDetail.tsx`** — Changes:
   - Add "Investigate Session" button in session header area
   - `openInvestigation` for session-level: find last assistant message, set scope type to `'session'`
   - Update `InvestigationContext` → `InvestigationScope` usage

4. **`dashboard/src/pages/TraceDetail.tsx`** — Changes:
   - Update `InvestigationContext` → `InvestigationScope` usage
   - Pass scope type through to panel

### Implementation order

1. Create `investigationStorage.ts` utility (localStorage CRUD + pruning)
2. Update `InvestigationPanel.tsx` (scope display, persistence hooks, clear buttons)
3. Update `SessionDetail.tsx` (session-level button, scope type)
4. Update `TraceDetail.tsx` (scope type passthrough)

## Sources

- Related issues: #128, #130
- Investigation panel architecture: `docs/solutions/architecture/investigation-panel-sse-agent-loop.md`
- Current panel component: `dashboard/src/components/InvestigationPanel.tsx`
- SSE client: `dashboard/src/api/investigate.ts`
- Session page: `dashboard/src/pages/SessionDetail.tsx`
- Trace page: `dashboard/src/pages/TraceDetail.tsx`
