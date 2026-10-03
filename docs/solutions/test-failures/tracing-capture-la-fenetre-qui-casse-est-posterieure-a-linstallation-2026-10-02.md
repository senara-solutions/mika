---
title: "Capture tracing vide : la fenêtre qui casse est postérieure à l'installation, pas antérieure"
date: 2026-10-02
category: test-failures
module: crates/mika-agent (tests de capture tracing)
problem_type: test_failure
component: testing_framework
symptoms:
  - "Un test qui capture un événement tracing voit 0 événement, de façon intermittente, surtout sur le job CI « Test with telemetry feature »"
  - "L'échec se déplace d'un test de capture à l'autre selon les runs"
root_cause: test_isolation
resolution_type: test_fix
severity: medium
tags: [tracing, interest-cache, rebuild-interest-cache, set-default, serial-test, flakiness, negative-control, mika-2646, mika-2633]
---

# Capture tracing vide : la fenêtre qui casse est postérieure à l'installation

## Problème

`teams::engine::tests::mika2633_la_ligne_de_refus_nomme_lequipe_et_le_redacteur`
voyait parfois 0 ligne de refus (PR #2643, run `37037943870`). Le ticket et
`crates/mika-common/tests/llm_retry.rs` décrivaient le remède :
`rebuild_interest_cache()` après `set_default`. Ce remède ne ferme pas la
fenêtre qui casse.

## Ce qui a été mesuré (binaire de test réduit à un seul test, `--exact`)

| Scénario | Capture |
|---|---|
| A. callsite atteint **sans abonné, avant** `set_default` ; puis `set_default` nu, sans rebuild | **1** |
| B. `set_default` d'abord ; callsite atteint **pour la première fois** depuis un autre thread sans abonné | **0** |
| B, puis `rebuild_interest_cache()` | **1** |

- **A est déjà réparé par `set_default`** : enregistrer un dispatcher
  recalcule l'`Interest` des callsites déjà connus. Le rebuild juste après
  l'installation est donc **redondant**. On le garde quand même (il est
  inoffensif et suit `llm_retry.rs`), mais ce n'est pas lui qui protège.
- **B est la vraie course.** Un callsite s'enregistre la première fois qu'il
  est atteint. Si un seul dispatcher est vivant dans le processus, son
  `Interest` est décidé par le dispatcher **du thread qui l'atteint**. Un
  voisin sans abonné l'éteint alors pour tout le processus, y compris pour la
  capture déjà installée. Seul un rebuild *postérieur* le rallume.

## Solution

- La moitié qui porte le correctif : `#[serial_test::serial]` sur **tous** les
  tests qui atteignent le callsite capturé, ceux qui capturent comme ceux qui
  l'atteignent sans abonné (`#[serial]` ne sérialise que contre les autres tests
  `#[serial]`). Dans ce module, ce sont tous les tests qui appellent
  `commit_deliverable` / `drive_deliver_phase` (`crates/mika-agent/src/teams/engine.rs`).
- L'installation passe par un seul site,
  `test_utils::test_helpers::install_capturing_subscriber`, figé par le scan
  `mika2646_set_default_a_un_site_dinstallation_unique`.

## Contrôle négatif : n'empoisonner qu'après l'installation, et dans un processus enfant

- Le contrôle intuitif (empoisonner, *puis* installer, et attendre un rouge
  sans rebuild) est **vert dans les deux cas** (scénario A). Il ne prouve rien.
- Le scénario B n'est déterministe que si **aucun autre dispatcher n'est
  vivant**. S'il y en a un autre, le callsite est enregistré auprès de tous les
  dispatchers vivants et n'est pas éteint. Dans le binaire de test parallèle,
  le contrôle serait lui-même aléatoire.
- Forme retenue :
  `mika2646_un_callsite_eteint_apres_installation_exige_un_rebuild`
  (`crates/mika-agent/src/teams/engine.rs`). Le test se relance lui-même via
  `std::env::current_exe()` avec `--exact <chemin> --test-threads=1` et une
  variable d'environnement de garde. Le parent exige `status.success()` **et**
  `"1 passed"` (un filtre qui ne correspond plus à rien donne `0 passed`, donc
  rouge). Vérifié : commenter le rebuild final le fait rougir sur l'assertion
  « rallume ».

## Prévention

- Avant d'écrire un contrôle négatif d'un cache global, mesurer **les deux
  ordres** (empoisonnement avant et après l'installation). Un contrôle qui ne
  rougit dans aucun ordre est une décoration.
- Limites connues, nommées et non couvertes : les captures de `panic_hook`,
  `builtin_handlers` et `kg/resolver_tick` passent par l'installateur mais ne
  sont pas `#[serial]`. Rien ne fige non plus le `#[serial]` des tests qui
  atteignent `commit_deliverable` : un nouveau test sans l'attribut rouvrirait
  la course sans rien faire rougir. Trois relecteurs ont relevé ce point
  indépendamment ; c'est un candidat de suivi (un scan qui exige l'attribut),
  dont la précondition est un rouge mesuré.
