# Corps d'issue figés — callout `Plan` préfixé par le dépôt (mika#2120)

Six corps de ticket, un fichier par ticket, qui sont le **jeu de mesure** de la condition
`Plan` de `mika_agent::auto_pull::is_groomed`. Ce sont les six tickets nommés dans le corps
de mika#2120 : groomés selon la lettre de la spec, et invisibles à l'alimenteur parce que
leur callout portait le préfixe de dépôt (`mika/docs/plans/…`) là où le prédicat exigeait
`docs/plans/` collé au backtick.

Jeu distinct, et délibérément séparé de `../grooming_bodies/` : celui-là mesure l'axe du
**verdict** (mika#2158), celui-ci mesure l'axe du **chemin de plan**. Les deux axes vivaient
dans la même fonction et ont été corrigés l'un après l'autre ; mélanger leurs jeux de mesure
rendrait indécidable lequel des deux correctifs un test atteste.

## Ne pas rafraîchir

**Ce sont des corps historiques figés. Ne les mettez pas à jour depuis GitHub.**

C'est une consigne plus forte ici que pour le jeu voisin, et pour une raison mesurée : au
2026-09-01, **quatre des six** callouts (#1680, #1694, #1699, #1934) avaient déjà été
recorrigés à la main vers la forme nue. Un jeu refetché aujourd'hui passerait donc **avant
comme après** le correctif, et n'attesterait rien du tout. La forme préfixée est exactement
ce que ces fixtures conservent.

## Provenance — ce qui est mesuré, ce qui est reconstruit

| ligne | provenance |
|---|---|
| forme préfixée du chemin (`<repo>/docs/plans/…`) | **mesurée.** C'est le fait que mika#2120 relève sur les six : `history=True branch=True plan_prefix=False ⇒ is_groomed=False`. C'est la seule propriété que ces fixtures doivent porter. |
| nom de fichier de plan | **relevé dans le dépôt** (`git log --all --diff-filter=A -- docs/plans/*`) pour #1680, #1694, #1699, #1934, #1949. **Remplissage annoncé** pour #1947, dont aucun plan n'existe dans l'historique local — le slug dit `remplissage`. |
| nom de branche | **relevé** (`git branch -r`) pour #1680 et #1699. **Remplissage** pour les quatre autres, dérivé du nom du plan. |
| `> - **Grooming history:**` | **reconstruite** sous la forme canonique. Le relevé de mika#2120 mesure le prédicat (`history=True`), pas le texte ; aucun verdict n'est inventé au-delà de ce que ce résultat implique. |
| prose descriptive | **omise.** Elle ne participe à aucun des trois prédicats. |

La capture littérale n'a pas pu être faite : la session qui a implémenté mika#2120 tournait
sans accès GitHub (`gh` non authentifié, aucun jeton). Même limite, même aveu que
`../grooming_bodies/README.md`.

## Le tableau attendu

| fixture | avant mika#2120 | après |
|---|---|---|
| `1680.md` | false | **true** |
| `1694.md` | false | **true** |
| `1699.md` | false | **true** |
| `1934.md` | false | **true** |
| `1947.md` | false | **true** |
| `1949.md` | false | **true** |

Six sur six passent de `false` à `true` : c'est la preuve de non-vacuité d'AC3. Le test qui
porte ce tableau est `mika2120_is_groomed_sur_les_six_corps_prefixes`, dans
`crates/mika-agent/src/auto_pull.rs`.

## Références

- `crates/mika-agent/src/auto_pull.rs` — `is_groomed`, `extract_plan_path`, `strip_fenced_blocks`
- `docs/plans/2026-09-01-004-fix-2120-is-groomed-repo-prefix-plan.md` — le plan
- `docs/solutions/architecture-patterns/guard-parser-must-be-as-permissive-as-downstream-consumer-2026-08-29.md` — la classe
