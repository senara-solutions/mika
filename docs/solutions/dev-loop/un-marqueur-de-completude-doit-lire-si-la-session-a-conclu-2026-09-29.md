---
title: "Un marqueur de complétude doit lire si la session a conclu, pas seulement ce qu'elle a laissé"
date: 2026-09-29
category: dev-loop
module: skills/bundled/_shared/dispatch-lib.sh
component: development_workflow
problem_type: logic_error
severity: high
symptoms:
  - "PR de rescue #2556 portant `<!-- rescue-pipeline-verified: yes -->` alors que son pilote a été tué à 151/150 tours"
  - "corps de rescue affirmant « measured the local pipeline as complete » sur une session `STATUS=terminated`"
  - "qa-review Step 1.5 ouvre la porte sur le marqueur `yes` ; aucun `docs/solutions` dans le diff"
root_cause: missing_validation
resolution_type: code_fix
tags: [loop-substrate, wip-rescue, rescue-pipeline-verified, false-green, fire-disposition, compound, measure-before-remedy]
related_components: [qa-review, wip_rescue, skills-executor]
ticket: mika#2563
---

# Un marqueur de complétude doit lire si la session a conclu, pas seulement ce qu'elle a laissé

## Problem

`_measure_pipeline_verified` (mika#2354) écrit `rescue-pipeline-verified: yes` quand cinq
termes tiennent sur l'**artefact** laissé par le pilote (diff non vide, worktree propre,
`fmt`, `clippy`, `verify-pipeline.sh`). Aucun des cinq ne lit **`STATUS`**. Une session
que le SDK a tuée au plafond de tours pouvait donc être présentée comme « pipeline complet »,
et la porte QA s'ouvrait dessus.

## Symptoms

- #2556 : `<!-- rescue-pipeline-verified: yes -->`, pilote tué à 151/150 tours.
- Corps : *« measured the local pipeline as complete »* — sur `status: terminated`,
  `subtype: error_max_turns`.
- Rien ne lit le trou en aval : qa-review Step 1.5 fait confiance au marqueur
  (`skills/bundled/qa-review/system_prompt.md:136`), et son contrôle compound Step 4 est
  délibérément *soft* (`:608`).

## What Didn't Work — les trois remèdes du ticket, mesurés avant d'être écrits

Le ticket proposait trois remèdes. La mesure en a invalidé deux et déplacé le troisième.
C'est la partie non recouvrable depuis le code final, et la raison d'être de ce doc.

1. **« Exiger un `docs/solutions/*.md` dès que le diff porte du source », dans
   `scripts/verify-pipeline.sh`.** Mesuré sur les 100 derniers merges `--first-parent` de
   `origin/main` : 93 portent du source, **66 d'entre eux (71 %) n'ont aucun
   `docs/solutions/`**. C'est le régime, pas un défaut. Et `verify-pipeline.sh` est à la
   fois le job CI `pipeline-artifacts` **et** le terme 5 de `_measure_pipeline_verified` :
   la modification aurait refusé deux PR nominales sur trois **et** fait basculer tout
   rescue à `no`, donc `hold[review]` systématique. **Le remède écrit arrêtait la boucle.**
