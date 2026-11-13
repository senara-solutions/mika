# mika#1727 — vol V2b-2b-1 : le détail d'une tâche se lit sur mika-spirit par HTTP, avec son premier lecteur

- **Ticket :** senara-solutions/mika#1727
- **Branche :** `feat/1727/tui-v2b2b1-detail-par-http` (neuve, depuis `main`)
- **Base :** `dbc6d2b01` (merge de V2b-2a, mika#3067)
- **PR :** `Refs mika#1727` — **jamais `Closes`**. Le ticket se ferme au merge de V4.
- **Revue et merge :** Vincent. Cette PR touche `docs/egress/egress-manifest.toml`, chemin
  CODEOWNERS `@samidarko`.
- **Matériau mesuré :** `refs/pull/3062/head`, commit `6f895f4e9` — `CallbackTaskDetail`
  (`tui/app.rs`, 13 lignes de type) et `fetch_callback_task_detail` (42 lignes). Repris en
  logique, **pas en forme** : le § D2 dit où et pourquoi la forme change.
- **Suite :** `Tracked in: senara-solutions/mika#1727` — l'abonnement, le drainage au tick, le
  filtre de session, la réclamation et la livraison `CallbackResult` partent en **V2b-2b-2** ; le
  recul, la reconnexion et la variante du factice au flux jamais fermé en **V2b-2b-3**. Aucun
  ticket n'est ouvert (gel jusqu'au 01/12).

---

## 0 — Ce qui est mesuré sur `dbc6d2b01`, avant toute conception

Huit mesures. Deux d'entre elles changent la forme du matériau, et une impose un arbitrage que le
§ 0.3 nomme au lieu de le cacher.

1. **`crates/mika-cli/src/commands/tasks.rs` fait 1 845 lignes.** `TaskEventStream` est l. 497,
   `open` l. 511, son `reqwest::Client::new()` l. 513 — **le seul client du fichier**. `stream_url`
   l. 456. `consume` l. 553, `drain_events` l. 595, `find_event_separator` l. 622.
2. **Le bras `TaskCommand::Get` est l. 86 à 112 et n'a aucun test.** Il résout un préfixe par
   `resolve_task_id` (l. 18), lit `db.get_task`, puis en `text` sonde `probe_pilot_liveness`
   (l. 718) avant `print_task_detail` (l. 860), et en `json`/`yaml` passe par `task_to_json`
   (l. 975). Les seuls tests du voisinage portent sur `task_to_json` seul (l. 1636 et 1651 — le
   corps les cite en 1639 et 1656, à trois lignes près du décalage introduit depuis sa rédaction) ;
   aucun ne traverse le bras.
3. **`run()` ouvre la base pour *tous* les sous-commandes**, `Stream` compris : `run` est l. 45 et
   `init::init_db_only_for_agent(agent_name)` l. 46, avant le `match`. Le critère « ne lit pas
   la base » porte donc sur **la tâche**, pas sur l'`init` — et `--spirit` n'y change rien, pas plus
   que `Stream` aujourd'hui. Le § D5 en tire la conséquence sur la formulation du DoD.
4. **`TaskDetailResponse` (`crates/mika-agent/src/server/dashboard.rs:786`) ne dérive que
   `Serialize`**, et porte **24 champs**. Elle n'est pas désérialisable côté client : le miroir local
   n'est pas une préférence, c'est la seule forme possible sans toucher `crates/mika-agent/`, que ce
   vol ne modifie pas.
5. **`handle_task_detail` (`dashboard.rs:845`) lit par identifiant exact** (`get_task_unscoped`,
   `db.rs`, `WHERE id = ?1`), rend 404 `{"error": …}` sur absence et 500 sur erreur interne. Elle
   est montée l. 204 de `server/mod.rs` sous `require_dashboard_or_internal_token` (l. 365) — la
   même couche que `/dashboard/tasks/stream`, donc **le jeton du flux suffit** et aucune
   authentification n'est à ajouter.
6. **Le matériau avale ses échecs.** `fetch_callback_task_detail` rend `Option<CallbackTaskDetail>`
   et transforme chaque échec en `None` + `tracing::warn!`. Le critère 2 de ce vol exige l'inverse :
   « font échouer la lecture explicitement ». La forme change donc, et le § D2 dit pourquoi les deux
   ont raison à leur place.
7. **Le miroir du matériau n'a pas `status`, délibérément**, et sa doc le dit : « `status` y est
   **délibérément absent**. Il n'y a rien à discipliner, le champ n'existe pas de ce côté. » Le
   corps de ce vol nomme `status` parmi les six champs. Le § 0.3 traite cette collision.
8. **Rien n'est à ajouter en dépendance.** `serde` est en dépendance normale de `mika-cli` avec la
   feature `derive` (`Cargo.toml:33`, workspace l. 25), `serde_json` l. 34, `yaml_serde` l. 35,
   `reqwest` l. 44, `axum` en `[dev-dependencies]` l. 68. Le lint egress est **vert sur
   `dbc6d2b01`** : `bash scripts/verify-egress-manifest.sh --report` → « 400 fichier(s) .rs de
   production scanné(s), 31 portant une construction de client […] No egress-manifest violations
   found. »

