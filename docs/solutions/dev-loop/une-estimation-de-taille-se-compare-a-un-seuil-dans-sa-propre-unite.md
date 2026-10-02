---
title: "Une estimation de taille se compare à un seuil exprimé dans sa propre unité, pas dans celle de la mesure"
date: 2026-10-02
category: dev-loop
module: skills/bundled/_shared/dispatch-lib.sh
component: development_workflow
problem_type: workflow_issue
severity: high
applies_when:
  - "un plan groomé porte une section « Taille estimée » et la porte de groom la compare au seuil `_plan_size_max_loc`"
  - "on recalibre `PLAN_SIZE_MAX_LOC` ou le littéral des deux prompts architecte"
  - "un groomeur estime le volume d'un livrable dont la moitié est un harnais shell (`test-dispatch-lib.sh`)"
symptoms:
  - "mika#2636 : plan estimé à 530 lignes de code, PR #2648 en porte 1 596 hors `docs/` et CLAUDE.md (×3,0)"
  - "mika#2641, même jour : estimé ≈ 550, livré 1 415 (×2,6), pilote mort au plafond de tours"
  - "la marge que le plan s'accordait (ratio 1,39 mesuré sur mika#1960 phase 2) a été dépassée d'un facteur 2"
root_cause: unit_mismatch_estimate_vs_measured_threshold
tags: [plan-size, taille-estimee, max-turns, groom, estimation, calibration, threshold, plafond-de-tours, measure-before-remedy]
related_components: [mika-arch-groom-ticket, mika-arch-second-review, dev-groom]
ticket: mika#2636
---

# Une estimation de taille se compare à un seuil exprimé dans sa propre unité, pas dans celle de la mesure

## Context

mika#2636 a rendu obligatoire une section « Taille estimée » dans tout plan groomé, et
a posé un seuil : au-delà de **1 000 lignes de code hors `docs/`**, le plan doit être
découpé en phases (`_plan_size_max_loc`, défaut `1000`,
`skills/bundled/_shared/dispatch-lib.sh:644` ; même littéral dans les deux
`system_prompt.md` architecte, tenu par le test S14).

Ce 1 000 vient d'une **mesure de diff** : les pilotes morts au plafond de tours
(mika#2161 ≈ 1 470 lignes, mika#2633 ≈ 1 170) avaient tous livré plus de ~1 000
lignes de code *mesurées*. Mais la porte le compare à un nombre **estimé** par le
groomeur avant toute ligne écrite. Les deux nombres ne sont pas dans la même unité.

Le premier point de données est la PR même qui pose la porte :

| livrable | estimé (plan) | 1er commit d'implémentation | après simplification + revue | ratio final |
|---|---|---|---|---|
| `dispatch-lib.sh` | 275 | 508 | 577 | ×2,1 |
| `test-dispatch-lib.sh` | 205 | 741 | 944 | **×4,6** |
| deux `system_prompt.md` | 50 | 61 | 61 | ×1,2 |
| `executor.rs` | 0 | 0 | 14 | — |
| **total** | **530** | **1 310** | **1 596** | **×3,0** |

Mesuré par `git show --numstat` commit par commit sur la branche
`feat/2636/groom-l-estimation-de-taille-devient-un` (PR #2648), `docs/` et
`CLAUDE.md` exclus comme le ticket le prescrit.

Trois constats sortent de ce tableau :

1. **L'écart est déjà là avant la revue.** Le premier commit d'implémentation pèse
   ×2,5 l'estimation. La simplification et la revue de code (deux P1, trois
   défauts de harnais, trois fixtures de régression) n'ajoutent que +22 %. Ce
   n'est pas la revue qui fait exploser le volume ; c'est l'estimation qui est
   fausse dès le départ.
2. **L'erreur se concentre sur les tests.** Le harnais shell porte des fixtures,
   des contrôles positifs et négatifs par assertion, et la densité de commentaire
   maison. L'estimation de 205 lignes pour « seize assertions + fixtures » valait
   ~13 lignes par assertion prévue ; la livraison en porte 944, soit ~59 par
   assertion prévue (la revue a ajouté des sondes S17, S18 et des bis, ce qui
   n'explique qu'une partie de l'écart : le premier commit portait déjà 741
   lignes de test). Les prompts, eux, sont estimés presque juste.
3. **La marge empruntée ne transférait pas.** Le plan corrigeait son estimation
   par le ratio 716/515 ≈ 1,39 mesuré sur mika#1960 phase 2, et en concluait
   « 737 lignes, toujours sous le seuil ». Un ratio mesuré sur un seul ticket, de
   composition différente, n'est pas un facteur de correction.

Le même jour, mika#2641 (estimé ≈ 550, livré 1 415, ×2,6, mesure MPC du
2026-10-02, PR #2647) est mort au plafond :
n=2 sur l'écart, dans le même sens et du même ordre.

## Guidance

**Un seuil qui gate une estimation doit être exprimé en unités d'estimation.** Si
la ligne de mort est ~1 000 lignes *livrées* et que les estimations sous-comptent
d'un facteur k, la porte qui lit l'estimation doit refuser vers 1 000 / k, pas à
1 000. Avec k ≈ 3 observé sur n=2, un plan estimé à 530 annonce ~1 600 lignes
livrées : au-dessus de la ligne de mort, et la porte le laisse passer.

Concrètement, jusqu'à ce que la distribution V4 (CLAUDE.md, *Sondes
post-déploiement*) soit lue :

