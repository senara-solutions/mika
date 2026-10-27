#!/usr/bin/env bash
#
# Harnais du handler qa_head_reviews (mika#2939).
#
# « Supprime ce que le test protège ; vérifie qu'il rougit. »
#
# L'ASSERTION POUR LAQUELLE CE FICHIER EXISTE est AC2 de mika#2939 : le cas de
# #2920 est rejoué, et le contrôle positif vit dans le même test. Une revue
# APPROVED dont le `commit_id` vaut la tête, soumise AVANT que la forge ait vu
# cette tête, ne doit rendre AUCUNE preuve ; une revue soumise après le push de
# la tête doit être rendue. Les deux moitiés ensemble, parce que ni l'une ni
# l'autre ne prouve rien seule : un handler qui rendrait toujours une liste vide
# passerait la première, un handler sans filtre passerait la seconde.
#
# `gh` est simulé, délibérément et complètement. La propriété porte sur DEUX
# états du monde — avant et après un force-push — dont l'un a déjà cessé
# d'exister sur #2920 : la revue 5663460195 porte aujourd'hui le `commit_id` de
# la nouvelle tête, et plus rien sur la forge ne rend l'ancien. Elle n'est donc
# pas établissable sur le vrai GitHub, ni depuis un bac à sable de pilote (ni
# jeton ni egress arbitraire), ni une seule fois à la main. Un faux `gh` rend
# les deux états rejouables pour toujours.
#
# Le contrat du handler est avec deux routes d'API, un code de sortie et un
# objet JSON — pas avec GitHub. Un faux qui parle ces trois choses est un
# substitut fidèle, et il laisse la batterie tourner sans jeton, sans réseau et
# sans quota.
#
# Le faux `gh` vérifie aussi que la route porte le dépôt, la PR et la SHA
# attendus : une régression d'interpolation ne se verrait sur aucune assertion
# de contenu, puisque le faux répondrait quand même.
#
# Câblé au CI dans le job `offline-script-suites` de `ci.yml` : la propriété est
# hors-ligne par construction, donc un job qui ne la rejoue pas est une garde
# que la pipeline n'exécute jamais (doctrine mika#2578 / mika#2752).

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HANDLER="$REPO_ROOT/skills/bundled/qa-review-webhook-success/handlers/qa_head_reviews.sh"

PASS=0
FAIL=0

TMPROOT="$(mktemp -d)"
cleanup() { rm -rf "$TMPROOT"; }
trap cleanup EXIT

BIN="$TMPROOT/bin"
FIXTURES="$TMPROOT/fixtures"
mkdir -p "$BIN" "$FIXTURES"

# -- les constantes du cas de #2920 -------------------------------------------
#
# Mesurées le 2026-10-27, et c'est ce qui en fait un rejeu plutôt qu'une
# illustration. La revue 5663460195 est soumise cinq heures et demie AVANT la
# création du commit que la forge lui attribue aujourd'hui.

CASE_REPO="senara-solutions/mika"
CASE_PR="2920"
CASE_HEAD="5e4918cf992d576e1098015c12fb50d9d94de6cc"
AUTRE_HEAD="f7542d43e2c1b09a8d6f4e35c7a1b0d9e8f26341"
FIRST_SEEN="2026-10-27T12:44:36Z"
AVANT_LE_PUSH="2026-10-27T07:16:49Z"
APRES_LE_PUSH="2026-10-27T12:55:57Z"

# -- le faux `gh` -------------------------------------------------------------
#
# Deux routes seulement sont simulées, parce que le handler n'en appelle que
# deux : les check_suites de la SHA (le repère temporel) et les revues de la PR.
#
# Il applique lui-même le `--jq` que le handler lui passe, via le vrai `jq` :
# `gh api --jq` rend du brut comme `jq -r`, et c'est cette sortie-là que le
# handler lit.
#
# Fixtures, par route :
#   check-suites.json  → 200 avec ce corps
#   check-suites.err   → 1, stderr = contenu du fichier
#   reviews.json       → 200 avec ce corps
#   reviews.err        → 1, stderr = contenu du fichier
#   (aucun)            → 1, stderr « fake gh: no fixture for <route> »
cat >"$BIN/gh" <<'FAKEGH'
#!/usr/bin/env bash
set -uo pipefail

[[ "${1:-}" == "api" ]] || { echo "fake gh: unexpected subcommand ${1:-}" >&2; exit 64; }
shift

route=""
filter=""
while (($#)); do
    case "$1" in
        --jq) filter="${2:-}"; shift 2 ;;
        --paginate) shift ;;
        -*) echo "fake gh: unexpected flag $1" >&2; exit 64 ;;
        *) route="$1"; shift ;;
    esac
