# Plan : faire dire la vérité au squelette, et le contrôle négatif qui le borne (mika#2108, vol 2)

**Ticket :** mika issue#2108 — `p1(dispatch-lib): le bac à sable ne monte que le worktree, mais les consignes citent les chemins de l'hôte — le pilote brûle ses tours à se chercher, puis meurt sur la forme large`
**Labels :** `bug`, `p1-important`, `ready`, `dispatch:loop`
**Type :** issue (bug — ralentit la boucle)
**Plans antérieurs :**
- `docs/plans/2026-09-03-001-fix-2108-orientation-du-pilote-dans-le-bac-a-sable-plan.md` — phases 1 à 4. Phase 1 livrée par #2791 (`8cf94749`).
- `docs/plans/2026-10-19-006-fix-2108-orienter-le-pilote-avant-le-corps-du-ticket-plan.md` — vol 1, livré par #2834 (`35a8b0ac`), branche supprimée.

Ce plan porte le **vol 2** : ce que #2834 nomme comme encore dû — les **phases 3 et 4** (faire
dire la vérité au squelette, contrôle négatif). Il re-mesure leurs prémisses avant tout.

---

## Ce que le re-mesurage a changé

Mesures exécutées le **2026-10-20** sur `origin/main` @ `059652e9`. Le plan du vol 1 datait
de la veille (`0d11df56`) ; celui de septembre du 2026-09-03 (`7b4ec10a`). Un jour sépare ce
plan du précédent, et cela a suffi à déplacer **deux** de ses prémisses — l'une par un merge
intervenu entre-temps, l'autre parce que ce qui était *déduit* est désormais *sondé*.

### D1 — Le câblage CI que ce vol devait porter est **déjà livré**

C'est la divergence qui commande le périmètre. Le vol 1 nommait, en P5, un troisième
livrable pour ce vol-ci : câbler `skills/bundled/_shared/tests/test_sandbox_git_usable.sh`
et retirer son entrée `unwired` de `scripts/suite-wiring.tsv` dans le même commit.

**`c209dd0f` (#2826, mika#2793) l'a fait.** Mesuré :

| surface | état mesuré aujourd'hui |
|---|---|
| `.github/workflows/ci.yml:219` | `bash skills/bundled/_shared/tests/test_sandbox_git_usable.sh` |
| `Makefile:267` | cible `test-sandbox-git-usable` |
| `Makefile:161` | la suite est aussi dans la cible agrégée |
| `scripts/suite-wiring.tsv` | 80 lignes ; **aucune** entrée pour cette suite |
| `bash scripts/check-suite-wiring.sh` | `|D|=105 |C|=103 |R|=2 **unwired=0**` |

Le vol 2 ne livre donc **aucun câblage CI**, et n'a **aucune entrée de registre** à retirer.
Un plan qui le prévoirait encore ferait écrire du code déjà présent, puis rougir la lint
bidirectionnelle sur une entrée réintroduite.

### D2 — Le squelette est **sondé**, et non plus déduit

Le vol 1 déclarait son P3 honnêtement incomplet : *« cette dérivation est lue sur le jeu de
binds, elle n'a pas été sondée dans un `bwrap` réel — la politique de permission refuse
`bwrap` depuis une session de grooming. La sonde réelle est un livrable du vol 2, pas une
prémisse acquise de celui-ci. »*

Elle n'a pas demandé de lancer `bwrap` : **cette session de grooming tourne elle-même dans
le bac à sable**. Un `ls` y est une sonde de première main. Exécuté aujourd'hui, depuis
l'intérieur :

```
$ ls -a /data/workspace/mika-platform
.  ..  .claude  claude-pilot  mika

$ ls -a /data/workspace/mika-platform/mika
.  ..  .git                       ← et rien d'autre

$ ls -a /data/workspace/mika-platform/claude-pilot
.  ..  src                        ← amputé, mais délibérément

$ ls -a /data/workspace/mika-platform/.claude/worktrees
.  ..  fix-2108-p1-dispatch-lib-le-bac-sable-ne-monte   ← le sien, seul

$ ls /data/workspace/mika-platform/mika/.git
config  info  logs  objects  packed-refs  refs  worktrees
```

Trois conclusions, chacune portant sur le périmètre :

1. **Le premier niveau rend trois entrées**, et c'est la liste que le vol 1 a mesurée côté
   hôte (`. .. .claude claude-pilot mika`). Le squelette est, à ce niveau, **indiscernable du
   workspace réel**. Le plan de septembre en mesurait deux : la dérivation du vol 1 était
   juste, et elle est maintenant confirmée plutôt que déduite.
2. **`mika/` ne porte que `.git`** — la forme exacte d'un dépôt dont le checkout aurait
   disparu, et le `.git` lui-même rend sept entrées parfaitement plausibles. C'est le
   menteur principal, et c'est ce que les deux sessions du premier commentaire (`2b74a8c6`,
   `9f144402`) ont lu avant de réclamer de « remonter le dépôt git backing du worktree ».
   Elles n'ont pas halluciné : elles ont lu ceci.
