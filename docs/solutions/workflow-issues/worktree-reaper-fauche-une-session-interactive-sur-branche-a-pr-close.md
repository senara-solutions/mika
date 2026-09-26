---
title: Le faucheur de worktrees retire le worktree d'une session interactive vivante quand la PR précédente de la branche est close
date: 2026-09-26
category: workflow-issues
module: worktree_reaper, claude-code-harness
problem_type: workflow_issue
component: tooling
severity: medium
applies_when:
  - "Reprendre en session interactive (spawn /mika, CC) une branche dont une PR antérieure est CLOSED — typiquement un sauvetage wip fermé, branche préservée pour ré-implémentation"
  - "Un worktree sous .claude/worktrees/ disparaît en cours de session, branche locale comprise, sans commande de votre part"
tags: [worktree_reaper, live-process-guard, proc-cwd, closed-pr, rescue, interactive-session]
---

# Le faucheur de worktrees retire le worktree d'une session interactive vivante quand la PR précédente de la branche est close

## Contexte

mika#2054, 2026-09-26. La PR de sauvetage #2546 de la branche
`fix/2054/verify-egress-no-log-cfg-test` a été **fermée sans merge**, la
branche distante explicitement préservée pour une ré-implémentation. Une
session `/mika` interactive (spawn Claude Code) a recréé le worktree
`.claude/worktrees/fix-2054-verify-egress-no-log-cfg-test/mika` et y a
travaillé. Le worktree **et la branche locale** ont disparu deux fois en
pleine revue, sans aucune commande de la session.

`/var/log/mika/server.log`, événement `worktree_reaped`,
`message: "worktree_reap: worktree de PR terminale retiré"`, `pr_number: 2546`,
`pr_state: "CLOSED"`, `branch_deleted: true`, `disposition: "armed"` — quatre
fois ce jour-là : 15:50Z, 16:40Z, 17:10Z, 17:20Z. Les deux derniers pendant la
session interactive.

## Le mécanisme

Le faucheur (`crates/mika-agent/src/worktree_reaper.rs`, mika#2420) retire
tout worktree géré dont la PR la plus récemment close l'est depuis plus que la
grâce (900 s par défaut), **sauf** si l'un de ses refus s'applique. Les refus
qui protègent le travail sont bien là : arbre sale (`REASON_DIRTY`), commits
non poussés (`REASON_UNPUSHED_COMMITS`), PR ouverte (`REASON_PR_OPEN`). Du
travail non commité ou non poussé **n'est donc pas perdu** — le faucheur ne
touche qu'un arbre propre et poussé, et la branche distante reste intacte.

Ce qui manque est la présence de la session. Le refus « processus vivant »
(`REASON_LIVE_PROCESS`, `worktree_reaper.rs:1101-1112`) lit `/proc/<pid>/cwd`
de chaque processus (`collect_live_cwds`, `:1410`) et épargne un worktree dont
un cwd est à l'intérieur. Or l'outil Bash de Claude Code **ramène le cwd à la
racine du projet après chaque appel** : entre deux commandes, aucun processus
de la session n'habite le worktree. Un pilote headless, lui, y a son cwd pour
toute sa durée — c'est la population pour laquelle la garde a été écrite.
Résultat : dès que l'arbre redevient propre (juste après un commit poussé,
ou avant la première édition), le tick suivant (toutes les dix minutes) le
retire sous la session.

Un second facteur rend le cas récurrent plutôt que rare : une branche dont
un sauvetage a été **fermé** puis ré-implémenté garde sa PR CLOSED comme
« PR la plus récemment close » tant qu'aucune nouvelle PR n'est ouverte. Le
faucheur ne distingue pas « PR close, travail livré » de « PR close, travail
repris ».

## Ce qui a marché

Ancrer un processus de longue durée dont le cwd est le worktree — c'est
exactement la preuve de vie que le faucheur sait lire :

```bash
# Bash, run_in_background: true
cd /data/workspace/mika-platform/.claude/worktrees/<slug>/mika && exec sleep 14400
```

Vérifier l'ancrage :

```bash
for p in $(pgrep -x sleep); do readlink /proc/$p/cwd; done | grep <slug>
```

Le tick de 17:30Z a épargné le worktree, et aucun `worktree_reaped` n'a suivi
tant que le gardien vivait. Ouvrir la nouvelle PR tôt ferme aussi la
fenêtre (`REASON_PR_OPEN`).

## Ce qui ne marche pas

- `cd` dans le worktree en début de session : le harness remet le cwd à
  chaque appel.
- Recréer le worktree après chaque disparition : il repart au tick suivant
  dès que l'arbre est propre.
- Chercher le coupable parmi les crons ou les scripts `mika-platform-*` : le
  retrait vient du moteur, et ne se lit que dans `/var/log/mika/server.log`
  (`grep worktree_reaped | grep <branche>`), pas dans les logs par agent.

## Quand appliquer

Toute session interactive qui travaille dans `.claude/worktrees/` sur une
branche portant déjà une PR close. Poser le gardien **avant** la première
commande longue (revue multi-agents, build), et le lever en fin de session.
Le correctif de fond — que le faucheur reconnaisse une session interactive,
ou traite « PR close + branche reprise » autrement que « PR close + travail
livré » — relève du faucheur lui-même, pas de cette consigne.

## Voir aussi

- `docs/solutions/cross-repo-patterns/garde-suppression-worktree-allowlist-2026-09-20.md`
- mika#2482 — `pr_unknown` confond travail vivant et vieux-sans-PR : autre
  angle mort du même faucheur, sur la même question « ce worktree est-il
  vivant ? ».
