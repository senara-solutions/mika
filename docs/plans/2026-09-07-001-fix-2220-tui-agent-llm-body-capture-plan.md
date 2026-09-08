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

## Phase 1 — résultat du dogfood : le candidat ci-dessus est INFIRMÉ

**Mesuré, pas déduit.** `init_pretty("warn", Some(dir), LogOutput::FileOnly, None, true)` — la
configuration exacte que le ticket décrit comme inerte — appelée pour de vrai, avec son `.init()`
global, ouvre bien la garde `tracing::enabled!(target: "mika::llm_debug", DEBUG)` et écrit le corps
DEBUG dans le fichier JSON quotidien. Le filtre n'a jamais été le défaut. Le test qui l'établit est
`crates/mika-common/tests/tui_llm_body_capture.rs` ; il reste en place précisément pour que cette
hypothèse ne soit pas re-plaidée, et pour qu'une future régression du filtre échoue là.

**Le vrai écart est en amont, dans la lecture de la *valeur* du drapeau — pas dans le filtre.** Le
daemon atteint `log_llm_bodies` via `Settings` (config-rs), qui accepte `1 / true / on / yes` **sans
tenir compte de la casse**. Les trois sites d'appel du CLI ré-implémentaient chacun
`v == "true" || v == "1"` : octet pour octet, minuscules seulement. Table mesurée (imprimée par la
sonde, config-rs interrogé et non transcrit) :

| valeur | daemon (config-rs) | CLI (avant) |
|---|---|---|
| `true`, `1` | ON | ON |
| `True`, `TRUE`, `on`, `yes` | **ON** | **inerte, en silence** |
| `nope`, `" true"` | erreur dure au démarrage | lu comme OFF, en silence |

`MIKA_LOG_LLM_BODIES=True` arme donc mika-spirit et ne fait **rien** sur un processus `mika` — très
exactement la forme rapportée (« efficace pour le daemon, inefficace pour l'agent »). Aucune refonte
de l'architecture du filtre n'est en cause : le critère objectif d'escalade vers Langfuse n'est pas
atteint, ce ticket ne l'escalade pas.

**Second écart, structurel, du même incident.** Le drapeau est *global au processus* alors que les
tours d'un agent ne sont pas tous servis par le même processus : depuis mika#1727 `mika ask` est un
client mince A2A, donc ses tours s'exécutent dans mika-spirit et s'y journalisent. Un opérateur qui
arme la variable sur le processus de l'agent puis lit le fichier de l'agent trouve un fichier vide
et conclut que le drapeau est cassé. C'est l'observation fondatrice du ticket. Un fichier vide ne
distingue pas « le drapeau est éteint ici » de « le tour a eu lieu ailleurs » — d'où la ligne
d'annonce qui **nomme le puits**.

**Ce que la Phase 1 n'a PAS pu faire, dit franchement.** Le bac à sable du pilote n'a accès ni à
`~/.mika/agents/mika-qa/logs/` ni à `/var/log/mika/`. Laquelle des deux voies ci-dessus a mordu la
nuit du 09-06→07 n'est donc pas établie : cela dépend de ce que Vincent a exporté et de quel
processus a servi le tour. Les deux sont des défauts réels, prouvés dans le dépôt, et toutes deux
sont fermées ici. La vérification AC2 sur l'hôte reste à faire par l'opérateur (voir Vérification).

## Décision de conception (révisée après mika-arch first-pass)

**Périmètre = fix in-process uniquement** (root-cause via dogfood → fix dans logging). **Langfuse
est SCINDÉ hors de ce ticket** (F1/F3) : il ne sera envisagé (ticket séparé) QUE si la Phase 1
établit que le root-cause exige une **refonte de l'architecture du filtre de logging** (critère
objectif de bascule) — pas un jugement « si c'est fragile ». Dans ce cas précis, ce ticket
ESCALADE avec le constat, et le fix Langfuse devient son propre ticket.

## Acceptance criteria

