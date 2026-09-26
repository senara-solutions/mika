---
issue: senara-solutions/mika#2034
type: fix
scope: loop-substrate
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
target_files:
  - skills/bundled/_shared/dispatch-lib.sh
  - skills/bundled/_shared/test-dispatch-lib.sh
---

# Plan — mika#2034: the dev-groom dispatch gate attests "already groomed" from the ticket's own claim

## Goal Capsule

**Objective.** A ticket is refused grooming only on evidence that a plan *for that ticket*
is on its dispatch branch — never on the strength of a claim the ticket makes about itself.

**Means.** Bind the dispatch gate's candidate to the target issue using the refutation
instrument that already exists in the same file (`_plan_header_refutes_issue`, built by
mika#2038), and stop asserting "committed on the dispatch branch" for a blob the branch
merely inherited from `main`.

**Authority hierarchy.** The design below is settled (see § Settled decisions). Evidence in
§ Why is measured, not recalled; do not re-derive it. Where this plan and mika#2034's own
stated method ("wait for a recurrence") conflict, this plan carries the ticket's *intent*
and overturns its *method*, with the falsification recorded in § Why.

**Stop conditions.** Bash only, no Rust. Every existing assertion in
`test-dispatch-lib.sh` passes unmodified. No change to `_iterate_groom_loop`'s convergence
logic — that is what this fix makes *reachable*, not what it changes.

## Why

mika#2034's scope says: wait for a recurrence carrying PR#2028's new diagnostic, then act
on what it names. **That recurrence is structurally impossible, and the ticket's premise is
therefore falsified.** Measured 2026-08-30:

| Fact | Measurement |
|---|---|
| PR#2028 merged | `2026-08-29T00:18:50Z` (commit `b84fdbc8`) |
| dev-groom dispatches since | 8 |
| …that reached `_iterate_groom_loop` | **0** |
| …that ended `auto_skipped` / `already_groomed` | **8** |
| New diagnostic deployed? | yes — `_groom_warn` ×17 in `~/.mika/skills/_shared/dispatch-lib.sh`; binary mtime `2026-08-30 09:04` |

The new diagnostic is live and correct. It never gets the chance to run, because a gate
upstream of it refuses the dispatch first.

**The gate.** `dispatch-lib.sh:1213` calls `_committed_plan_on_branch`, introduced by
mika#2012 and **not in PR#2028's scope**. It reads the plan path *out of the issue body's
own callout*, then asks only whether `git cat-file -e <gate_ref>:<path>` succeeds. Every
dispatch branch descends from `main`, and `main` carries 769 plan files — so any valid
plan path resolves, whatever ticket it belongs to. The attestation is produced by the very
claim it is supposed to check, against a tree that cannot refute it.

**Two live false positives, both currently stranded:**

| Ticket | Plan the gate cited | What that plan's own header says |
|---|---|---|
| mika#1887 | `docs/plans/2026-08-21-002-fix-1933-reader-completed-section-avancement-plan.md` | `issue: senara-solutions/mika#1933` |
| mika#2026 | `docs/plans/2026-04-11-003-chore-deps-bump-rand-clear-rustsec-2026-0097-plan.md` | `**Issue:** #539` |

Both files are on `origin/main`. They are **inherited** by those branches, not committed to
them. Each ticket is refused grooming permanently: the gate fires, the body callout stays
wrong, and nothing in the loop can correct it.

mika#1887's false callout was already known. What this plan adds is its **consumer**: the
callout is not cosmetic, it is load-bearing, and the gate turns it into a permanent refusal.

**The instrument already exists.** mika#2038 built `_plan_header_refutes_issue` /
`_plan_header_claimed_issues` in this same file — and its doc comment names *this exact
rand-bump plan* as its founding incident. `_find_issue_plan` uses it. `_committed_plan_on_branch`
does not.

**Why the suite is green while the defect is live.** The fixture repo's `main` carries no
plan files, so the fixture cannot express "inherited from main" — the production shape. The
suite tests the gate against a world in which the bug cannot occur.

## Scope overlap — verified, stated so it is not re-corrected

- **mika#2028** — same defect *class* (a guard resolving a plan path against a tree carrying
  769 plans; #2028's fourth false statement was `plan already committed from prior run`).
  **Different site, still live.** #2028 fixed the *reporting* guard in the failure callback;
  the *dispatch* gate was never in its scope. Not a re-correction, and no closure for
  overlap is proposed — #2028 is merged and its fix stands.
- **mika#2037** — same discipline (a rendered verdict that attests a review that did not
  happen), different mechanism, already owned by its own ticket and worktree. **Out of scope.**
