---
title: "fix(ci): verify-pipeline-test ne fuit plus GITHUB_EVENT_PATH du runner"
type: fix
status: active
date: 2026-11-13
ticket: senara-solutions/mika#2983 (Refs, pas de fermeture)
---

# fix(ci) : `verify-pipeline-test.sh` ne fuit plus `GITHUB_EVENT_PATH` du runner

Exception au gel des tickets, décidée par Vincent le 2026-11-13. Aucun ticket ouvert.

## Problème

Les PR dependabot de mika (3063, 2915, 2663, 2662) ont toutes 13 FAIL dans
`CI / Offline Script Suites`, tous dans `bash scripts/verify-pipeline-test.sh`.

- `run_verify()` ne pose pas `GITHUB_EVENT_PATH`, et le commentaire qui suit le
  dit « délibéré » (base fermée par défaut, F3). Mais il ne la **retire** pas.
- En CI, le runner pose `GITHUB_EVENT_PATH` sur l'événement réel de la PR.
  `scripts/verify-pipeline.sh` lit `.pull_request.user.login` contre
  `AUTOMATED_PR_AUTHORS`.
- Sur une PR de dependabot, chaque cas de base prend donc l'exemption
  `[pipeline-exempt: automated-author] dependabot[bot]` et sort à 0 : 13 cas
  qui attendent un refus deviennent rouges.

Reproduit sur 80cce6bc : sans la variable, rc=0 et 0 FAIL ; avec un événement
`dependabot[bot]`, rc=1 et 13 FAIL.

## Approche

1. **Garde d'abord, rouge sur main.** Un cas F3b exporte `GITHUB_EVENT_PATH`
   vers un événement `dependabot[bot]`, appelle `run_verify` sur une PR
   code-only et exige `[pipeline-exempt: none] REJECT: code-only PR`, puis
   retire la variable. Sur le code de main il doit être rouge.
2. **Correction.** `run_verify` lance `verify-pipeline.sh` sous
   `env -u GITHUB_EVENT_PATH`. Le commentaire de `run_verify_with_event` dit la
   vérité : la base retire la variable héritée du runner, sinon une PR de
   dependabot est exemptée dans chaque cas de base. `run_verify_with_event`
   ne change pas.
3. **F3** passe par `run_verify` (vérifié) ; il hérite donc de la correction.

Seul fichier de code touché : `scripts/verify-pipeline-test.sh`. Ni
`scripts/verify-pipeline.sh`, ni `.github/workflows/`.

## Definition of Done

- La suite est verte avec et sans `GITHUB_EVENT_PATH` pointé sur un événement
  `dependabot[bot]`.
- Le garde F3b est rouge sur main et vert après la correction.
- Au plus 40 lignes ajoutées hors ce plan.

## Acceptance criteria

- [ ] `GITHUB_EVENT_PATH=<événement dependabot[bot]> bash scripts/verify-pipeline-test.sh` sort à 0 avec 0 FAIL.
- [ ] `env -u GITHUB_EVENT_PATH bash scripts/verify-pipeline-test.sh` sort à 0 avec 0 FAIL.
- [ ] Le garde F3b échoue sur le `run_verify` de main (contrôle négatif montré) et passe après la correction.
- [ ] Seul `scripts/verify-pipeline-test.sh` change côté code ; `run_verify_with_event` est inchangé.
