#!/bin/bash
# Le second lecteur du callout `Plan` (mika#2120) — extrait de test-dispatch-lib.sh par mika#3178, vol 3.
# Le lecteur qui tolère le préfixe de dépôt, les fixtures inline parties au
# corpus commun (mika#2194), `_pilot_log_dir` et son ancre (mika#2165), le
# défaut de flotte armé à 150 (mika#2542), l'ancrage obligatoire du `.stderr`
# (mika#2050), `_pilot_max_turns` (mika#2039/#2496) et l'injection de la règle
# de taille (mika#2693).
#
# Plage d'origine : lignes 8005–8772 de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 8de06d66. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
# ============================================================================
# mika#2120 — le second lecteur du callout `Plan` tolère le préfixe de dépôt
# ============================================================================
#
# Parité avec `auto_pull::extract_plan_path` : les deux écritures du callout
# rendent le **même** chemin normalisé, relatif à la racine du sous-dépôt. La
# divergence entre ces deux lecteurs est la cause racine du ticket, rejouée si
# elle revient — d'où un test, pas un commentaire.

echo ""
echo "Test: _extract_plan_path — les deux écritures du callout (mika#2120)"
echo "---------------------------------------------------------------------"

# ─────────────────────────────────────────────────────────────────────────────
# mika#2194 — CES FIXTURES INLINE SONT PARTIES DANS LE CORPUS COMMUN
#
# Ce bloc portait **17 assertions à fixtures inline**, et le corpus Rust voisin
# portait **6 corps d'issue réels** : deux jeux soignés, tous les deux, et
# **aucune entrée commune**. Personne n'avait jamais exécuté les deux lecteurs
# sur la même entrée — c'est le défaut central que mika#2194 M3 relève, et il
# n'était ni dans le corps du ticket ni dans le TSV de mika#2201.
#
# Les cas sont donc rapatriés dans
# `crates/mika-agent/tests/fixtures/plan_callout_bodies/`, où les DEUX lecteurs
# les lisent. Le bloc « parité du callout Plan sur le corpus commun » en fin de
# fichier est leur nouveau site d'exécution côté bash ; `plan_callout_parity.rs`
# est l'autre. Correspondance, pour que rien ne se perde :
#
#   forme nue                       → bare-callout.md
#   forme préfixée + parité         → 1680.md … 1949.md (six corps MESURÉS)
#   préfixe ≠ `mika/`               → other-repo-prefix.md
#   quatre contrôles négatifs       → neg-brainstorms / neg-solutions /
#                                     neg-parent-traversal / neg-two-segments
#   ancrage en début de ligne       → neg-inline-prose.md
#   callout canonique / minimal     → bare-callout.md / minimal-callout.md
#   prose « Plan: » sans docs/plans → neg-prose-plan-no-dir.md
#   deux callouts, le premier gagne → double-callout.md
#
# Ne pas les réécrire ici : deux jeux de mesure pour un prédicat, c'est
# exactement la configuration qui a permis à la divergence de vivre.
#
# Les assertions **structurelles** de mika#2120 restent, elles : elles ne
# mesurent pas une entrée, elles mesurent qu'il n'y a qu'un lecteur.

# Un seul lecteur : `_detect_plan_on_branch` ne doit pas reporter sa propre
# extraction. C'est la copie-à-côté-du-lecteur qui a fait diverger les deux
# prédicats Rust pendant des mois (mika#2158) ; même discipline ici.
MIKA2120_DETECT_SRC=$(sed -n '/^_detect_plan_on_branch()/,/^}/p' "$DISPATCH_LIB")
assert_eq "mika#2120: _detect_plan_on_branch a bien été trouvée (guards the guard)" "yes" \
    "$(if [ -n "$MIKA2120_DETECT_SRC" ]; then printf 'yes'; else printf 'no'; fi)"
assert_contains "mika#2120: _detect_plan_on_branch appelle le lecteur partagé" \
    "_extract_plan_path" "$MIKA2120_DETECT_SRC"
assert_eq "mika#2120: _detect_plan_on_branch ne porte plus de motif de callout" "0" \
    "$(printf '%s\n' "$MIKA2120_DETECT_SRC" | grep -cF 'grep -oP' || true)"

echo ""
echo "Test: _pilot_log_dir — un seul point de vérité, résolu tard (mika#2165)"
echo "----------------------------------------------------------------------"
# Trois parties doivent nommer le MÊME répertoire : le pilote depuis l'intérieur
# du bac à sable (--log-dir), le bind bwrap, et l'hôte en post-flight. Ce bloc
# épingle le résolveur unique — et surtout le MOMENT de sa résolution.
#
# Pourquoi le moment est l'assertion principale. La première coupe de mika#2165
# figeait le répertoire dans une variable assignée au chargement de la lib
# (`_PILOT_LOG_DIR="${PILOT_LOG_DIR:-...}"`). Trois sondes de cette suite posent
# PILOT_LOG_DIR APRÈS le `source` ; elles se sont mises à lire
# /var/log/claude-pilot au lieu de leur tmpdir — sans erreur, comme la panne que
# le ticket répare. Une surcharge d'environnement lue une seule fois répond le
# défaut à tout appelant qui la pose trop tard, et le fait en silence.

MIKA2165_RESOLVER_SRC=$(sed -n '/^_pilot_log_dir()/,/^}/p' "$DISPATCH_LIB")
assert_eq "mika#2165: _pilot_log_dir a bien été trouvée (guards the guard)" "yes" \
    "$(if [ -n "$MIKA2165_RESOLVER_SRC" ]; then printf 'yes'; else printf 'no'; fi)"

# Le défaut littéral n'apparaît en CODE qu'à un seul endroit : le résolveur.
# Toute autre occurrence exécutable est une quatrième épellation, exactement ce
# que mika#2165 a fermé. Les lignes de commentaire sont exclues à dessein — la
# prose doit pouvoir nommer le chemin (l'en-tête du bac à sable et la note sur
# le .stderr le font) sans que nommer devienne réintroduire.
assert_eq "mika#2165: le défaut /var/log/claude-pilot n'est écrit qu'une fois en code" "1" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF '/var/log/claude-pilot' || true)"

# Aucun site ne doit revenir à une variable gelée au chargement. La forme
# dangereuse est l'assignation en COLONNE ZÉRO — au chargement de la lib, donc
# avant toute surcharge posée par l'appelant. L'assignation *dans* le résolveur
# est au contraire le mécanisme : elle s'exécute à chaque appel.
assert_eq "mika#2165: pas de variable _PILOT_LOG_DIR figée au source" "0" \
    "$(grep -cE '^_PILOT_LOG_DIR=' "$DISPATCH_LIB" || true)"
