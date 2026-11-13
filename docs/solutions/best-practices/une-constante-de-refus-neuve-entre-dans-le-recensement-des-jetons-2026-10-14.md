---
title: Une constante de refus neuve entre dans le recensement des jetons canoniques, et se déclare dans le même commit
date: 2026-10-14
category: best-practices
module: crates/mika-agent/src/tools
problem_type: best_practice
component: agent-core
severity: low
tags: [canonical-tokens, refusal, tool-guard, webhook-pr-turn, mika-2654, mika-2201]
applies_when:
  - "Ajouter un refus d'outil qui ouvre son ToolOutput::error sur un préfixe constant"
  - "Créer une constante `*_PREFIX` qui porte un jeton machine (REFUS, REFUSED, …)"
  - "Un plan annonce une Fire-Disposition « allowlist vide » pour un nom neuf"
---

# Une constante de refus neuve entre dans le recensement des jetons canoniques

## Problème

mika#2654 borne `promote_deferred_callback` : sur un tour ouvert par un événement GitHub PR ou check-suite, l'outil refuse la promotion forcée et rend un `ToolOutput::error` qui s'ouvre sur `PROMOTE_REFUSAL_PREFIX = "REFUS (webhook-pr-turn, mika#2654)"`. Le plan avait anticipé un scan, celui du nom d'audit `deferred_dispatch_force_promote_refused_webhook_pr_turn` (écrivain unique, allowlist vide vérifiée par `grep` avant le commit). Il n'avait pas anticipé le second.

Le job requis `Canonical Token Lint` est passé au rouge :

```
ERROR: match sites found in the tree and absent from scripts/canonical-tokens.tsv:
       REFUS	crates/mika-agent/src/tools/promote_deferred_callback.rs::PROMOTE_REFUSAL_PREFIX
```

Le recensement de mika#2201 énumère toute constante en `*_PREFIX` qui porte un jeton machine. Un préfixe de refus est un format de fil : les tests de l'outil le lisent par `starts_with`, et la requête opérateur sur `tool_calls.output` l'ancre en tête. Le reformuler sortirait la ligne de la population sans un mot. C'est la classe B.

## Solution

Déclarer le site dans `scripts/canonical-tokens.tsv`, à côté de son aîné `TESTIMONY_ACCESS_REFUSAL_PREFIX` (mika#2627), dans le même commit que la constante :

```
REFUS	B	crates/mika-agent/src/tools/promote_deferred_callback.rs::PROMOTE_REFUSAL_PREFIX	exact:prefix
```

Ne pas l'allowlister et ne pas renommer la constante pour sortir de la population : le TSV a écrit pourquoi ce serait cacher un lecteur réel derrière un nom (§ D5/D6).

## La règle

Un refus d'outil neuf entre dans **deux** recensements, pas un :

1. **Le nom d'audit** : écrivain unique, scanné par le test `*_SOLE_WRITER_EXCEPTIONS` du ticket.
2. **Le préfixe du corps** : recensement des jetons canoniques, scanné par `scripts/check-canonical-tokens.sh` et par `canonical_tokens::tests::mika2201_every_match_site_is_declared`.

Une Fire-Disposition qui ne nomme que le premier laisse le second tirer en CI. Avant de pousser, lancer `bash scripts/check-canonical-tokens.sh` en local : il rend en quelques secondes ce que le job requis dirait.
