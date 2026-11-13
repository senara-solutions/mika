# mika#2976 — vol 1 : une branche de vol mergé n'est plus reprise

Ce plan livre **AC1, AC2, AC3** et l'instrument d'**AC4** : le dispatch mesure, avant de
reprendre une branche, si elle porte un travail en cours ou le résidu d'un vol atterri.
Sur résidu il réinitialise la branche sur `origin/main` ; sur travail non fusionné il
s'arrête sans rien écraser ; sur vol en cours il reprend, exactement comme aujourd'hui.

**AC5 n'est pas dans ce vol.** Il porte sur un autre fichier, une autre couche et un
autre producteur (le tour de callback de mika-dev, plus la terminalité de la parente), et
il fait sortir le total de la borne par vol. Voir § 10.

Tracked in: senara-solutions/mika#2976 (vol 2 — AC5)

---

## 1. Ce que le dépôt dit, et qui corrige le ticket sur deux points

### 1.1 La reprise saine du second groom n'est pas un hasard — c'est le faucheur de worktrees

Le ticket demande : *« Le plan dit si la reprise saine du second groom est un comportement
voulu, sur lequel bâtir l'AC1, ou un hasard. »*

**Ni l'un ni l'autre : c'est un mécanisme réel, mais asynchrone.**
`crates/mika-agent/src/worktree_reaper.rs` (mika#2420) retire les worktrees de PR
terminale, et supprime **aussi la branche locale** quand le worktree l'avait checked out
— `should_delete_local_branch(resolution)` rend vrai pour `RESOLUTION_BRANCH`
(l. 960-962). Sa row récurrente est `0 */10 * * * *` (`task_engine/engine.rs` l. 6885) et
sa grâce par défaut est de **900 s après `closedAt`** (`ReapConfig::default().grace_secs`,
épinglé l. 6273).

Le reflog du 2026-10-30 à 14:55:19Z — « branch: Created from origin/main » — est donc la
signature d'un tick de faucheur passé entre le groom de 14:38Z et celui de 14:55Z, soit
2 h 32 après le merge de mika#2977 à 12:06:09Z.

**Conséquence sur l'AC1 : on ne peut pas bâtir dessus.** Le faucheur est périodique, le
dispatch de groom est événementiel, et le vainqueur de la course décide de la base. Entre
le merge et le premier tick utile il y a au mieux 900 s de grâce plus jusqu'à 600 s de
latence de tick, et au pire — mesuré — deux heures et demie. Tous les cas du ticket
tombent dans cette fenêtre. Le correctif doit donc **poser la même question au moment du
dispatch**, synchroniquement, sur la branche qu'il est sur le point de reprendre.

### 1.2 Le nettoyage après merge a déjà un propriétaire — rien à construire

Le ticket demande de trancher *« qui retire le worktree d'un vol mergé, et quand »*.
**Réponse : `worktree_reaper` le fait déjà**, avec ses sept termes tous fail-safe vers
*conserver*, son mode `observe`, et son STOP à chaud. Ce plan n'en livre pas un second et
n'y touche pas : un filet existant qui couvre tout le domaine n'a pas besoin d'un chemin
parallèle (doctrine mika#2334, citée par le module lui-même).

Ce que ce plan livre est l'autre moitié, celle que le faucheur ne peut pas tenir : **le
dispatch ne doit pas dépendre de la latence du faucheur.** Les deux cohabitent sans se
recouvrir — le faucheur libère du disque sur une population *scannée*, la porte ci-dessous
décide d'une base sur une branche *nommée par le dispatch en cours*.

---

## 2. Les trois portes où le résidu entre aujourd'hui

`_set_up_worktree` (`skills/bundled/_shared/dispatch-lib.sh` l. 4948) dérive `BRANCH` puis
`WORKTREE_DIR`, et choisit une base par trois chemins. **Aucun des trois ne teste si la
branche est périmée.**

| # | condition | ligne | ce qu'il prend | cas du ticket |
|---|---|---|---|---|
| A | `[ -d "$WORKTREE_DIR" ]` et c'est un git dir | 5274-5276 | `git checkout "$BRANCH"` — la branche locale telle quelle | **#2968 cas 2 et 3** : worktree et branche locale survivants au merge |
| B | worktree absent, `_remote_branch_exists` vrai | 5314 | `worktree add -b … origin/$BRANCH` | **#1727** : branche sur origin, 584 commits derrière |
| C | worktree absent, remote absent, **branche locale présente** | 5322 | `worktree add … "$BRANCH"` — repli du `-b` qui échoue en « branch already exists » | le même que A, worktree retiré mais branche locale restée |

Puis la porte de rebase (l. 5366-5420) : `BEHIND = rev-list --count HEAD..origin/main`, et
sur `BEHIND > 0` un `git rebase origin/main` qui rejoue des commits dont le contenu est
déjà dans `main` par squash — d'où le conflit, `STATUS=REBASE_CONFLICT`, `exit 1`.

Le chemin C est le plus traître : c'est un **repli implicite**. Le `-b` sur `origin/main`
est écrit, il échoue, et le second bras reprend silencieusement la branche périmée. Rien
dans le journal ne distingue ce cas d'une reprise voulue.

---

## 3. Le critère de reprise — formulé en négatif, et pourquoi

Une seule question, posée une seule fois, avant le bloc reprise-ou-création :
*cette branche porte-t-elle un travail en cours ?*

### 3.1 P1 — le verdict des PR

```
gh pr list --head "$BRANCH" --repo "senara-solutions/$REPO" --state all \
   --json number,state --limit 20
```

| lecture | verdict |
|---|---|
| **au moins une PR `OPEN`** | `live` — reprise nominale d'un vol en cours (AC2c) |
| au moins une PR, **aucune ouverte** | candidat `spent` ⇒ P2 |
| **aucune PR** pour ce head | `live` — population nominale « groomé, pas encore implémenté » |
| `gh` en échec, sortie non parsable, `jq` muet | `undetermined` |

**Le terme décisif est en négatif, et ce n'est pas un détail de style.** « Il existe une PR
mergée » serait vrai quand deux PR partagent le même `headRefName` — une fermée, une
rouverte — et déclarerait périmée une branche dont une PR est ouverte. C'est le même
raisonnement que le terme T4 du faucheur (`worktree_reaper.rs` l. 1035-1042), formulé en
négatif pour exactement cette raison, et la formulation se transporte parce que la
population est la même.

**« Aucune PR » vaut `live`, délibérément.** C'est la population d'une branche de groom
poussée avec son plan et pas encore implémentée — `pr_unknown` chez le faucheur, qui la
conserve aussi. La traiter comme périmée détruirait le plan d'un vol qui attend son
dispatch d'implémentation : c'est le faux positif le plus coûteux de tout ce ticket.

### 3.2 P2 — la containment, qui est l'AC3

P1 ne suffit pas : une PR mergée n'implique pas que **tout** ce que porte la branche est
dans `main`. Un commit poussé après l'approbation, ou le résidu d'un `wip()` de secours,
y vit encore.

L'idiome est celui que la maison a déjà, `_check_duplicate_commits` (l. 8537-8540) :

```
git -C "$SUB_REPO_DIR" log --cherry-mark --right-only \
    --format="%m %H %s" "origin/main...$tip"
```

`--cherry-mark` compare par **patch-id**, donc un commit absorbé par un squash sort avec
la marque `=` et un commit réellement neuf avec `>`. C'est précisément ce que
`rev-list --count` ne sait pas faire, et c'est l'origine du défaut : la porte de rebase
compte des commits là où il faut compter des patchs.

`$tip` est pris sur **les deux refs quand elles diffèrent** —
`refs/heads/$BRANCH` et `refs/remotes/origin/$BRANCH` — et l'union des `>` est retenue.
Le cas 3 du ticket est exactement celui-là : la ref locale `origin/…` pointait sur
`6069e503` que le distant ne portait plus. Une seule ref lue aurait manqué la moitié de la
population.

| `>` | verdict |
|---|---|
| 0 | `spent` — rien à perdre, la branche se réinitialise |
| > 0 | `unmerged_work` — **halte** (AC3) |
| `git` muet sur l'une des deux lectures | `undetermined` |

### 3.3 `undetermined` retombe sur le comportement d'aujourd'hui, et le dit

Le faucheur fail-safe vers *conserver* parce qu'une suppression fautive ne se répare pas.
**Ici la direction sûre est différente, et l'inversion est raisonnée** : la porte
ci-dessous *réinitialise* une branche. Une mesure manquante qui conduirait à réinitialiser
détruirait du travail ; une mesure manquante qui conduit à reprendre reproduit le
`REBASE_CONFLICT` d'aujourd'hui, c'est-à-dire un échec bruyant, borné, et déjà outillé.

Donc : `undetermined` ⇒ **reprise**, comportement inchangé, et une ligne de journal qui
nomme la non-mesure. Un repli silencieux se lirait comme une mesure (motif mika#2304).

Le régime attendu de cette population est **vide**. Elle est comptée pour que
« la porte ne mord sur rien » et « la porte ne mesure rien » n'impriment pas les mêmes
octets, et un contrôle négatif de test l'épingle (§ 6, T5).

---

## 4. Que faire d'une branche périmée — réinitialiser, pas renommer

Le ticket laisse le choix entre *« une branche neuve depuis `main` sous un autre nom »* et
*« réinitialiser la branche sur `origin/main` »*.

**Tranché : réinitialiser.** Trois raisons, par poids décroissant.

1. **Le nom de branche est dérivé, pas choisi.** `scripts/derive-branch-name` est une
   fonction pure de (titre, numéro, labels, callout). Un second nom exigerait un suffixe
   ou un compteur, donc un **second lecteur partout où le nom est re-dérivé** —
   `_push_branch`, `_committed_plan_on_branch`, `_write_canonical_callout`, le webhook QA
   — et casserait l'invariant inter-fichiers `worktree_path_slug == sanitize(branch_ref)`
   (`docs/solutions/best-practices/dispatcher-cross-file-invariant-2026-04-28.md`). Un nom
   dérivé par ticket est l'invariant sur lequel repose toute la chaîne ; un suffixe est un
   axe neuf.
2. **La réinitialisation reproduit l'état déjà observé sain.** Le reflog du second groom de
   14:55Z est la preuve que cet état marche de bout en bout jusqu'à `PLAN_GROOMED` et au
   premier push. On rend déterministe ce qui était une course.
3. **Le contenu non commité a déjà sa discipline dans ce fichier.** Voir § 5.

### 4.1 Le worktree et son contenu non commité

P2 a prouvé qu'il n'y a **rien à perdre en commits**. Le non-commité est une autre
question, et la réponse existe déjà vingt lignes plus haut : le bloc de nettoyage du relic
non canonique (l. 5240-5272) fait exactement le bon geste — `git status --porcelain`, et
sur sale un `git stash push --include-untracked -m "<tag>"` suivi de la capture du SHA et
d'un indice de récupération qui nomme le worktree **canonique**, jamais le checkout
principal (mika#2449).

Ce bloc est **extrait** en `_stash_and_remove_worktree`, et le site du relic l'appelle. Ce
n'est pas de la surface neuve : c'est le même geste, à un seul endroit, appelé deux fois.
Les deux invariants qu'il porte sont conservés mot pour mot : le passage obligatoire par
`_assert_removable_worktree_path` (mika#1943 — tout site de suppression, sans exception) et
l'indice de récupération nommant `$WORKTREE_DIR`.

### 4.2 La branche distante n'est pas supprimée

Dans le cas #1727 la branche périmée vit **sur origin**. On ne la supprime pas : un
`push origin --delete` est un acte sortant et irréversible, et il n'est pas nécessaire. Le
dispatch n'a besoin que d'une base locale saine ; `_push_branch` écrasera ensuite la ref
distante par son bras `diverged` existant (`--force-with-lease`, dont le bail est calculé
sur un vrai `ls-remote` depuis mika#2626).

**Conséquence nommée plutôt que cachée :** la ref distante d'un vol mergé sera réécrite par
le vol suivant. C'est acceptable ici et seulement ici — la PR est mergée, son contenu est
dans `main`, et P2 a prouvé la containment avant que quoi que ce soit ne bouge. Hors de ces
trois conditions la porte ne réinitialise pas.

---

## 5. L'implémentation — trois fonctions, un drapeau, une ligne de journal

### 5.1 `_branch_reuse_verdict <repo_dir> <repo> <branch>`

Écrit un seul jeton sur stdout — `live` | `spent` | `unmerged_work` | `undetermined` — et
pose trois globales pour la ligne de journal : `_BRANCH_REUSE_PR` (numéro et état, ou
`none`), `_BRANCH_REUSE_UNMERGED` (le compte de `>`), `_BRANCH_REUSE_TIPS` (les refs lues).
Globales plutôt qu'une sortie structurée : c'est la forme qu'a déjà `_pilot_max_turns` avec
`_PILOT_MAX_TURNS`, et le parsing d'une sortie composite serait un lecteur de plus.

Jeton sur stdout plutôt que code de retour à quatre valeurs : un `rc` à quatre valeurs
force chaque appelant à réécrire la table, et `_remote_branch_exists` montre déjà le coût
d'une troisième valeur mal lue.

### 5.2 `_stash_and_remove_worktree <repo_dir> <worktree_dir> <branch> <site>`

L'extraction du § 4.1. `<site>` est le quatrième argument parce que
`_assert_removable_worktree_path` prend un nom de site (`set_up_worktree_relic`,
`set_up_worktree_spent`) et que ces noms sont la trace d'audit d'un refus de suppression.

### 5.3 `_reset_spent_branch <repo_dir> <worktree_dir> <branch>`

1. `_stash_and_remove_worktree … set_up_worktree_spent` — et sur refus de la garde
   mika#1943, **on ne réinitialise pas** : on rend non-zéro, l'appelant retombe sur
   `live`, et le `REBASE_CONFLICT` d'aujourd'hui reste le pire cas. L'asymétrie est la
   même que celle du § 3.3.
2. `git -C "$repo_dir" branch -f "$branch" origin/main`.
   `-f` plutôt qu'une suppression suivie d'une recréation : c'est littéralement l'option 2
   du ticket, le reflog porte alors `branch: Reset to origin/main` — une provenance qui se
   lit — et git refuse de son propre chef un `-f` sur une branche encore checked out, ce
   qui est un filet gratuit sur l'ordre des deux étapes.

### 5.4 Le site d'appel

Inséré dans `_set_up_worktree` **après** le `git fetch origin main` (l. 5195) et le calcul
de `WORKTREE_DIR`, **avant** le nettoyage du relic non canonique. Les deux bornes portent
leur conséquence :

- après le fetch, sans quoi `origin/main` est périmé et P2 compterait comme non fusionné
  ce qui vient d'atterrir ;
- avant le bloc reprise-ou-création, qui est l'ensemble des trois portes du § 2. Le placer
  dedans obligerait à le dupliquer trois fois.

Conditionné à `[ "$SKILL" = "dev-groom" ]` ? **Non, inconditionnel.** Le défaut est dans la
dérivation de base, et `dev-pilot` dérive la même branche par la même fonction ; un vol 2
d'implémentation sur un ticket à PR mergée prend exactement le même mur. Restreindre au
groom bornerait la correction à la moitié de sa population pour aucun gain.

Le `BRANCH_SPENT=1` posé après une réinitialisation réussie sert deux sites :

- la condition `if _remote_branch_exists …` (l. 5311) devient
  `if [ "$BRANCH_SPENT" != "1" ] && _remote_branch_exists …` — sans quoi le cas #1727
  reprendrait `origin/$BRANCH`, que la réinitialisation locale ne touche pas ;
- un **troisième bras explicite** sous `BRANCH_SPENT=1` :
  `worktree add "$WORKTREE_DIR" "$BRANCH"` sur la branche qui vient d'être réinitialisée.
  Explicite, et non par le repli du `-b` en échec : ce repli implicite **est** la porte C du
  § 2, et s'y adosser referait le défaut par l'autre bout.

Le worktree ayant été retiré en 5.3, le test `[ -d "$WORKTREE_DIR" ]` de la porte A est
faux et le chemin de création est pris sans toucher à la structure du `if`.

### 5.5 La halte de l'AC3

Sur `unmerged_work`, rien n'est retiré, rien n'est réinitialisé, et le dispatch s'arrête,
dans la forme exacte de `REBASE_CONFLICT` (`RESULT=…` puis `exit 1`, que le trap EXIT
délivre en callback) :

```
STATUS=BRANCH_HAS_UNMERGED_WORK
Branch <b> carries <n> commit(s) absent from origin/main by patch-id, while its PRs are all terminal (<#n> <state>, …).
Unmerged commits: <sha> <subject> …
Nothing was reset, nothing was removed, nothing was stashed.
Resolve manually before re-dispatching <repo>#<n>.
```

Un statut neuf plutôt que `REBASE_CONFLICT` : les deux ont des remèdes opposés — l'un
demande de résoudre un conflit, l'autre d'arbitrer le sort de commits non fusionnés — et
deux populations sous un seul nom ne sont pas soustractibles (mika#2205).

### 5.6 La ligne de journal, qui est l'instrument de l'AC4

Une clé, tous les aboutissements dessous, sur le modèle littéral de `rebase_gate`
(l. 5363) et pour la raison que son commentaire énonce : la question que la porte ne peut
pas trancher de son côté — *la boucle reprend-elle encore quelque chose, ou la porte
est-elle devenue si stricte que plus rien ne reprend ?* — exige un nombre.

```
[dispatch-lib] branch_reuse_gate: branch=<b> verdict=<live|spent|unmerged_work|undetermined> \
  pr=<#n <state>|none> unmerged=<n> action=<reuse|reset|halt|reuse_unmeasured>
```

`verdict` et `action` sont deux champs et non un : ils coïncident aujourd'hui sauf sur
`undetermined` (verdict non mesuré, action de reprise) et sur un refus de la garde
mika#1943 (verdict `spent`, action `reuse`). Les fusionner rendrait ces deux cas
indistinguables de la reprise nominale, qui est précisément la population dont l'AC4
veut le compte.

---

## 6. Les tests — `test-dispatch-lib.sh`, déjà câblée

La suite tourne en CI par `make test-dispatch-lib` (`Makefile` l. 199, `ci.yml` l. 85).
**Aucune entrée dans `scripts/suite-wiring.tsv`** : la comparaison du lint est
bidirectionnelle, donc une entrée pour une suite que CI atteint ferait rougir
`check-suite-wiring`.

Le harnais existe : `_fixture_setup` (l. 2785) monte un bare `file://` plus un clone, avec
`_assert_fixture_is_local` en garde, et `gh` se stube par redéfinition de fonction
(précédent l. 9698). Les six cas étendent ce harnais avec un worktree réel sur le clone.

| # | scène | attendu |
|---|---|---|
| T1 | **AC2a** — branche **locale** seule (remote supprimée), stub `gh` rendant une PR `MERGED`, tête contenue dans `main` par squash | `verdict=spent action=reset` ; la branche pointe sur `origin/main` ; le worktree a disparu |
| T2 | **AC2b** — branche **sur origin**, PR `MERGED`, 584 commits derrière (forme réduite) | `verdict=spent action=reset` ; le bras `_remote_branch_exists` **n'est pas** pris ; la base est `origin/main` |
| T3 | **AC2c — contrôle négatif** — PR `OPEN` sur ce head | `verdict=live action=reuse` ; la branche **n'a pas bougé** (SHA identique avant/après) ; le worktree est intact |
| T4 | **AC2c bis — second contrôle négatif** — **aucune** PR pour ce head (branche de groom non implémentée) | `verdict=live action=reuse` ; branche et worktree intacts |
| T5 | **AC3** — PR `MERGED`, mais un commit de la branche porte un patch absent de `main` | `verdict=unmerged_work action=halt` ; `RESULT` porte `STATUS=BRANCH_HAS_UNMERGED_WORK` et le sha ; la branche **n'a pas bougé** ; aucune entrée de stash créée |
| T6 | **non-mesure** — stub `gh() { return 1; }` sur une branche par ailleurs périmée | `verdict=undetermined action=reuse_unmeasured` ; la branche **n'a pas bougé** |

T3 et T4 sont deux contrôles négatifs distincts et aucun n'est redondant : une
implémentation qui oublierait le bras « aucune PR » passerait T3 et casserait T4, et c'est
exactement le faux positif du § 3.1 — le plus coûteux du ticket. T6 sépare « la porte ne
mord sur rien » de « la porte ne mesure rien ».

Chaque test assertionne le **SHA de la branche avant et après**, et non seulement le jeton :
un verdict juste suivi d'un effet de bord faux passerait une assertion sur le seul jeton.

---

## 7. Ordre de commit, et le plancher qui protège d'une mort au plafond

| # | contenu | mergeable seul ? |
|---|---|---|
| 1 | `_stash_and_remove_worktree` extraite ; le site du relic l'appelle | oui — pur refactor, comportement identique, la suite existante le prouve |
| 2 | `_branch_reuse_verdict` + sa ligne de journal, **appelée en mode observation** (journalise, ne décide rien) | oui — observabilité seule, aucun effet |
| 3 | T1 à T6 contre la fonction du commit 2, armés sur le jeton | oui |
| 4 | `_reset_spent_branch`, le `BRANCH_SPENT`, le troisième bras, la halte AC3 ; T1-T6 étendus aux assertions d'effet | oui |

**Le plancher est le commit 2.** Un pilote mort après lui a livré l'instrument de l'AC4 —
la ligne `branch_reuse_gate` — qui chiffre la population sans rien changer au
comportement. C'est la moitié du ticket qui a le plus de valeur par ligne, et elle est
atteignable en un tiers des tours.

L'ordre 1 → 2 avant 4 n'est pas cosmétique : le commit 4 appelle
`_stash_and_remove_worktree`, et la livrer d'abord sous son seul usage existant la fait
valider par une suite qui tourne déjà.

---

## 8. La sonde de l'AC4 — ce n'est pas du code

AC4 demande : *sur les 10 grooms suivants d'un vol 2 ou plus, le compte de
`REBASE_CONFLICT` dus à une branche de vol mergé, cible 0.*

L'instrument est la ligne du § 5.6, lue dans les journaux de dispatch :

```
grep -h 'branch_reuse_gate:' /var/log/claude-pilot/*.log | grep -c 'action=reset'
grep -h 'branch_reuse_gate:' /var/log/claude-pilot/*.log | grep -c 'action=halt'
grep -h 'branch_reuse_gate:' /var/log/claude-pilot/*.log | grep -c 'action=reuse_unmeasured'
```

Trois nombres, et c'est la conjonction qui vaut relevé :

- `action=reset` **> 0** prouve que la porte mord sur une population réelle. À zéro, le
  relevé est muet et non vert — la cible 0 de l'AC4 serait alors indistinguable d'une porte
  qui ne s'est jamais armée (précédent mika#2272 : *zéro était l'absence de mesure, pas la
  présence de caution*).
- `action=reuse_unmeasured` est attendu **à 0**. Non nul, c'est la non-mesure du § 3.3 qui
  mord, et le remède est côté `gh`, pas côté porte.
- le compte de `REBASE_CONFLICT` sur ces mêmes grooms, cible 0, est écrit en commentaire du
  ticket avec les trois nombres ci-dessus et la fenêtre de relevé.

Le relevé est une obligation **post-déploiement**, et la ligne `branch_reuse_gate` ne
l'atteint qu'après un `make deploy` : `~/.mika/skills/` est une projection du binaire, pas
du checkout (root `CLAUDE.md` § *Deploying a bundled-skill change*).

---

## 9. Fire-Disposition

Ce plan livre des détecteurs : les six cas du § 6, dont le chemin de succès est « aucun
résidu trouvé », et la halte du § 5.5.

**Option retenue : (c) halte-et-remontée**, pour la seule population que la porte ne peut
pas disposer sans risque de perte.

- **Détail d'implémentation.** La population `unmerged_work` — PR terminale, commits
  absents de `main` par patch-id — n'est ni réinitialisée, ni reprise, ni nettoyée. Le
  dispatch écrit `STATUS=BRANCH_HAS_UNMERGED_WORK` (§ 5.5), nomme les SHA et leurs sujets,
  déclare explicitement que rien n'a été retiré ni stashé, et sort en 1 ; le trap EXIT
  délivre le callback à l'opérateur. C'est le **sens même de l'AC3** : *« le dispatch le dit
  et s'arrête, au lieu de les écraser »*.
- **T5 est l'assertion qui l'épingle**, et elle porte sur l'effet autant que sur le
  message : le SHA de la branche est identique avant et après, et aucune entrée de stash
  n'a été créée.
- **(a) exception nommée en allowlist est N/A, et il faut le dire** pour que l'absence ne
  se lise pas comme un oubli : il n'y a **aucun recensement de violations préexistantes** à
  allowlister. La population de cette porte est calculée par dispatch, sur la branche que
  ce dispatch vient de dériver — elle n'est jamais scannée sur le dépôt. Une allowlist
  n'aurait ni clé ni contenu.
- **(b) livrer désarmé est refusé.** Le commit 2 du § 7 est un palier d'observation, pas un
  désarmement : il atterrit armé sur son propre périmètre — il journalise — et le commit 4
  arme la décision dans la même PR. Livrer la décision derrière un `#[ignore]` ne fermerait
  rien, et le défaut est p1 avec trois occurrences mesurées en huit jours.
- **L'assertion auto-nettoyante** du dispositif est la population
  `action=reuse_unmeasured` du § 8 : son régime attendu est vide, elle est comptée
  séparément, et un non-zéro rougit le relevé plutôt que de passer pour une reprise
  nominale.

---

## 10. Pourquoi l'AC5 est un autre vol

AC5 — *le callback de `REBASE_CONFLICT` ne lance pas de `git rebase`, et la parente devient
terminale* — ne partage **ni fichier, ni couche, ni producteur** avec AC1-AC3 :

- la moitié « ne rebase pas » est un discriminateur à ajouter dans
  `skills/bundled/self-dev-callback/system_prompt.md`, dans la forme des cinq
  discriminateurs qui y vivent déjà (`CANCELLED_BY_*`, containment-refusal, launcher-dead,
  groom-escalation) ;
- la moitié « la parente devient terminale » est en Rust, autour de
  `reap_orphaned_parent_tasks` (`task_engine/engine.rs` l. 4493) et de
  `find_orphaned_parent_tasks`, dont la grâce s'ancre sur `MAX(child.updated_at)` depuis
  mika#2762 — et la parente zombie 19c11b09 du ticket tombe dans cette population. Dire
  *pourquoi* elle n'a pas été fauchée demande une mesure que ce vol n'a pas faite, et un
  verdict non mesuré ne se code pas.

Le découpage est en **vols** et non en phases : ce vol est mergeable seul, il ferme les
trois AC qui produisent les trois `REBASE_CONFLICT` mesurés, et il repasse sous la borne.
Le vol 2 regroome le même ticket, à l'idiome de la maison (`mika#2968, vol 2b`).

Tracked in: senara-solutions/mika#2976 (vol 2 — AC5)

---

## 11. Discipline de l'implémenteur — deux pièges nommés

1. **Ne jamais taper `git branch -D` au shell.** La permission-policy du pilote refuse
   cette forme et le refus est **terminal** : la session meurt sur le coup. Le § 5.3 n'en a
   pas besoin — il écrit `git branch -f` — et les fixtures de test qui créent et détruisent
   des branches ne sont exécutées qu'à travers `make test-dispatch-lib`, en sous-processus,
   jamais frappées à la main.
2. **`make deploy` est hors périmètre de la PR.** `~/.mika/skills/` est une projection du
   binaire : la porte n'atteint aucun agent avant un rebuild. La PR ne le fait pas ; le
   § 8 le nomme comme préalable au relevé.

---

## 12. Definition of Done

- `_branch_reuse_verdict`, `_stash_and_remove_worktree`, `_reset_spent_branch` dans
  `skills/bundled/_shared/dispatch-lib.sh`, appelées depuis `_set_up_worktree` au site du
  § 5.4.
- Le site du relic (l. 5240-5272) appelle l'extraction ; ses deux invariants
  (`_assert_removable_worktree_path`, indice de récupération nommant `$WORKTREE_DIR`) sont
  conservés, et la suite existante le prouve.
- `BRANCH_SPENT` consulté par la condition `_remote_branch_exists` et par un troisième bras
  explicite de `worktree add`.
- `STATUS=BRANCH_HAS_UNMERGED_WORK` émis sur la population AC3, sans retrait, sans
  réinitialisation, sans stash.
- La ligne `branch_reuse_gate` émise sur **tous** les aboutissements, avec `verdict` et
  `action` en deux champs.
- T1 à T6 dans `skills/bundled/_shared/test-dispatch-lib.sh`, chacun assertionnant le SHA
  de la branche avant et après.
- `make test-dispatch-lib` vert, `cargo test` vert, `cargo clippy` sans avertissement,
  `cargo fmt --check` propre, `make verify-bundled-skills` vert,
  `scripts/verify-pipeline.sh origin/main` vert.
- Aucune ligne ajoutée à `scripts/suite-wiring.tsv`.
- Le corps de PR porte `Code review: delegated to mika-qa (mika#2694)` et la ligne
  `Tracked in: senara-solutions/mika#2976 (vol 2 — AC5)`.

---

## 13. Acceptance criteria

- **AC1.** Le groom d'un vol suivant, sur un ticket dont la PR précédente est mergée, ne
  reprend pas l'ancienne branche, qu'elle soit sur origin ou seulement en local. Il produit
  un plan sur une branche qui part de `main`.
- **AC2.** Test dans le même appel, avec contrôle négatif : (a) une branche locale dont la
  PR est mergée n'est pas reprise ; (b) une branche sur origin dont la PR est mergée n'est
  pas reprise ; (c) une branche dont la PR est ouverte **reste** reprise, ce qui est le cas
  nominal de reprise d'un vol en cours.
- **AC3.** Aucune perte : si l'ancienne branche porte des commits absents de `main` et sans
  PR mergée, le dispatch le dit et s'arrête, au lieu de les écraser.
- **AC4.** Sonde après déploiement : sur les 10 grooms suivants d'un vol 2 ou plus, le
  compte de `REBASE_CONFLICT` dus à une branche de vol mergé est écrit dans le ticket. La
  cible est 0.
- **AC5.** *(vol 2 — hors périmètre de cette PR.)* Le callback de `REBASE_CONFLICT` ne lance
  pas de `git rebase` ni de résolution de conflit dans un worktree. Il rend la main, et la
  parente devient terminale, sans rester `in_progress` après que son dernier enfant est
  livré. Test dans le même appel : un callback `REBASE_CONFLICT` laisse le worktree tel que
  le dispatch l'a laissé, et la parente passe à un état terminal.
  Tracked in: senara-solutions/mika#2976 (vol 2)

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `_branch_reuse_verdict` (P1 + P2, les deux refs, les quatre jetons) | 100 |
| `_stash_and_remove_worktree` (extraction + réécriture du site du relic) | 30 |
| `_reset_spent_branch` | 30 |
| site d'appel, `BRANCH_SPENT`, troisième bras, halte AC3, ligne de journal | 50 |
| tests T1-T6 + extension du harnais `_fixture_setup` (`test-dispatch-lib.sh`) | 190 |

Total estimé : 400 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient.** 400 < 429, et sous le seuil par PR de 1000. La marge est de 29
lignes, donc mince, et trois choix la financent plutôt que de l'espérer :

1. **L'extraction du § 4.1 au lieu d'un second geste de nettoyage.** Le stash, la capture du
   SHA, l'indice de récupération et la garde mika#1943 existent déjà, écrits et testés ; les
   réécrire aurait coûté une soixantaine de lignes et un second comportement à maintenir.
2. **Aucune ligne de Rust.** La porte vit entièrement dans le shell qui dérive la base. La
   moitié Rust du ticket est l'AC5, renvoyée au vol 2 (§ 10).
3. **Le commit 2 comme plancher** (§ 7). Si le pilote meurt au plafond, l'instrument de
   l'AC4 est déjà sur la branche et la PR reste mergeable ; la décision suit au vol suivant
   sans rien jeter.

Le poste qui peut déraper est le dernier, les tests : six cas × ~30 lignes. S'il dépasse,
T2 se réduit à sa forme minimale (une branche sur origin à **un** commit de retard au lieu
de la forme à 584) — la scène testée est la même, le bras `_remote_branch_exists` étant le
seul terme sous test.
