#!/bin/bash
# La porte de sortie non vide (mika#1996) — extrait de test-dispatch-lib.sh par mika#3178, vol 3.
# Le nom ne décrit que la seconde moitié de la plage, et c'est assumé : elle
# OUVRE sur l'allowlist de dépôts dispatchables en défense de profondeur shell
# (mika#2062) et n'atteint mika#1996 qu'à sa ligne 162. Elle porte aussi l'ancre
# élargie au début de COMMANDE (mika#2165) et les compteurs de relances de
# mika#2306 et mika#2636.
#
# Plage d'origine : lignes 6680–7321 de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 8de06d66. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
# ============================================================================
# Dispatchable-repo allowlist: shell defense in depth (mika#2062)
# ============================================================================
# Follow-up of #2046 (KTD5). The Rust tool-boundary guard is load-bearing on the
# nominal loop path; _set_up_worktree adds a shell copy of the same allowlist for
# out-of-engine callers (manual runs, scripts, recovery paths) that never reach
# the Rust guard. These tests are anti-vacuity in both directions: an allowlisted
# repo must pass the gate, a non-allowlisted one must be refused.

echo ""
echo "Test: dispatchable-repo allowlist gate (mika#2062)"
echo "--------------------------------------------------"

# --- Predicate _is_dispatchable_repo: pure, both directions ---
assert_eq "_is_dispatchable_repo accepts mika"          "0" "$(_is_dispatchable_repo mika; echo $?)"
assert_eq "_is_dispatchable_repo accepts mika-cloud"    "0" "$(_is_dispatchable_repo mika-cloud; echo $?)"
assert_eq "_is_dispatchable_repo accepts mika-skills"   "0" "$(_is_dispatchable_repo mika-skills; echo $?)"
assert_eq "_is_dispatchable_repo accepts mika-platform" "0" "$(_is_dispatchable_repo mika-platform; echo $?)"
assert_eq "_is_dispatchable_repo refuses control-monitor" "1" "$(_is_dispatchable_repo control-monitor; echo $?)"
assert_eq "_is_dispatchable_repo refuses claude-pilot"    "1" "$(_is_dispatchable_repo claude-pilot; echo $?)"
assert_eq "_is_dispatchable_repo refuses empty"           "1" "$(_is_dispatchable_repo '' 2>/dev/null; echo $?)"

# --- Integration: the gate inside _set_up_worktree, both directions ---
# Hermetic probe (no gh, no network). PLATFORM_DIR is an EMPTY temp dir, so an
# allowlisted repo clears the gate and then trips the "is not a git repository"
# check that immediately follows SUB_REPO_DIR resolution — proof it got past the
# gate. A non-allowlisted repo is refused AT the gate with `repo_not_dispatchable`
# and never reaches that check. Were the gate removed, control-monitor would also
# reach the git check (verdict "passed-gate") and this test would fail — which is
# the anti-vacuity guarantee.
PROBE_PLATFORM=$(mktemp -d)

_repo_gate_verdict() {
    local out
    out=$(
        (
            PROMPT="$1"
            PLATFORM_DIR="$PROBE_PLATFORM"
            PLATFORM_REPO_NAME="mika-platform"  # so the PLATFORM_DIR self-branch is not taken for mika/mika-cloud
            TASK_ID="probe-2062"
            _set_up_worktree
        ) 2>&1 || true
    )
    if grep -q -- 'repo_not_dispatchable' <<<"$out"; then
        printf 'refused'
    elif grep -q -- 'is not a git repository' <<<"$out"; then
        printf 'passed-gate'
    else
        printf 'unexpected'
    fi
}

assert_eq "mika#N clears the gate, reaches worktree setup"       "passed-gate" "$(_repo_gate_verdict 'mika#214')"
assert_eq "mika-cloud#N clears the gate, reaches worktree setup" "passed-gate" "$(_repo_gate_verdict 'mika-cloud#50')"
assert_eq "control-monitor#N is refused at the gate"             "refused"     "$(_repo_gate_verdict 'control-monitor#159')"
assert_eq "claude-pilot#N is refused at the gate"                "refused"     "$(_repo_gate_verdict 'claude-pilot#119')"

# The refusal names the repo in a machine-readable field (mika#2062), consistent
# with `repo_not_dispatchable` on the Rust side.
REFUSAL_OUT=$(
    (
        PROMPT="control-monitor#159"
        PLATFORM_DIR="$PROBE_PLATFORM"
        PLATFORM_REPO_NAME="mika-platform"
        TASK_ID="probe-2062"
        _set_up_worktree
    ) 2>&1 || true
)
assert_contains "refusal is named repo_not_dispatchable" 'repo_not_dispatchable' "$REFUSAL_OUT"
assert_contains "refusal carries the repo in a machine-readable field" '"repo":"senara-solutions/control-monitor"' "$REFUSAL_OUT"

rm -rf "$PROBE_PLATFORM"

