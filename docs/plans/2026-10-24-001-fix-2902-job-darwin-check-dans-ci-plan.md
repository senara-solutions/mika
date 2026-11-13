---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
issue: mika#2902
branch: fix/2902/ci-compiler-pour-les-deux-cibles-darwin
---

# fix(ci): un job `darwin-check` compile `mika-agent` et `mika-cli` pour les deux cibles darwin sur chaque PR

## Problem Frame

mika#2900 : `crates/mika-agent/Cargo.toml` ne déclarait `libc` que sous
`[target.'cfg(target_os = "linux")'.dependencies]`, alors que le `Drop` de
`ProcessGroupKillGuard` (`crates/mika-agent/src/skills/executor.rs`) appelle
`libc::killpg` hors de tout `#[cfg]`. Le défaut est entré le 2026-09-22
(70127bbf, #2438) et n'a été vu que le 2026-10-23, au premier tag de release
(run `release.yml` 39961929051).

La cause est structurelle et survit au correctif : **aucun job de
`.github/workflows/ci.yml` ne compile pour une cible darwin.** `Check` tourne
sur `[self-hosted, Linux, X64, gentux]` (`ci.yml:22`), tous les autres jobs sur
`ubuntu-22.04`. Une table `[target.'cfg(...)'.dependencies]` n'est évaluée que
par une compilation pour sa cible ; un `cargo check` Linux vert ne dit rien
d'un `cfg(target_os)` qui n'est pas le sien.

mika#2900 a livré la moitié Linux de la garde
(`crates/mika-agent/tests/libc_target_scope_2900.rs`, qui prouve la
**déclaration**). Ce ticket livre l'autre moitié, qui prouve la **résolution**
pour darwin. Le pilote ne pouvait pas la livrer : toucher `.github/workflows/`
demande le scope `workflow`.

## Requirements

- **R1.** Un job `darwin-check` existe dans `ci.yml`, **mot pour mot** celui du
  corps de mika#2902 (le ticket fait foi). Il tourne sur `macos-14`, en matrice
  `aarch64-apple-darwin` / `x86_64-apple-darwin`, `fail-fast: false`, et lance
  `cargo check --locked --target "$TARGET" -p mika-agent -p mika-cli`.
- **R2.** Les SHA des actions sont exactement ceux déjà utilisés dans `ci.yml` :
  `actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1` (40 occurrences
  dans `ci.yml`) et `Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6`
  (`ci.yml:65`). Vérifié avant écriture : ceux du ticket sont identiques, aucun
  alignement n'est nécessaire.
- **R3.** Aucun autre job ne change : le diff de `ci.yml` est un ajout pur.
- **R4.** Aucun lint de dépôt qui lit les workflows ne rougit, et aucun
  registre n'est à compléter (voir D3).
- **R5.** La procédure du rouge-avant (AC2) est écrite et préparée sans être
  fabriquée : elle ne se prouve que sur un runner macOS.

## Non-objectifs

- **Prouver le comportement darwin.** `read_process_start_time` lit `/proc`,
  `cargo_build_lock_is_free` sonde avec `flock` ; ces replis Linux-only restent
  ce qu'ils sont (hors portée, dit le ticket).
- **Faire tourner le job sur Linux.** `cargo check --target <darwin>` depuis
  Linux exécute les build scripts, et `libsqlite3-sys` y compile du C pour la
  cible : sans toolchain Apple, le job rougirait pour une autre raison que celle
  qu'il surveille.
- **Modifier les déclencheurs de `ci.yml`** pour faciliter le rouge-avant.
- **Couvrir la feature `telemetry`.** `release.yml:168-169` construit avec
  `--features telemetry` ; le job du ticket vérifie les features par défaut.
  Aucune table `[target.*]` n'est aujourd'hui portée par cette feature
  (`crates/mika-common/Cargo.toml:54` ne fait qu'activer des dépendances
  optionnelles sans clé de cible), donc l'écart ne cache rien de cette classe à
  date. C'est une observation, pas un changement de ce ticket.

## Approche

### D1 — Le job, collé tel quel

Inséré juste après le job `check` (`ci.yml:20-235`), avant `dashboard` : c'est
le voisin naturel d'un job qui fait `cargo check` d'une autre plateforme. Aucun
`needs:`, donc aucun effet sur l'ordonnancement ni la durée des autres jobs
(AC3). Il hérite des `env:` globaux (`CARGO_TERM_COLOR`, `CARGO_INCREMENTAL: 0`),
des `permissions: contents: read` et de la `concurrency` du workflow.

`prefix-key: v1-darwin-check` sépare son cache de `v1-rust` (job `check`) et de
`v1-release` (`release.yml`) : aucun job existant ne voit son cache changer.

### D2 — Pourquoi `macos-14` et la cross-compilation x86_64

`release.yml:132-136` construit déjà les deux cibles sur `macos-14` ;
`x86_64-apple-darwin` y est cross-compilée depuis arm64 parce que `macos-13`
n'a jamais obtenu de runner (`release.yml:20-23` : trois runs annulés après 24 h
d'attente). Le job reprend exactement ce couple, pour que la CI évalue le même
ensemble de cibles que la release.

### D3 — Les lints qui lisent les workflows

Recensement de `scripts/check-*` et `scripts/verify-*` qui citent `workflows`
ou `ci.yml` :

| script | ce qu'il lit | concerné ? |
|---|---|---|
| `check-image-tags-immutable.sh` | `agent-image-build-push.yml` (`:82`) | non — aucun tag d'image |
| `check-suite-wiring.sh` | les suites sous `scripts/` et `skills/bundled/_shared/` atteintes par un workflow | non — le job n'ajoute ni ne retire de suite ; `suite-wiring.tsv` reste inchangé |
| `check-secrets.sh` | le diff | à lancer |
| `check-release-debuginfo.sh`, `check-eval-ignored-flags.sh`, `check-dispatch-seats-declared.sh`, `check-pilot-turn-ceiling-labels.sh`, `check-issue-comment-patch-guard.sh`, `verify-egress-manifest.sh` | des appels de script ou des labels | à lancer, aucun effet attendu |

Aucun fichier du dépôt ne tient de liste des jobs de `ci.yml` (recherche des
noms de jobs hors `ci.yml` : seules des mentions en prose dans `docs/`). Rien à
déclarer.

### D4 — Le rouge-avant : pourquoi une PR vers `main` ne le prouve pas

`ci.yml` se déclenche sur `push` vers `main` et sur `pull_request` (toute base),
sans `workflow_dispatch` (`ci.yml:3-6`). Sur un `pull_request`,
`actions/checkout` récupère `refs/pull/<N>/merge` : la fusion de la tête **dans
la base**. Une PR vers `main` contiendrait donc le correctif de mika#2900, déjà
sur `main`, et le job serait vert. Ce ne serait pas une preuve.

Procédure retenue, sans toucher aux déclencheurs :

1. une branche **base** `test/2902-rouge-avant-base` à `3993f91d` (pré-correctif ;
   son arbre ne diffère de `bc92d236`, sur `main`, que par un fichier de plan,
   et `crates/mika-agent/Cargo.toml:76` y porte encore
   `cfg(target_os = "linux")`) ;
2. une branche **tête** `test/2902-rouge-avant` = `3993f91d` + le commit du job,
   cherry-pické (`ci.yml` est identique entre `3993f91d` et `origin/main`, le
   cherry-pick est propre) ;
3. samidarko pousse les deux, ouvre une PR **brouillon** tête → base ; le
   `pull_request` fusionne dans un arbre pré-correctif, et `darwin-check` doit
   reproduire l'erreur `E0433` de l'AC2 sur les deux cibles ;
4. transcription de la sortie dans le corps de la vraie PR, puis fermeture de
   la PR brouillon et suppression des deux branches distantes.

Coût à savoir : la PR brouillon déclenche **tout** `ci.yml`, y compris `Check`
sur le runner gentux. Les autres jobs n'ont pas à être lus ; seuls les deux
`Darwin Check` comptent.

Les deux branches sont créées localement par ce vol, jamais poussées.

## Fichiers

- `.github/workflows/ci.yml` — ajout du job `darwin-check`.
- `docs/plans/2026-10-24-001-fix-2902-job-darwin-check-dans-ci-plan.md` — ce plan.

Pas de test à écrire : le job **est** la garde, et son rouge-avant est la
procédure D4. Le détecteur hors ligne existe déjà (mika#2900).

## Vérification

- **V1.** `git diff origin/main -- .github/workflows/ci.yml` ne contient que
  des lignes ajoutées, toutes dans le bloc `darwin-check`.
- **V2.** Le YAML se parse, et le job `darwin-check` y apparaît avec
  `runs-on: macos-14` et les deux cibles.
- **V3.** Les SHA des deux actions du job sont présents ailleurs dans `ci.yml`.
- **V4.** Les lints de D3 passent, `check-suite-wiring.sh` compris.
- **V5.** `scripts/verify-pipeline.sh origin/main` passe.
- **V6 (après push, par samidarko et MPC).** Sur la vraie PR, les deux
  `Darwin Check` sont verts (AC1) ; sur la PR brouillon de D4, les deux sont
  rouges avec `E0433` (AC2) ; les autres jobs gardent leur verdict (AC3).

## Definition of Done

Le job est dans `ci.yml`, V1-V5 sont verts en local, la branche du rouge-avant
existe localement, le corps de PR est écrit avec un emplacement nommé pour le
transcript du rouge, et le tout est commité sans push.

## Acceptance criteria

- [ ] **AC1** — le job existe dans `ci.yml`, tourne sur chaque `pull_request`, et passe sur `main` après le correctif de mika#2900.
- [ ] **AC2** — rouge-avant. Lancé sur l'arbre **pré-correctif** (`3993f91d`, ou tout commit antérieur à `489cab0d`), le job reproduit exactement l'erreur du tag :
  ```
  error[E0433]: failed to resolve: use of unresolved module or unlinked crate `libc`
      --> crates/mika-agent/src/skills/executor.rs:1430:27
      --> crates/mika-agent/src/skills/executor.rs:1444:39
  ```
  Sortie à transcrire dans la PR : un squash merge efface le `git log` intermédiaire, donc la preuve du rouge-avant doit vivre dans le corps de PR.
- [ ] **AC3** — aucun autre job de `ci.yml` ne change de durée ni de verdict.

## Assumptions

- La matrice `macos-14` obtient un runner dans un délai normal ; `release.yml`
  l'utilise déjà pour les deux cibles.
- La branche distante `origin/fix/2900/build-mika-agent-compile-pour-macos-libc`
  porte encore `3993f91d` ; si elle disparaît, la base locale suffit au push,
  et `bc92d236` (sur `main`) est une base équivalente.
