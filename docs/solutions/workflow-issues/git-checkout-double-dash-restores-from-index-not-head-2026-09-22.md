---
module: shell-exec, guard-shared-checkout, worktree_reaper, dispatch-lib
tags: [shared-checkout, deployment-invariant, run_shell, mika-qa, attribution, tool_calls, prompt-only-fails-at-substrate]
problem_type: silent-state-corruption
category: workflow-issues
---

# La « restauration » `git checkout -- <path>` relit l'index, pas HEAD

**Incident fondateur : mika#2449, 2026-09-21.** Avant un rebuild propre, le
checkout **main** de `mika/` portait 15 fichiers non committés (staged +
modified) et `git pull --ff-only` refusait. Le ticket accusait trois pilotes.
`tool_calls` a nommé le producteur au geste près en une requête : **mika-qa,
par `run_shell`**, pendant trois revues QA le 2026-09-20.

## Le mécanisme, à la lettre

```
cd ~/workspace/mika-platform/mika \
  && git checkout origin/fix/2135/… -- scripts/smoke-webhook-chain … \
  && bash scripts/test-smoke-webhook-chain.sh ; \
  git checkout -- scripts/smoke-webhook-chain …          # « nettoyage »
```

1. `git checkout <ref> -- <paths>` écrit **l'index ET l'arbre de travail** :
   c'est pourquoi les fichiers apparaissent *staged*, pas seulement modifiés.
2. `git checkout -- <paths>` — la « restauration » — relit l'arbre **depuis
   l'index**, que la commande précédente vient d'écraser. Elle ne restaure
   rien, et ne rend aucune erreur. Un modèle qui « nettoie derrière lui » de
   cette façon laisse exactement ce que le stash de secours a trouvé.

La forme qui aurait restauré est `git checkout HEAD -- <paths>` (ou
`git restore --source=HEAD --staged --worktree <paths>`). Mais la bonne
réponse n'est pas de corriger le nettoyage : c'est de **ne jamais extraire un
fichier de PR dans le checkout partagé**. Le prompt `qa-review` le
prescrivait déjà (§ 2B, worktree détaché jetable ; `git show <branch>:<path>`
pour lire) — cinq récurrences au moins sous prompt, classe mika#2120.

## Les trois moitiés livrées

| moitié | où | ce qu'elle achète |
|---|---|---|
| **la garde** — `run_shell` refuse tout verbe qui peut rompre l'invariant d'un checkout de déploiement (*arbre = index = HEAD sur origin/main*) visant un checkout principal sous la plateforme | `scripts/guard-shared-checkout --decide-primary`, branché dans `templates/skills/shell-exec/handlers/run.sh` | le geste casual en clair — 100 % du trafic mesuré (2 972 commandes, ~101 refusées, toutes de la classe) |
| **la sonde** — `main_checkout_dirty` dans le tick du faucheur, dédupliquée, non bloquante, qui **date** et recopie la requête `tool_calls` qui **nomme** | `crates/mika-agent/src/worktree_reaper.rs` | ce que la garde ne voit pas par construction : un script du dépôt qui mute, un geste humain, un bypass oublié |
| **la prescription réparée** — le message « recover with: `git -C $SUB_REPO_DIR stash apply …` » de `dispatch-lib.sh` (site B) nommait le checkout principal | `skills/bundled/_shared/dispatch-lib.sh`, scan à allowlist vide dans `test-dispatch-lib.sh` | un second producteur, **écrit**, de la même signature |

## Trois leçons transportables

1. **`tool_calls` est la surface d'attribution.** La rev 2 du plan refusait
   d'élire un producteur « parce que les traces étaient parties ». Elles ne
   l'étaient pas : le producteur avait estampillé son geste. Ce qui manquait
   était la **fenêtre** — d'où une sonde qui date, et une ligne d'audit qui
   porte la requête avec ses bornes.
2. **Une garde d'écriture sur un checkout de déploiement a une allow-list qui
   N'EST PAS vide, à dessein.** Refuser toute mutation casserait la boucle
   (mika-dev synchronise `main` par `fetch && merge --ff-only`). Le prédicat
   est l'*invariant*, pas « toute mutation » ; une entrée y est un verbe qui
   le préserve, jamais une dispense pour un agent ou un chemin.
3. **Une population exemptée par construction ne se couvre pas en élargissant
   le prédicat voisin.** mika#2107 exempte toute session sans worktree lié
   (voulu). mika-qa n'en a pas. Le remède est un **second mode** qui partage le
   parseur et inverse la population — pas un T1 assoupli, qui aurait cassé
   `/mika-platform-sync-main`.

## Pièges rencontrés en construisant

- **`~` n'est pas un répertoire relatif.** Le parseur de mika#2107 traitait
  `cd ~/workspace/…` comme `<cwd>/~/workspace/…`, donc hors plateforme, donc
  allow — la forme exacte des trois commandes mesurées. Expansion lexicale de
  `~`, `~/`, `$HOME`, `${HOME}` ajoutée à `normalize_path`.
- **`~/workspace` est un symlink** vers `/data/workspace`. Un platform-dir
  résolu par `pwd -P` (comme partout ailleurs) ne matche une cible lexicale
  que si la cible est résolue physiquement aussi.
- **Le handler scrubbe toutes les `MIKA_*`**, `MIKA_PLATFORM_DIR` et
  `MIKA_GUARD_SHARED_CHECKOUT` comprises. Lues avant le scrub, passées en
  arguments ; lues après, la garde tomberait ouverte partout sans rien dire.
  Et `MIKA_GUARD_SHARED_CHECKOUT_LOG` est scrubbée aussi : sous `run_shell`
  la garde journalise au chemin par défaut, `~/.mika/state/`.
- **Le scan de parité a mordu sur le fix lui-même** : la butée « NOT in the
  primary checkout `$SUB_REPO_DIR` » nommait encore la variable. R2 dit
  *aucun* message, même en négation. Corrigé, et c'est la preuve que le scan
  regarde.

Plan : `docs/plans/2026-09-21-004-fix-2449-checkout-principal-sali-attribution-plan.md`.
