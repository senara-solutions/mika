## Symptôme

Contrôle négatif d'**ancrage** : la prose qui *parle* du callout n'en est pas
un. Construit pour le test de parité de mika#2194.

L'ancrage `(?m)^` est ce qui distingue le callout de la prose. Sans lui, ce
corps-ci serait lu comme groomé alors qu'il ne fait que citer la ligne au fil du
texte.

Voir la ligne > - **Plan:** `docs/plans/2026-09-30-001-une-mention-plan.md` du
corps de l'autre ticket, qui n'est pas la nôtre.

> - **Branch:** `fix/2194/neg-inline-prose`
> - **Grooming history:** mika-arch first-pass (READY) → mika-arch second-pass (GROOMED — session-id: 00000000-0000-0000-0000-000000000000)
