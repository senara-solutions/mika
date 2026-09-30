## Symptôme

La divergence n°2 de mika#2194, **trouvée en écrivant le lecteur unique** et
absente du corps du ticket comme du plan.

Le backtick de fermeture manque. Le motif Rust l'exige (`` `(…)` ``) ; le PCRE
du bash s'arrêtait à `[^`]+` **sans borne droite**, donc lisait le chemin
jusqu'à la fin de la ligne. Les deux lecteurs répondaient donc différemment sur
cette forme, et personne ne l'avait mesuré.

Le lecteur unique garde la forme **stricte** : desserrer aurait élargi la
surface de faux positif de `plan_ownership`, qui décide d'un **abandon** de
ticket (mika#2020), ce que la borne B1 refuse dans les deux sens. Le
resserrement est donc dit plutôt que découvert, et il porte sur un callout
malformé que le pipeline de grooming ne produit pas.

> - **Branch:** `fix/2194/backtick-non-termine`
> - **Plan:** `docs/plans/2026-09-30-001-fix-2194-backtick-non-termine-plan.md
> - **Grooming history:** mika-arch first-pass (READY) → mika-arch second-pass (GROOMED — session-id: 00000000-0000-0000-0000-000000000000)
