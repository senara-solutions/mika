#!/usr/bin/env bash
#
# Harnais de scripts/canary-pilot-containment — la garde « un pilote frère
# vole-t-il ? » posée avant toute installation (mika#3051).
#
# POURQUOI CE FICHIER EXISTE. Le canari n'avait AUCUNE suite sur `main`. Son
# installation du proxy était inconditionnelle et tombait avant tout contrôle :
# `install -Dm755` rend le binaire plus récent que le démon en place,
# `_pilot_daemon_code_is_newer` le juge périmé, `_ensure_pilot_egress_proxy` le
# relance, le nouveau pid prend le socket, et les tunnels CONNECT du pilote qui
# vole à cet instant sont coupés. Mesuré le 2026-11-01 : installation à
# 13:57:51Z, relance à 13:57:52Z, `API Error: 502 … inference gateway` au tour 18
# du pilote en vol à 14:00:53Z, sortie sans plan. Le défaut était masqué depuis
# par une règle opératrice — « canari seulement sur moteur vide », un contrôle à
# la main avant chaque canari, qui a coûté ~40 min de report le 06/11.
#
# HERMÉTIQUE PAR CONSTRUCTION : pas de réseau, pas de jeton, pas de `bwrap`, pas
# de `cargo`, aucune écriture hors des répertoires temporaires de cette suite.
# `HOME` et `PILOT_LOG_DIR` sont redirigés vers des fixtures ; le canari refuse
# avant d'atteindre quoi que ce soit d'autre, et le seul mode exercé qui ne
# refuse pas est `--check-flight`, qui n'installe rien et ne touche aucun relais.
#
# CE QUE CETTE SUITE N'ÉTABLIT PAS, dit ici plutôt que laissé à déduire d'un
# vert. §3 établit que la garde LAISSE PASSER, pas que le canari entier tourne :
# le mode défaut, passé la garde, exécute `bwrap`, le relais et des appels
# réseau, et rien de cela n'est jouable dans un job CI hermétique. La partie de
# AC2 qui dit « les mêmes contrôles `ok:` » est donc établie STRUCTURELLEMENT par
# le scan d'ordre de §5 — aucun contrôle retiré ni réordonné, l'installation
# reste le site unique qu'elle était — et non par exécution. Le falsificateur par
# exécution reste le canari lui-même sur l'hôte, dont l'opérateur lit la sortie.
#
# Run: bash scripts/test-canary-pilot-containment.sh

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CANARY="$REPO_ROOT/scripts/canary-pilot-containment"
DISPATCH_LIB="$REPO_ROOT/skills/bundled/_shared/dispatch-lib.sh"

PASS=0
FAIL=0

ok() { PASS=$((PASS + 1)); printf '  ✓ %s\n' "$1"; }
ko() {
    FAIL=$((FAIL + 1))
    printf '  ✗ %s\n    expected: %s\n    actual:   %s\n' "$1" "$2" "$3"
}
eq() { if [ "$2" = "$3" ]; then ok "$1"; else ko "$1" "$2" "$3"; fi; }
contains() {
    if printf '%s' "$3" | grep -qF -- "$2"; then
        ok "$1"
    else
        ko "$1" "une sortie contenant « $2 »" "$(printf '%s' "$3" | tail -5)"
    fi
}

# `_canary_flight_fixture <état> <répertoire>` — pose l'état des fichiers qu'un
# pilote laisse derrière lui, tel qu'il a été MESURÉ sur l'hôte (mika#3031) : le
# `.log` naît au LANCEMENT et s'écrit jusqu'à la sortie ; le `.stderr` naît À LA
# SORTIE, copié après `PILOT_EXIT=$?`. Pendant le vol il n'existe pas. Le signal
# est donc l'APPARIEMENT, pas une fenêtre.
#
# RÉÉCRIT ICI plutôt que partagé avec `_pilot_flight_fixture` de
# test-dispatch-lib.sh, délibérément : les deux suites tournent dans deux jobs
# distincts et aucune ne source l'autre. Les faire dépendre l'une de l'autre pour
# huit lignes créerait un couplage entre jobs dont la rupture rendrait rouge la
# mauvaise suite.
_canary_flight_fixture() {   # $1 = état, $2 = répertoire de logs
    local state="$1" dir="$2" id
    case "$state" in
        en_vol) id=vol-en-cours ;;
        sorti)  id=vol-sorti ;;
        *)      return 1 ;;
    esac
    printf 'ligne de vol %s\n' "$id" > "$dir/$id.log"
    if [ "$state" = sorti ]; then
        printf 'sortie de %s\n' "$id" > "$dir/$id.stderr"
    fi
    printf '%s' "$dir/$id.log"
}

