## Dev Handsoff Skill (v0.1 — artifact-only)

Write a structured handsoff artifact to the agent's `handsoff/` directory at end of an autonomous run. No git operations. No cross-repo writes. The v0.2 pipeline pickup transforms the artifact into a published log at `mika-platform/docs/logs/`.

**Contract reference:** `mika-platform/docs/logs/HANDSOFF-CONTRACT.md` defines the canonical body-section names and merge rules. This skill's output conforms to that contract.

### Phase 1 — Resolve target file path

1. Compute `TODAY` as the current UTC date in `YYYY-MM-DD` format.
2. Identify the agent's handsoff directory: `handsoff/` (relative to agent home, written via `write_agent_file`).
3. Resolve slug via the fallback chain (first match wins):

| Priority | Condition | Slug value | Resulting filename |
|----------|-----------|------------|--------------------|
| 1 | Run has a primary work-item (e.g. `mika#864`) | Sanitised work-item ref (`mika-864`) | `<TODAY>-mika-864.md` |
| 2 | No primary work-item | First 8 chars of session id | `<TODAY>-<sid8>.md` |
| 3 | First handsoff invocation of the day AND no primary work-item | Empty (no slug, no leading hyphen) | `<TODAY>.md` |

4. Compute target path: `handsoff/<TODAY>[-<slug>].md` (omit the hyphen and slug for rule 3).
5. Use `read_agent_file` to check whether the target file already exists:
   - **Does not exist** → Phase 2 (new-file create).
   - **Exists** → Phase 3 (continuation append).

### Phase 2 — New-file create

Write the artifact using `write_agent_file`. Structure:

```markdown
---
date: "<TODAY>"
session_id: "<full-session-id>"
actor: "mika-dev"
branch: "<branch-name>"
repo: "<repo-name>"
summary: "<one-line summary>"
work_item: "<ticket-ref-or-null>"
slug: "<slug-or-empty>"
runs:
  - session_id: "<full-session-id>"
    started_at: "<iso8601-utc>"
    ended_at: "<iso8601-utc>"
    summary: "<one-line summary>"
---

# <TODAY> — mika-dev[ — <slug>]

## TL;DR

- <1–3 bullets summarizing state transitions>

## Story so far

<Third-person prose paragraph for this run.>

## Tickets & PRs touched

| Ref | State | Next action |
|-----|-------|-------------|
| <ref> | <state> | <action> |

## Blocked / carry-forward

| Ref | Reason | Action for next session |
|-----|--------|------------------------|
| <ref> | <reason> | <action> |

## Decisions in flight

- <item>

## Filed today

- <issue-ref> — <title>

## What to do next session

1. <item>
2. <item>
```

**Drop empty sections** rather than leaving placeholder text. If a section has no content, omit it entirely from the artifact.

### Phase 3 — Continuation append

When the target file already exists, read it end-to-end and apply per-section append discipline:

| Section | Discipline |
|---------|-----------|
| `## TL;DR` | Rewrite wholesale (latest run's summary). |
| `## Story so far` | Append new paragraph below existing prose. Never rewrite prior content. |
| `## Tickets & PRs touched` | Update existing rows in place by `Ref`; append new rows. Never duplicate rows for the same ref. |
| `## Blocked / carry-forward` | Append new entries; update existing rows by Ref. Drop section if now empty. |
| `## Decisions in flight` | Append new entries. Resolved entries get `(resolved <iso8601-time>)` suffix. |
| `## Filed today` | Append only. |
| `## What to do next session` | Rewrite wholesale — the previous run's checklist is stale. |

**Frontmatter updates on continuation:**
- Append a new entry to `runs[]`.
- Update top-level `summary` to the latest run's one-liner.
- Do NOT modify other top-level fields (`date`, `session_id`, `actor`, `branch`, `repo`, `work_item`, `slug`).

If a section was dropped on first creation and has content to add now, create the section at the appropriate position (section order follows the template above).

Write the updated artifact via `write_agent_file` with `confirm: true` (overwrite).

### Phase 4 — Synthesis discipline

Synthesise all content from session context:
- What tools were called, what tasks were updated, what PRs were touched.
- Derive the summary, story, and next-session checklist from the run's actual events.

**Zero `AskUserQuestion` calls.** Never ask the operator for input. The slug fallback chain guarantees a deterministic path. If synthesis is incomplete or wrong, the operator corrects in a follow-up turn.

### Phase 5 — Emit signal block

After writing the artifact, print exactly this terminal block and nothing else:

```
=== Dev handsoff artifact ===
Path:    handsoff/<filename>
Summary: <the one-line summary just written>
Pickup:  pending v0.2 pipeline step.
```

No restated next-session list. No restated blockers. No restated decisions. The single source of truth is the file.

### Discipline

- **Writer-scope is the agent home, period.** The skill never writes outside `handsoff/` via `write_agent_file`. Cross-repo writes (into `mika-platform/`) are the pickup step's job.
- **Artifact contract is load-bearing.** v0.2 pickup is written against the schema above. Section names, frontmatter fields, and append discipline are part of the contract.
- **Keyword-triggered, not lifecycle-triggered.** Until the engine exposes a `[lifecycle].on_session_end` field, this skill fires only on the configured keyword phrases.
- **Zero AskUserQuestion calls — backed by slug fallback.** Synthesise from run context. The slug fallback chain guarantees a buildable path without operator interaction.
- **Third-person voice, always.** "mika-dev did X." Never first-person. Never "we." Voice consistency is load-bearing because the published log is dual-audience (engineering trace + Director narrative source).
- **Session state, not compounding learning.** This skill captures what state this run leaves behind. If the run produced a generalising learning, file a separate compound doc — do not smuggle it into the handsoff.
- **No git, no push, no commit.** v0.1 is artifact-only. The pickup step owns the publish ceremony. The only git invocation permitted is `git rev-parse --show-toplevel` or `git branch --show-current` for metadata resolution (read-only).
- **No umbrella prose.** The terminal block is a signal pointing at the file, not a restatement of its content.
