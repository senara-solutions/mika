---
title: "Task tree visualization with recursive descendants endpoint"
date: 2026-05-05
category: dashboard-issues
module: dashboard, mika-agent
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding hierarchical/tree visualization to dashboard pages
  - Querying multi-level task relationships (parent → children → grandchildren)
  - Replacing flat list sections with collapsible tree components
tags: [dashboard, task-tree, recursive-cte, collapsible, tree-visualization, descendants, react, sqlite]
---

# Task tree visualization with recursive descendants endpoint

## Context

The Dev Runs detail page displayed child tasks as a flat list using `GET /api/v1/tasks/:id/children` (direct children only). The actual execution hierarchy is tree-shaped (milestone -> issue -> callback -> downstream tasks, up to depth 3), but operators had to mentally reconstruct it by cross-referencing the Child Tasks, Agent Activity, and Claude Pilot Metadata sections.

## Guidance

### Backend: Recursive CTE for bounded trees

Use `WITH RECURSIVE` CTE when the tree depth is structurally bounded (here by `CHECK (depth BETWEEN 0 AND 3)`). Collect IDs in the CTE, then SELECT full columns via `WHERE id IN (SELECT id FROM cte)` to avoid column-prefix issues with multi-line `TASK_COLUMNS` constants:

```sql
WITH RECURSIVE descendant_ids(id) AS (
    SELECT id FROM tasks WHERE parent_task_id = ?1
    UNION ALL
    SELECT t.id FROM tasks t
    JOIN descendant_ids d ON t.parent_task_id = d.id
    WHERE t.depth <= 3
)
SELECT {TASK_COLUMNS} FROM tasks
WHERE id IN (SELECT id FROM descendant_ids)
ORDER BY created_at ASC
```

Key decisions:
- **No row LIMIT needed** when depth is CHECK-constrained (max ~100 nodes for a 30-issue milestone)
- **No agent_id filter** — team task trees have cross-agent children (documented convention from `get_child_tasks`)
- **Root excluded from results** — the root task is already rendered as the page header; descendants fill the tree below it

### Frontend: Flat array + client-side tree building

Return descendants as a flat `TaskItem[]` array (reuses existing `TaskResponse` DTO). Build the tree client-side with a `Map<parentId, children[]>` grouping:

```typescript
function buildChildrenMap(tasks: TaskItem[]): Map<string, TaskItem[]> {
  const map = new Map<string, TaskItem[]>()
  for (const task of tasks) {
    if (task.parent_task_id) {
      const siblings = map.get(task.parent_task_id)
      if (siblings) siblings.push(task)
      else map.set(task.parent_task_id, [task])
    }
  }
  return map
}
```

### Polling: Parent-status-aware refetch

A critical edge case: when the first descendants fetch returns an empty array (children haven't been created yet), polling must continue if the parent task is still active. Without this, the tree section stays permanently empty until page reload.

```typescript
export function useTaskDescendants(rootTaskId: string | undefined, parentStatus?: string) {
  return useQuery<TaskItem[]>({
    queryKey: ['task-descendants', rootTaskId],
    queryFn: () => apiFetch(`/tasks/${rootTaskId}/descendants`),
    enabled: !!rootTaskId,
    refetchInterval: (query) => {
      const data = query.state.data
      const parentIsActive = !!parentStatus && !TERMINAL_STATUSES.has(parentStatus)
      if (!data || data.length === 0) return parentIsActive ? 15_000 : false
      const hasActive = data.some((t) => !TERMINAL_STATUSES.has(t.status))
      return hasActive || parentIsActive ? 15_000 : false
    },
  })
}
```

### Expand state: Structural, not absolute

Use tree-structural relationships (nodes whose `parent_task_id === rootTaskId`) to determine default expand state, not the absolute `depth` DB column. This handles cases where the root task is not at depth 0.

## Why This Matters

- Operators can trace failures through the full dispatch chain without mental reconstruction
- The recursive CTE pattern is safe for SQLite when depth is structurally bounded
- The flat-array-with-client-side-grouping pattern avoids nested JSON response complexity and reuses existing DTOs
- The polling fix prevents a class of "empty section" bugs that occur when children are created after the initial page load

## When to Apply

- Adding any hierarchical visualization to dashboard pages (Team Runs tree, Tasks page tree)
- Needing recursive parent-child traversal in the tasks table
- Building collapsible tree UIs that consume polling data

## Examples

The `get_sessions_for_task_tree()` function was also updated to use descendants instead of direct children, ensuring the Agent Activity section shows sessions linked to tasks at all depth levels.

## Related

- `docs/solutions/dashboard-issues/add-restful-detail-pages-pattern.md` — 4-layer pattern (DB → async → handler → route)
- `docs/solutions/dashboard-issues/task-session-bidirectional-linking.md` — sessions.task_id and get_sessions_for_task_tree
- `docs/solutions/best-practices/design-system-listrow-extraction-2026-04-27.md` — ListRow expandable variant patterns
- mika#661 — Dashboard task tree visualization
- mika#652 — Team Runs detail (future consumer of tree pattern)