# Un `HOME` de fixture portant un faux proxy installé d'mtime CONNU et ANCIEN.
# Ancien exprès : un mtime du jour comparé à la seconde pourrait rester égal à
# lui-même après une réécriture dans la même seconde, et l'assertion « le proxy
# garde son mtime » serait verte sur une installation qui a bien eu lieu.
_canary_home_fixture() {   # $1 = répertoire HOME
    mkdir -p "$1/.local/bin"
    printf '#!/bin/sh\nexit 0\n' > "$1/.local/bin/mika-pilot-egress-proxy"
    chmod 755 "$1/.local/bin/mika-pilot-egress-proxy"
    touch -d '2026-01-01T00:00:00Z' "$1/.local/bin/mika-pilot-egress-proxy"
}

_proxy_mtime() { stat -c %y "$1/.local/bin/mika-pilot-egress-proxy" 2>/dev/null; }

# Lance le canari dans un environnement de fixture et rend `rc=<n>` + la sortie
# fusionnée. `HOME` et `PILOT_LOG_DIR` sont les deux seules portes par lesquelles
# cette suite agit sur lui : aucune surcharge de son code, aucun drapeau de test.
_run_canary() {   # $1 = HOME, $2 = PILOT_LOG_DIR, $3 = PATH, $4.. = argv
    local _home="$1" _logs="$2" _path="$3"
    shift 3
    (
        export HOME="$_home"
        export PILOT_LOG_DIR="$_logs"
        export PATH="$_path"
        cd "$REPO_ROOT" || exit 127
        bash "$CANARY" "$@" 2>&1
    )
}

printf '\n== mika#3051 — canary-pilot-containment : la garde avant l'\''installation ==\n\n'

# ---------------------------------------------------------------------------
printf -- '-- 0. le canari et le prédicat sont là où cette suite les attend\n'
# ---------------------------------------------------------------------------
[ -f "$CANARY" ] && ok "canary-pilot-containment existe" \
    || ko "canary-pilot-containment existe" "un fichier à $CANARY" "absent"
[ -f "$DISPATCH_LIB" ] && ok "dispatch-lib.sh existe" \
    || ko "dispatch-lib.sh existe" "un fichier à $DISPATCH_LIB" "absent"
T3051_PARSE=0
bash -n "$CANARY" 2>/dev/null || T3051_PARSE=$?
eq "le canari passe \`bash -n\`" "0" "$T3051_PARSE"

# ---------------------------------------------------------------------------
printf -- '\n-- 1. T4 — contrôle de l'\''instrument, tiré AVANT ce qui en dépend\n'
# ---------------------------------------------------------------------------
# Les deux états de fichiers que cette suite pose doivent être DISTINCTS sur la
# seule propriété que le prédicat regarde : le `.stderr` apparié. Sans cette
# mesure, un fixture qui poserait un `.stderr` dans les deux cas rendrait §2 et
# §3 verts sur le même état, dont un seul est le bon — précédent littéral dans ce
# dépôt (test-dispatch-lib.sh, où un `touch -d` non reconnu rendait deux états
# indiscernables et deux assertions vertes sur le mauvais).
#
# Le TROISIÈME cas de la garde — l'instant indécidable — n'est pas un état de
# fichiers : son discriminant est le shim `date` de §4, dont la contre-vacuité
# est mesurée là-bas (T5). Dit ici pour qu'on ne cherche pas un troisième
# fixture qui n'existe pas.
T3051_FIXTURE_SHAPE="$(
    for _s in en_vol sorti; do
        _d=$(mktemp -d)
        _canary_flight_fixture "$_s" "$_d" >/dev/null
        printf '%s=%s/%s ' "$_s" \
            "$(find "$_d" -maxdepth 1 -name '*.log' 2>/dev/null | wc -l | tr -d ' ')" \
            "$(find "$_d" -maxdepth 1 -name '*.stderr' 2>/dev/null | wc -l | tr -d ' ')"
        rm -rf "$_d"
    done
)"
eq "T4: seul l'état « sorti » pose un \`.stderr\` apparié" \
    "en_vol=1/0 sorti=1/1 " "$T3051_FIXTURE_SHAPE"

