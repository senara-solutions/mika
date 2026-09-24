# Fixtures gelées — mika#1910, axe B

Consommées par `scripts/test-measure-empty-turns.sh`
(`make test-measure-empty-turns`).

## Ce qui est mesuré, et ce qui est reconstruit

**La forme est mesurée ; les valeurs sont fabriquées.** Chaque champ de ces
lignes est celui que `emit_turn_usage` écrit à travers la couche JSON de
`tracing_subscriber`, configurée avec `flatten_event(true)` — d'où des champs
**à la racine** de l'objet et non sous `fields`. Les valeurs (comptes de tokens,
latences, identifiants) sont synthétiques.

**Le point de fidélité qui compte est la forme de fil de `response_chars`.**
`emit_turn_usage` l'écrit avec le sigil `?` (`response_chars = ?fields.response_chars`),
choisi pour que la distinction `null` / valeur survive — et la couche JSON
sérialise un champ `record_debug` en **chaîne** :

```
"response_chars":"Some(240)"    "response_chars":"Some(0)"    "response_chars":"None"
```

Un analyseur qui attendrait un nombre JSON lirait **chaque** ligne comme « non
mesuré » et rapporterait **zéro** occurrence de la classe — un faux vert sur
exactement la population dont le ticket parle. C'est pour ça que la forme de
chaîne est dans la fixture plutôt que déduite.

## Pourquoi synthétiques, et pourquoi c'est délibéré

Elles ne sont **pas** échantillonnées de `$MIKA_SPIRIT_LOG_FILE` :

- une suite qui lit le corpus vivant **change de verdict** à mesure que le
  corpus grandit, donc n'est plus un test ;
- elle ne peut pas tourner sur une machine qui n'a jamais dispatché ;
- et surtout, le champ `response_chars` **n'existe qu'après le déploiement de
  mika#1910** : il n'y a aucun corpus historique à échantillonner.

La fidélité vient du **producteur**, pas de la provenance.

## `turn_usage_corpus.jsonl` — le corpus de contrôle

Un `trace_id` par confusion, pour que chaque contrôle négatif porte **seul**.
Une conjonction de termes fail-safe ne se prouve pas en les invalidant ensemble
(leçon mika#2277).

| `trace_id` | rôle | attendu |
|---|---|---|
| `t1` | anti-vacuité **positive** — `EndTurn`, `output_tokens: 800`, chars `0` | `empty_response` |
| `t2` | anti-vacuité **négative** — chars `240`, modèle non-GLM | `produced` |
| `t3` | **N1** (R5) — chars `None` | `undetermined` |
| `t4` | **N2** (R6) — `status: error`, `input_tokens: 0`, glm-5.3 | `error` |
| `t5` | **N3** (R7) — `MaxTokens`, `output_tokens: 1200`, chars `0` | `reasoning_budget_exhausted` |
| `t6` | **N4** — ligne de boucle (`step: 3`), chars `0` | hors population, **compté au dénominateur** |
| `t7`, `t8` | **N5a** — deux `trace_id`, **une** `session_id` | **deux** `empty_response` |
| `t9` | **N5b** — deux lignes de continuation, **un** `trace_id` | `undetermined` + `duplicate_continuation` |
| `tq1` | filtre `--agent` — agent `mika-qa` | exclu entièrement |
| *(dernière ligne)* | fail-safe — texte non-JSON contenant `turn_usage` | `unparseable_lines: 1` |

L'`output_tokens: 800` de `t1` **n'est pas décoratif** : il épingle la frontière
avec `t5` — mêmes `response_chars: 0` et `output_tokens > 0`, `stop_reason`
différent, donc classe différente. Sans lui, un prédicat qui aurait avalé la
règle 3 dans la règle 4 resterait vert. Il ancre aussi le faux positif
`strip_internal_tags` sur un cas exécuté plutôt que sur une note.

Agrégat attendu (avec `--agent mika-dev`) : `total_trace_ids_scanned: 9`,
`continuation_turns: 8`, `duplicate_continuation: 1`, `unparseable_lines: 1`,
`glm_continuation_turns: 7`.

## `wire_forms.jsonl` — tolérances et dégradation

| `trace_id` | forme de `response_chars` | attendu |
|---|---|---|
| `w1` | nombre JSON `0` | `empty_response` |
| `w2` | nombre JSON `312` | `produced` |
| `w3` | chaîne `"garbled"` | `undetermined` |
| `w4` | **champ absent** (la population d'avant le déploiement) | `undetermined` |
| `w5` | `"Some(0)"` avec `MaxTokens` et `output_tokens: 0` | `empty_response` |

`w1`/`w2` attestent que le nombre brut est accepté : un changement futur de
formateur dégrade vers la **correction**, jamais vers le silence. `w5` est le
contrôle qui rend le terme `output_tokens > 0` de la règle 3 porteur plutôt que
décoratif — sans lui, tout tour plafonné à zéro caractère quitterait la classe
mika#1910 pour être classé « problème de réglage ».

## Ne pas « rafraîchir » ces fichiers

Les régénérer depuis un log vivant effacerait précisément les formes que le
prédicat doit reconnaître — et, pour `w4`, la seule trace de la population
d'avant le déploiement, qui ne se reproduira jamais. Si le producteur change de
forme, **c'est la ligne concernée qu'on ajoute**, en datant pourquoi, jamais le
fichier qu'on remplace.
