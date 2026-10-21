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
# Le § 5 a été ajouté par le vol 2/2 de mika#2855 pour le SECOND TERME du
# registre — la ligne de sortie, jointe par `request_id`. Ses corpus vivent
# sous `tests/fixtures/mika2855/`, et pas parmi les six de `mika2680/` : ceux-là
# sont gelés et pinnés par les § 1 à 4, et un septième déposé au milieu
# inviterait la prochaine édition à l'étendre plutôt qu'à en ajouter un.
#
# LA DISCIPLINE ANNONCÉE : 4 POSITIFS + 16 NÉGATIFS, et le compte au pied de la
# sortie doit le dire. Un compte annoncé qui ne correspond plus est la première
# chose qu'un relecteur croit, et la dernière qu'il vérifie.
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

# Les corpus du vol 2/2 de mika#2855 vivent sous leur propre répertoire : les
# six corpus de `mika2680/` sont gelés et pinnés par les § 1 à 4, et un corpus
# du second terme déposé parmi eux inviterait la prochaine édition à l'étendre
# au lieu d'en ajouter un. Un répertoire par vol, une variable par répertoire.
FIXTURES_2855="$REPO_ROOT/crates/mika-agent/tests/fixtures/mika2855"
OUTCOME="$FIXTURES_2855/outcome_corpus.jsonl"
OUTCOME_ABSENT="$FIXTURES_2855/outcome_absent_corpus.jsonl"
OUTCOME_ONLY="$FIXTURES_2855/outcome_only_corpus.jsonl"

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
printf -- '\n-- 5. le second terme du registre : ce que le tour a FAIT\n'
# ---------------------------------------------------------------------------
# Le § 5 a été ajouté par le vol 2/2 de mika#2855 pour la jointure du terme de
# sortie. Il copie la structure du § 4 — un corpus principal construit pour que
# les seaux à trois clés soient AMBIGUS et le seau à quatre ne le soit pas — et
# en sort pour les deux confusions que ce corpus-là ne peut pas porter :
# `outcome_absent` contre `outcome_line_absent` (N12) demande deux tours
# IDENTIQUES sur les trois autres clés, et « des lignes de sortie seules »
# (N16) demande un corpus sans aucune ligne d'entrée. Une entrée par confusion
# (leçon mika#2277), jamais un corpus qui les neutraliserait toutes d'un coup.

MEASURE --log-file "$OUTCOME" --out "$WORK/outcome-turns.jsonl" \
        --summary "$WORK/outcome-summary.json" >/dev/null 2>&1

# La cellule du croisé à QUATRE clés. Copie de `cellule()` ci-dessus, avec la
# clé d'issue en plus et le fichier de résumé en premier argument : le § 5 lit
# trois résumés différents (le corpus principal, celui des absences, celui que
# `--agent` filtre), là où le § 4 n'en lit qu'un. Même convention de diagnostic
# — « cellule absente » plutôt qu'un champ vide, parce qu'un croisement resté à
# trois clés ne rend pas une cellule fausse : il n'en rend AUCUNE.
cellule_issue() {
    jq -r --arg a "$2" --arg t "$3" --arg r "$4" --arg o "$5" '
        [.by_agent_trigger_release_outcome[]?
         | select(.agent_id == $a and .trigger == $t
                  and .release == $r and .outcome == $o)]
        | if length == 0 then "cellule absente"
          else "\(.[0].turns):\(.[0].input_tokens):\(.[0].cache_read_tokens):\(.[0].llm_calls)"
          end' "$1"
}

# P4 — le positif du vol, et il porte LA RÉSERVE DE PRIME (AC3). Les deux
# cellules sont du même agent et du même déclencheur ; ce qui les sépare est
# qu'un tour a été relâché par une panne de lecture (`state_unreadable`) et
# l'autre arrêté proprement sur un SHA déjà revu. Avant ce vol, la seconde
# moitié de cette phrase n'était pas dans l'agrégat : le registre disait que le
# tour avait eu lieu, jamais ce qu'il avait fait, et le filtrage de (g′) se
# décide exactement là.
#
# Comme P3, les deux cellules portent le MÊME compte de tours (1) pour des
# coûts de 900 et 120 : un croisement qui ne porterait que `turns` — la forme
# la plus naturelle à écrire, copiée du seau plat — est indistinguable d'un
# croisement correct sur un contrôle de comptes, et devient rouge ici.
eq "P4 — state_unreadable se distingue d'un arrêt propre, en tokens" \
   "1:900:150:3|1:120:0:2" \
   "$(cellule_issue "$WORK/outcome-summary.json" mika-qa check_suite_pr_branch state_unreadable review)|$(cellule_issue "$WORK/outcome-summary.json" mika-qa check_suite_pr_branch head_not_previously_decided stop_already_reviewed_sha)"