# Et l'mtime du faux proxy est bien posé dans le passé, sinon l'assertion de §2
# comparerait un instant du jour à lui-même.
T3051_MTIME_FIXTURE_DIR=$(mktemp -d)
_canary_home_fixture "$T3051_MTIME_FIXTURE_DIR"
eq "T4: l'mtime du faux proxy installé est posé, et dans le passé" \
    "2026" "$(_proxy_mtime "$T3051_MTIME_FIXTURE_DIR" | cut -d- -f1)"
rm -rf "$T3051_MTIME_FIXTURE_DIR"

# ---------------------------------------------------------------------------
printf -- '\n-- 2. T1 — un `.log` sans `.stderr` apparié : refus, et le proxy garde son mtime\n'
# ---------------------------------------------------------------------------
# Le mode DÉFAUT, pas `--check-flight` : c'est le mode qui installait, donc le
# seul où « rien n'a été installé » veut dire quelque chose. C'est la mesure du
# 01/11 rejouée à l'envers.
T3051_T1_HOME=$(mktemp -d)
T3051_T1_LOGS=$(mktemp -d)
_canary_home_fixture "$T3051_T1_HOME"
T3051_T1_LOG=$(_canary_flight_fixture en_vol "$T3051_T1_LOGS")
T3051_T1_MTIME_BEFORE=$(_proxy_mtime "$T3051_T1_HOME")
T3051_T1_RC=0
T3051_T1_OUT=$(_run_canary "$T3051_T1_HOME" "$T3051_T1_LOGS" "$PATH") || T3051_T1_RC=$?
T3051_T1_MTIME_AFTER=$(_proxy_mtime "$T3051_T1_HOME")

eq "T1 (AC1-0, AC3-a): le canari refuse avec 3, distinct du 1 d'un contrôle rouge" \
    "3" "$T3051_T1_RC"
eq "T1 (AC3-a): le proxy installé garde son mtime — rien n'a été écrit" \
    "$T3051_T1_MTIME_BEFORE" "$T3051_T1_MTIME_AFTER"
contains "T1 (AC1-0): le refus NOMME le \`.log\` du pilote vivant" \
    "$T3051_T1_LOG" "$T3051_T1_OUT"
contains "T1: le refus nomme le geste dû" "attendre la fin de ce vol" "$T3051_T1_OUT"
# Le trap `EXIT` a bien tourné sur un refus, et il n'a pas réécrit `rc` : c'est
# la seule branche qui le fait (`VIOLATION`), et elle ne s'applique pas ici.
contains "T1: le self-check de jeton tourne quand même, et le refus survit au trap" \
    "ok:   host_gh_token_untouched" "$T3051_T1_OUT"
rm -rf "$T3051_T1_HOME" "$T3051_T1_LOGS"

# ---------------------------------------------------------------------------
printf -- '\n-- 3. T2 — le même `.log` AVEC son `.stderr` : la garde laisse passer\n'
# ---------------------------------------------------------------------------
T3051_T2_HOME=$(mktemp -d)
T3051_T2_LOGS=$(mktemp -d)
_canary_home_fixture "$T3051_T2_HOME"
_canary_flight_fixture sorti "$T3051_T2_LOGS" >/dev/null
T3051_T2_MTIME_BEFORE=$(_proxy_mtime "$T3051_T2_HOME")
T3051_T2_RC=0
T3051_T2_OUT=$(_run_canary "$T3051_T2_HOME" "$T3051_T2_LOGS" "$PATH" --check-flight) \
    || T3051_T2_RC=$?
T3051_T2_MTIME_AFTER=$(_proxy_mtime "$T3051_T2_HOME")

