---
module: CLAUDE.md, crates/mika-agent/CLAUDE.md, claude-pilot
tags: [autonomous-loop, claude-pilot, prompt-size, claude-md, context-window, sandbox]
problem_type: loop_breaker
category: workflow-issues
date: 2026-10-13
ticket: mika#2683 (groom mort au tour 1)
applies_when:
  - Un pilote finit en « Prompt is too long » ou en « zero new commits » avec Turns 1 et 0 $
  - Vous ajoutez de la documentation dans un CLAUDE.md
  - Une sonde hors du bac à sable dit « ça marche » alors que le pilote meurt
resolution_type: code_fix
---

# Un CLAUDE.md qui grossit tue les pilotes au tour 1, et une sonde hors bac à sable ne le voit pas

## Symptôme

Le groom de mika#2683 est mort en 140 ms : `Turns: 1`, `Cost: $0.0`, `HEAD unchanged`. Dans le log du pilote, la seule trace est une ligne `Prompt is too long`, 6 ms après `[init]`. Le transcript du pilote ne contient qu'une réponse `model: "<synthetic>"`, `error: "invalid_request"`, zéro token. Aucun appel n'est parti : c'est le CLI qui refuse, localement.

La même mort a frappé les pilotes mika des 22/09, 30/09 (quatre fois), 01/10 et 05/10. Elle était passée pour des « pipeline incomplete » ordinaires.

## Cause

Le prompt de départ d'une session Claude Code dans ce dépôt faisait **390 108 tokens**. Claude Code charge en entier le `CLAUDE.md` de la racine (825 140 octets, dont 767 Ko dans § Environment Variables), et charge celui d'un crate dès que la session lit un fichier de ce crate (`crates/mika-agent/CLAUDE.md` : 594 290 octets). Chaque PR y documentait ses variables et ses surfaces opérateur, et rien ne bornait la taille.

## Le piège de diagnostic

Lancée hors du bac à sable, avec le CLI embarqué du pilote et le même modèle, une session dans le même checkout **répond normalement** : le CLI y dispose d'une fenêtre de 1 M et accepte 390 k. Seul le pilote, dans son bac à sable, refuse. Une sonde « est-ce que ça démarre ? » donne donc le mauvais verdict.

Ce qui tranche, c'est la mesure, pas le succès :

```bash
echo "Réponds seulement OK." | claude -p --model claude-opus-5 --max-turns 1 --output-format json \
  | jq '.usage | {input_tokens, cache_creation_input_tokens, cache_read_input_tokens}'
```

Additionner les trois champs donne la taille du prompt de départ. Au-dessus de 200 000, le pilote mourra.

## Correctif

- § Environment Variables déplacée vers `docs/operator/environment-variables.md` (et non `docs/` : le site public publie chaque `.md` de premier niveau).
- Les sections de plus de 15 Ko de `crates/mika-agent/CLAUDE.md` déplacées vers `docs/agent-internals/`, titres conservés avec un pointeur.
- `crates/mika-common/tests/claude_md_size.rs` plafonne chaque `CLAUDE.md` du dépôt à 100 000 octets. Prompt de départ après correctif : 59 250 tokens.

## Règle

Un `CLAUDE.md` est un index, pas un journal. Documentez une variable, une sonde ou un sous-système dans `docs/`, et laissez une ligne de pointeur. Si le test de taille échoue, déplacez du contenu : ne relevez pas le plafond.
