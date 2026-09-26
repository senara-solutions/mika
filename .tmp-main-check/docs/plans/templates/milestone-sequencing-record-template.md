---
title: "Milestone <N> sequencing record"
type: milestone-sequencing
milestone: senara-solutions/<repo>#<N>
date: YYYY-MM-DD
status: active
---

# Milestone <N> — <title>

## Sub-issues

<!-- List every sub-issue in the milestone with its priority, plan file, and branch slug.
     Priority uses p0-p3 (p0 = critical path, p3 = nice-to-have).
     Plan and branch may be "TBD" if not yet created. -->

- #<n>: <title> (priority: <p0/p1/p2/p3>, plan: docs/plans/<file>, branch: <slug>)
- #<n>: <title> (priority: <p0/p1/p2/p3>, plan: docs/plans/<file>, branch: <slug>)

## Dependencies

<!-- Express dependency edges as: upstream issues -> downstream issue with a one-line reason.
     Use "+" to denote multiple upstream dependencies converging on a single downstream. -->

- #<a> + #<b> → #<c>: <one-line reason>
- #<d> → #<e>: <one-line reason>

## Recommended GitHub `blockedBy` edits

<!-- These edits bridge the sequencing record to the engine tool.
     The operator (or future automation) applies them to GitHub;
     once applied, `resolve_issue_order` consumes them via GraphQL.
     Format: downstream blockedBy upstream, with a reason and filing method. -->

- #<c> blockedBy #<a>: <reason -- file via gh issue edit or GraphQL>
- #<c> blockedBy #<b>: <reason>
- #<e> blockedBy #<d>: <reason>

## Order

<!-- Recommended execution order. Each entry is either a single issue or a parallel set
     (comma-separated issues that can proceed concurrently).
     The order reflects the dependency graph above. -->

1. #<a>, #<d> <!-- parallel: no mutual dependency -->
2. #<b>
3. #<c>, #<e> <!-- parallel: independent once their upstreams complete -->

## Cross-cutting concerns

<!-- Identify concerns that span multiple sub-issues: shared modules, migration ordering,
     API contract changes, feature flags, etc. Note which sub-issues are affected and
     any mitigation strategy. -->

- <concern>: <which sub-issues touch it, mitigation>
- <concern>: <which sub-issues touch it, mitigation>

## Open milestone-level questions

<!-- Questions that affect sequencing or scope but are not yet resolved.
     Include a resolution path (who decides, what info is needed) or escalation target. -->

- <question>: <resolution path or escalation>
- <question>: <resolution path or escalation>
