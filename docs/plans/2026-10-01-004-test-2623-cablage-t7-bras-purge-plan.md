# mika#2623 — Le câblage T7 → bras de purge, testé au site de production

**Ticket :** `senara-solutions/mika#2623`
**Parent :** mika#2619 — **Origine :** la revue multi-agents de PR #2621
**Type :** test (aucun comportement de production ne change, sauf si un test révèle un défaut)

---

## Ce que la lecture du code déplace dans le ticket

C'est le premier livrable : cinq constats, chacun changeant soit le périmètre, soit
la forme du test.

### R1 — AC1 est partiellement couvert, et le trou est exact

`mika2619_v4_un_refus_t7_traverse_la_production`
(`crates/mika-agent/src/worktree_reaper.rs:9742`) existe déjà et passe un
`t7_refusals` **fabriqué à la main** directement à `purge_stale_target_dirs`.
Son doc-comment dit « traverse la production » : il traverse le **bras de purge**
de production, jamais le **site d'assemblage**.

La mutation qui coupe le câblage — remplacer `&t7_refusals` par `&[]` au site
d'appel (`worktree_reaper.rs:1925`) — **laisse `v4` vert**, ainsi que `v5`, `v6`,
le scan de co-site et les 143 tests du module. C'est très exactement la lacune que
la revue a nommée : *« le prédicat d'admission et le bras de purge sont testés
séparément »*. AC1 demande donc un test qui **entre par**
`reap_terminal_worktrees`.

### R2 — AC2 est couvert aux trois quarts, et c'est la moitié comportementale qui manque

Déjà en place : `mika2619_v5_le_predicat_de_t7_a_ses_quatre_coins` épingle
`t7_is_needed(0, 2, true) == true` **et** l'équivalence avec
`!should_stop_repo_loop` sur les huit points du triplet ;
`mika2619_la_condition_de_t7_passe_par_le_predicat` épingle le co-site par scan de
source. Et « ce sens est nommé dans un commentaire » est déjà vrai **trois fois** :
le doc-comment de `t7_is_needed` (`:3671-3702`), le bloc de commentaire de
`reap_terminal_worktrees` (`:1779-1801`), et le § *Nine motives, N build dirs* du
`CLAUDE.md` du crate.

Ce qui manque est le **comportement de bout en bout** : que la purge tourne
réellement sur un dépôt dont le budget faucheur est à zéro.

### R3 — `MIKA_WORKTREE_REAP_MAX_PER_TICK=0` ne met PAS le budget faucheur à zéro

`parse_positive_usize` (`:598`) retombe sur le défaut avec un `warn!` — *« le `0`
ne désarme pas, c'est le rôle du kill-switch, et l'inverse ferait d'une coquille
un désarmement silencieux sur un scan destructif »*, écrit au site. Donc le
**seul** chemin vers `budget == 0` à l'entrée de `t7_is_needed` est **deux
dépôts** dans `MIKA_WORKTREE_REAP_REPO_DIRS`, le premier épuisant le budget —
c'est-à-dire précisément l'état que le doc-comment nomme comme *« atteignable dès
que le faucheur épuise son budget sur un dépôt antérieur »*.

**Un test mono-dépôt ne peut pas atteindre AC2.** C'est la contrainte la plus
structurante de ce plan.

### R4 — AC3 a déjà son contrôle positif, et le sens du code est plus étroit que l'AC ne le suggère

`mika2619_v6_un_repertoire_sous_pilot_scratch_est_purge` (`:9874`) purge déjà deux
caches sous `.pilot-scratch/ac6/`. Ce qui manque est la moitié négative.

Et le sens du code n'est pas « ce cache est protégé » mais **« le worktree entier
sort de la population »** : P3 dans `screen_target_purges` (`:3497-3513`) teste
`cwd == root || cwd.starts_with(root)` **avant** la boucle des répertoires, et
pousse un refus de portée worktree (`build_dir_path: None`, motif `live_process`).
Le test doit donc asserter **un seul** refus, pas un par cache — asserter le
contraire figerait une sémantique que le code ne porte pas.

### R5 — un test de bout en bout exige un faux `gh`, et le motif existe déjà dans ce crate

`list_prs` → `gh()` → `run_gh_subprocess`
(`tools/pr_merge_with_gate.rs:1736`) → `tokio::process::Command::new("gh")`, résolu
par `PATH`, non injectable. Un `gh` en échec fait `failed += 1; continue;` et le
dépôt est sauté — donc sans faux `gh`, aucun test de bout en bout n'observe quoi
que ce soit.