# --- Drift guard: the shell copy must equal the Rust DISPATCHABLE_REPOS ---
# The list is duplicated across two languages by necessity (the Rust side is a
# compile-time &[&str], not a runtime-readable data file). Two mute copies are
# exactly the drift webhook_dispatch.rs exists to prevent, so this fails if they
# diverge (mika#2062).
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
RUST_SRC="$REPO_ROOT/crates/mika-agent/src/webhook_dispatch.rs"
if [ -f "$RUST_SRC" ]; then
    RUST_LIST=$(sed -n '/const DISPATCHABLE_REPOS/,/\];/p' "$RUST_SRC" \
        | grep -oE '"senara-solutions/[^"]+"' \
        | sed 's#"senara-solutions/##; s#"##' \
        | sort | paste -sd' ' -)
    SHELL_LIST=$(printf '%s\n' "${DISPATCHABLE_REPO_BASENAMES[@]}" | sort | paste -sd' ' -)
    assert_eq "shell allowlist matches Rust DISPATCHABLE_REPOS (no drift)" "$RUST_LIST" "$SHELL_LIST"
fi

# --- Garde de dérive du marqueur, INVERSÉE par la bascule (mika#2194, vol 5) ---
#
# CE QU'ELLE GARDAIT. Le littéral du marqueur de disposition retenue existait en
# DEUX copies, une par langue, par nécessité (le côté moteur est un `&str` de
# compilation, pas un fichier de données lisible à l'exécution). L'enjeu dépassait
# le bruit de dérive : si les deux copies divergeaient, le moteur continuait de
# retirer la disposition mais le tier 0 cessait de reconnaître le substitut, et
# tous les tiers en dessous se remettaient à dériver un verdict du corps. Le refus
# échouait OUVERT, en silence (mika#2037). La garde assérait donc que le bash
# PORTAIT le littéral.
#
# CE QU'ELLE GARDE DEPUIS. Le vol 5 a retiré le tier 0 du bash : il n'y a plus
# deux copies, il y en a une, et c'est la constante Rust. La garde est donc
# retournée — elle assère que le littéral n'est PLUS dans le bash. Et les deux
# moitiés tiennent ensemble :
#   - la supprimer verdirait un arbre où le tier 0 aurait été RECOPIÉ ailleurs
#     dans le bash (le retour que l'AC4 vise) ;
#   - sa première moitié seule verdirait un arbre où le littéral aurait disparu
#     DES DEUX CÔTÉS, c'est-à-dire où le tier 0 n'existerait plus du tout.
# D'où la seconde moitié : la constante Rust a un lecteur, et c'est le moteur.
#
# L'extraction par `grep -oE 'const DISPOSITION_WITHHELD_MARKER: &str = "[^"]+"'`
# est conservée telle quelle : non ancrée, elle survit au `pub(crate)` posé au
# vol 4.
#
# Noter l'asymétrie avec le tier 0b, juste en dessous : `_engine_escalation_line`
# est CONSERVÉE dans le bash (second appelant de production, `_escalate_groom`,
# mika#2338), donc sa moitié « le bash porte le littéral » reste dans son sens
# d'origine. Les deux tiers n'ont pas basculé ensemble, et le test le dit.
ANCHOR_RUST_SRC="$REPO_ROOT/crates/mika-agent/src/agent_loop/mod.rs"
if [ -f "$ANCHOR_RUST_SRC" ]; then
    RUST_MARKER=$(grep -oE 'const DISPOSITION_WITHHELD_MARKER: &str = "[^"]+"' "$ANCHOR_RUST_SRC" \
        | sed 's/.*= "//; s/"$//')
    # Moitié 1 — le littéral a bien quitté le bash. `$WITHHELD_MARKER` reste
    # défini par cette suite : c'est l'entrée des cas de tier 0 ci-dessus, qui
    # exercent désormais le moteur à travers les enveloppes.
    assert_eq "mika#2194: le littéral du tier 0 n'est plus dans dispatch-lib.sh" "no" \
        "$(if grep -qF "$RUST_MARKER" "$DISPATCH_LIB"; then printf 'yes'; else printf 'no'; fi)"
    # Moitié 2 — anti-vacuité de la précédente : le marqueur existe toujours, et
    # il a un lecteur unique. Un arbre où la constante a disparu des deux côtés
    # passerait la moitié 1 et rougit ici.
    assert_eq "mika#2194: la constante Rust du tier 0 existe (lecteur unique)" "yes" \
        "$(if [ -n "$RUST_MARKER" ]; then printf 'yes'; else printf 'no'; fi)"
    assert_eq "mika#2194: le moteur est le lecteur du marqueur du tier 0" "yes" \
        "$(if grep -rqF "DISPOSITION_WITHHELD_MARKER" "$REPO_ROOT/crates/mika-agent/src/arch_disposition.rs" 2>/dev/null; then printf 'yes'; else printf 'no'; fi)"
    # mika#2338 — same necessity, same shape, for the engine escalation line. If the two
    # copies diverge, tier 0b stops recognizing the engine's line and the escalation falls
    # through to the unanchored textual tiers.
    RUST_ENGINE_MARKER=$(grep -oE 'const REVIEW_ANCHOR_ENGINE_FINDING_MARKER: &str = "[^"]+"' "$ANCHOR_RUST_SRC" \
        | sed 's/.*= "//; s/"$//')
    assert_eq "shell tier-0b marker matches Rust REVIEW_ANCHOR_ENGINE_FINDING_MARKER (no drift)" \
        "$RUST_ENGINE_MARKER" "$ENGINE_FINDING_MARKER"
    if grep -qF "$RUST_ENGINE_MARKER" "$DISPATCH_LIB"; then
        assert_eq "dispatch-lib carries the engine marker literal" "yes" "yes"
    else
        assert_eq "dispatch-lib carries the engine marker literal" "yes" "no"
    fi
    # The literal check above is satisfied by a comment; the RECOGNIZER stores the marker
    # as an escaped regex. Bind the regex itself to the Rust constant by feeding a line
    # composed exactly as escalate_unattested_disposition composes it (`<prefix> <MARKER> …`)
    # through _engine_escalation_line — a regex-only edit can no longer stay green.
    RUST_COMPOSED_LINE="F1: $RUST_ENGINE_MARKER attestation withheld after the corrective re-prompt — anchors_found=3, anchors_valid=2, miss_reason=QuoteNotInBrief: x"
    assert_eq "tier-0b regex recognizes a line composed from the Rust constant" "$RUST_COMPOSED_LINE" \
        "$(printf 'prose\n%s\nDisposition: ESCALATE\n' "$RUST_COMPOSED_LINE" | _engine_escalation_line)"
    assert_eq "tier-0b regex rejects the Rust constant when it is not at line start" "" \
        "$(printf 'the engine would write %s here\nDisposition: READY\n' "$RUST_ENGINE_MARKER" | _engine_escalation_line)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Rust source not found at $RUST_SRC — cannot verify allowlist drift"
