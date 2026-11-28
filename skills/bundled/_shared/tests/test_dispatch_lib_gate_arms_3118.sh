#!/bin/bash
# Les deux bras qui portent la porte (mika#3118) — extrait de test-dispatch-lib.sh par mika#3178, vol 3.
# Le scan qui refuse qu'un des deux bras perde la porte de halte au groom,
# mesuré d'abord sur le source puis REJOUÉ contre la boucle réelle — un scan de
# source seul passerait sur une porte morte. La signature du 2026-07-04 de
# mika#1772 est rejouée de la même façon, et pour la même raison.
#
# Plage d'origine : lignes 5988–6679 de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 8de06d66. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
# ===========================================================================
# mika#3118 — LES DEUX BRAS PORTENT LA PORTE, ET UN SCAN LE REFUSE SI L'UN LA PERD
#
# Sans ce scan, la porte peut disparaître d'un seul bras sans qu'aucun test
# rougisse : les sondes comportementales ci-dessous couvrent les deux chemins
# aujourd'hui, mais un bras ajouté demain (une troisième convergence sur
# `GROOMED`) naîtrait nu et personne ne le verrait. C'est la Fire-Disposition du
# plan, option (a).
#
# Le scan lit le FICHIER et non `declare -f` : un bras déplacé hors de
# `_iterate_groom_loop` resterait un bras.
echo ""
echo "Test: mika#3118 — la cardinalité des bras GROOMED gardés"
echo "------------------------------------------------------------"

EXPECTED_PLAN_HALT_GATE_SITES=2

# L'allowlist d'exceptions nommées, LIVRÉE VIDE — et son vide est asserté deux
# lignes plus bas. Elle est VRAIMENT consommée par le scan (la boucle de
# soustraction ci-dessous) : une allowlist que rien ne lit est du théâtre, et le
# jour où quelqu'un y écrit un stage, il faut que ça AIT un effet mesurable.
# Elle est soustractible parce qu'elle contient des stages, pas des fichiers —
# c'est la différence entre une exception qui expire et l'allowlist que
# mika#2309 condamne.
PLAN_HALT_GATE_SITE_EXCEPTIONS=()

