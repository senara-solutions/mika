#!/usr/bin/env bash
#
# Harnais de scripts/verify-npm-publish.sh (mika#2566).
#
# CE HARNAIS EST LE LIVRABLE PRINCIPAL DU TICKET, pas un accessoire du script.
# La boucle 5×15 s qu'on remplace était enfouie dans un `run:` de YAML, donc
# exercée par rien : le correctif de mika#1917 n'a pas pu être vérifié avant
# d'atteindre la production, et le seul moment où on a découvert qu'il ne
# suffisait pas fut le bump suivant, sur `main`, en rouge. C'est cette propriété
# structurelle que le ticket change — pas le nombre d'essais.
#
# RÈGLE, ET ELLE EST DURE : quand une assertion tire, on répare
# `verify-npm-publish.sh`. On n'ajoute PAS de ligne à la table ci-dessous
# (doctrine mika#2201). Un cas qu'on ne veut pas couvrir est un cas à retirer,
# pas à exempter.
#
# Aucun appel réseau, aucun `npm` réel, aucun sommeil réel hors de N4a : la
# suite tourne en moins d'une seconde.
#
# CONTRÔLES NÉGATIFS JOUÉS, ET CE QUI A ROUGI (2026-09-29) — chacun est une
# mutation du script gardé, passée en argument (voir plus bas) :
#
#   mutation                                  | rouge                | vert
#   ------------------------------------------+----------------------+------------
#   le script entier remplacé par `exit 1`    | 23 assertions, dt N7 | N3 (!)
#   sommeil rendu inconditionnel              | N4a, N4b, N4c, SEULS | le reste
#   `--prefer-online` retiré                  | N5, SEUL             | le reste
#   `npm_sees` décidé par recherche de sous-  | N6c, SEUL            | le reste
#     chaîne au lieu d'égalité stricte        |                      |
#
# La première ligne est la raison d'être de N7 : `exit 1` satisfait N3 — « une
# version qui n'apparaît jamais fait échouer la vérification » — sans jamais
# interroger le registre. Sans N7, AC2 serait verte pour la mauvaise raison, et
# un détecteur silencieusement inerte se lit exactement comme un détecteur sain
# (classe mika#2205).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Le script gardé. L'argument optionnel existe pour une seule raison — rejouer
# le contrôle négatif, c'est-à-dire pointer la suite vers une version mutilée et
# la regarder rougir. Un harnais qu'on n'a jamais vu rougir est une décoration.
# La CI ne passe aucun argument, et un script trivialement vert ne passerait de
# toute façon ni N5 ni N7.
#
#   bash scripts/test-verify-npm-publish.sh .pilot-scratch/neg/always-fails.sh
SCRIPT="$REPO_ROOT/scripts/verify-npm-publish.sh"
if [ "$#" -ge 1 ]; then
    SCRIPT="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
    printf 'contrôle négatif : script sous test = %s\n\n' "$SCRIPT"
fi

PKG="@scope/pkg"
VERSION="1.2.3"

PASS=0
FAIL=0

# Forme obligatoire d'une entrée (aucune n'existe aujourd'hui) :
#   "<assertion> | <donnée exacte> | <ticket de suivi> | <condition de péremption>"
# La table est livrée VIDE et deux assertions auto-nettoyantes la ferment :
# elle rougit le jour où une entrée apparaît sans ticket de suivi ni condition
# de péremption, et elle rougit si le script gardé se met à consulter une table
# du même genre au runtime — ce qui serait une échappatoire dans la garde
# elle-même, sur le chemin exact où AC2 doit tenir sans condition.
VERIFY_EXCEPTION_ALLOWLIST=()

ok() {
    printf 'PASS: %s\n' "$1"
    PASS=$((PASS + 1))
}

ko() {
    printf 'FAIL: %s\n' "$1"
    FAIL=$((FAIL + 1))
}

assert_eq() {
    local got="$1" want="$2" name="$3"
    if [ "$got" = "$want" ]; then
        ok "$name"
    else
        ko "$name (attendu \"$want\", obtenu \"$got\")"
    fi
}

assert_contains() {
    local haystack="$1" needle="$2" name="$3"
    if grep -qF -- "$needle" <<<"$haystack"; then
        ok "$name"
    else
        ko "$name (la sortie ne contient pas : $needle)"
    fi
}

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT

BIN="$ROOT/bin"
mkdir -p "$BIN"

