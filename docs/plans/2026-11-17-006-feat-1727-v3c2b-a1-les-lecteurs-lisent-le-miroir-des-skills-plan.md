# mika#1727 — vol V3c-2b-A1 : les lecteurs des skills lisent le miroir

**Ticket** : senara-solutions/mika#1727 · **Vol** : V3c-2b-A1 (coupe A, sous-vol A1,
choisie par Vincent le 2026-11-17) · **Branche** :
`feat/1727/v3c2b-a1-les-lecteurs-lisent-le-miroir-des-skills`, neuve depuis main
(`7f2e5531`) · **Label** : `loop-substrate`

**Les bornes tiennent, et ce plan ne s'arrête pas.** 469 lignes pour 560, 465 pour
555 — voir § *Taille estimée*, qui dit aussi pourquoi ce chiffrage n'est ni celui de
004 ni celui de 005.

---

## Ce que ce vol livre, et ce qu'il ne livre pas

A1 bascule la **source** et fait lire le miroir **directement** par les quatre
surfaces. Aucun adaptateur, aucune projection vers `SkillRegistry` : c'est la
différence entre la coupe A et la coupe B, et c'est elle qui fait tomber les 120
lignes jetables que le plan 005 (`39174bc6`) chiffrait pour B1.

**Hors de ce vol, en A2** : la garde neuve qui interdit `SkillRegistry` dans la
production de `tui/`, avec témoin et contre-témoin, et la doc complète de
`crates/mika-cli/CLAUDE.md`. **Puis en V3c-2c** : le test É1. A1 porte l'extension
de la garde **existante** (quatre motifs, critère 8) et les **deux** phrases de
`CLAUDE.md` que son merge rend fausses — rien de plus.

**Ce plan ne choisit aucune coupe** : la coupe est celle de la tête du ticket, et il
n'ajoute aucune halte aux siennes.

---

## L'état de main, mesuré sur `7f2e5531`

### La source — l'assemblage local

`commands/chat.rs::spawn_agent_worker` (l. 194-208) assemble sur le disque en trois
gestes : `migrate_disabled_markers_async` (l. 236-245), `get_skill_overrides`
(l. 250-254), `assemble_agent_registry` (l. 255-260). Le registre sort en **position
7** du tuple de retour (l. 206, `Arc<SkillRegistry>`) et se pose dans `app.skills`
au démarrage (l. 541), à la bascule d'agent (l. 807) et au `/restart` (l. 915).

**Les deux sites de respawn relisent `new_skills` du tuple** (l. 807 et 915,
vérifié) : ils n'ont aucune ligne de logique à changer, et le critère 5 tombe par
construction dès que `spawn_agent_worker` lit la route.

Deux `use` partent avec l'assemblage : `chat.rs:31` (`SkillRegistry`) et
`chat.rs:32` (`assemble_agent_registry`).

### Les lecteurs — recensement fermé

| surface | site | ce qu'il lit | poids |
|---|---|---|---|
| démarrage, skills ignorées | `chat.rs:578-600` | `app.skills.skipped()` | 23 l. |
| démarrage, avertissements | `chat.rs:602-632` | `app.skills.validated_warnings()` | 31 l. |
| autocomplétion de `/skill` | `completers.rs:258-276` | `ctx.skills.skills()` via `CompletionContext.skills` (`autocomplete.rs:20`), passé depuis `input.rs:792` | 19 l. |
| `/skills` | `handlers.rs:906-1004` | `skills()` **et** `skipped()` | 99 l. |
| `/skill <nom>` | `handlers.rs:1060-1128` | `skills()` seul | 69 l. |

**Quatre surfaces, cinq sites.** La tête compte « quatre lecteurs » parce qu'elle
groupe les deux messages de démarrage ; le code en a deux blocs distincts, à deux
accesseurs différents. Ce n'est pas une divergence de périmètre, et le plan le note
pour que le pilote n'en laisse pas un derrière lui.

`handle_skills` ouvre par un early-return **`"No skills loaded."`** (l. 910-912) —
le texte que le critère 2 interdit sur un échec de lecture et que le critère 3
réserve au miroir lu et vide.

### Le miroir, et ce qu'il couvre champ par champ

`skills_remote.rs:38` — `SkillsMirror { agent, skills, skipped, warnings }`, **tous
champs `pub(crate)`** : aucun accesseur à écrire. `SkillMirror` (l. 50) porte ses
treize champs `pub(crate)`, `VariantsMirror` (l. 79) ses deux.

