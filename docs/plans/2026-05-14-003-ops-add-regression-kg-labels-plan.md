---
type: ops
ticket: mika issue#1104
title: "Add regression and kg labels to labels.yml"
date: 2026-05-14
branch: ops/1104/labels-add-regression-and-kg-labels-to
---

# Plan: Add `regression` and `kg` labels to labels.yml

## Context

Two label gaps identified during 2026-05-13 orchestrator audit:

1. **No `regression` label** — multiple regression-class tickets (mika#1089, #1090, #1097, #1102) are indistinguishable from `bug` in filtered views. Regressions are functionality that previously worked and broke — distinct from new defects.

2. **No `kg` component label** — KG subsystem tickets (mika#1076, #1077, #1091, paused #960, #918) have no unifying filter. Needed for `gh issue list --label kg` when operating on the self-awareness research track.

## Change

Edit `mika/.github/labels.yml`. Add two entries in the appropriate sections:

### Under `# ── Type ──` section (after `wontfix` or `help wanted`):

```yaml
- name: regression
  color: "8b0000"
  description: Functionality that previously worked has broken — distinct from new bugs
```

### Under `# ── Component ──` section (after `dashboard`):

```yaml
- name: kg
  color: "5C6BC0"
  description: Knowledge Graph subsystem (extraction, resolution, corpora, schema)
```

## Color rationale

- `regression` (#8b0000, dark red) — differentiates from `bug` (#d73a4a, red) and `p0-critical` (#b60205, deep red). Semantically "red family" but visually distinct.
- `kg` (#5C6BC0, indigo) — differentiates from `agent-core` (#1d76db, blue), `tui` (#5319e7, purple), `team-engine` (#0e8a16, green). Blue-purple range for component labels.

## Verification

Label-sync action (`EndBug/label-sync` in `.github/workflows/labels.yml`) applies on merge. Post-merge verify:

```bash
gh label list --repo senara-solutions/mika | grep -E '(regression|kg)'
```

## Out of scope

- Retroactive label application to existing tickets (operator-side, no code change)
- Additional labels (`architect`, `workflow`, `release` rename) — deferred per ticket
