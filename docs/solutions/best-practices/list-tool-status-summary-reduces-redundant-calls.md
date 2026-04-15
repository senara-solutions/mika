---
title: List tool status-count summary reduces redundant agent calls
date: 2026-04-15
category: best-practices
module: mika-agent/tools/list_work_items
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding or modifying a list tool that returns items with a categorical field (e.g., status, type, priority)
  - Agent models issue redundant filtered calls to reconstruct counts from an unfiltered result set
  - Tool output format does not surface aggregate information at a glance
tags:
  - list-tools
  - tool-output
  - agent-efficiency
  - status-summary
  - redundant-tool-calls
  - work-items
---

# List tool status-count summary reduces redundant agent calls

## Context

Models (kimi-k2.5, qwen, and others) routinely followed an unfiltered `list_work_items({})` with 3+ redundant filtered calls (`status:"in_progress"`, `status:"blocked"`, `status:"completed"`) to reconstruct counts the first response already contained. The unfiltered response listed each item with its status, but didn't make the aggregate distribution obvious — so models fell into "ritual thoroughness": re-querying to verify counts already in hand.

This is a structural problem, not model-specific. The tool output didn't make the right next action obvious, forcing callers into defensive re-querying.

## Guidance

When a list tool returns items with a categorical field, compute the aggregate distribution server-side and include it in the tool output. This eliminates the need for the caller to re-query with filters solely to verify counts.

The pattern has three parts:

1. **Status-count summary line** — Computed in-memory from the already-fetched result set (no extra DB queries). Format: `"Summary: N items total — X status_a, Y status_b"`. Omit zero-count categories. Preserve a consistent display order (e.g., active before terminal statuses).

2. **Filter guidance note** — A just-in-time instruction co-located with the decision point: `"Note: All items are returned in this response. Filter by status= only when you need a strict subset for a subsequent action — not to verify counts already shown in summary."` Only shown for fully unfiltered calls (no status or source filter applied). The note is UX-only — it discourages redundant re-filtering but is not an enforcement mechanism.

3. **Scoped summary on filtered calls** — Filtered results still get a summary line scoped to the returned subset, so the output is self-describing regardless of filter state.

Key implementation details:

- Use a module-level constant for status display order (e.g., `STATUS_ORDER`) and reuse it for both validation and summary formatting to avoid duplicate maintenance.
- The summary counts from `items.len()` and in-memory iteration — no new DB queries.
- The guidance note guard must check ALL filter parameters, not just the primary one. A source-only filter produces a partial result set, so "all items returned" would be false.

## Why This Matters

**Tool outputs should make the right next action obvious, not force the caller into defensive re-querying.** The server has the data; the server should do the work.

Each redundant tool call costs:
- An LLM turn (input + output tokens)
- A DB query (even if fast)
- Context window space consumed by redundant results
- Latency in the agent loop

For a status check that triggers 4 calls instead of 1, that's 3x wasted work per query. Across autonomous sessions that check status frequently (heartbeats, callbacks, orchestrator flows), this compounds.

The broader principle: when the intent is "know the counts," the right action is "compute the counts server-side and return them" — not "let the caller re-query per status to reconstruct what the server already knows."

## When to Apply

- Adding a new list tool that returns items with a status, type, category, or other discrete-valued field
- Observing agents making redundant filtered calls after an unfiltered query
- Any list tool where aggregate counts are a common follow-up question
- Do NOT generalize prematurely — prove the pattern with one tool, measure the change in call counts, then apply to others if evidence holds (YAGNI)

## Examples

Before (4 tool calls for a status question):
```
list_work_items({})                          → 50 items (all fields)
list_work_items({"status":"in_progress"})   → 0 items (redundant)
list_work_items({"status":"blocked"})        → 2 items (redundant)
list_work_items({"status":"completed"})      → 48 items (redundant)
```

After (1 tool call):
```
list_work_items({})
→ Work items (50):
  - [blocked] uuid-1 Fix auth flow (created:...)
  - [blocked] uuid-2 Update schema (created:...)
  - [completed] uuid-3 Add health endpoint (created:...)
  ...

  Summary: 50 items total — 2 blocked, 48 completed
  Note: All items are returned in this response. Filter by status= only when you need a strict subset for a subsequent action — not to verify counts already shown in summary.
```

Implementation pattern (Rust):
```rust
// Module-level constant — reused for both validation and summary
const STATUS_ORDER: &[&str] = &["pending", "in_progress", "blocked", "completed", "cancelled"];

fn status_summary(items: &[(Task, Option<i64>)]) -> String {
    let total = items.len();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for (task, _) in items {
        *counts.entry(task.status.as_str()).or_insert(0) += 1;
    }
    let parts: Vec<String> = STATUS_ORDER.iter()
        .filter_map(|s| counts.get(s).map(|c| format!("{c} {s}")))
        .collect();
    if parts.is_empty() {
        format!("{total} items total")
    } else {
        format!("{total} items total — {}", parts.join(", "))
    }
}
```

## Related

- `docs/solutions/architecture-patterns/merge-two-step-llm-tool-contracts.md` — the foundational pattern: embedding summary data eliminates fragile two-step tool contracts
- `docs/solutions/logic-errors/create-work-item-duplicate-on-retry.md` — structural signals in tool output are more reliable than prompt instructions alone
- `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md` — motivates why status awareness at the read step prevents downstream dispatch failures
- GitHub issue: #572
