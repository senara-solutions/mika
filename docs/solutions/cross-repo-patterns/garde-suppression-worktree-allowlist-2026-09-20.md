---
module: skills/bundled/_shared, mika-agent/worktree_reaper, mika-platform/scripts
tags: [cleanup, worktree, allowlist, fail-safe, mika-1943, mika-2420, cross-repo]
problem_type: architecture
category: cross-repo-patterns
---

# Une seule sémantique décide qu'un chemin est supprimable

**Tickets :** mika#1943 (AC2, livré), mika#2420 (le reaper, déjà conforme).
**Suivi à ouvrir :** mika-platform — corps rédigé verbatim en §5.

## 1. La sémantique canonique : allowlist, jamais denylist

Un chemin n'est supprimable par un nettoyage automatisé que s'il **prouve**
être un worktree géré. Quatre termes conjonctifs :

| # | terme | illisible ⇒ |
|---|---|---|
| T1 | non vide | conserver |
| T2 | absolu | conserver |
| T3 | aucun composant `..` | conserver |
| T4 | contient `/.claude/worktrees/` | conserver |

**L'asymétrie qui décide de tout, et elle doit être écrite avant le reste.** Un
faux négatif laisse un worktree résiduel sur le disque : le reaper mika#2420 le
ramasse au tick suivant, ou l'opérateur. Coût borné, quelques Go, temporaire. Un
faux positif supprime un répertoire qui n'est pas un worktree : **irréversible**.
Donc tout terme illisible conserve.

