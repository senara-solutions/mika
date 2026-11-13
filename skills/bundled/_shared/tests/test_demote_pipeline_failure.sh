#!/bin/bash
# Suite : un `PIPELINE FAILURE` périmé est DÉMOTÉ, jamais décapité
# (mika#2077, reprise de mika#2032).
#
# LE DÉFAUT QU'ELLE GARDE. Sur le chemin GROOMED de `_iterate_groom_loop`,
# dispatch-lib faisait `sed '/^PIPELINE FAILURE:/d'`. Les messages d'échec de ce
# fichier sont des BLOCS de plusieurs lignes dont une seule porte le marqueur :
# `_classify_terminated_session` écrit la tête, puis `Session:`, `Turns:`,
# `Duration:`, `Halt:`, `Halt class:`, `Retry hint:` et un paragraphe. Le `sed`
# retirait la tête et livrait tout le corps orphelin — un `Halt class:` sans
# halte, un `Session:` sans session morte. Le diagnostic arrivait tronqué
# exactement quand il comptait, et sur le chemin de SUCCÈS du grooming : donc
# sur tout groom convergé.
#
# POURQUOI PAS « supprimer jusqu'à la ligne vide ». Mesuré sur les messages
# réels : la première ligne vide suit IMMÉDIATEMENT le marqueur, donc ce strip
# emporterait la tête ET le séparateur. Strictement pire. Le remède est la
# démotion : la chaîne `PIPELINE FAILURE:` — celle que
# `milestone_manager::evaluator::dead_cause_of` lit en `contains` — disparaît, la
# prose garde une tête qui dit « posé, puis dépassé ».
#
# QUATRE PARTIES, et pourquoi il en faut quatre.
#   PART P  préconditions. Ni bwrap, ni réseau, ni token : la suite est wirable
#           sur un runner hébergé par construction.
#   PART A  comportemental, sur les MESSAGES RÉELS du fichier — produits par
#           `_classify_terminated_session` elle-même, jamais recopiés ici. Un
#           fixture recopié dérive en silence ; celui-ci se lit comme un échec.
#   PART B  inertie. Le texte de démotion est confronté à la table des motifs
#           retryables LUE DEPUIS son site de vérité. Une table recopiée dans le
#           test est l'allowlist que mika#2309 condamne : elle vieillit le jour
#           où un motif est ajouté en face.
#   PART C  anti-vacuité. Les assertions centrales rejouées sur des copies
#           régressées de dispatch-lib.sh — elles DOIVENT échouer. Sans elle,
#           des assertions dont rien ne prouve qu'elles savent dire non : la
#           classe que mika#1772 a trouvée en câblant test-dispatch-lib.sh.
#
# POURQUOI DEUX RÉGRESSIONS ET NON UNE. Le plan de vol en prévoyait une seule —
# le `sed` d'origine — pour V1, V2 et V4. **Mesuré : V2 ne peut pas y échouer.**
# Le `sed` supprimait la ligne ENTIÈRE, donc la sous-chaîne `PIPELINE FAILURE:`
# disparaissait elle aussi : contre cette copie, V2 passe, et une assertion qui
# passe dans les deux mondes ne prouve rien. Le monde que V2 refuse est l'AUTRE :
# une démotion qui garderait le marqueur ailleurs qu'en tête de ligne — la forme
# « ancrée à l'écriture, non ancrée à la lecture » que mika#2590 a fermée sur
# `Outcome:`. D'où RÉGRESSION 2, qui n'altère que le texte de démotion. Chaque
# régression a un compte d'échecs EXACT et attendu ; un seul compte commun
# aurait laissé V2 se cacher derrière les échecs de V1.
#
# Run: bash skills/bundled/_shared/tests/test_demote_pipeline_failure.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# PART C se rejoue en RÉINVOQUANT ce fichier contre une copie régressée, plutôt
# qu'en recopiant ses assertions dans un second bloc. C'est ce qui garantit que
# les assertions rejouées sont LES MÊMES, octet pour octet, et non une imitation
# qui pourrait dériver de l'original qu'elle prétend contredire.
DISPATCH_LIB="${MIKA2077_REGRESSED_LIB:-$SCRIPT_DIR/../dispatch-lib.sh}"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