### 0.1 — Pourquoi un seul vol tient, et où la marge n'est pas disponible

Le corps borne ce vol à **279 lignes ajoutées**, « code, tests et manifeste compris, plan exclu ».
La règle mika#2693 borne le même vol à **279 lignes estimées** hors `docs/`. Les deux nombres
coïncident ici, et la seule divergence d'unité porte sur **une** ligne — le champ `data` du
manifeste, qui vit sous `docs/`. Comme les plans V2b-1b et V2b-2a, ce plan écrit son total en lignes
**ajoutées**, manifeste compris, et le soumet **tel quel** aux deux bornes, sans invoquer en sa
faveur le facteur 3 que la dérivation de 279 porte déjà.

Le total est de **241** (§ *Taille estimée*). Le vol touche **trois fichiers**, n'en crée aucun,
n'ajoute aucune dépendance, aucune route, aucune authentification, et ne modifie aucune signature
publique hors de `tasks.rs`.

La marge de 38 lignes n'est pas du remplissage disponible. Le découpage de V2b-2b en trois vols est
dicté par « chaque morceau avec son lecteur dans la même PR » — pas par l'enveloppe. Y remonter
l'abonnement de V2b-2b-2 produirait un abonnement qui pousse des trames sur un canal que personne ne
draine : exactement la faute que mika#2205 nomme et que le corps interdit.

### 0.2 — Ce que ce vol ne touche pas

`crates/mika-cli/src/tui/app.rs` n'est **pas modifié** — aucune ligne. `crates/mika-agent/` et
`crates/mika-a2a/` ne sont pas modifiés : `TaskDetailResponse` est consommée telle quelle, par
miroir. `poll_callback_tasks` (`app.rs`) reste en place, inchangé, et reste le seul chemin de
livraison des callbacks de la TUI à la fin de ce vol — son retrait est V2b-3.

Aucun fichier du plan de contrôle : `.github/workflows/`, `.claude/`, `skills/bundled/`,
`crates/mika-agent/src/well_known_agents.rs`, `.mika/`. **Une seule exception**, celle que le corps
ouvre nommément : le champ `data` de l'entrée `cli-spirit-tasks` de `docs/egress/egress-manifest.toml`
(§ D6). Aucune autre entrée n'est touchée.

Et `drain_events`, `find_event_separator`, `consume`, `frame_agent_id`, `TaskEventStream::open` et
`stream_url` ne bougent **pas d'une ligne** : ce vol ajoute un second geste HTTP à côté du premier,
il ne refond pas le premier.

### 0.3 — `status` dans le miroir : la collision, et ce qu'elle coûte

La mesure 7 et le corps se contredisent sur un champ, et il vaut mieux l'écrire que le résoudre en
silence.

Le matériau avait fait de `status` une **absence structurelle** : parce que `CallbackTaskDetail`
n'avait pas le champ, le chemin de réclamation de la TUI ne *pouvait pas* dériver la terminalité
d'une tâche du détail HTTP. Il devait la lire de la **variante de trame**
(`terminal_callback_frame` : `TaskCompleted` → `"completed"`, `TaskFailed` → `"failed"`). Et c'était
fondé : au moment où le `CallbackResult` se bâtit, la réclamation atomique a déjà passé la ligne à
`delivered`, donc `status` répondrait faux.

Le corps de **ce** vol nomme `status` parmi les six champs, et il a raison pour son lecteur : le
premier lecteur est `mika tasks get <id> --spirit`, une commande d'**affichage**, et un détail de
tâche qui n'affiche pas son statut n'est pas un détail de tâche. Le corps ajoute d'ailleurs la règle
qui tranche : un champ n'entre que si son lecteur est mesuré dans ce vol, « jamais pour plus tard ».
`status` a son lecteur ici ; `created_trace_id` et `parent_task_id`, que le matériau portait, n'en
ont aucun et **n'entrent pas**.

**Le coût est réel et il est nommé : l'interdit de V2b-2b-2 cesse d'être structurel.** Quand le
chemin de réclamation arrivera, rien dans le type ne l'empêchera plus de lire `detail.status`. Ce
vol ne peut pas réparer ça — il n'a pas de chemin de réclamation à discipliner, et inventer une
seconde représentation du détail pour la TUI serait la duplication que ce ticket existe pour
supprimer. Ce qui est dû à V2b-2b-2, et que son groom doit reprendre :

> `original_status` se dérive de la variante de trame par `terminal_callback_frame`, exhaustivement
> par variante, **jamais de `TaskDetailMirror::status`** — que la réclamation atomique a déjà fait
> mentir. L'interdit n'est plus tenu par le type : il lui faut donc son test.

Cette phrase va dans le corps de PR, sous `Tracked in: senara-solutions/mika#1727`. **Aucun ticket
n'est ouvert** (gel jusqu'au 01/12).

---

## 1 — Les points de conception

### D1 — Le miroir est local, à six champs, et chacun a son lecteur dans ce vol

```rust
#[derive(serde::Deserialize, serde::Serialize)]
struct TaskDetailMirror {
    id: String,
    agent_id: String,
    label: String,
    status: String,
    created_by_session: Option<String>,
    result: Option<String>,
}
```