# Rend une ligne par bras `*-to-groomed` : `gated` ou `ungated`, puis le stage.
# Fenêtre de 10 lignes : la porte et le callout sont séparés par le commentaire
# qui explique l'ordre, et exiger l'adjacence interdirait de l'expliquer.
#
# Les lignes de COMMENTAIRE sont écartées d'abord, et ce n'est pas un détail de
# propreté : la doc d'en-tête de `_iterate_groom_loop` énumère ses cinq terminaux
# en citant `_write_canonical_callout "ready-to-groomed"` et son jumeau (l. 12076
# et 12078). Les compter faisait voir QUATRE bras, dont deux nus — un rouge
# permanent sur une documentation juste, et la seule issue aurait été une
# allowlist. Le même `next` écarte aussi une porte citée en commentaire : un
# commentaire ne garde rien.
_m3118_groomed_arms() {
    awk '
        /^[ \t]*#/ { next }
        /_groom_plan_halt_gate[ \t]+"/ { gate = NR }
        /_write_canonical_callout[ \t]+"[a-z-]+-to-groomed"/ {
            stage = $0
            sub(/^.*_write_canonical_callout[ \t]+"/, "", stage)
            sub(/".*$/, "", stage)
            if (gate != 0 && NR - gate <= 10) printf "gated %s\n", stage
            else printf "ungated %s\n", stage
        }
    ' "$1"
}

M3118_ARMS=$(_m3118_groomed_arms "$DISPATCH_LIB")
# Anti-vacuité d'abord : un scan qui ne voit aucun bras passerait tout le reste.
assert_eq "mika#3118 (FD): le scan voit exactement deux bras GROOMED" \
    "$EXPECTED_PLAN_HALT_GATE_SITES" "$(printf '%s\n' "$M3118_ARMS" | grep -c . || true)"
assert_eq "mika#3118 (FD): l'allowlist d'exceptions est livrée VIDE" \
    "0" "${#PLAN_HALT_GATE_SITE_EXCEPTIONS[@]}"

# La soustraction de l'allowlist, écrite pour de vrai. Vide, elle est l'identité.
_m3118_ungated_after_allowlist() {
    local arms="$1" exc out
    out=$(printf '%s\n' "$arms" | sed -n 's/^ungated //p')
    for exc in ${PLAN_HALT_GATE_SITE_EXCEPTIONS[@]+"${PLAN_HALT_GATE_SITE_EXCEPTIONS[@]}"}; do
        out=$(printf '%s\n' "$out" | grep -vFx -- "$exc" || true)
    done
    printf '%s\n' "$out" | grep -c . || true
}
assert_eq "mika#3118 (FD): aucun bras GROOMED n'est dépourvu de la porte" \
    "0" "$(_m3118_ungated_after_allowlist "$M3118_ARMS")"
assert_contains "mika#3118 (FD): le bras after-ready est nommément gardé" \
    "gated ready-to-groomed" "$M3118_ARMS"
assert_contains "mika#3118 (FD): le bras after-iterate aussi" \
    "gated iterate-to-groomed" "$M3118_ARMS"

# --- Contrôle négatif : le scan VOIT-IL un bras nu ? -----------------------
#
# Sans lui, un scan cassé — un `awk` qui n'apparie plus rien, une fenêtre
# devenue infinie — rendrait `0 ungated` sur n'importe quoi et lirait exactement
# comme un scan sain. La fabrication retire la ligne de porte du bras
# after-iterate dans une COPIE ; le fichier livré n'est jamais touché.
M3118_NEG_TMP=$(mktemp -d)
sed '/_groom_plan_halt_gate "second-pass-after-iterate"/d' "$DISPATCH_LIB" \
    > "$M3118_NEG_TMP/dispatch-lib-ungated.sh"
M3118_NEG_ARMS=$(_m3118_groomed_arms "$M3118_NEG_TMP/dispatch-lib-ungated.sh")
assert_eq "mika#3118 (FD): la fabrication a bien retiré une ligne, pas zéro" \
    "1" "$(diff "$DISPATCH_LIB" "$M3118_NEG_TMP/dispatch-lib-ungated.sh" | grep -c '^< ' || true)"
assert_contains "mika#3118 (FD): le scan REFUSE le bras fabriqué sans porte" \
    "ungated iterate-to-groomed" "$M3118_NEG_ARMS"
assert_eq "mika#3118 (FD): et il en compte un, soustraction de l'allowlist faite" \
    "1" "$(_m3118_ungated_after_allowlist "$M3118_NEG_ARMS")"
rm -rf "$M3118_NEG_TMP"

# --- AC4 : les deux populations ne se touchent pas -------------------------
#
# Le `case` de la porte mappe le code 0 sur `plan_halt_declared` et le code 2 sur
# `plan_halt_unreadable`. L'inverse serait un `RESULT` parfaitement plausible —
# « le plan déclare une halte » pour une lecture en échec — et aucune sonde
# comportementale ne le verrait, puisque les deux terminent de la même façon.
M3118_MAP_DECL=$(printf '%s\n' "$GATE_SRC_3118" \
    | sed -n '/^[[:space:]]*0)/,/^[[:space:]]*;;/p')
M3118_MAP_UNRD=$(printf '%s\n' "$GATE_SRC_3118" \
    | sed -n '/^[[:space:]]*2)/,/^[[:space:]]*;;/p')
assert_contains "mika#3118 (AC4): le code 0 passe plan_halt_declared" \
    'GROOM_HALT_CAUSE_PLAN_HALT_DECLARED' "$M3118_MAP_DECL"
assert_eq "mika#3118 (AC4): et JAMAIS plan_halt_unreadable" "0" \
    "$(printf '%s\n' "$M3118_MAP_DECL" | grep -c 'PLAN_HALT_UNREADABLE' || true)"
assert_contains "mika#3118 (AC4): le code 2 passe plan_halt_unreadable" \
    'GROOM_HALT_CAUSE_PLAN_HALT_UNREADABLE' "$M3118_MAP_UNRD"
assert_eq "mika#3118 (AC4): et JAMAIS plan_halt_declared" "0" \
    "$(printf '%s\n' "$M3118_MAP_UNRD" | grep -c 'PLAN_HALT_DECLARED' || true)"
# AC5 — le plan introuvable rend 0. La grâce `GROOM_PROOF_UNBOUND_GRACE` garde sa
# population, et c'est la seule exception à la direction du fail-safe.
assert_contains "mika#3118 (AC5): un plan introuvable laisse passer, sans escalade" \
    'the halt gate reads nothing and refuses nothing' "$GATE_SRC_3118"

# ===========================================================================
# mika#3118 — LES DEUX BRAS, REJOUÉS CONTRE LA BOUCLE RÉELLE
#
# Le scan ci-dessus prouve que la porte est POSÉE ; ces sondes prouvent qu'elle
# MORD. `_arch_ask` est bouchonné à la frontière de processus, comme la sonde
# mika#1772 : les parseurs, l'enveloppe de relance, le trail, `_escalate_groom`
# et les terminaux restent réels.
# ===========================================================================

echo ""
echo "Test: mika#3118 — un plan qui déclare une halte ne converge pas"
echo "------------------------------------------------------------"

# $1 = disposition du premier passage ("READY" → bras after-ready, "ITERATE" →
#      bras after-iterate), $2 = ligne posée à la l. 11 du plan.
_groom_plan_halt_probe_3118() {
    local first_disposition="$1" line11="$2"
    local tmp wt rc
    tmp=$(mktemp -d)
    wt="$tmp/wt"
    mkdir -p "$wt/docs/plans"
    {
        echo "# Plan for the plan-halt gate"
        echo "**Ticket:** mika issue#3118"
        for i in $(seq 1 8); do
            echo "Body line $i — padding padding padding padding padding."
        done
        printf '%s\n' "$line11"
        for i in $(seq 12 30); do
            echo "Body line $i — padding padding padding padding padding."
        done
    } > "$wt/docs/plans/2026-11-26-001-fix-3118-gate-plan.md"

    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true

        _arch_ask() {
            local n body
            n=$(cat "$tmp/arch-calls")
            n=$((n + 1))
            printf '%s' "$n" > "$tmp/arch-calls"
            case "$n" in
                1) body="Disposition: ${first_disposition}" ;;
                *) body='Verdict: GROOMED' ;;
            esac
            jq -n --arg c "$body" \
                '{content:$c, metadata:{session_id:"probe-session-3118"}}'
        }
        # Le pilote de revise ne tourne pas ici : le bras after-iterate a besoin
        # qu'il rende 0, pas qu'il révise. Le plan reste donc celui du fixture,
        # ce qui est exactement le cas mesuré — le plan du 19/11 portait sa halte
        # APRÈS la révision.
        _launch_revise_pilot() { return 0; }
        _write_canonical_callout() { printf 'written\n' > "$tmp/callout"; return 0; }
        printf '0' > "$tmp/arch-calls"

        WORKTREE_DIR="$wt" ISSUE_NUM="3118" REPO="mika" BRANCH="probe-3118"
        RESULT=""
        if _iterate_groom_loop >/dev/null 2>"$tmp/err"; then rc=0; else rc=1; fi

        printf 'rc=%s\n' "$rc"
        printf 'arch_calls=%s\n' "$(cat "$tmp/arch-calls")"
        printf 'callout=%s\n' "$([ -f "$tmp/callout" ] && echo yes || echo no)"
        printf 'reason=%s\n' "${GROOM_LOOP_FAILURE_REASON:-<none>}"
        # `RESULT` à plat : c'est le seul champ que l'opérateur verra dans
        # `tasks.result`, et la ligne de cause y est ANCRÉE en début de ligne.
        printf 'cause_line=%s\n' \
            "$(printf '%s\n' "$RESULT" | sed -n 's/^Groom-halt-cause: //p' | head -1)"
        printf 'outcome=%s\n' \
            "$(printf '%s\n' "$RESULT" | sed -n 's/^Outcome: //p' | head -1)"
    )
    rm -rf "$tmp"
}

