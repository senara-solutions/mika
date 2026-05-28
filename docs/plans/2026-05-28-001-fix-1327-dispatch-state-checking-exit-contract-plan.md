---
title: "fix(dispatch): retire mika#1322 fabrication-string brake; remove templated exit-string from /mika-groom-plan-only"
type: fix
status: planned
date: 2026-05-28
issue: mika#1327
---

# fix(dispatch): retire mika#1322 fabrication-string brake; remove templated exit-string from /mika-groom-plan-only

## Overview

The dev-groom dispatch contract has two coupled defects that produce a structural false-positive:

1. `/mika-groom-plan-only` Phase 3 step 8 instructs the pilot to emit the literal exit text `Plan committed and pushed. Architect convergence pending via dispatch-lib iterate loop.`
2. `dispatch-lib.sh:643-661` greps the session log for the distinctive 60-char tail of that same text and prepends a `PIPELINE FAILURE: ... (idempotency-bypass-architect)` marker to `RESULT`.

When a pilot does the work correctly *and* emits the slash-command-instructed text, the brake fires and the post-flight outcome classifier (`dispatch-lib.sh:698`) flips the dispatch to `PIPELINE_INCOMPLETE` regardless of whether `_iterate_groom_loop` later returned `GROOMED`. Three of three fresh-ticket grooms 2026-05-28 (#1245, #1260, #856) tripped this exact false-positive.

This fix **fully retires** the symptom-string brake (per Vincent's explicit AC in issue comment IC_kwDORWsgGM8AAAABEDBqdw, 2026-05-28T17:22Z), and drops the templated exit instruction from the slash command so the pilot's last text becomes diagnostic, not gate-controlling. State-grounded checks that already exist in dispatch-lib (HEAD-unchanged, plan-file-missing, iterate-loop ESCALATE) cover the "pilot lied entirely" failure class that motivated the brake originally.

**Cross-link:** Joint 3 of the 2026-05-28 substrate-coherence cluster. Joints 1+2 (cpp#20 — visible `interrupt=True` denials + complete `permissions.yaml`) shipped 2026-05-28T18:54:51Z at commit `c3492b32`; that fix makes the LLM behavior "emit exit-string after silently-denied Bash" structurally impossible, so the only remaining producer of the fabrication string is the slash command itself. Removing both the producer (Unit 2) and the brake (Unit 1) closes the loop. Vincent confirmed cpp#20 deployment + soak gate open in ticket comment 2026-05-28T18:57Z: *"The mika#1322 brake retirement remains in this ticket's acceptance criteria (per the earlier comment) — post-implementation here, the fabrication-string grep becomes dead code."*

## Phase 0 Pins

Before drafting Units, the following load-bearing source-state was verified against `origin/main`:

### Pin A — `mika/skills/bundled/_shared/dispatch-lib.sh` brake region

- Lines 643-661 contain the brake exactly as the issue body describes.
- Comment header at line 643 references mika#1319.
- `FABRICATION_NEEDLE="Architect convergence pending via dispatch-lib iterate loop"` at line 651.
- `RESULT="PIPELINE FAILURE: dev-groom session exited without architect roundtrip (idempotency-bypass-architect). ..."` prepend at line 654.
- Fail-open stderr warning at line 659 (`Warning: session log not available at $SESSION_LOG — skipping idempotency-bypass-architect check`).
- Block-terminating `fi` at line 661.

### Pin B — All other dispatch-lib reads of pilot text/log (F2 scope check)

`grep -nE 'SESSION_LOG|PILOT_OUTPUT|FABRICATION_NEEDLE|grep.*log' mika/skills/bundled/_shared/dispatch-lib.sh` enumerates:

- Lines 426-436 — `PILOT_OUTPUT_RAW` parsed as **structured JSON via `jq`** (extracts `status`, `session_id`, `turns`, `cost_usd`, `duration_ms`). None of these depend on natural-language pilot text. The JSON envelope shape is owned by claude-pilot's stdout contract, not by what the LLM narrates.
- Lines 600-641 — `/ce:plan` invocation check on `$SESSION_LOG` (already demoted from gate to advisory per mika#1303). Pattern: `grep -qiE 'ce[.:\-_]plan'`. Not coupled to pilot exit text.
- Lines 643-661 — the brake (this PR's target).
- Lines 721, 731 — `PILOT_OUTPUT_RAW` echoed into RESULT body for diagnostic context when claude-pilot output is non-JSON. Diagnostic only; not a parser.

**Conclusion:** the brake at 643-661 is the ONLY dispatch-lib site whose behavior depends on a specific pilot exit string. Removing the brake removes the entire dispatch-lib coupling to pilot exit text. (Addresses architect F2.)

### Pin C — Downstream-consumer scope check (qa-review, webhooks, self-dev)

`grep -rnE "Plan committed|Architect convergence|fabrication" mika/skills/bundled/{qa-review,self-dev-webhook-ci,self-dev-webhook-qa,self-dev}` returns no consumer that parses pilot exit text. Two hits in `self-dev/system_prompt.md` (lines 244, 317) are mika-dev's outbound message templates ("Grooming completed for {repo}#{issue_number}. Plan committed on branch. PR: {url}.") that compose against the structured callback envelope (Outcome line + Plan path + PR URL), not against pilot session text. No re-write required.

### Pin D — `mika-platform/.claude/commands/mika-groom-plan-only.md` current Phase 3 step 8 (F3 scope check)

`git show origin/main:.claude/commands/mika-groom-plan-only.md` line 59 reads verbatim:

```
8. Output a brief confirmation: `Plan committed and pushed. Architect convergence pending via dispatch-lib iterate loop.`
9. Exit. The dispatch-lib outer layer (`_iterate_groom_loop`) takes over — finds the plan file, invokes `mika-arch-groom-ticket` first-pass, handles READY/ITERATE/ESCALATE, writes the canonical body-callout block via `_write_canonical_callout` on GROOMED.
```

Unit 2's before-state is unambiguous; the edit can proceed with the verified diff. (Addresses architect F3.)

### Pin E — Test 11 current shape

`mika/skills/bundled/_shared/test-dispatch-lib.sh:999-1025` Test 11 uses the sed-extraction pattern `/mika#1319.*idempotency-bypass-architect/,/^        fi$/p` to isolate the brake block. Assertions target presence of the block, the SKILL guard, the SESSION_LOG read, the fabrication-needle substring, the `PIPELINE FAILURE:` marker, the `idempotency-bypass-architect` sub-type identifier, and the fail-open warning.

After this PR retires the block, the sed pattern will return empty (no matching block exists). The test must be rewritten as a *block-absent* assertion, not a *block-present-with-different-shape* assertion.

## Problem Frame

### Failure chain (current, post-cpp#20)

1. dispatch-lib launches the pilot with `ENTRY_COMMAND=/mika-groom-plan-only`.
2. Pilot reads ticket, runs `/ce:plan`, writes `docs/plans/<file>.md`, runs `git add/commit/push`. HEAD advances. Plan file is `>500c`.
3. Pilot reaches Phase 3 step 8 of the slash command and emits the literal text: `Plan committed and pushed. Architect convergence pending via dispatch-lib iterate loop.`
4. `_run_claude_pilot` post-flight (line 651-661) greps `$SESSION_LOG` for the 60-char distinctive substring; match found; prepends `PIPELINE FAILURE: ... (idempotency-bypass-architect)` to `RESULT`.
5. `dispatch_claude_pilot` then runs `_iterate_groom_loop` (line 1622). Architect first-pass + second-pass run. Verdict is `GROOMED`. `_write_canonical_callout` writes the body callout successfully.
6. Outcome classifier (line 698) greps `RESULT` for `"PIPELINE FAILURE:"`. Match found (from step 4). Emits `Outcome: PIPELINE_INCOMPLETE — manual recovery needed.`
7. mika-dev receives `PIPELINE_INCOMPLETE` and surfaces a failure even though the architect actually converged and the canonical callout is on the issue body.

### Why the brake exists (mika#1322 context)

The brake was built against a different failure class: pilots that emitted the same string *without* doing the work (mika#806, mika#736 x2). In those incidents the pilot found a prior plan commit on HEAD, fabricated the success message, and exited without producing new commits. The brake correctly caught that — but it cannot distinguish "pilot lied" from "pilot did legit work + spoke the instructed exit text" because the input is identical (a string in the log).

### Why retire entirely (not demote to diagnostic)

This question was settled by Vincent in writing on the ticket. Two operative comments:

- **2026-05-28T17:22Z (IC_kwDORWsgGM8AAAABEDBqdw):** *"This ticket's scope should explicitly retire the #1322 grep as part of acceptance, not just add the state-check alongside it. Otherwise a reader six weeks out sees two fabrication-detection mechanisms in dispatch-lib.sh and can't tell which is canonical (a duplication that hides which version is load-bearing). The state-check is the replacement, not an addition. Defer the retirement until cpp#20 actually lands and soaks — so the brake's net stays up during the transition — but bake the retirement into this ticket's exit criteria now, while the relationship is fresh."*
- **2026-05-28T18:57Z:** *"cpp#20 (joints 1+2 + synthetic emit + cpp#21 source rename) deployed at `c3492b32` via cpp PR#22, merged + installed 2026-05-28T18:54Z. ... The mika#1322 brake retirement remains in this ticket's acceptance criteria (per the earlier comment) — post-implementation here, the fabrication-string grep becomes dead code."*

Two structural arguments for retirement, both Vincent-authored:

- **Readability/canonicality.** Keeping the brake as a diagnostic alongside the state-grounded checks creates a duplicate fabrication-detection mechanism in the same file. A future reader cannot tell which version is load-bearing. Retirement removes the ambiguity.
- **Dead code.** Post-cpp#20, the LLM-emits-exit-string-after-denied-Bash failure class can no longer happen structurally (denied Bash halts the pilot loop before the exit string can be emitted). Post-Unit 2 (this PR), the slash command no longer instructs the pilot to emit the string at all. The brake catches a class that the substrate no longer produces. It is dead code.

The state-grounded checks that already exist in dispatch-lib (HEAD-unchanged at line 451, plan-missing at line 621, iterate-loop ESCALATE inside `_escalate_groom`) cover all structural consequences of "pilot lied entirely." The brake's forensic-fingerprint value is real but small, and the cost of leaving it in place (canonicality ambiguity + dead code) is larger. Retirement wins on the operator's stated tradeoff.

(Pivot note: an earlier draft of this plan proposed demote-to-stderr-diagnostic on R4 forensic-continuity grounds. The architect's first-pass ESCALATE F1 finding cited Vincent's written AC; the plan was revised to honor the AC. The forensic-fingerprint concern is preserved by the historical record in mika#1322's commit + the updated solutions doc — Unit 4 — not by carrying the code itself.)

## Requirements Trace

- **R1** — Pilots that emit the historical fabrication string after producing legitimate `commit + push + plan file >500c` artifacts MUST NOT cause `Outcome: PIPELINE_INCOMPLETE`. The iterate-loop verdict is the authoritative source for groom convergence outcome.
- **R2** — Pilots that exit without producing legitimate artifacts (the original mika#806/#736 class) MUST still be caught and classified. Existing state-grounded checks (HEAD-unchanged at line 451, plan-missing at line 621) MUST continue to fire structurally on those paths.
- **R3** — The `/mika-groom-plan-only` slash command MUST NOT instruct the pilot to emit any templated exit string that the substrate later greps for. The pilot's exit text is diagnostic only; the contract is the commit + push + plan file artifacts.
- **R4** — The brake block at `dispatch-lib.sh:643-661` MUST be removed (per Vincent's explicit AC in ticket comment IC_kwDORWsgGM8AAAABEDBqdw). The historical fingerprint of the failure class is preserved in mika#1322's commit history and in the solutions doc (Unit 4); the running code does not need to carry it.
- **R5** — Test 11 in `test-dispatch-lib.sh` MUST be rewritten to assert the brake block is absent (regression guard against re-introduction). The new test asserts: no occurrence of the comment header `mika#1319.*idempotency-bypass-architect` and no occurrence of the fabrication needle string in the post-flight section.
- **R6** — The `/mika-groom-ticket` operator-direct slash command is OUT OF SCOPE. It does not emit the fabrication string and has its own architect convergence path (Phase 3 + Phase 4 of its spec). No edits to `.claude/commands/mika-groom-ticket.md`.

## Scope Boundaries

### In scope

- `mika/skills/bundled/_shared/dispatch-lib.sh` — REMOVE lines 643-661 entirely (the brake block + its preceding comment header).
- `mika-platform/.claude/commands/mika-groom-plan-only.md` — Phase 3 step 8 edit (drop templated string).
- `mika/skills/bundled/_shared/test-dispatch-lib.sh` — Test 11 rewrite to block-absent assertion.
- `mika/docs/solutions/best-practices/idempotency-bypass-architect-fabrication-detection-2026-05-28.md` — extend with retirement section.

### Out of scope

- `mika-platform/.claude/commands/mika-groom-ticket.md` — operator-direct path, unaffected.
- Engine-side changes (no Rust edits).
- Re-dispatching the 8 worktree-staled grooms from the 2026-05-28 morning fan-out (#763, #765, #768, #905, #917, #1179, #1182, #1258). Operator gates re-dispatch after this PR + cpp#20 are both live.
- Force-push authorization changes (mika#1318, separate ticket).
- Engine-side `idempotency-bypass-architect` recognition for structured reaper handling (mika#1322's deferred follow-up; the iterate-loop verdict already provides the structural signal needed).

### Explicit non-removals (preserved)

- HEAD-unchanged check at line 451-462 — unchanged. Catches "pilot exited 0 but did no work."
- Plan-file-missing check at line 621-630 — unchanged. Catches "pilot wrote no plan file."
- `/ce:plan` invocation check at lines 600-641 — unchanged. Already demoted to advisory per mika#1303 precedent.
- `_iterate_groom_loop` invocation at line 1622 — unchanged. Authoritative source of groom convergence outcome.
- `_escalate_groom` non-GROOMED path inside the iterate loop — unchanged. Appends its own PIPELINE FAILURE on ESCALATE.

## Key Technical Decisions

### Decision 1 — Remove the brake block entirely

**Choice:** Delete `dispatch-lib.sh:643-661` (comment header + `if [ "$SKILL" = "dev-groom" ]` block + closing `fi`). No replacement text; no stderr diagnostic; no historical comment in place.

**Why not demote to stderr diagnostic:**
- Vincent's ticket comment IC_kwDORWsgGM8AAAABEDBqdw explicitly forecloses this option ("The state-check is the replacement, not an addition.").
- Demotion creates a duplicate fabrication-detection mechanism that a future reader cannot disambiguate against the state-grounded checks. Vincent named this the "canonicality" concern.
- The brake's "pilot lied entirely" detection is structurally covered by HEAD-unchanged + plan-missing + iterate-loop ESCALATE. The fabrication-string fingerprint is recoverable from git history (mika#1322 commit) and from the solutions doc (Unit 4).
- Post-cpp#20 + Unit 2, no producer of the fabrication string remains. The brake is dead code.

**Why not leave a historical comment in place:**
- A comment without code is search-noise for future operators (grep hits with no behavior to inspect). The solutions doc (Unit 4) is the proper home for the historical record.
- The commit message for this PR + the linked ticket (mika#1327) + the prior brake PR (mika#1322) provide the git-history trail.

(review-guide.md § YAGNI — Vincent explicitly ruled on the YAGNI question in the issue body; ship the rule, not the speculation.)

### Decision 2 — Slash command Phase 3 step 8: drop the templated string entirely; instruct the pilot to exit silently after the push completes

**Choice:** Rewrite step 8 to: *"After `git push` succeeds, exit the session. Do not narrate the exit; do not emit any templated confirmation string. The pilot's last text is diagnostic-only — the substrate gates on commit-on-branch state, not on session text."*

**Why not just rephrase the string:**
- Any templated exit string can drift into the substrate as a brittle dependency. The contract change is that the pilot's last text becomes diagnostic, not gate-controlling.
- An LLM told to "exit silently" reliably exits silently for this class of task — there's no narrative content to generate (the work is done, the push succeeded, the dispatch-lib outer layer owns the next step).
- A worked alternative ("emit `Done.` and exit") still leaves a templated string the substrate could regress into matching on. Better to remove the templated-string concept entirely.

**Risk + mitigation:** The pilot might still emit *something* on exit (LLMs occasionally narrate even when instructed not to). Per Pin B + Pin C verification, no dispatch-lib post-flight check or downstream consumer keys on pilot exit text once the brake is removed. The iterate-loop verdict is what dispatches the outcome.

(review-guide.md § KISS — the simplest fix is to remove the contract surface entirely, not narrow it.)

### Decision 3 — Test 11 rewritten as block-absent regression guard

**Choice:** Replace Test 11's assertion set with assertions that the brake block does NOT exist:

```bash
# Test 11 (rewritten for mika#1327 retirement)
echo ""
echo "Test 11: Idempotency-bypass-architect fabrication brake retired (mika#1327)"
echo "---------------------------------------------------------------------------"

# Verify the brake block (mika#1322) is fully removed from dispatch-lib.
# Regression guard: re-introducing the fabrication-string grep brake without
# also re-introducing the AC discussion on mika#1327's ticket is forbidden.
assert_not_contains "Brake comment header (mika#1319.*idempotency-bypass-architect) is absent" 'mika#1319.*idempotency-bypass-architect' "$(cat "$DISPATCH_LIB")"
assert_not_contains "Brake comment header (idempotency-bypass-architect detection in dev-groom) is absent" 'idempotency-bypass-architect fabrication in dev-groom' "$(cat "$DISPATCH_LIB")"
assert_not_contains "Fabrication needle (Architect convergence pending via dispatch-lib iterate loop) is absent" 'Architect convergence pending via dispatch-lib iterate loop' "$(cat "$DISPATCH_LIB")"
assert_not_contains "FABRICATION_NEEDLE variable assignment is absent" 'FABRICATION_NEEDLE=' "$(cat "$DISPATCH_LIB")"
assert_not_contains "PIPELINE FAILURE marker with idempotency-bypass-architect sub-type is absent" 'PIPELINE FAILURE:.*idempotency-bypass-architect' "$(cat "$DISPATCH_LIB")"
```

**Why not delete the test:**
- Test 11 becomes the regression guard against re-introducing the brake. Without it, a future operator could add the brake back without anyone noticing.
- The block-absent assertions encode the AC explicitly: this ticket says the brake stays gone.

**Why use `assert_not_contains` against the whole file (not a sed-bounded region):**
- The block is being deleted entirely; there's no surviving sed-bounded region to extract.
- File-wide assertion is correct here precisely because the regression target is "anywhere in the file" — the test fails if the brake re-appears under a different comment header or in a different location.

(review-guide.md § Tests document the contract — when the contract changes from "block exists with shape X" to "block does not exist," the test changes with it.)

### Decision 4 — Solutions doc: extend the existing brake doc with a retirement section

**Choice:** Update `docs/solutions/best-practices/idempotency-bypass-architect-fabrication-detection-2026-05-28.md` with a new section: "Contract update 2026-05-28 — brake retired (mika#1327)." Cross-link to mika#1327, cpp#20 (joints 1+2), and the `project_cpp15_substrate_wedge_2026-05-28` memory.

The section MUST cover:
- What the brake was for (preserve mika#1322 incident-class history).
- Why it was retired (canonicality concern + dead-code argument, with Vincent's two ticket comments quoted verbatim).
- What replaced it (the state-grounded checks at HEAD-unchanged 451, plan-missing 621, iterate-loop ESCALATE).
- The structural principle for the future: substrate gates on state, not on text. When a string-gate is retired, retire it fully — the historical fingerprint goes in the solutions doc and in git, not in surviving dead code.

**Why not delete the doc:**
- The original section is the canonical entry for the failure class. Future operators searching for "idempotency-bypass-architect" should find both the original incident and the retirement context.
- Solutions-doc updates compound institutional learning; deletion erases it.

## Implementation Units

### Unit 1: Remove the fabrication brake block from dispatch-lib

**Goal:** Delete lines 643-661 from `dispatch-lib.sh`. No replacement code, no stderr diagnostic.

**Requirements:** R1, R2, R4

**Dependencies:** None.

**Files:**
- Modify: `mika/skills/bundled/_shared/dispatch-lib.sh` (delete lines 643-661 inclusive).

**Approach:**
- Use the Edit tool to delete the block. The exact span to remove:

  ```
  # (starts line 643, ends line 661)
  # mika#1319: Detect idempotency-bypass-architect fabrication in dev-groom
  # sessions. When the pilot finds a prior plan commit on HEAD, it sometimes
  # fabricates a success message claiming architect convergence is pending
  # "via dispatch-lib iterate loop" — but _iterate_groom_loop runs within
  # this same dispatch, not separately. The bit-identical fabrication string
  # appeared in 3/3 observed failures (mika#806, mika#736 x2). Detect it
  # structurally and classify as a distinct PIPELINE FAILURE sub-type.
  if [ "$SKILL" = "dev-groom" ]; then
      FABRICATION_NEEDLE="Architect convergence pending via dispatch-lib iterate loop"
      if [ -f "$SESSION_LOG" ] && [ -r "$SESSION_LOG" ]; then
          if grep -qF "$FABRICATION_NEEDLE" "$SESSION_LOG" 2>/dev/null; then
              RESULT="PIPELINE FAILURE: dev-groom session exited without architect roundtrip (idempotency-bypass-architect). Pilot claimed architect convergence is pending via dispatch-lib but dispatch-lib's _iterate_groom_loop runs within this same dispatch — not separately.

  ${RESULT}"
          fi
      else
          echo "Warning: session log not available at $SESSION_LOG — skipping idempotency-bypass-architect check" >&2
      fi
  fi
  ```

- After deletion, line 642 (closing `fi` of the plan-validation block) is followed directly by line 663 (the PR-discovery `# Issue #138: Discover actual PR URL ...` comment). Whitespace is preserved (one blank line between sections, matching surrounding style).

**Patterns to follow:**
- N/A (this is a deletion, not an addition).

**Test scenarios (verified by Unit 3 tests + Acceptance Validation):**
- Legit-plan + fabrication-string-in-log + iterate-loop GROOMED: `RESULT` has no PIPELINE FAILURE prefix; outcome is `PLAN_GROOMED`.
- Pilot-lied (HEAD unchanged) + fabrication-string-in-log: HEAD-unchanged check at line 451 fires; `RESULT` has PIPELINE FAILURE from that check; outcome is `PIPELINE_INCOMPLETE`.
- Pilot-lied (no commit, no plan file) + fabrication-string-in-log: HEAD-unchanged + plan-missing both fire; outcome is `PIPELINE_INCOMPLETE`.
- Legit-plan + fabrication-string-in-log + iterate-loop ESCALATE: `_escalate_groom` appends its own PIPELINE FAILURE; outcome is `PIPELINE_INCOMPLETE`.

### Unit 2: Drop the templated exit-string instruction from `/mika-groom-plan-only`

**Goal:** Rewrite Phase 3 step 8 so the pilot exits without narrating, removing the producer of the fabrication string.

**Requirements:** R3

**Dependencies:** None (independent of Unit 1; either can land first, both must land in the same coordinated PR pair for the contract to be coherent).

**Files:**
- Modify: `mika-platform/.claude/commands/mika-groom-plan-only.md` (Phase 3, lines 57-60).

**Approach:**

Replace (Pin D-verified before-state):

```
### Phase 3 — Exit cleanly

8. Output a brief confirmation: `Plan committed and pushed. Architect convergence pending via dispatch-lib iterate loop.`
9. Exit. The dispatch-lib outer layer (`_iterate_groom_loop`) takes over — finds the plan file, invokes `mika-arch-groom-ticket` first-pass, handles READY/ITERATE/ESCALATE, writes the canonical body-callout block via `_write_canonical_callout` on GROOMED.
```

With:

```
### Phase 3 — Exit cleanly

8. After `git push` succeeds, exit the session. Do not narrate the exit; do not emit any templated confirmation string. The pilot's last text is diagnostic-only — the substrate gates on commit-on-branch state, not on session text. (See mika#1327: the prior templated string was the producer of a false-positive in the mika#1322 fabrication brake, both retired together.)
9. The dispatch-lib outer layer (`_iterate_groom_loop`) takes over after the session exits — finds the plan file, invokes `mika-arch-groom-ticket` first-pass, handles READY/ITERATE/ESCALATE, writes the canonical body-callout block via `_write_canonical_callout` on GROOMED.
```

**Patterns to follow:**
- The `/mika-groom-ticket` operator-direct command (out of scope to edit) also does not instruct the pilot to emit a templated exit string. The autonomous-loop counterpart should mirror that contract shape.

**Test scenarios:**
- N/A for slash-command edits (no automated test surface). Validation is end-to-end per the Acceptance Validation section.

### Unit 3: Rewrite Test 11 as block-absent regression guard

**Goal:** Encode the retirement in the structural test so re-introduction is caught as a regression.

**Requirements:** R5

**Dependencies:** Unit 1 must land first or in the same commit (the test asserts against the post-Unit-1 dispatch-lib state).

**Files:**
- Modify: `mika/skills/bundled/_shared/test-dispatch-lib.sh` (lines 999-1025, Test 11 block).

**Approach:**

Replace the existing Test 11 block with:

```bash
# --- Test 11: Idempotency-bypass-architect fabrication brake retired (mika#1327) ---

echo ""
echo "Test 11: Idempotency-bypass-architect fabrication brake retired (mika#1327)"
echo "---------------------------------------------------------------------------"

# The mika#1322 brake (fabrication-string grep on session log) was retired in
# mika#1327. Per Vincent's ticket comment IC_kwDORWsgGM8AAAABEDBqdw, the brake
# was a duplicate fabrication-detection mechanism alongside state-grounded
# checks (HEAD-unchanged at line 451, plan-missing at line 621, iterate-loop
# ESCALATE inside _escalate_groom) and post-cpp#20 had become dead code.
#
# These assertions are regression guards against re-introducing the brake.

DISPATCH_LIB_CONTENT=$(cat "$DISPATCH_LIB")

assert_not_contains "Brake comment header (mika#1319 + idempotency-bypass-architect) is absent" 'mika#1319.*idempotency-bypass-architect' "$DISPATCH_LIB_CONTENT"
assert_not_contains "Brake comment header (idempotency-bypass-architect fabrication in dev-groom) is absent" 'idempotency-bypass-architect fabrication in dev-groom' "$DISPATCH_LIB_CONTENT"
assert_not_contains "Fabrication needle (Architect convergence pending via dispatch-lib iterate loop) is absent" 'Architect convergence pending via dispatch-lib iterate loop' "$DISPATCH_LIB_CONTENT"
assert_not_contains "FABRICATION_NEEDLE variable assignment is absent" 'FABRICATION_NEEDLE=' "$DISPATCH_LIB_CONTENT"
assert_not_contains "PIPELINE FAILURE marker with idempotency-bypass-architect sub-type is absent" 'PIPELINE FAILURE:.*idempotency-bypass-architect' "$DISPATCH_LIB_CONTENT"
```

**Patterns to follow:**
- `assert_not_contains` is already used elsewhere in `test-dispatch-lib.sh` (line ~997 region for the `.iterate/rescue-commit-err` regression guard) — same helper.
- File-wide assertion (not sed-bounded) is correct here because the regression target is "anywhere in the file" — the block is gone, not present-with-different-shape.

**Test scenarios:**
- Run `bash skills/bundled/_shared/test-dispatch-lib.sh` from the repo root. All assertions in Test 11 must pass after Unit 1's deletion.
- Negative test (mental): if the brake is re-introduced (any of the 5 forbidden strings appears anywhere in `dispatch-lib.sh`), Test 11 fails. Verified by inspecting the assertions before merge.

### Unit 4: Extend the solutions doc with the retirement section

**Goal:** Future operators reading the brake doc see the original incident (mika#1322) AND the retirement context (mika#1327) in one place.

**Requirements:** R4 (forensic continuity preserved by the solutions doc, not by surviving code).

**Dependencies:** Units 1–3 must land first; this doc edit reflects the shipped state.

**Files:**
- Modify: `mika/docs/solutions/best-practices/idempotency-bypass-architect-fabrication-detection-2026-05-28.md`.

**Approach:**
- Append a new section after the existing content, titled "Contract update 2026-05-28 — brake retired (mika#1327)."
- Cover (a) what the brake was for, (b) why retired (quote Vincent's two comments verbatim), (c) what replaced it (state-grounded checks at HEAD-unchanged 451, plan-missing 621, iterate-loop ESCALATE), (d) the principle for the future: substrate gates on state, not on text.
- Cross-link to cpp#20 (joints 1+2), `project_cpp15_substrate_wedge_2026-05-28` memory, and mika#1327.
- Do not rewrite the original section — keep it as historical record of the incident class.

**Patterns to follow:**
- Other solutions docs with multiple "Update YYYY-MM-DD" sections (e.g., `canonical-template-build-script` pattern doc).

**Test scenarios:**
- Doc-only change; no automated test. Spot-check that the new section is parseable markdown and cross-links resolve.

## Acceptance Criteria

**AC1.** A dev-groom dispatch on a clean ticket whose pilot writes the plan, commits, pushes, and exits (with or without emitting the historical fabrication string) results in `Outcome: PLAN_GROOMED` when the iterate-loop architect verdict is GROOMED. Verified by:
- Inspecting the dispatch RESULT post-run; no `PIPELINE FAILURE:` from the (now-removed) brake.
- `Outcome: PLAN_GROOMED — <plan-file-path>` MUST appear in the RESULT.
- The issue body MUST carry the canonical `Plan: ... (committed on branch @ <sha>)` callout.

**AC2.** A dev-groom dispatch where the pilot exits with HEAD unchanged (the original "pilot lied" failure class) still produces `Outcome: PIPELINE_INCOMPLETE`. Verified by:
- The HEAD-unchanged PIPELINE FAILURE marker (from line 451-462) MUST still appear in RESULT.
- Outcome classifier MUST still emit `PIPELINE_INCOMPLETE`.

**AC3.** The fabrication-string detection code is fully removed from `dispatch-lib.sh`. Verified by:
- `grep -F "FABRICATION_NEEDLE" mika/skills/bundled/_shared/dispatch-lib.sh` returns zero matches.
- `grep -F "idempotency-bypass-architect" mika/skills/bundled/_shared/dispatch-lib.sh` returns zero matches.
- `grep -F "Architect convergence pending via dispatch-lib iterate loop" mika/skills/bundled/_shared/dispatch-lib.sh` returns zero matches.

**AC4.** `bash skills/bundled/_shared/test-dispatch-lib.sh` passes all assertions (Test 11 reflecting the new block-absent contract; other tests unchanged).

**AC5.** `/mika-groom-plan-only` Phase 3 step 8 contains no templated exit-string instruction. Verified by:
- `grep -F "Architect convergence pending via dispatch-lib iterate loop" mika-platform/.claude/commands/mika-groom-plan-only.md` MUST return zero matches.
- `grep -F "Output a brief confirmation" mika-platform/.claude/commands/mika-groom-plan-only.md` MUST return zero matches.

**AC6.** The solutions doc `idempotency-bypass-architect-fabrication-detection-2026-05-28.md` includes a new section recording the 2026-05-28 retirement with mika#1327 cross-link and verbatim quotes of Vincent's two operative ticket comments.

## Acceptance Validation

**End-to-end validation (post-merge, not gating CI):**

1. Pick a clean ticket without an existing plan callout (e.g., one of the 8 staled grooms from the morning fan-out: #763, #765, #768, #905, #917, #1179, #1182, #1258 — each has an empty/stale worktree state per memory `project_cpp15_substrate_wedge_2026-05-28`).
2. Apply the `ready` label to trigger autonomous dispatch.
3. Verify the resulting RESULT line:
   - Contains `Outcome: PLAN_GROOMED — <path>` (not `PIPELINE_INCOMPLETE`).
   - Issue body carries the canonical Plan callout.
4. If the canary passes, dispatch the remaining 7 staled grooms in sequence.
5. If the canary fails with the same brake-false-positive symptom, halt and surface to operator — likely indicates one of the joints (cpp#20 deploy state, or this PR) is incomplete.

**CI gating:**

- `bash skills/bundled/_shared/test-dispatch-lib.sh` (existing CI hook in `mika`'s test workflow).
- No new CI surface needed.

## Risks and Mitigations

### Risk 1 — A future failure class re-emits the fabrication string but the retired brake no longer signals it

**Mitigation:** The state-grounded gates (HEAD-unchanged, plan-missing, iterate-loop ESCALATE) catch the *consequences* of any new failure class. The fabrication-string fingerprint is preserved in mika#1322's commit and in the Unit 4 solutions doc. Future operators investigating a similar failure will find the historical context via the solutions doc's frontmatter tags. Per Vincent's stated tradeoff, this loss of in-code fingerprint is acceptable in exchange for canonicality.

### Risk 2 — The slash command edit causes the pilot to emit a *different* templated string that future operators grep for

**Mitigation:** The edit explicitly instructs "Do not narrate the exit; do not emit any templated confirmation string." Per Pin B + Pin C verification, no dispatch-lib post-flight check or downstream consumer keys on pilot exit text. If the pilot emits something natural (e.g., a `git push` output paraphrase), it doesn't match any consumer dependency and doesn't trip any regression.

### Risk 3 — Test 11's file-wide `assert_not_contains` produces a false-positive if a benign reference to the brake's strings appears elsewhere in `dispatch-lib.sh` (e.g., in a comment)

**Mitigation:** The assertions target highly distinctive strings (`mika#1319.*idempotency-bypass-architect`, the full 60-char fabrication needle, `FABRICATION_NEEDLE=` variable assignment, `PIPELINE FAILURE:.*idempotency-bypass-architect`) that have no legitimate reason to appear in dispatch-lib after retirement. If a future PR needs to reference the historical brake in a comment, the comment text MUST avoid these distinctive strings (use prose, not the literal needle); the assertion failure surfaces this requirement loudly.

### Risk 4 — A re-dispatch on one of the morning fan-out staled grooms hits a different substrate hazard (e.g., the loader-walks-worktree class from mika#1326)

**Mitigation:** Out of scope for this PR. mika#1326 is the separate ticket for that class. If a canary on a staled ticket fails for that reason, the failure shape will be distinguishable (registry-drift symptom vs no-brake-false-positive symptom). The canary is gated to a single ticket to bound this risk.

### Risk 5 — `/mika-groom-ticket` (operator-direct, out of scope) silently regresses if a future edit copy-pastes the templated-string pattern

**Mitigation:** The Unit 4 solutions doc explicitly cross-references the contract principle ("substrate gates on state, not on text") so a future operator editing either slash command sees the rule. The audit surface is small (two slash-command files); a cross-link in the solutions doc is sufficient pre-emption.

### Risk 6 — The PR pair's atomicity: if Unit 2 (mika-platform) merges before Unit 1 (mika), the pilot still emits the string and… now the brake isn't there to catch even the legitimate-symptom case

**Mitigation:** That ordering is actually fine. Post-Unit-2-only, the pilot stops emitting the string entirely (because the slash command no longer instructs it). The brake then catches nothing because there's nothing to catch. No false-positive, no missed-detection. If the reverse order ships (Unit 1 first, Unit 2 second), the brake is gone but the slash command still instructs the pilot to emit the string — now harmless because no consumer matches on it. Both orderings are safe transitional states; the PR pair only needs to land within a single deploy window, not atomically.

## Cross-Repo Coordination

This PR touches both `mika/` and `mika-platform/` (the slash command lives in the meta-repo).

**Branch:** `fix/1327/dispatch-replace-idempotency-bypass` on **mika** (this branch).

**Companion change on mika-platform:** Same branch name (`fix/1327/dispatch-replace-idempotency-bypass`) for the slash-command edit. Per CLAUDE.md cross-repo conventions:

- Primary repo: `mika` (bulk of the work — dispatch-lib deletion + test rewrite + solutions doc).
- Secondary repo: `mika-platform` (single-file slash-command edit).
- Approach: "Primary + direct" per CLAUDE.md cross-repo table. Dispatch `/mika` for `mika`; make the slash-command edit directly on a branch in `mika-platform`.

**Coordination invariants:**
- Both PRs SHOULD merge within a single deploy window for cleanest substrate state, but per Risk 6 the ordering is robust.
- PR descriptions on both sides MUST cross-reference: `Companion PR: senara-solutions/<other>#<n>`.

## Compound Step (post-merge)

After merge, the `/mika` pipeline's compound step will:
1. Update `mika/docs/solutions/best-practices/idempotency-bypass-architect-fabrication-detection-2026-05-28.md` with the retirement section (Unit 4).
2. Add the cross-link to `project_cpp15_substrate_wedge_2026-05-28` memory and to cpp#20's PR/issue numbers.
3. Document the principle: "Substrate gates on state, not on text. When retiring a string-gate, retire it fully — the historical fingerprint goes in the solutions doc and in git, not in surviving dead code."

## References

- mika#1327 (this ticket)
- Ticket comments IC_kwDORWsgGM8AAAABEDBqdw (2026-05-28T17:22Z) and 2026-05-28T18:57Z (full retirement AC + cpp#20 deploy confirmation)
- mika#1322 (the brake PR being retired)
- mika#1319 (original idempotency-bypass diagnosis)
- mika#1303 (precedent demotion: `/ce:plan` check from gate to advisory)
- cpp#20 (joints 1+2 of this substrate-coherence cluster, shipped 2026-05-28T18:54:51Z at `c3492b32`)
- Memory: `project_cpp15_substrate_wedge_2026-05-28` (substrate-evaluation log for the 3-joint cluster)
- Memory: `feedback_prompt_enforcement_fragile` (why prompt-and-grep contracts collide)
- mika/docs/architecture/review-guide.md (YAGNI, KISS, Single Responsibility principles cited above)
- mika/docs/solutions/best-practices/idempotency-bypass-architect-fabrication-detection-2026-05-28.md (Unit 4 target)