- **mika#1723 / the 2026-07-04 convergence class** — untouched by this plan. Hypotheses A
  (session continuity) and B (paraphrase tolerance) stay unreproduced. This fix removes the
  reason no evidence about them can be collected.

## Product Contract

### Requirements

- **R1.** The gate refuses a dev-groom dispatch only when the plan it names is not refuted
  as belonging to a different issue.
- **R2.** A refuted candidate lets grooming proceed, and says why in a greppable operator
  diagnostic naming the issue the plan claims.
- **R3.** The gate's messages never assert that a blob was committed to the dispatch branch
  when the branch inherited it unchanged from `main`.
- **R4.** No ticket that is genuinely groomed starts being re-groomed. The false-negative
  direction (stranding) stays worse than the false-positive direction (one redundant groom),
  exactly as `test-dispatch-lib.sh:927-933` states.
- **R5.** All four existing call sites and all five existing fixture cases keep working
  without modification.

### Acceptance examples

- **AE1.** Body claims `docs/plans/…-fix-1933-…-plan.md`; the file resolves on the branch;
  its header reads `issue: senara-solutions/mika#1933`; target issue is 1887 →
  gate does **not** fire, grooming proceeds, stderr names issue 1933.
- **AE2.** Body claims a plan whose header carries no issue marker at all → gate fires as
  today (silence is not evidence; 95 of 745 plans carry no marker).
- **AE3.** Body claims a plan whose header claims the target issue → gate fires as today.
- **AE4.** The claimed path resolves on the branch at the same blob as on `origin/main` →
  gate still decides by R1, but its diagnostic says the plan is inherited from `main`, not
  committed to the branch.

## Planning Contract

### Settled decisions (KTDs)

- **KTD1 — Check A is the gate decision; Check B is diagnostic honesty only.**
  Issue-binding refutation decides whether the gate fires. Inheritance from `main` does
  **not** block the gate. A legitimately groomed ticket whose PR merged also has its plan on
  `main`; making inheritance blocking would re-strand exactly the tickets the gate exists to
  protect. Inheritance changes only what the message *claims*.
- **KTD2 — Optional 5th argument, not a signature change.**
  `_committed_plan_on_branch` gains `$5` defaulting to `$ISSUE_NUM`. The four call sites and
  five fixture cases stay untouched. A required parameter would break them all.
- **KTD3 — Refute, never confirm.** Reuse `_plan_header_refutes_issue` as-is, including its
  fail-open contract: unreadable input and a header claiming nothing both mean "not refuted."
  Requiring a positive header match would make the 95 marker-less plans undiscoverable and
  reopen the false-negative class bound by mika#1421, #1602, #1617.
- **KTD4 — Materialize the blob to a tempfile.** The helpers take a readable path; the
  candidate lives in a git object on `refs/dispatch-gate/<branch>`. Extract with `git show`
  to a `mktemp` file, remove it on every exit path.

### Technical design

`_committed_plan_on_branch` (`dispatch-lib.sh:1039`), after a candidate resolves:

```
for candidate in "$plan_path" "${plan_path#"${repo}/"}"; do
    git cat-file -e "${gate_ref}:${candidate}" || continue

    # Check A — issue binding (the gate decision).
    tmp=$(mktemp …); git show "${gate_ref}:${candidate}" > "$tmp"
    if _plan_header_refutes_issue "$tmp" "$issue_num"; then
        claimed=$(_plan_header_claimed_issues "$tmp" | tr '\n' ' ')
        echo "dispatch_gate_groom_plan_refuted: … plan=${candidate} claims=${claimed} target=${issue_num}" >&2
        rm -f "$tmp"; return 1          # grooming proceeds
    fi

    # Check B — provenance, for the message only.
    if [ "$(git rev-parse "${gate_ref}:${candidate}")" \
       = "$(git rev-parse "origin/main:${candidate}" 2>/dev/null)" ]; then
        COMMITTED_PLAN_PROVENANCE="inherited from main"
    else
        COMMITTED_PLAN_PROVENANCE="committed on the dispatch branch"
    fi
    rm -f "$tmp"; printf '%s' "$candidate"; return 0
done
```

`COMMITTED_PLAN_PROVENANCE` is global without `local`, the same contract as
`GROOM_LOOP_FAILURE_REASON` and `FIND_ISSUE_PLAN_REFUTED`: the caller reads it after the
function returns. Cleared on entry.

The `already_groomed` callback note and the `dispatch_gate_groom_refused` stderr line at
`dispatch-lib.sh:1213-1218` interpolate it in place of today's fixed clause "A committed
plan already exists on the dispatch branch."

## Implementation Units

### U1. Issue-binding refutation in the dispatch gate