HALT_H2_3118='## HALTE — la borne du groom est dépassée'
NOHALT_3118='## Contexte — rien à refuser ici'

# --- AC1 : le bras after-ready --------------------------------------------
P3118_READY=$(_groom_plan_halt_probe_3118 READY "$HALT_H2_3118") || P3118_READY=""
assert_contains "AC1: la sonde a conduit la boucle jusqu'au second passage" \
    "arch_calls=2" "$P3118_READY"
assert_contains "AC1: la convergence est détournée (la boucle rend 1)" \
    "rc=1" "$P3118_READY"
assert_contains "AC1: AUCUN callout canonique n'est écrit" \
    "callout=no" "$P3118_READY"
assert_contains "AC1: la cause voyage sur le fil, par sa ligne ancrée" \
    "cause_line=plan_halt_declared second-pass-after-ready" "$P3118_READY"
assert_contains "AC1: et la disposition est ESCALATE, jamais un feu vert" \
    "outcome=ESCALATE" "$P3118_READY"
# Le motif voyage dans `tasks.result` et c'est TOUT ce que l'opérateur voit
# (PR#2028). Il doit nommer le PLAN : dire « l'architecte a refusé » enverrait
# lire une objection qui n'existe pas — mika-arch a répondu GROOMED (mika#1772).
assert_contains "AC1: le motif nomme la halte DU PLAN, pas un refus d'architecte" \
    "the plan declares a halt of its own" "$P3118_READY"

# --- AC2 : le bras after-iterate ------------------------------------------
P3118_ITER=$(_groom_plan_halt_probe_3118 ITERATE "$HALT_H2_3118") || P3118_ITER=""
assert_contains "AC2: la sonde a conduit la boucle jusqu'au second passage" \
    "arch_calls=2" "$P3118_ITER"
assert_contains "AC2: la convergence est détournée (la boucle rend 1)" \
    "rc=1" "$P3118_ITER"
assert_contains "AC2: AUCUN callout canonique n'est écrit" \
    "callout=no" "$P3118_ITER"
assert_contains "AC2: la cause nomme l'autre étape, et c'est le même fil" \
    "cause_line=plan_halt_declared second-pass-after-iterate" "$P3118_ITER"
assert_contains "AC2: et la disposition est ESCALATE" \
    "outcome=ESCALATE" "$P3118_ITER"