# N11 — QUATRE CLÉS, PAS TROIS. `req-o-qa-sha` et `req-o-qa-nopr` sont
# identiques sur l'agent, le déclencheur ET le terme de relâche : seule l'issue
# les sépare. Un croisement resté sur `[agent, trigger, release]` ne rendrait
# donc pas une cellule absente — il rendrait UNE cellule de 2 tours / 420
# tokens, un chiffre juste pour une question qu'on n'a pas posée. L'assertion
# tient les deux moitiés : les deux cellules existent séparément, et aucune
# cellule du croisé ne porte la somme des deux.
eq "N11 — deux issues sous un même release font deux cellules, jamais leur somme" \
   "1:120:0:2|1:300:20:2|false" \
   "$(cellule_issue "$WORK/outcome-summary.json" mika-qa check_suite_pr_branch head_not_previously_decided stop_already_reviewed_sha)|$(cellule_issue "$WORK/outcome-summary.json" mika-qa check_suite_pr_branch head_not_previously_decided stop_no_open_pr)|$(jq -r '[.by_agent_trigger_release_outcome[]?.input_tokens] | any(. == 420)' "$WORK/outcome-summary.json")"

# N12 — LES DEUX ABSENCES NE SONT PAS LA MÊME, et rien dans le corpus principal
# ne peut le montrer : il faut deux tours identiques sur les trois autres clés,
# dont l'un a une ligne de sortie où le modèle n'a rien attesté
# (`outcome_absent`, une valeur du contrat émise par le MOTEUR) et l'autre
# aucune ligne du tout (`outcome_line_absent`, un régime du déploiement —
# motif mika#2340, un binaire d'avant le vol 1 n'en émet aucune pour TOUS ses
# tours). Les confondre ferait lire « le modèle n'atteste presque jamais » sur
# une fenêtre à cheval sur un déploiement.
#
# Quatre moitiés dans une seule assertion, parce qu'elles tiennent une seule
# propriété : le repli est dans la clé de groupement ET dans la projection, et
# la ligne per-tour ne porte PAS le nom du seau. Posé dans la clé seule, le
# seau aurait le bon compte sous un nom `null` ; posé sur la ligne per-tour,
# un tour sans ligne de sortie serait indiscernable d'un tour dont le modèle
# aurait attesté ce mot (motif mika#2331).
MEASURE --log-file "$OUTCOME_ABSENT" --out "$WORK/absent-turns.jsonl" \
        --summary "$WORK/absent-summary.json" >/dev/null 2>&1
eq "N12 — outcome_absent (le moteur) n'est pas outcome_line_absent (le journal)" \
   '{"outcome_absent":1,"outcome_line_absent":1}|1:200:0:1|false|outcome_absent/null' \
   "$(jq -cS '.charged_by_outcome' "$WORK/absent-summary.json")|$(cellule_issue "$WORK/absent-summary.json" mika-qa check_suite_pr_branch head_not_previously_decided outcome_line_absent)|$(jq -r '[.by_agent_trigger_release_outcome[]?.outcome] | any(. == null)' "$WORK/absent-summary.json")|$(champ "$WORK/absent-turns.jsonl" req-a-attested outcome)/$(champ "$WORK/absent-turns.jsonl" req-a-noline outcome)"

# N13 — CROISER N'EST PAS REMPLACER, sur le corpus neuf. Les trois seaux
# antérieurs répondent à des questions qui restent posées : « combien cette
# porte a-t-elle coûté dans cette cellule » n'a pas besoin du grain de l'issue,
# et le seau à quatre clés éclate la réponse en autant de lignes que le modèle
# a d'issues. La cellule de 2 tours / 420 tokens que N11 refuse de voir sur le
# croisé doit donc être EXACTEMENT ce que `by_agent_trigger_release` rend ici :
# les deux assertions se lisent ensemble, et une fusion déguisée en ajout les
# rendrait contradictoires.
eq "N13 — by_agent_trigger, by_agent_trigger_release et charged_by_release sont inchangés" \
   'mika-qa/check_suite_pr_branch=3:1320:170:7 mika-dev/check_suite_pr_branch=1:60:0:1|mika-qa/state_unreadable=1:900 mika-qa/head_not_previously_decided=2:420 mika-dev/head_not_previously_decided=1:60|{"head_not_previously_decided":3,"state_unreadable":1}' \
   "$(jq -r '[.by_agent_trigger[] | "\(.agent_id)/\(.trigger)=\(.turns):\(.input_tokens):\(.cache_read_tokens):\(.llm_calls)"] | join(" ")' "$WORK/outcome-summary.json")|$(jq -r '[.by_agent_trigger_release[] | "\(.agent_id)/\(.release)=\(.turns):\(.input_tokens)"] | join(" ")' "$WORK/outcome-summary.json")|$(jq -cS '.charged_by_release' "$WORK/outcome-summary.json")"

