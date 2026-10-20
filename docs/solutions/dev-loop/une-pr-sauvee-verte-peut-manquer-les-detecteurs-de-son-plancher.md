---
title: Une PR sauvée verte peut manquer les détecteurs de son plancher
date: 2026-10-20
category: dev-loop
module: skills/bundled/_shared/dispatch-lib.sh
problem_type: workflow_issue
component: development_workflow
severity: medium
applies_when:
  - "Une PR draft ouverte par le sauvetage wip (mika#1383 / mika#1396) doit être promue, relue ou finie"
  - "Le plan du vol déclare un plancher d'unités non retranchables (section « Ordre de commit »)"
  - "Les unités qui manquent sont des tests, des détecteurs ou des contrôles négatifs"
  - "Un commentaire du code livré renvoie à une suite ou une PART qui doit « tenir » l'invariant"
tags:
  - wip-rescue
  - plan-floor
  - plancher
  - detector
  - anti-vacuite
  - forward-reference
  - draft-pr
  - turn-ceiling
related_components:
  - wip_rescue
  - dispatch-lib
  - mika-qa
---

# Une PR sauvée verte peut manquer les détecteurs de son plancher

## Context

Le vol 2 de mika#2108 a été sauvé par le mécanisme wip (mika#1383 / mika#1396) :
le pilote a commité et poussé, puis son tour s'est terminé avant `gh pr create`,
et dispatch-lib a ouvert la PR draft #2842 avec un commit marqueur
`wip(mika#1383): auto-PR-create rescue`.

Le plan (`docs/plans/2026-10-20-001-fix-2108-faire-dire-la-verite-au-squelette-plan.md`)
fixait un ordre de commit en cinq unités et le déclarait en toutes lettres
(ligne 598) : *« Plancher du vol : U1 + U6 + U2 + U3 + U4 — les cinq, et aucun
n'est retranchable. »* La PR sauvée portait U1 (la fonction), U6 (la
re-spécification d'un lecteur de test) et U2 (l'injection dans les deux blocs
`bwrap`). Il lui manquait **U3** (PART E de `test_pilot_orientation_block.sh`,
qui tient la symétrie des deux blocs) et **U4** (le contrôle négatif réel dans
`test_sandbox_git_usable.sh`). Cinq critères d'acceptation sur huit (AC1, AC2,
AC3, AC4, AC6) n'étaient donc tenus par aucun test.

Trois choses rendaient le manque invisible :

1. **Toute la CI était verte, et elle ne pouvait pas être autre chose.** Les
   suites existantes passaient. Un détecteur qui n'a jamais été écrit ne peut
   pas rougir : l'absence de U3/U4 ne laisse aucune trace dans un run.
2. **L'ordre de commit place les détecteurs en dernier.** U3 et U4 mesurent
   l'état final, donc le plan les met après l'implémentation. Un pilote coupé en
   cours de vol perd précisément ces unités-là, et ce qui reste ressemble à un
   vol complet.
3. **Le code livré parlait déjà du détecteur comme d'un fait.** Le commentaire
   posé par U2 au-dessus des deux blocs (`dispatch-lib.sh:3020-3021`) disait :
   *« c'est PART E de tests/test_pilot_orientation_block.sh qui la tient »*.
   PART E n'existait pas. Un relecteur qui lit le code y trouve une garde
   annoncée et n'a aucune raison d'aller vérifier qu'elle existe.

Un premier spawn a établi le manque (session history) : *« Le diff ne touche
aucun des deux fichiers de test de U3/U4 »*, puis il s'est arrêté au lieu de
corriger. Un second spawn a livré U3 et U4 sur la même PR.

## Guidance

**Avant de promouvoir, relire ou finir une PR sauvée, confronte le plancher du
plan à la liste des commits. Ne te fie ni à la CI, ni au récit des messages de
commit.**

Concrètement :

1. Lis la section « Ordre de commit » (ou son équivalent) du plan, et relève le
   plancher et, pour chaque unité, les fichiers qu'elle nomme.
2. Liste ce que la branche touche vraiment :
   `git diff --name-only origin/main...HEAD` et `git log --oneline origin/main..HEAD`.
3. Pour chaque unité du plancher, cherche un commit qui la porte **et** le
   fichier qu'elle nomme dans le diff. Une unité de test dont le fichier n'est
   pas dans le diff est absente, quel que soit le reste.
