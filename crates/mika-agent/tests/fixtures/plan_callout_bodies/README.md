# Le corpus doré du callout `Plan` — un corpus, deux lecteurs (mika#2120, mika#2194, mika#2608)

Les **six premiers** fichiers (`1680.md` … `1949.md`) sont des corps de ticket réels, un
fichier par ticket, et le **jeu de mesure** de la condition `Plan` de
`mika_agent::auto_pull::is_groomed`. Ce sont les six tickets nommés dans le corps de
mika#2120 : groomés selon la lettre de la spec, et invisibles à l'alimenteur parce que leur
callout portait le préfixe de dépôt (`mika/docs/plans/…`) là où le prédicat exigeait
`docs/plans/` collé au backtick.

Jeu distinct, et délibérément séparé de `../grooming_bodies/` : celui-là mesure l'axe du
**verdict** (mika#2158), celui-ci mesure l'axe du **chemin de plan**. Les deux axes vivaient
dans la même fonction et ont été corrigés l'un après l'autre ; mélanger leurs jeux de mesure
rendrait indécidable lequel des deux correctifs un test atteste.

## Le second axe : la parité bash ↔ Rust (mika#2194)

Avant mika#2194, les deux lecteurs du callout avaient **chacun** un corpus soigné et
**aucune entrée n'était commune** :

| | lecteur | corpus |
|---|---|---|
| Rust | `auto_pull::extract_plan_path` | les **6 corps** ci-dessus |
| Bash | `dispatch-lib::_extract_plan_path` | **17 assertions** à fixtures inline dans `test-dispatch-lib.sh` |

Personne n'avait jamais exécuté les deux sur la même entrée et comparé. Ce n'est pas une
négligence — les deux jeux sont documentés, l'un ligne par ligne — c'est une configuration
dans laquelle une divergence **ne peut pas être vue**. Et il y en avait trois (voir plus bas).

Les fixtures inline du bloc bash sont donc **rapatriées ici**, et les attendus vivent dans
`expectations.tsv`, que les **deux** lecteurs lisent :

- `crates/mika-agent/tests/plan_callout_parity.rs` — le lecteur Rust
- le bloc « mika#2194 — parité du callout Plan sur le corpus commun » de
  `skills/bundled/_shared/test-dispatch-lib.sh` — le lecteur bash

**Ne pas réécrire une assertion d'entrée dans l'un des deux lecteurs.** Deux jeux de mesure
pour un prédicat, c'est exactement la configuration qui a permis à la divergence de vivre.

Doctrine complète (la forme du corpus, l'anti-vacuité, `pre-switch`/`post-switch`) :
`docs/architecture/dispatch-lib-migration.md` § 4.

## Ne pas rafraîchir

**Ce sont des corps historiques figés. Ne les mettez pas à jour depuis GitHub.**

C'est une consigne plus forte ici que pour le jeu voisin, et pour une raison mesurée : au
2026-09-01, **quatre des six** callouts (#1680, #1694, #1699, #1934) avaient déjà été
recorrigés à la main vers la forme nue. Un jeu refetché aujourd'hui passerait donc **avant
comme après** le correctif, et n'attesterait rien du tout. La forme préfixée est exactement
ce que ces fixtures conservent.

## Les trois divergences mesurées, et laquelle est nommée dans le corpus

Elles ont été relevées en exécutant la preuve de parité de mika#2194 contre le
`dispatch-lib.sh` d'**avant** la bascule (17 cas, 17 concordances, 3 divergences). **Deux
n'étaient nommées ni dans le ticket ni dans le plan.**

| fixture | divergence | corrigée ? |
|---|---|---|
| `fences-quoted-callout.md` | le bash lit un callout **cité dans un bloc clôturé**, le Rust non | **non — tranché par mika#2609** : le rattrapage par `-f` est *partiel* (il couvre un chemin inexistant, pas un chemin existant cité), mais la population que ce trou laisse passer est **vide côté tickets ouverts** (mesure du 2026-10-01 : 9 callouts cités dans un bloc, 2 pointant un plan existant, **0 sur un ticket ouvert**), et `dispatch-lib` ne lit le corps que d'un ticket qu'il dispatche |
| `backtick-unterminated.md` | le PCRE bash lisait un callout dont le **backtick fermant** manque ; le motif Rust l'exige | **non** — le lecteur unique garde la forme stricte, et le resserrement du bash est **dit** plutôt que découvert |
| `backtick-late-close.md` | `[^`]+` n'exclut pas `\n` en Rust, donc un backtick apparaissant plus loin fait **traverser les lignes** à la capture ; `grep` travaillait ligne à ligne | **non** — le motif est conservé à l'identique (B1) ; c'est le **canal** qui borne (`mika plan-callout` refuse un chemin qui n'est pas d'une seule ligne) |

**L'assertion auto-nettoyante** porte sur la première : un cas déclaré
`divergent-fences` dont les deux politiques de fence rendraient la **même** valeur fait
**échouer** le test — la ligne ne peut pas devenir périmée en silence. C'est ce qui distingue
une exception d'un contournement.

**Ce qu'elle ne mesure PAS, et c'était écrit de travers ici jusqu'à mika#2609.** Ce paragraphe
prescrivait « le jour où la divergence est tranchée, la ligne rougit et doit être retirée ». La
divergence **est** tranchée (mika#2609, décision MPC du 2026-10-01 : le rattrapage partiel
suffit, le canal garde `FenceHandling::Keep`) et la ligne est **toujours là**, verte. Elle ne
rougit pas, parce que `parity` décrit une propriété du **corps** — `Keep(corps) ≠ Strip(corps)`
— et non la politique d'un lecteur : changer la politique du canal ne change donc rien à cette
relation. La colonne qui basculerait, le jour d'un alignement, est **`rc`** (`0` → `1`), et ce
qui l'attesterait est `mika2194_le_canal_repond_comme_le_lecteur_sur_le_corpus_dore` plus les
quatre passages `rc` de `test-dispatch-lib.sh`. La séquence complète est écrite dans le bloc de
commentaire de cette ligne, dans `expectations.tsv`.

**Et ce que la fixture ne porte pas, non plus.** `fences-quoted-callout.md` exerce la divergence
de **lecture** (`Keep ≠ Strip`), qui est exactement ce que l'assertion auto-nettoyante demande
— mais **pas** le cas dangereux de bout en bout : le plan qu'elle cite,
`docs/plans/2026-09-30-001-fix-2194-exemple-cite-plan.md`, n'existe pas dans le dépôt, donc sur
cette fixture le test `-f` *rattraperait*. Elle est correcte pour son usage, et un lecteur pressé
croira le contraire. Lui faire pointer un plan réel est délibérément **refusé** : ça changerait
son objet sans rien acheter (aucune assertion du corpus n'exerce le `-f`) et la suppression de ce
plan un jour la casserait pour une raison sans rapport.

## Les trois corps de la phase 2 (mika#2608)

`gate-non-plan-path.md`, `gate-double-space.md` et `gate-foreign-prefix-resolves.md` sont
ajoutés par la phase 2, qui fait déléguer les **deux derniers lecteurs bash** du callout
(`_committed_plan_on_branch` et le `elif` de `_set_up_worktree`). Chacun exerce **un** delta
de tolérance que cette bascule produit, et le nomme dans son propre corps :

| fixture | delta | direction | `rc` |
|---|---|---|---|
| `gate-non-plan-path.md` | le littéral `docs/plans/` n'était **pas** exigé par le `sed` | resserre — **ferme un faux positif latent** | `1` |
| `gate-double-space.md` | ` *` (zéro espace ou plus) devient un espace exactement | resserre — fail-open pour la porte | `1` |
| `gate-foreign-prefix-resolves.md` | la normalisation ne retirait que le préfixe de **ce** dépôt | **élargit** — borné par la liaison mika#2034 | `0` |

Le premier est le seul dont la fermeture achète quelque chose : un corps portant
``> - **Plan:** `README.md` `` en extrayait `README.md`, `cat-file -t` rendait `blob` (tout
dépôt a un README), la liaison mika#2034 — dont le contrat est la **réfutation** — ne
trouvait aucun `issue:` et ne réfutait pas, donc la porte **tirait** et le ticket restait
bloqué en `already_groomed` de façon permanente.

`gate-foreign-prefix-resolves.md` ressemble à `other-repo-prefix.md` et en diffère par son
**objet** : l'un atteste que le *lecteur* accepte n'importe quel segment de tête, l'autre ce
que le *site 1* en fait — il est exercé par le passage de parité qui appelle
`_committed_plan_on_branch` contre un clone fixture où le plan est réellement committé.

Les deux passages de parité **par site** que ces corps servent vivent dans le bloc
« mika#2608 » de `skills/bundled/_shared/test-dispatch-lib.sh`, chacun avec son contrôle
négatif : la mutation du lecteur du site doit faire **rougir** la parité (11 écarts pour le
site 1, 6 pour le site 2 — vus rouges à l'implémentation).

## Provenance — ce qui est mesuré, ce qui est reconstruit

Les fixtures **ajoutées par mika#2194** portent leur provenance dans leur propre corps, en
tête : `bare-callout.md`, `other-repo-prefix.md`, `double-callout.md`, `minimal-callout.md`,
les six `neg-*.md`, et les trois fixtures de divergence ci-dessus. Celles de **mika#2608**
(les trois `gate-*.md`) font de même. Aucune n'est un corps d'issue réel — toutes sont
**construites pour ce test** ou **rapatriées** du bloc bash à fixtures inline, et chacune dit
laquelle des deux.

Le tableau ci-dessous décrit les **six corps mesurés** de mika#2120.

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