3. **`.claude/worktrees/` ne liste que le sien.** Cette réponse est *vraie* : elle ne trompe
   personne et ne reçoit donc aucun marqueur. La distinction est le périmètre même de ce
   vol — on corrige ce qui **répond faux**, pas ce qui est étroit.

### D3 — Les deux véhicules de test existent, et sont tous deux déjà câblés

Le vol 2 n'a pas de suite à créer. Il **étend** deux suites en place :

| suite | préconditions | câblage | ce qu'elle devient |
|---|---|---|---|
| `tests/test_pilot_orientation_block.sh` (294 l., vol 1) | aucune (PART D l'asserte) | `ci.yml:156` via `make test-pilot-orientation-block` | reçoit la part **structurelle** : symétrie des deux sites, dédup, silence |
| `tests/test_sandbox_git_usable.sh` (355 l., mika#2141) | `bwrap` réel + un root où ajouter un worktree | `ci.yml:219` | reçoit le **contrôle négatif réel** |

La seconde est le véhicule que le vol 1 avait désigné, et pour les raisons qu'elle énonce
elle-même : elle lance un **vrai `bwrap`** à travers le **vrai `_run_pilot_sandboxed`**
contre un **vrai worktree lié**, et déclare les assertions sur l'argv seul *hors contrat*.
Un argument ajouté à `dispatch-lib.sh` y est donc mesuré par son **effet**, pas par sa
présence dans une liste.

Elle porte déjà, en commentaire, la doctrine exacte dont ce vol a besoin :

> *« AC2 is about file access, not about path visibility, and a probe that confused the two
> would report a leak that is not one (or, worse, pass because the path it named never
> existed). »*

Et la même, côté `dispatch-lib.sh` (au-dessus de `_pilot_sandbox_enabled`) :

> *« A containment probe here must assert content unreachability, not path non-existence;
> asserting the latter reports a leak that is not one. »*

### D4 — `PLATFORM_DIR` et `SUB_REPO_DIR` ne sont **pas** définies dans `_run_pilot_sandboxed`

Contrainte neuve, absente des deux plans antérieurs, et elle commande la forme du livrable.

Les deux globales naissent dans `dispatch_claude_pilot` : `PLATFORM_DIR` au préambule
(auto-affectation avec défaut, mika#2536), `SUB_REPO_DIR` dans le branchement
`PLATFORM_REPO_NAME`. `_run_pilot_sandboxed` les voit *quand* elle est appelée depuis cette
chaîne — mais pas autrement, et il y a trois appelants autres : la suite mika#2141, la suite
mika#2572, le canari.

`tests/test_sandbox_git_usable.sh` tourne sous **`set -uo pipefail`** et appelle
`_run_pilot_sandboxed` directement sans définir ni l'une ni l'autre. Donc :

- une référence **nue** à `$PLATFORM_DIR` depuis la fonction d'arguments fait `unbound
  variable` et tue la suite ;
- un chemin **vide** passé à `bwrap` fait échouer le lancement — la classe que
  `_PILOT_GITDIR_BIND_ABORT` traite déjà avec trois gardes et un message nommé.

Le livrable doit donc **se taire proprement** sur globale absente, et ce silence est un
critère d'acceptation (AC4), pas une précaution.

### D5 — Le contrôle négatif réel ne sera **pas** mesurable en vol

`ci.yml:216-218` l'énonce, et c'est mika#2793 qui l'a établi :

> *« Preconditions: a real bwrap AND a root it can add a worktree to — it skips in a nested
> dispatch worktree, which is why its state had to be established on the runner rather than
> locally (mika#2793). »*

Un pilote d'implémentation tourne précisément dans un worktree de dispatche imbriqué.
`bwrap` **est** sur son PATH (`/usr/bin/bwrap`, mesuré depuis cette session), mais le
`git worktree add` de la sonde échoue et la suite sort en `⊘ skipped` à sa ligne 115.

**Le pilote verra un skip, jamais un vert.** Le plan l'énonce ici plutôt que de lui demander
une preuve locale qu'il ne peut pas produire — et la Definition of Done ne l'exige pas. La
vérification de cette moitié est **déléguée à la CI du PR**, qui est le seul lieu où elle a
lieu. Confondre un skip avec un succès est exactement ce que la suite mika#2141 a été écrite
pour empêcher ; le plan ne va pas le rejouer à son propre niveau.

### D6 — Rien ne garde la symétrie des deux sites `bwrap`

Le vol 1 nommait l'asymétrie comme « le défaut naturel de ce code » (P5). Mesuré
aujourd'hui : **aucun** script ne compare les deux blocs. `scripts/test-dispatch-symmetry.sh`
porte sur la symétrie des handlers `dev-pilot` / `dev-groom` (mika#893 R5), pas sur `bwrap`.

Et ce vol est justement celui qui ajoute un argument devant atterrir sur les deux. Les deux
blocs sont aujourd'hui identiques sur toute leur partie binds — vérifié ligne à ligne : de
`--ro-bind /usr /usr` à `--bind "$HOME/.mika/data/pilot-transcripts" …` puis
`${_PILOT_LOG_BIND_ARGS[@]}`, l'ordre et le contenu coïncident. Les seules divergences
légitimes sont en aval : `net_bwrap_args` / `net_setenv_args` (Phase 2b seule) et
l'entrypoint.

### D7 — Ancres : la fonction, jamais le numéro

`dispatch-lib.sh` : 5563 l. (09-03) → 6412 (09-09) → 12844 (10-19) → **13207** aujourd'hui.
Les ancres de ce plan nomment la **fonction** porteuse de l'invariant ; les numéros sont une
aide de navigation mesurée le 2026-10-20 sur `059652e9`.

| invariant | fonction | ligne au 10-20 |
|---|---|---|
| les deux sites `bwrap` | `_run_pilot_sandboxed` | 2872 (Phase 2b), 2935 (Phase 2a) |
| le canal `--ro-bind-data` et son fd | idem, bloc `secret_args` | 2841-2856 ; fermeture 2989-2992 |
| binds du gitdir, matérialisation des intermédiaires | `_pilot_gitdir_bind_args` | 2239 |
| texte d'orientation du vol 1 | `_pilot_orientation_rule` | 4281 |
| injection du vol 1 | `_set_up_worktree` | 5304 |
| les deux globales | `dispatch_claude_pilot` | 4817-4819, 12546 |

---

## La décision de conception

**Le squelette répond déjà ; il doit répondre juste — et son marqueur ne doit jamais pouvoir
tuer une dispatche.**

### Le mécanisme : `--ro-bind-data` depuis un descripteur, pas un fichier staged

Le fichier offre deux précédents, et le choix entre eux porte.

- **`--ro-bind` d'un fichier staged hôte** — c'est `_PILOT_GITCONFIG_HOST`
  (`$HOME/.mika/state/pilot-gitconfig`, mika#2141). Il donne une sonde opérateur : on peut
  `cat` le fichier hôte pour voir ce que le pilote lira. Mais une source `--ro-bind`
  manquante fait `bwrap: Can't find source path` — **un abort de lancement sans
  explication**, la classe que `_PILOT_GITDIR_BIND_ABORT` traite avec trois gardes et que
  `_PILOT_SANDBOX_REFUSAL` doit savoir nommer. Pour le gitconfig c'est le bon arbitrage :
  sans identité de committer le pilote ne livre rien, donc échouer fermé est juste.
- **`--ro-bind-data` depuis un fd** — c'est le canal de secrets (mika#2039). `bwrap` lit le
  contenu sur un descripteur et le matérialise comme fichier `0600` dans le namespace. Aucune
  source sur disque : **rien à stager, rien à garder, rien à nettoyer**, et rien qui puisse
  manquer.

**Retenu : `--ro-bind-data`.** Le raisonnement tient en une phrase : un fichier purement
informatif ne doit *jamais* pouvoir empêcher un pilote de démarrer. L'arbitrage du gitconfig
s'inverse ici, parce que l'enjeu s'inverse. La dépendance `bubblewrap >= 0.5.0` est déjà
payée par le canal de secrets — le commentaire de `_run_pilot_sandboxed` la nomme déjà, et
sa sonde ne détecte que l'*absence* de `bwrap`. Ce vol n'ajoute aucune exigence de version.

Le fd suit le motif en place et pour ses raisons déjà écrites : alloué par bash (`{var}<`,
donc ≥ 10, structurellement hors du fd 9 de `BASH_XTRACEFD`), et fermé **après** la capture
de `_sandbox_rc` pour que le statut de sortie du pilote reste celui de cette fonction. Le
contenu n'étant pas un secret, **la suppression de `xtrace` n'est pas nécessaire** — et
l'ajouter serait un ornement qui ferait croire à un secret là où il n'y en a pas.

### Les emplacements : deux, et la sonde dit lesquels

De D2 : on marque ce qui **répond faux**, pas ce qui est étroit.

| emplacement | pourquoi |
|---|---|
| `$PLATFORM_DIR/README-SANDBOX` | le premier niveau rend la même liste que l'hôte — c'est là qu'arrive un `ls` d'orientation |
| `$SUB_REPO_DIR/README-SANDBOX` | `<repo>/` ne porte que `.git` : le menteur principal, celui que `2b74a8c6` et `9f144402` ont lu |

**Écartés, mesure en main :**

- `claude-pilot/` — ne rend que `src`, mais ce bind est **volontairement étroit** et son
  contenu est utile et vrai. Personne n'y cherche son checkout. Un marqueur y serait du bruit.
- `.claude/worktrees/` — ne liste que le sien, et **c'est exact**. L'isolement voulu n'est
  pas un mensonge.

**Dédup obligatoire.** Quand la dispatche porte sur `mika-platform` lui-même,
`SUB_REPO_DIR` *est* `PLATFORM_DIR` (branchement `PLATFORM_REPO_NAME`) : les deux
`--ro-bind-data` nommeraient la même destination. `bwrap` applique en ordre et le second
recouvrirait le premier — pas fatal, mais c'est un argument qui ne dit rien, sur un chemin
qui est déjà couvert. Un seul est émis dans ce cas, et c'est asserté.

### Le texte : site unique, dérivé de `_pilot_orientation_rule`

Le contenu du `README-SANDBOX` est **la sortie de `_pilot_orientation_rule`**, précédée
d'une seule ligne de contexte qui dit pourquoi ce fichier est lu (« tu lis ceci parce que tu
as cherché un checkout ici »).

Deux raisons, et la seconde est la doctrine du fichier :

1. Les quatre faits que le bloc énonce sont exactement ceux qu'un lecteur du squelette a
   besoin d'entendre. Les réécrire produirait deux textes à maintenir.
2. **Le fichier applique déjà cette règle ailleurs** — mika#2544 fait du motif de détection
   `## Fire-Disposition` un site unique interpolé à ses deux lecteurs, « pour que les deux
   ne puissent pas diverger ». Même forme ici : un texte, deux canaux (le prompt, le
   namespace), aucune copie.

Corollaire mesurable : la racine hôte reste **dérivée** et jamais écrite en dur, puisque
c'est déjà la propriété de la fonction réutilisée (AC7 du vol 1, consigne « étendre par
propriété » de `scripts/check-pilot-instruction-paths.sh`). La garde de la phase 1 reste
verte sans effort.

### Le silence : la seule issue quand les globales manquent

De D4. La fonction d'arguments émet un tableau **vide** — et non un argument sur un chemin
vide — quand `PLATFORM_DIR` ou `SUB_REPO_DIR` est absente ou vide. Elle lit les deux en
`${VAR:-}`, jamais nues. Un pilote lancé hors de la chaîne `dispatch_claude_pilot` perd son
marqueur et **rien d'autre** : ni abort, ni `unbound variable`, ni message.

C'est le même arbitrage que ci-dessus, poussé jusqu'à son terme : ce fichier est de
l'information, et l'information ne refuse pas le service.

---

## Ce que ce vol livre

### U1 — `_pilot_sandbox_readme_args()` dans `dispatch-lib.sh`

Voisine de `_pilot_gitdir_bind_args` et `_pilot_log_bind_args`, dont elle prend la forme :
elle peuple un tableau global d'arguments `bwrap` et un tableau de fds à refermer, plutôt
que d'émettre sur `stdout` (un argument contenant un espace ne survit pas à une
substitution de commande non citée — raison pour laquelle les deux voisines sont déjà
écrites ainsi).

