# Plan — design(skills): intent-layer between keyword-match and required-tools-gate enforcement

**Ticket:** mika#1651
**Type:** design (architect-scope deliberation — no product-code delivery in this pass)
**Priority:** P2-normal
**Branch:** `design/1651/skills-intent-layer-between-keyword`

## Nature of this plan

This ticket is **design-bearing**, not carry-shape. It does not ask for a code
change; it asks mika-arch to **rule on whether the intent-layer is worth
building**, and if so, with which shape and scope. Per Mika Prime's bearing read
(2026-06-29), it is the same architectural class as mika#1575 (unsound-proxy
ruling): redesigning the gate's decomposition routes to architect, not carry.

Accordingly this plan is a **design brief + recommendation + branch-conditional
work definition**, structured so the architect's convergence pass (invoked by
dispatch-lib after this content-only groom exits) can rule on it directly. The
plan's own "Requirements" are the design artifacts the ticket's ACs demand
(a verdict, and — depending on the verdict — either scoped sub-issues or a
documented discipline), not Rust modules.

## Problem statement (restated from the ticket)

The required-tools-gate uses **keyword substring-match as a proxy for
user-intent-to-do-X**. This is an unsound proxy: "the word 'issue' appears in the
message" does not imply "the user wants to fetch an issue." The gate conflates
*mention* with *intent*.

Two-layer decomposition today:

1. **Match layer** — `crates/mika-agent/src/skills/matcher.rs:38`
   (`match_skills`; the substring test is at `matcher.rs:47`):
   `message_lower.contains(kw)` → `MatchedSkill { reason: Keyword }`. Pure
   substring, no word boundaries, no semantics.
