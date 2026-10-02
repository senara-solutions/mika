---
module: skills/bundled/_shared/dispatch-lib.sh
date: 2026-10-02
problem_type: best_practice
component: dispatch-lib
severity: high
applies_when: "un classeur tardif doit savoir ce qu'un rescue a fait plus tôt dans le même dispatch"
tags: [dispatch-lib, rescue, producer-stamp, compound-traversal, census, mika2631, loop-substrate]
category: best-practices
---

# Un fait passé a autant de stamps que de producteurs, et on recense les producteurs par site (mika#2631)

## Contexte

`_rescue_compound_traversal` rendait `not-applicable` dès que `STATUS = success`, avec
pour raison « la session a conclu, rien n'a été tronqué ». Le 2026-10-01, un pilote a
lancé ses relecteurs, écrit « waiting on them » et rendu la main (cpp#267) : session
`success`, rien de commité. Le rescue a commité son travail et ouvert la PR #2630, dont
le corps portait `compound-traversal: not-applicable`. L'armement
`MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=1` ne l'a donc pas parquée. **Conclure n'est
pas livrer.**

## Leçon 1 : le fait qu'on cherche a disparu au moment où on le lit

Le ticket décrivait la condition comme « HEAD inchangé + worktree sale ». Quand le
classeur tourne (dans `dispatch_claude_pilot`, après le commit de rescue et le push),
les deux moitiés se lisent **à l'envers** :

- le rescue a réécrit `POST_RUN_HEAD` sur son propre commit, donc `PRE != POST` est vrai ;
- il a tout stagé et commité, donc `git status` est propre.

Un prédicat qui relit l'état git détecte exactement le contraire de ce qu'il cherche. La
seule trace qui survit est un **stamp posé par le producteur** au moment où il agit
(`RESCUED_DIRTY_WORKTREE=1`). C'est le patron de
[pr-origin-stamped-by-producer-not-inferred](pr-origin-stamped-by-producer-not-inferred-2026-08-30.md).
Il reste deux points à vérifier à chaque fois : le stamp est **remis à zéro à chaque
dispatch** (`_run_claude_pilot`) pour ne pas fuir d'un dispatch au suivant, et il est lu
avec un test exact (`= "1"`), si bien qu'une valeur illisible garde le comportement
antérieur au lieu d'accorder une permission nouvelle.

Le test qui le prouve **pilote le vrai producteur** et ne se contente pas de poser la
variable à la main. T15k appelle `_rescue_dirty_worktree` sur un dépôt réel et vérifie
les trois faits : stamp à 1, HEAD avancé, arbre propre. Un cas qui poserait le stamp
lui-même ne testerait que le plan.

## Leçon 2 : recenser les producteurs par site d'auto-commit, pas par classe de recovery

Le plan avait raisonné par `RECOVERY_CLASS`. Il retenait `dirty-worktree` et écartait
`commit-pushed-no-pr` au motif que « le pilote a commité son propre travail ; le seul
commit est le marqueur vide `wip(mika#1383)` ». C'était vrai du marqueur, mais faux du
**même bloc** : sa Phase A commite le contenu que le pilote a laissé sale *après* avoir
commité une partie de son travail (`trailing content after pilot end_turn`). C'est un
auto-commit « à la place du pilote » au sens exact du ticket, et la classe se lit quand
même `commit-pushed-no-pr`. Deux relecteurs indépendants l'ont trouvé en revue
(correctness et adversarial). Les tests du plan étaient tous verts.

Une classe de recovery est une **étiquette dérivée**. Un même fait (« dispatch-lib a
commité du contenu du pilote ») peut avoir plusieurs producteurs sous plusieurs
étiquettes. On recense les producteurs à la source :

```bash
grep -nE 'git -C "\$[A-Za-z_]+" commit' skills/bundled/_shared/dispatch-lib.sh
```

Le 2026-10-02, ce grep rend cinq sites, et chacun reçoit un verdict :

| site | ce qu'il commite | contenu du pilote non commité ? |
|---|---|---|
| normalisation `style(…)` (mika#2348) | le reformatage du travail **déjà commité** par le pilote | non |
| rescue dirty-worktree, deux sites (mika#1282) | tout l'arbre sale d'une session à zéro commit | **oui** → `RESCUED_DIRTY_WORKTREE` |
| Phase A `trailing content` (mika#1383) | le reste sale après des commits du pilote | **oui** → `RESCUED_TRAILING_CONTENT` |
| marqueur vide `wip(mika#1383)` (`--allow-empty`) | rien | non, exempté (T15q) |

`_rescue_committed_in_the_pilots_place` lit les deux stamps. Quand un nouveau site de
commit apparaît dans le rescue, il faut lui donner un verdict dans ce tableau.

## Applicabilité

Ces règles valent pour tout lecteur tardif d'un fait du rescue : marqueurs de corps de
PR, classes, signaux. Avant d'écrire le prédicat, il faut se demander si l'état sera
encore lisible à l'instant de la lecture. Si la réponse est non, chaque producteur pose
un stamp. Pour énumérer les producteurs, on greppe les sites d'action eux-mêmes, jamais
les étiquettes qui en dérivent.
