# mika#1727 — vol V3c-2b1 : la source des skills bascule sur la route

**Ticket** : senara-solutions/mika#1727 · **Vol** : V3c-2b1 (coupe B, sous-vol B1,
choisie par Vincent le 2026-11-17) · **Branche** :
`feat/1727/v3c2b1-la-source-des-skills-bascule-sur-la-route`, neuve depuis main
(`7f2e5531`) · **Label** : `loop-substrate`

---

## HALTE — borne du groom dépassée

| unité | estimé | borne | verdict |
|---|---|---|---|
| `git diff --numstat origin/main...HEAD`, plan exclu | **543** | 475 (tête du vol) | **dépasse de 68** |
| estimation hors `docs/` et hors `CLAUDE.md` | **539** | 452 (tête du vol) | **dépasse de 87** |
| estimation hors `docs/` et hors `CLAUDE.md` | **539** | 429 (mika#2693) | **dépasse de 110** |

La tête de ce vol dit : « Si le premier dépasse 475, ou le second 452, le groom
s'arrête et le dit. Il ne recoupe pas lui-même. » Ce plan s'arrête donc ici sur la
décision, et **ne choisit aucune coupe** : la coupe de ce vol est celle de la tête,
B1, rien de plus, rien de moins. Le détail poste par poste est en
§ *Taille estimée*, et § *Pourquoi 543 et non 393* dit où ce chiffrage s'écarte de
celui du plan 004.

**Ce que la halte ne dit pas** : la halte du **pilote** (560 après chaque commit)
n'est pas atteinte. Le plan ci-dessous est donc exécutable tel quel si Vincent lève
la borne du groom ; aucune de ses lignes ne suppose une coupe.

**Ce que ce plan n'a pas fait** : il n'a pas raboté de commentaires pour tenir
(interdit par la tête), n'a pas reporté de test à un vol suivant (interdit par la
tête), et n'a pas élargi le périmètre à un fichier exclu. Le critère 10 tient sans
toucher `crates/mika-agent/` — voir § *L'adaptateur n'a besoin d'aucun constructeur
neuf*, qui est le point que la tête demandait de vérifier avant tout.

---

## L'état de main, mesuré sur 7f2e5531

**L'assemblage** est dans `spawn_agent_worker` (`commands/chat.rs:194-208`) :
`migrate_disabled_markers_async` (l. 237-245), `get_skill_overrides` (l. 250-254),
`assemble_agent_registry` (l. 255-260). Le registre sort en position 7 du tuple de
retour (l. 206) et se pose dans `app.skills` au démarrage (l. 541), à la bascule
d'agent (l. 807) et au `/restart` (l. 915). Les deux sites de respawn relisent
`new_skills` **du tuple** : ils n'ont donc aucune ligne à changer, et le critère 5
tombe par construction dès que `spawn_agent_worker` lit la route.

**Les cinq lecteurs** :

| lecteur | site | ce qu'il lit | poids |
|---|---|---|---|
| message de démarrage « skills ignorées » | `chat.rs:579-600` | `app.skills.skipped()` | 22 l. |
| message de démarrage « avertissements » | `chat.rs:602-630` | `app.skills.validated_warnings()` | 29 l. |
| autocomplétion de `/skill` | `completers.rs:258-276` | `ctx.skills.skills()` via `CompletionContext.skills` (`autocomplete.rs:20`), passé depuis `input.rs:792` | 19 l. |
| `/skills` | `handlers.rs:906-1005` | `skills()` **et** `skipped()` | 100 l. |
| `/skill <nom>` | `handlers.rs:1060-1130` | `skills()` seul | 71 l. |

**Le miroir** (`skills_remote.rs:38`) porte `agent`, `skills`, `skipped`,
`warnings`, **tous champs `pub(crate)`** — vérifié : aucun accesseur à écrire.
`SkillMirror` a ses 13 champs `pub(crate)`, `VariantsMirror` ses deux.

**Les six attentes** `#[cfg_attr(not(test), expect(dead_code, reason = "câblé au
vol V3c-2b de mika#1727"))]` sont aux l. 32-36, 44-48, 72-76, 85-89, 99-103 et
219-222 : **quatre lignes chacune**, sur `SkillsMirror`, `SkillMirror`,
`VariantsMirror`, `SkippedMirror`, `WarningMirror` et `lire`.