**Local parce qu'il n'y a pas d'alternative** (mesure 4) : `TaskDetailResponse` ne dérive que
`Serialize`. Six champs sur les 24 de la réponse ; serde ignore les dix-huit autres sans
`deny_unknown_fields`, ce qui est exactement la tolérance voulue — le serveur peut ajouter un champ
sans casser ce client.

**`Serialize` en plus de `Deserialize`**, et c'est un lecteur et non une commodité : les formats
`json` et `yaml` rendent le miroir par `serde_json::to_value` puis `to_string_pretty` /
`yaml_serde::to_string` (§ D4). Sans `Serialize`, il faudrait écrire un `json!{…}` à la main qui
redirait les six champs une seconde fois — deux vérités pour une.

**Les six champs et leur lecteur :** `id`, `agent_id`, `label`, `status` et `result` sont affichés
par les trois formats (§ D4) ; `created_by_session` aussi. Ce dernier est le seul dont le lecteur
d'affichage est plus faible que son lecteur à venir — c'est le champ sur lequel V2b-2b-2 bâtira le
filtre de session, parce que la session **ne voyage pas sur le fil** (`server/tasks_stream.rs`). Il
entre ici parce qu'il est affiché ici, pas parce qu'il servira là-bas : sans son affichage, il
serait sorti du miroir.

`created_trace_id` et `parent_task_id` du matériau **n'entrent pas** : aucun lecteur dans ce vol.

### D2 — `fetch_task_detail` échoue, là où le matériau rendait `None`

```rust
async fn fetch_task_detail(base_url: &str, token: &str, task_id: &str) -> Result<TaskDetailMirror>
```

Posée juste après l'`impl TaskEventStream`, dans le même fichier, parce que c'est là que le corps
la veut : « à côté de `TaskEventStream::open`, et nulle part ailleurs ». URL de base et jeton
**en argument**, par la raison déjà écrite en l. 412-414 de `tasks.rs` — `dashboard::auth_token`
lit l'environnement du processus et échoue sans lui, donc un consommateur qui résout ses propres
accès ne se pilote pas depuis un test.

Trois échecs, trois `Err` avec leur cause nommée, à l'image de `open` :

| cas | ce que rend `fetch_task_detail` |
|---|---|
| transport (serveur injoignable) | `Err`, contexte « failed to fetch task detail at {url} » |
| statut non-succès (404, 500, …) | `Err`, **corps compris** — comme `open` l. 524-531 |
| corps illisible / champ requis absent | `Err`, contexte nommant l'identifiant |

**Pourquoi la forme du matériau ne transfère pas.** Dans `tui/app.rs`, le `None` était juste : un
détail non répondu **ne doit pas** interrompre le drainage ni injecter un callback à moitié bâti, et
la ligne reste non réclamée pour que le poll la reprenne. Il y avait un plan de repli. Ici il n'y en
a aucun : le lecteur est une commande qui n'a qu'un geste à faire, et un `None` avalé rendrait un
détail vide comme un succès — exactement ce que le corps interdit, et la raison pour laquelle `open`
échoue plutôt que de rendre un flux silencieux. Quand V2b-2b-2 branchera le drainage, c'est **son
appelant** qui transformera ce `Err` en « je passe et je ne réclame pas », là où la décision se
prend. L'erreur voyage, la politique reste chez qui la porte.

**Un `timeout` de 5 s**, repris du matériau, et l'asymétrie avec `open` est voulue : `open` ouvre un
flux dont le silence est un état normal — un timeout l'abattrait — tandis que le détail est une
requête courte dont l'absence de réponse n'est jamais normale. Une ligne, et elle n'est attestée par
aucun test : un test de timeout achète cinq secondes de mur pour mesurer une constante de reqwest.
C'est une propriété de sûreté déclarée, pas une fonctionnalité, et le plan préfère le dire.

**Pas de `task_detail_url` en fonction**, contrairement à `stream_url`. Celle-là existe parce que
`stream_task_events` l'imprime dans son message de connexion (l. 418-421) : elle a deux lecteurs.
L'URL du détail n'en a qu'un, et le `format!` est inline avec le même `trim_end_matches('/')`.

### D3 — Le second `Client::new()` du fichier, et pourquoi le lint reste vert sans effort

`fetch_task_detail` construit son client comme `open` construit le sien. Trois faits sur le scanner
(`scripts/lib/egress_manifest_lint.py`) :

- **D1 (code → manifeste).** `CLIENT_CONSTRUCTION_RE` (l. 148) inventorie les **fichiers** qui
  portent une construction de client. `tasks.rs` y est déjà, couvert par `client_site` de
  `cli-spirit-tasks`. Un second client **dans le même fichier** n'ajoute rien à l'inventaire, et
  aucun fichier neuf n'est créé.
- **D3 (manifeste → code).** `client_site` et `call_site` restent `tasks.rs` : le client y naît et
  la requête y part, aux deux définitions du README (l. 57-58). C'est très exactement la D3 qui
  rougissait sur #3062, où le client avait déménagé dans un fichier neuf et la déclaration était
  devenue fantôme — et c'est la raison pour laquelle le corps exige que le client vive dans
  `tasks.rs` et non dans `tui/app.rs`.