eq "T2 (AC2, AC3-b): aucun pilote ne vole, la garde rend la main et le mode sort 0" \
    "0" "$T3051_T2_RC"
contains "T2 (AC2): la ligne \`ok:\` nommée est imprimée" \
    "ok:   no_sibling_pilot_in_flight" "$T3051_T2_OUT"
# `--check-flight` est la garde SEULE : il est placé avant l'installation, donc
# il n'écrit rien non plus. C'est ce qui le rend utilisable sur un hôte de boucle
# comme contrôle avant un canari.
eq "T2: \`--check-flight\` n'installe rien — l'mtime du proxy ne bouge pas" \
    "$T3051_T2_MTIME_BEFORE" "$T3051_T2_MTIME_AFTER"
rm -rf "$T3051_T2_HOME" "$T3051_T2_LOGS"

# ---------------------------------------------------------------------------
printf -- '\n-- 4. T3 + T5 — un instant de coupure indécidable : refus fail-closed\n'
# ---------------------------------------------------------------------------
# Le prédicat calcule sa coupure avec `date -u -d`, dont l'option `-d` n'a aucun
# précédent GNU-portable dans dispatch-lib et échouerait sur un `date` non-GNU.
# Le shim ci-dessous rejoue ce `date` : il ÉCHOUE sur `-d` et DÉLÈGUE sinon.
#
# L'état de fichiers posé est `sorti` — celui sur lequel la garde LAISSE PASSER.
# C'est le bon sens du contrôle : si le shim n'était pas atteint, le prédicat
# rendrait 1 et le canari sortirait 0, pas 4. Le vert ci-dessous ne peut donc pas
# venir d'un shim inerte.
T3051_T3_SHIM=$(mktemp -d)
T3051_T3_TRACE="$T3051_T3_SHIM/date-calls.txt"
T3051_REAL_DATE=$(command -v date)
{
    printf '#!/bin/sh\n'
    printf 'printf "%%s\\n" "$*" >> %s\n' "$T3051_T3_TRACE"
    printf 'for _a in "$@"; do [ "$_a" = "-d" ] && exit 1; done\n'
    printf 'exec %s "$@"\n' "$T3051_REAL_DATE"
} > "$T3051_T3_SHIM/date"
chmod 755 "$T3051_T3_SHIM/date"
: > "$T3051_T3_TRACE"

# T5 (a) — le shim fait bien les deux choses qu'on lui demande, mesuré hors du
# canari : refuser `-d`, et laisser passer le reste. Un shim qui refuserait TOUT
# casserait le canari ailleurs et le 4 ne viendrait plus de la cause voulue.
T3051_T5_SHIM_BEHAVIOUR="$(
    export PATH="$T3051_T3_SHIM:$PATH"
    _d_rc=0
    date -u -d '-1800 seconds' +%Y-%m-%dT%H:%M:%SZ >/dev/null 2>&1 || _d_rc=$?
    _plain_rc=0
    date -u +%Y >/dev/null 2>&1 || _plain_rc=$?
    printf 'avec_d=%s sans_d=%s' "$([ "$_d_rc" -ne 0 ] && echo refuse || echo passe)" \
        "$([ "$_plain_rc" -eq 0 ] && echo delegue || echo casse)"
)"
eq "T5 (a): le shim refuse \`date -u -d\` et délègue tout le reste" \
    "avec_d=refuse sans_d=delegue" "$T3051_T5_SHIM_BEHAVIOUR"

: > "$T3051_T3_TRACE"
T3051_T3_HOME=$(mktemp -d)
T3051_T3_LOGS=$(mktemp -d)
_canary_home_fixture "$T3051_T3_HOME"
_canary_flight_fixture sorti "$T3051_T3_LOGS" >/dev/null
T3051_T3_MTIME_BEFORE=$(_proxy_mtime "$T3051_T3_HOME")
T3051_T3_RC=0
T3051_T3_OUT=$(_run_canary "$T3051_T3_HOME" "$T3051_T3_LOGS" "$T3051_T3_SHIM:$PATH") \
    || T3051_T3_RC=$?
T3051_T3_MTIME_AFTER=$(_proxy_mtime "$T3051_T3_HOME")