**`mod tests` de `skills_remote.rs` (l. 279-280) est privé.** C'est le point dur de
l'ordre des commits : `miroir_de_test()` n'est atteignable depuis `tui/` qu'avec
`#[cfg(test)] pub(crate) mod tests`, la forme exacte de `rewind_remote.rs:524-525`.
Un `pub(crate) fn` dans un module privé reste invisible, et le pilote le
découvrirait à la compilation.

**`const CORPS` existe déjà** (`skills_remote.rs:298-313`) et satisfait **tous** les
cas que la tête exige d'un miroir de test : `self-dev` (outils 2, override,
variantes `total: 3` sur `{anthropic: 2, groq: 0}` — donc deux providers dont un
sans modèle), `weather` (désactivée, sans outil, sans variante), une ignorée
(`broken`), un avertissement (`self-dev`). `miroir_de_test()` le **désérialise**
plutôt que de recopier la forme : un seul lieu dit le contrat de fil, et le
constructeur prouve au passage que `CORPS` se relit.

**`updated_skills`** : la tête compte quatre sites d'écriture ; il y en a **cinq**,
dont un de test — `chat.rs:360`, `chat.rs:367`, `chat_turn.rs:228`,
`chat_turn.rs:246` (le tour de callback en porte **deux**, succès et erreur) et
`app.rs:2417`. Le lecteur est `app.rs:1344-1348`, la déclaration `app.rs:332`. Le
critère 6 exige zéro occurrence dans tout `crates/mika-cli/` ; un `grep` final
tranche.

**Le serveur** calcule `variants` ainsi (`server/dashboard.rs:1331-1363`) :
`total = entry.variant_count()` et `providers = { p → entry.variant_models(p).len() }`
pour `p ∈ entry.variant_providers()`. C'est l'invariant que l'adaptateur doit
rejouer à l'envers.

---

## L'adaptateur n'a besoin d'aucun constructeur neuf

La tête demandait de s'arrêter si un critère exigeait un constructeur neuf dans
`crates/mika-agent/`. Mesuré : il n'en exige aucun.

- `SkillRegistry::from_test_entries(Vec<SkillEntry>)` est **`pub`**
  (`skills/mod.rs:315`) et pose `skills` ; `skipped`, `disabled` et
  `validated_warnings` restent vides. C'est précisément la raison pour laquelle la
  tête fait lire `skipped` directement sur le miroir.
- `SkillEntry` a ses **dix** champs `pub` (`skills/index.rs:132-158`).
- `SkillManifest` a ses sept champs `pub` (`manifest.rs:24-51`) ; cinq ont un
  `Default` (`LlmOverride`, `Constraints`, `Output` via `impl Default` l. 266,
  `VariantsConfig`, et `HashMap` pour `context`).
- `SkillInfo` (`manifest.rs:351`) et `Triggers` (`manifest.rs:380`) : champs `pub`,
  `DataGrade` a un `Default`.
- `ResolvedSkillTool` (`index.rs:53`), `ToolDefinition`
  (`mika-common/src/claude.rs:249`) et `ToolHandler` (`manifest.rs:392`) : tous
  publics et constructibles.

Une tension reste, et elle se dit dans le code plutôt que se répare : le doc de
`from_test_entries` dit « for integration tests », et l'adaptateur l'appelle en
production. Le commentaire de l'adaptateur le nomme — c'est le seul constructeur
public qui pose `skills`, et l'alternative serait un constructeur neuf dans un
crate que ce vol n'ouvre pas.

### L'invariant des clés de variantes

`/skill` lit trois choses de l'entrée : `variant_count()`, `variant_providers()` et
`variant_models(p).len()`. Du côté des champs, `variant_count()` compte l'**union**
des clés de `provider_overrides`, de tous les `prompt_sources` et de
`model_overrides` ; une clé de provider (`"anthropic"`) ne porte pas de `/`, une clé
de modèle (`"anthropic/x"`) en porte un. D'où :

```
total = Σ_p n_p + |clés de provider|          avec n_p = |variant_models(p)|
providers (le set) = clés de provider ∪ { p : n_p > 0 }
```

