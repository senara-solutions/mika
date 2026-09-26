---
title: "Vérifier un créneau n'est pas le réclamer : la fenêtre entre le SELECT et la ligne qui prouve"
date: 2026-08-30
category: best-practices
module: agent-core
problem_type: best_practice
component: dispatch
severity: high
applies_when:
  - Sérialiser des dispatchers concurrents sur une ressource partagée
  - Un ticket parle d'« arbitrage » alors que le code ne fait qu'un contrôle
  - Choisir entre observabilité et politique sur une garde existante
  - Poser un verrou fail-closed sans arrêter une boucle autonome
---

# Vérifier un créneau n'est pas le réclamer

## Le contexte

mika#1948, Porte 2. Trois dispatchers se partagent les créneaux d'exécution :
la boucle autonome, le manager de milestones, et l'opérateur. Le ticket demandait
un arbitrage. Le dépôt croyait en avoir un depuis mika#583 et mika#1001.

## Leçon 1 — Un contrôle et une réclamation ne se distinguent pas à la lecture

`has_active_callback_tasks_excluding` a toutes les apparences d'une garde de
concurrence : elle est appelée « slot guard », elle refuse, elle a des tests, et
elle a survécu à deux tickets de raffinement. C'est un `SELECT` nu.

```rust
Ok(None) => { /* No conflicting dispatch in this class — proceed */ }
```

La ligne qui rend le créneau *observablement tenu* — la tâche callback — est
écrite bien plus tard, par l'appelant. Rien, entre les deux, n'appartient au
premier arrivant.

Le discriminant tient en une question : **quelle écriture, exactement, empêche un
second appelant de passer ?** Si la réponse est « aucune, on relit juste le même
état », c'est un contrôle. S'il n'y a pas d'écriture, il n'y a pas de
réclamation, et deux appelants peuvent croire simultanément qu'ils ont la
ressource. Un état que deux prétendants peuvent croire tenir n'est pas un
arbitrage, c'est une convention.

## Leçon 2 — Mesurer la fenêtre, pas la supposer

La tentation est de classer ça en « course théorique, fenêtre de quelques
microsecondes ». Ici la mesure dit autre chose. Entre le contrôle et l'écriture,
`validate_dispatch_readiness` fait :

- `fetch_issue_body` (timeout 10 s),
- le contrôle de PR ouverte (réseau),
- la garde de marqueurs de grooming (réseau).

Et **quatre** appelants de production y entrent : `ready_label_handler`, la
frontière d'outil, `task_engine::dispatcher`, `verdict_handler`.

La fenêtre se compte en secondes, avec quatre portes. Ce n'est pas une course
théorique, c'est la forme exacte de l'incident du 2026-08-30 (deux écrivains sur
la branche de mika#2055).

**À faire** : avant de qualifier une fenêtre TOCTOU de négligeable, lister ce qui
s'exécute *dedans*. Un appel réseau dans la fenêtre la fait changer d'ordre de
grandeur, et `grep -c` sur les appelants dit combien de portes elle a.

## Leçon 3 — Placer la réclamation en DERNIER supprime tous les chemins de libération

La position naturelle d'un verrou est là où le contrôle existait déjà. C'est un
piège : chaque étape faillible qui suit devient un chemin où il faut penser à
relâcher, et il suffit d'en oublier un pour geler la ressource.

En posant la réclamation comme **dernière** action de la validation, la question
disparaît : aucun refus ne survient après elle, donc aucun chemin d'erreur n'a
besoin de la relâcher. La seule chose entre la réclamation et l'écriture durable
est le dispatch lui-même.

Corollaire utile : ça ne réduit pas la protection. Deux dispatchers passent le
contrôle, dépensent tous les deux leurs secondes de réseau, puis se disputent la
réclamation — et exactement un gagne. Le point d'arbitrage est atomique, peu
importe combien de monde est arrivé jusque-là.

## Leçon 4 — Fail-closed a besoin d'une date de péremption pour ne pas casser la boucle

« Dans le doute, on ne dispatche pas » est correct et suffit à geler une classe
indéfiniment si le détenteur meurt. Le TTL est ce qui rend la règle tenable : un
dispatcher mort bloque sa classe **un TTL**, pas pour toujours.

Le dimensionner demande deux bornes, pas une :

- il doit **dépasser** la fenêtre qu'il garde (validation finie → ligne callback
  écrite, c'est-à-dire un lancement de processus) ;
- il doit rester **très en dessous** de la durée d'un vrai dispatch (des
  minutes), pour qu'un bail ne survive jamais au travail qu'il protégeait et ne
  bloque pas un créneau réellement libéré.

