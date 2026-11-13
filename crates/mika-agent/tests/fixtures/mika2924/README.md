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

---

# mika#2924, vol 2 — la timeline REST d'un ticket **ouvert**

`timeline-2698.json` et `timeline-2934.json` sont deux **sorties `gh`
réellement capturées** le 2026-10-27, par la commande que
`Reader::read_with_runner` émet à sa septième lecture :

```
gh api "/repos/senara-solutions/mika/issues/<N>/timeline?per_page=100"
```

réduites aux seuls chemins JSON que `compose_merged_prs` lit, par :

```
--jq '[ .[] | {event: .event, source: (if .source == null then null else
  {issue: {number: .source.issue.number,
           pull_request: (if .source.issue.pull_request == null then null
                          else {merged_at: .source.issue.pull_request.merged_at} end)}} end)} ]'
```

Les deux contrôles de la lecture 7 sont là, un par document, parce qu'ils
portent sur **deux tickets distincts** — un ticket n'a qu'une timeline :

| ticket | ce que la timeline porte | ce que le composeur doit rendre |
|---|---|---|
| #2698 | 36 événements, 6 PR mergées + 3 citations d'*issues* | `Citing([2931, 2929, 2892, 2706, 2704, 2712])` |
| #2934 | 12 événements, **une** PR citée (#2936) et **non mergée** | `NoneCiting` |

## Trois fidélités à ne pas confondre avec du bruit

- **Le nombre d'entrées est celui de la forge, pas celui des PR.** La réduction
  porte sur le *contenu* de chaque événement, jamais sur leur nombre : c'est la
  longueur du tableau qui décide de `IndeterminatePageFull` et que la sonde
  journalise en `events_read`. Un fixture filtré sur `cross-referenced` aurait
  rendu 9 et 1, et la garde de page pleine n'aurait plus rien mesuré.
- **#2886, #2924 et #2930 citent #2698 sans être des PR.** Leur
  `pull_request` est `null` : ce sont des *issues*. Elles sont le contrôle
  négatif de `mika2924_un_evenement_qui_nest_pas_une_pr_ne_compte_pas`, et
  elles arrivent dans le même document que les PR, comme en production.
- **#2936 porte `"merged_at": null`.** C'est une PR ouverte, pas une PR
  absente : sans le troisième filtre, #2934 rendrait `Citing([2936])` et
  « Next » dirait à l'opérateur que son travail est déjà livré.

## Ce que l'ordre prouve, au-delà de l'ensemble

Le tri est sur `merged_at` **décroissant**, et #2712 est ce qui l'atteste :
mergée le 2026-10-15, elle porte le **plus grand** numéro des trois premières
citations et arrive pourtant **dernière**. Un tri replié sur le numéro de PR —
la simplification plausible — rendrait `[2931, 2929, 2892, 2712, 2706, 2704]`
et rougirait ici. C'est aussi pourquoi le libellé ne nomme que les **trois plus
récemment mergées** : l'opérateur veut savoir où en est le dernier vol, pas
l'histoire complète.