done

# Le faux refuse une route mal interpolée au lieu de répondre quand même : une
# régression sur `${REPO}`, `${PR}` ou `${HEAD_SHA}` ne se verrait sur aucune
# assertion de contenu.
emit() {
    local fixture="$1"
    if [[ -f "$FAKE_GH_FIXTURES/$fixture.err" ]]; then
        cat "$FAKE_GH_FIXTURES/$fixture.err" >&2
        exit 1
    fi
    if [[ ! -f "$FAKE_GH_FIXTURES/$fixture.json" ]]; then
        echo "fake gh: no fixture for $route" >&2
        exit 1
    fi
    if [[ -n "$filter" ]]; then
        jq -r "$filter" <"$FAKE_GH_FIXTURES/$fixture.json"
    else
        cat "$FAKE_GH_FIXTURES/$fixture.json"
    fi
    exit 0
}

if [[ "$route" =~ ^repos/([^/]+/[^/]+)/commits/([0-9a-f]{40})/check-suites ]]; then
    [[ "${BASH_REMATCH[1]}" == "$EXPECT_REPO" ]] \
        || { echo "fake gh: check-suites on repo ${BASH_REMATCH[1]}, expected $EXPECT_REPO" >&2; exit 65; }
    [[ "${BASH_REMATCH[2]}" == "$EXPECT_SHA" ]] \
        || { echo "fake gh: check-suites on sha ${BASH_REMATCH[2]}, expected $EXPECT_SHA" >&2; exit 65; }
    emit check-suites
fi

if [[ "$route" =~ ^repos/([^/]+/[^/]+)/pulls/([0-9]+)/reviews$ ]]; then
    [[ "${BASH_REMATCH[1]}" == "$EXPECT_REPO" ]] \
        || { echo "fake gh: reviews on repo ${BASH_REMATCH[1]}, expected $EXPECT_REPO" >&2; exit 65; }
    [[ "${BASH_REMATCH[2]}" == "$EXPECT_PR" ]] \
        || { echo "fake gh: reviews on pr ${BASH_REMATCH[2]}, expected $EXPECT_PR" >&2; exit 65; }
    emit reviews
fi

echo "fake gh: unexpected route $route" >&2
exit 64
FAKEGH
chmod +x "$BIN/gh"

# -- corps de fixtures --------------------------------------------------------

reset_fixtures() {
    rm -rf "$FIXTURES"
    mkdir -p "$FIXTURES"
}

write_suites() {
    # write_suites <created_at> [<created_at> ...]
    # Le PREMIER élément du tableau n'est délibérément pas le plus ancien chez
    # les appelants : le repère est un MIN, pas un `[0]`.
    local out="$FIXTURES/check-suites.json" first=1 ts
    {
        printf '{"total_count": %d, "check_suites": [' "$#"
        for ts in "$@"; do
            ((first)) || printf ', '
            first=0
            printf '{"id": %d, "created_at": "%s", "status": "completed"}' "$RANDOM" "$ts"
        done
        printf ']}\n'
    } >"$out"
}

write_reviews() {
    # write_reviews <id|login|state|commit_id|submitted_at|verdict> ...
    # <verdict> vaut `pass`, `block` ou `none`.
    #
    # Le séparateur est `|` et non `:` : un horodatage ISO 8601 en porte deux,
    # donc un `IFS=':'` tronque `submitted_at` à son heure et le filtre écarte
    # alors TOUT — y compris le contrôle positif, qui rougit sans que le handler
    # soit en cause.
    local out="$FIXTURES/reviews.json" first=1 spec
    local id login state commit_id submitted verdict body
    {
        printf '['
        for spec in "$@"; do
            IFS='|' read -r id login state commit_id submitted verdict <<<"$spec"
            case "$verdict" in
                none) body="Quelques remarques, rien de bloquant." ;;
                *) body="VERDICT: $verdict\n\nLe détail du verdict." ;;
            esac
            ((first)) || printf ', '
            first=0
            printf '{"id": %s, "user": {"login": "%s"}, "state": "%s", "commit_id": "%s", "submitted_at": "%s", "body": "%s"}' \
                "$id" "$login" "$state" "$commit_id" "$submitted" "$body"
        done
        printf ']\n'
    } >"$out"
}

# -- exécution du handler -----------------------------------------------------

