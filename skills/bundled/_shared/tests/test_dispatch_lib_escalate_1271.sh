#!/bin/bash
# Flux ESCALATE (mika#1271 sub-PR 5) — extrait de test-dispatch-lib.sh par mika#3178, vol 2.
# Ce que dispatch-lib fait quand l'architecte rend ESCALATE : la sortie de boucle,
# la remontée à l'opérateur, et la note `already_groomed` qui nomme le geste qui
# marche plutôt que celui qui semble marcher.
#
# Plage d'origine : lignes 1096–1930 de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 890f3f5f. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
# ============================================================================
# ESCALATE flow (mika#1271 sub-PR 5)
# ============================================================================

echo ""
echo "Test: ESCALATE-flow helper (mika#1271 sub-PR 5)"
echo "-----------------------------------------------"

# _escalate_groom — writes findings file under .iterate/ + appends structured
# PIPELINE FAILURE marker to RESULT
ESC_TMP=$(mktemp -d)
RESULT=""
WORKTREE_DIR="$ESC_TMP" _escalate_groom "first-pass" "F1: Concern\nDisposition: ESCALATE" "session-esc-1" 2>/dev/null
if [ -r "$ESC_TMP/.iterate/escalate-first-pass.md" ]; then
    assert_eq "_escalate_groom writes findings file under .iterate/" "ok" "ok"
else
    assert_eq "_escalate_groom writes findings file under .iterate/" "ok" "missing"
fi
# mika#2545: the marker changed CLASS, not substance. `PIPELINE FAILURE:` is what
# put a terminal grooming verdict into `self-dev-callback`'s retryable population
# and got the groom of mika#2542 replayed eight times; the producer now stamps its
# own terminal fact instead. The negative control below is the half that matters —
# without it, "the terminal marker is written" would not say the retryable one is
# gone.
assert_contains "_escalate_groom stamps the terminal marker on RESULT (mika#2545)" "GROOM ESCALATED (terminal): mika-arch escalated at first-pass" "$RESULT"
assert_not_contains "_escalate_groom no longer writes the retryable marker (mika#2545)" "PIPELINE FAILURE:" "$RESULT"
assert_contains "_escalate_groom RESULT includes Verdict: ESCALATE" "Verdict: ESCALATE" "$RESULT"
assert_contains "_escalate_groom RESULT includes session_id" "Session: session-esc-1" "$RESULT"
assert_contains "_escalate_groom RESULT references findings file path" "Architect findings preserved at:" "$RESULT"
rm -rf "$ESC_TMP"

# _escalate_groom — engine escalation line (mika#2338): the cause travels into RESULT and
# the failure reason names the engine, not the architect.
ESC_TMP_ENG=$(mktemp -d)
RESULT=""
GROOM_LOOP_FAILURE_REASON="architect ESCALATE (first-pass)"
ENGINE_CONTENT="A1: x
A2: y

$ENGINE_LINE

Disposition: ESCALATE"
WORKTREE_DIR="$ESC_TMP_ENG" _escalate_groom "first-pass" "$ENGINE_CONTENT" "session-esc-eng" 2>/dev/null
assert_contains "_escalate_groom copies the engine line into RESULT as Engine reason" "Engine reason: $ENGINE_LINE" "$RESULT"
assert_eq "_escalate_groom names the engine in the failure reason" \
    "engine ESCALATE (first-pass): review-anchor attestation withheld" "$GROOM_LOOP_FAILURE_REASON"
rm -rf "$ESC_TMP_ENG"

# An architect ESCALATE whose prose echoes `[mika-engine]` is NOT relabelled as an engine refusal.
ESC_TMP_ECHO=$(mktemp -d)
RESULT=""
GROOM_LOOP_FAILURE_REASON="architect ESCALATE (first-pass)"
WORKTREE_DIR="$ESC_TMP_ECHO" _escalate_groom "first-pass" "F1: (BLOCKING) The [mika-engine] re-prompt was addressed but the plan contradicts ADR-008.
Disposition: ESCALATE" "session-esc-echo" 2>/dev/null
if grep -q "Engine reason:" <<<"$RESULT"; then
    assert_eq "_escalate_groom ignores an echo of [mika-engine] in architect prose" "no engine reason" "engine reason present"
else
    assert_eq "_escalate_groom ignores an echo of [mika-engine] in architect prose" "no engine reason" "no engine reason"
fi
assert_eq "_escalate_groom keeps the architect reason on an echo" "architect ESCALATE (first-pass)" "$GROOM_LOOP_FAILURE_REASON"
rm -rf "$ESC_TMP_ECHO"
GROOM_LOOP_FAILURE_REASON=""

# _escalate_groom — distinct stage labels write distinct findings files
ESC_TMP2=$(mktemp -d)
RESULT=""
WORKTREE_DIR="$ESC_TMP2" _escalate_groom "second-pass-after-ready" "second-pass-ready content" "sess-2" 2>/dev/null
if [ -r "$ESC_TMP2/.iterate/escalate-second-pass-after-ready.md" ]; then
    assert_eq "_escalate_groom: second-pass-after-ready stage label drives filename" "ok" "ok"
else
    assert_eq "_escalate_groom: second-pass-after-ready stage label drives filename" "ok" "missing"
fi
RESULT=""
WORKTREE_DIR="$ESC_TMP2" _escalate_groom "second-pass-after-iterate" "second-pass-iter content" "sess-3" 2>/dev/null
if [ -r "$ESC_TMP2/.iterate/escalate-second-pass-after-iterate.md" ]; then
    assert_eq "_escalate_groom: second-pass-after-iterate stage label drives filename" "ok" "ok"
else
    assert_eq "_escalate_groom: second-pass-after-iterate stage label drives filename" "ok" "missing"
fi
rm -rf "$ESC_TMP2"

