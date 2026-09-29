---
module: dispatch
tags: [dispatch-lib, cargo, disque, substrat-de-boucle, bwrap]
problem_type: resource-exhaustion
category: workflow-issues
type: fix
remeasured: 2026-09-29
remeasured_against: origin/main @ f236b5a6
---

# Plan : `CARGO_INCREMENTAL=0` atteint le build d'un spawn, sur les deux chemins (mika#2105)

**Ticket :** mika issue#2105
**Labels :** `bug`, `p1-important` — milestone *Substrat de boucle*
**Branche :** `fix/2105/dispatch-un-spawn-empile-debug-tests`
**Remplace :** le plan du 2026-09-03 (`…-profil-unique-build-spawn-plan.md`), jamais commité, arrêté
au checkpoint Phase 2.5 sur divergence de prémisse. Il visait le profil `release` ; la mesure a
déplacé le levier.

**Re-groom du 2026-09-29.** Ce plan date du 2026-09-09 (archivé à `archive/2105-plan-2026-09-09` @
`e73dd918`). Vingt jours et une quarantaine de commits sur `dispatch-lib.sh` plus tard (5 563 →
9 735 lignes), ses prémisses ont été re-mesurées contre `origin/main` @ `f236b5a6` avant tout
architecte. Relevé en section *Re-mesure du 2026-09-29* ; les numéros de ligne de tout le document
sont ceux de cette révision.

---

## Re-mesure du 2026-09-29

| prémisse du 09-09 | état au 09-29 | preuve |
|---|---|---|
| `debug/incremental` domine le `target/` d'un spawn (~42 %) | **tient** — 42,8 % | tableau ci-dessous |
| `CARGO_INCREMENTAL` n'atteint aucun chemin de dispatch | **tient** | `git grep CARGO_INCREMENTAL origin/main` : seulement `.github/workflows/{ci,eval-calibration}.yml` et deux docs ; absent de l'environnement des processus `mika` de l'hôte |
| sandbox : `--clearenv` + réinjection par `_PILOT_SANDBOX_ENV_ALLOWLIST`, test `[ -n "${!var:-}" ]` | **tient** — ancres déplacées | allowlist `dispatch-lib.sh:1021-1024`, boucle `:1615-1619`, `--clearenv` `:1684` (Phase 2b) et `:1768` (Phase 2a) — deux sites, alimentés par la même boucle |
| direct : opt-out `MIKA_PILOT_SANDBOX=0` et `bwrap` absent | **tient** — ancres déplacées | `_run_pilot_sandboxed` `:1372` ; branche opt-out `:1383-1386` ; `bwrap` absent `:1393-1396` |
| `address-pr-comments` et `resolve-pr-conflicts` lancent `claude-pilot` sans `dispatch-lib` | **tient** | `claude-pilot` direct à `address-pr-comments/handlers/run.sh:286` et `resolve-pr-conflicts/handlers/run.sh:336` ; `dispatch-lib.sh` n'y apparaît qu'en commentaire (`:55`, `:90`), jamais sourcé |
| **le mécanisme compte trois sites** | **faux, déjà au 09-09** — il en compte **quatre** | `AUDITED_SETENV_NAMES` (`skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh:193`, déjà présent au 09-09 à `:157`) — second registre deny-by-default sur les noms qui traversent `--setenv`, gardé par `make test-sandbox-secret-argv` (`Makefile:223`, CI `ci.yml:113`) |

**Taille réelle de l'incrémental, worktrees de spawn vivants du 2026-09-29** (`du -sm`) :

| worktree | `target/` | `debug/deps` | `debug/incremental` | `release` |
|---|---:|---:|---:|---:|
| `fix-2573-…` | 42 810 M | 20 579 M | **19 903 M (46,5 %)** | 1 498 M |
| `fix-2578-…` | 15 056 M | 9 451 M | **5 468 M (36,3 %)** | 0 M |
| `fix-2105-…` | 5 781 M | 4 121 M | **1 462 M (25,3 %)** | 0 M |
| `feat-2408-…` | 2 517 M | 879 M | **1 462 M (58,1 %)** | 0 M |
| **total (spawns debug)** | **66 164 M** | 35 030 M | **28 295 M (42,8 %)** | 1 498 M |