assert_eq "mika#2165: la seule assignation du résolveur est dans son corps" "1" \
    "$(printf '%s\n' "$MIKA2165_RESOLVER_SRC" | grep -cE '^[[:space:]]+_PILOT_LOG_DIR=' || true)"

# COLLISION mika#2039 — pourquoi le résolveur ASSIGNE au lieu d'IMPRIMER.
#
# La deuxième coupe de mika#2165 exposait `printf` et se lisait `$(_pilot_log_dir)`.
# Sous `set -x` (tout le dispatch tourne ainsi, BASH_XTRACEFD vers $TRACE_FILE que
# _emit_callback renvoie à l'appelant), cette forme écrit `++ printf %s <valeur>`.
# `_scrub_secrets_from_output` ne réécrit que `NAME=value` et les formes de token :
# cette ligne-là n'est couverte par rien. La garde R8 de mika#2039
# (test_sandbox_no_secret_in_argv.sh) interdit donc la FORME, sans regarder la
# valeur — et elle est passée rouge, 34/35, sur ce résolveur.
#
# La valeur en jeu ici est un chemin, jamais un identifiant. C'est précisément
# pourquoi la garde ne raisonne pas par valeur : une garde qui ferait l'exception
# pour ce printf-ci laisserait passer le prochain écrivain. Les deux autres issues
# ont été écartées — assouplir la garde échange une propriété structurelle contre
# une commodité, et emballer le printf dans `set +x` (le bracket de
# _stage_pilot_gh_token) masquerait la ligne au lieu de cesser de la produire.
#
# Cette assertion est ce qui empêche un futur contributeur de « simplifier » le
# résolveur en accesseur imprimant : la garde #2039 redeviendrait rouge, mais
# dans une autre suite, et le lien avec ce ticket serait perdu.
assert_eq "mika#2165 × mika#2039: le résolveur n'imprime pas (aucun printf/echo dans son corps)" "0" \
    "$(printf '%s\n' "$MIKA2165_RESOLVER_SRC" | grep -cE '^[[:space:]]*(printf|echo)[[:space:]]' || true)"
assert_eq "mika#2165 × mika#2039: aucune substitution \$(_pilot_log_dir) ne subsiste" "0" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF '$(_pilot_log_dir)' || true)"

# CO-LOCATION — le coût de l'accesseur assignant, refermé structurellement.
#
# Un accesseur qui assigne peut être lu périmé : un appelant qui lit
# $_PILOT_LOG_DIR sans appeler le résolveur juste avant obtient la valeur d'un
# appel précédent (ou rien). Ce serait une nouvelle panne silencieuse, la classe
# exacte que ce ticket referme — donc elle n'est pas laissée à la discipline.
#
# Règle : toute ligne de code qui LIT $_PILOT_LOG_DIR doit aussi APPELER
# _pilot_log_dir. Les lignes du corps du résolveur et les commentaires sont hors
# sujet ; les déclarations de tableau `_PILOT_LOG_BIND_ARGS` portent un autre nom
# et ne matchent pas.
MIKA2165_UNCOLOCATED=$(grep -nE '\$\{?_PILOT_LOG_DIR\b' "$DISPATCH_LIB" \
    | grep -vE '^[0-9]+:[[:space:]]*#' \
    | grep -vE '_pilot_log_dir' \
    || true)
assert_eq "mika#2165: chaque lecture de \$_PILOT_LOG_DIR appelle le résolveur sur la même ligne" "" \
    "$MIKA2165_UNCOLOCATED"

# `--log-dir` valué, jamais nu : nu, argparse retombe sur le `const` Python
# (/var/log/claude-pilot) et le pilote écrit ailleurs que là où l'hôte bind et
# relit — l'invariant à trois têtes casse sans qu'aucun test structurel ne bouge.
assert_eq "mika#2165: aucun --log-dir nu ne subsiste" "0" \
    "$(grep -cE -- '--log-dir([[:space:]]+--|[[:space:]]*$)' "$DISPATCH_LIB" || true)"
#
# Compte passé de 2 à 3 le 2026-09-21 (mika#2306) : la relance Fire-Disposition
# est un troisième lancement, et elle reproduit la forme co-localisée ci-dessus
# plutôt que de lire une valeur héritée du lancement nominal.
#
# Compte passé de 3 à 4 le 2026-10-02 (mika#2636) : la relance de taille est un
# quatrième lancement, copie structurelle du troisième, et elle reproduit la
# même forme co-localisée.
assert_eq "mika#2165: chaque --log-dir est valué par le résolveur" "4" \
    "$(grep -cF -- '--log-dir "$_PILOT_LOG_DIR"' "$DISPATCH_LIB" || true)"

# L'assertion de comportement : la surcharge posée APRÈS le source est honorée.
# C'est celle qui aurait attrapé la régression ci-dessus ; les assertions
# structurelles au-dessus n'y suffisent pas.
# Les sondes impriment $_PILOT_LOG_DIR elles-mêmes : le résolveur assigne, il ne
# rend rien sur stdout (voir la collision mika#2039 ci-dessus). Ce `printf` vit
# dans la suite de tests, pas dans le dispatch tracé — la garde R8 ne le voit pas
# et n'a pas à le voir.
_mika2165_late_override_probe() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        PILOT_LOG_DIR="/tmp/mika2165-late-override"
        _pilot_log_dir; printf '%s' "$_PILOT_LOG_DIR"
    )
}
assert_eq "mika#2165: PILOT_LOG_DIR posé APRÈS le source est honoré" \
    "/tmp/mika2165-late-override" "$(_mika2165_late_override_probe)"

_mika2165_default_probe() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        unset PILOT_LOG_DIR
        _pilot_log_dir; printf '%s' "$_PILOT_LOG_DIR"
    )
}
assert_eq "mika#2165: sans surcharge, le défaut opérationnel est rendu" \
    "/var/log/claude-pilot" "$(_mika2165_default_probe)"

# Le bind suit le résolveur, pas une copie. Un bind qui nommerait un autre
# répertoire que celui passé à --log-dir serait vert partout et vide : le
# pilote écrirait dans le tmpfs, exactement la panne d'origine.
_mika2165_bind_probe() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        PILOT_LOG_DIR=$(mktemp -d "${TMPDIR:-/tmp}/mika2165-bind.XXXXXX")
        _PILOT_LOG_BIND_ARGS=()
        _pilot_log_bind_args 2>/dev/null
        printf '%s' "${_PILOT_LOG_BIND_ARGS[*]}"
        rm -rf "$PILOT_LOG_DIR"
    )
}
MIKA2165_BIND_OUT=$(_mika2165_bind_probe)
assert_contains "mika#2165: le bind est rw (--bind, pas --ro-bind)" \
    "--bind " "$MIKA2165_BIND_OUT"
