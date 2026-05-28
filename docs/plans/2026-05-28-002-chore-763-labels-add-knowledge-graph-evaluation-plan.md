# Plan: chore(labels): add knowledge-graph, evaluation, observability to .github/labels.yml

**Issue:** mika issue#763
**Type:** chore (label hygiene)
**Scope:** `.github/labels.yml` only — no code changes, no test changes

## Context

Three labels referenced by recent issues (#757, #761, #762) don't exist in `.github/labels.yml`. The label-sync workflow (EndBug/label-sync) needs these entries to create them as first-class labels in GitHub.

**Pre-existing state:** A `kg` label already exists (added after this ticket was filed) covering the Knowledge Graph subsystem broadly. The new `knowledge-graph` label proposed in the ticket overlaps with the existing `kg` label. This plan reconciles the overlap.

## Analysis

### Overlap: `kg` vs `knowledge-graph`

The existing `kg` label (line 89-91) already covers "Knowledge Graph subsystem (extraction, resolution, corpora, schema)" — which is essentially what the ticket's `knowledge-graph` label would cover. Adding a second label with near-identical scope creates confusion. Two options:

1. **Skip `knowledge-graph`, keep `kg`.** The existing label already serves the purpose. Retroactive labeling uses `kg`.
2. **Rename `kg` → `knowledge-graph`** for consistency with the ticket's intent. This requires updating any existing issue references.

**Decision:** Keep `kg` as-is. It's already in use across issues, referenced in CLAUDE.md conventions, and the shorter name is idiomatic for this codebase. The ticket's intent (unifying KG work under one label) is already satisfied.

### New labels needed

Only two new labels are needed:

1. **`evaluation`** — No existing label covers eval harness / provider comparison work. The Evaluation milestone (#16) exists but labels and milestones serve different filtering purposes.
2. **`observability`** — No existing label covers logging/tracing/metrics. The `dashboard` label is scoped to the React SPA, not general operator observability.

## Implementation

### Step 1: Add `evaluation` and `observability` labels

Append two entries to the **Component** section of `.github/labels.yml`, after the existing `kg` entry (line 91):

```yaml
- name: evaluation
  color: "84cc16"
  description: Evaluation harness, eval scenarios, provider quality comparisons

- name: observability
  color: "f59e0b"
  description: Logging, tracing, metrics, dashboards — anything operator-visible
```

Colors match the ticket's proposal (lime green for evaluation, amber for observability) — both distinct from existing component labels.

### Step 2: Retroactive label application (post-merge)

After the label-sync workflow creates the labels on merge:

```bash
# evaluation
gh issue edit 762 --repo senara-solutions/mika --add-label evaluation

# observability
gh issue edit 761 --repo senara-solutions/mika --add-label observability

# kg (already exists, apply retroactively if not already applied)
gh issue edit 757 --repo senara-solutions/mika --add-label kg
gh issue edit 762 --repo senara-solutions/mika --add-label kg
```

### What about `knowledge-graph`?

Intentionally skipped. The existing `kg` label already covers this scope. Adding a second near-identical label would fragment filtering. If the operator prefers the longer name, a rename (`kg` → `knowledge-graph`) is a separate ticket to avoid scope creep here.

## Acceptance criteria mapping

| Criterion | Status |
|-----------|--------|
| Three label entries added to Component section | Partial: 2 new + 1 existing (`kg`) = 3 labels covering all requested scopes |
| Label-sync workflow creates labels on merge | Automatic via EndBug/label-sync |
| Retroactive label application | Post-merge `gh issue edit` sweep (Step 2) |

## Risk

None. Config-only change. No code, no tests, no build impact. The label-sync workflow is already proven.

## Pipeline tier

Config-tweak tier per the ticket's own guidance and `feedback_pipeline_scaling.md`. A passing `cargo check` is sufficient.
