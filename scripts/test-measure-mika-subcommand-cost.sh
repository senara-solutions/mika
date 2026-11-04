#!/usr/bin/env bash
# test-measure-mika-subcommand-cost.sh — ce que la sonde S3 refuse, et ce qu'elle
# rend (mika#2194, vol 7). Six contrôles.
#
# Run: make test-measure-mika-subcommand-cost
#
# AUCUN VRAI `mika`, ET C'EST LA CONDITION POUR QUE CI LA JOUE. La suite fabrique
# un faux binaire dans un répertoire temporaire et le passe par `--mika` : sa
# latence vient d'un `sleep` piloté par `$FAKE_MIKA_SLEEP`, donc les deux
# polarités du seuil sont reproductibles sur n'importe quelle machine. Mesurer un
# vrai `mika` rendrait un test dont le verdict dépend de la charge du runner —
# c'est-à-dire pas un test.
#
# `--mika` PLUTÔT QU'UN `PATH` VIDE pour le contrôle du binaire absent. Un `PATH`
# vide prive aussi l'instrument de `sed`, `grep`, `sort` et `mktemp` : ce qui
# échouerait alors est le harnais, pas la résolution de `mika`, et le contrôle
# serait vert pour la mauvaise raison. `--mika <nom introuvable>` exerce
# exactement le prédicat visé — et c'est le même chemin que le défaut, qui
# résout le nom nu `mika` sur `$PATH`.
#
# LES DEUX POLARITÉS DU SEUIL VIVENT DANS CE MÊME CORPS (contrôles 2 et 3), avec
# le MÊME instrument et la MÊME latence : seul le seuil change. Un test qui ne
# prouve que le refus ne distingue pas une garde qui marche d'une garde toujours
# rouge — c'est l'exigence qu'AC3 du ticket posait déjà sur les codes `≥2`.
#
# L'INSTRUMENT EST APPELÉ PAR `bash "$MEASURE_PATH"`, délibérément : le mode
# d'exécution n'est pas une dépendance de la suite (l'index git porte `100755`,
# et c'est lui qui le garantit, pas ce fichier).

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MEASURE_PATH="$REPO_ROOT/scripts/measure-mika-subcommand-cost"
DISPATCH_LIB="$REPO_ROOT/skills/bundled/_shared/dispatch-lib.sh"

PASS=0
FAIL=0
ok() {
    PASS=$((PASS + 1))
    printf '  ok   %s\n' "$1"
}
ko() {
    FAIL=$((FAIL + 1))
    printf '  FAIL %s\n     expected: %s\n     actual:   %s\n' "$1" "$2" "$3"
}

WORK="$(mktemp -d)" || exit 2
trap 'rm -rf "$WORK"' EXIT

# -- le faux `mika` ------------------------------------------------------------
#
# Contrat de sortie minimal : ce que les deux sous-commandes écrivent sur le
# chemin nominal, et un `0`. L'instrument VÉRIFIE ce `0` (il refuserait de
# chronométrer un chemin d'erreur), donc le faux doit le rendre.

FAKE_DIR="$WORK/bin"
mkdir -p "$FAKE_DIR"
cat >"$FAKE_DIR/mika" <<'FAKE_MIKA'
#!/usr/bin/env bash
# Faux `mika` de test. La latence est la seule chose qu'il simule.
sleep "${FAKE_MIKA_SLEEP:-0}"
case "${1:-}" in
    plan-callout)     printf 'docs/plans/2026-11-04-001-x-plan.md\n' ;;
    arch-disposition) printf 'READY\n1a\n' ;;
    *)                printf 'faux mika: sous-commande inattendue : %s\n' "${1:-}" >&2; exit 2 ;;
esac
exit 0
FAKE_MIKA
chmod +x "$FAKE_DIR/mika"

RC=0
run_measure() {
    "$@" >"$WORK/out" 2>"$WORK/err"
    RC=$?
}

printf '\n== measure-mika-subcommand-cost : six contrôles ==\n\n'

# -- 1. le chemin nominal ------------------------------------------------------

run_measure bash "$MEASURE_PATH" --mika "$FAKE_DIR/mika" --repeat 3
if [[ "$RC" -ne 0 ]]; then
    ko "1. chemin nominal : code 0" "0" "$RC — stderr: $(tr '\n' ' ' <"$WORK/err")"