assert_eq "mika#2165: le bind nomme le répertoire du résolveur, source == cible" "yes" \
    "$(set -- $MIKA2165_BIND_OUT; if [ "${2:-}" = "${3:-x}" ] && [ -n "${2:-}" ]; then printf 'yes'; else printf 'no'; fi)"
assert_eq "mika#2165: le bind ne s'élargit pas à /var/log" "no" \
    "$(set -- $MIKA2165_BIND_OUT; if [ "${2:-}" = "/var/log" ]; then printf 'yes'; else printf 'no'; fi)"

# Le repli est une liste VIDE, jamais un bind rigide : `--bind` dont la source
# manque fait échouer bwrap en entier. Le défaut d'aujourd'hui perd un journal ;
# un bind rigide perdrait la session.
_mika2165_unwritable_probe() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        PILOT_LOG_DIR="/proc/mika2165-cannot-exist"
        _PILOT_LOG_BIND_ARGS=(sentinel)
        _pilot_log_bind_args 2>/dev/null
        printf '%s' "${_PILOT_LOG_BIND_ARGS[*]}"
    )
}
assert_eq "mika#2165: un répertoire impossible laisse la liste vide (bwrap survit)" \
    "" "$(_mika2165_unwritable_probe)"
assert_contains "mika#2165: et le repli est bruyant — la moitié hôte d'AC3" \
    "pilot_log_guard." "$(
        (
            # shellcheck disable=SC1090
            source "$DISPATCH_LIB" 2>/dev/null || true
            PILOT_LOG_DIR="/proc/mika2165-cannot-exist"
            _PILOT_LOG_BIND_ARGS=()
            _pilot_log_bind_args 2>&1 >/dev/null
        )
    )"

echo ""
echo "Test: le plafond de tours du pilote est armé à la source (mika#2496)"
echo "---------------------------------------------------------------------"
# Aucun test comportemental ne peut voir la classe que ce bloc garde. Un
# quatrième site de lancement écrit demain sans `--max-turns` ne rend AUCUNE
# décision fausse : il tourne simplement sans borne, et toutes les assertions
# existantes restent vertes. C'est la forme exacte du défaut que mika#2496
# ferme — trois sites de lancement, zéro drapeau, depuis toujours — reproduite
# un cran plus tard. D'où un scan de source.
#
# LE MOTIF EST FIGÉ, parce qu'un scan sur « tout appel `claude-pilot` » est faux
# dans les deux directions. La chaîne apparaît ~47 fois dans le fichier et une
# seule famille est un lancement : commentaires, `--ro-bind-try
# "$HOME/.local/bin/claude-pilot"`, `--relay-config …/claude-pilot.json`,
# `command -v claude-pilot`, le smoke test `timeout 15 claude-pilot --help`, et
# de la prose dans les corps `RESULT=`. Un scan naïf rougirait sur les ~44
# autres ; resserré sur la LIGNE PHYSIQUE, il manquerait deux des trois sites
# réels, écrits sur plusieurs lignes avec continuation `\` — `--max-turns` y vit
# légitimement sur une ligne que le grep ne regarde pas.
#
# Cinq termes, dont le cinquième est une correction MESURÉE du plan :
#
#   1. unité d'analyse = l'INVOCATION LOGIQUE, obtenue en recollant les
#      continuations `\`, jamais la ligne physique. C'est le terme que la forme
#      des deux sites de revise impose (fixture N2).
#   2. les lignes de commentaire sont retirées AVANT le recollement.
#   3. `claude-pilot` en POSITION DE COMMANDE : précédé d'un début d'invocation
#      (début de ligne, `;`, `|`, `&`, `(`, `{`) ou du nom du wrapper
#      `_run_pilot_sandboxed`. Un `/` n'est pas dans ce jeu, ce qui exclut les
#      chemins (`--ro-bind-try "…/bin/claude-pilot"`).
#   4. le token suivant est exigé d'être un DRAPEAU (`[[:space:]]+-`). Ça borne
#      le jeton par la droite — `claude-pilot.json` et `claude-pilot-py` ne
#      peuvent plus matcher — et ça exclut la prose.
#   5. la CARDINALITÉ est assertée à 3. Sans elle, un prédicat devenu trop
#      étroit passerait au vert en ne regardant plus rien (classe mika#2205),
#      et c'est la seule forme de panne qu'aucune des cinq fixtures ne voit.
#
# Le terme 4 N'EST PAS dans le plan, et il a été ajouté sur une mesure : le plan
# nomme « de la prose dans les corps `RESULT=` » parmi ce que le scan doit
# exclure, mais ses quatre termes ne l'excluaient pas. `dispatch-lib.sh` porte
# `claude-pilot FAILED (exit code ${PILOT_EXIT}).` EN COLONNE ZÉRO, à
# l'intérieur d'une chaîne `RESULT="…"` multi-lignes — donc « début de ligne »,
# donc candidat, donc cardinalité 4 au lieu de 3. Le terme 4 l'écarte sans rien
# coûter : un lancement porte toujours des drapeaux. Fixture N5.
#
# Le terme 3 est délibérément une DISJONCTION (le wrapper *ou* la position de
# commande) et non le wrapper seul : un site futur qui appellerait le binaire
# sans passer par `_run_pilot_sandboxed` échapperait à un prédicat ancré sur le
# wrapper — et ce site-là est aussi celui qui perdrait le confinement réseau
# (mika#2049), donc il doit rougir ici plutôt que passer. Fixture N4.

# L'allowlist est **livrée vide**. Quand ce scan tire, la résolution est d'ARMER
# le site, jamais de l'allowlister : doctrine mika#2201, « on déclare, on
# n'allowliste pas ». Un site de lancement qu'on ne veut pas armer est un site
# qu'il faut supprimer.
MIKA2496_LAUNCH_EXCEPTIONS=()

# Rend une invocation logique par ligne : commentaires retirés, continuations
# recollées. Le `sed` est l'idiome standard de recollement (`N` + `ta`).
_mika2496_logical_invocations() {
    grep -vE '^[[:space:]]*#' "$1" | sed -e :a -e '/\\$/N; s/\\\n/ /; ta'
}

# Les invocations logiques qui LANCENT claude-pilot (termes 3 et 4).
_mika2496_launch_candidates() {
    _mika2496_logical_invocations "$1" \
        | grep -E '(^|[;|&(){}]|_run_pilot_sandboxed)[[:space:]]*claude-pilot[[:space:]]+-' \
        || true
}