run_handler() {
    # run_handler <repo> <pr> <head_sha> [<chemin du handler>]
    # Rend le code de sortie ; stdout dans $LAST_STDOUT, stderr dans $LAST_STDERR.
    local repo="$1" pr="$2" sha="$3" bin="${4:-$HANDLER}" rc
    local payload
    payload="$(jq -nc --arg repo "$repo" --arg sha "$sha" --argjson pr_raw "\"$pr\"" \
        '{repo: $repo, pr: ($pr_raw | tonumber? // $pr_raw), head_sha: $sha}')"
    LAST_STDOUT="$(printf '%s' "$payload" \
        | PATH="$BIN:$PATH" FAKE_GH_FIXTURES="$FIXTURES" \
          EXPECT_REPO="$CASE_REPO" EXPECT_PR="$CASE_PR" EXPECT_SHA="$CASE_HEAD" \
          bash "$bin" 2>"$TMPROOT/stderr")"
    rc=$?
    LAST_STDERR="$(cat "$TMPROOT/stderr")"
    return $rc
}

# -- assertions ---------------------------------------------------------------

check() {
    # check <libellé> <code attendu> <code obtenu>
    local label="$1" expected="$2" actual="$3"
    if [[ "$expected" == "$actual" ]]; then
        echo "  PASS  $label (exit $actual)"
        PASS=$((PASS + 1))
    else
        echo "  FAIL  $label — attendu exit $expected, obtenu $actual"
        echo "------ stdout ------"; echo "$LAST_STDOUT"
        echo "------ stderr ------"; echo "$LAST_STDERR"
        echo "--------------------"
        FAIL=$((FAIL + 1))
    fi
}

check_json() {
    # check_json <libellé> <filtre jq sur stdout> <valeur attendue>
    local label="$1" filter="$2" expected="$3" actual
    actual="$(jq -r "$filter" <<<"$LAST_STDOUT" 2>/dev/null)" \
        || actual="<stdout illisible comme JSON>"
    if [[ "$actual" == "$expected" ]]; then
        echo "  PASS  $label"
        PASS=$((PASS + 1))
    else
        echo "  FAIL  $label — « $filter » rend « $actual », attendu « $expected »"
        echo "------ stdout ------"; echo "$LAST_STDOUT"
        echo "--------------------"
        FAIL=$((FAIL + 1))
    fi
}

check_stderr() {
    # check_stderr <libellé> <motif attendu>
    local label="$1" needle="$2"
    if [[ "$LAST_STDERR" == *"$needle"* ]]; then
        echo "  PASS  $label"
        PASS=$((PASS + 1))
    else
        echo "  FAIL  $label — « $needle » absent de stderr"
        echo "------ stderr ------"; echo "$LAST_STDERR"
        echo "--------------------"
        FAIL=$((FAIL + 1))
    fi
}

check_stdout_vide() {
    # Une sortie partielle serait lue comme une liste de revues : c'est le
    # mode de panne que l'ordre des deux appels gh existe pour interdire.
    local label="$1"
    if [[ -z "$LAST_STDOUT" ]]; then
        echo "  PASS  $label"
        PASS=$((PASS + 1))
    else
        echo "  FAIL  $label — stdout n'est pas vide"
        echo "------ stdout ------"; echo "$LAST_STDOUT"
        echo "--------------------"
        FAIL=$((FAIL + 1))
    fi
}

echo "test-qa-head-reviews-handler — mika#2939"
echo

# -- 1. AC2 : le cas de #2920, et son contrôle positif ------------------------
#
# Les deux moitiés sont écrites ensemble parce que ni l'une ni l'autre ne prouve
# rien seule.

echo "1. AC2 — le cas de #2920 : une revue antidatée ne rend aucune preuve"
reset_fixtures
write_suites "2026-10-27T12:45:02Z" "$FIRST_SEEN"
write_reviews "5663460195|mika-platform-qa|APPROVED|$CASE_HEAD|$AVANT_LE_PUSH|pass"
run_handler "$CASE_REPO" "$CASE_PR" "$CASE_HEAD"; rc=$?
check "le handler réussit" 0 "$rc"
check_json "aucune revue n'est rendue" '.reviews | length' "0"
check_json "la revue écartée est comptée" '.dropped_predating_head' "1"
# Le fixture met délibérément le suite le PLUS RÉCENT en tête du tableau : la
# même assertion vaut alors « le repère est rendu » et « c'est un MIN, pas un
# `[0]` », et un handler qui lirait le premier élément rougirait ici.
check_json "le repère temporel est le MIN des suites, pas leur premier élément" \
    '.head_first_seen_at' "$FIRST_SEEN"
check_json "la tête est renvoyée en écho" '.head_sha' "$CASE_HEAD"
echo

