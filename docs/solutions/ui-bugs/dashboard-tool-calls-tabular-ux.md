---
title: Dashboard Tool Call Summaries — Tabular UX with Quick-Copy and Investigation
date: 2026-03-09
category: ui-bugs
tags:
  - dashboard
  - observability
  - tool-calls
  - ux-design
  - tailwind
  - click-to-copy
modules:
  - dashboard/src/pages/SessionDetail.tsx
  - dashboard/src/pages/Timeline.tsx
  - dashboard/src/components/InvestigationPanel.tsx
  - crates/mika-agent/src/agent.rs
severity: low
symptoms:
  - Tool call metadata stored in messages but not visible in the dashboard
  - No way to inspect tool inputs/outputs without reading raw JSON
  - Agent filter dropdown showed agents but selecting one returned no results
  - Messages subsystem badge dot was invisible in production builds
root_cause: >
  Tool call summaries were serialized to the messages.metadata JSON column but
  the dashboard had no UI to display them. Additionally, the Timeline page had
  two bugs: the agent filter sent display name instead of agent_id, and Tailwind
  purged dynamically-generated bg-* classes at build time.
---

# Dashboard Tool Call Summaries — Tabular UX

## Problem

The agent loop stored tool call summaries (name, input, output, success) in the
`messages.metadata` JSON column, but the dashboard's SessionDetail page showed
only message content — tool execution details were invisible. Developers had to
read raw database JSON to understand what tools ran and what they returned.

## Solution

### Tool Call Table Component

A collapsible tabular view rendered below each assistant message that has tool
call metadata:

```
┌─ "N tool calls" header with wrench icon
├─ Table Header: [▶] [Status] [Tool] [Input] [Output] [🔍]
├─ Row 1: ▶ ● run_shell  | command: ls -la  | total 42...   | 🔍
├─ Row 2: ▶ ● store_fact  | name: preference | Stored...     | 🔍
└─ (click row to expand full input/output with Copy All buttons)
```

### Data Flow

**Backend** (`agent.rs`): `ToolCallSummary` struct with `step`, `name`,
`input_summary`, `output_summary`, `success`. Serialized to JSON in
`messages.metadata` column. Per-field truncation was removed (commit 9d6dfd6)
so the dashboard receives full data. Total metadata capped at
`TOOL_METADATA_MAX` (4000 bytes) — entries dropped from tail if exceeded.

**Frontend** (`SessionDetail.tsx`): `parseToolCalls(metadata)` extracts the
`tool_calls` array from the JSON metadata string. Graceful fallback on parse
errors (returns empty array).

### Quick-Copy Pills

For common tools, a primary field is extracted and displayed as a clickable
pill badge for one-click copying:

```typescript
const QUICK_COPY_KEYS: Record<string, string> = {
  run_shell: 'command',
  read_workspace: 'path',
  search_memory: 'query',
  store_fact: 'name',
  delegate_task: 'task',
  web_search: 'query',
  run_team: 'goal',
  // ... 17 mappings total
}
```

The pill shows the key name (e.g., "command") with the value next to it,
plus a dedicated copy button. Falls back to showing full truncated input
for unmapped tools.

### CopyButton Component

Inline clipboard copy with 2-second green checkmark feedback.
`stopPropagation()` prevents row expansion when clicking copy.
Silent failure in restricted environments (no clipboard API).

### Click-to-Expand

Each row toggles between summary (truncated) and full detail view.
Expanded view shows complete input JSON and output text with
`whitespace-pre-wrap break-all` formatting and "Copy all" buttons.

### Client-Side Truncation

```typescript
function truncateText(text: string, maxLen = 80): string {
  const cleaned = text.endsWith('...') ? text.slice(0, -3) : text
  if (cleaned.length <= maxLen) return text
  return cleaned.slice(0, maxLen) + '...'
}
```

Strips backend trailing `...` before re-truncating to avoid double-ellipsis.

## Timeline Page Fixes

### Agent Filter (commit 8b444b4, fixed in f537c1f)

The agent dropdown sent `a.name` (display name) as the filter value, but the
backend `unified_timeline` view stores `agent_id`. Fix: `value={a.id}`.

### Subsystem Badge Dots (commit f537c1f)

The subsystem badge dot used dynamic class generation:
```tsx
// Before — Tailwind purges bg-blue-400 (never in source literally)
className={badge.text.replace('text-', 'bg-')}

// After — explicit dot field, Tailwind sees it in source
{ dot: 'bg-blue-400', ... }
className={badge.dot}
```

Added explicit `dot` field to `eventTypeBadge()` return type to ensure
Tailwind includes all dot color classes in production builds.

## Prevention Strategies

### Tailwind dynamic class rule

Never generate Tailwind classes via string manipulation at runtime.
Tailwind's JIT compiler scans source files for class names — dynamically
constructed classes (e.g., `replace('text-', 'bg-')`) are invisible to
the scanner and will be purged in production builds. Always use explicit
class names or an explicit mapping object.

### Filter value consistency

When building filter dropdowns from API data, always use the field that
matches the backend's filter parameter. If the API returns `{ id, name }`,
use `id` as the `<option>` value when the backend filters by `id`.

### Metadata schema stability

Tool call metadata is serialized as untyped JSON. Frontend parsing must
be defensive (`try/catch`, fallback to empty array). Changes to the
backend `ToolCallSummary` struct should be backwards-compatible — add
fields with defaults, never remove or rename.

## Related Documentation

- [Investigation Panel](../architecture/investigation-panel-sse-agent-loop.md)
- [Observability Dashboard](../architecture/observability-otel-tui-dashboard.md)
- [Trace ID Correlation](../architecture-patterns/trace-id-correlation-unified-observability.md)
