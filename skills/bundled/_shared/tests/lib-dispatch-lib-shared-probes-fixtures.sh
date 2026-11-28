#!/bin/bash
# Deux sondes de premier niveau partagées entre familles (mika#3178, vol 4).
#
# POURQUOI CE FICHIER EXISTE. Même cause qu'au vol 2 et qu'au vol 3 : une plage
# contiguë n'est pas auto-suffisante. Deux sondes sont DÉFINIES dans une famille
# déjà livrée et APPELÉES depuis des familles de ce vol et des suivants. Aucun
# découpage en plages contiguës ne garde cette chaîne entière.
#
# Les deux, mesurées à l'exécution pendant le groom du vol 4 — et par deux
# symptômes DIFFÉRENTS, dont le premier est le plus discret de la série :
#
#   - `_gate_probe` (agrégat l. 6858-6920, famille 8), lue par la famille 14 à
#     ses l. 11114/11120/11127/11132/11140. Symptôme : `_gate_probe: command not
#     found` à la l. 819 de la suite, et la suite MEURT sans imprimer son bloc
#     `Results:` — la forme que le vol 3 avait déjà rencontrée.
#   - `_t2178_line_of` + `_t2178_after` (agrégat l. 7419-7431, famille 9), lues
#     par la famille 13 (l. 9637), et plus loin par les familles 18
#     (l. 13509/13640/13725) et 24 (l. 18527). Symptôme : la suite tourne
#     jusqu'au bout, imprime son `Results:`, et rend UN `FAIL` de plus que
#     l'agrégat sur la même plage — `expected: yes / actual:` vide. L'appel vit
#     dans une substitution de commande, dont l'échec donne une chaîne vide au
#     lieu de tuer la suite. Une dépendance qui ne crie pas : elle se lit sur
#     l'écart de compteur avec l'agrégat, jamais sur une absence de sortie.
#
# C'est le cas que le ticket tranche d'avance : « un helper utilisé par
# plusieurs familles va dans cette bibliothèque ; un helper propre à une famille
# reste avec elle. » Promouvoir `_t2178_after` ici sert donc aussi les vols 5
# et 6, comme `REPO_ROOT` promu au vol 3 servait ce vol-ci.
#
# POURQUOI ICI ET NON DANS L'OSSATURE. `lib-dispatch-lib-suite.sh` est écrite par
# `Write` : y porter ces 76 lignes les ferait RECOPIER par le modèle, donc
# relire au lieu de prouver, et les compterait en lignes écrites (D2(b)). Extraites
# par `sed` vers ce nom-ci — admis par la permission claude-pilot#328 en `>>` —
# elles sont mot pour mot par construction. C'est le gabarit du vol 2
# (`lib-dispatch-lib-groom-gate-fixtures.sh`), repris terme pour terme.
#
# CE QUE LES DEUX PLAGES NE CONTIENNENT PAS, et c'est vérifié ligne à ligne avant
# extraction : aucune assertion `assert_*`, aucun `echo` de section. Les `echo`
# de `_gate_probe` sont des redirections vers des fichiers de son worktree
# temporaire. Sourcer ce fichier ne compte donc RIEN : les compteurs d'une suite
# restent ceux de sa seule plage, ce qui garde la parité d'AC2 lisible.
#
# Les lignes d'origine RESTENT en place dans les familles 8 et 9, où la
# redéfinition est sans effet — c'est ce qui garde exact le `diff` d'AC3 de ces
# deux suites déjà livrées, et celui des suites de ce vol.
#
# Plages d'origine, à la base 3849d757, extraites par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#328) — mot pour mot par construction :
#   - l. 6858-6920 : `_gate_probe` (famille 8). Monte un vrai worktree git, une
#     vraie plage de HEAD et un vrai `.stderr`, puis rend soit le verdict soit le
#     RESULT. Lit `$DISPATCH_LIB`, que l'ossature pose.
#   - l. 7419-7431 : `_t2178_line_of` et `_t2178_after` (famille 9). Comparateur
#     de position de deux sites dans la source de `dispatch-lib.sh` ; rend "no"
#     quand l'un des deux manque, de sorte qu'un invariant de position sur un
#     site absent n'est pas satisfait. Lit `$DISPATCH_LIB` lui aussi.
#
# Usage, depuis une suite de `_shared/tests/`, APRÈS le `source` de l'ossature :
#     source "$_SUITE_DIR/lib-dispatch-lib-shared-probes-fixtures.sh"
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
_t2178_line_of() {
    grep -nF -- "$1" "$DISPATCH_LIB" 2>/dev/null | head -1 | cut -d: -f1
}

# "yes" when $1 appears further down the source than $2, "no" otherwise —
# including when either is absent: a position invariant over a site that does
# not exist is not satisfied.
_t2178_after() {
    local a b
    a=$(_t2178_line_of "$1")
    b=$(_t2178_line_of "$2")
    if [ -n "$a" ] && [ -n "$b" ] && [ "$a" -gt "$b" ]; then printf 'yes'; else printf 'no'; fi
}
