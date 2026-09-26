# mika#2348 — les PR de rescue shippent du code non-formaté, et aucune des deux preuves ne vient du chemin que le ticket accuse

- **Ticket :** senara-solutions/mika#2348
- **Priorité :** p2 (substrate — chaque PR rescue échoue le CI Check et coûte un `cargo fmt` manuel)
- **Branche :** `feat/2348/rescue-auto-commit-mika-1282-wip-rescue`
- **Lignage :** mika#1282 (rescue dirty-worktree), mika#1336 (le `cargo fmt` proactif — déjà livré), mika#1383 (rescue trailing-content — le frère qui ne l'a pas reçu), mika#1685 (`--no-verify` sur les commits de rescue : salvage, pas gate), mika#1852 / mika#2199 / mika#2286 (`wip_rescue.rs`), le job CI « Check »

---

## Contexte

Deux PR produites par la boucle le 2026-09-16 ont échoué le job CI **Check**
(`cargo fmt --all -- --check`, `.github/workflows/ci.yml:70`) et ont demandé un
`cargo fmt` manuel à l'opérateur :

- **#2345** (bug/2286) → corrigé par `e2c7eef3`, « cargo fmt seul » sur `wip_rescue.rs`
- **#2344** (feat/2335) → corrigé par `d062f716`, « cargo fmt seul » sur `test_supersede_kills_live_pilot.rs`

Le ticket en tire une cause racine : *« Le chemin de rescue auto-commit (mika#1282,
`wip_rescue`) committe le contenu du pilote sans passer `cargo fmt` »*, et un remède :
*« exécuter `cargo fmt --all` sur le worktree AVANT le commit de rescue »*.

**Le défaut est réel, la classe est réelle, et le n=2 est réel.** Mais trois mesures
lisibles dans le dépôt seul déplacent la cause — et la différence décide du remède,
parce que le remède littéral du ticket porte sur un chemin qui exécute déjà
`cargo fmt` depuis huit mois.

---

## Ce qui est établi, et comment le vérifier

### E1 — Le chemin nommé par le ticket exécute DÉJÀ `cargo fmt --all`

`_rescue_dirty_worktree` (`skills/bundled/_shared/dispatch-lib.sh:3191-3206`), le
corps du rescue mika#1282, porte un bloc explicite :

```bash
# Proactive formatting (mika#1336): the dominant rescue-failure class is
# pilot-authored Rust that was never `cargo fmt`-ed [...]
if git -C "$WORKTREE_DIR" diff --cached --name-only 2>&9 | grep -q '\.rs$'; then
    PROACTIVE_FMT_ERR=$( (cd "$WORKTREE_DIR" && cargo fmt --all) 2>&1 ) || true
    git -C "$WORKTREE_DIR" add -u -- ':!.claude/commands/' ...
fi
```

Gaté sur `.rs` stagé, suivi d'un re-`add -u` avec le pathspec d'exclusion. Appliquer
la lettre du ticket à ce site produirait un second `cargo fmt` à côté du premier.

Vérification : `grep -n "Proactive formatting" skills/bundled/_shared/dispatch-lib.sh`.

### E2 — #2344 vient du chemin FRÈRE (mika#1383), qui n'a jamais reçu mika#1336

Le commit que `d062f716` a dû reformater est :

```
868e90e4  wip(mika#2335): trailing content after pilot end_turn (mika#1383)
```

C'est la **Phase A** du bloc mika#1383 (`dispatch-lib.sh:3512-3522`) — la population
« HEAD a avancé + contenu dirty résiduel », distincte de celle de mika#1282 (« HEAD
n'a pas bougé + dirty »). Son commit :

```bash
git -C "$WORKTREE_DIR" add -A -- ':!.claude/commands/' ... 2>&9 || true
if ! git -C "$WORKTREE_DIR" diff --cached --quiet 2>&9; then
    if git -C "$WORKTREE_DIR" commit -m "wip(...): trailing content after pilot end_turn (mika#1383)" --no-verify 2>&9; then
```

**Aucun `cargo fmt` entre le `add` et le `commit`.** mika#1336 a réparé un site et
laissé le second ouvert ; le commentaire de mika#1383 dit pourtant « Same exclusion
pattern as mika#1282 », ce qui est vrai du pathspec et faux du formatage.