- [x] **AC1** : un test épingle la cause racine, **rouge sur `main`, vert après**.
      *Livré sous une forme que la Phase 1 a corrigée* : le détecteur rouge-avant n'est pas le test
      de filtre (il est vert sur `main` — l'hypothèse était fausse), c'est la garde structurelle
      `mika2220_no_local_reparse_of_the_llm_bodies_env_var` (`mika-cli/src/main.rs`), rouge sur
      `main` sur ses **3** occurrences de la lecture ad hoc, verte après. Le test de filtre reste
      comme épinglage de l'hypothèse infirmée.
- [x] **AC1-bis** : parité CLI ↔ daemon sur la table de vérité, config-rs **interrogé** et non
      transcrit (`mika2220_cli_parse_agrees_with_the_daemon_parse`), plus casse, non-trim, et
      distinction `Unset` / `Unrecognized` / `Disabled`.
- [x] **AC2 (moitié dépôt)** : sur la surface agent-TUI (`init_pretty` + `FileOnly` +
      `default_level = "warn"`), un `debug!(target:"mika::llm_debug", …)` atterrit dans le fichier
      JSON **et** le processus annonce la capture en nommant son puits
      (`mika2220_tui_agent_captures_llm_bodies_and_names_its_sink`).
- [ ] **AC2 (moitié hôte)** : re-déclencher une revue mika-qa et lire `run_gh pr review` dans la
      trace. Non vérifiable depuis le bac à sable du pilote — procédure ci-dessous, à exécuter par
      l'opérateur.

## Vérification sur l'hôte (à faire par l'opérateur)

1. `grep llm_body_capture` sur le fichier lu. La ligne nomme le puits et le processus. Son
   **absence** est désormais une information : la capture n'est pas armée sur ce processus-là.
2. Un tour de revue mika-qa est servi par **mika-spirit** (webhook / callback), pas par le
   processus `mika` de l'agent. Armer `MIKA_LOG_LLM_BODIES` sur mika-spirit, redémarrer, puis lire
   `$MIKA_SPIRIT_LOG_FILE` — pas `~/.mika/agents/mika-qa/logs/`. Le détour est la § *Log Sinks* de
   `crates/mika-agent/CLAUDE.md` (mika#1727 / mika#2069).
3. `grep llm_body_capture_unrecognized_value` doit être **vide**. Une occurrence désigne une valeur
   que ni le CLI ni le daemon ne lisent (elle est citée entre guillemets, donc une espace parasite
   se voit).

## Fire-Disposition

Le détecteur est `mika2220_no_local_reparse_of_the_llm_bodies_env_var`, un **scan de source** dans
`mika-cli/src/main.rs`, exécuté par le gate CI **`Check`** (bloquant). Disposition : **rouge sur
`main`** (3 occurrences de la forme de lecture ad hoc), **vert après**. Un test unitaire sur le
parseur ne peut pas voir cette classe : le défaut n'a jamais été un mauvais parse, c'était un
appelant qui ne demandait pas au parseur — même raisonnement que mika#2205. Il reste en garde
permanente : tout nouvel appelant qui ré-ouvre la table de vérité refait échouer `Check`.
(Conforme mika#1574.)

## Phases

1. **Dogfood root-cause** — fait. Candidat infirmé, écart réel identifié (voir Phase 1 ci-dessus).
2. **Fix in-process** — fait, sur l'écart révélé et non sur le candidat :
   `LogLlmBodiesSetting` + `parse_log_llm_bodies` + `log_llm_bodies_from_env` dans
   `mika-common::logging` (table de vérité unique, alignée sur config-rs), consommés par les trois
   sites du CLI ; `Unrecognized` averti au lieu d'être lu comme `false` ; ligne d'annonce nommant
   le puits, émise par `init` et `init_pretty`. Pas de refonte du filtre → **pas d'escalade**.
3. **Vérif parité** — moitié dépôt verte (AC2 dépôt) ; moitié hôte déléguée à l'opérateur.

## Hors périmètre

- Le fix de #2218 (composer+poster le verdict) — débloqué PAR ce ticket, traité après.
- Langfuse/OTEL — scindé (ticket séparé, uniquement sur escalade objective de Phase 2).
- Le volume de log (la var reste opt-in).
- **Rendre le drapeau réglable à chaud, ou par agent.** Il est lu une fois au démarrage et vaut
  pour tout le processus ; changer cela demande de rendre le filtre `tracing` rechargeable
  (`reload::Layer`) et de décider ce que « par agent » veut dire dans un daemon qui les sert tous.
  Réel, hors sujet ici, et ce ticket rend au moins l'état courant lisible.
- **Le fait que `mika ask` n'exécute rien en local** (mika#1727). Nommé à l'endroit et au moment où
  il trompe ; non modifié.

## Fichiers touchés

- `mika/crates/mika-common/src/logging.rs` — table de vérité, annonce, tests
- `mika/crates/mika-common/tests/tui_llm_body_capture.rs` — bout en bout, `.init()` réel
- `mika/crates/mika-cli/src/main.rs` — 3 sites d'appel + garde structurelle
- `mika/CLAUDE.md` — l'entrée `MIKA_LOG_LLM_BODIES`
- `mika/docs/solutions/best-practices/` — le motif capitalisé