# ---------------------------------------------------------------------------
# Les doublures. `npm`, `curl` et `sleep` sont appelés sans chemin absolu par le
# script gardé ; un `$BIN` en tête de PATH suffit donc à les intercepter.
#
# Le `npm` factice journalise son argv (une ligne par appel) et rend la version
# au-delà d'un seuil d'appels lu dans un fichier compteur — c'est ce qui permet
# de rejouer « visible au 6ᵉ essai » de façon déterministe.
# ---------------------------------------------------------------------------
cat >"$BIN/npm" <<'STUB'
#!/usr/bin/env bash
set -u
printf '%s\n' "$*" >>"$NPM_STUB_LOG"
n=1
if [ -s "$NPM_STUB_COUNTER" ]; then
    n=$(( $(cat "$NPM_STUB_COUNTER") + 1 ))
fi
printf '%s' "$n" >"$NPM_STUB_COUNTER"

if [ -n "${NPM_STUB_ERROR_CODE:-}" ]; then
    printf 'npm error code %s\n' "$NPM_STUB_ERROR_CODE" >&2
    printf 'npm error network request to https://registry.npmjs.org failed\n' >&2
    exit 1
fi

if [ "$n" -ge "${NPM_STUB_VISIBLE_FROM:-1}" ]; then
    printf '%s\n' "${NPM_STUB_VERSION:?}"
    exit 0
fi
# Fidélité au vrai npm, et elle est PORTEUSE : sous `--json`, npm ≥ 7 écrit son
# objet d'erreur sur STDOUT en y citant le spec demandé — donc la version y
# figure alors même que npm ne la voit pas. Un script qui chercherait la version
# par sous-chaîne dans cette sortie répondrait « npm la voit » sur l'échec même
# qu'il diagnostique, et inverserait sa propre table H1/H2.
case "$*" in
    *--json*)
        printf '{"error":{"code":"E404","summary":"404 %s is not in this registry"}}\n' "$*"
        ;;
    *)
        printf 'npm error code E404\n' >&2
        printf "npm error 404 '%s' is not in this registry.\n" "$*" >&2
        ;;
esac
exit 1
STUB

cat >"$BIN/curl" <<'STUB'
#!/usr/bin/env bash
set -u
printf '%s\n' "$*" >>"$CURL_STUB_LOG"
case "${CURL_STUB_MODE:-packument-sans-version}" in
    injoignable)
        printf 'curl: (7) Failed to connect\n' >&2
        exit 7
        ;;
    packument-avec-version)
        printf '{"name":"stub","versions":{"%s":{}}}' "${NPM_STUB_VERSION:?}"
        ;;
    *)
        printf '{"name":"stub","versions":{"0.0.1":{}}}'
        ;;
esac
STUB

# Le `sleep` factice ne dort pas : il journalise la durée demandée. C'est ce qui
# rend N4b déterministe là où une mesure de durée (N4a) ne peut être qu'un
# encadrement.
cat >"$BIN/sleep" <<'STUB'
#!/usr/bin/env bash
set -u
printf '%s\n' "$1" >>"$SLEEP_STUB_LOG"
STUB

chmod +x "$BIN/npm" "$BIN/curl" "$BIN/sleep"

# ---------------------------------------------------------------------------
# Un scénario = un répertoire de journaux neuf. `run_case` N'EST JAMAIS appelé
# dans une substitution de commande : elle s'exécuterait en sous-shell et les
# artefacts du scénario courant ne remonteraient pas. Tout est donc écrit sur
# disque, et les lecteurs ci-dessous relisent `$CASE_DIR`.
# ---------------------------------------------------------------------------
CASE_DIR=""

run_case() {
    local name="$1"
    shift
    CASE_DIR="$ROOT/case-$name"
    mkdir -p "$CASE_DIR"
    : >"$CASE_DIR/npm.log"
    : >"$CASE_DIR/curl.log"
    : >"$CASE_DIR/sleep.log"
    : >"$CASE_DIR/counter"

    local real_sleep=0
    if [ "${1:-}" = "--real-sleep" ]; then
        real_sleep=1
        shift
        # N4a mesure une durée : il lui faut le vrai `sleep`. On écarte la
        # doublure du PATH le temps du scénario.
        mv "$BIN/sleep" "$ROOT/sleep.parked"
    fi

    local rc=0
    NPM_STUB_LOG="$CASE_DIR/npm.log" \
    CURL_STUB_LOG="$CASE_DIR/curl.log" \
    SLEEP_STUB_LOG="$CASE_DIR/sleep.log" \
    NPM_STUB_COUNTER="$CASE_DIR/counter" \
    NPM_STUB_VERSION="$VERSION" \
    PATH="$BIN:$PATH" \
    env "$@" bash "$SCRIPT" "$PKG" "$VERSION" >"$CASE_DIR/out" 2>&1 || rc=$?
    printf '%s' "$rc" >"$CASE_DIR/exit"

    if [ "$real_sleep" -eq 1 ]; then
        mv "$ROOT/sleep.parked" "$BIN/sleep"
    fi
}

