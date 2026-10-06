---
title: Une ligne de démarrage doit nommer l'événement qu'elle consomme
date: 2026-10-06
category: best-practices
module: mika-agent/server
problem_type: best_practice
component: qa-review
severity: high
applies_when:
  - Un registre append-only sert à la fois à retenir des événements et à constater qu'ils ont été traités
  - Plusieurs événements d'une même clé portent une charge utile identique
  - Une retenue « pendante » est rejouée après un redémarrage ou une perte
tags:
  - debounce
  - audit-events
  - durabilite
  - qa-review
  - mika-2671
---

# Une ligne de démarrage doit nommer l'événement qu'elle consomme

## Contexte

mika#2671 phase B1 retient les `pull_request.synchronize` d'une PR pendant une
fenêtre d'anti-rebond, et rend la retenue durable en l'inscrivant au registre
`qa_pr_sync_observed` (ligne `stage=held`). La phase A y écrivait déjà une ligne
au démarrage de chaque tour de revue. La définition naturelle d'une retenue
« pendante » était donc : *la dernière ligne de sa clé est une retenue*. Un tour
démarré après la retenue la solde.

## Ce que la revue du plan a trouvé

C'est faux dès qu'un tour démarre en retard. Scénario : la fenêtre de A se
ferme et A part en file. A attend `agent_lock` derrière un autre tour. Pendant
ce temps B arrive, est retenu (`held(B)`) et ouvre une nouvelle fenêtre. Puis A
démarre enfin et écrit sa ligne de tour démarré, d'`id` plus grand que `held(B)`.
Pour la requête « dernière ligne de la clé », B est alors soldé. Un redémarrage
avant l'échéance de B perd la revue de la dernière tête, ce que la retenue
durable existait précisément pour empêcher.

La ligne de démarrage ne disait pas **quel** événement elle consommait. Et le
texte ne pouvait pas le dire : le gateway formate tous les `synchronize` d'une
même PR à l'identique (titre, branche, URL, sans SHA de tête).

## La règle

Dans un registre où une ligne solde une autre, la ligne qui solde porte
l'identité de l'événement soldé. Ici, `reasoning = request_id=<uuid>` (un UUID
par livraison du gateway) sur la retenue **et** sur le tour démarré. Une retenue
est pendante tant qu'aucune ligne plus récente de sa clé n'est :

- une retenue plus récente (une tête plus récente l'a remplacée) ;
- le tour démarré de la **même** requête.

Un tour démarré pour une autre requête ne solde rien. Le rejeu au démarrage
réutilise le `request_id` d'origine, sinon le tour rejoué ne solderait jamais
sa propre retenue.

## Les deux autres trous de la même revue

1. **La file bornée évince sans le dire.** Une retenue versée puis évincée par
   drop-oldest reste pendante. La garde de la phase A la compte et saute le
   callback, alors que la tête ne sera revue qu'au prochain redémarrage. Le
   remède est un balayage périodique des pendants. Il exclut toute clé retenue,
   en file, ou **en vol**, c'est-à-dire sortie de file mais pas encore démarrée :
   sans ce troisième état, le balayage rejoue un événement qui attend seulement
   le verrou.
2. **Une reprise ne remplace jamais.** Sur le chemin paresseux, le worker de
   drain démarre alors que le serveur reçoit déjà des requêtes. Une reprise qui
   ferait `insert` écraserait une retenue vivante, plus récente, par une tête
   périmée. D'où `hold_if_absent`.

## Le faux contrôle négatif

Le premier contrôle négatif d'AC1 (« deux `synchronize` séparés de plus que la
fenêtre ⇒ deux éléments en file ») était faux sans drain. Le premier est encore
en file à l'échéance du second, et la coalescence v2 les fusionne. Le test
aurait lu 1 pour une raison qui n'est pas la fenêtre. Un contrôle négatif doit
reproduire le consommateur de production, ici un `dequeue` entre les deux.

## Vérification

- `server::handlers::tests::mika2671_le_tour_dune_tete_perimee_ne_solde_pas_la_suivante`,
  vu rouge quand la condition « même requête » est retirée de
  `list_pending_audit_holds`.
- `…_le_balayage_ne_rejoue_que_les_orphelines`, vu rouge quand le terme
  « en vol » est retiré.
- `server::sync_debounce::tests::mika2671_la_reprise_ne_remplace_pas_une_retenue_vivante`,
  vu rouge quand `hold_if_absent` remplace.