# Source isolation : dispatch-lib.sh n'a aucun code impératif au niveau
# supérieur — même audit que tests/test_pilot_phase2b_argv.sh.
# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

PASS=0
FAIL=0

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected: '$expected'"
        echo "    actual:   '$actual'"
    fi
}

assert_true() {
    local label="$1" cond_rc="$2"
    if [ "$cond_rc" -eq 0 ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
    fi
}

# `sed -n '0p'` est une erreur, et une adresse négative aussi : en mode régressé
# la tête du bloc a disparu et l'index calculé passe sous 1. On rend la chaîne
# vide plutôt que de laisser l'assertion rougir pour la mauvaise raison.
nth_line() {
    local n="$2"
    [ "$n" -ge 1 ] || { printf ''; return 0; }
    printf '%s\n' "$1" | sed -n "${n}p"
}

count_lines() { printf '%s\n' "$1" | wc -l | tr -d ' '; }

# ============================================================================
# PART P — préconditions (V9)
# ============================================================================
# `awk` est la seule dépendance externe de la fonction mesurée. POSIX, présent
# partout où dispatch-lib tourne ; la suite le dit plutôt que de le supposer.
if ! command -v awk >/dev/null 2>&1; then
    echo "⊘ skipped — awk absent du PATH (la démotion le requiert)"
    exit 0
fi

TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2077-demote-XXXXXX")
trap 'rm -rf "$TMPROOT"' EXIT

# ----------------------------------------------------------------------------
# Les fixtures : les messages RÉELS, produits par leur producteur
# ----------------------------------------------------------------------------
# `_classify_terminated_session` lit des globales. On pose celles du scénario
# mika#1772 (une session tuée par `idle_timeout` sans avoir rien produit) et la
# fonction rend le bloc tel qu'un callback le porterait. `_HALT_SUBTYPE_RESOLVED`
# est posé pour court-circuiter le scrape de stderr : la résolution est mémoïsée,
# donc la poser ICI est la forme supportée et non un contournement.
SUBTYPE="idle_timeout"
TERMINATION_REASON="No meaningful progress for 300s"
SESSION_ID="16580174-a429-4f28-9c63-7851397e2a9c"
TURNS="2"
DURATION="304112"
PRE_RUN_HEAD="28dbd237"
POST_RUN_HEAD="28dbd237"
_HALT_SUBTYPE_RESOLVED="idle_timeout"
_HALT_GUARDRAIL_LINE=""

FIXTURE_FULL="$(_classify_terminated_session full)"
FIXTURE_BANNER="$(_classify_terminated_session banner)"

# Deux blocs d'échec dans un même RESULT — la forme réelle quand le post-flight
# de `_run_claude_pilot` en pose un et la validation de plan un second.
FIXTURE_TWO="${FIXTURE_BANNER}

${FIXTURE_FULL}"

# Une mention NON ANCRÉE : la prose d'un plan, un extrait de log cité dans un
# diagnostic. L'ancrage du `sed` d'origine doit survivre à la démotion.
FIXTURE_UNANCHORED="Le plan du vol dit : un PIPELINE FAILURE: périmé est démoté, jamais décapité.
Deuxième ligne, intacte elle aussi."

# ============================================================================
# PART A — la démotion sur les messages réels (V1-V5, V7)
# ============================================================================
echo ""
echo "PART A — démotion des messages réels du substrat"
echo "-------------------------------------------------"

OUT_FULL="$(printf '%s' "$FIXTURE_FULL" | _demote_pipeline_failure_markers)"
OUT_TWO="$(printf '%s' "$FIXTURE_TWO" | _demote_pipeline_failure_markers)"

# --- V1 : aucune ligne de corps orpheline. Le témoin est la ligne qui PRÉCÈDE
# `Session:` — deux lignes au-dessus, le séparateur vide entre les deux. Avec le
# `sed` d'origine, `Session:` remonte en ligne 2 et il n'y a plus de ligne n-2.
session_lineno=$(printf '%s\n' "$OUT_FULL" | grep -n '^Session: ' | head -1 | cut -d: -f1)
session_lineno="${session_lineno:-0}"
head_line="$(nth_line "$OUT_FULL" "$((session_lineno - 2))")"
sep_line="$(nth_line "$OUT_FULL" "$((session_lineno - 1))")"
rc=1
case "$head_line" in "$_DEMOTED_PIPELINE_FAILURE_PREFIX"*) [ -z "$sep_line" ] && rc=0 ;; esac
assert_true "V1: 'Session:' a toujours sa tête démotée deux lignes au-dessus, séparateur intact" "$rc"

