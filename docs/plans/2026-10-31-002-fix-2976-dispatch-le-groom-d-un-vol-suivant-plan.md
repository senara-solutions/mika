# mika#2976 — vol 2 : la branche usée se juge sur son contenu cumulé, pas commit par commit

Ce plan livre **AC6** et **AC7**, et rend l'instrument d'**AC4** utilisable. Le vol 1
(mika#2985, squash `07e3d6ff0`, déployé le 2026-10-31 à 06:51Z) a remplacé un
`REBASE_CONFLICT` par un `BRANCH_HAS_UNMERGED_WORK`. L'arrêt est sûr — rien n'est perdu —
mais il **bloque tout vol suivant**, parce que son étage de containment compare les
patch-ids **commit par commit** et qu'un squash n'en préserve aucun.

Ce vol ajoute un second étage de containment, sur le **patch cumulé** de la branche, et
laisse intact tout ce que le vol 1 décide déjà correctement.

**AC5 n'est pas dans ce vol**, pour les raisons du § 10 du plan du vol 1 — autre fichier,
autre couche, autre producteur — et parce que le voisin que mika#2980 vient de fermer
(`d498bfc8b`) montre que la terminalité de la parente est un sujet à lui seul.

Tracked in: senara-solutions/mika#2976 (vol 3 — AC5)

---

## 1. Ce que la mesure dit, et où elle corrige le ticket

Toutes les mesures de cette section ont été prises le 2026-10-31 dans le worktree de
grooming, sur les objets réels du cas AC7. Les trois shas y sont accessibles
(`git cat-file -t` rend `commit` pour `59010336a`, `fcb5148fd`, `9d6c12257`).

### 1.1 La topologie du cas mesuré

```
merge-base(origin/main, 59010336a) = 9d6c122573d2…   (squash de #2970)
branche feat/2958/… :  8fedcffd3 (plan) → 740492e57 → 59010336a
atterrissage sur main : fcb5148fd  (squash de #2975), parent a219d0a3c
```

Le parent du squash est `a219d0a3c`, **pas** la merge-base : une autre PR a mergé entre la
création de la branche et son atterrissage. Ce détail est la raison pour laquelle deux des
trois critères envisageables échouent, et il est au cœur de ce plan.

### 1.2 Le premier critère proposé par le ticket est RÉFUTÉ par la mesure

Le ticket propose : *« la différence cumulée `merge-base..tête` est déjà contenue dans
`origin/main` (diff vide entre `origin/main` et la tête sur les fichiers de la
branche) »*, et affirme : *« `git diff fcb5148fd 59010336a` sur ces 8 fichiers est vide :
tout le contenu de la branche est dans `main` »*.

**Ce diff n'est pas vide.** Mesuré :

```
$ git diff --stat fcb5148fd 59010336a -- <les 8 fichiers de la branche>
 crates/mika-agent/src/milestone_manager/cadence.rs | 1 -
 crates/mika-agent/src/milestone_manager/mod.rs     | 6 +++---
 crates/mika-agent/src/milestone_manager/spawn.rs   | 1 -
 3 files changed, 3 insertions(+), 5 deletions(-)
```

Et l'écart n'est pas du travail de la branche : c'est le champ `milestone_read_audit` /
le type `MilestoneReadAudit`, apportés sur `main` par **un autre ticket** avant le squash,
que la branche ne portait pas parce qu'elle était basée sur `9d6c12257`, antérieur.

Le diff bidirectionnel confond donc deux choses de signes opposés :

- ce que la branche apporte et que `main` n'a pas — le seul vrai danger ;
- ce que `main` apporte et que la branche n'a pas — du simple **retard**, sans danger.

Un critère de non-perte doit être **directionnel**. Celui-là ne l'est pas, et il rendrait
`unmerged_work` sur le cas AC7, c'est-à-dire qu'il **échouerait AC7**. Il est écarté.

### 1.3 Un critère directionnel par `merge-tree` est exact, mais pas au bon point d'ancrage

`git merge-tree --write-tree A B` rend l'OID de l'arbre fusionné. Si cet arbre est celui de
`A`, alors `B` n'apporte rien à `A` : c'est un critère directionnel et exact. Mesuré au
point d'atterrissage :

```
$ git rev-parse fcb5148fd^{tree}                     → c9bb48afd9ba…
$ git merge-tree --write-tree fcb5148fd 59010336a    → c9bb48afd9ba…   (identique)
```

Mais le dispatch ne dispose pas du point d'atterrissage ; il dispose de `origin/main`, qui
a avancé depuis. Contre la tête réelle, le même critère **conflicte** :

```
$ git merge-tree --write-tree origin/main 59010336a
→ exit 1, CONFLICT (content) in crates/mika-agent/src/milestone_manager/cadence.rs
```

Ce conflit n'est pas du travail en péril : `main` a réécrit les mêmes lignes après
l'atterrissage. Trouver le point d'atterrissage demanderait un balayage de la fenêtre,
soit un `merge-tree` par candidat — le coût que le § 1.4 évite. Écarté.

### 1.4 Le second critère proposé par le ticket est CONFIRMÉ — appliqué au patch CUMULÉ

Le ticket propose aussi : *« ou son patch-id est celui d'un commit de `main` »*. Mesuré :

```
$ git diff 9d6c12257 59010336a | git patch-id --stable
402470424f2172c02df48aab26b4aeff0cb054c7 0000…           ← le patch cumulé de la branche

$ git show fcb5148fd | git patch-id --stable
402470424f2172c02df48aab26b4aeff0cb054c7 fcb5148fd211…   ← le squash sur main
```

**Égalité exacte.** Et elle n'est pas une coïncidence : `git patch-id --stable` hashe les
seules lignes ajoutées et supprimées, normalisées, en **ignorant les numéros de ligne et
le contexte**. C'est précisément ce qui le rend insensible au rebase qu'un squash subit
avant d'atterrir — le mécanisme même par lequel les critères des § 1.2 et 1.3 échouent.

C'est aussi l'idiome que la maison a déjà : `--cherry-mark` compare des patch-ids
(`_check_duplicate_commits`, mika#784), et le vol 1 s'en sert. Ce vol ne change pas
d'instrument, il change d'**unité de comparaison** : le patch cumulé de la branche au lieu
de chacun de ses commits.

### 1.5 Le coût du balayage, mesuré

La fenêtre à balayer est `merge_base..origin/main` — le squash d'atterrissage est
nécessairement après la merge-base, puisque la branche a divergé là. Elle fait 8 commits
dans le cas AC7 ; elle en ferait ~584 dans le cas mika#1727.

```
$ git log --format='%H' -p --no-merges 9d6c12257..origin/main | git patch-id --stable
→ 8 lignes « <patch-id> <sha> », dont  402470424…  fcb5148fd…

$ time git log -p --no-merges -n 600 origin/main | git patch-id --stable | wc -l
→ 600     1.77 s total
```

**1,77 s pour 600 commits.** Le balayage est donc abordable même dans le pire cas mesuré,
au site le plus synchrone du dispatch. Un plafond reste nécessaire pour borner le cas
pathologique (une branche de plusieurs milliers de commits de retard), mais il peut être
généreux : **1000**, soit ~3 s dans le pire cas autorisé.

### 1.6 `patch-id` est muet sur un diff vide

```
$ git diff origin/main origin/main | git patch-id --stable | wc -l
→ 0
```

Zéro ligne, pas une ligne de zéros. Un patch cumulé vide doit donc être traité **avant**
la comparaison, et il vaut `spent` de plein droit : une branche qui n'apporte aucun
contenu n'a rien à perdre. Le confondre avec une non-mesure ferait arrêter le dispatch sur
une branche sans enjeu.

---

## 2. Le défaut, en une phrase, et pourquoi il mord sur TOUT vol suivant

Le vol 1 décide `unmerged_work` dès qu'un commit de la branche porte un patch-id absent de
`main` (`log --cherry-mark --right-only origin/main...$tip`, les lignes `>`).

Un squash fusionne N patchs en **un** patch, dont le patch-id n'égale celui d'aucun des N.
Or **chaque vol porte au moins deux commits** — le plan puis le code. Donc tout vol dont
le prédécesseur a été mergé en squash sort avec N lignes `>` et s'arrête.

Les fixtures T1, T2, T5 et T6 du vol 1 ne simulent qu'une branche à **un** commit, qu'un
`cherry-pick -x` laisse au même patch-id. Elles sont justes, et elles ne pouvaient pas
voir ce défaut : aucune ne construit un squash. C'est exactement ce qu'AC6 exige de
corriger — « sur une fixture à plusieurs commits squashée en un seul commit ».

Le relevé du ticket le confirme sur la production : 1 arrêt à tort sur 1 groom de vol
suivant depuis le déploiement du vol 1.

---

## 3. Le critère retenu, et ce qui reste `unmerged_work`

La mesure de containment devient un **escalier à deux étages**, évalué **par ref de tête**
(le vol 1 lit `refs/heads/$branch` et `refs/remotes/origin/$branch`, et ce vol ne change
pas cette population) :

| étage | question | si oui |
|---|---|---|
| **E1** (vol 1, inchangé) | aucun commit de la branche n'a de patch-id absent de `main` ? | la ref est absorbée |
| **E2** (ce vol) | le patch-id du diff cumulé `merge_base..tip` est-il celui d'un commit de `merge_base..origin/main` ? | la ref est absorbée |
| sinon | — | la ref est **en péril** |

Et le verdict global : `spent` si **toutes** les refs lues sont absorbées, `unmerged_work`
dès qu'une seule est en péril.

**L'ordre des étages est celui du coût croissant, et il est non-régressif.** E1 est déjà
payé par le vol 1 ; E2 ne s'évalue que sur les refs qu'E1 n'a pas absorbées. Aucune
branche que le vol 1 déclare `spent` ne peut changer de verdict : ce vol ne peut
qu'**élargir** `spent`, donc que **réduire** la population d'arrêt. C'est la propriété qui
rend le vol sûr par construction, et le § 9 en fait l'argument de Fire-Disposition.

**Ce qui reste `unmerged_work`, donc l'arrêt d'AC3 :** un commit dont le contenu n'est ni
dans `main` commit par commit, ni absorbé dans le patch cumulé retrouvé sur `main`. C'est
le `wip()` de la chaîne de secours, le commit poussé après l'approbation — la population
que T5 exerce déjà, et que le contrôle négatif d'AC6 exercera dans la topologie du squash.

### 3.1 Les quatre non-mesures d'E2, et pourquoi elles ne valent pas `spent`

E2 peut ne pas conclure pour quatre raisons, et **aucune** ne doit valoir absorption : une
non-mesure qui vaudrait `spent` ferait réinitialiser une branche dont on ne sait rien, et
c'est le seul geste destructif de toute cette porte.

1. `git merge-base origin/main $tip` échoue (histoires disjointes).
2. La fenêtre `merge_base..origin/main` dépasse le plafond de 1000.
3. `git patch-id` ne rend pas un patch-id lisible pour le patch cumulé.
4. Le balayage de la fenêtre échoue.

Dans les quatre cas, le verdict retombe sur E1, donc sur `unmerged_work` — l'arrêt sûr.

**Mais le jeton de containment doit le DIRE**, et c'est le § 4 : « la branche porte du
travail réel » et « on n'a pas su mesurer » sont deux populations, et deux populations sous
un même nom ne sont pas soustrayables (classe mika#2205). Le relevé d'AC4 a précisément
besoin de les distinguer, puisqu'un arrêt de la seconde espèce est un défaut de ce vol-ci
alors qu'un arrêt de la première est son fonctionnement voulu.

---

## 4. L'observabilité — un cinquième champ, et pourquoi il n'est pas optionnel

La ligne du vol 1 est l'instrument d'AC4 :

```
[dispatch-lib] branch_reuse_gate: branch=… verdict=… pr=… unmerged=… tips=… action=…
```

Elle gagne **un champ**, `containment=`, à vocabulaire fermé :

| jeton | sens |
|---|---|
| `per_commit` | E1 a conclu seul — le comportement du vol 1, inchangé |
| `cumulative` | E2 a mordu — la population que ce vol débloque |
| `work` | ni E1 ni E2 : du travail réellement absent de `main` (AC3) |
| `unmeasured` | une des quatre non-mesures du § 3.1 |
| `none` | aucune ref lue, ou le verdict s'est joué en P1 (PR ouverte, aucune PR) |

Sans ce champ, le relevé d'AC4 ne peut pas répondre à la question qui décide s'il faut un
vol 3 : *l'arrêt restant est-il du travail réel, ou une non-mesure ?* C'est la même raison
qui a fait séparer `verdict` et `action` au vol 1, et elle vaut ici à l'identique.

La globale `_BRANCH_REUSE_CONTAINMENT` est posée par `_branch_reuse_verdict` et lue par
`_branch_reuse_gate`, **sans `$( )`** — comme les cinq autres, et pour la raison que le
vol 1 a déjà écrite : un sous-shell les ramènerait à leurs défauts et la ligne mesurerait
sa propre valeur par défaut (mika#2272).

---

## 5. L'implémentation — une fonction, une boucle remaniée, un champ

Tout tient dans `skills/bundled/_shared/dispatch-lib.sh`, dans le bloc mika#2976 existant.
Aucun autre fichier de code n'est touché.

### 5.1 `_branch_cumulative_absorbed` — nouvelle, et c'est le seul site d'E2

```
# Args : $1 repo_dir, $2 tip (un sha)
# Rend 0 si le patch cumulé de la branche est retrouvé sur `main`, 1 sinon.
# Pose `_BRANCH_CUMULATIVE_REASON` : absorbed | empty_patch | no_merge_base |
#                                     window_too_large | unreadable_patch_id | not_found
```

Corps, dans l'ordre :

1. `merge_base=$(git -C "$repo_dir" merge-base origin/main "$tip")` — échec ⇒
   `no_merge_base`, rend 1.
2. `window=$(git -C "$repo_dir" rev-list --count "$merge_base..origin/main")` — la mesure
   du coût **avant** de le payer. Au-delà de `_BRANCH_REUSE_WINDOW_MAX` (1000) ⇒
   `window_too_large`, rend 1.
3. Le patch cumulé :
   `cum=$(git -C "$repo_dir" diff "$merge_base" "$tip" | git patch-id --stable | awk '{print $1}')`.
   Vide ⇒ **`empty_patch`, rend 0** (§ 1.6 : rien à perdre). Non hexadécimal ⇒
   `unreadable_patch_id`, rend 1.
4. Le balayage, en **une** invocation :
   `git -C "$repo_dir" log --format='%H' -p --no-merges "$merge_base..origin/main" | git patch-id --stable`,
   et un `grep -q "^$cum "` dessus. Trouvé ⇒ `absorbed`, rend 0 ; sinon ⇒ `not_found`,
   rend 1 ; échec de lecture ⇒ `unreadable_patch_id`, rend 1.

Deux points de forme qui ne sont pas du style :

- **`--no-merges` est explicite.** Un commit de merge n'a pas de patch-id exploitable, et
  un squash n'est jamais un merge. L'écrire évite qu'une lecture future prenne l'absence
  de diff de merge pour un patch vide.
- **Le plafond est une constante nommée en tête de bloc**, `_BRANCH_REUSE_WINDOW_MAX=1000`,
  avec en commentaire la mesure du § 1.5 qui le justifie. Un littéral à son site d'usage ne
  survivrait pas à la prochaine recalibration.

### 5.2 La boucle P2 de `_branch_reuse_verdict` — remaniée par ref

Aujourd'hui la boucle accumule les `>` de chaque tip dans `unmerged_lines`, puis décide.
Elle devient, par tip :

```
right = les `>` de `origin/main...$tip`
si right est vide                          → la ref est absorbée, containment ⊇ per_commit
sinon si _branch_cumulative_absorbed tip   → la ref est absorbée, containment ⊇ cumulative
sinon                                      → right entre dans unmerged_lines
                                             containment ⊇ work | unmeasured
                                             (selon _BRANCH_CUMULATIVE_REASON)
```

Le containment global est le **plus informatif** des jetons collectés sur les refs, dans
l'ordre `work` > `unmeasured` > `cumulative` > `per_commit` : un seul tip en péril décide
du verdict, donc il doit décider aussi du mot qui le nomme.

La déduplication par sha de `unmerged_lines` (`awk '!seen[$1]++'`) et la lecture des deux
refs sont **conservées telles quelles** : ce sont les acquis du troisième cas du ticket, où
la ref locale `origin/…` pointait sur un sha que le distant ne portait plus.

### 5.3 Le message de halte — la raison, pas seulement le compte

Le `RESULT` du vol 1 dit « carries N commit(s) absent from origin/main **by patch-id** ».
Avec E2, cette phrase devient incomplète : les commits nommés sont ceux qu'E1 n'a pas
absorbés, mais ce qui décide de l'arrêt est qu'E2 n'a pas retrouvé le patch cumulé non
plus. La phrase le dit :

```
STATUS=BRANCH_HAS_UNMERGED_WORK
Branch <b> carries <N> commit(s) that origin/main contains neither commit-by-commit
nor in a squashed patch (<raison>), while its PRs are all terminal (<prs>).
Unmerged commits:
<sha> <sujet>…
Nothing was reset, nothing was removed, nothing was stashed.
Resolve manually before re-dispatching mika#<n>.
```

`<raison>` est `_BRANCH_CUMULATIVE_REASON`. Sur `window_too_large` ou `no_merge_base`,
l'opérateur lit que la porte **n'a pas su**, et non que son travail est en danger — deux
gestes de reprise différents.

Les trois invariants du vol 1 restent intacts : les commits sont **nommés** et pas
seulement comptés ; la phrase sur le non-effet reste vraie par le placement du site
d'appel, en amont du nettoyage du relic ; le statut reste distinct de `REBASE_CONFLICT`.

---

## 6. Les tests — `test-dispatch-lib.sh`, déjà câblée

La suite tourne dans CI par `make test-dispatch-lib` (`ci.yml:85`, `Makefile:199`). Les
nouveaux cas sont donc atteints sans entrée au registre `scripts/suite-wiring.tsv`, et le
lint `suite-wiring` n'a rien à déclarer.

Les trois cas s'ajoutent **après** T6, réutilisent le harnais existant
(`_t2976_fixture`, `_t2976_commit`, `_t2976_refs`), exercent `_branch_reuse_gate` — la
production, jamais une relecture de sa table — et assertionnent **les deux refs avant et
après**, comme T1-T6.

### T7 (AC6) — plusieurs commits, un seul squash, et son contrôle négatif

La fixture que le vol 1 n'avait pas. Deux commits sur la branche (le plan puis le code),
puis **un seul** commit sur `main` qui porte leur diff cumulé — le squash :

```
git checkout -b $br
_t2976_commit plan.md  …  "docs(plans): groom mika#2976"
_t2976_commit code.txt …  "feat(2976): le code du vol 1"
tip=HEAD ;  mb=$(git merge-base main $tip)
git checkout main
git merge --squash $tip  &&  git commit -m "feat(2976): … (#2985)"
git push origin main
```

`git merge --squash` et non deux `cherry-pick -x` : c'est le geste qui produit **un** patch
pour **deux** commits, donc le seul qui construise la scène d'AC6. Un `cherry-pick -x` par
commit garderait les patch-ids individuels et E1 absorberait la branche — la fixture
mesurerait alors le vol 1 et passerait quoi qu'on fasse d'E2. **La fixture assertionne sa
propre scène** avant de mesurer : `main` porte exactement un commit de plus que `mb`, et
aucun des deux shas de la branche n'est sur `main`.

Attendu : `verdict=spent`, `containment=cumulative`, `action=reset`, `BRANCH_SPENT=1`, la
branche repartie de `origin/main`, le worktree retiré, `rc=0`.

**Contrôle négatif dans la même fixture**, comme AC6 l'exige : un troisième commit ajouté
sur la branche **après** le squash, absent de `main`. Attendu : `verdict=unmerged_work`,
`containment=work`, `action=halt`, `rc=1`, le sha du troisième commit **nommé** dans
`RESULT`, et rien n'a bougé (refs identiques, aucune entrée de stash créée, worktree
intact).

### T8 (AC7) — la topologie mesurée, avec le retard de `main`

Le cas réel, reproduit plutôt que rejoué sur ses objets : `59010336a` vivait sur une
branche supprimée le 2026-10-31, donc un clone de CI ne porterait pas cet objet et un test
ancré dessus serait vert ici et rouge en CI. La fixture reproduit ce qui **fait** le cas,
c'est-à-dire la raison pour laquelle les critères des § 1.2 et 1.3 échouent :

1. branche à deux commits depuis `mb` ;
2. sur `main`, **d'abord** un commit tiers qui touche **un des fichiers de la branche**
   (le rôle de `milestone_read_audit`) ;
3. **puis** le squash de la branche, rebasé par-dessus ;
4. **puis** un commit de plus sur `main` qui réécrit encore ce fichier (le rôle de
   l'avancement de `main` qui fait conflicter `merge-tree`).

Attendu : `verdict=spent`, `containment=cumulative`. Et deux assertions qui **épinglent les
critères écartés**, pour que leur réintroduction rougisse ici au lieu de rejouer AC7 en
production : dans cette fixture, `git diff origin/main $tip -- <fichiers>` est **non vide**,
et `git merge-tree --write-tree origin/main $tip` **ne rend pas** l'arbre de `origin/main`.
Un futur refactor qui remplacerait E2 par l'un des deux casse T8 avec le motif en clair.

Le sha du cas réel (`59010336a`, contre `fcb5148fd`) est cité en commentaire de T8 avec sa
mesure du § 1.4, pour que la fixture nomme son origine.

### T9 (non-mesure) — la fenêtre dépassée ne vaut pas absorption

Sépare, pour le relevé d'AC4, « du travail réel » de « on n'a pas su mesurer ». La scène de
T7 avec `_BRANCH_REUSE_WINDOW_MAX=0` exporté, ce qui force le bras `window_too_large` sans
construire mille commits.

Attendu : `verdict=unmerged_work`, `containment=unmeasured`, la raison dans `RESULT`,
`action=halt`, `rc=1`, et rien n'a bougé. Ce test est aussi l'anti-vacuité de T7 : si le
plafond n'était pas lu, T9 passerait en `cumulative` et rougirait.

### Scans de bonne foi

Dans le style des scans du vol 1, trois assertions de source :

- `_branch_cumulative_absorbed` nomme `patch-id --stable` et `--no-merges` ;
- `_branch_reuse_gate` émet `containment=` dans sa ligne ;
- le plafond est lu depuis `_BRANCH_REUSE_WINDOW_MAX` et n'apparaît pas en littéral `1000`
  dans le corps de la fonction.

---

## 7. Ordre de commit, et le plancher

1. **`docs(plans)`** — ce fichier. Le plancher : si le vol meurt ensuite, le plan est sur
   la branche et le regroom repart d'ici.
2. **`fix(2976)`** — `_branch_cumulative_absorbed`, la boucle P2 remaniée, le champ
   `containment=`, le message de halte. Vérifié par `make test-dispatch-lib` : les six cas
   du vol 1 restent verts (non-régression) **avant** d'ajouter les nouveaux.
3. **`test(2976)`** — T7, T8, T9 et les scans.
4. **`docs(solutions)`** — mise à jour de
   `docs/solutions/logic-errors/stale-base-conflicting-prs-no-self-heal-2026-04-23.md`,
   qui porte déjà la section du vol 1, avec le § 1.2 de ce plan : le critère du diff
   restreint est réfuté, et par quelle mesure. C'est la partie du travail qui empêche le
   vol 3 de le réessayer.

Le commit 2 est le seul qui puisse casser la production ; le lancer avec la suite du vol 1
seule, avant d'écrire les nouveaux tests, est ce qui sépare « E2 marche » de « E2 ne casse
rien ».

---

## 8. La sonde d'AC4 — ce n'est pas du code

AC4 se relève après déploiement, et le champ `containment=` du § 4 est ce qui le rend
lisible. Le relevé s'écrit dans le corps du ticket, et il compte, sur les 10 grooms
suivants d'un vol 2 ou plus :

- les `REBASE_CONFLICT` dus à une branche de vol mergé — cible **0** ;
- les `BRANCH_HAS_UNMERGED_WORK` avec `containment=work` — fonctionnement voulu, pas un
  défaut ;
- les `BRANCH_HAS_UNMERGED_WORK` avec `containment=unmeasured` — défaut de ce vol, et
  cause d'un vol 3 ;
- les `verdict=spent containment=cumulative` — la population que ce vol débloque, et la
  preuve qu'il sert.

La ligne se lit par `grep 'branch_reuse_gate:'` dans les journaux de dispatch.

---

## 9. Fire-Disposition

Ce plan livre des détecteurs : T7, T8, T9 (tests), les trois scans de source, et la halte
`BRANCH_HAS_UNMERGED_WORK` dont le chemin de succès est « aucun travail en péril ».

**Option retenue : (a) exception nommée en allowlist — et l'allowlist est VIDE, par une
propriété démontrable plutôt que par constat.**

E2 ne s'évalue **que** sur les refs qu'E1 n'a pas absorbées (§ 3). Il ne peut donc
qu'élargir `spent`, c'est-à-dire que **réduire** la population qui s'arrête. Un détecteur
qui ne peut pas grossir sa propre population d'arrêt n'a aucune violation préexistante à
exempter : il n'y a rien à mettre dans l'allowlist, et ce n'est pas une omission.

L'assertion auto-nettoyante qui rougit si cela cesse d'être vrai est **T1 à T6 du vol 1,
inchangées** : elles couvrent `spent`/`per_commit` (T1, T2), `live` (T3, T4),
`unmerged_work` (T5) et `undetermined` (T6). Si E2 modifiait l'un de ces verdicts, la
propriété serait fausse et la suite rougirait au site exact. Le § 7 en fait une étape de
commit distincte précisément pour que ce rouge arrive avant les nouveaux tests.

À cela s'ajoute l'assertion explicite, dans le bloc des scans : aucun nom de branche, aucun
sha et aucun numéro de ticket ne figurent en exception dans la porte — la liste des
exceptions de mika#2976 est vide, et un `assert_not_contains` sur le motif d'allowlist le
tient. Elle rougit le jour où quelqu'un y ajoute une entrée sans ticket de suivi.

---

## 10. Discipline de l'implémenteur — quatre pièges nommés

1. **Ne pas toucher E1.** La tentation est de remplacer `--cherry-mark` par la mesure
   cumulée. Non : E1 absorbe des cas qu'E2 ne voit pas (une branche dont `main` a avancé
   sur ses fichiers après un cherry-pick), et le retirer ferait régresser T1/T2. Les deux
   étages coexistent.
2. **`git merge --squash` dans la fixture de T7**, jamais deux `cherry-pick -x`. Avec des
   cherry-picks, E1 absorbe la branche et T7 passe sans qu'E2 soit exercé : le test
   mesurerait le vol 1. La fixture assertionne sa scène pour que ce glissement rougisse.
3. **Pas de `branch -D`, ni dans la porte ni dans les fixtures.** Le vol 1 l'a exclu
   (son § 11) et un `assert_not_contains 'branch -D'` sur tout `dispatch-lib.sh` le tient
   déjà. Pour retirer une ref dans une fixture : `git update-ref -d refs/heads/<b>`, comme
   T2.
4. **`patch-id` muet n'est pas un patch vide.** Le § 1.6 : zéro ligne de sortie. Le bras
   `empty_patch` se décide sur le **diff** vide (`git diff --quiet`), pas sur l'absence de
   sortie de `patch-id`, qui est aussi ce qu'une erreur produit.

Les règles de dispatch du siège s'appliquent : brouillons sous `.pilot-scratch/` et jamais
supprimés, `pr-body.md` à la racine du worktree, aucun `push` depuis le pilote.

---

## 11. Definition of Done

- `_branch_cumulative_absorbed` existe dans `dispatch-lib.sh`, site unique d'E2, avec le
  plafond en constante nommée et les six raisons du § 5.1.
- La boucle P2 de `_branch_reuse_verdict` évalue E2 par ref, uniquement sur les refs
  qu'E1 n'a pas absorbées, et conserve la lecture des deux refs et la déduplication par
  sha.
- `containment=` figure dans la ligne `branch_reuse_gate:`, à vocabulaire fermé (§ 4), et
  la raison de non-mesure figure dans le `RESULT` de la halte.
- `make test-dispatch-lib` est vert : T1-T6 inchangées, T7 (avec son contrôle négatif), T8,
  T9 et les trois scans ajoutés.
- `docs/solutions/logic-errors/stale-base-conflicting-prs-no-self-heal-2026-04-23.md` porte
  le § 1.2 — le critère du diff restreint réfuté, avec sa mesure.
- `scripts/verify-pipeline.sh origin/main`, `cargo clippy`, `cargo fmt --check` et les
  lints du dépôt passent. Aucun fichier Rust n'est touché par ce vol.
- Le corps du ticket porte le relevé d'AC4 mis à jour après déploiement (§ 8), et AC5 est
  nommé comme vol 3 par la ligne `Tracked in:`.

---

## 12. Acceptance criteria

Transcrits du corps de senara-solutions/mika#2976. **AC6 et AC7 sont le périmètre de ce
vol** ; AC1-AC3 sont livrés par le vol 1 (mika#2985) et ce vol ne doit pas les faire
régresser ; AC4 est une sonde post-déploiement ; AC5 est renvoyé au vol 3.

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
- **AC5.** Le callback de `REBASE_CONFLICT` ne lance pas de `git rebase` ni de résolution
  de conflit dans un worktree. Il rend la main, et la parente devient terminale, sans
  rester `in_progress` après que son dernier enfant est livré. Test dans le même appel : un
  callback `REBASE_CONFLICT` laisse le worktree tel que le dispatch l'a laissé, et la
  parente passe à un état terminal. — **vol 3**, voir `Tracked in:`.
- **AC6.** Test dans le même appel, sur une fixture **à plusieurs commits** (au moins le
  plan et un commit de code) squashée en **un seul** commit sur `main` : le verdict est
  `spent` et l'action `reset`. Contrôle négatif dans la même fixture : un commit ajouté
  après le squash, absent de `main`, donne `unmerged_work` et l'arrêt.
- **AC7.** Le cas mesuré : la branche `feat/2958/manager-phase-2-c-vol-2-le-manager` à
  `59010336a`, contre `origin/main` qui porte `fcb5148fd`, rend `spent`. La branche locale
  a été supprimée le 2026-10-31 sur GO de samidarko ; elle se retrouve par son sha.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `dispatch-lib.sh` — `_branch_cumulative_absorbed` + sa doc | 85 |
| `dispatch-lib.sh` — boucle P2 remaniée par ref | 40 |
| `dispatch-lib.sh` — `containment=`, globales, message de halte | 30 |
| tests — T7 (AC6 + contrôle négatif) | 95 |
| tests — T8 (AC7, critères écartés épinglés) | 70 |
| tests — T9 (non-mesure, fenêtre dépassée) | 50 |
| tests — scans de bonne foi + entrées de boucle | 25 |

Total estimé : 395 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 395 < 429, et 395 < 1000 : les deux bornes passent, la plus
serrée (429) mord et laisse 34 lignes de marge. Le vol ne touche **qu'un seul fichier de
production**, dans un bloc de ~300 lignes déjà écrit et déjà testé par six cas verts, et
**aucun fichier Rust** — donc ni `cargo build` ni `cargo test` dans la boucle de
vérification, qui sont les deux postes de tours les plus chers. La fin de pipeline est
`make test-dispatch-lib` plus les lints shell, soit quelques dizaines de secondes.

AC5 est hors de ce vol pour la raison inverse : il touche un autre fichier (le prompt de
`self-dev-callback`), une autre couche (la terminalité de la parente, en Rust, dans le
voisinage que mika#2980 vient de modifier) et il ajouterait à lui seul plus de 300 lignes,
ce qui ferait sortir le total de la borne.

Tracked in: senara-solutions/mika#2976 (vol 3 — AC5)
