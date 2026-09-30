## Symptôme

La divergence n°3 de mika#2194, **trouvée en écrivant le corpus** et absente du
corps du ticket comme du plan. C'est la plus retorse des trois.

Le backtick de fermeture manque sur la ligne du callout, mais un backtick
apparaît **plus loin dans le corps**. La classe négative `[^`]+` du motif
n'exclut pas le retour à la ligne, et `regex` la fait donc traverser les lignes
jusqu'au backtick suivant : côté Rust la capture est un chemin **multi-ligne**.
Côté bash, `grep` travaille ligne par ligne et rendait le chemin tronqué à la
fin de sa ligne.

**Aucune tolérance n'est modifiée pour autant.** Le motif est conservé à
l'identique — c'est celui du lecteur qui *promeut* (`auto_pull`), et le
resserrer est ce que la borne B1 refuse en toutes lettres. Ce qui est borné est
le **canal** : `mika plan-callout` refuse d'émettre un chemin qui n'est pas
d'une seule ligne, sous le code `≥2` et le motif `path_not_single_line`. Le
refus vit là où la valeur ne peut pas être représentée, pas dans le prédicat.

C'est strictement plus sûr que l'état d'avant, où l'un des deux lecteurs rendait
un chemin absurde et l'autre un chemin tronqué, sans que personne l'ait mesuré.

> - **Branch:** `fix/2194/backtick-ferme-plus-loin`
> - **Plan:** `docs/plans/2026-09-30-001-fix-2194-backtick-ferme-plus-loin-plan.md
> - **Grooming history:** mika-arch first-pass (READY) → second-pass (GROOMED) — session `deadbeef`
