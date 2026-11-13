---
title: Stale-base branches cause CONFLICTING PRs with no self-heal path
date: 2026-04-23
category: logic-errors
module: claude-pilot-handler
problem_type: logic_error
component: tooling
symptoms:
  - PR stuck in mergeable=CONFLICTING state for hours with no autonomous recovery
  - Sequential tickets dispatched after upstream merge inherit stale base
  - resolve-pr-conflicts skill exists but is never invoked by self-dev
  - Le callback d'un arrêt de dispatch répare dans le worktree et laisse une parente in_progress sans enfant vivant
root_cause: missing_workflow_step
resolution_type: code_fix
severity: high
tags:
  - claude-pilot
  - self-dev
  - rebase
  - merge-conflicts
  - worktree
  - stale-base
  - resolve-pr-conflicts
  - callback
  - task-engine
---

# Stale-base branches cause CONFLICTING PRs with no self-heal path

## Problem

PRs created by the autonomous dev loop sit in `mergeable=CONFLICTING` state indefinitely because two structural gaps prevent self-healing: (1) the claude-pilot handler checks out pre-committed branches without rebasing onto the latest `origin/main`, and (2) the `resolve-pr-conflicts` skill is loaded nowhere and triggered by nothing in self-dev.

## Symptoms

- PR #746 sat in `mergeable=CONFLICTING, mergeState=DIRTY` for 8+ hours
- Multiple pre-committed branches (`feat/338/...`, `feat/339/...`, `feat/740-742/...`) all branched from stale commit `6a0315be` while main had advanced to `65abe6f2`
- `self-dev/skill.toml` dependencies array did not include `resolve-pr-conflicts`
- `self-dev/system_prompt.md` had zero matches for `resolve_pr_conflicts`, `mergeable`, or `CONFLICTING`

## What Didn't Work

- The milestone grooming pattern pre-commits plan docs on feature branches before dispatch. When those branches are later checked out by the handler, they carry the stale base from grooming time — not from dispatch time.
- The handler's worktree fallthrough path (`git worktree add <WORKTREE> <branch>`) was designed for branch reuse but did not account for the branch being behind `origin/main`.

## Solution

Three targeted fixes in one PR:

**Fix 1 — Rebase-or-abort guard in `skills/bundled/claude-pilot/handlers/run.sh`:**

After worktree creation/reuse (both paths converge), check if the branch is behind `origin/main` and auto-rebase. On conflict, capture the file list BEFORE `rebase --abort` (abort resets the index) and exit with a structured `STATUS=REBASE_CONFLICT` discriminator:

```sh
BEHIND=$(git -C "$WORKTREE_DIR" rev-list --count HEAD..origin/main 2>/dev/null || echo 0)
if [ "$BEHIND" -gt 0 ]; then
    if git -C "$WORKTREE_DIR" rebase origin/main 2>/dev/null; then
        echo "Rebased ${BRANCH} onto origin/main (${BEHIND} commits caught up)." >&2
    else
        CONFLICTS=$(git -C "$WORKTREE_DIR" diff --name-only --diff-filter=U 2>/dev/null | tr '\n' ' ')
        git -C "$WORKTREE_DIR" rebase --abort 2>/dev/null || true
        RESULT="STATUS=REBASE_CONFLICT
Branch ${BRANCH} is ${BEHIND} commits behind origin/main.
Conflicted files: ${CONFLICTS:-<unable to capture>}
Resolve manually before re-dispatching ${REPO}#${ISSUE_NUM}."
        exit 1
    fi
fi
```

Key ordering: `diff --diff-filter=U` runs BEFORE `rebase --abort` because abort resets the index and the conflict markers are gone after.

**Fix 2 — Dependency declaration in `skills/bundled/self-dev/skill.toml`:**

Added `"resolve-pr-conflicts"` to the `dependencies` array. The BFS dependency resolver loads it whenever self-dev activates, making `resolve_pr_conflicts` available in mika-dev's tool inventory.

**Fix 3 — Routing sentence in `skills/bundled/self-dev/system_prompt.md`:**

Added one sentence in the "On success" callback path: check `mergeable` state via `gh pr view --json mergeable` before treating PR as ready; if `CONFLICTING`, invoke `resolve_pr_conflicts` per that skill's documented routing table. No routing-table duplication — the skill's own prompt owns the contract.

## Why This Works

