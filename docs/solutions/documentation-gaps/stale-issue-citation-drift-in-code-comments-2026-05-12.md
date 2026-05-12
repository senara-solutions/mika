---
title: Stale issue citation drift in code comments and CLAUDE.md
date: 2026-05-12
category: documentation-gaps
module: mika-agent
problem_type: documentation_gap
component: documentation
severity: low
applies_when:
  - Issue citations in code comments reference a different issue than the design decision they describe
  - A PR fixes one citation but leaves sibling citations in the same file or related files untouched
  - Code comments cite an issue number for a feature or convention that was actually established by a different issue
tags:
  - issue-citation
  - documentation-drift
  - code-comments
  - claude-md
  - match-reason
  - required-tools
---

# Stale issue citation drift in code comments and CLAUDE.md

## Context

mika#265 was about `mika ask` vs TUI claude-pilot execution ("Mika announces implementation but doesn't execute"). However, 10 code comments and 2 CLAUDE.md paragraphs across `crates/mika-agent/` cited `#265` as the precedent for the `MatchReason::Keyword`-only filtering of `collect_required_tools()` and related constraint scoping. The actual precedent for that design decision is mika#463 (`resolve_skill_llm_override` should filter by `MatchReason::Keyword` only).

Discovered during mika#1011 investigation (Phase 0.2). The mika#1011 PR corrected one specific `agent.rs` citation; this audit (mika#1012) covered the remaining 10 sites.

## Guidance

When fixing a stale issue citation, audit all related citations in the same codebase area:

1. **Grep broadly**: `grep -rn '#<number>' crates/<crate>/src crates/<crate>/CLAUDE.md` to find all sites referencing the same issue number.
2. **Verify each site**: Read the actual GitHub issue (`gh issue view <number>`) and confirm the citation matches the issue's content.
3. **Distinguish legitimate from stale**: Some references may be correct (e.g., `db.rs:4368` legitimately referenced #265 for the `mika ask` callback detection feature that #265 actually introduced).
4. **Replace with the correct citation**: Don't just remove stale references -- replace with the issue that actually established the cited convention.

Historical documents (`docs/plans/`, `docs/solutions/`) should NOT be retroactively updated -- they record what was believed at the time and serve as historical records.

## Why This Matters

Stale issue citations create misleading provenance. A developer tracing the design history of `MatchReason::Keyword`-only filtering would read mika#265, find it's about `mika ask` TUI execution, and be confused about why the code cites it for skill constraint scoping. Correct citations make design archaeology reliable.

## When to Apply

- After fixing a single citation that was discovered during other work, check if sibling citations exist
- When a PR touches code with issue citations, verify the cited issues match the described behavior
- During documentation audits or CLAUDE.md updates

## Examples

**Before (stale):**
```rust
/// from skills that matched via keyword (not just `always_on`). See #265.
```

**After (correct):**
```rust
/// from skills that matched via keyword (not just `always_on`). See #463.
```

**Preserved (legitimate):**
```rust
/// agent loop but won't be consumed until TUI or server starts. See #265.
// ^ This is correct -- #265 IS about mika ask/TUI execution
```

## Related

- mika#1012 -- this audit ticket
- mika#1011 -- origin of the audit need (corrected one site)
- mika#463 -- the actual precedent for Keyword-only constraint scoping
- mika#265 -- the legitimately-referenced issue (mika ask TUI execution)
- `docs/solutions/architecture-patterns/conditional-required-tools-enforcement-via-match-reason.md` -- related pattern doc (retains historical #265 reference)