# Code-shape: _iterate_groom_loop has exactly 5 _escalate_groom call sites.
#
# mika#2641 — le compte passe de 3 à 5. Les deux bras de seconde passe qui
# escaladaient indistinctement (`*)`) se sont scindés en DEUX chacun :
# `ESCALATE)` (un refus réel, cause `verdict`, `RESULT` inchangé à l'octet près)
# et `*)` (chaîne vide, cause `architect_unreadable`, AUCUNE ligne `Verdict:`).
# C'est cette scission qui EST le correctif : avant, une absence de signal
# empruntait le bras d'un verdict.
#
#   1. first-pass ESCALATE                     → verdict
#   2. after-ready, verdict ESCALATE           → verdict
#   3. after-ready, verdict illisible ×2       → architect_unreadable
#   4. after-iterate, verdict ESCALATE         → verdict
#   5. after-iterate, verdict illisible ×2     → architect_unreadable
#
# Le scan de cardinalité du bloc mika#2641 (S-c) exige en plus que chacun des
# cinq passe sa cause par la CONSTANTE : un site qui l'omettrait tomberait dans
# le défaut et produirait un `RESULT` plausible.
ITERATE_NOW=$(declare -f _iterate_groom_loop)
ESCALATE_CALL_COUNT=$(printf '%s\n' "$ITERATE_NOW" | grep -c '_escalate_groom')
assert_eq "_iterate_groom_loop has 5 _escalate_groom call sites (mika#2641)" "5" "$ESCALATE_CALL_COUNT"

# Each stage label appears exactly once
assert_contains "ESCALATE branch uses first-pass stage" '_escalate_groom "first-pass"' "$ITERATE_NOW"
assert_contains "READY-second-pass-fail uses second-pass-after-ready stage" '_escalate_groom "second-pass-after-ready"' "$ITERATE_NOW"
assert_contains "ITERATE-second-pass-fail uses second-pass-after-iterate stage" '_escalate_groom "second-pass-after-iterate"' "$ITERATE_NOW"

# Preservation invariant: ESCALATE never calls _cleanup_iterate_findings.
# Cleanup count must still be exactly 2 (the two GROOMED success paths only).
CLEANUP_COUNT=$(printf '%s\n' "$ITERATE_NOW" | grep -c '_cleanup_iterate_findings')
assert_eq "_iterate_groom_loop still has 2 cleanup calls (GROOMED-only; ESCALATE preserves)" "2" "$CLEANUP_COUNT"

# Robustness: _escalate_groom populates RESULT even when WORKTREE_DIR is unset
# (defensive — should not crash; findings file write is best-effort, RESULT
# marker is the mandatory product).
RESULT=""
WORKTREE_DIR="" _escalate_groom "first-pass" "content" "sess-x" 2>/dev/null
assert_contains "_escalate_groom populates RESULT even when WORKTREE_DIR unset" "GROOM ESCALATED (terminal)" "$RESULT"
assert_contains "_escalate_groom stamps its Outcome line even when WORKTREE_DIR unset (mika#2545)" \
    "Outcome: ESCALATE — first-pass" "$RESULT"

# ----------------------------------------------------------------------------
# Phase D — canonical body-callout writer (mika#1271 sub-PR 6)
# ----------------------------------------------------------------------------
# _write_canonical_callout is the forward-path body-callout writer called on
# GROOMED success. As of sub-PR 7b it is the sole structural authority for the
# body callout — the Class D recovery shim (formerly _verify_and_write_body_callout,
# mika#1123) was retired. Tests below assert: definition exists, two call sites
# in _iterate_groom_loop (one per GROOMED path), zero call sites on ESCALATE,
# stage labels produce distinct Grooming-history shapes, unknown stage returns 1,
# gh failure on idempotency check leaves no write.

# Definition exists
if declare -f _write_canonical_callout >/dev/null 2>&1; then
    assert_eq "_write_canonical_callout is defined" "ok" "ok"
else
    assert_eq "_write_canonical_callout is defined" "ok" "missing"
fi

# Code-shape: _iterate_groom_loop has exactly 2 _write_canonical_callout call
# sites (the two GROOMED success branches — one for READY-to-GROOMED and one
# for ITERATE-to-GROOMED). Zero call sites on ESCALATE branches.
CANONICAL_CALL_COUNT=$(printf '%s\n' "$ITERATE_NOW" | grep -c '_write_canonical_callout')
assert_eq "_iterate_groom_loop has 2 _write_canonical_callout call sites" "2" "$CANONICAL_CALL_COUNT"

# Stage labels match the GROOMED paths exactly
assert_contains "READY-to-GROOMED branch uses ready-to-groomed stage" '_write_canonical_callout "ready-to-groomed"' "$ITERATE_NOW"
assert_contains "ITERATE-to-GROOMED branch uses iterate-to-groomed stage" '_write_canonical_callout "iterate-to-groomed"' "$ITERATE_NOW"

# Preservation invariant: writer is called BEFORE cleanup (so callout writes
# even if cleanup fails) AND callout is non-fatal (|| <fallback>). The
# non-fatal shape evolved (mika#1412): older code used a `|| true`
# fall-through; current code uses `|| echo "WARN: canonical_callout_failed ..."`
# so the operator sees a diagnostic if the writer fails. Assertion updated
# to match the current canonical_callout_failed warning string.
assert_contains "ready-to-groomed callout non-fatal on write failure (canonical_callout_failed warn)" 'canonical_callout_failed' "$ITERATE_NOW"

# Unknown stage label → returns 1 (defensive contract).
RESULT=""
unknown_rc=0
(WORKTREE_DIR="/tmp" REPO="mika" ISSUE_NUM="999" BRANCH="test/branch" _write_canonical_callout "unknown-stage" "sess-unknown" 2>/dev/null) || unknown_rc=$?
assert_eq "_write_canonical_callout: unknown stage returns non-zero" "1" "$unknown_rc"

# Missing required env → returns 1 (defensive contract). WORKTREE_DIR unset.
no_wd_rc=0
(WORKTREE_DIR="" REPO="mika" ISSUE_NUM="999" BRANCH="test/branch" _write_canonical_callout "ready-to-groomed" "sess-x" 2>/dev/null) || no_wd_rc=$?
assert_eq "_write_canonical_callout: missing WORKTREE_DIR returns non-zero" "1" "$no_wd_rc"

# Missing REPO/ISSUE_NUM/BRANCH → returns 1
CC_TMP=$(mktemp -d)
no_repo_rc=0
(WORKTREE_DIR="$CC_TMP" REPO="" ISSUE_NUM="999" BRANCH="test/branch" _write_canonical_callout "ready-to-groomed" "sess-x" 2>/dev/null) || no_repo_rc=$?
assert_eq "_write_canonical_callout: missing REPO returns non-zero" "1" "$no_repo_rc"
rm -rf "$CC_TMP"

