# mika#1727 — vol V3b-1 : le modèle choisi voyage sur le fil

- **Ticket** : senara-solutions/mika#1727 — *TUI as thin HTTP client of mika-spirit*
- **Vol** : V3b-1 (le premier des deux moitiés de V3b, coupées par Vincent le 2026-11-16)
- **Branche** : `feat/1727/v3b1-le-modele-choisi-voyage-sur-le-fil`
- **Base mesurée** : `main` @ `a9c229f9`
- **Label** : `loop-substrate` (plafond de tours : 200)
- **Borne du vol** : au plus 450 lignes ajoutées, plan exclu, par `git diff --numstat origin/main...HEAD`
- **La PR porte `Refs mika#1727`**, jamais `Closes`, `Fixes` ni `Resolves` — ni dans le corps, ni dans un commit. Le ticket se ferme au merge de V4.

---

## 0. Pourquoi ce vol tient en un seul vol

Le fil existe déjà en entier. `remote_ask::send_message_to_agent` prend un
`model_override` et le pose sous `mika_a2a::params::MODEL_OVERRIDE_KEY`
(`remote_ask.rs` l. 503, insertion conditionnelle l. 342). Côté serveur,
`resolve_caller_model_override` (`mika-agent/src/server/a2a.rs` l. 671) résout
l'alias, vérifie la clé d'API et **refuse fermement** le tour sur un modèle que
l'agent ne sait pas servir (mika#2304). Rien de tout cela ne change ici.

Ce qui manque est un argument. `chat_turn::send_turn_to_endpoint` passe `None`
(l. 98), le bras `SetModel` du worker valide le modèle puis le jette, et
`mika chat --model x` écrit un champ que personne ne lit. Le vol ajoute donc un
paramètre à quatre fonctions, le fait porter par treize sites, et remplace vingt-sept
lignes de validation locale par une fonction pure de trois lignes utiles. Le reste
du volume est en tests : quatre critères, quatre tests, dans le même commit que ce
qu'ils tiennent.

L'estimation est de **355 lignes** (§ *Taille estimée*), sous la borne de 450 et sous
la borne de 429 de l'unité de l'architecte, **avec** les tests. Il n'y a donc pas de
coupe à demander, et ce plan ne s'arrête pas.

---

## 1. Ce qui est mesuré sur main (@ `a9c229f9`)

### `crates/mika-cli/src/commands/chat_turn.rs` (1095 lignes)

| site | ligne | mesure |
|---|---|---|
| `send_turn_to_spirit` | 72 | 4 paramètres, aucun override. Sa doc **l. 59 à 63 dit l'inverse de ce vol** : « pas d'override de modèle (`/model` reste appliqué en processus en V1) » |
| `send_turn_to_endpoint` | 87 | 4 paramètres ; appelle `send_message_to_agent` avec `None` en 5ᵉ position (l. 98) |
| `livrer_tour_de_callback` | 166 | 6 paramètres ; transmet à la variante à endpoint résolu (l. 175) |
| `livrer_tour_de_callback_sur_endpoint` | 191 | appelle `send_turn_to_endpoint(cadrage, endpoint, session_id, true)` (l. 199) |
| `mod tests` | 324 | `spawn_mock` (l. 347, axum sur `/a2a/{agent}`, capture `last_body`), `ok_task_with_text` (l. 375), `tache_de_callback_livree` (l. 632), `base_de_callback` (l. 666) |
| test de référence | 562 | `mika1727_v3a2a_la_cle_de_callback_voyage_et_son_absence_aussi` — deux moitiés dans une fonction, positive puis négative |
| gardes en place | 872, 987 | `V1_ALLOWED` et `V2A_ALLOWED`, livrées vides et asserties vides |

**Sites d'appel à reprendre, comptés :** `send_turn_to_endpoint` est appelé aux
lignes 79, 199, 412, 439, 463, 505, 529, 545, 570, 598 ; `livrer_tour_de_callback_sur_endpoint`
aux lignes 175, 692, 766. **Treize sites au total.** La tête du ticket en annonce
onze (« 9 de `send_turn_to_endpoint`, 2 du callback ») : ce sont les mêmes, moins les
deux transmissions internes des l. 79 et 175, que la tête couvre déjà par sa clause
sur les signatures. **Les deux lectures concordent** — ce n'est pas un écart, et la QA
qui comptera treize ne doit pas y lire une dérive.

