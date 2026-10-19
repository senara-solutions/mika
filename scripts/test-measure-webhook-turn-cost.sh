#!/usr/bin/env bash
#
# Harnais anti-vacuité de scripts/measure-webhook-turn-cost (mika#2680, vol 2).
#
# Le § 3 a été ajouté par le vol 4 (mika#2680) pour le terme de relâche : même
# discipline, ses deux moitiés sur deux corpora gelés distincts.
#
# POURQUOI UN CONTRÔLE POSITIF ET SIX NÉGATIFS, ET PAS AUTRE CHOSE. Une batterie
# qui ne porte que des négatifs est satisfaite par un lecteur qui ne rend jamais
# rien ; une batterie qui ne porte que le positif est satisfaite par un lecteur
# qui attribue tout. Les deux moitiés sont donc requises, et les négatifs sont
# asserés SÉPARÉMENT — une entrée par confusion — jamais par un corpus qui les
# neutraliserait tous d'un coup : une conjonction de termes fail-safe n'est pas
# prouvée en les invalidant tous ensemble (leçon mika#2277). C'est pourquoi « pas
# de ligne payée » (N4) et « rien que de l'illisible » (N6) ont chacun leur
# corpus gelé plutôt qu'une branche d'un corpus commun.
#
# CE QUE LE CONTRÔLE POSITIF TIENT, ET QUE RIEN D'AUTRE NE TIENT. L'unité est le
# TOUR, la valeur est une SOMME : `emit_turn_usage` écrit une ligne par étape du
# prompt, plus une ligne de continuation (`step = u32::MAX`). La mutation « une
# ligne par tour » — la plus naturelle à écrire, et celle qu'un relecteur laisse
# passer — rendrait 500 au lieu de 1100 sur `req-g-1`, en sous-comptant d'un
# facteur égal au nombre d'étapes, donc le plus fort sur les tours les plus
# chers : exactement la population que la mesure poursuit. §1 la voit rouge.
#
# LES FIXTURES SONT GELÉES ET SYNTHÉTIQUES, DÉLIBÉRÉMENT. Elles ne sont pas
# échantillonnées depuis `$MIKA_SPIRIT_LOG_FILE` : une suite qui lirait le corpus
# vivant changerait de verdict à mesure qu'il grossit, et ne tournerait pas sur
# une machine qui n'a jamais dispatché. La fidélité vient du PRODUCTEUR — chaque
# champ est ce que `info!` écrit à travers la couche JSON de `tracing_subscriber`
# avec `flatten_event(true)`, y compris la collision de `target` sur la ligne
# écartée : le champ utilisateur (`senara-solutions/mika#2679`) et la métadonnée
# du module sont tous deux à la racine, et c'est le second qui gagne. Le lecteur
# ne lit jamais `.target` ; la ligne du fixture est là pour que quiconque
# voudrait l'y ajouter voie d'abord ce qu'il lirait vraiment.
#
# Le mode exécutable n'a pas d'assertion ici : le harnais invoque par `bash`
# précisément pour ne pas en dépendre, et l'index git le porte
# (`git ls-files -s scripts/measure-webhook-turn-cost` → `100755`).
#
# Run: make test-measure-webhook-turn-cost
#      (ou: bash scripts/test-measure-webhook-turn-cost.sh)

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MEASURE_PATH="$REPO_ROOT/scripts/measure-webhook-turn-cost"
FIXTURES="$REPO_ROOT/crates/mika-agent/tests/fixtures/mika2680"
CORPUS="$FIXTURES/register_corpus.jsonl"
NO_CHARGED="$FIXTURES/no_charged_corpus.jsonl"
UNPARSEABLE_ONLY="$FIXTURES/unparseable_only_corpus.jsonl"
RELEASE="$FIXTURES/release_corpus.jsonl"
RELEASE_ABSENT="$FIXTURES/release_absent_corpus.jsonl"

