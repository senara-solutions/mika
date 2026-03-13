---
title: "Investigation panel scoping and localStorage persistence"
module: dashboard
problem_type: ui-bugs
date: 2026-03-13
tags:
  - dashboard
  - investigation-panel
  - localStorage
  - react
  - stale-closure
  - scope-indicator
related_issues:
  - "#128"
  - "#130"
---

# Investigation Panel Scoping and localStorage Persistence

## Problem

The dashboard investigation panel had two UX gaps:

1. **No scope indicator (#128):** When opened from different contexts (message, tool call), there was no visual badge showing what was being investigated. No way to investigate a full session without targeting a specific message.
2. **No history persistence (#130):** Closing the panel discarded the entire conversation. Users had to re-ask questions from scratch.

## Solution

### localStorage Persistence Layer (`dashboard/src/lib/investigationStorage.ts`)

A self-contained CRUD module storing all investigations under a single key `mika:investigations`:

```typescript
interface InvestigationStore {
  version: 1
  entries: StoredInvestigation[]
}

interface StoredInvestigation {
  scopeKey: string           // deterministic key from scope
  messages: ChatMessage[]
  lastUpdatedAt: number      // for pruning
  scope: InvestigationScope  // for display
}
```

**Scope key format:** `sess:{sessionId}:agent:{agentId}:msg:{messageId}[:tool:{toolCallIndex}]` or `sess:{...}:session` for full-session scope.

**Pruning on every write:** Remove entries older than 14 days, then cap at 20 most recent. All `localStorage` access wrapped in try/catch for `QuotaExceededError`.

### Scope Badge (`InvestigationPanel.tsx`)

Replaced the old `InvestigationContext` (flat interface with optional fields) with `InvestigationScope` (typed with `type: 'message' | 'tool_call' | 'session'`). Panel header shows an icon + label badge:
- Message #42 (Cpu icon)
- Tool: run_shell (step 3) (Wrench icon)
- Full Session (MessageSquare icon)

### Session-Level Investigation (`SessionDetail.tsx`)

An "Investigate" button in the session info bar opens the panel with `type: 'session'`. Since the API requires `message_id`, the frontend picks the last assistant message via `allMessages.findLast(m => m.role === 'assistant')` as a proxy. The server's context window (5 before + target + 3 after) naturally captures recent activity.

## Key Decisions

| Decision | Rationale |
|----------|-----------|
| localStorage over sessionStorage | History should survive tab closes and browser restarts |
| Proxy message for session investigation | Avoids backend API changes; server's sliding window captures recent context naturally |
| Versioned schema (`version: 1`) | On read, unknown versions reset to empty -- clean migration path without parse errors |
| Single key with indexed entries | Enables global pruning across scopes; `clearAll` is a single `removeItem` |
| `messagesRef` pattern | Prevents stale closure bugs during SSE streaming; removes `messages` from callback deps |

## Review Findings and Fixes

### Stale closure in `sendQuestion`

The initial implementation captured `messages` in the `useCallback` dependency array and read it directly in the closure. During SSE streaming, `messages` updates on every `text_delta`, causing the callback to be recreated on every render -- wasteful and potentially stale.

**Fix:** Introduce `messagesRef = useRef<ChatMessage[]>([])` synced via `useEffect`. The callback reads `messagesRef.current` for history and uses functional `setMessages(prev => ...)` for state changes. Dependency array becomes `[scope, streaming]` only.

### Side effect in `setMessages` updater

The initial `done` handler used `setMessages` as a vehicle to read the latest state:

```typescript
// Anti-pattern: side effect inside state updater
case 'done':
  setMessages((prev) => {
    saveInvestigation(scope, prev)  // localStorage write
    return prev                     // identity return
  })
```

React updaters should be pure. StrictMode may call them twice.

**Fix:** Read from `messagesRef.current` directly:

```typescript
case 'done':
  saveInvestigation(scope, messagesRef.current)
```

### `buildScopeKey` producing `msg:undefined`

If `messageId` was `undefined` (type allows it), the key would contain the literal string `"undefined"`, causing unrelated scopes to collide.

**Fix:** Early guard falls back to session key:

```typescript
if (scope.messageId == null) return `${base}:session`
```

Plus a corresponding guard in `sendQuestion` that shows a user-facing error instead of sending `message_id: 0` to the API.

### Duplicated 429 error handling

Three inline blocks checked for 429/busy strings with slightly different conditions.

**Fix:** Extract `friendlyError()` helper used at all three call sites.

## Prevention

- **Use refs for mutable state accessed in async callbacks.** When a `useCallback` needs the latest state but should not re-create on every state change, use a ref synced via `useEffect`.
- **Never use `setState` updaters for side effects.** If you need to read the latest state and perform a side effect, use a ref.
- **Guard against `undefined` in key construction.** Template literal interpolation silently converts `undefined` to the string `"undefined"`. Always validate or default.
- **Extract shared error normalization.** When the same error detection logic appears in multiple catch blocks, extract a helper.

## Related

- [Investigation panel SSE architecture](../architecture/investigation-panel-sse-agent-loop.md) -- core SSE streaming and read-only agent loop
- [Investigation panel Shift+Enter fix](dashboard-investigation-panel-shift-enter-newline.md) -- textarea UX patterns
- [Dashboard tool calls tabular UX](dashboard-tool-calls-tabular-ux.md) -- tool call display and Tailwind patterns
- [Conditional investigation tool registration](../architecture-patterns/conditional-investigation-tool-registration.md) -- GitHub issue tool
- [Plan document](../../plans/2026-03-13-feat-investigation-panel-scoping-and-persistence-plan.md) -- localStorage schema design
