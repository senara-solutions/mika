---
title: Une garde de découverte doit réfuter sur preuve, pas confirmer — et tenir à tous les paliers
tags:
  - mika-platform
  - dispatch
  - workflow
  - claude-pilot
  - dev-groom
  - testing
module: skills/bundled/_shared/dispatch-lib.sh
problem_type: workflow_issue
category: dispatch
severity: high
created: 2026-08-29
---

# Une garde de découverte doit réfuter sur preuve, pas confirmer — et tenir à tous les paliers

## Symptôme

Un pilote dispatché pour un ticket est lancé sur le plan d'un **autre** ticket, sans aucune erreur :

```
claude-pilot … --command "/ce-work docs/plans/2026-04-11-003-chore-deps-bump-rand-clear-rustsec-2026-0097-plan.md" … -- mika#2026
```

Le plan choisi date du 11 avril et concerne le bump de la crate `rand` pour un avis RustSec. Le ticket, lui, porte sur l'origine des PR. La session travaille avec assurance sur le mauvais contrat.

Le dégât survit à la session : `_iterate_groom_loop` écrit le chemin retenu dans le corps de l'issue sous `> - **Plan:** …`, et `_detect_plan_on_branch` le relit pour construire la commande d'entrée du pilote suivant. Le corps de mika#2026 portait encore le chemin d'avril des heures après.

## Cause racine

Le palier 1 de `_find_issue_plan` globait `*-${ISSUE_NUM}-*-plan.md` et retournait le premier résultat. Le motif attrape **n'importe quelle séquence de 4 chiffres encadrée de tirets**, où qu'elle soit dans le nom : identifiant d'avis de sécurité, année dans un slug, numéro d'une autre issue citée dans le titre. Pour `ISSUE_NUM=2026`, il matche `rustsec-2026-0097`.

Le palier 1 réussissant, `return 0` : les paliers 2 et 3, qui lisent l'en-tête et auraient trouvé le bon plan, ne s'exécutent jamais.

