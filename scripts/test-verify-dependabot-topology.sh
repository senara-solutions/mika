#!/usr/bin/env bash
#
# Harnais anti-vacuité pour scripts/verify-dependabot-topology.sh (mika#1997).
#
# « Supprime ce que le test protège ; vérifie qu'il rougit. »
#
# L'ASSERTION POUR LAQUELLE CE FICHIER EXISTE est le **contrôle négatif** du
# plan de mika#1997 : « lancé avant la phase 2, le vérificateur rend 1 en nommant
# les deux dépôts manquants ; lancé après, il rend 0. Un vérificateur qui rend 0
# des deux côtés du correctif n'atteste rien. » Cette propriété ne peut pas être
# établie sur le vrai GitHub depuis un bac à sable de pilote — il n'a ni jeton
# ni egress arbitraire — et elle ne peut pas non plus être établie *une seule
# fois à la main*, puisqu'elle porte sur deux états du monde dont l'un cessera
# d'exister dès que la phase 2 aura livré les deux fichiers. Un faux `gh` rend
# les deux états rejouables pour toujours, et c'est l'exigence R5 du plan :
# la vérification est exécutable, pas une mémoire d'opérateur.
#
# `gh` est simulé, délibérément et complètement. Le contrat du vérificateur est
# avec des codes de sortie de `gh`, un message d'erreur et un corps de fichier —
# pas avec GitHub. Un faux qui parle ces trois choses est un substitut fidèle, et
# il laisse la batterie tourner sans jeton, sans réseau et sans quota.
#
# CE N'EST PAS CÂBLÉ AU CI, à dessein : mika#1997 NF4 refuse un job qui
# affirmerait l'état de fichiers d'autres dépôts. Ce harnais n'affirme rien de
# GitHub — il n'affirme que le vérificateur — donc rien n'interdirait de le
# câbler ; il ne l'est pas parce que le plan n'en a pas fait la demande et
# qu'ajouter un job est une décision qui se prend, pas un effet de bord.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERIFY="$REPO_ROOT/scripts/verify-dependabot-topology.sh"

PASS=0
FAIL=0

TMPROOT="$(mktemp -d)"
cleanup() { rm -rf "$TMPROOT"; }
trap cleanup EXIT

BIN="$TMPROOT/bin"
FIXTURES="$TMPROOT/fixtures"
mkdir -p "$BIN" "$FIXTURES"

# -- le faux `gh` -------------------------------------------------------------
#
# Deux appels seulement sont simulés, parce que le vérificateur n'en fait que
# deux : la lecture du fichier, et — uniquement sur 404 — la lecture du dépôt
# qui sépare « fichier absent » de « dépôt hors de portée du jeton ».
#
# Par dépôt `owner/repo`, le slug est `owner__repo` et le fixture le plus
# spécifique gagne :
#   <slug>.yml     → 200 avec ce corps
#   <slug>.404     → 1, stderr « gh: Not Found (HTTP 404) »
#   <slug>.err     → 1, stderr = contenu du fichier (panne non-404)
#   <slug>.norepo  → le probe `repos/<owner>/<repo>` échoue aussi (404 de portée)
#   (aucun)        → 1, stderr « gh: Not Found (HTTP 404) »
cat >"$BIN/gh" <<'FAKEGH'
#!/usr/bin/env bash
set -uo pipefail

[[ "${1:-}" == "api" ]] || { echo "fake gh: unexpected subcommand ${1:-}" >&2; exit 64; }
route="${2:-}"

# `repos/<owner>/<repo>` exactement — le probe de lisibilité du dépôt.
if [[ "$route" =~ ^repos/([^/]+)/([^/]+)$ ]]; then
    slug="${BASH_REMATCH[1]}__${BASH_REMATCH[2]}"
    if [[ -f "$FAKE_GH_FIXTURES/$slug.norepo" ]]; then
        echo "gh: Not Found (HTTP 404)" >&2
        exit 1
    fi
    echo "${BASH_REMATCH[1]}/${BASH_REMATCH[2]}"
    exit 0
fi

if [[ "$route" =~ ^repos/([^/]+)/([^/]+)/contents/.+$ ]]; then
    slug="${BASH_REMATCH[1]}__${BASH_REMATCH[2]}"
    if [[ -f "$FAKE_GH_FIXTURES/$slug.yml" ]]; then
        cat "$FAKE_GH_FIXTURES/$slug.yml"
        exit 0
    fi
    if [[ -f "$FAKE_GH_FIXTURES/$slug.err" ]]; then
        cat "$FAKE_GH_FIXTURES/$slug.err" >&2
        exit 1
    fi
    echo "gh: Not Found (HTTP 404)" >&2
    exit 1
fi

echo "fake gh: unexpected route $route" >&2
exit 64
FAKEGH
chmod +x "$BIN/gh"

# -- corps de fixtures --------------------------------------------------------

