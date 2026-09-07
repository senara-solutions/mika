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
(mesuré la nuit 09-06→07 : le tour initial de mika-qa a lancé un build donc appelé le LLM, et n'a
rien loggé). Sans capture des tool-calls d'un agent TUI, #2218 est aveugle (impossible de voir si
`run_gh pr review` est appelé).

## Candidat root-cause (à confirmer par dogfood — Phase 1)

`init` (daemon) et `init_pretty` (agent) appliquent la MÊME directive `mika::llm_debug=debug` au
filtre global — en lecture statique, identiques. La divergence est runtime. Candidat principal :
le **default_level**. Le chemin `--agent` (main.rs) calcule `log_level` via `resolve_log_level`,
dont le défaut est **`"warn"`** (`.unwrap_or_else(|| "warn")`), alors que le daemon diffère.
Hypothèse : le base-level `warn` interagit avec le hint de niveau max de tracing de façon à
supprimer les events DEBUG `mika::llm_debug` malgré la directive spécifique — OU un autre écart
init/init_pretty. Autres candidats à écarter : couche `DevFormat` (arm PrettyAndFile, pas
FileOnly), `EnvFilter::add_directive` qui ne recompute pas le max-level hint.

## Décision de conception

Approche **investigate-then-fix**, avec repli Langfuse si le fix in-process n'est pas propre.

## Acceptance criteria

- **AC1** : un test (dogfood) exerce `init_pretty(FileOnly, log_llm_bodies=true, default_level)`,
  émet un event `debug!(target: "mika::llm_debug", …)`, et **assert** qu'il atterrit dans le
  fichier JSON. Ce test échoue AVANT le fix (rouge-avant) et passe après. (pin le root-cause)
- **AC2** : le fix rend la capture effective pour un agent TUI — sur mika-qa avec
  `MIKA_LOG_LLM_BODIES=true`, un tour LLM produit `llm request/response body` dans
  `~/.mika/agents/<agent>/logs/mika.log.<date>`. Parité avec le daemon.
- **AC3** (repli, conditionnel) : si le fix in-process s'avère non trivial/fragile, activer
  Langfuse/OTEL sur les agents comme surface de capture des tool-calls, documenté + testé.

## Phases

1. **Dogfood root-cause** : écrire le test AC1 ; observer si la directive `mika::llm_debug=debug`
   est effective sous `default_level="warn"` + FileOnly. Confirmer/infirmer le candidat.
2. **Fix** : selon la Phase 1 — corriger l'écart (ex. garantir que la directive llm_debug survit
   au base-level, ou aligner init_pretty sur init) OU basculer sur Langfuse (AC3).
3. **Vérif parité** : capture effective sur agent TUI (AC2).

## Hors périmètre

- Le fix de #2218 lui-même (composer+poster le verdict) — débloqué PAR ce ticket, traité après.
- Le volume de log (la var reste opt-in ; on ne l'active que pour capturer).

## Fichiers probables

- `mika/crates/mika-common/src/logging.rs` (init_pretty / filtre)
- `mika/crates/mika-cli/src/main.rs` (resolve_log_level / default_level) — si c'est la cause
- tests dans `mika-common` (AC1)