fi

# ============================================================================
# Non-empty output gate (mika#1996)
# ============================================================================
#
# A cycle that produced nothing may no longer report success — and, just as
# load-bearing, a cycle that produced something must come through the gate
# untouched. A gate that only ever fails is satisfied by "always fail", which
# is worth exactly as much as the silent success it replaces. Both directions
# are asserted below, plus the third verdict (`undetermined`) that keeps the
# gate from inventing a red when it has no ground to measure.

echo ""
echo "Test: non-empty output gate — measurement (mika#1996)"
echo "-------------------------------------------------------------------"

# Builds the exact state the gate reads: a real git worktree, a real HEAD range,
# a real stderr file with a real number of `[tool:request]` lines. Echoes either
# the verdict (mode=verdict) or the resulting RESULT (mode=result).
_gate_probe() {
    local mode="$1" dirty="$2" moved="$3" pr="$4" tools="$5" result_in="$6" wt_mode="${7:-real}" pilot_ran="${8:-1}"
    local base wt logdir pre post i
    base=$(mktemp -d "${TMPDIR:-/tmp}/mika-1996-test.XXXXXX")
    wt="$base/wt"; logdir="$base/logs"
    mkdir -p "$logdir"
    git init -q "$wt" 2>/dev/null
    echo initial > "$wt/f"
    git -C "$wt" add -A 2>/dev/null
    git -C "$wt" commit -q -m initial --no-verify 2>/dev/null
    pre=$(git -C "$wt" rev-parse HEAD)
    post="$pre"
    case "$moved" in
        content)
            echo more > "$wt/g"
            git -C "$wt" add -A 2>/dev/null
            git -C "$wt" commit -q -m "feat: real work" --no-verify 2>/dev/null
            post=$(git -C "$wt" rev-parse HEAD)
            ;;
        empty)
            # The wip(mika#1383) rescue marker is an --allow-empty commit by
            # construction: it moves HEAD without producing anything.
            git -C "$wt" commit -q --allow-empty --no-verify \
                -m "wip(mika#1383): auto-PR-create rescue" 2>/dev/null
            post=$(git -C "$wt" rev-parse HEAD)
            ;;
    esac
    if [ "$dirty" = "yes" ]; then
        echo uncommitted > "$wt/pilot-wrote-this.rs"
    fi
    if [ "$tools" != "none" ]; then
        : > "$logdir/probe-1996.stderr"
        i=0
        while [ "$i" -lt "$tools" ]; do
            echo "[tool:request] Bash {\"command\":\"ls\"}" >> "$logdir/probe-1996.stderr"
            i=$((i + 1))
        done
    fi
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        WORKTREE_DIR="$wt"
        [ "$wt_mode" = "absent" ] && WORKTREE_DIR="$base/never-created"
        PRE_RUN_HEAD="$pre"
        POST_RUN_HEAD="$post"
        PR_URL="$pr"
        LOG_ID="probe-1996"
        PILOT_LOG_DIR="$logdir"
        RESULT="$result_in"
        TASK_ID="task-1996"
        SKILL="dev-pilot"
        SESSION_ID="sess-1996"
        PILOT_RAN="$pilot_ran"
        if [ "$mode" = "verdict" ]; then
            _measure_cycle_output 2>/dev/null
            printf '%s' "$CYCLE_OUTPUT_VERDICT"
        else
            _gate_non_empty_cycle 2>/dev/null
            printf '%s' "$RESULT"
        fi
    )
    rm -rf "$base"
}

