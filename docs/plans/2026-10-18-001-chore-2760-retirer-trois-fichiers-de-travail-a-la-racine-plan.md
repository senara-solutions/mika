# mika#2760 — Retirer les trois fichiers de travail de pilote suivis à la racine, et fermer les deux chemins qui les y ont fait entrer

> Plan content-only produit par `/mika-groom-plan-only`. Ticket : senara-solutions/mika#2760.
> Labels portés : `bug`, `p3-nice-to-have`, `ready`, `dispatch:loop`, `loop-substrate`.

---

## 1. Contexte : trois résidus, et la question de savoir ce qui les a produits

### 1.1 L'état mesuré dans ce worktree, le 2026-10-18

`git ls-files | grep -v '/' | sort` rend **30** entrées. Trois ne sont pas du
dépôt :

| fichier | lignes | entré par |
|---|---|---|
| `groom-brief-mika-789-pass2.md` | 127 | `4dcb2c65` (2026-05-19), squash de #1212 — commité par le pilote dans sa propre PR |
| `followup-issue-body.md` | 70 | `b99c6219` → squash de #2374 (2026-09-18), commit `wip(mika#2317)` d'un site de récupération |
| `followup-issue.md` | 59 | `50e98793` → squash de #2386 (2026-09-18), commit `wip(mika#2309)` d'un site de récupération |

**Personne ne les lit.** `grep -rln "followup-issue"` hors `.git/` ne rend rien ;
`grep -rln "groom-brief"` rend cinq fichiers, dont **aucun** ne nomme
`groom-brief-mika-789-pass2.md` — ce sont les prescriptions de brief de
`.claude/commands/` (§ 1.3), qui pointent toutes vers `/tmp`.

Les deux brouillons de suite ont été déposés comme tickets le 2026-10-17 :
mika#2758 (mergée depuis, `ca4be133`) et mika#2759. Rien n'est perdu.

### 1.2 Pourquoi le ticket distingue deux chemins, et pourquoi la distinction tient

**Chemin 1 — la récupération post-vol.** `dispatch-lib.sh` stage *tout* ce que le
pilote a laissé, à cinq endroits, et la seule exclusion partagée est
`RESCUE_EXCLUDE_PATHSPEC` (`dispatch-lib.sh:5963`) :

