---
title: Deux lecteurs d'une même variable d'environnement divergent en silence — cherchez l'appelant, pas le parseur
date: 2026-09-07
last_updated: 2026-09-07
category: best-practices
module: mika-common/logging
problem_type: best_practice
component: observability
severity: high
applies_when:
  - Une variable `MIKA_*` est lue à la fois par `Settings` (config-rs) et à la main quelque part
  - Un drapeau de diagnostic « marche sur le daemon » et « ne fait rien » ailleurs
  - Vous vous apprêtez à écrire `v == "true" || v == "1"`
  - Vous écrivez un scan de source dont le motif recherché est un littéral du fichier scanné
---

# Deux lecteurs d'une même variable d'environnement divergent en silence

## Le problème

`MIKA_LOG_LLM_BODIES` armait la capture des corps LLM sur mika-spirit et ne faisait
**rien** sur un processus `mika`. Le ticket mika#2220 a été groomé sur l'hypothèse
naturelle : les deux binaires posent la même directive `mika::llm_debug=debug`, donc
la divergence doit être dans le filtre `tracing` — probablement le `default_level`
`"warn"` du CLI qui écraserait la directive spécifique.

**L'hypothèse était fausse, et le dogfood l'a montré en un test.** Appelé pour de
vrai, avec son `.init()` global, `init_pretty("warn", …, FileOnly, log_llm_bodies =
true)` ouvre la garde et écrit le corps DEBUG dans le fichier. Le filtre n'a jamais
été en cause.

L'écart était en amont, dans la lecture de la **valeur** :

| valeur | daemon (`Settings`, config-rs) | CLI (3 sites, à la main) |
|---|---|---|
| `true`, `1` | ON | ON |
| `True`, `TRUE`, `on`, `yes` | **ON** | **inerte, en silence** |
| `nope`, `" true"` | erreur dure au démarrage | lu comme OFF, en silence |

config-rs accepte `1 / true / on / yes` sans tenir compte de la casse. Les trois
sites du CLI faisaient `v == "true" || v == "1"` : octet pour octet, minuscules
seulement. Un opérateur qui exporte `MIKA_LOG_LLM_BODIES=True` obtient un daemon qui
journalise et un CLI qui n'en dit rien.

## La leçon

**Une table de vérité dupliquée ne diverge pas au moment où on l'écrit — elle diverge
plus tard, quand un seul des deux côtés est élargi.** Ici, personne n'a « cassé » le
CLI : config-rs a toujours été plus permissif, et le second lecteur est né plus
strict. Rien ne le signalait parce que les deux réponses sont un `bool` parfaitement
valide.

Trois règles qui en découlent.

**1. Le parseur canonique doit être *interrogé*, pas transcrit.** Le test de parité
(`mika2220_cli_parse_agrees_with_the_daemon_parse`) construit un `config::Config` et
lui demande son verdict pour chaque valeur, au lieu de recopier la table dans une
constante. Une table recopiée ment le jour où config-rs élargit la sienne — c'est
exactement le mode de défaillance qu'on vient de corriger, réintroduit dans le test
censé le prévenir.

**2. Le détecteur doit viser l'appelant, pas le parseur.** Un test unitaire sur
`parse_log_llm_bodies` est vert avant comme après le correctif : le défaut n'a jamais
été un mauvais parse, c'était un appelant qui ne demandait pas au parseur. Seul un
**scan de source** voit cette classe. Même raisonnement, même semaine, que mika#2205
(`dispatcher::tests::mika2205_periodic_scans_do_not_read_the_pat_field_directly`) :
un test du résolveur serait resté vert pendant toute la panne.

**3. « Non reconnu » n'est pas « faux ».** Les deux lecteurs lisaient `nope` ou
`" true"` comme OFF (CLI) ou refusaient de démarrer (daemon) ; aucun ne le *disait*
au CLI. Un troisième état explicite (`Unrecognized`, averti, valeur **citée** pour
qu'une espace parasite se voie) coûte dix lignes et transforme un échec invisible en
échec lisible. Ne pas *trimmer* au passage : config-rs ne trimme pas, donc trimmer
côté CLI recréerait la même divergence en sens inverse.

## Corollaire : un drapeau global à un processus, dans un système multi-processus

Le second défaut du même incident n'est pas un parse. `MIKA_LOG_LLM_BODIES` vaut pour
**tout le processus**, mais les tours d'un agent ne sont pas tous servis par le même
processus : depuis mika#1727, `mika ask` est un client mince A2A et le tour s'exécute
dans mika-spirit. Armer la variable sur le processus de l'agent puis lire le fichier
de l'agent donne un fichier vide — et un fichier vide ne distingue pas « le drapeau
est éteint ici » de « le tour a eu lieu ailleurs ».

**Un drapeau d'observabilité doit annoncer son puits.** Une ligne au démarrage qui
nomme le fichier écrit rend l'absence de trace interprétable, et son **absence à elle**
devient une information : la capture n'est pas armée sur ce processus-là. Au niveau
WARN, pas INFO — le niveau par défaut du CLI est `warn`, donc une ligne INFO serait
filtrée sur précisément la surface qui en avait besoin.

## Piège de mise en œuvre : le scan qui se voit lui-même

Le premier jet de la garde structurelle échouait immédiatement :

```rust
assert!(!THIS_FILE.contains("MIKA_LOG_LLM_BODIES\")"), …);
```

Le fichier scanné *est* le fichier qui contient le littéral. Le motif se trouve
lui-même, la garde échoue pour toujours, et le message d'erreur n'a rien à voir avec
le vrai contenu. Assemblez l'aiguille à l'exécution :

```rust
let read_shape = format!("{}\")", mika_common::logging::LOG_LLM_BODIES_ENV);
assert!(!THIS_FILE.contains(&read_shape), …);
```

Les gardes `mika2195_*` de `logging.rs` évitent le même écueil autrement, en
découpant d'abord la portion de source concernée. Les deux marchent ; ce qui ne
marche pas, c'est un motif littéral non borné.

## Où c'est appliqué

- `crates/mika-common/src/logging.rs` — `LogLlmBodiesSetting`, `parse_log_llm_bodies`,
  `log_llm_bodies_from_env`, `announce_llm_body_capture`, et la table de parité
- `crates/mika-cli/src/main.rs` — trois sites d'appel + garde structurelle
  `mika2220_no_local_reparse_of_the_llm_bodies_env_var`
- `crates/mika-common/tests/tui_llm_body_capture.rs` — épinglage de l'hypothèse
  infirmée (le filtre va bien) et de la ligne d'annonce

## Voir aussi

- `verifier-un-creneau-nest-pas-le-reclamer-2026-08-30.md` — même famille : un
  prédicat correct dont l'appelant est le défaut
- `un-label-denforcement-non-declare-echoue-en-silence-2026-09-01.md` — même famille :
  un mécanisme qui « existe » et dont l'effet ne s'observe jamais
- `docs/solutions/architecture-patterns/2026-09-06-accesseur-etroit-a-cote-du-resolveur-canonique.md`
  (mika#2205) — l'accesseur étroit à côté du résolveur canonique