# Et le bloc COMMENCE par cette tête : la première ligne du RESULT délivré n'est
# jamais le corps d'un échec dont l'en-tête a été retiré.
rc=1
case "$(nth_line "$OUT_FULL" 1)" in "$_DEMOTED_PIPELINE_FAILURE_PREFIX"*) rc=0 ;; esac
assert_true "V1: la sortie commence par la tête démotée, pas par du corps orphelin" "$rc"

# --- V2 : la SOUS-CHAÎNE a disparu, pas seulement l'ancrage. `grep -cF` non
# ancré, comme `evaluator.rs:305` qui lit en `contains`.
assert_eq "V2: 'PIPELINE FAILURE:' est absent de la sortie, ancré ou non" \
    "0" "$(printf '%s\n' "$OUT_FULL" | grep -cF 'PIPELINE FAILURE:' || true)"

# --- V4 : DEUX blocs démotés, pas un.
assert_eq "V4: les deux marqueurs ancrés sont démotés, pas seulement le premier" \
    "2" "$(printf '%s\n' "$OUT_TWO" | grep -c "^$_DEMOTED_PIPELINE_FAILURE_PREFIX" || true)"

# Et aucune ligne n'est PERDUE : une démotion remplace, elle ne retire pas. C'est
# la mesure qui distingue ce vol du `sed`, sur le fixture où le `sed` en retirait
# deux.
assert_eq "V4: la sortie à deux blocs a exactement autant de lignes que l'entrée" \
    "$(count_lines "$FIXTURE_TWO")" "$(count_lines "$OUT_TWO")"

# Le reste de PART A, PART B et PART C ne sont pas rejoués en mode régressé : la
# question posée là-bas est « les cinq assertions ci-dessus savent-elles dire
# non ? », et y ajouter des assertions qui passent dans les deux mondes
# brouillerait la réponse.
if [ -n "${MIKA2077_REGRESSED_LIB:-}" ]; then
    echo ""
    echo "REGRESSED_FAILED=$FAIL"
    echo "REGRESSED_PASSED=$PASS"
    exit 0
fi

# --- V3 : la prose est intégralement préservée. Mesuré par la transformation
# INVERSE : on remet `PIPELINE FAILURE:` à la place de la tête démotée et on
# exige l'identité avec l'entrée. Rien d'autre que les marqueurs n'a donc bougé.
# (Les sauts de ligne terminaux sont normalisés des DEUX côtés par `$(...)`,
# seule différence admise — awk termine toujours sa dernière ligne.)
RESTORED="$(printf '%s\n' "$OUT_FULL" \
    | LC_ALL=C awk -v marker="$_DEMOTED_PIPELINE_FAILURE_PREFIX" '
        index($0, marker) == 1 { print "PIPELINE FAILURE:" substr($0, length(marker) + 1); next }
        { print }')"
assert_eq "V3: la prose du bloc traverse octet pour octet (hors marqueur)" \
    "$FIXTURE_FULL" "$RESTORED"

RESTORED_TWO="$(printf '%s\n' "$OUT_TWO" \
    | LC_ALL=C awk -v marker="$_DEMOTED_PIPELINE_FAILURE_PREFIX" '
        index($0, marker) == 1 { print "PIPELINE FAILURE:" substr($0, length(marker) + 1); next }
        { print }')"