L'adaptateur reconstruit donc, pour chaque skill du miroir, en laissant
`prompt_sources` vide (aucun lecteur ici) :

1. pour chaque `(p, n)` de `variants.providers` avec `n > 0` : `n` clés
   `"{p}/m{i}"` dans `model_overrides` ;
2. pour chaque `(p, 0)` : la clé `"{p}"` dans `provider_overrides` — un provider
   sans modèle **est** une clé de provider, sinon il ne serait pas dans le set ;
3. l'écart `E = total − (clés posées)` se comble par des clés de provider sur les
   providers à `n > 0`, dans l'ordre du `BTreeMap`. `E ≥ 0` et
   `E ≤ |{p : n_p > 0}|` sur tout miroir que la route a servi, donc la
   reconstruction est exacte.

Sur un miroir incohérent — un `total` plus grand que ce que les clés permettent —
l'adaptateur **ne fabrique aucun provider** pour combler : il rend le total qu'il
peut poser. Inventer un provider serait inventer une provenance (mika#2331). Sur
`miroir_de_test()`, `self-dev` donne `E = 3 − (2 + 1) = 0` : le cas nominal ne passe
pas par l'étape 3, et un test la couvre à part.

### Les valeurs neutres, et pourquoi

Six champs n'ont aucune source dans le miroir, et le rendu des deux handlers n'en
lit aucun. Chacun porte un commentaire disant cela, pas une paraphrase du code :
`keywords_lower` (`Vec::new()` — le filtrage par mot-clé est le travail du
serveur), `prompt_snippet` (`String::new()`), `prompt_sources` (`Vec::new()` — voir
l'invariant ci-dessus), `ToolDefinition.description` / `input_schema` (vides : seul
`definition.name` est rendu), `ToolHandler` (`Builtin { function: String::new() }` —
la variante sans commande ni URL ; rien n'appelle jamais ce handler, le registre
meurt à la fin du handler de slash-commande), `skill_dir` (`PathBuf::new()`), et
dans `SkillInfo` : `dependencies`, `max_prompt_size`, `data_grade`.

---

## Périmètre

**Changent** : `commands/chat.rs`, `commands/chat_turn.rs` (pour `updated_skills`
seul), `commands/skills_remote.rs`, `tui/app.rs`, `tui/input.rs`,
`tui/commands/autocomplete.rs`, `tui/commands/completers.rs`,
`tui/commands/handlers.rs`, et `crates/mika-cli/CLAUDE.md` aux **deux** phrases
nommées par la tête — « l'assemblage de démarrage reste, une fois » et « Rien ne
l'appelle ».

**Ne changent pas** : tout `crates/mika-agent/`, `session_remote.rs`,
`rewind_remote.rs`, `docs/egress/` (l'entrée `cli-spirit-skills` reste juste : le
client se construit toujours dans `skills_remote.rs`), `crates/mika-common/`,
`crates/mika-a2a/`, `crates/mika-gateway/`, `dashboard/`, et aucun fichier du plan
de contrôle.

**Hors de ce vol** : la lecture directe du miroir par `/skills` et `/skill`, le
retrait de l'adaptateur et de ses tests, la garde neuve interdisant `SkillRegistry`
dans `tui/`, la doc complète — tout cela est B2. Puis V3c-2c : le test É1.

---

## L'état, et les quatre surfaces

`pub(crate) type EtatSkills = Result<SkillsMirror, String>;` vit dans
`skills_remote.rs`, près du miroir. Un `String` et non `SkillsRemoteIssue` : la
cause est figée en **une ligne** à l'instant de la lecture, et ce que les quatre
surfaces ont à dire est cette ligne, pas un vocabulaire à rejouer. `App.skills`
devient `Arc<EtatSkills>`.

Une lecture en échec **n'est pas fatale** (défaut 14) : `spawn_agent_worker` rend
`Err(cause)` dans le tuple et la TUI démarre.

| surface | sur un miroir lu | sur échec |
|---|---|---|
| `/skills` | le rendu d'aujourd'hui, `SKIPPED` comprise | la cause, **jamais** « No skills loaded. » |
| `/skill <nom>` | le rendu d'aujourd'hui | la cause |
| autocomplétion de `/skill` | les noms du miroir | une liste **vide** |
| démarrage | les deux messages d'aujourd'hui | un message `System` qui dit la cause |

Le mode équipe pose `Arc::new(Ok(SkillsMirror::default()))` : « lu et vide » est
distinct de « pas lu », et les handlers vérifient `is_team_mode()` avant d'y
toucher. `SkillsMirror` gagne `Default` dans sa derive — ses quatre champs en ont
un.

### Les trois fonctions pures de démarrage

Extraites de `chat.rs:579-630`, elles prennent `&EtatSkills` et rendent
`Option<String>` — rien du monde en argument, donc testables sans É1 ni TUI :

- `message_des_skills_ignorees` — « ⚠ {n} skill(s) skipped at startup: », cinq au
  plus, puis « ... and {n} more. Run `mika skills validate` for details. » ;
- `message_des_avertissements` — « ⚠ {n} skill(s) loaded with validation
  warnings: », et pour chaque skill **le premier message** (`messages.first()`, là
  où le code lit `diagnostics.first()`) ;
- `message_de_cause` — `None` sur `Ok`, sinon la ligne de cause.

Le texte reste celui d'aujourd'hui à donnée égale : les `format!` et `writeln!`
déménagent sans se réécrire.

### `lire_les_skills`

```rust
async fn lire_les_skills(base_url: &str, lire_env: SourceEnv<'_>, agent: &str) -> EtatSkills
```

La forme d'`ouvrir_session_par_spirit` (`chat.rs:70`) : tout ce qui touche le monde
arrive par argument, donc le test se joue contre `fake_spirit` sans variable
d'environnement. Elle prend le jeton par `session_remote::jeton_interne` et rend
`Err(issue.to_string())` sur refus. L'appel vit dans `spawn_agent_worker`, avec
`dashboard::spirit_url()` comme l. 226-234.

---

## Ordre des commits

Chacun compile et passe ses tests. Les deux contraintes qui fixent cet ordre :
`App.skills` change de type d'un seul geste (six fichiers en dépendent et aucun
sous-ensemble ne compile), et **aucun item ne peut rester sans appelant de
production** — le critère 9 interdit toute attente neuve, donc l'adaptateur arrive
avec ses deux appelants.