### `crates/mika-cli/src/commands/chat.rs` (1800 lignes)

- `async fn spawn_agent_worker(ctx, agent_name, session_id)` l. 189, rend un 8-uplet.
  Le **préfixe littéral `async fn spawn_agent_worker(`** est le témoin de la garde
  `mika1727_v2d4a_la_tui_ne_cree_plus_sa_session_en_local` (l. 1762) : il reste.
- `let mut worker_settings = ctx.settings.clone();` l. 275. **Aucun autre lecteur** que
  le bras `SetModel`.
- Bras `Message` : `chat_turn::send_turn_to_spirit(&text, …, false)`. Bras `CallbackResult` :
  `chat_turn::livrer_tour_de_callback(…)`. Tous deux partent déjà sur le réseau depuis V3a-2b.
- Bras `SetModel` l. 446 à 472 : clone les réglages, découpe un éventuel préfixe
  `provider/`, parse un `ProviderKind`, appelle `set_provider_model`, puis **valide par
  `updated_settings.make_llm_provider().is_ok()` (l. 469)** et garde la copie.
- `run` applique `--model` par `ctx.override_model(model)?` (l. 541) — qui ne sert aucun
  tour — puis appelle `spawn_agent_worker(ctx, agent_name, session)` (l. 553).
- Deux respawns : bascule d'agent l. 806, `/restart` l. 916, tous deux avec `None` en session.
- `use mika_common::config::Settings;` (l. 34) **reste nécessaire** : `Settings::load(&worker_home)`
  l. 1042, chemin des équipes. Retirer `worker_settings` n'orpheline pas l'import.

### Autour, hors périmètre, mesuré pour décider

- `mika_cli::remote_ask` **réexporte** `MODEL_OVERRIDE_KEY` (bloc `pub use` l. 14 à 30),
  donc les tests lisent la clé par son nom sans qu'aucune ligne de visibilité soit ajoutée
  nulle part. Contraste avec V3a-2a, qui avait dû passer par `mika_a2a::params::`.
- `make_llm_provider` a **quatre lecteurs légitimes hors de ce vol** : `init.rs` l. 118 et 126,
  `commands/provider.rs` l. 71, `commands/doctor.rs` l. 687, `commands/agents.rs` l. 621.
- `handlers.rs` envoie `SetModel` depuis trois sites (l. 670, 737, 783), avec la valeur que
  rend `format_worker_model` — une chaîne `provider/model`. **Il ne change pas dans ce vol.**

### Écarts entre la tête du ticket et la mesure

**É1 — la seconde phrase du critère 3 ne se mesure pas où elle est écrite.**
« Après un `SetModel`, le tour suivant porte l'override » demande d'observer le worker,
or le corps de `spawn_agent_worker` n'est atteignable d'aucun test : il exige un
`AppContext` complet (base, provider, MCP). C'est la prémisse même de l'existence de
`chat_turn.rs`, que sa doc de module énonce. Le critère se ferme donc **par composition** :
la table de cas de la fonction pure, puis la sortie de cette fonction donnée à
`send_turn_to_endpoint` et la clé relue sur le fil. Le maillon qui reste hors test est
**une** affectation de champ dans le worker, visible en revue. Le plan le nomme plutôt
que de prétendre le couvrir.

**É2 — la garde du critère 4 est par fichier, pas par crate.**
Le critère dit « `commands/chat.rs` ne contient plus `worker_settings` ni
`make_llm_provider` » : c'est littéralement un prédicat de fichier, et il sera livré
comme tel (`ProductionScanner::production_of`). Un scan d'étendue crate serait **rouge
sur les quatre lecteurs légitimes** listés ci-dessus — un faux rouge, livré armé, qui
coûterait une PR à quelqu'un d'autre.

**É3 — cette garde vit dans le `mod tests` de `chat.rs`, pas dans celui de `chat_turn.rs`.**
La tête place les tests du vol dans le module `tests` de `commands/chat_turn.rs` ; cette
phrase vise les tests qui ont besoin de `spawn_mock`. Un scan de source a besoin de son
**témoin** et de son **contre-témoin** dans le fichier qu'il scanne : le contre-témoin
nomme une fonction de test de ce fichier, et déclaré depuis `chat_turn.rs` il nommerait
une fonction d'un autre module — l'anti-vacuité ne prouverait plus rien sur le fichier
visé. La garde va donc à côté de `mika1727_v2d4a_la_tui_ne_cree_plus_sa_session_en_local`,
dont elle reprend la forme. Les trois autres tests restent dans `chat_turn.rs`, comme la
tête le demande.

