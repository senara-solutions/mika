---
title: "Citation-fabrication prompt anchoring — verbatim-quote and session-id chain discipline for architect review skills"
date: 2026-05-02
category: best-practices
module: skills/bundled
problem_type: best_practice
component: tooling
severity: high
applies_when:
  - mika-arch review skills produce findings that cite prior-session content or verbatim issue body quotes
  - Any agent skill produces confident-tone citations to content not actually in its context window
  - Cross-session parametric memory bleed causes the model to conflate prior sessions with the current one
tags: [mika-arch, citation-fabrication, prompt-anchoring, grounding, verbatim-quote, session-id, hallucination]
---

# Citation-fabrication prompt anchoring — verbatim-quote and session-id chain discipline for architect review skills

## Context

mika-arch produces review findings with confident-but-wrong provenance citations. Two verified instances on 2026-05-02:

1. **mika#931 pass-1** — cited a non-existent prior architect session ("Prior mika#931 first-pass F2 — this is a persisted pattern"). Reality: no prior architect first-pass existed for that issue.
2. **mika#928 pass-1** — fabricated "verbatim" concept lists claiming they were "from the issue body." Reality: the actual issue body contained different prose concepts; the specific entity-key tokens were generated, not extracted.

Both instances had architecturally-sound conclusions — the fabrication was the false-detail attached to support them. The model self-corrected when challenged in pass-2, indicating the failure mode is at quote-emission time, not reasoning time.

Distinct from mika#947 (persistence-meta hallucination — a refusal-style failure where no review is delivered).

## Guidance

Add two anchoring instructions to the Operating Discipline section of architect review skill prompts:

**1. Verbatim-quote anchoring** — require `gh_read` at quote time:

```markdown
**Verbatim-quote anchoring.** When citing verbatim content from issue bodies, PR bodies,
or prior commits, you MUST invoke `gh_read` (or equivalent file/issue read tool) to fetch
the source at quote time — not paraphrase from the brief's summary or parametric memory.
If the verbatim content cannot be retrieved via a fresh tool call, do NOT claim "verbatim"
— describe the content in your own words and flag the inability to anchor.
```

**2. Session-id chain anchoring** — restrict prior-session references to the current conversation chain:

```markdown
**Session-id chain anchoring.** When referencing prior-session findings, only cite session
IDs that appear in the current conversation's brief or `--session-id` parameter. If you
have a sense of "I've seen something like this before" but cannot point to a session ID in
the current chain, frame as a new finding — not a "persisted pattern" or continuation of
a prior review.
```

Both instructions are placed after the existing "Citation or silence" rule in the Operating Discipline section, reinforcing the same principle: cite what you can verify, stay silent on what you cannot.

## Why This Matters

Citation fabrication is a half-ground-truth failure — review delivered but with fabricated provenance. An operator who treats the citations as ground truth will misroute investigation (e.g., searching for a prior architect session that doesn't exist, or trying to reconcile "verbatim" content that was never in the issue body).

The failure mode carries extra weight because it appears in BLOCKING findings (ITERATE/ESCALATE dispositions), where false provenance carries decision weight. The findings themselves may be sound architecture, but the false provenance misleads triage.

## When to Apply

- When adding review or analysis skills that reference external content (issue bodies, PR bodies, prior sessions)
- When a skill's output includes "verbatim" quotes or claims about prior-session continuity
- When cross-session parametric memory bleed is a risk (agent has access to multiple sessions on the same issue)
- As a general pattern: any skill that cites external content should anchor those citations via fresh tool calls

## Examples

**Before (fabrication):**
```
F2: Plan must confirm `pipeline-exempt:no-plan` entry's current exact YAML shape
(identical requirement to my prior first-pass on this issue)
Prior mika#931 first-pass F2 (same finding on prior draft — this is a persisted pattern)
```

**After (anchored):**
```
F2: Plan must confirm `pipeline-exempt:no-plan` entry's current exact YAML shape
[Verified via gh_read: issue #931 body does not contain a prior architect review reference.
This is a new finding based on the current plan content.]
```

**Before (fabricated verbatim quote):**
```
The issue body already provides concrete B1 and B2 lists:
B1 — concept:cross-repo:*: worktree, plan-on-branch, callout, branch-slug...
```

**After (anchored or flagged):**
```
The issue body describes concepts for B1 and B2 categories in prose form.
[Fetched via gh_read: actual issue body lists "companion-PR pattern, branch-name-immutable
invariant, plan-doc-on-branch contract..." — not the entity-key tokens listed above.
Using the actual issue body content as the reference.]
```

## Related

- mika#952 — citation-fabrication failure mode ticket
- mika#953 — companion ticket for telemetry-driven fabrication detection (deferred)
- mika#947 — persistence-meta hallucination (sibling failure mode, distinct family)
- `docs/solutions/prompt-engineering/grounding-rule-downstream-state-hallucination.md` — four-part anti-hallucination formula (related grounding discipline)
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — prompt-level vs structural enforcement doctrine
- `docs/solutions/best-practices/operator-grooming-marathon-2026-05-02.md` — incident report documenting both verified fabrication instances
