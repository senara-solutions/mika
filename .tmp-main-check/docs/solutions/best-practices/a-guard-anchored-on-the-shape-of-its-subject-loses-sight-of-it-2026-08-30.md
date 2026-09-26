---
title: "Une garde ancrée sur la forme de son sujet peut le perdre de vue en silence"
date: 2026-08-30
category: best-practices
module: skills/bundled/_shared
problem_type: best_practice
component: dispatch
severity: high
tags: [guards, test-design, anti-vacuity, dispatch-lib, positive-verification, claude-pilot]
applies_when:
  - Écrire une garde qui extrait quelque chose d'un fichier source par motif
  - Ajouter un plancher « anti-vacuité » à un test structurel
  - Choisir entre s'ancrer sur la forme d'une ligne et sur un point de passage obligé
  - Décider combien de réécritures du défaut doivent faire rougir une garde neuve
---

# Une garde ancrée sur la forme de son sujet peut le perdre de vue en silence

## Le contexte

mika#2043. `dispatch-lib.sh` construisait un drapeau `--trace` que `claude-pilot` n'a jamais accepté. Correctif : retirer le drapeau, et ajouter à `test-dispatch-lib.sh` une garde qui empêche `dispatch-lib` de construire un drapeau que le CLI refuse.

La garde extrayait les drapeaux des lignes correspondant à `claude-pilot[[:space:]]+-`, résolvait les variables interpolées, et comparait le tout à une liste blanche. Elle était verte, elle attrapait la forme littérale du défaut supprimé, et elle avait un plancher anti-vacuité : « au moins 4 drapeaux trouvés, sinon la garde ne regarde rien ».

Trois défauts distincts. Aucun n'a été trouvé en la relisant.

## Ce qui s'est cassé

### 1. Le sujet peut sortir du champ sans que rien ne change de couleur

Déplacer `$CWD_ARGS` devant le premier drapeau littéral — un réordonnancement d'arguments parfaitement anodin — fait que la ligne ne correspond plus à `claude-pilot[[:space:]]+-`. **L'invocation principale de dispatch sort entièrement du champ de la garde, en silence.** Les trois assertions restent vertes.

Le plancher anti-vacuité ne peut structurellement pas le voir : il compte les **drapeaux trouvés**, et l'invocation survivante en porte sept à elle seule. Le total ne baisse jamais. Un plancher global sur les objets trouvés ne détecte pas la perte d'un site d'observation.

### 2. Le motif d'ancrage encode une convention d'écriture, pas une propriété

`^[[:space:]]*(local[[:space:]]+)?VAR=` reconnaît l'assignation telle qu'elle était écrite. Mais `[ "${COND:-}" = "1" ] && VAR="--trace"` — la réécriture la plus naturelle des cinq lignes qu'on vient de supprimer — n'est pas en début de ligne. Ni `export`, ni `declare`, ni `+=`. Tous verts.

De même, `(^|[[:space:]])--flag` exige un espace avant le tiret, alors que le code écrit `CWD_ARGS="--cwd $DIR"`. La garde ratait `--cwd` **à l'état propre**, tout en affichant vert.

### 3. Sous `set -euo pipefail`, une garde qui perd son sujet ne rougit pas : elle disparaît

Un `grep` sans correspondance retourne 1 et tue la suite entière. En cassant le motif d'invocation exprès, la suite s'arrêtait en plein milieu — sans résumé, sans échec, sans ligne rouge. Muette exactement le jour où la forme de l'invocation change, c'est-à-dire le jour où la garde compte.

## La cause commune

Les trois viennent de la même erreur : **la garde s'ancrait sur la forme de son sujet plutôt que sur le point par lequel ce sujet doit passer.** La forme est une convention d'écriture — elle change au premier refactor, et son changement est indolore. Le point de passage est une propriété du système : ici, tout lancement réel traverse `_run_pilot_sandboxed`.

