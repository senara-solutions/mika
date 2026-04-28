---
title: "Core memory is a citation surface, not a rules accumulator — three-way filter for accreted blocks"
date: 2026-04-28
category: best-practices
module: agent-memory, mika-arch, mika-dev
problem_type: best_practice
component: agent-behavior
severity: medium
applies_when:
  - An agent's core memory block (self_model, current_priorities, key_people, workflows) is approaching the 500-token-per-block cap
  - Auditing a core memory block that has accreted incident-derived rules over many edits
  - Designing a promotion protocol for moving rules from core memory to durable artifacts (compound docs, system prompts, tickets)
  - Reviewing whether an `update_core_memory` write is the right tool vs the rule belonging elsewhere
related_components:
  - core-memory
  - soul-md
  - compound-docs
tags:
  - core-memory
  - accretion
  - promotion-protocol
  - three-way-filter
  - as-above-so-below
  - citation-surface
  - dry
---

# Core memory is a citation surface, not a rules accumulator — three-way filter for accreted blocks

## Context

On 2026-04-28, while debugging the architect skill skipping its prescribed pass-2 verdict line (mika#788, see `required-tools-gate-evasion-patterns-2026-04-28.md`), an audit of mika-arch's `current_priorities` and mika-dev's `self_model` core memory blocks surfaced a more general accretion pattern:

- mika-dev's `self_model` was at 471/500 tokens, holding 7 incident-derived behavioral rules accreted over 13 days. Each edit reasoning explicitly described compress-to-fit-to-add-new-rule. The block's stated purpose ("identity and active interaction notes") had been crowded out by what was effectively a `behavioral_rules` accumulator.
- mika-arch's `current_priorities` was at 372/500, holding 5 structural items (foundational citation list, deferred decisions, known gaps, N≥2 patterns) with explicit "promote to ticket on real pressure" / "Compound doc pending one more cycle" markers in the text itself. Each promotion trigger was written into the rule but never fired — items stayed in-line, the block compressed.

The shared root cause: **core memory was being used as the durable artifact for content whose durable artifact already existed (or should exist) elsewhere**. The 500-token-per-block cap forced compression, but compression preserved every category and just tightened formatting. Items that should have been promoted (to a compound doc, to a ticket reference, to the system prompt) were instead reformatted to fit alongside new accretions.

This is the inverse of what core memory is for. Layer 1 (core memory) is supposed to be identity + active interaction notes — content that genuinely needs to be in every system prompt because it's stateful per-agent context. Layer 2 (structured facts via `store_fact`) and Layer 3 (hybrid search over compound docs) are where rules and references belong. The 500-token cap was doing the right thing — surfacing pressure — but the agent's response to pressure was compress, not promote.

## The Three-Way Filter

For each accreted item in core memory, ask three questions in order. The first "yes" determines the bucket:

### Bucket 1 — Existing artifact, drop in-line, replace with citation

Does the item have a durable artifact already, somewhere other than core memory?

- A filed ticket → cite the ticket, drop the in-line text (the ticket is the durable record)
- A compound doc in `docs/solutions/` → cite the doc, drop the in-line text (the doc is the durable artifact)
- System prompt content (soul.md, skill prompts) → confirm it's there and drop the duplicate
- The agent's identity templates in `crates/mika-agent/src/well_known_agents.rs` → ditto

If yes: drop in-line. Replace with a one-line citation. **Recurrence count is irrelevant** — the artifact already exists, additional in-line copies are pure DRY violation.

This is the most common bucket. Today's audit on mika-dev's `self_model` found that 5 of 7 rules already had compound-doc artifacts (fabrication risk → `741-grounding-fabrication-regression-scenarios.md`; persistence-before-output → `engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`; skill prompt access → `pre-tool-context-redundancy-check.md`; etc.). The block was hoarding paraphrased duplicates of docs it had already written.

### Bucket 2 — N≥2 recurrence, promote to compound doc

Does the item document a failure class that has recurred (N≥2) with distinct ticket references?

- N=1: keep in core memory as a single-incident fact (see Bucket 3)
- N≥2 with distinct tickets: promote to `docs/solutions/best-practices/<rule>.md`, then cite from core memory

The N=2 threshold matches the existing memory-classification practice: a single incident is a fact, a recurrence is a pattern that earns a doc. The threshold is load-bearing — N=1 promotion is too aggressive (every incident becomes a doc, `solutions/best-practices/` becomes its own accretion); N=3+ is too lenient (the second occurrence is decisive evidence the failure class is structural, not a one-off).

Today's audit on mika-arch's `current_priorities` found two clean Bucket-2 items: pre-commit-discovery (N=4 across mika#52, #636, #665, #663) and required-tools-gate evasion (N=2 across mika#654, #788). Both were promoted to compound docs in PR #860 and are now cited from core memory rather than duplicated.

### Bucket 3 — N=1 with no existing artifact, keep with recurrence-watch

Single-incident, no durable artifact yet?

- Keep in core memory annotated `[recurrence-watch: N=1, <ticket-ref>]`
- Next occurrence triggers re-evaluation: jumps to Bucket 2 (promote to compound doc)
- The annotation is the trigger — when the next audit sees the watch and a new occurrence, the rule is promoted, not re-accreted

Today's audit found exactly one Bucket-3 item on mika-dev's `self_model`: "Worktree ownership — Only claude-pilot owns the worktree" (N=1, mika#844). It stays with `[recurrence-watch: N=1]`.

## Why The Filter Beats Alternatives

Two simpler frameworks were considered and rejected:

**Two-way (existing-artifact / N≥2-promote) without bucket 3.** Fails on the N=1 case — either the rule is dropped (lose the lesson) or it stays in-line indefinitely (re-accretion). The recurrence-watch annotation is the load-bearing piece that prevents both failure modes.

**Age-only (>7 days unchanged → promote).** Fails because staleness alone has no decision content. The architect's foundational citation surface had been unchanged for 30+ days and never got promoted — the agent had no procedure for *what to do with* a stale item. Age is a *trigger* for the filter, not a *substitute* for it.

The three-way filter's actual decision logic is bucket assignment, not age. Reflection passes that scan for promotion candidates should surface items by bucket, not by staleness — otherwise accretion just moves from `update_core_memory` to `store_fact`.

## Applied To Today's Audit

mika-dev `self_model`:
- Identity preamble (1 sentence) — KEEP, identity content
- Fabrication risk rule → Bucket 1, cite `docs/solutions/741-grounding-fabrication-regression-scenarios.md`
- Root-cause discipline (mika#844) → Bucket 1, cite `docs/solutions/workflow-issues/centralized-branch-derivation-companion-2026-04-28.md`
- Operational memory (persistence-before-output) → Bucket 1, cite `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`; also note mika#864 as the engine-side migration path
- Communication style → DROP, already in `soul.md` "Communication style" section
- Worktree ownership → Bucket 3, keep with `[recurrence-watch: N=1, mika#844]`
- Scope task checks → DROP, already in `soul.md` "Proactive behaviors" section
- Skill prompt access → Bucket 1, cite `docs/solutions/architecture-patterns/pre-tool-context-redundancy-check.md`

Net: 471/500 → ~120/500 tokens (75% reduction), no information loss. The block becomes citation-shaped + 1 active recurrence-watch fact.

mika-arch `current_priorities`:
- "Foundational citation surface" (Item 1) → Bucket 1, the durable artifact is the agent's `soul.md`. **This PR moves the citation list into `MIKA_ARCH_SOUL` constant in `crates/mika-agent/src/well_known_agents.rs`.** Drop from core memory after deploy.
- "Deferred decisions" (Item 2) → DELETE. Process noise — deferred decisions that aren't actively being decided don't belong in `current_priorities`.
- "Known gap (MIKA_ARCH_DISABLED_TOOLS ticket filed)" (Item 3) → Bucket 1, the ticket itself is the durable artifact. Drop in-line, the agent finds the ticket via `gh_read` when relevant.
- "Pre-commit discovery (N=4)" (Item 4) → Bucket 2, promoted in PR #860 to `verification-claims-with-expected-output-shape-2026-04-28.md`. Cite, don't duplicate.
- "Required-tools-gate evasion patterns (N=2)" (Item 5) → Bucket 2, promoted in PR #860 to `required-tools-gate-evasion-patterns-2026-04-28.md`. Cite, don't duplicate.

Net: 372/500 → near-empty. The block returns to its stated purpose (active grooming queue) instead of being a permanent-knowledge accumulator.

## Application

When auditing an accreted core memory block:

1. List each accreted item.
2. Apply the three-way filter to each, in order.
3. For Bucket 1: identify the existing artifact, replace the in-line text with a one-line citation.
4. For Bucket 2: write the compound doc (or confirm one exists), then cite from core memory.
5. For Bucket 3: rewrite with a `[recurrence-watch: N=1, <ticket>]` annotation if not already present.
6. After filter applied, the block should be substantially smaller and contain mostly citations + identity content + a small set of N=1 recurrence-watch facts.

When writing new core memory:

- Before calling `update_core_memory`, ask: does this content already exist somewhere durable? If yes, cite it instead of duplicating.
- If the content is incident-derived and you have a ticket reference, write the rule with `[recurrence-watch: N=1, <ticket>]` from the start. Future audits then have a clean trigger for promotion.
- Resist the impulse to "promote" rules into core memory. The direction of promotion is the inverse: out of core memory, into compound docs / system prompts / tickets.

When designing a reflection-pass spec for runtime enforcement (separate ticket):

- Surface candidates by bucket assignment, not by age. Bucket 1 candidates (existing-artifact duplicates) are the most common and the highest-leverage to surface.
- Don't auto-promote. Surface the candidate + bucket + suggested action; let the agent (or operator) confirm.
- Bucket 3 staleness-only triggers a re-evaluation, not a promotion — the rule was N=1; the audit checks whether N has incremented before promoting.

## Why The Block Cap Stays At 500

A natural reaction to today's pressure would be "raise the per-block cap so the architect can fit more." This is rejected: the 500-token cap is the only structural pressure that surfaces the accretion problem. Removing it eliminates the only signal that forces a promotion review. The cap is the feature, not the bug.

The right response to "block is full" is the three-way filter. The wrong response is compress-to-fit; the also-wrong response is raise-the-cap.

If a future agent genuinely needs more identity surface than 500 tokens, that's a per-agent calibration ticket with concrete evidence — not a global cap bump. Today's audit found the opposite: every full block had Bucket-1 items that should have been dropped, not preserved at higher density.

## Citations

- senara-solutions/mika#788 — the architect skill skipping its pass-2 verdict that triggered today's audit
- senara-solutions/mika#860 — PR1 of this redesign: shipped two compound docs (`required-tools-gate-evasion-patterns`, `verification-claims-with-expected-output-shape`) as the bucket-2 promotion targets cited above
- `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` — Rule 3 makes the same prompt-vs-structural argument applied to the catalogue itself; this doc is the policy that follows from that observation
- `crates/mika-agent/src/well_known_agents.rs` — `MIKA_ARCH_SOUL` constant where this PR adds the foundational citation list (Bucket 1 move out of mika-arch's `current_priorities`)
- mika#862, mika#863, mika#864 — engine-side guard tickets that the rules cited from core memory point at as their structural migration path
- `feedback_compound_infra_fixes.md` (mika-platform memory) — infra fixes evaporate fast; compound them
- `docs/memory-classification.md` — Layer 1/2/3 design that today's audit confirms is correct (the failure was in *application*, not in *layer design*)
- `docs/solutions/best-practices/structural-check-replaces-human-discipline-2026-04-27.md` — methodological precedent for threshold-based promotion (N=3 prompt-discipline → CI gate). The N≥2 promotion threshold in this doc is the same shape applied to memory accretion.
- `docs/solutions/architecture-patterns/structural-readonly-agent-binds-at-every-layer-2026-04-25.md` — methodological precedent for layer-aware decomposition. The three-way filter is structurally analogous: decompose accreted items into independent buckets, each with its own binding mechanism (drop-and-cite / promote-to-doc / recurrence-watch).

## Out of scope (separate tickets)

- `mika core-memory set --agent X --section Y --content "..."` operator CLI for direct admin edits to core memory. Today's audit had to use `mika ask --agent X "use update_core_memory replace ..."` (canonical agent-loop path), which is correct for runtime but heavyweight for one-time operator surgery. Worth filing as a separate enhancement.
- The reflection-pass spec for runtime enforcement of this protocol — a separate ship that uses this doc as the policy reference.
- The promotion-protocol additions to mika-dev's and mika-arch's system prompts — same separate ship; the system prompts will reference this doc rather than re-encoding the protocol.

## Post-deploy steps (operator)

**Step 0 — Refresh the provisioned soul.md on existing hosts.** The `provision_well_known_agents()` path is idempotent and skips agents that already exist on disk (`crates/mika-agent/src/well_known_agents.rs:351-413`). Hosts that have ever booted with `MIKA_DEV_MODE=true` will keep their old `~/.mika/agents/mika-arch/soul.md` template *without* the new `## Foundational references` section unless explicitly refreshed. To pull in the new template:

```bash
sudo rc-service mika-server stop
rm ~/.mika/agents/mika-arch/soul.md
sudo rc-service mika-server start
# Verify:
grep -A 5 "## Foundational references" ~/.mika/agents/mika-arch/soul.md
```

This step is host-by-host and is NOT applied automatically by deploy. Operators with manually-customized mika-arch souls should diff before deletion.

After this is done — and the live core-memory blocks still hold the old accreted content — to complete the extraction:

```bash
# mika-dev self_model: extract per the bucket assignments above
mika ask --agent mika-dev --format json "Use update_core_memory action=replace section=self_model with content: \
'I am Mika, lead engineer. Orchestrate, vision, manifest. Claude implements via claude-pilot. I direct, track, review, decide.

**Behavioral rules — see compound docs (durable artifacts, do not duplicate inline):**
- Fabrication risk on read-tool failures → docs/solutions/741-grounding-fabrication-regression-scenarios.md
- Root-cause discipline (cite tool output, never assert against it) → docs/solutions/workflow-issues/centralized-branch-derivation-companion-2026-04-28.md
- Persistence-before-output → docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md (mika#864 tracks engine-side enforcement)
- Skill prompt access (context vs disk) → docs/solutions/architecture-patterns/pre-tool-context-redundancy-check.md

**Worktree ownership** [recurrence-watch: N=1, mika#844]: Only claude-pilot owns the worktree. No sandbox fixes for worktree bugs.

(Communication style + scope-task-checks already in soul.md — not duplicated here.)' \
reasoning='Extract to citations per docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md three-way filter. 5 of 7 rules are Bucket-1 (existing artifact); 1 already in soul.md (Bucket DROP); 1 is Bucket-3 (recurrence-watch).'"

# mika-arch current_priorities: drop the accreted items now that soul.md carries the foundational refs
mika ask --agent mika-arch --format json "Use update_core_memory action=replace section=current_priorities with content: \
'(No active priorities. Use this block for current grooming queue items as they appear. Foundational references moved to soul.md per docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md. Compound-doc citations for verification-claims and required-tools-gate-evasion patterns are findable via gh_read on referenced tickets when needed.)' \
reasoning='Block was holding permanent institutional knowledge that belongs elsewhere — foundational refs moved to soul.md (PR #N this is from), N≥2 patterns moved to compound docs in PR #860, ticket-filed gaps are findable via gh_read. current_priorities returns to active grooming queue purpose.'"
```

These commands are not run as part of the PR — they are operator follow-ups after deploy.
