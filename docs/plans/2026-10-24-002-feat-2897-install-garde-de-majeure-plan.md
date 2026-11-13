---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
issue: mika#2897
branch: feat/2897/install-install-sh-s-arr-te-avant-une
---

# feat(install): `install.sh` s'arrête avant une mise à jour majeure et pointe le guide de migration

## Problem Frame

Depuis mika#2894, Mika suit le semver strict à partir de v1.0 : un changement
cassant sort en version majeure, avec un guide de migration
(`CLAUDE.md:49`, `README.md:143`).

`install.sh` ne sait rien de cette règle. Il résout `releases/latest` (ou prend
la version passée en argument), télécharge l'archive, vérifie l'empreinte, puis
`mv "${TMPDIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}"` (`install.sh:80`) — **sans
jamais lire la version déjà installée**. Le chemin le plus courant du dépôt,
`curl … | sh` sans argument, est donc exactement celui qui emporte une personne
de 1.x à 2.x sans un mot. Elle apprend la migration quand son `~/.mika/` ne
s'ouvre plus.

Trois contraintes de forme cadrent la solution, et aucune n'est négociable :

1. **Le script est autonome.** Il est consommé par
   `curl -fsSL …/main/install.sh | sh`. Il ne peut pas sourcer un fichier du
   dépôt, donc la décision ne peut pas être déportée dans un `scripts/*.sh`
   comme l'a été la classification de tag (`scripts/release-tag.sh`, mika#2890).
   La garde vit dans `install.sh`.
2. **Il n'y a pas de stdin disponible.** Sous `curl | sh`, stdin *est* le
   script. Une question interactive est structurellement impossible : d'où une
   **option**, et non une confirmation — ce que le ticket demande déjà.
3. **Il est en `/bin/sh`** (`install.sh:1`), exécuté par dash sur Debian et
   Ubuntu. Pas de `[[ ]]`, pas de `BASH_REMATCH`, pas de tableau.

La quatrième contrainte vient d'AC4 : le test est **hors réseau**. Un test qui
mène l'installation à son terme devrait servir une archive et son empreinte.
L'approche retenue (D3) ne le fait pas : elle intercepte `curl` et lit le
journal de ses appels, ce qui atteste directement la propriété d'AC2 — *rien
n'a été remplacé* — au lieu de l'inférer d'une installation réussie.

## Requirements

- **R1.** Avant le téléchargement de l'archive, `install.sh` lit la version du
  binaire présent dans `INSTALL_DIR` — et nulle part ailleurs : un `mika` sur le
  `PATH` mais hors d'`INSTALL_DIR` n'est pas celui que ce script remplace.
- **R2.** Quand la majeure de la version cible diffère de la majeure installée,
  le script sort en non-zéro **avant** le premier `curl` de téléchargement et
  **avant** tout `mv`.
- **R3.** Le message de refus nomme la version installée, la version cible, et
  l'URL du guide de migration de la version cible.
- **R4.** Une option explicite, documentée dans l'en-tête du script et dans
  `docs/releasing.md`, lève la garde.
- **R5.** La garde s'applique à la version **effectivement résolue**, y compris
  quand elle vient de `releases/latest` et non de l'argument — c'est le chemin
  du README, et donc le chemin qui compte.
- **R6.** Installation absente, ou de même majeure : aucun changement de
  comportement observable (ni sortie supplémentaire, ni code de retour, ni appel
  réseau de plus).
- **R7.** Une version installée **illisible** (binaire non exécutable,
  `--version` en échec, sortie non conforme) laisse passer, avec un
  avertissement sur stderr. Voir D5 pour la justification du sens du défaut.
- **R8.** La suite de test tourne hors réseau et est atteinte par la CI.

## Non-objectifs

- **Aucune création de guide de migration.** Le document pointé est
  `docs/releases/vX.Y.Z.md`, qui existe déjà pour v1.0.0 et dont la présence est
  exigée par `release.yml:105` avant toute publication. Enrichir ces notes d'une
  section « migrer depuis la majeure précédente » est le travail du vol de
  release de la majeure concernée, pas de celui-ci.
