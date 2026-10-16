# mika#2715 — généraliser `run_gh` aux sites `gh` restants du crate — vol 1

**Ticket :** senara-solutions/mika#2715
**Type :** refactor
**Vol :** 1 sur 2 — « la collecte par `ToolOutput` » (les trois sites de
`skills/builtin_handlers.rs`)

## Pourquoi

mika#2713 a posé `crates/mika-agent/src/gh_process.rs` : le point de passage
unique `run_gh(token, GhSubject, args) -> io::Result<Output>`, qui sous
`#[cfg(test)]` ne spawne rien, enregistre l'appel et **panique** sur un
`GhSubject::Issue` sous `FIXTURE_ISSUE_FLOOR`. Il l'a posé parce qu'un test de
la suite `auto_pull` avait écrit 29 commentaires sur le ticket **fermé**
mika#2636, et il a converti les huit sites de son périmètre.

Cinq sites `Command::new("gh")` du crate restent hors couture, portés par
l'allowlist nommée `GH_SPAWN_ALLOWLIST` du scan structurel de `gh_process.rs`.
Ce ticket les convertit, pour que l'allowlist redevienne vide et que la classe
soit fermée par construction plutôt que par vigilance.

## Ce que le groom a établi

Le ticket énonce *« la couture rend le même `io::Result<Output>` que
`output().await`, donc chaque mapping d'erreur reste inchangé au site »*. **Cette
prémisse est fausse pour quatre des cinq sites**, et c'est le constat qui
décide le découpage. Mesure, site par site :

| site | forme de collecte | type rendu |
|---|---|---|
| `milestone_manager/reader.rs` — `ProcessGhRunner::run` | `cmd.output().await` | `Result<String>` (anyhow) |
| `skills/builtin_handlers.rs` — `gh_read` | `spawn_and_collect(cmd, …)` | `ToolOutput` |
| `skills/builtin_handlers.rs` — `fetch_pr_wip_rescue_view` | `spawn_and_collect(cmd, …)` | `ToolOutput` |
| `skills/builtin_handlers.rs` — `run_gh` | `spawn_and_collect(cmd, …)` | `ToolOutput` |
| `tools/pr_merge_with_gate.rs` — `run_gh_subprocess` | `spawn()` + `take(MAX_OUTPUT_LEN)` + `wait()` | `Result<String, String>` |

