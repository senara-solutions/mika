## Symptôme

Contrôle négatif : **un seul** segment de tête est autorisé, donc
`a/b/docs/plans/` échoue. Construit pour le test de parité de mika#2194.

La normalisation ne retire qu'un segment ; accepter deux rendrait un chemin que
`"$WORKTREE_DIR/$PLAN_PATH"` ne résoudrait toujours pas.

> - **Branch:** `fix/2194/neg-two-segments`
> - **Plan:** `a/b/docs/plans/2026-09-30-001-fix-2194-deux-segments-plan.md` (committed on branch @ `abc1234`)
> - **Grooming history:** mika-arch first-pass (READY) → mika-arch second-pass (GROOMED — session-id: 00000000-0000-0000-0000-000000000000)