Elle fait quatre choses, pas plus :

1. lit `${PLATFORM_DIR:-}` et `${SUB_REPO_DIR:-}` ; si l'une est vide, elle **rend un
   tableau vide et retourne 0** ;
2. compose le contenu : une ligne de contexte + la sortie de
   `_pilot_orientation_rule "$WORKTREE_DIR" "$PLATFORM_DIR" "$SUB_REPO_DIR"` ;
3. ouvre un fd par destination et émet `--ro-bind-data <fd> <dest>` ;
4. **déduplique** quand `SUB_REPO_DIR` = `PLATFORM_DIR`.

Son commentaire d'en-tête nomme : le défaut visé (les quatre lignes de la sonde D2), le
choix `--ro-bind-data` *contre* `--ro-bind` et pourquoi l'arbitrage du gitconfig s'inverse,
le silence de D4 et sa raison, et le site unique du texte.

### U2 — L'injection, sur les **deux** sites

Un `${_PILOT_README_BIND_ARGS[@]+"…"}` dans chacun des deux blocs `bwrap`, à la même
position relative : **après** les binds du gitdir, dont il réutilise les répertoires
intermédiaires. La règle d'ordre est celle que le fichier documente déjà — `bwrap` applique
les montages dans l'ordre de l'argv, et un bind de répertoire posé par-dessus recouvrirait
un fichier matérialisé avant lui.

