# mika#3038 — le secours de fin de vol ne publie pas un arbre qui est déjà celui de `main`

**Ticket :** senara-solutions/mika#3038
**Labels :** `bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`
**Branche :** `fix/3038/dispatch-lib-le-secours-de-fin-de-vol`
**Dépôt unique :** `mika`. `dispatch-lib.sh` vit dans `skills/bundled/_shared/`
et le moteur en installe la copie dans `~/.mika/skills/_shared/` au deploy.
claude-pilot n'est pas touché, et il n'y a pas de vol entre dépôts.

---

## 0. Le vol, en une phrase

Le secours de fin de vol décide d'ouvrir une draft sans jamais demander si la
branche **apporte** quelque chose à `origin/main` ; on lui donne ce prédicat —
l'égalité des arbres de tête — et on le fait consulter aux trois endroits qui en
dépendent : l'ouverture de la draft, le marqueur `rescue-diff` qu'elle porte, et
le push qui la précède.

---

## 1. L'état mesuré

### 1.1 La population

Quatre drafts de secours depuis le 2026-09-04 dont l'arbre de tête est
**identique** à celui d'`origin/main` à l'heure d'ouverture. Toutes portent
`rescue-diff: carries-work`, toutes s'ouvrent moins de 20 minutes après le merge
d'une PR de la même branche, toutes ont été fermées à la main par `samidarko`.

| draft | ouverte | merge précédent sur la branche | écart |
|---|---|---|---|
| #2197 | 2026-09-05 14:41:12Z | #2196, 14:22:33Z | 19 min |
| #2702 | 2026-10-14 20:37:16Z | #2696, 20:31:53Z | 5 min |
| #2899 | 2026-10-23 11:44:19Z | #2898, 11:38:36Z | 6 min |
| #3034 | 2026-11-04 08:07:36Z | #3033, 07:58:23Z | 9 min |