**É4 — une conséquence visible par l'opérateur, que la tête ne dit pas.**
`/model` et `/provider set model …` envoient `SetModel` avec une chaîne `provider/model`.
La valeur voyageant désormais **brute**, et spirit ne retirant un préfixe que lorsqu'il
nomme son propre provider configuré (mika#1591), un `/model autre-provider/x` cesse d'être
une bascule locale à chaud et devient **un refus ferme au tour suivant**, journalisé
`a2a_model_override_refused`. Ce n'est pas une régression de ce vol : c'est le prix,
assumé, de faire voyager le choix au lieu de le garder. La reformulation de `/model` est
explicitement portée par V3b-2. Le plan l'écrit ici pour qu'une QA qui l'observe sache
qu'elle est attendue, et la PR le redira.

---

## 2. Décisions

**D1 — l'override traverse la chaîne en `Option<&str>`, et part brut.**
`send_turn_to_spirit`, `send_turn_to_endpoint`, `livrer_tour_de_callback` et
`livrer_tour_de_callback_sur_endpoint` prennent `model_override: Option<&str>` en
**dernière** position, après `callback_turn`. `&str` et non `String` : les quatre sites
n'ont rien à posséder, et le worker garde déjà la valeur. La doc des quatre dit que la
valeur part **telle que l'opérateur l'a tapée**, que ni l'alias ni le préfixe ne sont
résolus ici, et que spirit résout et refuse (mika#2304). La doc de `send_turn_to_spirit`
l. 59 à 63, qui affirme aujourd'hui l'inverse, est réécrite dans le même commit — un
commentaire qui mentirait d'un vol est pire qu'un commentaire absent.

**D2 — une fonction pure dit ce que `SetModel` fait de l'override.**
`override_de_set_model(model: &str) -> Option<String>` dans `chat_turn.rs` : la valeur
`trim`ée si elle reste non vide, `None` sinon. Pourquoi `None` et pas la chaîne vide :
une clé `mika.model_override` vide serait transmise au serveur, qui n'a aucun modèle de
ce nom et **refuserait fermement le tour** — l'opérateur perdrait son tour sur une touche
mal tapée. `None` rend le tour d'avant la clé, bit pour bit. La fonction est pure pour
une seule raison : c'est la seule partie du bras `SetModel` qu'un test peut atteindre
(É1).

**D3 — le worker garde l'override dans un `let mut`, comme sa session.**
`let mut worker_override: Option<String>`, initialisé par l'argument de
`spawn_agent_worker`, capturé par la tâche : exactement la forme de `worker_session` et
`worker_skills`. Aucune primitive de synchronisation nouvelle — l'état du worker est déjà
possédé par une seule tâche, et le faire autrement serait répondre à un problème que ce
vol n'a pas.

**D4 — `SetModel` ne valide plus rien localement.**
Le bras devient `worker_override = chat_turn::override_de_set_model(&model);`.
`worker_settings` (l. 275) et le `make_llm_provider()` de la l. 469 partent avec lui.
La validation **ne disparaît pas**, elle change de côté : c'est le propre de mika#2304 que
le côté exécutant arbitre, puisque c'est lui qui détient les clés d'API et les alias. Un
client qui pré-valide avec sa propre configuration répond à une question qu'il ne peut
pas se poser. Conséquence opérateur : É4.

**D5 — le `--model` de départ voyage, `ctx.override_model` reste.**
`run` passe la valeur de `--model` à `spawn_agent_worker`, telle que tapée.
`ctx.override_model(model)?` (l. 541) **reste à sa place** : il valide encore le provider
local du processus, et son retrait appartient à V3b-2 avec le reste du chemin local. Les
deux coexistent un vol durant, et le commentaire du site le dit.

**D6 — les deux respawns passent `None`.**
Bascule d'agent (l. 806) et `/restart` (l. 916) repartent sans override. La survie de
l'override à ces deux gestes est **hors de ce vol**, nommément, par la tête du ticket. Le
`None` porte un commentaire disant lequel des deux il est — un `None` muet se lirait comme
un oubli au prochain passage.

**D7 — la garde du critère 4 est un scan par fichier, allowlist livrée vide.**
`mika1727_v3b1_le_tour_de_la_tui_ne_construit_plus_son_provider`, dans le `mod tests` de
`chat.rs` (É3) : `ProductionScanner::production_of` sur `commands/chat.rs`, motifs
`worker_settings` → *« l'override vit dans le worker, pas dans une copie des réglages »*
et `make_llm_provider(` → *« le provider est construit par spirit, qui détient les clés »*.
`const ALLOWED: &[&str] = &[]`, **assertie vide sur place**. Témoin d'anti-vacuité :
`production.contains("async fn spawn_agent_worker(")`. Contre-témoin : la production ne
contient pas le nom d'une fonction de test de ce fichier — la garde prouve ainsi qu'elle
lit bien la production et non le module de tests. Même forme que la garde V2d-4a voisine,
délibérément : une garde qui ressemble à sa voisine se relit sans la redécouvrir.

**D8 — ce que les tests mesurent, et ce qu'ils ne mesurent pas.**
Les trois tests de fil passent par `spawn_mock` et relisent `last_body`, comme
`mika1727_v3a2a_la_cle_de_callback_voyage_et_son_absence_aussi`. La clé se lit par
`mika_cli::remote_ask::MODEL_OVERRIDE_KEY`, **jamais par un littéral** : une clé renommée
en amont doit rougir ici, pas passer. Chaque test positif porte sa moitié négative dans la
**même** fonction, et chaque assertion d'absence est accompagnée d'une assertion de
présence sur une autre clé — sans quoi un corps vide, ou un mock jamais atteint,
satisferait l'absence (mika#2103). Aucun endpoint mort n'est nécessaire ici ; s'il le
devenait, ce serait `mika_common::dead_endpoint::DeadEndpoint::reserve()`, jamais un
`bind(:0)` relâché (mika#2569).

**D9 — ce que ce vol ne livre pas, et ne tente pas de livrer.**
`init_for_agent`, `AppContext`, `override_model`, MCP, les rechargements et `skills_dirty`,
`ask.rs`, `DbContext.github_app`, la garde de scan de V3b-2, `/provider`, la reformulation
de `/model` et la doc : **V3b-2**. L'affichage de `mika.effective_model` rendu par spirit,
la survie de l'override à `/restart` et à la bascule d'agent : hors vol. Le reste se note
`Tracked in: senara-solutions/mika#1727` dans la PR. **Ce vol n'ouvre aucun ticket**, pas
même de suivi (gel jusqu'au 01/12).

---

## Fire-Disposition

Une garde est livrée (D7), donc cette section est obligatoire (mika#2306, mika#1574).

**Option retenue : (a) exception d'allowlist nommée — livrée vide.**

- **Violations préexistantes : zéro.** Le commit qui arme la garde (C2) est celui qui
  retire les deux motifs. Il n'y a donc rien à excepter, et `ALLOWED` est livrée **vide
  et assertie vide** sur place (doctrine mika#2201, #2323, #2425, #2457, #2759 : une
  allowlist vide s'assure d'être vide, sinon elle se remplit en silence).
- **Portée : un fichier, `crates/mika-cli/src/commands/chat.rs`.** Pas le crate (É2) :
  quatre lecteurs légitimes de `make_llm_provider` vivent ailleurs et sont hors de ce vol.
- **Ce qui arrive quand elle tire.** Le motif cité nomme son remède. Si un futur éditeur
  a besoin d'un provider local dans la TUI, le geste n'est **pas** d'ajouter une entrée à
  `ALLOWED` : c'est de rouvrir la décision D4 avec Vincent, parce qu'un provider construit
  dans la TUI est précisément la duplication que mika#1727 ferme. La garde refuse par
  construction la seule issue qui la contournerait sans discussion.
- **Anti-vacuité.** Témoin et contre-témoin au § D7. Sans eux, un scan qui lirait un
  fichier vide, ou le mauvais fichier, resterait vert pour toujours (mika#2103, #2205,
  #2398).
- **Option (b) — livrer désarmée : refusée.** Il n'y a rien à désarmer : zéro violation.
- **Option (c) — halte et escalade : sans objet.** Rien n'oblige à s'arrêter ici.

---

## 3. Ordre de commit

Un commit par garde, après que la garde est verte **avec son test**, et non en fin de vol.
Les tests ne se reportent jamais à un vol suivant.

### C1 — la chaîne du tour porte l'override (critères 1 et 2)

`crates/mika-cli/src/commands/chat_turn.rs`, et deux lignes de `chat.rs`.

1. Les quatre fonctions de D1 prennent `model_override: Option<&str>` en dernière
   position et le transmettent, jusqu'au 5ᵉ argument de `send_message_to_agent` (l. 98).
2. Les treize sites du § 1 prennent l'argument. Les onze sites de test et les deux
   transmissions internes passent `None` quand l'override n'est pas le sujet du test.
3. La doc l. 59 à 63 est réécrite ; les trois autres fonctions gagnent la phrase de D1.
4. **Les deux bras de `chat.rs` passent `None`**, une ligne chacun, pour que l'arbre
   compile et que la suite soit verte à ce commit. C2 les remplace par le champ du worker.
5. **Test du critère 1** — `mika1727_v3b1_le_modele_choisi_voyage_et_son_absence_aussi` :
   deux moitiés `spawn_mock` dans une fonction. Positive : `metadata[MODEL_OVERRIDE_KEY]`
   vaut la valeur donnée, **à côté** de `metadata[CALLER_SESSION_ID_KEY]`. Négative : le
   même envoi sans override ne porte pas la clé, et porte toujours la clé de session
   (anti-vacuité).
6. **Test du critère 2** — `mika1727_v3b1_le_tour_de_callback_porte_le_modele_choisi` :
   via `base_de_callback` et `tache_de_callback_livree`, `livrer_tour_de_callback_sur_endpoint`
   porte `MODEL_OVERRIDE_KEY` **et** `CALLBACK_TURN_KEY == true`. La seconde assertion est
   ce qui sépare « l'override voyage » de « l'override a écrasé le reste des métadonnées ».

`cargo test -p mika-cli` vert. ≈ 180 lignes.

### C2 — la fonction pure, le bras `SetModel`, et la garde (critères 3 et 4)

1. `override_de_set_model` et sa doc dans `chat_turn.rs` (D2).
2. `spawn_agent_worker` prend l'override initial (`Option<String>`) et **garde le préfixe
   littéral `async fn spawn_agent_worker(`**. `worker_override` (D3). Les bras `Message` et
   `CallbackResult` le passent à la place du `None` de C1.
3. Le bras `SetModel` devient une ligne (D4). `worker_settings` l. 275 et le
   `make_llm_provider()` l. 469 partent. `use mika_common::config::Settings;` **reste**
   (l. 1042).
4. Les trois appels de `spawn_agent_worker` (l. 553, 806, 916) prennent l'argument ;
   les deux respawns passent `None` avec leur commentaire (D6). `run` passe encore `None` —
   C3 lui donne `--model`.
5. **Test du critère 3** — `mika1727_v3b1_loverride_de_set_model_se_lit_et_se_tait` :
   table de cas sur la fonction pure (valeur posée, valeur à espaces autour, chaîne vide,
   chaîne blanche), **puis** la sortie de la fonction donnée à `send_turn_to_endpoint` à
   travers `spawn_mock` et la clé relue sur le fil. C'est la composition de É1, et le test
   la nomme dans son commentaire.
6. **Garde du critère 4** — D7, avec son allowlist vide assertie, son témoin et son
   contre-témoin, dans le `mod tests` de `chat.rs`.

`cargo test -p mika-cli` vert, `cargo clippy --all-targets -p mika-cli` vert. ≈ 165 lignes.

### C3 — le `--model` de départ voyage

`run` passe la valeur de `--model` à `spawn_agent_worker` au lieu de `None` (D5), avec le
commentaire disant pourquoi `ctx.override_model` reste à côté un vol durant.

**Ce commit n'apporte pas de test, et c'est dit plutôt que caché** : `run` exige un
`AppContext` complet et n'est atteignable d'aucun test — la même prémisse qu'en É1. Ce qui
le rend relisable est sa taille (une ligne de passage d'argument) et le fait que tout ce
qu'il alimente est déjà tenu par les tests de C1 et C2. ≈ 10 lignes.

### Fin de pipeline

Dans cet ordre, avant d'ouvrir la PR :

1. `cargo fmt`
2. `cargo clippy --all-targets -p mika-cli` — vert
3. `cargo test -p mika-cli` — vert
4. `bash scripts/verify-egress-manifest.sh --report`
5. `bash scripts/verify-pipeline.sh origin/main`

**Si l'un des deux derniers est rouge, le pilote s'arrête et le dit** (critère 6). Il ne
contourne pas, il ne commente pas la garde.

Vérifications de périmètre, à la main, avant la PR :

- `git diff --stat origin/main...HEAD` ne nomme que `commands/chat_turn.rs`,
  `commands/chat.rs` et le plan.
- `git diff origin/main...HEAD -- crates/mika-cli/src/tui/app.rs` est **vide** (critère 5,
  à l'octet) ; de même pour `init.rs`, `commands/ask.rs`, `tui/commands/handlers.rs`,
  `commands/model.rs`, `commands/provider.rs`, `remote_ask.rs`.
- Aucun `#[allow(` ni `#[expect(` neuf dans le diff (critère 4).
- **Aucun fichier du plan de contrôle** : ni `.github/workflows/`, ni `.claude/`, ni
  `skills/bundled/`, ni `crates/mika-agent/src/well_known_agents.rs`, ni `.mika/`, ni
  aucun chemin de CODEOWNERS, `docs/egress/` compris.

**Vol lean (mika#2694)** : pas de `ce-simplify-code`, pas de `ce-code-review`. Le corps de
PR porte `Code review: delegated to mika-qa (mika#2694)`, `Refs mika#1727`, la conséquence
É4 et `Tracked in: senara-solutions/mika#1727` pour le reste de V3b-2. Le corps s'écrit
dans `pr-body.md` **sous le worktree**, jamais dans `/tmp`, jamais par heredoc (mika#2211) ;
les brouillons vivent sous `.pilot-scratch/` et **ne se suppriment jamais** (mika#2548).

### Halte du pilote sur la borne

Après **chaque** commit, mesurer `git diff --numstat origin/main...HEAD`, plan exclu.

- **Au-dessus de 450 : arrêt.** Pas de commit suivant, pas de PR.
- **Au-dessus de 405 après le premier commit : arrêt aussi** — le reste ne tiendrait pas.

Le dernier message dit alors, mot pour mot :
**« HALTE borne : N lignes pour 450 »**, suivi du compte par commit.

Les commentaires comptent dans la borne et **ne se rabotent pas** pour la tenir. Un
commentaire qui porte un pourquoi reste ; si le code dépasse, le pilote s'arrête selon la
règle ci-dessus.

**Si un hook ou un classifieur refuse un geste**, au pilote ou à un sous-agent, le pilote
**s'arrête et le déclare**, même si le refus paraît un faux positif. Il ne le contourne
jamais par un autre outil : pas de heredoc Bash à la place de `Write`, pas de
reformulation (contre-exemple mika#2869, 2026-10-21).

### Si le code s'écarte de ce plan

La QA juge contre ce plan, pas contre le corps du ticket. Si le pilote doit s'en écarter
sur un point — un fichier, un critère, un nom — **il amende ce plan sur la branche dans un
commit `docs(plans)` avant de pousser le code qui s'en écarte**, en nommant le point et la
raison.

---

## 4. Definition of Done

- [ ] Les quatre fonctions de `chat_turn.rs` portent `model_override: Option<&str>` et le
      transmettent à `send_message_to_agent` ; les treize sites prennent l'argument.
- [ ] La doc de `send_turn_to_spirit` ne dit plus l'inverse du vol, et les quatre docs
      disent que la valeur part brute et que spirit résout et refuse (mika#2304).
- [ ] `override_de_set_model` existe, est pure, et rend `None` sur vide ou blanc.
- [ ] `spawn_agent_worker` prend l'override initial, garde le préfixe littéral
      `async fn spawn_agent_worker(`, et le worker le passe aux bras `Message` et
      `CallbackResult`.
- [ ] Le bras `SetModel` pose l'override par la fonction pure et ne valide plus rien
      localement ; `worker_settings` et le `make_llm_provider` de la l. 469 ont disparu.
- [ ] `run` passe `--model` au worker ; `ctx.override_model` est toujours là.
- [ ] Les deux respawns passent `None`, avec le commentaire qui dit lequel ils sont.
- [ ] Quatre tests livrés dans les mêmes commits que ce qu'ils tiennent, dont une garde de
      scan par fichier à allowlist vide **assertie vide**, avec témoin et contre-témoin.
- [ ] `cargo clippy --all-targets -p mika-cli` et `cargo test -p mika-cli` verts.
- [ ] `bash scripts/verify-egress-manifest.sh --report` et
      `bash scripts/verify-pipeline.sh origin/main` lancés et verts.
- [ ] Seuls `commands/chat_turn.rs` et `commands/chat.rs` changent ; `tui/app.rs` est
      inchangé à l'octet ; aucun fichier du plan de contrôle n'est touché.
- [ ] Aucun `allow` ni `expect` neuf dans le crate.
- [ ] Au plus 450 lignes ajoutées, plan exclu.
- [ ] La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves` — corps et
      commits. Si le gabarit de reprise l'ouvre avec un `Closes`, il est corrigé **avant
      toute revue**, puis `gh pr view <N> --json closingIssuesReferences` rend `[]`.
- [ ] Aucun ticket ouvert par ce vol (gel jusqu'au 01/12) ; le reste est noté
      `Tracked in: senara-solutions/mika#1727`.
- [ ] La conséquence É4 est écrite dans la PR.

---

## 5. Acceptance criteria

Les sept critères de V3b-1, transcrits du corps du ticket.

1. À travers `spawn_mock`, un tour envoyé avec un override porte `MODEL_OVERRIDE_KEY` égal
   à la valeur donnée, à côté de `mika.caller_session_id`. Le même envoi sans override ne
   porte pas la clé. Cette seconde moitié est le test négatif du vol.
2. `livrer_tour_de_callback` porte l'override de la même façon, et `CALLBACK_TURN_KEY`
   reste à `true`.
3. La fonction pure de `SetModel` rend l'override posé, et `None` pour une valeur vide ou
   blanche. Après un `SetModel`, le tour suivant porte l'override.
4. `commands/chat.rs` ne contient plus `worker_settings` ni `make_llm_provider`. `init.rs`,
   `ask.rs` et `handlers.rs` sont inchangés, et aucun `allow` ni `expect` neuf n'apparaît
   dans le crate.
5. Seuls changent `commands/chat_turn.rs` et `commands/chat.rs`. `tui/app.rs` est inchangé,
   à l'octet.
6. `cargo clippy --all-targets -p mika-cli` est vert, de même que `cargo test -p mika-cli`.
   Le pilote lance `bash scripts/verify-egress-manifest.sh --report` et
   `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il trouve l'un des
   deux rouge, il s'arrête et le dit.
7. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y compris si le
   gabarit de reprise l'ouvre.

**Comment le critère 3 est tenu, puisque sa seconde phrase n'est pas mesurable où elle est
écrite** : voir É1 et C2.5. La table de cas tient la première phrase ; la composition —
sortie de la fonction pure donnée à `send_turn_to_endpoint`, clé relue sur le fil — tient
la seconde à un maillon près, une affectation de champ dans le worker, visible en revue.

---

## Taille estimée

| livrable | lignes estimées |
|---|---|
| `chat_turn.rs` — chaîne d'override : 4 signatures, 13 sites, doc réécrite (C1) | 60 |
| test du critère 1 — la clé voyage, et son absence aussi (C1) | 65 |
| test du critère 2 — le tour de callback porte l'override (C1) | 55 |
| `chat_turn.rs` — `override_de_set_model` et sa doc (C2) | 25 |
| `chat.rs` — worker, bras `SetModel`, respawns (C2) | 35 |
| test du critère 3 — table de cas et composition (C2) | 45 |
| garde du critère 4 — scan par fichier, témoin et contre-témoin (C2) | 60 |
| `chat.rs` — le `--model` de départ voyage (C3) | 10 |

Total estimé : 355 lignes

Plafond de tours : 200 (loop-substrate)

Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Les deux unités coïncident ici** : rien n'est livré sous `docs/` ni dans un `CLAUDE.md`
hors ce plan, donc l'unité de l'architecte (hors `docs/`, hors `CLAUDE.md`) et celle de la
borne du vol (`git diff --numstat origin/main...HEAD`, plan exclu) comptent les mêmes 355
lignes. **355 pour 429, et 355 pour 450 : les deux bornes tiennent, tests compris.** Le
groom ne demande donc aucune coupe — et il n'en pratique aucune de lui-même : une coupe
demande le mot de Vincent.

Marge : 95 lignes avant la borne de 450. Après C1 (≈ 180), le seuil de 405 est loin ; la
halte du § 3 reste la règle qui arbitre, pas cette estimation.

Tracked in: senara-solutions/mika#1727