assert_eq "V3: idem sur le fixture à deux blocs" "$FIXTURE_TWO" "$RESTORED_TWO"

# --- V5 : une mention non ancrée survit INTACTE. C'est le comportement du `sed`
# d'origine, et le périmètre de ce vol est le résidu orphelin, pas l'ancrage.
OUT_UNANCHORED="$(printf '%s' "$FIXTURE_UNANCHORED" | _demote_pipeline_failure_markers)"
assert_eq "V5: une mention non ancrée de 'PIPELINE FAILURE:' n'est pas altérée" \
    "$FIXTURE_UNANCHORED" "$OUT_UNANCHORED"

# --- V7 : `GROOM_SUCCESS_MARKER` reste atteignable après démotion +
# `_set_outcome_line`. Le bloc `full` porte son propre
# `Outcome: PIPELINE_INCOMPLETE — …` ; la démotion n'a ni à le connaître ni à y
# toucher, parce que `_set_outcome_line` tourne juste après au site d'appel et
# rend « exactement une ligne `Outcome:` ancrée » vraie par construction. Ce
# point est ÉPINGLÉ ici plutôt que supposé : si `_set_outcome_line` cessait de le
# faire, cette assertion le dirait.
#
# Le marqueur est LU depuis son site de vérité Rust, jamais recopié — même raison
# que la table de PART B.
GROOM_MARKER="$(sed -n 's/^pub const GROOM_SUCCESS_MARKER: &str = "\(.*\)";$/\1/p' \
    "$REPO_ROOT/crates/mika-agent/src/task_state/tasks.rs")"
assert_eq "V7: GROOM_SUCCESS_MARKER est lu depuis task_state/tasks.rs" \
    "Outcome: PLAN_GROOMED" "$GROOM_MARKER"

RESULT="$OUT_FULL"
_set_outcome_line "$GROOM_MARKER"
assert_eq "V7: exactement une ligne '^Outcome: ' après démotion + _set_outcome_line" \
    "1" "$(printf '%s\n' "$RESULT" | grep -c '^Outcome: ' || true)"
assert_eq "V7: le PIPELINE_INCOMPLETE du bloc a été remplacé, pas doublé" \
    "0" "$(printf '%s\n' "$RESULT" | grep -c '^Outcome: PIPELINE_INCOMPLETE' || true)"
# `groom_result_convergence` lit une ligne qui COMMENCE par le marqueur ; la
# dernière ligne non vide est celle que `_set_outcome_line` vient d'apposer.
assert_eq "V7: la ligne de verdict est exactement le marqueur de convergence" \
    "$GROOM_MARKER" "$(printf '%s\n' "$RESULT" | grep -v '^$' | tail -1)"

# ============================================================================
# PART B — inertie : le texte de démotion ne réveille aucun lecteur (V6)
# ============================================================================
echo ""
echo "PART B — le texte démoté face à la table des motifs retryables"
echo "---------------------------------------------------------------"