**Pourquoi personne ne l'avait vu.** Toute la conception en paliers a été bâtie contre le **faux négatif** — le plan existe et n'est pas trouvé (mika#1421 n=2, mika#1602 n=3, mika#1617 N=5). Les trois messages `PIPELINE FAILURE` parlent tous d'un plan introuvable. Personne n'a gardé la direction inverse : un palier qui matche par accident ne rate rien, il répond avec assurance et court-circuite les paliers qui auraient corrigé.

## Ce qui n'a PAS marché

**Ancrer le numéro à sa position dans la convention de nommage.** C'était la correction évidente, et c'était la mauvaise. Mesuré sur les 745 fichiers `*-plan.md` de `docs/plans/` :

| Mesure | Compte | Part |
|---|---|---|
| Conformes à `<date>-<NNN>-<type>-<issue>-` | 255 | 34 % |
| En-tête ancré lisible au palier 2 | 208 | 28 % |
| Aucun marqueur d'issue, nulle part | 95 | 13 % |

Un filtre positionnel exclusif aurait fait tomber ~490 plans hors du palier 1 — il aurait échangé un faux positif contre des centaines de faux négatifs, la classe même que trois tickets antérieurs ont fermée. **La convention de nommage n'était pas assez respectée pour servir de filtre.** Elle sert de signal de départage, pas de porte.

## La correction

Trois propriétés, chacune découverte en confrontant le code au corpus réel plutôt qu'à des fixtures.

### 1. Réfuter sur preuve, ne jamais exiger confirmation

Un candidat n'est écarté que si son en-tête nomme **explicitement une autre issue**. L'absence d'en-tête n'est pas une preuve : 13 % du corpus n'en porte aucun, et exiger une confirmation positive les rendrait tous indécouvrables.

```
Le silence n'est pas une réfutation.
```

C'est asymétrique par nécessité. Un motif qui cherche une raison d'**accepter** peut se permettre d'être large : l'erreur est rattrapable par un palier suivant. Un motif qui cherche une raison de **rejeter** doit être étroit : son erreur cache un plan qui existe.

### 2. La garde tient au palier qui lit les corps, pas seulement au premier

Réfuter au seul palier 1 **déplaçait** le défaut au lieu de le fermer. Le palier 3 matche une simple mention du numéro dans les 50 premières lignes. Après correction du palier 1, `ISSUE_NUM=2026` écartait le plan d'avril puis recevait le plan de mika#2038 — dont le Problem Frame nomme mika#2026 neuf fois. Le pilote repartait toujours sur un plan étranger ; seule l'identité du mauvais plan avait changé. Même forme pour `#1383`, qui recevait le plan de `#1685`.

Le palier 2 n'a pas besoin de la garde : il matche un en-tête ancré qui nomme **cette** issue, donc un candidat qu'il accepte ne peut pas être réfuté. Une garde y serait du code mort. Vérifier cette immunité plutôt que d'ajouter la garde partout par symétrie.

### 3. Le motif de revendication se règle sur le corpus, pas sur l'intuition

Trois rétrécissements successifs, chacun imposé par un vrai plan :

| Forme rencontrée | Ce que le motif naïf en faisait | Correction |
|---|---|---|
| `groom_session_id`, dont l'UUID commence par `557` | revendique « issue 557 » → réfute le plan de mika#1469 | ancrer l'étiquette en début de ligne ; `id` n'est pas une étiquette réfutante |
| `Related issue: #456` | revendication de propriété | l'étiquette doit commencer la ligne |
| `The issue: 3 phases remain` | revendique « issue 3 » | idem |
| `**Ticket:** mika#1772/#1773` | ne revendique que `1773` | compter tous les `#N` de la ligne |

Et le motif de réfutation doit être **plus large** que celui du palier 2 sur un point : le palier 2 exige `mika#N`, alors que l'en-tête du cas fondateur est `**Issue:** #539`, sans préfixe. Une sonde de forme palier 2 ne l'aurait pas vu — le bug aurait survécu à sa propre correction.

## La preuve : rejouer sur le corpus réel

Les tests unitaires n'ont trouvé aucun de ces quatre défauts. Le diff de corpus les a tous trouvés.

```sh
# pour chaque numéro apparaissant dans un nom de plan, comparer
# l'ancienne et la nouvelle sortie de _find_issue_plan
git show HEAD:skills/bundled/_shared/dispatch-lib.sh > /tmp/old-lib.sh
# … sourcer chaque version tour à tour, boucler sur les numéros, diff des deux TSV
```

271 numéros probés. Le diff attendu est **petit et entièrement explicable** : un diff large signifie que la garde sur-déclenche et que la classe faux-négatif est rouverte. C'est ce signal qui a fait remonter la régression sur mika#1469, invisible autrement.

Chaque entrée du diff se vérifie une par une. Deux plans dont le nom porte un numéro à quatre chiffres ont été correctement écartés parce que ce numéro était un **numéro de PR**, pas d'issue : `…-chore-rebase-1004-…` porte `**Ticket:** mika issue#1035`, `…-chore-rebase-1005-…` porte `**Ticket:** mika issue#1037`.

## Deux pièges de test qui rendent une suite verte pour rien

**Une fixture de remplissage ne peut pas déclencher un palier qui lit les corps.** Toutes les fixtures écrivaient `Body padding line N for size.`, donc aucune ne citait jamais le numéro cible. Les assertions « ce plan n'est pas retourné » étaient vertes alors que les mêmes entrées, sur le corpus réel, rendaient un plan étranger. Le commentaire qui les accompagnait — « Verified, not assumed » — sur-affirmait. Une fixture négative doit contenir la forme qui déclencherait le mécanisme, sinon elle prouve seulement que le mécanisme n'a pas tourné.

**Un glob de fixture doit être vérifié avant d'être cru.** Un cas de départage utilisait le nom `…-about-3300-plan.md`, qui ne matche pas `*-3300-*-plan.md` : après `-3300-` il faut encore un `-plan.md`, donc un tiret. Le test passait avant même l'implémentation. Un test qui devient vert trop tôt se vérifie ; il ne se célèbre pas.

## Une suite de tests non câblée ne garde rien

`skills/bundled/_shared/tests/test_find_issue_plan.sh` existe depuis mika#1421 et couvre toute la lignée faux-négatif. Elle n'était câblée **ni au Makefile ni à la CI** : seule `test-dispatch-lib.sh` tournait, via `make test-dispatch-lib`. Trois tickets successifs y ont déposé leurs régressions sans que rien ne les rejoue.

Avant de considérer qu'un critère « ajouter un test de régression » est satisfait, vérifier que la suite qui l'accueille tourne quelque part.

## Le message d'échec doit suivre la nouvelle cause

Ajouter une garde crée une cause d'échec que les messages existants ne connaissent pas. Les trois chaînes `PIPELINE FAILURE` affirmaient « no filename match … no anchored header match … » et orientaient l'opérateur vers un bug de découverte ou une dérive du pilote. Après une réfutation, c'est faux : un plan a matché et a été écarté délibérément, et la cause réelle la plus probable est un en-tête qui nomme le mauvais ticket — un parent de milestone au lieu de la sous-issue. Cela se corrige dans l'en-tête, pas en élargissant la découverte.

`_find_issue_plan` publie donc ses écartements dans `FIND_ISSUE_PLAN_REFUTED`, sur le même contrat de globale non-`local` que `GROOM_LOOP_FAILURE_REASON` juste à côté, et les messages nomment le candidat écarté avec l'issue que son en-tête revendique.

## Prévention

Avant de resserrer un motif de découverte qui a mal matché, mesurer le corpus qu'il traverse. La question n'est pas « ce motif est-il trop large ? » mais « combien d'entrées réelles perdrais-je en le resserrant ? ». Ici la réponse était 490 sur 745, et elle a changé toute la conception.

## Références

- `skills/bundled/_shared/dispatch-lib.sh` — `_plan_header_claimed_issues`, `_plan_header_refutes_issue`, `_plan_filename_issue_slot`, les paliers de `_find_issue_plan`.
- `docs/solutions/workflow-issues/find-issue-plan-header-shape-widening-2026-06-27.md` — la lignée faux-négatif (mika#1381, mika#771, mika#1600) et sa consigne de tester par le comportement plutôt qu'en ré-encodant l'expression régulière.
- mika#2038 — le ticket. mika#2029 est un défaut **distinct** : mika#2013 et mika#1963 ont reçu le bon plan la même nuit et se sont arrêtés de la même façon.