elif ! grep -qE '^plan-callout +p50=[0-9]+ms p95=[0-9]+ms sites=[0-9]+ ' "$WORK/out"; then
    ko "1. chemin nominal : p50 et p95 pour plan-callout" \
        "une ligne 'plan-callout p50=<n>ms p95=<n>ms sites=<n>'" "$(tr '\n' '|' <"$WORK/out")"
elif ! grep -qE '^arch-disposition +p50=[0-9]+ms p95=[0-9]+ms sites=[0-9]+ ' "$WORK/out"; then
    ko "1. chemin nominal : p50 et p95 pour arch-disposition" \
        "une ligne 'arch-disposition p50=<n>ms p95=<n>ms sites=<n>'" "$(tr '\n' '|' <"$WORK/out")"
elif ! grep -qE '^sites-total: [1-9][0-9]*$' "$WORK/out"; then
    # Le compte EXACT n'est pas épinglé ici : ce serait une seconde source de
    # vérité pour un chiffre qui vit dans `dispatch-lib.sh`, et le prochain vol
    # qui ajoute un site rougirait cette suite sans qu'aucune garde n'ait cédé.
    # C'est la DÉRIVATION qu'on prouve, au contrôle 5.
    ko "1. chemin nominal : un compte de sites non nul" \
        "'sites-total: <n≥1>'" "$(grep -c . "$WORK/out") lignes, pas de sites-total"
elif ! grep -q '^pire-cas-dispatch: ' "$WORK/out"; then
    ko "1. chemin nominal : le produit p95 × sites" "'pire-cas-dispatch: <n> ms'" "absent"
else
    ok "1. chemin nominal — code 0, p50 et p95 pour les deux sous-commandes, compte et produit"
fi

# -- 2. le seuil est franchi ---------------------------------------------------

export FAKE_MIKA_SLEEP=0.05
run_measure bash "$MEASURE_PATH" --mika "$FAKE_DIR/mika" --repeat 2 --threshold-ms 1
if [[ "$RC" -ne 3 ]]; then
    ko "2. seuil franchi : code 3" "3" "$RC — stderr: $(tr '\n' ' ' <"$WORK/err")"
elif ! grep -q 'plan-callout' "$WORK/err"; then
    ko "2. seuil franchi : stderr nomme la sous-commande" "'plan-callout'" "$(tr '\n' ' ' <"$WORK/err")"
elif ! grep -q 'placement dans `main.rs`' "$WORK/err"; then
    ko "2. seuil franchi : stderr nomme le remède de la doctrine" \
        "'placement dans \`main.rs\`'" "$(tr '\n' ' ' <"$WORK/err")"
elif grep -q 'cache' "$WORK/err" && ! grep -q 'jamais un cache' "$WORK/err"; then
    ko "2. seuil franchi : le cache est REFUSÉ, pas proposé" "'jamais un cache'" "$(tr '\n' ' ' <"$WORK/err")"
else
    ok "2. seuil franchi — code 3, la sous-commande et le remède (placement, jamais un cache) nommés"
fi

# -- 3. le seuil n'est pas franchi — même instrument, même latence -------------

run_measure bash "$MEASURE_PATH" --mika "$FAKE_DIR/mika" --repeat 2 --threshold-ms 5000
if [[ "$RC" -ne 0 ]]; then
    ko "3. seuil non franchi : code 0" "0" "$RC — stderr: $(tr '\n' ' ' <"$WORK/err")"
elif [[ -s "$WORK/err" ]]; then
    ko "3. seuil non franchi : stderr VIDE" "rien" "$(tr '\n' ' ' <"$WORK/err")"
else
    ok "3. seuil non franchi — code 0 et stderr vide, à latence identique au contrôle 2"
fi
unset FAKE_MIKA_SLEEP

# -- 4. le binaire est absent --------------------------------------------------
#
# Le nom cherché est DÉLIBÉRÉMENT sans chiffre : l'assertion « aucun chiffre dans
# la sortie » porte sur toute la sortie, et un chemin `mktemp` en porterait.

run_measure bash "$MEASURE_PATH" --mika mika-qui-nexiste-pas --repeat 3
if [[ "$RC" -ne 2 ]]; then
    ko "4. binaire absent : code 2" "2" "$RC"
elif ! grep -q 'introuvable' "$WORK/err"; then
    ko "4. binaire absent : message explicite" "'introuvable'" "$(tr '\n' ' ' <"$WORK/err")"