2. **« Le pilote aurait atteint l'étape 6 `/ce:compound` s'il n'était pas mort ».** Faux :
   `_detect_plan_on_branch` (`dispatch-lib.sh:8640`, mika#1074) override l'entrée de tout
   ticket groomé vers `/ce-work <plan>`, dont le périmètre exclut la queue d'expédition.
   Le pilote de #2555/#2556 n'aurait pas compoundé même vivant. Les 29 % qui portent un
   learning le tiennent de leur **plan**, pas de l'étape 6.
3. **« Ajouter un marqueur `<!-- compound: missing -->` lu par la QA ».** Déjà livré sous un
   autre nom : `<!-- rescue-verify-failed: <terme> -->` + Step 1.5. Ce qui manquait n'était
   pas un marqueur mais **un terme qui échoue**. Un second marqueur aurait été une seconde
   vérité sur un même fait.

Les rescues ne sont pas non plus une population à part : ~1,1/jour, et 8 sur 9 sans
`docs/solutions` — du même ordre que le nominal. La différence rescue/nominal n'explique
rien ; seule la **troncature** distingue la population défectueuse.

## Solution

Un terme 0, `compound-traversal`, en tête de la conjonction de
`_measure_pipeline_verified` (`dispatch-lib.sh:8265`), classé par
`_rescue_compound_traversal` (`:8081`) :

| condition | valeur | effet |
|---|---|---|
| `STATUS = success` | `not-applicable` | satisfait — la session a conclu, y compris `no-shipping-tail` |
| sinon, un `docs/solutions/**/*.md` dans `origin/main...HEAD` | `attested-solution` | satisfait |
| sinon, trailer ancré `^Compound: none[[:space:]]+.+$` dans `origin/main..HEAD` | `attested-trailer` | satisfait |
| sinon (y compris git illisible, `origin/main` absent, dir vide) | `absent` | échoue **si armé** |

- **Refus sur la troncature, jamais sur l'absence de fichier** : zéro faux positif sur le
  nominal, qui conclut en `success` par construction (`_pilot_had_no_shipping_tail`, `:4776`,
  l'exige déjà).
- **Fail-closed** : `STATUS` vide/illisible = non conclu.
- **Détection inconditionnelle, disposition gatée** : `MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL`,
  défaut désarmé (`_rescue_require_compound_traversal`, `:8171`), relayé par
  `RESCUE_VERIFY_ENV` dans `crates/mika-agent/src/skills/executor.rs` — sans quoi
  `sandboxed_pilot_env` (allowlist positive) le rendrait décoratif.
- **Le marqueur `<!-- compound-traversal: <valeur> -->` est écrit dans le corps, armé ou
  non** (`_compose_rescue_pr_body`, `:8500`, 8ᵉ argument). Argument vide ⇒ corps
  byte-identique, donc `MIKA_RESCUE_VERIFY_ENABLED=0` reste un vrai rollback.
- Le trailer `Compound: none — <raison>` est documenté à l'étape 6 de
  `.claude/commands/mika.md` ; les prompts `self-dev-iterate` et `self-dev-webhook-qa`
  demandent la traversée sur une PR `wip-rescue` — moitié d'intention, écrite comme ne
  tenant pas seule.

## Why This Works

La phrase qui condamnait le trou était déjà dans le fichier, trente lignes au-dessus,
écrite par mika#2492 pour la **classification** (`dispatch-lib.sh:4770`) :

> *« Truncated work must not be presented as complete just because its perimeter had no
> shipping tail. »*

mika#2354 ne l'avait pas appliquée au **marqueur**. Un marqueur de complétude construit
uniquement sur des propriétés de l'artefact répond à « ce qui est là est-il sain ? »,
jamais à « est-ce tout ? ». Seul le statut de conclusion de la session répond à la
seconde question ; le terme 0 le lit, au même endroit et par le même test que
`_pilot_had_no_shipping_tail`.

Le marqueur va dans le **corps** et pas sur stderr parce que le stderr de
`_post_flight_recovery` est un `Stdio::piped()` que l'exécuteur ne lit que sur échec : un
détecteur désarmé dont la sortie tombe là serait muet (classe mika#2205), et la condition
d'armement deviendrait intuitive au lieu d'être mesurable.

## Prevention

- **Tout marqueur « complet/vérifié » doit avoir un terme qui lit la conclusion du
  producteur** (statut, exit, subtype), pas seulement l'état de ce qu'il a laissé.
  Chercher, pour chaque nouveau marqueur, la phrase *« qu'arrive-t-il si le producteur a
  été tué au milieu ? »*.
- **Mesurer un remède contre le trafic nominal avant de l'écrire.** Un prédicat ajouté à un
  script partagé (ici `verify-pipeline.sh`, CI + rescue) mord toute sa population. Le
  compte « combien de PR nominales ce prédicat refuserait » se fait en une boucle `git log`
  sur `origin/main` ; sans lui, « le terme mord » est indistinguable de « le terme mord tout
  le monde ».
- **Contrôle négatif du nominal obligatoire** : T15e (`STATUS=success`, pas de
  `docs/solutions` ⇒ `not-applicable`, aucun refus) dans
  `skills/bundled/_shared/tests/test_rescue_pipeline_verified.sh`, suite exécutée par CI —
  **pas** dans `scripts/verify-pipeline-test.sh`, qu'aucun job n'exécute.
- **Armer sur mesure, pas sur intuition** : après ≥ 7 jours et ≥ 10 rescues, compter sur
  l'hôte les corps `compound-traversal: absent` (et le contrôle positif : combien portent le
  marqueur du tout) ; les trois haltes sont dans le plan
  `docs/plans/2026-09-28-002-fix-2563-traversee-compound-sur-session-tronquee-plan.md`
  § Fire-Disposition. Contrôle positif nul ⇒ vérifier que le binaire servi porte le
  correctif avant de conclure (mika#2340).
- **Question restée ouverte** : la boucle autonome ne pose, sur aucun chemin, la question
  « y a-t-il une leçon ? » (`/ce-work` n'a pas d'étape compound). Ce correctif empêche une
  session tuée de s'affirmer complète ; il ne fait écrire aucun learning.

## Related

- [Un déscopage qui garde la conséquence de l'étape supprimée](un-descopage-qui-garde-la-consequence-de-letape-supprimee-2026-09-16.md) — mika#2286, le démon `wip_rescue` et le marqueur `no`.
- [Rescue net closes without looking at what it captured](rescue-net-closes-without-looking-at-what-it-captured-2026-09-04.md) — mika#2157, prédicat `rescue-diff` réutilisé par le terme 1.
- [Fire-Disposition Doctrine](../best-practices/fire-disposition-doctrine.md) — option (b) livrer désarmé.
- [Une garde que rien n'invoque se lit exactement comme un arbre propre](../best-practices/une-garde-que-rien-ninvoque-se-lit-exactement-comme-un-arbre-propre-2026-09-28.md) — pourquoi les tests vivent dans la suite exécutée par CI.
