## Symptôme

**Deux** espaces après `**Plan:**`. Construit pour mika#2608 phase 2.

Le `sed` retiré de `_committed_plan_on_branch` portait ` *` — zéro espace ou
plus — donc il lisait cette forme. Le lecteur unique exige exactement un espace,
comme la ligne que le pipeline de grooming écrit
(`_write_canonical_callout`). C'est un **resserrement**, et sa direction est
fail-open pour la porte : un callout que le nouveau lecteur refuse fait que la
porte **ne tire pas**, donc que le grooming procède — la doctrine écrite de cette
fonction (« when in doubt, returns 1 and grooming runs »), appliquée à un
changement de tolérance.

La population visée est un callout malformé, que le pipeline ne produit pas ; le
corpus la mesure plutôt que de la supposer vide.

> - **Branch:** `fix/2608/double-space`
> - **Plan:**  `docs/plans/2026-09-30-001-fix-2608-deux-espaces-plan.md` (committed on branch @ `abc1234`)
> - **Grooming history:** mika-arch first-pass (READY) → mika-arch second-pass (GROOMED — session-id: 00000000-0000-0000-0000-000000000000)