SUCCESS_RESULT="claude-pilot completed (status: success).
Session: sess-1996
Turns: 2
Cost: \$0.31
Duration: 600000ms"

# --- Direction 1: producing nothing is no longer a success -------------------

assert_eq "clean tree, HEAD unmoved, zero tool calls -> empty (the 102-of-120 shape)" \
    "empty" "$(_gate_probe verdict no none "" 0 "$SUCCESS_RESULT")"
assert_eq "an --allow-empty commit moves HEAD but produces nothing -> empty" \
    "empty" "$(_gate_probe verdict no empty "" 0 "$SUCCESS_RESULT")"
# Reading is not producing: the tool-call count can never rescue a cycle that
# left no trace outside its own process.
assert_eq "40 tool calls with nothing written -> empty" \
    "empty" "$(_gate_probe verdict no none "" 40 "$SUCCESS_RESULT")"
# PIPELINE_INCOMPLETE is a non-conclusion, not a terminal disposition.
assert_eq "a non-conclusive Outcome does not satisfy P4" \
    "empty" "$(_gate_probe verdict no none "" 5 "Outcome: PIPELINE_INCOMPLETE — manual recovery needed.")"
# P4 is a conjunction. A disposition with no tool call is text about work.
assert_eq "a terminal disposition with zero tool calls -> empty" \
    "empty" "$(_gate_probe verdict no none "" 0 "Outcome: PLAN_GROOMED")"

# --- Direction 2: producing something still succeeds -------------------------

assert_eq "commits carrying a non-empty diff -> produced" \
    "produced" "$(_gate_probe verdict no content "" 0 "$SUCCESS_RESULT")"
assert_eq "a dirty worktree -> produced (the mika#1282 rescue shape)" \
    "produced" "$(_gate_probe verdict yes none "" 0 "$SUCCESS_RESULT")"
assert_eq "an open PR -> produced" \
    "produced" "$(_gate_probe verdict no none "https://github.com/senara-solutions/mika/pull/1" 0 "$SUCCESS_RESULT")"
# The legitimately short cycle: a grooming that concludes, having actually acted.
assert_eq "a terminal disposition backed by tool calls -> produced (no false red)" \
    "produced" "$(_gate_probe verdict no none "" 3 "Outcome: PLAN_GROOMED")"
assert_eq "the crash path's PR: line is read when PR_URL was never set" \
    "produced" "$(_gate_probe verdict no none "" 0 "HANDLER CRASH (exit code 1).
PR: https://github.com/senara-solutions/mika/pull/2")"

# --- Third verdict: no ground to measure is not a failure --------------------

assert_eq "an unreadable worktree -> undetermined, never empty" \
    "undetermined" "$(_gate_probe verdict no none "" 0 "$SUCCESS_RESULT" absent)"

echo ""
echo "Test: non-empty output gate — effect on the callback (mika#1996)"
echo "-------------------------------------------------------------------"

GATE_EMPTY_OUT=$(_gate_probe result no none "" 0 "$SUCCESS_RESULT")
assert_contains "an empty cycle is banner-marked as a pipeline failure" \
    "PIPELINE FAILURE: empty_completion" "$GATE_EMPTY_OUT"
assert_contains "the banner carries the measurement, not an assertion" \
    "HEAD did not move" "$GATE_EMPTY_OUT"
assert_contains "the banner names the tool-call count it read" \
    "tool calls: 0" "$GATE_EMPTY_OUT"
assert_contains "the banner states the definition it applied" \
    "observable outside its own process" "$GATE_EMPTY_OUT"
assert_contains "an empty cycle gets a PIPELINE_INCOMPLETE outcome" \
    "Outcome: PIPELINE_INCOMPLETE — empty_completion:" "$GATE_EMPTY_OUT"
GATE_EMPTY_OUTCOMES=$(grep -c '^Outcome: ' <<<"$GATE_EMPTY_OUT" || true)
assert_eq "an empty cycle carries exactly one Outcome line" "1" "$GATE_EMPTY_OUTCOMES"

# The positive direction is an invariant, not an optimisation: byte for byte.
for _case in "no content  0" "yes none  0" "no none  3"; do
    set -- $_case
    _d="$1"; _m="$2"; _t="$3"
    GATE_PRODUCED_IN="$SUCCESS_RESULT
Outcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/3"
    GATE_PRODUCED_OUT=$(_gate_probe result "$_d" "$_m" "" "$_t" "$GATE_PRODUCED_IN")
    assert_eq "a producing cycle leaves RESULT byte-for-byte identical (dirty=$_d moved=$_m tools=$_t)" \
        "$GATE_PRODUCED_IN" "$GATE_PRODUCED_OUT"
done

