---
module: dispatch
tags: [dispatch-lib, cargo, disque, substrat-de-boucle, bwrap]
problem_type: resource-exhaustion
category: workflow-issues
type: fix
---

# Plan : `CARGO_INCREMENTAL=0` atteint le build d'un spawn, sur les deux chemins (mika#2105)

**Ticket :** mika issue#2105
**Labels :** `bug`, `p1-important`, `dispatch:mpc` — milestone *Substrat de boucle*
**Branche :** `fix/2105/dispatch-un-spawn-empile-debug-tests`
**Remplace :** le plan du 2026-09-03 (`…-profil-unique-build-spawn-plan.md`), jamais commité, arrêté
au checkpoint Phase 2.5 sur divergence de prémisse. Il visait le profil `release` ; la mesure a
déplacé le levier.

---

## Problème

Un worktree de spawn pèse 19 à 31 G de `target/`. À 73 G libres sur `/data`, cela plafonne à deux
spawns simultanés — une contrainte d'ordonnancement, donc un défaut de substrat de boucle.

Le poste dominant est `debug/incremental/` : **42,4 %** du total sur les cinq worktrees vivants du
2026-09-09 (53 603 M sur 126 356 M). C'est l'état de compilation incrémentale de Cargo, qui n'a de
valeur que pour des rebuilds successifs du *même* arbre sur la durée. Un worktree de spawn est
construit une à trois fois puis jeté : cet état est produit intégralement et **jamais** réutilisé.
C'est le seul poste du `target/` qui soit du déchet pur plutôt qu'un artefact dont le pipeline dépend.

**Quatre chemins invoquent `claude-pilot` dans un worktree qui compile**, et ils ne sont pas de
même régime :

| chemin | source `dispatch-lib` | régime du worktree |
|---|---|---|
| `dev-pilot/handlers/run.sh` | oui | **créé puis jeté** |
| `dev-groom/handlers/run.sh` | oui | **créé puis jeté** |
| `address-pr-comments/handlers/run.sh:254` | non — `claude-pilot` direct, sans bwrap | **réutilisé** sur la durée d'une PR |
| `resolve-pr-conflicts/handlers/run.sh:196` | non — idem, worktree via `derive-worktree-path` | **réutilisé** |

Aujourd'hui, `CARGO_INCREMENTAL` n'atteint le build par aucun de ces chemins. Sur les deux premiers :

- **Chemin sandboxé** — `dispatch-lib.sh` lance le pilote sous `bwrap --clearenv` (ligne 1144). Toute
  variable non listée dans `_PILOT_SANDBOX_ENV_ALLOWLIST` (ligne 563) est supprimée. Même exportée
  côté hôte, `CARGO_INCREMENTAL` n'entre pas dans le sandbox.
- **Chemin direct** — deux sorties `"$@"` sans bwrap : opt-out `MIKA_PILOT_SANDBOX=0`
  (`dispatch-lib.sh:916`) et `bwrap` absent du PATH (ligne 927). Là, l'env de l'hôte est hérité tel
  quel — donc rien n'est posé non plus.

---

## Décision — le point d'application, et pourquoi celui-là

**Deux lignes dans `dispatch-lib.sh` couvrent les deux chemins par construction :**

1. `export CARGO_INCREMENTAL=0` au setup du dispatch, avant l'appel au wrapper d'invocation.
2. `CARGO_INCREMENTAL` ajouté à `_PILOT_SANDBOX_ENV_ALLOWLIST`.

Le chemin direct hérite de l'export. Le chemin sandboxé le récupère via la boucle de réinjection
(`dispatch-lib.sh:1074-1078`), qui teste `[ -n "${!var:-}" ]` — et `-n "0"` est **vrai** en shell,
la valeur `0` n'étant pas la chaîne vide. Aucun troisième site à tenir synchronisé.

**Pourquoi seulement les deux chemins `dispatch-lib`, et pas les quatre** (AC1). L'argument qui
porte ce plan est que le worktree est *jeté* : l'état incrémental est produit puis perdu, donc c'est
du déchet pur. Cet argument **ne tient pas** sur `address-pr-comments` et `resolve-pr-conflicts` :
une PR reçoit plusieurs rounds, et ces handlers recompilent le *même* worktree à chaque round —
précisément le régime où l'incrémental paie. Trois options ont été pesées :

- **A (retenue)** — les deux chemins jetables. Le gain porte là où le raisonnement tient.
- **B** — les quatre chemins. Gain maximal, mais paie un coût de rebuild répété sur les worktrees de
  PR, contre l'argument même du ticket. Ce serait appliquer la bonne conclusion au mauvais
  raisonnement, et ce qui ralentit la boucle (palier 1 du tri) coûte plus cher que ce qui la remplit
  (palier 2).
