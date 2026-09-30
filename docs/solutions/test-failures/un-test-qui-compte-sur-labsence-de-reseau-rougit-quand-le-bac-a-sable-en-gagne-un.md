---
title: Un test qui compte sur l'absence de réseau rougit le jour où le bac à sable en gagne un
date: 2026-09-29
category: test-failures
module: mika-agent
problem_type: test_failure
component: testing_framework
symptoms:
  - "`verdict_pass_completed_task_perimeter_fail_closed_holds_for_operator` panique sur « should be Handled (forge-gate fail-closed), not Passthrough »"
  - "Rouge déterministe en session pilote dispatchée, sur une PR dont le diff ne touche ni le test ni son chemin"
  - "Le test échoue seul, hors parallélisme — ce n'est pas une course de ports"
root_cause: environment_assumption
resolution_type: diagnosis
severity: medium
tags: [tests, sandbox, pilot, egress-proxy, gh-cli, fail-closed, faux-rouge, mika-2572, mika-2573, mika-2569]
---

# Un test qui compte sur l'absence de réseau rougit le jour où le bac à sable en gagne un

## Problem

En session pilote dispatchée sur mika#2573, `cargo test -p mika-agent` rend un
rouge **déterministe** :

```
eval::test_verdict_handler::verdict_pass_completed_task_perimeter_fail_closed_holds_for_operator
  panicked: pass with completed task + fetch-error should be Handled
            (forge-gate fail-closed), not Passthrough
```

Le diff de la PR est **strictement additif** (`git diff --numstat` : zéro
suppression) et ne touche ni `test_verdict_handler.rs`, ni `fetch_pr_files`, ni
`try_handle_pr_review_verdict`. Le test échoue aussi **seul**, donc ce n'est pas
la classe « course de ports » de mika#2569.

Le piège de lecture est là : un rouge déterministe sur un job de test se lit
comme une régression du diff, et c'est la première chose qu'un pilote va
« réparer ».

## Root cause

Le test est écrit sur une **hypothèse d'environnement**, énoncée dans son propre
commentaire :

```rust
// fetch_pr_files errors in test env → fail-closed to DECISION-CORE → Handled.
```

Il passe un `Some("fake-token")` et compte sur l'échec de l'appel pour que le
forge-gate retombe en `DECISION-CORE` et rende `Handled`.

Or `fetch_pr_files` (`crates/mika-agent/src/perimeter/fetch.rs`) ne fait pas un
appel HTTP : elle **shell-out** sur

```
gh pr view <n> --repo <repo> --json files
```

et depuis **mika#2572** cette commande fonctionne dans le bac à sable pilote —
le sandbox porte le placeholder `GH_TOKEN=proxy-managed-no-secret`, et
`mika-pilot-github-auth-addon.py` retire l'`Authorization` du client pour poser
la vraie credential **côté hôte** sur `api.github.com`. Le `fake-token` du test
n'a donc aucune importance : le proxy le remplace.

Mesuré dans le bac à sable, sur la commande exacte que le test provoque :

```console
$ gh pr view 42 --repo senara-solutions/mika --json files
{"files":[{"path":"CHANGELOG.md",…},{"path":"Cargo.lock",…},…]}
```

L'appel **réussit**, la classification n'est plus fail-closed, et le gate rend
`Passthrough` — ce que le test traite comme un échec.

C'est la classe que mika#2569 vient de nommer pour un port local, transposée
d'un cran : *un test qui a besoin qu'un appel échoue ne peut pas obtenir cet
échec de l'absence d'un service*. Là-bas le service imprévu était le serveur
factice d'un test voisin ; ici c'est `api.github.com`, rendu joignable par le
correctif d'un ticket voisin.

## Solution

**Rien n'a été changé sur ce test** dans le cadre de mika#2573 : il est hors
périmètre, et le « corriger » sous la pression d'un rouge aurait été réparer un
test qu'on ne comprenait pas encore.

Ce qui est livré ici est la **procédure de discrimination**, parce que c'est
elle qui coûte cher à redécouvrir. Trois questions, dans cet ordre :

1. **Le diff touche-t-il ce chemin ?**

   ```bash
   git diff --numstat HEAD~1 HEAD
   git diff HEAD~1 HEAD | grep -n 'fetch_pr_files\|perimeter\|try_handle_pr_review_verdict'
   ```

   Zéro suppression et zéro occurrence ⇒ le diff est étranger au chemin. Ce
   n'est pas une preuve d'innocence, c'est ce qui autorise la question 2.

2. **Le test a-t-il une hypothèse d'environnement ?** Chercher, dans son corps
   et ses commentaires, une phrase de la forme *« errors in test env »*, *« no
   network in tests »*, *« unreachable »*. Une telle phrase est un contrat avec
   l'environnement, et un contrat avec l'environnement se vérifie.

3. **Cette hypothèse tient-elle ici ?** Rejouer la commande que le code
   construit, à la main. Si elle réussit, la cause est établie — et ce n'est ni
   le diff, ni le test, c'est le bac à sable.

## Remède, et ce qu'il n'est pas

Le remède durable est de **ne pas dépendre d'une absence** :

- soit un point de terminaison mort **réservé** au sens de mika#2569
  (`DeadEndpoint::reserve()`), tenu pour toute la durée de l'appel ;
- soit une injection qui rend l'échec explicite plutôt qu'accidentel.

Ce qui n'est **pas** un remède : neutraliser le proxy pour la suite de tests.
Ça rendrait le test vert et retirerait à tous les autres l'accès réseau dont
certains ont besoin — et, surtout, ça laisserait l'hypothèse en place pour le
prochain ticket qui rend un service joignable.

## Le faux vert symétrique

Sur le job CI `Check`, `gh` n'est pas authentifié : l'appel échoue, le gate
retombe en fail-closed, et le test est **vert**. C'est le miroir exact de
l'avertissement que mika#2569 s'est écrit — *vert sur CI et rouge en pilote* —
et c'est ce qui explique qu'un test dans cet état ait pu vivre dans l'arbre sans
que personne le voie.

La conséquence pour la lecture est la partie qui compte : **sur cette famille de
tests, un vert sur CI ne dit pas que l'hypothèse d'environnement est vraie ; il
dit que CI la satisfait.** Un pilote qui voit rouge n'a donc pas besoin de
supposer que sa branche a cassé quelque chose — il a besoin de la question 2.

## Related

- `mika#2569` — un point de terminaison mort se réserve, il ne se libère pas
  (même classe, port local ; porte `DeadEndpoint::reserve()`)
- `mika#2572` — `GH_TOKEN` factice dans le bac à sable pilote, pour que `gh`
  parte sur le réseau (le correctif qui a rendu l'hypothèse fausse)
- `mika#2495` — le proxy egress intercepte une adresse non routable et rend un
  statut HTTP, jamais une erreur de transport
- `mika#2573` — le ticket pendant lequel ce rouge a été rencontré et écarté
