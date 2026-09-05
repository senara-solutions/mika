---
module: mika-agent/grooming_marker
tags: [dispatch-gate, grooming, loop-substrate, predicate-ordering, fallback-rule]
problem_type: predicate-ordering
category: workflow-issues
---

# Une règle de repli ne cohabite pas avec un discriminateur positionnel

**Incident fondateur : mika#2188, 2026-09-05.** `mika-cloud#205` était groomé,
son escalade résolue par l'opérateur, son plan approuvé par l'architecte — et
structurellement indispatchable. Le prédicat ne rendait pas « je ne sais pas
lire » ; il rendait `Escalated`.

## Le mécanisme

`grooming_verdict` portait deux règles écrites à des moments différents :

1. **Positionnelle** (mika#2158) — « le **dernier** token de verdict du callout
   est l'état ». Deux tokens : `GROOMED`, `ESCALATE`.
2. **Un repli** (AC1 de mika#2158) — une première passe `READY` sans marque de
   passe ultérieure vaut `Groomed`.

La règle 2 rendait **toujours** dès qu'un token de verdict existait. Le repli
n'était donc atteignable que sur un callout n'en portant aucun. Sur le chemin
**prescrit** par `/mika-groom-ticket` :

```
/ce:plan → checkpoint Phase 2.5 (ESCALATE-divergence, résolu par l'opérateur)
        → réconciliation → mika-arch first-pass (READY)
```

`\b(GROOMED|ESCALATE[DS]?)\b` matche `ESCALATE` **à l'intérieur** de
`ESCALATE-divergence` — le tiret satisfait la frontière de mot finale. Dernier
token de verdict = `ESCALATE`. `READY` n'en étant pas un, le repli n'était
jamais lu.

## Ce qui rend cette classe difficile à voir

**Les deux règles sont correctes séparément.** Chacune a ses tests, verts.
Aucune ne contient de bug. Le défaut vit dans leur *composition* : l'une déclare
un ordre total sur les marqueurs, l'autre introduit un marqueur qui ne participe
pas à cet ordre. Un test par règle ne peut pas voir ça — il faut un corps qui
traverse les deux, et ce corps venait d'un chemin nominal que personne n'avait
transcrit en fixture.

**Le symptôme est une absence.** Un ticket qui ne part pas ne réveille personne.
Et `Escalated` est *plus* trompeur qu'`Absent` : il ressemble à une décision.

## La leçon transférable

> Quand un module déclare que son discriminateur est **positionnel**, tout
> marqueur d'état doit entrer dans l'ordre. Un marqueur traité par un repli
> — une branche atteinte seulement quand les autres se taisent — n'est pas
> dans l'ordre : c'est une seconde règle qui contredit la première dès que les
> deux populations se croisent.

Le remède n'est pas de détecter le croisement (« reconnaître le motif escalade
résolue ») : ça répare le symptôme et coûte un concept de plus. Le remède est de
**supprimer l'exception** — une seule liste ordonnée, le dernier marqueur fait
foi. Le module retrouve une règle au lieu de deux qui se marchent dessus.

Corollaire testé : la forme retenue traite gratuitement le cas symétrique
(`first-pass (READY) → revue opérateur (ESCALATE)` → `Escalated`) que la
détection de motif aurait manqué. **Quand une forme candidate rend un cas
symétrique gratuit et l'autre demande une seconde règle, c'est le signe que la
première répare la cause.**

## Le corollaire que la revue a trouvé, et que le plan n'avait pas vu

Promouvoir un repli en marqueur de première classe lui donne une **autorité** qu'il
n'avait pas. Il faut alors lui donner la **discipline** qui va avec cette autorité.

`FIRST_PASS_READY_RE` portait `(?i)` depuis toujours, et c'était inoffensif : tant que
`READY` n'était qu'un repli, il ne pouvait jamais renverser un token de verdict. Le
correctif lui donne le pouvoir de surclasser un `ESCALATE` qui le précède — et le `(?i)`
devenait alors un trou : `first-pass (ready ?)`, écrit en passant dans une phrase
française, valait ordre de dispatch sur un ticket escaladé.

`VERDICT_TOKEN_RE` était sensible à la casse pour une raison écrite dans le module :
« `GROOMED` est un token produit par le pipeline, "groomed" en prose n'en est pas un ».
Cette raison s'applique mot pour mot à `READY` **dès qu'il devient comparable**.

> Quand vous promouvez une valeur au rang de celles qu'elle peut désormais renverser,
> auditez ce qu'elle n'a jamais eu besoin d'être. Les laxismes tolérables dans un repli
> deviennent des vulnérabilités dans un marqueur de première classe. Le diff n'introduit
> aucune ligne fautive : il change ce qu'une ligne existante **signifie**.

Cette classe est invisible au test de non-régression — tous les tests passaient avant
comme après la correction de casse, parce qu'aucun n'écrivait `ready` en minuscules. Elle
n'est visible qu'à la relecture du **delta d'autorité**.

## Le garde-fou qui a tenu

Le désarmement par `LATER_PASS_RE` — « une passe ultérieure annoncée retire le
`READY` du jeu » — a survécu intact au changement de forme. Il gouverne
désormais la *participation* de `READY` à l'ordre, là où il gouvernait un repli.
C'est lui qui préserve `first-pass (READY) → second-pass (GROOMEDLY)` → `Absent`
: une seconde passe dont le verdict est illisible n'est pas rattrapée par sa
première. Un correctif qui aurait perdu cette porte aurait rendu ce corps
groomé.

**Le test qui l'atteste — `word_continuation_after_groomed_is_not_a_verdict` —
est le plus exposé du lot.** Le plan l'avait nommé d'avance comme détecteur :
« si l'implémenteur se trouve tenté de le modifier, c'est le signe que la porte
a été perdue dans la réécriture ». Il n'a pas été touché.

## Discipline de mesure

Le plan exigeait un **rouge-avant lu, pas seulement obtenu** : le test devait
échouer en rendant `Escalated`, et `Absent` aurait imposé la halte — parce
qu'`Absent` aurait signifié une *autre* cause, et invalidé tout le diagnostic.
Mesuré : `left: Escalated, right: Groomed`.

> Un contrôle négatif dont on ne lit que la couleur n'atteste que la moitié de
> ce qu'il pouvait dire. La valeur du rouge est une donnée, pas un détail.

## Ce que ce correctif ne ferme pas

**mika#2120** — le callout `Plan` préfixé par le dépôt — reste ouvert et sous
arbitrage opérateur. C'est la **seconde** cause pour laquelle mika-cloud#205
reste invisible. Les deux sont nécessaires ; fermer mika#2188 ne rend pas
mika-cloud#205 dispatchable à lui seul. Le test
`mika2120_divergence_is_still_open_and_this_test_pins_it` fige cette divergence
plutôt que de la laisser invisible, et devra être supprimé dans le commit qui
rend mika#2120 — pas avant.

## Voir aussi

- `verdict-writer-and-gate-must-share-one-vocabulary-2026-08-27.md` (mika#2012)
  — le prédécesseur : l'écrivain et le lecteur d'un verdict divergent.
  mika#2188 est le cas d'après : le lecteur diverge d'avec **lui-même**.
- `ready-label-dispatch-requires-grooming-marker-2026-04-30.md`