# Celles qui le lancent SANS plafond — la population que le scan refuse.
_mika2496_unbounded_launches() {
    local line
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        local skip=""
        local exc
        for exc in "${MIKA2496_LAUNCH_EXCEPTIONS[@]+"${MIKA2496_LAUNCH_EXCEPTIONS[@]}"}"; do
            case "$line" in *"$exc"*) skip=1 ;; esac
        done
        [ -n "$skip" ] && continue
        case "$line" in *--max-turns*) ;; *) printf '%s\n' "$line" ;; esac
    done < <(_mika2496_launch_candidates "$1")
}

# Terme 5 — la cardinalité. Anti-vacuité : un scan qui ne trouve PERSONNE se lit
# exactement comme un scan propre.
# Compte passé de 3 à 4 le 2026-10-02 (mika#2636) : la relance de taille est un
# quatrième site de lancement, et il porte `--max-turns` comme les trois autres.
assert_eq "mika#2496: le scan voit exactement les quatre sites de lancement" "4" \
    "$(_mika2496_launch_candidates "$DISPATCH_LIB" | wc -l | tr -d ' ')"

# L'assertion elle-même : aucun lancement sans son plafond.
assert_eq "mika#2496: aucun site de lancement ne part sans --max-turns" "" \
    "$(_mika2496_unbounded_launches "$DISPATCH_LIB" | cut -c1-100)"

assert_eq "mika#2496: l'allowlist des lancements non bornés est livrée vide" "0" \
    "${#MIKA2496_LAUNCH_EXCEPTIONS[@]}"

# --- Contrôles négatifs N1–N5, sur les formes d'appel RÉELLEMENT présentes ---
#
# Un scan vert par vacuité sur les formes qu'il ne reconnaît pas est
# indistinguable d'un scan qui couvre (mika#2205). Chaque fixture est donc
# calquée sur une forme qui existe dans `dispatch-lib.sh`, jamais inventée, et
# chacune est vue rouge ou vue verte explicitement.
MIKA2496_FIXDIR=$(mktemp -d "${TMPDIR:-/tmp}/mika2496-fixtures.XXXXXX")

# N1 — mono-ligne, la forme du dispatch principal, sans le drapeau. ROUGE.
printf '%s\n' \
    '    _pilot_log_dir; _run_pilot_sandboxed claude-pilot --verbose --log-dir "$D" --task-id "$I" --command "$C" -- "$P"' \
    > "$MIKA2496_FIXDIR/n1"

# N2 — multi-lignes avec continuations, la forme des deux pilotes de revise,
# sans le drapeau. ROUGE — atteste le recollement du terme 1.
printf '%s\n' \
    '    _pilot_log_dir; _run_pilot_sandboxed claude-pilot --verbose --log-dir "$D" --task-id "$I" \' \
    '        --command "/mika-revise-plan" $CWD_ARGS \' \
    '        -- "@$F"' \
    > "$MIKA2496_FIXDIR/n2"

# N3 — même forme, drapeau présent mais SUR UNE LIGNE DE CONTINUATION. VERT.
# Le miroir de N2 : sans lui, « le scan lit l'invocation » ne se distingue pas
# de « le scan rougit sur tout multi-ligne ».
printf '%s\n' \
    '    _pilot_log_dir; _run_pilot_sandboxed claude-pilot --verbose --log-dir "$D" --task-id "$I" \' \
    '        --max-turns 120 \' \
    '        --command "/mika-revise-plan" $CWD_ARGS' \
    > "$MIKA2496_FIXDIR/n3"

# N4 — appel direct, SANS `_run_pilot_sandboxed`, sans le drapeau. ROUGE —
# atteste la disjonction du terme 3.
printf '%s\n' \
    '    claude-pilot --verbose --log-dir "$D" --task-id "$I" --command "$C"' \
    > "$MIKA2496_FIXDIR/n4"

# N5 — bonne foi : un commentaire, un chemin bindé, la config du relais, et la
# prose `RESULT=` en colonne zéro. VERT — sans quoi le scan serait rouge en
# permanence, donc désarmé.
printf '%s\n' \
    '# on lancera claude-pilot --verbose ici un jour' \
    '            --ro-bind-try "$HOME/.local/bin/claude-pilot" "$HOME/.local/bin/claude-pilot" \' \
    '            --ro-bind-try "$HOME/.local/share/uv/tools/claude-pilot" "$X" \' \
    '        CWD_ARGS="$CWD_ARGS --relay-config $W/.claude/claude-pilot.json"' \
    '    command -v claude-pilot >/dev/null 2>&1 || exit 1' \
    '    if ! timeout 15 claude-pilot --help >/dev/null 2>&9; then' \
    'claude-pilot FAILED (exit code ${PILOT_EXIT}).' \
    > "$MIKA2496_FIXDIR/n5"

for _mika2496_red in n1 n2 n4; do
    assert_eq "mika#2496: fixture $_mika2496_red est VUE ROUGE (lancement sans plafond)" "1" \
        "$(_mika2496_unbounded_launches "$MIKA2496_FIXDIR/$_mika2496_red" | wc -l | tr -d ' ')"
done
assert_eq "mika#2496: fixture n3 est VUE VERTE (drapeau sur une continuation)" "0" \
    "$(_mika2496_unbounded_launches "$MIKA2496_FIXDIR/n3" | wc -l | tr -d ' ')"
assert_eq "mika#2496: fixture n3 EST bien un candidat (le vert n'est pas de la vacuité)" "1" \
    "$(_mika2496_launch_candidates "$MIKA2496_FIXDIR/n3" | wc -l | tr -d ' ')"
assert_eq "mika#2496: fixture n5 (bonne foi) n'est candidate à rien" "0" \
    "$(_mika2496_launch_candidates "$MIKA2496_FIXDIR/n5" | wc -l | tr -d ' ')"

rm -rf "$MIKA2496_FIXDIR"

# --- CO-LOCATION, le coût de l'accesseur assignant, refermé structurellement ---
#
# Même mécanisme et même raison que pour `$_PILOT_LOG_DIR` (mika#2165) : un
# accesseur qui ASSIGNE peut être lu périmé, et un site de lancement qui lirait
# `$_PILOT_MAX_TURNS` sans appeler `_pilot_max_turns` juste avant hériterait du
# plafond d'un appel précédent — ou de rien, c'est-à-dire d'un dispatch non
# borné qui se lirait comme borné.
# La co-location se lit sur l'INVOCATION LOGIQUE, pas sur la ligne physique —
# même unité d'analyse que le scan ci-dessus, et pour la même raison : deux des
# trois sites portent le drapeau sur une ligne de continuation, donc la lecture
# de `$_PILOT_MAX_TURNS` et l'appel du résolveur y vivent sur des lignes
# physiques différentes tout en appartenant à la même commande. Un prédicat à la
# ligne les accuserait toutes les deux.
MIKA2496_UNCOLOCATED=$(_mika2496_logical_invocations "$DISPATCH_LIB" \
    | grep -E '\$\{?_PILOT_MAX_TURNS\b' \
    | grep -vE '_pilot_max_turns' \
    || true)