# Source-shape: writer source contains the canonical 3-line dispatch-gate shape
# (matches the Pin B / check_grooming_markers regex in executor.rs).
WRITER_SRC=$(declare -f _write_canonical_callout)
assert_contains "writer source contains Branch line" '> - **Branch:**' "$WRITER_SRC"
assert_contains "writer source contains Plan line with committed-on-branch" 'committed on branch @' "$WRITER_SRC"
assert_contains "writer source contains second-pass (GROOMED) marker for dispatch gate" 'second-pass (GROOMED)' "$WRITER_SRC"
assert_contains "writer source includes session_id in Grooming history" 'session-id: ${session_id}' "$WRITER_SRC"

# Stage-label-driven history lines
assert_contains "ready-to-groomed stage produces READY → GROOMED history" 'first-pass (READY) → second-pass (GROOMED)' "$WRITER_SRC"
assert_contains "iterate-to-groomed stage produces ITERATE → revised → GROOMED history" 'first-pass (ITERATE) → revised → second-pass (GROOMED)' "$WRITER_SRC"

# Idempotency check uses the dispatch-gate three-signal pattern from executor.rs::check_grooming_markers
assert_contains "writer reuses Pin B has_branch signal" 'has_branch=' "$WRITER_SRC"
assert_contains "writer reuses Pin B has_plan signal" 'has_plan=' "$WRITER_SRC"
assert_contains "writer reuses Pin B has_verdict signal" 'has_verdict=' "$WRITER_SRC"

# --- Single-pass grooming exit (mika#2012) ---
#
# The first-pass READY disposition is a legitimate grooming exit
# (/mika-groom-ticket Phase 3 step 10) that had no stage before mika#2012:
# _write_canonical_callout fell into `*)` and returned 1 silently, so no verdict
# was written and the ticket re-groomed forever. The history line deliberately
# does NOT claim `second-pass (GROOMED)` — no second pass ran, and a body that
# lies is a debt the next reader inherits. executor.rs::SINGLE_PASS_GROOMED_RE
# is the paired recognizer.
assert_contains "ready-single-pass stage exists" 'ready-single-pass)' "$WRITER_SRC"
assert_contains "ready-single-pass produces truthful single-pass history" 'first-pass (READY, single-pass GROOMED)' "$WRITER_SRC"
assert_contains "ready-single-pass does not fabricate a second pass" 'no second pass required' "$WRITER_SRC"

# Unknown stage must be operator-visible, not a silent return 1 — the silent
# failure IS the mika#2012 root cause.
CC_UNKNOWN_TMP=$(mktemp -d)
unknown_stderr=$( (WORKTREE_DIR="$CC_UNKNOWN_TMP" REPO="mika" ISSUE_NUM="999" BRANCH="test/branch" _write_canonical_callout "bogus-stage" "sess-unknown" 2>&1 >/dev/null) || true )
assert_contains "unknown stage emits greppable operator diagnostic" 'write_canonical_callout_unknown_stage' "$unknown_stderr"
assert_contains "unknown stage diagnostic names the consequence" 'NO VERDICT WRITTEN' "$unknown_stderr"
rm -rf "$CC_UNKNOWN_TMP"

# --- Callout replacement + path discipline (mika#2012 U3) ---

# Pin: the writer's idempotency pattern must stay in lockstep with executor.rs's
# three verdict regexes. Drift is not cosmetic — a form the gate accepts but this
# check misses makes the writer prepend a SECOND callout on every pass.
assert_contains "idempotency check knows canonical GROOMED (delimiter-tolerant)" 'second-pass \(GROOMED[[:space:]).,;:—-]' "$WRITER_SRC"
assert_contains "idempotency check knows paraphrased GROOMED" 'second-pass \(READY, paraphrased GROOMED' "$WRITER_SRC"
assert_contains "idempotency check knows single-pass GROOMED" 'first-pass \(READY, single-pass GROOMED' "$WRITER_SRC"

# The delimiter-tolerant form is the load-bearing fix: the OLD pattern was
# `second-pass \(GROOMED\)`, which required an immediate closing paren and so
# missed `second-pass (GROOMED — session-id: …)` — the exact shape this same
# function emits. That mismatch is the callout-stacking mechanism.
VERDICT_RE='second-pass \(GROOMED[[:space:])._,;:—-]|second-pass \(READY, paraphrased GROOMED|first-pass \(READY, single-pass GROOMED'
assert_eq "verdict pattern matches canonical em-dash session-id shape" "1" \
    "$(printf '%s' '> - **Grooming history:** first-pass (READY) → second-pass (GROOMED — session-id: abc)' | grep -cE "$VERDICT_RE" || true)"
assert_eq "verdict pattern matches single-pass shape" "1" \
    "$(printf '%s' '> - **Grooming history:** first-pass (READY, single-pass GROOMED) — no second pass required' | grep -cE "$VERDICT_RE" || true)"
assert_eq "verdict pattern does not match bare first-pass READY" "0" \
    "$(printf '%s' '> - **Grooming history:** first-pass (READY) — awaiting second pass' | grep -cE "$VERDICT_RE" || true)"
assert_eq "old pattern would have MISSED the em-dash shape (regression witness)" "0" \
    "$(printf '%s' '> - **Grooming history:** second-pass (GROOMED — session-id: abc)' | grep -cE 'second-pass \(GROOMED\)' || true)"

# Replacement, not stacking: existing callout lines are stripped before prepend.
assert_contains "writer strips existing callout lines" 'stripped_body=' "$WRITER_SRC"
assert_contains "writer strips all three callout line kinds" 'Branch|Plan|Grooming history' "$WRITER_SRC"
assert_contains "writer prepends onto the STRIPPED body, not the raw one" 'new_body=$(printf '"'"'%s\n\n%s'"'"' "$callout_block" "$stripped_body")' "$WRITER_SRC"

# Functional: a body carrying TWO stacked callout blocks (the mika#1962 shape)
# must come out with zero callout lines left.
STACKED_BODY='> - **Branch:** `fix/1962/new`
> - **Plan:** `docs/plans/new-plan.md` (committed on branch @ `bbb2222`)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED — session-id: two)

