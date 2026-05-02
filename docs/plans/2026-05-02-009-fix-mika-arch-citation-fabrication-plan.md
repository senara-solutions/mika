---
title: "fix(mika-arch): citation-fabrication failure mode — anchor verbatim quotes via gh_read at quote time"
type: fix
status: active
date: 2026-05-02
ticket: mika#952
---

# fix(mika-arch): citation-fabrication failure mode — anchor verbatim quotes via gh_read at quote time

## Overview

mika-arch fabricates concrete provenance citations (prior-session findings, "verbatim" quotes from issue bodies) within otherwise-sound review findings. Two verified instances on 2026-05-02 (mika#931 pass-1: fabricated prior-session reference; mika#928 pass-1: fabricated verbatim concept lists). Distinct from mika#947 (persistence-meta refusal pattern). Fix shape: prompt-level instruction in mika-arch's review skills (`mika-arch-groom-ticket`, `mika-arch-second-review`, `mika-arch-groom-milestone`) requiring verbatim quotes to be anchored via fresh `gh_read` invocations at quote time, not reproduced from parametric memory.

## Problem Frame

Per mika#952 issue body: confident-tone fabrications appear in BLOCKING findings where false provenance carries decision weight. Both verified instances had architecturally-sound conclusions (preserve existing entry; pin verbatim concepts) — the fabrication was the false-detail attached to support them. The model self-corrects when challenged in pass-2, indicating the failure mode is at quote-emission time, not reasoning time.

Cross-session attribution is the secondary surface: mika-arch's pass-2 on mika#931 explicitly conflated session `02cb26ed` (different mika#931 brief, different plan) with the current session. Parametric memory of "I've seen something like this" appears to fill in details the model expected but did not actually have.

## Requirements Trace

- **R1.** When mika-arch reviews a brief that references issue body content, verbatim quotes must be sourced from a fresh `gh_read` call at quote time (not paraphrased from the brief's summary or parametric memory).
- **R2.** When mika-arch references prior-session findings, the cited session ID must be in the current conversation's session-id chain (i.e., the prior session_id passed via `--session-id`), not invented from parametric memory.
- **R3.** Existing review findings remain architecturally sound — this fix changes WHERE quotes come from, not WHAT findings the architect produces.
- **R4.** Five consecutive mika-arch reviews show ZERO fabricated provenance citations (manual operator audit).

## Scope Boundaries

- **In scope:** `mika/skills/bundled/mika-arch-groom-ticket/system_prompt.md`, `mika/skills/bundled/mika-arch-second-review/system_prompt.md`, `mika/skills/bundled/mika-arch-groom-milestone/system_prompt.md` — the three architect-skill prompts.
- **Out:** mika#947 (persistence-meta) — distinct failure mode.
- **Out:** mika#939 / Opus deadline-exceeded — different surface.
- **Out:** Structured telemetry for fabrication detection (mika#952 direction option 3) — file as separate observability ticket if pursued.

### Deferred to Separate Tasks

- **Telemetry-driven fabrication catalog** (mika#952 direction 3): structured `kg_arch_fabrication_detected` event logging. Useful but adds infrastructure scope; this prompt-level fix addresses the primary failure first. File as observability follow-up after this fix verifies.
- **Cross-skill prompt-anchoring discipline** (apply same pattern to mika-qa, mika-dev review skills): out of scope here; same family but distinct skills.

## Context & Research

### Relevant Code and Patterns

- `mika/skills/bundled/mika-arch-groom-ticket/system_prompt.md` — first-pass prompt; primary surface.
- `mika/skills/bundled/mika-arch-second-review/system_prompt.md` — second-pass prompt; same fix applied for symmetry.
- `mika/skills/bundled/mika-arch-groom-milestone/system_prompt.md` — milestone-grooming prompt; same fix applied (broader review surface).
- `mika/docs/architecture/review-guide.md` — review principles; verbatim-quote-anchoring is consistent with existing "citation-or-silence" discipline.

### Institutional Learnings

- mika#947 — persistence-meta hallucination (sibling ticket; different family).
- mika#939 / PR #941 — mika-arch routing fix (orthogonal).
- `project_mika_arch_failure_modes.md` (Vincent's institutional memory) — failure-mode catalog. This ticket extends with citation-fabrication.
- Verified sessions in mika#952 issue body: `39f5f998-1199-4fdf-bfb2-2119eab9d5aa` (mika#931 pass-1+2), `fba22d43-c46e-49ad-82af-9992e7c4636a` (mika#928 pass-1+2).

## Key Technical Decisions

### KTD-1. Prompt-level verbatim-quote anchoring

**Decision:** Add explicit instruction to all three mika-arch review skill prompts requiring `gh_read` at quote time for any verbatim claim:

> *"When citing verbatim content from issue bodies, PR bodies, or prior commits, you MUST invoke `gh_read` (or equivalent file/issue read tool) to fetch the source at quote time, not paraphrase from the brief's summary or parametric memory. If the verbatim content cannot be retrieved via fresh tool call, do NOT claim 'verbatim' — describe the content in your own words and flag the inability to anchor."*

**Rationale:**
- Prompt-level fix is the cheapest correct surface — model is being asked to use a tool it already has access to.
- Aligns with existing "citation-or-silence" principle in review-guide.md.
- Fails-loud: if `gh_read` is unavailable or the source can't be fetched, the model self-flags rather than fabricating.

### KTD-2. Session-id chain anchoring

**Decision:** Add explicit instruction requiring prior-session references to come from the current conversation's session-id chain only:

> *"When referencing prior-session findings, only cite session IDs that appear in the current conversation's brief or `--session-id` parameter. If you have a sense of 'I've seen something like this before' but cannot point to a session ID in the current chain, frame as a new finding, not a 'persisted pattern.'"*

**Rationale:**
- Cross-session parametric memory bleed is the verified failure mode in instance 1 (mika#931).
- Constraint is enforceable: session IDs are explicitly visible in the brief.

## Open Questions

### Resolved During Planning

- **Prompt-level vs structural fix?** → Prompt-level (KTD-1). Structural telemetry deferred.
- **All three skills or just groom-ticket?** → All three (groom-ticket, second-review, groom-milestone). Same failure mode applies; consistent application.

### Deferred to Implementation

- **Exact prompt wording** — implementer drafts using KTD-1 + KTD-2 as constraint. The shape is locked; the exact phrasing can be tuned.

## Implementation Units

- [ ] **Unit 1: Add verbatim-quote anchoring + session-id chain anchoring to all three mika-arch skill prompts**

**Goal:** Edit three skill prompts to add the two anchoring instructions per KTD-1 and KTD-2.

**Requirements:** R1, R2, R3, R4.

**Dependencies:** None.

**Files:**
- Modify: `mika/skills/bundled/mika-arch-groom-ticket/system_prompt.md`
- Modify: `mika/skills/bundled/mika-arch-second-review/system_prompt.md`
- Modify: `mika/skills/bundled/mika-arch-groom-milestone/system_prompt.md`

**Approach:**

1. Read each skill's `system_prompt.md` to find the citation-or-silence section (existing).
2. Insert the verbatim-quote anchoring instruction (KTD-1) immediately after the citation-or-silence rule.
3. Insert the session-id chain anchoring instruction (KTD-2) immediately after that.
4. Verify no other instruction in the prompt contradicts these (e.g., a "you may paraphrase" line that needs reconciliation).

**Patterns to follow:**

- Existing "citation-or-silence" instruction in `review-guide.md` — language style and assertion shape.

**Test scenarios:**

| Category | Scenario |
|---|---|
| Happy path | Send mika-arch a brief that quotes "this is in the issue body" with a real fragment. Architect's review uses `gh_read` to anchor before quoting. |
| Edge case | Send mika-arch a brief without the issue body content embedded. Architect's review either fetches via `gh_read` or flags inability to anchor — does NOT fabricate. |
| Edge case | Send mika-arch a brief with `--session-id <existing>`. Architect cites that session's findings. Does NOT cite a different session ID from parametric memory. |
| Integration (R4) | 5 consecutive mika-arch reviews on real briefs (e.g., next 5 grooming sessions in queue) — operator audits final responses for fabricated provenance. Acceptance: 0 instances. |

**Verification:**

- `git diff` shows changes ONLY in the three skill prompt files.
- New instructions are present and non-contradictory with existing prompt content.
- Post-deploy: 5-call audit shows zero citation fabrications.

## System-Wide Impact

- **Interaction graph:** mika-arch loads these skill prompts on every review invocation. Edit propagates via bundled-skill resync on next agent session start.
- **Error propagation:** None affected. The new instructions are guidance, not new error paths.
- **State lifecycle risks:** None. Prompt-only change.
- **API surface parity:** Other agents (mika-dev, mika-qa) have similar review-style skills that may benefit from the same anchoring discipline. Out of scope here; file follow-up after observing fix verification on mika-arch.
- **Unchanged invariants:**
  - mika-arch's review-finding architecture unchanged (R3).
  - Skill `[llm:]` routing unchanged.
  - `gh_read` tool surface unchanged (it already exists and works).

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Prompt-level fix doesn't actually change behavior — model still fabricates because the instruction is "soft." | Verification gate (R4) requires 0/5 fabrication rate. If verification fails, escalate to structural fix (telemetry-driven detection + retry, mika#952 direction 3). |
| Adding instructions bloats the prompt; model loses focus on review work. | Two short instructions; minimal token cost. Verify against pass-rate baseline post-deploy. |
| `gh_read` is not available in mika-arch's tool set. | Verify before /ce:work — if not available, this fix is blocked on adding it (likely already present per mika-arch's existing capability). |
| Plan-doc-check hook fails. | Manually cite plan path in PR body. |

## Documentation / Operational Notes

- **Rollout:** Prompt-only change. PR merge → bundled-skill resync on next agent session start. No deploy step beyond the standard.
- **Verification timeline:** After merge, observe next 5 mika-arch reviews. Operator-audit each response against the brief's referenced content. Fabrication rate should be 0/5.
- **Pattern claim (N=2 for mika-arch family):** mika#947 (persistence-meta) + mika#952 (citation-fabrication) form a 2-instance pattern of "mika-arch-specific failure modes that surface as confident-but-wrong outputs." After both fixes ship, author compound doc on the discipline (failure-mode catalog + per-mode prompt-level mitigations + telemetry future-pointer).

## Sources & References

- **Ticket:** [mika#952](https://github.com/senara-solutions/mika/issues/952)
- **Verified sessions:** `39f5f998-1199-4fdf-bfb2-2119eab9d5aa` (mika#931 pass-1+2), `fba22d43-c46e-49ad-82af-9992e7c4636a` (mika#928 pass-1+2).
- **Sibling tickets:** mika#947 (persistence-meta), mika#939 / PR #941 (Opus deadline).
- **Source files:** `mika/skills/bundled/mika-arch-groom-ticket/system_prompt.md`, `mika/skills/bundled/mika-arch-second-review/system_prompt.md`, `mika/skills/bundled/mika-arch-groom-milestone/system_prompt.md`.
- **Related institutional knowledge:** `project_mika_arch_failure_modes.md`, `feedback_qa_provider_perf.md`, `mika/docs/architecture/review-guide.md` (citation-or-silence principle).