Les fds s'ajoutent à la boucle de fermeture existante, **après** la capture de
`_sandbox_rc`, pour la raison que mika#2039 y a déjà écrite.

Le commentaire d'accompagnement nomme l'invariant de symétrie et sa conséquence (D6) :
*un argument sur un seul site produit un bac à sable qui dit la vérité en Phase 2b et ment
en Phase 2a*, et c'est le détecteur U3 qui le retient.

### U3 — La part structurelle : PART E de `test_pilot_orientation_block.sh`

Dans la suite du vol 1 — offline, déjà câblée, même ticket, même classe. Cinq assertions et
leur anti-vacuité :

- **E1** l'argument est présent dans le bloc Phase 2b ;
- **E2** il est présent dans le bloc Phase 2a ;
- **E3** il y a **exactement deux** sites d'injection — un compte exact, parce qu'« au moins
  un » laisserait passer l'asymétrie que E1/E2 visent ;
- **E4** globales absentes → tableau vide, **et** aucun argument portant une destination
  vide (comportemental, la fonction est appelée) ;
- **E5** `SUB_REPO_DIR` = `PLATFORM_DIR` → une seule destination (comportemental) ;
- **anti-vacuité** : E1-E3 relancées sur une copie régressée où l'injection n'est que sur un
  site, qui doit **échouer**. La copie vit sous `.pilot-scratch/` (mika#2548), comme celle de
  PART C.

Aucune de ces assertions ne demande `bwrap`. PART D de la suite asserte déjà l'absence de
précondition et continue de valoir.

### U4 — Le contrôle négatif réel, dans `test_sandbox_git_usable.sh`

Le commentaire opérateur du 2026-09-03 pose la contrainte : *« Toute proposition d'élargir
les binds doit venir avec son contrôle négatif. »* Ce vol ajoute deux arguments de bind — il
y est donc tenu, et l'AC6 du vol 1 (« le diff n'ajoute aucune ligne portant `--bind`,
`--ro-bind`, `--ro-bind-try` ou `--ro-bind-data` ») est précisément ce qui transfère la
charge ici.