**Root cause 1 (stale base):** The handler's worktree fallthrough path checked out existing branches at their current tip, which could be arbitrarily behind `origin/main`. The rebase guard catches this gap after both worktree paths converge — `origin/main` is already fresh from the fetch at line 233. For clean rebases (no conflicts), claude-pilot starts from a current base. For real conflicts, the structured `STATUS=REBASE_CONFLICT` exits early with diagnostic details instead of silently producing a CONFLICTING PR.

**Root cause 2 (capability not loaded):** The `resolve-pr-conflicts` skill existed with a working `resolve_pr_conflicts` tool, but self-dev didn't declare the dependency (BFS resolver never loaded it) and the prompt never checked mergeable state. Adding the dependency makes the tool structurally available; adding the routing sentence makes the agent check for the condition. This follows the `feedback_prompt_enforcement_fragile.md` principle: structural dependency > prompt enforcement.

## Prevention

- When adding new skills that should be invoked by an orchestrator skill, always add both: (1) the dependency in `skill.toml` so the tool is in the inventory, and (2) the trigger condition in the orchestrator's system prompt. A skill that exists but isn't loaded is invisible.
- When worktree reuse or branch checkout paths exist in handlers, always consider whether the branch may be behind the base. Fetch + rebase-or-abort is the standard pattern.
- The EXIT trap in `run.sh` checks `[ -z "$RESULT" ]` — populate `RESULT` before `exit 1` to deliver a structured callback instead of a generic "HANDLER CRASH" message.

## Follow-up: mid-session duplicate-commit guard (#784)

The #747 rebase-or-abort guard runs once at session START, but mid-session `git pull` or `git merge main` can reintroduce duplicate-hash copies of upstream commits onto the branch. This was observed in PR #782: after the startup guard ran cleanly, the claude-pilot session ran `git pull origin main` mid-session, creating commit `8693b3fd` — a duplicate of main's `14279524` (same author date, message, diff, different hash). GitHub's 3-way merge saw both commits touching the same lines → `mergeable=CONFLICTING`.

**Fix:** A pre-push `_check_duplicate_commits()` guard in `dispatch-lib.sh` uses `git log --cherry-mark --right-only origin/main...HEAD` to detect patch-equivalent commits before push. If duplicates are found, it attempts an automatic rebase onto `origin/main` (rebase naturally drops patch-equivalent commits). On rebase failure, push is skipped with a structured error message.

