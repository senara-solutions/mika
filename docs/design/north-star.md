# Mika Design North Star

**Status:** Active — the WHY behind every visual and interaction decision across the Mika ecosystem.
**Scope:** All three product surfaces — Observability Dashboard, Cloud Console, Landing Page — and the shared `@senara-solutions/ui` component library that backs them.
**Owner:** Vincent. Updates land as direct commits to main; this document is not relitigated through PRs.
**Companion:** [`luminescent-core.md`](./luminescent-core.md) — the design system rulebook this north star asks us to apply.

---

## The frame

We are building an **ecosystem**, not a tool. The three surfaces are rooms in the same house. People who use Mika — and we who build it — should feel at home moving between them.

Mika is **new-generation technology**. The interface signals that. It does not look like 2015 enterprise software, and it does not look like every other AI platform. It looks like what comes next — premium, quiet, intentional.

## Principles, in priority order

1. **Intuitive — low cognitive load.** What you need is in front of you. Noise is muted. Depth is one click away when you want it; we don't strip information, we layer it. Smooth and enjoyable to navigate. The right test for a screen is "does this feel easy?" — not "does this show everything we have?"
2. **New-gen, not legacy.** Modern, tech-forward aesthetic. The Luminescent Core captures this exactly: dark palette, soft minimalism, tonal layering, ambient light, no heavy borders, no sharp corners.
3. **Pleasant to the eye, smooth to the touch.** Sensory before functional. If a screen does not feel good to look at and move through, it has failed the first test, regardless of what it shows.
4. **Elegant + modern.** Refined, contemporary, restrained. Nothing decorative. Nothing nostalgic. Nothing that calls attention to itself instead of the content.
5. **Uniform across the ecosystem.** **One design system** ([The Luminescent Core](./luminescent-core.md)), three surfaces. Same colors, same typography, same spacing, same components, same motion language. Anywhere in Mika, you should know you're in Mika.
6. **At home — for them and for us.** We are going to live in this. Make it a place worth living in. The people we ship to should feel cared for; we should also want to spend time here.
7. **The most recent decision wins.** When older work conflicts with newer work, the newer work is right. The Cloud Console is the most recently iterated surface; The Luminescent Core captures the current taste. Older surfaces (Dashboard, Landing Page) bring themselves into alignment with it — not the reverse.
8. **The system is the law.** Implementation PRs apply the rulebook; they do not relitigate it. Once a rule is in The Luminescent Core, deviation is the violation. PRs that "feel different" don't ship — they propose an extension to the system first. Updates to the rulebook itself are direct commits, owned by Vincent.

## What this means in practice

### One rulebook, three surfaces

[The Luminescent Core](./luminescent-core.md) is the single Mika design system. Every surface adopts it; surfaces never fork it. When a surface needs something the rulebook doesn't cover (e.g., the Dashboard needs trace widgets the Cloud Console doesn't), the answer is to **extend** the rulebook — add the new pattern to it, mark which surfaces use it. The rulebook grows. It never splits.

The technical embodiment of the rulebook is the `@senara-solutions/ui` package (`packages/ui/` in the `mika` repo, published to GitHub Packages). Tokens, primitives, and components defined there are consumed by all three surfaces. Anything that needs to be uniform lives there.

### Three reconciliations, sequenced

Each surface needs a reconciliation pass: walk through what exists, categorize relevant / stale / orphaned, map to the rulebook, fill gaps. The order — and expected effort — is set by the trust hierarchy (principle #7):

| Order | Surface | Why this order | Effort |
|---|---|---|---|
| 1 | Observability Dashboard | First reconciliation establishes the workflow. No design system attached today. | Heavier |
| 2 | Cloud Console | Source of The Luminescent Core. Mostly aligned with itself; needs a confirmation pass that the implementation matches the rulebook and tokens flow cleanly through `@senara-solutions/ui`. | Lighter |
| 3 | Landing Page | Oldest surface, most likely drifted. Likely needs a redesign pass. | Heaviest |

The workflow we agree on for the Dashboard becomes the template for the other two.

### What gets reviewed in PRs (and what doesn't)

| Artifact | How it changes |
|---|---|
| The north star (this file) | Direct commit to main, owned by Vincent. |
| The Luminescent Core rulebook ([`luminescent-core.md`](./luminescent-core.md)) | Direct commit to main, owned by Vincent. Extensions are also direct commits. |
| `@senara-solutions/ui` primitives — adding/changing components to match the rulebook | PR. The PR's job is to faithfully apply what the rulebook already says, not to debate the rulebook. |
| Surface-level implementation (Dashboard pages, Console pages, Landing Page sections) | PR. Reviewed against the rulebook + this north star, not against personal taste. |

The discipline is: **arguments about the system happen with Vincent on the rulebook itself.** Arguments about implementation happen in PRs against the system.

## How to use this document

Before designing or implementing anything visual in Mika:

1. Read this file. It is short on purpose.
2. Read [`luminescent-core.md`](./luminescent-core.md). It is the rulebook.
3. Make a decision. If the rulebook covers it, follow the rulebook. If it doesn't, propose an extension to Vincent before shipping a one-off.

When reviewing a PR that touches a visual surface, the question is not "do I like this?" but "**does this honor the system?**" If the answer is no, the fix is either to bring the change into compliance with the rulebook, or to propose a rulebook extension first and then revisit the PR.

## Status

- **2026-04-25:** Document created. First reconciliation underway on the Observability Dashboard (mika#669). Cloud Console and Landing Page reconciliations to follow as separate sessions.