case_out() { cat "$CASE_DIR/out"; }
case_exit() { cat "$CASE_DIR/exit"; }

# Appels de BOUCLE : ceux qui ne portent pas `--json`. L'appel [2] du bloc de
# diagnostic est le seul à le porter, et c'est ce qui le rend soustractible —
# sans quoi « exactement MAX_ATTEMPTS » serait indécidable.
loop_calls() { grep -cv -- '--json' "$CASE_DIR/npm.log" || true; }

sleep_count() { grep -c '' "$CASE_DIR/sleep.log" || true; }

# ===========================================================================
# N1 — chemin nominal : visible au premier essai.
# ===========================================================================
run_case n1 NPM_STUB_VISIBLE_FROM=1
assert_eq "$(case_exit)" 0 "N1: version visible au 1ᵉʳ essai → exit 0"
assert_eq "$(loop_calls)" 1 "N1: un seul appel npm — le chemin nominal ne dort pas"
assert_eq "$(sleep_count)" 0 "N1: aucun sommeil sur le chemin nominal"
assert_contains "$(case_out)" "attempt 1/8" "N1: le succès nomme l'essai et la géométrie"

# ===========================================================================
# N2 — AC1. Le cas que l'ancienne géométrie (5 essais) ratait.
# ===========================================================================
run_case n2 NPM_STUB_VISIBLE_FROM=6 PUBLISH_VERIFY_SLEEP_BASE_SECS=0 PUBLISH_VERIFY_SLEEP_CAP_SECS=0
assert_eq "$(case_exit)" 0 "N2 (AC1): version visible au 6ᵉ essai → exit 0"
assert_contains "$(case_out)" "attempt 6/8" "N2 (AC1): le succès a bien eu lieu au 6ᵉ essai"

# ===========================================================================
# N3 — AC2, l'invariant dur. Une version qui n'apparaît jamais échoue.
# N7 — anti-vacuité : sans elle, remplacer le script par `exit 1` rendrait N3
#      vert pour la mauvaise raison. Un détecteur silencieusement inerte se lit
#      exactement comme un détecteur sain (classe mika#2205).
# ===========================================================================
run_case n3 NPM_STUB_VISIBLE_FROM=999 PUBLISH_VERIFY_MAX_ATTEMPTS=3 PUBLISH_VERIFY_SLEEP_BASE_SECS=0 PUBLISH_VERIFY_SLEEP_CAP_SECS=0
assert_eq "$(case_exit)" 1 "N3 (AC2): version jamais visible → exit 1"
assert_contains "$(case_out)" "::error::" "N3 (AC2): l'échec est annoté pour GitHub Actions"
assert_eq "$(loop_calls)" 3 "N7: le npm factice a été appelé exactement MAX_ATTEMPTS fois"

# ===========================================================================
# N4 — les sommeils sont ENTRE les essais, jamais après le dernier.
#   (a) encadrement de durée avec le vrai `sleep`
#   (b) contrôle positif déterministe : nombre et valeurs des sommeils
#
# N4a est le seul cas qui mesure le vrai temps écoulé. N4b et N4c comptent les
# appels à la doublure `sleep` : une attente réelle qui ne passe pas par la
# commande `sleep` (un `/bin/sleep` en chemin absolu, un `read -t`, un
# `timeout`) leur échappe et n'est visible qu'ici.
#
# Pourquoi des millisecondes (mika#2584). La mesure lisait `date +%s`, en
# secondes entières, avec un seuil à 3 s : un intervalle réel d'environ 2,05 s
# qui commence à x,97 s et finit à x+3,02 s se lit « 3 ». C'est un faux rouge
# vu en CI sur la PR #2583 : un défaut de MESURE du harnais, pas un
# comportement de verify-npm-publish.sh. La règle de l'en-tête (« on répare le
# script ») ne s'applique donc pas à ce cas-là.
#
# Pourquoi 2900 ms. Nominal : 3 essais, BASE=1, CAP=1 → 2 sommeils, ≈ 2000 ms
# plus le démarrage de bash et des doublures. Mutation « sommeil terminal de
# retour » : 3 sommeils d'au moins 1 s chacun → ≥ 3000 ms, plancher DUR puisque
# `sleep` garantit au moins sa durée. Tout seuil < 3000 ms garde donc le
# contrôle négatif rouge ; 2900 laisse ~900 ms au démarrage et 100 ms sous le
# plancher. Un seuil à 4 s (l'autre voie proposée par le ticket) laisserait
# passer la mutation : avec CAP=1, le sommeil terminal ne coûte qu'une seconde
# de plus.
#
# `date +%s%N` est GNU. Une horloge qui ne rend pas que des chiffres (un `date`
# BSD rend `%N` littéral) fait échouer N4a en le nommant : une garde qui ne peut
# plus mesurer ne doit pas se lire comme une garde qui passe.
N4A_THRESHOLD_MS=2900

