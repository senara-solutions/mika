---
title: "Recursive self-review carve-out — when an agent's own surface is the change target, route grooming to an external reviewer"
date: 2026-04-26
category: best-practices
module: mika-arch
problem_type: best_practice
component: grooming_workflow
severity: medium
applies_when:
  - A ticket modifies the reviewing agent's own tool kit, skill set, identity config, or permission surface
  - The reviewing agent would be both the reviewer and the beneficiary of the change being reviewed
  - The agent's role design (e.g., mika-arch as advisory architect) intersects with a permission audit of itself
tags:
  - mika-arch
  - grooming-workflow
  - recursive-review
  - external-reviewer
  - claude-chat
  - permission-audit
  - vested-interest
---

# Recursive self-review carve-out

## Context

mika-arch grooms tickets via `/mika-groom-ticket` (the engine-coupled workflow established at `mika-platform/.claude/commands/mika-groom-ticket.md`). The architect produces a Disposition (READY / ITERATE / ESCALATE) on first pass, a Verdict (GROOMED / ESCALATE) on second pass — citation-or-silence operating discipline per `mika/docs/architecture/review-guide.md` § 6.

When a ticket modifies mika-arch's *own* tool kit, skill set, identity config, or permission surface — e.g., mika#818 (drop memory-write tools from `MIKA_ARCH_DISABLED_TOOLS`), the eventual `[tools].allowlist` migration, the per-skill `[llm]` revival, or any future change to her review-guide reference — the architect would be both the reviewer and the beneficiary of the change. Even if the citation discipline holds, the optics undermine the architect's authority on her own permission surface, and the vested-interest concern is real (she has incentive to grant herself tools, and "I find no reason not to" is harder to trust when the reviewer benefits from the conclusion).

mika#818 surfaced this concretely on 2026-04-26: the ticket asks "should mika-arch have memory-write tools" and the natural reviewer for any other ticket is mika-arch herself. Routing it through mika-arch creates a recursion that's structurally OK (she'd cite review-guide § Orthogonality, which the operator could verify) but operationally awkward.

## Guidance

**When a ticket's primary scope is mika-arch's own tool/skill/identity/permission surface, route grooming to an external reviewer instead of mika-arch.** Today the external reviewer is Claude Chat (Mika project) — a distinct Claude instance with the mika repo as project knowledge but without mika-arch's session memory, tool kit, or vested interest. Tomorrow it could be any external Claude/peer with comparable context.

The carve-out trigger is whether the ticket changes how mika-arch operates, not just whether mika-arch is mentioned:

