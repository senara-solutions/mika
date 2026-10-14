---
title: Une décision « rien à faire » prise en code laisse une trace que la porte lit, au lieu d'être re-dérivée
date: 2026-10-14
category: best-practices
module: crates/mika-agent/src/server
problem_type: best_practice
component: agent-core
severity: medium
tags: [webhooks, llm-cost, prefilter, ci-failure, github-actions, mika-2683]
applies_when:
  - "Un handler déterministe décide qu'il n'y a rien à faire, mais le tour LLM a lieu quand même"
  - "Étendre le pré-filtre webhook à une nouvelle classe d'événements"
  - "Un workflow GitHub Actions échoue à chaque exécution sans que la cause soit lisible"
---

# Une décision « rien à faire » prise en code laisse une trace que la porte lit

## Problème

Le CI failure handler (`server/ci_failure_handler.rs`) tranchait déjà en code plusieurs cas : CI rouge sur `main` (prévention de boucle), aucune PR ouverte pour la branche, tâche hors `in_progress`, PR sans tâche signalée pour adoption. Il rendait pourtant `VerdictAction::Passthrough`, et le tour LLM avait lieu pour conclure la même chose : « no action ». Mesuré : 116 k tokens d'entrée pour une CI rouge sur `main`, 55 k pour une PR dependabot en échec.

La cause qui revenait chaque lundi : `eval-calibration.yml` passait `--ignored` et `--include-ignored` à la même commande `cargo test`. libtest refuse la paire, `|| true` avalait le refus, et c'est l'étape suivante qui échouait, faute d'artefact. 17 échecs sur 17 exécutions, dont aucun ne nommait la cause.

## Solution

**Le handler écrit, la porte lit.** Chaque sortie terminale « rien à faire » du handler écrit une ligne d'audit `ci_failure_no_action` avec son motif (`no_open_pr`, `task_not_in_progress`). Le pré-filtre ne re-dérive pas la décision. Il lit les lignes d'audit de *cet* événement et écarte le tour seulement s'il y trouve l'une de ces traces.

Trois règles rendent ce schéma sûr :

- **Seule l'absence de ligne garde le tour.** Les sorties qui doivent informer le modèle (jeton absent, état de la forge illisible, correctif en vol, escalade) n'écrivent délibérément aucune ligne `ci_failure_no_action`.
- **Une action annule tout.** Si l'événement porte une ligne `ci_failure_handled` ou `ci_failure_escalated`, la porte rend le tour, quelle que soit la ligne « rien à faire » à côté.
- **Un nom, un écrivain, une constante.** Les noms d'outil et la clé `check_suite:{repo}@{branch}` sont des constantes et une fonction (`branch_key`) partagées par les deux fichiers. `webhook_queue_v2` construit une clé qui ne diffère que d'un caractère (`:` au lieu de `@`) : une clé retapée à la main aurait divergé sans bruit.

**Un handler qui enrichit le texte demande sa propre fenêtre.** `surface_for_adoption` réécrit `req.text`. Le test « texte touché par un handler » utilisé jusque-là serait donc vrai pour tout événement signalé, et la classe ne mordrait jamais. La porte compare le texte juste avant et juste après l'appel du CI failure handler (`outside_ci_failure_handler`), comme elle le fait déjà autour du CI success handler.

**Le workflow.** `--ignored` seul (la matrice de calibration, c'est-à-dire ce que `#[ignore]` marque), sans `|| true`. Un lint CI (`scripts/check-eval-ignored-flags.sh`) refuse la paire de flags *au même site d'invocation*, jamais la présence d'un seul : chacun est légitime seul, et une règle qui accuse une invocation correcte finit désarmée. Son test négatif montre qu'il rougit sur une fixture fautive.

## Le coût caché d'un correctif

Corriger le flag faisait passer le cron de « échoue avant tout appel » à « appelle quatre vrais fournisseurs chaque lundi » (~2,57 $ par exécution). Une dépense récurrente est une décision d'opérateur, et une session dispatchée ne peut pas poser la question. Le `schedule` est donc retiré, `workflow_dispatch` reste, et le commentaire en tête du workflow donne les trois lignes à remettre une fois le budget décidé sur mika#2683.

Quand un correctif rend effectif quelque chose qui échouait avant de rien coûter, chiffrer ce que la réussite coûtera avant de la laisser partir en cron.

## Pièges

- `|| true` sur une commande de test masque la cause et déplace l'échec à l'étape suivante. Ne pas l'ajouter ; s'il faut tolérer un échec, le nommer.
- Une trace lue par la porte ne vaut que pour l'événement courant. La lire par `trace_id`, jamais « la dernière ligne de cette branche », qui peut venir d'un autre événement.
- Une valeur de motif inconnue de la porte rend `None` : une valeur ajoutée en amont n'écarte aucun tour tant que la porte ne la connaît pas.
