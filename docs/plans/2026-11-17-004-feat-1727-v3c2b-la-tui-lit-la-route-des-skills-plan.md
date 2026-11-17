# V3c-2b — la TUI lit la route des skills

**Ticket :** senara-solutions/mika#1727
**Vol :** V3c-2b (après V3c-2a-i/-ii/-iii : mika#3101, #3102, #3103, `7f2e5531`)
**Branche :** `feat/1727/v3c2b-la-tui-lit-la-route-des-skills`
**Base mesurée :** `origin/main` @ `7f2e5531`

---

## HALTE BORNE — le périmètre ne tient pas, et ce plan ne le recoupe pas

Ce vol est d'abord une mesure, et la mesure est faite. Le périmètre complet de
V3c-2b pèse **600 lignes ajoutées** dans l'unité
`git diff --numstat origin/main...HEAD` (plan exclu) et **575 lignes** hors
`docs/` et hors `CLAUDE.md`.

| unité | estimé | borne | verdict |
|---|---|---|---|
| `git diff --numstat origin/main...HEAD`, plan exclu | **600** | 475 | **dépasse de 125** |
| estimation hors `docs/` et hors `CLAUDE.md` | **575** | 452 (corps du ticket) | **dépasse de 123** |
| estimation hors `docs/` et hors `CLAUDE.md` | **575** | 429 (mika#2693) | **dépasse de 146** |

**Le groom s'arrête ici.** Il ne pratique aucune coupe : le corps du ticket
réserve ce choix à Vincent. Le détail du chiffrage est en § *Taille estimée*, et
les coupes candidates — chiffrées, non pratiquées, chacune confrontée aux deux
contraintes du corps — sont en § *Coupes candidates*.

Le reste de ce plan chiffre et ordonne le périmètre **entier**, tel qu'un vol
unique le livrerait. Chaque coupe candidate se lit comme une partition de cette
liste de livrables, et chaque sous-vol se groomera ensuite depuis ce plan.

---

## Ce qui est sur main (mesuré sur `7f2e5531`)

### La source — l'assemblage local

`commands/chat.rs::spawn_agent_worker(ctx: DbContext, agent_name, session_id, model_override)`
assemble le registre sur le disque et le rend en 7ᵉ position de son tuple
(`chat.rs:206`, `Arc<SkillRegistry>`) :

- `mika_agent::skills::migrate_disabled_markers_async(&skills_dir, …)` (l. 237) ;
- `ctx.async_db.get_skill_overrides(…)` (l. 252) ;
- `assemble_agent_registry(&skills_dir, &identity, &overrides, …)` (l. 255).

Le registre se pose dans `app.skills` à trois endroits : au démarrage
(`App::new`), à la bascule d'agent (l. 807) et au `/restart` (l. 915).

### L'état

`tui/app.rs:578` — `pub skills: Arc<SkillRegistry>`, paramètre d'`App::new`
(l. 690/728), posé à `Arc::new(SkillRegistry::empty())` sur le chemin team mode
(l. 822).

`AgentResponse::Reply` porte `updated_skills: Option<Arc<SkillRegistry>>`
(l. 332), lu l. 1344-1349, et vaut `None` à tous ses sites d'écriture depuis
V3b-2. **Ils sont cinq, pas quatre** — `chat.rs:360` et `chat.rs:367` (bras
`Message`), `chat_turn.rs:228` et `chat_turn.rs:246` (tour de callback : succès
*et* erreur), plus `app.rs:2417` dans un test. Le corps du ticket dit « quatre » :
le compte juste est **quatre sites de production** plus un site de test, et
`chat_turn.rs` en porte deux, pas un. Le plan le note ici pour que le pilote ne
laisse pas le second derrière lui — le critère 5 exige zéro occurrence dans tout
`crates/mika-cli/`, et un `grep` final tranche.

### Les lecteurs — recensement fermé

| lecteur | site | ce qu'il lit |
|---|---|---|
| message de démarrage « skills ignorées » | `chat.rs:580` | `app.skills.skipped()` |
| message de démarrage « avertissements » | `chat.rs:605` | `app.skills.validated_warnings()` |
| autocomplétion de `/skill` | `completers.rs::complete_skill` | `ctx.skills.skills()` via `CompletionContext.skills` (`autocomplete.rs:20`), passé depuis `input.rs:792` |
| `/skills` | `handlers.rs:906-1000` (~95 l.) | `app.skills.skills()` / `.skipped()` |
| `/skill <nom>` | `handlers.rs:1060-1135` (~75 l.) | `app.skills.skills()` |

`handle_skills` ouvre par un early-return **`"No skills loaded."`** — le texte
que le critère 2 interdit sur un échec de lecture.

### Les constructeurs de `SkillRegistry` en test

`completers.rs:397`, `input.rs:1148`, `handlers.rs:1572` (`test_app`),
`handlers.rs:2115` (`with_skipped`), `app.rs:2357`. Cinq sites, tous à router par
le constructeur de test partagé.

### Le client, déjà livré et sans appelant

`commands/skills_remote.rs::lire(base_url, token, agent) -> Result<SkillsMirror, SkillsRemoteIssue>`.
Le miroir porte les quatre clés de la route et treize champs par skill
(`name`, `description`, `origin`, `enabled`, `always_on`, `tools`,
`has_override`, `version`, `timeout_secs`, `keywords`, `tool_names`, `path`,
`variants`), plus `skipped` et `warnings`. **Chaque champ lu par les cinq
lecteurs ci-dessus a son correspondant dans le miroir** — vérifié champ par
champ, y compris `variant_count()` → `variants.total` et
`variant_providers()` / `variant_models(p).len()` → `variants.providers`
(`BTreeMap<String, usize>`). Aucune dérivation n'est nécessaire au-delà d'un
`Default` sur `SkillsMirror` pour le site team mode.

Six attentes `#[cfg_attr(not(test), expect(dead_code, …))]` portent les cinq
structs du miroir et `lire`.

### Les gardes

`mika1727_v3b1_le_tour_de_la_tui_ne_construit_plus_son_provider` (`chat.rs:1804`)
et `mika1727_v2d4a_la_tui_ne_cree_plus_sa_session_en_local` : forme
`ProductionScanner`, `ALLOWED` vide et assertie vide, `MOTIFS` motif→remède,
`SITES` fichier→témoin, contre-témoin sur `chat.rs`.

---

## Périmètre

Conforme à la tête du ticket, sans élargissement. Ce plan **n'ajoute aucune
halte** à celles de la tête.

### La source

`spawn_agent_worker` lit la route au lieu d'assembler. La lecture est extraite
en `chat.rs::lire_les_skills(base_url: &str, lire_env: SourceEnv<'_>, agent: &str) -> EtatSkills`,
sur le patron d'`ouvrir_session_par_spirit` : **tout ce qui touche le monde
arrive par argument**, donc la fonction se teste contre un banc sans le test É1
(qui est hors vol, V3c-2c). Elle lit le jeton par `session_remote::jeton_interne`
et l'URL par `dashboard::spirit_url()` chez l'appelant.

`migrate_disabled_markers_async`, `get_skill_overrides` et
`assemble_agent_registry` quittent la TUI. La lecture a lieu au démarrage, à la
bascule d'agent et au `/restart` — les trois sites qui respawnaient un worker.

### L'état

`tui/app.rs` déclare `pub(crate) type EtatSkills = Result<SkillsMirror, String>;`
— le miroir, ou la cause en une ligne. `App.skills` devient
`Arc<EtatSkills>` ; le site team mode pose `Arc::new(Ok(SkillsMirror::default()))`.

**Une lecture en échec n'est pas fatale.** La TUI démarre, et chaque lecteur dit
la cause au lieu de mentir.

### Les lecteurs

Les cinq lecteurs lisent le miroir. Leur rendu reste celui d'aujourd'hui, **à
donnée égale** — c'est ce que les tests mesurent. Sur `Err(cause)` :

| lecteur | sur échec |
|---|---|
| `/skills` | dit la cause ; **jamais** `"No skills loaded."` |
| `/skill <nom>` | dit la cause |
| autocomplétion de `/skill` | rend une liste vide |
| démarrage | un message `System` dit la cause |

Les deux messages de démarrage sont **extraits en fonctions pures**
(`message_des_skills_ignorees(&EtatSkills) -> Option<String>` et
`message_des_avertissements(&EtatSkills) -> Option<String>`), sans quoi le
critère 3 n'est pas testable sans É1. Leur texte est repris littéralement :
`"⚠ {count} skill(s) skipped at startup:\n"` + `"  • {name}: {reason}"`,
`max_display = 5`, `"  ... and {n} more. Run \`mika skills validate\` for details."` ;
`"⚠ {count} skill(s) loaded with validation warnings:\n"` avec le **premier**
message de chaque skill et le fallback `"unknown issue"`.

### `updated_skills`

Quitte `AgentResponse::Reply`, son lecteur (`app.rs:1344-1349`) et ses cinq sites
d'écriture (`chat.rs` ×2, `chat_turn.rs` ×2, `app.rs:2417`) — voir le
recensement ci-dessus pour le second site de `chat_turn.rs`, que le corps du
ticket ne compte pas.

**Le volume ne bouge pas pour autant** : les cinq sites sont des lignes
`updated_skills: None,` **retirées**, et le tableau de § *Taille estimée* compte
les lignes **ajoutées**. La seule ligne ajoutée de `chat_turn.rs` est la
réécriture du passage de doc (l. 160) qui décrit aujourd'hui le champ.

### Les attentes

Les six attentes `dead_code` de `skills_remote.rs` tombent avec leur premier
appelant. **Aucune attente ni aucun `allow` neuf** ;
`cargo clippy --all-targets -p mika-cli` en tient le compte exact.

### Le constructeur de test partagé

**Décision du plan : `skills_remote::tests::miroir_de_test() -> SkillsMirror`,
`pub(crate)`.** Précédent dans le crate : `rewind_remote::tests::{RequeteRecue, fake_spirit}`,
`pub(crate)` depuis V2c-1b, consommés par le module `tests` de `skills_remote.rs`
lui-même.

**Le module porte la visibilité, pas seulement la fonction.** Le `mod tests` de
`skills_remote.rs` (l. 280) est privé aujourd'hui ; `rewind_remote.rs` l. 525
l'écrit `pub(crate) mod tests`. Une `pub(crate) fn` dans un module privé reste
inatteignable depuis `tui/`, et le pilote le découvrirait à la compilation du
commit 2. Le commit 1 pose donc `#[cfg(test)] pub(crate) mod tests`, sur la forme
exacte de `rewind_remote`. Il rend deux skills (une `always_on`, une à la demande), une ignorée et
un avertissement — la forme exacte dont les lecteurs ont besoin pour mesurer « à
donnée égale ». Les cinq sites qui construisaient un `SkillRegistry` passent par
lui. **Aucun test ne recopie la forme du miroir.**

### Les gardes

1. `mika1727_v3b1_…` **s'étend** de quatre motifs, chacun avec sa raison :
   `assemble_agent_registry`, `migrate_disabled_markers_async`,
   `get_skill_overrides`, `updated_skills`.
2. **Garde neuve** : aucune production de `tui/` ne nomme `SkillRegistry`. Elle
   balaie tout `tui/` par `ProductionScanner::for_each_under`, porte son **témoin**
   (le balayage a bien lu la production — un chemin faux serait vert par cécité)
   et son **contre-témoin** (le masquage `#[cfg(test)]` a bien opéré — sans lui,
   les cinq constructeurs de test feraient rougir la garde et on l'aurait
   « réparée » par une entrée d'allowlist). `ALLOWED` livrée vide et **assertie
   vide** (mika#2398, mika#2309, mika#2201).

### La doc

`crates/mika-cli/CLAUDE.md` : la section « La route des skills a son client, pas
encore son appelant » devient « La TUI lit la route des skills ». Retrait de
« l'assemblage de démarrage reste, une fois » (l. 735, section V3b-2) et de
« **Rien ne l'appelle** » (l. 764).

### Hors de ce vol

Le test É1 (bout-en-bout du démarrage contre un spirit) — c'est V3c-2c.

---

## Fichiers touchés

**Seuls changent** (tête du ticket, à la lettre) :

- `crates/mika-cli/src/commands/chat.rs`
- `crates/mika-cli/src/commands/chat_turn.rs` — pour `updated_skills` seulement
- `crates/mika-cli/src/commands/skills_remote.rs` — retrait des six attentes,
  `#[derive(Default)]` sur `SkillsMirror`, constructeur de test partagé
- `crates/mika-cli/src/tui/app.rs`
- `crates/mika-cli/src/tui/input.rs`
- `crates/mika-cli/src/tui/commands/autocomplete.rs`
- `crates/mika-cli/src/tui/commands/completers.rs`
- `crates/mika-cli/src/tui/commands/handlers.rs`
- `crates/mika-cli/CLAUDE.md`

**Restent intacts** : tout `crates/mika-agent/`, `session_remote.rs`,
`rewind_remote.rs`, `docs/egress/` (l'entrée `cli-spirit-skills` reste juste : le
client se construit toujours dans `skills_remote.rs`), `crates/mika-common/`,
`crates/mika-a2a/`, `crates/mika-gateway/`, `dashboard/`.

**Aucun fichier du plan de contrôle** : ni `.github/workflows/`, ni `.claude/`,
ni `skills/bundled/`, ni `crates/mika-agent/src/well_known_agents.rs`, ni
`.mika/`, ni aucun chemin de CODEOWNERS.

**Aucun critère de V3c-2b n'exige de toucher un fichier exclu.** Le point
vérifié : la route `GET /api/v1/skills?agent=<nom>` sert déjà les quatre clés
depuis V3c-1 et V3c-2a-i, donc le serveur n'a rien à gagner ; la migration des
marqueurs `.disabled` est déjà faite côté serveur à l'`init_agent`
(`mika-agent/src/server/mod.rs:670`), donc la retirer de la TUI ne la perd pas.

---

## Ordre des commits (vol unique, périmètre entier)

1. **`feat(1727): le miroir des skills a son constructeur de test`** —
   `skills_remote.rs` : `#[derive(Default)]`, `miroir_de_test()` `pub(crate)`.
   Les six attentes restent encore : rien ne les contredit à ce commit.
2. **`feat(1727): l'état de la TUI porte le miroir des skills`** —
   `app.rs` (`EtatSkills`, champ, `App::new`, team mode, retrait de
   `updated_skills` et de son lecteur), `chat_turn.rs`, les deux sites de
   `chat.rs`, `autocomplete.rs`, `input.rs`. Les cinq constructeurs de test
   passent par `miroir_de_test()`.
3. **`feat(1727): la TUI lit la route des skills au lieu d'assembler`** —
   `chat.rs` : `lire_les_skills`, les trois sites d'appel, retrait du bloc
   d'assemblage et des deux imports. Retrait des six attentes de
   `skills_remote.rs`. Tests de `lire_les_skills` contre `fake_spirit`.
4. **`feat(1727): les lecteurs des skills lisent le miroir`** —
   `handlers.rs` (`handle_skills`, `handle_skill`), `completers.rs`
   (`complete_skill`), `chat.rs` (les deux messages de démarrage extraits + le
   message `System` de cause), **avec tous leurs tests** : succès et échec pour
   chacun.
5. **`feat(1727): la garde refuse l'assemblage local des skills`** —
   extension de `mika1727_v3b1_…` (4 motifs) et garde neuve sur `tui/`, **avec
   témoin, contre-témoin et tests dans le même commit**.
6. **`docs(mika-cli): la TUI lit la route des skills`** — `CLAUDE.md`.

**Chaque garde arrive avec ses tests, dans le même commit.** Aucun test ne se
reporte à un vol suivant.

**Halte du pilote sur la borne.** Après chaque commit, le pilote mesure
`git diff --numstat origin/main...HEAD`, plan exclu. **Si le total dépasse 475,
il s'arrête** : pas de commit suivant, pas de PR. Son dernier message dit
« HALTE borne : N lignes pour 475 », avec le compte par commit. *Au chiffrage de
ce plan, cette halte se déclenche au commit 4.*

---

## Definition of Done

- `cargo build -p mika-cli --bin mika` passe, **d'abord**.
- `cargo test -p mika-cli` et `cargo test -p mika-common --test claude_md_size`
  sont verts.
- `cargo clippy --all-targets -p mika-cli` est vert, sans attente ni `allow` neuf.
- `bash scripts/verify-pipeline.sh origin/main` est vert avant l'ouverture de la
  PR. S'il est rouge, le pilote s'arrête et le dit.
- La garde neuve rougit sur `main` et passe sur la branche — vérifié en la jouant
  contre l'arbre de `main`.
- Les neuf fichiers listés plus haut sont les seuls modifiés
  (`git diff --name-only origin/main...HEAD`, plan exclu).
- Le corps de PR porte `Refs mika#1727` et aucun `Closes`, `Fixes` ni `Resolves`
  — y compris si le gabarit de reprise (`wip-rescue`) l'ouvre, auquel cas le
  `Closes #N` par défaut est corrigé avant toute revue et
  `gh pr view <N> --json closingIssuesReferences` rend `[]`.
- Le corps de PR porte `Tracked in: senara-solutions/mika#1727`.
- Le corps de PR porte `Code review: delegated to mika-qa (mika#2694)`.
- Aucun ticket n'est ouvert (gel jusqu'au 01/12) : un manque ou un reste va en
  commentaire sur mika#1727 ou dans la PR.

---

## Acceptance criteria

Les dix critères de V3c-2b, transcrits verbatim du corps du ticket :

1. Au démarrage, contre un spirit qui sert la route, `app.skills` porte le miroir de l'agent, sans aucun assemblage local. `/skills` et `/skill <nom>` rendent, à donnée égale, ce qu'ils rendent aujourd'hui.
2. Contre un spirit qui refuse la route, la TUI démarre. `/skills` et `/skill` disent la cause, jamais « No skills loaded. ». L'autocomplétion de `/skill` est vide, et un message `System` de démarrage dit la cause.
3. Les messages de démarrage des skills ignorées et des avertissements se rendent depuis le miroir, avec le même texte qu'aujourd'hui à donnée égale.
4. Une bascule d'agent et un `/restart` relisent la route pour l'agent visé.
5. `updated_skills` n'existe plus dans `crates/mika-cli/`.
6. La garde structurelle rougit sur main et passe sur la branche. Elle porte son témoin et son contre-témoin.
7. Les seules attentes retirées sont les six de `skills_remote.rs`. Aucune attente ni aucun `allow` neuf.
8. Seuls changent les fichiers listés plus haut.
9. `cargo build -p mika-cli --bin mika` d'abord, puis `cargo test -p mika-cli` et `cargo test -p mika-common --test claude_md_size` sont verts. `cargo clippy --all-targets -p mika-cli` est vert. Le pilote lance `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il le trouve rouge, il s'arrête et le dit.
10. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y compris si le gabarit de reprise l'ouvre.

### Comment chaque critère se mesure

| critère | mesure |
|---|---|
| 1 | `lire_les_skills` contre `fake_spirit` rend le miroir ; `handle_skills`/`handle_skill` sur `miroir_de_test()` rendent le texte attendu, comparé littéralement |
| 2 | les mêmes lecteurs sur `Err("…")` ; `assert!(!sortie.contains("No skills loaded."))` — un `assert` **négatif**, pas une relecture |
| 3 | `message_des_skills_ignorees` et `message_des_avertissements` sur `miroir_de_test()`, textes comparés littéralement |
| 4 | `lire_les_skills` appelée avec deux noms d'agent contre `fake_spirit` : la requête reçue porte l'agent visé (`RequeteRecue`) |
| 5 | `grep -rn 'updated_skills' crates/mika-cli/src` rend zéro ligne ; le compilateur le confirme |
| 6 | la garde jouée contre l'arbre de `main` rougit ; témoin et contre-témoin sont deux tests distincts |
| 7 | `cargo clippy --all-targets -p mika-cli` vert, et `grep -c 'expect(dead_code' crates/mika-cli/src/commands/skills_remote.rs` rend 0 |
| 8 | `git diff --name-only origin/main...HEAD` |
| 9 | les quatre commandes, dans cet ordre |
| 10 | `gh pr view <N> --json closingIssuesReferences` rend `[]` |

---

## Fire-Disposition

**Le détecteur livré :** la garde structurelle neuve — aucune production de
`tui/` ne nomme `SkillRegistry` — plus l'extension de quatre motifs de
`mika1727_v3b1_…`.

**Disposition retenue : (c) halte-et-remontée.** La garde est livrée **armée**,
`ALLOWED` vide et **assertie vide**. Quand elle rougit, `cargo test -p mika-cli`
est rouge, le pilote s'arrête et le dit ; le remède est de router le nouvel
appelant par `lire_les_skills`, **jamais** d'ajouter une ligne d'allowlist.

**Pourquoi pas (a) exception nommée en allowlist :** une allowlist qui ne naît
pas vide ne mesure plus rien — c'est précisément le faux vert que mika#2309
condamne, et que les gardes V1, V2a, V2c-1b, V2d-4a/b et V3b-1 de ce crate ont
toutes refusé.

**Pourquoi pas (b) livrer désarmé :** le corps du ticket l'interdit nommément
(« Chaque garde arrive avec ses tests, dans le même commit. Les tests ne se
reportent jamais à un vol suivant, même si le code reste »), et une garde que
rien ne joue refuse en silence (classe mika#2578 / mika#2752).

**Zéro population attendue à la livraison.** Le périmètre retire les cinq
derniers sites `SkillRegistry` de `tui/` ; le balayage doit donc être vide à la
fin du commit 5. S'il ne l'est pas, c'est que le périmètre est incomplet — le
pilote s'arrête, il n'ajoute pas d'entrée.

**La garde ne refuse pas `skills::`** : rien d'autre de `mika_agent::skills` n'est
lu par `tui/` après ce vol, mais refuser le chemin de module au lieu du type
rougirait sur une importation légitime future sans rien gagner. Le motif est le
**type**, comme `AppContext` l'était à V3b-2.

---

## Taille estimée

| livrable | fichier | ajoutées (numstat) | dont hors docs |
|---|---|---|---|
| `Default` sur le miroir, retrait des six attentes, `miroir_de_test()` `pub(crate)` | `commands/skills_remote.rs` | 46 | 46 |
| `EtatSkills`, champ `skills`, `App::new`, team mode, retrait de `updated_skills` et de son lecteur, helper de test | `tui/app.rs` | 12 | 12 |
| Type du contexte d'autocomplétion | `tui/commands/autocomplete.rs` | 2 | 2 |
| `complete_skill` sur le miroir + test d'échec | `tui/commands/completers.rs` | 32 | 32 |
| `handle_skills` et `handle_skill` sur le miroir | `tui/commands/handlers.rs` | 155 | 155 |
| Tests des deux lecteurs (succès + échec) et adaptation des deux tests existants | `tui/commands/handlers.rs` | 77 | 77 |
| `lire_les_skills` + les trois sites d'appel, retrait du bloc d'assemblage et des imports | `commands/chat.rs` | 28 | 28 |
| Les deux messages de démarrage extraits et rendus depuis le miroir + le message `System` de cause | `commands/chat.rs` | 67 | 67 |
| Tests des deux messages de démarrage et du message de cause | `commands/chat.rs` | 51 | 51 |
| Test de la relecture par agent (bascule, `/restart`) | `commands/chat.rs` | 25 | 25 |
| Extension de `mika1727_v3b1_…` : 4 motifs et leurs raisons | `commands/chat.rs` | 6 | 6 |
| Garde neuve `tui/` ne nomme plus `SkillRegistry`, témoin et contre-témoin | `commands/chat.rs` | 70 | 70 |
| Passage du miroir à l'autocomplétion + test | `tui/input.rs` | 3 | 3 |
| Retrait de `updated_skills` | `commands/chat_turn.rs` | 1 | 1 |
| La doc dit que la TUI lit la route | `crates/mika-cli/CLAUDE.md` | 25 | 0 |

Total estimé : 600 lignes
Estimation hors `docs/` et hors `CLAUDE.md` : 575 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Les deux unités sont données parce que le corps du ticket en exige deux.**
`Total estimé :` porte l'unité `git diff --numstat origin/main...HEAD`, plan
exclu, bornée à 475 par la tête du ticket ; la ligne suivante porte l'unité de
l'architecte, bornée à 452 par la tête et à 429 par mika#2693. **La plus serrée
mord, et les trois sont dépassées.**

**Les commentaires sont comptés et ne se raboteront pas pour tenir.** Les 600
lignes incluent le pourquoi de `lire_les_skills` (pourquoi par argument), celui
de la garde neuve (pourquoi le type et pas le module), celui du contre-témoin, et
celui du constructeur partagé. Aucune de ces lignes n'est une paraphrase du code.

**Les tests sont comptés.** Ils pèsent **236 lignes** des 575 hors docs (77 dans
`handlers.rs`, 76 dans `chat.rs` hors garde, 70 de garde neuve avec ses deux
témoins, 13 ailleurs). Les retirer tiendrait la borne et violerait la règle de la
tête ; c'est la raison principale du dépassement, et elle est assumée.

---

## Coupes candidates

**Aucune n'est pratiquée.** Chacune est chiffrée dans les deux unités et
confrontée aux deux contraintes du corps du ticket :

> **C1** — aucun vol ne laisse deux sources de vérité en production : entre deux
> vols mergés, les lecteurs lisent ou bien le registre assemblé, ou bien la
> route, jamais les deux selon le lecteur.
>
> **C2** — aucun vol ne laisse un lecteur mentir : par exemple, un échec de
> lecture qui afficherait « No skills loaded. ».

**Ce que C1 interdit d'emblée**, et qui est donc absent de cette liste : couper
par lecteur (`/skills` d'abord, `/skill` ensuite), couper par moment (le
démarrage d'abord, la bascule et `/restart` ensuite), couper l'autocomplétion
des deux autres lecteurs. Chacune de ces coupes laisserait un lecteur sur le
registre assemblé pendant qu'un autre lit la route.

---

### Coupe A — la garde et la doc en second sous-vol

| sous-vol | livre | numstat | hors docs |
|---|---|---|---|
| **A1** | la source, l'état, les cinq lecteurs, `updated_skills`, les six attentes, tous les tests de comportement (critères 1-5, 7-10) | **499** | **499** |
| **A2** | la garde neuve + les 4 motifs de `v3b1` + la doc (critère 6) | **101** | **76** |

- **C1** : tenue. A1 commute **tous** les lecteurs au même vol.
- **C2** : tenue. A1 porte le traitement de l'échec pour les quatre surfaces.
- **Borne : non tenue.** A1 reste à 499 pour 475 et pour 452. **Rejetée seule.**
- **Tests** : A1 — `lire_les_skills` contre `fake_spirit`, les deux messages de
  démarrage, les deux lecteurs en succès et en échec, l'autocomplétion vide,
  l'agent visé à la relecture. A2 — la garde neuve avec témoin et contre-témoin,
  jouée contre l'arbre de `main`.
- **Réserve** : A2 reporte d'un vol la garde qui empêche l'assemblage de
  revenir, alors que le code qu'elle garde est livré en A1. C'est un report de
  **garde**, pas de test : chacun des deux sous-vols arrive avec les siens. C'est
  néanmoins la coupe la plus proche de ce que la tête interdit, et c'est à
  Vincent de dire si elle est acceptable.

Utile en **combinaison** avec B ou C, pas seule.

---

### Coupe B — l'adaptateur, les deux lecteurs lourds en second (recommandée)

L'idée : commuter la **source** dès le premier sous-vol — ce que C1 exige — et
servir les deux gros lecteurs par un adaptateur `miroir_en_registre(&SkillsMirror) -> SkillRegistry`
le temps d'un vol, avec un pré-filtre d'une ligne dans chacun pour dire la cause
sur `Err` — ce que C2 exige.

| sous-vol | livre | numstat | hors docs |
|---|---|---|---|
| **B1** | la source, l'état, les deux messages de démarrage, l'autocomplétion, `updated_skills`, les six attentes, l'extension `v3b1`, **l'adaptateur** et les **pré-filtres** de `/skills` et `/skill` | **393** | **393** |
| **B2** | `handle_skills` et `handle_skill` lisent le miroir, **l'adaptateur et ses tests partent**, la garde neuve, la doc | **327** | **302** |

- **C1** : tenue. Dès B1 l'assemblage local est retiré et **toute** lecture
  dérive du miroir ; l'adaptateur n'est pas une seconde source, c'est une
  projection de la seule.
- **C2** : tenue. Les pré-filtres font dire la cause à `/skills` et `/skill`
  avant que l'adaptateur soit atteint, donc `"No skills loaded."` ne reste
  atteignable que quand le miroir a bien été lu et est vide — ce qui est vrai,
  pas un mensonge.
- **Borne** : les deux sous-vols tiennent, avec 82 et 148 lignes de marge sur 475.
- **Coût** : **~90 lignes jetables** (l'adaptateur, ~60, et ses tests, ~30),
  livrées en B1 et retirées en B2. Le total des deux sous-vols monte à 720 pour
  600 d'un vol unique.
- **Tests de B1** : `lire_les_skills` contre `fake_spirit` ; les deux messages de
  démarrage depuis le miroir ; l'autocomplétion vide sur échec ; l'adaptateur
  rend, à donnée égale, ce que `/skills` et `/skill` rendaient sur le registre
  (comparaison littérale sur `miroir_de_test()`) ; les deux pré-filtres disent la
  cause et ne disent pas `"No skills loaded."`.
- **Tests de B2** : les quatre tests de rendu direct (succès et échec pour chaque
  lecteur), les deux tests existants adaptés, la garde neuve avec témoin et
  contre-témoin, et l'absence de l'adaptateur (le compilateur suffit).

---

### Coupe C — relever la borne, sans couper

Vincent relève la borne de ce vol de 475 à **620** (numstat) et de 452 à **595**
(estimées), et le vol se livre tel que ce plan l'ordonne, en six commits.

- **C1** et **C2** : tenues par construction — un vol unique ne laisse aucun état
  transitoire.
- **Coût en lignes jetables : zéro.** Pas d'adaptateur, pas de pré-filtre, pas de
  garde reportée.
- **Ce que ça déplace** : le risque passe du nombre de lignes au **plafond de
  tours**. Le vol porte `loop-substrate` (plafond 200). Quinze livrables, dont
  deux lecteurs de ~80 lignes, une garde de 70 et quatre passes de vérification
  (`build`, `test`, `clippy`, `verify-pipeline`) : l'estimation est de **110 à
  150 tours**, sous le plafond, mais sans la marge qu'un vol coupé donne. Un
  `cargo test -p mika-cli` rouge sur un détail de texte à 140 tours ne laisse plus
  de quoi converger.
- **C'est l'option à retenir si le coût des 90 lignes jetables de B paraît plus
  cher que le risque de plafond.**

---

### Tableau de décision

| coupe | sous-vols | pire sous-vol (numstat) | C1 | C2 | borne | lignes jetables |
|---|---|---|---|---|---|---|
| aucune (ce plan) | 1 | 600 | ✅ | ✅ | **✗** | 0 |
| **A** | 2 | 499 | ✅ | ✅ | **✗** | 0 |
| **B** | 2 | 393 | ✅ | ✅ | ✅ | ~90 |
| **C** (borne relevée) | 1 | 600 | ✅ | ✅ | ✅ (relevée) | 0 |

**Le groom ne tranche pas.** Il nomme B comme la seule coupe qui tienne les deux
contraintes *et* la borne telle qu'elle est écrite, et C comme la seule qui ne
coûte aucune ligne jetable. Le mot est à Vincent.

---

Tracked in: senara-solutions/mika#1727