- **Aucune correction du préfixe `v` sur l'URL de téléchargement.**
  `install.sh:45-51` accepte `1.2.3` *et* `v1.2.3`, mais `install.sh:55`
  construit son URL avec la chaîne brute, alors que les tags sont tous `vX.Y.Z`
  — donc `sh -s -- 1.2.3` échoue déjà aujourd'hui, en 404. Hors périmètre ; la
  garde normalise pour son seul usage (D4).
- **Aucune variable d'environnement en doublon de l'option.** Deux portes pour
  une décision font deux sites à documenter, et la seconde finit non testée.
  `MIKA_INSTALL_DIR` reste la seule variable du script.
- **Pas de garde sur la mineure ni sur la patch.** Le semver strict ne promet
  une migration que sur la majeure.
- **Pas de shellcheck en CI.** Le dépôt n'en a pas (vérifié : aucune occurrence
  dans `ci.yml` ni dans le `Makefile`), et en ajouter un est un autre ticket.

## Approche

### D1 — Où la garde se place dans le script, et pourquoi exactement là

L'ordre actuel d'`install.sh` est : `INSTALL_DIR` → argument → `uname` →
résolution de `latest` par l'API (`:34`) → validation semver (`:41-52`) →
`URL` (`:55`) → téléchargement (`:62`) → empreinte (`:66-75`) → `mv` (`:80`).

La garde s'insère **entre la validation semver et la construction de `URL`** :

- après la validation, parce qu'elle a besoin d'une `VERSION` dont la forme est
  déjà connue — comparer des majeures sur une chaîne non validée, c'est écrire
  un second validateur ;
- après la résolution par l'API, et non avant, parce que R5 l'exige : sur le
  chemin sans argument la version cible n'existe pas encore à l'étape
  précédente ;
- avant `URL`, parce que c'est le dernier point où aucun octet d'archive n'a
  été demandé. R2 porte sur *avant le remplacement* ; se tenir avant le
  *téléchargement* est strictement plus fort, et c'est cette borne-là que le
  test mesure.

Le seul appel réseau qui précède donc la garde est celui de l'API `latest`, et
il est inévitable : c'est lui qui produit la version à juger.

### D2 — L'option, et le parsing qu'elle impose

`--allow-major-upgrade`. Nom long, sans forme courte : il est tapé une fois dans
la vie, par quelqu'un qui vient de lire le message de refus, et un `-a` dans un
one-liner collé depuis un forum ne se relit pas.

Le parsing actuel est `VERSION="${1:-}"` (`install.sh:12`). Il devient une
boucle `while` / `case` POSIX acceptant, dans n'importe quel ordre :

```sh
ALLOW_MAJOR=0
VERSION=""
while [ $# -gt 0 ]; do
    case "$1" in
        --allow-major-upgrade) ALLOW_MAJOR=1 ;;
        -h|--help) … ; exit 0 ;;
        -*) echo "Error: unknown option '$1'" >&2; exit 2 ;;
        *)  if [ -n "$VERSION" ]; then
                echo "Error: version given twice ('$VERSION' then '$1')" >&2; exit 2
            fi
            VERSION="$1" ;;
    esac
    shift
done
```

L'ordre libre n'est pas un luxe : la forme du README est
`… | sh -s -- v2.0.0`, et la personne qui ajoute le drapeau l'écrira aussi bien
avant qu'après. Le refus d'un `-*` inconnu évite qu'une faute de frappe sur le
drapeau (`--allow-major`) soit avalée comme une version, puis refusée bien plus
loin par le validateur semver avec un message qui ne nomme pas la cause.

`-h|--help` tombe du même mouvement : AC3 exige que l'option soit documentée
dans l'en-tête du script, et le script devient le seul endroit qu'une personne
puisse interroger quand elle l'a sous la main.