**Chaque champ lu par les cinq sites a son correspondant**, vérifié un à un :
`entry.manifest.skill.name` → `name`, `.description` → `description`, `.always_on` →
`always_on`, `.version` → `version`, `.timeout_secs` → `timeout_secs`,
`entry.enabled` → `enabled`, `entry.has_override` → `has_override`,
`entry.skill_tools.len()` → `tools`, les noms d'outils → `tool_names`,
`m.triggers.keywords` → `keywords`, `entry.dir.display()` → `path`,
`entry.variant_count()` → `variants.total`, et le couple
`variant_providers()` / `variant_models(p).len()` → `variants.providers`
(`BTreeMap<String, usize>`, dont l'ordre stable est la raison écrite sur le type).

**Aucune dérivation n'est nécessaire.** C'est le fait qui rend la coupe A plus
courte que la coupe B : l'adaptateur de B1 existait pour reconstruire un
`SkillRegistry` dont les clés de variantes devaient rejouer à l'envers l'invariant
du serveur (plan 005, § *L'invariant des clés de variantes*, 78 lignes plus 42 de
tests). En lecture directe, `variants.total` et `variants.providers` **sont** ce que
`/skill` affiche.

### Le constructeur de test, et `CORPS`

`mod tests` de `skills_remote.rs` (l. 279-280) est **privé**. C'est le point dur de
l'ordre des commits : une `pub(crate) fn` dans un module privé reste inatteignable
depuis `tui/`, et le pilote le découvrirait à la compilation. La forme est
`#[cfg(test)] pub(crate) mod tests`, celle de `rewind_remote.rs:524-525`, dont le
banc (`RequeteRecue`, `fake_spirit`) est déjà emprunté par ce fichier.

**`const CORPS` existe déjà** (`skills_remote.rs:298-313`) et satisfait **tous** les
cas que la tête exige d'un miroir de test : `self-dev` (2 outils nommés, override,
variantes `total: 3` sur `{anthropic: 2, groq: 0}` — deux providers dont un sans
modèle), `weather` (désactivée, sans outil, sans variante), une ignorée (`broken`),
un avertissement (`self-dev`). `miroir_de_test()` le **désérialise** au lieu de
recopier la forme : un seul lieu dit le contrat de fil, et le constructeur prouve au
passage que `CORPS` se relit.

### Les six attentes

`#[cfg_attr(not(test), expect(dead_code, reason = "câblé au vol V3c-2b de mika#1727"))]`,
**quatre lignes chacune**, sur `SkillsMirror` (l. 32-36), `SkillMirror` (l. 44-48),
`VariantsMirror` (l. 72-76), `SkippedMirror` (l. 85-89), `WarningMirror` (l. 99-103)
et `lire` (l. 219-222). Six, exactement — c'est le compte du critère 9.

### `updated_skills`

Déclaration `app.rs:332`, lecteur `app.rs:1344-1349`, **quatre sites de production**
— `chat.rs:360`, `chat.rs:367`, `chat_turn.rs:228`, `chat_turn.rs:246` (le tour de
callback en porte **deux**, succès et erreur) — plus **un site de test**,
`app.rs:2417`. La tête compte quatre sites d'écriture : c'est juste pour la
production. Le critère 6 exige zéro occurrence dans tout `crates/mika-cli/` ; un
`grep` final tranche, et il doit aussi emporter le commentaire `chat.rs:343` et le
passage de doc `chat_turn.rs:160` qui décrivent le champ.

### Les cinq constructeurs de `SkillRegistry` en test

