# mika#2924 — la référence de clôture, côté issue

`issues-milestone-33.json` est une **sortie `gh` réellement capturée**, le
2026-10-26, par la commande que `Reader::read_with_runner` émet :

```
gh issue list --repo senara-solutions/mika --milestone 33 --state all \
  --json number,title,state,body,labels,updatedAt,closedByPullRequestsReferences \
  --limit 100
```

Elle porte les **deux contrôles dans le même document**, parce que c'est ainsi
qu'ils arrivent en production — le rapport ne lit pas deux fichiers :

| ticket | `closedByPullRequestsReferences` | ce que le rapport doit rendre |
|---|---|---|
| #2886 | une entrée, PR `2892`, URL `…/pull/2892` | `PR https://github.com/senara-solutions/mika/pull/2892` |
| #1680 | `[]` — fermé à la main | `closed (no linked PR)`, **mot pour mot** |

#1680 est l'un des 14 tickets fermés à la main que la sonde AC3 attend de
retrouver dans le compte résiduel. Son texte doit rester identique : c'est la
chaîne que `grep -c 'closed (no linked PR)'` compte sur le rapport du sink.

## Deux fidélités à ne pas confondre avec du bruit

- **Les corps sont tronqués à leurs trois premières lignes.** Rien d'autre n'est
  modifié. Le composeur ne lit du corps que les encarts `Plan:`/`Branch:` et les
  bloqueurs, tous présents dans la fenêtre conservée ; transporter 16 Ko de
  prose n'aurait rien ajouté au contrôle.
- **Le corps de #1680 contient `d''atteindre` et `c''est-à-dire`.** Ce ne sont
  pas des coquilles de capture : l'apostrophe est réellement doublée sur la
  forge, résidu d'un échappement shell du producteur du corps. La fidélité
  passe avant la correction orthographique — un fixture retouché ne témoigne
  plus de rien.

## Ce que la forme prouve, au-delà des valeurs

L'entrée de #2886 porte un objet `repository` imbriqué. C'est lui qui justifie
le refus de recomposer `{repo}/pull/{number}` sur ce chemin (R3 du plan) : la
référence peut nommer un **autre** dépôt, et une URL recomposée pointerait
alors vers une PR qui n'existe pas. Le lecteur prend le champ `url` tel quel.
