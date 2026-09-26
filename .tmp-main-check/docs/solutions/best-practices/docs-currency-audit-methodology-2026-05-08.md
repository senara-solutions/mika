---
title: Investigation-class audit methodology for docs/solutions/ currency
date: 2026-05-08
category: best-practices
module: docs
problem_type: best_practice
component: documentation
severity: low
applies_when:
  - Evaluating whether a docs/solutions/ corpus needs a curation policy
  - Corpus has grown past 500 entries and age-based decay is a concern
  - Operator needs decision-grade data before committing to a retention policy
tags: [docs-currency, audit-methodology, kg, investigation, compound-discipline, cross-repo]
---

# Investigation-class audit methodology for docs/solutions/ currency

## Context

As the `docs/solutions/` corpus grows across multiple repos (mika, mika-platform, mika-cloud, mika-skills), the question arises: do entries decay — become stale, contradicted, or superseded — and if so, what curation policy is appropriate? This is judgment-class: the operator needs structured data before committing to a policy. mika#1029 established the investigation methodology; mika#1027 established the per-ticket one-shot script pattern.

## Guidance

Use a six-axis investigation script that reads the corpus filesystem + KG state from SQLite, producing structured markdown output per axis:

1. **Inventory (A1):** Entry count per repo, per category subdirectory, per `problem_type` frontmatter tag. Surfaces the corpus shape and taxonomy hygiene (inconsistent tags like `logic_error` vs `logic-error`).

2. **Age distribution (A2):** Three-source date derivation (filename `YYYY-MM-DD` prefix > frontmatter `date:` > `git log` introduction date). Records both `introduction_date` and `last_modified` — the distinction matters for policy: "old + recently modified" is different from "old + untouched."

3. **Supersession analysis (A3):** Grep for explicit supersession markers (`supersedes`, `replaced by`, `see instead`, etc.). Build chains (source → target). Flag orphan markers where the target doc is missing. High false-positive rate in solution docs due to coding-advice phrases ("use X instead of Y") — hand-validate.

4. **Topic-cluster overlap (A4):** Use existing KG `kg_subject_resolutions` to find doc pairs sharing ≥3 domain entities. Zero LLM cost in primary path. Hand-classify top 10 pairs as `adjacent` / `supersession-candidate` / `genuine-contradiction`. Falls back to `problem_type` overlap if KG resolutions are sparse (< 100).

5. **KG-resolution drift (A5):** `no_match` rate per age-quartile from `kg_resolutions_log`. Measures whether old docs' subject entities still resolve to current domain graph entities. Thresholds: PASS < 10pp drift, FAIL ≥ 25pp, MIXED 10-25pp.

6. **Consumption signal (A6):** Fraction of `query_knowledge_graph` tool calls surfacing old docs. Pre-flight check: if `tool_calls.output` doesn't contain `source_doc_path` strings, axis is UNAVAILABLE (schema limitation, not a bug).

**Axis 7 (synthesis)** maps signal levels (Aging, Drift, Load-bearing) to one of six candidate policies via a predetermined lookup table. The investigation recommends a single policy with a named runner-up.

### Key implementation lessons

- **Use `kg_subject_resolutions` (not `kg_resolutions_log`) for topic clustering.** `kg_subject_resolutions` contains only successfully matched entities with `domain_entity_id NOT NULL`, which is exactly what you need for doc-pair overlap. The resolution log contains all attempts including `no_match`.

- **Batch git log for speed.** Per-file `git log --follow` is prohibitively slow for 500+ files. Use `git log --name-only --format="DATE:%aI" -- "docs/solutions/"` in one pass, cache results, then look up per file. Trade-off: loses `--follow` rename tracking (renamed files get rename date, not original creation date).

- **Guard against `set -o pipefail` with grep.** In pipefail mode, `grep | wc -l` returns non-zero when grep finds no matches. Use `grep -c ... || echo 0` or `{ grep ... || true; } | ...`.

- **Avoid piped `while read` for counter variables.** Bash pipes create subshells — `grep ... | while read -r line; do COUNT=$((COUNT + 1)); done` loses the increment. Use process substitution: `while read -r line; do ...; done < <(grep ...)`.

## Why This Matters

The `/ce:compound` pipeline produces 500+ entries over months. Without periodic currency checks, the corpus could accumulate stale, contradictory, or superseded entries that mislead rather than help. The six-axis methodology provides decision-grade data — not opinions — for whether a curation policy is warranted. The 2026-05-08 audit found Outcome A (corpus too young at ~3 months to need curation), establishing a baseline for future comparison.

## When to Apply

- Re-run when the corpus reaches 12 months of age (earliest: 2027-Q1) or exceeds 1,000 entries
- Re-run after any significant architectural refactor that renames skills, tools, or agents (may cause KG-resolution drift)
- The investigation script (`scripts/investigate-docs-currency-1029.sh`) is idempotent and committed; re-run verbatim or adapt for a new ticket

## Examples

**Signal-triple → policy mapping (from the finding doc):**

| Aging | Drift | Load-bearing | Recommended policy |
|-------|-------|--------------|--------------------|
| Low | Low | any | No curation (status quo) |
| Medium | Low | Medium/High | Keyword supersession |
| Medium/High | High | any | Auto-sunset on KG decay |
| High | any | High | Quarterly retention sweep |

**Outcome declaration pattern:**
- Outcome A: Corpus is current → close ticket, no follow-up
- Outcome B: Curation needed → file implementation ticket for chosen policy
- Outcome C: Signals mixed → escalate to operator with options

## Related

- mika#1029 — this investigation
- mika#1027 / PR #1028 — sibling investigation (KG sync verification) establishing the one-shot script pattern
- `docs/audits/2026-05-08-002-investigate-compound-knowledge-currency.md` — finding doc
- `scripts/investigate-docs-currency-1029.sh` — investigation script
- `docs/solutions/best-practices/kg-post-deploy-verification-methodology-2026-05-08.md` — mika#1027's compound doc (same methodology class)