- **D2 (littéraux de host).** Aucun littéral neuf : l'URL est bâtie sur `base_url`, et
  `fake_spirit` lie `"127.0.0.1:0"` (couvert par `LOOPBACK_V4_RE`, l. 256) et sert sur
  `format!("http://{addr}")`, forme dont `normalize_host` rend `None`.

Ce qui **n'est pas** couvert sans geste, c'est le champ `data` : voir § D6.

### D4 — La sortie passe par un puits, et le texte a son propre écrivain

```rust
async fn get_task_detail_from_spirit_into<W: std::io::Write>(
    base_url: &str, token: &str, task_id: &str, format: OutputFormat, out: &mut W,
) -> Result<()>
```

Le puits est un paramètre pour la raison déjà écrite en l. 426-432 pour `stream_task_events_into` :
la propriété à tenir est « le détail, **et rien d'autre** », et elle n'est mesurable qu'en comparant
tout le puits à une chaîne attendue — ce que `std::io::stdout()` n'autorise pas depuis un test Rust
sans sous-processus.

Trois formats, et **l'ordre des gestes est la moitié du critère 2** : la lecture HTTP d'abord, le
`?` ensuite, l'écriture seulement après. Rien n'est écrit avant que le détail soit en main, donc un
échec laisse le puits **vide** — ce n'est pas une politique à appliquer, c'est la conséquence de
l'ordre, et c'est elle que les trois tests d'échec mesurent par `assert!(puits.is_empty())`.

- `json` → `serde_json::to_string_pretty(&mirror)`
- `yaml` → `yaml_serde::to_string(&mirror)`
- `text` → `write_task_detail_text(&mirror, out)`, un écrivain **neuf** de six lignes de champs sur
  le modèle visuel de `print_task_detail` (`ID:`, `Label:`, `Status:`, `Agent:`, …).

**Pourquoi un écrivain neuf plutôt que `print_task_detail`.** Trois raisons, et la première est un
critère : le critère 3 exige que `print_task_detail` ne soit pas modifié. La deuxième est de type :
elle prend `&Task` — 20 champs, dont `trigger_type`, `process_id`, `metadata` — que le miroir n'a
pas et que ce vol n'a aucune raison de demander au serveur. La troisième est qu'elle prend aussi un
`&PilotLiveness`, et la vivacité du pilote est une mesure **locale** (`probe_pilot_liveness` lit la
base et sonde un PID du système) : sous `--spirit`, elle n'a pas de réponse, et en afficher une
serait prétendre savoir. L'écrivain du miroir n'affiche donc pas de ligne `Dispatch pilot:` — son
absence est exacte, pas incomplète.

### D5 — `--spirit` est un booléen, et son absence est le comportement d'aujourd'hui

```rust
Get {
    id: String,
    #[arg(long, value_enum, default_value = "text")]
    format: OutputFormat,
    /// Lire le détail sur mika-spirit par HTTP, au lieu de la base locale.
    #[arg(long)]
    spirit: bool,
}
```

Le bras `TaskCommand::Get` gagne **un `if spirit { … } else { <le corps d'aujourd'hui, intact> }`**.
Sous `false`, le chemin est celui de `dbc6d2b01` à l'octet près : `resolve_task_id`, `db.get_task`,
`probe_pilot_liveness`, `print_task_detail`, `task_to_json`. Sous `true` : `dashboard::spirit_url()`,
`dashboard::auth_token()?`, puis `get_task_detail_from_spirit_into` sur `std::io::stdout()`. Pas de
résolution de préfixe — le serveur lit par identifiant exact (mesure 5) et un client qui devinerait
un préfixe que le serveur ne résout pas inventerait une réponse. Pas de sonde de vivacité (§ D4).

**Pas de `--url`**, contrairement à `TaskCommand::Stream`. Le corps n'en demande pas, le test pilote
la fonction interne et non le bras, et `dashboard::spirit_url()` lit déjà `MIKA_SPIRIT_URL` : un
opérateur qui veut un autre serveur l'a par l'environnement. Un flag sans lecteur mesuré est la
faute que le corps interdit.

**Une précision que la mesure 3 impose.** `run()` ouvre la base pour **tous** les sous-commandes,
avant le `match` — `Stream` compris, aujourd'hui. « Ne lit pas la base » porte donc sur **la
tâche**, et c'est ce que le DoD écrit : aucune lecture de la ligne de tâche, aucune résolution de
préfixe, aucune sonde. Déplacer l'`init` hors du chemin de `--spirit` toucherait le bras de tous les
autres sous-commandes pour un gain nul dans ce vol — c'est de la matière de V3.

### D6 — Le champ `data` nomme les deux gestes, parce qu'il en décrit maintenant deux

Aujourd'hui `cli-spirit-tasks` dit : « ouverture d'un flux SSE de tâches ; aucun corps envoyé au-delà
du jeton » (`egress-manifest.toml:352-360`). C'était exact tant que `tasks.rs` n'avait qu'un geste.
V2b-2a l'a laissé exact en n'ajoutant **aucune** requête — le filtre `--agent` est appliqué côté
client, sur le fil déjà reçu. Ce vol ajoute une **seconde requête**, et le champ cesserait d'être
vrai en silence.

