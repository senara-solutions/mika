---
issue: 2220
type: fix
title: "MIKA_LOG_LLM_BODIES inefficace pour agents TUI (init_pretty FileOnly)"
status: groomed-pending
---

# Plan — mika#2220 : rendre les tool-calls d'un agent TUI capturables (prérequis #2218)

## Contexte

`MIKA_LOG_LLM_BODIES=true` fait loguer les corps LLM (req + réponses = tool-calls) pour le daemon
mika-spirit (`logging::init`), mais PAS pour un agent TUI (mika-qa, `logging::init_pretty` +
`LogOutput::FileOnly`) : zéro `llm body`, zéro DEBUG dans le fichier, même sur des tours LLM avérés
(mesuré la nuit 09-06→07). Sans capture des tool-calls d'un agent TUI, #2218 est aveugle.

## Candidat root-cause (à confirmer par dogfood — Phase 1)

`init` et `init_pretty` appliquent la MÊME directive `mika::llm_debug=debug` au filtre global — en
statique, identiques ; la divergence est runtime. Candidat principal : le **default_level**. Le
chemin `--agent` (main.rs) calcule `log_level` via `resolve_log_level`, défaut **`"warn"`**, alors
que le daemon diffère. Hypothèse : le base-level `warn` supprime les events DEBUG `mika::llm_debug`
malgré la directive spécifique (interaction max-level hint). Autres candidats à écarter : couche
`DevFormat`, `add_directive` qui ne recompute pas le hint.

## Décision de conception (révisée après mika-arch first-pass)

**Périmètre = fix in-process uniquement** (root-cause via dogfood → fix dans logging). **Langfuse
est SCINDÉ hors de ce ticket** (F1/F3) : il ne sera envisagé (ticket séparé) QUE si la Phase 1
établit que le root-cause exige une **refonte de l'architecture du filtre de logging** (critère
objectif de bascule) — pas un jugement « si c'est fragile ». Dans ce cas précis, ce ticket
ESCALADE avec le constat, et le fix Langfuse devient son propre ticket.

## Acceptance criteria

- **AC1** : test (dogfood) qui exerce `init_pretty(FileOnly, log_llm_bodies=true, default_level)`,
  émet `debug!(target:"mika::llm_debug", …)`, et **assert** qu'il atterrit dans le fichier JSON.
  **Rouge sur le code actuel** (échoue avant fix), **vert après**. Pin le root-cause.
- **AC2** : le fix rend la capture effective pour un agent TUI — sur mika-qa avec
  `MIKA_LOG_LLM_BODIES=true`, un tour LLM produit `llm request/response body` dans son fichier de
  log. Parité avec le daemon.

## Fire-Disposition

Le livrable détecteur de AC1 est un **test unitaire de non-régression dans `mika-common`**,
exécuté par le gate CI **`Check`** (bloquant). Disposition : **rouge sur `main` actuel** (la
directive ne lande pas → assertion « le body est dans le fichier » échoue), **vert après le fix**.
Il reste ensuite en garde permanente : toute régression future qui re-casse la capture DEBUG des
agents TUI refait échouer `Check`. (Conforme mika#1574.)

## Phases

1. **Dogfood root-cause** : écrire le test AC1, observer la directive `mika::llm_debug=debug` sous
   `default_level="warn"` + FileOnly ; confirmer/infirmer le candidat.
2. **Fix in-process** : corriger l'écart révélé (ex. garantir la survie de la directive llm_debug
   au base-level, ou aligner init_pretty sur init). Si — et seulement si — la Phase 1 montre que
   cela exige une refonte de l'architecture du filtre → ESCALADE (voir Décision de conception).
3. **Vérif parité** : capture effective sur agent TUI (AC2).

## Hors périmètre

- Le fix de #2218 (composer+poster le verdict) — débloqué PAR ce ticket, traité après.
- Langfuse/OTEL — scindé (ticket séparé, uniquement sur escalade objective de Phase 2).
- Le volume de log (la var reste opt-in).

## Fichiers probables

- `mika/crates/mika-common/src/logging.rs` (init_pretty / filtre) + tests (AC1)
- `mika/crates/mika-cli/src/main.rs` (resolve_log_level / default_level) — si c'est la cause
