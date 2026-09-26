## Bearing — Operational Ground-Truth Render

You are rendering a bearing: a priority-ordered snapshot of current operational state with verdicts.

### Ground Rule (structural, non-negotiable)

Before composing ANY bearing text, you MUST call all three tools:

1. **`run_gh`** — fetch current issue counts, open PRs, milestone state
2. **`search_memory`** — retrieve recent decisions, commitments, blockers
3. **`query_knowledge_graph`** — check entity counts, resolution coverage, library state

Do NOT use `core_memory` snapshots as ground truth for world state. `core_memory` is for stable identity facts (who, what, why), not refreshable state (issue counts, PR status, KG coverage). The tools above are the only source of current operational reality.

### Output Format

Begin every bearing with a single Ground watermark line:

```
Ground: <ISO 8601 timestamp> · gh ✓ <issue count> open · search_memory ✓ · kg ✓ <entity count>
```

This is an operator-glance freshness signal, not a report. The full tool-call trail is in the trace.

### Bearing Shape

After the Ground line, produce the bearing in your established shape:
- Priority calls with evidence citations from the tool results
- Diverge / converge structure where appropriate
- Named traps and trade-offs grounded in fetched state
- Verdicts with confidence levels tied to evidence freshness

### What NOT to do

- Do not cite `core_memory` issue counts, PR states, or KG statistics as current fact
- Do not skip any of the three required tool calls
- Do not produce a bearing if any required tool call fails — report the failure instead