# Le scratch reste DANS le worktree (`target/` est gitignoré) : la
# permission-policy du dispatch refuse toute écriture hors worktree, et une
# suite qui ne peut pas écrire est une suite qui ne rapporte rien.
WORK="$REPO_ROOT/target/mika2680-test"
rm -rf "$WORK"
mkdir -p "$WORK"

# Invoqué par `bash` plutôt que par chemin : le bit d'exécution est une propriété
# de l'arbre committé, pas du checkout qui exécute. Un worktree restauré sans lui
# cacherait tous les résultats ci-dessous derrière un « permission denied ».
MEASURE() { bash "$MEASURE_PATH" "$@"; }

PASS=0
FAIL=0
ok() { PASS=$((PASS + 1)); printf '  ok   %s\n' "$1"; }
ko() { FAIL=$((FAIL + 1)); printf '  FAIL %s\n     attendu : %s\n     obtenu  : %s\n' "$1" "$2" "$3"; }
eq() { if [[ "$2" == "$3" ]]; then ok "$1"; else ko "$1" "$2" "$3"; fi; }

# Un champ d'un tour payé, désigné par son request_id.
champ() { jq -r --arg r "$2" --arg f "$3" 'select(.request_id == $r) | .[$f]' "$1"; }

printf '\n== mika#2680 — measure-webhook-turn-cost ==\n\n'

MEASURE --log-file "$CORPUS" --until 2026-10-16 \
        --out "$WORK/turns.jsonl" --summary "$WORK/summary.json" 2>"$WORK/stderr.txt"
RC=$?
if [[ "$RC" -ne 0 ]]; then
    printf '  FAIL le corpus gelé n'"'"'est pas lisible (rc=%s)\n     stderr: %s\n\n' \
        "$RC" "$(cat "$WORK/stderr.txt")"
    exit 1
fi

# ---------------------------------------------------------------------------
printf -- '-- 1. contrôle positif : l'"'"'unité est le tour, la valeur est une somme\n'
# ---------------------------------------------------------------------------
# `req-g-1` porte trois lignes `turn_usage` — étapes 0, 1 et la continuation
# `u32::MAX` — à 500 + 400 + 200. Un lecteur qui prendrait une ligne par tour
# rendrait 500 ; un lecteur qui oublierait la continuation rendrait 900.
eq "les trois lignes turn_usage d'un tour font 1100, pas 500" \
   "1100" "$(champ "$WORK/turns.jsonl" req-g-1 input_tokens)"

# ---------------------------------------------------------------------------
printf -- '\n-- 2. six négatifs, une entrée chacun\n'
# ---------------------------------------------------------------------------
# N1 — `t-orphan-1` n'a aucune ligne payée de même trace_id. Il ne reçoit donc
# AUCUN trigger : lui en attribuer un (fût-ce « unknown ») ferait entrer du coût
# Telegram dans le tableau des déclencheurs webhook, sur la case même que la
# mesure doit chiffrer. Il tombe dans le seau non attribué, sous son nom.
eq "N1 — un turn_usage orphelin n'a pas de trigger et vit dans le seau" \
   "|t-orphan-1" \
   "$(champ "$WORK/turns.jsonl" t-orphan-1 trigger)|$(jq -r '(.unattributed_undetermined // .unattributed).trace_ids | join(",")' "$WORK/summary.json")"

# N2 — motif mika#2331 : absent n'est ni zéro ni "none". `req-nocand-1` n'a pas
# de champ `candidate` sur le fil ; le rendre `"none"` ou `"0"` créerait une
# classe candidate qui n'existe pas, et `//` de jq y suffirait.
eq "N2 — un candidate absent sort null, jamais \"none\" ni \"0\"" \
   "null" "$(champ "$WORK/turns.jsonl" req-nocand-1 candidate)"

# N3 — un tour écarté n'a pas eu lieu. Il ne doit produire aucune ligne côté
# payé (sinon le registre compterait comme coût ce que le pré-filtre a évité),
# et doit rester visible dans sa propre moitié.
eq "N3 — un tour écarté ne produit aucune ligne payée mais reste compté" \
   "|1" \
   "$(champ "$WORK/turns.jsonl" req-skip-1 request_id)|$(jq -r '.skipped_by_class.green_check_suite_head_decided' "$WORK/summary.json")"