2. **Enforcement layer** — `crates/mika-agent/src/agent_loop/mod.rs:4709`
   (`collect_required_tools`): unions `required_tools` across `MatchReason::Keyword`
   skills; the EndTurn required-tools gate (post-condition #3) rejects the turn
   once if those tools were not called.

The unsound proxy lives in the **gap between the layers**: layer 1 says "the
substring appeared," layer 2 trusts that as "the user wants this skill's tools
called." Nothing validates intent.

## State of the world since the ticket was filed (load-bearing context)

Two facts materially change the design calculus and must anchor the architect's
verdict:

1. **Sibling mika#1650 is CLOSED (shipped).** The keyword-tighten fix landed:
   `gh-read-only`'s keyword list was narrowed to multi-word intent phrases, and
   regression tests on both axes were added (`test_gh_read_only_does_not_fire_on_incidental_prose`,
   `test_gh_read_only_fires_on_genuine_fetch_intent` in `matcher.rs`). The
   immediate ~80% relief is already in main.

2. **AC3's deliverable already exists.** The ticket's AC3 asks: *"if ratifying
   tighten-only, document the design discipline in
   `docs/architecture/skill-authoring-guide.md` (or equivalent)."* That document
   already exists as **`docs/architecture/skill-keyword-design-rules.md`** (shipped
   with mika#1650). It codifies the load-bearing rule ("no bare common-English
   bigrams as keywords"), the four correct-authoring patterns (multi-word intent
   phrases, non-colliding domain tokens, literal-number patterns, drop
   discussion-noisy bare topics), the tightening trade-off (false-neg vs
   false-pos), and — critically — a "Note on the matcher engine" that already
   **tracks the substring→word-boundary matcher change as a separate larger
   structural change**.

The design question the architect must rule on is therefore **narrower than the
ticket framed it**: given tighten-shipped + discipline-documented + matcher-swap
already-tracked, is a *new semantic intent-layer* additionally warranted, or is
the existing three-part defense (narrow keywords + authoring discipline +
tracked matcher-swap) the correct permanent shape?

## The three candidate shapes (from the ticket, with cost/risk grounding)

| Shape | Mechanism | Cost | Chief risk |
|-------|-----------|------|-----------|
| **A — LLM intent classifier** | haiku-class call between layers 1&2; input = message + matched skill name/desc; output = intent bool | +1 LLM call per gated turn; adds latency; adds a turn-shape the model must reason about | False-negative: classifier reads genuine intent as discussion → user must re-phrase. Non-determinism in a *gate* (gates should be predictable). Cost on every gated turn, forever. |
| **B — Pattern-based intent** | per-skill `intent_patterns` alongside `keywords`; gate fires only on keyword AND intent-pattern match | per-skill authoring effort; regex/DSL maintenance | Lower false-neg than A, but patterns are just richer keywords — same class of proxy, one level up. Duplicates much of what multi-word keywords already do. |
| **C — Heuristic distance-from-tool-call** | message-structure heuristics (imperative+object vs question vs quoted-log vs meta-discussion) suppress the gate | heuristic rules, edge cases | Brittle; English-shape-specific; hard to reason about why it fired or didn't. |

## Recommendation (design position — for the architect to ratify or overturn)

**Recommend: ratify the existing three-part defense as the permanent shape for
now; do NOT build a standalone semantic intent-layer as a near-term deliverable.
Instead, fold the "intent-layer" ambition into the already-tracked
substring→word-boundary matcher upgrade, and file ONE scoped sub-issue for that
matcher change if it is not already ticketed.**

Reasoning (evidence-grounded, not deference):

1. **Multi-word keyword phrases already ARE the cheapest correct intent-layer.**
   Shape B ("intent patterns") is, in the limit, what
   `skill-keyword-design-rules.md` §"How to write keywords correctly" already
   prescribes — a two-word phrase requires both tokens adjacent, which *is* an
   intent signal, at zero new machinery. mika#1650 proved this works with tests on
   both axes. Building Shape B as a new layer would largely re-implement what the
   keyword list now does, adding a second place to express the same constraint
   (an Orthogonality violation — see `docs/architecture/review-guide.md`).

2. **A classifier in a gate is a category mismatch.** The required-tools gate is a
   deterministic post-condition (one retry, predictable). Shape A injects
   non-determinism and per-turn cost into a mechanism whose value is precisely its
   predictability. The false-negative failure mode (gate silently fails to fire
   when the user *did* want a fetch) is worse than the current false-positive
   (one wasted re-prompt), because a missed gate produces a wrong-but-confident
   answer with no correction signal.

3. **The root cause is substring matching, and it's already tracked.** The true
   defect is that `contains()` has no word boundaries. The documented,
   correctly-scoped fix is the matcher-engine swap to `\b…\b` word-boundary
   matching (`skill-keyword-design-rules.md` §"Note on the matcher engine"). That
   change *raises the floor for every skill at once* — it removes the whole
   `"gh"`→"thought"/"through" collision class structurally, so per-skill keyword
   tightening becomes belt-and-suspenders rather than the sole defense. This is a
   strictly better lever than a new intent-layer bolted between the existing two
   layers, and it subsumes most of what Shapes A/B/C would buy.

4. **Cost discipline.** The ticket is P2-normal, explicitly "not loop-blocking,"
   and the immediate relief already shipped. Adding a per-turn LLM classifier
   (Shape A) or a per-skill pattern DSL (Shape B) is a large, permanent surface
   for a false-positive class that mika#1650 already reduced by ~80% at near-zero
   cost.

This is a **recommendation, not the verdict** — mika-arch owns the AC1 ruling.
If the architect disagrees and elects to build a layer, the "If the architect
elects to FILE" branch below defines that work.

## Requirements (the design deliverables — mapped to the ticket ACs)

The architect's convergence pass produces exactly one of two branch outcomes.

### R1 (AC1) — Design verdict [always required]

mika-arch produces a design verdict, one of:

- **RATIFY** — the existing three-part defense (narrow multi-word keywords +
  `skill-keyword-design-rules.md` discipline + tracked matcher-engine swap) is
  the correct permanent shape; no standalone semantic intent-layer is filed as a
  near-term deliverable. (This is the recommended branch.)
- **FILE** — a semantic intent-layer IS warranted; select shape A/B/C/other,
  define scope, ACs, and rollout.

### R2 — Branch-conditional deliverable

**If RATIFY (recommended):**

- R2a (AC3) — Confirm `docs/architecture/skill-keyword-design-rules.md` already
  satisfies AC3's "document the design discipline" requirement. If any gap versus
  AC3's intent exists (e.g., the doc should also state explicitly that a semantic
  intent-layer was considered and deliberately declined, with the reasoning), add
  that short "Design decision: intent-layer declined (mika#1651)" note to the doc
  so future authors don't re-litigate. This is a **docs-only** change, in-scope
  for a content pass.
- R2b — Ensure the substring→word-boundary matcher upgrade is captured as a
  concrete tracked ticket. **Satisfied: filed as mika#1878 —
  *"feat(skills): word-boundary keyword matching in `match_skills` to
  structurally retire the substring-collision class"* (OPEN).** This is the real
  long-term correctness lever the ticket gestures at; the RATIFY branch folds the
  intent-layer ambition into that matcher-swap rather than building a standalone
  semantic layer.

**If FILE:**

- R2c (AC2) — Scope-bind the work into one or two concrete sub-issues, each with:
  selected shape, ACs, the false-negative-cost mitigation, and the rollout
  pattern. Confirm composability with mika#1650 (narrower keywords reduce the new
  layer's load) and pin the sequence: tighten-first (done) → layer-second.