# --- CONTRE-TÉMOIN : sans halte, les deux bras convergent à l'octet près ---
#
# C'est la moitié qui empêche la porte d'être un refus universel. Sans elle, une
# porte qui escalade TOUJOURS passerait les six assertions ci-dessus.
P3118_OK_READY=$(_groom_plan_halt_probe_3118 READY "$NOHALT_3118") || P3118_OK_READY=""
assert_contains "contre-témoin (after-ready): la boucle converge" \
    "rc=0" "$P3118_OK_READY"
assert_contains "contre-témoin (after-ready): le callout EST écrit" \
    "callout=yes" "$P3118_OK_READY"
assert_contains "contre-témoin (after-ready): et aucun motif d'échec n'est posé" \
    "reason=<none>" "$P3118_OK_READY"
P3118_OK_ITER=$(_groom_plan_halt_probe_3118 ITERATE "$NOHALT_3118") || P3118_OK_ITER=""
assert_contains "contre-témoin (after-iterate): la boucle converge" \
    "rc=0" "$P3118_OK_ITER"
assert_contains "contre-témoin (after-iterate): le callout EST écrit" \
    "callout=yes" "$P3118_OK_ITER"

# --- AC3 : les deux faux positifs mesurés convergent NORMALEMENT -----------
#
# Deux assertions distinctes, parce que ce sont deux grammaires mesurées
# distinctes : le gras des plans 2420/2518 et la citation de la règle mika#2693
# que porte TOUT plan groomé. Attestées ici sur la BOUCLE, là où elles coûtent,
# et pas seulement sur le détecteur.
P3118_FP_BOLD=$(_groom_plan_halt_probe_3118 READY \
    '**HALTE 1 —** le périmètre du vol 1 ne couvre pas cette moitié') || P3118_FP_BOLD=""
assert_contains "AC3 (a): un '**HALTE 1 —**' en prose converge en GROOMED" \
    "callout=yes" "$P3118_FP_BOLD"
P3118_FP_RULE=$(_groom_plan_halt_probe_3118 READY \
    'HALTE borne : 312 lignes ajoutées, pour une borne du groom de 300.') || P3118_FP_RULE=""
assert_contains "AC3 (b): une citation de la règle mika#2693 converge en GROOMED" \
    "callout=yes" "$P3118_FP_RULE"

# ===========================================================================
# mika#1772 — the 2026-07-04 signature, replayed against the real loop
#
# The incident this ticket was opened for: on mika#1723 a plan DID exist on the
# branch and `_iterate_groom_loop` still returned non-zero, while a manual
# `/mika-ask-arch` converged GROOMED the same day. The historical trace is gone
# — the worktree (and its `groom-verdict-trail.log`) was destroyed,
# GROOM_LOOP_FAILURE_REASON did not exist yet, and dispatch-lib's stderr has no
# durable sink. So the class is pinned here by construction instead: an
# architect first pass that answers plausibly WITHOUT the `Disposition:` line.
#
# Two states, because one of them alone proves nothing:
#   A. the corrective retry of mika#1823 (f8b63530, 2026-07-25) recovers, and
#      the loop converges — this is the mechanism that closed the class, and
#      nothing until now failed when it was removed;
#   B. the architect misses the line TWICE. The loop still returns 1 — correctly,
#      two failed attempts at a verdict are a real non-convergence — and the
#      question is only what it tells the operator it did.
#
# These run the real `_iterate_groom_loop` with `_arch_ask` stubbed at the
# process boundary. A structural grep would pass over both.
# ===========================================================================

echo ""
echo "Test: the 2026-07-04 UNPARSED signature (mika#1772 / #1823)"
echo "------------------------------------------------------------"

# Architect prose that is genuinely unparsable: no `Disposition:` line, no
# `Verdict:` line, and none of the fourteen paraphrase patterns tier 2 matches.
# Checked against `_parse_disposition_fuzzy` — a body that accidentally said
# "proceed" or "revise" would be parsed, and the fixture would replay a
# different class than the one it names.
UNPARSED_1772='I have read the document end to end. The framing is coherent and the
acceptance criteria tie back to the ticket. My notes on section three are
minor and mostly editorial. Nothing in here surprises me.'