# N4 — un scan inerte en silence se lit exactement comme un scan qui n'a rien
# trouvé (mika#2205). Ce corpus-ci est bien formé et contient du `turn_usage` :
# seule l'absence de ligne payée décide.
MEASURE --log-file "$NO_CHARGED" >/dev/null 2>&1
eq "N4 — un corpus sans aucune ligne webhook_turn_charged sort 2" "2" "$?"

# N5 — une fenêtre qui n'est pas celle qu'on a demandée est un mensonge
# d'instrument. La borne est refusée, pas interprétée au mieux.
MEASURE --log-file "$CORPUS" --since hier >/dev/null 2>&1
eq "N5 — une borne mal formée est refusée, pas interprétée" "2" "$?"

# N6 — le compte d'illisibles NE CONJUGUE PAS la décision « rien mesuré ». Ici
# l'unique ligne du corpus est illisible : elle est comptée, et elle n'autorise
# à elle seule aucun `exit 0`. Réciproquement, si elle entrait dans la
# conjonction, une seule ligne tronquée — quasi certaine sur un journal de
# 19 Go — rendrait `exit 2` inatteignable pour toujours.
ERR="$(MEASURE --log-file "$UNPARSEABLE_ONLY" 2>&1 >/dev/null)"
RC6=$?
eq "N6 — une ligne illisible est comptée et n'autorise pas un exit 0" \
   "2|oui" \
   "$RC6|$(case "$ERR" in *'(1 ligne(s) illisible(s))'*) echo oui ;; *) echo "non : $ERR" ;; esac)"

# ---------------------------------------------------------------------------
printf -- '\n-- 3. le terme de relâche : un seau par terme, l'"'"'absence nommée\n'
# ---------------------------------------------------------------------------
# P2 — le positif. Deux tours payés par deux portes différentes font DEUX seaux
# de 1, pas un seau de 2 : un agrégat qui fusionnerait les termes serait vert
# sur une batterie purement négative, et ne saurait plus dire quelle porte
# relâche le coût — la seule question que ce champ sert à poser.
MEASURE --log-file "$RELEASE" --out "$WORK/rel-turns.jsonl" \
        --summary "$WORK/rel-summary.json" >/dev/null 2>&1
eq "P2 — deux release distincts font deux seaux de 1" \
   '{"head_not_previously_decided":1,"pr_open":1}|head_not_previously_decided' \
   "$(jq -cS '.charged_by_release' "$WORK/rel-summary.json")|$(champ "$WORK/rel-turns.jsonl" req-rel-1 release)"

# N7 — le négatif, sur son propre corpus gelé (leçon mika#2277 : une entrée par
# confusion). Une ligne écrite par un binaire d'AVANT le champ n'a pas de
# `release` ; elle tombe dans `absent_pre_field` et la ligne du tour garde
# `null`. Un seau littéralement nommé `null` — ce que `group_by(.release)` sans
# repli produirait — se lirait comme un terme de relâche qui n'existe pas
# (motif mika#2331), sur la colonne même qui doit dire ce qui reste à filtrer.
MEASURE --log-file "$RELEASE_ABSENT" --out "$WORK/abs-turns.jsonl" \
        --summary "$WORK/abs-summary.json" >/dev/null 2>&1
eq "N7 — un release absent est nommé absent_pre_field, jamais un seau null" \
   '{"absent_pre_field":1}|null|false' \
   "$(jq -cS '.charged_by_release' "$WORK/abs-summary.json")|$(champ "$WORK/abs-turns.jsonl" req-pre-field-1 release)|$(jq -r '.charged_by_release | has("null")' "$WORK/abs-summary.json")"

# ---------------------------------------------------------------------------
printf '\n%d passées, %d échouées\n\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]] || exit 1