Un cinquième worktree (`fix-2532-…`, 1 521 M) ne porte que du `release` et n'entre pas dans le
ratio. `/data` est à 59 % (148 G libres) contre 80 % au 09-09 : la pression immédiate a baissé,
le poste relatif est inchangé à quatre dixièmes de point près. Un worktree isolé monte désormais
à 42,8 G, au-dessus de la fourchette 19–31 G du corps.

**Conséquence.** Les prémisses causales tiennent ; la dérive est celle des ancres, plus un défaut
du plan d'origine (un registre oublié), corrigé ci-dessous. Pas de divergence à arbitrer.

---

## Problème

Un worktree de spawn pèse 19 à 43 G de `target/`. Le poste dominant est `debug/incremental/` :
**42,4 %** au 2026-09-09 (53 603 M sur 126 356 M, cinq worktrees), **42,8 %** au 2026-09-29
(28 295 M sur 66 164 M, quatre worktrees disjoints des premiers). C'est l'état de compilation
incrémentale de Cargo, qui n'a de valeur que pour des rebuilds successifs du *même* arbre sur la
durée. Un worktree de spawn est construit une à trois fois puis jeté : cet état est produit
intégralement et **jamais** réutilisé. C'est le seul poste du `target/` qui soit du déchet pur
plutôt qu'un artefact dont le pipeline dépend.

**Quatre handlers lancent `claude-pilot` dans un worktree qui compile**, et ils ne sont pas de
même régime :

| chemin | passe par `_run_pilot_sandboxed` | régime du worktree |
|---|---|---|
| `dev-pilot/handlers/run.sh` | oui (via `_run_claude_pilot`, `dispatch-lib.sh:3698`) | **créé puis jeté** |
| `dev-groom/handlers/run.sh` | oui (idem, plus les relances `:7007` et `:7129`) | **créé puis jeté** |
| `address-pr-comments/handlers/run.sh:286` | non — `claude-pilot` direct, sans bwrap | **réutilisé** sur la durée d'une PR |
| `resolve-pr-conflicts/handlers/run.sh:336` | non — idem | **réutilisé** |

Aujourd'hui, `CARGO_INCREMENTAL` n'atteint le build par aucun de ces chemins :

- **Chemin sandboxé** — `bwrap --clearenv` (`dispatch-lib.sh:1684` en Phase 2b, `:1768` en
  Phase 2a). Toute variable absente de `_PILOT_SANDBOX_ENV_ALLOWLIST` (`:1021`) est supprimée. Même
  exportée côté hôte, `CARGO_INCREMENTAL` n'entre pas dans le sandbox.
- **Chemin direct** — deux sorties `"$@"` sans bwrap dans `_run_pilot_sandboxed` : opt-out
  `MIKA_PILOT_SANDBOX=0` (`:1383-1386`) et `bwrap` absent du PATH (`:1393-1396`). L'env de l'hôte
  est hérité tel quel — et l'hôte ne pose rien.

---

## Décision — le point d'application, et pourquoi celui-là

**Un site pose le réglage ; trois registres l'autorisent.**