- **C** — A, plus un nettoyage du seul `incremental/` à la fermeture de la PR. Rend le disque sans
  coûter de temps de rebuild pendant la vie de la PR. Surface différente
  (`scripts/mika-platform-worktree-cleanup`) : second ticket si la mesure AC4 montre que les
  worktrees de PR pèsent encore.

Le périmètre est donc **nommé**, pas subi : AC1 couvre `dev-pilot` et `dev-groom`, et l'assertion D
de la Phase 2 fait échouer le garde si un cinquième chemin apparaît sans décision.

**Pourquoi pas `mika/.cargo/config.toml` avec `incremental = false`** (AC2) : ce fichier s'applique à
tout build du dépôt, y compris le checkout principal, où l'itération répétée sur le même arbre rend
l'incrémental utile — et où le ticket réserve l'arbitrage à l'opérateur. `dispatch-lib.sh` n'est
invoqué **que** par le dispatch : porter le réglage là satisfait AC2 par construction, pas par
discipline.

**Pourquoi pas une clause en prose dans `.claude/commands/mika.md`** (AC3) : c'est la forme qu'avait
prise le plan du 09-03, avec un garde qui vérifiait la présence du paragraphe. Un garde sur un
paragraphe ne garde aucun comportement — `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`.
Le réglage doit être dans le code qui lance le processus.

**Contrainte de sécurité à honorer.** Le bloc d'audit `dispatch-lib.sh:549-562` (mika#2039 R6) exige
que toute valeur atteignant `--setenv` soit **pesée et documentée**, parce que `--setenv NAME VALUE`
place la valeur dans l'argv de bwrap et que `/proc/<pid>/cmdline` est lisible par tous. Ajouter une
entrée à l'allowlist sans étendre cet audit viole la discipline même si la valeur est anodine. Le
plan étend le bloc : `CARGO_INCREMENTAL` — littéral `0`, drapeau de build, aucun matériel de
créance. `scripts/verify-no-secret-in-setenv.sh` reste vert.

---

## Phases

### Phase 1 — Poser le réglage sur les deux chemins (AC1, AC2)

- `dispatch-lib.sh` : `export CARGO_INCREMENTAL=0` au setup du dispatch, avec un commentaire qui
  nomme le ticket, le chiffre mesuré (42,4 %) et la raison (worktree éphémère → état incrémental
  jamais réutilisé).
- `dispatch-lib.sh:563-566` : ajouter `CARGO_INCREMENTAL` à `_PILOT_SANDBOX_ENV_ALLOWLIST`.
- `dispatch-lib.sh:549-562` : étendre le bloc d'audit R6 d'une puce pour cette variable.
- **Ne pas** créer ni modifier `mika/.cargo/config.toml`.

### Phase 2 — Le garde, avec son comportement négatif pinné (AC3)

