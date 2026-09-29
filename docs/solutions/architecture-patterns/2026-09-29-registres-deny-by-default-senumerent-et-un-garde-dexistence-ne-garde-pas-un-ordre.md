---
title: "Un registre deny-by-default se compte par énumération, jamais de mémoire — et un garde d'existence ne garde pas un invariant d'ordre"
date: 2026-09-29
category: architecture-patterns
module: dispatch
problem_type: architecture_pattern
component: development_workflow
severity: high
applies_when:
  - "Ajouter un nom de variable d'environnement à l'allowlist `--setenv` du bac à sable de dispatch"
  - "Un invariant tient à la POSITION d'une ligne dans une fonction, pas à sa présence"
  - "Un plan affirme « N sites à mettre à jour » et ce N vient d'une lecture, pas d'une énumération"
  - "Écrire un garde structurel dont le comportement négatif doit être pinné"
tags:
  - dispatch-lib
  - deny-by-default
  - setenv-allowlist
  - garde-structurel
  - bwrap
  - clearenv
  - cargo
  - mika-2105
  - mika-2039
  - mika-2141
  - mika-2572
---

# Un registre deny-by-default se compte par énumération, et un garde d'existence ne garde pas un ordre

## Contexte

mika#2105 ajoute `CARGO_INCREMENTAL=0` au build d'un spawn : l'état de compilation incrémentale pèse
42 % du `target/` d'un worktree de dispatch, et ce worktree est jeté après une à trois compilations —
donc cet état est produit intégralement et jamais réutilisé.

Le changement fonctionnel tient en deux lignes : un `export` et une entrée d'allowlist. Ce qui a
coûté deux passes de grooming et une CI rouge, c'est tout le reste.

## Leçon 1 — combien de registres admettent un nom à `--setenv` ? La réponse s'énumère

Un nom autorisé à traverser `bwrap --clearenv` par `--setenv` ne vit pas dans une liste : il vit dans
**plusieurs registres deny-by-default indépendants**, chacun comparé strictement à l'allowlist
réelle. Ajouter le nom d'un seul côté ne produit pas un garde partiellement satisfait — il produit
une **violation**, parce que la posture est « tout nom non audité est une violation, quelle que soit
l'apparence du nom ».

Le compte réel au 2026-09-29 est **trois** :

| registre | fichier |
|---|---|
| l'allowlist elle-même | `skills/bundled/_shared/dispatch-lib.sh` (`_PILOT_SANDBOX_ENV_ALLOWLIST`) |
| le double registre du garde de secrets | `scripts/verify-no-secret-in-setenv.sh` (`EXPECTED_ENV_ALLOWLIST`) |
| le registre du test d'argv | `skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh` (`AUDITED_SETENV_NAMES`) |

**Ce compte a été faux deux fois de suite, et les deux fois par la même méthode.** Le plan du
2026-09-09 a d'abord annoncé **deux** sites, puis **trois** après lecture de
`verify-no-secret-in-setenv.sh` — en comptant l'allowlist elle-même parmi eux, donc **deux** registres
de contrôle sur les trois qui existaient. Les deux passes d'architecte ont validé ce compte. La CI de
la PR a rougi sur celui que personne n'avait ouvert : `AUDITED_SETENV_NAMES`, présent dans l'arbre
depuis bien avant le premier grooming.

Chaque itération avait lu *un registre de plus* et en avait déduit qu'il n'y en avait plus. C'est la
méthode qui est en cause, pas l'attention : **lire un registre apprend qu'il existe, et rien sur le
nombre de ses frères.**

**Le remède n'est pas une liste, c'est une commande.** Une liste de registres écrite dans un plan
pourrit exactement comme le compte qu'elle remplace. Ce qui ne pourrit pas, c'est un prédicat
d'énumération — et il en existe un, gratuit : tout registre contient nécessairement les noms déjà
admis, donc n'importe lequel d'entre eux sert de sonde.

```bash
git grep -c 'ANTHROPIC_LOG_FILE' -- scripts skills
```

Rend, au 2026-09-29 :

