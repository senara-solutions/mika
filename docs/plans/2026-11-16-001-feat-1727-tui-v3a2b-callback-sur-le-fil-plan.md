# mika#1727 — vol V3a-2b : le tour de callback part sur le fil

- **Ticket** : senara-solutions/mika#1727
- **Branche** : `feat/1727/v3a2b-le-tour-de-callback-part-sur-le-fil`
- **Base mesurée** : `main` @ `567c78e4` (merge de V3a-2a, mika#3093)
- **Label** : `loop-substrate`
- **La PR porte `Refs mika#1727`**, jamais `Closes`, `Fixes` ni `Resolves`.

---

## 0. Pourquoi ce vol tient en un seul vol

Le fil sait déjà dire « ceci est un tour de callback », **des deux côtés**. La
porte A2A de mika-spirit lit `mika.callback_turn` en booléen strict depuis V3a-1
(mika#3092), et le client sait poser la clé depuis V3a-2a (mika#3093,
`567c78e4`) — `remote_ask::build_send_params` la pose, et `callback_turn: bool`
traverse `chat_turn::send_turn_to_spirit`. Il ne manque **qu'un appelant qui
passe `true`**.

Ce vol est donc, en production, un remplacement d'appel : le bras
`AgentRequest::CallbackResult` de `chat.rs` (l. 441 à 559) cesse de faire
tourner `agent::run_agent` dans le processus de la TUI et appelle
`chat_turn::livrer_tour_de_callback`. **C'est le dernier appel local de la
boucle dans `mika-cli`** : l'entrée d'allowlist qui le tolérait disparaît avec
lui, et la garde structurelle du vol V1 passe à une allowlist vide et pinnée
vide.

Ce qui fait le volume n'est pas la bascule, qui est courte. C'est ce qu'elle
libère : onze liaisons `worker_*` perdent leur dernier lecteur dans le même
geste, et clippy refuse le retrait de l'une sans l'autre — d'où un premier
commit nécessairement gros, et une halte de pilote calibrée sur lui (§ 3).

---

## 1. Ce qui est mesuré sur main (@ `567c78e4`)

Toutes les lignes ci-dessous ont été lues sur la branche, pas transcrites du
corps du ticket.

| Fait | Site mesuré |
|---|---|
| Le bras `CallbackResult` écrit le `tool_result` sur la session | `commands/chat.rs` l. 449 à 465 (`save_message_with_metadata`) |
| …puis bâtit le cadrage | `chat.rs` l. 467 à 475 (`agent::build_callback_trigger_context`, `is_failed = original_status == "failed"`) |
| …puis fait tourner la boucle **en processus** avec `is_callback_turn: true` | `chat.rs` l. 481 (`agent::check_onboarding`) et l. 482 à 523 (`agent::run_agent(&agent::AgentParams{…})`) |
| …et sur erreur, remet la tâche dans son statut d'origine, en journalisant l'échec de cette remise sans le propager | `chat.rs` l. 533 à 554 (`update_task_status`, `tracing::warn!("failed to unclaim callback task after agent error")`, puis `content: format!("Failed to process callback result: {e}")`, `is_error: true`) |
| L'envoi sur le fil existe, avec son paramètre, et personne ne le pose à `true` | `chat_turn.rs` l. 70 à 78 (`send_turn_to_spirit`), l. 85 à 124 (`send_turn_to_endpoint`), appelé une seule fois en production : `chat.rs` l. 386 à 392, à `false` |
| `TurnReply` porte exactement ce dont le bras a besoin | `chat_turn.rs` l. 29 à 32 : `content: String`, `input_tokens: Option<u32>` |
| La garde V1 tolère un fichier | `chat_turn.rs` l. 589 : `const ALLOWED: &[&str] = &["commands/chat.rs"];`, aiguilles l. 590 : `["agent::run_agent(", "AgentParams"]` |
| …et sa seconde moitié compte exactement un appel local dans ce fichier | `chat_turn.rs` l. 633 à 654 (`mika1727_v1_le_site_tolere_na_quun_seul_appel_local`) |
| L'anti-vacuité du balayage est séparée et ne bouge pas | `chat_turn.rs` l. 662 à 684 (`mika1727_v1_le_scan_nest_pas_vide`) |
| La forme à recopier pour pinner une allowlist vide | `chat_turn.rs` l. 686 à 692 (`const V2A_ALLOWED: &[&str] = &[];`, au **niveau module** et la doc dit pourquoi) + l. 791 à 799 (`mika1727_v2a_lallowlist_est_livree_vide`) |
| Le banc d'essai du module capture le corps envoyé | `chat_turn.rs` l. 243 à 269 (`spawn_mock`, route `/a2a/{agent}`, `last_body`), l. 271 à 289 (`ok_task_with_text`) |
| Le modèle de test à deux moitiés dans une seule fonction, clé lue par la constante | `chat_turn.rs` l. 457 à 514 (`mika1727_v3a2a_la_cle_de_callback_voyage_et_son_absence_aussi`) |
| L'endpoint mort se réserve, il ne se libère pas | `DeadEndpoint` déjà importé : `chat_turn.rs` l. 224 |
| `make_message_sender` n'a qu'un appelant | `init.rs` l. 211 à 247, appelé une seule fois : `chat.rs` l. 268 à 273 |
| …et `http_client` ne sert qu'à lui | `chat.rs` l. 194 (paramètre), l. 271 (seul usage), l. 654 (construction), appelants l. 664, 917, 1027 |
| `AppContext.llm` et `DbContext.github_app` n'ont pas d'autre lecteur dans le crate | `grep -rn "\.llm\b\|\.github_app\b" crates/mika-cli/src` rend `chat.rs:237`, `chat.rs:310`, `chat.rs:325` et rien d'autre (les occurrences de `commands/skills.rs` portent sur `entry.manifest.llm`, pas sur le champ) |
| Le gabarit d'une tâche de callback livrable | `tui/app.rs` l. 2439 à 2480 (`couture_dun_callback_livrable` : `NewTask` `long_running:run_claude_pilot` / `trigger_type: "callback"` / `action_type: "resume_agent"`, puis `update_task_status`) |
| La base en mémoire du crate | `chat.rs` l. 1437 : `AsyncDatabase::new(Database::open_in_memory().expect("une base en mémoire"))` |
| Lire un statut de tâche | `mika_agent::async_db::AsyncDatabase::get_task(&id) -> Result<Option<Task>>` (`async_db.rs` l. 442) |
| `AgentResponse` est déjà `pub` | `tui/app.rs` l. 325 à 343 — aucune ligne de visibilité à ouvrir |

### Sept écarts entre la tête du ticket et la mesure

La QA juge contre **ce plan**, et un plan qui recopie un inventaire faux fait
rougir un critère sans que le code ait tort. Les écarts sont donc nommés ici,
avec ce que le vol fait dans chaque cas.

**É1 — la doc de la garde V1 est fausse sur un autre crate.** `chat_turn.rs`
l. 580 à 586 affirme que le bras « part au vol V2 » et que « la porte A2A écrit
`is_callback_turn: false` en dur (`server/a2a.rs:343`) ». Les deux sont faux
depuis V3a-1 : la porte lit `mika.callback_turn`. Réécrire cette doc n'est donc
pas cosmétique, c'est retirer une affirmation fausse sur `crates/mika-agent/`
qu'un lecteur prendrait pour l'état du jour. Le vol la réécrit (D7).

**É2 — `CLAUDE.md` et la tête du ticket se contredisent sur le poll.**
`crates/mika-cli/CLAUDE.md` l. 390-391 dit que le bras « part avec le poll des
callbacks au vol V3a-2b ». La tête du ticket dit que `tui/app.rs` reste intact,
**poll compris**. C'est `CLAUDE.md` qui a tort ; le plan suit la tête, et la
phrase est corrigée (D7). `tui/app.rs` est inchangé à l'octet (critère 6).

**É3 — `SetModel` valide déjà.** La tête décrit « `SetModel` valide le modèle
par `make_llm_provider()` et ne met à jour `worker_settings` que si la
validation passe » comme un changement. Sur main (l. 578 à 581) c'est **déjà**
la forme. Ce qui change réellement : `worker_llm` perd son dernier lecteur
(l. 486), donc la liaison l. 310 et l'affectation l. 579 partent, et
`make_llm_provider()` devient une validation **pure** — `if updated_settings
.make_llm_provider().is_ok()`. Le comportement observable de `/model` est
inchangé : un modèle invalide ne met toujours pas à jour les réglages.

**É4 — trois liaisons `worker_*` que la tête ne nomme pas partent aussi.**
`worker_gateway_url` (l. 319), `worker_internal_token` (l. 320 à 324) et
`worker_global_home` (l. 329) n'ont, eux aussi, d'autre lecteur que le bras
(l. 501, 502, 507). Elles sont couvertes par « les `worker_*` correspondants
(l. 310 à 329) » mais leur amont n'est pas une variable locale : c'est
`ctx.settings` et `ctx.global_home`, qui gardent des lecteurs ailleurs
(`ctx.settings` l. 311, 319, 595 ; `ctx.global_home` via `worker._ctx` l. 677).
Aucun `#[expect]` n'est donc dû pour ces deux champs.

**É5 — deux imports de tête de `chat.rs` perdent leurs derniers lecteurs.**
`use secrecy::ExposeSecret;` (l. 9) n'est lu qu'en l. 279 et l. 324, qui partent
toutes deux. `use mika_agent::tools;` (l. 35) n'est lu qu'en l. 232-233, qui
part avec `tool_registry`. Les deux lignes partent. En revanche
`use mika_agent::agent;` (l. 29) **reste** : `build_callback_trigger_context`
reste dans le bras.

**É6 — l'import de `init.rs` qui part est la ligne entière.** La tête dit « et
l'import `GatewayMessageSender` ». Mesure : `init.rs` l. 4 est
`use mika_agent::messaging::{GatewayMessageSender, MessageSender};`, et
`MessageSender` n'a d'autre lecteur que le type de retour de
`make_message_sender` (l. 216). C'est donc la ligne complète qui part, pas un
seul nom.

**É7 — cinq liaisons que le bras lit restent, et c'est voulu.**
`worker_skills`, `worker_home`, `worker_dirty`, `worker_mcp` et
`worker_settings` sont lues par le bras **et ailleurs** : rechargement à chaud
des skills (l. 345 à 353, l. 396 à 404, l. 413), `worker_mcp.take()` du
`shutdown` (l. 590), `worker_settings.clone()` de `SetModel` (l. 563). Elles
restent, donc `connect_mcp`, `assemble_agent_registry` et `skill_registry`
restent. C'est cohérent avec « le retrait du provider local et de MCP » laissé
au vol V3, hors de ce périmètre.

---

## 2. Décisions

### D1 — `livrer_tour_de_callback` vit dans `chat_turn.rs`, et rend un `AgentResponse`

Le bras est inatteignable depuis un test : il vit dans la `tokio::spawn` de
`chat::spawn_agent_worker`, qui exige un `AppContext` complet. C'est la même
raison qui a fait naître `chat_turn.rs` au vol V1, et la même réponse : la
logique du bras descend dans le module, au périmètre, avec ses tests en ligne.
Ce qui reste dans `chat.rs` est l'écriture du `tool_result` et le cadrage —
inchangés — plus un appel.

La fonction rend un `AgentResponse::Reply` et non un `Result` : le bras
d'aujourd'hui convertit déjà **les deux issues** en `Reply` (l. 525 à 555), et
l'erreur n'est pas propagée mais affichée. Rendre un `Result` obligerait
`chat.rs` à reconstruire la moitié erreur, c'est-à-dire à garder ici la remise
du statut — exactement ce qui doit devenir testable. `AgentResponse` est déjà
`pub` (`tui/app.rs` l. 325) : aucune ligne de visibilité n'est ouverte.

### D2 — deux fonctions, comme `send_turn_to_spirit` / `send_turn_to_endpoint`

```rust
pub(crate) async fn livrer_tour_de_callback(
    db: &AsyncDatabase,
    agent_name: &str,
    session_id: &str,
    cadrage: &str,
    task_id: &str,
    statut_dorigine: &str,
) -> AgentResponse
```

résout l'endpoint par `spirit_turn_endpoint(agent_name)` et délègue à

```rust
async fn livrer_tour_de_callback_sur_endpoint(
    db: &AsyncDatabase,
    endpoint: &str,
    session_id: &str,
    cadrage: &str,
    task_id: &str,
    statut_dorigine: &str,
) -> AgentResponse
```

Même découpe, même raison qu'en V1 : les tests visent un serveur factice, et
l'alternative serait de muter `MIKA_SPIRIT_URL`, un état de processus que les
tests `cargo` parallèles se disputent. Les trois tests du vol visent la seconde.

Le corps délègue l'envoi à `send_turn_to_endpoint(cadrage, endpoint,
session_id, true)` — **c'est le seul site du crate qui passe `true`**, et il n'y
a pas de second chemin d'envoi : la chaîne V1/V3a-2a est réutilisée telle
quelle, y compris sa lecture de `attested_run_usage` (mika#1883) et son refus
d'aplatir un rendu illisible en chaîne vide (mika#2270).

### D3 — la réponse de succès perd `thinking`, et le dit

Sur succès : `content` = le texte rendu, `input_tokens` = ce que le serveur a
attesté, `thinking: None`, `updated_skills: None`, `is_error: false`.

`thinking: None` est une perte réelle et mesurée : un `Task` A2A ne porte pas de
part de pensée, et le bloc `thinking:` du bras local disparaît donc. Ce n'est
pas une régression nouvelle — `crates/mika-cli/CLAUDE.md` la documente déjà pour
le tour utilisateur depuis V1, dans le tableau des trois entrées inertes. Le
tour de callback rejoint ce régime. Aucun avis d'inertie n'est émis ici :
`inertia_notice` parle des entrées **que l'opérateur exerce** (`/think`, les
pièces jointes), et personne ne tape un tour de callback.

`updated_skills: None` est l'état d'aujourd'hui : le bras local ne renvoyait
déjà pas de registre rechargé (l. 531).

### D4 — sur toute erreur, la tâche revient à son statut d'origine, et l'échec de cette remise est journalisé sans être propagé

C'est la règle d'aujourd'hui (l. 533 à 554), reconduite à l'identique, y compris
le texte `"Failed to process callback result: {e}"` — le changer casserait
l'affichage sans rien gagner.

Le **pourquoi** est écrit dans la doc de la fonction, parce qu'il n'est pas
lisible dans le code : une erreur peut survenir **après** que spirit a fait le
tour, si la relecture du `Task` par `context_id` échoue aussi. La tâche repart
alors en file, et la livraison suivante rejoue le tour. La règle préfère donc
**un tour en double à un résultat perdu**. C'est un choix, pas une évidence, et
c'est exactement le genre de pourquoi qu'un commentaire doit porter.

L'échec de la remise elle-même est journalisé en `warn!` et **non** propagé :
propager écraserait le message d'erreur que l'opérateur doit lire par un second
message qui ne parle que de la base.

### D5 — un seul assistant de tâche dans le module `tests`, lifté du gabarit de `tui/app.rs`

Les trois tests du vol ont besoin d'une tâche de callback **livrée**. Le gabarit
existe (`tui/app.rs` l. 2439 à 2480) : `NewTask` avec `label:
"long_running:run_claude_pilot"`, `trigger_type: "callback"`, `action_type:
"resume_agent"`, puis `update_task_status(&id, "delivered")`.

Il est lifté en **un seul** assistant du module `tests` de `chat_turn.rs` —
`tache_de_callback_livree(db: &AsyncDatabase) -> String` — appelé par les trois
tests. Un par test divergerait en silence et les tests seraient verts sans rien
livrer (mika#2205) ; c'est la raison que la doc de `tui/app.rs` donne déjà pour
son propre assistant, et elle vaut ici.

La base est `AsyncDatabase::new(Database::open_in_memory()…)`, la même expression
que `chat.rs::tests::base_en_memoire` (l. 1437). Aucune fixture de serveur n'est
recopiée : `spawn_mock` et `ok_task_with_text` existent (l. 243, l. 271).

`tui/app.rs` n'est **pas** modifié : son assistant reste là où il est, pour ses
propres tests. Lifter veut dire « écrire le même gabarit ici », pas « déplacer ».

### D6 — l'allowlist de la garde V1 monte au niveau module, et se livre vide

`ALLOWED` est aujourd'hui locale à la fonction de test (l. 589). Une assertion
qui la pinne vide doit observer **la constante que le scan lit réellement**, et
non une copie qui pourrait diverger d'elle. Elle monte donc au niveau module
sous le nom `V1_ALLOWED`, exactement comme `V2A_ALLOWED` (l. 692), dont la doc
porte déjà cette raison mot pour mot.

Elle est livrée `&[]`, et `mika1727_v3a2b_lallowlist_du_tour_est_livree_vide`
l'asserte vide, sur la forme de `mika1727_v2a_lallowlist_est_livree_vide`
(l. 791 à 799). Doctrine mika#2201 / #2323 / #2425 / #2457 / #2759 : le remède à
un rouge est de router le nouvel appelant par `chat_turn`, jamais d'ajouter une
ligne.

`mika1727_v1_le_site_tolere_na_quun_seul_appel_local` (l. 633 à 654) **part** :
elle n'existait que pour empêcher un second appel local de se cacher derrière
l'exception de fichier. Sans exception, elle n'a plus d'objet, et la garder
voudrait dire garder un compte `== 1` qui doit valoir `0` — un faux rouge au
premier passage. Sa disparition n'ouvre aucun angle mort : la garde principale,
allowlist vide, couvre désormais `commands/chat.rs` comme tout le reste.

`mika1727_v1_le_scan_nest_pas_vide` (l. 662 à 684) reste **tel quel**, sans un
caractère de différence : c'est l'anti-vacuité (mika#2103, mika#2205) qui
empêche les deux gardes d'être vertes par cécité, et elle vise déjà nommément
`commands/chat.rs` et `commands/chat_turn.rs`.

### D7 — ce que la doc doit cesser de dire

Trois textes décrivent un état qui n'existera plus, et deux d'entre eux
décrivent déjà un état qui n'existe plus (É1, É2) :

1. **`chat_turn.rs` l. 63 à 69** (doc de `send_turn_to_spirit`) — « Le premier
   appelant à `true` est le vol V3a-2b » devient : le bras `CallbackResult`
   passe `true`, par `livrer_tour_de_callback`.
2. **`chat_turn.rs` l. 578 à 586** (doc de la garde V1) — la phrase sur le vol
   V2 et sur `server/a2a.rs:343` part (É1). La doc dit qu'**aucun site n'est
   plus toléré**, et nomme le remède : router par `chat_turn`, jamais élargir.
3. **`remote_ask.rs` l. 706 à 708** — le commentaire de `run_remote` garde sa
   première moitié (`callback_turn` ne voyage pas depuis `mika ask --remote` :
   c'est l'opérateur qui tape le message) et corrige la seconde : le bras qui en
   pose un ne « tourne pas ailleurs », il passe par `send_turn_to_spirit`.
4. **`crates/mika-cli/CLAUDE.md` l. 384 à 393** — « L'unique appel local restant
   est le bras `CallbackResult` » devient « il n'en reste aucun », et la phrase
   sur le poll est corrigée : **le poll de `tui/app.rs` reste** (É2).

`cargo test -p mika-common --test claude_md_size` garde le plafond : le
paragraphe est réécrit, pas allongé.

### D8 — ce que ce vol ne livre pas

- **L'attestation par un test de l'ordre « `tool_result` en base avant le
  tour »**. L'ordre est préservé — le `save_message_with_metadata` reste en tête
  du bras, avant l'appel — mais l'attester demanderait d'atteindre le bras
  depuis un test, c'est-à-dire un `AppContext` complet. Hors périmètre.
  `Tracked in: senara-solutions/mika#1727`.