- R2d — Document, in `skill-keyword-design-rules.md`, how skill authors interact
  with the new layer (what they must declare, e.g. `intent_patterns` for Shape B).

### R3 — No product-code change in this pass

This grooming pass and its parent ticket deliver **design artifacts only**
(a verdict + issue(s)/doc-note). Any implementation (matcher swap, or an
intent-layer if FILE'd) ships under the sub-issue(s) R2b/R2c produce, through the
normal `/mika` pipeline. This plan does not touch `matcher.rs` or `agent_loop`.

## Verification contract

Because the deliverable is design artifacts, verification is contract-shaped, not
test-shaped:

- **V1** — AC1 verdict is present and unambiguous (RATIFY or FILE), with reasoning
  that engages the mika#1650-shipped + doc-exists + matcher-swap-tracked state of
  the world (not the pre-mika#1650 framing).
- **V2** — If RATIFY: `skill-keyword-design-rules.md` covers AC3's discipline
  requirement (verified by inspection — the load-bearing rule, the four authoring
  patterns, and the tightening trade-off are all present today), and the
  matcher-swap ticket reference exists (cited or newly filed).
- **V3** — If FILE: each new sub-issue has shape + ACs + false-negative mitigation
  + rollout + explicit composability-with-mika#1650 note.
- **V4** — No diff to `crates/mika-agent/src/skills/matcher.rs` or
  `crates/mika-agent/src/agent_loop/mod.rs` in this ticket's PR (product code is
  out of scope; it lands under R2b/R2c sub-issues).
- **V5** — `cargo test -p mika-agent` still green (this plan changes no code; the
  existing mika#1650 regression tests continue to pass — they are the standing
  proof that the ratified keyword-discipline holds on both axes).

## Definition of Done

- [ ] mika-arch has issued an AC1 verdict (RATIFY or FILE) grounded in the
      post-mika#1650 state of the world.
- [ ] RATIFY branch: AC3 discipline confirmed in `skill-keyword-design-rules.md`
      (with an optional short "intent-layer declined" decision note added); the
      word-boundary matcher-swap is tracked as a cited or newly-filed sub-issue
      (**filed: mika#1878**).
- [ ] FILE branch: AC2 sub-issue(s) filed with shape + scope + ACs + rollout +
      mika#1650 composability + sequence.
- [ ] No product-code change in this ticket's PR; implementation (if any) is
      routed to the sub-issue(s).
- [ ] `cargo test -p mika-agent` green (unchanged code path).

## Acceptance criteria

Transcribed from the mika#1651 issue body ("Acceptance criteria (for the
architect's design pass)"), which frames them for the architect's deliberation:

- **AC1** — mika-arch produces a design verdict: file the layer (with shape
  selected + scope defined), OR ratify keyword-tighten + design discipline as
  sufficient with reasoning.
- **AC2** — If filing: scope-bind the work into a concrete sub-issue or two, with
  ACs and shape selected.
- **AC3** — If ratifying tighten-only: document the design discipline (no bare
  common-bigram keywords, multi-word intent phrases) in
  `docs/architecture/skill-authoring-guide.md` (or equivalent) so future skills
  don't reintroduce the pattern. **Status: already satisfied by
  `docs/architecture/skill-keyword-design-rules.md` (shipped with mika#1650);**
  the RATIFY branch confirms this and optionally adds an intent-layer-declined
  decision note.

## References

- Ticket: mika#1651 (design, P2-normal, agent-core)
- Sibling: mika#1650 — keyword-tighten (**CLOSED / shipped** — the cheap 80% fix)
- Long-term correctness lever (RATIFY R2b deliverable): mika#1878 — word-boundary
  keyword matching in `match_skills` (**OPEN**)
- Existing discipline doc: `docs/architecture/skill-keyword-design-rules.md`
- Same architectural class: mika#1575 (unsound-proxy ruling)
- Match function: `crates/mika-agent/src/skills/matcher.rs:38` (`match_skills`;
  substring test at `matcher.rs:47`)
- Required-tools collection: `crates/mika-agent/src/agent_loop/mod.rs:4709`
  (`collect_required_tools`)
- `MatchReason` enum (#463 conditioning): `crates/mika-agent/src/skills/matcher.rs:10`
- Required-tools gate (post-condition #3): `crates/mika-agent/CLAUDE.md`
  §"Post-Conditions (EndTurn Chain)"
- Orthogonality / DRY grounding for the recommendation:
  `docs/architecture/review-guide.md`
- Mika Prime bearing read: 2026-06-29 ~13:30 UTC