# La table est LUE depuis le site qui la porte (`_guard_empty_cycle_output`), pas
# recopiée : doctrine mika#2309. Une liste recopiée dans le test vieillit en
# silence le jour où un motif est ajouté en face — et c'est précisément le jour
# où l'on voudrait que ce test parle.
RETRYABLE_ALT="$(grep -E "^[[:space:]]*if grep -qE '\(.*STRUCTURAL VIOLATION:.*\)' <<<" "$DISPATCH_LIB" \
    | sed -E "s/^[^']*'\(//; s/\)'.*$//")"
IFS='|' read -r -a RETRYABLE_TERMS <<<"$RETRYABLE_ALT"

# L'extraction a-t-elle MORDU ? Sans cette garde, une dérive du site de vérité
# viderait la table et V6 deviendrait une boucle sur zéro terme : verte, et muette.
assert_true "V6: la table des motifs retryables a été extraite (≥ 5 termes)" \
    "$([ "${#RETRYABLE_TERMS[@]}" -ge 5 ] && echo 0 || echo 1)"

# Contrôle POSITIF : la tête NON démotée doit, elle, matcher au moins un terme.
# C'est ce qui prouve que la table extraite est bien celle qui classe, et non une
# chaîne quelconque contre laquelle tout passerait.
UNDEMOTED_HEAD="$(nth_line "$FIXTURE_FULL" 1)"
matched=0
for term in "${RETRYABLE_TERMS[@]}"; do
    grep -qE -- "$term" <<<"$UNDEMOTED_HEAD" && matched=$((matched + 1))
done
assert_true "V6: contrôle positif — la tête non démotée matche au moins un motif" \
    "$([ "$matched" -ge 1 ] && echo 0 || echo 1)"

# Et la tête démotée n'en matche AUCUN.
DEMOTED_HEAD="$(nth_line "$OUT_FULL" 1)"
hits=""
for term in "${RETRYABLE_TERMS[@]}"; do
    grep -qE -- "$term" <<<"$DEMOTED_HEAD" && hits="${hits}${term} "
done
assert_eq "V6: la tête démotée ne matche aucun motif retryable" "" "$hits"

# Le même test sur le BLOC entier : un motif ancré ailleurs dans le corps
# (`^Outcome: PIPELINE_INCOMPLETE`) y est encore, et c'est normal —
# `_set_outcome_line` le remplace au site d'appel, mesuré par V7. Ce qui doit
# être vrai ici, c'est que la démotion n'AJOUTE rien à cette population et qu'elle
# en retire au moins un terme.
before_hits=0; after_hits=0
for term in "${RETRYABLE_TERMS[@]}"; do
    grep -qE -- "$term" <<<"$FIXTURE_FULL" && before_hits=$((before_hits + 1))
    grep -qE -- "$term" <<<"$OUT_FULL" && after_hits=$((after_hits + 1))
done
assert_true "V6: la démotion n'ajoute aucun motif retryable au bloc (après ≤ avant)" \
    "$([ "$after_hits" -le "$before_hits" ] && echo 0 || echo 1)"
assert_true "V6: et elle en retire au moins un (avant > après)" \
    "$([ "$before_hits" -gt "$after_hits" ] && echo 0 || echo 1)"

# ============================================================================
# PART C — anti-vacuité : les cinq assertions savent-elles dire non ? (V8)
# ============================================================================
echo ""
echo "PART C — les cinq assertions centrales sur deux copies régressées"
echo "------------------------------------------------------------------"

# Rejoue CE fichier contre une copie. Imprime "<failed> <passed>", ou rien si le
# rejeu est mort avant sa première assertion — et « rien » ne doit pas se lire
# comme « tout a échoué », ce que C1/C3 vérifient.
replay_counts() {
    local lib="$1" out="$2"
    MIKA2077_REGRESSED_LIB="$lib" bash "${BASH_SOURCE[0]}" >"$out" 2>&1
    printf '%s %s' \
        "$(sed -n 's/^REGRESSED_FAILED=//p' "$out")" \
        "$(sed -n 's/^REGRESSED_PASSED=//p' "$out")"
}

# --- RÉGRESSION 1 : le corps de la fonction redevient le `sed` d'origine. C'est
# le monde d'avant ce vol, pas une imitation écrite ici.
REG_SED="$TMPROOT/dispatch-lib-regressed-sed.sh"
awk '
    $0 ~ /^_demote_pipeline_failure_markers\(\) \{$/ { in_fn = 1; print; next }
    in_fn && $0 ~ /^\}$/ {
        print "    sed '"'"'/^PIPELINE FAILURE:/d'"'"'"
        print "}"
        in_fn = 0
        next
    }
    in_fn { next }
    { print }
' "$DISPATCH_LIB" > "$REG_SED"

rc=1; ! cmp -s "$DISPATCH_LIB" "$REG_SED" && rc=0
assert_true "C0: la copie 1 diffère de l'original (régression effective)" "$rc"
# Ancré sur la LIGNE DE CODE injectée, pas sur le motif n'importe où : la
# doctrine de `_demote_pipeline_failure_markers` et le commentaire du site
# d'appel citent tous deux le `sed` d'origine, et un compte non ancré lirait
# cette prose comme du code. 0 ici voudrait dire que la régression n'a pas mordu.
assert_eq "C0: et son corps porte bien, en code, le sed d'origine" \
    "1" "$(grep -cx "    sed '/\^PIPELINE FAILURE:/d'" "$REG_SED" || true)"

read -r reg1_failed reg1_passed <<<"$(replay_counts "$REG_SED" "$TMPROOT/replay-sed.txt")"
assert_true "C1: le rejeu 1 a produit son compte (le mode régressé s'est exécuté)" \
    "$([ -n "${reg1_failed:-}" ] && [ -n "${reg1_passed:-}" ] && echo 0 || echo 1)"
# V1 (2 assertions) et V4 (2) échouent ; V2 PASSE, parce que le `sed` faisait
# disparaître la ligne entière donc la sous-chaîne avec. C'est exactement le
# monde que V2 ne garde pas — d'où la régression 2 ci-dessous.
assert_eq "C2: contre le sed, les quatre assertions V1/V4 échouent" \
    "4" "${reg1_failed:-0}"
assert_eq "C2: et seule V2 passe (le sed effaçait aussi la sous-chaîne)" \
    "1" "${reg1_passed:-0}"

# --- RÉGRESSION 2 : la démotion garde le marqueur en fin de préfixe. Forme
# plausible et pourtant fausse — « ancrée à l'écriture, non ancrée à la lecture »
# (mika#2590) : `evaluator.rs` lit en `contains` et classerait toujours le
# callback retryable. C'est le monde que V2 refuse, et elle seule.
REG_SUBSTR="$TMPROOT/dispatch-lib-regressed-substr.sh"
awk '
    $0 ~ /^_DEMOTED_PIPELINE_FAILURE_PREFIX=/ {
        print "_DEMOTED_PIPELINE_FAILURE_PREFIX='"'"'Superseded (grooming converged, mika#2077) — ex PIPELINE FAILURE:'"'"'"
        next
    }
    { print }
' "$DISPATCH_LIB" > "$REG_SUBSTR"

rc=1; ! cmp -s "$DISPATCH_LIB" "$REG_SUBSTR" && rc=0
assert_true "C0: la copie 2 diffère de l'original (régression effective)" "$rc"
assert_eq "C0: et son texte de démotion reporte le marqueur" \
    "1" "$(grep -c "^_DEMOTED_PIPELINE_FAILURE_PREFIX=.*ex PIPELINE FAILURE:" "$REG_SUBSTR" || true)"

read -r reg2_failed reg2_passed <<<"$(replay_counts "$REG_SUBSTR" "$TMPROOT/replay-substr.txt")"
assert_true "C3: le rejeu 2 a produit son compte" \
    "$([ -n "${reg2_failed:-}" ] && [ -n "${reg2_passed:-}" ] && echo 0 || echo 1)"
assert_eq "C4: contre un marqueur reporté, V2 échoue" "1" "${reg2_failed:-0}"
assert_eq "C4: et les quatre autres passent (seule V2 garde la sous-chaîne)" \
    "4" "${reg2_passed:-0}"

# Quand un compte attendu n'est pas au rendez-vous, la sortie du rejeu est la
# seule chose qui dit POURQUOI. On la verse plutôt que de laisser un nombre nu.
if [ "${reg1_failed:-0}" != "4" ] || [ "${reg1_passed:-0}" != "1" ] \
   || [ "${reg2_failed:-0}" != "1" ] || [ "${reg2_passed:-0}" != "4" ]; then
    echo ""
    echo "    --- rejeu 1 (sed d'origine) ---"
    sed 's/^/    /' "$TMPROOT/replay-sed.txt"
    echo "    --- rejeu 2 (marqueur reporté) ---"
    sed 's/^/    /' "$TMPROOT/replay-substr.txt"
    echo "    ----------------------------------"
fi

# ============================================================================
echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="

[ "$FAIL" -eq 0 ] || exit 1
exit 0