> - **Branch:** `fix/1962/old`
> - **Plan:** `docs/plans/stale-plan.md` (committed on branch @ `aaa1111`)
> - **Grooming history:** body callout recovered by post-flight (mika#1123)

## Symptom
Real content that must survive.'
STRIPPED_OUT=$(printf '%s' "$STACKED_BODY" | grep -vE '^> - \*\*(Branch|Plan|Grooming history):\*\*' | sed '/./,$!d')
assert_eq "stripping removes every callout line from a stacked body" "0" \
    "$(printf '%s' "$STRIPPED_OUT" | grep -cE '^> - \*\*(Branch|Plan|Grooming history):\*\*' || true)"
assert_contains "stripping preserves the real issue content" '## Symptom' "$STRIPPED_OUT"
assert_contains "stripping preserves body prose" 'Real content that must survive.' "$STRIPPED_OUT"
assert_not_contains "stripping drops the stale plan path" 'stale-plan.md' "$STRIPPED_OUT"

# Path discipline: the body must carry a repo-relative path that exists.
assert_contains "writer refuses a plan outside the worktree" 'write_canonical_callout_plan_outside_worktree' "$WRITER_SRC"
assert_contains "writer refuses a plan file that is absent" 'write_canonical_callout_plan_missing' "$WRITER_SRC"

# Review finding: the character class must mirror Rust's exactly. An extra
# member is not harmless — a body the writer thinks is stamped but the gate
# rejects re-groom-loops the ticket, which is #2012 in mirror image.
assert_not_contains "verdict class carries no member Rust lacks (underscore)" 'GROOMED[[:space:])._' "$WRITER_SRC"

# Review finding: the strip is preamble-scoped. A ticket that DOCUMENTS the
# callout format quotes these exact lines lower in the body — mika#2012's own
# issue does. A body-wide grep -v would delete that documentation.
DOCUMENTING_BODY='> - **Branch:** `fix/2012/gate`
> - **Plan:** `docs/plans/p.md` (committed on branch @ `abc1234`)
> - **Grooming history:** first-pass (READY, single-pass GROOMED) — session-id: x

## Symptom
The writer emits this shape:

```
> - **Branch:** `<branch>`
> - **Plan:** `<path>`
> - **Grooming history:** <verdict>
```

That block must survive body rewrites.'
PREAMBLE_STRIPPED=$(printf '%s' "$DOCUMENTING_BODY" | awk '
    BEGIN { preamble = 1 }
    preamble && /^> - \*\*(Branch|Plan|Grooming history):\*\*/ { next }
    preamble && /^[[:space:]]*$/ { next }
    { preamble = 0 }
    { print }
')
assert_eq "fixture carries 6 callout-shaped lines (3 preamble + 3 documented)" "6" \
    "$(printf '%s' "$DOCUMENTING_BODY" | grep -cE '^> - \*\*(Branch|Plan|Grooming history):\*\*' || true)"
assert_eq "documented callout lines inside the body SURVIVE the strip" "3" \
    "$(printf '%s' "$PREAMBLE_STRIPPED" | grep -cE '^> - \*\*(Branch|Plan|Grooming history):\*\*' || true)"
assert_contains "documenting body keeps its prose" 'must survive body rewrites' "$PREAMBLE_STRIPPED"
assert_contains "writer uses the preamble-scoped strip" 'preamble = 1' "$WRITER_SRC"

# The stacked-body case must still fully strip under the preamble-scoped rule.
STACKED_PREAMBLE_OUT=$(printf '%s' "$STACKED_BODY" | awk '
    BEGIN { preamble = 1 }
    preamble && /^> - \*\*(Branch|Plan|Grooming history):\*\*/ { next }
    preamble && /^[[:space:]]*$/ { next }
    { preamble = 0 }
    { print }
')
assert_eq "preamble strip still clears BOTH stacked blocks" "0" \
    "$(printf '%s' "$STACKED_PREAMBLE_OUT" | grep -cE '^> - \*\*(Branch|Plan|Grooming history):\*\*' || true)"
assert_contains "preamble strip preserves content after stacked blocks" '## Symptom' "$STACKED_PREAMBLE_OUT"

# Review finding: the gate must not read FETCH_HEAD. It is a single file shared
# by every process on the checkout, and mika#1001 allows a concurrent
# implement+groom pair on the same sub-repo. A sibling fetch between our fetch
# and our cat-file would test the wrong branch's tree.
PLAN_GATE_SRC=$(declare -f _committed_plan_on_branch)
assert_not_contains "gate does not read the shared FETCH_HEAD" 'FETCH_HEAD' "$PLAN_GATE_SRC"
assert_contains "gate fetches into a branch-named ref" 'refs/dispatch-gate/' "$PLAN_GATE_SRC"

# ----------------------------------------------------------------------------
# Redundant-groom refusal gate (mika#2012)
# ----------------------------------------------------------------------------
# _committed_plan_on_branch is the gate's evidence step: it answers "is there a
# committed plan on the dispatch branch?", NOT "does the body mention a plan?".
# The distinction is the whole ticket. A body-only grep would refuse grooming
# for a ticket whose plan was never pushed or was deleted — stranding it
# forever, which is a worse failure than the loop it fixes.

# The five cases below are DEFINED here next to the gate's code-shape
# assertions, but INVOKED near the end of this file — they need
# `_fixture_setup`, which is defined further down.
CALLOUT_PRESENT_BODY='## Symptom
Some text.

> - **Branch:** `fix/2012/plan-gate`
> - **Plan:** `docs/plans/2026-08-27-001-plan.md` (committed on branch @ `deadbeef`)
> - **Grooming history:** first-pass (READY, single-pass GROOMED) — no second pass required — session-id: t1
'

# --- Case 1 (the discriminating one): callout present, file ABSENT on branch.
# The gate must NOT fire — this ticket genuinely needs grooming.
test_groom_gate_callout_present_file_absent() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    # Branch exists on the remote but carries NO plan file.
    git -C "$FIXTURE_CLONE" checkout -q -b fix/2012/plan-gate
    echo "code" > "$FIXTURE_CLONE/src.txt"
    git -C "$FIXTURE_CLONE" add src.txt
    git -C "$FIXTURE_CLONE" commit -q -m "work, no plan"
    git -C "$FIXTURE_CLONE" push -q origin fix/2012/plan-gate

    local rc=0
    _committed_plan_on_branch "$FIXTURE_CLONE" "fix/2012/plan-gate" "$CALLOUT_PRESENT_BODY" "mika" >/dev/null 2>&1 || rc=$?

    _fixture_cleanup
    if [ "$rc" -ne 0 ]; then echo "PASS"; else echo "FAIL: gate fired on a callout whose plan file is absent — would strand the ticket"; fi
}

# --- Case 2: callout present AND file committed on branch → gate fires.
test_groom_gate_plan_committed() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    git -C "$FIXTURE_CLONE" checkout -q -b fix/2012/plan-gate
    mkdir -p "$FIXTURE_CLONE/docs/plans"
    echo "# plan" > "$FIXTURE_CLONE/docs/plans/2026-08-27-001-plan.md"
    git -C "$FIXTURE_CLONE" add docs/plans/2026-08-27-001-plan.md
    git -C "$FIXTURE_CLONE" commit -q -m "commit plan"
    git -C "$FIXTURE_CLONE" push -q origin fix/2012/plan-gate

    local out rc=0
    out=$(_committed_plan_on_branch "$FIXTURE_CLONE" "fix/2012/plan-gate" "$CALLOUT_PRESENT_BODY" "mika" 2>/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ] && [ "$out" = "docs/plans/2026-08-27-001-plan.md" ]; then
        echo "PASS"
    else
        echo "FAIL: expected rc=0 and the plan path, got rc=$rc out='$out'"
    fi
}

# --- Case 3: repo-prefixed callout form (`mika/docs/plans/...`) still resolves.
# Tickets groomed before U3's path normalization carry this shape.
test_groom_gate_repo_prefixed_path() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    git -C "$FIXTURE_CLONE" checkout -q -b fix/2012/prefixed
    mkdir -p "$FIXTURE_CLONE/docs/plans"
    echo "# plan" > "$FIXTURE_CLONE/docs/plans/legacy-plan.md"
    git -C "$FIXTURE_CLONE" add docs/plans/legacy-plan.md
    git -C "$FIXTURE_CLONE" commit -q -m "commit plan"
    git -C "$FIXTURE_CLONE" push -q origin fix/2012/prefixed

    local prefixed_body out rc=0
    prefixed_body='> - **Plan:** `mika/docs/plans/legacy-plan.md` (committed on branch @ `abc1234`)'
    out=$(_committed_plan_on_branch "$FIXTURE_CLONE" "fix/2012/prefixed" "$prefixed_body" "mika" 2>/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ] && [ "$out" = "docs/plans/legacy-plan.md" ]; then
        echo "PASS"
    else
        echo "FAIL: repo-prefixed callout should resolve to the relative path, got rc=$rc out='$out'"
    fi
}