write_fixture() {
    # write_fixture <slug> <ecosystem:day> [<ecosystem:day> ...]
    local slug="$1"
    shift
    local path="$FIXTURES/$slug.yml"
    {
        echo "version: 2"
        echo "updates:"
        local spec eco day
        for spec in "$@"; do
            eco="${spec%%:*}"
            day="${spec##*:}"
            echo "  - package-ecosystem: \"$eco\""
            echo "    directory: \"/\""
            echo "    schedule:"
            echo "      interval: \"weekly\""
            echo "      day: \"$day\""
            echo "    open-pull-requests-limit: 5"
        done
    } >"$path"
}

reset_fixtures() {
    rm -rf "$FIXTURES"
    mkdir -p "$FIXTURES"
}

run_verify() {
    # Rend le code de sortie ; la sortie combinée va dans $LAST_OUTPUT.
    LAST_OUTPUT="$(PATH="$BIN:$PATH" FAKE_GH_FIXTURES="$FIXTURES" \
        bash "$VERIFY" "$@" 2>&1)"
    return $?
}

check() {
    # check <libellé> <code attendu> <code obtenu>
    local label="$1" expected="$2" actual="$3"
    if [[ "$expected" == "$actual" ]]; then
        echo "  PASS  $label (exit $actual)"
        PASS=$((PASS + 1))
    else
        echo "  FAIL  $label — attendu exit $expected, obtenu $actual"
        echo "------ sortie ------"
        echo "$LAST_OUTPUT"
        echo "--------------------"
        FAIL=$((FAIL + 1))
    fi
}

check_line() {
    # check_line <libellé> <sous-chaîne A> <sous-chaîne B>
    # Vraie quand UNE MÊME ligne porte les deux. Délibérément insensible à
    # l'espacement : assertionner sur la largeur d'une colonne ferait rougir ce
    # harnais au premier ajustement cosmétique du rapport, ce qui apprendrait à
    # le désarmer plutôt qu'à le lire.
    local label="$1" a="$2" b="$3"
    if printf '%s\n' "$LAST_OUTPUT" | grep -q -- "$a.*$b"; then
        echo "  PASS  $label"
        PASS=$((PASS + 1))
    else
        echo "  FAIL  $label — aucune ligne ne porte « $a » puis « $b »"
        echo "------ sortie ------"
        echo "$LAST_OUTPUT"
        echo "--------------------"
        FAIL=$((FAIL + 1))
    fi
}

check_output() {
    # check_output <libellé> <motif attendu dans la sortie>
    local label="$1" needle="$2"
    if [[ "$LAST_OUTPUT" == *"$needle"* ]]; then
        echo "  PASS  $label"
        PASS=$((PASS + 1))
    else
        echo "  FAIL  $label — « $needle » absent de la sortie"
        echo "------ sortie ------"
        echo "$LAST_OUTPUT"
        echo "--------------------"
        FAIL=$((FAIL + 1))
    fi
}

echo "test-verify-dependabot-topology — mika#1997"
echo

# -- 1. Le contrôle négatif, dans les deux sens -------------------------------
#
# C'est l'assertion centrale. Les deux moitiés sont écrites ensemble parce que
# ni l'une ni l'autre ne prouve rien seule : un script qui rendrait toujours 1
# passerait la première, un script qui rendrait toujours 0 passerait la seconde.

echo "1. contrôle négatif — l'état d'AVANT la phase 2 (seul mika porte le fichier)"
reset_fixtures
write_fixture "senara-solutions__mika" "cargo:monday" "github-actions:monday"
run_verify; rc=$?
check "les deux dépôts nus rendent 1" 1 "$rc"
check_line "mika-cloud est nommé absent" "senara-solutions/mika-cloud" "ABSENT"
check_line "mika-platform est nommé absent" "senara-solutions/mika-platform" "ABSENT"
check_line "mika reste rapporté présent" "senara-solutions/mika " "PRÉSENT"
echo

echo "2. contrôle négatif — l'état d'APRÈS la phase 2 (les trois portent le fichier)"
reset_fixtures
write_fixture "senara-solutions__mika" "cargo:monday" "github-actions:monday"
write_fixture "senara-solutions__mika-cloud" "cargo:tuesday" "github-actions:tuesday"
write_fixture "senara-solutions__mika-platform" "github-actions:wednesday"
run_verify; rc=$?
check "les trois présents rendent 0" 0 "$rc"
check_output "les écosystèmes sont rapportés" "écosystèmes : github-actions"
check_output "les jours sont rapportés" "jours       : wednesday"
check_output "la limite PR est rapportée" "limite PR   : 5"
echo

# -- 3. Le troisième état, qui n'est pas un succès ----------------------------

echo "3. rien n'a pu être établi"
reset_fixtures
write_fixture "senara-solutions__mika" "cargo:monday"
printf 'To get started with GitHub CLI, please run:  gh auth login\n' \
    >"$FIXTURES/senara-solutions__mika-cloud.err"
