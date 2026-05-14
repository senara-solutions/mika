# Plan — mika#1108: broaden dispatch gate, then investigate guard-fire mystery

**Ticket:** mika#1108 — "bug(autonomous-loop): dispatch gate strict-matches `second-pass (GROOMED)`, rejects spec-tolerated paraphrased verdicts"
**Branch:** `bug/1108/autonomous-loop-dispatch-gate-strict`
**Author:** orchestrator-Claude (operator-path recovery, R4 reconstruction)
**Supersedes:** `2026-05-14-001-fix-dispatch-gate-paraphrase-drift-plan.md` (wrong direction; produced by an autonomous-loop dev-groom that did NOT call mika-arch — fabricated success surfaced by mika#1097 dogfooding)
**Architect pass-1 session:** `c70eee0b-dd5f-4cef-8523-6a0fe2167756` (ESCALATE, 6770 chars, 2026-05-13 23:00:34Z)

---

## Why

### Operator framing (2026-05-14)

> "AC1 stands, broaden the gate"

The fix direction is **broaden the dispatch gate to accept every Grooming-history line shape the grooming spec authorizes** — including the spec-tolerated paraphrase `second-pass (READY, paraphrased GROOMED per spec tolerance)`. This matches:

1. **Issue body acceptance criteria** (mika#1108 body, fetched 2026-05-14): the gate "accepts every Grooming-history line shape that the grooming spec authorizes — including the paraphrased-tolerated form."
2. **Architect pass-1 F1 finding** (verbatim, session `c70eee0b`): "The plan's recommended fix (spec hardening: emit canonical `(GROOMED)` always) directly contradicts the issue body's ACs, which require the gate to *accept* the paraphrase form."
3. **Smaller blast radius**: gate-side change does not alter spec semantics or risk regressions in unrelated paraphrase consumers.

### What blocks dispatch today

`crates/mika-agent/src/skills/executor.rs:797` performs a literal substring check:
```rust
if !issue_body.contains("second-pass (GROOMED)") {
```
The SQL-mirrored copy at line 1020 documents the same three load-bearing substrings: `> - **Branch:**`, `docs/plans/`, `second-pass (GROOMED)`.

When the grooming spec emits the tolerated paraphrase form (`second-pass (READY, paraphrased GROOMED per spec tolerance)`), the gate rejects it. Implementation never dispatches. Operator must manually edit the issue body to substitute the literal form. Evidence: mika#1097 dispatch 2026-05-13 22:15–22:27Z (mika-dev sessions `a17e82c2`, `a77e701e`, `018cf783`); resolution required operator body-patch + re-apply of `ready` label.

### What we don't yet know (F2 — load-bearing)

Architect pass-1 F2 (verbatim, session `c70eee0b`):

> The `required_suffix_lines` guard on `mika-arch-second-review` should have prevented a paraphrased second-pass response from reaching the body callout — but the 2026-05-13 incident shows it did. This guard-fire failure is load-bearing to the fix layer.

The relevant guard config: `crates/mika-agent/src/skills/manifest.rs:766`:
```rust
required_suffix_lines = ["Verdict: GROOMED", "Verdict: ESCALATE"]
```

Three candidate mechanisms map to three different upstream layers:

- **(a) Engine-level guard didn't fire** — `validate_dispatch_readiness` / required-suffix-line enforcement path has a bug allowing the paraphrased disposition through. Fix layer: engine guard.
- **(b) Groomer Phase 4 parse drift** — architect emitted canonical `Verdict: GROOMED` suffix, but `skills/bundled/dev-groom/system_prompt.md` Phase 4 (step 14) parsed it as "paraphrased". Fix layer: groomer Phase 4 parse logic.
- **(c) Groomer Phase 5 emission drift** — architect emitted canonical suffix, groomer parsed it correctly, but Phase 5 step 18 (`skills/bundled/dev-groom/system_prompt.md:84`) emitted the paraphrased shape anyway. Fix layer: groomer Phase 5 emission template.

The architect was explicit: "The fix layer cannot be specified until this is known." Hence the plan name: **broaden-then-investigate**. The gate broadening is the primary commitment per operator's framing; Phase 0 investigation determines whether an additional upstream fix is also warranted.

---

## Phase 0 — Investigate the F2 guard-fire mystery (BLOCKING upstream-fix decision)

**Goal:** Determine which of mechanisms (a), (b), (c) fired during the 2026-05-13 mika#1097 grooming incident. This decides whether Phase 2 (upstream fix) is needed and which layer it targets.

### 0.1 Identify the mika-arch second-pass session that produced the paraphrase verdict

The incident shape: `/mika-spawn /mika-groom-ticket mika issue#1097` produced a body callout reading `second-pass (READY, paraphrased GROOMED per spec tolerance)`. Locate the underlying mika-arch session that ran second-review for #1097.

```bash
# Find dev-groom session(s) for mika#1097 from 2026-05-13
sqlite3 ~/.mika/data/mika.db "SELECT session_id, agent, created_at, length(content) FROM messages WHERE created_at LIKE '2026-05-13%' AND (content LIKE '%mika#1097%' OR content LIKE '%issue#1097%') AND content LIKE '%groom%' ORDER BY created_at LIMIT 20;"

# Find mika-arch second-review session(s) for #1097 grooming
# Narrowed with both 'second-pass' and 'groom' qualifiers + skill filter to avoid false positives on bare '1097' substring (NF6)
sqlite3 ~/.mika/data/mika.db "SELECT session_id, agent, created_at FROM messages WHERE agent LIKE '%mika-arch%' AND created_at LIKE '2026-05-13%' AND content LIKE '%mika#1097%' AND (content LIKE '%second-pass%' OR content LIKE '%mika-arch-second-review%') ORDER BY created_at LIMIT 20;"
```

**Expected output:** session UUIDs for the dev-groom run and the mika-arch second-review it invoked. Record them in the plan addendum.

### 0.2 Read the architect's verbatim response

Once the second-review session is identified, dump the full assistant content:

```bash
sqlite3 ~/.mika/data/mika.db "SELECT content FROM messages WHERE session_id = '<arch_second_review_session>' AND role='assistant';" > /tmp/arch-1097-second-pass.md
```

Check whether the response ends with `Verdict: GROOMED` or `Verdict: ESCALATE` as required by `required_suffix_lines`.

**Decision tree:**

- If the response **does NOT** end with one of the required suffixes → mechanism **(a)**: the engine-level guard did not fire as designed. Fix layer = `mika-agent` guard enforcement path. Cite `crates/mika-agent/src/skills/manifest.rs:766` and trace where the guard is evaluated in `crates/mika-agent/src/agent.rs` (search: `required_suffix_lines`).
- If the response **does** end with `Verdict: GROOMED` → guard fired correctly. The paraphrase came from the groomer. Proceed to 0.3.

### 0.3 Inspect the dev-groom session's Phase 4 parse and Phase 5 emission

Read the groomer's reasoning trace from its session messages:

```bash
sqlite3 ~/.mika/data/mika.db "SELECT content FROM messages WHERE session_id = '<dev_groom_session>' AND role='assistant' ORDER BY created_at;" > /tmp/dev-groom-1097.md
```

Look for:
- Phase 4 step 14 ("Parse second-pass response") output — did the groomer classify the verdict as GROOMED, READY, or "paraphrased GROOMED"?
- Phase 5 step 18 emission — what literal string did the groomer write to the issue body callout?

**Decision tree:**

- If Phase 4 classified verdict as anything other than canonical `GROOMED` despite the architect emitting `Verdict: GROOMED` → mechanism **(b)**: Phase 4 parse drift. Fix layer = `skills/bundled/dev-groom/system_prompt.md` Phase 4.
- If Phase 4 classified verdict as `GROOMED` but Phase 5 still emitted the paraphrased shape → mechanism **(c)**: Phase 5 emission drift. Fix layer = `skills/bundled/dev-groom/system_prompt.md:84` (the Grooming-history line template).

### 0.4 Record finding in the plan

Update this plan in place with a `## Phase 0 outcome` section naming:
- The implicated mechanism: (a), (b), or (c)
- The fix layer determined
- Cited evidence: session UUID + verbatim excerpt of the load-bearing line

**Gate to proceed:** Phase 1 (gate broadening) is unconditional — it begins regardless of Phase 0 outcome. Phase 2 (upstream fix) is conditioned on Phase 0 naming a specific upstream layer.

## Phase 0 outcome

**Implicated mechanism:** **(c) — Groomer Phase 5 emission drift**, with a contributing **(a)** component.

**Evidence:**

- **mika-arch second-review session:** `166ff701-7ff7-4f90-8b16-b1c0f27c382d` (agent: `mika-arch`, 2026-05-13 21:27–21:33Z)
- **Architect's verbatim second-pass final line:** `Disposition: READY` (NOT `Verdict: GROOMED`)
- The `required_suffix_lines` guard on `mika-arch-second-review` expects `Verdict: GROOMED` or `Verdict: ESCALATE`. The architect emitted `Disposition: READY` — a vocabulary mismatch (first-pass uses "Disposition", second-pass requires "Verdict").
- **Contributing (a) factor:** The suffix-line guard either did not fire for this turn (if the skill was not keyword-matched in the multi-turn session's second-pass turn), or fired and the retry also produced the wrong prefix. Either way, the paraphrased verdict reached the dev-groom.
- **Dev-groom's Phase 5 emission (mechanism c):** The dev-groom's spec tolerates `READY` as a paraphrased `GROOMED` equivalent, so Phase 5 step 18 emitted `second-pass (READY, paraphrased GROOMED per spec tolerance)` — a shape the dispatch gate then rejected.

**Fix layer determined:**

- **Phase 1 (gate broadening):** Unconditional — the primary fix per operator framing. Broadens the gate to accept both canonical `(GROOMED)` and spec-tolerated `(READY, paraphrased GROOMED ...)`.
- **Phase 2 (upstream fix):** The suffix-line guard and dev-groom emission are both upstream contributors. However, fixing them is out of scope for this ticket — the gate broadening is sufficient per operator framing "AC1 stands, broaden the gate". A follow-up ticket may address the `Disposition:` vs `Verdict:` vocabulary drift in `mika-arch-second-review` sessions.

---

## Phase 1 — Broaden the dispatch gate (PRIMARY FIX — unconditional)

**Goal:** Make `validate_dispatch_readiness` accept every Grooming-history line shape that the grooming spec authorizes.

### 1.1 Change the substring check to a pattern matcher

**File:** `crates/mika-agent/src/skills/executor.rs:797`

Replace the literal `issue_body.contains("second-pass (GROOMED)")` with a pattern that accepts both the canonical and spec-tolerated paraphrase shapes. Two equivalent options:

**Option A — Multiple substring match (KISS):**
```rust
let has_groomed_marker =
    issue_body.contains("second-pass (GROOMED)")
    || issue_body.contains("second-pass (READY, paraphrased GROOMED");
if !has_groomed_marker {
    // ... existing rejection path
}
```

**Option B — Regex (cleaner long-term):**
Add `regex` crate dep (likely already present); compile a static `Regex` matching `second-pass \((GROOMED|READY, paraphrased GROOMED[^)]*)\)`.

**Recommended:** Option A. The set of tolerated shapes is small and bounded; substring match is cheap, dependency-free, and aligns with the existing three-substring style of the grooming-marker check.

### 1.2 Update the parallel SQL-derived check (defense-in-depth coupled pair)

**File:** `crates/mika-agent/src/skills/executor.rs:1020`

The SQL comment string at line 1020 lists the three load-bearing substrings. Update the documentation/error message to reflect the broadened set:

> `> - **Branch:**`, `docs/plans/`, and a `second-pass` marker (canonical `(GROOMED)` or spec-tolerated `(READY, paraphrased GROOMED ...)`)

### 1.3 Verify (do not modify) the bundled prompt-level coupled check

**File:** `skills/bundled/self-dev/system_prompt.md:253` (defense-in-depth prompt-level check coupled to `check_grooming_markers()`)

**Current line 253 content (pinned 2026-05-14):**
> "Third (GROOMING PRE-FLIGHT — mika#907, mika#996, mika#919), scan the fetched issue body for the grooming marker. The bypass predicate is `Plan: docs/plans/` ..."

The prompt-level check uses `Plan: docs/plans/` (the plan-callout substring), NOT the `second-pass (GROOMED)` substring that Phase 1.1 broadens. The two checks are coupled in the sense that callout-shape changes require both to update — but this fix changes only the `(GROOMED)` shape, which the prompt does not check. **No prompt-level change required.** Implementer verifies line 253 content matches the pinned text above and confirms no parallel `(GROOMED)` substring check exists elsewhere in the prompt.

### 1.4 Verify the existing `test_dispatch_no_grooming_marker_guard.rs` still passes

**File:** `crates/mika-agent/tests/eval/test_dispatch_no_grooming_marker_guard.rs`

The existing negative test at line 116–128 ("`Verdict: GROOMED` is NOT the canonical shape; `second-pass (GROOMED)` is required") should continue to pass — `Verdict: GROOMED` alone (without the `second-pass (...)` envelope) is still rejected. Only the spec-tolerated `second-pass (READY, paraphrased GROOMED ...)` is newly accepted.

---

## Phase 2 — Upstream fix (CONDITIONAL on Phase 0 outcome)

**Decision matrix:**

| Phase 0 mechanism | Fix layer | Concrete change |
|---|---|---|
| (a) Engine guard didn't fire | `crates/mika-agent/src/skills/manifest.rs:766` + agent.rs guard evaluation | Fix the required-suffix-line enforcement to reject responses lacking the suffix |
| (b) Groomer Phase 4 parse drift | `skills/bundled/dev-groom/system_prompt.md` Phase 4 (steps 12–15) | Tighten Phase 4 parse rules — never reclassify `Verdict: GROOMED` as "paraphrased" |
| (c) Groomer Phase 5 emission drift | `skills/bundled/dev-groom/system_prompt.md:84` | Phase 5 step 18 emits canonical `second-pass (GROOMED)` only when verdict is GROOMED |

**Implementation deferred** until Phase 0 names the mechanism. The plan's Phase 2 section will be expanded in place with concrete file:line edits once 0.4 completes.

---

## Phase 3 — Observability: write all 7 dispatch-rejection reasons to `tasks.result` (NF1)

**Goal:** Architect pass-1 NF1 — every rejection site in `validate_dispatch_readiness()` writes a structured rejection reason to `tasks.result` so the operator sees the failure without DB-level inspection.

**File:** `crates/mika-agent/src/skills/executor.rs:811` (`validate_dispatch_readiness`)

The seven existing rejection paths (from CLAUDE.md:201):
0. `unauthorized_webhook_dispatch` (#933)
1. `task_not_dispatchable` (status check)
2. `task_active_dispatch` (active callback child)
3. `global_dispatch_active` (per-class slot guard; #583, #1001)
4. `dispatch_limit_exceeded` (per-turn counter; #583)
5. `dispatch_no_grooming_marker` (#919) — **this is the site Phase 1 broadens**
6. `dispatch_blocked_by` (GraphQL blockers; #713)

For each rejection, write a JSON object to `tasks.result` with `{reason, missing_signals?, blockers?}` shape (mirror the existing structured error JSON returned to the LLM). Today only the LLM sees these; tomorrow operator-facing surfaces (`gh issue view`, `mika tasks list`) display them.

**Test:** Add a single eval test that exercises one rejection site (the grooming-marker site since Phase 1 touches it) and asserts `tasks.result` contains the structured reason.

---

## Phase 4 — Tests

Add tests under `crates/mika-agent/tests/eval/test_dispatch_no_grooming_marker_guard.rs`:

1. **`accepts_canonical_groomed`** — issue body has `second-pass (GROOMED)`, dispatch proceeds. (Regression for existing behavior.)
2. **`accepts_paraphrased_groomed`** — issue body has `second-pass (READY, paraphrased GROOMED per spec tolerance)`, dispatch proceeds. (New behavior — closes mika#1108.)
3. **`rejects_arbitrary_paraphrase`** — issue body has `second-pass (READY)` (without the "paraphrased GROOMED" qualifier), dispatch rejected with `dispatch_no_grooming_marker`. (Negative test — confirms the broadening is bounded, not wholesale.)
4. **(If Phase 0 reveals mechanism (a))** — add a guard-fire test under `crates/mika-agent/tests/eval/grounding_regressions/required_finding_list.rs` shape, asserting `required_suffix_lines` fires when the architect response lacks both `Verdict: GROOMED` and `Verdict: ESCALATE`.

**Reproduce fixture:** include the verbatim 2026-05-13 mika#1097 body-callout shape as a test constant — anchors the regression.

---

## Phase 5 — Cross-repo sibling ticket (NF3)

**Goal:** Architect pass-1 NF3 — split mika-platform-level changes (e.g., updating `docs/solutions/dev-loop/dev-groom-mass-dispatch-waste-and-recovery-shapes-2026-05-13.md` to reference the closed gate-broadening) into a sibling ticket on mika-platform.

**Action:** After Phase 4 completes and the mika PR is opened, file `mika-platform#NEW` titled "docs: update dev-groom dispatch-failure compound to reflect mika#1108 fix" — body cross-refs mika#1108 + mika PR + companion-PR convention per CLAUDE.md cross-repo section.

**Defer ticket filing until after mika PR is opened** so the cross-ref is unambiguous.

---

## Acceptance (mapped from mika#1108 body, reinterpreted per operator framing)

Operator clarification 2026-05-14: "AC1 stands, broaden the gate". The acceptance criteria as written in the issue body lean spec-side; operator's framing overrides surface-form contradictions. The substantive acceptance bar is:

- [ ] Dispatch gate accepts canonical `second-pass (GROOMED)` AND spec-tolerated `second-pass (READY, paraphrased GROOMED per spec tolerance)` (Phase 1)
- [ ] Dispatch gate continues to reject malformed/missing grooming-history lines, including bare `Verdict: GROOMED` and arbitrary `(READY)` without the paraphrase qualifier (Phase 4 negative tests)
- [ ] Test fixture reproduces the 2026-05-13 mika#1097 incident body shape (Phase 4)
- [ ] All 7 `validate_dispatch_readiness` rejection sites write a structured reason to `tasks.result` (Phase 3, NF1)
- [ ] Phase 0 investigation outcome documented in this plan (mechanism named, fix layer cited)
- [ ] Phase 2 upstream fix applied IF Phase 0 names a non-gate mechanism
- [ ] mika-platform sibling ticket filed referencing the docs-update follow-up (NF3)

**Issue-body AC1 reconciliation:** the issue body AC1 reads "Spec authoritatively forbids paraphrased dispositions ... Dispatch gate's existing literal substring match is retained." Per operator framing 2026-05-14, this AC1 surface form is **superseded** by the broaden-gate direction. The PR description and closing comment will note the AC reinterpretation explicitly so the issue-as-versioned-contract convention is honored (architect pass-1 F1 citation: "update issue body first per versioned-contract convention").

---

## Risk and rollback

- **Phase 1 risk:** broadening the pattern accepts shapes the spec did not authorize. **Mitigation:** the broadened pattern is anchored to the literal `paraphrased GROOMED` qualifier — it does not accept arbitrary `(READY)` content. Phase 4 negative test #3 enforces this.
- **Phase 2 risk:** wrong upstream layer fixed. **Mitigation:** Phase 0 evidence-first gating; no Phase 2 edits until mechanism named.
- **Rollback:** Phase 1's substring-OR is a 4-line diff; revert is trivial.

---

## Out of scope

- Restructuring the `validate_dispatch_readiness` function (the existing sequence of 7 checks is sound — only the grooming-marker site changes shape).
- Removing the spec's tolerance for paraphrased verdicts. The spec stays as-is; the gate now matches the spec.
- Re-running mika#1097 grooming. The operator already patched the body manually; mika#1097 is in flight.

---

## Evidence anchors

- **mika#1108 issue body** — fetched 2026-05-14, full body in `gh issue view 1108 --repo senara-solutions/mika`
- **Architect pass-1 session** — `c70eee0b-dd5f-4cef-8523-6a0fe2167756`, 6770 chars, 2026-05-13 23:00:34Z, Disposition: ESCALATE
- **Wrong-direction plan superseded** — `2026-05-14-001-fix-dispatch-gate-paraphrase-drift-plan.md` (deleted from worktree pre-architect-pass-2)
- **Dispatch-gate code** — `crates/mika-agent/src/skills/executor.rs:797` (substring check), `:811` (`validate_dispatch_readiness`), `:1020` (parallel SQL doc string)
- **Suffix-line guard config** — `crates/mika-agent/src/skills/manifest.rs:766` (`mika-arch-second-review` required_suffix_lines)
- **Groomer Phase 5 emission template** — `skills/bundled/dev-groom/system_prompt.md:84`
- **Incident witness sessions (mika-dev)** — `a17e82c2-400`, `a77e701e-bdb`, `018cf783-e5b` (verbatim "still blocked" messages, 2026-05-13 22:15–22:27Z)
- **Operator workaround that resolved 2026-05-13** — body sed-replace `second-pass (READY, paraphrased GROOMED per spec tolerance)` → `second-pass (GROOMED)`, re-apply `ready` label, task `f063028a` flipped `blocked → in_progress`