Les deux moitiés dans le **même run**, selon la doctrine que la suite porte déjà :

- **MUST WORK** — les deux `README-SANDBOX` sont lisibles depuis l'intérieur, et leur contenu
  nomme la racine de travail réelle. Une sonde qui ne montre que des absences ne distingue
  pas « correctement confiné » de « `bwrap` n'a pas démarré ».
- **MUST FAIL** — la contenance de mika#2141 est **inchangée**, re-mesurée après l'ajout : le
  working tree du checkout parent reste inaccessible, un autre worktree reste invisible par
  chemin et illisible par `git -C`, et la suppression d'un head étranger reste refusée. Les
  assertions existent déjà ; ce vol les fait porter sur le nouveau jeu d'arguments, et
  **n'en relâche aucune**.

La suite définit elle-même `PLATFORM_DIR` et `SUB_REPO_DIR` sur des chemins **fictifs sous
son `TMPROOT`** — jamais sur la racine hôte réelle. La propriété mesurée est « le marqueur
atterrit là où les globales le disent », ce qui est la propriété, pas l'orthographe.

**Rappel de D5 :** cette moitié **skip** dans un worktree de dispatche. Elle est vérifiée par
la CI du PR, et c'est tout ce que la Definition of Done peut exiger du pilote.

### U5 — Aucun câblage

Les deux suites sont déjà atteintes par `ci.yml` (lignes 156 et 219) et par la cible
agrégée. **Aucune ligne de `Makefile`, aucune ligne de `ci.yml`, aucune entrée de
`scripts/suite-wiring.tsv`.** C'est D1 : le travail a été fait par mika#2793, et le constater
est moins cher que de le refaire.

---

## Fire-Disposition

Ce vol livre des détecteurs : les six assertions de U3 et les assertions MUST WORK / MUST
FAIL de U4. Disposition retenue : **(a) exception nommée en allowlist — avec une population
d'exceptions vide à l'atterrissage.**