assert_eq "mika#2496: chaque lecture de \$_PILOT_MAX_TURNS appelle le résolveur sur la même ligne" "" \
    "$MIKA2496_UNCOLOCATED"

# Le résolveur ASSIGNE, il n'imprime pas — collision mika#2039, identique à
# celle que `_pilot_log_dir` documente : un accesseur imprimant se lirait
# `$(_pilot_max_turns)` et poserait `++ printf %s <valeur>` dans la trace
# `set -x`, forme qu'aucun scrubber ne couvre.
MIKA2496_RESOLVER_SRC=$(sed -n '/^_pilot_max_turns()/,/^}/p' "$DISPATCH_LIB")
assert_eq "mika#2496: _pilot_max_turns a bien été trouvée (guards the guard)" "yes" \
    "$(if [ -n "$MIKA2496_RESOLVER_SRC" ]; then printf 'yes'; else printf 'no'; fi)"
assert_eq "mika#2496 × mika#2039: le résolveur n'imprime pas" "0" \
    "$(printf '%s\n' "$MIKA2496_RESOLVER_SRC" | grep -cE '^[[:space:]]*(printf|echo)[[:space:]]' || true)"
assert_eq "mika#2496 × mika#2039: aucune substitution \$(_pilot_max_turns) ne subsiste" "0" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF '$(_pilot_max_turns)' || true)"

# --- AC7 : rien n'annonce un budget dollars qui n'existe pas ---
#
# `--max-budget` serait accepté, validé, résolu, porté jusqu'à
# `_sdk_guardrail_kwargs` — et ignoré (son `if config.maxBudgetUsd > 0:` se
# termine sur `pass`). Le passer ferait paraître le budget fermé dans l'argv.
assert_eq "mika#2496 (AC7): aucun site ne passe --max-budget" "0" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF -- '--max-budget' || true)"

# --- V4 : le comportement du résolveur, les trois paliers ---
# `$2` (optionnel, mika#2542) : le CSV des labels du ticket, tel que les sites de
# lancement le passent au résolveur.
_mika2496_resolve_probe() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        unset PILOT_MAX_TURNS
        if [ "$1" != "__UNSET__" ]; then export PILOT_MAX_TURNS="$1"; fi
        _pilot_max_turns "${2:-}"
        printf '%s|%s|%s' "$_PILOT_MAX_TURNS" "$_PILOT_MAX_TURNS_SOURCE" "$_PILOT_MAX_TURNS_INVALID"
    )
}
# mika#2542 (D2) : le défaut de flotte est ARMÉ à 150 — V2 de mika#2496 rapportée.
# Le diff de cette assertion (`|default|` → `150|default|`) EST la décision.
assert_eq "mika#2542 (AC9): sans surcharge, le défaut de flotte est armé à 150" \
    "150|default|" "$(_mika2496_resolve_probe __UNSET__)"
assert_eq "mika#2496: une valeur entière est honorée, provenance env" \
    "120|env|" "$(_mika2496_resolve_probe 120)"
# Contrôle négatif du rollback (AC4) : `0` omet le drapeau, ce qui rend le
# comportement d'avant mika#2496 — claude-pilot retombe sur maxTurns=200.
assert_eq "mika#2496 (AC4): PILOT_MAX_TURNS=0 est le ROLLBACK, pas une borne" \
    "|env|" "$(_mika2496_resolve_probe 0)"
assert_eq "mika#2496 (AC4): une valeur vide vaut rollback" \
    "|env|" "$(_mika2496_resolve_probe '')"
# Un désarmement par coquille sur un frein de coût serait la panne silencieuse
# que tout ceci ferme : il retombe au défaut ET il est DIT.
assert_eq "mika#2496: une valeur illisible retombe au défaut et est nommée" \
    "150|default|abc" "$(_mika2496_resolve_probe abc)"
assert_eq "mika#2496: une valeur négative retombe au défaut et est nommée" \
    "150|default|-5" "$(_mika2496_resolve_probe '-5')"

# --- V4 : le drapeau atteint l'argv, avec la valeur résolue ---
#
# C'est LE contrat côté mika. « Le budget tue avant 120 tours » est exécuté par
# le SDK, dans un autre processus, avec un vrai fournisseur — la moitié
# comportementale est la sonde S2, pas ce harnais.
_mika2496_argv_probe() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        unset PILOT_MAX_TURNS
        if [ "$1" != "__UNSET__" ]; then export PILOT_MAX_TURNS="$1"; fi
        _pilot_max_turns "${2:-}"
        # shellcheck disable=SC2086
        printf '%s' "claude-pilot --verbose ${_PILOT_MAX_TURNS:+--max-turns $_PILOT_MAX_TURNS} --command /mika"
    )
}
assert_contains "mika#2496 (AC1): armé, l'argv porte --max-turns avec la valeur résolue" \
    '--max-turns 120' "$(_mika2496_argv_probe 120)"
# Depuis mika#2542 le défaut est armé : le seul désarmement est le rollback `0`.
assert_not_contains "mika#2496 (AC4): désarmé, l'argv ne porte AUCUN --max-turns" \
    '--max-turns' "$(_mika2496_argv_probe 0)"
assert_contains "mika#2542 (AC2): sans surcharge, l'argv porte le défaut armé" \
    '--max-turns 150' "$(_mika2496_argv_probe __UNSET__)"

# --- U2/AC2 : le budget en vigueur est dit, et la ligne lit l'ARGV ---
_mika2496_budget_line() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        unset PILOT_MAX_TURNS
        if [ "$1" != "__UNSET__" ]; then export PILOT_MAX_TURNS="$1"; fi
        _pilot_max_turns "${2:-}"
        # shellcheck disable=SC2086
        _emit_pilot_budget_line claude-pilot --verbose ${_PILOT_MAX_TURNS:+--max-turns $_PILOT_MAX_TURNS} --command /mika 2>&1
    )
}
assert_contains "mika#2496 (AC2): armé par l'env, la ligne dit la valeur et sa provenance" \
    'pilot_budget_armed max_turns=120 source=env cost_bound=absent_upstream' \
    "$(_mika2496_budget_line 120)"
assert_contains "mika#2496 (AC2): désarmé, la ligne dit max_turns=none — jamais une borne inventée" \
    'pilot_budget_armed max_turns=none source=env cost_bound=absent_upstream' \
    "$(_mika2496_budget_line 0)"
assert_contains "mika#2542 (AC2): sans surcharge, la ligne dit le défaut armé et sa provenance" \
    'pilot_budget_armed max_turns=150 source=default cost_bound=absent_upstream' \
    "$(_mika2496_budget_line __UNSET__)"