```
scripts/canary-pilot-containment:1
scripts/verify-no-secret-in-setenv.sh:1
skills/bundled/_shared/dispatch-lib.sh:6
skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh:1
```

**Et il faut savoir lire la quatrième ligne comme un non-registre.** `canary-pilot-containment:387`
fait `export ANTHROPIC_LOG_FILE="…"` : il pose une **valeur** pour son propre sandbox de test, il ne
déclare pas un **nom** comme admissible. Le discriminant est là et pas ailleurs — un registre tient
une liste de noms, un exportateur tient une valeur. Confondre les deux ajoute un site à mettre à jour
qui n'a rien à apprendre.

### Pourquoi la duplication est délibérée, donc à honorer plutôt qu'à supprimer

Le réflexe devant trois registres est de les fusionner. C'est refusé par leur raison d'être :
`verify-no-secret-in-setenv.sh:48-52` écrit que l'addition d'un nom **doit être un acte délibéré** —
« confirm the variable carries no credential material, note why in the audit comment, then update
this set ». Un registre unique rendrait l'addition mécanique ; trois registres à mettre à jour dans
le même diff est le coût d'entrée qui force la relecture. La friction *est* le mécanisme.

Ce que ça prescrit à l'implémenteur : les trois gestes, dans le même diff.

1. La valeur ne porte aucun matériel de créance. Ici c'est le littéral `0`, et — c'est le point qui
   rend l'audit concluant — il est posé par une **constante** dans `_run_pilot_sandboxed`, jamais lu
   depuis l'environnement parent. Aucune valeur fournie par un appelant ne peut monter sur ce nom
   jusqu'à l'argv, qui est lisible par tout le monde.
2. La raison est notée dans les commentaires d'audit, à côté de chaque registre.
3. Les registres bougent ensemble.

## Leçon 2 — un garde d'existence ne garde pas un invariant de position

L'invariant de mika#2105 n'est pas « l'export existe ». C'est « l'export précède la branche de bac à
sable ». `_run_pilot_sandboxed` a trois sorties :

```
export CARGO_INCREMENTAL=0        # ← ici, les trois sorties l'ont
if ! _pilot_sandbox_enabled; then "$@"; fi          # sortie directe 1 (opt-out)
if ! command -v bwrap; then       "$@"; fi          # sortie directe 2 (bwrap absent)
…                                                   # sortie sandboxée
```

Placé **sous** la branche, l'export n'atteindrait que le chemin sandboxé : les deux sorties directes
perdraient le réglage, en silence, et le gain serait perdu là où personne ne regarde. C'est le
demi-correctif que l'AC1 du ticket excluait nommément.

**Un garde qui teste la présence de l'export passe sur cette régression.** Les deux assertions
n'appartiennent pas au même monde :

- **B** — l'export existe et vaut `0`. Passe avec un export mal placé.
- **B'** — l'export précède `_pilot_sandbox_enabled` dans le corps de la fonction. C'est la seule qui
  garde l'AC.

La généralisation vaut au-delà de ce ticket : **quand un invariant est positionnel, l'assertion doit
comparer deux positions.** Une assertion d'existence à côté d'une assertion de position n'est pas
redondante — elle donne le diagnostic (« absent » vs « mal placé »), qui sont deux remèdes
différents. Les garder séparées est ce qui rend le rouge lisible.

Deux détails d'implémentation du prédicat méritent d'être repris tels quels :

- **Les commentaires sont retirés avant de juger.** `dispatch-lib.sh` explique ses propres réglages
  en prose, sur des dizaines de lignes, et cette prose contient le nom de la variable. Sans le filtre
  `grep -vE '^[[:space:]]*#'`, la prose satisferait le garde — même raison que le `CODE_ONLY` de
  `verify-no-secret-in-setenv.sh`. Un garde qu'un commentaire satisfait ne garde rien.