**Commit 1 — `feat(1727): la TUI lit ses skills sur la route (vol V3c-2b1)`.**
`EtatSkills`, `Default` sur le miroir, `#[cfg(test)] pub(crate) mod tests`,
`miroir_de_test()`, `miroir_en_registre` ; `lire_les_skills` et le retrait du bloc
d'assemblage et des deux `use` de `chat.rs` ; `App.skills` et le mode équipe ; les
trois fonctions de démarrage ; `complete_skill` et le type de
`CompletionContext.skills` ; les pré-filtres et l'adaptateur dans `handle_skills` et
`handle_skill`, avec `skipped` lu sur le miroir. Les six attentes tombent ici : les
cinq miroirs ont un lecteur de production par l'adaptateur et les messages de
démarrage, `lire` par `lire_les_skills`. Tous les tests des critères 1 à 5, et les
cinq constructeurs de test migrent sur `miroir_de_test()`.

**Commit 2 — `refactor(1727): updated_skills quitte la réponse du worker`.** Le
champ de `AgentResponse::Reply`, son lecteur `app.rs:1344-1348`, les quatre sites de
production et le site de test. `chat_turn.rs` n'ajoute qu'une ligne : la réécriture
du passage de doc l. 160 qui décrit le champ.

**Commit 3 — `test(1727): la garde de chat.rs refuse l'assemblage local des
skills`.** Quatre motifs ajoutés à `MOTIFS` de
`mika1727_v3b1_le_tour_de_la_tui_ne_construit_plus_son_provider` — après les commits
1 et 2 seulement, sinon elle rougit sur son propre arbre :

| motif | raison |
|---|---|
| `assemble_agent_registry` | le registre arrive par la route, pas par le disque |
| `migrate_disabled_markers_async` | spirit migre les marqueurs à l'`init_agent` |
| `get_skill_overrides` | les overrides sont déjà appliqués dans le miroir |
| `updated_skills` | plus de rechargement à chaud : la route est relue au respawn |