eq "T3 (AC1-2, AC3-c): coupure indécidable ⇒ refus fail-closed avec 4" \
    "4" "$T3051_T3_RC"
eq "T3 (AC1-2): rien n'est installé non plus sur un refus indécidable" \
    "$T3051_T3_MTIME_BEFORE" "$T3051_T3_MTIME_AFTER"
contains "T3 (AC1-2): la CAUSE est nommée par le prédicat lui-même" \
    "pilot_sibling_in_flight.undecidable" "$T3051_T3_OUT"
contains "T3: et le refus dit qu'il est fail-closed" "fail-closed" "$T3051_T3_OUT"

# T5 (b) — le shim a bien été ATTEINT dans le canari. Sans cette ligne, un
# `rc=4` venu d'une tout autre cause se lirait comme la cause voulue.
T3051_T5_TRACE_SEEN="$(
    _n=$(wc -l < "$T3051_T3_TRACE" 2>/dev/null | tr -d ' ')
    _withd=$(grep -c -- '-d' "$T3051_T3_TRACE" 2>/dev/null || true)
    printf 'appels=%s avec_d=%s' \
        "$([ "${_n:-0}" -gt 0 ] && echo oui || echo non)" \
        "$([ "${_withd:-0}" -gt 0 ] && echo oui || echo non)"
)"
eq "T5 (b): le shim est atteint DANS le canari, et sur un appel portant \`-d\`" \
    "appels=oui avec_d=oui" "$T3051_T5_TRACE_SEEN"
rm -rf "$T3051_T3_HOME" "$T3051_T3_LOGS" "$T3051_T3_SHIM"

# ---------------------------------------------------------------------------
printf -- '\n-- 5. T6 — scan d'\''ordre : la garde précède l'\''installation, site unique\n'
# ---------------------------------------------------------------------------
# FIRE-DISPOSITION, option (a) : la table d'exceptions est DÉCLARÉE et VIDE, et
# son vide est asserté à l'exécution. Déclarée pour que le prochain qui veut y
# mettre une entrée voie d'abord qu'elle existe exprès ; assertée vide pour
# qu'une entrée ajoutée en silence rougisse la suite. Le détecteur part ARMÉ —
# doctrine mika#2201. C'est cette assertion-ci qui empêche la table de devenir
# l'allowlist que mika#2309 condamne.
#
# ZÉRO VIOLATION EXISTANTE, ET C'EST MESURÉ, PAS SUPPOSÉ : `install -Dm755`
# n'apparaît qu'une fois dans le fichier, et c'est le site que ce vol a déplacé.
T3051_CANARY_GUARD_ALLOWLIST=()
eq "T6: table d'exceptions du scan « la garde précède l'installation » — zero entries" \
    "0" "${#T3051_CANARY_GUARD_ALLOWLIST[@]}"

# COMPTÉ ET ANCRÉ, JAMAIS SEULEMENT ORDONNÉ. Un `garde < install` serait aussi
# vrai pour `0 < 368`, c'est-à-dire pour une garde DISPARUE. Les deux ancres sont
# donc rendues présentes dans le même verdict que l'ordre.
T3051_GUARD_LINE=$(grep -n '^_canary_refuse_if_pilot_in_flight$' "$CANARY" \
    | head -1 | cut -d: -f1)
T3051_INSTALL_LINE=$(grep -n '^install -Dm755' "$CANARY" | head -1 | cut -d: -f1)
T3051_INSTALL_SITES=$(grep -c 'install -Dm755' "$CANARY" || true)
eq "T6 (AC1, AC2): la garde est appelée, l'installation existe, et la garde précède — site unique" \
    "garde=oui install=oui ordre=garde_avant_install sites=1" \
    "garde=$([ -n "$T3051_GUARD_LINE" ] && echo oui || echo non) install=$([ -n "$T3051_INSTALL_LINE" ] && echo oui || echo non) ordre=$(
        if [ -n "$T3051_GUARD_LINE" ] && [ -n "$T3051_INSTALL_LINE" ] \
            && [ "$T3051_GUARD_LINE" -lt "$T3051_INSTALL_LINE" ]; then
            echo garde_avant_install
        else
            echo install_avant_garde
        fi
    ) sites=$T3051_INSTALL_SITES"