# --- Case 4: no Plan callout at all → gate never fires (ungroomed ticket).
test_groom_gate_no_callout() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    local rc=0
    _committed_plan_on_branch "$FIXTURE_CLONE" "main" "## Symptom
Plain ungroomed body, no callout." "mika" >/dev/null 2>&1 || rc=$?

    _fixture_cleanup
    if [ "$rc" -ne 0 ]; then echo "PASS"; else echo "FAIL: gate fired on a body with no Plan callout"; fi
}

# --- Case 5: branch does not exist on the remote → gate must not fire.
test_groom_gate_branch_absent() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    local rc=0
    _committed_plan_on_branch "$FIXTURE_CLONE" "fix/2012/never-pushed" "$CALLOUT_PRESENT_BODY" "mika" >/dev/null 2>&1 || rc=$?

    _fixture_cleanup
    if [ "$rc" -ne 0 ]; then echo "PASS"; else echo "FAIL: gate fired although the dispatch branch does not exist on the remote"; fi
}

# ----------------------------------------------------------------------------
# Issue binding in the dispatch gate (mika#2034)
# ----------------------------------------------------------------------------
# Cases 1–5 above all run against a fixture whose `main` carries no plan files.
# That is precisely why they were green while the defect was live: the fixture
# could not express the production shape, where every dispatch branch descends
# from a `main` carrying 769 plans and therefore resolves ANY valid plan path.
# The gate read the path out of the ticket's own callout and asked only whether
# it resolved — an attestation produced by the claim it was meant to check.
#
# Measured 2026-08-30, both stranded by that gate:
#   mika#1887 → a plan whose header reads `issue: senara-solutions/mika#1933`
#   mika#2026 → a plan whose header reads `**Issue:** #539`
#
# Case 6 is the one that would have caught them.

# Writes a plan file on `main` whose header claims $2, and pushes. The branch in
# $3 is then created on top and adds NOTHING — so the plan resolves on it purely
# by inheritance, exactly as in production.
_fixture_foreign_plan_on_main() {
    local plan_rel="$1" claims_issue="$2" branch="$3"
    mkdir -p "$FIXTURE_CLONE/docs/plans"
    {
        printf -- '---\n'
        printf 'issue: senara-solutions/mika#%s\n' "$claims_issue"
        printf 'type: fix\n'
        printf -- '---\n\n'
        printf '# Plan — a plan belonging to another ticket\n\n'
        printf 'Padding so the file clears any size filter. %s\n' \
            "$(head -c 600 /dev/zero | tr '\0' 'x')"
    } > "$FIXTURE_CLONE/$plan_rel"
    git -C "$FIXTURE_CLONE" add "$plan_rel"
    git -C "$FIXTURE_CLONE" commit -q -m "plan for another ticket, on main"
    git -C "$FIXTURE_CLONE" push -q origin main

    git -C "$FIXTURE_CLONE" checkout -q -b "$branch"
    echo "code" > "$FIXTURE_CLONE/src.txt"
    git -C "$FIXTURE_CLONE" add src.txt
    git -C "$FIXTURE_CLONE" commit -q -m "work, no plan of its own"
    git -C "$FIXTURE_CLONE" push -q origin "$branch"
}

FOREIGN_PLAN_REL='docs/plans/2026-08-21-002-fix-1933-reader-completed-section-plan.md'
FOREIGN_PLAN_BODY='## Symptom
Some text.

> - **Branch:** `feat/1887/research-peer-b`
> - **Plan:** `docs/plans/2026-08-21-002-fix-1933-reader-completed-section-plan.md` (committed on branch @ `deadbeef`)
'

# --- Case 6 (the discriminating one): the plan the body names belongs to
# ANOTHER ticket and the branch merely inherited it from main. The gate must NOT
# fire — this ticket has never been groomed.
test_groom_gate_foreign_plan_inherited_from_main() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    _fixture_foreign_plan_on_main "$FOREIGN_PLAN_REL" 1933 "feat/1887/research-peer-b"

    local rc=0 err
    err=$(_committed_plan_on_branch "$FIXTURE_CLONE" "feat/1887/research-peer-b" \
        "$FOREIGN_PLAN_BODY" "mika" 1887 2>&1 >/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ]; then
        echo "FAIL: gate fired on a plan whose header claims issue 1933 while grooming 1887 — this is the mika#1887 / mika#2026 strand"
    elif ! grep -q -- 'dispatch_gate_groom_plan_refuted' <<<"$err"; then
        echo "FAIL: gate declined but emitted no greppable refusal diagnostic; got: $err"
    elif ! grep -q -- '1933' <<<"$err"; then
        echo "FAIL: refusal diagnostic does not name the issue the plan claims; got: $err"
    else
        echo "PASS"
    fi
}