- **Le retrait du provider LLM local et de MCP** : `worker_mcp` et
  `worker_settings` gardent d'autres lecteurs (É7). Vol V3.
- **Le poll des callbacks et `AgentRequest::CallbackResult`** : `tui/app.rs` est
  inchangé à l'octet.
- **Aucun ticket n'est ouvert** (gel jusqu'au 01/12), pas même de suivi. Les
  restes vont dans la PR, en `Tracked in: senara-solutions/mika#1727`.

---

## Fire-Disposition

Ce vol **modifie un détecteur** — `mika1727_v1_le_tour_de_la_tui_ne_fait_plus_tourner_la_boucle`,
dont le chemin de succès est « aucune violation trouvée ». La gate mika#2306
s'applique donc, et la disposition est nommée ici.

**Option retenue : (a), dans sa forme dégénérée — l'exception disparaît au lieu
d'être nommée, et une assertion auto-nettoyante la maintient absente.**

- L'allowlist est livrée **vide** (`V1_ALLOWED: &[&str] = &[]`). Il n'y a aucune
  exception à nommer, parce que le seul site exempté — `commands/chat.rs` — perd
  sa violation dans le commit C1, **avant** que C2 vide l'allowlist. À aucun
  instant de la séquence le détecteur n'est rouge (§ 3).