# $1 = what the architect replies to the mika#1823 corrective retry.
# Emits `key=value` lines plus `trail:` / `stderr:` prefixed lines.
_groom_signature_probe_1772() {
    local retry_reply="$1"
    local tmp wt rc
    tmp=$(mktemp -d)
    wt="$tmp/wt"
    mkdir -p "$wt/docs/plans"
    {
        echo "# Plan for the replayed signature"
        echo "**Ticket:** mika issue#1723"
        for i in $(seq 1 14); do
            echo "Body line $i — padding padding padding padding padding."
        done
    } > "$wt/docs/plans/2026-07-01-001-fix-1723-signature-plan.md"

    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true

        ARCH_CALLS="$tmp/arch-calls"
        printf '0' > "$ARCH_CALLS"

        # Stub at the process boundary: the parsers, the retry envelope, the
        # trail and the terminal states all stay real.
        _arch_ask() {
            local n body
            n=$(cat "$ARCH_CALLS")
            n=$((n + 1))
            printf '%s' "$n" > "$ARCH_CALLS"
            case "$n" in
                1) body="$UNPARSED_1772" ;;   # first pass — the 2026-07-04 shape
                2) body="$retry_reply" ;;     # the mika#1823 corrective retry
                *) body='Verdict: GROOMED' ;; # second review
            esac
            jq -n --arg c "$body" \
                '{content:$c, metadata:{session_id:"probe-session-1723"}}'
        }
        # Real one shells out to `gh issue edit`.
        _write_canonical_callout() { printf 'written\n' > "$tmp/callout"; return 0; }

        WORKTREE_DIR="$wt" ISSUE_NUM="1723" REPO="mika" BRANCH="probe-1723"
        if _iterate_groom_loop >/dev/null 2>"$tmp/err"; then rc=0; else rc=1; fi

        printf 'rc=%s\n' "$rc"
        printf 'arch_calls=%s\n' "$(cat "$ARCH_CALLS")"
        printf 'reason=%s\n' "${GROOM_LOOP_FAILURE_REASON:-<none>}"
        printf 'callout=%s\n' "$([ -f "$tmp/callout" ] && echo yes || echo no)"
        # Field 4 of the trail is the outcome, in call order. Joined rather than
        # emitted line-per-line so an assertion can pin the whole sequence: the
        # individual tokens overlap (`UNPARSED` is a prefix of
        # `UNPARSED-after-retry`) and match each other on a substring test.
        printf 'trail_outcomes=%s\n' \
            "$(cut -f4 "$wt/.claude/groom-verdict-trail.log" 2>/dev/null | paste -sd, -)"
        sed 's/^/stderr: /' "$tmp/err" 2>/dev/null
    )
    rm -rf "$tmp"
}

# --- A: the retry recovers, and the loop converges -------------------------