Le contrôle négatif est dans la mesure elle-même : cinq autres secours ouverts
sur une branche déjà mergée (#2198, #2834, #2842, #2920, #3016) portaient un
**vrai** diff et ont été mergés. Le geste de ce vol ne doit rien leur retirer, et
c'est ce que le témoin d'AC1 vérifie.

Sur #3034, GitHub affiche `+953 −23` alors que l'arbre est identique
(`d97b53bdf^{tree}` = `d7de73137^{tree}`). Le nombre vient du squash : les
commits de la branche ne sont pas des ancêtres de `main`, donc tout ce que la
branche a touché depuis la base de fusion reparaît comme diff, bien que son
contenu soit déjà dans `main`. C'est la même illusion que lit
`_rescue_diff_carries_work`, et c'est pourquoi le marqueur dit `carries-work`.

Effet en aval, et ce qui fait de ce bug un bug de substrat : à 08:07:42Z, six
secondes après l'ouverture de #3034, `ready` a été retiré de #2194 par
`mika-platform-qa` au titre de cette draft. Une draft vide a arrêté un ticket.

### 1.2 Les quatre sites, lus sur `790c8959b`

| élément | ligne | ce qu'il fait |
|---|---|---|
| choix de la classe | 14106-14115 | `no-shipping-tail`, puis `commit-pushed-no-pr`, sur `PR_URL` vide et `PRE_RUN_HEAD != POST_RUN_HEAD` |
| `_recovery_pr_due` | 14122-14125 | les quatre termes de l'ouverture, évalués une fois |
| refus mika#2503 | 14146-14172 | `_rescue_touches_tracked_tree` : refuse un contenu qui ne touche aucun répertoire de premier niveau connu. Ne dit rien d'un arbre **égal** à `main` |
| `_rescue_diff_carries_work` | 12172-12206 | base `origin/main...HEAD`, **trois** points : l'écart depuis la base de fusion |
| `_push_branch` | 8848-9077 | appelé l. 14078, avant le secours. En `first-push` il repousse la branche que le merge vient de supprimer d'`origin` |
| `_pr_list_url` | 3287-3297 | `gh pr list --head <branche>` sans `--state` : seules les PR **ouvertes**. La PR mergée de la branche ne compte pas, donc `PR_URL` est vide |

Le seul cas mesuré traverse `no-shipping-tail`. `commit-pushed-no-pr` et
`dirty-worktree` partagent le même test d'entrée et le même
`_recovery_pr_due` ; le geste se place donc là où il les couvre toutes les
trois par construction, ce qui est l'argument déjà écrit au site mika#2503
(l. 14130-14137) et qu'on n'a pas à réinventer.

### 1.3 Trois faits de terrain qui dictent la forme du geste

**(a) `origin/main` local peut être en retard d'un merge — celui qui crée le
défaut.** La draft naît quelques minutes après le merge de la PR de la branche.
Si la ref locale `origin/main` du worktree date du setup du vol, elle ne porte
pas ce merge, les arbres ne sont pas égaux, et un prédicat qui se contenterait
de lire la ref locale **ne verrait rien**. Le prédicat doit donc rafraîchir
`refs/remotes/origin/main` avant de comparer. Un fetch, une fois par dispatch.

**(b) Le stderr d'un dispatch qui réussit n'est lu par personne.** Classe
mika#2050, Signaux M et Q : le `Stdio::piped()` de `spawn_long_running_exec`
n'est relu que sous `if !status.success()`. Toute ligne que ce vol veut rendre
lisible va dans `RESULT` — la doctrine déjà appliquée aux lignes `Remote-probe:`
(l. 8943) et `Stale-ref:` (l. 8958) de `_push_branch`.

**(c) La ligne `NO_PR:` a un consommateur strict.**
`dispatcher.rs::parse_no_pr_reason` lit `(?m)^NO_PR:\s+([a-z_]+)`, classe B de
`scripts/canonical-tokens.tsv`. La raison est un token nu en snake_case ; de la
prose y serait tronquée au premier mot minuscule. Et `_set_pr_status_line`
garantit qu'il n'y a **qu'une** ligne de statut par callback.

---

## 2. Le geste

### 2.1 Un prédicat, et il est factuel

`_branch_tree_equals_main <wt_dir>` dans `dispatch-lib.sh`, voisin de
`_rescue_touches_tracked_tree`. Il ne répond pas « faut-il ouvrir » mais
« l'arbre de `HEAD` est-il celui d'`origin/main` ? » — la formalisation que le
ticket a lui-même retenue pour mesurer sa population.

**Trois issues, sur le précédent exact de `_remote_branch_exists`**
(mika#2626, l. 8837-8846), où la maison a déjà refusé de faire d'un signal
illisible un terme satisfait :

| rc | sens | ce que l'appelant en fait |
|---|---|---|
| 0 | les deux arbres sont **égaux** (mesuré) | il refuse |
| 1 | ils **diffèrent** (mesuré) | comportement nominal |
| 2 | **indéterminé** — `origin/main` absent, dépôt illisible | comportement d'avant ce ticket (AC4) |

Corps :

```bash
_branch_tree_equals_main() {
    local wt_dir="${1-}" _head_tree _main_tree
    [ -n "$wt_dir" ] || return 2
    git -C "$wt_dir" rev-parse --is-inside-work-tree >/dev/null 2>&1 || return 2
    _fetch_main_ref_once "$wt_dir"
    _head_tree=$(git -C "$wt_dir" rev-parse --verify 'HEAD^{tree}' 2>/dev/null) || return 2
    _main_tree=$(git -C "$wt_dir" rev-parse --verify 'origin/main^{tree}' 2>/dev/null) || return 2
    [ -n "$_head_tree" ] && [ -n "$_main_tree" ] || return 2
    [ "$_head_tree" = "$_main_tree" ]
}
```

Le garde `-z "$wt_dir"` est celui que trois fonctions voisines portent déjà avec
sa raison écrite (`_rescue_diff_carries_work` l. 12179-12188) : `git -C ""`
opère silencieusement sur le CWD du processus de dispatch, un checkout vivant.
Ici il rend 2 — ni égal, ni différent : non mesuré.

Et une comparaison d'**arbres**, jamais de SHA de commit : c'est ce qui rend le
prédicat insensible au squash, qui change le commit et conserve l'arbre. C'est
le fait même que la mesure du ticket a relevé sur #3034.

### 2.2 Le fetch, une fois par dispatch

```bash
_MAIN_REF_FETCHED=0
_fetch_main_ref_once() {
    [ "$_MAIN_REF_FETCHED" = "1" ] && return 0
    _MAIN_REF_FETCHED=1
    git -C "${1-}" fetch --quiet origin main 2>/dev/null || true
}
```

`|| true` parce que l'échec du fetch n'est pas une raison de changer de
comportement : on compare alors la ref locale telle qu'elle est, ce qui ne peut
qu'affirmer **moins** souvent l'égalité (fail vers le comportement d'avant).
Le drapeau existe parce que le prédicat est consulté à trois sites du même
dispatch et qu'un fetch réseau par site serait trois fois le coût pour une seule
réponse. `fetch origin main` — jamais `fetch --prune origin` nu, écarté avec ses
trois raisons l. 8801-8810 : il mute les refs du common dir, partagé par tous
les worktrees du checkout, dont ceux des dispatches en vol.

### 2.3 AC1 — la draft ne s'ouvre pas

Un second refus, **avant** celui de mika#2503, dans la même forme et au même
endroit (l. ~14146) :

```bash
local _tree_rc=0
_branch_tree_equals_main "$WORKTREE_DIR" || _tree_rc=$?
if [ "$_recovery_pr_due" = "1" ] && [ "$_tree_rc" -eq 0 ]; then
    _recovery_pr_due=0
    echo "rescue_empty_tree_refused: HEAD tree equals origin/main — draft PR withheld (branch=$BRANCH, mika#3038)" >&2
    RESULT="${RESULT}
Draft PR withheld (mika#3038): the tree at HEAD is byte-identical to origin/main, so this branch adds nothing to main — the work it carried is already merged (typically by a squash of its own PR, which leaves the commits unmerged while their content is in main).
Nothing was lost — there is nothing to lose: no content here is absent from main.
If you believe otherwise: git -C <worktree> diff origin/main HEAD --stat"
    _set_pr_status_line "NO_PR: rescue_adds_nothing_to_main"
elif [ "$_tree_rc" -eq 2 ]; then
    _note_tree_compare_undetermined
fi
```

**Avant mika#2503, et c'est voulu :** les deux refus s'excluent par le terme
`_recovery_pr_due = 1` que le premier éteint, donc celui qui s'arme en premier
donne la raison, et « l'arbre est celui de `main` » est la plus précise des
deux — un arbre égal à `main` ne touche, par construction, rien que `main`
n'ait déjà. L'ordre inverse donnerait `rescue_scratch_only` sur la population de
ce ticket, ce qui serait un diagnostic faux.

**La raison est `rescue_adds_nothing_to_main`** — snake_case, token nu, distinct
de `rescue_scratch_only`. Elle est **inscrite là où le sont les raisons
existantes** : la liste nominative du doc-comment de
`dispatcher.rs::parse_no_pr_reason` (l. 5979-5982) et la table du test
`test_parse_no_pr_reason_known_reasons` (l. 7383-7390). Ces deux sites portent
déjà `rescue_pr_create_failed`, qui est de la même famille (une raison écrite
par le secours, pas par la classification de `gh pr list`) ; le nouveau token
s'y range sans élargir le regex, qui est générique.

Le refus **ne détruit rien**, et sur cette population c'est plus vrai que sur
celle de mika#2503 : là-bas le `wip()` existait et l'opérateur avait une
commande pour le publier, ici il n'y a aucun contenu absent de `main`. La
dernière ligne du `RESULT` offre quand même la mesure, pas un geste de
publication : si le lecteur doute, il regarde le diff lui-même.

Les **trois** classes sont couvertes parce que le terme porte sur
`_recovery_pr_due`, pas sur `RECOVERY_CLASS` — le même argument, écrit au même
endroit, que mika#2503 l. 14130-14137.

### 2.4 AC2 — le marqueur `rescue-diff` se mesure sur ce que la branche apporte

`_rescue_diff_carries_work` lit aujourd'hui `origin/main...HEAD`, **trois**
points : l'écart depuis la base de fusion. Son commentaire l. 12146-12151
justifie ce choix contre la forme deux-points de `_ac6_verbatim_stats_block` —
« la question ici est “que porte cette PR”, pas “en quoi diffère-t-elle de la
tête de main” ». Le squash montre que cette justification est incomplète : après
un squash, ce que la PR « porte » depuis la base de fusion **est déjà dans
main**.

Le geste n'est pas de passer au deux-points nu. Le deux-points nu rendrait aussi
les chemins que `main` a avancés **sans** que la branche y touche — une branche
en retard, qui est le cas nominal de la boucle — et les compterait comme
travail : un fail-**open** vers `Closes`, la seule direction que mika#2157
interdit à cette fonction. Le geste est l'**intersection** des deux formes :

- trois points (`origin/main...HEAD`) : ce que la branche a touché depuis la
  base de fusion ;
- deux points (`origin/main HEAD`) : ce qui diffère de la tête de `main`
  maintenant ;
- l'intersection : ce que la branche a touché **et** qui diffère encore de
  `main` — c'est-à-dire ce qu'elle **apporte**.

Un chemin présent dans le seul trois-points est du travail déjà mergé (le cas du
ticket). Un chemin présent dans le seul deux-points est une avance de `main` que
la branche n'a pas faite. Ni l'un ni l'autre n'est apporté par la branche.

```bash
    # Le set « diffère de la tête de main », lu une fois.
    local _vs_main
    _vs_main=$(git -C "$wt_dir" -c core.quotePath=false diff --name-only -z origin/main HEAD 2>/dev/null | tr '\0' '\n')

    while IFS= read -r -d '' f; do
        [ -n "$f" ] || continue
        grep -qxF -- "$f" <<<"$_vs_main" || continue
        case "$f" in
            .claude/claude-pilot.json) continue ;;
            .claude/*.local.*)         continue ;;
        esac
        _is_scaffold_path "$f" || return 0
    done < <(git -C "$wt_dir" -c core.quotePath=false diff --name-only -z origin/main...HEAD 2>/dev/null)
    return 1
```

Trois points de forme, chacun déjà payé ailleurs dans ce fichier :

- `-c core.quotePath=false` + `-z` sur **les deux** diffs, pour la raison écrite
  l. 12153-12161 : sous le défaut de git, tout chemin non-ASCII revient entre
  guillemets avec des échappements octaux, ne correspond à aucun `case`, et
  tombe en « porte du travail » — un fail-open sur exactement ce que ce dépôt
  produit chaque jour, ses plans et ses tickets étant écrits en français.
- `grep -qxF -- … <<<"$_vs_main"`, here-string et jamais `printf | grep -q` :
  raison mika#2055, déjà écrite l. 12299-12304 — `grep -q` sort au premier
  match, ferme le tube, le producteur prend SIGPIPE et rend 141, et sous le
  `pipefail` dans lequel ce fichier est sourcé ce 141 devient le statut du
  pipeline.
- `tr '\0' '\n'` rend le set vulnérable à un chemin contenant un retour à la
  ligne : un tel chemin ne serait pas reconnu comme apporté, donc compté comme
  n'apportant rien. La direction est `Refs` plutôt que `Closes`, c'est-à-dire la
  polarité fail-closed que cette fonction déclare l. 12164-12171. On le dit au
  site plutôt que de l'ignorer.

Le `_vs_main` **vide** — parce que les arbres sont égaux, ou parce que
`origin/main` n'est pas là — fait rendre 1 à la fonction : « ne porte pas de
travail ». C'est déjà son comportement documenté sur une mesure impossible
(l. 12165-12171) et sur un diff vide, donc AC4 n'a rien à ajouter ici.

Le troisième appelant de cette fonction, `_measure_pipeline_verified` Term 1
(l. 12753), devient juste du même coup : un arbre égal à `main` rend
`rescue-pipeline-verified: no` avec `failed_term: diff`, au lieu de `yes` sur un
diff de squash.

### 2.5 AC3 — la branche supprimée par le merge n'est pas repoussée

Dans `_push_branch`, après le bloc `Stale-ref:` (l. 8956-8960) et avant le bloc
`if [ "$remote_has_branch" -eq 1 ]` :

```bash
    local _tree_rc=0
    _branch_tree_equals_main "$WORKTREE_DIR" || _tree_rc=$?
    if [ "$remote_known_absent" -eq 1 ] && [ "$_tree_rc" -eq 0 ]; then
        echo "push_branch.skipped_empty: HEAD tree equals origin/main and the remote has no branch $BRANCH — nothing to publish (mika#3038)" >&2
        RESULT="${RESULT}
Push: SKIPPED — the remote has no branch $BRANCH (its PR was merged, which deleted it) and the tree at HEAD is identical to origin/main; re-creating the branch would publish nothing (mika#3038)"
        return 0
    fi
    [ "$_tree_rc" -eq 2 ] && _note_tree_compare_undetermined
```

**Le prédicat distant est `remote_known_absent`, jamais `remote_has_branch`** —
l'absence **mesurée** par `ls-remote`, pas la décision de repli. Sur un
`ls-remote` qui n'a pas répondu, l'absence n'a pas été mesurée, et ce vol ne
doit pas s'autoriser à sauter un push sur un fait qu'il n'a pas. C'est
exactement la discipline que mika#2626 a déjà écrite dans cette fonction, pour
cette fonction, l. 9055-9060.

**Pourquoi la conjonction, et pas chaque terme seul.** Un arbre égal à `main`
avec la branche **présente** sur le distant ne passe pas par ici : le
court-circuit `ahead == 0` (l. 8981) rend 0 sans rien faire dans le cas
nominal, et s'il y a de l'avance en commits, c'est une branche vivante dont le
push est le comportement attendu. Une branche **absente** du distant qui apporte
du vrai travail est le témoin d'AC3, et elle doit être poussée : c'est le
premier push d'une branche neuve, le cas le plus courant de la boucle.

**Rien n'est perdu, et c'est démontrable, pas espéré.** Ce qui ne serait pas
poussé est un arbre dont chaque octet est dans `main`. Le worktree est
force-removed au prochain dispatch (`_set_up_worktree`), et cette perte est
réelle pour du contenu — c'est tout le sens de mika#1282 — mais ici il n'y a pas
de contenu : il n'y a qu'un graphe de commits dont le résultat est déjà publié.

### 2.6 AC4 — la ligne de l'indéterminé

```bash
_TREE_COMPARE_UNDETERMINED_NOTED=0
_note_tree_compare_undetermined() {
    [ "$_TREE_COMPARE_UNDETERMINED_NOTED" = "1" ] && return 0
    _TREE_COMPARE_UNDETERMINED_NOTED=1
    echo "tree_compare.undetermined: could not compare HEAD to origin/main — pre-mika#3038 behaviour in force" >&2
    RESULT="${RESULT}
Tree-probe: HEAD could not be compared to origin/main (ref absent or repository unreadable) — the pre-mika#3038 behaviour is in force (mika#3038)"
}
```

Une ligne, **exactement une**, quel que soit le nombre de sites qui ont consulté
le prédicat sur ce dispatch : le drapeau le garantit par construction, comme
`_set_pr_status_line` le garantit pour la ligne de statut. Elle va dans `RESULT`
et pas seulement sur stderr pour le fait (b) du § 1.3 — le stderr d'un dispatch
qui réussit n'est lu par personne, donc une ligne qui n'y vit que là n'existe
pas.

Et sur l'issue 2, **aucun** des deux refus ne s'arme et le diff reste mesuré
comme avant : la draft s'ouvre, le push a lieu, le marqueur se calcule. Le
comportement d'avant ce ticket, entier.

---

## 3. Ce que ce vol ne fait pas

- **Le merge pendant un vol vivant**, qui a déclenché #3034. Hors vol par le
  ticket, et à raison : c'est une discipline humaine.
- **Le feeder qui re-promeut `ready` après un merge** — mika#3036, mergé en
  `790c8959`. Ce vol traite le producteur de la draft, pas le lecteur qui en a
  tiré une conclusion sur #2194.
- **`_pr_list_url` sans `--state`** (l. 3287). Qu'une PR mergée de la branche ne
  compte pas est ce qui laisse `PR_URL` vide et arme la classe de secours. Y
  toucher changerait le routage de toutes les classes, pour tous les dispatches,
  et le défaut mesuré se ferme sans ça. Non fait, délibérément, et nommé ici
  pour que le prochain lecteur ne le prenne pas pour un oubli.
- **Aucune sonde livrée en script.** AC5 est un relevé, pas un livrable (§ 7).

---

## 4. Fire-Disposition

Ce vol livre **un** détecteur : le scan de câblage S1 de la nouvelle suite —
celui dont le chemin de succès est « aucune violation trouvée ». Les assertions
de comportement V1..V12 du § 5.1 n'en sont pas : leur chemin de succès est « le
comportement attendu s'est produit ».

Disposition retenue : **(a) exception nommée en allowlist**, avec une allowlist
**vide**, et c'est une mesure prise sur `790c8959b`, non un espoir.

| détecteur | violations avant le geste | après | allowlist |
|---|---|---|---|
| S1 — le prédicat est défini une fois et consulté aux deux sites de décision ; `_rescue_diff_carries_work` porte les deux bases de diff | 0 (ni le prédicat ni les sites n'existent encore) | 0 | `T3038_WIRING_ALLOWLIST=()` |

S1 scanne le texte de `dispatch-lib.sh` — le motif que
`test_rescue_scratch_not_published.sh` tient déjà en V14..V18 — et asserte des
**comptes**, jamais des présences :

- `_branch_tree_equals_main` apparaît exactement **4** fois : sa définition, sa
  consultation dans `_push_branch`, sa consultation au site du secours, et sa
  mention dans le doc-comment du prédicat ;
- `_note_tree_compare_undetermined` apparaît exactement **3** fois : définition
  et deux appels ;
- la région de `_rescue_diff_carries_work` contient **une** occurrence de
  `origin/main HEAD` et **une** de `origin/main...HEAD` — l'intersection, pas le
  remplacement d'une base par l'autre ;
- contrôle positif : le garde `declare -F _branch_tree_equals_main` du helper
  `verdict`, sans lequel une fonction **absente** fait sortir bash en 127, le
  `if` prend la branche `else`, et les assertions de refus passeraient sur un
  `dispatch-lib.sh` sans aucun correctif. L'argument est celui de
  `test_rescue_scratch_not_published.sh` l. 94-99, et il vaut mot pour mot ici :
  « un détecteur silencieusement inerte se lit exactement comme un détecteur en
  bonne santé ».

```bash
T3038_WIRING_ALLOWLIST=()
[ "${#T3038_WIRING_ALLOWLIST[@]}" -eq 0 ] || {
    echo "S1: table d'exceptions du scan de câblage — doit rester vide ; toute entrée nomme sa donnée et son ticket de suivi (mika#3038)" >&2
    exit 1
}
```

---

## 5. Acceptance criteria

Transcrits du corps de senara-solutions/mika#3038.

- **AC1.** Quand l'arbre de `HEAD` est égal à celui d'`origin/main`, aucune
  draft de secours ne s'ouvre, quelle que soit la classe. Le callback porte une
  seule ligne `NO_PR: <raison>`, avec une raison en snake_case distincte de
  `rescue_scratch_only`, inscrite là où le sont les raisons existantes. Le même
  test, dans le même appel, porte le contrôle négatif : une branche dont un
  merge précédent existe mais qui porte un vrai diff ouvre toujours sa draft (le
  cas de #3016).
- **AC2.** Le marqueur `rescue-diff` d'une draft qui s'ouvre se mesure sur ce
  que la branche apporte à `origin/main`, et non sur l'écart depuis la base de
  fusion. Un test le prouve avec un squash : les commits de la branche sont
  absents de `main`, mais leur contenu y est.
- **AC3.** Une branche que le merge de sa PR a supprimée d'`origin` n'y est pas
  repoussée quand elle n'apporte rien à `origin/main`. Un test par cas, avec le
  témoin d'une branche qui apporte du travail et qui est poussée.
- **AC4.** Fail-closed : si la comparaison d'arbres ne peut pas se mesurer
  (`origin/main` absent, dépôt cassé), le comportement d'avant ce ticket reste en
  place, et le résultat le dit en une ligne.
- **AC5.** Sonde : on relève, sur les 5 vols suivants terminés après le merge
  d'une PR de leur branche, les drafts de secours ouvertes à arbre égal à `main`,
  et on l'écrit ici. La cible est 0.

### 5.1 Comment chacun est tenu

Suite nouvelle : `skills/bundled/_shared/tests/test_rescue_tree_equals_main.sh`.
Elle **appelle les vraies fonctions** contre de vrais dépôts git temporaires,
jamais une réimplémentation — la critique que la suite mika#2503 porte sur ses
aînées (`test_auto_rescue_*`), qui réimplémentent le secours et ne peuvent donc
pas falsifier le code livré. Aucun réseau, aucun cargo : un `origin` est un
second dépôt local, et `origin/main` une ref posée par `update-ref` ou par un
`fetch` depuis ce dépôt-là.

| # | AC | assertion |
|---|---|---|
| V1 | AC1 | arbre égal à `main` → `_recovery_pr_due` éteint, aucun `gh pr create` dans l'argv du faux `gh` |
| V2 | AC1 | le même appel porte le témoin #3016 : branche avec un merge précédent **et** un vrai diff → la draft s'ouvre |
| V3 | AC1 | `RESULT` porte exactement une ligne `^NO_PR: `, et sa raison est `rescue_adds_nothing_to_main` |
| V4 | AC1 | la raison matche `(?m)^NO_PR:\s+([a-z_]+)` du consommateur, et diffère de `rescue_scratch_only` |
| V5 | AC1 | les trois classes : `dirty-worktree`, `no-shipping-tail`, `commit-pushed-no-pr` → refus sur chacune |
| V6 | AC2 | fixture **squash** : `main` reçoit un commit unique portant le contenu de la branche, la branche garde ses trois commits → `_rescue_diff_carries_work` rend « ne porte pas de travail », et le corps composé porte `rescue-diff: incident-only` + `Refs #N` |
| V7 | AC2 | squash **partiel** : contenu mergé + un fichier neuf → `carries-work` + `Closes #N`, et le diff trois-points nu en aurait dit autant sur un faux motif |
| V8 | AC2 | branche **en retard** sur `main` sans rien apporter → ne porte pas de travail (le fail-open que le deux-points nu aurait créé) |
| V9 | AC3 | `origin` sans la branche + arbre égal à `main` → aucun `push` dans l'argv du faux `git`, `RESULT` porte `Push: SKIPPED` |
| V10 | AC3 | témoin : `origin` sans la branche + vrai travail → la branche est poussée |
| V11 | AC4 | aucun `origin/main` → `_branch_tree_equals_main` rend 2, la draft s'ouvre, le push a lieu, `RESULT` porte **une** ligne `Tree-probe:` |
| V12 | AC4 | `wt_dir` vide et répertoire non-git → rc 2, jamais 0 ; et le CWD du processus n'a pas été mesuré |
| S1 | — | le scan de câblage du § 4 |

V5, V7, V8 et V10 ne sont pas du remplissage : sans eux, un prédicat qui
refuserait **tout** passerait V1, V3, V6 et V9 et désactiverait la boucle en
silence. C'est l'asymétrie que la suite mika#2503 énonce l. 14-23, et elle est
la même ici — à ceci près qu'ici le faux positif est moins coûteux : refuser une
draft dont l'arbre est celui de `main` ne perd aucun contenu, alors que refuser
un contenu non commité le perd irréversiblement. Le témoin existe donc pour la
**boucle**, non pour le contenu.

---

## 6. Definition of Done

- `bash skills/bundled/_shared/tests/test_rescue_tree_equals_main.sh` : vert,
  12 assertions de comportement + S1.
- `make test-dispatch-lib` : vert (aucune régression des suites voisines —
  `test_rescue_closes_guard.sh`, `test_rescue_pipeline_verified.sh`,
  `test_rescue_scratch_not_published.sh`, `test_push_with_rebase_retry.sh` lisent
  les trois fonctions modifiées).
- `cargo test -p mika-agent --lib -- parse_no_pr_reason` : vert.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
  `make verify-bundled-skills`, `bash scripts/check-suite-wiring.sh`,
  `scripts/verify-pipeline.sh origin/main` : verts.
- La suite est **atteinte par un workflow**, sans quoi `suite-wiring-lint`
  refuse : un step dans le job `dispatch-lib-offline-suites` de `ci.yml`
  (voisin immédiat de `test_rescue_scratch_not_published.sh`, l. 745-748), et un
  target `test-rescue-tree-equals-main` dans le `Makefile` pour la lancer seule.
  Pas d'entrée dans `scripts/suite-wiring.tsv` : le registre ne porte que le
  résidu, et cette suite tourne hors-ligne et est câblée.
- Un fichier sous `docs/solutions/dev-loop/` porte l'apprentissage — faute de
  quoi le Term 0 de `_measure_pipeline_verified` (traversée compound, mika#2563)
  refusera d'attester le pipeline sur cette PR même.

---

## 7. La sonde AC5, et comment elle se ferme

Relevé, pas livrable : aucun script n'est ajouté par ce vol. Sur les **5
premiers vols** terminés après le merge d'une PR de leur branche, postérieurs au
deploy du correctif, on compte les drafts de secours dont l'arbre d'ouverture
égale celui d'`origin/main`. Cible : **0**.

Méthode, celle de la mesure du § 1.1 appliquée à la fenêtre d'après :

1. `gh pr list --repo senara-solutions/mika --search "rescue-diff in:body" --state all --json number,createdAt,headRefOid --limit 50`
2. pour chaque PR ouverte après le deploy : comparer
   `git rev-parse <headRefOid>^{tree}` à l'arbre du `main` de l'instant
   d'ouverture (`git rev-list -1 --before=<createdAt> origin/main`, puis
   `^{tree}`) ;
3. le relevé — les 5 vols, leur date, le verdict d'égalité — est posté en
   commentaire sur mika#3038, et c'est ce commentaire qui ferme AC5.

Un relevé qui trouverait **un** cas n'est pas un échec du prédicat avant d'avoir
été lu : la première question est si `RESULT` portait la ligne `Tree-probe:`,
c'est-à-dire si la comparaison avait pu se faire. C'est à ça que sert AC4 : il
rend l'échec de mesure distinguable d'un prédicat faux, sur l'artefact durable.

---

## 8. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `_branch_tree_equals_main` + `_fetch_main_ref_once` + `_note_tree_compare_undetermined` (dispatch-lib.sh, commentaires inclus) | 55 |
| AC1 — second refus au site du secours (dispatch-lib.sh) | 22 |
| AC2 — intersection dans `_rescue_diff_carries_work` + réécriture de sa justification de base (dispatch-lib.sh) | 30 |
| AC3 — saut de push dans `_push_branch` (dispatch-lib.sh) | 26 |
| token inscrit dans `dispatcher.rs` (doc-comment + table du test) | 4 |
| tests (`skills/bundled/_shared/tests/test_rescue_tree_equals_main.sh`) | 250 |
| câblage (`ci.yml` 3 lignes, `Makefile` 3 lignes) | 6 |

Total estimé : 393 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 393 sous 429, et sous les 1000 par
PR. Le découpage en vols a été envisagé et écarté sur un argument de substance :
livrer le prédicat sans ses consommateurs serait livrer un détecteur **mute** —
du code qu'aucun chemin n'atteint, qui passe revue et CI par construction, et
dont la panne est indistinguable de la santé. C'est la classe que
`suite-wiring-lint` existe pour refuser (mika#2578, mika#2752). Et livrer les
consommateurs séparément les ferait atterrir sans le prédicat : ils ne
compileraient même pas en bash, ils rendraient 127.

Le poste dominant est la suite de tests (250 des 393, soit 64 %). Son voisin le
plus proche en forme, `test_rescue_scratch_not_published.sh`, fait 547 lignes
pour 19 assertions et trois étages de fixtures ; la présente en porte 13 et
partage une seule fabrique de dépôt (`make_repo` + une variante squash), ce qui
est la base de l'estimation. Les quatre postes de `dispatch-lib.sh` totalisent
133 lignes pour **quatre** éditions localisées, dans un fichier dont la densité
de commentaire est d'environ trois pour un : c'est ce ratio qui est estimé, pas
le code nu.

---

## 9. Ordre de commit

Chaque commit laisse l'arbre vert ; le plancher mergeable est le dernier.

1. **`feat(3038)` — le prédicat et ses deux helpers.** `_branch_tree_equals_main`,
   `_fetch_main_ref_once`, `_note_tree_compare_undetermined`. Aucun consommateur
   encore : à ce commit le prédicat est délibérément mute, et c'est la seule
   raison pour laquelle il n'est pas un vol séparé (§ 8).
2. **`fix(3038)` — AC1.** Le second refus au site du secours, + le token inscrit
   dans `dispatcher.rs` (doc-comment et table du test).
3. **`fix(3038)` — AC2.** L'intersection dans `_rescue_diff_carries_work`, avec
   la réécriture de sa justification de base.
4. **`fix(3038)` — AC3.** Le saut de push dans `_push_branch`.
5. **`test(3038)` — la suite + son câblage.** Les 12 assertions, S1, le step
   `ci.yml` et le target `Makefile`. Le détecteur cesse d'être mute ici.
6. **`docs(3038)` — l'apprentissage** sous `docs/solutions/dev-loop/` : « un
   secours qui ne demande pas ce que la branche apporte publie l'arbre de main »
   — le squash, l'illusion du trois-points, et pourquoi l'intersection plutôt
   que le deux-points nu.
