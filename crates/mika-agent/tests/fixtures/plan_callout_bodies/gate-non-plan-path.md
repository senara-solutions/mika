## Symptôme

Le callout nomme un fichier qui **n'est pas un plan**. Construit pour mika#2608
phase 2, et il porte le seul delta de tolérance dont la direction achète quelque
chose au lieu d'en coûter.

Avant la bascule, le `sed` de `_committed_plan_on_branch` n'exigeait pas le
littéral `docs/plans/` : il extrayait `README.md`, `cat-file -t` rendait `blob`
(tout dépôt a un README), et la liaison mika#2034 — dont le contrat est la
**réfutation**, jamais la confirmation — ne trouvait aucun `issue:` dans un
README et ne réfutait donc pas. La porte **tirait**, et le ticket restait bloqué
en `already_groomed` de façon permanente : la classe exacte que mika#2034 a
ouverte pour fermer (#1887, #2026, tous deux « stranded »).

Le lecteur unique exige `docs/plans/`, donc ce faux positif latent est fermé.
C'est un effet **collatéral** de la migration, pas son objectif — il est nommé
plutôt que découvert.

> - **Branch:** `fix/2608/non-plan-path`
> - **Plan:** `README.md` (committed on branch @ `abc1234`)
> - **Grooming history:** mika-arch first-pass (READY) → mika-arch second-pass (GROOMED — session-id: 00000000-0000-0000-0000-000000000000)