- L'assertion auto-nettoyante est
  `mika1727_v3a2b_lallowlist_du_tour_est_livree_vide` : elle rougit dès qu'une
  entrée réapparaît, et son message nomme le remède (router par `chat_turn`).
  C'est le pendant exact de `mika1727_v2a_lallowlist_est_livree_vide`, livré au
  vol V2a pour la même raison.
- **Ni (b) ni (c)** : le détecteur est livré **armé**, pas désarmé — il balaie la
  moitié production du crate entier à chaque `cargo test -p mika-cli`. Et rien
  n'escalade : la violation qu'il refusait n'existe plus dans l'arbre.
- La vacuité est couverte séparément et sans modification, par
  `mika1727_v1_le_scan_nest_pas_vide` (l. 662 à 684) : une garde qui ne voit
  rien ne refuse rien, et une allowlist vide sur un balayage vide serait un vert
  par cécité.

---

## 3. Ordre de commit

Trois commits. L'ordre est **contraint**, pas choisi : C2 vide une allowlist qui
protège une violation que C1 supprime, et C2 avant C1 serait rouge.

### C1 — le tour de callback part sur le fil

`feat(1727): le bras CallbackResult part sur le fil (vol V3a-2b)`

Fichiers : `commands/chat_turn.rs` (production + tests), `commands/chat.rs`,
`init.rs`.