# --- Case 7: same file, but its header claims the TARGET issue → gate fires.
# Without this, case 6 could be satisfied by refusing everything.
test_groom_gate_plan_header_claims_target_issue() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    _fixture_foreign_plan_on_main "$FOREIGN_PLAN_REL" 1887 "feat/1887/research-peer-b"

    local out rc=0
    out=$(_committed_plan_on_branch "$FIXTURE_CLONE" "feat/1887/research-peer-b" \
        "$FOREIGN_PLAN_BODY" "mika" 1887 2>/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ] && [ "$out" = "$FOREIGN_PLAN_REL" ]; then
        echo "PASS"
    else
        echo "FAIL: a plan whose header claims the target issue must still fire the gate, got rc=$rc out='$out'"
    fi
}

# --- Case 8: the plan carries NO issue marker at all → gate still fires.
# Refutation, not confirmation: 95 of the 745 plans in docs/plans/ name no
# issue, and demanding a positive match would strand every one of them.
test_groom_gate_plan_without_issue_marker_still_fires() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    mkdir -p "$FIXTURE_CLONE/docs/plans"
    printf '# Plan — no issue marker anywhere in this header\n\nBody.\n' \
        > "$FIXTURE_CLONE/$FOREIGN_PLAN_REL"
    git -C "$FIXTURE_CLONE" add "$FOREIGN_PLAN_REL"
    git -C "$FIXTURE_CLONE" commit -q -m "markerless plan on main"
    git -C "$FIXTURE_CLONE" push -q origin main
    git -C "$FIXTURE_CLONE" checkout -q -b feat/1887/research-peer-b
    git -C "$FIXTURE_CLONE" push -q origin feat/1887/research-peer-b

    local out rc=0
    out=$(_committed_plan_on_branch "$FIXTURE_CLONE" "feat/1887/research-peer-b" \
        "$FOREIGN_PLAN_BODY" "mika" 1887 2>/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ] && [ "$out" = "$FOREIGN_PLAN_REL" ]; then
        echo "PASS"
    else
        echo "FAIL: silence is not refutation — a markerless plan must still fire the gate, got rc=$rc out='$out'"
    fi
}

# --- Case 9: the four existing call shapes keep working with no 5th argument.
# KTD2 — the parameter is optional; a required one would break all five cases
# above and both production call sites.
test_groom_gate_issue_arg_is_optional() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    _fixture_foreign_plan_on_main "$FOREIGN_PLAN_REL" 1933 "feat/1887/research-peer-b"

    # No 5th argument and no ambient ISSUE_NUM: the binding check is skipped
    # rather than guessed, so the gate behaves exactly as it did before mika#2034.
    local out rc=0
    out=$(ISSUE_NUM='' _committed_plan_on_branch "$FIXTURE_CLONE" "feat/1887/research-peer-b" \
        "$FOREIGN_PLAN_BODY" "mika" 2>/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ] && [ "$out" = "$FOREIGN_PLAN_REL" ]; then
        echo "PASS"
    else
        echo "FAIL: the 5th argument must be optional, got rc=$rc out='$out'"
    fi
}

# --- Case 10: provenance is measured, not asserted. A blob identical to main's
# is reported as inherited; one the branch actually added is reported as
# committed. This is the sentence the operator reads.
test_plan_provenance_distinguishes_inherited_from_committed() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    _fixture_foreign_plan_on_main "$FOREIGN_PLAN_REL" 1933 "feat/1887/research-peer-b"

    # Prime the gate ref the same way _committed_plan_on_branch does.
    git -C "$FIXTURE_CLONE" fetch --quiet --force origin \
        "refs/heads/feat/1887/research-peer-b:refs/dispatch-gate/feat/1887/research-peer-b" 2>/dev/null

    local inherited committed own_plan='docs/plans/2026-08-30-001-fix-2034-own-plan.md'
    inherited=$(_plan_provenance "$FIXTURE_CLONE" "feat/1887/research-peer-b" "$FOREIGN_PLAN_REL")

    printf -- '---\nissue: senara-solutions/mika#1887\n---\n\nOwn plan.\n' \
        > "$FIXTURE_CLONE/$own_plan"
    git -C "$FIXTURE_CLONE" add "$own_plan"
    git -C "$FIXTURE_CLONE" commit -q -m "this branch's own plan"
    git -C "$FIXTURE_CLONE" push -q origin feat/1887/research-peer-b
    git -C "$FIXTURE_CLONE" fetch --quiet --force origin \
        "refs/heads/feat/1887/research-peer-b:refs/dispatch-gate/feat/1887/research-peer-b" 2>/dev/null
    committed=$(_plan_provenance "$FIXTURE_CLONE" "feat/1887/research-peer-b" "$own_plan")

    _fixture_cleanup
    if ! grep -q -- 'inherited unchanged from main' <<<"$inherited"; then
        echo "FAIL: a blob identical to main's must be reported as inherited, got '$inherited'"
    elif ! grep -q -- 'committed on the dispatch branch' <<<"$committed"; then
        echo "FAIL: a blob the branch added must be reported as committed, got '$committed'"
    else
        echo "PASS"
    fi
}

# --- Case 11: a callout naming a DIRECTORY must not fire the gate.
# Written to pin the unbindable-candidate path and it found a real one instead:
# `cat-file -e` is satisfied by a tree, and `git show` on a tree prints a
# listing — non-empty, carrying no issue header, therefore "not refuted". A
# ticket whose callout said `docs/plans` was refused grooming on the strength of
# a directory. The gate now demands a blob.
test_groom_gate_declines_on_directory_candidate() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    _fixture_foreign_plan_on_main "$FOREIGN_PLAN_REL" 1933 "feat/1887/research-peer-b"

    local rc=0
    _committed_plan_on_branch "$FIXTURE_CLONE" "feat/1887/research-peer-b" \
        "> - **Plan:** \`docs/plans\` (committed on branch @ \`deadbeef\`)" \
        "mika" 1887 >/dev/null 2>&1 || rc=$?

    _fixture_cleanup
    if [ "$rc" -ne 0 ]; then
        echo "PASS"
    else
        echo "FAIL: a directory is not a plan — the gate must not fire on one"
    fi
}