- **Ne pas lire « sous 1 000 » comme « sûr ».** Un total estimé au-delà de ~350
  porte le risque de volume que le seuil est censé exclure.
- **Estimer les tests séparément, et par assertion.** Sur `test-dispatch-lib.sh`,
  compter ~60 lignes par assertion prévue (fixture, contrôle positif, contrôle
  négatif, commentaire), pas ~13. Un livrable de test estimé plus petit que le
  code qu'il teste est un signal d'alarme sur ce harnais, pas une économie.
- **Recalibrer par le ratio estimé/livré, pas par la mort.** La ligne
  `plan_size_estimate total_loc=…` (porte 3) est journalisée au dispatch ; la
  croiser avec `--numstat` du diff de la PR rend le facteur k sur la vraie
  distribution. C'est **le seuil** qui bouge selon k (halte 2 du CLAUDE.md), pas
  la mesure.
- **Se méfier d'une estimation qui atterrit juste sous le seuil qu'elle doit
  passer.** Le plan de mika#2636 écrivait que rester sous 1 000 « n'est pas une
  coïncidence de rédaction » : une estimation produite sous la contrainte du
  nombre qu'elle doit battre est ancrée par lui. La porte transforme l'estimation
  en cible, et une cible se rate dans le sens qui arrange.

## Why This Matters

La porte de mika#2636 existe pour qu'un pilote ne meure plus au plafond de tours
(35 à 90 USD par pilote, puis un spawn de reprise). Si l'estimation qu'elle lit
vaut un tiers du volume livré, elle valide précisément les plans qui tuent : deux
cas sur deux le jour de son écriture. Rien dans le code ne le dit — le seuil est
un littéral, l'estimation un entier, et la comparaison `-gt` est correcte. Le
défaut est dans l'unité, invisible à la lecture et aux tests.

La lecture inverse est aussi un piège : conclure que « la porte ne marche pas » et
la désarmer. La porte marche ; c'est son étalon qui est posé dans la mauvaise
unité. Le remède est un facteur mesuré, appliqué au seuil.

## When to Apply

- À la revue d'une section « Taille estimée » par l'architecte (première ou
  seconde passe) : juger le total contre ~1 000 / k, et exiger une ligne de tests
  dimensionnée par assertion.
- À toute révision de `_plan_size_max_loc` ou des littéraux S14 : partir du ratio
  estimé/livré mesuré sur les PR, jamais d'une nouvelle mort isolée.
- Au post-mortem d'un pilote mort au plafond : relever `total_loc` de la ligne
  `plan_size_estimate` et le diff livré, et ajouter le couple à la distribution.

## Examples

Lecture fausse, telle qu'elle figurait au plan de mika#2636 (§ 6) :

```
Total estimé : 530 lignes
Marge : 530 × 1,39 ≈ 737 — toujours sous le seuil, avec de la marge.
```

Lecture juste avec le facteur observé :

```
Total estimé : 530 lignes   (dont tests 205 pour 16 assertions ≈ 13 l/assertion)
Tests re-dimensionnés : 16 × ~60 ≈ 950
Livraison attendue : ~1 300–1 600 → au-dessus de la ligne de mort → découper en phases
```

Livré : 1 596.

## Related

- CLAUDE.md, section d'observabilité de l'estimation de taille (sondes 1 à 4,
  halte 2 « c'est le seuil qui bouge ») — le thermomètre ; ce document est la
  première lecture qu'il produit.
- `docs/solutions/best-practices/sealing-a-pre-registered-estimand-2026-08-29.md` —
  sceller ce qu'on mesure avant de le mesurer.
- `docs/solutions/dev-loop/un-marqueur-de-completude-doit-lire-si-la-session-a-conclu-2026-09-29.md` —
  un pilote tué à 151/150 tours, la classe de mort que cette porte vise.