Seul `reader.rs` passe par `output().await`. Les quatre autres collectent par
lecture **bornée** (`MAX_OUTPUT_LEN`), et `spawn_and_collect` porte en plus la
comptabilité d'octets rejetés, le ticker de progression (#900) et la convention
de préfixe `Exit code: N` / `Killed by signal:` sur laquelle quatre gardes
amont branchent (`has_non_zero_exit_prefix`, mika#2237). Router ces quatre
sites par la couture **telle qu'elle existe** remplacerait une lecture bornée
par un `output()` non borné : ce n'est pas une conversion neutre, c'est la
régression de mika#1746 réintroduite.

Trois autres faits mesurés, qui bornent le vol :

1. **`run_gh_subprocess` a 26 appelants dans le crate** (`grep -rn
   'run_gh_subprocess(' crates/mika-agent/src/`, hors définition). Le convertir
   met le double de test en travers de ces 26 chemins d'un coup : chaque test
   qui y passe aujourd'hui reçoit un échec de spawn et recevrait désormais
   `exit 0` / `"[]"`. Cet audit est un travail à part entière, et il ne se
   déclare pas depuis ici.
2. **`GhRunner` porte huit doubles de test** (`verdict_poster.rs` ×2,
   `cadence.rs` ×1, `reader.rs` ×1, `spawn.rs` ×4), plus une garde structurelle
   qui **asserte** la présence de `impl GhRunner for ProcessGhRunner`
   (`reader.rs:1152`, anti-vacuité de mika#1975 D5). La question du ticket
   (« le trait délègue-t-il, ou disparaît-il ? ») a donc une réponse de coût :
   **il délègue**. Le supprimer réécrit huit doubles et casse l'anti-vacuité
   d'une garde voisine, pour un bénéfice — « deux doubles pour un même
   comportement sont libres de divergir » — que la délégation obtient déjà :
   une fois `ProcessGhRunner::run` routé par `run_gh`, les doubles `GhRunner`
   ne doublent plus un spawn, ils doublent une composition de haut niveau.
   Cette conversion est **au vol 2**, avec le reste de `reader.rs`.
3. **`GhSubject` n'a pas besoin d'une variante.** Les trois sites de
   `builtin_handlers.rs` s'adressent à une issue, à une PR, ou au dépôt — ce que
   les trois variantes existantes couvrent déjà, `Pr` ayant été ajoutée par
   mika#2713 exactement pour ce vol. Ce qui manque n'est pas une variante,
   c'est la **dérivation** du sujet depuis l'entrée de chaque site (§ Décision).

## Décision

**D1 — une seconde entrée de couture, pas un second spawn.**
`gh_process` gagne `run_gh_tool(token, subject, args, repo) -> ToolOutput`.

- Sous `cfg(not(test))` : construit la `Command` (`GH_PROMPT_DISABLED=1`,
  `scrub_mika_env_vars`, puis `GH_TOKEN` ré-injecté **après** le scrub — l'ordre
  de #380, qu'aucun site ne doit avoir à se rappeler), et la remet à
  `builtin_handlers::spawn_and_collect(cmd, "gh", "Is the GitHub CLI
  installed?")`. La lecture bornée, le ticker, la comptabilité d'octets rejetés
  et la convention `Exit code: N` restent **byte-identiques** en production,
  parce que c'est le même collecteur qui tourne.
- Sous `cfg(test)` : enregistre dans le même `CALL_LOG` que `run_gh`, applique
  le **même** plancher `FIXTURE_ISSUE_FLOOR` sur `GhSubject::Issue`, et rend
  `ToolOutput::success` portant le `stdout` canné du sujet (défaut `"[]"`).
  **Zéro processus spawné** — la propriété que le doc-comment du module énonce
  reste vraie au mot près, pour les deux entrées.

`spawn_and_collect` passe de `async fn` privée à `pub(crate) async fn`. C'est
le seul changement de signature du vol. Elle reste dans `builtin_handlers.rs` :
elle sert aussi `git` (ligne 925), un troisième outil (ligne 6104) et trois
tests, donc la déplacer sortirait du périmètre pour ne rien fermer.

**D2 — le sujet se dérive de l'entrée du site, et le plancher est armé partout
où il peut l'être honnêtement.**

- `gh_read` : l'entrée est **structurée** (`GhReadArgs { op, target, … }`), donc
  la dérivation est exacte. `issue_view` → `Issue(target)` ; `pr_view` /
  `pr_diff` → `Pr(target)` ; `issue_list` et `file_view` → `Repo` (aucun numéro
  d'issue dans l'argv). Un `target` qui ne parse pas en `u64` → `Repo` : la
  couture ne décide rien, et un target non numérique n'est pas un numéro de
  ticket.
- `fetch_pr_wip_rescue_view` : `Pr(pr_num)` quand `pr_num` parse, `Repo` sinon.
- `run_gh` : l'argv est libre (c'est un outil piloté par un handler de skill),
  donc une fonction `gh_subject_from_argv(args: &[String]) -> GhSubject` :
  premier argument `"issue"` et premier positionnel numérique → `Issue(n)` ;
  `"pr"` → `Pr(n)` (via `normalize_pr_identifier`, pour qu'une URL de PR donne
  le même sujet qu'un numéro nu) ; **tout le reste** → `Repo`.

  Ce que `Repo` concède, et le plan le dit plutôt que de le laisser croire le
  contraire : un `gh api /repos/o/r/issues/2636/comments` est classé `Repo` et
  **ne réveille pas le plancher**. Armer cette forme demanderait de parser un
  chemin d'API, ce qui est un analyseur de plus pour une population que ce vol
  ne touche pas. Le vol arme ce qu'une entrée structurée donne gratuitement, et
  nomme ce qu'il n'arme pas.

**D3 — l'allowlist perd une entrée, pas son assertion de non-vacuité.**
`("skills/builtin_handlers.rs", 3)` disparaît de `GH_SPAWN_ALLOWLIST`. Les deux
autres entrées restent, leur doc-comment est réécrit pour ne plus nommer que
les deux sites restants et pour renvoyer au **vol 2** comme suivi. L'assertion
de non-vacuité reste donc justifiée et reste en place — c'est le vol 2 qui la
retirera en même temps que la dernière entrée. L'égalité d'ensembles fait le
reste : oublier le retrait de l'entrée rougit en disant « entrée stale ».

## Découpage en vols

Deux vols, parce que les deux formes de collecte restantes demandent chacune
leur propre couture et que `run_gh_subprocess` demande en plus un audit de 26
chemins de test.

**Vol 1 — celui-ci.** La collecte par `ToolOutput` : les trois sites de
`skills/builtin_handlers.rs`, par `run_gh_tool`. Mergeable seul : l'allowlist
passe de trois entrées à deux, l'égalité le vérifie, aucun autre fichier du
périmètre n'est touché.

**Vol 2 — la suite.** `milestone_manager/reader.rs` (`ProcessGhRunner::run`
délègue à `run_gh` — la seule des cinq conversions qui tient dans la couture
existante — et `GhRunner` survit, cf. constat 2) et
`tools/pr_merge_with_gate.rs` (`run_gh_subprocess` : une troisième forme de
collecte, bornée et rendant `Result<String, String>`, plus l'audit de ses 26
appelants). Au terme du vol 2, `GH_SPAWN_ALLOWLIST` est vide et son assertion
de non-vacuité est retirée dans le même commit.

Tracked in: senara-solutions/mika#2715 — vol 2 (« `reader.rs` et les 26
appelants de `run_gh_subprocess` »)

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/gh_process.rs` — `run_gh_tool` (moitié production + moitié double) et amendement du doc-comment du module | 70 |
| `crates/mika-agent/src/gh_process.rs` — `gh_subject_from_argv` + son doc | 30 |
| `crates/mika-agent/src/gh_process.rs` — retrait de l'entrée `builtin_handlers.rs` de `GH_SPAWN_ALLOWLIST` + réécriture du doc-comment | 12 |
| `crates/mika-agent/src/skills/builtin_handlers.rs` — `spawn_and_collect` en `pub(crate)` + conversion des trois sites (dérivation du sujet, pliage du `--repo` dans l'argv) | 50 |
| tests (`crates/mika-agent/src/gh_process.rs` § tests) | 70 |

Total estimé : 232 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

Sous la borne, et le plan dit pourquoi : la moitié production de chaque site
est un **remplacement** (dix à douze lignes de construction de `Command`
cèdent la place à un appel de trois lignes plus une dérivation de sujet de
cinq), pas un ajout ; et le collecteur n'est pas réécrit, il est appelé depuis
un cran plus haut.

## Fichiers

- `crates/mika-agent/src/gh_process.rs` — `run_gh_tool` (les deux moitiés
  `cfg`), `gh_subject_from_argv`, `GH_SPAWN_ALLOWLIST` amendée, doc-comment du
  module amendé, tests.
- `crates/mika-agent/src/skills/builtin_handlers.rs` — `spawn_and_collect` en
  `pub(crate)` ; `gh_read` (≈ 1881–1894), `fetch_pr_wip_rescue_view`
  (≈ 2967–2980), `run_gh` (≈ 5336–5372) convertis.

Hors périmètre, et inchangés : `milestone_manager/reader.rs`,
`tools/pr_merge_with_gate.rs` (vol 2) ; `milestone_manager/no_dispatch_test.rs`
(déjà sous un chemin de test, exclu du scan par `is_test_source_path`).

## Fire-Disposition

Ce vol livre un détecteur — il **amende** le scan structurel
`mika2713_aucun_spawn_gh_hors_la_couture_et_son_allowlist` et ajoute un
contrôle de bonne foi pour `gh_subject_from_argv`. Disposition retenue :
**(a) exception nommée en allowlist**, par continuité stricte avec mika#2713.

Détail d'implémentation :

- Les deux violations qui subsistent après ce vol gardent une entrée
  grep-visible dans `GH_SPAWN_ALLOWLIST`, **par fichier et par compte** :
  `("milestone_manager/reader.rs", 1)` et `("tools/pr_merge_with_gate.rs", 1)`.
  Le compte plutôt qu'un numéro de ligne ou un nom de fonction, pour la raison
  que mika#2713 a écrite : une ligne bouge au premier `cargo fmt`.
- Chaque entrée **référence son suivi** : le doc-comment de la constante nomme
  mika#2715 vol 2 et, pour chacune des deux, ce que sa conversion demande
  (délégation de `ProcessGhRunner::run` ; troisième forme de collecte + audit
  des 26 appelants).
- L'assertion est **auto-nettoyante** et le reste : `assert_eq!(observed,
  expected)` est une **égalité** d'ensembles. Retirer l'entrée
  `builtin_handlers.rs` est donc obligatoire dans ce même commit — l'oublier
  rougit avec « Une entrée EN MOINS : le site a été converti, retire son entrée
  de `GH_SPAWN_ALLOWLIST` ». La conversion se vérifie elle-même.
- L'assertion de non-vacuité reste en place et reste vraie (deux entrées
  subsistent). Son message dit déjà quoi en faire quand l'allowlist se videra ;
  c'est le vol 2 qui l'exécutera.
- Le détecteur atterrit **armé** : aucun `#[ignore]`, aucun `#[cfg(skip)]`.

## Definition of Done

- `cargo test -p mika-agent` vert, dont
  `mika2713_aucun_spawn_gh_hors_la_couture_et_son_allowlist` sur l'allowlist à
  deux entrées.
- `cargo clippy --all-targets -- -D warnings` propre.
- `cargo fmt --check` propre.
- `make verify-bundled-skills` vert (aucun bundle touché, mais la garde
  pré-merge tourne).
- `scripts/verify-pipeline.sh origin/main` vert.
- Corps de PR : `Refs #2715`, vol 2 nommé par la ligne `Tracked in:`,
  `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de la *Proposed Solution* et des deux points
qu'elle renvoie au grooming.

- [ ] **AC1 — les trois sites de `skills/builtin_handlers.rs` passent par la
  couture.** `grep -c 'Command::new("gh")' crates/mika-agent/src/skills/builtin_handlers.rs`
  rend `0`, et `gh_read`, `fetch_pr_wip_rescue_view` et `run_gh` appellent
  `gh_process::run_gh_tool`.
- [ ] **AC2 — l'allowlist a perdu cette entrée, et le scan l'exige.**
  `GH_SPAWN_ALLOWLIST` ne porte plus que `milestone_manager/reader.rs` et
  `tools/pr_merge_with_gate.rs`, et
  `mika2713_aucun_spawn_gh_hors_la_couture_et_son_allowlist` est vert. Un test
  négatif montre que l'entrée stale rougit : avec l'entrée
  `builtin_handlers.rs` laissée en place, l'égalité échoue.
- [ ] **AC3 — la production ne change pas de collecteur.** `run_gh_tool` sous
  `cfg(not(test))` appelle `spawn_and_collect`, donc la lecture bornée à
  `MAX_OUTPUT_LEN`, la comptabilité d'octets rejetés, le ticker de progression
  (#900) et la convention de préfixe `Exit code: N` / `Killed by signal:`
  restent inchangés — un test asserte que la convention de préfixe est
  préservée, parce que quatre gardes amont la lisent
  (`has_non_zero_exit_prefix`, mika#2237).
- [ ] **AC4 — le plancher de fixture est armé sur la nouvelle entrée.**
  `run_gh_tool("t", GhSubject::Issue(2636), …)` panique en nommant `2636`,
  comme `run_gh` le fait déjà. Un test `#[should_panic(expected = "2636")]`
  le pose.
- [ ] **AC5 — la dérivation du sujet est exacte là où l'entrée est structurée,
  et honnête là où elle ne l'est pas.** `gh_subject_from_argv` rend
  `Issue(990_123)` sur `["issue","view","990123"]`, `Pr(990_124)` sur
  `["pr","view","https://github.com/o/r/pull/990124"]`, et `Repo` sur
  `["api","/repos/o/r/issues/2636/comments"]`. Ce dernier cas est **asserté
  comme `Repo`** : il documente la limite dans le test plutôt que dans un
  commentaire seul.
- [ ] **AC6 — le double ne spawne rien sur la nouvelle entrée non plus.** Un
  appel `run_gh_tool` sous test avec une sous-commande inexistante rend un
  `ToolOutput` de succès au `stdout` canné du double (`"[]"` par défaut) : un
  vrai `gh` aurait rendu un préfixe `Exit code:`. Même forme d'argument que
  `mika2713_aucun_vrai_gh_ne_repond_sous_test`.
- [ ] **AC7 — le `stdout` canné reste keyed par sujet.** Un override posé sur
  un sujet n'est pas lu par un autre, sur `run_gh_tool` comme sur `run_gh` :
  les deux entrées partagent `CANNED_STDOUT`, et la suite tourne en parallèle
  dans un seul processus (mika#2713 D4).

## Hors périmètre

- **La conversion de `milestone_manager/reader.rs` et de
  `tools/pr_merge_with_gate.rs`** — vol 2, nommé par la ligne `Tracked in:`
  ci-dessus. Leurs deux entrées d'allowlist restent, référencées.
- **La suppression du trait `GhRunner`.** Tranchée au grooming : il **délègue**.
  Huit doubles et l'anti-vacuité de `reader.rs:1152` en dépendent, et la
  délégation obtient le bénéfice visé sans les réécrire (constat 2).
- **Une variante supplémentaire de `GhSubject`.** Tranchée au grooming : les
  trois variantes existantes couvrent les trois sites, `Pr` ayant été posée par
  mika#2713 pour ce vol (constat 3).
- **L'armement du plancher sur un chemin `gh api`.** Demanderait un analyseur
  de chemin d'API ; classé `Repo` et dit comme tel (D2, AC5).
- **Le durcissement de l'egress des sandboxes de pilote** — mika#2716, cité par
  le doc-comment de `gh_process.rs` et inchangé ici.
- **Le déplacement de `spawn_and_collect`** hors de `builtin_handlers.rs` : elle
  sert aussi `git` et un troisième outil, donc la déplacer sortirait du
  périmètre pour ne rien fermer. Seule sa visibilité change.