elif ! grep -q 'mika-qui-nexiste-pas' "$WORK/err"; then
    ko "4. binaire absent : le nom cherché est nommé" "'mika-qui-nexiste-pas'" "$(tr '\n' ' ' <"$WORK/err")"
elif [[ -s "$WORK/out" ]]; then
    ko "4. binaire absent : stdout VIDE" "rien" "$(tr '\n' ' ' <"$WORK/out")"
elif grep -qE '[0-9]' "$WORK/err"; then
    ko "4. binaire absent : AUCUN chiffre dans la sortie" \
        "pas un chiffre — un SKIP ne rend pas de mesure (mika#2205)" "$(tr '\n' ' ' <"$WORK/err")"
else
    ok "4. binaire absent — code 2, message explicite, aucun chiffre nulle part"
fi

# -- 5. le compte de sites est DÉRIVÉ -----------------------------------------
#
# Un septième site ajouté à une COPIE doit se lire. Une constante dans
# l'instrument rendrait toujours six.

SEVEN="$WORK/dispatch-lib-sept-sites.sh"
cp "$DISPATCH_LIB" "$SEVEN"
{
    printf '\n_mika2194_vol7_septieme_site() {\n'
    printf '    printf '"'"'%%s'"'"' "$1" | _parse_verdict\n'
    printf '}\n'
} >>"$SEVEN"

run_measure bash "$MEASURE_PATH" --mika "$FAKE_DIR/mika" --repeat 1 --dispatch-lib "$DISPATCH_LIB"
BASELINE="$(sed -n 's/^sites-total: //p' "$WORK/out")"
run_measure bash "$MEASURE_PATH" --mika "$FAKE_DIR/mika" --repeat 1 --dispatch-lib "$SEVEN"
WITH_SEVENTH="$(sed -n 's/^sites-total: //p' "$WORK/out")"
if [[ "$RC" -ne 0 ]]; then
    ko "5. compte dérivé : code 0 sur la copie" "0" "$RC — stderr: $(tr '\n' ' ' <"$WORK/err")"
elif [[ -z "$BASELINE" || -z "$WITH_SEVENTH" ]]; then
    ko "5. compte dérivé : un compte sur chaque copie" "deux entiers" "'$BASELINE' et '$WITH_SEVENTH'"
elif [[ "$WITH_SEVENTH" -ne $((BASELINE + 1)) ]]; then
    ko "5. compte dérivé : le septième site est vu" \
        "$((BASELINE + 1)) (= $BASELINE + 1)" "$WITH_SEVENTH"
else
    ok "5. compte dérivé — $BASELINE sites dans l'arbre, $WITH_SEVENTH sur une copie qui en porte un de plus"
fi

# -- 6. anti-vacuité : population vide ≠ coût nul -----------------------------

EMPTY="$WORK/dispatch-lib-sans-site.sh"
grep -v -e '_extract_plan_path' -e '_parse_disposition' -e '_parse_verdict' \
    "$DISPATCH_LIB" >"$EMPTY"

run_measure bash "$MEASURE_PATH" --mika "$FAKE_DIR/mika" --repeat 1 --dispatch-lib "$EMPTY"
BOTH="$(cat "$WORK/out" "$WORK/err")"
if [[ "$RC" -ne 2 ]]; then
    ko "6. anti-vacuité : code 2" "2" "$RC — sortie: $(printf '%s' "$BOTH" | tr '\n' ' ')"
elif ! grep -qF -- 'sites-total: 0' <<<"$BOTH"; then
    ko "6. anti-vacuité : le compte rend 0" "'sites-total: 0'" "$(printf '%s' "$BOTH" | tr '\n' ' ')"
elif ! grep -qF -- 'population vide' <<<"$BOTH"; then
    ko "6. anti-vacuité : la population vide est SIGNALÉE" \
        "'population vide'" "$(printf '%s' "$BOTH" | tr '\n' ' ')"
elif grep -qF -- 'pire-cas' <<<"$BOTH"; then
    ko "6. anti-vacuité : AUCUN coût n'est rendu" \
        "pas de 'pire-cas' — un 0 ms se lirait « négligeable »" "$(printf '%s' "$BOTH" | tr '\n' ' ')"
else
    ok "6. anti-vacuité — code 2, 'sites-total: 0' et une population vide nommée, aucun coût rendu"
fi

printf '\n%d passed, %d failed\n\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]] || exit 1
