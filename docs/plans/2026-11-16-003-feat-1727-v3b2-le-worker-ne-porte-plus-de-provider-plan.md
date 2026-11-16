# mika#1727 — vol V3b-2 : le worker de la TUI ne porte plus de provider

- **Ticket** : senara-solutions/mika#1727 — *TUI as thin HTTP client of mika-spirit*
- **Vol** : V3b-2 (la seconde des deux moitiés de V3b, coupées par Vincent le 2026-11-16)
- **Branche** : `feat/1727/v3b2-le-worker-ne-porte-plus-de-provider`
- **Base mesurée** : `main` @ `b458b978` (le merge de V3b-1, mika#3098) — la même tête que celle sur laquelle MPC a mesuré
- **Label** : `loop-substrate` (plafond de tours : 200)
- **Borne du vol** : au plus 400 lignes ajoutées, plan exclu, par `git diff --numstat origin/main...HEAD`
- **La PR porte `Refs mika#1727`**, jamais `Closes`, `Fixes` ni `Resolves` — ni dans le corps, ni dans un commit. Le ticket se ferme au merge de V4.

---

## 0. Pourquoi ce vol tient en un seul vol

Depuis V3b-1, aucun tour de la TUI ne tourne plus dans son propre processus : le
modèle choisi voyage sous `mika.model_override` et spirit le résout. Ce qui reste
dans `mika-cli` est un **échafaudage sans lecteur** : un provider LLM bâti au
démarrage (`init::init_for_agent`), revalidé pour `--model` (`override_model`),
des serveurs MCP connectés puis fermés sans qu'un outil y passe, deux
rechargements de skills derrière un drapeau que **rien ne pose jamais à `true`**,
et un `DbContext.github_app` dont le dernier lecteur est parti au vol V3a-2b.

Le volume du vol est donc d'abord **négatif** : `init.rs` perd une centaine de
lignes, `chat.rs` une cinquantaine, et `git diff --numstat` ne compte pas les
suppressions. Ce qui coûte des lignes ajoutées est ailleurs, en trois postes :
l'extension de la garde de V3b-1 à deux fichiers et huit motifs, le
retournement de `/provider` avec les tests qui tenaient l'ancien comportement, et
la doc qui dit aujourd'hui l'inverse (« `mika chat --model` … runs in process and
really does execute against the provider it builds »).

L'estimation est de **377 lignes** (§ *Taille estimée*), sous la borne de 400 et
sous la borne de 429 de l'unité de l'architecte, **avec** les tests. Il n'y a donc
pas de coupe à demander, et ce plan ne s'arrête pas. **La marge est mince — 23
lignes** : le § *Halte du pilote sur la borne* est ce qui arbitre, pas cette
estimation.

---

## 1. Ce qui est mesuré sur main (@ `b458b978`)

### `crates/mika-cli/src/init.rs` (330 lignes)

| site | lignes | mesure |
|---|---|---|
| `AppContext { db_ctx, llm }` + sa doc | 12 à 23 | le `llm` ne porte **pas** de `#[expect(dead_code)]` : `override_model` l'**écrit**, et rustc compte une écriture de champ comme un usage |
| `impl Deref for AppContext` → `DbContext` | 26 à 31 | c'est lui qui fait que `worker._ctx.async_db` compile aujourd'hui (voir É5) |
| `DbContext { settings, async_db, home_dir, global_home, github_app }` | 35 à 48 | `github_app` est sous `#[expect(dead_code, reason = "mika#1727 V3 : dernier lecteur parti au vol V3a-2b")]` |
| `impl AppContext { fn override_model }` | 86 à 121 | résout par `mika_common::llm::model_override`, écrit `set_provider_model`, puis rebâtit `self.llm`. Sa doc de 25 lignes finit par « Le provider bâti est posé dans un champ que V3 retire » |
| `init_for_agent` | 124 à 128 | `init_db_only_for_agent` + `make_llm_provider` |
| `init_db_only_for_agent`, dont `GitHubApp::from_settings` | 131 à 141 | la l. 133 part avec le champ |
| `connect_mcp` + sa doc mika#1737 AC3/AC5 | 192 à 222 | `pub async fn`, rend `Option<McpManager>` |
| `mod tests` : `mika2304_the_cli_keeps_no_second_resolver`, `mika2304_the_resolver_scan_reads_the_crate`, `visit_rust_files` | 224 à 330 | **intouchés** : leur témoin est le nom du test lui-même (l. 306), pas une chaîne d'`override_model` |

Imports : `GitHubApp` (l. 6), `LlmProvider` (l. 8) et `Arc` (l. 10) n'ont pas
d'autre lecteur dans le fichier. `Path` (l. 9) en garde deux
(`ensure_initialized_for_agent`, l. 150-151) et reste.

### `crates/mika-cli/src/commands/chat.rs` (1878 lignes)

| site | lignes | mesure |
|---|---|---|
| `use crate::init::{self, AppContext};` | 20 | devient `DbContext` |
| doc d'`AgentWorker` (« the AppContext (for DB shutdown) ») et `_ctx: AppContext` | 38, 48 | |
| deux docs de production qui nomment `&AppContext` | 62 à 63 | et une troisième dans la moitié test, l. 1337 (voir É7) |
| `async fn spawn_agent_worker(` + `ctx: AppContext` | 194 à 208 | le **préfixe littéral** est le témoin des gardes V2d-4a (l. 1775) et V3b-1 (l. 1852) : il reste |
| l'unique assemblage de démarrage | 248 à 258 | `get_skill_overrides` puis `assemble_agent_registry` — **il survit** (critère 4 : « une seule fois ») |
| `let skills_dirty = Arc::new(AtomicBool::new(false));` | 259 | rien ne le pose jamais à `true` |
| `connect_mcp(&ctx.home_dir)` | 262 | |
| `worker_skills`, `worker_dirty`, `worker_mcp` | 287, 290, 291 | `worker_skills` n'a que les l. 314, 366 et 376 pour lecteurs |
| premier rechargement à chaud | 306 à 321 | |
| second rechargement à chaud | 359 à 373 | |
| `updated_skills = if skills_reloaded {…} else { None }` | 375 à 379 | consommé l. 390 et 397 |
| fermeture MCP en sortie | 477 à 480 | deux commentaires de production la nomment encore, l. 787 et 804 |
| `let model = ctx.settings.active_model_display();` | 483 | ne connaît pas l'override (critère 1) |
| `AgentWorker { …, _ctx: ctx }` | 512 à 516 | |
| `run` : `init_for_agent`, le commentaire de V3b-1, `ctx.override_model` | 530 à 558 | la l. 542 **annonce ce vol** : « Son retrait est porté par V3b-2, avec celui de `AppContext` » |
| `App::new(…)` lit `worker._ctx.{async_db, home_dir, global_home, settings}` | 561 à 575 | à travers le `Deref` (É5) |
| respawn de bascule d'agent + son commentaire | 809 à 816 | `init_for_agent` puis `spawn_agent_worker(new_ctx, …, None, None)` |
| respawn de `/restart` + son commentaire | 924 à 929 | idem |
| le chemin d'équipe bâtit **son propre** `GitHubApp::from_settings` | 1065, 1082 | comme `ask.rs:841` → É4 |
| garde V2d-4a (sessions) | 1737 à 1812 | **intouchée**. Son nom n'a pas changé au second vol : c'est une garde **par fichier**, pas par vol |
| garde V3b-1 (provider) | 1814 à 1877 | deux motifs, allowlist vide assertie, témoin l. 1852, contre-témoin l. 1856 |

`use std::sync::atomic::{AtomicBool, Ordering};` (l. 11) et
`use mika_agent::skills::assembly::assemble_agent_registry;` (l. 32) **restent** :
voir É6.

### `crates/mika-cli/src/commands/ask.rs` (1469 lignes, `#[cfg(test)]` à partir de la l. 935)

| site | lignes | mesure |
|---|---|---|
| `let ctx = init::init_for_agent(agent_name)?;` | 167 | `ctx` ne sert plus qu'à sa base : son provider n'a aucun appelant |
| le commentaire mika#2304 | 169 à 184 | il dit déjà pourquoi la clé d'API ne se vérifie pas ici (mika#2293) ; son premier paragraphe nomme `ctx.override_model` |
| le refus du `--model` vide | 185 à 189 | message : `--model value must not be empty` |
| `anyhow::bail!("--task-id value must not be empty")` | 194 | **unique dans le crate**, en production, hors périmètre du vol → témoin de la garde pour ce site |

### `crates/mika-cli/src/tui/commands/handlers.rs` (3322 lignes)

| site | lignes | mesure |
|---|---|---|
| `handle_model` | 626 à 691 | `switch_model` persiste ; `app.provider = target_provider` **si** `result.provider_changed` (l. 660-662) ; `format_worker_model` l. 666 ; `SetModel` l. 670 ; réponse « Switched to … (switched provider to …) » |
| `handle_provider`, bras `set <field> <value>` | 717 à 759 | `field == "model"` envoie `SetModel` (l. 737) — **ce bras ne change pas** (critère 3) |
| `handle_provider`, bras de bascule | 760 à 808 | `switch_provider` écrit `llm_provider`, puis `app.model` (l. 779), `app.provider` (l. 780), `SetModel` (l. 783), `fetch_models` en tâche de fond (l. 795) |
| `test_app()` et `drain_requests()` | 1545 à 1595 | base en mémoire, `agent_rx` capturé — **réutilisés tels quels**, aucune fixture recopiée |

Les tests qui tiennent l'ancien comportement, et qui s'adaptent :

| test | ligne | ce qu'il asserte aujourd'hui |
|---|---|---|
| `test_model_direct_name_sets_model` | 1888 | se **prépare** par `handle_provider(app, "deepseek")`, puis vide la file « clear SetModel from provider switch » |
| `test_provider_switch_reads_configured_model` | 2176 | `app.provider == DeepSeek` |
| `test_model_cross_provider_alias_switches_provider` | 2252 | `app.provider == DeepSeek` + un `SetModel` part |
| `test_model_direct_with_provider_prefix` | 2283 | `app.provider == DeepSeek` |
| `test_provider_switch_worker_gets_provider_prefix` | 2330 | un `SetModel` préfixé `deepseek/` part sur `/provider` |
| `test_provider_switch_to_anthropic_worker_gets_prefix` | 2350 | idem pour `anthropic/`, et `app.model == "claude-sonnet-4-6"` |

Deux tests **restent verts sans y toucher**, et ce sont eux qui disent que le
critère 3 ne casse qu'un seul bras :
`test_model_alias_on_openrouter_uses_provider_prefix` (l. 2462) et
`test_model_on_openrouter_keeps_slash_model_name` (l. 2515) assertent déjà
`app.provider` **inchangé**.

### Autour, hors périmètre, mesuré pour décider

| mesure | commande | résultat |
|---|---|---|
| qui appelle `init_for_agent`, `AppContext`, `connect_mcp`, `McpManager` dans le crate | `grep -rn` sur `crates/mika-cli/src` | `init.rs`, `commands/chat.rs`, `commands/ask.rs:167`, et deux phrases de doc de `chat_turn.rs` — **rien d'autre** : aucun fichier hors de la liste du ticket n'est forcé |
| la migration mika#1737 AC5 que `connect_mcp` déclenche | `grep -rn "migrate_from_agent_home_if_needed"` | quatre autres déclencheurs vivants : `commands/mcp.rs:18`, `mika-agent/src/validate.rs:116`, `mika-agent/src/server/mod.rs:763`, plus ses tests → É3 |
| taille de `crates/mika-cli/CLAUDE.md` | `wc -c` | 82 869 o pour un plafond de 100 000 o (`claude_md_size.rs:40`) → É2 |
| un test de `/provider set model` existe-t-il ? | `grep -n "set model"` sur `handlers.rs` | une seule occurrence, un commentaire l. 713 → É9 |

### Écarts entre la tête du ticket et la mesure

**É1 — deux documents hors périmètre deviennent faux, et ce vol ne peut pas les
toucher.** `docs/mcp.md` dit l. 79 que `mika chat` connecte MCP « via
`init::connect_mcp` », l. 87 que le tour emploie le `McpManager` de spirit, et
l. 386 que `McpManager::shutdown()` « n'est appelé que par `mika chat` ».
`docs/solutions/architecture-patterns/cli-model-override-one-shot.md` décrit
`AppContext::override_model` comme la forme retenue. Le critère 6 et la liste des
fichiers interdisent de les éditer ici. Ils ne sont donc **pas** corrigés par ce
vol : le reste est noté dans la PR sous `Tracked in: senara-solutions/mika#1727`.
Le gel des tickets (jusqu'au 01/12) interdit d'en ouvrir un pour ça. Un document
de `docs/solutions/` est par nature daté et vaut pour son époque ; `docs/mcp.md`,
lui, est une référence courante et c'est l'écart qui compte.

**É2 — `claude_md_size` n'est pas la contrainte qui mord.** La tête dit « le
fichier se réécrit sans s'allonger, **puisque** `claude_md_size` le plafonne ».
La mesure : 82 869 o pour un plafond de 100 000 o, soit 17 131 o de marge. La
discipline reste bonne — un `CLAUDE.md` est chargé en entier dans chaque prompt
de pilote — mais sa raison n'est pas celle-là, et ce plan ne promet pas un
net-zéro qu'il ne pourrait tenir qu'en supprimant du contenu étranger au vol.
**Ce qu'il promet** : retirer tout ce que le vol rend faux, et écrire la section
neuve au plus court (D12). Le gate réel du critère 7 reste
`cargo test -p mika-common --test claude_md_size`.

**É3 — retirer `connect_mcp` ne perd aucun chemin de migration.** La fonction
déclenche la migration mika#1737 AC5 de `{agent_home}/mcp.json` vers le chemin
operator-shell. Quatre autres sites la déclenchent (tableau ci-dessus), dont la
sous-commande `mika mcp` et le démarrage de spirit. Mesuré, et non supposé : une
provenance ne se fabrique pas (mika#2331).

**É4 — `DbContext.github_app` a zéro lecteur, et son retrait *supprime* un
`expect`.** Le chemin d'équipe de `chat.rs` (l. 1065, 1082) et `ask.rs:841`
bâtissent chacun leur `GitHubApp::from_settings`. Le champ ne sert donc rien, et
son `#[expect(dead_code)]` part avec lui — ce qui va dans le sens du critère 4
(« aucun `allow` ni `expect` **neuf** ») plutôt que contre lui.

**É5 — retirer le `Deref` ne coûte aucun site d'appel.** `App::new` (l. 561-575)
et les deux respawns lisent `worker._ctx.async_db`, `.home_dir`, `.global_home`
et `.settings` à travers `Deref<Target = DbContext>`. Dès que `_ctx` **est** un
`DbContext`, ces quatre lectures résolvent directement. L'`impl Deref` part sans
qu'une ligne d'appel bouge.

**É6 — deux imports de `chat.rs` survivent aux retraits.**
`use std::sync::atomic::{AtomicBool, Ordering};` reste : `shutdown_initiated`
l'emploie (l. 491, 784, 909, 990), et seul `skills_dirty` part.
`use …::assemble_agent_registry;` reste aussi : l'appel de démarrage de la l. 253
survit, et c'est exactement ce que le critère 4 demande. Les trois imports qui
meurent sont dans `init.rs` : `GitHubApp`, `LlmProvider`, `Arc`.

**É7 — quatre commentaires de `chat.rs` nomment un type ou un geste qui
disparaît, et la tête ne les liste pas.** Les l. 62-63 et 1337 nomment
`&AppContext` ; les l. 787 et 804 nomment le `mcp.shutdown` de sortie. Ce sont
des lignes de commentaire, donc le filtre de la garde les ignore et **rien ne les
forcerait**. Elles sont dans un fichier autorisé, et un commentaire qui nomme un
type supprimé est faux. Ce plan les réécrit (D1, D2) : c'est un ajout au
périmètre de la tête, nommé ici plutôt que découvert en revue.

**É8 — `/model` ne changeait déjà pas `app.provider` dans deux cas sur trois.**
Les deux tests OpenRouter l'assertent. Le critère 3 ne retire donc qu'un bras,
celui de `result.provider_changed` (l. 660-662), et deux tests existants
deviennent la mesure de référence du nouveau comportement.

**É9 — `/provider set model …` n'a aucun test aujourd'hui.** La dernière phrase
du critère 3 (« `/provider set model <x>` envoie toujours `SetModel` ») demande
donc un test **neuf**, et non une adaptation. Sans lui, « toujours » n'est tenu
par rien : c'est le bras qu'un éditeur pressé retirerait en même temps que
l'autre, croyant finir le travail.

**É10 — « le message d'`ask.rs` » : une constante, pas une copie.** Le critère 1
demande que `mika chat --model ""` soit refusé « avec le message d'`ask.rs` ».
Deux littéraux identiques dans deux fichiers dérivent — c'est la forme exacte que
`init::tests::mika2304_the_cli_keeps_no_second_resolver` existe pour refuser une
porte plus loin. Décision D4 : une constante `pub(crate)` dans `ask.rs`, lue par
les deux sites. Le message rendu reste identique à l'octet, donc le critère est
tenu au sens littéral, et il le reste après la prochaine reformulation.

---

## 2. Décisions

**D1 — `AgentWorker._ctx` et `spawn_agent_worker` prennent un `DbContext`.** Le
worker n'a jamais eu besoin que de la base, et `_ctx` ne vit que pour la fermer
en sortie. Le **préfixe littéral `async fn spawn_agent_worker(`** est conservé :
c'est le témoin des deux gardes de ce fichier (l. 1775, l. 1852), et le changer
rendrait les deux vertes par cécité. La doc de la l. 38 et les deux phrases des
l. 62-63 cessent de nommer `AppContext` (É7).

**D2 — MCP quitte la TUI en entier.** `connect_mcp` (l. 262), `worker_mcp`
(l. 291) et la fermeture de sortie (l. 477-480) partent de `chat.rs` ;
`init::connect_mcp` part de `init.rs`. Les deux commentaires du superviseur qui
nomment encore `mcp.shutdown` (l. 787, 804) sont réécrits : ce qu'ils disaient
— pourquoi la poignée de main de sortie peut être sautée — n'a plus de poignée de
main à nommer, mais l'abandon de la tâche du superviseur, lui, reste vrai et se
dit sans MCP. Aucun outil de la TUI ne passait par ces connexions depuis V1 : le
tour tourne chez spirit, qui monte son propre `McpManager`.

**D3 — les deux rechargements à chaud, `skills_dirty`, `skills_reloaded` et
`worker_skills` partent ; `updated_skills` vaut `None`.** Le drapeau n'est posé
par personne : les deux branches étaient mortes depuis leur écriture.
`updated_skills` est passé `None` **à ses deux sites** (l. 390, 397) plutôt que
par une liaison intermédiaire, et un commentaire dit pourquoi le champ reste dans
la réponse : `tui/app.rs` le lit, et `tui/app.rs` doit rester inchangé à l'octet
(critère 6). L'assemblage de démarrage de la l. 253 **reste** — une fois, ce que
le critère 4 mesure.

**D4 — `run` passe par `init_db_only_for_agent` et refuse localement un `--model`
vide.** Trois conséquences. (a) `let mut ctx` redevient `let ctx` : plus rien ne
mute le contexte, et un `mut` inutile est un rouge de clippy. (b) Le refus prend
la forme d'`ask.rs` — `if let Some(model) = model_override && model.trim().is_empty()`
puis `anyhow::bail!` — et son message vient d'une constante `pub(crate)` posée
dans `ask.rs` et lue par les deux (É10). (c) Le commentaire des l. 538-542, qui
**annonçait ce retrait**, est réécrit pour dire ce qui est désormais vrai :
l'override part brut au worker, et spirit le résout et le refuse (mika#2304).

**D5 — le modèle affiché au démarrage est l'override quand il y en a un, et la
décision vit dans `spawn_agent_worker`.** Une ligne,
`model_override.clone().unwrap_or_else(|| ctx.settings.active_model_display())`,
posée **avant** que `model_override` ne soit déplacé dans `worker_override`, et la
l. 483 part. Elle est dans `spawn_agent_worker` et non dans `run` parce que les
deux respawns passent par la même porte : sans override ils retombent sur
`active_model_display()`, c'est-à-dire sur le comportement d'aujourd'hui, sans
qu'un second site ait à le redire.

**D6 — les deux respawns passent par `init_db_only_for_agent`, sans override.**
Leurs commentaires (l. 811-815, l. 926-928) disaient « porté par V3b-2 ». Ce vol
**est** V3b-2 et ne le porte pas : la survie de l'override à `/restart` et à la
bascule d'agent est hors périmètre, nommément. Les commentaires sont réécrits
pour dire cela, et pas un vol qui viendrait.

**D7 — `init.rs` perd six choses et trois imports.** `AppContext`, son `Deref`
(É5), `impl override_model`, `init_for_agent`, `connect_mcp` (É3) et
`DbContext.github_app` avec son `GitHubApp::from_settings` de la l. 133 (É4).
Les imports `GitHubApp`, `LlmProvider` et `Arc` partent ; `Path` reste. Le
`mod tests` de mika#2304 est intouché : son témoin est le nom du test lui-même.
Aucun `allow` ni `expect` n'est ajouté — un `expect` **disparaît**.

**D8 — `/provider <nom>` écrit, n'envoie rien, ne touche aucun champ, et le
dit.** Le bras de bascule garde `switch_provider` (donc l'écriture de
`llm_provider`, la persistance du `{provider}_model` par défaut et les
avertissements), et perd `app.model`, `app.provider`, l'envoi de `SetModel` et le
`needs_redraw` qui n'a plus rien à redessiner. Sa réponse nomme le redémarrage :
« écrit dans `config.toml` ; mika-spirit le prend à son prochain redémarrage, pour
tous les tours de l'agent ». **Le pourquoi, qui n'est pas lisible dans le code** :
un override ne re-dispatche jamais vers un autre provider
(`mika-common/src/llm/model_override.rs` l. 72, mika#1591) — un `SetModel` préfixé
d'un provider que le spirit **en cours d'exécution** n'a pas configuré serait
refusé fermement, et couperait tous les tours suivants au lieu de changer de
provider. `app.provider` devient donc un champ d'**affichage** : il dit sous quel
provider les tours de cette TUI partent, et ce provider ne change qu'au
redémarrage de spirit. C'est le cœur du vol, et le test négatif du critère 2 est
ce qui l'épingle.

**D9 — `/model <x>` qui change de provider envoie l'override et laisse
`app.provider` tranquille.** Le bras `result.provider_changed` ne touche plus le
champ ; `format_worker_model(target_provider, …)` continue de rendre la valeur
**préfixée du provider cible**, et c'est elle qui part sur le fil — spirit décide
(mika#2304). La réponse garde « (switched provider to …) » mais dit ce que ce
« switched » veut dire : persisté dans `config.toml` pour **tout l'agent**, pris
au prochain redémarrage de mika-spirit. Le bras `/provider set model <x>` n'est
pas touché et continue d'envoyer `SetModel` : il ne change pas de provider, donc
il ne tombe pas sous mika#1591.

**D10 — `fetch_models` en tâche de fond reste.** Il préchauffe le cache des
modèles du provider qui vient d'être écrit dans `config.toml`, et `/model` sans
argument liste depuis ce même `config.toml` (`list_models` lit les réglages, pas
`app.provider`). Le préchauffage sert donc toujours le geste suivant de
l'utilisateur. Le retirer serait une perte de confort sans rapport avec le vol.

**D11 — la garde de V3b-1 s'étend, garde son nom, et passe à deux sites.**

- **Son nom ne change pas.** Le critère 5 l'autorise, la garde voisine de ce même
  fichier le déconseille : « c'est une garde **par fichier**, pas par vol, et la
  renommer casserait la trace des deux vols qui la citent » (l. 1743-1745). Le
  nom `mika1727_v3b1_le_tour_de_la_tui_ne_construit_plus_son_provider` couvre
  bien `init_for_agent`, `AppContext` et `override_model(` ; il couvre moins bien
  `connect_mcp`, `McpManager` et `skills_dirty`, et **cette tension est nommée
  ici** plutôt que réparée par un renommage qui coûterait la trace.
- **Deux sites, un témoin chacun** : `commands/chat.rs` avec
  `async fn spawn_agent_worker(`, et `commands/ask.rs` avec
  `--task-id value must not be empty` — unique dans le crate, en production
  (l. 194), et hors du périmètre de ce vol, donc sa disparition signifie bien « la
  garde ne vise plus le bon fichier » et non « le vol a bougé ». Un témoin
  générique comme `pub async fn run(` passerait sur le mauvais fichier : c'est
  exactement la cécité que le témoin existe pour refuser (mika#2398).
- **Huit motifs** : les deux de V3b-1 (`worker_settings`, `make_llm_provider(`)
  plus `init_for_agent`, `AppContext`, `connect_mcp`, `McpManager`,
  `override_model(` et `skills_dirty`. Aucune collision de sous-chaîne :
  `init_db_only_for_agent` et `init_base_for_agent` ne contiennent pas
  `init_for_agent`.
- **Le contre-témoin reste, et sur `chat.rs` seulement.** Il y est
  indispensable : la table de motifs de la garde vit dans la moitié **test** du
  même fichier et contient les motifs en clair. Un masquage perdu la ferait se
  dénoncer elle-même, et le réflexe serait de « réparer » par une entrée
  d'allowlist — précisément le piège. Sur `ask.rs` il n'y a rien à
  contre-témoigner : sa moitié test (à partir de la l. 935) ne contient aucun des
  huit motifs, donc un masquage perdu n'y produirait aucun rouge. Un
  contre-témoin y serait du théâtre, et ce plan préfère le dire.
- **Allowlist vide et assertie vide**, sur place.

**D12 — la doc dit ce qui est vrai, au plus court.** Dans
`crates/mika-cli/CLAUDE.md` : une section `### Le worker de la TUI ne porte plus
de provider (mika#1727, vol V3b-2)` qui dit que le worker ne porte plus ni
provider ni MCP, que `/model` voyage sous `mika.model_override`, et ce que
`/provider` fait désormais ; le paragraphe « *Unchanged:* `mika chat --model`,
which runs in process and really does execute against the provider it builds »
(l. 253-257) dit l'inverse de la vérité et est réécrit ; la puce `/provider` et
`/model` de la l. 726 est refaite. Le fichier **perd** ce que le vol rend faux,
dans le même commit (É2). Dans `docs/slash-commands.md` : le paragraphe
« Cross-provider switch » de `/model` (l. 240-250) et le bloc de bascule de
`/provider` (l. 273-302).

**Amendement du pilote (2026-11-16) — un huitième fichier change :
`crates/mika-agent/docs/slash-commands.md`.** `slash-commands.md` est dans la
liste de `scripts/sync-agent-docs.sh` (l. 21), et le job `docs-sync` de `ci.yml`
(l. 349-367) relance ce script puis échoue sur tout `git diff` de
`crates/mika-agent/docs/`. Toucher `docs/slash-commands.md` — ce que D12 exige —
rend donc la copie locale du crate désynchronisée, et la PR rouge. Le fichier
n'est pas du plan de contrôle, c'est une copie mécanique produite par le script
et jamais éditée à la main. Ni le corps du ticket ni ce plan ne l'avaient prévu,
d'où cet amendement. **Conséquence sur la borne** : les 27 lignes de
`docs/slash-commands.md` sont comptées **deux fois**, le miroir compris, soit 27
lignes de plus que le « Total estimé » de ce plan. Mesure après C3 : 367 pour
400.

---

## Fire-Disposition

Un détecteur est livré — la garde étendue de D11 — donc cette section est
obligatoire (mika#2306, mika#1574).

**Option retenue : (a) exception d'allowlist nommée — livrée vide.**

- **Violations préexistantes : zéro, par construction.** Le commit qui arme la
  garde (C1) est celui qui retire les huit motifs de la production des deux
  fichiers. Il n'y a donc rien à excepter, et `ALLOWED` est livrée **vide et
  assertie vide** sur place (doctrine mika#2201, #2323, #2425, #2457, #2759 : une
  allowlist vide s'assure d'être vide, sinon elle se remplit en silence).
- **Portée : deux fichiers**, `commands/chat.rs` et `commands/ask.rs`, et non le
  crate. `make_llm_provider` garde quatre lecteurs légitimes ailleurs dans
  `mika-cli` (`init.rs`, `commands/provider.rs`, `commands/doctor.rs`,
  `commands/agents.rs`) qui bâtissent un provider pour autre chose qu'un tour, et
  `init_db_only_for_agent` vit dans `init.rs`. Une garde par crate aurait demandé
  une allowlist peuplée, et une allowlist peuplée ne refuse plus rien
  (mika#2309).
- **Ce qui arrive quand elle tire.** Chaque motif nomme son remède. Si un futur
  éditeur a besoin d'un provider, d'un `McpManager` ou d'un rechargement de
  skills dans la TUI, le geste n'est **pas** d'ajouter une entrée à `ALLOWED` :
  c'est de rouvrir D8 et D2 avec Vincent, parce qu'un provider construit dans la
  TUI est la duplication même que mika#1727 ferme.
- **Anti-vacuité.** Un témoin par site, et le contre-témoin de `chat.rs` : voir
  D11, qui dit aussi pourquoi `ask.rs` n'en reçoit pas. Sans eux, un scan qui
  lirait le mauvais fichier resterait vert pour toujours (mika#2103, #2205,
  #2398).
- **Option (b) — livrer désarmée : refusée.** Il n'y a rien à désarmer : zéro
  violation après C1.
- **Option (c) — halte et escalade : sans objet.** Les deux bornes tiennent
  (§ *Taille estimée*), et rien n'oblige à s'arrêter ici.

---

## 3. Ordre de commit

Un commit par garde, après que la garde est verte **avec son test**, et non en fin
de vol. Les tests ne se reportent jamais à un vol suivant. C'est l'ordre suggéré
par la tête du ticket, et ce plan le confirme : il arme la garde dans le commit
qui retire ses motifs.

### C1 — les retraits, et la garde qui les tient (critères 1, 4, 5)

`commands/chat.rs`, `init.rs`, `commands/ask.rs`, `commands/chat_turn.rs`.

1. `chat.rs` : `DbContext` à la place d'`AppContext` à l'import, dans
   `AgentWorker`, dans la signature de `spawn_agent_worker` (préfixe littéral
   gardé) et dans les quatre commentaires d'É7 (D1).
2. `chat.rs` : MCP, les deux rechargements, `skills_dirty`, `skills_reloaded`,
   `worker_skills` ; `updated_skills` passé `None` à ses deux sites, avec le
   pourquoi (D2, D3).
3. `chat.rs` : la ligne d'affichage de D5, et la l. 483 part.
4. `chat.rs` : `run` passe par `init_db_only_for_agent`, `let ctx` perd son `mut`,
   le refus du `--model` vide arrive, le commentaire de V3b-1 est réécrit (D4).
5. `chat.rs` : les deux respawns et leurs commentaires (D6).
6. `init.rs` : les six retraits et les trois imports (D7).
7. `ask.rs` : la l. 167, la constante partagée du message, et le premier
   paragraphe du commentaire mika#2304 — qui dit pourquoi la base suffit : le tour
   s'exécute chez spirit, qui a son propre environnement, et une clé absente du
   shell ne doit pas refuser un tour que spirit sait servir (mika#2293).
8. `chat_turn.rs` : les deux phrases de doc (l. 6, l. 257) disent ce qui reste
   vrai — le worker exige une base et un canal, pas un provider — sans nommer
   `AppContext`.
9. **Garde des critères 4 et 5** — D11, avec son allowlist vide assertie, ses
   deux témoins et le contre-témoin de `chat.rs`, dans le `mod tests` de
   `chat.rs`.

Le critère 1 n'apporte **pas** de test, et c'est dit plutôt que caché : `run`
n'est atteignable d'aucun test (il ouvre une base, une TUI et un worker), la même
prémisse qu'en V3b-1 § É1. Ce qui le rend relisable est sa taille — cinq lignes de
refus et une ligne d'affichage — et le fait que la valeur refusée et la valeur
affichée soient toutes deux des constantes locales lisibles à l'œil.

`cargo clippy --all-targets -p mika-cli` vert, `cargo test -p mika-cli` vert.
≈ 151 lignes.

### C2 — `/provider` et `/model` (critères 2 et 3)

`tui/commands/handlers.rs`.

1. `handle_provider` : D8. Le bras de bascule perd `SetModel`, `app.model`,
   `app.provider` et `needs_redraw` ; sa réponse nomme le redémarrage ; le
   commentaire porte le pourquoi mika#1591.
2. `handle_model` : D9. Le bras `provider_changed` ne touche plus `app.provider` ;
   la réponse dit ce que la persistance couvre.
3. **Test du critère 2 — le test négatif du vol.** C'est
   `test_provider_switch_worker_gets_provider_prefix` (l. 2330) **retourné**, pas
   un test neuf à côté : le test qui assertait qu'un `SetModel` partait asserte
   désormais que `drain_requests` rend **vide**, que `app.provider` et `app.model`
   sont inchangés, que `config.toml` porte `llm_provider = "deepseek"`, et que la
   réponse nomme le redémarrage de mika-spirit. Renommé pour dire ce qu'il tient.
4. **Test du critère 3** — `test_model_cross_provider_alias_switches_provider`
   (l. 2252) adapté : `app.provider` **inchangé**, un `SetModel` qui porte
   exactement ce que `format_worker_model` rend pour le provider cible, et la
   réponse qui nomme la persistance pour tout l'agent.
5. **Test neuf du critère 3, seconde phrase** (É9) : `/provider set model <x>`
   envoie toujours `SetModel`, avec le préfixe d'`app.provider`. Sans lui, le
   « toujours » du critère n'est tenu par rien.
6. Adaptation des quatre autres tests du § 1 : `test_provider_switch_reads_configured_model`,
   `test_provider_switch_to_anthropic_worker_gets_prefix`,
   `test_model_direct_name_sets_model` (sa préparation passe par un `app.provider`
   posé et un `config.toml` écrit, puisque `/provider` ne bascule plus l'`App`) et
   `test_model_direct_with_provider_prefix`. Aucun n'est supprimé.

`cargo clippy --all-targets -p mika-cli` vert, `cargo test -p mika-cli` vert.
≈ 170 lignes.

### C3 — la doc (critère 7, `claude_md_size` compris)

`crates/mika-cli/CLAUDE.md` et `docs/slash-commands.md`, selon D12. Le commit
retire ce que le vol rend faux **et** ajoute la section neuve, pour que le fichier
ne s'allonge pas d'un ajout sans retrait (É2).

`cargo test -p mika-common --test claude_md_size` vert. ≈ 56 lignes.

### Fin de pipeline

Dans cet ordre, avant d'ouvrir la PR :

1. `cargo fmt`
2. `cargo clippy --all-targets -p mika-cli` — vert
3. `cargo test -p mika-cli` — vert
4. `cargo test -p mika-common --test claude_md_size` — vert
5. `bash scripts/verify-egress-manifest.sh --report`
6. `bash scripts/verify-pipeline.sh origin/main`

**Si l'un des deux derniers est rouge, le pilote s'arrête et le dit** (critère 7).
Il ne contourne pas, il ne commente pas la garde.

Vérifications de périmètre, à la main, avant la PR :

- `git diff --stat origin/main...HEAD` ne nomme que `commands/chat.rs`,
  `init.rs`, `commands/ask.rs`, `commands/chat_turn.rs`,
  `tui/commands/handlers.rs`, `crates/mika-cli/CLAUDE.md`,
  `docs/slash-commands.md`, `crates/mika-agent/docs/slash-commands.md`
  (amendement de D12 ci-dessus) et ce plan.
- `git diff origin/main...HEAD -- crates/mika-cli/src/tui/app.rs` est **vide**
  (critère 6, à l'octet) ; de même pour `commands/model.rs`,
  `commands/provider.rs`, `remote_ask.rs`, `crates/mika-a2a/`,
  `crates/mika-agent/src/`, `crates/mika-common/` et `crates/mika-gateway/`. La
  racine `crates/mika-agent/` est remplacée par son `src/` : seule la copie
  mécanique `crates/mika-agent/docs/slash-commands.md` y change, et elle n'est
  produite que par `scripts/sync-agent-docs.sh`.
- Aucun `#[allow(` ni `#[expect(` neuf dans le diff (critère 4) — un `expect`
  disparaît (É4).
- **Aucun fichier du plan de contrôle** : ni `.github/workflows/`, ni `.claude/`,
  ni `skills/bundled/`, ni `crates/mika-agent/src/well_known_agents.rs`, ni
  `.mika/`, ni aucun chemin de CODEOWNERS, `docs/egress/` compris.

**Vol lean (mika#2694)** : pas de `ce-simplify-code`, pas de `ce-code-review`. Le
corps de PR porte `Code review: delegated to mika-qa (mika#2694)`,
`Refs mika#1727`, la conséquence É1 (`docs/mcp.md` reste à corriger) et
`Tracked in: senara-solutions/mika#1727` pour le reste — le test de bout en bout
de l'override après `SetModel`, l'affichage de `mika.effective_model`, la survie
de l'override à `/restart` et à la bascule, une clé `mika.provider_override`. Le
corps s'écrit dans `pr-body.md` **sous le worktree**, jamais dans `/tmp`, jamais
par heredoc (mika#2211) ; les brouillons vivent sous `.pilot-scratch/` et **ne se
suppriment jamais** (mika#2548).

**Aucun ticket n'est ouvert par ce vol**, pas même de suivi (gel jusqu'au 01/12).

### Halte du pilote sur la borne

Après **chaque** commit, mesurer `git diff --numstat origin/main...HEAD`, plan
exclu.

- **Au-dessus de 400 : arrêt.** Pas de commit suivant, pas de PR.
- **Au-dessus de 360 après le premier commit : arrêt aussi** — le reste ne
  tiendrait pas.

Le dernier message dit alors, mot pour mot :
**« HALTE borne : N lignes pour 400 »**, suivi du compte par commit.

La marge estimée est de 23 lignes seulement, et le poste le plus susceptible de
la manger est le lot de tests de C2. **Le pilote ne rabote ni commentaire ni
test pour tenir** : les commentaires comptent dans la borne et portent un
pourquoi qu'on ne lit pas dans le code ; les tests arrivent dans le commit de ce
qu'ils tiennent. Si le code dépasse, il s'arrête selon la règle ci-dessus et le
dit.

**Si un hook ou un classifieur refuse un geste**, au pilote ou à un sous-agent, le
pilote **s'arrête et le déclare**, même si le refus paraît un faux positif. Il ne
le contourne jamais par un autre outil : pas de heredoc Bash à la place de
`Write`, pas de reformulation (contre-exemple mika#2869, 2026-10-21).

### Si le code s'écarte de ce plan

La QA juge contre ce plan, pas contre le corps du ticket. Si le pilote doit s'en
écarter sur un point — un fichier, un critère, un nom — **il amende ce plan sur la
branche dans un commit `docs(plans)` avant de pousser le code qui s'en écarte**,
en nommant le point et la raison.

---

## 4. Definition of Done

- [ ] `spawn_agent_worker` et `AgentWorker._ctx` portent un `DbContext`, et le
      préfixe littéral `async fn spawn_agent_worker(` est conservé.
- [ ] MCP a quitté `chat.rs` et `init.rs` ; les deux commentaires du superviseur
      ne nomment plus de `mcp.shutdown`.
- [ ] Les deux rechargements à chaud, `skills_dirty`, `skills_reloaded` et
      `worker_skills` ont disparu de `chat.rs` ; `updated_skills` vaut `None` à
      ses deux sites ; `assemble_agent_registry` n'est appelé qu'une fois.
- [ ] `run` passe par `init_db_only_for_agent`, refuse un `--model` vide avec le
      message d'`ask.rs` porté par une constante partagée, et passe l'override au
      worker tel que tapé.
- [ ] Le modèle affiché au démarrage est l'override quand il y en a un.
- [ ] Les deux respawns passent par `init_db_only_for_agent` sans override, et
      leurs commentaires ne renvoient plus à V3b-2.
- [ ] `init.rs` a perdu `AppContext`, son `Deref`, `override_model`,
      `init_for_agent`, `connect_mcp`, `DbContext.github_app` et trois imports ;
      le `mod tests` de mika#2304 est intouché.
- [ ] Aucun `allow` ni `expect` neuf dans le crate ; celui de `github_app` a
      disparu.
- [ ] Les deux phrases de doc de `chat_turn.rs` ne nomment plus `AppContext`.
- [ ] `/provider <nom>` écrit `llm_provider`, n'envoie aucun `SetModel`, ne
      change ni `app.provider` ni `app.model`, et sa réponse nomme le redémarrage
      de mika-spirit.
- [ ] `/model <x>` qui change de provider envoie l'override rendu par
      `format_worker_model`, ne change pas `app.provider`, et sa réponse nomme la
      persistance pour tout l'agent. `/provider set model <x>` envoie toujours
      `SetModel`.
- [ ] La garde de V3b-1 balaie la production de `commands/chat.rs` **et** de
      `commands/ask.rs`, refuse huit motifs, garde une allowlist vide **assertie
      vide**, porte un témoin par site et le contre-témoin de `chat.rs`.
- [ ] Les tests qui tenaient l'ancien comportement sont **adaptés**, aucun n'est
      supprimé ; le test négatif du critère 2 et le test de `/provider set model`
      existent ; chaque test est dans le commit de ce qu'il tient.
- [ ] `cargo clippy --all-targets -p mika-cli`, `cargo test -p mika-cli` et
      `cargo test -p mika-common --test claude_md_size` verts.
- [ ] `bash scripts/verify-egress-manifest.sh --report` et
      `bash scripts/verify-pipeline.sh origin/main` lancés et verts.
- [ ] `crates/mika-cli/CLAUDE.md` et `docs/slash-commands.md` disent ce qui est
      vrai de `/model`, de `/provider` et du worker ; le paragraphe
      « *Unchanged:* `mika chat --model` » a disparu.
- [ ] Seuls les sept fichiers listés changent, plus la copie mécanique
      `crates/mika-agent/docs/slash-commands.md` (amendement de D12) ;
      `tui/app.rs` est inchangé à l'octet ; aucun fichier du plan de contrôle
      n'est touché.
- [ ] Au plus 400 lignes ajoutées, plan exclu.
- [ ] La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves` —
      corps et commits. Si le gabarit de reprise l'ouvre avec un `Closes`, il est
      corrigé **avant toute revue**, puis
      `gh pr view <N> --json closingIssuesReferences` rend `[]`.
- [ ] Aucun ticket ouvert par ce vol (gel jusqu'au 01/12) ; le reste est noté
      `Tracked in: senara-solutions/mika#1727`.
- [ ] La conséquence É1 — `docs/mcp.md` l. 79, 87 et 386 restent faux, hors
      périmètre — est écrite dans la PR.

---

## 5. Acceptance criteria

Les huit critères de V3b-2, transcrits du corps du ticket.

1. `mika chat --model ""` est refusé avant tout tour, avec le message d'`ask.rs`.
   Un `--model` non vide part au worker tel que tapé, et le modèle affiché au
   démarrage est cette valeur.
2. `/provider <nom>` écrit `llm_provider` dans `config.toml`, n'envoie aucun
   `SetModel` et ne change ni `app.provider` ni `app.model`. Sa réponse nomme le
   redémarrage de mika-spirit. **Ce test-ci est le test négatif du vol.**
3. `/model <x>` qui change de provider envoie un `SetModel` portant `x` tel que
   `format_worker_model` le rend, ne change pas `app.provider`, et sa réponse
   nomme la persistance pour tout l'agent au redémarrage de mika-spirit.
   `/provider set model <x>` envoie toujours `SetModel`.
4. `commands/chat.rs` n'appelle plus `assemble_agent_registry` qu'une fois, et
   `skills_dirty` n'y existe plus. `DbContext.github_app` n'existe plus, et aucun
   `allow` ni `expect` neuf n'apparaît dans le crate.
5. La garde `mika1727_v3b1_le_tour_de_la_tui_ne_construit_plus_son_provider`
   balaie la production de `commands/chat.rs` et de `commands/ask.rs`, et refuse,
   en plus de ses deux motifs, `init_for_agent`, `AppContext`, `connect_mcp`,
   `McpManager`, `override_model(` et `skills_dirty`. Son allowlist reste vide et
   assertie. Chaque site a son témoin, et le contre-témoin de `chat.rs` reste.
6. Seuls changent les fichiers listés. `tui/app.rs` est inchangé, à l'octet.
7. `cargo clippy --all-targets -p mika-cli` est vert. `cargo test -p mika-cli`
   est vert, de même que `cargo test -p mika-common --test claude_md_size`. Le
   pilote lance `bash scripts/verify-egress-manifest.sh --report` et
   `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il
   trouve l'un des deux rouge, il s'arrête et le dit.
8. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
   compris si le gabarit de reprise l'ouvre.

**Comment le critère 1 est tenu, puisqu'il n'est pas mesurable où il est écrit** :
voir C1. `run` n'est atteignable d'aucun test — il ouvre une base, un worker et
une TUI — donc la propriété est tenue par la revue sur six lignes dont le message
est une constante partagée avec `ask.rs`, dont le refus a la forme exacte du
refus déjà testé là-bas, et dont la valeur affichée est l'argument lui-même. La
garde du critère 5, elle, refuse structurellement le retour de l'ancien chemin.

**La garde garde le nom d'un vol antérieur** : D11 dit pourquoi, et nomme la part
des huit motifs que ce nom couvre mal.

---

## Taille estimée

| livrable | lignes estimées |
|---|---|
| `chat.rs` — `DbContext`, MCP, rechargements, `updated_skills`, docs (C1) | 25 |
| `chat.rs` — `run` : porte, refus du `--model` vide, affichage, commentaire (C1) | 23 |
| `chat.rs` — les deux respawns et leurs commentaires (C1) | 9 |
| `init.rs` — six retraits, trois imports, docs restantes (C1) | 6 |
| `ask.rs` — l. 167, constante partagée, commentaire mika#2304 (C1) | 11 |
| `chat_turn.rs` — les deux phrases de doc (C1) | 4 |
| garde étendue — deux sites, huit motifs, témoins, contre-témoin, doc (C1) | 73 |
| `handlers.rs` — `handle_provider` (C2) | 18 |
| `handlers.rs` — `handle_model` (C2) | 20 |
| test négatif du critère 2 — l'ancien test retourné (C2) | 45 |
| test du critère 3 — `/model` qui change de provider (C2) | 26 |
| test neuf du critère 3 — `/provider set model` (C2) | 28 |
| adaptation des quatre autres tests (C2) | 33 |
| `crates/mika-cli/CLAUDE.md` — section V3b-2, « Unchanged », puce `/provider` (C3) | 30 |
| `docs/slash-commands.md` — `/model` et `/provider` (C3) | 26 |

Total estimé : 377 lignes

Plafond de tours : 200 (loop-substrate)

Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Les deux unités diffèrent ici.** L'unité de la borne du vol
(`git diff --numstat origin/main...HEAD`, plan exclu) compte les 377 lignes.
L'unité de l'architecte exclut `docs/` et les `CLAUDE.md`, donc les 56 lignes de
C3 : elle compte **321 lignes**. **377 pour 400, et 321 pour 429 : les deux
bornes tiennent, tests compris.** Le groom ne demande donc aucune coupe — et il
n'en pratique aucune de lui-même : une coupe demande le mot de Vincent.

Marge : **23 lignes** avant la borne de 400. C'est mince, et deux choses le
rendent tenable. D'abord le volume du vol est en grande partie **négatif** —
`init.rs` et `chat.rs` perdent près de cent cinquante lignes que `--numstat` ne
compte pas — donc l'estimation ci-dessus ne porte que sur ce qui s'écrit. Ensuite
le poste le plus incertain, les tests de C2, est aussi le plus découpé : six
entrées distinctes, dont quatre adaptations de tests existants. Après C1 (≈ 151),
le seuil de 360 est loin ; après C2 (≈ 321), il reste 79 lignes pour une doc
estimée à 56. **La halte du § 3 reste la règle qui arbitre, pas cette
estimation** — et si elle tombe, elle tombe sur C3, c'est-à-dire sur la doc, ce
que la PR devra dire.

Tracked in: senara-solutions/mika#1727