`ALLOWED` reste vide et asserté vide. Le nom de la garde ne change pas : c'est une
garde **par fichier**, pas par vol, et la renommer casserait la trace des trois vols
qui la citent. Son témoin (`async fn spawn_agent_worker(`) et son contre-témoin sur
`chat.rs` sont inchangés et couvrent les quatre motifs neufs sans ajout.

**Commit 4 — `docs(1727): la TUI lit la route des skills`.** Les deux phrases de
`crates/mika-cli/CLAUDE.md`, et rien d'autre.

Après chaque commit, le pilote mesure `git diff --numstat origin/main...HEAD`, plan
exclu. **Au-dessus de 560 il s'arrête** : pas de commit suivant, pas de PR, et son
dernier message dit « HALTE borne : N lignes pour 560 » avec le compte par commit.

---

## Comment chaque critère se mesure

| critère | mesure |
|---|---|
| 1 | `handle_skills` et `handle_skill` sur `miroir_de_test()`, comparés à une chaîne **littérale** écrite dans le test. Le corps de rendu ne change pas — seule sa source change — donc l'égalité à donnée égale tient par construction, et le littéral la verrouille contre une régression |
| 2 | les mêmes lecteurs sur `Err("…")` : la sortie contient la cause, et `assert!(!sortie.contains("No skills loaded."))` — un `assert` **négatif**, pas une relecture ; `complete_skill` rend `vec![]` ; `message_de_cause` rend `Some(cause)` |
| 3 | trois états : `Ok` à deux listes vides → « No skills loaded. » ; `Ok` à `skipped` seul → section `SKIPPED` et pas la phrase ; `Err` → la cause |
| 4 | `message_des_skills_ignorees` et `message_des_avertissements` sur `miroir_de_test()`, textes comparés littéralement |
| 5 | `lire_les_skills` appelée avec deux noms d'agent contre `fake_spirit` : la requête reçue porte l'agent visé (`RequeteRecue`). Le respawn lui-même relit par construction — les deux sites prennent `new_skills` du tuple |
| 6 | `grep -rn 'updated_skills' crates/mika-cli/` rend zéro ligne ; le compilateur le confirme |
| 7 | `grep -rn 'miroir_en_registre' crates/mika-cli/src/` : la définition, `handle_skills`, `handle_skill`, et rien d'autre hors tests |
| 8 | la garde étendue jouée contre l'arbre de main rougit sur les quatre motifs ; verte sur la branche |
| 9 | `grep -c 'expect(dead_code' crates/mika-cli/src/commands/skills_remote.rs` rend 0 ; `cargo clippy --all-targets -p mika-cli` vert ; aucun `allow` neuf au `git diff` |
| 10 | `git diff --name-only origin/main...HEAD` ; `git diff origin/main...HEAD -- crates/mika-cli/CLAUDE.md` ne touche que les deux phrases |
| 11 | les quatre commandes dans cet ordre, puis `bash scripts/verify-pipeline.sh origin/main` |
| 12 | `gh pr view <N> --json closingIssuesReferences` rend `[]` |

---

## Definition of Done

- `app.skills` porte le miroir lu sur `GET /api/v1/skills?agent=<nom>`, ou la cause
  en une ligne ; aucun assemblage local ne subsiste dans `crates/mika-cli/`.
- Les quatre surfaces de lecture disent la cause sur échec, et la TUI démarre.
- `/skills` et `/skill` passent par `miroir_en_registre`, dont le registre vit le
  temps de l'appel et n'est jamais rangé dans `App` ; `/skills` lit `skipped`
  directement sur le miroir.
- `updated_skills` n'existe plus dans `crates/mika-cli/`.
- Les six attentes `dead_code` de `skills_remote.rs` sont tombées ; aucune attente
  ni aucun `allow` neuf.
- La garde `mika1727_v3b1_…` porte quatre motifs de plus, chacun avec sa raison, et
  rougit contre l'arbre de main.
- `cargo build -p mika-cli --bin mika`, puis `cargo test -p mika-cli`,
  `cargo test -p mika-common --test claude_md_size`,
  `cargo clippy --all-targets -p mika-cli` et
  `bash scripts/verify-pipeline.sh origin/main` sont verts.