# Undetermined says so and leaves the cycle's own outcome alone.
GATE_UNDET_OUT=$(_gate_probe result no none "" 0 "$SUCCESS_RESULT
Outcome: PLAN_COMMITTED — docs/plans/x.md" absent)
assert_contains "an undetermined measurement is reported as such" \
    "Measurement: cycle output undetermined" "$GATE_UNDET_OUT"
assert_not_contains "an undetermined measurement never fabricates a failure" \
    "PIPELINE FAILURE" "$GATE_UNDET_OUT"
assert_contains "an undetermined measurement leaves the cycle's outcome intact" \
    "Outcome: PLAN_COMMITTED" "$GATE_UNDET_OUT"

# A cycle that is already red keeps its own, more specific diagnosis.
ALREADY_RED="PIPELINE FAILURE: the claude-pilot session was terminated before it produced any work.

Outcome: PIPELINE_INCOMPLETE — pilot session terminated by claude-pilot before producing work."
GATE_RED_OUT=$(_gate_probe result no none "" 0 "$ALREADY_RED")
GATE_RED_COUNT=$(grep -c 'PIPELINE FAILURE:' <<<"$GATE_RED_OUT" || true)
assert_eq "an already-red callback does not get a second banner stacked on it" \
    "1" "$GATE_RED_COUNT"
assert_contains "an already-red callback keeps its own diagnosis" \
    "terminated before it produced any work" "$GATE_RED_OUT"

# Idempotence: _deliver_callback and the EXIT trap can both fire in one process.
_gate_twice_probe() {
    local base wt logdir pre
    base=$(mktemp -d "${TMPDIR:-/tmp}/mika-1996-twice.XXXXXX")
    wt="$base/wt"; logdir="$base/logs"; mkdir -p "$logdir"
    git init -q "$wt" 2>/dev/null
    echo initial > "$wt/f"; git -C "$wt" add -A 2>/dev/null
    git -C "$wt" commit -q -m initial --no-verify 2>/dev/null
    pre=$(git -C "$wt" rev-parse HEAD)
    : > "$logdir/probe-1996.stderr"
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        WORKTREE_DIR="$wt" PRE_RUN_HEAD="$pre" POST_RUN_HEAD="$pre" PR_URL=""
        LOG_ID="probe-1996" PILOT_LOG_DIR="$logdir"
        RESULT="claude-pilot completed (status: success)."
        PILOT_RAN=1
        _gate_non_empty_cycle 2>/dev/null
        _gate_non_empty_cycle 2>/dev/null
        printf '%s' "$RESULT"
    )
    rm -rf "$base"
}
GATE_TWICE_OUT=$(_gate_twice_probe)
GATE_TWICE_COUNT=$(grep -c 'PIPELINE FAILURE: empty_completion' <<<"$GATE_TWICE_OUT" || true)
assert_eq "two gate passes produce exactly one banner" "1" "$GATE_TWICE_COUNT"

