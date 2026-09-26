# `control-monitor` and the Luminescent Core — scope decision

**Status:** Recommendation, pending Vincent's confirmation.
**Date:** 2026-09-20
**Recommendation:** `control-monitor` stays visually distinct. It does not adopt the Luminescent Core.
**Ticket:** mika#1804 (AC2), sub-issue of the LC reconciliation milestone mika#1799.
**Confirms or overturns:** Vincent, as owner of `docs/design/luminescent-core.md`.

---

## The question

mika#1804 observes that `senara-solutions/control-monitor` is "fully off-brand":
a blue `#3a82e0` rather than the canonical primary, React 18 rather than 19, no
Tailwind. It asks for an explicit decision — *adopt* or *stay distinct* — with a
rationale, and files a cross-repo ticket only if the answer is *adopt*. The
ticket frames this as "a bearing question, potential Prime escalation".

## The recommendation, and why it is a reading rather than a preference

**The rulebook already answers it, in its own second line.**
`docs/design/luminescent-core.md:4` states its scope, and states it by
enumeration:

> **Scope:** Observability Dashboard, Cloud Console, Landing Page, and the
> shared `@samidarko/ui` component library.

Four surfaces, named. The Landing Page is one of them — which is what makes the
other half of mika#1804 (aligning `site/` on canon) an obligation resting on the
normative document itself, not on anyone's taste. `control-monitor` is not one
of them.

A second reading points the same way and is worth stating because it is the one
the ticket itself reaches for. The rulebook's companion, `north-star.md`,
grounds the design system in what a *customer* meets: the system exists so that
the surfaces a user of Mika encounters read as one product. `control-monitor` is
an operator tool. Nobody outside the operators sees it, and it is not part of
the product's visual promise. Its coherence requirement is with the operator's
other instruments, not with the customer-facing surfaces.

So the recommendation is: **stay distinct**, and the rationale is that the
document which would have to demand adoption declines to name it.

## The nuance that could overturn this, stated plainly

A scope section that does not name something can mean two different things, and
they are not distinguishable from the text:

- an **exclusion** — the rulebook's owner considered `control-monitor` and left
  it out; or
- an **omission** — `control-monitor` simply was not on the table. The rulebook
  was authored during work on the Cloud Console and promoted to an
  ecosystem-wide rulebook on **2026-04-25** (`luminescent-core.md:7`). A surface
  that came into existence after that date would be absent from the list without
  anyone having decided anything.

**The datum that separates the two is the creation date of
`senara-solutions/control-monitor`, and it could not be established from this
worktree** — the repository is not checked out in the `mika-platform` workspace
(only a built artifact is installed, at
`/usr/local/share/control-monitor/frontend/assets/`), and this session had no
authenticated GitHub access to query it. It is one `gh repo view
senara-solutions/control-monitor --json createdAt` away for whoever confirms.

This is written down rather than resolved by assumption, because assuming it
either way would turn a recommendation into a fabricated decision. If
`control-monitor` predates 2026-04-25, the silence is an exclusion and this
recommendation is on firm ground. If it postdates it, the silence proves nothing
and the decision is genuinely Vincent's to make — the recommendation then rests
on the operator-tool argument alone, which is weaker but not empty.

## What was verified

- `luminescent-core.md:4` — the scope list, quoted above. Enumerative, four
  entries, `control-monitor` absent.
- `luminescent-core.md:7` — promotion to ecosystem rulebook, 2026-04-25.
- `#3a82e0` is genuinely present in the installed
  `control-monitor` bundle (`/usr/local/share/control-monitor/frontend/assets/index-bvQOwMl0.js`),
  so the ticket's symptom is real and not a stale observation.
- `senara-solutions/control-monitor` is **not** in this workspace. No pull
  request on `mika` can change a line of its code, which mika#1804 acknowledges
  ("Fix requiert ticket cross-repo OU décision").

## Consequences of this recommendation

**No cross-repo ticket is opened.** AC2 requires one only "si adopte". Filing one
while recommending the opposite would be acting against the conclusion.

**Nothing in `control-monitor` changes.** It keeps its palette, its React 18, its
absence of Tailwind. None of those are defects under this reading; they are the
normal state of a surface outside the rulebook's scope.

**`docs/design/luminescent-core.md` is not modified by this work.** It is
Vincent-owned and updated by direct commit, not through PRs
(`luminescent-core.md:5`). That said, this decision would be more durable as a
line in the rulebook than as a document beside it — see below.

## How to confirm, and what to do if this is overturned

**To confirm:** reply on mika#1804, or add `control-monitor` to the rulebook's
§Scope line as an explicit exclusion. The second is the stronger gesture: it puts
the answer where the next person will look for it, and it removes the
exclusion-versus-omission ambiguity permanently for every surface that comes
later. It is an operator commit, not a PR.

**To overturn** — i.e. to decide that `control-monitor` does adopt the
Luminescent Core — the gesture is:

1. Add `control-monitor` to `luminescent-core.md:4` §Scope (direct commit).
2. File an issue on `senara-solutions/control-monitor` naming the three measured
   gaps: the palette (`#3a82e0` → rulebook §2 `primary` `#ada3ff`), React 18 →
   19, and the absence of Tailwind v4 — which is the prerequisite for consuming
   `@samidarko/ui/theme.css` the way `dashboard/` and, since mika#1804, `site/`
   do.
3. Mark this document superseded, naming that issue.

Step 3 matters: a recommendation left standing beside a decision that reversed it
is how a repository starts holding two answers to one question.