printf 'To get started with GitHub CLI, please run:  gh auth login\n' \
    >"$FIXTURES/senara-solutions__mika-platform.err"
run_verify; rc=$?
check "une panne non-404 rend 2, jamais 0" 2 "$rc"
check_output "le verdict dit que la question reste ouverte" "NI un succès NI une absence"
echo

echo "4. \`gh\` absent du PATH rend 2"
reset_fixtures
# Un PATH vide emporterait `bash` lui-même et le test rendrait 127 — une panne
# du harnais déguisée en verdict. D'où un répertoire réellement vide sur le
# PATH, et `bash` appelé par son chemin absolu résolu AVANT la substitution.
mkdir -p "$TMPROOT/emptybin"
BASH_ABS="$(command -v bash)"
LAST_OUTPUT="$(PATH="$TMPROOT/emptybin" FAKE_GH_FIXTURES="$FIXTURES" \
    "$BASH_ABS" "$VERIFY" 2>&1)"; rc=$?
check "gh introuvable rend 2" 2 "$rc"
check_output "l'absence de gh est nommée" "introuvable sur le PATH"
echo

# -- 5. Le piège du 404 de portée ---------------------------------------------
#
# Sans ce comportement, un jeton sans accès à mika-cloud ferait dire « le fichier
# manque » d'un fichier peut-être présent — la fausse mesure que le vérificateur
# existe pour empêcher.

echo "5. un 404 de portée n'est pas une absence"
reset_fixtures
write_fixture "senara-solutions__mika" "cargo:monday"
write_fixture "senara-solutions__mika-platform" "github-actions:wednesday"
touch "$FIXTURES/senara-solutions__mika-cloud.norepo"
run_verify; rc=$?
check "dépôt hors de portée → 2, pas 1" 2 "$rc"
check_output "le 404 double est nommé" "404 sur le fichier ET sur le dépôt"
echo

# -- 6. L'absence établie prime sur l'indétermination -------------------------

echo "6. absence établie + indétermination → 1"
reset_fixtures
write_fixture "senara-solutions__mika" "cargo:monday"
printf 'gh: connection refused\n' >"$FIXTURES/senara-solutions__mika-platform.err"
# mika-cloud n'a aucun fixture → 404 sur le fichier, dépôt lisible → ABSENT.
run_verify; rc=$?
check "un fait établi l'emporte sur une incertitude" 1 "$rc"
check_output "l'indétermination résiduelle est dite" "par ailleurs indéterminé"
echo

# -- 7. Un fichier présent mais muet compte comme manquant --------------------

echo "7. fichier présent, aucun écosystème → inerte, compté avec les manquants"
reset_fixtures
write_fixture "senara-solutions__mika" "cargo:monday"
write_fixture "senara-solutions__mika-cloud" "cargo:tuesday"
printf 'version: 2\nupdates: []\n' >"$FIXTURES/senara-solutions__mika-platform.yml"
run_verify; rc=$?
check "un fichier inerte rend 1" 1 "$rc"
check_output "l'inertie est nommée à part de l'absence" "INERTE — fichier présent"
echo

# -- 8. L'observation d'étalement voit la collision et ne gate pas ------------

echo "8. jours partagés — observation visible, verdict inchangé"
reset_fixtures
write_fixture "senara-solutions__mika" "cargo:monday"
write_fixture "senara-solutions__mika-cloud" "cargo:monday"
write_fixture "senara-solutions__mika-platform" "github-actions:monday"
run_verify; rc=$?
check "une collision de jours ne change pas le code de sortie" 0 "$rc"
check_output "la collision est rapportée" "jours partagés"
check_output "elle est dite non gatante" "Ce n'est pas un échec de vérification"

reset_fixtures
write_fixture "senara-solutions__mika" "cargo:monday"
write_fixture "senara-solutions__mika-cloud" "cargo:tuesday"
write_fixture "senara-solutions__mika-platform" "github-actions:wednesday"
run_verify; rc=$?
if [[ "$LAST_OUTPUT" == *"jours partagés"* ]]; then
    echo "  FAIL  des jours distincts ne doivent produire aucune observation"
    FAIL=$((FAIL + 1))
else
    echo "  PASS  des jours distincts ne produisent aucune observation"
    PASS=$((PASS + 1))
fi
echo

# -- 9. La liste de dépôts est paramétrable -----------------------------------

echo "9. une liste explicite de dépôts est honorée"
reset_fixtures
write_fixture "senara-solutions__mika" "cargo:monday"
run_verify "senara-solutions/mika"; rc=$?
check "un seul dépôt présent rend 0" 0 "$rc"
check_output "seul le dépôt demandé est interrogé" "sur 1 dépôt(s)"
echo

# -- bilan --------------------------------------------------------------------

echo "────────────────────────────────────────"
echo "PASS: $PASS   FAIL: $FAIL"
if [[ $FAIL -gt 0 ]]; then
    exit 1
fi
exit 0