# ----------------------------------------------------------------------------
# mika#2158 — le croisement contractuel Rust ↔ Bash (AC7)
# ----------------------------------------------------------------------------
#
# Trois composants répondent à « ce ticket est-il groomé ? » : `is_groomed`
# (promotion, Rust), `check_grooming_markers` (routage du dispatch, Rust) et
# cette garde (refus, Bash). Les deux premiers sont croisés entre eux dans
# `grooming_marker::tests::ac7_both_rust_predicates_agree_on_the_frozen_bodies`.
# Le troisième ne peut pas l'être au même endroit : il mesure un artefact git
# que le Rust ne peut pas évaluer sans I/O.
#
# Le croisement testable est donc unidirectionnel, et c'est celui qui compte :
#
#     si la garde refuse pour `already_groomed`, alors le Rust doit dire groomé.
#
# C'est exactement l'implication qui était fausse avant mika#2158, et c'est ce
# qui faisait le livelock : la garde refusait le grooming de #2108/#2127 parce
# que leur plan était sur la branche, pendant que le Rust les déclarait non
# groomés et les redemandait en grooming toutes les dix minutes. Douze refus en
# trois heures et demie, sans qu'aucun état ne change entre deux tours.
#
# L'implication INVERSE n'est pas vraie et ne doit pas être testée : un ticket
# peut être groomé et son plan absent de la branche (jamais poussé), et c'est le
# cas que la branche `elif` du site d'appel traite déjà.
#
# La moitié Rust est vérifiée ici structurellement, pas paraphrasée : le test
# échoue si la fixture n'est plus dans la table Rust. Retirer `2108.md` de
# `grooming_marker.rs` casse ce test — ce qui est le point.
GROOMING_FIXTURE_DIR="$SCRIPT_DIR/../../../crates/mika-agent/tests/fixtures/grooming_bodies"
GROOMING_MARKER_RS="$SCRIPT_DIR/../../../crates/mika-agent/src/grooming_marker.rs"

test_groom_gate_refusal_implies_rust_says_groomed() {
    local ticket fixture body plan_path branch rc out problems=""

    if [ ! -d "$GROOMING_FIXTURE_DIR" ]; then
        echo "FAIL: fixture dir absent: $GROOMING_FIXTURE_DIR"
        return 0
    fi
    if [ ! -f "$GROOMING_MARKER_RS" ]; then
        echo "FAIL: grooming_marker.rs absent: $GROOMING_MARKER_RS"
        return 0
    fi

    # Les deux corps que la garde refusait pendant que le Rust les redemandait :
    # #2108 (première passe READY, AC1) et #2127 (GROOMED après arbitrage, AC3).
    for ticket in 2108 2127; do
        fixture="$GROOMING_FIXTURE_DIR/$ticket.md"
        if [ ! -f "$fixture" ]; then
            problems="$problems; fixture $ticket.md absente"
            continue
        fi
        # Moitié Rust : la fixture doit être dans la table du module.
        if ! grep -qF "grooming_bodies/$ticket.md" "$GROOMING_MARKER_RS"; then
            problems="$problems; $ticket.md n'est plus dans la table Rust de grooming_marker.rs"
            continue
        fi

        body=$(cat "$fixture")
        plan_path=$(printf '%s\n' "$body" \
            | sed -n 's/^> - \*\*Plan:\*\* *`\([^`]*\)`.*/\1/p' | head -1)
        branch=$(printf '%s\n' "$body" \
            | sed -n 's/^> - \*\*Branch:\*\* *`\([^`]*\)`.*/\1/p' | head -1)
        if [ -z "$plan_path" ] || [ -z "$branch" ]; then
            problems="$problems; $ticket.md ne porte pas les callouts Branch/Plan"
            continue
        fi

        _fixture_setup
        _assert_fixture_is_local || return 1

        git -C "$FIXTURE_CLONE" checkout -q -b "$branch"
        mkdir -p "$FIXTURE_CLONE/$(dirname "$plan_path")"
        printf '# Plan — mika#%s\n' "$ticket" > "$FIXTURE_CLONE/$plan_path"
        git -C "$FIXTURE_CLONE" add "$plan_path"
        git -C "$FIXTURE_CLONE" commit -q -m "commit plan for $ticket"
        git -C "$FIXTURE_CLONE" push -q origin "$branch"

        rc=0
        out=$(_committed_plan_on_branch "$FIXTURE_CLONE" "$branch" "$body" "mika" "$ticket" 2>/dev/null) || rc=$?
        _fixture_cleanup

        if [ "$rc" -ne 0 ] || [ "$out" != "$plan_path" ]; then
            problems="$problems; $ticket: la garde n'a pas refusé (rc=$rc out='$out', attendu '$plan_path')"
        fi
    done

    if [ -z "$problems" ]; then
        echo "PASS"
    else
        echo "FAIL${problems}"
    fi
}

# Code-shape: the binding must not be quietly dropped by a later edit. The gate
# is the only thing standing between a false body callout and a permanent
# strand, so its call to the refutation helper is an assertion of its own.
PLAN_GATE_SRC_2034=$(declare -f _committed_plan_on_branch)
assert_contains "gate binds the candidate to the issue before believing it" \
    '_plan_header_refutes_issue' "$PLAN_GATE_SRC_2034"
assert_contains "gate names the issue the refuted plan claims" \
    '_plan_header_claimed_issues' "$PLAN_GATE_SRC_2034"
assert_contains "gate emits a greppable refusal diagnostic" \
    'dispatch_gate_groom_plan_refuted' "$PLAN_GATE_SRC_2034"
# Provenance lives outside the gate on purpose: every call site invokes the gate
# in a command substitution, and a subshell cannot set a variable in its parent.
assert_not_contains "gate keeps stdout to the plan path alone" \
    'COMMITTED_PLAN_PROVENANCE=' "$PLAN_GATE_SRC_2034"
PLAN_PROVENANCE_SRC=$(declare -f _plan_provenance)
assert_contains "provenance compares the branch blob against main's" \
    'origin/main:' "$PLAN_PROVENANCE_SRC"

