## Symptôme

Préfixe de dépôt **étranger** sur un ticket de `mika`. Construit pour mika#2608
phase 2, et il exerce le seul **élargissement** de la bascule (D2b) — là où
`other-repo-prefix.md` exerce la tolérance du lecteur, celui-ci exerce ce que le
**site 1** en fait.

Avant la bascule, `_committed_plan_on_branch` essayait deux candidats :
`$plan_path` brut, puis `${plan_path#"${repo}/"}` — qui ne retire que le préfixe
de **ce** dépôt. Sur `mika-cloud/docs/plans/x.md` avec `repo=mika`, ni l'un ni
l'autre ne résolvait, donc la porte ne tirait pas. Après la bascule,
`normalized` vaut `docs/plans/x.md` et **peut** résoudre.

**La borne est la liaison mika#2034**, pas la chance : l'en-tête d'un plan de
`mika-cloud` nommerait `mika-cloud#220`, donc `_plan_header_refutes_issue`
réfute et la porte ne tire pas. Population pratique quasi vide — il faudrait
qu'un fichier de ce nom existe **aussi** dans l'autre dépôt — et doublement
gardée. Nommée plutôt que masquée, et couverte par ce cas.

> - **Branch:** `fix/2608/foreign-prefix`
> - **Plan:** `mika-skills/docs/plans/2026-09-30-002-fix-2608-prefixe-etranger-plan.md` (committed on branch @ `abc1234`)
> - **Grooming history:** mika-arch first-pass (READY) → mika-arch second-pass (GROOMED — session-id: 00000000-0000-0000-0000-000000000000)
