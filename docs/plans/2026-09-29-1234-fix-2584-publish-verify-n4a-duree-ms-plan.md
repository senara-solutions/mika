---
title: N4a mesure sa durée en millisecondes - Plan
type: test
date: 2026-09-29
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# N4a mesure sa durée en millisecondes - Plan

## Goal Capsule

- Objective: le job CI **Publish Verify Lint** ne rougit plus sur une PR qui ne touche pas au script de vérification post-publish, et il continue de rougir si le sommeil terminal revient.
- Means: mesurer N4a en millisecondes, avec un seuil strictement sous le plancher de la mutation (KTD1, KTD2).
- Authority: mika#2584 (le constat), `scripts/verify-npm-publish.sh:296-305` (la géométrie réelle), puis ce plan.
- Stop conditions: si le contrôle négatif « sommeil inconditionnel » ne rougit plus N4a, s'arrêter — le correctif aurait désarmé la garde.
- Execution profile: un seul fichier de test, un pilote interactif, PR vers `senara-solutions/mika`.

## Product Contract

### Summary

N4a garde son rôle de seul test qui mesure le vrai `sleep`, mais lit l'horloge en millisecondes et compare à 2900 ms au lieu de 3 s entières.
Les commentaires du harnais expliquent l'arithmétique du seuil.

### Problem Frame

Le job **Publish Verify Lint**, introduit par mika#2566 (PR #2581), a échoué sur la PR #2583 qui ne touche pas le script : `FAIL: N4a: durée 3s ≥ 3s`.
N4a fait tourner 3 essais avec `BASE=1`, `CAP=1` : 2 sommeils d'1 s, soit environ 2,05 s.
L'assertion `ELAPSED < 3` lit `date +%s`, des secondes entières : un intervalle réel de 2,05 s qui commence à x,97 s et finit à x+3,02 s se lit 3.
Le faux rouge vient donc d'abord de la quantification, pas seulement de la charge du runner.

Le ticket propose deux voies, dont un seuil à 4 s.
Cette voie casse la garde : avec `CAP=1`, un sommeil terminal revenu coûte 3 × 1 s = 3,0 s au total, soit +1 s seulement sur le nominal, et passerait sous 4 s.
L'arithmétique du ticket (« ≥ 3 s de plus ») est fausse ; la voie retenue est la mesure en millisecondes.

### Requirements