# --- mika#2606: the reader's two patterns have ONE definition site ---
# `_plan_header_claimed_issues` reads a plan header by two branches: a `grep`
# for `#N`, and a `sed` that strips the prefix so a `grep` can see a BARE
# numeric value (`issue: 1679`, mika#1617). Each used to carry its own copy of
# the prefix. Widening one alone leaves the other blind on exactly the lines the
# first just admitted — measured on mika#2606: `issue : 1679`, `> issue: 1679`
# and `> issue : 1679` all return empty while EVERY `#N` fixture stays green.
# A half-fix is therefore green on the whole `#N` population and silently broken
# only on the bare-numeric shapes, which is this ticket's own failure class one
# notch later. A behavioural test cannot catch the class: recopying the prefix
# makes no decision wrong on the day it is written.
# `|| true` so a rename produces a NAMED failure instead of an unexplained
# abort: under `set -e`, `declare -f` on a missing function kills the suite
# before the anti-vacuity assertion below can say what went wrong. Verified by
# renaming the function — without it the run stops with exit 1 and no diagnostic.
CLAIMED_SRC_2606=$(declare -f _plan_header_claimed_issues || true)

# Anti-vacuity FIRST. Without it, a rename or deletion makes every assertion
# below pass against an empty string, and a scan that looks at nothing reads
# exactly like a clean tree — literally the mika#2205 class this ticket closes.
assert_contains "mika#2606: the header reader still exists (anti-vacuity)" \
    '_plan_header_claimed_issues (' "$CLAIMED_SRC_2606"

assert_contains "mika#2606: the line prefix has one definition" \
    'local hdr=' "$CLAIMED_SRC_2606"
assert_contains "mika#2606: the label/value separator has one definition" \
    'local sep=' "$CLAIMED_SRC_2606"
assert_contains "mika#2606: the grep interpolates both, never its own copy" \
    '${hdr}(ticket|issue|number)${sep}' "$CLAIMED_SRC_2606"
assert_contains "mika#2606: the sed interpolates both, never its own copy" \
    '${hdr}[A-Za-z]+${sep}' "$CLAIMED_SRC_2606"

# The teeth: each literal may appear exactly once — inside its own definition.
# A future editor who inlines the prefix back into either pattern goes red here
# instead of reopening the desynchronisation.
assert_eq "mika#2606: the dash-prefix literal appears once (in hdr alone)" \
    "1" "$(grep -oF -- '(-[[:space:]]+)?' <<<"$CLAIMED_SRC_2606" | wc -l | tr -d '[:space:]')"
assert_eq "mika#2606: the colon literal appears once (in sep alone)" \
    "1" "$(grep -oF -- '(:\*\*|:)' <<<"$CLAIMED_SRC_2606" | wc -l | tr -d '[:space:]')"

# Code-shape: the gate must use mika#988 exit semantics — _deliver_callback +
# exit 0, never exit 1. An `exit 1` here is wrapped as HANDLER CRASH by the EXIT
# trap and stalls the loop (7 h on 2026-05-06).
SETUP_WT_SRC=$(declare -f _set_up_worktree)
assert_contains "groom refusal gate is scoped to dev-groom" '[ "$SKILL" = "dev-groom" ]' "$SETUP_WT_SRC"
assert_contains "groom refusal gate calls _committed_plan_on_branch" '_committed_plan_on_branch' "$SETUP_WT_SRC"
assert_contains "groom refusal delivers a structured callback" 'already_groomed' "$SETUP_WT_SRC"
assert_contains "groom refusal emits a greppable operator diagnostic" 'dispatch_gate_groom_refused' "$SETUP_WT_SRC"

# --- Re-grooming visibility (mika#2012 U4) ---
# A second grooming of the same ticket must not read like a first one. When the
# gate correctly declines to fire but the body still carries a Plan callout, the
# run is a RE-groom on a stale claim — it proceeds, but says so distinctly so a
# grep separates the two populations.
assert_contains "allowed-but-stale re-groom emits its own signal" 'dispatch_gate_groom_allowed_stale_callout' "$SETUP_WT_SRC"
assert_not_contains "the stale-callout signal is not the refusal signal reused" \
    'dispatch_gate_groom_refused: repo=${REPO} issue=${ISSUE_NUM} branch=${BRANCH} — issue body carries' "$SETUP_WT_SRC"

# The refusal RESULT must be machine-readable: mika-dev's callback turn and the
# audit dashboard both consume it. Rebuild it with the same printf and prove jq
# can reach every field.
REFUSAL_JSON=$(printf '{"status":"auto_skipped","reason":"already_groomed","issue":"senara-solutions/%s#%s","branch":"%s","plan":"%s","note":"A committed plan already exists on the dispatch branch. Re-grooming would re-derive it and stack a second body callout. Do NOT dispatch dev-pilot: the provenance gate refuses it. Remove the plan from the branch AND the grooming callouts from the issue body, then let the loop re-groom it."}' \
    "mika" "2012" "fix/2012/plan-gate" "docs/plans/2026-08-27-001-plan.md")
assert_eq "refusal RESULT is valid JSON" "0" "$(printf '%s' "$REFUSAL_JSON" | jq -e . >/dev/null 2>&1; echo $?)"
assert_eq "refusal RESULT exposes .reason to jq" "already_groomed" "$(printf '%s' "$REFUSAL_JSON" | jq -r '.reason')"
assert_eq "refusal RESULT exposes .status to jq" "auto_skipped" "$(printf '%s' "$REFUSAL_JSON" | jq -r '.status')"
assert_eq "refusal RESULT exposes .plan to jq" "docs/plans/2026-08-27-001-plan.md" "$(printf '%s' "$REFUSAL_JSON" | jq -r '.plan')"
assert_eq "refusal RESULT exposes .branch to jq" "fix/2012/plan-gate" "$(printf '%s' "$REFUSAL_JSON" | jq -r '.branch')"
assert_contains "the refusal printf in the source matches the shape tested here" \
    '{"status":"auto_skipped","reason":"already_groomed"' "$SETUP_WT_SRC"

# --- mika#2484 U5 : la note ne prescrit plus une route morte ---
# Le texte disait « Dispatch dev-pilot to implement ». Depuis mika#2287 cette
# moitié mène droit à `dispatch_grooming_not_verified` : la porte exige un
# callback groom terminé portant `Outcome: PLAN_GROOMED`, et un
# `already_groomed` n'en frappe aucun — délibérément (une garde qui lit sa
# preuve de la revendication ne peut pas la réfuter). Un texte de remède qui
# nomme une route morte coûte un tour de boucle et une lecture.
assert_not_contains "the already_groomed note no longer prescribes dev-pilot" \
    'Dispatch dev-pilot to implement' "$SETUP_WT_SRC"
assert_contains "the already_groomed note names the gesture that works" \
    'remove the plan from the branch AND the grooming callouts from the issue body' \
    "$SETUP_WT_SRC"

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
