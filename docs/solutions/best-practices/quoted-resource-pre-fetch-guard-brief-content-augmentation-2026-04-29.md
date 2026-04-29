---
title: "Quoted-resource pre-fetch guard — brief-content-driven required-tools augmentation"
date: 2026-04-29
category: best-practices
module: skills-pipeline, agent-loop
problem_type: best_practice
component: agent-behavior
severity: medium
applies_when:
  - Adding a new skill that reviews operator-supplied briefs containing quoted GitHub resources
  - Designing `[constraints]` for skills that process issue bodies, PR diffs, or file content
  - Debugging why an agent treated a brief-quoted resource as sufficient without live-fetching
  - Extending the detection patterns for new resource types (commit SHAs, PR comments)
related_components:
  - required-tools-gate
  - skills-pipeline
  - mika-arch
  - quoted-resources
tags:
  - pre-fetch-guard
  - required-tools
  - brief-as-claims-not-facts
  - structural-guard
  - mika-arch
  - skills-pipeline
---

# Quoted-resource pre-fetch guard — brief-content-driven required-tools augmentation

## Context

When an operator-supplied brief quotes an issue body, PR diff, or file content inside a fenced block, the agent may treat the quote as constituting the content rather than as a claim *about* the content. mika#788 demonstrated this: the architect issued a verdict against a brief-quoted issue body without calling `gh_read`, and the existing required-tools-gate caught it one turn later — after the verdict was already shaped, costing a retry turn at architect rates.

The existing `[constraints] required_tools` mechanism is static per skill manifest. It can require that `gh_read` is called, but it doesn't know *whether* the brief actually contains quoted resources that need fetching. A brief with no quoted resources still triggers the same `required_tools` enforcement; a brief with quoted resources doesn't get pre-emptive injection of specific resource fetches.

## Guidance

### Opt-in via skill manifest

Skills that process briefs containing quoted fetchable resources should declare `required_fetches_for_quoted_resources = true` in their `[constraints]` section:

```toml
[constraints]
required_tools = ["gh_read"]
required_fetches_for_quoted_resources = true
```

This is opt-in per skill. Skills that legitimately work from briefs alone (e.g., brainstorming skills where the brief is the input, not a pointer) should NOT opt in.

### How the guard works

1. At turn-start, `collect_required_tools()` checks whether any keyword-matched skill has `required_fetches_for_quoted_resources == true`.
2. If so, `detect_quoted_resources(user_message)` scans the initial user message for triple-backtick-fenced blocks with recognizable resource headers.
3. Detected resources are mapped to `gh_read` and merged into the required-tools set.
4. The existing EndTurn required-tools gate enforces the augmented set — but earlier, before the verdict-shape detector runs.

### Detection patterns (five concrete shapes)

| Pattern | Header/content marker | Resource kind |
|---------|----------------------|---------------|
| Issue body | `issue/<n>` or `issue #<n>` | `Issue { number }` |
| PR view/diff | `PR/<n>` or `pr/<n>` | `PullRequest { number }` |
| `gh issue view <n>` output | Literal `gh issue view <n>` | `Issue { number }` |
| `gh pr view/diff <n>` output | Literal `gh pr view/diff <n>` | `PullRequest/PullRequestDiff { number }` |
| File content | `<owner/repo>/<path>` header | `File { path }` |

Detection is conservative: only fenced content triggers augmentation, not prose `#NNN` references.

### Lifetime invariant

The augmentation runs ONCE per agent-loop entry against the initial user message. Corrective re-prompts from intent guards do NOT re-trigger detection. This matches the existing static `required_tools` lifetime — compute once at loop entry, hold for loop lifetime. See plan F1 resolution for the full rationale.

## Why This Matters

Without the pre-fetch guard, the required-tools gate fires *after* the verdict is shaped. The agent generates a verdict, the gate rejects it, and the agent retries — burning a turn on re-justifying the verdict against the now-fetched content. Under load, this retry can cascade (mika#788 demonstrated a retry that triggered core-memory writes that hit per-block caps that dropped the verdict line entirely).

The pre-fetch guard moves the enforcement *before* the LLM generates text, eliminating the verdict-then-retry waste.

## When to Apply

- **New mika-arch skills:** Any skill that reviews plans or briefs citing GitHub resources should opt in.
- **Non-mika-arch skills:** Only opt in if the skill receives briefs with quoted resources AND the skill should always fetch those resources live. Skills that work from the brief content itself (not as a pointer) should NOT opt in.

### Adding new detection patterns

New patterns in `detect_quoted_resources` MUST be preceded by a compound-doc Rule 1 update (`docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`) with the observed brief shape and trace ID citation. This prevents silent pattern accumulation without institutional record.

## Examples

Before (mika#788 failure shape):
```
Turn 1: Agent receives brief with quoted issue/788 body
         → Emits verdict without calling gh_read
         → Required-tools gate fires (static ["gh_read"])
Turn 2: Agent retries, calls gh_read, re-generates verdict
         → Extra turn at architect rates
```

After (with pre-fetch guard):
```
Turn 1: Agent receives brief with quoted issue/788 body
         → Pre-fetch guard detects fenced issue/788 block
         → gh_read augmented into required-tools set
         → Agent must call gh_read before EndTurn
         → No verdict-then-retry waste
```

## Citations

- mika#863 — implementation ticket
- mika#788 — recurrence that motivated this guard (sufficiency hallucination)
- `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` — Rule 1 (brief-as-claims-not-facts)
- `crates/mika-agent/src/skills/quoted_resources.rs` — detection module
- `crates/mika-agent/src/agent.rs` — `collect_required_tools()` augmentation site
- `tests/eval/grounding_regressions/quoted_resource_pre_fetch.rs` — three eval scenarios
