# mika#2889 — Tout assemblage du `SkillRegistry` d'un agent vivant passe les deux contrôles de l'init

**Ticket :** senara-solutions/mika#2889
**Labels :** `bug`, `p1-important`, `agent-core`, `ready`, `origin:manual`, `dispatch:loop`
**Base :** `main` @ `94b21916`
**Type :** fix

---

## Requirements

### La règle à mettre en force

Tout assemblage du `SkillRegistry` d'un agent **vivant** passe les deux contrôles
de l'initialisation — `apply_load_safety_check` (#530/#1335) et
`apply_required_tools_coherence_check` (mika#1576) — **dans l'ordre de l'init**.
Un rechargement à chaud est un assemblage.

La séquence de référence est celle de l'init serveur
(`crates/mika-agent/src/server/mod.rs:639-657`), dans cet ordre :

1. `SkillRegistry::from_dir(&skills_dir)`
2. `apply_identity_allowlist(&identity.skills.allowlist)` si présente
3. `apply_overrides(&overrides)`
4. `apply_testimony_grade_ban()` (mika#1798)
5. `apply_load_safety_check()`
6. `apply_required_tools_coherence_check(agent_id)`
7. `log_summary()`

### Inventaire des sites — mesuré sur `main` @ `94b21916`

Les six sites nommés par le ticket :

| site | étapes 5 et 6 ? |
|---|---|
| `crates/mika-agent/src/server/mod.rs:639` — init serveur | **oui** (référence) |
| `crates/mika-cli/src/commands/chat.rs:137` — init TUI | **oui** |
| `crates/mika-agent/src/server/a2a.rs:261` — rechargement à chaud A2A | **non** |
| `crates/mika-agent/src/server/handlers.rs:1685` — rechargement à chaud HTTP | **non** |
| `crates/mika-cli/src/commands/chat.rs:308` — rechargement TUI avant le tour | **non** |
| `crates/mika-cli/src/commands/chat.rs:386` — rechargement TUI après le tour | **non** |

Deux sites que le ticket ne nomme pas, mais qu'**AC3 couvre nécessairement** —
tous deux assemblent le registre d'un agent vivant et tous deux omettent les
deux contrôles :

| site | nature | étapes 5 et 6 ? |
|---|---|---|
| `crates/mika-agent/src/tools/delegate_task.rs:130-140` | agent délégué vivant | **non** |
| `crates/mika-agent/src/teams/engine.rs:310-330` | agents d'équipe vivants | **non** |

Ils ne sont pas hors périmètre : l'allowlist d'AC3 est exigée **vide**, et elle
ne peut l'être que si ces deux sites passent eux aussi par la fonction unique.
Les y router n'est pas une extension du périmètre, c'est la condition
d'atteignabilité d'AC3.

Un neuvième site, de **diagnostic**, applique déjà la séquence complète :
`crates/mika-agent/src/tools/list_skills.rs:44-65`, dont le commentaire dit en
propres termes qu'il la tient « en parité avec les chemins de démarrage ». Il est
routé lui aussi — non parce que la règle l'exige (c'est un outil de diagnostic),
mais parce que le laisser dupliquer la séquence est précisément ce que la garde
AC3 doit interdire, et qu'une exception nommée coûterait plus que le routage.

### Hors périmètre, explicitement

- **Les vues de diagnostic de `crates/mika-cli/src/commands/skills.rs`**
  (`:29`, `:57`, `:63`, `:1715`). Elles font un `from_dir` nu — sans allowlist
  d'identité, sans ban, sans contrôle — et `:1715` (`coherence_registry`) est
  délibérément la vue **allowlist-UNAWARE** de la surface d'outils du disque, un
  axe orthogonal à l'autorité par agent. Elles n'assemblent pas le registre d'un
  agent vivant : la garde AC3 doit les laisser passer, et son prédicat est conçu
  pour cela (§ *AC3 — forme du prédicat*).
- **Toute autre convergence de la TUI**, qui reste l'objet de mika#1727. Ce vol
  extrait l'assemblage et rien d'autre : pas de convergence du cycle de vie du
  worker, pas d'unification du `skills_dirty`.
- `apply_only_skills` (mika#2363), qui restreint un **clone** par tour et n'est
  jamais réécrit dans le registre partagé. Ce n'est pas un assemblage.

### La signature, et pourquoi elle est synchrone

```rust
// crates/mika-agent/src/skills/assembly.rs
pub fn assemble_agent_registry(
    skills_dir: &Path,
    identity: &crate::prompt::Identity,
    overrides: &[SkillOverride],
    agent_id: &str,
) -> SkillRegistry
```

Trois décisions, chacune contrainte par un site réel :

- **Synchrone, et les overrides en paramètre.** `teams/engine.rs:319` lit les
  overrides sur une `Database` **synchrone** (`db.get_skill_overrides(&ta.name)`),
  avant la construction de l'`AsyncDatabase`. Rendre la fonction `async` ou lui
  faire chercher ses overrides elle-même imposerait de réordonner l'init
  d'équipe autour de `register_agent`, `seed_core_memory_if_empty` et
  `migrate_disabled_markers`, qui prennent tous `&mut db` — une réécriture
  invasive hors du périmètre du ticket. Chaque appelant garde la charge de lire
  ses overrides par la porte qu'il a déjà ouverte.
- **`&Identity`, pas l'allowlist nue.** Les neuf sites tiennent déjà une
  `Identity` (ou l'obtiennent par `crate::prompt::load_identity`). Prendre
  l'identité rend « j'ai oublié l'allowlist » **inexprimable** : l'omission
  n'est plus un oubli possible à l'appel, elle serait un bug dans un corps unique.
- **`agent_id: &str`** parce que `apply_required_tools_coherence_check` en a
  besoin pour nommer l'agent dans ses skips. Les sites le tiennent sous trois
  noms (`agent_name`, `ctx.async_db.agent_id()`, `ta.name`) ; le paramètre les
  réconcilie sans les renommer.

La fonction vit dans `crates/mika-agent/src/skills/assembly.rs` — un module
frère de `skills/mod.rs`, pas dans `mod.rs` (4184 lignes, et le fichier porte
déjà les huit `apply_*`). `mika-cli` dépend de `mika-agent`, donc les quatre
sites de la TUI l'atteignent sans nouvelle dépendance.

---

## Verification Contract

### AC1 / AC2 — le rouge-avant, et ce qu'il atteint vraiment

Les quatre sites de rechargement à chaud vivent dans des fonctions privées
`async` inatteignables depuis un test unitaire (`run_a2a_agent`, le corps du
handler HTTP, deux blocs dans la boucle `tokio::spawn` du worker TUI). **Le
rouge-avant est donc obtenu par l'ordre de commit**, et non par un test qui
traverse le handler.

Ce que les tests atteignent : la fonction **partagée** par les quatre sites.
Ce qu'ils n'atteignent pas : l'`AppState` des handlers. Ce qui lie l'un à
l'autre : la garde AC3, dont c'est la raison d'être structurelle. Cette borne
est déclarée ici parce qu'un plan qui prétendrait traverser les handlers
mentirait sur sa couverture.

- **AC1** — un test pose dans un `skills_dir` temporaire une skill que
  `apply_load_safety_check` écarte (handler cassé / `tools.json` illisible),
  appelle `assemble_agent_registry`, et exige que la skill soit **écartée**
  (`skipped`, absente de `skills()`).
- **AC2** — même forme pour `apply_required_tools_coherence_check` : une skill
  déclarant un `required_tools` que l'allowlist d'identité ne résout pas, et
  l'exigence d'un skip `coherence:`.

Les deux vivent dans le `mod tests` **déjà présent** de `skills/mod.rs`, pour
réutiliser ses fabriques (`create_valid_skill`, `registry_from_temp`,
`entry_with_required_tools`, `entry_requiring_and_providing`, `registry_of`)
plutôt que d'en bâtir de nouvelles. C'est une réduction de volume, pas une
commodité : une fixture neuve coûterait ici ~35 lignes pour une couverture
identique.

### AC3 — forme du prédicat

**La garde prédique sur la FORME, jamais sur une liste de fichiers** (doctrine
mika#2309). Prédicat :

> Dans la **moitié de production** de `crates/mika-agent/src/` et de
> `crates/mika-cli/src/`, hors lignes de commentaire, aucun fichier autre que
> `skills/assembly.rs` ne contient à la fois `from_dir` et l'un des quatre
> marqueurs d'assemblage d'agent vivant :
> `.apply_identity_allowlist(`, `.apply_testimony_grade_ban(`,
> `.apply_load_safety_check(`, `.apply_required_tools_coherence_check(`.

Quatre propriétés du prédicat, chacune mesurée sur l'arbre actuel :

- **`.apply_overrides(` est délibérément hors du motif.**
  `mika-cli/src/commands/skills.rs` l'atteint par
  `apply_db_overrides_if_available`, et c'est une vue de diagnostic légitime.
  Les quatre marqueurs retenus ne concernent qu'un agent vivant : l'allowlist
  **d'identité**, le ban, et les deux contrôles.
- **Ancrage sur le point d'appel de méthode** (`.apply_…(` et non
  `apply_…`), parce que la prose *à propos* d'une méthode n'est pas un appel —
  `skills.rs:1713` et `:1744` nomment
  `apply_required_tools_coherence_check` dans des commentaires de trois lignes
  (forme du Signal S, mika#2050). Les lignes de commentaire sont retirées en
  plus, par `crate::source_scan::strip_comment_lines`.
- **Moitié de production** via `mika_common::source_guard::ProductionScanner`
  (`dev-dependency` `test-utils`, déjà déclarée par les deux crates), qui masque
  les régions `#[cfg(test)]` **et** les fichiers test-only — donc la garde ne
  voit ni `well_known_agents.rs § tests`, ni les `mod tests` de `chat.rs`. Le
  scan des deux crates résout la racine du workspace par
  `env!("CARGO_MANIFEST_DIR").parent().parent()`, le motif de
  `mika_common::home § mika2230_le_tier_a_un_seul_analyseur`.
- **Contrôle négatif sur contenu synthétique**, pas sur un corpus sur disque :
  `ProductionScanner::production_of_content(path, content)` accepte un contenu
  arbitraire, donc le contrôle passe au **même** prédicat une chaîne portant un
  second appelant et exige qu'il la signale. ~15 lignes, aucune fixture, aucun
  fichier sous `.pilot-scratch/`.

### AC4 — non-régression de l'ensemble chargé

Ancré sur le motif existant de
`crates/mika-agent/src/well_known_agents.rs:1790-1815`
(`test_well_known_agents_pass_required_tools_coherence`) : `seed_bundled_skills`
sème la bibliothèque complète dans un `tempdir`, puis pour chaque
`(agent, allowlist)` on compare l'ensemble des skills rendu par
`assemble_agent_registry` à celui rendu par la séquence manuelle de l'init
serveur. Égalité exigée, et aucun skip `coherence:` ni `load:`.

C'est ce test qui dit que l'extraction n'a **rien changé** pour les agents que
l'init servait déjà correctement — la propriété que ni AC1, ni AC2, ni AC3 ne
peuvent voir.

### Commandes

```
cargo test -p mika-agent skills::assembly
cargo test -p mika-agent skills::tests
cargo test -p mika-agent well_known_agents
cargo test -p mika-cli
cargo clippy --all-targets -- -D warnings
cargo fmt --check
scripts/verify-pipeline.sh origin/main
```

---

## Ordre de commit — le rouge-avant est ici

L'ordre est le dispositif, pas une préférence de style. Chaque commit est
nommé par ce qu'il rend observable.

| # | contenu | état de la suite |
|---|---|---|
| **C1** | `skills/assembly.rs` avec **cinq** étapes (1-4 et 7), **sans** les deux contrôles. Routage des **quatre rechargements à chaud seulement** (`a2a.rs`, `handlers.rs`, `chat.rs:308`, `chat.rs:386`). | **vert** — le corps est byte-pour-byte la séquence que ces quatre sites portaient déjà ; aucun comportement ne bouge, aucun site d'init n'est touché |
| **C2** | les tests AC1 et AC2, contre `assemble_agent_registry`. | **ROUGE** — la fonction n'écarte pas la skill, exactement comme les rechargements à chaud sur `main`. C'est l'attestation d'AC1 « il rougit sur main, sur un rechargement à chaud au moins » |
| **C3** | ajout de `apply_load_safety_check()` puis `apply_required_tools_coherence_check(agent_id)` dans la fonction. Routage des cinq sites restants : `server/mod.rs`, `chat.rs:137`, `delegate_task.rs`, `teams/engine.rs`, `list_skills.rs`. | **vert** — AC1 et AC2 passent ; les deux sites d'init conservent leur comportement, les deux sites d'agent vivant non listés le gagnent |
| **C4** | garde AC3 + contrôle négatif, allowlist **vide**. | **vert** — l'allowlist ne peut être vide qu'après C3, qui a routé le dernier site. L'ordre C3 → C4 est ce qui rend l'allowlist vide atteignable plutôt que souhaitée |
| **C5** | test AC4 de non-régression. | **vert** |

Un relecteur vérifie le rouge-avant par `git checkout C2 && cargo test -p mika-agent skills::tests` — deux échecs nommés, et rien d'autre.

---

## Definition of Done

- `assemble_agent_registry` existe dans `crates/mika-agent/src/skills/assembly.rs`
  et porte les sept étapes dans l'ordre de l'init serveur.
- Les six sites nommés par le ticket l'appellent, ainsi que
  `delegate_task.rs`, `teams/engine.rs` et `list_skills.rs`.
- Aucun site de production ne duplique la séquence d'assemblage d'un agent
  vivant ; la garde AC3 l'atteste avec une allowlist vide et un contrôle négatif.
- Le rouge-avant est attesté par l'ordre de commit : C2 rougit, C3 verdit.
- `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`
  et `scripts/verify-pipeline.sh origin/main` passent.
- Aucune autre convergence de la TUI n'est livrée (mika#1727 reste ouvert sur
  son périmètre).

## Acceptance criteria

- AC1 : rouge-avant. Un test pose une skill que le contrôle de chargement écarte, déclenche un rechargement à chaud et exige qu'elle soit écartée. Il rougit sur main, sur un rechargement à chaud au moins.
- AC2 : même chose pour le contrôle de cohérence des outils requis.
- AC3 : une garde de source exige qu'il n'existe qu'**un seul** appel de production à `SkillRegistry::from_dir` qui assemble un registre d'agent vivant, hors outils de diagnostic et tests. Son allowlist est livrée vide, avec un contrôle négatif.
- AC4 : l'ensemble des skills chargées à l'initialisation est inchangé (non-régression).

---

## Fire-Disposition

Ce plan livre trois détecteurs : les tests AC1/AC2, le test de non-régression
AC4, et la garde de source AC3.

**Option (a) — exception nommée en allowlist, allowlist livrée VIDE.**

C'est le choix du ticket lui-même (« Son allowlist est livrée vide, avec un
contrôle négatif »), et il est **atteignable** ici, ce qui n'est pas toujours le
cas : à l'instant où la garde atterrit (C4), C3 a déjà routé les neuf sites de
production, donc la population des violations résiduelles est vide **par
mesure**, pas par espoir.

Détail d'implémentation :

```rust
/// Sites de production qui dupliquent l'assemblage d'un agent vivant.
///
/// Livrée VIDE (mika#2889 AC3) et tenue vide par l'assertion ci-dessous.
/// Une entrée future doit nommer le chemin EXACT, référencer son ticket de
/// suivi, et mourir avec lui — pas « les fichiers de la TUI ».
const ASSEMBLY_DUPLICATES_ALLOWLIST: &[&str] = &[];
```

Les trois propriétés exigées par mika#1574 (a) :

- **grep-visible** — une entrée est un chemin littéral dans un `const` nommé ;
  `grep -n ASSEMBLY_DUPLICATES_ALLOWLIST` rend la population entière.
- **assertion auto-nettoyante** — un test frère
  (`mika2889_lallowlist_de_la_garde_est_livree_vide`, motif mika#2201/mika#2425)
  exige `ASSEMBLY_DUPLICATES_ALLOWLIST.is_empty()`. Elle rougit au premier ajout,
  donc l'exception ne peut pas être silencieuse. Et la garde elle-même rougit
  quand une entrée devient **stale** : un chemin allowlisté qui ne porte plus de
  violation est signalé comme entrée périmée — le geste qui rend l'exception
  inutile est celui qui la fait expirer, ce qui est ce qui distingue cette forme
  de l'allowlist que mika#2309 condamne.
- **ticket référencé** — le doc-comment de la constante l'exige par écrit, et
  une entrée sans référence est un refus de revue.

Aucun détecteur n'atterrit désarmé : pas de `#[ignore]`, pas de `#[cfg(skip)]`.
Le seul détecteur rouge du vol est C2, et son rouge est l'attestation d'AC1 —
C3, dans la même PR, le verdit.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/skills/assembly.rs` (fonction + doc-comment de la règle) | 50 |
| routage des 9 sites de production (6 du ticket + `delegate_task.rs` + `teams/engine.rs` + `list_skills.rs`) | 50 |
| tests AC1 + AC2 (`crates/mika-agent/src/skills/mod.rs` § `mod tests`) | 70 |
| garde AC3 + contrôle négatif + assertion d'allowlist vide (`skills/assembly.rs` § `mod tests`) | 65 |
| test AC4 de non-régression (`crates/mika-agent/src/well_known_agents.rs` § `mod tests`) | 35 |

Total estimé : 270 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 270 < 279, et la marge n'est pas
une estimation optimiste : elle vient de quatre réutilisations mesurées sur
l'arbre, chacune remplaçant un livrable que ce plan aurait dû écrire.

1. **Les tests AC1/AC2 vivent dans le `mod tests` existant de `skills/mod.rs`**
   et consomment ses cinq fabriques (`create_valid_skill` écrit un vrai
   `skill.toml` + `system_prompt.md`, `registry_from_temp`,
   `entry_with_required_tools`, `entry_requiring_and_providing`, `registry_of`).
   Un module de test neuf aurait coûté ~35 lignes de fixture pour la même
   couverture. La règle de visibilité qui le permet : un module enfant voit les
   items privés de son parent, et les champs `skills / skipped / disabled /
   validated_warnings` sont privés.
2. **La garde AC3 ne construit aucune infrastructure de scan.**
   `mika_common::source_guard::ProductionScanner` est `pub` sous la feature
   `test-utils`, déjà déclarée en `dev-dependency` par `mika-agent`
   (`Cargo.toml:84`) et `mika-cli` (`Cargo.toml:67`), et il rend la moitié de
   production **y compris le masquage des fichiers test-only** — le défaut
   mika#2321 que corrigerait sinon une troncature maison au premier
   `#[cfg(test)]`. Reste à écrire le prédicat, la boucle sur les deux crates et
   l'assertion : ~50 lignes au lieu de ~120.
3. **Le contrôle négatif n'a pas de corpus.**
   `production_of_content(path, content)` prend un contenu en mémoire, donc le
   contrôle est un `&str` littéral passé au prédicat extrait : ~15 lignes, aucune
   fixture sur disque, aucun `.pilot-scratch/`.
4. **AC4 est un ancrage, pas un fichier d'or.** Il se greffe sur le `seed_bundled_skills`
   de `well_known_agents.rs:1804`, déjà écrit, déjà vert.

Et le périmètre est **borné par construction** : le ticket exclut nommément
toute autre convergence de la TUI (mika#1727), et ce plan exclut en plus les
quatre vues de diagnostic de `mika-cli/src/commands/skills.rs`, dont le
prédicat d'AC3 est conçu pour ne pas les toucher. Il n'y a donc pas de dérive
de périmètre disponible à l'implémentation.

**La soupape, si l'implémentation déborde malgré tout.** Le livrable sécable est
le routage des deux sites de **diagnostic** et d'agent-vivant-non-listés
(`list_skills.rs`, et à la limite `delegate_task.rs`) : chacun deviendrait une
entrée d'allowlist nommée au sens de la Fire-Disposition (a), avec son ticket de
suivi. Ce n'est **pas** le plan — AC3 exige une allowlist vide et ce vol la
livre vide — c'est le seul recours qui préserve les quatre AC sans dépasser la
borne, et il doit être consigné dans le corps de PR s'il est pris.

---

## Risques

- **Un `teams/engine.rs` rendu `async` par accident.** Le site lit ses overrides
  sur une `Database` synchrone ; une signature `async` y propage un
  réordonnancement de trois appels `&mut db`. La signature synchrone est la
  mitigation, et elle est tenue par le compilateur.
- **Les deux contrôles sur un site qui ne les avait pas peuvent écarter des
  skills qui chargeaient.** C'est le but du ticket, pas une régression — mais
  le passage est **observable** : `log_summary()` et les skips `load:` /
  `coherence:` nomment chaque skill écartée. AC4 borne le risque côté init ; côté
  rechargement à chaud, la convergence est précisément ce que la règle demande.
- **Le prédicat d'AC3 pourrait attraper une vue de diagnostic future** qui
  appliquerait l'allowlist d'identité sans être un agent vivant. La réponse
  prescrite est le routage par la fonction unique, jamais l'ajout d'une entrée
  d'allowlist — c'est écrit dans le doc-comment de la garde (doctrine mika#2201).