Le harnais existe, dans ce même crate, pour ce même `run_gh_subprocess` :
`wip_rescue.rs:2248-2340` (`PathGuard`, `prepend_to_path`, `install_fake_gh`,
`#[serial_test::serial]`), **avec son analyse de risque résiduel écrite**.
`serial_test`, `filetime` et `tempfile` sont déjà des dev-dependencies du crate.

**Conséquence pour AC4 : zéro ligne de production à modifier.** Tout ce dont les
tests ont besoin est déjà `pub` (`reap_terminal_worktrees`, `collect_live_cwds`,
`TARGET_PURGED_TOOL`, `REASON_DIRTY`, `AsyncDatabase::get_audit_events`).

---

## Requirements

### U1 — AC1 : le câblage, observé en entrant par `reap_terminal_worktrees`

Un test de bout en bout monte un **vrai** dépôt git avec de **vrais** worktrees
liés, un faux `gh` qui déclare les PR, et appelle `reap_terminal_worktrees`. Il
observe que le bras de purge a reçu la population T7, **par l'effet** :

- le worktree refusé par T7 **existe toujours** (le faucheur l'a conservé) ;
- son répertoire de build **n'existe plus** (le refus T7 a atteint la purge) ;
- la ligne d'audit `target_purged` porte `keep_reason=dirty` dans son `reasoning` ;
- un contrôle négatif dans le **même tick** : un worktree `Clean` est bel et bien
  fauché, donc le test ne constate pas « rien ne se passe ».

### U2 — AC2 : budget faucheur nul, la purge tourne

Deux dépôts, `MIKA_WORKTREE_REAP_MAX_PER_TICK=1`. Le premier épuise le budget du
faucheur ; le second doit voir T7 évalué et son répertoire de build purgé. Le sens
choisi par le code est nommé dans un commentaire de test, et le **trou résiduel**
que le code nomme déjà en production (`:1914-1919`) est épinglé dans le même test :
un worktree `Clean` du second dépôt n'est **ni fauché ni refusé**, donc son
répertoire de build échappe à ce tick.

### U3 — AC3 : un cwd vivant sous `.pilot-scratch/<x>`, vu par `purge_stale_target_dirs`

Un test in-crate, sans PATH ni `git`, avec son contrôle positif.

### U4 — Aucune ligne de production modifiée

Sauf si U1, U2 ou U3 révèle un défaut — auquel cas un commit par défaut, test vu
rouge d'abord.

---

## Approche

### Le site : un fichier d'intégration propre pour U1/U2, in-crate pour U3

**U1 et U2 → `crates/mika-agent/tests/worktree_reap_t7_wiring_2623.rs`.**

Un fichier sous `tests/` a **son propre binaire**, donc son propre process : le
`PATH` muté n'atteint pas les ~3400 tests du binaire de la lib. C'est le
confinement que `wip_rescue.rs` n'a pas pu obtenir et dont il nomme le coût
résiduel en toutes lettres. `#[serial_test::serial]` sérialise ensuite les tests
**du fichier** entre eux.

Prix, nommé : `TargetPurgeStats` et `PURGE_IDLE_DEFAULT_SECS` sont privés, donc
l'observation se fait **par l'effet** (disque + lignes d'audit) et la fenêtre
d'inactivité est **déclarée** par le test via `MIKA_TARGET_PURGE_IDLE_SECS`. Les
deux sont des gains : pour AC1 ce qu'on veut observer est qu'un répertoire
disparaît pendant que le worktree reste, un fait sur le disque et non un compteur
interne ; et une fenêtre déclarée est plus lisible qu'une constante empruntée.

**U3 → le `mod tests` de `worktree_reaper.rs`**, à côté de `mika2619_v6`.
`purge_stale_target_dirs` est privée, les helpers (`fake_worktree`,
`fake_build_dir`, `age_tree`, `no_processes`, `refusal_with`) y sont déjà, et ce
test ne dépend ni du `PATH` ni de `git`. Chaque test au niveau le plus bas qui
l'atteste.

### Le harnais du fichier d'intégration

Quatre helpers, et aucun ne touche quoi que ce soit hors de son `TempDir` :

| helper | ce qu'il fait |
|---|---|
| `fake_repo(root, name)` | `git init`, `user.email`/`user.name` **locaux** (jamais globaux), un `.gitignore` committé portant `target/`, `.pilot-scratch/`, `.claude/`, puis `git remote add origin https://github.com/senara-solutions/<name>.git` — `parse_owner_repo` en dérive le `owner/repo` |
| `add_worktree(repo, slug, branch)` | `git worktree add <repo>/.claude/worktrees/<slug>/mika -b <branch>` — le segment `/.claude/worktrees/` est ce que T1 exige |
| `install_fake_gh(bin_dir, prs_json)` | un `#!/bin/sh` qui journalise ses appels et rend `prs_json` sur `pr list`, `[]` sinon. **Son tmpdir est inscrit dans le script** : `run_gh_subprocess` scrubbe tout `MIKA_*` avant l'exec, donc le faux ne peut pas communiquer par une variable préfixée (la note de `wip_rescue.rs`, mot pour mot) |
| `age_tree(root, secs)` | recopié du `mod tests` in-crate (helper privé, non réutilisable depuis l'extérieur) : vieillit fichiers **puis** répertoires, du plus profond au moins profond |

Plus un `EnvGuard` qui restaure chaque variable posée (même forme que `PathGuard`),
et un `git_available()` / `proc_readable()` dont l'échec **nomme la cause**
(§ Risques).

### U1 — le test, en détail

Un dépôt `R`, trois worktrees, un seul tick :

| worktree | branche | état | PR | attendu |
|---|---|---|---|---|
| `W1` | `fix/2623/dirty` | `DIRTY.txt` non suivi | mergée, close il y a longtemps | **conservé** (T7 `dirty`) ; `target/` **purgé** |
| `W2` | `fix/2623/clean` | propre | mergée, close il y a longtemps | **fauché** (contrôle négatif : le faucheur marche toujours) |
| `W3` | `fix/2623/ahead` | un commit local au-dessus de `refs/remotes/origin/fix/2623/ahead` | mergée, close il y a longtemps | **conservé** (T7 `unpushed_commits`) ; `target/` **purgé** |

`W3` demande de fabriquer la ref distante : `git update-ref
refs/remotes/origin/fix/2623/ahead <sha>` avant le commit local, pour que
`collect_work_state` atteigne sa branche `rev-list --count` plutôt que son repli
`Clean` sur ref absente.

Environnement posé : `MIKA_WORKTREE_REAP_REPO_DIRS=<R>`,
`MIKA_WORKTREE_REAP_GRACE_SECS=1`, `MIKA_WORKTREE_REAP_MAX_PER_TICK=4`,
`MIKA_TARGET_PURGE_IDLE_SECS=120`, `MIKA_TARGET_PURGE_MAX_PER_TICK=4`, les deux
dispositions explicitement `armed`. Rien n'est laissé à un défaut : un test qui
hérite d'un défaut change de sens le jour où le défaut change.

**L'ordre des assertions est porteur.** D'abord *« le dépôt a été traité »* — au
moins une ligne `worktree_reap_skipped` existe — puis les faits. Sans ce premier
pas, un faux `gh` cassé produirait un échec dont le message accuserait le câblage.

Mutation à consigner dans la PR : `worktree_reaper.rs:1925`, `&t7_refusals` → `&[]`.
Elle laisse `v4`, `v5`, `v6`, le scan de co-site et les 143 tests du module
**verts** ; seul U1 rougit, sur `W1/target` et `W3/target` qui survivent.

### U2 — le test, en détail

Deux dépôts, `MIKA_WORKTREE_REAP_MAX_PER_TICK=1`,
`MIKA_WORKTREE_REAP_REPO_DIRS=<A>:<B>` (`parse_repo_dirs` préserve l'ordre) :

- dépôt `A` : `WA` propre, PR mergée, hors grâce → **fauché**, et le budget du
  faucheur tombe à 0 ;
- dépôt `B` : `WB` sale + `target/` âgé → T7 doit être évalué **malgré** le budget
  nul, et `WB/target` purgé ; `WC` propre + `target/` âgé → **ni fauché ni
  refusé**, son `target/` **survit**.

`WC` est l'épinglage du trou résiduel. Il n'est pas un défaut : c'est la décision
que le commentaire de production nomme déjà (*« fermer ce cas demanderait de
pousser un refus synthétique pour un worktree que rien ne refuse — une ligne
d'audit fausse »*), et l'épingler est ce qu'AC2 demande par *« épinglé dans le sens
que le code choisit »*.

**Les deux mutations, et la première est la raison d'être de ce test.**

1. Au **site d'appel**, `t7_is_needed(budget, 0, purge_cfg.enabled)` — le prédicat
   reste intact, donc `v5` reste vert, et le scan de co-site aussi puisque
   `t7_is_needed(` est toujours appelé. **Aucun test existant ne rougit.** Seul U2
   le voit, sur `WB/target` qui survit. C'est la démonstration qu'U2 apporte ce
   que `v5` ne peut pas donner.
2. `should_stop_repo_loop` ramené au seul budget faucheur ⇒ le dépôt `B` n'est
   jamais ouvert, et `WB/target` survit aussi.

### U3 — le test, en détail

In-crate, `#[tokio::test]`, sur `purge_stale_target_dirs` :

- un `fake_worktree`, un `fake_target`, deux `fake_build_dir` sous
  `.pilot-scratch/ac6/{a,b}` (un par marqueur, comme `v6`), tous vieillis
  au-delà de la fenêtre ;
- **cas négatif** : `LiveCwds::Enumerated(vec![wt.join(".pilot-scratch/ac6/a")])`
  ⇒ `stats.purged == 0`, **exactement un** refus, de motif `live_process` et de
  portée worktree (`build_dir_path: None` — le sens du code par R4), et les
  **trois** répertoires intacts ;
- **contrôle positif** : le même arbre avec `no_processes()` ⇒ les trois purgés.

Le contrôle positif est ce qui sépare « le terme P3 décide » de « la purge ne
marche pas sur `.pilot-scratch/` ».

Mutation à consigner : une exception sur `.pilot-scratch` dans le terme P3 (ignorer
les cwd situés sous ce répertoire, au motif qu'il porte des brouillons). C'est la
forme réelle du défaut qu'AC3 craint, et c'est la seule mutation qui rougit U3
**sans** rougir `mika2497_v2_un_processus_vivant_dedans_conserve` — restreindre
`starts_with` à `== root` rougirait les deux et ne démontrerait rien de propre à
`.pilot-scratch`.

---

## Fire-Disposition

Ce plan livre **trois détecteurs comportementaux** (U1, U2, U3). Aucun n'est un
scan de source.

**Option retenue : (a) exception nommée en allowlist — allowlist vide par absence
de population.**

Détail d'implémentation : il n'y a **aucun fichier d'allowlist à créer**, parce
qu'il n'y a aucune violation existante à exempter. Le comportement de production
est correct — établi par lecture au § *Ce que la lecture du code déplace* — et ces
trois tests le **figent**. Ils doivent donc être **verts au premier jet**.

La conduite si l'un rougit est écrite, et ce n'est pas une exemption : **c'est AC4
qui s'applique** — un défaut révélé, un commit par défaut, le test vu rouge avant
le correctif, et la mutation consignée dans le corps de la PR. Ajouter une
exemption pour faire passer le build serait exactement l'inverse : un détecteur
dont l'allowlist absorbe sa première prise est un détecteur désarmé le jour de sa
naissance.

**Clause de repli vers (b), avec sa condition de déclenchement.** U1 et U2 portent
un harnais qui mute `PATH` et dépend de `git` et de `/proc`. Si la CI montre une
instabilité **de cause environnementale** (et non un défaut), ils passent
`#[ignore]` **avec un ticket de suivi ouvert nommant la cause mesurée**, et **U3 —
qui ne dépend ni du `PATH`, ni de `git`, ni de `/proc` — reste le livrable non
négociable**. C'est la clause que `wip_rescue.rs:2272-2274` s'est écrite pour la
même classe de harnais, et la transposer plutôt que la réinventer est le point.

Ce qui n'est **pas** une raison de désarmer : un test rouge parce que le câblage
est coupé. C'est le succès du détecteur.

---

## Verification Contract

| # | Vérification | Commande | Attendu |
|---|---|---|---|
| V1 | U1 passe | `cargo test -p mika-agent --test worktree_reap_t7_wiring_2623 -- --test-threads=1 cablage` | vert |
| V2 | U1 **vu rouge** sur la mutation de câblage | `&t7_refusals` → `&[]` à `:1925`, puis V1 | rouge sur `W1/target` **et** `W3/target` |
| V2b | la mutation de V2 laisse l'existant vert | `cargo test -p mika-agent --lib worktree_reaper` sous la mutation | vert — c'est ce qui prouve que le trou existait |
| V3 | U2 passe | `cargo test -p mika-agent --test worktree_reap_t7_wiring_2623 -- --test-threads=1 budget` | vert |
| V4 | U2 **vu rouge** sur la mutation du site d'appel | `t7_is_needed(budget, 0, …)`, puis V3 | rouge sur `WB/target` |
| V4b | la mutation de V4 laisse `v5` et le scan de co-site verts | `cargo test -p mika-agent --lib mika2619` sous la mutation | vert |
| V5 | U3 passe, cas négatif **et** contrôle positif | `cargo test -p mika-agent --lib mika2623` | vert |
| V6 | U3 **vu rouge** sur l'exception `.pilot-scratch` dans P3 | mutation, puis V5 | rouge ; `mika2497_v2_un_processus_vivant_dedans_conserve` reste vert |
| V7 | aucune ligne de production modifiée | `git diff --stat main -- crates/mika-agent/src/worktree_reaper.rs` | seul le `mod tests` apparaît ; **zéro** ligne sous `\n#[cfg(test)]` |
| V8 | le reste de la suite ne bouge pas | `cargo test -p mika-agent` | vert, et le compte de tests `worktree_reaper` monte de 143 à 145 |
| V9 | lint et format | `cargo clippy --all-targets -- -D warnings` ; `cargo fmt --check` | vert |
| V10 | taille du fichier sous le cap | `wc -c crates/mika-agent/src/worktree_reaper.rs` | < 1 048 576 (450 311 aujourd'hui ; `LARGE_FILE_ALLOWLIST` est vide et doit le rester) |

**V2b et V4b sont les vérifications porteuses.** Sans elles, « le nouveau test
attrape la mutation » ne se distingue pas de « un test quelconque attrape la
mutation », et le ticket ne serait pas fermé : la lacune nommée par la revue est
exactement qu'**aucun** test existant ne l'attrape.

---

## Risques nommés

1. **`/proc` illisible** ⇒ `collect_live_cwds()` rend `Unavailable` ⇒ P3 refuse
   tout sous `process_scan_unreadable` et U1/U2 échouent pour une cause
   d'environnement. Mitigation : une assertion de contrôle en tête de test qui
   **nomme la cause** plutôt qu'un `return` silencieux — un test qui se saute tout
   seul se lit exactement comme un test qui passe (classe mika#2205).
2. **`git` absent** ⇒ même traitement : assertion nommant la cause.
3. **`gh` réel installé sur la machine** ⇒ le faux doit être en **tête** du `PATH`.
   `run_gh_subprocess` réinjecte `GH_TOKEN`, sans effet sur un script `sh`.
4. **`PATH` process-global.** Confiné par le binaire de test propre **plus**
   `#[serial_test::serial]`. Résidu nommé, hérité de `wip_rescue.rs` : l'attribut
   sérialise contre les autres tests `#[serial]`, pas contre tout le binaire —
   aucun autre test de ce fichier ne spawne `gh`, et le fichier n'en contient que
   deux.
5. **`TMPDIR` via lien symbolique** ⇒ `canonical_path_is_managed` canonicalise ; le
   segment `/.claude/worktrees/` survit à la canonicalisation. Un `TMPDIR` qui
   contiendrait lui-même `.claude/worktrees` ferait entrer le checkout principal
   dans la population : improbable, nommé.
6. **Durée** : trois dépôts `git init` + worktrees ≈ quelques secondes. Acceptable,
   et c'est le prix d'un test qui entre par le site de production.
7. **`git config --global` du poste** : `user.email` / `user.name` sont posés
   **localement** dans chaque dépôt factice. Aucun test n'écrit hors de son
   `TempDir`.

---

## Hors périmètre, délibérément

- **Un scan de source sur le câblage.** Tentant, et refusé : AC1 demande une
  **observation comportementale**, et le câblage **cesse d'être invisible aux tests
  comportementaux** dès qu'U1 existe. La doctrine du scan structurel (mika#2201)
  s'applique aux classes qu'aucun test comportemental ne peut voir ; celle-ci en
  sort. Le co-site de `t7_is_needed` a déjà le sien
  (`mika2619_la_condition_de_t7_passe_par_le_predicat`), inchangé.
- **Le troisième motif T7, `work_state_unreadable`.** Le produire de bout en bout
  demanderait de rendre `git` défaillant à l'intérieur d'un worktree. Il reste
  couvert par les tests purs d'`apply_work_states`. **Limite nommée**, non couverte
  par U1.
- **Fermer le trou résiduel du budget faucheur épuisé en cours de boucle de
  disposition.** U2 l'épingle ; le fermer demanderait le refus synthétique que la
  production refuse par écrit.
- **La fenêtre résiduelle du verrou mika#2511** (unlink → fin de suppression),
  inchangée.
- **Les cinq termes P1–P5, les neuf motifs d'éligibilité, les sept termes du
  faucheur, toute valeur de réglage.** Rien ne bouge.
- **Extraire le `mod tests` de `worktree_reaper.rs`** à la manière de mika#2321 :
  le fichier est à 450 Ko sur un cap de 1 Mo, et ce plan ajoute ~60 lignes in-crate.
  Pas de besoin mesuré.

---

## Ce que ce travail n'achète PAS

- **Il ne corrige aucun défaut.** Il fige un comportement établi par lecture, à
  l'endroit où il était jusqu'ici invisible. Si l'un des trois tests rougit au
  premier jet, c'est un résultat et c'est AC4 qui s'applique.
- **Il ne rend la purge ni plus large ni plus sûre.** Pas un octet de disque
  supplémentaire n'est récupéré.
- **Il n'ajoute aucun compteur, aucun événement de journal, aucune ligne
  d'audit.** Les surfaces opérateur de mika#2619 et mika#2497 sont inchangées ;
  rien de neuf n'est à grepper, donc rien de neuf ne peut se taire.
- **Il ne couvre pas la population réelle.** Les worktrees de l'incident du
  2026-10-01 sont fauchés ; ces tests décrivent des dépôts factices. Ce qu'ils
  achètent est que la **prochaine** régression du câblage soit rouge en CI au lieu
  d'être silencieuse.
- **Il ne prouve rien sur un hôte sans `/proc` ni `git`.** Les deux contrôles
  d'environnement font échouer franchement plutôt que passer à vide — c'est le
  mieux qu'un test puisse faire là, et ce n'est pas une couverture.

---

## Definition of Done

- [ ] `crates/mika-agent/tests/worktree_reap_t7_wiring_2623.rs` livré, portant U1
      et U2, chacun `#[serial_test::serial]`, chacun avec son contrôle
      d'environnement nommant la cause.
- [ ] U3 livré dans le `mod tests` de `worktree_reaper.rs`, avec son contrôle
      positif.
- [ ] Les six mutations de V2, V2b, V4, V4b, V6 exécutées et **consignées dans le
      corps de la PR**, chacune avec ce qui a rougi et ce qui est resté vert.
- [ ] `git diff` sur `worktree_reaper.rs` ne touche que le `mod tests` (V7).
- [ ] `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
      `cargo fmt --check` verts.
- [ ] Le corps de la PR porte les cinq rectifications R1–R5 — un relecteur qui
      ouvre cette PR en croyant le ticket littéral conclurait que U1 duplique
      `v4`.
- [ ] Aucune entrée ajoutée à `LARGE_FILE_ALLOWLIST`.

---

## Acceptance criteria

Transcrits verbatim du corps de mika#2623.

- [ ] **AC1.** Un test de bout en bout passe par `reap_terminal_worktrees` et
      observe que le bras de purge reçoit exactement les worktrees refusés par T7.
      Il est **vu rouge** quand on coupe le câblage (mutation consignée dans la
      PR).
- [ ] **AC2.** Le cas budget faucheur = 0 est épinglé dans le sens que le code
      choisit, et ce sens est nommé dans un commentaire.
- [ ] **AC3.** Un test montre qu'un cwd vivant sous `.pilot-scratch/<x>` protège ce
      cache au niveau de `purge_stale_target_dirs`. Contrôle positif : le même
      cache, sans cwd vivant, est purgé.
- [ ] **AC4.** Aucune ligne de production modifiée, sauf si un des tests révèle un
      défaut ; dans ce cas, un commit par défaut, avec test vu rouge.

**Lecture d'AC2, explicitée par R3 :** « budget faucheur = 0 » n'est pas
atteignable par `MIKA_WORKTREE_REAP_MAX_PER_TICK=0` — cette valeur retombe sur le
défaut. Le test l'atteint par deux dépôts, le premier épuisant le budget, ce qui
est l'état que le doc-comment de `t7_is_needed` nomme comme la raison d'être de ce
prédicat.