- La PR porte `Refs mika#1727`, aucun `Closes`/`Fixes`/`Resolves`, et
  `Code review: delegated to mika-qa (mika#2694)`.

---

## Acceptance criteria

Transcrits du corps de mika#1727, § *Critères de V3c-2b1* :

1. Au démarrage, contre un spirit qui sert la route, `app.skills` porte le miroir de
   l'agent, sans aucun assemblage local. `/skills` et `/skill <nom>` rendent, à
   donnée égale, ce qu'ils rendent aujourd'hui, comparé littéralement sur
   `miroir_de_test()`, section `SKIPPED` comprise.
2. Contre un spirit qui refuse la route, la TUI démarre. `/skills` et `/skill`
   disent la cause, jamais « No skills loaded. ». L'autocomplétion de `/skill` est
   vide, et un message `System` de démarrage dit la cause.
3. « No skills loaded. » ne s'affiche que sur un miroir lu dont `skills` et
   `skipped` sont vides. Un miroir qui n'a que des skills ignorées rend la section
   `SKIPPED`.
4. Les messages de démarrage des skills ignorées et des avertissements se rendent
   depuis le miroir, avec le même texte qu'aujourd'hui à donnée égale.
5. Une bascule d'agent et un `/restart` relisent la route pour l'agent visé.
6. `updated_skills` n'existe plus dans `crates/mika-cli/`.
7. Dans la production de `crates/mika-cli/src/`, `miroir_en_registre` n'apparaît
   qu'à sa définition et dans `handle_skills` et `handle_skill`.
8. La garde existante, étendue de quatre motifs, rougit contre l'arbre de main et
   passe sur la branche.
9. Les seules attentes retirées sont les six de `skills_remote.rs`. Aucune attente
   ni aucun `allow` neuf.
10. Seuls changent les fichiers listés plus haut, et `crates/mika-cli/CLAUDE.md` ne
    change qu'aux deux phrases nommées.