### D3 — Comment un test hors réseau prouve « rien n'a été remplacé »

Le test (`scripts/test-install-major-gate.sh`, bash, invoqué par
`sh "$REPO_ROOT/install.sh"`) construit pour chaque cas un bac à sable jetable
sous `mktemp -d` — la discipline de `scripts/test-release-tag.sh:25` — et y
place deux choses :

1. **Un `INSTALL_DIR` peuplé à la demande.** Le faux binaire est un script shell
   de deux lignes qui imprime `mika <version> (deadbee)` et sort 0. Ce format
   n'est pas inventé : il est celui de
   `crates/mika-common/src/build_info.rs:22-25` (`"{VERSION} ({GIT_HASH})"`),
   préfixé du nom du binaire par clap (`crates/mika-cli/src/cli.rs:4-10`). Le
   fichier porte en plus une **ligne sentinelle** dont le test vérifie la
   survie après coup.
2. **Un stub `curl` en tête de `PATH`.** Il journalise chacun de ses arguments
   dans un fichier, sert un JSON `{"tag_name": "…"}` quand l'URL contient
   `api.github.com`, et sort 1 sur tout le reste.

De là, deux assertions suffisent, et elles sont de signe opposé :

- **La garde a refusé** quand (a) le code de retour est non nul, (b) le journal
  du stub ne contient **aucune** URL `releases/download`, et (c) la sentinelle
  du faux binaire est intacte.
- **La garde a laissé passer** quand le journal contient une URL
  `releases/download`. Le script meurt ensuite sur le `curl` stubbé, et c'est
  sans importance : la question posée est « la garde a-t-elle laissé passer »,
  pas « l'installation aboutit-elle ». Mener l'installation à son terme
  demanderait de fabriquer une archive et une empreinte valides, c'est-à-dire de
  tester le téléchargement et la vérification d'empreinte — deux comportements
  que ce ticket ne touche pas, et dont la mise sous test ici rendrait la suite
  solidaire de choses qui n'ont rien à voir avec la garde.

Aucun hook de testabilité n'entre dans `install.sh` : pas de `--dry-run`, pas de
`MIKA_INSTALL_GATE_ONLY`. Un mode qui n'existe que pour le test est un mode dont
le test ne prouve rien sur le chemin réel.

### D4 — L'URL du guide, et pourquoi elle ne peut pas être morte

```
https://github.com/senara-solutions/mika/blob/<tag>/docs/releases/<tag>.md
```

où `<tag>` est la version cible, préfixée de `v` si elle ne l'est pas.

Ce chemin est le seul du dépôt dont une garde existante interdit l'absence :
`.github/workflows/release.yml:105` refuse de préparer un tag quand
`docs/releases/v$base.md` manque, et `:328` publie ce même fichier comme corps
de la release. Donc pour toute version qu'`install.sh` est capable d'installer,
le document existe. L'ancrage se fait sur le **tag** et non sur `main` : un lien
sur `main` décrit la version du jour, pas celle vers laquelle on migre.

La normalisation du `v` est locale à la garde et ne touche ni `ARCHIVE` ni
`URL` (voir Non-objectifs).

### D5 — La version illisible laisse passer, et ce n'est pas une facilité

R7 est le seul endroit du plan où le défaut se choisit, et les deux sens ont un
coût réel :

- **Fail-closed** (refuser quand on ne sait pas lire) bloque la réinstallation
  d'un binaire cassé — tronqué, bâti pour la mauvaise architecture, à demi
  écrasé par un `mv` interrompu. Or réinstaller *est* le geste de réparation. La
  garde interdirait précisément la sortie de panne.
- **Fail-open** (laisser passer avec un avertissement) retombe sur le
  comportement d'aujourd'hui, qui est le pire cas connu et accepté : exactement
  ce qu'AC1 appelle « le comportement actuel ne change pas ».

