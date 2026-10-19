#!/usr/bin/env bash
#
# Harnais anti-vacuité de scripts/measure-webhook-turn-cost (mika#2680, vol 2).
#
# Le § 3 a été ajouté par le vol 4 (mika#2680) pour le terme de relâche : même
# discipline, ses deux moitiés sur deux corpora gelés distincts.
#
# Le § 4 a été ajouté par le vol 5 (mika#2680) pour le croisement des deux
# taxonomies. Ses quatre assertions partagent UN corpus, et c'est la propriété
# de ce corpus qui l'autorise : il est construit pour que les deux seaux plats
# soient ambigus — un `{head_not_previously_decided: 2}` et un
# `{green_check_suite_head_decided: 2}` qui ne disent ni de quelle cellule ni de
# quel agent ils parlent — et pour que les deux seaux croisés lèvent chacune de
# ces ambiguïtés séparément. Ce n'est donc pas le corpus que mika#2277 proscrit,
# celui qui neutraliserait plusieurs gardes fail-safe d'un coup : chaque
# assertion y tient une confusion distincte, sur une propriété que les trois
# autres ne touchent pas.
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
CROSS_TAB="$FIXTURES/cross_tab_corpus.jsonl"

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
printf -- '\n-- 4. croiser les deux taxonomies : la cellule, et son coût\n'
# ---------------------------------------------------------------------------
# Un seul corpus pour les quatre, et c'est sa PROPRIÉTÉ qui le permet : il est
# construit pour que les deux seaux plats soient AMBIGUS et les deux seaux
# croisés ne le soient pas. `charged_by_release` y rend
# `{head_not_previously_decided: 2}` et `skipped_by_class`
# `{green_check_suite_head_decided: 2}` — deux chiffres qui ne disent pas de
# quelle cellule ni de quel agent ils parlent. Chaque assertion ci-dessous tient
# une confusion distincte sur cette même entrée (leçon mika#2277 : une entrée
# par confusion, pas un corpus qui les neutralise toutes d'un coup).
MEASURE --log-file "$CROSS_TAB" --out "$WORK/xtab-turns.jsonl" \
        --summary "$WORK/xtab-summary.json" >/dev/null 2>&1

# Une cellule du seau croisé, désignée par ses trois clés. « cellule absente »
# plutôt qu'un champ vide : un croisement à deux clés ne rendrait pas une
# cellule à la valeur fausse, il ne rendrait AUCUNE cellule, et les deux cas se
# diagnostiquent différemment.
cellule() {
    jq -r --arg a "$1" --arg t "$2" --arg r "$3" '
        [.by_agent_trigger_release[]?
         | select(.agent_id == $a and .trigger == $t and .release == $r)]
        | if length == 0 then "cellule absente"
          else "\(.[0].turns):\(.[0].input_tokens):\(.[0].cache_read_tokens):\(.[0].llm_calls)"
          end' "$WORK/xtab-summary.json"
}

# P3 — le positif, et il porte DEUX propriétés que rien d'autre ne porte.
#
# (a) L'UNITÉ EST LE TOKEN, PAS LE TOUR. La forme la plus naturelle à écrire —
# copier le `value: length` du seau plat juste au-dessus — rendrait un agrégat
# qui a l'air complet et sur lequel (g) reste indécidable : tout ce ticket
# chiffre en tokens d'entrée (193 k, 279 k, 741 k, 1,30 M), et un plafond en
# tokens moins un soustrahend en tours n'est pas une soustraction mais une
# comparaison de deux unités. Les deux cellules portent ici le MÊME compte de
# tours (1) pour des coûts de 900 et 120 : un lecteur qui n'émettrait que des
# comptes est indistinguable d'un lecteur correct sur un contrôle de comptes, et
# devient rouge ici.
#
# (b) LE CROISEMENT EST SUR TROIS CLÉS. Les deux cellules ne diffèrent que par
# l'agent — même trigger, même terme de relâche. Un croisement sur
# `[trigger, release]` les fusionnerait en une cellule de 2 ; un croisement sur
# `[agent_id, trigger]` n'aurait pas de champ `release` et ne rendrait aucune
# cellule. Les deux mutations sont rouges sur cette ligne.
eq "P3 — deux cellules d'un même release, comptes égaux et coûts en tokens distincts" \
   "1:900:150:3|1:120:0:2" \
   "$(cellule mika-qa check_suite_pr_branch head_not_previously_decided)|$(cellule mika-dev check_suite_pr_branch head_not_previously_decided)"