Un TTL choisi sans la seconde borne transforme le correctif en perte de débit.

## Leçon 5 — Fail-closed et fail-open dans le même correctif, et pourquoi

Deux contrôles voisins de ce même correctif ont des dispositions opposées, et
c'est délibéré :

| Contrôle | Erreur → | Pourquoi |
|---|---|---|
| Le créneau est-il occupé / puis-je le réclamer | **Refus** | C'est l'information dont dépend la sécurité. Ne pas savoir qui tient le créneau n'est pas une autorisation. |
| L'opérateur a-t-il du travail prioritaire en attente | **Passage** | C'est une préférence. Fail-closed y échouerait les wrappers différés et coûterait à la boucle son chemin de reprise. |

Le réflexe « fail-closed partout » est le mode d'échec dominant de ce genre de
garde parce qu'il se déguise en prudence. Le tri se fait sur une question :
est-ce que cette information protège une invariante, ou exprime un ordre de
passage ?

## Leçon 6 — Un plan groomé vieillit, et l'axe qu'il propose peut déjà exister sous un autre nom

Le plan datait de huit jours et avait passé deux revues d'architecte. Trois de
ses ancrages avaient bougé : le numéro de schéma était pris, et mika#2084 avait
entre-temps introduit une notion de « siège » qui *ressemble* à ce que le plan
appelait `dispatcher_source`.

La ressemblance était le vrai piège. Siège (`loop|ssc|mpc`) et source
(`mika_dev|mika_manager|operator`) répondent à deux questions différentes : quel
*moteur* possède un TICKET, versus quel *rôle dans un moteur* a demandé une
TÂCHE. Les fusionner aurait produit une colonne qui ment dans les deux sens.

**À faire** : à la reprise d'un plan groomé, relire d'abord ce qui a été mergé
depuis, et pour chaque concept proposé demander « existe-t-il déjà sous un autre
nom, et si oui répond-il à la même question ? ». Deux axes qui se ressemblent se
distinguent par la question, pas par le vocabulaire.

## Leçon 7 — La mutation dit si les tests mesurent la garde ou la décrivent

Cassée dans les deux sens, en comptant ce qui rougit :

- Réclamation neutralisée (réussit toujours) : **2 tests rougissent**, les deux
  tests de refus. Le refus est bien mesuré.
- Réclamation élargie (refuse toujours) : **26 tests rougissent**, dont 24
  écrits des mois plus tôt pour d'autres raisons — gardes `blocked_by`, gardes de
  PR ouverte, compteur de dispatch par tour.

Le second chiffre est le plus informatif, et reproduit ce que mika#2084 avait
déjà observé : la suite existante est le vrai filet anti-sur-refus, parce qu'elle
exerce le chemin nominal. Un verrou trop zélé se fait attraper par les tests des
autres, pas par les siens.

Vaut aussi pour les gardes textuelles : le verrou `FORBIDDEN_TOKENS` a été
vérifié en plantant une violation. Un `//` commentaire ne le déclenche pas (les
commentaires de ligne sont retirés à dessein), un littéral dans du code oui. La
première tentative de vérification, faite en commentaire, avait conclu à tort que
le verrou était vide.

## Voir aussi

- `a-guard-must-sit-where-the-incident-actually-passed-2026-08-30.md` — mika#2084,
  d'où viennent la forme du refus, la distinction absent/non-résoluble, et la
  règle « refuser avant la file »
- mika#583, mika#1001 — le contrôle par classe que ce ticket transforme en
  réclamation
- mika#1163 — la dérive de prédicats asymétriques sur ces mêmes créneaux