This closes the gap between the startup guard (#747) and the push boundary — duplicates introduced mid-session by any mechanism (pull, merge, cherry-pick) are caught before they reach GitHub.

See: `docs/solutions/logic-errors/mid-session-duplicate-commit-pre-push-guard-2026-05-26.md`

## Suite : la porte de rebase bute sur la branche d'un vol déjà mergé (mika#2976)

La porte de rebase du Fix 1 suppose que la branche reprise porte un travail en cours. Ce n'est plus vrai quand un ticket a plusieurs vols. Le nom de branche est dérivé du ticket (`scripts/derive-branch-name`), donc le groom du vol 2 dérive le nom du vol 1. Si la PR du vol 1 a été mergée en squash, la branche reprise ne porte plus que des commits dont le contenu est déjà dans `main`. La porte compte `rev-list --count HEAD..origin/main`, voit un retard, rebase, et rejoue ces commits contre leur propre squash : conflit, `STATUS=REBASE_CONFLICT`, groom sans plan ni PR. Trois occurrences en huit jours (mika#1727 le 2026-10-23, mika#2968 deux fois le 2026-10-30), sur origin comme en local seulement.

Le défaut est de compter des **commits** là où un squash demande de compter des **patchs**. Le correctif ne touche pas à la porte de rebase. Il pose avant elle une question qu'elle ne sait pas poser : *cette branche porte-t-elle un travail en cours ?* (`_branch_reuse_gate` dans `skills/bundled/_shared/dispatch-lib.sh`).

- **P1, les PR, en négatif.** « Aucune PR ouverte sur ce head », jamais « il existe une PR mergée » : deux PR peuvent partager un `headRefName`. Et **aucune PR du tout vaut `live`** : c'est une branche de groom poussée avec son plan, pas encore implémentée. La déclarer périmée détruirait ce plan.
- **P2, la containment par patch-id.** Le même idiome que le garde #784 : `git log --cherry-mark --right-only origin/main...<tip>`, sur la ref locale **et** sur la ref `origin/…` quand elles diffèrent, car la ref de suivi locale peut survivre à la branche distante. Zéro `>` : la branche se réinitialise sur `origin/main` (`git branch -f`, après retrait du worktree par le geste unique `_stash_and_remove_worktree`). Au moins un `>` : halte sur un statut neuf, `STATUS=BRANCH_HAS_UNMERGED_WORK`, qui nomme les commits et ne touche à rien.
- **Une non-mesure retombe sur la reprise d'aujourd'hui.** C'est l'inverse du faucheur de worktrees, et c'est voulu : ici l'action est destructive (réinitialiser), donc le repli sûr est l'échec bruyant déjà outillé, le `REBASE_CONFLICT`. La ligne `branch_reuse_gate:` le dit par `action=reuse_unmeasured`.

**Règle qui se généralise.** Une porte qui reprend un état existant doit d'abord savoir si cet état est vivant. Le compter (en retard de *n* commits) ne le dit pas : un état atterri par squash a l'air en retard, et le « rattraper » rejoue ce qui a déjà atterri.

**Le nettoyage asynchrone ne remplace pas la question synchrone.** `worktree_reaper` (mika#2420) finit par retirer worktree et branche locale d'une PR terminale. Mais il passe toutes les dix minutes, après 900 s de grâce, et un dispatch arrivé avant lui reprend la branche périmée. Dans le troisième cas, le premier groom a buté et le second est reparti sain, seulement parce qu'un tick du faucheur était passé entre les deux, 2 h 32 après le merge. Quand le résultat dépend de qui passe en premier, la porte doit poser la question au moment où elle décide.

### Vol 2 : la porte du vol 1 arrêtait tout vol suivant (mika#2976, 2026-10-31)

Le vol 1 a remplacé le `REBASE_CONFLICT` par un arrêt sûr, mais l'arrêt est tombé sur tout vol suivant, pour trois raisons mesurées par la sonde d'AC4.

**Un squash de plusieurs commits n'a le patch-id d'aucun d'eux.** `--cherry-mark` compare commit par commit. Un vol porte au moins deux commits (le plan puis le code), et le squash les fond en un seul patch. Le cas mesuré : `feat/2958/…` à `59010336a`, contre le squash `fcb5148fd`, sortait avec trois `>`. Le correctif ajoute un second étage, `_branch_cumulative_absorbed` : le patch-id du diff cumulé `merge_base..tip` est cherché dans `merge_base..origin/main`. Sur le cas réel, les deux valent `402470424f…`. Les deux côtés se calculent en `-U0`, et c'est indispensable : `patch-id --stable` ignore numéros de ligne et espaces, mais il **hache les lignes de contexte**. Le plan du vol 2 affirmait le contraire, et la revue l'a réfuté : un commit tiers atterri sur `main` avant le squash, à trois lignes ou moins d'un hunk de la branche, change le contexte du squash et donc son patch-id (`854db6a4…` contre `4259ad0b…` en `-U3`, `eede3fa7…` des deux côtés en `-U0`). Le cas réel passait en `-U3` par chance. T9 construit maintenant cette proximité et vérifie que les patch-ids en `-U3` diffèrent.

**Deux critères qui semblent justes et que la mesure réfute.** Ne pas les réessayer :

- *Le diff entre `origin/main` et la tête, restreint aux fichiers de la branche, est vide.* Sur le cas réel il ne l'est pas (3 fichiers, +3 −5). L'écart est un champ apporté sur `main` par un autre ticket, que la branche n'avait jamais vu. Ce diff confond ce que la branche apporte (le danger) et ce que `main` apporte en plus (du retard, sans danger) : un critère de non-perte doit être directionnel.
- *`git merge-tree --write-tree origin/main <tip>` rend l'arbre de `origin/main`.* C'est vrai au point d'atterrissage du squash, mais contre la tête réelle de `main` il conflicte dès que `main` a réécrit les mêmes lignes après l'atterrissage.

T9 de `test-dispatch-lib.sh` reproduit cette topologie et épingle les deux critères : les réintroduire fait rougir le test.

**Le plan du groom courant n'est pas un travail laissé derrière.** Au dispatch de dev-pilot d'un vol suivant, la branche recréée depuis `main` porte le plan que le groom vient de frapper, et la seule PR de ce nom est la PR mergée du vol précédent. La porte prenait ce plan pour du travail en péril (dev-pilot `0be4647a`, plan `0c9203aaf`, #2985). La règle retenue partitionne par la **date** : un commit daté après le `mergedAt` de la dernière PR terminale est le vol courant, il n'est pas jugé, et sa présence donne `live`. La topologie ne suffit pas : la tête mergée n'est pas un ancêtre d'une branche recréée, et un commit local jamais poussé avant le merge en descend. Les deux sens d'erreur restent sûrs : un commit réécrit par un rebase reçoit une date neuve et conduit à la reprise, jamais à une réinitialisation ; un commit mal daté vers le passé passe par les deux étages, qui l'arrêtent s'il manque à `main`.

**Une ref de suivi survit à sa branche.** Un `git fetch` sans `--prune` ne retire jamais `refs/remotes/origin/<b>`. Restée sur l'ancienne tête, elle suffisait à faire buter la porte. Le correctif demande au distant si la branche existe (`_remote_branch_exists`, mika#2626) avant de lire la ref. Absente : la ref est ignorée et la ligne la nomme (`stale=`). Pas de réponse : la ref est lue comme avant, ce qui peut arrêter mais jamais réinitialiser.

**Ce qu'on retient.** Une non-perte se mesure sur le contenu, dans un seul sens, et sur la population que la porte doit juger. Le vol 1 mesurait la bonne chose (les patchs) dans la mauvaise unité (le commit), et sur une population trop large (tout ce que porte la branche, y compris le vol en cours et une ref que le distant a oubliée). La ligne `branch_reuse_gate:` porte maintenant `containment=` (`per_commit`, `cumulative`, `work`, `unmeasured`), `current=` et `stale=`, pour que le relevé d'AC4 sépare un vrai travail en péril d'une non-mesure.

### Vol 3 : le second visage — le callback de l'arrêt répare hors pipeline, et la parente ne survit pas (mika#2976, AC5)

Les deux vols précédents ont fermé la porte. Le défaut avait un second visage, en aval : **le tour de callback qui reçoit la bannière d'arrêt tentait la réparation lui-même**, dans le worktree, hors du pipeline. Mesuré le 2026-10-30 (session `callback-233cab88-90f9-4fb8-a43c-7e49015bcdd7`) : `git rebase origin/main` à 09:30:43Z, puis `git checkout --theirs` sur le fichier de plan à 09:31:28Z — dans un rebase, « theirs » est le commit rejoué, donc l'ancien brouillon du vol 1 l'emportait sur la version finale de `main`, 14 insertions contre 22 suppressions, indexées — puis « max steps exhausted » à 09:32:07Z. Restaient un worktree en HEAD détachée à `a219d0a3` avec `rebase-merge` ouvert à 1/7, et la parente de groom `19c11b09` figée en `in_progress` alors que son seul enfant `1f838a67` était `delivered` et qu'aucun processus ne tournait. Trois gestes à la main pour nettoyer.

**Le modèle n'a pas dérivé : il a suivi le prompt.** Le routage du callback de groom teste launcher-dead, `Outcome: ESCALATE`, `PIPELINE FAILURE:`, puis *« Otherwise → route to the "On failure" handler (Step 4.5) »*. Or aucune des deux bannières d'arrêt ne porte le préfixe `PIPELINE FAILURE:` — elles tombaient donc dans le dernier « Otherwise », et Step 4.5 s'intitule *« Diagnose and recover from failure »*, point 3 : *« Attempt recovery (max 1) »*. Le défaut était **un discriminateur absent**, pas une désobéissance. C'est la règle générale : quand une branche de routage se termine par un « sinon » qui invite à réparer, tout producteur neuf de bannière hérite de cette invitation sans que personne l'ait décidé.

Le correctif est en deux moitiés, qui ne vivent ni dans le même fichier ni dans la même couche — et **l'asymétrie entre les deux est le fait à retenir** :

- **Moitié A, prescriptive.** Un sixième discriminateur dans `skills/bundled/self-dev-callback/system_prompt.md`, placé **avant** `Pipeline result classification` : prédicat en table close de deux valeurs sur la **première ligne** du `RESULT` (`STATUS=REBASE_CONFLICT`, `STATUS=BRANCH_HAS_UNMERGED_WORK` — jamais `STATUS=` nu, qui avalerait `CANCELLED_BY_OPERATOR` et `CANCELLED_BY_SIGNAL` et rendrait une annulation opérateur indistinguable d'un arrêt de branche), puis sept pas : aucune réparation, aucune relance, pas d'incrément de `pipeline_retry_count`, aucun label touché, `update_task_status(task_id, "blocked")` avec `{"dispatch_halt": true, "halt_status": …}`, `send_message` du `RESULT` verbatim, fin de tour.
- **Moitié B, structurelle.** `parse_dispatch_halt_status` + `try_terminate_parent_on_dispatch_halt` dans `crates/mika-agent/src/task_engine/dispatcher.rs`, septième frère de la famille de hooks `try_*` de `deliver_callback`. Ces hooks tournent **avant** le tour silencieux — le commentaire du site le disait déjà : *« persist it to the parent task BEFORE the silent agent runs … even if the agent exhausts its step budget »*. La parente passe `failed` avec un motif `dispatch_halt_<status>`, un événement d'audit `dispatch_halt_resolver` et un `info!` `dispatch_halt_parent_terminated`.

**L'asymétrie, dite plutôt que supposée.** Un mock valide la forme du prompt, jamais l'obéissance du modèle (doctrine mika#1190). La moitié A reste donc prescriptive : rien n'empêche structurellement un tour futur de lancer un `git rebase`. Ce qui est garanti structurellement, c'est que **ce rebase ne coûte plus la parente** — elle est déjà terminale quand le tour prend la main, et la relance n'est plus bloquée. Les trois gestes manuels du 2026-10-30 se réduisent aux deux premiers, qui ne bloquent rien. Fermer la moitié A structurellement demanderait une garde EndTurn refusant un outil mutant un worktree dans un tour de callback portant une bannière d'arrêt ; elle **n'est pas livrée**, et pas par oubli : c'est une garde sur le domaine entier des tours de callback, dont la population — combien de tours de callback touchent légitimement un worktree aujourd'hui ? — n'est pas mesurée, et une garde dont on ne connaît pas la population qu'elle va mordre est la forme que mika#2309 condamne. Suivi : mika#2976 (garde EndTurn du worktree en tour de callback).

**La place dans le prompt EST le correctif, et rien de comportemental ne le dit.** Déplacé sous `Pipeline result classification`, le discriminateur redevient inopérant — les deux bannières retombent dans le « Otherwise » — sans qu'aucune assertion de comportement ne rougisse. Deux tests d'ordre dans `dispatcher.rs` lisent donc le prompt par `include_str!` et comparent les positions ; leur allowlist (`ORDER_SCAN_ALLOWLIST`) est livrée **vide** et pinée vide par un test frère, le compte de départ mesuré par le scan lui-même. Qui « range » ce fichier doit les faire passer.

**Ce que ce vol ajoute à la sonde d'AC4**, sans en toucher le compte : un terme lisible. La limite nommée par le vol 2 — la ligne `[dispatch-lib] branch_reuse_gate: …` n'est écrite dans aucun journal persistant — reste entière ; mais `dispatch_halt_parent_terminated` et le `tool_name` `dispatch_halt_resolver` sont en base, donc comptables. Le relevé suivant pourra dire *combien d'arrêts de dispatch ont rendu leur parente terminale sans geste humain*. L'**absence** de ces lignes signifie un binaire antérieur à ce vol (motif mika#2340), jamais une absence d'arrêt — la moitié A, elle, ne mord qu'après `make deploy`, ce qui est pourquoi la moitié B part en premier : entre les deux, un callback d'arrêt peut encore tenter une réparation, mais sans laisser de parente zombie.

**Deux termes qui ne se somment pas.** Le motif d'échec est `dispatch_halt_<status>`, pas `callback_no_pr_*` : les deux `RESULT` d'arrêt portent aussi `NO_PR: no_pr_on_branch`, que `parse_no_pr_reason` lit déjà, et réutiliser ce motif ferait écrire aux deux producteurs les mêmes octets (classe mika#2205). Deux populations sous un nom ne sont pas soustractibles.

**Pourquoi le faucheur ne suffisait pas, encore une fois.** `find_orphaned_parent_tasks` porte neuf prédicats, et aucune mesure depuis un bac à sable de pilote ne sépare celui qui a exclu `19c11b09` : la base de production n'y est pas joignable. Ce vol n'y touche donc pas — *un verdict non mesuré ne se code pas*. Ce qu'il obtient sans cette mesure : la parente devient terminale **au moment de la livraison**, soit au plus tard 600 s avant que le faucheur n'en devienne la question. Les deux ne se contredisent pas : `update_task_failed` est un UPDATE gardé sur l'état terminal, donc le second à passer ne fait rien et le dit (`Ok(false)`).

## Related Issues

- [#747](https://github.com/senara-solutions/mika/issues/747) — this fix
- [#746](https://github.com/senara-solutions/mika/pull/746) — motivating PR (stuck CONFLICTING)
- [#744](https://github.com/senara-solutions/mika/issues/744) — dashboard observability gap that let the stuck state stay hidden
- `feedback_prompt_enforcement_fragile.md` — structural > prompt-level enforcement principle