4. Cherche dans le diff les **références anticipées** : un commentaire ou un
   message de commit qui nomme une suite, une PART ou un test censé « tenir »,
   « garder » ou « attraper » l'invariant. Vérifie que la cible existe :
   `grep -n "PART E" <suite>`. Une référence à un détecteur absent est le signe
   le plus sûr d'un vol tronqué.
5. Quand tu livres les unités manquantes, prouve qu'elles savent dire non : mute
   une copie du code (après un commit vert) pour régresser l'invariant, montre
   que le nouveau détecteur rougit, puis restaure et vérifie que l'arbre est
   propre.

**Une prévision du plan sur le lieu de vérification ne vaut que pour le lieu
qu'elle décrit.** Le plan (§ D5) annonçait que `test_sandbox_git_usable.sh`
sortirait en `⊘ skipped — could not create the probe worktree` depuis un
worktree de dispatche imbriqué, donc que la moitié `bwrap` réelle de U4 ne
serait vérifiée qu'en CI. Depuis un spawn hôte, sur un worktree rangé sous
le `.claude/worktrees` du méta-dépôt (hors de ce dépôt-ci), `git worktree add` réussit, `bwrap` est réel, et la suite a
tourné pour de vrai (35 passed / 0 failed). Lance la suite avant d'accepter un
skip qu'un plan a prédit pour un autre contexte.

## Why This Matters

Le plancher existe parce que l'implémentation sans ses détecteurs est
précisément la régression que le plan veut empêcher. Dans ce vol, la symétrie
des deux blocs `bwrap` n'était gardée par rien : retirer l'injection du bloc
Phase 2a (le repli, qui ne tourne qu'un jour de panne) n'aurait rougi aucune
suite. La mutation faite à la livraison de U3 l'a montré : sans PART E, seule
la moitié Phase 2b est mesurée, par `test_sandbox_git_usable.sh`. Le bloc 2a
n'est tenu que par E2 et E3.

Une PR sauvée et verte qui manque son plancher a l'air mergeable. Si on la
merge, on livre une garde annoncée par un commentaire et tenue par rien, et la
première régression passe la CI sans bruit.

## When to Apply

- Chaque fois qu'on reprend une PR portant un commit `wip(mika#1383)` ou ouverte
  par le sauvetage, avant tout « ton merge » ou tout undraft.
- Chaque fois qu'un pilote a atteint son plafond de tours sur un plan dont les
  dernières unités sont des tests.
- En revue (mika-qa ou orchestrateur), quand un commentaire du diff renvoie à un
  test : la référence se vérifie, elle ne se croit pas.

## Examples

Le geste qui a trouvé le manque sur #2842, réduit à l'essentiel :

```bash
# Le plancher, tel que le plan l'écrit
grep -n "Plancher du vol" docs/plans/2026-10-20-001-fix-2108-*.md
# → U1 + U6 + U2 + U3 + U4

# Ce que la branche touche vraiment
git diff --name-only origin/main...HEAD
# → dispatch-lib.sh, test_sandbox_no_secret_in_argv.sh, canary-pilot-containment, le plan
#   (ni test_pilot_orientation_block.sh, ni test_sandbox_git_usable.sh : U3 et U4 manquent)

# La référence anticipée, et sa cible
grep -n "PART E de" skills/bundled/_shared/dispatch-lib.sh             # le code l'annonce
grep -c "PART E" skills/bundled/_shared/tests/test_pilot_orientation_block.sh   # → 0
```

La preuve de capacité d'échec des détecteurs livrés, sur arbre commité :

```bash
sed -i "${L2a}d" skills/bundled/_shared/dispatch-lib.sh   # retire le site Phase 2a
bash skills/bundled/_shared/tests/test_pilot_orientation_block.sh
# → ✗ E2 … ✗ E3 …   (et test_sandbox_git_usable.sh reste vert : il ne mesure que 2b)
git checkout -- skills/bundled/_shared/dispatch-lib.sh && git diff --quiet
```

## Related

- `docs/solutions/best-practices/un-plancher-danti-vacuite-peut-se-rembourrer-avec-le-trou-quil-garde.md` : même famille (vert avec un détecteur vide ou absent, la mutation comme preuve)
- `docs/solutions/dev-loop/un-essai-dabsence-exige-un-controle-positif-sur-son-instrument.md` : le contrôle positif qui rend un essai d'absence non vide
- `docs/solutions/best-practices/dispatch-lib-finalize-pr-gate-2026-08-22.md` : la garde du passage draft → ready porte sur l'état de la PR, pas sur la complétude du plancher
- mika#2108, PR #2842 ; mécanisme de sauvetage mika#1383 / mika#1396
