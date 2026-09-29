---
module: skills/bundled/_shared/dispatch-lib.sh
component: dispatch-lib
date: 2026-09-29
problem_type: logic_error
severity: medium
ticket: mika#2144
category: workflow-issues
tags: [autonomous-loop, dispatch-lib, worktree, resume, git-stash, scaffold, untracked, porcelain-z, fail-closed]
symptoms:
  - "136 des 152 stashes lisibles du dépôt ne touchaient que de l'échafaudage (docs/plans/, .iterate/, .claude/commands/, groom-verdict-trail.log)"
  - "11 stashes le seul 2026-09-01, chacun contenant un unique docs/plans/…-plan.md non committé"
  - "la ligne « resume-cleanup stashed … recover with » imprimée pour du bruit, donc ignorée, enterrant les 16 stashes de code réels"
root_cause: logic_error
resolution_type: code_fix
---

# Le filet de reprise stashait son propre échafaudage, parce qu'un `checkout HEAD --` ne mord pas sur un untracked

## Problème

`_clean_worktree_for_rebase` (`dispatch-lib.sh:2339`, mika#1414) a trois tiers. Le
Tier 2 remet à HEAD les quatre chemins d'échafaudage que dispatch-lib possède ; le
Tier 3 stashe « le résidu vraiment inattendu », puis `reset --hard` + `clean -fd`.
Le doc mika#1414 (`dispatch-lib-dirty-worktree-resume-rebase-2026-06-06.md`) affirme
que le Tier 2 « garde ces chemins hors du stash de récupération ». **C'est vrai
seulement pour leurs versions TRACKÉES.**

`git checkout HEAD -- docs/plans/` restaure les entrées connues de l'arbre et ne
supprime **jamais** un fichier untracked — comportement git. Un pilote de groom qui
écrit `docs/plans/<date>-<nnn>-…-plan.md` puis meurt avant de committer (signature
mika#2141) laisse un `?? docs/plans/…` que le Tier 2 ne peut pas toucher ; le Tier 3
le stashe, imprime une ligne de récupération, et `clean -fd` le supprime de toute
façon. Résultat mesuré : 8,5 stashes de bruit pour 1 à examiner.

## Ce qui n'a pas marché (les hypothèses à ne pas refaire)

- **« `.iterate/` est ignoré mais `stash --include-untracked` le capture quand
  même »** — faux. `--include-untracked` n'inclut pas les fichiers **ignorés**
  (c'est `--all`), et `status --porcelain`, qui arme le Tier 3, ne les liste pas non
  plus. La vraie cause : le nettoyage tourne **avant** le rebase, donc sous le
  `.gitignore` et l'**index de la branche**, pas de `main`. Sur une branche antérieure
  à l'untrack de `.iterate/` (mika#1862), ces chemins sont encore **trackés** — et un
  fichier tracké reste tracké quoi que dise `.gitignore`.
- **Étendre le Tier 2 par un `git clean -fd -- docs/plans/ …`** — écarté : change le
  nettoyage et la décision au même endroit, et un cinquième chemin rouvrirait le trou.
  La question est de **classement**, pas de nettoyage.
- **Classer `.claude/` en entier comme échafaudage** (formulation du ticket) — refusé :
  `.claude/settings.json` et `.claude/claude-pilot.json` sont du contenu tracké du
  dépôt ; leur modification cesserait d'être stashée.

## Solution

Le Tier 3 **décide** avant de stasher ; le nettoyage est inchangé dans les deux
branches.

- `_is_scaffold_path` (`dispatch-lib.sh:2186`) — **site unique** des quatre motifs
  du Tier 2, consommé par le Tier 3 et par `_rescue_diff_carries_work`
  (`dispatch-lib.sh:8063`), qui garde localement ses deux motifs propres (issus des
  exclusions `git add -A` du rescue, pas du Tier 2).
- `_residue_is_scaffold_only` (`dispatch-lib.sh:2251`) — lit
  `git -c core.quotePath=false status --porcelain -z` par substitution de
  **processus** et rend 0 seulement si **tous** les chemins sont de l'échafaudage.
- Abstention dite, par classes, jamais par chemins (`dispatch-lib.sh:2406`) :
  `dispatch-lib: resume_cleanup_scaffold_only paths=<n> classes=<…> (mika#2144)`.

## Pourquoi ça marche — et les pièges du format qui rendent le classificateur inerte

1. **Offset 3.** Chaque enregistrement `--porcelain` commence par `XY ` : sans retirer
   ce préfixe, aucun motif ne matche, et la fonction répond « travail » partout — elle
   a l'air correcte, elle ne fait rien.
2. **Renommage = deux enregistrements en `-z`**, destination (avec `XY `) puis
   **origine sans préfixe**. Appliquer l'offset 3 à l'origine la tronque
   (`.iterate/a` → `terate/a`). Les deux doivent être classés : un `git mv code/y.rs
   docs/plans/d.md` a une destination d'échafaudage mais une origine de code, et doit
   stasher.
3. **`core.quotePath=false` + `-z` est porteur** : sous le défaut, un chemin non-ASCII
   revient cité en octal (`"docs/plans/\303\251tude…"`), ne matche rien. Ce dépôt
   écrit ses plans en français.
4. **Jamais `$(…)` pour lire du `-z`** : bash supprime les NUL d'une substitution de
   commande.
5. **Fail-closed vers le stash** : `$wt` vide, statut illisible, zéro chemin lu alors
   que l'arbre est sale, enregistrement trop court → on stashe. Un stash de trop coûte
   une ligne ; un stash manquant coûte le travail d'un pilote, irréversiblement,
   puisque `clean -fd` suit.

## Prévention — les pièges de test, qui ont chacun produit un faux vert

- **Capturer stderr via `$(fn 2>&1)` perd l'effet de bord** : la substitution est un
  sous-shell, `RESUME_CLEANUP_STASH` y reste. Le contrôle négatif passait alors sur la
  mauvaise assertion. Rediriger stderr vers un **fichier** hors de l'arbre mesuré (le
  dépôt nu de la fixture), puis le lire.
- **Le test de renommage « sortant de `docs/plans/` » ne discrimine rien** : sa
  destination est déjà du code, le premier enregistrement suffit. Et un renommage
  **interne** à `docs/plans/` n'atteint jamais le Tier 3 : le `checkout HEAD --
  docs/plans/` du Tier 2 restaure l'origine et laisse la destination seule (un seul
  enregistrement). Le volet discriminant vit donc sous un `.iterate/` **tracké** — qui
  arrive au Tier 3 parce que le `rm -rf` du Tier 2 est refusé hors de
  `/.claude/worktrees/` (`_assert_removable_worktree_path`, mika#1943).
- **Un scan de source « littéral d'échafaudage en motif de `case` » accuse à tort
  `_extract_plan_path`** (`docs/plans/*)` et `*/docs/plans/*)`), qui normalise un
  callout et n'est pas une redéclaration. Le prédicat porte sur la **liste** : un bloc
  `case` contenant **deux ou plus** des quatre chemins (`_t2144_scaffold_decl_scan`,
  `test-dispatch-lib.sh:3130`).
- **Contrôle négatif vu rouge** (2026-09-29, rescue de la PR mika#2583) : avec
  `if false && _residue_is_scaffold_only …`, les tests 12l/12m/12n rougissent
  (`test-dispatch-lib.sh:2838`, `:2934`, `:3038`) et la phase 2 d'AC6 (code → stash +
  message) reste verte. Suite complète : 1169 passés, 0 échec.

## Ce que ça coûte, nommé

Le résidu d'échafaudage untracked est désormais **définitivement perdu** là où il
était stashé. Prix accepté parce que la récupérabilité était fictive : une pile de
163 entrées que personne ne lisait, effacée en bloc (reflog `refs/stash` disparu,
auteur non attribué) avant ce correctif. En échange, la ligne `resume-cleanup stashed`
redevient un signal : elle ne doit plus apparaître que pour du code. Sa disparition
totale sur 30 jours alors que des reprises ont eu lieu est une **anomalie** (filet
muet), pas un succès.

## Liens

- `workflow-issues/dispatch-lib-dirty-worktree-resume-rebase-2026-06-06.md` —
  mika#1414, la création des trois tiers ; son affirmation sur le Tier 2 ne vaut que
  pour les chemins trackés.
- `dev-loop/rescue-net-closes-without-looking-at-what-it-captured-2026-09-04.md` —
  mika#2157, le second classificateur d'échafaudage, dont la dérive prévue est fermée
  par `_is_scaffold_path`.
- mika#2141 — la source amont (pilote qui meurt sans committer son plan), hors
  périmètre ; `classes=plans` en est la mesure.