assert_contains "mika#2496: une coquille est nommée entre guillemets" \
    'pilot_budget_invalid PILOT_MAX_TURNS="oops"' \
    "$(_mika2496_budget_line oops)"
# AC7 : la ligne REFUSE d'annoncer un budget dollars, parce qu'il n'en existe
# aucun en amont. Elle nomme l'absence au lieu de la taire.
assert_not_contains "mika#2496 (AC7): la ligne n'annonce jamais un budget dollars" \
    'cost_bound=40' "$(_mika2496_budget_line 120)"
# Ancrage obligatoire à la lecture (mika#2050) : le `.stderr` porte aussi la
# prose du pilote, et une session discutant du signal s'est déjà lue comme une
# émission.
assert_contains "mika#2496: la ligne est ancrable sur '^dispatch-lib: '" \
    'dispatch-lib: pilot_budget_armed' "$(_mika2496_budget_line 120)"

# La ligne est émise depuis `_run_pilot_sandboxed`, donc SOUS la redirection
# `2>"$STDERR_FILE"` du site de lancement — donc dans le sillon forensique
# per-dispatch. L'émettre avant la ligne de lancement l'enverrait sur le stderr
# propre de dispatch-lib, que l'exécuteur ne lit QUE sur `if !status.success()` :
# sur un dispatch qui réussit, le tuyau est jeté sans être lu (Signal M,
# mesuré par mika#2050). Cette assertion est ce qui empêche un futur
# contributeur de « simplifier » en déplaçant l'appel sur la ligne de lancement.
assert_eq "mika#2496: la ligne est émise depuis _run_pilot_sandboxed, pas avant le lancement" "1" \
    "$(sed -n '/^_run_pilot_sandboxed()/,/^}/p' "$DISPATCH_LIB" | grep -cF '_emit_pilot_budget_line' || true)"
# Un seul APPEL en production — la définition est exclue par sa parenthèse, pas
# par son indentation : un appel écrit en colonne zéro resterait compté.
assert_eq "mika#2496: et depuis nulle part ailleurs" "1" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -E '_emit_pilot_budget_line' | grep -cvE '_emit_pilot_budget_line\(\)' || true)"

# --- mika#2542 : le plafond de tours se résout depuis le label du ticket ---
echo ""
echo "Test: le plafond de tours se résout depuis le label du ticket (mika#2542)"
echo "-------------------------------------------------------------------------"
# Trois implements `loop-substrate` coupés à 151 tours sous le plafond 150. Le
# ticket porteur du label reçoit 200 ; tout autre reste à 150. L'ORDRE de la
# cascade est le livrable : l'hôte de production porte `PILOT_MAX_TURNS=150`,
# donc un palier label placé sous l'env serait inerte partout où il compte
# (classe mika#2205). Les probes réutilisent celles de mika#2496, étendues d'un
# second argument (le CSV des labels).

# Les deux allowlists sont LIVRÉES VIDES (doctrine mika#2201 : on déclare, on
# n'allowliste pas). Quand G1 tire : déclarer le label dans labels.yml. Quand G2
# tire : passer "${LABELS:-}" au résolveur.
MIKA2542_CEILING_LABEL_EXCEPTIONS=()
MIKA2542_UNARGUMENTED_SITES=()
assert_eq "mika#2542: l'allowlist des labels non déclarés est livrée vide" "0" \
    "${#MIKA2542_CEILING_LABEL_EXCEPTIONS[@]}"
assert_eq "mika#2542: l'allowlist des sites sans argument est livrée vide" "0" \
    "${#MIKA2542_UNARGUMENTED_SITES[@]}"

# --- G3 / V1 : la cascade, palier par palier (`valeur|source|invalid`) ---
assert_eq "mika#2542 (AC1): un ticket loop-substrate reçoit 200, provenance label" \
    "200|label|" "$(_mika2496_resolve_probe __UNSET__ loop-substrate)"
assert_eq "mika#2542 (AC2): sans label, une valeur d'env reste honorée" \
    "120|env|" "$(_mika2496_resolve_probe 120 '')"
# R1-bis — le cas qui décide si le ticket a un effet en production.
assert_eq "mika#2542 (AC3): le label BAT l'env (sinon inerte sur l'hôte de prod)" \
    "200|label|" "$(_mika2496_resolve_probe 150 loop-substrate)"
assert_eq "mika#2542 (AC3): le label bat l'env même quand l'env vaut plus bas" \
    "200|label|" "$(_mika2496_resolve_probe 120 loop-substrate)"
# Le rollback mika#2496 reste un rollback, label ou pas.
assert_eq "mika#2542 (AC4): PILOT_MAX_TURNS=0 reste le ROLLBACK, le label n'a pas voix" \
    "|env|" "$(_mika2496_resolve_probe 0 loop-substrate)"
assert_eq "mika#2542 (AC4): une valeur vide reste le rollback sur un ticket substrat" \
    "|env|" "$(_mika2496_resolve_probe '' loop-substrate)"
# L'invalidité est dite indépendamment du palier qui décide.
assert_eq "mika#2542: une coquille sur un ticket substrat — le label décide ET la coquille est nommée" \
    "200|label|abc" "$(_mika2496_resolve_probe abc loop-substrate)"
assert_eq "mika#2542: une coquille sans label retombe au défaut armé, nommée" \
    "150|default|abc" "$(_mika2496_resolve_probe abc '')"

# --- G3 / V2 : appariement EXACT sur un élément du CSV (AC6) ---
#
# Fixtures négatives d'abord : un glob de sous-chaîne (`*loop-substrate*`, la
# forme de `_label_to_type`) relèverait chacune d'elles. Vues rouges avant
# d'être vues vertes — c'est la Halte 4 du plan tenue à l'unité.
for _mika2542_near in not-loop-substrate loop-substrate-v2 xloop-substrate loop-substratex \
    'bug,loop-substrate-v2' 'not-loop-substrate,p1-important' 'LOOP-SUBSTRATE' ' loop-substrate'; do
    assert_eq "mika#2542 (AC6): '$_mika2542_near' ne relève RIEN" \
        "150|default|" "$(_mika2496_resolve_probe __UNSET__ "$_mika2542_near")"
done
# Contrôle de bonne foi : le label relève en tête, au milieu et en queue de CSV.
for _mika2542_pos in 'loop-substrate,p1-important' 'ready,loop-substrate,p1-important' \
    'p1-important,loop-substrate'; do
    assert_eq "mika#2542 (AC6): '$_mika2542_pos' relève bien" \
        "200|label|" "$(_mika2496_resolve_probe __UNSET__ "$_mika2542_pos")"
done

