# Fixtures de mika#2996 — la relance UNPARSED doit rester ancrée

Lues par `tests/eval/grounding_regressions/review_anchor_unparsed_retry_2996.rs`.

| fichier | provenance | rôle |
|---|---|---|
| `plan-mika2995-61a3fb572.md` | `docs/plans/2026-10-31-006-fix-2995-…-plan.md` au commit `61a3fb572` de la branche `fix/2995/consent-mika-dev-peut-se-poser-lui-m-me`, copié à l'octet | le plan réellement en revue le 2026-10-31. Il est identique au brief du tour d'origine (message 123047 de la session mika-arch `460a81ba`), vérifié par `diff` le 2026-10-31 |
| `response-15ea28bd-msg123050.md` | réponse de mika-arch à la relance de mika#1823, trace `15ea28bd`, message 123050, lue en lecture seule dans `~/.mika/data/mika.db` | la fixture **refusée** (AC3) : une revue d'un **autre** plan (celui de mika#2752), close par `Disposition: READY`. Ses trois ancres ne citent rien du plan ci-dessus |

Aucun email ni UUID de tenant dans l'une ou l'autre, vérifié par `grep` à la
copie. Les identifiants de trace et de session cités ici sont permis.

Ne pas « nettoyer » ces fichiers. Ils valent par leur fidélité à l'octet : la
garde compare des citations verbatim, et une fixture réécrite à la main
mesurerait une autre chose que l'incident.