SIG_RECOVERED_1772=$(_groom_signature_probe_1772 'Apologies — restating with the required line.

Disposition: READY') || SIG_RECOVERED_1772=""

# Anti-vacuity anchor, first: without it every assertion below would pass on an
# empty string if the probe ever stopped running.
assert_contains "A: the probe drove the loop through all three architect calls" \
    "arch_calls=3" "$SIG_RECOVERED_1772"
assert_contains "A: the 2026-07-04 signature converges (loop returns zero)" \
    "rc=0" "$SIG_RECOVERED_1772"
assert_contains "A: convergence is announced for the issue" \
    "converged on GROOMED for mika#1723" "$SIG_RECOVERED_1772"
assert_contains "A: the canonical callout is written on convergence" \
    "callout=yes" "$SIG_RECOVERED_1772"
assert_contains "A: a converged run records no failure reason" \
    "reason=<none>" "$SIG_RECOVERED_1772"
# The trail is what distinguishes "converged" from "converged BECAUSE of the
# retry". Pinned as the whole sequence: a loop that never needed a second
# attempt would read `READY,GROOMED` and satisfy any per-token check.
assert_contains "A: the trail is first-pass UNPARSED, then a retry, then GROOMED" \
    "trail_outcomes=UNPARSED,READY-after-retry,GROOMED" "$SIG_RECOVERED_1772"
assert_contains "A: the retry announces itself as the mika#1823 mechanism" \
    "disposition recovered on retry (READY) — mika#1823" "$SIG_RECOVERED_1772"

# --- B: the architect misses the line twice --------------------------------

SIG_DOUBLE_1772=$(_groom_signature_probe_1772 "$UNPARSED_1772") || SIG_DOUBLE_1772=""

assert_contains "B: the retry envelope is bounded at two attempts" \
    "arch_calls=2" "$SIG_DOUBLE_1772"
assert_contains "B: a double miss is a non-convergence (loop returns non-zero)" \
    "rc=1" "$SIG_DOUBLE_1772"
assert_contains "B: the trail records both attempts, the second as a retry" \
    "trail_outcomes=UNPARSED,UNPARSED-after-retry" "$SIG_DOUBLE_1772"

# The reason travels to `tasks.result` (PR#2028) and is the only thing the
# operator sees. This exit is the loop's designed, bounded double-UNPARSED
# terminal — `_parse_disposition` emits READY|ITERATE|ESCALATE or nothing, so
# the `*)` arm fires on empty and on nothing else. Calling that an invariant
# violation sends the operator hunting a bug in the loop instead of reading
# "the architect never emitted the line, twice" — which is the mika#1772 defect
# class exactly, surviving PR#2028 at the one site its author believed
# unreachable.
#
# Asserted against the `reason=` line ALONE, not against the probe's whole
# output: the loop's own stderr already carries both "disposition UNPARSED" and
# "mika#1823" from the FIRST attempt, so a whole-output match reported these as
# satisfied while the reason itself said none of it. Measured — the two needles
# below passed against the pre-fix reason before this narrowing.
SIG_DOUBLE_REASON_1772=$(printf '%s\n' "$SIG_DOUBLE_1772" | grep '^reason=' || true)
assert_contains "B: the reason names the unparsed disposition" \
    "disposition UNPARSED" "$SIG_DOUBLE_REASON_1772"
assert_contains "B: the reason says both attempts missed" \
    "2 attempts" "$SIG_DOUBLE_REASON_1772"
assert_contains "B: the reason names the retry that already ran" \
    "mika#1823" "$SIG_DOUBLE_REASON_1772"
assert_not_contains "B: the designed terminal is not reported as an invariant violation" \
    "invariant violation" "$SIG_DOUBLE_REASON_1772"

# --- Test: the egress launch guard must constate, not affirm (mika#2041) ---
#
# Founding incident (2026-08-29 06:07-06:11 CEST): the host egress proxy was
# killed. A kill does not unlink a unix socket, so the path survived as an
# orphan. dispatch-lib respawned a proxy on every dispatch, every proxy died,
# and dispatch-lib reported success each time -- the pilot went out with no
# egress and no `fs-only` line to say so.
#
# The wait guard asked `[ -S "$sock" ]` -- "is there a file of type socket" --
# where the question is "is anyone listening". An orphan satisfies the first
# and fails the second, so the loop exited on its first iteration and the
# fs-only fallback became unreachable in exactly the scenario that needs it.
#
# ANTI-VACUITY (plan KTD6): the `ghost` assertions below FAIL against the
# pre-fix guard -- measured rc=0 / launched-ok where the fix gives rc=1 /
# fs-only. A structural grep would have passed over dead code; this exercises
# the real `_ensure_pilot_egress_proxy`.

echo ""
echo "Test: egress launch guard tests connectability, not file existence (mika#2041)"
echo "------------------------------------------------------------"

# The operational proxy log is the diagnostic surface this whole ticket rests
# on. A suite that appends fake-proxy output into it destroys the evidence it
# exists to make legible, so the probe redirects the log and we assert the real
# file never grew (plan R8).
#
# Assert on the fixture's own line, not on the file's size: on a host where the
# proxy is alive and serving, the log grows from real traffic while the suite
# runs, so a size comparison would fail for reasons that have nothing to do
# with this test. The fixture string can only appear there if the override broke.
_REAL_EGRESS_LOG="/var/log/mika/pilot-egress-proxy.log"
_egress_log_fixture_hits() {
    # -r, not -f: a deploy run as root leaves a log this suite cannot read, and
    # `grep -c` then exits 2 printing nothing, so the assertion would compare
    # "0" against "" and fail for a reason unrelated to the test.
    [ -r "$_REAL_EGRESS_LOG" ] || { echo 0; return 0; }
    # `grep -c` already prints 0 on no-match and exits 1; `|| true` swallows the
    # status without printing a second count.
    grep -c "mika#2041 test fixture" "$_REAL_EGRESS_LOG" 2>/dev/null || true
}

# _egress_guard_probe <sock_state> <bin_state> [sock_basename]
#   sock_state: ghost  -- bind then close without unlink (the orphan shape)
#               live   -- a real listener held open
#               absent -- nothing at the path
#   bin_state:  dies   -- an executable proxy that exits before binding
#               missing-- no proxy binary at all
# Echoes "rc=<n> launched=<yes|no> msg=<fs-only|phase2b|launched-ok|none>".
_egress_guard_probe() {
    local sock_state="$1" bin_state="$2" sock_name="${3:-mika-pilot-egress.sock}"
    local tmp sock bin logdir marker out rc listener_pid launched msg i
    tmp=$(mktemp -d)
    sock="$tmp/$sock_name"
    bin="$tmp/fake-proxy"
    logdir="$tmp/logs"
    marker="$tmp/launched"
    mkdir -p "$logdir"

    # A proxy that records its own launch and then dies WITHOUT binding --
    # the incident shape. Why it died is still unknown (mika#2041 comment);
    # the guard must notice either way.
    cat > "$bin" <<'FAKE_PROXY'
#!/bin/bash
# The marker lives next to this script, not in an environment variable: the
# launcher starts the relay under `env -i`, so nothing exported by the test
# reaches it.
touch "$(dirname "$0")/launched"
# Noisy on purpose: a proxy dying before bind() prints a traceback, and that
# output goes wherever the launcher points its log. If the log override ever
# regresses, this line lands in the operational log and the size assertion at
# the end of the block catches it. A silent fake would make that check vacuous.
echo "fake-proxy: dying before bind (mika#2041 test fixture)" >&2
exit 1
FAKE_PROXY
    chmod +x "$bin"
    [ "$bin_state" = "missing" ] && rm -f "$bin"

    listener_pid=""
    case "$sock_state" in
        ghost)
            python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.bind(sys.argv[1])
s.close()
' "$sock"
            ;;
        live)
            python3 -c '