**Pourquoi un seul commit** : clippy refuse la bascule sans les retraits (onze
liaisons deviennent inutilisées dans le même geste) et refuse
`livrer_tour_de_callback` sans son appelant (`dead_code` sur un `pub(crate)` que
seul `#[cfg(test)]` lirait). Le ticket le dit déjà : « la bascule du bras et le
retrait des variables vont dans le même commit ».

1. **`chat_turn.rs`** — `livrer_tour_de_callback` et son corps à endpoint résolu
   (D1, D2, D3, D4), avec leur doc. Imports ajoutés :
   `use crate::tui::app::AgentResponse;` et
   `use mika_agent::async_db::AsyncDatabase;`.
2. **`chat.rs`** — le bras garde l. 449 à 475 **inchangées** (`tool_result` puis
   cadrage) et remplace l. 477 à 555 par un appel à
   `chat_turn::livrer_tour_de_callback(&worker_db, &worker_agent_name,
   &worker_session, &framing, &task_id, &original_status).await`, avec un
   commentaire qui dit pourquoi l'ordre « base d'abord » tient.
3. **`chat.rs`** — les liaisons sans lecteur partent : `tool_registry`
   (l. 232-241), `embedding_client` (l. 267), `message_sender` (l. 268-273),
   `brave_api_key` (l. 275-279), `github_token` (l. 280), `cli_tier` (l. 288),
   `cli_deployment` (l. 291), et les `worker_*` correspondants — `worker_llm`
   (l. 310), `worker_tools` (l. 312), `worker_embedding` (l. 316),
   `worker_brave_key` (l. 317), `worker_github_token` (l. 318),
   `worker_gateway_url` (l. 319), `worker_internal_token` (l. 320-324),
   `worker_github_app` (l. 325), `worker_sender` (l. 326), `worker_global_home`
   (l. 329). Les imports `use secrecy::ExposeSecret;` (l. 9) et
   `use mika_agent::tools;` (l. 35) partent avec eux (É5).