# --- G3 / V3 : le drapeau atteint l'argv avec la valeur du label ---
assert_contains "mika#2542 (AC1): un ticket loop-substrate porte --max-turns 200 dans l'argv" \
    '--max-turns 200' "$(_mika2496_argv_probe 150 loop-substrate)"
assert_not_contains "mika#2542 (AC4): rollback sur un ticket substrat — aucun --max-turns" \
    '--max-turns' "$(_mika2496_argv_probe 0 loop-substrate)"

# --- G3 / V5 : la ligne d'observabilité ---
assert_contains "mika#2542 (AC1): la ligne dit source=label et QUEL label a décidé" \
    'pilot_budget_armed max_turns=200 source=label label=loop-substrate cost_bound=absent_upstream' \
    "$(_mika2496_budget_line 150 loop-substrate)"
# Contrôle négatif : aucun champ `label=` quand le label n'a pas décidé — un
# champ vide se lirait comme un label nommé « vide ».
assert_not_contains "mika#2542 (AC2): sans label, aucun champ label= n'est émis" \
    'label=' "$(_mika2496_budget_line 150 '')"
assert_not_contains "mika#2542 (AC4): rollback sur un ticket substrat, aucun champ label=" \
    'label=' "$(_mika2496_budget_line 0 loop-substrate)"
assert_not_contains "mika#2542 (AC7 mika#2496): la ligne relevée n'annonce pas de budget dollars" \
    'cost_bound=4' "$(_mika2496_budget_line __UNSET__ loop-substrate)"

# --- G4 : la valeur du défaut in-file, pinnée littéralement ---
#
# Lue dans la source, pas au comportement : l'assertion comportementale
# (`150|default|`) rougirait aussi, mais celle-ci nomme LE site à relire.
assert_eq "mika#2542 (AC9): le défaut in-file de _pilot_max_turns vaut 150" "1" \
    "$(sed -n '/^_pilot_max_turns()/,/^}/p' "$DISPATCH_LIB" | grep -cE '^[[:space:]]*local _default="150"$' || true)"

# --- AC5 : un seul site nommé, et aucun second lecteur de label ne décide ---
assert_eq "mika#2542 (AC5): PILOT_LABEL_TURN_CEILINGS est déclarée exactement une fois" "1" \
    "$(grep -cE '^PILOT_LABEL_TURN_CEILINGS=\(' "$DISPATCH_LIB" || true)"
assert_eq "mika#2542 (AC5): la table porte loop-substrate=200" "1" \
    "$(sed -n '/^PILOT_LABEL_TURN_CEILINGS=(/,/^)/p' "$DISPATCH_LIB" | grep -cxE '[[:space:]]*"loop-substrate=200"' || true)"
# Le seul consommateur de la table est le helper ; le seul appelant du helper
# est le résolveur. Un second lecteur ferait deux sites de décision.
assert_eq "mika#2542 (AC5): la table n'est itérée que par _pilot_label_turn_ceiling" "1" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF '"${PILOT_LABEL_TURN_CEILINGS[@]}"' || true)"
assert_eq "mika#2542 (AC5): _pilot_label_turn_ceiling n'est appelée que depuis le résolveur" "1" \
    "$(sed -n '/^_pilot_max_turns()/,/^}/p' "$DISPATCH_LIB" | grep -cF '_pilot_label_turn_ceiling' || true)"
assert_eq "mika#2542 (AC5): …et depuis nulle part ailleurs" "1" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -F '_pilot_label_turn_ceiling' | grep -cvE '_pilot_label_turn_ceiling\(\)' || true)"
# Le helper assigne, il n'imprime pas (mika#2039).
assert_eq "mika#2542 × mika#2039: le helper de label n'imprime pas" "0" \
    "$(sed -n '/^_pilot_label_turn_ceiling()/,/^}/p' "$DISPATCH_LIB" | grep -cE '^[[:space:]]*(printf|echo)[[:space:]]' || true)"

# --- G2 / V4 : les trois sites passent les labels au résolveur (AC10) ---
#
# Sur les invocations LOGIQUES (continuations recollées), même unité que le scan
# mika#2496. Un appel `_pilot_max_turns` suivi d'un `;` ou d'une fin de ligne est
# un site qui a oublié l'argument : il résout « aucun label », donc 150 — fail-
# safe, mais silencieusement hors du mécanisme.
# L'amorce est « tout caractère hors identifiant », pas une liste de ponctuations :
# un appel écrit après un mot-clé (`then _pilot_max_turns …`) doit être vu aussi.
_mika2542_resolver_calls() {
    _mika2496_logical_invocations "$1" \
        | grep -E '(^|[^A-Za-z0-9_])_pilot_max_turns([^A-Za-z0-9_]|$)' \
        | grep -vE '^[[:space:]]*_pilot_max_turns\(\)' \
        || true
}
_mika2542_unargumented_calls() {
    local line exc skip
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        skip=""
        for exc in "${MIKA2542_UNARGUMENTED_SITES[@]+"${MIKA2542_UNARGUMENTED_SITES[@]}"}"; do
            case "$line" in *"$exc"*) skip=1 ;; esac
        done
        [ -n "$skip" ] && continue
        if ! grep -qE '_pilot_max_turns[[:space:]]+"\$\{LABELS:-\}"' <<<"$line"; then
            printf '%s\n' "$line"
        fi
    done < <(_mika2542_resolver_calls "$1")
}
# Compte passé de 3 à 4 le 2026-10-02 (mika#2636) : la relance de taille appelle
# le résolveur sur son propre site, en lui passant `"${LABELS:-}"` comme les
# trois autres — un site qui l'oublierait résoudrait « aucun label », donc le
# défaut, donc fail-safe vers 150 et jamais vers 200.
# Puis de 4 à 5 le 2026-10-14 (mika#2693) : l'injection de la règle de taille
# dans le prompt du groomeur a besoin du plafond pour en dériver la borne par
# vol, et `_pilot_max_turns` reste le LECTEUR UNIQUE de la table des labels
# (AC5) — le cinquième site est donc un appel du résolveur, jamais une seconde
# lecture de `PILOT_LABEL_TURN_CEILINGS`. C'est précisément ce que ce
# recensement existe pour rendre visible : il n'empêche pas d'ajouter un site,
# il empêche d'en ajouter un en silence.
# Puis de 5 à 6 (mika#2693, revue) : le finding synthétique du rattrapage rend
# la même règle que le groom, donc il lui faut le même plafond, par le même
# résolveur.
assert_eq "mika#2542 (AC10): le scan voit exactement six appels du résolveur" "6" \
    "$(_mika2542_resolver_calls "$DISPATCH_LIB" | wc -l | tr -d ' ')"