Dans `skills/bundled/_shared/test-dispatch-lib.sh` (suite existante, cible `make test-dispatch-lib`,
`Makefile:156`, déjà câblée en CI par mika#1772) :

1. **Assertion positive A** — `CARGO_INCREMENTAL` figure dans `_PILOT_SANDBOX_ENV_ALLOWLIST`.
2. **Assertion positive B** — l'export existe dans `dispatch-lib.sh` et vaut `0`.
3. **Assertion positive C** — `mika/.cargo/config.toml` ne définit pas `incremental` (AC2 vérifiée,
   pas seulement promise) ; le fichier peut être absent, ce qui satisfait aussi l'assertion.
4. **Assertion positive D** — les handlers qui invoquent `claude-pilot` **sans** passer par
   `dispatch-lib.sh` sont énumérés dans le garde (`address-pr-comments`, `resolve-pr-conflicts`).
   Un cinquième chemin ajouté plus tard fait échouer le test, forçant une décision de périmètre au
   lieu de perdre le gain en silence. C'est le risque « un troisième chemin apparaît » rendu
   structurel plutôt que confié à la vigilance en revue.
5. **Comportement négatif pinné** — sur une copie de `dispatch-lib.sh` privée de l'export, puis sur
   une copie privée de l'entrée d'allowlist, le garde doit **échouer**. Modèle exact :
   `verify-egress-no-log` (`Makefile:186`) et `check-byte-slices` (`Makefile:193`), qui pinnent tous
   deux leur négatif. Un garde qui passe encore une fois le réglage retiré est vide.

Le contrôle positif et le contrôle négatif vivent dans le même appel de test
(`feedback_a_probe_needs_both_controls_in_the_same_call`).

### Phase 3 — La mesure, ventilée (AC4)

Sur le **premier spawn compilant** qui suit la fusion — un spawn documentaire ne touche pas
`target/` et ne mesure rien :

| grandeur | commande |
|---|---|
| `target/` total | `du -sm <wt>/mika/target` |
| `debug/deps` | `du -sm <wt>/mika/target/debug/deps` |
| `debug/incremental` | `du -sm <wt>/mika/target/debug/incremental` |
| `release` | `du -sm <wt>/mika/target/release` |

Référence *avant* : le tableau du corps du ticket (2026-09-09, cinq worktrees ; incremental
53 603 M / 42,4 %). Attendu : `debug/incremental` **absent ou négligeable**, total en baisse d'un
tiers à moitié.

Si le gain mesuré s'écarte de l'attendu, **c'est le chiffre mesuré qui va dans la PR**, avec la
commande qui l'a produit — pas l'attendu. C'est la faute que l'AC4 nomme.

### Phase 4 — L'arbitrage disque/temps, chiffré (AC6)

Désactiver l'incrémental peut allonger les rebuilds successifs d'un même spawn. Protocole sur le
worktree de mesure, une fois le travail fonctionnel terminé :

1. `cargo test` à froid (`target/` vierge), `CARGO_INCREMENTAL=0` — relever la durée.
2. Modification d'une ligne dans un crate feuille, `cargo test` à nouveau — relever la durée du
   rebuild.
3. Mêmes deux mesures avec `CARGO_INCREMENTAL=1` sur un `target/` séparé (via `CARGO_TARGET_DIR`
   temporaire, pour ne pas polluer le worktree).

Les quatre durées vont dans la PR. **Critère de renoncement explicite :** si le rebuild incrémental
désactivé coûte plus de +50 % sur l'étape 2, le gain disque ne justifie pas le coût de boucle — le
palier 1 du tri prime sur le palier 2 — et le plan remonte à l'opérateur au lieu de fusionner.
Nommer le seuil avant de mesurer est ce qui empêche de le rationaliser après.

### Phase 5 — La porte de qualité intacte (AC5)

Sur le spawn de mesure, attester que `cargo test`, `cargo clippy --all-targets --all-features` (via
`pre-commit`, `lefthook.yml:16-18`) et `cargo fmt --check` tournent et passent ; sorties jointes à la
PR. Si une étape casse, la nommer et la traiter — jamais réintroduire le réglage en silence.

---

## Fire-Disposition

Les livrables de la Phase 2 sont des **détecteurs de régression** (assertions A/B/C/D + comportement
négatif pinné). Leur disposition quand ils tirent, spécifiée d'avance (mika#1574 Fire-Disposition
Gate) :

**Disposition : (c) halt-and-surface, via gate CI bloquant.** `make test-dispatch-lib` échoue et la
CI bloque la fusion dès que la configuration diverge de l'invariant — export retiré, entrée
d'allowlist retirée, `incremental` réapparu dans un `.cargo/config.toml`, ou nouveau chemin
d'invocation non déclaré.

**Violations pré-existantes : aucune.** Le réglage n'existe pas encore ; le garde naît avec le
comportement qu'il garde. Il n'y a donc ni période de grâce, ni allowlist de dérogations à porter,
ni tri de dette à faire avant d'activer le gate.

**Pourquoi halt-and-surface et pas warn-only :** l'invariant est binaire et bon marché à satisfaire
(deux lignes). Un avertissement laisserait le gain se perdre en silence — exactement le mode
d'échec que l'AC1 nomme.

## Definition of Done

- [ ] `export CARGO_INCREMENTAL=0` posé dans `dispatch-lib.sh`, commenté avec ticket + chiffre.
- [ ] `CARGO_INCREMENTAL` dans `_PILOT_SANDBOX_ENV_ALLOWLIST`, bloc d'audit R6 étendu.
- [ ] Aucun `mika/.cargo/config.toml` créé ni modifié.
- [ ] Les cinq assertions de la Phase 2 (A/B/C/D + négatif pinné) dans `test-dispatch-lib.sh`, négatif pinné, `make
      `make test-dispatch-lib` vert, et rouge quand le réglage est retiré.
- [ ] Section `## Fire-Disposition` présente et honorée par le câblage CI.
- [ ] Mesure ventilée du premier spawn compilant dans la PR (4 postes, avec les commandes).
- [ ] Quatre durées de l'arbitrage disque/temps dans la PR, verdict contre le seuil de +50 %.
- [ ] `cargo test`, `clippy --all-targets --all-features`, `fmt --check` verts, sorties jointes.