| Ticket scope | Reviewer |
|---|---|
| Modifies `MIKA_ARCH_DISABLED_TOOLS`, mika-arch's identity.toml `[tools]`/`[skills]`/`[kg]` blocks, mika-arch's skill.toml manifests, mika-arch's prompts | **External (Claude Chat)** |
| Modifies code mika-arch uses (e.g., `gh_read.file_view` extension — mika#817) but doesn't change her configuration | **mika-arch** |
| Modifies skills mika-arch isn't allowlisted for, agents other than mika-arch, infrastructure unrelated to her role | **mika-arch** (default) |

The principle: **the reviewer who is structurally a stakeholder in the outcome shouldn't be the sole authority on the outcome.**

## Why this matters

Two compatible reasons that go past optics:

**1. Vested interest is a real reasoning hazard.** Even with citation discipline, a reviewer evaluating "should I have this capability" can rationalize toward yes more easily than a neutral reviewer can. Citation discipline catches *unmoored* concerns; it doesn't catch *biased framing* of the cited concerns. External review pierces that.

**2. The audit trail is cleaner.** When the architect ratifies a change to her own surface, the trail reads "the agent approved her own permission expansion." That's a defensible position only because the citation discipline holds, and verifying that requires re-reading the entire review for principle-grounded reasoning. When an external reviewer ratifies the change, the trail reads "external reviewer approved the agent's permission expansion." No verification overhead — the structural separation is the audit value.

## When to apply (concrete recipe)

Before dispatching `/mika-groom-ticket` on a ticket, ask: *"does this ticket modify how mika-arch operates — her tool kit, skill kit, identity config, prompts, or any other config that mika-arch herself reads at runtime?"*

- **Yes:** route external. Workflow: draft the plan locally, paste the plan + relevant source to Claude Chat (Mika project), receive the review, apply iterations, commit, push, attach callouts to the issue body — same artifact shape as mika-arch grooming, with the reviewer named in the Grooming history callout (`> - **Architect verdict:** GROOMED (Claude Chat / Mika project external review per the recursive-self-review carve-out)`).
- **No:** dispatch `/mika-groom-ticket` normally. mika-arch reviews on her own session.

The external-review path preserves the same artifact shape as the mika-arch path: plan-on-branch is still the contract, branch + plan callouts on the issue body are still canonical, the Grooming history just names a different reviewer. Audit symmetry holds.

## Examples

### Established 2026-04-26 — mika#818

Ticket: drop `update_core_memory`, `store_fact`, `update_fact` from `MIKA_ARCH_DISABLED_TOOLS`. Modifies mika-arch's own tool kit. Routed to Claude Chat. External reviewer returned `Verdict: GROOMED with 5 small additions` (D3 rationale extension, test naming + local array, validation rows-unchanged check, doc-comment positive framing, new Unit 5 PR-description security summary). Plan committed at `0c5e3f63` on branch `fix/mika-arch-allow-memory-writes`. Issue body callouts include `> - **Architect verdict:** GROOMED (Claude Chat / Mika project external review per the recursive-self-review carve-out — see "External review" comment below)` — naming the reviewer explicitly so the audit trail captures the carve-out reasoning.

### Counter-example — mika#817

Ticket: add `gh_read.file_view` op to the existing `gh_read` builtin. mika-arch *uses* `gh_read` (it's in her tool kit), but the change is to the implementation in `crates/mika-agent/src/skills/builtin_handlers.rs`, not to her configuration. mika-arch is not a structural stakeholder in whether the implementation is correct — anyone using `gh_read` would benefit equally. Routed to mika-arch normally. First-pass ITERATE with 7 findings (including the load-bearing path-charset URL-decoding catch); second-pass after iterations returned `Verdict: GROOMED on corrected spec`.

The distinction held: mika-arch reviews implementation-of-tools-she-uses but external review is correct for changes-to-mika-arch-herself.

## Causation vs outcome-shape (sharpened 2026-05-01, mika#904)

The original carve-out trigger was **surface-shape only**: does the plan touch the reviewer's operational surface? If yes, fire. This proved over-broad — it routes second-pass externally for any change touching mika-arch's substrate, even when the reshaping was driven by operator judgment or external peer review rather than reviewer pressure. The empirical case was mika#874 (see Worked example below).

The sharpened trigger requires **both** conditions:

1. **Surface-shape gate:** the plan modifies the reviewer's surface (skills, identity config, permission surface, prompts).
2. **Provenance gate:** the iteration history shows reviewer-driven reshaping — the reviewer's first-pass critique, reviewer-emitted pressure, or reviewer-authored position (persisted in memory and cited by second-pass) FIRST introduced the position the plan now ratifies.

If only the surface-shape gate holds (coincidental alignment with externally-driven decisions), the carve-out does NOT fire. The second-pass quality gate (mika-arch-second-review, Sonnet 4.6) is the appropriate reviewer.

**The provenance test:** read the iteration-history trace top-to-bottom. The first tool/message/comment that asserts the position the final plan ratifies is the **introducer**. If the introducer is the reviewer, fire. If the introducer is the operator, an external peer reviewer, or a canonical workflow document independent of the reviewer, don't fire on outcome-shape grounds alone.

**Memory-cite transport boundary:** when second-pass cites reviewer's persisted memory (`store_fact` rows readable via `search_memory`), trace through the persistence to the original introducer. Reviewer-persisted-but-operator-introduced positions are still operator-introduced — persistence is transport, not position-authoring.

This sharpening narrows the trigger condition, not the consequences when triggered. The three-state taxonomy:

| Case | Surface-shape | Provenance | Verdict |
|------|--------------|------------|---------|
| Neither condition | ✗ | ✗ | Review normally |
| Surface-shape only | ✓ | ✗ | Review normally (carve-out narrowed) |
| Both conditions | ✓ | ✓ | Fire carve-out, route external |

## Isolation axis: identity vs prompt vs memory (sharpened 2026-05-01, mika#904)

The original carve-out didn't specify what makes a review "recursive." mika-arch has:

- **Two skill prompts:** `mika-arch-groom-ticket` (Opus 4.7, first-pass) and `mika-arch-second-review` (Sonnet 4.6, second-pass).
- **One agent identity:** mika-arch.
- **One shared memory store:** `store_fact` rows written by the first-pass session are readable by the second-pass session via `search_memory`.

Empirical evidence from mika#874 grooming: the second-pass cited F8/F9 findings extracted from `store_fact` rows the first-pass wrote (verified via `sqlite3 ~/.mika/data/mika.db "SELECT input FROM tool_calls WHERE session_id='...' AND tool_name='store_fact'"`). The two passes look structurally separated (different prompts, different models) but are memory-coupled.

**Memory-shared is the load-bearing isolation axis.** The prompt+model split provides genuine structural diversity (different reasoning surfaces). But shared memory means the second-pass reads the first-pass's persisted findings, defeating the diversity. Memory-shared coupling is what makes the second-pass non-independent, regardless of prompt-and-model diversity.

The three candidate axes:

| Axis | Trigger condition | Assessment |
|------|------------------|------------|
| **Agent-identity-shared** | Any review by an agent on its own substrate | Over-conservative — fires on cases where prompt-and-memory split eliminates the recursive coupling |
| **Prompt-shared** | Same skill prompt on both ends | Under-conservative — the current two-skill split would defuse it, but shared memory means the second-pass still reads first-pass reasoning |
| **Memory-shared** | First-pass and second-pass share memory state on the same ticket | **Load-bearing** — captures the actual coupling vector empirically demonstrated in mika#874 |

**Future work:** if memory-key namespacing between `mika-arch-groom-ticket` and `mika-arch-second-review` skills is introduced (separate ticket), the memory-shared trigger axis would be defused, narrowing the carve-out to agent-identity-shared cases. That makes the eventual memory-namespacing ticket's value proposition explicit: it shrinks the carve-out trigger surface from "memory-shared" to "identity-shared."

## Precedent walk under the sharpened rule

The original evidence base cited mika#788/#872/#879. Per mika#868's PR body (the most recent codification ratification, verified via `gh issue view` on 2026-04-30), the authoritative three instances are **mika#818, mika#868, mika#879**. mika#788 is a `run_gh` tool allowlist fix, NOT a §7 carve-out instance (zero recursive-self-review framing in the ticket). mika#872 is the implementation PR for mika#868's grooming, not a distinct instance.

### mika#818 (instance 1 — drop memory-write tools from MIKA_ARCH_DISABLED_TOOLS)

- **Surface-shape gate ✓** — modifies mika-arch's tool denylist (`MIKA_ARCH_DISABLED_TOOLS`). Direct substrate change.
- **Provenance gate ✓** — the ticket originated from operator + architect discussion about whether mika-arch should have `store_fact`/`update_core_memory` tools. The question "should mika-arch have memory-write tools" is inherently self-interested when the reviewer is mika-arch herself — she would be evaluating her own capability expansion. The reviewer IS the natural introducer of the "yes" position.
- **Verdict under sharpened rule:** carve-out fires. Consistent with original classification. No regression.

### mika#868 (instance 2 — promotion-protocol system-prompt additions + reflection-pass spec)

- **Surface-shape gate ✓** — modifies mika-arch's skill prompts (promotion protocol additions to mika-arch bundled skills).
- **Provenance gate ✓** — the promotion-protocol additions reshape mika-arch's own review discipline and reflection-pass specification. The spec originated from architect-initiated grooming patterns (the reflection pass is a capability mika-arch exercises). The reviewer is a stakeholder in how the reflection spec is shaped.
- **Verdict under sharpened rule:** carve-out fires. Consistent with original classification. No regression.

### mika#879 (instance 3 — milestone grooming additions to mika-arch bundled skills)

- **Surface-shape gate ✓** — adds to mika-arch bundled skills (milestone grooming skill surface).
- **Provenance gate ✓** — the milestone grooming additions extend mika-arch's skill set. The architect reviewing additions to her own skill portfolio has inherent vested interest in what capabilities she receives.
- **Verdict under sharpened rule:** carve-out fires. Consistent with original classification. No regression.

**Summary:** all three precedent instances fire under the sharpened rule. The provenance gate does not reverse any prior classification — the sharpening narrows the trigger for cases where externally-driven decisions coincidentally benefit the reviewer, not for cases where the reviewer is the natural stakeholder.

## Worked example: mika#874 (provenance test in action)

This is the case that motivated the sharpening (mika#904). It demonstrates the provenance test producing a deterministic verdict on a case the surface-shape-only rule mis-classified.

### Iteration history

1. **mika-arch first-pass (Opus 4.7):** reviewed plan for mika#874. F4 finding asked for explicitness on the KG resolver outcome enum value — "commit to schema bump OR mark out-of-scope-with-rationale." The first-pass did NOT advocate for the schema-bump direction; it asked the plan to be explicit about whichever choice was made.

2. **Operator decision:** Vincent chose schema-bump per Signal C citation in `mika/CLAUDE.md` § Post-restart safety check #757 — the KG resolver observability surface benefits from a schema version bump that makes the v28→v29 transition explicit in the resolver's audit queries. This decision was informed by external peer-review brief (Claude Chat session outside the mika-arch identity).

3. **Plan iteration:** F4 iterated from "out-of-scope" to "schema bump" — ratifying the operator's externally-informed decision.

4. **mika-arch second-pass (Sonnet 4.6, mika-arch-second-review skill):** ESCALATEd citing F9: *"when a plan's iteration history shows decisions reshaped to benefit the reviewer's own consumption surface, that IS the vested-interest signature, regardless of whether the reasoning is sound."*

### Applying the sharpened test

- **Surface-shape gate ✓** — plan modifies mika-arch's KG resolver observability surface (schema bump v28→v29 makes resolver audit queries consumable by mika-arch's grooming workflow).

- **Provenance gate ✗** — trace the iteration history top-to-bottom:
  - The position "schema bump" was FIRST introduced by Signal C citation (`mika/CLAUDE.md` § Post-restart safety check #757) AND external peer-review brief, NOT by mika-arch's first-pass.
  - mika-arch's first-pass F4 explicitly asked for explicitness ("commit OR mark out-of-scope-with-rationale") — a procedural ask, not a design position. The first-pass did not introduce the "schema bump" position.
  - The F4 flip ratifies the operator's externally-introduced position, not a reviewer-introduced position.
  - Provenance gate fails.

- **Both gates must hold → § 7 does NOT fire.**

### Deterministic verdict

F9 does not apply. mika-arch second-pass is the appropriate quality gate. The second-pass should route in-pipeline (mika-arch-second-review skill) under the sharpened rule. The F9 ESCALATE was a false positive caused by the surface-shape-only trigger condition — the decision coincidentally benefited the reviewer's consumption surface, but the decision was externally driven.

## When to revisit this carve-out

**Historical context** — the two conditions below were the original triggers for revisiting. Condition 2 is now satisfied; condition 1 remains precautionary.

1. **mika-arch's vested-interest reasoning becomes empirically demonstrable.** If a future review of her own surface produces reasoning that an external reviewer flags as biased (rationalized toward yes when no was warranted), the carve-out has empirical justification beyond optics. Until that happens, the carve-out is precautionary, not corrective. *(Status: not yet observed.)*
2. **~~A formal review-guide section codifies the carve-out.~~** *(Satisfied 2026-04-29.)* The rule is now codified in `docs/architecture/review-guide.md` § 7 "Self-review boundary". Three instances formed the evidence base: mika#818 (first instance), mika#868 (second instance — promotion protocol prompts), mika#879 (third instance — milestone grooming; triggered the 3-instance codification threshold). The review-guide section is now the authoritative reference; this compound doc remains as origin context and the detailed rationale record.
3. **Sharpened 2026-05-01 (mika#904).** Added causation-vs-outcome-shape distinction (provenance test) and isolation-axis specification (memory-shared as load-bearing axis). Fourth instance (mika#874) added as worked example demonstrating the sharpened rule. Precedent enumeration corrected from #788/#872/#879 to #818/#868/#879 per mika#868's PR body (authoritative codification ratification).

This compound doc is the origin record. The review-guide section (§ 7) is the authoritative rule for reviewers. Future operators reading "why didn't we send #818 through mika-arch" find this doc for the full reasoning; reviewers enforcing the rule cite § 7.

**Escalation threshold:** if the sharpened rule fails to discriminate (false-positives recur 3 times OR mika#874-class deterministic-verdict cases recur with iteration-history ambiguity), escalate to an engine-layer substrate-adjacency detector.

## Related

- senara-solutions/mika#818 — first instance exercising the carve-out (drop memory-write tools from `MIKA_ARCH_DISABLED_TOOLS`).
- senara-solutions/mika#868 — second instance (promotion protocol prompts and reflection spec touching mika-arch skills). mika#872 is the implementation PR for this grooming ticket, not a distinct instance.
- senara-solutions/mika#879 — third instance (milestone grooming); triggered the 3-instance codification threshold.
- senara-solutions/mika#874 — worked example demonstrating the sharpened rule (mika#904). Provenance test deterministic verdict: F9 does not apply.
- senara-solutions/mika#904 — the sharpening ticket (causation vs outcome-shape + isolation axis).
- senara-solutions/mika#817 — counter-example where the carve-out doesn't apply (gh_read.file_view extension; mika-arch reviews implementation, not her own config).
- `mika/docs/architecture/review-guide.md` § 7 Self-review boundary — the codified rule (added 2026-04-29, sharpened 2026-05-01).
- `mika/docs/architecture/review-guide.md` § 5 Orthogonality — the agent self-state vs platform side-effects distinction (commit `2bba6223`) that mika#818 cites and that the carve-out's reasoning rests on.
- `mika-platform/.claude/commands/mika-groom-ticket.md` — the standard grooming workflow this carve-out diverges from.
- `mika/docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — broader context on the architect's operational discipline; the operator-proxy memory-seeding pattern (added 2026-04-26) is a parallel scaffolding pattern for related gaps.