Le fail-open l'emporte, et l'avertissement sur stderr est ce qui l'empêche
d'être silencieux :

```
Warning: could not read the version installed at <dir>/mika; skipping the major-upgrade check.
```

La lecture de version est en `|| true` : sous `set -eu` (`install.sh:5`), un
`--version` en échec tuerait le script avec un code que personne ne peut
interpréter.

### D6 — L'extraction de la majeure, en POSIX

Deux entrées, deux formes :

- la **cible** est `vX.Y.Z[-pre]` ou `X.Y.Z[-pre]`, déjà validée par
  `install.sh:45-51` ;
- l'**installée** est la sortie de `mika --version`, soit `mika X.Y.Z (sha)`.
  Le deuxième champ est pris par `awk '{print $2}'`, puis validé contre
  `^[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*` avant usage. Un champ qui ne valide
  pas est une version illisible (R7) ; c'est ce qui rend le parsing sûr sans
  dépendre de la stabilité du format du suffixe entre parenthèses.

La majeure est ensuite le préfixe numérique : retrait d'un `v` initial, puis
tout jusqu'au premier `.`. Le suffixe de pré-release ne gêne pas — la majeure de
`v2.0.0-rc.1` se lit avant le tiret, et ce cas est tenu par un test.

**La comparaison est symétrique.** `2.1.0 → v1.0.0` est aussi un changement de
majeure, et un retour en arrière de majeure est au moins aussi cassant qu'une
avancée : le `~/.mika/` migré par la 2.x n'est pas lisible par la 1.x. Le
message parle de « major change », jamais de « upgrade », hors le nom de
l'option.

### D7 — Le branchement CI

La suite va dans le job `offline-script-suites` de `ci.yml`, par une cible
`make` et non par un `bash scripts/…` direct — la cible est le site où
l'ordonnancement vit, et la dupliquer en YAML en créerait un second
(`ci.yml:593-596` le dit dans ces termes). Trois éditions :

- `Makefile:4` — `check-install-major-gate` ajouté à `.PHONY` ;
- `Makefile` — la cible, posée après `check-release-tag` (son voisin de
  sujet : les deux gardent une propriété d'`install.sh`) ;
- `.github/workflows/ci.yml` — une step nommée dans `offline-script-suites`,
  une step par suite comme le job l'impose déjà.

Le nom `test-install-major-gate.sh` porte le préfixe `test-`, l'un des cinq que
`scripts/check-suite-wiring.sh:113` énumère : la suite est donc **comptée** par
le lint, et son absence de wiring serait refusée. Aucune ligne n'est ajoutée à
`scripts/suite-wiring.tsv` — ce registre ne tient que le résidu, et une suite
que la CI atteint y ferait rougir le lint (`suite-wiring.tsv:33-36`).

## Fire-Disposition

**Option (a) — exception nommée en allowlist. Aucune n'est créée, et la raison
est structurelle.**

Le détecteur livré est `scripts/test-install-major-gate.sh`. Son corpus est un
seul fichier, `install.sh`, et la seule « donnée existante » qu'il pourrait
condamner est la version d'`install.sh` du commit où il atterrit — celle qui
porte la garde. Il n'existe donc pas de violation à amnistier : le détecteur
arrive **armé et vert**, sans `#[ignore]`, sans fichier d'exceptions, et le vol
ne crée aucun ticket de suivi pour l'armer plus tard.

Ce que (a) demande encore, et qui s'applique : l'**assertion auto-nettoyante**.
Un détecteur dont le corpus est un seul fichier peut passer au vert en perdant
sa prise — si quelqu'un retire la garde d'`install.sh`, chaque cas « laisse
passer » reste vert par construction (tout passe), et seuls les cas de refus
rougissent. La suite porte donc deux assertions d'ancrage, hors cas de
comportement :

1. `install.sh` contient la chaîne `--allow-major-upgrade`. Elle rougit si la
   garde ou son option disparaît du script.
2. `install.sh` ne contient pas `[[` — l'ancre POSIX de D1, grep-visible, qui
   rougit si un bashisme entre dans un script exécuté par dash.

Chaque assertion nomme `mika#2897` dans son message d'échec, pour que le rouge
dise d'où vient la règle.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `install.sh` (parsing d'args, lecture de version, garde, en-tête) | 60 |
| tests (`scripts/test-install-major-gate.sh`) | 130 |
| `Makefile` (cible + `.PHONY`) | 3 |
| `.github/workflows/ci.yml` (une step dans `offline-script-suites`) | 5 |
| `CHANGELOG.md` | 3 |