Nouveau libellé :

> `data = "deux gestes vers mika-spirit : l'ouverture d'un flux SSE de tâches, et la lecture du détail d'une tâche par son identifiant (GET /api/v1/tasks/{task_id}, l'identifiant dans le chemin) ; aucun corps envoyé au-delà du jeton"`

`client_site` et `call_site` restent `tasks.rs`. `destination`, `destination_source`, `class`,
`logged` et `owner` ne changent pas : la destination est la même, la classe est `internal`, et
`logged = false` reste vrai — il n'y a ni corps ni query à journaliser.

**Le pilote lance `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il le
trouve rouge, il s'arrête et le dit** — vert sur `dbc6d2b01` (mesure 8), donc un rouge est une
découverte, pas une routine à absorber.

### D7 — Le factice apprend la route du détail, sans que son bloc bouge

`fake_spirit` reste le même **bloc** `#[cfg(test)] pub(crate) mod`, à la même place : en fin de
fichier, après le module `tests`. Sa doc actuelle (l. 1784-1800) dit pourquoi, et rien n'y change —
le scanner egress n'écarte le code de test que sous cette forme et à cet endroit, et le remonter
déplacerait sa coupure.

Une fonction neuve à côté de `spawn`, qui ne la touche pas :

```rust
pub(crate) async fn spawn_detail(statut: u16, corps: String)
    -> (String, Arc<Mutex<Option<String>>>)
```

Elle sert `/api/v1/tasks/{task_id}` — un paramètre de chemin, donc n'importe quel identifiant — avec
le statut et le corps que le test choisit, et **enregistre l'en-tête `authorization` reçu** dans le
`Mutex` qu'elle rend. C'est ce second membre qui fait du « le jeton part en `Bearer` » du critère 1
une mesure prise **sur la socket**, et non une relecture du code qui l'écrit. Les trois tests
d'échec ignorent ce membre (`let (base, _) = …`) : le statut et le corps leur suffisent.

Un seul `spawn_detail` répond à plusieurs requêtes — le corps est cloné par le gestionnaire, comme
`spawn` le fait déjà — donc le test des formats (T2) n'en lance qu'un pour deux appels.

La **variante au flux jamais fermé** de `spawn` n'arrive pas ici : son unique lecteur est le test de
reconnexion, qui part en V2b-2b-3 avec lui.

---

## 2 — La forme livrée

**`crates/mika-cli/src/commands/tasks.rs`**

- `struct TaskDetailMirror` — six champs, `Deserialize` + `Serialize`, privée au fichier (§ D1).
  Elle passera `pub(crate)` en V2b-2b-2 si et seulement si un lecteur hors du fichier existe alors.
- `async fn fetch_task_detail(base_url, token, task_id) -> Result<TaskDetailMirror>` — posée après
  l'`impl TaskEventStream` (§ D2).
- `fn write_task_detail_text<W: Write>(&TaskDetailMirror, &mut W) -> Result<()>` (§ D4).
- `async fn get_task_detail_from_spirit_into<W: Write>(base_url, token, task_id, format, out)`
  (§ D4).
- Le bras `Some(TaskCommand::Get { id, format })` devient `{ id, format, spirit }` et gagne son
  `if spirit { … } else { … }`, la branche `else` **inchangée** (§ D5).
- `fake_spirit::spawn_detail` (§ D7). `fake_spirit::spawn` et `sse_payload` inchangées.

**`crates/mika-cli/src/cli.rs`**

- `TaskCommand::Get` gagne `#[arg(long)] spirit: bool`, avec son doc-comment : la lecture part sur
  mika-spirit, par identifiant exact, sans préfixe ni sonde de vivacité locale.

**`docs/egress/egress-manifest.toml`**

- Le champ `data` de `cli-spirit-tasks` (§ D6). **Une ligne, aucune autre entrée.**

**Rien d'autre.** Pas de fichier neuf, pas de dépendance, pas de route, pas d'authentification
touchée, aucune signature publique hors de `tasks.rs`, et ni `crates/mika-agent/`, ni
`crates/mika-a2a/`, ni `crates/mika-cli/src/tui/` ne sont modifiés.

---

## 3 — Les tests

Tous dans le `mod tests` existant de `tasks.rs`. **Cinq neufs, aucun adapté** : le bras `Get` n'a
aucun test sur `main` (mesure 2), et les quatre tests du flux comme les trois de `task_to_json`
touchent du code que ce vol ne modifie pas.

- **T1 — `mika1727_v2b2b1_mika_tasks_get_spirit_rend_le_detail_lu_sur_mika_spirit`.** Un
  `spawn_detail(200, <corps des six champs>)`, `get_task_detail_from_spirit_into` en `Text` sur un
  puits `Vec<u8>`, et **une égalité sur tout le puits**. Puis l'assertion sur le `Mutex` rendu :
  l'en-tête reçu est exactement `Bearer jeton-de-test`. Deux mesures dans un test parce qu'elles
  portent sur **la même requête** : ce qui en est revenu, et ce qui y est parti.