1. `export CARGO_INCREMENTAL=0` **en tête de `_run_pilot_sandboxed`** (`dispatch-lib.sh:1372`),
   **avant** le test `_pilot_sandbox_enabled` (`:1383`). C'est la fonction que les trois sites de
   lancement de `dispatch-lib` traversent (`:3698`, `:7007`, `:7129`) — le même argument de
   placement que la ligne de budget `_emit_pilot_budget_line` (mika#2496), qui y vit déjà. Placé
   sous la branche, l'export n'atteindrait que le chemin sandboxé : **l'ordre est porteur**.
2. `CARGO_INCREMENTAL` ajouté à `_PILOT_SANDBOX_ENV_ALLOWLIST` (`dispatch-lib.sh:1021-1024`), avec
   une puce dans le bloc d'audit mika#2039 R6 (`:1007-1020`).
3. `CARGO_INCREMENTAL` ajouté à `EXPECTED_ENV_ALLOWLIST`
   (`scripts/verify-no-secret-in-setenv.sh:53`, liste triée), **sans quoi `make
   verify-no-secret-in-setenv` rougit** (règle 1, `:184-204`).
4. `CARGO_INCREMENTAL` ajouté à `AUDITED_SETENV_NAMES`
   (`skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh:193`, liste triée), avec son
   entrée d'audit dans le commentaire qui précède (`:168-192`, même forme que les ajouts mika#2141
   et mika#2572), **sans quoi `make test-sandbox-secret-argv` rougit** sur les assertions « 2b: no
   unaudited name travels by --setenv » et « 2b: an unknown-vendor secret cannot ride --setenv ».

Le chemin direct hérite de l'export. Le chemin sandboxé le récupère via la boucle de réinjection
(`dispatch-lib.sh:1615-1619`), qui teste `[ -n "${!var:-}" ]` — et `-n "0"` est **vrai** en shell,
la valeur `0` n'étant pas la chaîne vide.

**Le registre oublié au 09-09.** Le plan d'origine comptait trois sites et affirmait qu'il n'y en
avait pas de quatrième. `AUDITED_SETENV_NAMES` existait déjà à la date du plan (`:157` au 09-09) ;
la vérification avait porté sur `scripts/`, pas sur `skills/bundled/_shared/tests/`. L'implémentation
qui a suivi le plan à la lettre (PR #2589) rougit exactement là. Ce n'est pas une dérive de vingt
jours : c'est la même classe d'erreur que celle corrigée au 09-09 pour `EXPECTED_ENV_ALLOWLIST` —
une vérification plus étroite que l'affirmation. D'où la règle ci-dessous.

**Règle d'énumération des registres.** Tout fichier qui cite `_PILOT_SANDBOX_ENV_ALLOWLIST` ou qui
tient une liste littérale de noms admis à `--setenv` est un registre. À l'implémentation,
`git grep -c 'ANTHROPIC_LOG_FILE' -- scripts skills` rend l'ensemble, puisque ce nom figure dans
chaque registre. Relevé au 09-29 : `dispatch-lib.sh`, `verify-no-secret-in-setenv.sh`,
`test_sandbox_no_secret_in_argv.sh` — les trois registres — plus `scripts/canary-pilot-containment:387`,
qui **exporte** la variable pour son propre sandbox de test et n'est pas un registre (il contrôle
les valeurs, pas les noms). Tout autre site que la commande rendrait à l'implémentation est à
traiter dans le même diff, pas à découvrir en CI.

**Pourquoi seulement les deux chemins `dispatch-lib`, et pas les quatre** (AC1). L'argument qui
porte ce plan est que le worktree est *jeté* : l'état incrémental est produit puis perdu. Cet
argument **ne tient pas** sur `address-pr-comments` et `resolve-pr-conflicts` : une PR reçoit
plusieurs rounds, et ces handlers recompilent le *même* worktree à chaque round — précisément le
régime où l'incrémental paie. Trois options ont été pesées :

- **A (retenue)** — les deux chemins jetables. Le gain porte là où le raisonnement tient.
- **B** — les quatre chemins. Gain maximal, mais paie un coût de rebuild répété sur les worktrees de
  PR, contre l'argument même du ticket. Ce qui ralentit la boucle (palier 1 du tri) coûte plus cher
  que ce qui la remplit (palier 2).
- **C** — A, plus un nettoyage du seul `incremental/` à la fermeture de la PR
  (`scripts/mika-platform-worktree-cleanup`) : second ticket si la mesure AC4 montre que les
  worktrees de PR pèsent encore.

Le périmètre est **nommé**, pas subi : AC1 couvre `dev-pilot` et `dev-groom`, et l'assertion D fait
échouer le garde si un cinquième chemin apparaît sans décision. La re-mesure du 09-29 confirme que
le placement dans `_run_pilot_sandboxed` n'atteint pas les deux handlers de PR : ils ne sourcent
pas `dispatch-lib.sh`, donc un export dans une fonction de ce fichier ne peut pas leur fuir.

**Effet de bord accepté du placement.** `export` dans une fonction modifie le shell appelant pour
la suite du handler : les étapes post-pilote de `dispatch-lib` qui compilent dans le même worktree
(vérification du pipeline de sauvetage mika#2354 : `cargo fmt --check`, `cargo clippy`) tournent
aussi sans incrémental. C'est le même worktree jetable, donc le même argument ; c'est voulu, pas
subi.

**Pourquoi pas `mika/.cargo/config.toml` avec `incremental = false`** (AC2) : ce fichier s'applique à
tout build du dépôt, y compris le checkout principal, où l'itération répétée sur le même arbre rend
l'incrémental utile. `dispatch-lib.sh` n'est sourcé **que** par des handlers de skills : porter le
réglage là satisfait AC2 par construction.

**Pourquoi pas une clause en prose dans `.claude/commands/mika.md`** (AC3) : un garde sur un
paragraphe ne garde aucun comportement —
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`. Le réglage doit être dans le
code qui lance le processus.

**Contrainte de sécurité — mécanique, pas documentaire.** Les deux registres de la décision (3 et 4)
sont **deny-by-default** : toute addition, quelle que soit l'apparence du nom, échoue tant qu'elle
n'est pas auditée. C'est délibéré (`verify-no-secret-in-setenv.sh:48-52`) : « confirm the variable
carries no credential material, note why in the audit comment above `_PILOT_SANDBOX_ENV_ALLOWLIST`,
then update this set ». Les trois gestes sont honorés :

- la variable ne porte aucun matériel de créance — c'est le littéral `0`, posé par une constante,
  jamais lu depuis l'environnement parent ;
- la raison est notée dans le bloc d'audit R6 **et** dans le commentaire d'audit de
  `test_sandbox_no_secret_in_argv.sh` ;
- les deux registres sont mis à jour dans le même diff.

---

## Phases

### Phase 1 — Poser le réglage sur les deux chemins (AC1, AC2)

- `dispatch-lib.sh:1372` : `export CARGO_INCREMENTAL=0` en tête de `_run_pilot_sandboxed`, **avant**
  `_pilot_sandbox_enabled`, avec un commentaire qui nomme le ticket, le chiffre mesuré et la raison
  (worktree éphémère → état incrémental jamais réutilisé) et marque l'ordre comme porteur.
- `dispatch-lib.sh:1021-1024` : ajouter `CARGO_INCREMENTAL` à `_PILOT_SANDBOX_ENV_ALLOWLIST`.
- `dispatch-lib.sh:1007-1020` : étendre le bloc d'audit R6 d'une puce pour cette variable.
- `scripts/verify-no-secret-in-setenv.sh:53` : ajouter `CARGO_INCREMENTAL` à
  `EXPECTED_ENV_ALLOWLIST` (liste triée).
- `skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh:193` : ajouter
  `CARGO_INCREMENTAL` à `AUDITED_SETENV_NAMES` (ordre alphabétique, entre `ANTHROPIC_LOG_FILE` et
  `CLAUDE_CODE_API_BASE_URL`), et une entrée d'audit au commentaire `:168-192` sur le modèle
  mika#2141 : `CARGO_INCREMENTAL  the literal "0", a build flag (mika#2105).`
- Appliquer la *règle d'énumération des registres* : aucun registre rendu par le `git grep` hors de
  ces trois.
- **Ne pas** créer ni modifier `mika/.cargo/config.toml`.

### Phase 2 — Le garde, avec son comportement négatif pinné (AC3)

Dans `skills/bundled/_shared/test-dispatch-lib.sh` (cible `make test-dispatch-lib`, `Makefile:185`,
câblée en CI par mika#1772, `ci.yml:85`) :

1. **Assertion A** — `CARGO_INCREMENTAL` figure dans `_PILOT_SANDBOX_ENV_ALLOWLIST`, avec un contrôle
   de bonne foi (le prédicat sait dire non).
2. **Assertion B** — l'export existe dans le code (commentaires exclus) et vaut `0`.
3. **Assertion B'** — l'export **précède** `_pilot_sandbox_enabled` dans le corps de
   `_run_pilot_sandboxed`. C'est elle qui garde réellement les deux chemins ; B seule passerait avec
   un export sous la branche.
4. **Assertion C** — `mika/.cargo/config.toml` ne définit pas `incremental` ; un fichier absent
   satisfait l'assertion.
5. **Assertion D** — les handlers qui lancent `claude-pilot` sans passer par `dispatch-lib.sh` sont
   exactement `{address-pr-comments, resolve-pr-conflicts}`, comparaison bidirectionnelle, avec un
   contrôle anti-vacuité du glob. Le prédicat de lancement réutilise
   `_mika2496_launch_candidates` (`test-dispatch-lib.sh:6942`) au lieu d'en recopier un.
6. **Comportement négatif pinné** — sur des copies mutées (export retiré ; entrée d'allowlist
   retirée ; export sous la branche ; `.cargo/config.toml` posant `incremental` ; cinquième chemin
   d'invocation), le prédicat correspondant rend **rouge**, et chaque mutation a son miroir vert.
   Modèle : `verify-egress-no-log` (`Makefile:254`) et `check-byte-slices` (`Makefile:264`).

Les deux registres deny-by-default (décision 3 et 4) portent déjà leur propre négatif : ils ont été
**vus rouges** par la CI de la PR #2589 avant l'ajout à `AUDITED_SETENV_NAMES`. Aucun garde
supplémentaire n'est requis pour eux.

Le contrôle positif et le contrôle négatif vivent dans le même appel de test
(`feedback_a_probe_needs_both_controls_in_the_same_call`).

### Phase 3 — La mesure, ventilée (AC4)

Deux relevés, dans cet ordre, parce que le premier spawn compilant qui suit la fusion n'existe pas
avant la fusion :

1. **Avant fusion, relevé contrôlé.** Les deux `target/` produits par le protocole de la Phase 4
   (`CARGO_INCREMENTAL=0` et `=1`, même commit, même séquence froid + rebuild) sont ventilés par
   poste. Le chiffre va dans le corps de la PR. Il isole l'effet du réglage de la variation de
   composition entre spawns, que les deux tableaux du ticket montrent forte.
2. **Après fusion, relevé réel.** Sur le **premier spawn compilant** qui suit la fusion et le
   déploiement de `dispatch-lib.sh` — un spawn documentaire ne touche pas `target/` — même
   ventilation, postée en commentaire sur la PR fusionnée et sur le ticket. C'est ce relevé qui
   ferme l'AC4 au sens du corps, et qui réveille les dormeurs du *Hors périmètre*.

| grandeur | commande |
|---|---|
| `target/` total | `du -sm <wt>/mika/target` |
| `debug/deps` | `du -sm <wt>/mika/target/debug/deps` |
| `debug/incremental` | `du -sm <wt>/mika/target/debug/incremental` |
| `release` | `du -sm <wt>/mika/target/release` |

Références *avant* : le tableau du corps (2026-09-09, 42,4 %) et celui de la re-mesure (2026-09-29,
42,8 %). Attendu : `debug/incremental` **absent ou négligeable**. Si le gain mesuré s'écarte de
l'attendu, **c'est le chiffre mesuré qui est publié**, avec la commande qui l'a produit.

### Phase 4 — L'arbitrage disque/temps, chiffré (AC6)

Protocole sur le worktree de la PR, une fois le travail fonctionnel terminé :

1. `cargo test` à froid (`CARGO_TARGET_DIR` vierge), `CARGO_INCREMENTAL=0` — relever la durée.
2. Modification d'une ligne dans un crate feuille, `cargo test` à nouveau — relever la durée du
   rebuild.
3. Mêmes deux mesures avec `CARGO_INCREMENTAL=1` sur un second `CARGO_TARGET_DIR` temporaire.

Les quatre durées vont dans la PR. **Critère de renoncement explicite, fixé au 09-09 avant toute
mesure :** si le rebuild incrémental désactivé coûte plus de **+50 %** sur l'étape 2, le gain disque
ne justifie pas le coût de boucle et le travail remonte à l'opérateur **au lieu de fusionner**.

Contrainte d'hôte : le protocole lance deux builds complets. Il respecte le plafond de deux builds
`cargo` concurrents sur l'hôte et nettoie les deux `CARGO_TARGET_DIR` temporaires après relevé.

### Phase 5 — La porte de qualité intacte (AC5)

Sur la PR, attester que `cargo test`, `cargo clippy --all-targets --all-features` (via `pre-commit`,
`lefthook.yml:15-17`) et `cargo fmt --check` tournent et passent, ainsi que les trois cibles de
garde touchées : `make test-dispatch-lib`, `make test-sandbox-secret-argv`,
`make verify-no-secret-in-setenv`. Sorties jointes à la PR. Si une étape casse, la nommer et la
traiter — jamais réintroduire le réglage en silence.

---

## Fire-Disposition

Les livrables de la Phase 2 sont des **détecteurs de régression** (assertions A/B/B'/C/D +
comportement négatif pinné). Leur disposition quand ils tirent, spécifiée d'avance (mika#1574
Fire-Disposition Gate) :

**Disposition : (c) halt-and-surface, via gate CI bloquant.** `make test-dispatch-lib` échoue et la
CI bloque la fusion dès que la configuration diverge de l'invariant — export retiré ou déplacé sous
la branche de sandbox, entrée d'allowlist retirée, `incremental` réapparu dans un
`.cargo/config.toml`, ou nouveau chemin d'invocation non déclaré.

**Violations pré-existantes : aucune.** Le garde naît avec le comportement qu'il garde : ni période
de grâce, ni allowlist de dérogations, ni tri de dette avant d'activer le gate.

**Pourquoi halt-and-surface et pas warn-only :** l'invariant est binaire et bon marché à satisfaire.
Un avertissement laisserait le gain se perdre en silence — exactement le mode d'échec que l'AC1
nomme.

## Definition of Done

- [ ] `export CARGO_INCREMENTAL=0` en tête de `_run_pilot_sandboxed`, avant `_pilot_sandbox_enabled`,
      commenté avec ticket + chiffre + ordre porteur.
- [ ] `CARGO_INCREMENTAL` dans `_PILOT_SANDBOX_ENV_ALLOWLIST`, bloc d'audit R6 étendu.
- [ ] `CARGO_INCREMENTAL` dans `EXPECTED_ENV_ALLOWLIST` ; `make verify-no-secret-in-setenv` vert.
- [ ] `CARGO_INCREMENTAL` dans `AUDITED_SETENV_NAMES` avec entrée d'audit ;
      `make test-sandbox-secret-argv` vert.
- [ ] Règle d'énumération des registres appliquée ; aucun registre hors des trois nommés.
- [ ] Aucun `mika/.cargo/config.toml` créé ni modifié.
- [ ] Assertions A/B/B'/C/D + négatifs pinnés dans `test-dispatch-lib.sh` ;
      `make test-dispatch-lib` vert, et rouge sur chaque mutation.
- [ ] Section `## Fire-Disposition` présente et honorée par le câblage CI.
- [ ] Relevé contrôlé ventilé (4 postes × 2 réglages, commandes jointes) dans le corps de la PR.
- [ ] Quatre durées de l'arbitrage disque/temps dans la PR, verdict contre le seuil de +50 %.
- [ ] `cargo test`, `clippy --all-targets --all-features`, `fmt --check` verts, sorties jointes.
- [ ] Après fusion : relevé du premier spawn compilant posté sur la PR et le ticket.

## Acceptance criteria

- [ ] **AC1** — `CARGO_INCREMENTAL=0` atteint réellement le build d'un spawn, sur les deux chemins de
      dispatch : sandboxé (`--setenv` malgré `--clearenv`, `dispatch-lib.sh:1684`/`:1768`) et direct
      (`dispatch-lib.sh:1383-1386` et `:1393-1396` ; le corps cite `:916`/`:928`, ancres du 09-09).
- [ ] **AC2** — Le réglage ne quitte pas le dispatch : pas de `mika/.cargo/config.toml`, checkout
      principal inchangé, vérifiable.
- [ ] **AC3** — Un garde structurel, pas une clause en prose, avec le comportement négatif pinné.
- [ ] **AC4** — Le gain est mesuré et ventilé par poste, comparé au tableau du ticket : relevé
      contrôlé dans la PR avant fusion, relevé du premier spawn compilant après fusion.
- [ ] **AC5** — Le pipeline ne perd rien : tests, clippy, fmt, et les trois gardes touchés passent.
- [ ] **AC6** — L'arbitrage disque/temps est chiffré, avec un seuil de renoncement nommé d'avance.

## Rattachement aux critères d'acceptation

| AC | Phase(s) | Preuve |
|---|---|---|
| AC1 | 1 | Export en tête de `_run_pilot_sandboxed` + entrée d'allowlist ; les deux chemins couverts par construction (héritage / réinjection), ordre gardé par B' |
| AC2 | 1, 2 | Absence de `.cargo/config.toml` + assertion C du garde |
| AC3 | 2 | Assertions A/B/B'/C/D dans `test-dispatch-lib.sh`, négatif pinné par mutation |
| AC4 | 3 | Relevé contrôlé ventilé avant fusion ; relevé du premier spawn compilant après fusion |
| AC5 | 5 | Sorties `cargo test` / `clippy` / `fmt --check` + trois cibles de garde |
| AC6 | 4 | Quatre durées + verdict contre le seuil +50 % nommé avant la mesure |

## Hors périmètre

Repris du corps du ticket, avec la même condition de réveil — **quand l'AC4 aura rendu son relevé
ventilé du premier spawn compilant après fusion** :

- La double passe `rustc`/`clippy` (~19 %) — surface distincte (invocation de clippy, pas
  environnement de dispatch).
- Le profil unique `debug` (~6,7 %) — pas de point d'application structurel.
- Le nettoyage a posteriori des `target/` — `scripts/mika-platform-worktree-cleanup` le fait déjà.
- Un `CARGO_TARGET_DIR` partagé — décision d'architecture, pas d'hygiène.
- `wizzard` et les `target/` des checkouts principaux — arbitrage opérateur.

## Risques

| Risque | Effet | Traitement |
|---|---|---|
| Le rebuild non incrémental ralentit les spawns | Ralentit la boucle (palier 1 > palier 2) | Phase 4 : quatre durées + seuil de renoncement +50 % nommé d'avance |
| Un registre deny-by-default n'est pas mis à jour | CI rouge, cause non évidente | Décision : trois registres nommés ; règle d'énumération par `git grep` ; DoD case par registre. **Réalisé une fois** (PR #2589, `AUDITED_SETENV_NAMES`) |
| L'export glisse sous la branche de sandbox | Chemins directs perdus en silence | Assertion B' + négatif N3 |
| Le premier spawn suivant ne compile pas | Relevé post-fusion retardé | Relevé contrôlé avant fusion (Phase 3.1) ; le post-fusion attend un spawn compilant |
| Les handlers de PR ne passent pas par `dispatch-lib` | Gain nul sur les worktrees de PR | Hors périmètre par décision (option A), assertion D |
| Un cinquième chemin apparaît plus tard | Gain perdu en silence | Assertion D bidirectionnelle |
| La branche dérive encore de `main` avant fusion | Conflit dans `dispatch-lib.sh` (fichier très actif) | Rebaser sur `main` avant la revue finale ; re-vérifier que l'export reste en tête de `_run_pilot_sandboxed` |

## Références (origin/main @ f236b5a6)

- `mika/skills/bundled/_shared/dispatch-lib.sh:1007-1020` — bloc d'audit mika#2039 R6
- `…:1021-1024` — `_PILOT_SANDBOX_ENV_ALLOWLIST`
- `…:1372` — `_run_pilot_sandboxed` ; `:1383-1386` opt-out ; `:1393-1396` `bwrap` absent
- `…:1615-1619` — boucle de réinjection `--setenv`, test `[ -n "${!var:-}" ]`
- `…:1684`, `:1768` — `--clearenv` (Phase 2b, Phase 2a)
- `…:3698`, `:7007`, `:7129` — les trois sites de lancement qui traversent `_run_pilot_sandboxed`
- `mika/scripts/verify-no-secret-in-setenv.sh:48-52` — les trois gestes exigés ; `:53` —
  `EXPECTED_ENV_ALLOWLIST` ; `:184-204` — règle 1
- `mika/skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh:168-192` — audit ; `:193` —
  `AUDITED_SETENV_NAMES`
- `mika/skills/bundled/address-pr-comments/handlers/run.sh:286`,
  `mika/skills/bundled/resolve-pr-conflicts/handlers/run.sh:336` — lancements hors `dispatch-lib`
- `mika/Makefile:185` `test-dispatch-lib` ; `:223` `test-sandbox-secret-argv` ; `:250`
  `verify-no-secret-in-setenv` ; `:254`, `:264` — modèles de garde à négatif pinné
- `mika/.github/workflows/ci.yml:85`, `:113-114` — câblage CI des gardes
- `lefthook.yml:15-17` — `cargo clippy --all-targets --all-features -- -D warnings` en pre-commit
- `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`,
  `feedback_a_probe_needs_both_controls_in_the_same_call`,
  `feedback_estimated_counts_undercount_measured`