Le détail, parce que « vide » doit être mesuré et non supposé :

- **La population de violations à l'atterrissage est zéro, par construction de l'ordre de
  commit.** Les détecteurs gardent une propriété de U1+U2, qui atterrissent dans le même vol
  et **avant** eux. Il n'existe aucun site préexistant à exempter : les deux sites
  d'injection sont neufs, et il n'y en a que deux — ce que E3 asserte par un compte exact.
- **Aucune allowlist n'est donc créée.** En créer une vide serait un ornement, et
  `scripts/check-pilot-instruction-paths.sh` énonce la règle inverse : « l'exemption est la
  surface d'attaque, donc elle est étroite et nommée ». Zéro exemption est l'état le plus
  étroit possible, et c'est l'état d'atterrissage.
- **Ce qui remplace l'assertion auto-nettoyante,** dont l'objet est de rougir quand une
  exemption devient stale : il n'y a pas d'exemption à faire expirer, donc la charge se
  déplace sur la preuve que les détecteurs savent **dire non**. C'est l'anti-vacuité de U3
  (copie régressée à un seul site, échec exigé) et le contrôle positif de U4 (« le chemin du
  checkout parent *est* matérialisé, sinon les deux assertions suivantes sont vides » —
  l'assertion existe déjà dans la suite et ce vol s'y adosse).
- **Une nuance d'honnêteté, et elle n'est pas l'option (b).** La moitié `bwrap`-réelle de U4
  atterrit **armée**, mais elle est exécutée par la **CI**, pas par le pilote (D5). Ce n'est
  pas un détecteur désarmé : il n'y a ni `#[ignore]`, ni suivi d'armement, et il rougit sur
  le runner au premier push. C'est un détecteur dont le *lieu* d'exécution est la CI, ce qui
  est le cas de tous les gardes de ce dépôt — la seule différence est que le pilote ne peut
  pas le pré-vérifier localement, et le plan le dit plutôt que de le laisser découvrir.

Les détecteurs atterrissent donc **armés**, sans `#[ignore]`, sans suivi d'armement, et sans
halte opérateur.

---

## Acceptance criteria

**AC1 — Le squelette répond juste aux deux emplacements qui mentent.** Depuis l'intérieur du
bac à sable, `cat $PLATFORM_DIR/README-SANDBOX` et `cat $SUB_REPO_DIR/README-SANDBOX` rendent
chacun un texte qui nomme la racine de travail réelle en toutes lettres et déclare le
workspace hôte non monté. Mesuré dans un `bwrap` réel, à travers `_run_pilot_sandboxed`.

**AC2 — Le marqueur atterrit sur les DEUX sites `bwrap`, et c'est asserté comme une
symétrie.** Le compte de sites d'injection est **exactement deux**. Une injection présente
sur un seul site fait **échouer** la suite. L'assertion est sur le compte, pas sur une
présence : « au moins un » laisserait passer exactement le défaut visé (D6).

**AC3 — Aucun accès n'est ouvert, et c'est mesuré dans le même run.** Les assertions de
contenance de mika#2141 restent toutes vertes après l'ajout : working tree du checkout parent
inaccessible, autre worktree invisible par chemin et illisible par `git -C`, suppression d'un
head étranger refusée, un seul worktree listé sous `worktrees/`. Aucune n'est relâchée,
réécrite ni retirée. Le diff n'ajoute **que** des `--ro-bind-data` : aucun `--bind`, aucun
`--ro-bind`, aucun `--ro-bind-try` neuf.