## Acceptance criteria

- [ ] **AC1** — `CARGO_INCREMENTAL=0` atteint réellement le build d'un spawn, sur les deux chemins de
      dispatch : sandboxé (`--setenv` malgré `--clearenv`) et direct (`dispatch-lib.sh:916` et `:927`).
- [ ] **AC2** — Le réglage ne quitte pas le dispatch : pas de `mika/.cargo/config.toml`, checkout
      principal inchangé, vérifiable.
- [ ] **AC3** — Un garde structurel, pas une clause en prose, avec le comportement négatif pinné.
- [ ] **AC4** — Le gain est mesuré et ventilé par poste, comparé au tableau du ticket, dans la PR.
- [ ] **AC5** — Le pipeline ne perd rien : tests, clippy, fmt passent.
- [ ] **AC6** — L'arbitrage disque/temps est chiffré, avec un seuil de renoncement nommé d'avance.

## Rattachement aux critères d'acceptation

| AC | Phase(s) | Preuve |
|---|---|---|
| AC1 | 1 | Export + entrée d'allowlist ; les deux chemins couverts par construction (héritage / réinjection) |
| AC2 | 1, 2 | Absence de `.cargo/config.toml` + assertion C du garde |
| AC3 | 2 | Quatre assertions dans `test-dispatch-lib.sh`, négatif pinné sur deux mutations distinctes |
| AC4 | 3 | Relevé `du -sm` ventilé en 4 postes, commandes jointes, comparé au tableau du corps |
| AC5 | 5 | Sorties `cargo test` / `clippy --all-targets --all-features` / `fmt --check` |
| AC6 | 4 | Quatre durées + verdict contre le seuil +50 % nommé avant la mesure |

## Hors périmètre

Repris du corps du ticket, avec la même condition de réveil — **quand l'AC4 aura rendu son chiffre
ventilé** :

- La double passe `rustc`/`clippy` (~19 %) — surface distincte (invocation de clippy, pas
  environnement de dispatch).
- Le profil unique `debug` (~6,7 %) — pas de point d'application structurel ; retiré au grooming du
  2026-09-09.
- Le nettoyage a posteriori des `target/` — `scripts/mika-platform-worktree-cleanup` le fait déjà.
- Un `CARGO_TARGET_DIR` partagé — décision d'architecture, pas d'hygiène.
- `wizzard` et les `target/` des checkouts principaux — arbitrage opérateur.

## Risques

| Risque | Effet | Traitement |
|---|---|---|
| Le rebuild non incrémental ralentit les spawns | Ralentit la boucle (palier 1 > palier 2) | Phase 4 : quatre durées + seuil de renoncement +50 % nommé d'avance |
| L'entrée d'allowlist ajoutée sans peser l'audit R6 | Discipline mika#2039 érodée par précédent | Phase 1 : le bloc d'audit est étendu dans le même diff |
| Le premier spawn suivant ne compile pas | AC4 non mesurable | Prérequis explicite en Phase 3 : attendre un spawn compilant |
| Deux chemins d'invocation (`address-pr-comments`, `resolve-pr-conflicts`) ne passent pas par `dispatch-lib` | Gain nul sur les worktrees de PR | **Mesuré, pas supposé** : hors périmètre par décision (option A), régime réutilisé où l'argument du ticket ne tient pas. Assertion D du garde |
| Un cinquième chemin apparaît plus tard | Gain perdu en silence | Assertion D : l'énumération des handlers hors `dispatch-lib` est dans le garde ; un ajout non déclaré fait rougir la CI |

## Références

- `mika/skills/bundled/_shared/dispatch-lib.sh:563-566` — `_PILOT_SANDBOX_ENV_ALLOWLIST`
- `…:549-562` — bloc d'audit mika#2039 R6 (toute addition à `--setenv` doit être pesée)
- `…:916`, `…:927` — les deux sorties `"$@"` du chemin direct
- `…:1074-1078` — boucle de réinjection `--setenv`, test `[ -n "${!var:-}" ]`
- `…:1144` — `--clearenv`
- `mika/Makefile:156` — cible `test-dispatch-lib`, câblée CI (mika#1772)
- `mika/Makefile:186`, `:193` — modèles de garde à comportement négatif pinné
- `lefthook.yml:16-18` — `cargo clippy --all-targets --all-features -- -D warnings` en pre-commit
- `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` — pourquoi le réglage est dans le code, pas dans la prose
- `feedback_a_probe_needs_both_controls_in_the_same_call` — pourquoi positif et négatif dans le même test
- `feedback_estimated_counts_undercount_measured` — pourquoi la mesure est ventilée
