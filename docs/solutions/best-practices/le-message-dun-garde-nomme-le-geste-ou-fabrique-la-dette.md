---
title: "Le message d'un garde nomme le geste, ou il fabrique la dette qu'il signale"
date: 2026-11-26
category: best-practices
module: scripts, skills/bundled/_shared, Makefile
problem_type: best_practice
component: development_workflow
severity: medium
applies_when:
  - "Écrire le message d'échec d'un lint, d'un scan ou d'une garde structurelle"
  - "Lire un échec de CI qui propose une allowlist, une exemption ou un relèvement de seuil"
  - "Geler une métrique (taille, compte, durée) par une constante dans le dépôt"
  - "Constater qu'un défaut déjà gardé se reproduit, et que la garde a pourtant parlé"
related_components:
  - tooling
  - testing_framework
tags:
  - garde-structurelle
  - message-dechec
  - allowlist
  - plafond-statique
  - anti-vacuite
  - mika-3177
  - mika-3178
  - mika-3179
---

# Le message d'un garde nomme le geste, ou il fabrique la dette qu'il signale

## Contexte

`skills/bundled/_shared/test-dispatch-lib.sh` est l'agrégat d'assertions de
`dispatch-lib.sh`. Il pesait 1 041 079 octets sur `main`, pour un plafond
`LARGE_FILE_LIMIT` de 1 048 576 octets déclaré par `scripts/check-secrets.sh` :
moins de 7,5 Ko de marge, après ~200 Ko pris en un mois.

Sur #3177, un bloc de 238 lignes ajouté à cet agrégat a fait rougir le job
`Secret Scan` (run 45856244085) **après** le push. Le pilote n'avait aucun moyen
de l'apprendre avant la CI. Mais le coût réel n'était pas la latence : c'était ce
que le message disait.

```
ERROR: <f> is 1028KB — exceeds 1MB limit (add to LARGE_FILE_ALLOWLIST with a named reason if legitimate)
```

Ce message est correct pour le défaut qu'il a été écrit pour attraper — un
fixture binaire committé par erreur. Lu par un pilote dont le diff est un bloc de
tests shell, il **nomme le mauvais geste**. Le geste qu'il propose (une entrée en
allowlist, avec une raison nommée, ce qui a l'air rigoureux) fait passer la PR et
laisse l'agrégat croître jusqu'au plafond. Le geste correct — une suite neuve sous
`skills/bundled/_shared/tests/`, un maillon par fichier — n'apparaît nulle part.

Sur #3177 l'itération supplémentaire a trouvé le bon geste. Rien ne garantissait
qu'elle le trouve : le chemin le moins coûteux, celui que le message indiquait,
menait à l'allowlist.

## Consigne

**Le message d'échec d'une garde est une instruction opérationnelle, pas un
constat.** Quelqu'un va l'exécuter, souvent sans contexte et sous pression de
tour. Écrivez-le donc en trois parties, dans cet ordre :

1. **La mesure.** Ce qui a été lu, la borne, et le delta. Un lecteur doit pouvoir
   vérifier le verdict sans relancer la garde.
