---
title: "Une ref de suivi locale ne dit pas si le distant porte la branche — demander au distant, par un lecteur unique"
date: 2026-10-01
category: dev-loop
module: skills/bundled/_shared/dispatch-lib.sh
component: development_workflow
problem_type: logic_error
severity: high
ticket: mika#2626
symptoms:
  - "`Push: FAILED — remote advanced since fetch (lease aborted); commits remain local-only` alors que le distant n'avait AUCUNE ref pour la branche"
  - "`! [rejected] … (stale info)` sur un `--force-with-lease` en mode `diverged`"
  - "rescue de mika#1960 phase 2 (pilote 624656b1, 90,40 USD, ~770 lignes) resté local-only ; un push nu à la main rendait `* [new branch]`"
  - "`_push_with_rebase_retry` épuise ses 2 tentatives et rebase l'historique local sur la foi d'un faux diagnostic"
root_cause: logic_error
resolution_type: code_fix
related_components:
  - background_job
tags:
  - loop-substrate
  - dispatch-lib
  - push
  - force-with-lease
  - git-fetch-prune
  - ls-remote
  - stale-tracking-ref
  - predicate-drift
  - single-reader
  - branch-reuse
---

# Une ref de suivi locale ne dit pas si le distant porte la branche — demander au distant, par un lecteur unique

## Le problème

`_push_branch` (`skills/bundled/_shared/dispatch-lib.sh:5829`) décidait si le
distant portait la branche en lisant la **ref de suivi locale**
(`git rev-parse --verify "origin/$BRANCH"`). Quand une branche est **réutilisée
entre phases** (pratique adoptée le 2026-10-01), la forge supprime la branche de
la phase précédente au merge de sa PR, mais le checkout partagé garde
`origin/<branche>` : une ref **orpheline**. Elle répondait « présente », le mode
devenait `diverged`, et le `--force-with-lease` partait contre un SHA que le
distant ne portait plus. Le travail du pilote restait local-only.

Incident fondateur : rescue de mika#1960 phase 2, 2026-10-01, 90,40 USD de
travail non publié, rattrapé à la main par un push nu.

## Symptômes

```
 ! [rejected]        test/1960/phase -> test/1960/phase (stale info)
_push_with_rebase_retry: race-shaped rejection on attempt 1/2 — fetching + rebasing
_push_with_rebase_retry: exhausted 2 attempts on race errors — bailing to draft rescue
Push: FAILED — remote advanced since fetch (lease aborted); commits remain local-only on test/1960/phase
```

La dernière ligne est un **énoncé faux** : le distant n'avait rien avancé, il
n'avait plus de ref du tout. Cascade aval : `gh pr create` échoue faute de
branche distante, d'où `NO_PR: rescue_pr_create_failed`.

## Ce qui n'a pas marché — trois pièges, chacun mesuré

Mesures du plan `docs/plans/2026-10-01-004-fix-2626-push-branch-ref-origin-orpheline-plan.md`
(§ R1 à R3), sur git 2.53.0, fixture « dépôt bare + clone » où la branche est
supprimée côté bare par `update-ref -d`.

### 1. `git fetch --prune origin <branche>` est inopérant sur une branche supprimée

C'était l'option A de l'AC1 du ticket, et l'intuition naturelle. Elle ne fait
rien :