import socket, sys, time
s = socket.socket(socket.AF_UNIX)
s.bind(sys.argv[1])
s.listen(1)
time.sleep(30)
' "$sock" &
            listener_pid=$!
            i=0
            while [ $i -lt 50 ] && [ ! -S "$sock" ]; do sleep 0.1; i=$((i + 1)); done
            ;;
    esac

    out=$(
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB"
        _PILOT_EGRESS_SOCK="$sock"
        _PILOT_EGRESS_PROXY_BIN="$bin"
        MIKA_PILOT_EGRESS_LOG_DIR="$logdir"
        _ensure_pilot_egress_proxy 2>&1 >/dev/null
    ) && rc=0 || rc=$?

    [ -n "$listener_pid" ] && kill "$listener_pid" 2>/dev/null
    [ -n "$listener_pid" ] && wait "$listener_pid" 2>/dev/null

    # The launch is a detached `nohup ... &`, so the marker can land after the
    # guard returns. Without this bounded wait the assertion races the fork and
    # reports launched=no for a proxy that did start.
    launched=no
    i=0
    while [ $i -lt 20 ] && [ ! -f "$marker" ]; do sleep 0.05; i=$((i + 1)); done
    [ -f "$marker" ] && launched=yes
    # mika#2049 re-tokenised these two lines. Pre-2049 BOTH ended in "(falling
    # back to fs-only)" and only one carried a stable token, so this `case` had
    # to be ordered defensively to keep them apart -- and an operator using
    # `pilot_egress_guard.unreachable` as THE predicate read a nominal regime on
    # a fleet whose proxy binary was never deployed (the gap mika#2050 had to
    # document). Each cause now carries its own token, so the two arms are
    # disjoint by construction rather than by ordering.
    #
    # The strings no longer say "falling back" because nothing falls back any
    # more: the caller refuses. Leaving them would have made Signal S (mika#2050)
    # count a population that can no longer exist.
    case "$out" in
        *"pilot_egress_guard.binary_missing"*) msg=binary-missing ;;
        *"pilot_egress_guard.unreachable"*) msg=unreachable ;;
        *"pilot-egress-proxy launched"*) msg=launched-ok ;;
        *) msg=none ;;
    esac
    rm -rf "$tmp"
    printf 'rc=%s launched=%s msg=%s' "$rc" "$launched" "$msg"
}

# THE regression. Pre-fix this is "rc=0 launched=yes msg=launched-ok": the
# orphan file satisfies [ -S ], the guard affirms a launch that never happened.
assert_eq "orphan socket + proxy that dies before binding => guard reports unreachable" \
    "rc=1 launched=yes msg=unreachable" \
    "$(_egress_guard_probe ghost dies)"

# The liveness probe must still recognise a real listener after the probe was
# factored out -- and must not relaunch behind its back.
assert_eq "live listener => already-alive, proxy not relaunched" \
    "rc=0 launched=no msg=none" \
    "$(_egress_guard_probe live dies)"

# No file at the path: this already worked pre-fix. Locks it against regression.
assert_eq "no socket at path + proxy that dies => guard reports unreachable" \
    "rc=1 launched=yes msg=unreachable" \
    "$(_egress_guard_probe absent dies)"

# The two causes must stay distinguishable: a missing binary is a deploy state
# (remedy: `make install`), an unreachable socket is a runtime failure (remedy:
# restart the relay). Same rc, different token, different gesture -- which is
# why mika#2049 gave each one a token of its own.
assert_eq "missing proxy binary => binary_missing, not unreachable" \
    "rc=1 launched=no msg=binary-missing" \
    "$(_egress_guard_probe ghost missing)"

# The probe passes the path as argv, never interpolated into python source
# (plan KTD2). A quote in the path used to be a syntax error waiting to happen.
assert_eq "socket path containing a single quote does not break the probe" \
    "rc=1 launched=yes msg=unreachable" \
    "$(_egress_guard_probe ghost dies "mika'\''egress.sock")"

# R8: no fake-proxy output may reach the operational proxy log.
assert_eq "fake-proxy output never reaches the operational proxy log" \
    "0" "$(_egress_log_fixture_hits)"