2. **Le geste.** Ce qu'il faut faire, assez précisément pour être exécuté : le
   chemin du fichier à créer, un précédent existant à copier, et **tous** les
   points de câblage (ici les deux cibles `test` et `test-dispatch-lib` du
   Makefile — une suite câblée dans `test` seulement n'est pas une porte de merge,
   mika#2578).
3. **Le non-geste.** L'issue de facilité qu'il faut refuser, nommée en propre, et
   pourquoi. C'est la partie qu'on oublie, et c'est celle qui a manqué sur #3177.

Quand la garde partage son terrain avec une autre dont le message pointe ailleurs,
le non-geste doit citer cette autre garde. `check-dispatch-lib-aggregate-frozen.sh`
dit en toutes lettres de ne pas suivre ce que `Secret Scan` suggère.

**Une garde qui gèle une métrique par une constante a un mode de défaillance
unique, et ce n'est pas celui qu'elle surveille.** Personne ne peut ajouter un
octet à l'agrégat ; n'importe qui peut relever la constante. Deux contrôles
rendent cette relève visible, et ils vivent dans le harnais de la garde, pas dans
la garde :

- **Population.** La constante est extraite du garde par `sed` ancré en colonne
  zéro, puis comparée à la mesure réelle. L'égalité est exigée. Sans ce contrôle,
  le cas vert (« le fichier conforme passe ») est une tautologie : une constante
  dérivée vers le haut le laisse passer tout en ne refusant plus rien.
- **Dérive.** La constante gelée est bornée par le seuil de l'autre garde
  (`LARGE_FILE_LIMIT`), **lu par `sed` et jamais sourcé**. Une constante montée
  jusqu'au plafond rend le gel sans objet, puisque l'autre garde rougirait
  d'abord. Un rouge sur un renommage de l'une des deux variables est le lockstep
  voulu, pas un faux positif.

**Un plafond statique se juge sur une seule donnée, et c'est sa qualité.** Un diff
contre `origin/main` mesurerait la croissance introduite par la PR, mais dépend de
la profondeur du checkout et de la présence de la ref : muet en local sur une
branche fraîche, muet dans `make test` hors d'un clone complet. Une taille absolue
rend la même mesure en local, dans `make test` et en CI.

**Les deux verts d'une telle garde ne sont pas le même vert.** « Conforme » et
« l'objet surveillé n'existe plus » doivent être distinguables dans la sortie :
un 0 muet passe vert sur un fichier *renommé*, ce qui est exactement la panne
silencieuse de mika#2578. Le message du vert « absent » nomme aussi le retrait de
la garde devenue inutile, pour qu'elle ne survive pas à son objet.

## Pourquoi c'est important

Un garde dont le message nomme l'exemption ne rate pas le défaut : il
**l'autorise**, avec la signature de la CI, et chaque exemption rend la
réparation de fond plus coûteuse. Dans ce cas précis, la réparation de fond est la
scission de l'agrégat (#3178), ouverte en dormeur en attente de deux décisions de
l'opérateur. Chaque allowlist ajoutée en attendant déplace la frontière que #3178
devra franchir.

C'est aussi pourquoi un gel ne se documente pas dans un plan : la consigne doit
vivre là où le défaut se produit. Le bloc d'en-tête du fichier gelé le dit à qui
l'ouvre, et le message d'échec le redit à qui l'a quand même modifié.

## Quand l'appliquer

- À l'écriture de toute garde dont le chemin de succès est « rien n'a changé » :
  seuil de taille, compte d'objets, budget de tours, plafond de durée.
- Chaque fois qu'un échec de CI vous propose une exemption : demandez-vous si le
  message a été écrit pour *votre* diff. Un scan générique parle au défaut qui l'a
  fait naître, pas au vôtre.
- À la relecture d'un PR qui relève une constante : exigez le contrôle de
  population dans le harnais. Sans lui, « la population d'exceptions est vide »
  est une phrase du plan, pas un fait mesuré à chaque run.

## Exemples

Ce que livre mika#3179 : `scripts/check-dispatch-lib-aggregate-frozen.sh`
(plafond statique, quatre sorties distinguables — 0 conforme, 1 dépassement,
0 absent avec son message dédié, 2 mesure impossible) et son harnais
`scripts/test-check-dispatch-lib-aggregate-frozen.sh` (C1–C4 sur des copies
temporaires, plus les contrôles de population et de dérive). Les deux sont câblés
dans les cibles `test` et `test-dispatch-lib` du Makefile, la seconde étant celle
que `ci.yml` nomme.

Avant — le message que `Secret Scan` rend, écrit pour un fixture binaire :

```
ERROR: <f> is 1028KB — exceeds 1MB limit (add to LARGE_FILE_ALLOWLIST with a named reason if legitimate)
```

Après — le message du lint dédié, sur le même fichier :

```
ERROR: l'agrégat a grossi de 1 octet(s).
  mesuré   : 1041744 octets
  gelé à   : 1041743 octets (AGGREGATE_FROZEN_BYTES, mika#3179)

LE GESTE — votre bloc d'assertions va dans une suite NEUVE :
  skills/bundled/_shared/tests/test_<sujet>_<ticket>.sh
sur le modèle de `…/tests/test_callout_plan_identity_3174.sh` […]. Puis câblez-la
dans LES DEUX cibles du Makefile […].

LE NON-GESTE — ne relevez pas AGGREGATE_FROZEN_BYTES, et n'ajoutez pas
d'entrée à LARGE_FILE_ALLOWLIST de `scripts/check-secrets.sh`. […]

La SEULE sortie légitime de ce plafond est #3178, la scission de l'agrégat.
```

Voisins utiles :
[une garde que rien n'invoque se lit exactement comme un arbre propre](./une-garde-que-rien-ninvoque-se-lit-exactement-comme-un-arbre-propre-2026-09-28.md)
(le câblage, l'autre moitié du problème),
[structural guard fails open: parser, fixture, harness](./structural-guard-fails-open-parser-fixture-harness.md)
(un garde qui ne peut pas mesurer ne rend pas un vert),
[an exempted disposition is the attack surface](./an-exempted-disposition-is-the-attack-surface-2026-08-30.md)
(l'exemption comme surface),
[invariant test stricter than spec needs an allowlist](./invariant-test-stricter-than-spec-needs-an-allowlist-2026-06-27.md)
(le cas où une allowlist est, elle, le bon geste — et ce qui l'en distingue :
une population d'exceptions mesurée, pas supposée).