- **T2 — `mika1727_v2b2b1_les_trois_formats_rendent_le_meme_miroir`.** Le même factice, appelé en
  `Json` puis en `Yaml`, deux égalités sur tout le puits. Avec T1 (`Text`), les trois formats du
  critère sont couverts, et l'égalité-sur-tout mesure qu'aucun format ne rend un champ de plus ni de
  moins que les six.
- **T3 — `mika1727_v2b2b1_un_404_echoue_explicitement_et_necrit_rien`.** `spawn_detail(404,
  r#"{"error":"task 't-absente' not found"}"#)` — le corps **que le serveur rend vraiment**
  (mesure 5). `is_err()`, le message d'erreur contient `404`, et `assert!(puits.is_empty())`.
- **T4 — `mika1727_v2b2b1_un_500_echoue_explicitement_et_necrit_rien`.** Même forme en 500. Les deux
  statuts sont deux tests et non un seul paramétré, parce que le critère 2 demande « un test pour
  chacun des trois cas ».
- **T5 — `mika1727_v2b2b1_un_corps_illisible_echoue_explicitement_et_necrit_rien`.**
  `spawn_detail(200, "{\"id\":\"t-1\"}")` — un **200**, un JSON valide, et `agent_id` absent. C'est
  le cas qui compte : un corps non-JSON échouerait aussi, mais celui-ci est la forme qu'un serveur
  ayant dérivé rendrait réellement, et c'est elle qui doit ne jamais passer pour un succès.
  `is_err()` et `assert!(puits.is_empty())`.

**Le puits vide des T3, T4 et T5 est la moitié mesurable du critère 2.** L'autre moitié —
« explicitement » — est tenue par l'`is_err()` et par la présence du statut dans le message.

---

## 4 — Fire-Disposition

Ce vol livre **cinq détecteurs** — cinq tests dont le chemin de succès est « aucune violation
trouvée ».

**Option (a) — exception nommée en allowlist, avec une allowlist vide.** Les cinq atterrissent
**armés**, sans `#[ignore]` et sans entrée d'exception, parce qu'il n'existe **aucune violation
existante à exempter** : `TaskDetailMirror`, `fetch_task_detail`, l'écrivain texte, le puits et le
flag sont du code neuf, et le comportement préexistant que ce vol encadre — le chemin local de
`mika tasks get` — n'est pas touché d'une ligne (§ D5) et n'est de toute façon attesté par aucun
test sur `main` (mesure 2). Rien à nommer, rien à référencer en suivi, donc aucune assertion
auto-nettoyante à écrire : « l'allowlist est vide » est ici le résultat, pas une dette. En inventer
une entrée produirait l'allowlist de complaisance que mika#2309 condamne.

**Deux situations basculent en (c) — halte-et-remontée, et aucune en `#[ignore]` :**

- **Si `scripts/verify-egress-manifest.sh --report` rougit** après la correction du § D6. Il est
  vert sur `dbc6d2b01` et ce vol ne crée aucun fichier ni aucun client dans un fichier non déclaré
  (§ D3). Un rouge signifierait que le scanner voit quelque chose que le § D3 n'a pas prévu : le
  pilote s'arrête et le dit. Corriger le manifeste au-delà du champ `data` est le geste de Vincent
  (CODEOWNERS `@samidarko`).
- **Si un test préexistant rougit** — les trois de `task_to_json` (l. 1636, 1651, 1668) ou les
  quatre du flux. Le § D5 prétend ne pas toucher leur code ; s'ils rougissent, c'est que la
  prétention est fausse. Ce n'est pas une exception à déclarer, c'est un défaut de conception à
  remonter en commentaire sur mika#1727 avant de pousser. Jamais un `#[ignore]` posé sur la mesure
  qui vient de contredire le plan.

---

## 5 — Ordre de commit

Plancher de rebase : `dbc6d2b01`. Chaque commit compile, passe `cargo test -p mika-cli` et
`cargo clippy` seul.

1. **`feat(1727): le détail d'une tâche se lit sur mika-spirit par HTTP`**
   `TaskDetailMirror`, `fetch_task_detail`, `fake_spirit::spawn_detail`, et T3, T4, T5 — la lecture
   et ses trois échecs dans le même commit, parce que « échoue explicitement » est la propriété que
   cette fonction livre, et non un raffinement ultérieur. `open`, `consume`, `drain_events`,
   `find_event_separator` et `frame_agent_id` inchangées.
2. **`feat(1727): mika tasks get prend --spirit`**
   `write_task_detail_text`, `get_task_detail_from_spirit_into`, le flag de `cli.rs`, le
   `if spirit` du bras `Get`, et T1 et T2 — l'appelant et ses deux mesures dans le même commit.
3. **`docs(egress): cli-spirit-tasks déclare la lecture du détail`**
   Le champ `data` seul (§ D6), séparé parce qu'il est le seul chemin CODEOWNERS de la PR et qu'un
   relecteur doit pouvoir le lire sans traverser le reste.

Avant la PR : `cargo test`, `cargo clippy`, `cargo fmt`, les lints du dépôt,
`bash scripts/verify-egress-manifest.sh --report` et `bash scripts/verify-pipeline.sh origin/main`.