4. **`chat.rs`** — `spawn_agent_worker` perd `http_client` (l. 194) ; ses trois
   appelants (l. 664, 917, 1027) perdent l'argument ; la construction l. 654
   part, n'ayant plus de lecteur.
5. **`chat.rs`** — `SetModel` : `worker_llm = new_llm;` part, la validation
   devient `if updated_settings.make_llm_provider().is_ok()` (É3).
6. **`init.rs`** — `make_message_sender` (l. 209 à 247) et la ligne d'import
   l. 4 partent (É6). `AppContext.llm` (l. 17) et `DbContext.github_app` (l. 38)
   prennent `#[expect(dead_code, reason = "mika#1727 V3 : dernier lecteur parti
   au vol V3a-2b ; V3 retire le champ")]` — `expect` et non `allow`, pour que
   l'attribut rougisse de lui-même le jour où un lecteur revient ou où le champ
   part.

**Tests livrés dans ce commit** (ils ne se reportent jamais) :

- `mika1727_v3a2b_le_tour_de_callback_part_sur_le_fil_avec_la_cle` —
  **critères 1 et 2 dans une seule fonction**, comme
  `mika1727_v3a2a_…` (l. 457) et pour la même raison : la moitié « la tâche
  reste `delivered` » n'a de valeur que mesurée sur le même envoi que la moitié
  « la clé est posée ». À travers `spawn_mock`, le corps capturé porte
  `metadata[mika_cli::remote_ask::CALLBACK_TURN_KEY] == Bool(true)` — **lu par la
  constante, jamais par un littéral** — et
  `metadata[…CALLER_SESSION_ID_KEY] == session`. La réponse n'est pas une
  erreur, porte le texte rendu par le serveur, et `get_task(&id)` rend encore
  `"delivered"`.