Une garde ancrée sur la forme a une propriété perverse : elle ne se trompe pas, elle *ne voit plus*. Un faux négatif franc est visible à l'usage ; un sujet disparu du champ ne produit aucun signal, et le vert qui reste ressemble à une confirmation.

## La correction

```sh
# Ancrer sur le lanceur, pas sur la forme de l'argv.
CP_INVOCATIONS=$(printf '%s\n' "$CP_JOINED" \
    | grep -E '^[[:space:]]*(_run_pilot_sandboxed[[:space:]]+claude-pilot|(if ! )?timeout[[:space:]]+[0-9]+[[:space:]]+claude-pilot)([[:space:]]|$)' || true)

# Compter les SITES observés, pas les objets trouvés.
CP_SITE_COUNT=$(printf '%s\n' "$CP_INVOCATIONS" | grep -c . || true)
[ "$CP_SITE_COUNT" -eq 3 ] || fail "expected 3 launch sites, saw $CP_SITE_COUNT"
```

Trois règles qui se généralisent :

1. **Ancrer sur le point de passage obligé**, pas sur la forme de la ligne. « Par où cela doit-il passer » survit au refactor ; « à quoi cela ressemble » non.
2. **Le plancher anti-vacuité compte les sites d'observation, pas les objets observés.** Sinon il est aveugle à la perte d'un site — le mode d'échec le plus silencieux.
3. **Sous `set -euo pipefail`, toute capture finit en `|| true`**, et c'est l'assertion de comptage qui transforme une capture vide en échec. Une garde doit rougir quand elle perd son sujet, jamais s'évanouir.

Corollaire sur le bruit : en élargissant l'ancrage des drapeaux, les formes courtes captaient du texte shell (`"${LOG_ID}-revise-$(date +%s)"` rend un « drapeau » `-revise-`). La réponse n'est pas de renoncer aux formes courtes — un `-X` inconnu abrège argparse tout autant — mais de **retirer les segments quotés avant d'extraire** : seule une interpolation non quotée peut se découper en mots. Le bruit vivait entre guillemets ; les charges utiles aussi (`ENTRY_COMMAND`, `PROMPT`), qui produisaient des faux positifs symétriques.

## La discipline qui a trouvé tout ça

Aucun des trois n'a été vu en relisant. Tous ont été vus en **réintroduisant le défaut et en exigeant du rouge**.

Et pas seulement sa forme littérale : la garde attrapait correctement `TRACE_FLAG="--trace"` tel qu'il était écrit. Ce sont les **réécritures plausibles** qui l'ont mise en défaut. La question à poser n'est donc pas « mon test échoue-t-il sur le bug ? » mais :

> *Si quelqu'un réintroduisait ce défaut demain, sans avoir lu ce test, sous quelle forme l'écrirait-il ?*

Puis écrire chacune de ces formes et exiger du rouge. Pour mika#2043, la matrice minimale a été : forme littérale, conditionnelle en une ligne, `export`, `declare`, `+=`, réordonnancement d'arguments, drapeau court, site de lancement renommé — plus deux cas qui doivent rester **verts** (charge utile quotée, état propre), parce qu'une garde qui crie au loup finit désarmée.

Limites assumées plutôt que contournées, écrites dans le fichier : une indirection à un niveau (`A="--trace"; B="$A"`) et un drapeau composé (`F="--$name"`) passent encore. Les fermer demanderait d'évaluer du shell, ce qui changerait la nature de la garde. Une limite connue et écrite vaut mieux qu'une limite ignorée.

## Voir aussi

- `a-guard-must-observe-not-assert-2026-08-29.md` — la garde doit observer la propriété, pas un proxy. Ici le proxy n'était pas la propriété mesurée mais **le champ d'observation** lui-même.
- `a-guard-must-sit-where-the-incident-actually-passed-2026-08-30.md` — même famille : se poser au point de passage, pas là où le défaut se raconte.
- `full-multi-agent-review-catches-p2-bugs-empirical-2026-08-20.md` (mika-platform) — les trois derniers défauts viennent d'une revue adversariale qui a exécuté la suite au lieu de la lire.