Le corps de PR porte `Refs mika#1727` — **jamais `Closes`** —, la ligne
`Code review: delegated to mika-qa (mika#2694)`, la ligne
`Tracked in: senara-solutions/mika#1727` pour V2b-2b-2 et V2b-2b-3, le fait que la revue et le merge
reviennent à Vincent (chemin CODEOWNERS), et il nomme **deux** choses non traitées ici :

- `mark_task_delivered` reste une écriture **locale** sur la base, à faire passer par mika-spirit en
  V2b-3 ou après ;
- l'interdit du § 0.3 — `original_status` se dérive de la variante de trame, jamais de
  `TaskDetailMirror::status` — n'est plus tenu par le type et doit l'être par un test en V2b-2b-2.

---

## 6 — Definition of Done

- [ ] `fetch_task_detail` lit `GET /api/v1/tasks/{task_id}` avec URL de base et jeton **en
      argument**, dans `crates/mika-cli/src/commands/tasks.rs` et nulle part ailleurs.
- [ ] Un statut non-succès (404, 500) échoue avec le statut et le corps dans le message ; un corps
      illisible échoue aussi. Aucun de ces cas n'écrit quoi que ce soit dans le puits.
- [ ] `TaskDetailMirror` porte exactement six champs — `id`, `agent_id`, `label`, `status`,
      `created_by_session`, `result` — et chacun est affiché par les trois formats.
- [ ] `mika tasks get <id> --spirit` rend le détail dans les trois formats, et le jeton part en
      `Bearer`, mesuré sur l'en-tête reçu par le factice.
- [ ] Sans `--spirit`, le chemin local est inchangé : le diff ne modifie ni `resolve_task_id`, ni
      `probe_pilot_liveness`, ni `print_task_detail`, ni `task_to_json`, et sous `--spirit` la
      commande ne lit pas la ligne de tâche, ne résout pas de préfixe et ne sonde pas la vivacité.
- [ ] Les trois tests de `task_to_json` et les quatre tests du flux passent **sans modification**.
- [ ] `fake_spirit` est resté le même bloc `#[cfg(test)] pub(crate) mod`, à la même place ; `spawn`
      et `sse_payload` sont inchangées.
- [ ] Le champ `data` de `cli-spirit-tasks` nomme les deux gestes ; `client_site` et `call_site`
      restent `tasks.rs` ; aucune autre entrée du manifeste n'est touchée.
- [ ] `crates/mika-agent/`, `crates/mika-a2a/` et `crates/mika-cli/src/tui/` ne sont pas modifiés ;
      aucun fichier du plan de contrôle non plus, hors l'exception `cli-spirit-tasks`.
- [ ] `poll_callback_tasks` est en place et inchangé.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check`,
      `scripts/verify-egress-manifest.sh --report` et `scripts/verify-pipeline.sh origin/main` sont
      verts.
- [ ] Le total ajouté, mesuré par `git diff --numstat origin/main...HEAD` hors plan, est ≤ 279.
- [ ] La PR porte `Refs mika#1727`, `Tracked in: senara-solutions/mika#1727`, et nomme les deux
      dettes du § 5. Aucun ticket n'est ouvert (gel jusqu'au 01/12).

---

## Acceptance criteria

Les quatre critères de V2b-2b-1, transcrits du corps du ticket, et deux que la conception ajoute.

1. **Contre le serveur factice, `mika tasks get <id> --spirit` rend le détail lu par
   `GET /api/v1/tasks/{task_id}`.** Un test compare la sortie entière, et vérifie que le jeton part
   en `Bearer`.
2. **Une réponse en erreur HTTP (404 et 500) et un corps illisible font échouer la lecture
   explicitement : rien n'est écrit dans le puits.** Il y a un test pour chacun des trois cas.
3. **Sans `--spirit`, le chemin local de `mika tasks get` est inchangé.** Le diff ne modifie ni
   `resolve_task_id`, ni `probe_pilot_liveness`, ni `print_task_detail`, ni `task_to_json`, et les
   tests existants de `task_to_json` passent sans modification. Il n'existe sur main aucun test du
   bras `TaskCommand::Get` lui-même.
4. **Egress Manifest Lint est vert avec l'entrée `cli-spirit-tasks` corrigée** pour dire la donnée
   réellement envoyée. Le pilote lance `bash scripts/verify-egress-manifest.sh --report` avant
   d'ouvrir la PR. S'il le trouve rouge, il s'arrête et le dit.
5. **Le miroir ne porte que des champs qui ont un lecteur dans ce vol.** Six champs, chacun affiché
   par les trois formats. `created_trace_id` et `parent_task_id`, que le matériau portait, sont
   absents.
6. **Le client du détail naît dans `tasks.rs`**, à côté de `TaskEventStream::open`, et reçoit URL de
   base et jeton en argument — donc il se pilote depuis un test sans toucher l'environnement du
   processus, et la déclaration egress `client_site = tasks.rs` reste vraie.

### Correspondance AC → critères de V2b-2 (corps)

