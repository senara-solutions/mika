# `auto_pull` compare fixtures — mika#2123, extended by mika#2140

Real payloads from `GET /repos/senara-solutions/mika/compare/main...<branch>`.
`commits`, `base_commit` and `merge_base_commit` are dropped — they are
megabytes and the gate reads none of them. Of `files`, only `filename` is kept,
for the same reason (the `patch` bodies are megabytes); it is the only field the
gate reads.

## Two capture dates, and why that is not an inconsistency

The counters (`behind_by`, `ahead_by`, `status`) of the four original fixtures
were captured **2026-09-01**. The `files` lists were added **2026-09-04**
(mika#2140). Mixing the dates is safe, and the reason is verifiable rather than
asserted: `compare/main...branch` is a **three-dot** diff, so `files` is computed
against the **merge base**, which does not move when `main` advances. Only
`behind_by` moves.

Measured control: between 2026-09-01 and 2026-09-04, `fix/1680/…` went from 180
to 202 behind while `ahead_by` stayed at 2 and the file list stayed identical.
The fixtures keep the 2026-09-01 counters, which is what their filenames say.

The three fixtures added by mika#2140 (`2118-`, `2120-`, `1727-`) were captured
whole on **2026-09-04**; their filenames carry that day's counters.

## The fixtures

| fixture | branch | behind | ahead | status | files | measured `git rebase origin/main` |
|---|---|---|---|---|---|---|
| `1680-diverged-180-behind-2-ahead.json` | `fix/1680/mika-dev-tui-broken-glyph-rendering-in` | 180 | 2 | `diverged` | 4 × `crates/**` + 1 plan | **CONFLICT** — `crates/mika-agent/src/agent_loop/mod.rs`, `crates/mika-agent/src/evidence/guards.rs` |
| `1959-diverged-75-behind-1-ahead.json` | `feat/1959/mcp-manifest-data-grade-field-l4-forward` | 75 | 1 | `diverged` | 1 plan | OK |
| `2048-diverged-17-behind-1-ahead.json` | `ci/2048-re-enable-release-please` | 17 | 1 | `diverged` | 3 config, **no plan** | OK |
| `2123-ahead-0-behind-1-ahead.json` | `fix/2123/dispatch-lib-le-rebase-est-tent-au` | 0 | 1 | `ahead` | **absent** (see below) | n/a (nothing to rebase) |
| `2118-diverged-13-behind-3-ahead.json` | `fix/2118/skills-cloud-sur-un-tenant-cloud-google` | 13 | 3 | `diverged` | 1 plan | OK (rebased by hand 2026-09-02, no conflict) |
| `2120-diverged-13-behind-2-ahead.json` | `fix/2120/auto-pull-is-groomed-exige-docs-plans` | 13 | 2 | `diverged` | 1 plan | OK (rebased by hand 2026-09-02, no conflict) |
| `1727-diverged-190-behind-3-ahead.json` | `feat/1727/tui-tui-as-thin-http-client-of-mika` | 190 | 3 | `diverged` | 1 plan + 1 doc outside `docs/plans/` | not re-measured (refusal is overdetermined by distance) |

## The rows that carry an argument

**`#1680`** reproduces the issue report verbatim — same two conflicted files,
eleven days later. It is the frozen body behind mika#2140 AC2/AC3: a stale branch
carrying real code, refused before and after the predicate change.

**`#1959`** is the honest one: **the gate refuses a branch that would have
rebased cleanly.** That is the declared cost of a policy threshold that cannot
predict a conflict (KTD2b), not a defect. It is also the only fixture the
distance rule refuses *alone*, which is what makes that non-vacuity proof
possible.

**`#2048`** is the one verdict mika#2140 flips in the dangerous direction:
promoted before, refused after, with the `docs/plans/` prefix as the *sole*
cause (17 behind is well inside the threshold). It is kept, and asserted in the
open, rather than deleted or widened away — see the test of the same name for
the measurement that bounds it (this branch cannot reach the gate: Phase 0 and
Phase 1 filter on `is_groomed`, which requires a plan callout; Phase 2 could,
but zero of the 18 open tickets with a branch callout on 2026-09-04 point at a
plan-less branch; and #2048 itself is closed with no grooming callout).

**`#2118` / `#2120`** are the two bodies mika#2140 was filed on: three and two
grooming commits, nothing but their own plan file, both labelled `operator-gated`
by the old `ahead_by > 1` predicate and held out of the `ready` pool for days.
They are also the negative control the `#2048` row used to provide — "behind,
but promoted" — now carried by branches that are actually grooming branches.

**`#2123`** deliberately has **no** `files` key and cannot get one: its branch was
merged and deleted from origin (`404`, verified 2026-09-04). It does not need
one — `behind_by == 0` short-circuits before the salvage rule reads the list —
and it doubles as the integration-level control that a payload without `files`
parses to `changed_files: None` rather than to an error or an empty list.

**`#1727`** is the living boundary case: its one non-plan file is an *audit
document*, not code, which the narrow prefix classifies as "not grooming" —
literally true, semantically arguable. Its refusal is **overdetermined** (190
behind, threshold 50), so the prefix decides the reason and never the outcome.
The test asserts that overdetermination on purpose: the day a boundary case
appears where the prefix is the sole cause of a refusal that would otherwise
have promoted, the assertion fails and the prefix question reopens instead of
answering itself in silence.