- **Le prédicat de « qu'est-ce qu'un lancement de `claude-pilot` » est réutilisé, pas recopié.**
  L'assertion qui énumère les handlers hors `dispatch-lib` appelle `_mika2496_launch_candidates`
  (mika#2496), avec ses cinq termes mesurés et ses cinq fixtures. Une seconde définition aurait
  divergé — la leçon que `grooming_marker` a dû graver une fois (mika#2158).

## Leçon 3 — le périmètre se nomme quand l'argument porteur ne tient pas partout

Quatre chemins lancent `claude-pilot` dans un worktree qui compile, et ils ne sont pas de même
régime :

| chemin | source `dispatch-lib` | worktree |
|---|---|---|
| `dev-pilot`, `dev-groom` | oui | **créé puis jeté** |
| `address-pr-comments`, `resolve-pr-conflicts` | non | **réutilisé** entre rounds de PR |

L'argument qui porte le changement — « l'état incrémental est produit puis perdu » — **ne tient pas**
sur les deux derniers : une PR reçoit plusieurs rounds, et ces handlers recompilent le *même*
worktree, ce qui est précisément le régime où l'incrémental paie. Étendre le réglage aux quatre
chemins aurait appliqué la bonne conclusion au mauvais raisonnement : du disque gagné contre du temps
de boucle perdu, et le temps de boucle coûte plus cher.

Ce qui rend ce périmètre défendable plutôt que subi, c'est qu'il est **gardé** : l'assertion D
énumère les handlers hors `dispatch-lib` dans les deux sens, avec un contrôle anti-vacuité du glob. Un
cinquième chemin ajouté plus tard fait rougir la CI et force une décision de périmètre — au lieu de
perdre le gain en silence, qui est le mode d'échec par défaut de tout périmètre écrit en prose.

## Leçon 4 — nommer le seuil de renoncement avant de mesurer

Désactiver l'incrémental peut allonger les rebuilds. Le ticket a fixé le seuil — **+50 % sur le
rebuild, au-delà duquel le travail remonte à l'opérateur au lieu de fusionner** — le 2026-09-09, vingt
jours avant la première mesure. C'est ce qui empêche de le rationaliser après coup : un seuil posé
après la mesure est une justification, pas un critère.

Corollaire sur la conduite du protocole : les quatre durées se mesurent **en séquence**, jamais deux
builds concurrents. Sur 16 cœurs, deux `cargo test` simultanés se ralentissent mutuellement, et la
contention ne se répartit pas également entre les deux bras — c'est exactement le bruit que le ratio
ne doit pas porter. Et le crate touché pour le rebuild est `mika-agent`, non un crate strictement
feuille : c'est celui que les pilotes modifient, donc la mesure la plus défavorable au réglage. Un
verdict qui passe dans le cas défavorable passe a fortiori dans l'autre.

## Ce que ça n'achète pas

Le garde tient la configuration ; il ne mesure pas le gain. Celui-là se relève sur le premier spawn
compilant qui suit le déploiement, et **`skills/bundled/` est une projection du binaire, pas du
checkout** (mika#2340) : un `dispatch-lib.sh` édité dans l'arbre de travail n'atteint aucun agent
avant `make deploy` → seed. Une mesure prise avant cette chaîne décrit le binaire d'hier.

## Références

- `skills/bundled/_shared/dispatch-lib.sh` — `_PILOT_SANDBOX_ENV_ALLOWLIST`, bloc d'audit R6 (mika#2039),
  `_run_pilot_sandboxed` et ses trois sorties
- `scripts/verify-no-secret-in-setenv.sh` — règle deny-by-default, `EXPECTED_ENV_ALLOWLIST`, `CODE_ONLY`
- `skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh` — `AUDITED_SETENV_NAMES`
- `skills/bundled/_shared/test-dispatch-lib.sh` — assertions A/B/B'/C/D et négatifs N1–N5 de mika#2105 ;
  `_mika2496_launch_candidates`
- `docs/plans/2026-09-09-001-fix-2105-cargo-incremental-zero-dispatch-plan.md`
- `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` — pourquoi le réglage est dans
  le code et pas dans un prompt
- `feedback_a_probe_needs_both_controls_in_the_same_call` — pourquoi chaque négatif porte son miroir vert