Total estimé : 201 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 201 lignes estimées contre une borne par vol de
279 et un seuil par PR de 1000 : les deux bornes passent, la plus serrée (279)
avec 78 lignes de marge. La forme du travail soutient l'estimation plutôt que de
la contredire : un seul fichier de production, modifié en un seul endroit
contigu ; aucun code Rust, donc aucun cycle `cargo build` dans la boucle de
mise au point ; la suite tourne en moins d'une seconde, ce qui rend l'itération
quasi gratuite en tours. Le poste le plus lourd est le harnais de test (130
lignes), dont la moitié est de la plomberie sans jugement — stub, fabrique de
fixture, deux helpers d'assertion — écrite d'un coup. Pas de découpage en vols,
donc pas de ligne `Tracked in:`.

## Fichiers

- `install.sh` — parsing d'arguments, `-h|--help`, lecture de la version
  installée, garde de majeure, en-tête d'usage mis à jour (AC3).
- `scripts/test-install-major-gate.sh` — nouveau. La suite hors réseau (AC4).
- `Makefile` — cible `check-install-major-gate` + entrée `.PHONY`.
- `.github/workflows/ci.yml` — une step dans `offline-script-suites` (AC4).
- `docs/releasing.md` — l'option et le refus, dans « Rerun, rollback,
  fallbacks » (AC3).
- `CHANGELOG.md` — l'entrée de la version non publiée.
- `docs/plans/2026-10-24-002-feat-2897-install-garde-de-majeure-plan.md` — ce
  plan.

**`.github/workflows/ci.yml` demande le scope `workflow`.** Si le push de la
branche est refusé pour cette raison, le vol s'arrête à la step CI : tout le
reste est commité, et le corps de PR nomme la step à poser pour que l'opérateur
la mette en place. Ne pas la poser du tout laisserait AC4 à moitié tenu — la
suite existerait sans que rien ne la lance, exactement la classe mika#2578.

## Vérification

- **V1.** `bash scripts/test-install-major-gate.sh` : 9 cas verts, 0 rouge.
- **V2. Rouge-avant, par la négative.** Une copie d'`install.sh` **sans** le
  bloc de garde est posée sous `.pilot-scratch/` ; la suite, pointée sur elle
  par sa variable de script, rend rouges les trois cas de refus (3, 5, 8) et
  l'assertion d'ancrage 1, et garde verts les cas qui laissent passer. Le
  transcrire dans le corps de PR : un squash merge efface la copie.
- **V3.** `sh -n install.sh` ne relève rien, et
  `grep -c '\[\[' install.sh` rend 0.
- **V4.** `make check-install-major-gate` passe.
- **V5.** `bash scripts/check-suite-wiring.sh` passe et rapporte `unwired=0`.
  Une suite nouvelle non branchée s'y verrait : le contrôle n'est pas
  décoratif.
- **V6.** `bash scripts/check-root-files.sh` passe (`install.sh` est modifié,
  pas ajouté ; aucun fichier racine ne naît).
- **V7.** Le YAML de `ci.yml` se parse, et la step apparaît dans
  `offline-script-suites`.
