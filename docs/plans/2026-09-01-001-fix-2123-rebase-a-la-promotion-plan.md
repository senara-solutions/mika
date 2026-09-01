---
issue: mika#2123
title: Rebase at Promotion, Not at Dispatch - Plan
type: fix
scope_repo: mika
priority: p1-important
date: 2026-09-01
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# Rebase at Promotion, Not at Dispatch - Plan

## Goal Capsule

**Objective.** A ticket that reaches the autonomous loop can be worked on. Today a
ticket can be promoted, dispatched, and killed before the pilot starts, and the
operator learns nothing until the dispatch is already spent.

**Means.** Move the staleness check and the refresh attempt from dispatch time to
promotion time, and hand a branch that cannot be refreshed back to the operator
instead of promoting it (KTD1).

**Authority hierarchy.** Issue ACs > this plan > implementer judgment.

**Stop conditions.**
- Stop if the change would resolve content conflicts automatically. The goal is to
  **refuse cleanly**, never to guess a merge.
- Stop if the guard would refuse branches that rebase fine. A gate that blocks
  healthy work has not fixed the loop, it has closed it (AC3).

**Execution profile.** Two surfaces, one repo: `auto_pull.rs` (promotion) and
`dispatch-lib.sh` (dispatch). Sequential.

**Tail ownership.** PR on `mika`, routed to mika-qa.

## Product Contract

### Summary

Measure how far a branch is behind `origin/main` **before** promoting it, try to
refresh it there, and if that fails, apply `operator-review` and do not promote.
Dispatch stops being the place where staleness is discovered.

### Problem Frame

On 2026-08-31, four callouts were corrected so their tickets became visible to the
feeder. The loop woke: 16 `mika-dev` tasks in 41 minutes after 15 hours at zero.
**Every dispatch died at the same place** — seven `STATUS=REBASE_CONFLICT`.

Measured against `origin/main = 8de0100b`:

| ticket | retard | avance |
|---|---|---|
| #1680 | **179** | 2 |
| #1699 | **172** | — |
| #1403 | **119** | — |
| #1651 | **108** | — |

These branches carry `wip(...)` salvage commits left by earlier dead pilots. The
rebase at `skills/bundled/_shared/dispatch-lib.sh:~1540` fails on files heavily
rewritten since, and the dispatch `exit 1`s **before claude-pilot is launched**.

**It is not new, it was masked.** `STATUS=REBASE_CONFLICT` in `tasks.result`, by
day: 47 (08-26), 37 (08-29), 32 (08-27), 20 (08-28), 18 (08-30), 11 (08-31). The
feeder starvation described in mika#2120 kept most dispatches from happening;
those that happened died here.

