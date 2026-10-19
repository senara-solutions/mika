---
module: mika-agent/db/tasks.rs + mika-agent/task_engine/engine.rs
tags: [loop-substrate, reaper, sql, grace-window, aggregate, grooming, dispatch-class, false-positive]
problem_type: logic_error
category: dev-loop
created: 2026-10-18
ticket: mika#2762
---

# Une grâce par ligne n'est pas une grâce sur la population

## Le problème

Le reaper de parents orphelins, étendu à la classe `groom` par mika#2750
(PR mika#2754, déployé le 2026-10-17 à 06:39Z), faisait échouer le parent d'un
groom **réussi**, à la seconde où son callback était livré.

Mesuré deux fois le même matin, pendant que l'implémentation de mika#2680
tenait la place :

| ticket | callback groom livré | parent passé `failed` | motif |
|---|---|---|---|
| mika#2752 | 07:37:19Z (succès) | 07:37:20Z | `groom_callback_delivered_without_redispatch` |
| mika#2751 | 07:54:39Z (succès, 53 tours, 5,43 $) | 07:55:20Z | idem |

Un vol de groom entier perdu à chaque fois (≈ 5 $, 15 à 20 min), et le tour de
reprise devancé : le corps du ticket n'était pas marqué, donc le feeder
reprenait le ticket et le regroomait. En régime cap implement 2, c'est le cas
nominal, pas un cas limite.

Le contrôle fait avant le marqueur de mika#2754 — 25 parents historiques,
aucun faux positif — ne pouvait pas voir ce cas : à l'époque, aucun groom ne
finissait pendant qu'une autre implémentation tenait la place.

## La cause

La requête cherchait un enfant callback `delivered` de la bonne classe dont
`updated_at` dépasse la grâce de 600 s, et l'absence de frère actif. La grâce
était écrite **par ligne** :

```sql
AND child.status = 'delivered'
AND child.updated_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
```

Ce prédicat ne dit pas *« la dernière livraison de ce parent a plus de 600 s »*.
Il dit *« ce parent a **au moins une** livraison de plus de 600 s »* — et une
jointure qui trouve une seule ligne satisfaisante rend le parent, quoi que
disent les autres. **Une grâce est une propriété de la population d'un parent,
jamais d'une de ses lignes.** Dès qu'un parent a deux enfants livrés, les deux
lectures divergent, et le sens voulu est toujours le premier.

Deux populations fournissent le vieil enfant, structurellement :

1. **Le lanceur différé `long_running:run_claude_pilot:deferred`.** Il est de
   classe `groom`, `trigger_type='callback'`, `action_type='resume_agent'` — il
   coche tous les termes du prédicat. Et il est livré quand il *lance* le
   groom, soit 11 min avant le callback du groom lui-même. Il satisfaisait donc
   la grâce à lui seul, dans la seconde qui suivait la livraison du vrai
   callback. C'est le cas mesuré.
2. **Un groom en deux passes a toujours un enfant de passe 1 ancien.** Même
   sans lanceur différé, la même arithmétique se reproduit.

Le second trou — le frère `pending`/`in_progress` qui protège normalement le
parent **n'existe pas encore** quand la place implement est prise
(`global_dispatch_active` refuse le dispatch suivant) — n'est pas une cause
indépendante : c'est ce qui ouvre la fenêtre où le premier trou détruit quelque
chose.

## Le piège du correctif

« Ajouter un `HAVING MAX(...)` » n'est **pas** le correctif. Garder les deux
clauses reproduit le défaut à l'identique : le `WHERE` écarte la livraison
récente **avant** l'agrégation, et `MAX` ne voit plus que les anciennes. Le
`WHERE` par ligne doit **disparaître** et être remplacé :

```sql
AND child.status = 'delivered'
AND child.label NOT LIKE '%:deferred'   -- un lanceur différé n'ancre rien
...
GROUP BY parent.id
HAVING MAX(child.updated_at) < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
```

Les deux clauses ne sont pas redondantes et ne sont pas interchangeables : le
`WHERE` décide *quelles livraisons peuvent ancrer*, le `HAVING` décide
*laquelle ancre*. (SQLite refuse de toute façon un agrégat dans `WHERE` — d'où
`HAVING`.) La forme était déjà dans le fichier, trois fonctions plus bas :
`find_settleable_dispatch_parents` (mika#2405), dont le doc-comment explique
que la différence de forme est intentionnelle. Il n'y avait rien à inventer.

## Deux propriétés qu'un correctif « permissif » rate

**L'exclusion du lanceur est plus *juste* dans les deux sens, pas seulement
plus permissive.** Le cas mesuré est le côté *trop vieux* : un lanceur ancien
promu ancre fauche un parent sain. Le cas symétrique est aussi faux : un
lanceur livré *dans* la grâce deviendrait le `MAX` et **épargnerait**
indéfiniment un parent dont le vrai callback a livré depuis longtemps — le
défaut d'origine de mika#2750 rouvert par son propre correctif. Les deux
directions méritent leur test ; seul le premier avait une mesure.

**L'identité de l'ancre n'est pas celle du callback.** La requête rend
`MIN(child.id)` comme `callback_task_id` (l'identité dont le motif `NO_PR:` est
résolu) et `MAX(child.updated_at)` comme `last_callback_at` (l'instant contre
lequel la grâce a été mesurée). Ce sont **deux enfants différents** dès qu'un
parent a plus d'un callback livré. Lire le premier comme le second, c'est
écrire un journal qui mentira sur ce que le reaper a vu. D'où le champ
`last_callback_at` sur la ligne `task_engine_reaper.evaluated` : sans lui,
« la grâce a été satisfaite par le lanceur » et « par le vrai callback »
impriment les mêmes octets.

## Le prédicat qui *protège* n'a pas la même tolérance que celui qui *cible*

`%:deferred` est un suffix-anchor **strict**, et c'était déjà pinné —
par `test_has_active_callback_tasks_excluding_ignores_deferred_wrappers`
(mika#1163), qui vérifie explicitement qu'un `:deferred:retry` **échappe** à
l'exclusion. Là, l'échappement est inoffensif : le prédicat *cible* les
wrappers à exclure d'un décompte de dispatchs actifs, et un wrapper compté en
trop ne détruit rien.

Ici le même prédicat *protège*. Un lanceur différé sorti de la portée du
suffixe redevient ancre de grâce et détruit un vol de groom. **Le même
littéral, la même tolérance, deux conséquences de gravité opposée** — ce qui
justifie une garde de dérive propre à ce site plutôt qu'une confiance dans la
garde voisine :

```rust
assert!(DEFERRED_DISPATCH_LABEL.ends_with(":deferred"), …);
```

Écrite en `#[test]` et non en `const assert!` : `str::ends_with` n'est pas une
`const fn`.

## Le contrôle de bonne foi du câblage

Le défaut vivait dans **deux** fichiers : le prédicat SQL, et l'appel du tick
qui le déclenche pour la classe `groom`. Le rollback du vol 1 (PR mika#2765)
avait retiré l'appel, pas le prédicat — donc un test qui épingle le cas mesuré
*à travers `tick()`* était **vert pour la mauvaise raison**, et l'aurait
continué si on avait réparé le SQL sans rebrancher l'appel.

La séquence qui ferme ce trou, et qui vaut pour tout correctif dont la cible
est désarmée au moment où on l'écrit :

1. Rétablir **seulement** le câblage, sans toucher au prédicat.
2. Vérifier que le test **rougit**. Ici :
   `left: "failed" / right: "in_progress"` — le défaut mesuré, reproduit.
   S'il reste vert, le câblage n'est pas branché et tout le reste du vol est
   non testé.
3. Écrire le prédicat, et le voir repasser vert.

Un test vert sur une cause désarmée n'atteste rien. L'ordre des commits suit
la même règle en sens inverse : le prédicat d'abord (mergeable seul, le tick ne
balaie pas encore `groom`), le câblage ensuite — on ne réarme jamais un
balayage avant que son prédicat soit vert.

## Hors périmètre, nommé plutôt que corrigé

- **Le résidu à T+600 s.** Un groom réussi dont personne ne reprend la main
  reste faucheable une fois la grâce écoulée sur son *vrai* callback. Assumé :
  à cet instant le tour de reprise a tourné, les callouts et le verdict sont
  dans le corps du ticket, et faucher ne détruit plus rien. Le trou n'est pas
  fermé frontalement — il est désarmé sur la fenêtre où il détruisait
  (T+1 s).
- **Les dix sites `label = DEFERRED_DISPATCH_LABEL`** (égalité exacte) et les
  huit `label NOT LIKE '%:deferred'` (suffixe) cohabitent dans le même
  fichier. L'asymétrie est réelle et n'est pas traitée ici ; la nouvelle clause
  est écrite **en ligne comme ses huit voisines**, pour ne pas introduire un
  neuvième dialecte.

## Référence

- Prédicat : `crates/mika-agent/src/db/tasks.rs`,
  `find_orphaned_parent_tasks` et sa jumelle de succès
  `find_completable_parent_tasks_on_pr_url` (contrat de couple mika#1162 : la
  forme de la grâce ne doit pas diverger entre les deux moitiés).
- Modèle de forme : `find_settleable_dispatch_parents` (mika#2405).
- Câblage et garde TOCTOU : `crates/mika-agent/src/task_engine/engine.rs`,
  `reap_orphaned_parent_tasks` (mika#1126 AC-3).
- Tests : `mika2762_*` dans `src/db/tests/reapers.rs` (cinq cas) et
  `tests/eval/test_groom_parent_reap_2750.rs` (le cas mesuré, à travers
  `tick()`).