Vérification : `git show --stat 868e90e4` puis `git log --format='%s' -1 d062f716`,
et lecture du bloc `Phase A` autour de `dispatch-lib.sh:3512`.

### E3 — #2345 ne vient d'AUCUN rescue : c'est un commit du pilote lui-même

Le commit que `e2c7eef3` a dû reformater est nommé dans son propre message
(« Le commit `0931d86c` n'était pas fmt-clean ») :

```
0931d86c  refactor(2286): pipeline_verified sans allocation, et le mot « parked » désambiguïsé
```

Pas de préfixe `wip(`, pas de mention de rescue : c'est un commit ordinaire écrit par
la session pilote. Le `wip(mika#1383): auto-PR-create rescue for mika#2286` qui le
précède (`69f109c2`) est un marqueur `--allow-empty`, sans contenu.

**Conséquence directe : un remède qui ne toucherait que les chemins de rescue
fermerait #2344 et laisserait #2345 entièrement ouvert.**

Vérification : `git log --oneline -6 0931d86c` ; `git show --stat 69f109c2`
(0 fichier).

### E4 — Le filet censé attraper E3 n'est pas installé, et ne l'est nulle part

`lefthook.yml:9-12` déclare la gate :

```yaml
rust-fmt:
  glob: "*.rs"
  run: cargo fmt --all -- --check
  stage_fixed: true
```

Sur la machine de dispatch :

| Sonde | Résultat |
|---|---|
| `git rev-parse --git-common-dir` | `/data/workspace/mika-platform/mika/.git` |
| `ls .../mika/.git/hooks/` | **`No such file or directory`** |
| `git config --list --show-origin \| grep hooksPath` | **aucune ligne, aucune portée** |

**Aucun hook pre-commit n'existe.** La gate `rust-fmt` ne s'est jamais exécutée sur
cette machine — ce qui explique E3 sans rien d'autre à invoquer, et ce qui rend
inexactes plusieurs pages de commentaires de `dispatch-lib.sh` qui raisonnent sur
« lefthook rejette le commit » (le bloc réactif `elif grep -q "rust-fmt\|cargo fmt"`
de `dispatch-lib.sh:3268` est, sur cette machine, structurellement inatteignable —
mika#1685 le notait déjà comme « effectively unreachable », pour une autre raison).

*Un gate déclaré dans un fichier versionné mais dépendant d'un geste machine
non versionné se lit exactement comme un gate qui marche.*

### E5 — `wip_rescue.rs` gate clippy et ne gate pas fmt

Le scan Rust qui reprend les drafts `wip-rescue` (rebase → clippy → push →
un-draft → qa-review) porte une clippy gate explicite
(`crates/mika-agent/src/wip_rescue.rs:1143-1148`) :

```rust
// Step 4: clippy gate. On errors, bail [...]
if let Err(e) = clippy(worktree).await {
    return PrepareOutcome::Bail("clippy-errors-need-human".to_string());
}
```

`grep -n "fmt" crates/mika-agent/src/wip_rescue.rs` ne rend **aucune** occurrence de
`cargo fmt`. C'est le dernier point de contrôle avant que la draft ne parte en
revue — il vérifie la moitié la plus coûteuse (clippy, compilation complète, 900 s
de budget) et laisse passer la moitié gratuite.

---

## Les trois trous, nommés

| # | Producteur du commit non-formaté | Preuve | Couvert aujourd'hui |
|---|---|---|---|
| **T1** | mika#1383 Phase A — trailing content | #2344 / `868e90e4` | non |
| **T2** | la session pilote elle-même | #2345 / `0931d86c` | non (E4 : aucun hook) |
| **T3** | draft reprise par `wip_rescue.rs` | — (gate absente, E5) | non |
| — | mika#1282 — dirty-worktree | — | **oui** (mika#1336) |

---

## Décisions

### D1 — Le remède ne peut pas vivre au niveau du commit

Trois emplacements possibles, deux écartés pour des raisons qui ne sont pas des
préférences :

**(a) `lefthook install` — écarté.** Trois coûts. (i) C'est un geste machine non
versionné : il ne survit ni à une nouvelle machine, ni à un `git clone`, et le
défaut se reposerait en silence — exactement la forme qui a produit E4. (ii) Il
ré-arme **aussi** `rust-clippy` en pre-commit, ce que mika#1685 a explicitement
refusé sur le chemin de rescue, mesure à l'appui : un nit clippy d'une ligne
rejetait le commit de rescue et abandonnait 29 tours de pilote (cause modale de
wedge, n≥3 au 2026-06-30, arbitrage Mika Prime du 2026-06-30 ~16:32Z). (iii) Il
gaterait le commit du pilote, c'est-à-dire déplacerait l'échec *dans* la session
au lieu de le corriger après.

**(b) Un `cargo fmt` ajouté au seul site mika#1383 — insuffisant.** C'est la lettre
du ticket corrigée par E2. Elle ferme T1 et laisse T2 entier, alors que T2 porte la
moitié des preuves.

**(c) Retenu — une normalisation post-flight, plus l'alignement du site T1.** Deux
gestes, parce qu'il y a deux natures de trou : un site de rescue qui stage sans
formater (T1, se répare *dans* le site), et des commits déjà écrits par un producteur
qu'on ne gate pas (T2, se répare *après* tous les producteurs).

### D2 — Un helper partagé, pas une troisième copie du bloc

`_fmt_and_stage_rust()` extrait le bloc mika#1336 de `_rescue_dirty_worktree` et est
appelé par les deux sites de rescue. Effet DRY joint, non cosmétique : le pathspec
d'exclusion à quatre entrées (`:!.claude/commands/`, `:!.claude/claude-pilot.json`,
`:!.claude/settings.local.json`, `:!.claude/*.local.*`) est aujourd'hui écrit
**quatre fois** dans le fichier, chaque entrée ajoutée par un incident distinct
(mika#1288, mika#1419, mika#1552). Une cinquième divergerait ; c'est précisément la
classe de dérive qui a produit T1.

### D3 — La normalisation post-flight ne reformate que le périmètre de la branche

`cargo fmt --all` opère sur tout le workspace. Comme aucun hook ne tourne (E4), il
est plausible que `main` porte déjà des fichiers non-fmt sans rapport avec la
branche ; les embarquer gonflerait le diff d'une PR de rescue, déjà fragile.

`_normalize_committed_rust()` intersecte donc les fichiers salis par le fmt avec le
diff de la branche (`git diff --name-only $(git merge-base origin/main HEAD)..HEAD`)
et **restaure les autres** (`git checkout --`). Coût nommé, assumé : un fichier
non-fmt sur `main` et non touché par la branche reste non-fmt. C'est le périmètre du
ticket, pas une régression — et le CI Check le dira sur la PR qui le touchera.

### D4 — Jamais bloquer la préservation sur le formatage

`cargo fmt` échoue sur du Rust syntaxiquement invalide, et un pilote interrompu en
plein fichier en produit. Dans ce cas : journaliser, **continuer**, commiter quand
même. Le rescue est du salvage, pas une gate (mika#1685) ; un `cargo fmt` qui échoue
laisse le CI rouge, c'est-à-dire l'état d'aujourd'hui — on ne perd rien, alors qu'un
`set -e` sur cette ligne perdrait le contenu du pilote.

Même règle côté `wip_rescue.rs` : un fmt qui échoue **ne bail pas**. La clippy gate
voisine attrape déjà le code cassé, avec un message qui nomme la bonne cause.

### D5 — Côté `wip_rescue.rs`, normaliser, ne pas bailer

Le fmt est mécanique et auto-réparable ; bailer-to-human dessus enverrait un humain
faire ce que la machine sait faire, et consommerait l'exclusion durable
`wip_rescue_bailed` (mika#2199) qui est terminale. La gate produit donc un commit
`style()` avant le push de l'étape 5, ou ne fait rien.

### D6 — Une garde structurelle, parce qu'une garde comportementale ne voit pas T1

La régression à empêcher n'est pas « une décision devient fausse » mais « un
producteur de commit n'est pas couvert ». Tous les tests de comportement restent
verts pendant qu'elle court — c'est littéralement ce qui s'est passé entre mika#1336
et aujourd'hui. Un scan statique de `dispatch-lib.sh` refuse donc un site
`git commit` **de contenu** qui ne soit pas précédé d'un passage par
`_fmt_and_stage_rust`. Exclus de la garde : les commits `--allow-empty` (marqueurs
mika#1383 auto-PR-create, `dispatch-lib.sh:6751`), qui ne portent aucun contenu.

Précédent de forme : le static guard `--no-verify` de
`test_rescue_commit_no_verify.sh`, et `auto_pull::tests::mika2131_exclusion_skips_never_return_to_an_uncollected_debug`.

### D7 — Les tests utilisent un vrai crate jetable, pas un stub `cargo`

Les suites `_shared/tests/` annoncent « No network / cargo / clippy required ». Ce
plan les fait dépendre de `cargo fmt` — délibérément : un stub qui simule `cargo fmt`
ne peut pas prouver la propriété demandée par le ticket (*« asserter que le commit de
rescue est fmt-clean »*), il prouverait seulement qu'on a appelé quelque chose.

Le coût est faible et il faut le dire pour qu'il ne soit pas confondu avec celui de
clippy : **`cargo fmt` ne compile pas**. Un `Cargo.toml` minimal et un `src/lib.rs`
de cinq lignes dans un `mktemp -d` rendent en dixièmes de seconde, sans réseau et
sans registre (aucune dépendance). Si `cargo` est absent du PATH, la suite **skip
avec un message explicite** plutôt que d'échouer — un test qui rougit sur une
machine sans toolchain apprend la mauvaise chose.

---

## Changements

### 1. `skills/bundled/_shared/dispatch-lib.sh`

**1a.** Extraire le pathspec d'exclusion en constante unique
(`RESCUE_EXCLUDE_PATHSPEC`), documentée par ses quatre incidents d'origine, et
remplacer les quatre littéraux.

**1b.** Nouveau `_fmt_and_stage_rust()` : corps du bloc mika#1336 (gate sur `.rs`
stagé → `cargo fmt --all` → re-`add -u` avec le pathspec). Fail-safe par D4.
Émet `rescue_fmt_failed` sur stderr si le fmt échoue.

**1c.** `_rescue_dirty_worktree` appelle `_fmt_and_stage_rust` à la place de son
bloc inline. **Zéro changement de comportement** — c'est le contrôle négatif du
refactor.

**1d.** Bloc mika#1383 Phase A (`~:3512`) : appeler `_fmt_and_stage_rust` entre le
`git add -A` et le `git commit`. **Ferme T1.**

**1e.** Nouveau `_normalize_committed_rust()` : `cargo fmt --all` → intersection
avec le diff de branche (D3) → restauration du hors-périmètre → si reste non vide,
commit `style(<repo>#<n>): normalisation cargo fmt post-flight (mika#2348)` en
`--no-verify`, puis `_record_rescue_commit` et avance de `POST_RUN_HEAD`. Émet
`rescue_fmt_normalized` sur stderr (champs : branche, nombre de fichiers).
**Ferme T2.**

**1f.** Site d'appel de `_normalize_committed_rust` : dans `_post_flight_recovery`,
après `_rescue_dirty_worktree` et **avant** le bloc mika#1383 — celui-ci pousse en
ligne (`git push origin "$BRANCH"`, `~:3527`, accroché à mika#2151), donc une
normalisation placée après lui laisserait un commit derrière le push. Placée avant,
le dirty qu'elle produit est soit commité par elle-même, soit — si elle a déjà
commité — sans effet sur la Phase A, qui trouve un tree propre.

*Point d'attention à vérifier à l'implémentation :* l'ordre exact des deux blocs
dans `_post_flight_recovery` conditionne quel site commite quoi. Le test T3
ci-dessous est écrit pour tomber si l'ordre est inversé.

### 2. `skills/bundled/_shared/tests/test_rescue_fmt_clean.sh` (nouveau)

Source `dispatch-lib.sh` et appelle les vraies fonctions contre un crate jetable
(D7), sur le modèle de `test_dev_groom_dirty_rescue.sh` — jamais une réimplémentation
du rescue dans le test, qui ne pourrait pas falsifier le code livré.

| Test | Situation | Assertion |
|---|---|---|
| T-a | worktree dirty, `.rs` non-formaté, HEAD inchangé | commit de rescue fmt-clean (**non-régression mika#1336**) |
| T-b | HEAD avancé + trailing dirty `.rs` non-formaté | commit trailing fmt-clean (**ferme #2344 / T1**) |
| T-c | pilote a commité du `.rs` non-formaté, tree propre | un commit `style()` est produit, et il est fmt-clean (**ferme #2345 / T2**) |
| T-d | tree propre, code déjà fmt-clean | **aucun** commit produit (le no-op — la moitié qu'on oublie) |
| T-e | `.rs` syntaxiquement invalide | le rescue aboutit, le contenu est préservé (**D4**) |
| T-f | fichier non-fmt hors diff de branche | il n'entre **pas** dans le commit (**D3**) |
| T-g | scan statique de `dispatch-lib.sh` | tout `git commit` de contenu passe par `_fmt_and_stage_rust` (**D6**) |

### 3. `crates/mika-agent/src/wip_rescue.rs`

Étape 4.5 dans `prepare`, entre la clippy gate et le push : `cargo fmt --all` sur le
worktree, puis commit `style()` si le tree a bougé. Ne bail jamais (D5). Journalise
`wip_rescue_fmt_normalized` (INFO) / `wip_rescue_fmt_failed` (WARN). Tests unitaires
du module pour les deux issues. **Ferme T3.**

### 4. `Makefile`

Ajouter `test_rescue_fmt_clean.sh` aux deux cibles qui listent les suites `_shared`
(`~:136-145` et `~:165`).

### 5. `mika/CLAUDE.md`

Une entrée de signaux opérateur (voir ci-dessous). Pas de nouvelle variable
d'environnement : ce correctif n'a rien à régler.

---

## Surfaces opérateur

Dans la sortie de dispatch (stderr du dispatch, capturée dans le `RESULT`) :

- **`rescue_fmt_normalized`** — un commit `style()` a été produit. **Régime attendu :
  non nul.** C'est la mesure directe de T2 : tant qu'aucun hook pre-commit n'existe
  (E4), des pilotes continueront de commiter du non-fmt et cette ligne le dira. Une
  décroissance vers zéro signifierait que les pilotes formatent d'eux-mêmes —
  information utile, pas une condition d'arrêt.
- **`rescue_fmt_failed`** — `cargo fmt` a échoué, le rescue a continué (D4). **Doit
  rester vide.** Toute occurrence est du Rust syntaxiquement invalide commité par un
  pilote interrompu ; le contenu est préservé et le CI dira le reste.

Dans `$MIKA_SPIRIT_LOG_FILE` : `wip_rescue_fmt_normalized` (INFO, une ligne par
draft normalisée) et `wip_rescue_fmt_failed` (WARN, doit rester vide).

---

## Sonde post-déploiement, et sa halte

Sur les **cinq prochaines PR de rescue** (`wip(` en tête, ou label `wip-rescue`) : le
job CI **Check** doit être vert sans intervention.

- Check **vert** + `rescue_fmt_normalized` présent → T2 fermé et mesuré.
- Check **rouge** + `rescue_fmt_normalized` **absent** → la normalisation ne tourne
  pas sur ce chemin. Lire le `RESULT` du dispatch : soit `cargo` est absent du PATH
  de la session de dispatch, soit un **quatrième** producteur de commit existe.
- Check **rouge** + `rescue_fmt_normalized` **présent** → **halte.** Le fmt tourne et
  quelque chose re-salit **après** lui. Ne pas ajouter un second appel de
  normalisation par réflexe : chercher le producteur qui commite en aval du site
  d'appel, et le nommer avant de toucher quoi que ce soit.

---

## Hors périmètre, délibérément

- **Installer lefthook** (`lefthook install`, ou un `core.hooksPath` versionné).
  E4 est une trouvaille réelle et mérite son propre ticket, mais le corriger ici
  ré-armerait `rust-clippy` en pre-commit et contredirait frontalement mika#1685 sur
  le chemin de rescue. Ce plan **documente** l'absence du hook et s'en rend
  indépendant ; il ne la répare pas.
- **Retirer `--no-verify`** des commits de rescue. Même raison, et mika#1685 a
  ratifié l'arbitrage, arbitrage Mika Prime à l'appui.
- **Nettoyer les commentaires de `dispatch-lib.sh` qui décrivent un hook qui ne
  tourne pas** (le bloc réactif `elif grep -q "rust-fmt\|cargo fmt"` de `~:3268`,
  entre autres). Leur inexactitude est établie par E4, mais les supprimer dans le
  même commit mélangerait une correction fonctionnelle et un débroussaillage —
  et ils redeviennent exacts le jour où le ticket ci-dessus aboutit.
- **Les autres gates du CI** (clippy, tests) sur les PR de rescue : mika#1685 a
  décidé qu'elles se jouent en CI, pas au commit. Ce plan ne touche que `fmt`.
- **Le secret-scan que `--no-verify` contourne** — c'est mika#1689.

---

## Definition of Done

1. `_fmt_and_stage_rust()` existe, est appelé par les deux sites de rescue, et le
   pathspec d'exclusion n'est plus écrit qu'une fois.
2. `_normalize_committed_rust()` existe, est appelé une fois dans
   `_post_flight_recovery` avant le push inline de mika#1383, et respecte D3/D4.
3. `wip_rescue.rs` normalise le fmt entre sa clippy gate et son push, sans bailer.
4. `test_rescue_fmt_clean.sh` passe (T-a à T-g), est câblé au `Makefile`, et skip
   proprement si `cargo` est absent.
5. `cargo test -p mika-agent` passe ; `cargo fmt --all -- --check` et
   `cargo clippy --all-targets --all-features -- -D warnings` passent sur la branche.
6. `make verify-bundled-skills` passe.
7. `CLAUDE.md` nomme les quatre signaux opérateur et leur régime attendu.

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria`. Les critères
ci-dessous sont dérivés de son « Fix attendu » et des trois trous établis.

- **AC1** — Un rescue simulé sur un fichier Rust non-formaté produit un commit de
  rescue fmt-clean : `cargo fmt --all -- --check` passe sur l'arbre commité.
  *(Formulation littérale du ticket ; couverte par T-a et T-b.)*
- **AC2** — La couverture vaut pour **les deux** chemins de rescue de
  `dispatch-lib.sh` — mika#1282 (dirty-worktree) et mika#1383 (trailing content) —
  et pas seulement pour celui que le ticket nomme. *(T-a, T-b ; ferme #2344.)*
- **AC3** — Un commit **du pilote lui-même** laissé non-formaté, sur un worktree
  propre, est normalisé avant le push : la branche poussée est fmt-clean.
  *(T-c ; ferme #2345, que le remède littéral du ticket ne couvrait pas.)*
- **AC4** — `wip_rescue.rs` ne pousse plus une draft non-fmt-clean vers qa-review :
  la normalisation tourne entre la clippy gate et le push, et un échec de fmt ne
  produit **pas** de bail-to-human. *(Ferme T3 ; D5.)*
- **AC5** — Un arbre propre et déjà fmt-clean ne produit **aucun** commit. Un rescue
  qui tire inconditionnellement est indistinguable d'un rescue qui ne tire jamais.
  *(T-d.)*
- **AC6** — Un `cargo fmt` en échec (Rust syntaxiquement invalide) ne fait perdre
  aucun contenu : le commit de rescue aboutit quand même. *(T-e ; D4.)*
- **AC7** — La normalisation n'embarque pas de fichier hors du diff de la branche.
  *(T-f ; D3.)*
- **AC8** — Une garde structurelle refuse l'ajout d'un futur site `git commit` de
  contenu dans `dispatch-lib.sh` qui ne passerait pas par `_fmt_and_stage_rust`, les
  commits `--allow-empty` exceptés. *(T-g ; D6 — c'est la garde qui aurait attrapé
  T1 en 2026-01.)*
- **AC9** — Les quatre signaux opérateur sont émis et documentés dans `CLAUDE.md`
  avec leur régime attendu, `rescue_fmt_failed` et `wip_rescue_fmt_failed` étant
  déclarés « doit rester vide ».