- `mika1727_v3a2b_un_endpoint_mort_rend_la_tache_a_son_statut_dorigine` —
  **critère 3**, deux moitiés, `completed` et `failed`, pour que la remise soit
  mesurée comme « le statut d'origine » et non comme une constante. L'endpoint
  vient de `DeadEndpoint::reserve()` (mika#2569) et le garde est tenu pendant
  tout l'appel. La réponse est une erreur, et `get_task` rend le statut
  d'origine.

**Après C1, le pilote mesure.** `git diff --numstat origin/main...HEAD`, plan
exclu. **Si le total dépasse 540, il s'arrête** et dit « HALTE borne : N lignes
pour 600 », avec le compte par commit.

### C2 — l'allowlist du tour se livre vide

`test(1727): l'allowlist du tour se livre vide (vol V3a-2b)`

Fichier : `commands/chat_turn.rs` (module `tests` uniquement).

1. `ALLOWED` monte au niveau module sous `V1_ALLOWED`, livrée `&[]`, avec la doc
   qui dit pourquoi elle est au niveau module (D6).
2. `mika1727_v3a2b_lallowlist_du_tour_est_livree_vide` l'asserte vide, sur la
   forme de `mika1727_v2a_lallowlist_est_livree_vide` — **critère 4**.
3. `mika1727_v1_le_site_tolere_na_quun_seul_appel_local` (l. 633 à 654) part.
4. La doc de `mika1727_v1_le_tour_de_la_tui_ne_fait_plus_tourner_la_boucle`
   (l. 578 à 586) est réécrite : aucun site n'est plus toléré (D7.2, É1).
5. `mika1727_v1_le_scan_nest_pas_vide` est **inchangée**.

Vérification du commit : `cargo test -p mika-cli` vert, et la garde principale
verte **sans exception de fichier**.

### C3 — la doc dit l'état d'après

`docs(1727): la doc dit qu'aucun tour ne reste local (vol V3a-2b)`

Fichiers : `commands/chat_turn.rs` (doc de `send_turn_to_spirit`),
`remote_ask.rs` (commentaire de `run_remote`, l. 706 à 708 uniquement),
`crates/mika-cli/CLAUDE.md`.

Contenu : D7.1, D7.3, D7.4.

### Fin de pipeline

Dans cet ordre, avant d'ouvrir la PR :

1. `cargo fmt`
2. `cargo clippy --all-targets -p mika-cli` — vert
3. `cargo test -p mika-cli` — vert
4. `cargo test -p mika-common --test claude_md_size` — vert
5. `bash scripts/verify-egress-manifest.sh --report`
6. `bash scripts/verify-pipeline.sh origin/main`
7. `git diff --numstat origin/main...HEAD` — plan exclu, ≤ 600
8. `git diff --stat origin/main...HEAD -- crates/mika-cli/src/tui/app.rs` — **vide**

**Si 5 ou 6 est rouge, le pilote s'arrête et le dit** (critère 7). Il ne répare
pas un rouge de pipeline par un élargissement de périmètre.

La PR porte `Refs mika#1727`, aucun `Closes`/`Fixes`/`Resolves`, et
`Code review: delegated to mika-qa (mika#2694)`. Si le gabarit de reprise
(`wip-rescue`) l'ouvre, il écrit `Closes #N` par défaut : le corriger avant toute
revue, puis vérifier que `gh pr view <N> --json closingIssuesReferences` rend
`[]` (critère 8).

### Halte du pilote sur la borne

Après **chaque** commit : `git diff --numstat origin/main...HEAD`, plan exclu.

- total > 600 à tout moment → **halte**, pas de commit suivant, pas de PR ;
- total > 540 après C1 → **halte** aussi : le reste ne tiendrait pas.

Dernier message : « HALTE borne : N lignes pour 600 », avec le compte par
commit.

**Les commentaires comptent dans la borne et ne se rabotent pas pour la tenir.**
Si le code écrit dépasse, le pilote s'arrête ; il ne retire pas après coup un
commentaire qui porte un pourquoi (D4 en est un).

**Si le pilote doit s'écarter de ce plan** — un fichier, un nom, un critère — il
amende ce plan sur la branche dans un commit `docs(plans)` **avant** de pousser
le code qui s'en écarte, en nommant le point et la raison.

---

## 4. Definition of Done

- [ ] `chat_turn::livrer_tour_de_callback` existe, envoie le cadrage par
      `send_turn_to_endpoint(…, true)`, et rend un `AgentResponse::Reply`.
- [ ] Le bras `AgentRequest::CallbackResult` de `chat.rs` garde l'écriture du
      `tool_result` et le cadrage inchangés, et appelle cette fonction.
- [ ] Sur toute erreur, la tâche revient à son statut d'origine ; l'échec de
      cette remise est journalisé sans être propagé, et la doc dit pourquoi un
      tour en double vaut mieux qu'un résultat perdu.
- [ ] Les liaisons et imports sans lecteur sont partis de `chat.rs` ;
      `spawn_agent_worker` n'a plus de paramètre `http_client`, ses trois
      appelants non plus.
- [ ] `make_message_sender` et son import sont partis de `init.rs` ;
      `AppContext.llm` et `DbContext.github_app` portent un `#[expect(dead_code,
      reason = …)]` nommant V3.
- [ ] `V1_ALLOWED` est au niveau module, vide, et une assertion la pinne vide.
      `mika1727_v1_le_site_tolere_na_quun_seul_appel_local` est partie.
      `mika1727_v1_le_scan_nest_pas_vide` est inchangée.
- [ ] Les trois corrections de doc (D7) sont faites, `CLAUDE.md` compris.
- [ ] `tui/app.rs` est inchangé à l'octet.
- [ ] `cargo fmt` ; `cargo clippy --all-targets -p mika-cli` vert ;
      `cargo test -p mika-cli` vert ; `cargo test -p mika-common --test
      claude_md_size` vert.
- [ ] `bash scripts/verify-egress-manifest.sh --report` et
      `bash scripts/verify-pipeline.sh origin/main` lancés avant la PR ; un
      rouge arrête le pilote.
- [ ] `git diff --numstat origin/main...HEAD` ≤ 600 lignes ajoutées, plan exclu.
- [ ] La PR porte `Refs mika#1727` et aucun `Closes`/`Fixes`/`Resolves`.
- [ ] Aucun ticket ouvert (gel) ; les restes en `Tracked in:
      senara-solutions/mika#1727`.

---

## 5. Acceptance criteria

Transcrits du corps de mika#1727, § « Critères de V3a-2b ».

1. `livrer_tour_de_callback`, à travers `spawn_mock`, envoie le texte de cadrage
   avec `mika_a2a::params::CALLBACK_TURN_KEY` à `true` et
   `mika.caller_session_id` égal à la session. Le test lit la clé par la
   constante, jamais par un littéral.
2. Sur ce succès, la réponse n'est pas une erreur, porte le texte rendu par le
   serveur, et la tâche reste `delivered`. C'est le test négatif de ce vol.
3. Sur `DeadEndpoint`, la réponse est une erreur, et la tâche revient à son
   statut d'origine : `completed` pour l'une, `failed` pour l'autre.
4. `production` de `commands/chat.rs` ne contient plus `agent::run_agent(` ni
   `AgentParams`. `ALLOWED` est vide, et un test l'asserte vide.
   `mika1727_v1_le_tour_de_la_tui_ne_fait_plus_tourner_la_boucle` est verte sans
   exception de fichier.
5. `grep -rn "make_message_sender" crates/mika-cli/src` ne rend rien.
   `AppContext.llm` et `DbContext.github_app` portent un `#[expect(dead_code,
   reason = …)]` qui nomme V3, et aucun nouvel `allow` n'apparaît dans le crate.
6. Seuls changent les fichiers listés dans le périmètre du ticket :
   `commands/chat.rs`, `commands/chat_turn.rs`, `init.rs`, `remote_ask.rs` (le
   seul commentaire de `run_remote`) et `crates/mika-cli/CLAUDE.md`.
   `tui/app.rs` est inchangé, à l'octet.
7. `cargo clippy --all-targets -p mika-cli` est vert. `cargo test -p mika-cli`
   est vert, de même que `cargo test -p mika-common --test claude_md_size`. Le
   pilote lance `bash scripts/verify-egress-manifest.sh --report` et
   `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il
   trouve l'un des deux rouge, il s'arrête et le dit.
8. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
   compris si le gabarit de reprise l'ouvre.

---

## Taille estimée

| Fichier | Lignes ajoutées estimées |
|---|---|
| `commands/chat_turn.rs` — production (`livrer_tour_de_callback`, son corps, leur doc, deux imports) | 72 |
| `commands/chat_turn.rs` — tests du vol (assistant de tâche D5 + les deux tests de C1) | 140 |
| `commands/chat_turn.rs` — gardes V1 de C2 (`V1_ALLOWED` + sa doc, assertion pin-vide, doc réécrite) | 22 |
| `commands/chat_turn.rs` — doc de `send_turn_to_spirit` (C3) | 7 |
| `commands/chat.rs` (appel du bras + commentaire, signature, trois appelants, `SetModel`) | 45 |
| `init.rs` (deux `#[expect(dead_code, reason = …)]`) | 6 |
| `remote_ask.rs` (commentaire de `run_remote`) | 4 |
| `crates/mika-cli/CLAUDE.md` | 12 |
| **Total** | **308** |

Par commit : **C1 ≈ 263**, **C2 ≈ 22**, **C3 ≈ 23**.

Les retraits ne sont pas comptés : `git diff --numstat` compte les ajoutées, et
ce vol en retire beaucoup plus qu'il n'en ajoute (le seul bras `CallbackResult`
perd 119 lignes, `make_message_sender` 39).

**Marge, et d'où elle vient.** Le vol V3a-2a a réellement produit 312 lignes
pour 195 estimées (× 1,6), et le plan de ce vol en a tiré la leçon : les corps de
doc et les tests à deux moitiés coûtent plus que prévu, et `fn_call_width = 60`
(pas de `rustfmt.toml` dans le dépôt ⇒ `max_width = 100`) fait exploser les
listes d'arguments — `livrer_tour_de_callback` en a six. Au même facteur, 308
donnerait 493, et C1 donnerait 421 : les deux bornes de halte (600 au total, 540
après C1) tiennent encore, mais sans luxe. C'est la raison pour laquelle la
mesure après C1 est un point d'arrêt explicite et non une formalité.

**Les deux bornes sont tenues** : 308 ≤ 600 (unité du ticket, `git diff
--numstat origin/main...HEAD`, plan exclu) et 296 ≤ 429 (unité de l'architecte,
hors `docs/` et hors `CLAUDE.md` : 308 − 12).

Total estimé : 308 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3