# The rewrite is line-anchored and hits the first Outcome only: a quoted
# `Outcome:` inside the pilot's own text is data, not the cycle's verdict.
GATE_QUOTED_OUT=$(_gate_probe result no none "" 0 "claude-pilot completed (status: success).
    Outcome: PLAN_GROOMED   <- quoted from the transcript, not the verdict
Outcome: UNKNOWN — inspect worktree manually.")
assert_contains "the quoted Outcome line survives the rewrite" \
    "    Outcome: PLAN_GROOMED   <- quoted from the transcript" "$GATE_QUOTED_OUT"
assert_not_contains "the cycle's own UNKNOWN outcome is replaced" \
    "Outcome: UNKNOWN" "$GATE_QUOTED_OUT"

# No Outcome line at all: one is appended rather than silently dropped.
GATE_NOOUTCOME_OUT=$(_gate_probe result no none "" 0 "claude-pilot completed (exit 0) but output was not structured JSON.")
assert_contains "a callback with no Outcome line gets one" \
    "Outcome: PIPELINE_INCOMPLETE — empty_completion:" "$GATE_NOOUTCOME_OUT"

# The dispatcher's own deliberate exits are decisions, not cycles. Their
# callbacks are a JSON document mika-dev and the audit dashboard parse; a banner
# prefixed onto one would both invent a failure and break the parse.
AUTOSKIP_IN='{"status":"auto_skipped","reason":"issue_closed","issue":"senara-solutions/mika#1","note":"Issue was already closed before dispatch fired."}'
AUTOSKIP_OUT=$(_gate_probe result no none "" 0 "$AUTOSKIP_IN" real 0)
assert_eq "an auto-skip callback (no pilot ran) is left byte-for-byte identical" \
    "$AUTOSKIP_IN" "$AUTOSKIP_OUT"
if jq -e . >/dev/null 2>&1 <<<"$AUTOSKIP_OUT"; then
    PASS=$((PASS + 1)); echo "  ✓ the auto-skip callback is still valid JSON after the gate"
else
    FAIL=$((FAIL + 1)); echo "  ✗ the auto-skip callback is still valid JSON after the gate"
fi

# A push violation already names a more specific cause than "nothing produced".
PUSHVIOL_IN="STRUCTURAL VIOLATION: pilot push detected (mika#1318).

Outcome: PIPELINE_INCOMPLETE — push violation"
PUSHVIOL_OUT=$(_gate_probe result no none "" 0 "$PUSHVIOL_IN")
assert_eq "a push-violation callback keeps its own diagnosis untouched" \
    "$PUSHVIOL_IN" "$PUSHVIOL_OUT"

# An operator cancel must keep STATUS= as the first line the parser sees.
CANCEL_OUT=$(_gate_probe result no none "" 0 "STATUS=CANCELLED_BY_OPERATOR

Original exit code: 143
claude-pilot completed (status: success).")
assert_eq "a cancelled cycle still leads with its STATUS= line (mika#749 parser contract)" \
    "STATUS=CANCELLED_BY_OPERATOR" "$(head -1 <<<"$CANCEL_OUT")"
assert_not_contains "a cancelled cycle is not relabelled as an empty completion" \
    "empty_completion" "$CANCEL_OUT"

echo ""
echo "Test: the gate cannot be bypassed (mika#1996, CONTROL-MUST-BE-UNAVOIDABLE)"
echo "-------------------------------------------------------------------"

# A guarantee exists only if EVERY path producing the guarded effect crosses the
# control point. The guarded effect is "a cycle verdict reaches mika-dev". This
# guard fails if a future delivery site is ever added without the gate — the
# exact way an invariant of this shape gets lost.
_delivery_blocks() {
    local want="$1"   # all | ungated
    awk -v want="$want" '
        /^[A-Za-z_][A-Za-z0-9_]*\(\)[[:space:]]*\{/ {
            fn = $1; sub(/\(\).*/, "", fn); body = ""; inbody = 1; next
        }
        inbody && /^\}/ {
            if (body ~ /--task-complete/) {
                if (want == "all") print fn
                else if (body !~ /_gate_non_empty_cycle/) print fn
            }
            inbody = 0; body = ""; next
        }
        inbody { body = body "\n" $0 }
    ' "$DISPATCH_LIB"
}
UNGATED=$(_delivery_blocks ungated)
assert_eq "every function that delivers a callback calls the gate" "" "$UNGATED"
DELIVERY_FNS=$(_delivery_blocks all | sort | paste -sd' ' -)
assert_eq "the delivery sites are the two we know about" \
    "_deliver_callback _dispatch_lib_exit_trap" "$DELIVERY_FNS"

# The gate must be the FIRST executable statement of _deliver_callback: an early
# return added above it would be a hole opened without anyone noticing.
DELIVER_FIRST=$(sed -n '/^_deliver_callback() {/,/^}/p' "$DISPATCH_LIB" \
    | sed '1d' | grep -vE '^[[:space:]]*(#|$)' | head -1)
assert_contains "the gate is the first executable statement of _deliver_callback" \
    "_gate_non_empty_cycle" "$DELIVER_FIRST"

# The measurement stays side-effect free: it is what makes the positive
# direction provable. If it ever writes RESULT, the byte-for-byte invariant
# above becomes untestable rather than false, which is worse.
MEASURE_SRC=$(sed -n '/^_measure_cycle_output() {/,/^}/p' "$DISPATCH_LIB")
assert_not_contains "the measurement never writes RESULT" \
    "RESULT=" "$MEASURE_SRC"

# PILOT_RAN is what separates "the cycle produced nothing" from "no cycle ran".
# A second writer would blur the two, which is how a gate starts excusing the
# population it exists to catch.
PILOT_RAN_WRITERS=$(grep -c '^[[:space:]]*PILOT_RAN=1[[:space:]]*$' "$DISPATCH_LIB" || true)
assert_eq "PILOT_RAN has exactly one writer" "1" "$PILOT_RAN_WRITERS"
PILOT_RAN_FN=$(awk '
    /^[A-Za-z_][A-Za-z0-9_]*\(\)[[:space:]]*\{/ { fn = $1; sub(/\(\).*/, "", fn); next }
    /^[[:space:]]*PILOT_RAN=1[[:space:]]*$/ { print fn }
' "$DISPATCH_LIB")
assert_eq "PILOT_RAN is set by the function that launches the pilot" \
    "_run_claude_pilot" "$PILOT_RAN_FN"

# In the EXIT trap the gate has to run BEFORE the cancel prefix, or the banner
# displaces the STATUS= line mika-dev's parser reads first (mika#749).
TRAP_ORDER=$(sed -n '/^_dispatch_lib_exit_trap() {/,/^}/p' "$DISPATCH_LIB" \
    | grep -nE '_gate_non_empty_cycle|Cancel discriminator envelope prefix' | head -1)
assert_contains "in the EXIT trap the gate precedes the cancel prefix" \
    "_gate_non_empty_cycle" "$TRAP_ORDER"


# --- Unknown-flag guard: dispatch-lib may only build flags claude-pilot accepts (mika#2043) ---
#
# Founding defect: dispatch-lib built a `--trace` flag from CLAUDE_PILOT_TRACE
# citing mika#1097 Step 0-B, but claude-pilot never accepted `--trace` — only
# Step 0-B's dispatch-lib half ever shipped. Measured against the installed CLI
# with dispatch-lib's exact argv, an unknown flag is NOT swallowed by the
# REMAINDER positional: argparse exits 2 with `unrecognized arguments` before
# the session starts. Nothing broke only because no skill ever set the env var;
# the first one to follow the comment's invitation would have killed every
# dispatch of that skill.
#
# Two passes, and the second is what keeps the first honest:
#   A. every flag dispatch-lib builds is in the accepted list  — hermetic, always runs
#   B. every entry of that list is really in `claude-pilot --help` — needs the binary
# Without B, pass A is just another unverified claim about claude-pilot's
# interface, written once and never reconfronted with the world — which is
# exactly the defect being closed here.

echo ""
echo "Test: claude-pilot flag-surface guard (mika#2043)"
echo "-------------------------------------------------"

# Flags accepted by claude-pilot's `_build_parser` (src/claude_pilot/cli.py).
# Pass B below is what keeps this list from drifting into a lie.
CP_ACCEPTED_FLAGS="--task-id --no-relay --relay-config --cwd --log-dir --command --verbose -i --interactive --max-turns --max-budget --stall-threshold --empty-threshold --idle-timeout --min-detection-turns --no-guardrails -h --help"

# A flag is anchored on any non-word character, NOT just whitespace: dispatch-lib
# writes `CWD_ARGS="--cwd $DIR"` and the dead flag was `TRACE_FLAG="--trace"`, both
# glued to `="`. A whitespace-only anchor silently missed both — caught by running
# the guard against a deliberately reintroduced --trace, not by reading it.
#
# Short flags count too: `-X` aborts argparse exactly like `--trace`. They are
# only safe to extract AFTER quoted segments are stripped (see CP_UNQUOTED
# below) — an earlier version scanned raw text, where `"${LOG_ID}-revise-$(date
# +%s)"` reads as a flag named `-revise-`.
CP_FLAG_RE='(^|[^A-Za-z0-9_-])--?[A-Za-z][A-Za-z0-9-]*'

# Every capture below ends in `|| true`: the suite runs under `set -euo pipefail`,
# where a grep that matches nothing kills the whole run. Measured — breaking the
# invocation pattern on purpose ended the suite mid-section with no summary and
# no failure. A guard that vanishes when its subject changes shape is worse than
# no guard: the run goes quiet exactly when it should be shouting. The
# site-count assertion below is what turns an empty capture into a failure.

# Join backslash continuations (the revise-pilot invocation spans four lines),
# then drop comment lines (the mika#2043 comment names --trace on purpose, as a
# warning).
CP_JOINED=$(awk '
    { line = line $0 }
    /\\$/ { sub(/\\$/, " ", line); next }
    { print line; line = "" }
' "$DISPATCH_LIB" | grep -v '^[[:space:]]*#' || true)

# Anchor on the launcher, NOT on the shape of the argv. Anchoring on
# `claude-pilot -...` looked equivalent and was not: moving $CWD_ARGS ahead of
# the first literal flag — an unremarkable reordering — dropped the main
# dispatch invocation out of the guard's sight entirely, silently, while all
# three assertions stayed green (measured). Every real launch goes through
# `_run_pilot_sandboxed`, plus the venv smoke test; that is the chokepoint.
#
# mika#2165 widened the anchor from start-of-line to start-of-COMMAND (`^` or a
# preceding `;`). The launcher is still the anchor; what changed is that it no
# longer has to be the first command on its line. This was not a preference:
# mika#2165 co-locates `_pilot_log_dir;` with the reads of $_PILOT_LOG_DIR, and
# the launch lines are two of those reads. Under the old anchor both real launch
# sites dropped out of sight at once — the site count fell 3 → 1 and said so,
# which is precisely the "renamed, reordered, or added launch point turns red
# instead of evaporating" property below doing its job. Widening here strictly
# increases what the guard can see; it never lets a launch line hide.
CP_INVOCATIONS=$(printf '%s\n' "$CP_JOINED" \
    | grep -E '(^|;)[[:space:]]*(_run_pilot_sandboxed[[:space:]]+claude-pilot|(if ! )?timeout[[:space:]]+[0-9]+[[:space:]]+claude-pilot)([[:space:]]|$)' || true)

# Quoted segments carry payload, not argv words: `ENTRY_COMMAND="/mika"` and the
# prompt text cannot word-split into flags. Scanning them made the guard accuse
# claude-pilot of `--no-verify` when the inner slash-command grew an argument
# (measured). Only unquoted interpolation can inject a flag, so strip quotes
# before looking for either flags or variables.
CP_UNQUOTED=$(printf '%s\n' "$CP_INVOCATIONS" | sed -E 's/"[^"]*"//g; s/'"'"'[^'"'"']*'"'"'//g' || true)

# Literal flags on those lines.
CP_LITERAL_FLAGS=$(printf '%s\n' "$CP_UNQUOTED" | grep -oE -- "$CP_FLAG_RE" | sed -E 's/^[^-]*//' | sort -u || true)

# Flags reaching the CLI through an interpolated variable. This half is not
# optional: $TRACE_FLAG is exactly how the dead flag got in, and a guard reading
# only literals would have missed it. The assignment pattern is anchored on a
# word boundary rather than start-of-line, so `export`/`declare`/`readonly`, a
# mid-line `[ cond ] && VAR=--flag`, and `VAR+=` are all covered — the one-line
# conditional being the most natural rewrite of the five lines just deleted, and
# invisible to a start-of-line anchor (measured).
CP_VARS=$(printf '%s\n' "$CP_UNQUOTED" | grep -oE '\$\{?[A-Za-z_][A-Za-z0-9_]*\}?' | tr -d '${}' | sort -u || true)
CP_VAR_FLAGS=""
for v in $CP_VARS; do
    _assigns=$(grep -E "(^|[^A-Za-z0-9_-])${v}\+?=" "$DISPATCH_LIB" || true)
    _found=$(printf '%s\n' "$_assigns" | grep -oE -- "$CP_FLAG_RE" | sed -E 's/^[^-]*//' || true)
    [ -n "$_found" ] && CP_VAR_FLAGS="$CP_VAR_FLAGS $_found"
done
CP_VAR_FLAGS=$(printf '%s\n' $CP_VAR_FLAGS | sort -u || true)

# `--` is the prompt separator, not a flag.
CP_BUILT_FLAGS=$(printf '%s\n%s\n' "$CP_LITERAL_FLAGS" "$CP_VAR_FLAGS" | grep -vE '^(--)?$' | sort -u || true)

# Known limit, stated rather than coded around: one level of indirection
# (`A="--trace"; B="$A"`) and a composed flag (`F="--$name"`) both slip through.
# Closing them would mean evaluating shell, which changes what this guard is.

# Count SITES, not flags. A flag floor cannot notice a lost invocation — the
# surviving one carries seven flags on its own, so the total never drops
# (measured). Pinning the site count means a renamed, reordered, or added
# launch point turns red instead of evaporating.
#
# Count moved 3 → 4 on 2026-09-21 (mika#2306): the Fire-Disposition retry in
# `_fd_retry_if_section_still_missing` is a fourth launch point, deliberately a
# direct invocation rather than a recursion into `_launch_revise_pilot` — the
# recursion would make the retry's own findings-file the predicate's input and
# render its first term true by construction.
#
# Count moved 4 → 5 on 2026-10-02 (mika#2636): the plan-size retry in
# `_plan_size_retry_if_section_still_missing` is a fifth launch point, a
# structural copy of the fourth and a direct invocation for the same reason —
# a recursion into `_launch_revise_pilot` would make the retry's own
# findings-file the predicate's input and render its first term true by
# construction.
CP_SITE_COUNT=$(printf '%s\n' "$CP_INVOCATIONS" | grep -c . || true)
if [ "$CP_SITE_COUNT" -eq 5 ]; then
    PASS=$((PASS + 1))
    echo "  ✓ all 5 claude-pilot launch sites are in the guard's sight"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ expected 5 claude-pilot launch sites, saw $CP_SITE_COUNT"
    echo "    A site the guard cannot see is a site it cannot police. If a launch"
    echo "    point was legitimately added or removed, update this count."
fi

# --- Pass A: every built flag is accepted (hermetic) ---
CP_UNKNOWN=""
for f in $CP_BUILT_FLAGS; do
    case " $CP_ACCEPTED_FLAGS " in
        *" $f "*) ;;
        *) CP_UNKNOWN="$CP_UNKNOWN $f" ;;
    esac
done
if [ -z "$CP_UNKNOWN" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ every flag dispatch-lib builds is accepted by claude-pilot"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ dispatch-lib builds flag(s) claude-pilot does not accept:$CP_UNKNOWN"
    echo "    claude-pilot exits 2 on an unrecognized argument, before the session starts."
    echo "    Either the flag is wrong, or claude-pilot gained it and this list is stale."
fi

# --- Pass B: the accepted list is not stale (needs the binary) ---
if command -v claude-pilot >/dev/null 2>&1 && CP_HELP=$(timeout 20 claude-pilot --help 2>&1); then
    CP_MISSING=""
    for f in $CP_ACCEPTED_FLAGS; do
        grep -qE -- "(^|[^A-Za-z0-9-])${f}([^A-Za-z0-9-]|$)" <<<"$CP_HELP" || CP_MISSING="$CP_MISSING $f"
    done
    if [ -z "$CP_MISSING" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ the accepted-flag list matches the installed claude-pilot --help"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ accepted-flag list is stale — not in claude-pilot --help:$CP_MISSING"
        echo "    Pass A is now asserting against a list the CLI does not back."
    fi
else
    echo "  - skipped: claude-pilot not runnable here, cannot confirm the accepted-flag list"
    echo "    (pass A still ran; only the freshness check of the list is skipped)"
fi

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
