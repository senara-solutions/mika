Contrôle négatif : le mot « Plan: » en prose, sans le littéral `docs/plans/`.

Rapatrié du bloc bash à fixtures inline vers le corpus commun par mika#2194 R4
(« Prose Plan: without docs/plans/ prefix does not match »). C'est le littéral
`docs/plans/` qui évite les faux positifs sur une prose contenant « Plan: » —
distinct de l'ancrage, que `neg-inline-prose.md` mesure.

The Plan: is to refactor the module, and no callout says otherwise.