`completers.rs:397` (`Box::leak` d'un `SkillRegistry::empty()`), `input.rs:1148`,
`handlers.rs:1572` (`test_app`), `handlers.rs:2115` (`with_skipped`, deux
`SkippedSkill`), `app.rs:2357`. Tous à router par `miroir_de_test()`.

**Après A1, `SkillRegistry` n'est plus nommé nulle part dans `tui/`** — ni en
production, ni en test. C'est ce qui fait naître vide la garde d'A2, et c'est
vérifié ici plutôt que supposé là-bas.

### La garde existante

`mika1727_v3b1_le_tour_de_la_tui_ne_construit_plus_son_provider` (`chat.rs:1803`) :
`ProductionScanner::for_crate`, `ALLOWED` vide et **assertie vide**, `MOTIFS` à huit
couples motif→remède, `SITES` à deux couples fichier→témoin (`chat.rs` avec
`async fn spawn_agent_worker(`, `ask.rs` avec son littéral), contre-témoin sur
`chat.rs` seulement, avec la raison écrite.

---

## Périmètre

**Changent**, et rien d'autre :

- `crates/mika-cli/src/commands/chat.rs`
- `crates/mika-cli/src/commands/chat_turn.rs` — pour `updated_skills` seul
- `crates/mika-cli/src/commands/skills_remote.rs` — `EtatSkills`, `Default`, le
  constructeur de test partagé, retrait des six attentes
- `crates/mika-cli/src/tui/app.rs`
- `crates/mika-cli/src/tui/input.rs`
- `crates/mika-cli/src/tui/commands/autocomplete.rs`
- `crates/mika-cli/src/tui/commands/completers.rs`
- `crates/mika-cli/src/tui/commands/handlers.rs`
- `crates/mika-cli/CLAUDE.md` — aux **deux** phrases nommées par la tête

**Ne changent pas** : tout `crates/mika-agent/`, `session_remote.rs`,
`rewind_remote.rs`, `docs/egress/`, `crates/mika-common/`, `crates/mika-a2a/`,
`crates/mika-gateway/`, `dashboard/`.

**Aucun fichier du plan de contrôle** : ni `.github/workflows/`, ni `.claude/`, ni
`skills/bundled/`, ni `crates/mika-agent/src/well_known_agents.rs`, ni `.mika/`, ni
`docs/egress/`, ni aucun chemin de CODEOWNERS.

**Aucun critère n'exige de toucher un fichier exclu**, et les trois points qui
auraient pu le demander sont vérifiés :

1. La route sert déjà les quatre clés depuis V3c-1 et V3c-2a-i : le serveur n'a rien
   à gagner.
2. La migration des marqueurs `.disabled` est déjà faite côté serveur à
   l'`init_agent` (`mika-agent/src/server/mod.rs:670`) : la retirer de la TUI ne la
   perd pas.
3. La lecture directe du miroir **n'appelle aucun constructeur** de
   `crates/mika-agent/` — c'était le besoin de l'adaptateur de B1, et il disparaît
   avec lui. L'entrée d'egress `cli-spirit-skills` reste juste telle quelle : le
   client se construit toujours dans `skills_remote.rs`.

---

## L'état, et les quatre surfaces

`pub(crate) type EtatSkills = Result<SkillsMirror, String>;` vit dans
`skills_remote.rs`, près du miroir. Un `String` et non `SkillsRemoteIssue` : la cause
est figée en **une ligne** à l'instant de la lecture, et ce que les quatre surfaces
ont à dire est cette ligne, pas un vocabulaire à rejouer.

`App.skills` devient `Arc<EtatSkills>`. Une lecture en échec **n'est pas fatale**
(défaut 14) : `spawn_agent_worker` rend `Err(cause)` en position 7 de son tuple et la
TUI démarre.

| surface | sur un miroir lu | sur échec |
|---|---|---|
| `/skills` | le rendu d'aujourd'hui, section `SKIPPED` comprise | la cause, **jamais** « No skills loaded. » |
| `/skill <nom>` | le rendu d'aujourd'hui | la cause |
| autocomplétion de `/skill` | les noms du miroir | une liste **vide** |
| démarrage | les deux messages d'aujourd'hui | un message `System` qui dit la cause |

Le mode équipe (`app.rs:822`) pose `Arc::new(Ok(SkillsMirror::default()))` : « lu et
vide » est distinct de « pas lu », et les handlers vérifient `is_team_mode()` avant
d'y toucher. `SkillsMirror` gagne `Default` dans sa derive — ses quatre champs en
ont un.

### Les trois fonctions pures de démarrage

Extraites de `chat.rs:578-632`, elles prennent `&EtatSkills` et rendent
`Option<String>` — **rien du monde en argument**, donc testables sans É1 ni TUI.
Sans cette extraction, les critères 2 et 4 ne sont pas mesurables avant V3c-2c :

- `message_des_skills_ignorees` — « ⚠ {n} skill(s) skipped at startup: », cinq au
  plus, puis « ... and {n} more. Run `mika skills validate` for details. » ;
- `message_des_avertissements` — « ⚠ {n} skill(s) loaded with validation
  warnings: », et pour chaque skill **le premier message** (`messages.first()`, là où
  le code lit `diagnostics.first()`), avec le fallback `"unknown issue"` ;
- `message_de_cause` — `None` sur `Ok`, sinon la ligne de cause.

**Le texte reste celui d'aujourd'hui à donnée égale** : les `format!` et `writeln!`
déménagent sans se réécrire, et les tests les comparent littéralement.

### `lire_les_skills`

```rust
async fn lire_les_skills(base_url: &str, lire_env: SourceEnv<'_>, agent: &str) -> EtatSkills
```

La forme d'`ouvrir_session_par_spirit` (`chat.rs:70`) : tout ce qui touche le monde
arrive par argument, donc le test se joue contre `fake_spirit` sans variable
d'environnement. Elle prend le jeton par `session_remote::jeton_interne`
(`session_remote.rs:177`, déjà importé `chat.rs:56`) et rend `Err(issue.to_string())`
sur refus — y compris sur jeton absent, qui est un refus comme un autre ici puisque
la TUI démarre quand même. L'appel vit dans `spawn_agent_worker`, avec
`dashboard::spirit_url()` comme l. 226-234.

### Les lecteurs, en direct

Chacun des cinq sites gagne un pré-filtre de quatre à six lignes sur `&EtatSkills`,
puis lit le miroir. Le corps de rendu **ne change pas de forme** : seule sa source
change, accès par accès, selon la correspondance vérifiée en § *Le miroir*. C'est ce
qui rend l'égalité « à donnée égale » vraie par construction, et les littéraux des
tests la verrouillent contre une régression.

Pour `/skills`, `skipped` se lit **directement sur le miroir** et non derrière une
projection : c'est la liste que `SkillRegistry::from_test_entries` ne savait pas
poser, et la raison pour laquelle la coupe B devait de toute façon la traiter à part.

---

## Ordre des commits

Chacun compile et passe ses tests. La contrainte qui fixe cet ordre : **`App.skills`
change de type d'un seul geste** — six fichiers en dépendent et aucun sous-ensemble
ne compile. Et **aucun item ne peut rester sans appelant de production**, puisque le
critère 9 interdit toute attente neuve : les six attentes tombent donc dans le commit
qui pose leurs lecteurs.

**Commit 1 — `feat(1727): les lecteurs des skills lisent le miroir (vol V3c-2b-A1)`.**
`skills_remote.rs` : `EtatSkills`, `Default` sur le miroir,
`#[cfg(test)] pub(crate) mod tests`, `miroir_de_test()` par `CORPS`, retrait des six
attentes. `chat.rs` : `lire_les_skills`, son appel, le type du tuple, retrait du bloc
d'assemblage et des deux `use` ; les trois fonctions pures de démarrage et leurs
appels. `app.rs` : le champ, `App::new`, le mode équipe. `autocomplete.rs` : le type
du contexte. `completers.rs` : `complete_skill` sur le miroir. `handlers.rs` :
`handle_skills` et `handle_skill` en lecture directe. `input.rs` : le banc.
Les cinq constructeurs de test migrent sur `miroir_de_test()`. **Tous les tests des
critères 1 à 5** : `lire_les_skills` contre `fake_spirit` (succès, refus, agent
visé), les trois messages de démarrage en littéral, les deux lecteurs en succès et
en échec, l'autocomplétion vide, les trois états de « No skills loaded. ».

**Commit 2 — `refactor(1727): updated_skills quitte la réponse du worker`.** Le champ
`app.rs:332`, les quatre sites de production, le site de test `app.rs:2417`, le
commentaire `chat.rs:343` et le passage de doc `chat_turn.rs:160`.

> **Amendement du pilote (2026-11-17, écart sur l'affectation d'une ligne).** Le
> **lecteur** `app.rs:1344-1349` passe au **commit 1**, et non au commit 2 comme
> écrit ci-dessus. Son corps est `self.skills = new_skills;` : il pose un
> `Arc<SkillRegistry>` dans un champ qui devient `Arc<EtatSkills>` au commit 1, donc
> l'affecter au commit 2 rend le commit 1 non compilable — ce que la première phrase
> de cette section interdit (« Chacun compile et passe ses tests »). Au commit 1 le
> champ reste donc déclaré et ignoré au motif (`updated_skills: _`) ; le commit 2
> retire la déclaration, le motif et les cinq sites. Rien d'autre ne change : le
> contenu livré, le nombre de commits et les critères sont ceux du plan.

**Commit 3 — `test(1727): la garde de chat.rs refuse l'assemblage local des skills`.**
Quatre motifs ajoutés à `MOTIFS` — **après** les commits 1 et 2 seulement, sinon elle
rougit sur son propre arbre :

| motif | raison |
|---|---|
| `assemble_agent_registry` | le registre arrive par la route, pas par le disque |
| `migrate_disabled_markers_async` | spirit migre les marqueurs à l'`init_agent` |
| `get_skill_overrides` | les overrides sont déjà appliqués dans le miroir |
| `updated_skills` | plus de rechargement à chaud : la route est relue au respawn |

`ALLOWED` reste vide et assertie vide. **Le nom de la garde ne change pas** : c'est
une garde **par fichier**, pas par vol, et la renommer casserait la trace des trois
vols qui la citent. Son témoin (`async fn spawn_agent_worker(`) et son contre-témoin
sur `chat.rs` sont inchangés et couvrent les quatre motifs neufs sans ajout — le
doc-comment de la garde, qui énumère ce qu'elle couvre, s'ajuste.

**Commit 4 — `docs(1727): la TUI lit la route des skills`.** Les deux phrases de
`crates/mika-cli/CLAUDE.md`, et rien d'autre : « l'assemblage de démarrage reste, une
fois » (l. 735) et « **Rien ne l'appelle** » (l. 764). La doc complète est en A2.

**Halte du pilote.** Après chaque commit, le pilote mesure
`git diff --numstat origin/main...HEAD`, plan exclu, code, commentaires, doc et tests
compris. **Si le total dépasse 720, il s'arrête** : pas de commit suivant, pas de PR,
et son dernier message dit « HALTE borne : N lignes pour 720 » avec le compte par
commit. Cette halte est celle de la tête ; ce plan ne la change pas.

---

## Definition of Done

- `app.skills` porte le miroir lu sur `GET /api/v1/skills?agent=<nom>`, ou la cause
  en une ligne ; aucun assemblage local ne subsiste dans `crates/mika-cli/`.
- Les quatre surfaces lisent le miroir **directement**, sans projection vers
  `SkillRegistry`, et disent la cause sur échec ; la TUI démarre.
- `SkillRegistry` n'est plus nommé dans `crates/mika-cli/src/tui/`, ni en production
  ni en test.
- `updated_skills` n'existe plus dans `crates/mika-cli/`.
- Les six attentes `dead_code` de `skills_remote.rs` sont tombées ; aucune attente ni
  aucun `allow` neuf.
- La garde `mika1727_v3b1_…` porte quatre motifs de plus, chacun avec sa raison, et
  rougit contre l'arbre de main.
- `cargo build -p mika-cli --bin mika` **d'abord**, puis `cargo test -p mika-cli`,
  `cargo test -p mika-common --test claude_md_size`,
  `cargo clippy --all-targets -p mika-cli` et
  `bash scripts/verify-pipeline.sh origin/main` sont verts. Si le dernier est rouge,
  le pilote s'arrête et le dit.
- Les neuf fichiers listés sont les seuls modifiés
  (`git diff --name-only origin/main...HEAD`, plan exclu).
- La PR porte `Refs mika#1727` et **aucun** `Closes`, `Fixes` ni `Resolves` — y
  compris si le gabarit de reprise (`wip-rescue`) l'ouvre, auquel cas le `Closes #N`
  par défaut est corrigé avant toute revue et
  `gh pr view <N> --json closingIssuesReferences` rend `[]`.
- La PR porte `Tracked in: senara-solutions/mika#1727` et
  `Code review: delegated to mika-qa (mika#2694)`.
- Aucun ticket n'est ouvert (gel jusqu'au 01/12) : un manque ou un reste va en
  commentaire sur mika#1727 ou dans la PR.