- R1. N4a passe sur le chemin nominal même quand l'intervalle réel chevauche une frontière de seconde ou subit quelques centaines de millisecondes de démarrage.
- R2. N4a rougit toujours sous la mutation « sommeil rendu inconditionnel » (contrôle négatif existant de l'en-tête du harnais).
- R3. N4a reste une mesure du vrai `sleep`, sans doublure ; il n'est ni supprimé ni affaibli en test déterministe.
- R4. Une horloge incapable de fournir des millisecondes fait échouer N4a bruyamment au lieu de le laisser passer.

### Scope Boundaries

- Hors périmètre : `scripts/verify-npm-publish.sh` (le script gardé), `.github/workflows/ci.yml`, N4b et N4c.
- Hors périmètre : rendre N4a déterministe ; N4b et N4c le sont déjà via la doublure `sleep`.

## Planning Contract

### Key Technical Decisions

- KTD1. Lire l'horloge avec `date +%s%N` et calculer une durée en millisecondes. Rejeté : le seuil à 4 s du ticket, qui laisse passer la mutation (3,0 s < 4 s). Rejeté : `$EPOCHREALTIME`, dont le séparateur décimal dépend de la locale. Le runner est `ubuntu-22.04` (`.github/workflows/ci.yml:414`), GNU `date` y fournit `%N`.
- KTD2. Seuil à 2900 ms, strict. La mutation coûte au moins 3000 ms, puisque `sleep 1` garantit au moins une seconde chacun ; tout seuil < 3000 ms garde R2 rouge de façon déterministe. 2900 ms laisse environ 850 ms au démarrage de bash et des doublures pour R1, et 100 ms de marge sous le plancher de la mutation.
- KTD3. Valider que les deux lectures d'horloge ne contiennent que des chiffres ; sinon `ko` avec un message qui nomme la cause. Un `date` non GNU rend `%N` littéral, et une arithmétique shell sur cette valeur échouerait sous `set -e` sans message utile ou, pire, produirait un nombre trompeur.

## Implementation Units

### U1. Mesure en millisecondes et seuil sous le plancher de la mutation

**Goal:** N4a mesure en millisecondes et compare à 2900 ms, avec une garde de format d'horloge.

**Requirements:** R1, R2, R3, R4 ; KTD1, KTD2, KTD3.

**Dependencies:** aucune.

**Files:**
- `scripts/test-verify-npm-publish.sh` (bloc N4, et l'en-tête qui décrit N4a comme encadrement)

**Approach:**
1. Remplacer les deux lectures `date +%s` du bloc N4a par des lectures en nanosecondes, puis dériver la durée en millisecondes.
2. Avant l'arithmétique, vérifier que chaque lecture est entièrement numérique (KTD3) ; sinon émettre un `ko` explicite et ne pas évaluer le seuil.
3. Comparer à 2900 ms (KTD2) ; les messages `ok`/`ko` affichent la durée en ms et le seuil.
4. Mettre à jour le commentaire du bloc N4 : pourquoi les millisecondes (quantification), pourquoi 2900 (plancher 3000 de la mutation), et pourquoi pas 4 s.

**Patterns to follow:** les helpers `ok`/`ko` et le style de commentaires en français du harnais existant.

**Test scenarios:**
- Chemin nominal : la suite complète passe (30/30) sur la machine locale.
- Stabilité : N4a passe sur plusieurs exécutions consécutives de la suite (au moins 10), sans échec.
- Contrôle négatif : une copie de `verify-npm-publish.sh` sous `.pilot-scratch/` où le sommeil est rendu inconditionnel, passée en argument, fait rougir N4a, N4b et N4c, et eux seuls.
- Garde d'horloge : une horloge qui rend `%N` littéral fait émettre un `ko` nommé à N4a au lieu d'un succès.

**Verification:** la suite est verte ; le contrôle négatif rougit N4a ; la garde d'horloge a été vue rougir.

## Verification Contract

- `bash scripts/test-verify-npm-publish.sh` : sortie `30 passed, 0 failed` (ou le total courant, sans échec).
- `bash scripts/test-verify-npm-publish.sh .pilot-scratch/neg/<mutation>.sh` : N4a, N4b, N4c en `FAIL`, le reste vert.
- `bash scripts/verify-pipeline.sh` au pipeline `/mika`.
- Le job CI **Publish Verify Lint** de la PR est vert, et la durée nominale en ms que la ligne `ok` de N4a affiche est recopiée dans le corps de PR. Le faux rouge n'a été vu que sur le runner partagé : la stabilité locale ne prouve pas R1 à elle seule. Une durée CI au-delà de 2500 ms (moins de 400 ms de marge) est une halte à réexaminer avant merge.

## Definition of Done

- U1 fait, commité, avec le commentaire d'en-tête à jour.
- Contrôle négatif rejoué et consigné dans le message de commit ou le corps de PR.
- Aucun fichier de mutation laissé hors de `.pilot-scratch/` ; aucun code d'essai abandonné dans le diff.

## Acceptance criteria

- [ ] N4a mesure sa durée en millisecondes, et plus en secondes entières.
- [ ] Le seuil de N4a est strictement inférieur à 3000 ms (2900 ms).
- [ ] La suite `scripts/test-verify-npm-publish.sh` est verte sur au moins 10 exécutions consécutives en local.
- [ ] Le contrôle négatif existant (sommeil rendu inconditionnel) rougit toujours N4a.
- [ ] N4a n'est pas supprimé et utilise toujours le vrai `sleep`.
- [ ] Une horloge sans millisecondes fait échouer N4a avec un message explicite.
- [ ] Le job CI Publish Verify Lint de la PR passe, et la durée N4a mesurée en CI (ms) est consignée dans le corps de PR.