echo "2. AC2 — contrôle positif : une revue postérieure au push est rendue"
reset_fixtures
write_suites "2026-10-27T12:45:02Z" "$FIRST_SEEN"
write_reviews "5667360863|mika-platform-qa|APPROVED|$CASE_HEAD|$APRES_LE_PUSH|pass"
run_handler "$CASE_REPO" "$CASE_PR" "$CASE_HEAD"; rc=$?
check "le handler réussit" 0 "$rc"
check_json "la revue est rendue" '.reviews | length' "1"
check_json "son auteur est rendu" '.reviews[0].author' "mika-platform-qa"
check_json "son état est rendu" '.reviews[0].state' "APPROVED"
check_json "sa soumission est rendue" '.reviews[0].submitted_at' "$APRES_LE_PUSH"
check_json "son verdict est relevé" '.reviews[0].carries_verdict_line' "true"
check_json "rien n'est écarté" '.dropped_predating_head' "0"
echo

echo "3. le filtre PARTITIONNE — l'état réel de #2920, les deux revues ensemble"
reset_fixtures
write_suites "$FIRST_SEEN"
write_reviews \
    "5663460195|mika-platform-qa|APPROVED|$CASE_HEAD|$AVANT_LE_PUSH|pass" \
    "5667360863|mika-platform-qa|APPROVED|$CASE_HEAD|$APRES_LE_PUSH|pass"
run_handler "$CASE_REPO" "$CASE_PR" "$CASE_HEAD"; rc=$?
check "le handler réussit" 0 "$rc"
check_json "seule la postérieure est rendue" '.reviews | map(.submitted_at) | join(",")' "$APRES_LE_PUSH"
check_json "l'antérieure est comptée, pas perdue" '.dropped_predating_head' "1"
echo

# -- 4. La borne est inclusive, et c'est une décision ------------------------

echo "4. borne inclusive — une revue de la seconde même du premier suite est rendue"
reset_fixtures
write_suites "$FIRST_SEEN"
write_reviews "5667000000|mika-platform-qa|APPROVED|$CASE_HEAD|$FIRST_SEEN|pass"
run_handler "$CASE_REPO" "$CASE_PR" "$CASE_HEAD"; rc=$?
check "le handler réussit" 0 "$rc"
check_json "le prédicat est >=, pas >" '.reviews | length' "1"
check_json "rien n'est écarté à la borne" '.dropped_predating_head' "0"
echo

# -- 5. Une autre tête n'entre dans aucune des deux moitiés -------------------

echo "5. une revue d'une autre tête n'est ni rendue ni comptée comme écartée"
reset_fixtures
write_suites "$FIRST_SEEN"
write_reviews "5660000000|mika-platform-qa|APPROVED|$AUTRE_HEAD|$APRES_LE_PUSH|pass"
run_handler "$CASE_REPO" "$CASE_PR" "$CASE_HEAD"; rc=$?
check "le handler réussit" 0 "$rc"
check_json "elle n'est pas rendue" '.reviews | length' "0"
check_json "elle n'est pas comptée écartée — l'instrument mesure le force-push" \
    '.dropped_predating_head' "0"
echo

# -- 6. Un repère indéterminable est une ERREUR, jamais une liste vide --------
#
# Classe mika#2205 : une liste vide est la réponse « aucune revue n'a jugé cette
# tête ». Les deux ne peuvent pas partager une sortie.

echo "6. aucun check_suite — le repère est indéterminable, donc c'est une erreur"
reset_fixtures
write_suites
write_reviews "5667360863|mika-platform-qa|APPROVED|$CASE_HEAD|$APRES_LE_PUSH|pass"
run_handler "$CASE_REPO" "$CASE_PR" "$CASE_HEAD"; rc=$?
check "le handler échoue" 1 "$rc"
check_stderr "l'erreur nomme le repère" "head_first_seen_at is undeterminable"
check_stderr "l'erreur refuse la confusion avec une liste vide" "NOT an empty review list"
check_stdout_vide "rien n'est écrit sur stdout"
echo

echo "7. gh en panne sur les check_suites — aucune sortie partielle"
reset_fixtures
printf 'gh: Server Error (HTTP 500)\n' >"$FIXTURES/check-suites.err"
write_reviews "5667360863|mika-platform-qa|APPROVED|$CASE_HEAD|$APRES_LE_PUSH|pass"
run_handler "$CASE_REPO" "$CASE_PR" "$CASE_HEAD"; rc=$?
check "le handler échoue" 1 "$rc"
check_stderr "l'erreur nomme la lecture qui a échoué" "could not read the check suites"
check_stdout_vide "rien n'est écrit sur stdout"
echo

