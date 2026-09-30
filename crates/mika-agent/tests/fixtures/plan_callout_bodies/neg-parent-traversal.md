## Symptôme

Contrôle négatif : le premier caractère du segment ne peut pas être un point,
ce qui exclut `../docs/plans/` et `./docs/plans/`. Construit pour le test de
parité de mika#2194.

C'est la borne qui empêche un callout de désigner un plan **hors** du
sous-dépôt une fois résolu contre `$WORKTREE_DIR`.

> - **Branch:** `fix/2194/neg-parent-traversal`
> - **Plan:** `../docs/plans/2026-09-30-001-fix-2194-hors-perimetre-plan.md` (committed on branch @ `abc1234`)
> - **Grooming history:** mika-arch first-pass (READY) → mika-arch second-pass (GROOMED — session-id: 00000000-0000-0000-0000-000000000000)