# LE SCAN QUE AC1 DEMANDE VRAIMENT : avant la garde, RIEN ne touche au relais, au
# proxy ni au sandbox. L'ordre seul ne le dit pas — un `_ensure_pilot_egress_proxy`
# glissé plus haut couperait les tunnels sans déplacer l'installation d'une ligne.
# Les commentaires sont retirés d'abord : ils nomment légitimement ces fonctions
# plusieurs fois au-dessus de la garde, et les compter serait le faux rouge qui
# fait désarmer un scan.
T3051_PRE_GUARD_TOUCHERS=$(awk -v g="${T3051_GUARD_LINE:-0}" '
    g == 0 { next }
    NR >= g { exit }
    /^[[:space:]]*#/ { next }
    /_ensure_pilot_helper|_ensure_pilot_egress_proxy|_run_pilot_sandboxed|install -D/ { n++ }
    END { printf "%d", n + 0 }
' "$CANARY")
eq "T6 (AC1): avant la garde, aucun geste ne touche au relais, au proxy ni au sandbox" \
    "0" "$T3051_PRE_GUARD_TOUCHERS"

# ANTI-VACUITÉ DU MOTIF ci-dessus : apparie-t-il bien les quatre gestes qu'il
# refuse, et laisse-t-il passer une ligne inoffensive ? Sans ça, `0` serait aussi
# la réponse d'une expression cassée, et le scan serait vert le jour même où
# quelqu'un remonte un `_ensure_*` au-dessus de la garde. Tiré sur des lignes
# synthétiques, jamais sur la source.
T3051_PRE_GUARD_MOTIF="$(
    _m() {
        printf '%s\n' "$1" | awk '
            /^[[:space:]]*#/ { next }
            /_ensure_pilot_helper|_ensure_pilot_egress_proxy|_run_pilot_sandboxed|install -D/ { n++ }
            END { printf "%d", n + 0 }'
    }
    printf 'helper=%s proxy=%s sandbox=%s install=%s commentaire=%s inoffensif=%s' \
        "$(_m '_ensure_pilot_helper >/dev/null 2>&1 || true')" \
        "$(_m '_ensure_pilot_egress_proxy >/dev/null 2>&1 || true')" \
        "$(_m '    _run_pilot_sandboxed /bin/true')" \
        "$(_m 'install -Dm755 "$SCRIPT" "$INSTALLED_PROXY"')" \
        "$(_m '# _ensure_pilot_helper launches mitmdump with nohup')" \
        "$(_m 'export WORKTREE_DIR="$REPO_ROOT"')"
)"
eq "T6 anti-vacuité: le motif apparie les quatre gestes, ignore le commentaire et l'inoffensif" \
    "helper=1 proxy=1 sandbox=1 install=1 commentaire=0 inoffensif=0" "$T3051_PRE_GUARD_MOTIF"

# Le mode public annoncé dans l'en-tête `INVOKE:`, et le prédicat partagé plutôt
# qu'un second scan écrit sur place.
contains "T6: \`--check-flight\` est documenté dans l'en-tête INVOKE" \
    "--check-flight" "$(sed -n '1,80p' "$CANARY")"
T3051_PREDICATE_CALLS=$(grep -vE '^[[:space:]]*#' "$CANARY" \
    | grep -cE '_pilot_sibling_pilot_in_flight' || true)
eq "T6: le canari réutilise le prédicat partagé, et l'appelle une seule fois" \
    "1" "$T3051_PREDICATE_CALLS"

# ---------------------------------------------------------------------------
printf '\n== %s réussies, %s échouées ==\n\n' "$PASS" "$FAIL"
# ---------------------------------------------------------------------------
# Un plancher de population : une suite qui n'a rien pu tirer doit le DIRE plutôt
# que rendre un vert de zéro assertion.
if [ "$PASS" -eq 0 ]; then
    echo "ERREUR: aucune assertion n'a été tirée — la suite n'a rien mesuré." >&2
    exit 2
fi
[ "$FAIL" -eq 0 ] || exit 1
exit 0