# N8 — motif mika#2331, sur l'axe du nouveau seau. Le repli doit être dans la
# clé de GROUPEMENT **et** dans la projection. Posé dans la clé seule, deux
# lignes sans `release` grouperaient bien ensemble mais la projection rendrait
# `null` : le seau porterait le bon compte sous un nom faux. L'assertion tient
# les deux moitiés — la cellule nommée existe, et aucune cellule de l'axe ne
# vaut `null`.
eq "N8 — un release absent fait une cellule absent_pre_field, jamais une cellule null" \
   "1:60:0:1|false" \
   "$(cellule mika-qa issue_comment absent_pre_field)|$(jq -r '[.by_agent_trigger_release[]?.release] | any(. == null)' "$WORK/xtab-summary.json")"

# N9 — le négatif qui compte le plus : CROISER N'EST PAS REMPLACER. Les trois
# seaux antérieurs répondent à des questions qui restent posées — « combien de
# tours une panne de lecture a-t-elle coûtés sur toute la fenêtre » se lit sur le
# plat et n'a pas besoin du grain fin — et deux assertions de ce harnais (N3, P2,
# N7) portent sur leur forme. Une fusion déguisée en ajout serait réécrire une
# preuve au lieu d'en ajouter une. Assérable par littéraux parce que les trois
# seaux sont des fonctions pures du même corpus gelé : rien ici ne se compare à
# une exécution antérieure.
eq "N9 — by_agent_trigger, charged_by_release et skipped_by_class sont inchangés" \
   'mika-qa/check_suite_pr_branch=1:900:150:3 mika-dev/check_suite_pr_branch=1:120:0:2 mika-qa/issue_comment=1:60:0:1|{"absent_pre_field":1,"head_not_previously_decided":2}|{"green_check_suite_head_decided":2}' \
   "$(jq -r '[.by_agent_trigger[] | "\(.agent_id)/\(.trigger)=\(.turns):\(.input_tokens):\(.cache_read_tokens):\(.llm_calls)"] | join(" ")' "$WORK/xtab-summary.json")|$(jq -cS '.charged_by_release' "$WORK/xtab-summary.json")|$(jq -cS '.skipped_by_class' "$WORK/xtab-summary.json")"

# N10 — la moitié écartée, et la symétrie qu'il ne faut PAS faire. (g) est une
# assertion sur mika-qa, et (b) décide par agent : un
# `skipped_by_class.green_check_suite_head_decided` de 2 peut être 2 écarts côté
# mika-dev et ZÉRO côté mika-qa, auquel cas (b) ne couvre rien du côté qui
# intéresse (g). Les deux lectures mènent à deux décisions différentes.
# Et le seau croisé ne porte AUCUNE valeur de coût : un tour écarté n'a pas eu
# lieu, donc il n'a aucun coût en tokens à porter, et lui en inventer un ferait
# entrer dans le tableau un coût que personne n'a payé — la somme même que le
# § 4 du protocole interdit.
eq "N10 — skipped_by_agent_class sépare deux agents de même classe, sans clé de coût" \
   "mika-dev=1,mika-qa=1|agent_id,class,skipped" \
   "$(jq -r '[.skipped_by_agent_class[]? | select(.class == "green_check_suite_head_decided") | "\(.agent_id)=\(.skipped)"] | sort | join(",")' "$WORK/xtab-summary.json")|$(jq -r '[.skipped_by_agent_class[]? | keys] | flatten | unique | join(",")' "$WORK/xtab-summary.json")"

# ---------------------------------------------------------------------------
printf '\n%d passées, %d échouées\n\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]] || exit 1