| critère de V2b-2 | disposition |
|---|---|
| **1.** un callback terminé atteint la TUI en `CallbackResult`, une seule fois, avec son détail HTTP | **la moitié « détail HTTP » est tenue ici** (AC1, AC2, AC6) ; l'abonnement, le filtre session, la réclamation et la livraison partent en V2b-2b-2 |
| **2.** une trame d'un **autre agent** n'est pas livrée | **tenu par V2b-2a** (`frame_agent_id` + `consume`) |
| **2.** une trame d'une **autre session** du même agent n'est pas livrée | **V2b-2b-2** — le champ `created_by_session` arrive ici (AC5), son filtre arrive là-bas |
| **3.** `Unknown` / `OverflowMarker` ne cassent pas l'abonnement | **tenu par V2b-2a** pour la lecture du fil ; la part « abonnement » part en V2b-2b-2 |
| **3.** un détail illisible ou en erreur HTTP n'est ni livré ni réclamé | **la moitié « ni livré » est tenue ici** (AC2 : `Err`, puits vide) ; « ni réclamé » suppose une réclamation, donc V2b-2b-2 |
| **4.** échec d'ouverture → recul ; fin de flux → reconnexion | **V2b-2b-3**, avec la variante du factice au flux jamais fermé, son unique lecteur |
| **5.** Egress lint vert avec `cli-spirit-tasks` **corrigée** | **tenu ici** (AC4) — la correction accompagne la requête qu'elle déclare |

`Tracked in: senara-solutions/mika#1727`

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `TaskDetailMirror` + doc (`crates/mika-cli/src/commands/tasks.rs`) | 26 |
| `fetch_task_detail` + doc | 40 |
| `write_task_detail_text` + les bras `json`/`yaml` | 26 |
| `get_task_detail_from_spirit_into` + le bras `TaskCommand::Get` | 28 |
| `--spirit` sur `TaskCommand::Get` + doc (`crates/mika-cli/src/cli.rs`) | 10 |
| `fake_spirit::spawn_detail` — statut, corps, et le jeton reçu | 28 |
| T1, T2 — sortie entière des trois formats, et le jeton en `Bearer` | 44 |
| T3, T4, T5 — 404, 500, corps illisible : échec explicite, puits vide | 38 |
| `docs/egress/egress-manifest.toml` — champ `data` de `cli-spirit-tasks` | 1 |

Total estimé : 241 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**L'unité, et pourquoi le nombre n'est pas assoupli.** 241 est une prévision de lignes **ajoutées**
au sens `git diff --numstat origin/main...HEAD`, plan exclu — l'unité que le corps impose. Ce plan la
compare **telle quelle** aux deux bornes, sans invoquer le facteur 3 que la dérivation de 279 porte
déjà : 241 ≤ 279 pour les deux. La seule ligne qui vit sous `docs/` est le champ `data` du
manifeste ; l'inclure ou non ne déplace rien, et le corps demande explicitement de l'inclure.

**Provenance des neuf postes : mesurés sur le matériau, pas estimés de zéro.** Les deux premiers
(66 lignes) correspondent aux 55 lignes que `6f895f4e9` dépensait pour `CallbackTaskDetail` (13) et
`fetch_callback_task_detail` (42), plus la dizaine que coûtent le `Result` du § D2 et le champ
`status` du § 0.3 — en doc et en contexte d'erreur. Les trois suivants (64) sont du câblage **neuf** :
#3062 n'exposait aucun flag, aucun format et aucun écrivain texte, parce que son unique lecteur était
la TUI. `spawn_detail` (28) est neuf : le factice de `main` ne sert que la route du flux. Les cinq
tests (82) sont neufs : le matériau n'attestait la lecture du détail par **rien** — ses 434 lignes de
test (`1f24523a7`) portaient sur le fil et sur la livraison.

**Pourquoi un seul vol tient.** Le vol touche **trois fichiers** et n'en crée aucun. Il n'ajoute
aucune dépendance (mesure 8), aucune route, aucune authentification (mesure 5 : la couche existante
suffit), aucun client dans un fichier non déclaré (§ D3), et aucune signature publique hors de
`tasks.rs`. Il ne modifie ni `tui/app.rs`, ni `crates/mika-agent/`, ni `crates/mika-a2a/`. Ses trois
commits sont séquencés par la mesure (§ 5) et chacun compile et passe seul. La marge de 38 lignes
n'est pas disponible : § 0.1.

**Ce que la suite pèse, et pourquoi elle n'est pas chiffrée ici.** V2b-2b-2 porte l'abonnement, le
drainage au tick, le filtre de session, la réclamation et la livraison `AgentRequest::CallbackResult`
— l'essentiel des ~263 lignes que le corps a mesurées sur `6f895f4e9`, moins les 66 qui atterrissent
ici, plus ses propres tests. V2b-2b-3 porte le recul, la reconnexion et la variante du factice au
flux jamais fermé. Le recoupage de chacun est la décision de **son** groom, sur une mesure prise sur
`main` à ce moment-là — pas une prévision à faire aujourd'hui.

`Tracked in: senara-solutions/mika#1727` — V2b-2b-2, V2b-2b-3, puis V2b-3 (retrait de
`poll_callback_tasks` et de la dette `mark_task_delivered`), V2c, V3 et V4. Aucun ticket n'est ouvert
(gel jusqu'au 01/12) : la suite se note sur mika#1727.