**The contrast that shows the way out.** Two tickets dispatched by hand the same
evening (#1947, #1949) had branches at **retard 0** and each advanced by four
commits. An interactive session rebases *before* it starts; the autonomous path
attempts a late rebase on a weeks-old branch and gives up.

### Key Decisions

- **Refuse at promotion, not at dispatch.** A dispatch is a scarce resource; a
  promotion is free to withdraw. Governs R1, R2, R3.
- **Never auto-resolve content conflicts.** Refusing cleanly is the deliverable.
  Governs R4.

### Requirements

- R1. A branch's distance from `origin/main` is measured **before** the ticket is
  promoted to `ready`, and recorded in a structured audit field — not as a
  substring in a log message.
- R2. A refresh is attempted at promotion. On success the ticket is promoted as
  today.
- R3. On failure the ticket is **not promoted**. `operator-review` is applied, an
  audit event names the failure (`rebase_conflict` plus the conflicted files),
  and **no dispatch is consumed**.
- R4. No content conflict is ever resolved automatically.
- R5. A branch that is up to date, or behind but cleanly rebasable, is promoted
  exactly as today. The guard adds a refusal path, it does not narrow the
  existing accept path.
- R6. The disposition of `wip(...)` salvage commits on a stale branch is decided
  and written: a branch carrying partial work from a dead pilot is treated
  differently from one carrying only a plan.

### Acceptance Examples

- AE1. A branch 179 commits behind with conflicts on `agent_loop/mod.rs` is
  **not** promoted; `operator-review` is applied; an audit row names the
  conflicted files; the ready pool is unchanged. Covers R1, R3.
- AE2. A branch 40 commits behind that rebases cleanly is refreshed and promoted.
  Covers R2, R5.
- AE3. A branch at distance 0 is promoted with no refresh attempted and no extra
  round trip. Covers R5.

### Sources

- Issue `senara-solutions/mika#2123`
- `skills/bundled/_shared/dispatch-lib.sh:~1527` — `BEHIND` computed at dispatch;
  `:~1540` — rebase attempt; `:~1554` — `STATUS=REBASE_CONFLICT` and `exit 1`
- `crates/mika-agent/src/auto_pull.rs:1327` — `gh_apply_label(.., "ready")`, the
  promotion site
- `crates/mika-agent/src/auto_pull.rs:966` — `gh_apply_label(.., "operator-review")`,
  the existing hand-back gesture; `:783` — `gh_remove_label`
- `crates/mika-agent/src/auto_pull.rs:349` — `operator-review` is already a
  structural feeder exclusion, so applying it is sufficient to withdraw the ticket
- Adjacent, not this ticket: mika#2120, mika#2121, mika#2125, mika#2108

## Planning Contract

### Key Technical Decisions

- KTD1. **The gate lives in `auto_pull` and withdraws a ticket with a label.**
  ~~It reuses the gesture that exists: `operator-review`, already a structural
  exclusion in `is_feeder_excluded` and already applied at `:966`.~~
  **Corrected during implementation, 2026-09-01 — see Verified constraints C5.**
  That gesture has never worked: `operator-review` is declared nowhere and does
  not exist. The gate refuses with `operator-gated`, which does, and
  `is_feeder_excluded` learns it so the refusal persists. Governs R3.
- KTD2. **The promotion-time measurement is an API call, not a git call.**
  `auto_pull` has no checkout (see Verified constraints), so it cannot run
  `rev-list`. It calls `gh api .../compare/main...<branch>` and reads `behind_by`,
  `ahead_by`, `status`. This is the same quantity `dispatch-lib.sh` computes
  locally, obtained by the only means available at that site.
- KTD2b. **The gate cannot predict a conflict, and does not pretend to.** Only a
  real rebase decides whether a branch merges, and `auto_pull` cannot run one.
  The gate therefore answers a **policy** question, not a technical one: *is this
  branch old enough that a human should look before a dispatch is spent on it?*
  Stating it that way is what keeps the threshold honest — a number chosen to
  predict conflicts would be a number pretending to knowledge nobody has.
- KTD2c. **The threshold is provisional by construction, and the plan says so.**
  Measured evidence: `behind_by = 0` promoted and produced a mergeable PR; 109,
  120 and 180 all died at the rebase. Any cut in `(0, 109]` fits that evidence,
  which means the evidence does **not** determine one. Default: **50**, declared
  provisional, configurable by env var. Every promotion decision logs
  `behind_by`, `ahead_by` and `status` **whether it promotes or refuses**, so the
  threshold becomes tunable from a real distribution instead of from this
  paragraph. A first revision is expected once fifty decisions are on record.
- KTD3. **The dispatch-time rebase is kept, and it is now the ONLY rebase.**
  Since `auto_pull` has no checkout, it never refreshes anything — it only
  measures and decides. Every branch that *is* promoted still gets its real rebase
  at dispatch, exactly as today. The first-pass review read this unit as future
  dead code; the verified constraint inverts that reading. It is load-bearing, not
  a fallback, and removing it would leave the loop with no rebase at all.
- KTD4. **The dispatch-time rebase keeps its abort discipline unchanged.**
  `dispatch-lib.sh:1553` already does `rebase --abort` on any failure, leaving the
  worktree as found. This plan does not touch that behaviour; it is named here so
  a reader does not conclude the abort moved with the measurement.

### Verified constraints (measured 2026-09-01, not assumed)

- **`auto_pull` can spawn subprocesses.** It already does, six times, via
  `tokio::process::Command::new("gh")` (`:642`, `:707`, `:750`, `:784`, `:827`,
  `:1052`).
- **`auto_pull` has no local checkout.** Zero occurrences of any repo-path
  variable (`repo_path`, `workspace_root`, `REPO_PATH`, `MIKA_WORKSPACE`,
  `sub_repo_dir`), against 28 for `DEFAULT_REPO` — the GitHub slug. It is a **pure
  API client**. `gh` needs no working tree; `git rebase` does.
- **Therefore `auto_pull` cannot rebase, and this plan does not ask it to.** The
  measurement moves to promotion; the rebase stays at dispatch.
- **The API supplies the measurement without a checkout.**
  `gh api repos/<repo>/compare/main...<branch>` returns `behind_by`, `ahead_by`
  and `status`. Measured on the real branches:

| branche | `behind_by` | `ahead_by` | `status` | issue de dispatch observée |
|---|---|---|---|---|
| `fix/1680/…` | 180 | 2 | `diverged` | `REBASE_CONFLICT` |
| `feat/1403/…` | 120 | 3 | `diverged` | `REBASE_CONFLICT` |
| `design/1651/…` | 109 | 6 | `diverged` | `REBASE_CONFLICT` |
| `feat/1949/…` | **0** | 5 | `ahead` | PR ouverte, `MERGEABLE` |

- **C5 (added 2026-09-01, during implementation). The hand-back label does not
  exist, and the existing hand-back has never fired.** Measured, three ways:
  `gh label list --repo senara-solutions/mika` does not list `operator-review`;
  `.github/labels.yml` does not declare it — with the control that it *does*
  declare `ready` (`:102`) and `operator-gated` (`:106`), so the file is the
  source of truth; and `server.log` carries **48** occurrences of

  ```text
  gh issue edit --add-label failed for #2117:
    'operator-review' not found
  ```

  `blocked`, the module's other exclusion label, is equally absent. The
  consequence is worse than a failed label: `abandon_stuck_ready` removes
  `ready` only *after* the label lands, so when the label fails the ticket keeps
  `ready` and stays in the pool permanently. The mika#2020 arrest is inert.

  **What this changes for U2.** Refusing with `operator-review` would have
  reproduced that failure exactly — refusing to promote while marking nothing,
  leaving the ticket promotable on the next tick. The gate therefore (a) refuses
  with `operator-gated`, whose declared description is already the state a
  refusal creates ("Groomed work requiring operator-host-time… No ready label"),
  (b) teaches `is_feeder_excluded` that label so the refusal persists, (c)
  **checks** the apply result and escalates a failure to `ERROR` under its own
  event key `auto_pull_refusal_marker_unavailable` instead of one WARN among
  thousands, and (d) carries a test that reads `.github/labels.yml` and fails if
  the gate's label is not declared there.

  **Explicitly NOT in this lot:** declaring `operator-review`/`blocked`, and
  repairing `abandon_stuck_ready`'s inert arrest. Distinct defect, distinct
  blast radius (it strands `ready` forever on disjuncted tickets — #2117, #1651,
  #1403 observed). Filed separately rather than fixed silently inside a
  rebase-gate change.

### Sequencing

U1 (measure) before U2 (refuse), because U2 refuses on U1's number. U3 (wip
disposition) is independent and can land in any order. U4 last: it verifies the
whole path against the real branches.

## Implementation Units

### U1. Measure staleness at promotion

**Goal.** The number exists before the decision that needs it.

**Requirements.** R1.

**Files.** `crates/mika-agent/src/auto_pull.rs`.

**Approach.** Before applying `ready` at `:1327`, call
`gh api repos/<repo>/compare/main...<branch>` in the shape `gh_apply_label`
already uses (`tokio::process::Command::new("gh")`, `GH_TOKEN` in env, piped
output, `kill_on_drop`). Read `behind_by`, `ahead_by`, `status`.

Write the three values to the audit event as **structured fields**, not inside a
message string — a number embedded in prose cannot be queried later, and KTD2c
depends on querying these. Log them on **every** decision, promote or refuse.

A `404` from the compare endpoint (branch absent from origin) is **not** a
distance of zero: it means the branch does not exist, and the ticket is not
promotable. Treat it as its own outcome and name it.

**Test scenarios.** Distance 0, distance N, and a branch that does not exist on
origin (must not panic and must not promote).

**Verification.** `cargo test -p mika-agent auto_pull`.

### U2. Refuse instead of promoting

**Goal.** A branch that cannot be refreshed costs a label, not a dispatch.

**Requirements.** R2, R3, R4, R5.

**Files.** `crates/mika-agent/src/auto_pull.rs`.

**Approach.** No rebase happens here (KTD3). The decision is taken on the measured
values:

| condition | action |
|---|---|
| `status == "ahead"` (`behind_by == 0`) | promote, no further check |
| `ahead_by > 1` and `behind_by > 0` | **refuse** — `operator-review` (U3) |
| `behind_by > threshold` | **refuse** — `operator-review` |
| otherwise | promote; the real rebase happens at dispatch (KTD3) |

A refusal applies `operator-review` via the existing `gh_apply_label`, does **not**
apply `ready`, and writes an audit event naming the reason and the three measured
values. No dispatch is consumed.

**Test scenarios.** AE1, AE2, AE3.
**Negative control (AC3 of the issue):** a branch behind but cleanly rebasable
**must** be promoted. A test asserts it, and it must go red if the refusal branch
is made unconditional. Demonstrate that and record it in the PR body — a gate that
refuses everything looks exactly like a gate that works, from the outside.

**Verification.** `cargo test -p mika-agent auto_pull`.

### U3. Decide the `wip(...)` disposition

**Goal.** The aggravating factor is named and handled rather than carried.

**Requirements.** R6.

**Files.** `crates/mika-agent/src/auto_pull.rs`; a note in the module docstring.

**Approach — the decision is taken here, not deferred.** `ahead_by` from the same
API call already separates the two populations: a branch carrying only its plan
commit has `ahead_by == 1`; every branch that died at the rebase carried more
(2, 3 and 6 in the measured cases).

**Decision: a branch with `ahead_by > 1` AND `behind_by > 0` is never
auto-promoted.** It goes to `operator-review` regardless of the threshold.

Rationale, and it is not the obvious one. It is *not* that such branches are more
likely to conflict — that would be predicting a conflict, which KTD2b forbids. It
is that a stale branch carrying unpushed partial work from a dead pilot has
**two** possible resolutions — rebase the work, or abandon it — and choosing
between them is a judgement about *work*, not about git. The loop should not make
that choice silently by rebasing over it.

A branch with `ahead_by == 1` carries only a plan; refreshing it destroys nothing,
so it is governed by the threshold alone.

**Cost, stated:** this refuses some branches that would have rebased fine. That is
accepted — the alternative is an autonomous loop deciding the fate of a dead
pilot's partial work without anyone reading it.

**Test scenarios.** A branch with only a plan commit, and a branch with a plan
commit plus a `wip(...)` commit, take the decided paths.

**Verification.** `cargo test -p mika-agent auto_pull`.

### U4. Replay against the real branches

**Goal.** The fix is verified against the thing that broke, not against a mock.

**Requirements.** R1, R3, R5.

**Files.** `crates/mika-agent/tests/` — fixtures frozen from the 2026-08-31 state.

**Approach.** Freeze the measured cases as fixtures: `#1680` (179 behind,
conflicts on `agent_loop/mod.rs` and `evidence/guards.rs`), and one branch that
rebases cleanly. Assert the first is refused and the second promoted.

**Test scenarios.** The two fixtures. **Non-vacuity:** the refusal test must go red
if the distance threshold is removed.

**Verification.** `cargo test -p mika-agent auto_pull`.

## Observability (first-pass F3, kept as a live question)

The review asked what happens if the dispatch-time rebase becomes unreachable. It
will not, for the reason in KTD3 — but the question is worth an answer that does
not rely on my reading being right. Every dispatch that reaches the rebase logs
whether it ran and whether it succeeded, with the distance. If that count reaches
zero over a sustained window while promotions continue, one of two things is true:
either every promoted branch is already current, or the promotion gate has become
too strict. **The counter distinguishes them** — a gate refusing everything and a
world with nothing to refresh look identical from the dispatch side without it.

## Verification Contract

- `cargo test -p mika-agent auto_pull` — including U2's negative control and U4's
  non-vacuity proof, both demonstrated in the PR body.
- `cargo clippy --all-targets -- -D warnings`
- `shellcheck` clean if `dispatch-lib.sh` is touched at all.
- `bash skills/bundled/_shared/test-dispatch-lib.sh` — the dispatch-time rebase is
  kept (KTD3) and must not regress.
- **Post-deploy, operator-driven:** the rate of `STATUS=REBASE_CONFLICT`
  **per dispatch**, before and after. A raw count would fall if the loop simply
  ran less; that is not an improvement.

## Acceptance criteria

Transcribed from the issue, with the unit that satisfies each.

- [x] **AC1** — Le retard d'une branche sur `origin/main` est mesuré et journalisé
  **avant** la promotion, dans un champ structuré (pas un substring de message).
  → **U1**.
- [x] **AC2** — Une tentative de rafraîchissement a lieu à la promotion ; si elle
  échoue, le ticket n'est pas promu, l'échec est nommé, une trace d'audit est
  écrite, aucun dispatch n'est consommé. → **U2**, avec une **rectification que le
  ticket ne pouvait pas connaître** : `auto_pull` n'a pas de checkout, donc aucune
  tentative de rafraîchissement ne peut avoir lieu à la promotion. Ce que l'AC
  protège — ne pas brûler un dispatch sur une branche irrécupérable — est tenu par
  une **décision sur mesure** plutôt que par une tentative. Le reste de l'AC
  (pas de promotion, échec nommé, audit, dispatch préservé) est satisfait tel
  quel. La rectification est signalée ici plutôt que l'AC réputé satisfait en
  silence.
- [x] **AC3** — Contrôle négatif : une branche à retard 0 et une branche en retard
  mais rebasable doivent **toutes deux passer**. → **U2**, test nommé, avec la
  démonstration qu'il vire au rouge si la branche de refus est rendue
  inconditionnelle.
- [x] **AC4** — Preuve de non-vacuité : rejouer la promotion de #1680 doit produire un
  refus **nommé**, pas un dispatch. Fixture figée sur l'état du 2026-08-31.
  → **U4**.
- [x] **AC5** — Le sort des commits `wip(...)` est décidé et écrit ; une branche qui
  en porte est traitée différemment d'une branche qui ne porte qu'un plan, et la
  règle est explicite. → **U3**, tranché : `ahead_by > 1` et `behind_by > 0` →
  jamais auto-promue.
- [x] **AC6** — Mesure post-correctif du taux `STATUS=REBASE_CONFLICT` **par
  dispatch**, jamais en compte brut. → Verification Contract.

## Definition of Done

**Global.**
- R1–R6 satisfied, each traced to a landed unit.
- No dispatch is consumed by a branch that could not be refreshed.
- The `wip(...)` decision is written with its reason, not merely implemented.
- The dispatch-time rebase still exists and still names the distance.
- No content conflict is ever auto-resolved — verified by the absence of any
  `-X ours`, `-X theirs`, or `--strategy` flag in the diff.
- U2's negative control and U4's non-vacuity proof demonstrated in the PR body.

**Per unit.** Each unit's Verification passes.