**AC4 — Une globale absente n'émet rien, et jamais un chemin vide.** Avec `PLATFORM_DIR` ou
`SUB_REPO_DIR` vide ou non définie, `_pilot_sandbox_readme_args` rend un tableau vide,
retourne `0`, n'émet aucun argument portant une destination vide, et ne produit ni `unbound
variable` sous `set -u` ni message. Vérifié par appel de la fonction, pas par lecture du
source.

**AC5 — Le texte a un site unique.** Le contenu du `README-SANDBOX` est dérivé de
`_pilot_orientation_rule` et non recopié : il existe exactement une définition de ce texte
dans le dépôt. Corollaire mesurable : la racine hôte n'est écrite en dur nulle part, et
`make check-pilot-instruction-paths` reste vert.

**AC6 — Les détecteurs savent dire non.** Les assertions de symétrie de l'AC2, relancées sur
une copie régressée où l'injection n'est que sur un site, **échouent**. Une suite qui ne
démontre pas sa propre capacité d'échec ne compte pas comme livrée.

**AC7 — Aucun câblage n'est ajouté, et la lint bidirectionnelle reste verte.** Le diff ne
touche ni `Makefile`, ni `.github/workflows/ci.yml`, ni `scripts/suite-wiring.tsv` : les deux
suites étendues y sont déjà. `bash scripts/check-suite-wiring.sh` continue de rendre
`unwired=0`.

---

## Definition of Done

- `_pilot_sandbox_readme_args()` est sur la branche, et son injection est présente sur les
  **deux** blocs `bwrap`, après les binds du gitdir.
- `bash skills/bundled/_shared/tests/test_pilot_orientation_block.sh` rend `0` en local, ses
  **cinq** parties passent, et PART E porte son anti-vacuité.
- `bash skills/bundled/_shared/tests/test_sandbox_git_usable.sh` rend `0` en local. **Un
  `⊘ skipped` est le résultat attendu depuis un worktree de dispatche** (D5) : il ne vaut pas
  preuve, et la vérification de cette suite est la CI du PR. Le corps de PR le dit.
- `make check-pilot-instruction-paths`, `make check-suite-wiring`,
  `make test-dispatch-symmetry` et `make lint` restent verts.
- `scripts/verify-pipeline.sh origin/main` passe.
- Le corps de PR nomme le vol 2, ce que D1 a rendu inutile, et le fait que la moitié
  `bwrap`-réelle est vérifiée par la CI et non en vol.

---

## Ordre de commit

1. **U1** — la fonction seule. Rien ne l'appelle encore ; le dépôt est fonctionnel et aucun
   argument `bwrap` n'a changé.
2. **U2** — l'injection sur les deux sites et la fermeture des fds. C'est le commit qui
   change le namespace du pilote.
3. **U3** — PART E, verte dès son arrivée puisque U2 la précède.
4. **U4** — le contrôle négatif réel. En dernier parce qu'il mesure l'état final du jeu
   d'arguments.

Plancher du vol : U1 + U2 + U3 + U4. Retirer **U4** livrerait un élargissement d'arguments
`bwrap` sans son contrôle négatif — ce que le commentaire opérateur du 2026-09-03 interdit
explicitement, et ce que l'AC6 du vol 1 a transféré ici. Retirer **U3** livrerait l'asymétrie
non gardée de D6, dans un fichier qui a pris 7 644 lignes depuis le plan de septembre.

---

## Hors périmètre

- **Le câblage CI et le retrait de l'entrée `unwired`.** Livrés par mika#2793 (D1). Les
  refaire réintroduirait une entrée que la lint bidirectionnelle refuse.
- **Un garde de symétrie *générale* des deux blocs `bwrap`.** Tentant, et écarté avec sa
  raison : les deux blocs divergent **légitimement** en aval des binds
  (`net_bwrap_args`, `net_setenv_args`, l'entrypoint), donc un comparateur générique
  exigerait une allowlist de différences autorisées — c'est-à-dire l'allowlist par valeur que
  mika#2309 condamne, et qui serait pleine le jour où elle devrait mordre. U3 garde la
  symétrie de **cet** argument par un compte exact. Le garde général est un ticket à lui
  seul, pas un bonus de celui-ci.
- **Un marqueur dans `claude-pilot/` ou `.claude/worktrees/`.** Écartés mesure en main (D2) :
  le premier est un bind étroit dont le contenu est vrai, le second rend une réponse exacte.
- **Le canari `scripts/canary-pilot-containment`.** Sonde post-déploiement, lancée hors CI ;
  elle n'est pas un véhicule de ce vol et son extension n'est pas nommée comme due.
- **Élargir les binds.** Ce vol n'en élargit aucun : `--ro-bind-data` matérialise un contenu
  fourni sur un descripteur et ne donne accès à rien de l'hôte. La surface passe de *n*
  répertoires visibles à *n* répertoires visibles **plus deux fichiers de texte que nous
  écrivons nous-mêmes**. L'invariant de mika#2146 — autres worktrees, autres sous-dépôts,
  `~/.ssh`, `~/.config/gh`, `~/.gitconfig` inaccessibles — ne se relâche pas et n'est pas
  sollicité (AC3).
- **Le montage du gitdir.** Clos par mika#2146, et le commentaire opérateur du 2026-09-03
  interdit de le rouvrir.
- **Réécrire les chemins hôte du corps de ticket à l'injection.** Écarté par le vol 1, et la
  raison tient : pour #2108 les chemins hôte *sont* le sujet du ticket.
- **Les fichiers historiques de `docs/`.** Comptes rendus datés, non chargés automatiquement.
- **La létalité de la forme large** (cpp#128) et le garde-fou d'inactivité (cpp#147).

---

## Risques et limites, nommés

**Un fichier répond à un `ls` ; rien ne force à le lire.** Le vol 1 corrige le raisonnement,
celui-ci corrige le geste — et aucun des deux ne ferme la classe. C'est la même limite que
`DENIED_BASH_PATTERNS_HINT` porte dans sa propre note de clôture. Ce qui change avec ce vol
est qu'un pilote qui cherche **quand même** reçoit une réponse vraie au lieu d'une réponse
plausible et fausse, ce qui est strictement mieux que le silence et strictement mieux que le
squelette muet de D2.

**La moitié décisive n'est pas mesurable en vol.** D5. Le pilote verra `⊘ skipped` et devra
s'en satisfaire ; la CI tranche. Le risque réel est qu'un pilote lise ce skip comme un succès
et n'inspecte pas le rouge de la CI — le plan et le corps de PR le nomment pour cette raison.

**Deux arguments de plus dans un argv déjà long.** `/proc/<pid>/cmdline` est lisible par tout
le monde côté hôte, donc l'argv est une surface. Celle-ci ne porte qu'un numéro de descripteur
et deux chemins déjà présents ailleurs dans le même argv — aucune information neuve n'y
entre, et le contenu du fichier n'y figure pas (c'est tout l'objet de `--ro-bind-data`).

**La dédup de D-conception est un cas réel, pas théorique.** Une dispatche sur
`mika-platform` lui-même l'emprunte. E5 l'asserte parce qu'une destination dupliquée est un
argument qui ne dit rien, et parce que le jour où un bind de répertoire arrivera sur ce
chemin, l'ordre déciderait silencieusement lequel gagne.

**Ancres.** Re-mesurées aujourd'hui (D7), mais `dispatch-lib.sh` a doublé en un mois. Les
ancres de ce plan sont des **fonctions** ; les numéros valent pour `059652e9` et pour lui
seul.

**Ce plan ne prétend pas que le pilote livrera.** Il retire une friction du chemin ; il ne
mesure pas ce qui se trouve après elle.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` — `_pilot_sandbox_readme_args()` + sa justification | 62 |
| `skills/bundled/_shared/dispatch-lib.sh` — injection sur les deux sites + fds + commentaire de symétrie | 24 |
| `skills/bundled/_shared/tests/test_pilot_orientation_block.sh` — PART E (E1-E5 + anti-vacuité) | 62 |
| `skills/bundled/_shared/tests/test_sandbox_git_usable.sh` — contrôle négatif réel (MUST WORK + MUST FAIL re-mesurées) | 46 |
| `Makefile` + `.github/workflows/ci.yml` + `scripts/suite-wiring.tsv` | 0 |

