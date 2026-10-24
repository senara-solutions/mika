---
title: "Le rouge-avant d'un garde CI dont le correctif est déjà sur main ne se prouve pas par une PR vers main"
category: ci-cd
module: ci
component: github-actions
date: 2026-10-24
problem_type: best_practice
severity: medium
applies_when:
  - "un job de CI est ajouté APRÈS le correctif du défaut qu'il doit attraper, et ce correctif est déjà sur main"
  - "le workflow ne se déclenche que sur push vers main et sur pull_request, sans workflow_dispatch"
  - "un AC demande de voir le nouveau garde rougir sur l'arbre pré-correctif"
tags:
  - ci
  - github-actions
  - pull-request
  - merge-ref
  - rouge-avant
  - red-before
  - required-checks
  - ruleset
related_components:
  - pr_merge_with_gate
---

# Le rouge-avant d'un garde CI dont le correctif est déjà sur main ne se prouve pas par une PR vers main

## Context

mika#2902 ajoute un job `darwin-check` à `.github/workflows/ci.yml`. Ce job
attrape la classe de mika#2900 : `libc` déclaré pour `cfg(target_os = "linux")`
seulement, alors qu'il est appelé hors `#[cfg]`. Le correctif de mika#2900 est
**déjà sur `main`** quand le job est écrit. L'AC2 du ticket demande de voir le
job rougir sur l'arbre pré-correctif (`3993f91d`), avec l'`E0433` du tag.

Le geste qui vient d'abord à l'esprit : rebaser la branche du job sur
`3993f91d`, la pousser et ouvrir une PR brouillon vers `main`. **Il produit un
vert, et ce vert ne prouve rien.**

## Guidance

Sur un événement `pull_request`, `actions/checkout` récupère par défaut
`refs/pull/<N>/merge` : un commit de fusion de la **tête dans la base**, calculé
par GitHub. Ce n'est pas la tête seule. Si la base est `main`, la fusion contient
le correctif, quelle que soit l'ancienneté de la tête. Le job compile l'arbre
corrigé et passe au vert.

Pour voir rougir un garde sur un arbre pré-correctif, c'est la **base** qui doit
être pré-correctif, pas seulement la tête :

1. une branche base à un commit pré-correctif (ici `test/2902-rouge-avant-base`
   à `3993f91d`) ;
2. une branche tête = cette base + le seul commit qui ajoute le job, en
   cherry-pick (`test/2902-rouge-avant`) ;
3. une PR brouillon tête → base. `ci.yml` déclare `pull_request:` sans filtre de
   branches (`.github/workflows/ci.yml:6`), donc une base autre que `main`
   déclenche bien le workflow, et le fichier de workflow lu est celui du commit
   de fusion, qui contient le nouveau job ;
4. transcrire la sortie dans le corps de la vraie PR (un squash merge efface le
   `git log` intermédiaire), puis fermer la PR brouillon et supprimer les deux
   branches distantes.

Avant de cherry-picker, vérifier que le fichier de workflow est identique entre
la base pré-correctif et `origin/main` (`git diff --stat <base> origin/main --
.github/workflows/ci.yml` vide). Sinon, la PR brouillon ne fait pas tourner le
même workflow que celui qu'on livre.

Coût à prévoir : la PR brouillon fait tourner **tout** le workflow, y compris
les jobs sur runner self-hosted (`Check` sur gentux). Seul le verdict du nouveau
job compte.

### Corollaire : un job ajouté n'est pas un check requis

Ajouter un job à `ci.yml` ne l'ajoute pas aux checks requis du ruleset de
`main`. Au 2026-10-24, le ruleset « Main protection » (`~DEFAULT_BRANCH`)
exige 28 contextes ; un nouveau job n'en fait pas partie tant que l'opérateur
ne l'y ajoute pas. Qui le voit rouge dépend de qui merge :

- le moteur (`pr_merge_with_gate`) lit **tous** les checks de la tête depuis
  mika#2617 (`crates/mika-agent/src/tools/pr_merge_with_gate.rs:1096`). Un
  nouveau job rouge bloque donc déjà ses merges ;
- un merge par l'interface GitHub et `gh pr merge --auto` ne regardent que les
  checks requis. Pour eux, le nouveau job reste consultatif.

Quand un garde doit tenir contre **tous** les chemins de merge, la PR qui
l'ajoute doit nommer le geste opérateur qui le rend requis, et le moment où le
faire (après quelques runs verts sur `main`, pour ne pas bloquer le dépôt sur
un garde qui n'a encore jamais tourné).

## Why This Matters

Un rouge-avant qui sort vert est pire qu'un rouge-avant absent. Il laisse
croire que le garde ne rougit pas sur le défaut, donc qu'il est inutile ou mal
écrit, ce qui pousse à le « corriger » ou à l'abandonner. Il peut aussi se lire
comme une preuve, alors que l'arbre compilé n'était pas celui qu'on croyait.
Dans les deux cas la conclusion est fausse, et rien dans la sortie du job ne le
signale : le log montre un checkout et une compilation réussis.

## When to Apply

- Tout AC de type « rouge-avant » sur un job de CI, quand le défaut visé est
  déjà corrigé sur la branche par défaut.
- Plus largement, toute preuve qui dépend de l'arbre compilé par un job
  `pull_request` : se demander quelle base il fusionne.
- Ne s'applique pas quand le workflow a un `workflow_dispatch` qui accepte une
  ref (on lance alors le job sur la ref pré-correctif directement). Ne
  s'applique pas non plus quand le correctif n'est pas encore sur la base :
  une PR vers `main` voit alors l'arbre défectueux.

## Examples

Ce qui ne prouve rien (base = `main`, déjà corrigée) :

```
test/2902-rouge-avant (3993f91d + job)  ──PR──▶  main (porte le correctif #2900)
refs/pull/N/merge = tête ⊕ main  →  Cargo.toml en cfg(unix)  →  darwin-check vert
```

Ce qui prouve le rouge (base pré-correctif) :

```
test/2902-rouge-avant (3993f91d + job)  ──PR brouillon──▶  test/2902-rouge-avant-base (3993f91d)
refs/pull/N/merge = tête ⊕ base  →  Cargo.toml en cfg(target_os = "linux")  →  E0433 attendu
```

Sortie attendue, d'après le run `release.yml` 39961929051, qui a échoué au tag
avec le même montage `rustup` sur `macos-14` : trois `E0433` par cible, à
`crates/mika-agent/src/skills/executor.rs:1430:27`, `1430:51` et `1444:39`.
Le transcript réel du rouge-avant de mika#2902 n'est pas dans ce document. Il
sera dans le corps de la PR de mika#2902, une fois le run fait.

## Voir aussi

- `docs/solutions/build-errors/2026-10-23-libc-declare-par-cible-casse-au-premier-tag.md` :
  le défaut que ce garde attrape.
- `docs/solutions/best-practices/deux-definitions-du-meme-vert-produisent-un-main-rouge-2026-10-02.md` :
  pourquoi le moteur lit tous les checks et pas seulement les requis.