| forme | rc | prune la ref orpheline ? |
|---|---|---|
| `git fetch origin "$BRANCH"` (code d'origine) | 128 | non |
| `git fetch --prune origin "$BRANCH"` | **128** | **non** |
| `git fetch --prune origin refs/heads/X:refs/remotes/origin/X` | 128 | non |
| `git fetch --prune origin` — **nu** | 0 | oui |
| `git ls-remote --exit-code origin refs/heads/X` | **2** | n/a, ne mute rien |

Git échoue sur `fatal: couldn't find remote ref <branche>` **avant d'atteindre le
prune**. Après les formes ciblées, `rev-parse` montre la ref orpheline intacte.

Seul le fetch **nu** prune, et il est écarté : il fetche toutes les branches à
chaque dispatch ; il **mute les refs de suivi du common dir, partagé par tous les
worktrees** du checkout, donc il prunerait aussi celles d'autres dispatches en
vol ; et surtout il ne répond pas à la question posée — il modifie l'état local
pour y répondre de biais.

### 2. Deux prédicats différents pour un même fait, dans un même fichier

`_set_up_worktree` (`dispatch-lib.sh:3160`) posait **déjà** la bonne question par
`ls-remote --exit-code`. Dans le même dispatch, elle a répondu « absente » (le
worktree a été basé sur `origin/main`), puis `_push_branch` a répondu « présente »
par la ref locale. Les deux sites, à ~2 300 lignes d'écart, ont divergé sans que
rien ne le signale. C'est la troisième occurrence du motif dans la maison, après
mika#2158 (voir `docs/solutions/dev-loop/two-predicates-for-one-concept-livelock-2026-09-03.md`) et
mika#2484.

### 3. Le court-circuit `ahead == 0` contre une ref orpheline masque un push dû

Le no-op « rien à pousser » (état (a) de mika#1407) compte
`origin/$BRANCH..HEAD`. Contre une ref orpheline, ce compte peut rendre 0 alors
que **tout** est à pousser : un `return 0` muet. Un correctif qui ne déplacerait
que le choix de mode laisserait cette moitié silencieuse ouverte.

### Bonus : le retry de mika#1857 aggrave au lieu de sauver

`(stale info)` matche le grep race-shape (`rejected|fetch first|remote contains
work`). Le retry fetche `origin/main`, **rebase l'historique local**, retente le
lease et épuise ses tentatives : deux pushes et un rebase gratuits, sur un faux
diagnostic. Non modifié par le correctif — une fois le mode juste, cette
population ne l'atteint plus.

## La solution (PR senara-solutions/mika#2629, ouverte en draft à l'écriture)

**Un lecteur unique, qui demande au distant, avec un rc à trois issues.**

```sh
_remote_branch_exists() {            # dispatch-lib.sh:5818
    local repo_dir="$1" branch="$2"
    local rc=0
    git -C "$repo_dir" ls-remote --exit-code origin "refs/heads/$branch" >/dev/null 2>&1 || rc=$?
    case "$rc" in
        0) return 0 ;;   # PRÉSENTE
        2) return 1 ;;   # ABSENTE
        *) return 2 ;;   # INDÉTERMINÉE — le distant n'a pas répondu
    esac
}
```

La sémantique est **contractuelle**, pas observée par accident :
`git ls-remote --help` documente *« Exit with status "2" when no matching refs
are found in the remote repository »*. Tout autre rc non nul (128 : réseau,
auth, remote inexistant) veut dire « pas de mesure ».

- **`_set_up_worktree`** délègue au lecteur (`dispatch-lib.sh:3463`) sans changer
  de valeur de vérité : l'issue indéterminée rend `if` faux, comme le rc 128
  d'avant.
- **`_push_branch`** décide le mode depuis le lecteur : présente → comportement
  inchangé (fast-forward ou `--force-with-lease`) ; absente → `first-push`, quoi
  que dise la ref locale ; indéterminée → **repli nommé** sur la ref locale (le
  comportement d'avant), avec une ligne `Remote-probe:` dans le `RESULT`.
- Le court-circuit `ahead == 0` (`dispatch-lib.sh:5962`) ne s'applique **que sous
  la branche « présente »**.
- Le diagnostic cesse de mentir : une ligne `Stale-ref:` nomme la ref orpheline
  et son SHA même quand le push aboutit (`dispatch-lib.sh:5940`), et « remote
  advanced since fetch » (`dispatch-lib.sh:6051`) n'est émis que si le distant
  porte la branche.
- Deux drapeaux distincts, `remote_has_branch` (une **décision**, qui retombe sur
  la ref locale faute de mesure) et `remote_known_absent` (un **fait** mesuré) :
  les confondre ferait dire « le distant n'a pas la branche » sur un `ls-remote`
  qui n'a pas répondu.

Rien n'est élagué : le correctif cesse de **croire** la ref orpheline, il ne la
supprime pas. L'hygiène des refs du checkout partagé est un suivi distinct, dont
la ligne `Stale-ref:` compte la population d'abord.

## Pourquoi ça marche

La question « le distant porte-t-il cette branche *maintenant* ? » n'a qu'une
source de vérité : le distant. Une ref de suivi est un **cache local** de la
dernière fois qu'on a regardé, et rien ne l'invalide quand la forge supprime une
branche au merge. `ls-remote --exit-code` pose la question sans muter d'état
partagé, et son rc sépare proprement « absente » de « je ne sais pas ».

Le lecteur unique ferme la classe, pas l'instance : tant que deux sites posaient
la même question par deux prédicats, l'un pouvait dériver sans que l'autre le
voie.

## Prévention

- **Ne jamais déduire l'existence distante d'une ref `origin/*`.** Une ref de
  suivi sert à lire un SHA déjà fetché, pas à dire si la branche existe encore.
  Corollaire : `fetch --prune <ref>` n'élague pas une ref absente du distant ;
  ne pas le proposer comme remède.
- **Un fait, un lecteur.** Quand un fichier pose deux fois la même question,
  extraire le lecteur et y faire déléguer les deux sites. Ne pas unifier pour
  autant des questions voisines : les deux autres `ls-remote` de
  `dispatch-lib.sh` (`PRE_RUN_REMOTE_HEAD`, `_check_pilot_force_push`) lisent un
  **SHA**, pas l'existence.
- **Un rc à trois issues reste à trois issues.** « Indéterminé » ne se replie ni
  sur « absent » ni sur « présent » en silence : le repli est nommé et écrit dans
  une surface lue (le `RESULT`, pas le stderr d'avant-pilote, perdu sur un
  dispatch qui réussit — classe mika#2050).
- **Un court-circuit « rien à faire » se garde par la même prémisse que la
  décision qu'il court-circuite.** Un `ahead == 0` compté contre une ref dont on
  n'a pas établi la validité est un no-op qui peut avaler du travail payé.
- **Tests** (`skills/bundled/_shared/test-dispatch-lib.sh`, blocs `Test mika#2626`) :
  fixture par `git -C "$FIXTURE_BARE" update-ref -d "refs/heads/$br"` — **jamais**
  `git push origin --delete` depuis le clone, qui supprime aussi la ref de suivi
  locale, c'est-à-dire le fixture même. Le test AC2 est vu rouge avant le
  correctif avec `mode=diverged` / `Push: FAILED` ; un rouge pour une autre raison
  ne confirme rien. Les tests 12a-12h (lease légitime) passent sans modification.
  Le scan V5 refuse tout `ls-remote --exit-code` sur `refs/heads/` hors du
  lecteur, **et** affirme exactement deux appelants (anti-vacuité, classe
  mika#2205).

## Références

- Ticket : senara-solutions/mika#2626 ; PR : senara-solutions/mika#2629
- Plan : `docs/plans/2026-10-01-004-fix-2626-push-branch-ref-origin-orpheline-plan.md`
- Motif voisin : `docs/solutions/dev-loop/two-predicates-for-one-concept-livelock-2026-09-03.md` (mika#2158)
- Contrats de push antérieurs : mika#1364 (lease), mika#1407 (décision de push au code), mika#1857 (retry)
- Récupération manuelle du travail non poussé : `docs/solutions/best-practices/recover-unpushed-claude-pilot-work-2026-04-27.md`