| site | ligne | gesture |
|---|---|---|
| `_fmt_and_stage_rust` (re-add post-`cargo fmt`) | 6001 | `git add -u -- "${RESCUE_EXCLUDE_PATHSPEC[@]}"` |
| `_rescue_dirty_worktree` (mika#1282) | 6284 | `git add -A -- …` |
| `_rescue_dirty_worktree`, reprise post-fmt (mika#1296) | 6406 | `git add -A -- …` |
| contenu traînant après `end_turn` (mika#1383) | 6894 | `git add -A -- …` |
| `_rescue_verify_pipeline`, terme 2 (mika#2354) | 10911 | `git status --porcelain -- …` |

Le tableau ne porte que de l'échafaudage `.claude/`. Un `followup-issue.md` à la
racine du worktree passe donc — et c'est littéralement ce que disent les deux
messages de commit cités au § 1.1.

**Chemin 2 — le pilote lui-même, dans sa PR.** Mai 2026 : aucune exclusion de
récupération n'aurait arrêté `groom-brief-mika-789-pass2.md`, puisque le pilote
l'a `git add`é et commité de sa main. La revue QA ne l'a pas vu.

Les deux chemins demandent deux gardes différentes, et c'est l'architecture de ce
vol : D2 ferme le chemin 1 *à la source*, D4 ferme le chemin 2 *au bord*.

### 1.3 Un troisième fait, trouvé au grooming : la prescription qui a produit le brief

Les commandes d'orchestration prescrivent au pilote d'écrire son brief sous
`/tmp` :

```
.claude/commands/mika-groom-ticket.md:194,197,216,219   → /tmp/groom-brief-<repo>-<n>-pass{1,2}.md
.claude/commands/mika-groom-milestone.md:126,130,155,165 → /tmp/groom-brief-<repo>-milestone-<N>-pass{1,2}.md
.claude/commands/mika-ask.md:67-68                       → @/tmp/groom-brief.md (exemple)
```

Or la permission-policy de la sandbox **refuse** toute écriture hors worktree
(`[policy:deny] Write: /tmp/…`, règle de dispatch mika#2211 ; depuis mika#2548 le
brouillon vit sous `.pilot-scratch/`). Un pilote qui suit la prescription reçoit
un refus, et replie son brief là où il peut — à la racine du worktree. C'est la
cause de production du fichier de mai.

**Ce vol ne peut pas la corriger.** `git ls-files .claude/commands/` ne rend que
quatre fichiers ici (`mika.md`, `mika-doc-audit.md`, `mika-issue.md`,
`mika-issues.md`) ; les trois prescripteurs sont des commandes du méta-dépôt,
seedées dans le worktree par `_seed_worktree_slash_commands` et exclues de
`git status` (mika#1415). Un ticket ouvert sur ce dépôt ne peut pas les éditer —
c'est exactement ce que dit `CLAUDE.md § .claude/commands/`.

**Tracked in: senara-solutions/mika-platform#207** — ouvert par ce grooming, avec
les trois fichiers, leurs numéros de ligne et la règle qui impose
`.pilot-scratch/`.

Ce que ce vol en retient : la cause reste vivante pendant un temps, donc le lint
de D4 n'est pas une ceinture de confort — c'est **la seule** garde qui arrête le
symptôme tant que mika-platform#207 n'a pas atterri.

---

## 2. Les livrables, site par site

### D1 — retirer les trois fichiers

```
git rm followup-issue.md followup-issue-body.md groom-brief-mika-789-pass2.md
```

Pas de `git rm --cached` : aucun de ces fichiers n'a d'usage local, et en laisser
une copie non suivie dans le worktree du vol ferait rougir le terme 2 de
`_rescue_verify_pipeline` à la fin du vol même.

### D2 — `RESCUE_EXCLUDE_PATHSPEC` apprend les deux formes de brouillon

`skills/bundled/_shared/dispatch-lib.sh:5963`, **une seule ligne modifiée** —
c'est la propriété que mika#2348 D2 a établie et que ce vol doit laisser
intacte : *une liste, tous les sites de staging.* Les cinq sites du § 1.2 héritent
du changement sans être touchés.

```bash
RESCUE_EXCLUDE_PATHSPEC=(':!.claude/commands/' ':!.claude/claude-pilot.json' ':!.claude/settings.local.json' ':!.claude/*.local.*' ':(exclude,glob)followup-issue*.md' ':(exclude,glob)groom-brief-*.md')
```

**La forme longue `:(exclude,glob)` est obligatoire, et ce n'est pas une
préférence de style.** Mesuré dans un dépôt de sonde le 2026-10-18 :

| pathspec | `followup-issue.md` (racine) | `sub/followup-issue.md` | `followup-issue-dir/x.md` |
|---|---|---|---|
| `:!followup-issue*.md` | exclu | **indexé** | **exclu** ← faux positif |
| `:(exclude,glob)followup-issue*.md` | exclu | **indexé** | **indexé** |

Sans magie `glob`, git applique `fnmatch` sans `FNM_PATHNAME` : le `*` traverse
les `/`, et `:!followup-issue*.md` avale un répertoire entier dont le nom
commence par `followup-issue`. Le ticket demande une exclusion **à la racine** ;
`glob` est ce qui la rend ancrée à un seul segment de chemin. Le mélange des deux
formes dans une même invocation a été mesuré au vert (la magie est par-pathspec,
pas globale), donc les quatre entrées historiques restent inchangées — les
toucher serait un diff sans mesure derrière lui.

**Effet de bord à nommer, parce qu'il est voulu.** Le cinquième site (ligne
10911, `_rescue_verify_pipeline` terme 2) lit le même tableau pour décider si le
worktree est propre. Après D2, un `followup-issue.md` résiduel ne fait plus lire
le worktree sale, donc ne disqualifie plus un rescue « pipeline-verified ». C'est
cohérent : un brouillon de corps de ticket n'est pas du contenu de pilote, et la
règle *« un chemin que la récupération refuse de stager ne doit pas faire lire le
worktree sale »* — écrite en commentaire à cette ligne — l'exige dans ce sens.

Le commentaire d'en-tête du tableau (lignes 5941-5962) gagne les deux entrées,
avec leur incident : les deux commits `wip(` du § 1.1, nommés par leur sha.

### D3 — `test-dispatch-lib.sh` : le test comportemental de l'AC2, et son contrôle négatif

Deux ajouts, à placer dans le bloc *« Auto-rescue excludes scaffold files »*
(lignes 1890-2173), à la suite du cas E.

**D3a — `test_auto_rescue_excludes_pilot_draft_bodies()`** (modelé sur
`test_auto_rescue_excludes_scaffold_files`, ligne 1897). Dépôt `git init`
temporaire avec `trap … RETURN`, puis :

| fichier posé | attendu |
|---|---|
| `followup-issue.md` | **non** indexé |
| `followup-issue-body.md` | **non** indexé |
| `groom-brief-mika-999-pass2.md` | **non** indexé |
| `src/new_feature.rs` | indexé |
| `docs/plans/2026-10-18-001-x-plan.md` | indexé |
| `sub/followup-issue.md` | indexé (l'exclusion est ancrée à la racine, § D2) |

**Le test exerce le tableau RÉEL, jamais un littéral recopié.** Les cas A, C et E
voisins codent leur pathspec en dur (`':!.claude/commands/'`, ligne 1921) : ils
testent leur propre copie, et resteraient verts si le tableau de `dispatch-lib.sh`
perdait l'entrée. Le nouveau cas charge la définition :

```bash
eval "$(grep -E '^RESCUE_EXCLUDE_PATHSPEC=\(' "$DISPATCH_LIB" | head -1)"
git -C "$test_dir" add -A -- "${RESCUE_EXCLUDE_PATHSPEC[@]}" 2>/dev/null
```

`eval` sur une ligne unique, auto-contenue, extraite par ancrage en colonne zéro :
le fichier entier n'est pas sourcé (cela exécuterait son setup top-level, ce que
l'en-tête de la suite interdit), et une seule ligne littérale n'a pas d'effet de
bord. C'est le même ancrage que la ligne 2165 utilise déjà pour son assertion
textuelle.

**D3b — le contrôle négatif, qui est ce qui donne sa valeur à D3a.** Le même
dépôt fixture, re-stagé avec le tableau **privé des deux nouvelles entrées** :
les trois brouillons doivent alors être **indexés**. Sans ce cas, D3a passerait
aussi si l'exclusion venait d'un `.gitignore`, d'un `core.excludesFile` de la
machine, ou de rien du tout — et la suite dirait vert sur un tableau vide.

**D3c — deux assertions structurelles**, ajoutées aux lignes 2168-2171 à côté des
deux existantes : `RESCUE_EXCLUDE_DEF` contient `:(exclude,glob)followup-issue*.md`
et `:(exclude,glob)groom-brief-*.md`. Elles tiennent la **forme** (`glob`, et non
`:!`), que D3a seul ne distingue pas : les deux formes excluent le fichier racine,
et seul `sub/followup-issue.md` les sépare — ce que D3a mesure, et ce que D3c
rend lisible dans un nom de test.

Le bloc de la phase A (lignes 3934-3943) n'est pas dupliqué : il assère que le
site mika#1383 **expand** le tableau, propriété inchangée par ce vol.

### D4 — `scripts/check-root-files.sh` : la racine est une liste déclarée, dans les deux sens

Le garde du chemin 2. Interface calquée sur
`scripts/check-pilot-turn-ceiling-labels.sh` (mika#2542), dont il reprend le
contrat de sortie :

```
check-root-files.sh [repo-dir] [declared-file]
```

Les deux arguments défaillent sur les vrais fichiers du dépôt ; les passer
explicitement est ce qui permet au harnais de D6 de pointer le garde sur des
fixtures.

**Côté A — ce que le dépôt suit à la racine.**

```bash
git -C "$repo" -c core.quotePath=false ls-files -z | tr '\0' '\n' | grep -v '/'
```

`ls-files`, pas `ls-tree HEAD` : c'est ce que le ticket nomme, et c'est aussi
l'index — donc un `.md` *stagé* par un pilote est déjà une violation, ce qui est
le bon moment pour le dire. `-z` + `core.quotePath=false` parce que la population
de ce dépôt écrit en français et qu'un nom accentué sous quoting par défaut
ressort échappé, donc jamais égal à sa ligne déclarée.

**Côté B — `scripts/root-files-declared.txt`** (D5). Une entrée par ligne, lignes
vides et `#` ignorés.

**La comparaison est bidirectionnelle**, comme
`check-dispatch-seats-declared.sh` (mika#2092), parce que les deux listes sont la
*même* vocabulaire — pas un sous-ensemble l'un de l'autre :

- fichier suivi à la racine, non déclaré → **exit 1**, message nommant le fichier
  et renvoyant à ce ticket ;
- entrée déclarée qui ne correspond à aucun fichier suivi → **exit 1**, message
  nommant l'entrée stale. C'est l'assertion auto-nettoyante du § Fire-Disposition :
  la liste ne peut pas accumuler de résidu, et un fichier racine légitimement
  supprimé doit l'être dans le même commit que sa ligne.

**Les deux façons dont un garde de ce genre passe pour la mauvaise raison → exit 3** :

- côté A vide — zéro fichier suivi à la racine, ou le répertoire n'est pas un
  dépôt git : un garde qui n'a rien lu ne doit pas dire « d'accord » ;
- côté B sans aucune entrée parseable (fichier vide, ou commentaires seuls) :
  l'ensemble vide refuserait tout, ou n'accepterait rien, selon le sens — dans les
  deux cas le garde ne mesure plus le dépôt.

**exit 2** quand `repo-dir` ou `declared-file` n'est pas lisible.

Comparaison par `comm` sur deux listes `LC_ALL=C sort`-ées — pas une double
boucle `grep -qxF`. La locale est fixée parce qu'un tri dépendant d'elle rend
`comm` faux sur les entrées en point (`.gitignore`, `.env.example`) selon la
machine, et le runner n'a pas la locale du poste de l'opérateur.

Sortie du chemin vert : le compte lu, non nul, comme le fait mika#2542
(`Pilot turn-ceiling labels are declared (1 ceiling label(s) checked: …)`).

### D5 — `scripts/root-files-declared.txt` : la liste, générée et non écrite

Le ticket l'exige : *« la liste doit être celle de `git ls-files` à la racine au
moment du vol, pas une liste écrite de mémoire »*. Donc, **après D1** :

```
git ls-files | grep -v '/' | LC_ALL=C sort
```

27 entrées attendues (30 − 3). Les 30 d'aujourd'hui, pour que l'écart soit
vérifiable à la relecture : `Cargo.lock`, `Cargo.toml`, `CHANGELOG.md`,
`CLAUDE.md`, `cliff.toml`, `clippy.toml`, `CONCEPTS.md`, `CONTRIBUTING.md`,
`deny.toml`, `docker-compose.yml`, `Dockerfile.agent`, `Dockerfile.gateway`,
`.dockerignore`, `.env.example`, ~~`followup-issue-body.md`~~,
~~`followup-issue.md`~~, `.gitattributes`, `.gitignore`,
~~`groom-brief-mika-789-pass2.md`~~, `install.sh`, `lefthook.yml`, `LICENSE`,
`Makefile`, `.node-version`, `package.json`, `package-lock.json`, `README.md`,
`release-please-config.json`, `.release-please-manifest.json`,
`rust-toolchain.toml`.

Le fichier porte un en-tête de commentaires disant : ce qu'il déclare, que le
garde est bidirectionnel, et qu'on **ajoute une ligne ici dans le même commit**
que le fichier racine — jamais après coup.

### D6 — `scripts/test-check-root-files.sh` : le contrôle négatif de l'AC3

Modelé sur `scripts/test-check-pilot-turn-ceiling-labels.sh` : un `FIXTURE_ROOT`
unique sous `mktemp -d`, `trap … EXIT`, helpers `assert_exit` /
`assert_output_contains`, et un helper `make_repo` qui `git init -q`, pose les
fichiers demandés et les `git add`.

| # | cas | attendu |
|---|---|---|
| 1 | **positif** : le dépôt réel + la liste réelle | 0, et la sortie annonce un compte **non nul** |
| 2 | **négatif AC3** : dépôt fixture dont la racine est exactement la liste réelle **plus** `NOTES.md` | 1, et le message nomme `NOTES.md` |
| 3 | non-`.md` non déclaré (`setup.cfg`) | 1 — la règle porte sur tout fichier suivi, pas sur les `.md` |
| 4 | les trois fichiers de mika#2760, remis dans un fixture | 1, et les trois sont nommés |
| 5 | entrée déclarée orpheline (`cliff.toml` déclaré, absent du dépôt) | 1, message nommant l'entrée stale |
| 6 | `sub/NOTES.md`, `docs/plans/x.md` : hors racine | 0 |
| 7 | liste déclarée vide | 3 |
| 8 | liste de commentaires seuls | 3 |
| 9 | dépôt sans aucun fichier suivi à la racine | 3 |
| 10 | `repo-dir` qui n'est pas un dépôt git | 3 |
| 11 | `repo-dir` / `declared-file` illisibles | 2 |
| 12 | nom accentué déclaré et suivi (`archivé.md`) | 0 — la population écrit en français |

Le cas 2 construit sa racine **depuis la liste réelle**, pas depuis une liste
inventée : un `root-files-declared.txt` vidé par accident ferait alors rougir le
cas 2 plutôt que de le laisser passer, et le cas 1 réclame déjà un compte non nul.

### D7 — câblage : `Makefile` puis `.github/workflows/ci.yml`

`Makefile`, exactement comme les deux lints voisins (lignes 317-323) :

```make
check-root-files: ## Reject a tracked file at the repo root that scripts/root-files-declared.txt does not declare + pin the guard's negative behaviour (mika#2760)
	@bash scripts/check-root-files.sh
	@bash scripts/test-check-root-files.sh
```

plus `check-root-files` sur la ligne `.PHONY` (ligne 4) et les deux `@bash` dans
le corps de la cible `test` (après les lignes 180-181, les deux du lint mika#2542).

`.github/workflows/ci.yml`, un job neuf inséré **après** `pilot-turn-ceiling-labels-lint`
(lignes 410-422), dont il reprend la forme à deux étapes :

```yaml
  root-files-lint:
    name: Root File Declaration Lint
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1  # v6
      # Un brouillon de pilote commité à la racine n'est lu par personne, n'est
      # référencé par rien, et chaque worktree l'embarque depuis main — trois
      # l'ont été, dont un pendant cinq mois (mika#2760). Aucune exclusion de
      # récupération n'arrête celui que le pilote commite lui-même.
      - name: Reject a tracked root file that root-files-declared.txt does not declare (mika#2760)
        run: bash scripts/check-root-files.sh
      # Un garde que personne n'a vu rougir est une décoration (mika#2103).
      - name: Pin the guard's negative behaviour
        run: bash scripts/test-check-root-files.sh
```

Le sha de `actions/checkout` est recopié des jobs voisins — toutes les actions de
ce dépôt sont épinglées par sha.

**Le hunk `.github/workflows/` n'est plus une inconnue.** mika#2752 l'a mesuré :
`8ac93d62` (2026-10-17) a poussé trois lignes de `ci.yml` depuis la sandbox et a
mergé. Le scope `workflow` du credential est acquis et exercé. Si ce push est
malgré tout refusé pour scope, c'est une régression du credential, pas du vol :
le résultat cite la ligne de refus verbatim et s'arrête là.

### D8 — la note de solution

`docs/solutions/best-practices/un-fichier-de-travail-de-pilote-suivi-a-la-racine-2026-10-18.md`,
avec frontmatter YAML (`module`, `tags`, `problem_type`) comme ses voisines. Ce
qu'elle porte, et qui ne vit nulle part ailleurs : les deux chemins d'entrée du
§ 1.2, la mesure `glob` vs `:!` du § D2, et le fait du § 1.3 (une prescription
`/tmp` que la policy refuse produit un fichier à la racine du worktree).

Hors comptage (`docs/`), et ce n'est pas une formalité : la garde
compound-traversal exige un `docs/solutions/**/*.md` dans `origin/main...HEAD` ou
un trailer `Compound: none — <raison>`, et ce vol a un apprentissage à écrire.

**`CLAUDE.md` n'est pas touché.** Son § CI/CD énumère certains lints, mais le
fichier est chargé en entier dans chaque prompt de pilote et
`crates/mika-common/tests/claude_md_size.rs` le borne ; la consigne du fichier
lui-même est de documenter ailleurs. La note de D8 est cet ailleurs.

---

## Fire-Disposition

Le vol livre **deux** détecteurs : `scripts/check-root-files.sh` (garde CI, chemin
de succès = « aucun fichier racine non déclaré ») et le test D3a/D3b de
`test-dispatch-lib.sh`. Le gate s'applique.

**Option retenue : (a) — exception nommée en allowlist — avec une allowlist de
zéro entrée, et le zéro est *produit* par le vol, pas constaté par chance.**

- **Violations existantes à l'atterrissage : zéro, par construction de l'ordre de
  commit.** Les trois seules violations du dépôt sont retirées au commit 1, et la
  liste déclarée de D5 est générée au commit 3, *après* ce retrait (§ 5). Le
  détecteur atterrit donc sur un dépôt déjà conforme. L'ordre est une contrainte
  de correction, pas de présentation : la liste générée avant D1 contiendrait les
  trois fichiers, et le garde serait vert sur exactement l'état qu'il existe pour
  refuser.
- **Aucune entrée d'exception n'est écrite, et aucune ne doit l'être.**
  `root-files-declared.txt` n'est pas une allowlist d'exceptions : c'est la
  déclaration du contenu légitime de la racine, la même nature que
  `.github/labels.yml` pour mika#2542. « On déclare, on n'allowliste pas »
  (mika#2201) : il n'y a pas de second fichier d'exemptions, et le remède à un
  run rouge est une ligne déclarée ou un fichier déplacé, jamais une exemption.
- **L'assertion auto-nettoyante est la bidirectionnalité elle-même (§ D4).** Une
  entrée déclarée qui ne correspond plus à aucun fichier suivi fait **exit 1** et
  est nommée. La liste ne peut donc pas devenir stale en silence : elle ne peut
  pas survivre à la suppression du fichier qu'elle déclare. C'est la propriété
  que mika#1574 demande d'une exception, appliquée ici aux 27 entrées.
- **Les deux détecteurs atterrissent ARMÉS.** Pas de `#[ignore]`, pas de
  `continue-on-error`, pas de `if:`, pas de `|| true`. Le job `root-files-lint`
  peut rougir le jour de son merge, et la PR du vol est son premier exercice
  réel : si D5 a généré une liste fausse, c'est la PR du vol qui devient rouge.
- **Ni le garde ni le test ne peuvent passer à vide.** D4 exit 3 sur chacune des
  deux vacuités (§ D4) ; D6 cas 1 réclame un compte non nul ; D3b exige que les
  brouillons soient indexés quand les deux entrées sont retirées du tableau.
- **Ce que le vol n'arme pas, et pourquoi ce n'est pas un désarmement.** La
  prescription `/tmp` du § 1.3 reste en place : elle vit dans un dépôt que ce
  ticket ne peut pas éditer (mika-platform#207). Le détecteur de D4 l'arrête au
  bord sans dépendre d'elle — c'est la raison d'être du chemin 2.

---

## 3. Ce que le vol ne fait PAS

- **Il ne touche aucune des quatre entrées historiques de
  `RESCUE_EXCLUDE_PATHSPEC`.** Chacune a son incident et son test ; les
  re-formuler en `glob` serait un diff sans mesure, sur le chemin le plus coûteux
  à réparer du dépôt.
- **Il ne généralise pas l'exclusion à `*.md` à la racine du worktree.** Un
  pilote écrit légitimement `pr-body.md` à la racine (règle mika#2211), déjà
  couvert par `.gitignore:90`, et un plan `docs/plans/…` n'est pas à la racine.
  Deux motifs nommés, tirés des trois incidents réels, valent mieux qu'une règle
  large dont le premier faux positif mangerait du contenu de pilote.
- **Il ne modifie pas `.gitignore`.** Un `.gitignore` n'arrête pas un `git add`
  explicite, donc il ne couvrirait pas le chemin 2 ; et il masquerait le fichier
  dans `git status`, c'est-à-dire exactement le signal que D4 veut voir.
- **Il ne corrige pas les prescriptions `/tmp`** (§ 1.3,
  mika-platform#207) : fichiers hors de ce dépôt.
- **Il ne réécrit pas les cas A/C/E voisins de `test-dispatch-lib.sh`** pour
  qu'ils exercent le tableau réel plutôt que leur littéral. C'est une faiblesse
  réelle, lisible au § D3, mais toucher cinq cas verts d'un garde de rescousse
  n'est pas le périmètre d'un ticket `p3` — et le nouveau cas pose le bon motif
  pour qui reprendra les autres.
- **Il ne touche pas `CLAUDE.md`** (§ D8).

---

## 4. Contrat de vérification

| # | affirmation | comment elle est tenue |
|---|---|---|
| V1 | les trois fichiers ne sont plus suivis | `git ls-files \| grep -v '/' \| wc -l` rend 27 ; `git ls-files followup-issue.md followup-issue-body.md groom-brief-mika-789-pass2.md` ne rend rien |
| V2 | un brouillon de corps de ticket laissé non commité n'est pas stagé par la récupération | D3a, exerçant le tableau réel de `dispatch-lib.sh` |
| V3 | V2 n'est pas vrai par accident | D3b — tableau privé des deux entrées ⇒ les trois brouillons SONT indexés |
| V4 | l'exclusion est ancrée à la racine | D3a : `sub/followup-issue.md` reste indexé ; D3c : la définition porte `:(exclude,glob)`, pas `:!` |
| V5 | les cinq sites de staging partagent la liste | inchangé par le vol, tenu par les assertions existantes (lignes 2167, 3940) ; D2 ne modifie qu'une ligne de définition |
| V6 | un `.md` nouveau à la racine fait rougir le lint | D6 cas 2, dont la racine fixture est construite depuis la liste réelle |
| V7 | le lint est vert sur le dépôt du vol | D6 cas 1 + le job `root-files-lint` sur la PR du vol |
| V8 | la liste déclarée est celle de `git ls-files`, pas une liste de mémoire | D5 : générée après D1 par la commande citée ; V7 rougirait sur tout écart |
| V9 | une entrée déclarée stale fait rougir | D6 cas 5 |
| V10 | le lint ne peut pas passer à vide | D6 cas 7-10, tous exit 3 ; cas 1 réclame un compte non nul |
| V11 | le lint est exécuté par CI | job `root-files-lint` dans `ci.yml`, deux étapes, aucune désarmée |
| V12 | la cible `make` existe et est `.PHONY` | `make -n check-root-files` se résout ; `grep -n '^\.PHONY' Makefile` contient `check-root-files` |
| V13 | la suite dispatch-lib reste verte dans son ensemble | `make test-dispatch-lib` — déjà câblée dans `ci.yml` (ligne 84) |
| V14 | le hunk `ci.yml` part | push du commit 4 ; le scope est attesté par `8ac93d62` (mika#2752) |

**Pré-contrôles locaux, avant la fin de pipeline** (chacun coûte moins d'une
seconde, et chacun attrape une faute qui a l'air juste) :

```
bash scripts/check-root-files.sh
bash scripts/test-check-root-files.sh
make test-dispatch-lib
make -n check-root-files
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"
```

`python3` + PyYAML sont présents sur cette machine (attesté par mika#2752) ; ni
`actionlint` ni `yamllint` ne le sont, et ce vol n'en installe aucun.

---

## 5. Ordre de commit — contraint, pas esthétique

**Commit 1 — `git rm` des trois fichiers.** Seul. C'est l'AC1, c'est trivialement
relisible, et c'est ce qui rend D5 générable. *Il doit précéder le commit 3 :* une
liste déclarée générée avant ce retrait inscrirait les trois fichiers et rendrait
le lint vert sur l'état même qu'il existe pour refuser.

**Commit 2 — `dispatch-lib.sh` + `test-dispatch-lib.sh` (D2, D3).** La garde du
chemin 1. Indépendante du commit 1 ; groupée avec son test parce qu'une ligne de
pathspec sans son test comportemental n'est pas vérifiable à la relecture.

**Commit 3 — `scripts/check-root-files.sh`, `scripts/root-files-declared.txt`,
`scripts/test-check-root-files.sh`, `Makefile` (D4, D5, D6, moitié de D7).** La
garde du chemin 2. `git ls-files` est lancé **ici**, après le commit 1.

**Commit 4 — `.github/workflows/ci.yml` seul (moitié de D7).** Isolé parce que le
push d'un hunk de workflow a sa propre histoire sur ce dépôt (§ D7) : un hunk
seul rend le diagnostic univoque si la forge refuse.

**Commit 5 — la note de solution (D8).**

**Puis la fin de pipeline** : les cinq pré-contrôles du § 4, `cargo fmt --check`,
`cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`,
`scripts/verify-pipeline.sh origin/main`, puis `gh pr create` avec le corps écrit
dans `pr-body.md` à la racine du worktree (règle mika#2211), supprimé après.

Note utile à qui verra la fin de pipeline rougir : **le diff ne touche aucun
crate.** Si `cargo clippy` ou `cargo test` rougit, la cause est dans `origin/main`
ou dans l'environnement du runner, jamais dans ce diff — et le vol ne doit pas
partir en chasse d'une régression Rust qu'il n'a pas pu produire. Les deux
commandes restent obligatoires : elles contrôlent l'état de la branche.

---

## 6. Risques

| # | risque | atténuation |
|---|---|---|
| R1 | `:(exclude,glob)` mal combiné avec les formes `:!` existantes ⇒ `git add` échoue, et le site de récupération meurt là où il est censé sauver du contenu | Le mélange a été **mesuré** au vert le 2026-10-18 (§ D2), et D3a l'exerce sur le tableau réel à chaque run de `make test-dispatch-lib`, câblée dans le job requis `check`. |
| R2 | L'exclusion est trop large et avale du contenu de pilote | Deux motifs nommés, tirés des trois incidents réels ; `glob` les ancre à la racine ; D3a pin `sub/followup-issue.md` et `docs/plans/…` du côté indexé. |
| R3 | La liste de D5 est écrite de mémoire et diverge du dépôt | Générée par la commande du § D5, après D1. Tout écart rougit sur la PR du vol (V7) — le garde est son propre premier contre-exemple. |
| R4 | Le lint rougit sur une PR légitime qui ajoute un fichier racine | C'est le comportement voulu : le remède est une ligne dans `root-files-declared.txt`, dans le même commit. Le message de refus le dit, et la note de D8 le documente. |
| R5 | Tri dépendant de la locale ⇒ `comm` faux sur les entrées en point selon la machine | `LC_ALL=C sort` des deux côtés (§ D4), fixé dans le script et non hérité de l'environnement. |
| R6 | Chemins accentués échappés par le quoting git ⇒ jamais égaux à leur ligne déclarée | `-c core.quotePath=false` + `ls-files -z` (§ D4) ; D6 cas 12 le pin. |
| R7 | `eval` de la ligne de définition casse si le tableau change de forme | L'extraction est ancrée en colonne zéro, comme la ligne 2165 existante ; une définition multi-lignes ou renommée rendrait une chaîne vide, le tableau serait vide, et D3a rougirait aussitôt (les brouillons seraient indexés). La forme qui casse est donc rouge, pas verte. |
| R8 | Dépassement du plafond de tours | § Taille estimée : 387 estimées pour une borne de 429, zéro ligne de Rust, deux suites shell sous la seconde. La soupape, si le réel déborde : s'en tenir aux 12 cas de D6 tels qu'énumérés, sans en ajouter. |
| R9 | Un fichier racine arrive sur `main` entre le grooming et le vol ⇒ 27 ≠ le compte réel | D5 regénère la liste au moment du vol ; le compte du § D5 est une attente de relecture, pas une constante. V1 compte 27 **après** D1 sur la base rebasée. |

---

## 7. Hors périmètre

- Les prescriptions `/tmp` des commandes de groom — **Tracked in:
  senara-solutions/mika-platform#207** (§ 1.3).
- Le reformatage des quatre entrées historiques du tableau (§ 3).
- La reprise des cas A/C/E de `test-dispatch-lib.sh` pour qu'ils exercent le
  tableau réel (§ 3).
- `.gitignore`, `CLAUDE.md` (§ 3, § D8).
- Le contenu des deux brouillons de suite : déjà déposé en mika#2758 (mergée) et
  mika#2759.

---

## Definition of Done

- [ ] `followup-issue.md`, `followup-issue-body.md` et
      `groom-brief-mika-789-pass2.md` ne sont plus suivis ; la racine compte 27
      fichiers suivis.
- [ ] `RESCUE_EXCLUDE_PATHSPEC` porte `:(exclude,glob)followup-issue*.md` et
      `:(exclude,glob)groom-brief-*.md`, et son commentaire d'en-tête nomme les
      deux incidents par leur sha.
- [ ] Les cinq sites de staging/status partagent toujours la liste — une seule
      ligne de définition modifiée, aucun site touché.
- [ ] `test-dispatch-lib.sh` porte D3a (comportemental, sur le tableau réel), D3b
      (contrôle négatif) et D3c (deux assertions de forme) ; `make
      test-dispatch-lib` passe.
- [ ] `scripts/check-root-files.sh` existe, est bidirectionnel, exit 3 sur
      chacune des deux vacuités, exit 2 sur chemin illisible, et annonce un
      compte non nul au vert.
- [ ] `scripts/root-files-declared.txt` a été **généré** par
      `git ls-files | grep -v '/' | LC_ALL=C sort` après le retrait des trois
      fichiers.
- [ ] `scripts/test-check-root-files.sh` porte les 12 cas du § D6, dont le
      contrôle négatif AC3 (cas 2) et l'entrée stale (cas 5).
- [ ] `Makefile` : cible `check-root-files`, entrée `.PHONY`, et les deux `@bash`
      dans le corps de `test`.
- [ ] `ci.yml` porte le job `root-files-lint` à deux étapes, inséré après
      `pilot-turn-ceiling-labels-lint`, sans `continue-on-error`, `if:` ni
      `|| true`, avec le sha d'`actions/checkout` recopié des jobs voisins.
- [ ] Les cinq pré-contrôles du § 4 sont passés localement avant la fin de
      pipeline.
- [ ] `docs/solutions/best-practices/un-fichier-de-travail-de-pilote-suivi-a-la-racine-2026-10-18.md`
      est écrite, avec frontmatter.
- [ ] `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo fmt --check` et `scripts/verify-pipeline.sh origin/main` passent.
- [ ] Le corps de PR porte `Tracked in: senara-solutions/mika-platform#207` et
      `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

Transcrits de la section « Critère d'acceptation » du corps de mika#2760, chacun
rattaché à ses livrables et à son terme de vérification.

- **AC1** — Les trois fichiers ne sont plus suivis. *(D1 ; V1)*
- **AC2** — Un brouillon `followup-issue.md` laissé non commité dans un worktree
  n'est pas commité par la récupération, et un test le tient. *(D2, D3 ; V2, V3,
  V4, V5)*
- **AC3** — Un `.md` nouveau à la racine fait rougir le lint, et le contrôle
  négatif vit dans la suite du lint. *(D4, D6 ; V6, V9, V10)*

Dérivés de la section « Travail attendu » du même corps :

- **AC4** — Les deux sites de récupération partagent le même tableau
  d'exclusion, et c'est ce qui reste vrai après le vol. *(D2 ; V5)*
- **AC5** — Le lint vit dans `scripts/` et est câblé dans `ci.yml`. *(D4, D7 ;
  V11, V12)*
- **AC6** — La liste déclarée est celle de `git ls-files` à la racine au moment
  du vol, pas une liste écrite de mémoire. *(D5 ; V8)*

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| D1 — retrait des trois fichiers (`git rm`) | 0 |
| D2 — `skills/bundled/_shared/dispatch-lib.sh` (1 ligne de définition + commentaire d'en-tête) | 20 |
| D3 — `skills/bundled/_shared/test-dispatch-lib.sh` (D3a + D3b + D3c + harnais d'appel) | 90 |
| D4 — `scripts/check-root-files.sh` | 100 |
| D5 — `scripts/root-files-declared.txt` (27 entrées + en-tête) | 33 |
| D6 — `scripts/test-check-root-files.sh` (12 cas + helpers) | 125 |
| D7 — `Makefile` (`.PHONY` + cible + 2 lignes dans `test`) | 5 |
| D7 — `.github/workflows/ci.yml` (job neuf, 2 étapes) | 14 |
| D8 — note de solution | hors comptage (`docs/`) |

Total estimé : 387 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 387 < 429 (marge ≈ 10 %) et
387 < 1000. Le ticket demande explicitement un seul vol, et les trois livrables
sont couplés par l'ordre de commit du § 5 : le lint de D4 ne peut pas atterrir
vert avant le retrait de D1, et le découper en deux PR ferait atterrir un garde
rouge ou une liste fausse.

Trois raisons concrètes au-delà du total :

- **Zéro ligne de Rust.** Aucun fichier du diff n'entre dans une unité de
  compilation, donc aucun cycle `cargo clippy` / `cargo test` d'itération — la
  source d'empilement de tours la plus coûteuse de ce dépôt. Les deux suites
  shell s'exécutent en moins d'une seconde chacune.
- **Deux des huit livrables sont du calque.** D4 et D6 reprennent l'interface et
  le contrat de sortie de `check-pilot-turn-ceiling-labels.sh` et de son harnais,
  lus pendant ce grooming ; D7 reprend la forme à deux étapes du job voisin. Il
  n'y a pas de forme à inventer.
- **La seule inconnue technique a été levée au grooming.** Le comportement de
  `:(exclude,glob)` contre `:!`, et le mélange des deux formes, sont mesurés dans
  le tableau du § D2 — c'est ce qui aurait coûté une boucle d'essais en vol.

Le poste le plus susceptible de déborder est D6 (12 cas). Soupape, si le réel
déborde : s'en tenir aux 12 cas tels qu'énumérés, sans en ajouter — chacun pin
une façon distincte dont le garde passerait pour la mauvaise raison, et aucun
n'est redondant.
