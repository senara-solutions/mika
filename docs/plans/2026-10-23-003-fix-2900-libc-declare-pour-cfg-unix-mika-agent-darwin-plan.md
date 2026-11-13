---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
issue: mika#2900
branch: fix/2900/build-mika-agent-compile-pour-macos-libc
---

# fix(build): `libc` déclaré pour `cfg(unix)` — `mika-agent` compile pour les deux cibles darwin

## Problem Frame

`crates/mika-agent/Cargo.toml:76-77` déclare :

```toml
[target.'cfg(target_os = "linux")'.dependencies]
libc.workspace = true
```

`crates/mika-agent/src/skills/executor.rs:1415-1452` — le `Drop` de
`ProcessGroupKillGuard`, posé par 70127bbf (mika#2438, 2026-09-22) — appelle
`libc::killpg`, `libc::SIGKILL` et `libc::ESRCH` **hors de tout `#[cfg]`**. Sous
Linux la table de cible fournit le crate et le code résout ; pour une cible
darwin la table ne s'applique pas, `libc` n'entre pas dans le graphe, et les
trois chemins d'accès ne résolvent plus :

```
error[E0433]: failed to resolve: use of unresolved module or unlinked crate `libc`
    --> crates/mika-agent/src/skills/executor.rs:1430:27
    --> crates/mika-agent/src/skills/executor.rs:1444:39
error: could not compile `mika-agent` (lib) due to 3 previous errors
```

**Ce que la combinaison a produit.** `release.yml:116-135` construit quatre
cibles — `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
`x86_64-apple-darwin`, `aarch64-apple-darwin` — et ne publie que si les quatre
passent. Le candidat `v1.0.0-rc.1` (run `release.yml` 39961929051, 2026-10-23)
sort avec le code 101 sur les deux builds macOS. Aucune release v1.0.0 ne peut
sortir tant que ce défaut vit.

**Pourquoi il a vécu un mois.** Aucun job de `ci.yml` ne compile pour une cible
darwin : le job `Check` tourne sur `[self-hosted, Linux, X64, gentux]` et les
autres sur `ubuntu-22.04`. La première évaluation du manifeste pour une cible
darwin a eu lieu au premier tag. Le défaut est entré le 2026-09-22 et a été vu
le 2026-10-23.

**Les deux autres consommateurs de `libc` dans le crate sont déjà bornés** — et
c'est pour cela qu'eux ne cassaient pas :

| site | garde | repli hors Linux |
|---|---|---|
| `task_engine/process_liveness.rs:57` (`libc::kill`) | `#[cfg(target_os = "linux")]` interne | `false` — watchdog désactivé |
| `worktree_reaper.rs:4364,4427,4461,4466` (`libc::flock`) | `#[cfg(target_os = "linux")]` sur le `Drop`, sur `probe_one_cargo_lock`, et en bloc interne | `LockProbe::Unevaluable` / `LockAcquisition::Unevaluable` — **conserve** |
| `skills/executor.rs:1430,1444` (`libc::killpg`) | **aucune** | — |

Le troisième est le seul non gardé, et c'est exactement celui que le compilateur
nomme.

**Ce que `cfg(unix)` ne sur-approxime pas.** `mika-agent` n'est compilable sur
aucune cible non-unix aujourd'hui : `executor.rs:1495` et `executor.rs:6014`
appellent `.process_group(0)` sans `#[cfg]`, et cette méthode vient de
`std::os::unix::process::CommandExt`. Il n'existe donc pas de cible où
`cfg(unix)` déclarerait `libc` dans le vide, et élargir la clé de
`cfg(target_os = "linux")` à `cfg(unix)` n'ouvre aucune plateforme nouvelle :
elle fait coïncider la déclaration avec la portée que le code avait déjà.

**Une affirmation du dépôt devient fausse avec le correctif.** Deux textes
disent que la clé Cargo *impose* le `#[cfg(target_os = "linux")]` des sites
`flock` :

- `crates/mika-agent/src/worktree_reaper.rs:4493` — « `libc` vit sous
  `[target.'cfg(target_os = "linux")'.dependencies]`, donc cet appel suit le
  patron de `process_liveness::is_same_process_alive` » ;
- `docs/agent-internals/tools.md:1180-1182` — la même phrase en anglais.

Après le correctif la raison du repli n'est plus le manifeste : c'est que la
sémantique de `flock` et la lecture de `/proc` ne sont pas vérifiées hors Linux.
Laisser la phrase en l'état ferait conclure à un lecteur que la clé linux est
une contrainte, et c'est précisément la lecture qui a produit ce ticket.

## Requirements

- **R1.** `mika-agent` compile pour `aarch64-apple-darwin` et
  `x86_64-apple-darwin`. Forme retenue : élargir la clé de la table de cible à
  `cfg(unix)`, **pas** borner les appels d'`executor.rs` à Linux — `killpg`,
  `SIGKILL` et `ESRCH` existent sur macOS, et borner la garde reviendrait à
  laisser un groupe de processus orphelin sur la plateforme où le pilote n'a
  justement pas de superviseur OpenRC.
- **R2.** Le comportement sous Linux est inchangé, bit pour bit. Aucun `#[cfg]`
  de site n'est touché : les replis non-Linux de `process_liveness` et de
  `worktree_reaper` restent ce qu'ils sont.
- **R3.** Une garde **dans le dépôt, exécutable hors ligne** rougit avant le
  correctif et passe après (AC2), et rougit à nouveau si quelqu'un re-restreint
  la clé.
- **R4.** Les deux textes qui affirment la contrainte manifeste sont corrigés,
  y compris le doc-comment du code.
- **R5.** La garde de CI qui compile pour darwin sur une PR est **nommée et
  tracée**, pas livrée ici : `.github/workflows/` demande le scope `workflow`
  que le pilote n'a pas. La PR porte une ligne `Tracked in:`.
- **R6.** `cargo test` et `cargo clippy` restent verts sous Linux (AC3).

## Non-objectifs

- **Porter les chemins linux-only sur macOS.** `read_process_start_time` lit
  `/proc`, `cargo_build_lock_is_free` sonde avec `flock`. Hors Linux ils rendent
  respectivement `None`/`false` et `Unevaluable` — régime documenté, délibéré,
  et ce vol ne l'élargit pas. Le crate **compilera** sur darwin ; il n'y sera
  pas plus fonctionnel qu'avant.
- **Ajouter un runner macOS à `ci.yml`.** Hors scope OAuth du pilote (R5).
- **Tagger `v1.0.0-rc.2`.** Geste de Vincent, après le run manuel de
  `release.yml` qui porte la preuve d'AC1.
- **Toucher `mika-cli`.** `commands/agents.rs:2900` appelle un `libc_geteuid`
  déclaré localement en `extern "C"`, pas le crate `libc` : ce site compile déjà
  pour darwin et n'est pas concerné.
- **Vérifier un build darwin complet depuis le sandbox.** Impossible sans SDK
  Apple. La preuve d'AC1 est le run `release.yml` de l'opérateur (V6).

## Approche

### D1 — Le détecteur, livré d'abord et rouge

`crates/mika-agent/tests/libc_target_scope_2900.rs`, nouveau test d'intégration.
Il ne compile rien et n'appelle aucun sous-processus : il lit deux fichiers via
`CARGO_MANIFEST_DIR` et parse le manifeste avec `toml`
(`crates/mika-agent/Cargo.toml:45`, déjà une dépendance du crate).

Deux assertions, et elles jouent des rôles différents.

**A1 — la garde.** Dans `Cargo.toml`, la table de cible qui déclare `libc` doit
avoir pour clé `cfg(unix)`. Le test balaye toutes les clés de `[target.*]`,
retient celles dont la section `dependencies` nomme `libc`, et exige qu'il y en
ait exactement une, de clé `cfg(unix)`. Une déclaration sous
`cfg(target_os = "linux")` — l'état d'avant — échoue, et le message nomme
mika#2900, le run 39961929051, les deux cibles darwin de `release.yml` et le
site `executor.rs` qui ne résout plus.

Pourquoi une clé **exacte** plutôt qu'une évaluation de cfg : évaluer
`cfg(any(target_os = "linux", target_os = "macos"))` contre les quatre triples
de `release.yml` demanderait un mini-évaluateur de cfg dans un test, c'est-à-dire
plus de code que ce qu'il garde. Le prix est un faux rouge sur une reformulation
délibérée et équivalente ; il est payé par un message d'échec qui dit quoi faire
(F2).

**A2 — l'auto-nettoyage.** `src/skills/executor.rs` doit encore contenir
`libc::killpg`. C'est le seul site non gardé du crate, donc le seul qui rend
`cfg(unix)` nécessaire : s'il disparaît, A1 contraint le manifeste pour rien et
le test est stale. Le message d'échec d'A2 demande alors de réévaluer mika#2900
plutôt que de supprimer la ligne sans la lire.

### D2 — Le correctif

`crates/mika-agent/Cargo.toml:76` : la clé de la table passe de
`cfg(target_os = "linux")` à `cfg(unix)`. La ligne `libc.workspace = true` ne
bouge pas. Sous Linux, `cfg(unix)` est vrai : le graphe de dépendances y est
identique, et `Cargo.lock` ne liste pas les dépendances par cible — il ne devrait
pas bouger (V4 le vérifie plutôt que de le supposer).

### D3 — Les deux textes

- `crates/mika-agent/src/worktree_reaper.rs:4491-4500` : le paragraphe « Le repli
  non-Linux **conserve**, et son sens n'est pas libre » garde sa conclusion et
  change sa raison. Nouvelle rédaction : depuis mika#2900 `libc` est déclaré pour
  `cfg(unix)`, donc **rien dans le manifeste n'impose** le
  `#[cfg(target_os = "linux")]` de ce site ; ce qui l'impose est que la sémantique
  de `flock` entre OS et la population de `.cargo-lock` ne sont vérifiées que sous
  Linux. Le repli reste `Unevaluable`, pour la raison déjà écrite.
- `docs/agent-internals/tools.md:1180-1182` : même correction en anglais. Ce
  fichier n'est **pas** dans la liste `DOCS` de `scripts/sync-agent-docs.sh` :
  aucun `sync-agent-docs.sh` à lancer, le job `docs-sync` n'est pas concerné.

### D4 — Le ticket de suivi pour la garde de CI

Le pilote ouvre, avec `gh issue create --repo senara-solutions/mika`, un ticket
`fix(ci): compiler pour les deux cibles darwin sur chaque PR (suite de
mika#2900)`, labels `infrastructure`, `p1-important`. Son corps porte le job à
appliquer, pour que l'opérateur n'ait qu'à le coller :

```yaml
  darwin-check:
    name: Darwin Check (${{ matrix.target }})
    runs-on: macos-latest
    strategy:
      fail-fast: false
      matrix:
        target: [aarch64-apple-darwin, x86_64-apple-darwin]
    steps:
      - uses: actions/checkout@<sha pinné, aligné sur les autres jobs de ci.yml>
      - run: rustup target add ${{ matrix.target }}
      - uses: Swatinem/rust-cache@<sha pinné>
        with:
          key: ${{ matrix.target }}
      - run: cargo check --locked --target ${{ matrix.target }} -p mika-agent -p mika-cli
```

`cargo check` et non `cargo build` : l'erreur de ce ticket est une erreur de
résolution de noms, elle tombe avant le link, et `check` coûte une fraction du
build. Le corps du ticket note aussi **pourquoi le job ne peut pas tourner sur
Linux** : `cargo check --target <darwin>` depuis Linux exécute les build scripts,
et `libsqlite3-sys` y compile du C — sans toolchain darwin le job rougirait pour
une raison qui n'est pas celle qu'on surveille.

Le numéro rendu par `gh issue create` va en `Tracked in:
senara-solutions/mika#<n>` dans le corps de la PR, exigé par
`pr-body-validation.yml` (phrase de report présente).

### D5 — La trace

`docs/solutions/build-errors/2026-10-23-libc-declare-par-cible-casse-au-premier-tag.md` :
une table de dépendances par cible est un contrat que **seule une compilation
pour cette cible évalue** ; un `cargo check` Linux vert ne dit rien d'un
`cfg(target_os)` qui n'est pas le sien, et le premier lecteur du contrat a été
un tag de release. Ce que le fichier retient : quand un site appelle un crate
déclaré par cible, la clé de la table doit couvrir **toutes** les cibles de
`release.yml`, et c'est la matrice de release — pas celle de CI — qui définit
l'ensemble à couvrir.

### Ordre de commit

1. **`test(build): la déclaration de libc doit atteindre darwin (mika#2900)`** —
   D1 seul. `cargo test -p mika-agent --test libc_target_scope_2900` est **ROUGE**
   ici, sur A1. La sortie est transcrite dans le corps de la PR (V1).
2. **`fix(build): déclarer libc pour cfg(unix) (mika#2900)`** — D2 + D3. Le test
   passe au **VERT**. C'est le **plancher mergeable** : ce commit seul débloque
   la release.
3. **`docs(solutions): une dépendance par cible n'est évaluée que par sa cible (mika#2900)`**
   — D5.

Le rouge de l'étape 1 vit dans le corps de la PR, pas seulement dans
l'historique : un squash au merge l'effacerait de `git log` (F3).

## Fenêtres d'échec

- **F1 — `cargo tree --target <darwin>` indisponible dans le sandbox.** V2 est la
  preuve sémantique la plus forte atteignable hors ligne : `cargo tree` résout
  les cfg via `rustc --print cfg --target`, qui répond pour une cible **non
  installée**. Si le sandbox refuse malgré tout, V2 est best-effort : la PR le
  dit, et la preuve d'AC1 reste V6.
- **F2 — A1 rougit sur une reformulation équivalente.** Écrire
  `cfg(any(target_os = "linux", target_os = "macos"))` couvrirait darwin et
  ferait pourtant échouer A1. Assumé : le message d'échec nomme mika#2900 et
  indique qu'une clé autre que `cfg(unix)` doit être validée en élargissant
  l'ensemble accepté du test, jamais en supprimant l'assertion.
- **F3 — le rouge-avant disparaît au squash.** Traité par l'ordre de commit
  ci-dessus : la transcription dans le corps de PR est la preuve durable.
- **F4 — `Cargo.lock` bouge.** Non attendu, mais pas supposé : V4 le vérifie. S'il
  bouge, la PR liste ce qui change et pourquoi, et `--locked` reste la forme de
  build de `release.yml`.
- **F5 — on croit avoir prouvé AC1.** Aucune commande du pilote ne prouve qu'un
  binaire darwin se **link**. Le vol prouve que la résolution de noms passe
  (V1, V2) ; le link est prouvé par V6 et par rien d'autre. Le corps de la PR
  doit le dire dans ces termes plutôt que d'écrire « AC1 vérifié ».
- **F6 — on referme le ticket sur le merge.** AC1 demande un run manuel de
  `release.yml` sur `main` **après** le merge, quatre builds verts, cité dans
  mika#2900. Le merge ne ferme pas : la PR écrit `Refs #2900`, pas `Closes`.

## Verification Contract

| # | commande / geste | attendu |
|---|---|---|
| V1 | `cargo test -p mika-agent --test libc_target_scope_2900` au commit 1, puis au commit 2 | **ROUGE** puis **VERT**, les deux sorties transcrites dans le corps de la PR (AC2) |
| V2 | `cargo tree -p mika-agent -i libc --target aarch64-apple-darwin`, idem `x86_64-apple-darwin`, avant et après D2 | avant : `libc` absent du graphe darwin ; après : `mika-agent` apparaît comme dépendant inverse (best-effort, F1) |
| V3 | `cargo build --workspace` | vert |
| V4 | `git status --short Cargo.lock` après D2 | vide ; sinon la PR explique le delta (F4) |
| V5 | `cargo test --workspace` et `cargo clippy --all-targets --all-features -- -D warnings` | verts (AC3) |
| V6 | `scripts/verify-pipeline.sh origin/main` | vert |
| V7 | run manuel de `release.yml` sur `main` après le merge | quatre builds verts, cité en commentaire de mika#2900 (AC1) — **opérateur, hors du vol** |
| V8 | `gh issue view <n>` sur le ticket de D4 | existe, et son numéro est dans la ligne `Tracked in:` de la PR (R5) |

## Fire-Disposition

Le vol livre **un détecteur** : `crates/mika-agent/tests/libc_target_scope_2900.rs`
(D1), dont le chemin de succès est « la déclaration de `libc` atteint darwin ».

**Option (a) — exception nommée en allowlist, avec une allowlist vide.**

- **Violations existantes : une seule, et le vol la supprime.** La déclaration
  sous `cfg(target_os = "linux")` est l'unique violation du dépôt ; le commit 2
  la corrige. Le détecteur atterrit donc **armé et vert**, et l'allowlist est
  vide — il n'y a aucune donnée à y nommer. Elle n'est pas omise : il n'y en a
  pas.
- **Rouge-avant démontré, pas supposé.** L'ordre de commit place le test avant
  le correctif précisément pour que la seule violation soit observée rouge
  (V1) ; c'est la substance d'AC2.
- **Assertion auto-nettoyante.** A2 exige que `src/skills/executor.rs` contienne
  encore `libc::killpg`. C'est la condition qui rend A1 nécessaire : si le
  dernier site non gardé disparaît, A1 contraindrait le manifeste sans raison et
  le test rougit en demandant de réévaluer mika#2900. Le détecteur signale donc
  sa propre péremption au lieu de survivre en contrainte morte.
- **Aucune entrée dans `scripts/suite-wiring.tsv`.** Le détecteur est un test
  `cargo`, pas une suite sous `scripts/` ni sous `skills/bundled/_shared/` : il
  est atteint par l'étape `Test` du job `Check` (`ci.yml:78-80`), donc le lint
  `suite-wiring` n'a rien à déclarer. Le registre reste à `unwired=0`.

La garde de CI darwin (D4) n'est **pas** livrée par ce vol — scope `workflow`
absent. Elle est tracée par le ticket de D4 et nommée en `Tracked in:`.

## Definition of Done

- `crates/mika-agent/Cargo.toml` déclare `libc` sous `cfg(unix)`.
- `crates/mika-agent/tests/libc_target_scope_2900.rs` existe, a été observé rouge
  au commit 1 et vert au commit 2, et les deux sorties sont dans le corps de la PR.
- `worktree_reaper.rs:4491-4500` et `docs/agent-internals/tools.md:1180-1182` ne
  présentent plus la clé Cargo comme la cause du `#[cfg(target_os = "linux")]`.
- V3, V4, V5 et V6 sont verts et cités dans le corps de la PR.
- Le ticket de suivi de D4 existe et son numéro est en `Tracked in:`.
- Le corps de la PR écrit `Refs #2900`, jamais `Closes` (F6), et dit explicitement
  que le link darwin n'est pas prouvé par le vol (F5).
- `docs/solutions/build-errors/2026-10-23-libc-declare-par-cible-casse-au-premier-tag.md`
  est écrit.

## Acceptance criteria

- **AC1.** `cargo build --release -p mika -p mika-spirit` passe pour les deux
  cibles darwin. La preuve est un run manuel de `release.yml` sur `main` après le
  merge, quatre builds verts, cité dans mika#2900.
- **AC2.** Rouge-avant. La garde de CI du point 2 rougit sur `main` avant le
  correctif, et passe après.
- **AC3.** `cargo test` et `cargo clippy` restent verts sous Linux.

**Ce que le vol atteint, et ce qu'il délègue.** AC3 est entièrement dans le vol
(V5). AC2 est satisfait **dans le dépôt** par le détecteur de D1, observé rouge
puis vert (V1) ; sa forme CI — un job qui compile réellement pour darwin sur
chaque PR — part dans le ticket de D4, parce que `.github/workflows/` est hors du
scope OAuth du pilote, comme le ticket le prévoit. AC1 n'est **pas** atteignable
depuis le sandbox : le vol en prouve la moitié qui est une erreur de résolution
de noms (V1, V2) et délègue le link à V7, geste de l'opérateur.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/Cargo.toml` — clé de la table de cible | 2 |
| `crates/mika-agent/src/worktree_reaper.rs` — doc-comment du repli non-Linux | 8 |
| `crates/mika-agent/tests/libc_target_scope_2900.rs` — détecteur A1 + A2 | 75 |

Total estimé : 85 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 85 ≤ 279 et 85 ≤ 1000 : les deux
bornes passent avec une marge large. Le correctif lui-même est de **deux
lignes** — le volume est presque entièrement dans le détecteur, qui est un
parseur TOML de quarante lignes et deux assertions, sans sous-processus, sans
fixture et sans dépendance nouvelle. Aucun découpage n'a de sens : séparer le
test du correctif donnerait une PR rouge et une PR verte dont aucune n'est
mergeable seule, alors que l'ordre de commit à l'intérieur d'un vol unique
donne la même preuve de rouge-avant sans ce coût.

Le risque de ce vol n'est pas le volume : il est dans ce que le pilote **ne peut
pas vérifier** (le link darwin, la garde de CI), et les deux sont traités —
le premier par F5 et V7, le second par D4 et `Tracked in:`.

Les livrables sous `docs/` — la correction de `docs/agent-internals/tools.md` et
le fichier de `docs/solutions/build-errors/` — sont hors comptage par la règle.
