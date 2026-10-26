---
title: "Un registre écrit par un skill et lu par le moteur : le moteur compare deux faits attestés, il ne décide pas"
date: 2026-10-26
category: architecture-patterns
module: crates/mika-agent/src/server
problem_type: architecture_pattern
component: agent-core
severity: medium
applies_when:
  - "Le moteur doit écarter un tour LLM sur la foi d'une décision que seul un skill sait prendre"
  - "Un nouveau terme de PrefilterState lit une ligne d'audit_events écrite par un outil du modèle"
  - "Une propriété de périmètre (agent_id, tenant) doit être attestée par un test, et non par la discipline de l'appelant"
  - "Ajouter une classe de pré-filtre à côté d'une classe existante qui écarte déjà une partie de la même population"
tags: [webhooks, prefilter, llm-cost, audit-events, sole-writer, agent-id, kill-switch, check-suite, mika-2917, mika-2680]
related_components: [crates/mika-agent/src/qa_reviewed_head.rs, crates/mika-agent/src/async_db.rs]
---

# Un registre écrit par un skill et lu par le moteur : le moteur compare deux faits attestés, il ne décide pas

## Contexte

Mesuré le 2026-10-26 (mika#2680) : sur 4,9 jours, 51 tours `check_suite` de branche de PR se terminaient en `stop_verdict_recorded` (35) ou `stop_already_reviewed_sha` (16), pour 7,06 M tokens d'entrée. Chacun payait un tour complet pour conclure qu'il n'y avait rien à faire, parce que la tête de la PR avait déjà été revue.

Savoir si « cette tête est revue » relève du skill `qa-review-webhook-success`. Le bearing de mika#2680 refusait deux raccourcis : un appel `gh` depuis `webhook_prefilter.rs` (mika#2260), et toute copie de la logique de dédoublonnage du skill dans le Rust. mika#2917 a donc livré la classe (g′) en trois vols : le texte du `check_suite` porte la tête (vol 1, #2918), le skill inscrit ce qu'il a conclu (vol 2, #2919), et le pré-filtre compare les deux (vol 3, PR #2920, ouverte en brouillon et non mergée à la date de ce document).

Ce document consigne le patron du vol 3. Les deux autres vols ont leurs plans dans `docs/plans/2026-10-26-00{1,2}-…`.

## Le patron

### 1. La décision reste dans le skill, et le moteur compare deux faits

Le moteur ne calcule jamais « cette tête est revue ». Il lit deux faits qu'un autre a attestés :

- **la tête nommée par le texte** : `parse_check_suite_heads` → `CheckSuiteHeads::One { pr, head_sha }` ;
- **la ligne du seul écrivain** : `audit_events` sous `tool_name = QA_REVIEWED_HEAD_TOOL`, `target_key = head_key(repo, pr, head_sha)` (`pr:{repo}#{n}@{sha}`), écrite par `crates/mika-agent/src/tools/record_reviewed_head.rs` et par lui seul.

Le moteur écarte le tour seulement si la clé construite depuis le texte trouve une ligne dont l'`after_value` passe `reviewed_head_outcome`, c'est-à-dire l'un des trois littéraux de `REVIEWED_HEAD_OUTCOMES`. La clé, l'issue canonique et le nom de l'outil viennent du module du registre (`qa_reviewed_head.rs`) et ne sont jamais retapés. Une garde du vol 2 interdit d'ailleurs de retaper la grammaire de clé.

Ce que cette forme achète, c'est que **le contrôle négatif est structurel**. Un nouveau push produit un nouveau `head_sha`, donc une clé que le registre ne porte pas, donc un tour qui a lieu. Aucune convention ni aucun calcul de fraîcheur n'entre en jeu, seulement l'égalité de deux clés. Le corpus d'AC1 (`mika2917_vol3_le_corpus_rejoue`) matérialise ce cas en écrivant la ligne sur l'ancienne tête et en servant un texte qui porte la nouvelle.

Le registre n'a demandé aucune table et aucune migration. C'est une ligne d'`audit_events` sous un `tool_name` dédié, sur le même modèle que `qa_head_supersession::SYNC_OBSERVED_TOOL`. Le lecteur est un accesseur qui existait déjà, `AsyncDatabase::latest_audit_event_for_target` (mika#2242). C'est surtout ce choix qui a gardé le vol 3 petit.

Deux gardes de `qa_reviewed_head.rs` tiennent le « un écrivain, un lecteur » :

- `mika2917_a2_un_seul_ecrivain_et_au_plus_un_lecteur` est resserrée au vol 3 de `readers.len() <= 1` à l'exact-set `["server/webhook_prefilter.rs"]` ;
- `mika2917_a1_les_sites_gh_du_prefiltre_sont_recenses` laisse `GH_CALL_SITES_ALLOWED` sans aucune entrée nouvelle.

### 2. Le périmètre `agent_id` est tenu par la signature, et mesuré sur une vraie base

L'audit MPC de #2919 demandait qu'une ligne écrite par mika-qa n'écarte jamais un tour de mika-dev. Le vol 3 n'ajoute **aucun** filtre pour cela. La propriété est déjà dans la signature de l'accesseur : `latest_audit_event_for_target(tool_name, target_key, since)` clone `self.agent_id` du handle et le passe comme premier paramètre SQL (`async_db.rs:838`). L'appelant ne peut pas fournir l'`agent_id`, donc il ne peut pas le fournir de travers. L'écriture suit la même règle : `log_audit_event` ne prend pas d'`agent_id` en argument.

Une propriété tenue par la structure se teste quand même, mais pas avec un bouchon. `Stub::reviewed_head` rend ce que le test lui dicte : il atteste le décideur, jamais le périmètre. Le test `qa_reviewed_head::tests::mika2917_vol3_une_ligne_de_mika_qa_necarte_pas_le_tour_de_mika_dev` mesure la propriété avec :

- **une seule base** (`Database::open_in_memory`) et **deux handles** obtenus par `with_agent`, qui clone en partageant l'`Arc` interne (`async_db.rs:122`). Deux bases en mémoire rendraient la propriété intestable : l'isolation observée viendrait des deux fichiers, et le test resterait vert même sans aucun filtrage (motif mika#1926) ;
- **les deux agents enregistrés** (`register_agent`), parce que `audit_events.agent_id` est une clé étrangère et qu'un refus de la FK ferait rougir le test pour une raison qui n'est pas la sienne ;
- **un contrôle positif dans le même test** : le handle mika-qa relit la même clé et obtient `Some("review")`. Sans ce contrôle, « mika-dev ne voit rien » ne se distingue pas de « rien n'a été écrit » ni de « la fenêtre ne couvre pas la ligne » (classe mika#2205).

### 3. Le repli « tente (g′), consulte (b) entière, nomme la relâche »

Une classe (b), `GreenCheckSuiteHeadDecided`, écartait déjà une partie de la même population sur une autre preuve, la redécision DECISION-CORE du même événement. Le danger était qu'une classe plus précise **retire** des écarts à (b). `decide_green_pr_branch` n'a donc qu'une forme, factorisée dans `head_decided_or` :

| le texte dit | résultat |
|---|---|
| `One`, interrupteur armé, issue canonique au registre | `Skip { GreenCheckSuiteHeadAlreadyReviewed }` |
| `One`, registre muet | (b) entière, sinon relâche `reviewed_head_absent` |
| `One`, valeur hors des trois littéraux | (b) entière, sinon `reviewed_head_out_of_contract` |
| `One`, lecture en erreur | WARN, puis (b) entière, sinon `state_unreadable` |
| `One`, interrupteur désarmé | (b) entière, sinon `reviewed_head_switch_disarmed` |
| `LineAbsent`, `Empty`, `Many` | (b) entière, avec ses propres relâches, comme avant |

Trois conséquences, chacune voulue :

- **Aucun écart de (b) n'est perdu**, sur aucune entrée et quel que soit l'état de l'interrupteur. C'est un test dédié, `mika2917_vol3_aucun_ecart_de_b_nest_perdu`, et non un corollaire.
- **L'interrupteur est sans danger.** `MIKA_WEBHOOK_PREFILTER_REVIEWED_HEAD=off` rend exactement le régime de `main` avant le vol, pas un troisième régime que personne n'a mesuré (`mika2917_vol3_linterrupteur_desarme_rend_le_regime_anterieur`). C'est aussi ce qui justifie que l'absence de variable vaille `on`, comme pour `MIKA_WEBHOOK_PREFILTER` et à l'inverse de `MIKA_WEBHOOK_COMMENT_TRIAGE` : tous les échecs de (g′) rendent le tour, donc une erreur coûte un tour payé, celui d'aujourd'hui. L'interrupteur est lu **dans le décideur** et non dans `skip_turn`, parce que désarmer (g′) doit laisser (b) décider.
- **Chaque relâche a un nom, y compris celles qui doivent rester à zéro.** `reviewed_head_out_of_contract` signale un défaut de l'écrivain, et `reviewed_head_switch_disarmed` sert de contrôle positif au désarmement. Sans ces deux noms, « l'interrupteur est baissé » et « la classe ne mord sur rien » rendraient les mêmes octets.

L'erreur de lecture demandait une petite extraction. `state_unreadable` rend la `Decision` immédiatement, or (g′) doit émettre le WARN **puis** replier. Le vol extrait donc `warn_state_unreadable`, qui reste le seul site du WARN, et les cinq appelants existants sont inchangés.

## Ce qu'il faudra lire après le déploiement (AC4)

**Un transfert n'est pas une régression.** Sur la population `One`, `skipped_by_class.green_check_suite_head_decided` va baisser au profit de `green_check_suite_head_already_reviewed`. La grandeur à comparer avant et après est la **somme** des deux classes, jamais l'une seule. Il en va de même entre les relâches `no_decision_attested` / `head_not_previously_decided` et `reviewed_head_absent`.

**Un résidu voulu, à nommer.** Sur les 5 derniers jours, les `check_suite` verts hors `main` représentent **12 tours mika-qa et 3 tours mika-dev**. mika-dev n'a pas le skill qui écrit le registre. Par le périmètre `agent_id` du § 2, (g′) ne peut donc pas écarter ses tours. C'est voulu, puisque c'est le même périmètre qui empêche un agent d'écarter le tour d'un autre. Ces 3 tours sont le résidu que la mesure d'AC4 devra nommer, et non une panne de la classe à diagnostiquer. Les écarter demanderait un écrivain côté mika-dev, ce qui est une autre décision.

**L'ordre de la sonde.** On atteste d'abord le binaire (`.manifest-writer`), puis le prompt déployé par `diff` (une mise à jour de skill peut ne rien faire, mika#2340), puis l'écrivain (une ligne `qa_reviewed_head` après un `check_suite` réel), et seulement ensuite le lecteur. Sans ligne au registre, (g′) n'a rien à comparer, et un `skipped_by_class` sans la nouvelle classe ne dirait pas lequel des vols est en cause. La procédure complète est dans le plan du vol 3, § Sonde post-déploiement.

## Quand reprendre ce patron

Dès qu'un tour LLM conclut « rien à faire » sur une décision qui appartient au modèle ou à un skill :

1. faire écrire au skill une ligne d'`audit_events` sous un `tool_name` dédié, avec un seul écrivain et une clé fournie par une fonction partagée ;
2. faire porter au texte de l'événement la moitié de la clé que le moteur doit comparer ;
3. lire par un accesseur dont la signature porte le périmètre, et tester ce périmètre sur une vraie base, avec deux handles et un contrôle positif ;
4. si une classe existante couvre déjà une partie de la population, replier sur elle **entière** et nommer la relâche, afin que l'interrupteur de la nouvelle classe rende l'ancien régime.

Voir aussi `docs/solutions/best-practices/une-decision-prise-en-code-laisse-une-trace-que-la-porte-lit-2026-10-14.md`, la même idée quand c'est un handler déterministe, et non un skill, qui écrit la trace, et `docs/solutions/best-practices/un-pre-filtre-de-webhook-se-place-apres-les-handlers-2026-10-07.md` pour l'emplacement de la porte.