- **V8.** `scripts/verify-pipeline.sh origin/main` passe.
- **V9. Le chemin du dépôt reste intact.** `git diff origin/main -- install.sh`
  ne modifie aucune ligne entre `URL=` et la fin du script : le téléchargement,
  l'empreinte, l'extraction et l'avertissement de `PATH` sont hors diff. C'est
  la lecture mécanique d'AC1 — le comportement actuel ne change pas parce que le
  code qui le produit n'est pas touché.
- **V10. Le lien n'est pas mort.** `docs/releases/v1.0.0.md` existe, et
  l'URL que la garde fabriquerait pour `v1.0.0` pointe dessus.

## Definition of Done

La garde est dans `install.sh` avant toute requête de téléchargement, son option
est documentée dans l'en-tête du script et dans `docs/releasing.md`, la suite de
9 cas est verte et atteinte par `ci.yml` à travers une cible `make`, V1-V10
passent en local, le rouge-avant de V2 est transcrit dans le corps de PR, et
le tout est commité sans push.

## Acceptance criteria

- [ ] **AC1** — Sur une installation absente, ou de même majeure que la cible,
  le comportement actuel ne change pas.
- [ ] **AC2** — Quand la majeure change, `install.sh` sort en non-zéro avant de
  remplacer le binaire. Son message nomme les deux versions et le guide de
  migration.
- [ ] **AC3** — Une option documentée dans l'en-tête du script et dans
  `docs/releasing.md` permet de poursuivre explicitement.
- [ ] **AC4** — Un test hors réseau couvre les trois cas : absent, même majeure,
  majeure différente. Il est branché dans une suite que la CI atteint.

## Cas de test

Les trois premiers sont ceux d'AC4. Les six suivants sont les trajectoires où
une garde mal posée passe quand même les trois premiers.

| # | installée | cible | option | attendu |
|---|---|---|---|---|
| 1 | aucune | `v1.2.0` | — | laisse passer |
| 2 | `1.0.0` | `v1.2.0` | — | laisse passer |
| 3 | `1.4.0` | `v2.0.0` | — | **refus** : non-zéro, aucun `releases/download`, sentinelle intacte, message avec `1.4.0`, `v2.0.0` et l'URL |
| 4 | `1.4.0` | `v2.0.0` | `--allow-major-upgrade` | laisse passer (AC3) |
| 5 | `2.1.0` | `v1.0.0` | — | **refus** — la majeure change dans les deux sens (D6) |
| 6 | binaire illisible | `v2.0.0` | — | laisse passer, avertissement sur stderr (R7) |
| 7 | `1.0.0` | résolue à `v2.0.0` par l'API | — | **refus** — la garde juge la version résolue, pas l'argument (R5) |
| 8 | `1.4.0` | `v2.0.0-rc.1` | — | **refus** — la majeure se lit avant le tiret |
| 9 | `1.4.0` | `v2.0.0`, drapeau **avant** la version | `--allow-major-upgrade` | laisse passer — l'ordre des arguments est libre (D2) |

Plus les deux assertions d'ancrage de la section Fire-Disposition.

## Assumptions

- `mika --version` garde la forme `mika X.Y.Z (sha)`. Si elle dérive, la version
  installée devient illisible et la garde laisse passer avec un avertissement
  (R7) : la dérive dégrade la garde, elle ne casse pas l'installation. Le cas 6
  est précisément celui-là.
- Les tags de release restent `vX.Y.Z` et `vX.Y.Z-rc.N`
  (`scripts/release-tag.sh:47`), et `docs/releases/vX.Y.Z.md` reste le document
  que `release.yml:105` exige. Les deux sont tenus par des gardes existantes.
- Le `curl` d'`install.sh` est résolu par le `PATH` — il n'est pas appelé par
  chemin absolu (`install.sh:34,62,66`), donc le stub de D3 l'intercepte.
- La garde ne peut pas être exercée contre une vraie release avant qu'une v2
  existe. C'est pourquoi tout le poids de la preuve est sur la suite hors réseau
  et sur son rouge-avant (V2), et non sur un essai manuel.
