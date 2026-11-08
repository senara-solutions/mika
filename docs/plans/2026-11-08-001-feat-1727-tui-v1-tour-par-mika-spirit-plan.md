# mika#1727 — V1 : le tour de la TUI passe par mika-spirit

- **Ticket :** senara-solutions/mika#1727 — **vol V1 seul**
- **Base :** `f8a9fcdf3` (branche neuve depuis `main`, `first-push`)
- **Branche :** `feat/1727/tui-v1-tour-par-mika-spirit`
- **Dépôt touché :** `mika` seul.
- **PR :** `Refs mika#1727`, **jamais `Closes`** — le ticket se ferme au merge de V4.

---

## 0 — L'écart avec le texte du ticket, à juger avant le reste

Le § « Découpage en vols bornés » prescrit pour V1 : « l'envoi A2A **en
streaming** de `remote_ask::send_message_to_agent`, et consommer les frames
ToolCallStart/ToolCallResult (#1757) dans `tui/app.rs` ». **Ce plan livre
l'envoi synchrone `message/send` et ne consomme aucune frame.** Trois mesures,
prises sur `f8a9fcdf3`, en sont la cause :

1. **`send_message_to_agent` n'est pas un envoi en streaming.** C'est un
   `message/send` synchrone qui rend un `Task` terminal
   (`remote_ask.rs:465`). Le client SSE n'existe pas : `mika-a2a/src/client.rs`
   ne porte que `send_message` (l. 161) et `get_task` (l. 192), et rien n'y
   parle `message/stream`. Le seul consommateur SSE du dépôt est
   `commands/tasks.rs:397-495` — un analyseur à la main, écrit par mika#1758
   comme **stub de diagnostic**, dont le commentaire dit lui-même « mika#1727
   handles TUI consumption ». Écrire le client de streaming, c'est écrire des
   lignes dans `crates/mika-a2a/`, **hors du périmètre déclaré**
   (`crates/mika-cli/`).
2. **« Le rendu des outils est conservé » est une non-régression, pas une
   fonction.** La TUI ne rend **aucun** appel d'outil en direct aujourd'hui :
   `AgentResponse` (`tui/app.rs:325-336`) n'a que `Reply` et `WorkerCrashed`,
   sans variante de frame. Les deux seules surfaces liées aux outils sont
   `"tool_result" => ChatRole::System` (l. 207) et le repli
   `EMPTY_RESPONSE_FALLBACK` (« Agent responded with tool-use only », ~l. 1335).
   Consommer les frames #1757 serait donc une **amélioration** du rendu, pas sa
   conservation — et le critère du ticket serait tenu sans elle.
3. **La borne par vol ne tient pas les deux.** Client SSE + plomberie de frames
   + rendu des frames + détecteur + tests dépassent les 279 lignes estimées
   (voir § *Taille estimée*). Un vol mort au plafond coûte plus cher qu'un
   découpage — c'est le raisonnement du ticket lui-même.

**Ce que le vol conserve donc, et comment c'est vérifiable** : mika-spirit écrit
dans la **même** base que la TUI (`~/.mika/data/mika.db`, un conteneur une base),
et `app.rs:1076` remet `last_seen_msg_id = max_message_id()` après la révélation
complète de la réponse. Les lignes `tool_result` écrites pendant le tour sont donc
avalées par la marque d'eau **exactement comme aujourd'hui**, que la boucle ait
tourné en processus ou côté serveur. `session_message_to_chat_message` et la
branche `EMPTY_RESPONSE_FALLBACK` continuent de fonctionner sans modification.
C'est cette identité-là que les tests gardent.

**La consommation des frames en direct reste à faire.** *Tracked in:
senara-solutions/mika#1727* — la structure V2/V3/V4 du ticket porte déjà le
reste, et le gel des tickets jusqu'au 01/12 interdit d'en ouvrir un.

---

## 1 — L'état mesuré sur `main`, et une affirmation du corps à corriger

`chat.rs` fait **1 321 lignes** et appelle `agent::run_agent` à **deux** sites :

| site | bras | ce qu'il fait de particulier |
|---|---|---|
| `chat.rs:336` | `AgentRequest::Message` | le tour utilisateur ; `channel_type: "cli"`, `thinking`, `user_images`, `stream_ctx: None` |
| `chat.rs:464` | `AgentRequest::CallbackResult` | `is_callback_turn: true`, cadrage par `agent::build_callback_trigger_context`, et sur `Err` **déréclame** la tâche par `update_task_status(&task_id, &original_status)` |

La population est exactement ces deux-là : `grep run_agent` attrape aussi
`wizard::run_agent_wizard` et un commentaire de
`tests/agent_worker_supervision.rs:22`, qui n'en font pas partie. Le choix des
aiguilles du détecteur (§ 4) en découle.

**Une ligne du corps est périmée.** Le tableau « Vérification à la source » dit
que l'assemblage du `SkillRegistry` est « recopié trois fois dans `chat.rs`
(128-154, 308-325, 386-403) » avec des copies qui « ont déjà dérivé ». Sur
`f8a9fcdf3` les trois sites appellent le **même** `assemble_agent_registry`, avec
le commentaire « Same assembler as the server init and as the two hot reloads
below (mika#2889) — the TUI no longer carries its own copy of the sequence ». La
duplication textuelle est close ; il reste trois *appels* du même assembleur, ce
que V3 retirera du chemin TUI. Ce plan ne reprend pas l'affirmation, et aucun de
ses livrables ne la suppose.

Le patron à imiter existe déjà et il est éprouvé : `commands/ask.rs:467-505`
bâtit `{spirit_url}/a2a/{agent_name}` et appelle `send_message_to_agent`, avec
`wrap_send_error` (l. 91-98) pour aplatir la chaîne `anyhow` en `{err:#}` et
ré-attacher `TransportClass` (mika#2278). `send_message_to_agent` apporte déjà,
sans une ligne neuve : le plancher d'enveloppe per-agent
(`budget_floor::attested_envelope_secs` + `resolve_send_timeout_with_floor`,
mika#2759), le `context_id` frappé **avant** l'envoi et la récupération par
`recover_by_context` (mika#2036), la classification des échecs, et
`render_task_parts` qui fait d'un `Task` illisible un **échec** et jamais une
réponse vide (mika#2270).

---

## 2 — Les points de conception

### D1 — Un seul site d'envoi, dans un module neuf : `commands/chat_turn.rs`

Le worker de la TUI vit dans `spawn_agent_worker`, qui exige un `AppContext`
complet (base, provider, MCP, `TaskEngine`) : rien de ce bras n'est atteignable
depuis un test. Le vol extrait donc l'envoi dans un module au périmètre, avec ses
tests en ligne :

```rust
pub(crate) struct TurnReply {
    pub content: String,
    pub input_tokens: Option<u32>,
}

/// L'unique site par lequel un tour de la TUI atteint mika-spirit.
pub(crate) async fn send_turn_to_spirit(
    text: &str,
    agent_name: &str,
    session_id: &str,
) -> anyhow::Result<TurnReply>
```

Il bâtit `{spirit_url}/a2a/{agent_name}`, appelle
`remote_ask::send_message_to_agent(text, &endpoint, Some(session_id), &[], None, false)`,
rend le texte par `remote_ask::render_task_parts` et les jetons par
`mika_a2a::params::attested_run_usage`. **Pas de second décodeur** : l'aiguille
`"mika.run_usage"` n'apparaît nulle part ici, et le lecteur partagé est appelé par
son nom — ce que `mika1883_both_client_surfaces_read_the_one_reader` exige déjà des
deux portes de `mika ask`, et dont l'assertion est « au moins ces deux surfaces »,
donc une troisième par le même lecteur est conforme.

**`session_id` est ce qui fait tenir l'historique.** La TUI garde sa session
locale (V2 la déplace) et la passe en `caller_session_id` (`mika.caller_session_id`,
mika#2070) : spirit l'adopte comme session de boucle puisqu'il possède déjà la
ligne, dans la même base. Les messages du tour atterrissent donc sur la session
que la TUI affiche, et non sur une session parallèle.

### D2 — Le bras `CallbackResult` garde `run_agent`, et c'est V2 qui le prend

`server/a2a.rs:343` écrit `is_callback_turn: false` en dur : **la porte A2A ne
sait pas exprimer un tour de callback.** Router ce bras par `message/send`
enverrait le cadrage de `build_callback_trigger_context` comme un message
*utilisateur*, ce qui (a) le ferait persister en rôle `user` et l'afficherait
comme une saisie de l'opérateur, et (b) perdrait le contexte de cadrage et de
mise à la terre que les sites `agent_loop/mod.rs:6083, 6182, 6323, 6869` ne
posent que sur un tour de callback. Le réparer demande une clé de métadonnée
`mika.is_callback_turn` dans `crates/mika-a2a/` **et** sa lecture dans
`crates/mika-agent/src/server/a2a.rs` — deux crates hors périmètre, et un
élargissement du contrat A2A (n'importe quel appelant pourrait réclamer le
cadrage de callback) qui mérite sa propre revue.

Or le **mécanisme** de callback — `TaskEngine` local, `spawn_tick_loop`,
`startup_recovery`, poll et réclamation dans `app.rs:1576`, déréclamation sur
erreur — est, mot pour mot, le sujet de V2. Couper le tour du mécanisme qui
l'entoure livrerait une moitié dont la justesse reposerait sur une clé de fil que
V2 effacerait. **Le bras reste local, et il est l'unique exception du
détecteur** (§ 4). *Tracked in: senara-solutions/mika#1727*

### D3 — Les inerties sont déclarées à voix haute, jamais tues

Trois entrées de la TUI cessent d'atteindre le tour. Aucune ne devient
silencieuse : c'est le geste `cli_skill_flag_inert` de mika#1883, et la doctrine
« absent n'est pas zéro » de mika#2331.

| entrée | pourquoi elle n'atteint plus rien | ce que le vol affiche |
|---|---|---|
| `/think` → `thinking_budget` | aucune clé de fil ne la porte, et **le serveur ne lit pas `thinking_level`** : `config_keys.rs` le valide, personne ne le convertit en `ThinkingConfig` côté serveur | à chaque tour qui porte un budget, un message `System` nommant l'inertie et `/think off` comme geste qui l'éteint |
| image collée (Ctrl+V) | `build_send_params` (`remote_ask.rs:294`) ne bâtit que `Part::Text` — aucune part image | à chaque tour qui porte une image, un message `System` nommant le nombre d'images abandonnées |
| bloc `thinking:` de la réponse | le `Task` A2A ne porte pas de part de pensée ; `AgentResponse::Reply.thinking` vaut désormais toujours `None` | dit dans le même avis que `/think` : le niveau est affiché en pied mais ne produit plus de bloc |

**Un avis par tour, pas un par session.** Bruyant, oui — et c'est le but : un
réglage qui ne fait plus ce qu'il annonce doit se voir à chaque fois qu'on
l'exerce, et l'avis nomme le geste qui le fait taire. C'est la forme de mika#1883,
qui avertit à chaque invocation.

**Ce qui n'est *pas* inerte, contrairement à ce qu'on pourrait croire.** Le
rechargement à chaud des skills reste vivant : `skills_dirty` est aussi cloné
vers le `TaskDispatcher` local (`chat.rs:203`), qui reste en place en V1, et le
bras callback continue de le lever. Ce qui disparaît est étroit et nommé comme
tel : une skill installée **par un appel d'outil pendant un tour utilisateur** ne
rafraîchit plus l'autocomplétion. V3 (« L'autocomplétion des skills lit
`GET /api/v1/skills` ») est le correctif.

### D4 — Un spirit injoignable est dit, et la cause est nommée

Deux surfaces, aucune invention :

- **Au tour** : l'échec de `send_turn_to_spirit` est aplati à la façon de
  `ask.rs::wrap_send_error` — `format!("le tour de la TUI vers {endpoint} a
  échoué : {err:#}")` — et rendu en `AgentResponse::Reply { is_error: true }`.
  La chaîne porte déjà le statut HTTP, l'échec de transport, la transition d'état
  A2A et l'erreur serveur. L'indice trompeur « (is it running?) » est écarté,
  comme `ask.rs` l'écarte.
- **Au démarrage** : `run` appelle déjà `dashboard::is_dashboard_running()`. Le
  vol lit `dashboard::query_dashboard_status()`, dont le `None` dit « spirit ne
  répond pas », et pousse un message `System` avant le premier tour. C'est la
  disposition (b) de l'AC4 d'origine assumée : **la TUI exige désormais un
  mika-spirit vivant**, et elle le dit avant que l'opérateur ait tapé.

### D5 — Les clones devenus morts partent dans le même diff

Retirer le bloc `AgentParams` du bras `Message` oriente vers `unused_variables`
plusieurs clones de `spawn_agent_worker` : `worker_tools`, `worker_embedding`,
`worker_brave_key`, `worker_github_token`, `worker_sender`, `worker_gateway_url`,
`worker_internal_token`, `worker_github_app`, `cli_tier`, `cli_deployment`.
**Chacun est vérifié contre le bras callback et contre le `TaskDispatcher` avant
d'être retiré** : `worker_llm` et `worker_settings` restent (mutés par
`SetModel`), `worker_mcp` reste (`mcp.shutdown()`), `worker_home` et
`worker_dirty` restent (rechargement), `worker_skills` reste (bras callback), et
le `TaskDispatcher` garde ses propres clones des registres. Le critère est
`cargo clippy --all-targets -- -D warnings`, pas une lecture à l'œil.

### D6 — `remote_ask.rs` est lu, pas modifié ; `docs/egress/` n'est pas touché

Le ticket nomme `remote_ask.rs` au périmètre. Le vol le laisse **inchangé** :
`send_message_to_agent` convient tel quel, et toute signature élargie se
paierait sur les deux portes de `mika ask`.

Conséquence à nommer plutôt qu'à découvrir : le sink `a2a-client` du manifeste
d'egress (`docs/egress/egress-manifest.toml:293-303`) garde son `client_site`
(`crates/mika-a2a/src/client.rs`) et son `call_site` — **aucun sink neuf, donc
aucune entrée à ajouter**, et `scripts/verify-egress-manifest.sh` est satisfait
par construction. Mais sa note dit « Depuis mika#1727 `mika ask` est un client
mince : c'est par ici que passe tout tour d'agent lancé en ligne de commande », et
V1 élargit ce « tout » aux tours de la TUI. Corriger la prose serait éditer
`docs/egress/`, **un chemin de CODEOWNERS que ce vol s'interdit**. L'élargissement
est donc consigné dans le corps de la PR et dans un commentaire sur #1727, et il
n'est pas écrit dans le manifeste.

---

## 3 — Les tests

Tous en ligne dans `commands/chat_turn.rs`, module `tests`, sur le patron déjà
en place dans `tests/remote_ask_integration.rs` : un `Router` axum
(`axum` est dev-dep de ce crate) sur `TcpListener::bind("127.0.0.1:0")`, une
`Arc<Mutex<Capture>>` qui retient le dernier corps, et la fixture JSON-RPC
`ok_task_with_text`.

- **T1 — le tour part sur le fil.** Un mock A2A répond un `Task` complété ;
  `send_turn_to_spirit` rend son texte. Le corps capturé est un `message/send`
  dont le `Part::Text` porte la saisie.
- **T2 — la session voyage.** Le corps capturé porte le `session_id` passé, sous
  `mika.caller_session_id`, lu par le lecteur de `mika_a2a::params` et non par
  une aiguille littérale.
- **T3 — un spirit injoignable est un échec nommé.** Endpoint réservé par
  `mika_common::dead_endpoint::DeadEndpoint::reserve()`, garde tenue pendant tout
  l'appel (jamais un `bind(:0)` relâché — mika#2569). L'erreur rendue nomme
  l'endpoint et conserve la chaîne sous-jacente ; `is_transport_failure` la
  reconnaît.
- **T4 — un `Task` illisible est un échec, pas une réponse vide.** Un `Task`
  complété sans artefact, sans historique et sans `status.message` fait échouer
  l'appel (mika#2270), et le contenu rendu n'est jamais la chaîne vide.
- **T5 — les jetons absents ne deviennent pas zéro.** Un `Task` sans
  `mika.run_usage` rend `input_tokens: None` ; un `Task` qui le porte rend la
  valeur attestée. Contrôle négatif du motif mika#2331.
- **T6 à T8 — le détecteur structurel et son anti-vacuité** (§ 4).

Ce que les tests **ne** prétendent pas couvrir, dit ici plutôt que découvert en
revue : le worker lui-même reste hors test (il exige un `AppContext`), donc
« le bras `Message` appelle bien `send_turn_to_spirit` » est tenu par le
détecteur T6, pas par un test de comportement. C'est précisément pourquoi T7
existe.

---

## 4 — Fire-Disposition

Ce plan livre un détecteur structurel. Disposition : **option (a) — exception
nommée en allowlist**, avec **exactement une** entrée, et une seconde garde qui
empêche cette entrée de couvrir plus que son dû.

```rust
/// **AC1** — aucun tour utilisateur de la TUI ne fait tourner la boucle ici.
#[test]
fn mika1727_v1_le_tour_de_la_tui_ne_fait_plus_tourner_la_boucle() {
    // Le bras `CallbackResult` de `chat.rs` est le dernier appel local du crate.
    // Il part au vol V2 avec le TaskEngine et le poll des callbacks : la porte
    // A2A écrit `is_callback_turn: false` en dur (server/a2a.rs:343), et le
    // réparer demanderait deux crates hors périmètre (§ D2).
    // Tracked in: senara-solutions/mika#1727
    const ALLOWED: &[&str] = &["commands/chat.rs"];
    const NEEDLES: &[&str] = &["agent::run_agent(", "AgentParams"];
    // ... ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR")),
    //     strip_prefix(scanner.src_root()), saute ALLOWED et les lignes "//",
    //     collecte les hits.
}
```

**L'allowlist seule serait un faux vert, et c'est T7 qui le refuse.** Les deux
bras vivent dans le *même fichier* : une entrée au niveau du fichier ne verrait
pas le tour utilisateur régresser à côté du bras toléré.

```rust
/// **AC1, seconde moitié** — le site toléré n'a qu'un seul appel local.
#[test]
fn mika1727_v1_le_site_tolere_na_quun_seul_appel_local() {
    // Deux `agent::run_agent(` dans `commands/chat.rs` = le tour utilisateur
    // est revenu en processus. Le remède est de le router par
    // `chat_turn::send_turn_to_spirit`, JAMAIS d'incrémenter ce compte.
}
```

```rust
/// Anti-vacuité : le balayage a réellement vu les deux fichiers du vol.
#[test]
fn mika1727_v1_le_scan_nest_pas_vide() {
    // files().len() > 10, et any(ends_with("commands/chat.rs")),
    // et any(ends_with("commands/chat_turn.rs")).
}
```

**Pourquoi l'allowlist n'est pas vide, et pourquoi c'est la bonne forme ici.**
La forme la plus forte du dépôt (mika#2425 / 2457 / 2759) est l'allowlist vide,
parce qu'elle est exacte : il n'y a rien à excepter. Ici il reste **une**
exception réelle et datée, et la prétendre absente exigerait d'écrire la clé de
fil `mika.is_callback_turn` hors périmètre dans ce vol. L'entrée nomme son bras,
sa raison et son vol de sortie ; elle **expire** au vol V2, qui supprime le bras
avec le `TaskEngine` local. Et le couple T6+T7 fait que l'exception ne protège
rien de plus que ce qu'elle nomme — ce qui est exactement le reproche que
mika#2309 adresse aux allowlists.

**Le verrou définitif est V4**, pas ce détecteur : `agent_loop` et
`task_engine` y passent `pub(crate)` dans `crates/mika-agent/src/lib.rs`, et
l'appel local cesse de compiler. Le détecteur de V1 est la garde de l'intervalle,
et il est utile pendant tout cet intervalle : `agent::run_agent` et `AgentParams`
restent `pub`, et `mika-agent` reste une dépendance de `mika-cli` jusqu'à V4 —
donc la régression est exprimable, et le scan la verrait.

---

## 5 — Definition of Done

- [ ] `commands/chat_turn.rs` existe, déclaré dans `commands/mod.rs`, et porte
      l'unique `send_turn_to_spirit` du crate.
- [ ] Le bras `AgentRequest::Message` de `chat.rs` n'appelle plus
      `agent::run_agent` et ne construit plus d'`AgentParams` ; il appelle
      `send_turn_to_spirit`.
- [ ] Le bras `AgentRequest::CallbackResult` est **inchangé**, y compris la
      déréclamation `update_task_status(&task_id, &original_status)` sur `Err`.
- [ ] L'import de tête devient `use mika_agent::agent;` — `AgentParams` et
      `check_onboarding` ne sont plus importés ; `agent::build_callback_trigger_context`
      reste atteignable.
- [ ] Les trois inerties (D3) produisent un message `System` visible, chacune
      nommant le geste qui l'éteint. Aucune n'est silencieuse.
- [ ] Un spirit injoignable est dit au démarrage **et** au tour, chaîne aplatie,
      endpoint nommé.
- [ ] `input_tokens` vient de `attested_run_usage` ; absent reste `None`, jamais `0`.
- [ ] Aucun clone orphelin : `cargo clippy --all-targets -- -D warnings` vert
      sans `#[allow]` neuf.
- [ ] T1 à T8 écrits et verts, sans `#[ignore]`.
- [ ] `cargo test`, `cargo clippy --all-targets -- -D warnings`,
      `cargo fmt --check`, `make verify-bundled-skills`,
      `bash scripts/verify-pipeline.sh origin/main` verts.
- [ ] `crates/mika-a2a/`, `crates/mika-agent/` et `remote_ask.rs` **non
      modifiés**. Aucun fichier du plan de contrôle (`.github/workflows/`,
      `.claude/`, `skills/bundled/`, `well_known_agents.rs`, `.mika/`), aucun
      chemin de CODEOWNERS — `docs/egress/` compris.
- [ ] Le corps de la PR porte `Refs mika#1727` (jamais `Closes`),
      `Code review: delegated to mika-qa (mika#2694)`, l'écart du § 0 avec son
      `Tracked in: senara-solutions/mika#1727`, et l'élargissement de la note du
      sink `a2a-client` (D6). **Aucun ticket ouvert** (gel jusqu'au 01/12).

---

## Acceptance criteria

- **AC1.** Un tour utilisateur de la TUI n'appelle plus `agent::run_agent` dans
  son processus. Un détecteur structurel rougit sur l'appel local : zéro
  occurrence de `agent::run_agent(` et d'`AgentParams` dans la production de
  `crates/mika-cli/` hors l'unique entrée d'allowlist `commands/chat.rs`, **et**
  ce fichier n'en compte qu'une seule — le bras callback, qui part au vol V2. Le
  balayage atteste avoir vu `commands/chat.rs` et `commands/chat_turn.rs`.
- **AC2.** Le tour part réellement sur le fil, et il porte la session. Un test
  sur mock A2A capture un `message/send` dont le texte est la saisie et dont la
  métadonnée porte le `caller_session_id` de la session que la TUI affiche —
  lu par `mika_a2a::params`, jamais par une aiguille littérale.
- **AC3.** Le rendu des outils est conservé, et c'est une non-régression
  vérifiable : `session_message_to_chat_message`, la ligne
  `"tool_result" => ChatRole::System` et la branche `EMPTY_RESPONSE_FALLBACK`
  sont inchangées, et la remise à zéro de la marque d'eau (`app.rs:1076`) avale
  les lignes du tour comme avant. Le vol n'ajoute aucune variante à
  `AgentResponse` et ne consomme aucune frame #1757.
- **AC4.** Rien ne devient silencieux. Les trois entrées de D3 produisent chacune
  un avis visible nommant l'inertie et le geste qui l'éteint ; un `Task` sans
  `mika.run_usage` rend `None` et non `0` ; un `Task` illisible est un échec et
  non une réponse vide.
- **AC5.** La TUI exige un mika-spirit vivant, et le dit. Un endpoint mort
  (`DeadEndpoint::reserve()`) produit une erreur qui nomme l'endpoint, conserve
  la chaîne sous-jacente et est reconnue par `is_transport_failure` ; le démarrage
  sans spirit affiche son avis avant le premier tour.
- **AC6.** Le périmètre est tenu : seuls `crates/mika-cli/src/commands/chat.rs`,
  `crates/mika-cli/src/commands/chat_turn.rs` (neuf),
  `crates/mika-cli/src/commands/mod.rs`, `crates/mika-cli/src/tui/app.rs` et
  `crates/mika-cli/CLAUDE.md` sont modifiés. `crates/mika-agent/` n'est pas
  touché, donc rien à justifier de ce côté.

### Correspondance AC → livrable

| AC | livrable | garde |
|---|---|---|
| AC1 | bras `Message` réécrit, import de tête, `chat_turn.rs` | T6, T7, T8 |
| AC2 | `send_turn_to_spirit` + `caller_session_id` | T1, T2 |
| AC3 | — (non-régression : aucune ligne touchée sur ces surfaces) | `cargo test -p mika-cli` existant |
| AC4 | avis d'inertie (D3), `attested_run_usage`, `render_task_parts` | T4, T5 |
| AC5 | diagnostic de tour + avis de démarrage (D4) | T3 |
| AC6 | — (exigence sur le diff) | `git diff --stat` en fin de pipeline |

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-cli/src/commands/chat_turn.rs` — `send_turn_to_spirit`, `TurnReply`, endpoint, aplatissement de l'erreur | 60 |
| `crates/mika-cli/src/commands/chat.rs` — bras `Message` réécrit, import de tête, clones orphelins retirés ; `commands/mod.rs` | 45 |
| `crates/mika-cli/src/tui/app.rs` — avis d'inertie (`/think`, images) et avis de démarrage | 25 |
| détecteur structurel + anti-vacuité (`chat_turn.rs`, module `tests` — T6 à T8) | 55 |
| tests de comportement sur mock A2A (`chat_turn.rs`, module `tests` — T1 à T5) | 85 |

Total estimé : 270 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient, et ce qui a été coupé pour qu'il tienne.** 270 <
279, et très loin du seuil par PR de 1 000. Ce qui rentre dans la borne est un
seul bras de `match` dérouté vers un module de 60 lignes, plus ses gardes. Ce qui
a été coupé est nommé au § 0 et non caché : le client SSE de `mika-a2a`
(hors périmètre), la plomberie et le rendu des frames #1757, et le bras callback
(§ D2, sujet de V2). Découper plus bas n'a pas de sens : le détecteur sans le
déroutement rougirait sur `main`, et le déroutement sans le détecteur livrerait
l'AC1 sans sa garde.

---

## Ordre de commit

1. `feat(1727)` — `commands/chat_turn.rs` et sa déclaration dans
   `commands/mod.rs`, avec T1 à T5. **Plancher :** compile,
   `cargo test -p mika-cli` vert. À ce point le nouveau site existe et est
   éprouvé, mais personne ne l'appelle encore — aucun comportement n'a changé.
2. `feat(1727)` — le bras `Message` de `chat.rs` passe par
   `send_turn_to_spirit` ; import de tête, clones orphelins, avis d'inertie et
   avis de démarrage. **Plancher :** `cargo clippy --all-targets -- -D warnings`
   vert sans `#[allow]` neuf.
3. `test(1727)` — le détecteur T6, sa seconde moitié T7 et son anti-vacuité T8.
   **Plancher :** les trois verts sur ce diff, et T6 rouge si l'on rétablit
   l'appel du bras `Message` (vérifié à la main avant de commiter).
4. Fin de pipeline : `cargo test` complet, `cargo fmt --check`,
   `make verify-bundled-skills`,
   `bash scripts/verify-pipeline.sh origin/main`, mise à jour de
   `crates/mika-cli/CLAUDE.md` (la TUI exige mika-spirit ; les trois inerties),
   puis la PR en `Refs mika#1727`.

Les commits 1 à 3 atterrissent dans la **même** PR : le premier seul laisse du
code mort, et le second sans le troisième livre l'AC1 sans la garde qui la tient.