---

## Acceptance criteria

Transcrits du corps de mika#1727, § *Critères de V3c-2b-A1* :

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
7. Aucun lecteur des skills de la TUI ne passe par un `SkillRegistry` :
   `handle_skills`, `handle_skill`, `complete_skill` et les messages de démarrage
   lisent le miroir.
8. La garde existante, étendue de quatre motifs, rougit contre l'arbre de main et
   passe sur la branche.
9. Les seules attentes retirées sont les six de `skills_remote.rs`. Aucune attente ni
   aucun `allow` neuf.

   > **Amendement du pilote (2026-11-17, écart mesuré sur deux champs du miroir).**
   > Les six attentes de **type** tombent comme écrit, et **deux attentes de champ**
   > les remplacent : `SkillsMirror::agent` et `SkillMirror::origin`. Ce sont les
   > deux seuls champs du miroir qu'aucune des quatre surfaces de lecture ne lit —
   > `handle_skill` sur `origin/main` n'affichait ni l'un ni l'autre, et le critère 1
   > (« rendu identique au caractère près ») interdit de leur donner un lecteur en
   > les affichant. Mesuré par `cargo clippy --all-targets -p mika-cli`, qui rend
   > `field agent is never read` et `field origin is never read` dès que les six
   > attentes de type partent ; le plan n'avait pas mesuré ce reste.
   >
   > **Les retirer du miroir est interdit par le périmètre de ce vol**, et c'est ce
   > qui fixe le choix : `crates/mika-cli/CLAUDE.md` l. 764 écrit que `lire` « rend
   > un miroir des quatre clés de la route, **treize champs par skill** », et cette
   > phrase n'est pas l'une des deux que ce vol a le droit de toucher (l. 735 et
   > l. 764 § « Rien ne l'appelle »). Un miroir à douze champs la rendrait fausse en
   > silence, et romprait le contrat de fil que `mika1727_le_miroir_relit_tout_ce_que_la_route_sert`
   > atteste depuis V3c-2a-ii : le miroir relit **tout** ce que la route sert, parce
   > qu'un champ absent du miroir se lit comme un champ absent de la route
   > (motif mika#2331).
   >
   > Les deux attentes gardent la forme de celles qu'elles remplacent —
   > `#[cfg_attr(not(test), expect(dead_code, reason = "…"))]`, le module de tests
   > les lisant — et chacune porte sa raison : le champ est au miroir parce que la
   > route le sert, et sans lecteur dans la TUI parce qu'aucune des quatre surfaces
   > ne l'affiche. Elles sont donc **portées**, pas neuves : la dette passe de six
   > types entiers à deux champs nommés. Rien d'autre ne change : le contenu livré,
   > le nombre de commits et les autres critères sont ceux du plan.
10. Seuls changent les fichiers listés plus haut, et `crates/mika-cli/CLAUDE.md` ne
    change qu'aux deux phrases nommées.
11. `cargo build -p mika-cli --bin mika` d'abord, puis `cargo test -p mika-cli` et
    `cargo test -p mika-common --test claude_md_size` sont verts.
    `cargo clippy --all-targets -p mika-cli` est vert. Le pilote lance
    `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il le
    trouve rouge, il s'arrête et le dit.
12. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y compris
    si le gabarit de reprise l'ouvre.

### Comment chaque critère se mesure

| critère | mesure |
|---|---|
| 1 | `handle_skills` et `handle_skill` sur `miroir_de_test()`, comparés à une chaîne **littérale** écrite dans le test. Le corps de rendu ne change pas de forme — seule sa source change — donc l'égalité à donnée égale tient par construction, et le littéral la verrouille |
| 2 | les mêmes lecteurs sur `Err("…")` : la sortie contient la cause, et `assert!(!sortie.contains("No skills loaded."))` — un `assert` **négatif**, pas une relecture ; `complete_skill` rend `vec![]` ; `message_de_cause` rend `Some(cause)` |
| 3 | trois états : `Ok` à deux listes vides → « No skills loaded. » ; `Ok` à `skipped` seul → section `SKIPPED` et **pas** la phrase ; `Err` → la cause |
| 4 | `message_des_skills_ignorees` et `message_des_avertissements` sur `miroir_de_test()`, textes comparés littéralement |
| 5 | `lire_les_skills` appelée avec deux noms d'agent contre `fake_spirit` : la requête reçue porte l'agent visé (`RequeteRecue`). Le respawn lui-même relit par construction — les deux sites prennent `new_skills` du tuple |
| 6 | `grep -rn 'updated_skills' crates/mika-cli/` rend zéro ligne ; le compilateur le confirme |
| 7 | `grep -rn 'SkillRegistry' crates/mika-cli/src/tui/` rend zéro ligne, production **et** test ; le type de `App.skills` le garantit pour la production |
| 8 | la garde étendue jouée contre l'arbre de main rougit sur les quatre motifs ; verte sur la branche |
| 9 | `grep -c 'expect(dead_code' crates/mika-cli/src/commands/skills_remote.rs` rend **2** — les deux attentes de champ de l'amendement ci-dessus, portant sur `SkillsMirror::agent` et `SkillMirror::origin`, et aucune attente de type ; `cargo clippy --all-targets -p mika-cli` vert ; aucun `allow` neuf au `git diff` |
| 10 | `git diff --name-only origin/main...HEAD` ; `git diff origin/main...HEAD -- crates/mika-cli/CLAUDE.md` ne touche que les deux phrases |
| 11 | les quatre commandes dans cet ordre, puis `bash scripts/verify-pipeline.sh origin/main` |
| 12 | `gh pr view <N> --json closingIssuesReferences` rend `[]` |

---

## Fire-Disposition

Ce vol livre un détecteur : l'extension de
`mika1727_v3b1_le_tour_de_la_tui_ne_construit_plus_son_provider` par quatre motifs
(`assemble_agent_registry`, `migrate_disabled_markers_async`, `get_skill_overrides`,
`updated_skills`).

**Option retenue (mika#1574) : (c) halte-et-remontée.**

La garde est livrée **armée** et verte sur la branche : les quatre motifs
disparaissent de la production de `chat.rs` aux commits 1 et 2, et le commit 3 les
interdit. `ALLOWED` est livrée vide et **assertie vide** dans le corps du test. Quand
elle rougira, `cargo test -p mika-cli` est rouge, le pilote s'arrête et le dit ; le
remède est la porte que chaque motif nomme — router le nouvel appelant par la route
—, **jamais** une entrée d'allowlist (mika#2309, mika#2398, mika#2201) : une
allowlist peuplée est exactement ce qui ne refuse plus rien. Si un appelant futur ne
peut pas être routé, le geste est de s'arrêter et de le déclarer à Vincent, pas
d'ouvrir la garde.

**Pourquoi pas (a) exception nommée en allowlist** : la population attendue à la
livraison est **zéro** — les quatre motifs quittent la production dans ce vol même.
Une allowlist qui ne naît pas vide ne mesure plus rien, et les gardes V1, V2a,
V2c-1b, V2d-4a/b et V3b-1 de ce crate l'ont toutes refusé.

**Pourquoi pas (b) livrer désarmé** : la tête l'interdit nommément (« Chaque garde
arrive avec ses tests, dans le même commit »), et une garde que rien ne joue refuse
en silence (classe mika#2578 / mika#2752). Ici elle est jouée par
`cargo test -p mika-cli`, que le critère 11 exige vert.

Les tests de rendu des lecteurs sont des tests de **comportement**, pas des
détecteurs structurels : ils ne portent pas de disposition propre.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `EtatSkills`, `Default`, `mod tests` en `pub(crate)`, `miroir_de_test()` par `CORPS`, retrait des six attentes (`commands/skills_remote.rs`) | 27 |
| `lire_les_skills`, son appel, le type du tuple, retrait du bloc d'assemblage et des deux `use` (`commands/chat.rs`) | 39 |
| Les trois fonctions pures de démarrage et leurs appels (`commands/chat.rs`) | 68 |
| Tests de `lire_les_skills` contre `fake_spirit` : succès, refus, agent visé (`commands/chat.rs`) | 36 |
| Tests des trois messages de démarrage, textes littéraux (`commands/chat.rs`) | 48 |
| Extension de `mika1727_v3b1_…` : quatre motifs, leurs raisons, le doc-comment (`commands/chat.rs`) | 16 |
| Champ `skills`, `App::new`, mode équipe, retrait de `updated_skills` et de son lecteur, site de test (`tui/app.rs`) | 8 |
| Type du contexte d'autocomplétion (`tui/commands/autocomplete.rs`) | 2 |
| `complete_skill` sur le miroir, banc de test, test de la liste vide (`tui/commands/completers.rs`) | 27 |
| `handle_skills` et `handle_skill` en lecture directe, avec leurs pré-filtres (`tui/commands/handlers.rs`) | 88 |
| Tests des deux lecteurs : rendus littéraux, cause, « No skills loaded. », `SKIPPED` seul, deux tests existants adaptés (`tui/commands/handlers.rs`) | 100 |
| Banc de test de l'autocomplétion (`tui/input.rs`) | 3 |
| Retrait de `updated_skills` et réécriture du passage de doc (`commands/chat_turn.rs`) | 3 |
| Les deux phrases (`crates/mika-cli/CLAUDE.md`) | 4 |

Total estimé : 469 lignes

Estimation hors `docs/` et hors `CLAUDE.md` : 465 lignes

Plafond de tours : 200 (loop-substrate)

Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

### Les bornes, et laquelle mord

| unité | estimé | borne | verdict |
|---|---|---|---|
| `git diff --numstat origin/main...HEAD`, plan exclu | **469** | 560 (tête du vol) | tient, 91 de marge |
| estimation hors `docs/` et hors `CLAUDE.md` | **465** | 555 (tête du vol) | tient, 90 de marge |
| estimation hors `docs/` et hors `CLAUDE.md` | 465 | 429 (mika#2693) | dépassée de 36 |

**La borne de mika#2693 est dépassée, et ce plan ne s'arrête pas sur elle** : la tête
de ce vol écrit « Ces deux bornes sont posées par Vincent pour ce vol, au-dessus de
la borne par vol de mika#2693 ; le plan cite cette décision au lieu de s'arrêter sur
429. » Les bornes qui mordent sont donc 560 et 555, et les deux tiennent.

**Pourquoi ce vol tient en un seul vol** : la coupe est déjà faite. A1 est le premier
des deux sous-vols de la coupe A, et A2 — la garde neuve sur `tui/` et la doc
complète — est nommé hors périmètre par la tête et par la ligne `Tracked in:`.

`Total estimé :` porte l'unité `git diff --numstat origin/main...HEAD`, plan exclu.
Un bloc qui **déménage** y compte deux fois, en ajouté et en retiré : les trois
fonctions de démarrage reprennent 54 lignes de `chat.rs:578-632` et en ajoutent 68
ailleurs dans le même fichier.

**Les tests sont comptés** : 214 des 465 lignes hors docs (84 dans `chat.rs`, 100
dans `handlers.rs`, 12 dans `completers.rs`, 3 dans `input.rs`, et les 16 de la
garde, qui **est** son test). Aucun ne se reporte à un vol suivant.

**Les commentaires sont comptés et ne se raboteront pas pour tenir.** Les 469 lignes
incluent le pourquoi de `lire_les_skills` (pourquoi par argument), celui du `String`
d'`EtatSkills` (pourquoi pas le vocabulaire de refus), celui de chaque motif neuf de
la garde, celui des pré-filtres (pourquoi la cause avant « No skills loaded. ») et
celui de `miroir_de_test()` par `CORPS`. Aucune n'est une paraphrase du code.

**Lignes jetables : zéro.** C'est la propriété de la coupe A, et la raison pour
laquelle elle a été retenue : B1 en portait 120 (l'adaptateur, 78, et ses tests, 42),
livrées pour être retirées en B2.

### Pourquoi 469, et non 499 ni 543

Les plans 004 (`59c94bb1`) et 005 (`39174bc6`) sont lus comme chiffrages de référence
et non recopiés. Trois postes expliquent l'écart, et chacun vient d'une mesure que
l'un des deux n'avait pas faite :

- **004 chiffrait A1 à 499**, mais il plaçait l'extension de `v3b1` **et** la doc en
  A2 ; la tête de ce vol les met en A1. À périmètre égal, 004 vaudrait donc 499 + 6 +
  25 = 530. L'écart avec 469 vient surtout des deux handlers : 004 les comptait à
  **155 ajoutées**, soit la taille de leurs corps entiers (99 + 69 lignes), alors que
  `numstat` ne compte en ajouté que les **lignes qui changent**. Les sections de
  contrôle pur — les `if !always_on.is_empty()`, les en-têtes de section, la boucle
  `SKIPPED` dont `name` et `reason` portent déjà le même nom dans le miroir — ne
  changent pas. Mesuré ligne par ligne sur l'arbre : ~34 pour `handle_skills`
  (21 lignes d'accès plus le pré-filtre) et ~44 pour `handle_skill` (dont 14 pour le
  bloc des variantes, qui se réécrit sur `variants.providers`). Le poste est chiffré
  à **88**, marge comprise pour les replis de `cargo fmt`.
- **005 chiffrait B1 à 543**, dont 120 jetables. Les retirer donne 423 ; la lecture
  directe des deux handlers coûte 88 là où les pré-filtres plus l'appel de
  l'adaptateur coûtaient 42, soit +46. 423 + 46 = 469. **La coupe A est donc moins
  chère que la coupe B pour un périmètre strictement plus grand**, et c'est
  arithmétique, pas une opinion : B payait 120 lignes pour n'en économiser que 46.
- **Les postes que 005 a mesurés et que ce plan reprend sans les recalculer** : les
  trois fonctions de démarrage (68) et leurs tests (48), `miroir_de_test()` par
  `CORPS` (plus court que les ~40 de 004, parce que `CORPS` existe déjà et couvre
  tous les cas exigés), les tests littéraux des deux lecteurs (100, parce que le
  critère 1 demande une comparaison **littérale** section `SKIPPED` comprise : la
  chaîne attendue de `/skills` fait une douzaine de lignes à elle seule), et
  l'extension de la garde à 16 et non 6 (004 comptait les motifs sans leurs raisons
  ni l'ajustement du doc-comment).

---

Tracked in: senara-solutions/mika#1727