Total estimé : 194 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

Le vol tient sous la borne, et voici pourquoi il tient plutôt que de simplement passer.

**La ligne à zéro est le gain du re-mesurage.** Le vol 1 prévoyait ici un câblage `Makefile`
+ `ci.yml` et le retrait d'une entrée de registre ; mika#2793 les a livrés (D1). Mieux : les
**deux** véhicules de test existent déjà, sont déjà câblés, et portent déjà leur harnais
d'assertions et leur doctrine de contenance (D3). Ce vol écrit des assertions dans des suites
en place, jamais une suite neuve — ce qui supprime la part la plus chère en tours d'un
livrable de test (l'échafaudage, le câblage, la vérification du câblage).

**Un seul mécanisme, dont le précédent est dans le même fichier.** `--ro-bind-data` + fd
alloué par bash est déjà écrit vingt lignes au-dessus du site d'injection, avec ses deux
propriétés porteuses commentées. Aucun schéma à migrer, aucun appelant à mettre à jour,
aucune dépendance de version neuve.

**Le texte n'est pas écrit, il est réutilisé.** Le contenu du marqueur est la sortie de
`_pilot_orientation_rule`, livrée par le vol 1 : une ligne de composition remplace un bloc
de texte à rédiger, et le site unique est une propriété au lieu d'une discipline.

**Le périmètre qui aurait fait déborder est sorti et nommé** : le garde de symétrie générale
des deux blocs `bwrap` (Hors périmètre, avec le raisonnement mika#2309 qui l'écarte d'ici) et
le canari. Rien n'est comprimé pour passer sous la borne — le travail retiré est du travail
qui n'appartient pas à ce vol.

Aucun vol ultérieur n'est dû au titre de mika#2108 : ce vol 2 épuise les phases 3 et 4, qui
étaient le reste nommé par #2834. Pas de ligne `Tracked in:`, donc, et c'est une clôture
plutôt qu'un oubli.