now_ms() {
    local ns
    ns="$(date +%s%N)"
    case "$ns" in
        '' | *[!0-9]*) return 1 ;;
    esac
    printf '%s\n' "$((ns / 1000000))"
}

N4A_START_MS="$(now_ms)" || N4A_START_MS=""
run_case n4a --real-sleep NPM_STUB_VISIBLE_FROM=999 PUBLISH_VERIFY_MAX_ATTEMPTS=3 PUBLISH_VERIFY_SLEEP_BASE_SECS=1 PUBLISH_VERIFY_SLEEP_CAP_SECS=1
N4A_END_MS="$(now_ms)" || N4A_END_MS=""
if [ -z "$N4A_START_MS" ] || [ -z "$N4A_END_MS" ]; then
    ko "N4a: horloge sans millisecondes (\`date +%s%N\` ne rend pas que des chiffres) — la durée n'a pas pu être mesurée"
else
    N4A_ELAPSED_MS=$((N4A_END_MS - N4A_START_MS))
    if [ "$N4A_ELAPSED_MS" -lt "$N4A_THRESHOLD_MS" ]; then
        ok "N4a: 3 essais à 1 s de base coûtent 2 sommeils (${N4A_ELAPSED_MS} ms < ${N4A_THRESHOLD_MS} ms)"
    else
        ko "N4a: durée ${N4A_ELAPSED_MS} ms ≥ ${N4A_THRESHOLD_MS} ms — le sommeil terminal est de retour"
    fi
fi

run_case n4b NPM_STUB_VISIBLE_FROM=999 PUBLISH_VERIFY_MAX_ATTEMPTS=3 PUBLISH_VERIFY_SLEEP_BASE_SECS=1 PUBLISH_VERIFY_SLEEP_CAP_SECS=1
assert_eq "$(sleep_count)" 2 "N4b: exactement 2 sommeils pour 3 essais"

# La forme du backoff elle-même, à la géométrie livrée : 15, 30, puis plafond.
run_case n4c NPM_STUB_VISIBLE_FROM=999 PUBLISH_VERIFY_MAX_ATTEMPTS=4
assert_eq "$(tr '\n' ' ' <"$CASE_DIR/sleep.log")" "15 30 60 " "N4c: backoff 15 → 30 → plafond 60"

# ===========================================================================
# N5 — D3 est réellement branché.
# ===========================================================================
run_case n5 NPM_STUB_VISIBLE_FROM=1
if grep -q -- '--prefer-online' "$CASE_DIR/npm.log"; then
    ok "N5 (D3): l'argv du npm factice porte --prefer-online"
else
    ko "N5 (D3): --prefer-online absent de l'argv du npm factice"
fi

# ===========================================================================
# N6 — D4. La cause n'est plus effacée : un ENETWORK ne se lit plus comme un
#      « not visible on the default registry ».
# ===========================================================================
run_case n6 NPM_STUB_ERROR_CODE=ENETWORK PUBLISH_VERIFY_MAX_ATTEMPTS=2 PUBLISH_VERIFY_SLEEP_BASE_SECS=0 PUBLISH_VERIFY_SLEEP_CAP_SECS=0 CURL_STUB_MODE=injoignable
assert_eq "$(case_exit)" 1 "N6 (D4): une erreur npm persistante échoue toujours"
assert_contains "$(case_out)" "ENETWORK" "N6 (D4): le message final porte la cause npm"
assert_contains "$(case_out)" "diagnostic post-échec (mika#2566)" "N6 (D4): le bloc de diagnostic est émis"
assert_contains "$(case_out)" "Table de lecture" "N6 (D4): la table de lecture est imprimée"
# D5 — une erreur non-E404 ne raccourcit PAS la boucle : elle va au bout.
assert_eq "$(loop_calls)" 2 "N6 (D5): une erreur réseau ne raccourcit pas la boucle"