# -- 8. La validation d'entrée est intacte -----------------------------------
#
# Les trois champs construisent le chemin d'API : un `../` ou un `--flag` qui
# passerait serait de la traversée de chemin ou de l'injection d'option.

echo "8. entrées invalides — les trois messages, inchangés"
reset_fixtures
write_suites "$FIRST_SEEN"
write_reviews "5667360863|mika-platform-qa|APPROVED|$CASE_HEAD|$APRES_LE_PUSH|pass"

run_handler "senara-solutions/../etc/passwd" "$CASE_PR" "$CASE_HEAD"; rc=$?
check "un repo avec ../ est refusé" 1 "$rc"
check_stderr "le message du repo est intact" "repo is required and must be owner/name"
check_stdout_vide "rien sur stdout pour un repo invalide"

run_handler "$CASE_REPO" "deux-mille-neuf-cent-vingt" "$CASE_HEAD"; rc=$?
check "une PR non numérique est refusée" 1 "$rc"
check_stderr "le message de la PR est intact" "pr is required and must be a PR number"
check_stdout_vide "rien sur stdout pour une PR invalide"

run_handler "$CASE_REPO" "$CASE_PR" "5e4918cf"; rc=$?
check "une SHA tronquée est refusée" 1 "$rc"
check_stderr "le message de la SHA est intact" \
    "head_sha is required and must be a 40-character lowercase hex SHA"
check_stdout_vide "rien sur stdout pour une SHA invalide"
echo

# -- 9. Le contrôle de rougeur --------------------------------------------------
#
# « Supprime ce que le test protège ; vérifie qu'il rougit. » Sans cette case,
# le harnais ci-dessus serait indiscernable d'un test sur la FORME de l'objet :
# un handler qui rend les quatre champs et ne filtre rien passerait les cas 2,
# 4, 5, 6, 7 et 8, et ne rougirait qu'au cas 1 — ce que personne ne saurait
# avant d'avoir cassé le filtre.
#
# La copie désarmée est écrite ici, depuis l'arbre, et n'est jamais commitée :
# un handler cassé qui vit dans le dépôt est un handler que quelqu'un déploiera.

echo "9. contrôle de rougeur — le handler privé de son terme temporel rejoue le défaut"
DESARME="$TMPROOT/qa_head_reviews_desarme.sh"
sed 's/map(select((\.submitted_at \/\/ "") >= \$first_seen))/map(select(true))/' \
    "$HANDLER" >"$DESARME"
chmod +x "$DESARME"

if cmp -s "$HANDLER" "$DESARME"; then
    echo "  FAIL  anti-vacuité : le désarmement n'a pas trouvé sa cible, donc ce"
    echo "        contrôle ne prouve rien — le terme temporel du handler a changé de forme"
    FAIL=$((FAIL + 1))
else
    echo "  PASS  le désarmement a bien retiré le terme >= \$first_seen"
    PASS=$((PASS + 1))

    reset_fixtures
    write_suites "$FIRST_SEEN"
    write_reviews "5663460195|mika-platform-qa|APPROVED|$CASE_HEAD|$AVANT_LE_PUSH|pass"
    run_handler "$CASE_REPO" "$CASE_PR" "$CASE_HEAD" "$DESARME"; rc=$?
    check "le handler désarmé réussit quand même" 0 "$rc"

    rendues="$(jq -r '.reviews | length' <<<"$LAST_STDOUT" 2>/dev/null || echo "?")"
    if [[ "$rendues" == "1" ]]; then
        echo "  PASS  désarmé, il rend la revue antidatée — le cas 1 teste bien le filtre"
        PASS=$((PASS + 1))
    else
        echo "  FAIL  désarmé, il rend $rendues revue(s) au lieu de 1 : le cas 1 ne"
        echo "        dépend donc pas du filtre, il mesure autre chose"
        echo "------ stdout ------"; echo "$LAST_STDOUT"
        echo "--------------------"
        FAIL=$((FAIL + 1))
    fi

    # Et la forme de l'objet, elle, survit au désarmement : c'est ce qui prouve
    # qu'un test sur la seule forme n'aurait rien vu.
    check_json "désarmé, les quatre champs sont toujours là" \
        '[.head_sha, .head_first_seen_at, (.reviews|type), (.dropped_predating_head|type)] | length' "4"
fi
echo

# -- bilan --------------------------------------------------------------------

echo "────────────────────────────────────────"
echo "PASS: $PASS   FAIL: $FAIL"
if [[ $FAIL -gt 0 ]]; then
    exit 1
fi
exit 0
