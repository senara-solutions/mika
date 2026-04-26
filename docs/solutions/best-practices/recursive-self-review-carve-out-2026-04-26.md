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

## When to revisit this carve-out

If two conditions become true:

1. **mika-arch's vested-interest reasoning becomes empirically demonstrable.** If a future review of her own surface produces reasoning that an external reviewer flags as biased (rationalized toward yes when no was warranted), the carve-out has empirical justification beyond optics. Until that happens, the carve-out is precautionary, not corrective.
2. **A formal review-guide section codifies the carve-out.** If we want this discipline to outlast the current operator/architect arrangement, it should live in `review-guide.md` as a structural rule, not just here. Consider adding a "self-review boundary" subsection if the carve-out gets exercised more than 3 times.

Until then: this compound is the durable record. Future operators reading "why didn't we send #818 through mika-arch" find this doc.

## Related

- senara-solutions/mika#818 — first instance exercising the carve-out (drop memory-write tools from `MIKA_ARCH_DISABLED_TOOLS`).
- senara-solutions/mika#817 — counter-example where the carve-out doesn't apply (gh_read.file_view extension; mika-arch reviews implementation, not her own config).
- `mika/docs/architecture/review-guide.md` § 5 Orthogonality — the agent self-state vs platform side-effects distinction (commit `2bba6223`) that mika#818 cites and that the carve-out's reasoning rests on.
- `mika-platform/.claude/commands/mika-groom-ticket.md` — the standard grooming workflow this carve-out diverges from.
- `mika/docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — broader context on the architect's operational discipline; the operator-proxy memory-seeding pattern (added 2026-04-26) is a parallel scaffolding pattern for related gaps.