`skills/bundled/_shared/dispatch-lib.sh` — `_committed_plan_on_branch`.
Add `$5` (default `$ISSUE_NUM`); after each candidate resolves, materialize and run
`_plan_header_refutes_issue`; on refutation emit `dispatch_gate_groom_plan_refuted:` naming
the claimed issues and return 1. Comment block records the two measured incidents and the
reason refutation lives here and not at the call site. Satisfies R1, R2, AE1–AE3.

### U2. Provenance in the gate's own words

Same function plus the call site at `dispatch-lib.sh:1213-1218`. Set and clear
`COMMITTED_PLAN_PROVENANCE`; interpolate it into the stderr diagnostic and the
`already_groomed` JSON note. Satisfies R3, AE4.

### U3. A fixture that can express the production shape

`skills/bundled/_shared/test-dispatch-lib.sh`, beside the five existing cases.
Add a fixture whose `main` carries a foreign plan (header claiming another issue) and whose
branch adds nothing, then assert the gate does **not** fire — the case that would have
caught mika#1887 and mika#2026. Add the "header claims the target issue → still fires" and
"no issue marker → still fires" counterparts so U1 cannot be satisfied by refusing
everything. Add a code-shape assertion that the gate calls `_plan_header_refutes_issue`, so
a future edit that drops the binding fails loudly. Satisfies R4, R5.

## Verification Contract

```bash
bash -n skills/bundled/_shared/dispatch-lib.sh
bash -n skills/bundled/_shared/test-dispatch-lib.sh
make test-dispatch-lib          # every pre-existing assertion, unmodified, plus U3's
make test-find-issue-plan       # _plan_header_* helpers are shared — prove no regression
make verify-bundled-skills
make test-dispatch-symmetry
```

Green required under a normal git config **and** under the hostile one PR#2028 documented
(`commit.gpgsign=true`, no `init.defaultBranch`), since U3 adds fixture commits.

**Live-state check before calling it done:** re-run the gate's logic against the two real
tickets (`feat/1887/…`, `obs/2026/…`) and confirm both now refute, per
`docs/solutions/best-practices/run-the-new-check-against-live-state-before-calling-it-done-2026-08-29.md`.

## Definition of Done

- U1–U3 implemented; the diff contains the binding, the provenance, and the fixture.
- Full verification contract green, both git configs.
- The two measured incidents demonstrably refute under the new gate.
- No change to `_iterate_groom_loop`'s convergence logic.
- PR states the mika#2028 / mika#2037 overlap verdict from § Scope overlap, so neither is
  re-corrected, and records that mika#2034's stated method was falsified rather than met.

## Acceptance criteria

- [ ] `_committed_plan_on_branch` accepts an optional 5th argument defaulting to `$ISSUE_NUM`; the four existing call sites and five existing fixture cases are unmodified and pass.
- [ ] A candidate whose header claims a different issue does NOT fire the gate; grooming proceeds and stderr carries `dispatch_gate_groom_plan_refuted:` naming the claimed issue.
- [ ] A candidate whose header claims the target issue, and a candidate whose header claims no issue at all, both still fire the gate (refute-never-confirm preserved).
- [ ] When the resolved blob is identical to `origin/main`'s at the same path, the operator diagnostic and the `already_groomed` callback note say the plan is inherited from `main` and do not assert it was committed to the dispatch branch.
- [ ] A regression fixture exists in which `main` carries a foreign plan and the branch adds nothing, asserting the gate does not fire — reproducing the mika#1887 / mika#2026 shape.
- [ ] A code-shape assertion fails if the gate stops calling `_plan_header_refutes_issue`.
- [ ] `make test-dispatch-lib`, `make test-find-issue-plan`, `make verify-bundled-skills`, `make test-dispatch-symmetry` and `bash -n` on both files are green, under a normal and a `commit.gpgsign=true` git config.
- [ ] Re-running the gate's logic against `feat/1887/…` and `obs/2026/…` shows both now refute.

## References

- `skills/bundled/_shared/dispatch-lib.sh:1039` (`_committed_plan_on_branch`), `:1213` (call site)
- `skills/bundled/_shared/test-dispatch-lib.sh:920-1042` (existing gate assertions and the five fixture cases)
- mika#2012 (gate origin), mika#2038 (`_plan_header_refutes_issue`, same founding plan), mika#2028 (same class, reporting site), mika#1772 / mika#1725 (the 07-04 lineage), mika#2029 (pilot stall, separate)
- `docs/solutions/best-practices/diagnostics-must-be-measured-not-asserted-2026-08-29.md`
- `feedback_a_stub_built_from_the_doc_cannot_falsify_the_doc` — why the fixture's `main` had to change