# N14 — LES TROIS COMPTES DE LIGNE, ET L'ÉTANCHÉITÉ DE L'AXE. Le corpus porte
# cinq lignes de sortie : trois qui se joignent, une sans `request_id`
# (inutilisable — elle quitte la jointure mais reste COMPTÉE, sans quoi une
# ligne qui disparaît sans trace ferait lire « le modèle n'a pas attesté » là
# où la vérité est « le lecteur n'a pas su lire ») et une orpheline, dont le
# `request_id` n'a aucun tour payé.
#
# L'orpheline porte `stop_out_of_scope` — délibérément la seule valeur du
# contrat qu'aucun tour payé ne porte. Une fuite de la jointure (un `//` mal
# placé, une orpheline recollée au mauvais tour) ne se verrait pas sur des
# comptes, qui resteraient justes : elle se voit ici, par PRÉSENCE DE CLÉ dans
# `charged_by_outcome`.
eq "N14 — cinq lignes, une inutilisable, une orpheline, et aucune fuite dans l'axe" \
   "5|1|1|false" \
   "$(jq -r '.outcome_lines' "$WORK/outcome-summary.json")|$(jq -r '.outcome_lines_unusable' "$WORK/outcome-summary.json")|$(jq -r '.outcome_lines_orphan' "$WORK/outcome-summary.json")|$(jq -r '.charged_by_outcome | has("stop_out_of_scope")' "$WORK/outcome-summary.json")"

# N15 — `agent_id` SUR LA LIGNE DE SORTIE EST UNE CONDITION DE CORRECTION, pas
# un ornement. Le filtre `--agent` du passage en flux s'applique à CHAQUE ligne
# du corpus, pas aux seules lignes d'entrée : un émetteur qui omettrait ce
# champ ferait tomber toutes les lignes de sortie au filtre, et toute mesure
# ciblée — c'est-à-dire toute mesure de (g′), qui porte sur mika-qa — rendrait
# un `outcome_line_absent` universel. Vert sur les comptes, et faux.
MEASURE --log-file "$OUTCOME" --agent mika-qa --out "$WORK/qa-turns.jsonl" \
        --summary "$WORK/qa-summary.json" >/dev/null 2>&1
eq "N15 — sous --agent, les trois cellules attestées gardent leur issue" \
   "state_unreadable/review=900 head_not_previously_decided/stop_no_open_pr=300 head_not_previously_decided/stop_already_reviewed_sha=120|false" \
   "$(jq -r '[.by_agent_trigger_release_outcome[] | "\(.release)/\(.outcome)=\(.input_tokens)"] | join(" ")' "$WORK/qa-summary.json")|$(jq -r '[.by_agent_trigger_release_outcome[]?.outcome] | any(. == "outcome_line_absent")' "$WORK/qa-summary.json")"

# N16 — UNE LIGNE DE SORTIE N'EST PAS UNE LIGNE D'ENTRÉE. « Rien mesuré » reste
# décidé sur `webhook_turn_charged` SEUL : un corpus qui ne porte que des
# lignes de sortie n'a aucun tour payé, donc rien à chiffrer. Les laisser
# conjuguer la décision rendrait `exit 0` sur un agrégat à zéro tour, qui se
# lirait comme « la fenêtre n'a rien coûté » — un scan inerte en silence qui se
# lit comme un scan qui n'a rien trouvé (classe mika#2205), la confusion même
# que N4 tient sur l'autre moitié du registre.
MEASURE --log-file "$OUTCOME_ONLY" >/dev/null 2>&1
eq "N16 — un corpus de lignes de sortie seules sort 2" "2" "$?"

# ---------------------------------------------------------------------------
printf '\n%d passées, %d échouées\n\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]] || exit 1