# --- mika#2051: the refusal line names the pid it just launched -------------
#
# A SISTER function rather than an extension of `_egress_guard_probe`, and the
# reason is the probe's contract: it CLASSIFIES the output into one token
# (`msg=unreachable`) and then throws the text away. Five assertions above
# compare its `rc=… launched=… msg=…` string by strict equality, so widening it
# by one field would redden all five for a need that concerns one. The two
# functions say what they each do: one tests the DECISION, this one the TEXT.
#
# Same fake proxy, same log redirect -- the redirect is not incidental. Without
# it the fake proxy's traceback lands in the operational log, the very file this
# ticket exists to make legible, and the assertion at the end of the block above
# is what catches that regression.
# _egress_guard_line [bin_state]   bin_state: dies (default) | missing
_egress_guard_line() {
    local bin_state="${1:-dies}"
    local tmp sock bin logdir out
    tmp=$(mktemp -d)
    sock="$tmp/mika-pilot-egress.sock"
    bin="$tmp/fake-proxy"
    logdir="$tmp/logs"
    mkdir -p "$logdir"

    cat > "$bin" <<'FAKE_PROXY'
#!/bin/bash
echo "fake-proxy: dying before bind (mika#2041 test fixture)" >&2
exit 1
FAKE_PROXY
    chmod +x "$bin"
    [ "$bin_state" = "missing" ] && rm -f "$bin"

    # The orphan shape: bind then close without unlink. `[ -S ]` is satisfied,
    # nothing listens -- the population mika#2041 measured.
    python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.bind(sys.argv[1])
s.close()
' "$sock"

    out=$(
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB"
        _PILOT_EGRESS_SOCK="$sock"
        _PILOT_EGRESS_PROXY_BIN="$bin"
        MIKA_PILOT_EGRESS_LOG_DIR="$logdir"
        _ensure_pilot_egress_proxy 2>&1 >/dev/null
    ) || true
    rm -rf "$tmp"
    printf '%s' "$out"
}

_EGRESS_UNREACHABLE_LINE=$(_egress_guard_line)

# The joint. `pilot_egress_startup.begin pid=<pid>` is the proxy's first line
# (contract held by test_startup_emits_a_begin_breadcrumb_before_bind); the
# launcher's SUCCESS line already carried the pid and its FAILURE line -- the
# only one this ticket exists to diagnose -- did not. Without it the operator
# joins the dispatch `.stderr` to a cumulative proxy log by timestamp, which is
# the friction that made the 2026-08-29 diagnosis expensive.
assert_eq "mika#2051: the unreachable refusal names the pid it launched" \
    "yes" \
    "$(grep -qE 'pid [0-9]+' <<<"$_EGRESS_UNREACHABLE_LINE" && echo yes || echo no)"

# The published predicates grep `^dispatch-lib: ` + a contiguous token (Signal S
# anchor, runbook §2). Assert THAT invariant, never the pid's position: pinning
# the position would freeze a wording choice where what matters is what the
# operator greps bite on.
assert_eq "mika#2051: the refusal keeps its anchor and contiguous token" \
    "yes" \
    "$(grep -qE '^dispatch-lib: pilot_egress_guard\.unreachable ' <<<"$_EGRESS_UNREACHABLE_LINE" && echo yes || echo no)"

# The Signal S predicate (CLAUDE.md) stops at the token FAMILY, `pilot_egress_guard.`,
# precisely so it covers both causes -- mika#2050's lesson being that an operator
# who takes `unreachable` for THE predicate reads a nominal regime on a fleet
# whose proxy binary was never deployed. A predicate published over two tokens
# needs both to be witnessed, or half of it is an assumption.
_EGRESS_BINARY_MISSING_LINE=$(_egress_guard_line missing)

assert_eq "mika#2051: the Signal S family predicate bites on unreachable" \
    "yes" \
    "$(grep -qE '^dispatch-lib: pilot_egress_guard\.' <<<"$_EGRESS_UNREACHABLE_LINE" && echo yes || echo no)"

assert_eq "mika#2051: the Signal S family predicate bites on binary_missing" \
    "yes" \
    "$(grep -qE '^dispatch-lib: pilot_egress_guard\.binary_missing ' <<<"$_EGRESS_BINARY_MISSING_LINE" && echo yes || echo no)"

# binary_missing launched NO proxy, so it has no pid to name. Stamping one would
# be a false statement, and this is the negative control that keeps the pid from
# being spread to the line where it would be a lie.
assert_eq "mika#2051: binary_missing names no pid, because it launched none" \
    "no" \
    "$(grep -qE 'pid [0-9]+' <<<"$_EGRESS_BINARY_MISSING_LINE" && echo yes || echo no)"

# R8 again, and it must be HERE rather than only above: the sister function
# launches its own fake proxy, so the earlier assertion -- which runs before
# these calls -- cannot witness it. A log override that regressed only in
# `_egress_guard_line` would leave the first check green while this suite wrote
# fake-proxy tracebacks into the operational log, i.e. destroyed the very
# evidence surface mika#2051 exists to make legible.
assert_eq "mika#2051: the sister function does not reach the operational proxy log either" \
    "0" "$(_egress_log_fixture_hits)"

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