11. `cargo build -p mika-cli --bin mika` d'abord, puis `cargo test -p mika-cli` et
    `cargo test -p mika-common --test claude_md_size` sont verts.
    `cargo clippy --all-targets -p mika-cli` est vert. Le pilote lance
    `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il le
    trouve rouge, il s'arrête et le dit.
12. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
    compris si le gabarit de reprise l'ouvre.

---

## Fire-Disposition

Ce vol livre un détecteur : l'extension de
`mika1727_v3b1_le_tour_de_la_tui_ne_construit_plus_son_provider` par quatre motifs.

**Option retenue (mika#1574) : (c) halte-et-escalade.**

La garde est livrée **armée** et verte sur la branche : les quatre motifs
disparaissent de la production de `chat.rs` dans les commits 1 et 2, et le commit 3
les interdit. `ALLOWED` est livrée vide et **assertée vide** dans le corps du test.
Quand elle rougira, le remède est la porte que chaque motif nomme — router le nouvel
appelant par la route — **jamais** une entrée d'allowlist (mika#2309, mika#2398,
mika#2201) : une allowlist peuplée est exactement ce qui ne refuse plus rien. Si un
appelant futur ne peut pas être routé, le geste est de s'arrêter et de le déclarer à
Vincent, pas d'ouvrir la garde.

Les tests de rendu de l'adaptateur sont des tests de comportement, pas des
détecteurs structurels : ils ne portent pas de disposition propre.

---

## Taille estimée

| livrable | fichier | ajoutées (numstat) | dont hors docs |
|---|---|---|---|
| `EtatSkills`, `Default` sur le miroir, `mod tests` en `pub(crate)`, `miroir_de_test()` par `CORPS`, retrait des six attentes | `commands/skills_remote.rs` | 27 | 27 |
| `miroir_en_registre` : l'invariant des clés de variantes et les six valeurs neutres commentées | `commands/skills_remote.rs` | 78 | 78 |
| Tests de l'adaptateur : total et providers reproduits, l'écart non nul, les noms d'outils | `commands/skills_remote.rs` | 42 | 42 |
| `lire_les_skills`, son appel, retrait du bloc d'assemblage et des deux `use` | `commands/chat.rs` | 39 | 39 |
| Les trois fonctions pures de démarrage et leurs appels | `commands/chat.rs` | 68 | 68 |
| Tests de `lire_les_skills` contre `fake_spirit` : succès, refus, agent visé | `commands/chat.rs` | 36 | 36 |
| Tests des trois messages de démarrage, textes littéraux | `commands/chat.rs` | 48 | 48 |
| Extension de `mika1727_v3b1_…` : quatre motifs et leurs raisons | `commands/chat.rs` | 16 | 16 |
| Champ `skills`, `App::new`, mode équipe, retrait de `updated_skills` et de son lecteur, site de test | `tui/app.rs` | 8 | 8 |
| Type du contexte d'autocomplétion | `tui/commands/autocomplete.rs` | 2 | 2 |
| `complete_skill` sur le miroir, banc de test, test de la liste vide | `tui/commands/completers.rs` | 27 | 27 |
| Pré-filtres de `handle_skills` et `handle_skill`, `skipped` lu sur le miroir, appel de l'adaptateur | `tui/commands/handlers.rs` | 42 | 42 |
| Tests des deux lecteurs : rendus littéraux, cause, « No skills loaded. », `SKIPPED` seul, deux tests existants adaptés | `tui/commands/handlers.rs` | 100 | 100 |
| Banc de test de l'autocomplétion | `tui/input.rs` | 3 | 3 |
| Retrait de `updated_skills` et réécriture du passage de doc | `commands/chat_turn.rs` | 3 | 3 |
| Les deux phrases | `crates/mika-cli/CLAUDE.md` | 4 | 0 |

Total estimé : 543 lignes

Estimation hors `docs/` et hors `CLAUDE.md` : 539 lignes

Plafond de tours : 200 (loop-substrate)

Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

`Total estimé :` porte l'unité `git diff --numstat origin/main...HEAD`, plan exclu.
Un bloc qui **déménage** y compte deux fois, en ajouté et en retiré : les trois
fonctions de démarrage reprennent 51 lignes de `chat.rs:579-630` et en ajoutent 68
ailleurs dans le même fichier.

**Les tests sont comptés** : 256 des 539 lignes hors docs (42 dans
`skills_remote.rs`, 84 dans `chat.rs`, 100 dans `handlers.rs`, 12 dans
`completers.rs`, 3 dans `input.rs`, et les 16 de la garde, qui **est** son test).
Aucun ne se reporte à un vol suivant.

**Les lignes jetables** : 120 — l'adaptateur (78) et ses tests (42) — livrées ici et
retirées en B2, auxquelles s'ajouteront les tests de rendu par l'adaptateur que B2
réécrira en lecture directe.

### Pourquoi 543 et non 393

Le plan 004 (59c94bb1) chiffrait B1 à 393. Ce plan le lit comme référence et ne le
recopie pas ; trois postes expliquent l'essentiel des 150 lignes d'écart, et chacun
vient d'une mesure que 004 n'avait pas faite :

- **L'adaptateur : 120 contre ~90.** 004 l'estimait sans avoir lu `SkillEntry`
  (dix champs), `SkillManifest` (sept) ni `SkillInfo` (neuf). Le compte réel est de
  dix-sept champs à poser, dont six sans source dans le miroir — chacun avec son
  commentaire, que la tête interdit de raboter — plus la reconstruction des clés de
  variantes, qui est un invariant à écrire, pas une affectation.
- **Les tests des deux lecteurs : 100 contre ~60.** Le critère 1 demande une
  comparaison **littérale**, section `SKIPPED` comprise : la chaîne attendue de
  `/skills` sur `miroir_de_test()` fait une douzaine de lignes à elle seule, celle
  de `/skill` une dizaine. S'y ajoutent les deux `assert` négatifs du critère 2 et
  les trois états du critère 3.
- **L'extension de la garde : 16 contre 6.** 004 comptait les quatre motifs sans
  leurs raisons ni l'ajustement du doc-comment de la garde, qui énumère ce qu'elle
  couvre.

Le reste des postes est à moins de dix lignes de 004, et `miroir_de_test()` est
**plus court** que ce que 004 prévoyait (11 contre ~40) : `CORPS` existe déjà et
couvre tous les cas que la tête exige, donc le constructeur le désérialise au lieu
de redire la forme.

---

Tracked in: senara-solutions/mika#1727