Cette asymétrie est **locale et ne se transporte pas** : `wip_rescue` (mika#2199)
conclut dans l'autre sens sur la même primitive, parce qu'y rejouer coûte toute la
file. Copier une conclusion de fail-safe d'un module à l'autre sans refaire
l'arbitrage est la faute que ce paragraphe existe pour prévenir.

## 2. Pourquoi une denylist est le mauvais outil, mesuré sur l'incident

Le ticket mika#1943 prescrivait une **denylist** : refuser
`^/data/workspace/[^/]+/?$`. Trois raisons l'ont écartée, la troisième décide.

1. **Une denylist est fausse le jour où elle servirait.** `/data/workspace/bbytaa`
   — le répertoire emporté le 28/07 — n'aurait figuré dans aucune liste écrite
   avant lui. Il n'était protégé par aucune liste : il était protégé par les
   instantanés btrbk qui l'encadraient, et c'est un accident heureux, pas un
   mécanisme.
2. **Deux sémantiques opposées pour une même question dans un même dépôt** est la
   divergence programmée que `grooming_marker` (mika#2158) a dû fermer une fois —
   deux prédicats répondant différemment à « ce chemin est-il supprimable ».
3. **`/data/workspace/` est le disque d'une machine, pas une propriété du
   système.** Coder ce préfixe en dur ne protège que `gentux` et devient muet
   partout ailleurs : un garde-fou qui *paraît* poser une règle générale.
   `/.claude/worktrees/` est, lui, une propriété **structurelle** du layout, donc
   vraie sur toute machine où le layout existe.

L'allowlist satisfait la denylist *a fortiori* : `/data/workspace/bbytaa` échoue
T4, comme tout ce qui n'est pas un worktree géré.

## 3. Les consommateurs, par dépôt, et leur état au 2026-09-20

| dépôt | consommateur | primitive | état |
|---|---|---|---|
| `mika` | `crates/mika-agent/src/worktree_reaper.rs` | `git worktree remove --force` toutes les 10 min | **conforme** (mika#2420) — `is_managed_worktree_path` + re-vérification après canonicalisation |
| `mika` | `skills/bundled/_shared/dispatch-lib.sh` | 3 × `worktree remove --force`, 2 × `rm -rf` | **conforme** (mika#1943) — `_assert_removable_worktree_path` |
| `mika-platform` | `make prune-worktrees`, `worktrees-audit`, `worktrees-clean` (couches A/B de #1694) | inconnue | **non vérifié** — voir §4 |

Le reaper porte une garde que dispatch-lib n'avait pas — et c'est la plus récente
qui était la plus sûre. L'ancien consommateur, celui qui tourne à **chaque**
dispatch, était le plus exposé.

## 4. Ce qui n'a pas pu être vérifié, et pourquoi c'est dit plutôt qu'omis

Les trois consommateurs `mika-platform` **n'ont pas été lus**. Le dispatch dérive
un worktree par dépôt et le bac à sable ne binde que celui-là :
`/data/workspace/mika-platform/scripts/` est inaccessible depuis une session
dispatchée sur `mika` (mesuré — `No such file or directory`, alors que
`/data/workspace/mika-platform/.claude/worktrees/` est bien monté).

**Donc la ligne « inconnue » du tableau ci-dessus est un fait sur la vue, pas sur
le code.** Elle peut être conforme, partiellement gardée, ou nue. Écrire l'une de
ces trois choses sans l'avoir lue serait fabriquer une mesure (classe #953), et
la table serait ensuite citée comme si elle en était une.

## 5. Corps du ticket `mika-platform` à ouvrir

> **Titre :** `chore: porter la garde allowlist de suppression aux couches A/B de #1694 (suivi mika#1943)`
>
> **Labels :** `type:chore`, `p2-normal`
>
> ---
>
> ## Contexte
>
> mika#1943 a fermé la moitié qui vit dans `mika` : `dispatch-lib.sh` ne supprime
> plus un chemin qu'il ne peut pas prouver être un worktree géré
> (`_assert_removable_worktree_path`, quatre termes conjonctifs, fail-safe vers
> *conserver*). `worktree_reaper.rs` (mika#2420) portait déjà la même sémantique.
>
> Les couches A/B de #1694 — `make prune-worktrees`, `worktrees-audit`,
> `worktrees-clean` — vivent dans ce dépôt et **n'ont pas pu être lues** depuis la
> session qui a livré mika#1943 : le dispatch ne binde que le worktree du dépôt
> cible. Leur état est donc inconnu, pas supposé mauvais.
>
> ## AC1 — établir l'état avant de changer quoi que ce soit
>
> Lire les trois consommateurs et rapporter, pour chaque site destructif : la
> primitive (`rm -rf`, `git worktree remove`, `rmdir`), d'où vient le chemin, et
> quelle validation il subit aujourd'hui. **Si les trois sont déjà conformes, le
> livrable est ce constat écrit** — pas un changement de code.
>
> ## AC2 — porter la sémantique, à l'identique
>
> Pour chaque site non conforme : une **allowlist positive**, alignée terme pour
> terme sur `_assert_removable_worktree_path` (`skills/bundled/_shared/dispatch-lib.sh`
> dans le dépôt `mika`). **Pas une denylist**, et pas une variante : deux
> sémantiques répondant différemment à « ce chemin est-il supprimable » est la
> divergence que mika#2158 a dû fermer une fois. La raison décisive est écrite
> dans `docs/solutions/cross-repo-patterns/garde-suppression-worktree-allowlist-2026-09-20.md`
> (dépôt `mika`, §2) : `/data/workspace/` est le disque d'une machine, pas une
> propriété du système.
>
> Un lecteur unique par dépôt. Refus **dit** sur `stderr` avec le chemin et le
> terme qui a échoué — un refus muet se lit exactement comme une absence de
> travail (mika#2205).
>
> ## AC3 — vérification par injection
>
> Neutraliser **un** terme à la fois et observer le test rougir à chaque fois.
> Neutraliser les quatre d'un coup ne prouve rien : une conjonction ne se teste
> pas en désarmant tous ses termes ensemble (mika#2277). Prévoir le garde-fou du
> garde-fou — une injection qui ne mord plus (marqueur renommé, termes fusionnés)
> doit rougir, pas passer au vert en n'ayant rien vérifié.
>
> **Contrôle négatif obligatoire :** un vrai chemin de worktree est accepté. Sans
> lui, une fonction qui refuse *tout* passerait la suite en vert tout en cassant
> chaque nettoyage (doctrine mika#2420).
>
> ## Hors périmètre
>
> L'inventaire du 28/07 (AC1 de mika#1943) — geste opérateur, procédure dans
> `docs/operator/exhumation-nettoyage-2026-07-28.md` du dépôt `mika`.
>
> ## Références
>
> - `senara-solutions/mika#1943` — la moitié livrée, et le raisonnement complet.
> - `senara-solutions/mika-platform#1694` — les couches A/B.
> - Sémantique canonique : `docs/solutions/cross-repo-patterns/garde-suppression-worktree-allowlist-2026-09-20.md` (dépôt `mika`).

## 6. Halte

Si un futur nettoyage a besoin de supprimer un chemin **hors** de
`.claude/worktrees/`, **ne pas élargir T4**. T4 est ce qui rend la garde vraie sur
toute machine ; l'élargir pour un cas d'usage la rend fausse pour tous les autres.
Le remède est un prédicat séparé, avec sa propre asymétrie explicitement
ré-arbitrée — et si ce prédicat finit par autoriser une racine de
`/data/workspace/`, c'est l'incident du 28/07 qui est ré-ouvert, pas une
commodité qui est ajoutée.