assert_eq "mika#2542 (AC10): chaque appel du résolveur passe \"\${LABELS:-}\"" "" \
    "$(_mika2542_unargumented_calls "$DISPATCH_LIB" | cut -c1-100)"

# Contrôles négatifs de G2, calqués sur la forme réelle des sites.
MIKA2542_FIXDIR=$(mktemp -d "${TMPDIR:-/tmp}/mika2542-fixtures.XXXXXX")
printf '%s\n' \
    '    _pilot_log_dir; _pilot_max_turns; _run_pilot_sandboxed claude-pilot --verbose ${_PILOT_MAX_TURNS:+--max-turns $_PILOT_MAX_TURNS} --command "$C"' \
    > "$MIKA2542_FIXDIR/g2_red"
printf '%s\n' \
    '    _pilot_log_dir; _pilot_max_turns "${LABELS:-}"; _run_pilot_sandboxed claude-pilot --verbose --task-id "$I" \' \
    '        ${_PILOT_MAX_TURNS:+--max-turns $_PILOT_MAX_TURNS} \' \
    '        --command "/mika-revise-plan"' \
    > "$MIKA2542_FIXDIR/g2_green"
assert_eq "mika#2542: fixture g2_red (site sans argument) est VUE ROUGE" "1" \
    "$(_mika2542_unargumented_calls "$MIKA2542_FIXDIR/g2_red" | wc -l | tr -d ' ')"
assert_eq "mika#2542: fixture g2_green (argument passé, multi-lignes) est VUE VERTE" "0" \
    "$(_mika2542_unargumented_calls "$MIKA2542_FIXDIR/g2_green" | wc -l | tr -d ' ')"
assert_eq "mika#2542: fixture g2_green EST bien un appel vu (le vert n'est pas de la vacuité)" "1" \
    "$(_mika2542_resolver_calls "$MIKA2542_FIXDIR/g2_green" | wc -l | tr -d ' ')"
printf '%s\n' \
    '    if [ -n "$X" ]; then _pilot_max_turns; fi' \
    > "$MIKA2542_FIXDIR/g2_keyword"
assert_eq "mika#2542: fixture g2_keyword (appel après then, sans argument) est VUE ROUGE" "1" \
    "$(_mika2542_unargumented_calls "$MIKA2542_FIXDIR/g2_keyword" | wc -l | tr -d ' ')"
rm -rf "$MIKA2542_FIXDIR"

# Auto-nettoyage des allowlists (exigence (a) de mika#1574) : une entrée qui
# n'apparie plus rien fait rougir. Livrées vides, elles le restent sans qu'un
# relecteur ait à y penser.
_mika2542_stale=""
for _mika2542_exc in "${MIKA2542_UNARGUMENTED_SITES[@]+"${MIKA2542_UNARGUMENTED_SITES[@]}"}"; do
    _mika2542_resolver_calls "$DISPATCH_LIB" | grep -qF -- "$_mika2542_exc" \
        || _mika2542_stale+="$_mika2542_exc;"
done
for _mika2542_exc in "${MIKA2542_CEILING_LABEL_EXCEPTIONS[@]+"${MIKA2542_CEILING_LABEL_EXCEPTIONS[@]}"}"; do
    sed -n '/^PILOT_LABEL_TURN_CEILINGS=(/,/^)/p' "$DISPATCH_LIB" | grep -qF -- "\"$_mika2542_exc=" \
        || _mika2542_stale+="$_mika2542_exc;"
done
assert_eq "mika#2542: aucune entrée d'allowlist n'est périmée" "" "$_mika2542_stale"

# G1 (table ↔ labels.yml) vit dans scripts/check-pilot-turn-ceiling-labels.sh,
# branché en CI (`pilot-turn-ceiling-labels-lint`). On le rejoue ici pour que
# `make test-dispatch-lib` ne puisse pas être vert avec une table non déclarée.
assert_eq "mika#2542 (AC7): G1 — chaque clé de la table est déclarée dans labels.yml" "0" \
    "$(bash "$(dirname "$DISPATCH_LIB")/../../../scripts/check-pilot-turn-ceiling-labels.sh" >/dev/null 2>&1; echo $?)"

# --- mika#2296: un `.content` vide et un `session_id` absent ne se lisent plus pareil ---
#
# Avant ce ticket, les deux échouaient sur un seul message nommant un champ JSON
# manquant. La cause mesurée du premier est un budget de sortie épuisé par le
# raisonnement du modèle : le fournisseur répond 200, l'enveloppe est bien
# formée, et son `content` est authentiquement vide. Lire « champ manquant » là
# où il y a « réponse vide » a coûté trois essais.

_mika2296_empty_content_reason() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        _groom_warn_empty_content "first-pass" >/dev/null 2>&1
        printf '%s' "$GROOM_LOOP_FAILURE_REASON"
    )
}
MIKA2296_EMPTY=$(_mika2296_empty_content_reason) || MIKA2296_EMPTY=""

assert_contains "mika#2296: le message du content vide dit que la réponse est VIDE" \
    "EMPTY .content" "$MIKA2296_EMPTY"
assert_contains "mika#2296: il nomme la cause probable (budget de raisonnement)" \
    "output budget" "$MIKA2296_EMPTY"
assert_contains "mika#2296: il nomme le grep serveur qui confirme (AC3/AC4)" \
    "llm_reasoning_budget_exhausted" "$MIKA2296_EMPTY"
assert_contains "mika#2296: il nomme la remédiation" \
    "llm_max_tokens" "$MIKA2296_EMPTY"
# Le point du ticket : il ne se lit plus comme une enveloppe incomplète.
assert_not_contains "mika#2296: il n'accuse plus un champ JSON manquant" \
    "missing .content" "$MIKA2296_EMPTY"

# La garde de première passe teste désormais les deux conditions séparément —
# sans quoi les deux messages existeraient sans que personne ne puisse les voir.
MIKA2296_GUARD=$(awk '/first-pass response missing \.metadata\.session_id/{found=1} END{print found+0}' "$DISPATCH_LIB")
assert_eq "mika#2296: la garde session_id a son propre message" \
    "1" "$MIKA2296_GUARD"
assert_contains "mika#2296: le message session_id se démarque du cas content vide" \
    "NOT the mika#2296 empty-content case" "$(grep -A 2 'first-pass response missing \.metadata\.session_id' "$DISPATCH_LIB")"

# Les trois passes architecte qui lisent un `.content` passent par le même
# message : n'en corriger qu'une laisserait la même énigme aux deux autres.
MIKA2296_CALLSITES=$(grep -c '_groom_warn_empty_content "' "$DISPATCH_LIB" || true)
assert_eq "mika#2296: les trois sites de lecture de .content partagent le message" \
    "3" "$MIKA2296_CALLSITES"

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