# La branche H2 de la table : le registre porte la version, le client npm ne la
# voit pas. C'est le cas où rallonger la fenêtre ne réglerait rien.
run_case n6b NPM_STUB_VISIBLE_FROM=999 PUBLISH_VERIFY_MAX_ATTEMPTS=1 CURL_STUB_MODE=packument-avec-version
assert_contains "$(case_out)" "la version est presente dans le packument servi" \
    "N6b (D4): le registre HTTP direct sépare H2 de H1"
# L'autre moitié de la même ligne de table, et c'est celle qui se casse en
# silence : le `npm view --json` du diagnostic a rendu un objet d'erreur CITANT
# la version. Une recherche de sous-chaîne y répondrait « oui », inverserait la
# lecture H1/H2 et enverrait l'opérateur rallonger une fenêtre qui n'y peut rien.
assert_contains "$(case_out)" "le client npm voit la version : non" \
    "N6c (D4): un objet d'erreur JSON citant le spec ne compte pas comme « npm voit la version »"

# ===========================================================================
# N8 — D7. La géométrie dite est celle appliquée.
# ===========================================================================
run_case n8 NPM_STUB_VISIBLE_FROM=999 PUBLISH_VERIFY_MAX_ATTEMPTS=3 PUBLISH_VERIFY_SLEEP_BASE_SECS=1 PUBLISH_VERIFY_SLEEP_CAP_SECS=1
assert_contains "$(case_out)" "verify geometry: attempts=3 base=1s cap=1s window=2s prefer-online=yes" \
    "N8 (D7): la ligne geometry porte les valeurs injectées"

run_case n8b NPM_STUB_VISIBLE_FROM=1
assert_contains "$(case_out)" "verify geometry: attempts=8 base=15s cap=60s window=345s prefer-online=yes" \
    "N8b (D2): la géométrie livrée couvre 345 s"

# ===========================================================================
# N9 — D6. Pas de désarmement silencieux.
# ===========================================================================
run_case n9 NPM_STUB_VISIBLE_FROM=999 PUBLISH_VERIFY_MAX_ATTEMPTS=0 PUBLISH_VERIFY_SLEEP_BASE_SECS=0 PUBLISH_VERIFY_SLEEP_CAP_SECS=0
assert_contains "$(case_out)" '"0"' "N9 (D6): l'avertissement nomme la valeur entre guillemets"
assert_contains "$(case_out)" "attempts=8" "N9 (D6): un MAX_ATTEMPTS=0 retombe sur le défaut"
assert_eq "$(loop_calls)" 8 "N9 (D6): huit essais ont bien eu lieu"

run_case n9b NPM_STUB_VISIBLE_FROM=1 PUBLISH_VERIFY_MAX_ATTEMPTS=nope
assert_contains "$(case_out)" '"nope"' "N9b (D6): une valeur illisible est nommée entre guillemets"
assert_contains "$(case_out)" "attempts=8" "N9b (D6): une valeur illisible retombe sur le défaut"

run_case n9c NPM_STUB_VISIBLE_FROM=1 PUBLISH_VERIFY_SLEEP_BASE_SECS=0
assert_contains "$(case_out)" "base=0s" "N9c (D6): SLEEP_BASE=0 est honoré, pas refusé"

# ===========================================================================
# Fire-Disposition — les deux assertions auto-nettoyantes.
# ===========================================================================
if [ "${#VERIFY_EXCEPTION_ALLOWLIST[@]}" -eq 0 ]; then
    ok "Table de dérogations vide — aucune violation préexistante n'est exemptée"
else
    ko "Table de dérogations non vide : ${#VERIFY_EXCEPTION_ALLOWLIST[@]} entrée(s). Réparer le script, pas la table (mika#2201)."
fi

if grep -qiE 'ALLOWLIST|EXCEPTION' "$SCRIPT"; then
    ko "Le script gardé consulte une table de dérogations : échappatoire sur le chemin où AC2 doit tenir sans condition"
else
    ok "Le script gardé ne consulte aucune table de dérogations au runtime"
fi

# ===========================================================================
printf '\n%s passed, %s failed\n' "$PASS" "$FAIL"
if [ "$FAIL" -ne 0 ]; then
    exit 1
fi
