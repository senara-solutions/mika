# mika#1727 — vol V2b-2b-2 : un callback terminé de la conversation arrive par le SSE, se lit par HTTP et se livre une fois

- **Ticket :** senara-solutions/mika#1727
- **Branche :** `feat/1727/tui-v2b2b2-livraison-par-le-sse` (neuve, depuis `main`)
- **Base :** `766a62788` (merge de V2b-2b-1, mika#3068)
- **PR :** `Refs mika#1727` — **jamais `Closes`**. Le ticket se ferme au merge de V4.
- **Matériau mesuré :** `refs/pull/3062/head`, commits `6f895f4e9` (294 lignes dans `tui/app.rs`,
  13 dans `commands/task_events.rs`) et `1f24523a7` (434 lignes de test). Repris en **logique**,
  pas en forme : le § 0.3 dit les quatre endroits où la forme change, et pourquoi.
- **Recoupé.** Le périmètre du corps ne tient pas sous la borne : le § 0.1 le mesure et nomme ce
  qui part. `Tracked in: senara-solutions/mika#1727`.
- **Suite :** `Tracked in: senara-solutions/mika#1727` — les deux autres axes du critère 2 et le
  critère 3 en entier partent au vol suivant (§ 0.1) ; le recul, la reconnexion et la variante du
  factice au flux jamais fermé restent **V2b-2b-3** ; puis V2b-3 (retrait de
  `poll_callback_tasks` et de la dette `mark_task_delivered`), V2c, V3 et V4. Aucun ticket n'est
  ouvert (gel jusqu'au 01/12).

---

## 0 — Ce qui est mesuré sur `766a62788`, avant toute conception

Neuf mesures. Une seule change la conception du matériau de fond en comble — la cinquième.

1. **`crates/mika-cli/src/commands/tasks.rs` fait 2 116 lignes.** `TaskEventStream` l. 509,
   `open(base_url, token)` l. 523 — **déjà jeton par argument**, donc l'`open_with_token` que le
   matériau ajoutait n'a plus d'objet. `consume(agent_filter, on_event)` l. 565. `frame_agent_id`
   l. 490. `TaskDetailMirror` l. 611 et `fetch_task_detail` l. 632, **tous deux privés au module**.
2. **`crates/mika-cli/src/tui/app.rs` fait 2 261 lignes.** `poll_callback_tasks` l. 1596,
   `POLL_INTERVAL_TICKS = 167` l. 663, l'appel du poll l. 1122 à 1127 sous
   `!self.is_team_mode() && self.tick_count.is_multiple_of(POLL_INTERVAL_TICKS) && self.status ==
   AgentStatus::Idle`. L'envoi `AgentRequest::CallbackResult` l. 1673.
3. **`chat.rs` a deux sites de construction, et ils sont disjoints :** `App::new` l. 545 (mode
   agent) et `App::new_team` l. 1125 (mode équipe), dans deux fonctions différentes (`run` et
   `run_team`). Le tick l'est aussi : `tick_agent_mode` porte le poll, `tick_team_mode` est une
   autre fonction (`app.rs:1133`). **Le critère 4 est donc structurel avant d'être gardé** : le
   § D7 en tire la conséquence.
4. **`TaskDetailResponse` (`crates/mika-agent/src/server/dashboard.rs:785`) porte bien les quatre
   champs demandés**, sous ces noms exacts : `trigger_type`, `action_type` (l. 789-790),
   `parent_task_id` (l. 793) et `created_trace_id` (l. 802). Elle ne dérive que `Serialize` : le
   miroir local reste la seule forme possible sans toucher `crates/mika-agent/`.
5. **`consume` prend une fermeture *synchrone*** — `impl FnMut(&str, Option<&TaskEventFrame>)`
   (l. 568). **Aucun `await` n'est possible dedans.** La lecture du détail ne peut donc pas vivre
   dans la tâche d'abonnement : elle vit dans le drainage, qui est `async`. C'est la mesure qui
   fixe la frontière du § D3, et elle n'était pas visible depuis le corps du ticket.
6. **`TaskEventFrame` dérive `Clone`** (`server/tasks_stream.rs:100`) et ses variantes
   `TaskCompleted` (l. 123) et `TaskFailed` (l. 132) portent `task_id`. Les huit variantes sont
   énumérées par `frame_agent_id` : la doctrine du bras exhaustif est déjà posée dans le fichier,
   et le § D2 s'y range au lieu d'en écrire une seconde.
7. **`mark_task_delivered` (`db/tasks.rs:1935`) est un `UPDATE … WHERE id = ?1 AND status IN
   ('completed','failed')`** et rend `n > 0`. Donc : la réclamation est atomique, elle rend
   `Ok(false)` sur une ligne déjà passée à `delivered`, et **une ligne réclamée ne peut plus
   répondre son statut d'origine**. C'est la mesure qui force l'`original_status` à venir de la
   variante de la trame (§ D2).
8. **Le factice a deux constructeurs sur deux serveurs** (`spawn` l. 2057, `spawn_detail` l. 2088),
   et la queue de chacun est la même : `TcpListener::bind("127.0.0.1:0")`, `tokio::spawn(serve)`,
   `format!("http://{addr}")`. `spawn_detail` a déjà la route **paramétrée**
   `/api/v1/tasks/{task_id}` et un témoin de l'en-tête reçu.
9. **Le lint egress est vert sur `766a62788`** : `bash scripts/verify-egress-manifest.sh --report`
   → « 400 fichier(s) .rs de production scanné(s), 31 portant une construction de client, 22
   host(s) distinct(s), 31 entrée(s) [[sink]] confrontée(s). No egress-manifest violations
   found. » `cli-spirit-tasks` (l. 356) déclare déjà les deux gestes, `cli-spirit-tui` (l. 374) la
   seule activation du dashboard, et `chat.rs` porte sa propre entrée (l. 413). **Ce vol ne
   construit aucun client neuf** (§ D8).

### 0.1 — Le recoupage : ce que le corps demande, ce que la borne autorise

Le corps borne ce vol à **279 lignes ajoutées**, et pose cinq critères dont trois exigent des
contrôles négatifs. Le périmètre entier ne tient pas, et la mesure le dit avant la conception :

- la production seule — miroir élargi, prédicat, filtre de trame, abonnement, drainage, livraison,
  câblage dans `chat.rs` — pèse **~198 lignes** (§ Taille estimée, dix premiers postes) ;
- le matériau dépensait **434 lignes de test** pour quatre lignes d'attestation et un second cas ;
  les neuf cas que les critères 1 à 4 demandent ici, même en tables, en pèsent de l'ordre de 190.

198 + 190 ≈ 388, pour une borne de 279 **et** un facteur d'expansion mesuré sur ce ticket même :
les trois vols précédents ont estimé 252, 209 et 241, et mesuré 301, 243 et 279 — soit ~1,16×. Une
estimation à 388 livrerait ~450 lignes.

**Ce vol livre donc la première part, et elle est verticale :** toute la production, de la trame
jusqu'au `CallbackResult`, plus les attestations qui traversent le chemin entier. Ce qui part au
vol suivant est **de l'attestation, jamais du chemin** :

| ce qui part | pourquoi il est séparable |
|---|---|
| critère 2, axe **agent** : « aucun aller-retour de détail » | demande un témoin des chemins demandés sur le factice ; le filtre lui-même est sur le fil, dans `consume`, déjà attesté par les quatre tests de V2b-2a |
| critère 2, axe **`trigger_type`/`action_type`** | même prédicat, même méthode, un cas de table de plus |
| critère 3 en entier : `Unknown`/`OverflowMarker`, détail en erreur HTTP ou illisible | le refus est **par construction** ici (§ D2 pour les trames, le `?` de `fetch_task_detail` pour le détail) ; ce qui manque est la mesure, pas le comportement |

Aucune de ces trois lignes ne laisse un livrable sans appelant (mika#2205) : l'abonnement est
drainé, le drainage livre, et chaque élément de production de cette part est traversé par T1. La
part suivante est un vol d'attestation sur du code déjà en place.

### 0.2 — Ce que ce vol ne touche pas

- **`poll_callback_tasks` : pas une ligne.** C'est le filet (critère 5). Son retrait est V2b-3.
- **`docs/egress/` : intact** (critère 5). Le § D8 dit pourquoi le manifeste reste exact sans être
  édité.
- **`crates/mika-agent/` et `crates/mika-a2a/` : intacts.** Le miroir local existe pour ça.
- **Aucun fichier du plan de contrôle** : `.github/workflows/`, `.claude/`, `skills/bundled/`,
  `well_known_agents.rs`, `.mika/`.
- **`TaskEventStream::open`, `consume`, `frame_agent_id`, `drain_events`, `find_event_separator` :
  inchangés.** Ce vol est leur appelant, pas leur réviseur.
- **Aucune dépendance ajoutée** : `reqwest` (`Cargo.toml:44`), `serde` l. 33, `axum` en
  `[dev-dependencies]` l. 68 — le factice en vit déjà.
- **Trois fichiers touchés, aucun créé** : `commands/tasks.rs`, `tui/app.rs`, `commands/chat.rs`.

### 0.3 — Les quatre endroits où la forme du matériau change, et pourquoi

1. **`open_with_token` disparaît.** `open` prend déjà l'URL et le jeton par argument (mesure 1).
   Le matériau ajoutait ce second constructeur parce que son `open` résolvait l'environnement ;
   V2b-1b a réglé la question à la source. Une ligne de `commands/` en moins à toucher.
2. **`fetch_callback_task_detail` et `CallbackTaskDetail` disparaissent** au profit de
   `fetch_task_detail` et `TaskDetailMirror`, déjà sur `main` (mesure 1). Le matériau avalait ses
   échecs en `None` ; V2b-2b-1 a fait l'inverse, et sa doc (l. 627-630) a **écrit d'avance** que
   l'appelant du drainage traduirait ce `Err` en « je passe et je ne réclame pas ». Ce vol est cet
   appelant, et il honore la phrase.
3. **Le prédicat de session devient le prédicat du poll, entier.** Le matériau ne comparait que
   `created_by_session`. Le critère 2 exige les quatre colonnes de
   `get_undelivered_callback_tasks_for_session` : agent, `trigger_type`, `action_type`, session.
   Le § D4 les porte en **une** méthode du miroir.
4. **Le recul et la reconnexion ne sont pas repris.** `spawn_task_event_subscription` perd sa
   boucle, son `backoff` et son `tx.is_closed()`. Le corps le demande, et le § D5 dit ce que ça
   coûte et qui paie.

---

## 1 — Les points de conception

### D1 — Le miroir grandit de quatre champs, et chacun a son lecteur dans ce vol

```rust
#[derive(serde::Deserialize, serde::Serialize)]
pub(crate) struct TaskDetailMirror {
    id: String,
    agent_id: String,
    pub(crate) label: String,
    status: String,
    trigger_type: String,
    action_type: String,
    created_by_session: Option<String>,
    pub(crate) result: Option<String>,
    pub(crate) created_trace_id: Option<String>,
    pub(crate) parent_task_id: Option<String>,
}
```

Dix champs. **Le miroir ne prend aucun champ « pour plus tard »**, et la visibilité le dit champ
par champ :

- `label`, `result`, `created_trace_id`, `parent_task_id` sont `pub(crate)` : leur lecteur est le
  drainage, dans `tui/app.rs` (§ D6) ;
- `agent_id`, `trigger_type`, `action_type`, `created_by_session`, `status` et `id` restent
  **privés** : leur seul lecteur hors des trois formats est `is_resume_callback_for` (§ D4), qui
  est une méthode du miroir. Un champ qui n'a besoin de personne dehors ne sort pas.

`created_trace_id` et `parent_task_id` entrent ici exactement pour la raison que leur exclusion de
V2b-2b-1 nommait (l. 606-609 : « faute de lecteur ») : le critère 1 les exige dans le
`CallbackResult`.

**La doc du miroir est réécrite, et son invariant change.** Sur `main` elle dit « Six champs, et
chacun a son lecteur ici : les trois formats de `get_task_detail_from_spirit_into` les rendent
tous. » Ce n'est plus vrai, et le § D9 dit pourquoi on ne le rétablit pas en élargissant l'écrivain
texte. La phrase devient : *dix champs ; `Serialize` en rend dix en `json` et `yaml`, l'écrivain
texte en rend les six que l'opérateur lit, et les quatre neufs sont lus par la livraison.*

### D2 — `terminal_callback_frame` : la variante dit l'`original_status`, et le bras est exhaustif

```rust
pub(crate) fn terminal_callback_frame(frame: &TaskEventFrame) -> Option<(&str, &'static str)> {
    match frame {
        TaskEventFrame::TaskCompleted { task_id, .. } => Some((task_id, "completed")),
        TaskEventFrame::TaskFailed { task_id, .. } => Some((task_id, "failed")),
        TaskEventFrame::TaskCreated { .. }
        | TaskEventFrame::TaskClaimed { .. }
        | TaskEventFrame::TaskDelivered { .. }
        | TaskEventFrame::TaskCancelled { .. }
        | TaskEventFrame::OverflowMarker { .. }
        | TaskEventFrame::Unknown => None,
    }
}
```

**Elle vit dans `tasks.rs`, à côté de `frame_agent_id`**, pour trois raisons et aucune n'est de
goût : c'est son jumeau (une lecture de variante, rien d'autre), sa doc d'exhaustivité est déjà
écrite là (mesure 6) et n'a pas à être redite, et `tui/app.rs` n'a ainsi aucune connaissance du
format du fil — la même frontière que « aucun client HTTP dans `tui/app.rs` » trace.

**Pas de bras `_ => None`.** La raison est celle que `frame_agent_id` porte déjà : une variante
neuve portant un `task_id` tomberait silencieusement hors du chemin des callbacks. Énumérée, elle
ne compile plus et le compilateur nomme le site qui doit décider.

**`original_status` vient de la variante, et de rien d'autre.** La mesure 7 le force : au moment où
le `CallbackResult` se bâtit, `mark_task_delivered` a déjà passé la ligne à `delivered`, donc le
`status` du détail dirait `delivered` et serait faux. Le corps le demande ; la mesure l'impose.
Noter que le miroir **porte** `status` (il est dans les six champs de `main`, lus par les trois
formats) : l'interdit est donc **dit** ici, pas structurel comme dans le matériau. C'est le prix
d'un miroir partagé entre deux lecteurs, et T1 en fait une mesure — le détail servi par le factice
porte délibérément `status: "delivered"`, donc un `original_status` tiré du détail rougit.

### D3 — La frontière abonnement / drainage est fixée par la mesure 5, pas par un choix

`consume` prend une fermeture **synchrone**. Le partage est donc contraint :

| geste | où | pourquoi là |
|---|---|---|
| ouvrir le flux, filtrer sur l'agent, lire la variante terminale | tâche d'abonnement (`tasks.rs`) | aucun `await` requis : `terminal_callback_frame` est pur |
| lire le détail, appliquer le prédicat, réclamer, livrer | drainage au tick (`tui/app.rs`) | chacun de ces quatre gestes est `async` |

Le canal porte donc **`(String, &'static str)`** — l'identifiant et l'`original_status` — et rien de
plus. Pas de type nommé : la paire est exactement ce que la variante sait dire, et un `struct` de
deux champs n'ajouterait qu'un nom.

Ce que le canal **ne** porte pas est aussi une décision : les trames d'un autre agent sont écartées
par `consume` (le premier étage, gratuit, sur le fil), les non-terminales par
`terminal_callback_frame`. Le canal ne reçoit donc que des candidats, jamais le fil brut.

### D4 — Le prédicat du poll, en une méthode du miroir

```rust
impl TaskDetailMirror {
    pub(crate) fn is_resume_callback_for(&self, agent_id: &str, session_id: &str) -> bool {
        self.agent_id == agent_id
            && self.trigger_type == "callback"
            && self.action_type == "resume_agent"
            && self.created_by_session.as_deref() == Some(session_id)
    }
}
```

**Les quatre colonnes de `get_undelivered_callback_tasks_for_session` (`db/tasks.rs:1908`), dans
l'ordre où elle les écrit.** Le corps l'exige : « elle applique **le prédicat du poll** sur le
détail ».

**Une méthode du miroir, pas une fonction de `tui/app.rs`,** pour une raison de visibilité qui est
aussi une raison de conception : quatre des cinq champs qu'elle lit n'ont aucun autre lecteur hors
du module (§ D1). Posée dehors, elle les rendrait tous `pub(crate)` et le miroir perdrait l'énoncé
« chaque champ a son lecteur, et pas un de plus ».

**Les deux colonnes que le prédicat ne porte pas, et pourquoi :**

- `status IN ('completed','failed')` : c'est `terminal_callback_frame` qui l'a déjà dit, et la
  mesure 7 montre que `mark_task_delivered` le redira en SQL. Le reprendre ici le ferait dire trois
  fois, dont une sur une valeur déjà périmée (§ D2).
- `completed_at NOT NULL` : une trame terminale est émise **à la transition**. Le corps en tire
  aussi l'autre conséquence — la garde d'ancienneté des échecs reste au poll, parce qu'une trame
  n'est jamais ancienne à l'arrivée.

### D5 — Une seule ouverture, aucun recul, et ce que ça coûte

```rust
pub(crate) fn spawn_callback_subscription(
    agent_id: String, base_url: String, token: String,
) -> mpsc::UnboundedReceiver<(String, &'static str)>
```

Une tâche, **une** ouverture. Un échec d'ouverture laisse une ligne `tracing::warn!` ; une fin de
flux laisse une ligne `tracing::debug!` ; dans les deux cas la tâche s'arrête et **rien ne la
relance**.

**Ce que ça coûte, nommé plutôt que découvert :** à partir de cet instant, les callbacks de cette
conversation n'arrivent plus que par le poll, toutes les 167 ticks (~5 s). C'est une **dégradation
de latence, pas une perte** — et c'est précisément pourquoi le corps garde le poll comme filet de
ce vol (critère 5). Le recul et la reconnexion sont V2b-2b-3, et l'observabilité de la mort
silencieuse part avec eux.

**Rien ne relance depuis le tick, et c'est délibéré.** Le tick bat toutes les 30 ms : une relance
conditionnée à `task_event_rx.is_none()` ne pourrait pas distinguer « pas encore démarré » de
« mort », et relancer sur un serveur absent serait une boucle chaude à 30 ms. Le drapeau « déjà
démarré » n'existe pas ici parce que **le spawn n'est pas au tick** (§ D7).

### D6 — Le drainage : un élément par tick, à l'endroit et sous la condition du poll

Appelé depuis `tick_agent_mode`, **immédiatement avant** l'appel du poll (`app.rs:1122`), sous
`!self.is_team_mode() && self.status == AgentStatus::Idle` — **sans**
`is_multiple_of(POLL_INTERVAL_TICKS)`. C'est tout l'intérêt : la latence devient celle du fil.

Le corps de la fonction, dans l'ordre :

1. `try_recv()` **une fois**. Rien → retour.
2. `fetch_task_detail(base_url, token, &task_id).await` ; un `Err` → `tracing::debug!` et retour.
   « Je passe et je ne réclame pas » : la ligne reste `completed`, le poll la reprendra.
3. `detail.is_resume_callback_for(self.db.agent_id(), &self.session_id)` faux → retour, sans
   réclamer.
4. `self.db.mark_task_delivered(&task_id).await` : `Ok(false)` → retour (le poll ou une autre TUI
   l'a prise) ; `Err` → `tracing::warn!` et retour.
5. Le résultat, **règle du poll à l'identique** : `Some(r)` non vide → `r` ; sinon si
   `original_status == "failed"` → `FAILED_TASK_FALLBACK` ; sinon retour (un `completed` sans
   résultat est sauté).
6. Message système `[{label}] {original_status}`, puis `AgentRequest::CallbackResult { task_id,
   label, result, trace_id: created_trace_id, original_status, parent_task_id }`, puis
   `self.status = AgentStatus::Thinking` et `needs_redraw = true`.

**Aucune boucle, et c'est un choix mesuré.** Le poll boucle parce qu'il reçoit une liste et doit
sauter les échecs anciens. Ici tout ce qui est sur le canal est un candidat (§ D3), et le corps
borne à « une livraison par tick au plus ». Un élément consommé qui ne livre pas (étapes 2 à 5)
consomme son tick — 30 ms, soit 167 fois plus serré que le poll qu'il double. Une boucle ne
gagnerait que sur un cas qui ne se produit pas en rafale, et coûterait le ré-emprunt de `self`
entre deux `await` que le matériau devait commenter.

**Trois champs sur `App`,** et la duplication avec le poll est **assumée, pas subie** : le corps
interdit de toucher `poll_callback_tasks`, donc les étapes 4 à 6 existent en deux copies pendant
exactement un vol — V2b-3 supprime le poll et la duplication avec lui. L'extraire ici serait
modifier le filet du vol, ce que le critère 5 interdit.

### D7 — Le critère 4 est structurel, et le mode équipe n'a rien à garder

La mesure 3 est décisive : `App::new` et `App::new_team` vivent dans deux fonctions disjointes de
`chat.rs`, et `tick_agent_mode` / `tick_team_mode` sont deux fonctions. L'abonnement part donc de
**`run`, après `App::new` (l. 545)**, et `run_team` ne le voit pas. En mode équipe,
`task_event_rx` reste `None` parce que le code qui le remplit n'est pas sur le chemin — pas parce
qu'un `if` l'a refusé.

Le `!self.is_team_mode()` au site d'appel du tick est conservé **par convention locale** (le poll
et les deux autres sondes l'ont, l. 1104 et 1122) et non comme la garantie : un garde redondant
avec une structure est gratuit, un garde qui remplace une structure est fragile.

**Le spawn est dans `chat.rs`, pas dans le constructeur et pas au tick.** Le constructeur a quatre
appelants de test qui n'entrent dans aucun runtime tokio — un `tokio::spawn` y paniquerait. Le tick
amènerait le problème de relance du § D5. `run` est `async`, dans le runtime, et possède déjà l'URL
et le jeton à résoudre une fois :

```rust
if let Ok(token) = crate::commands::dashboard::auth_token() {
    app.spirit_base_url = Some(crate::commands::dashboard::spirit_url());
    app.spirit_token = Some(token);
    app.task_event_rx = Some(tasks::spawn_callback_subscription(
        agent_name.to_string(), crate::commands::dashboard::spirit_url(), token,
    ));
}
```

**Un jeton absent n'est pas une erreur** : la TUI parle à un démon qu'elle ne possède pas. Les
trois champs restent `None`, le drainage sort au premier `try_recv` sur `None`, et le poll couvre.
Rien n'est écrit dans la conversation — la sonde de démarrage de `run` le dit déjà.

**Aucune variable d'environnement n'est mutée, ni lue par les tests.** L'URL et le jeton voyagent
par champ et par argument : c'est la couture qui permet à T1 de viser un `axum::Router` jetable.
La raison est celle que `chat_turn.rs:70-74` écrit déjà — `MIKA_SPIRIT_URL` est un état global que
les tests `cargo` parallèles se disputent, et les tests de `commands/agents.rs` posent **et
retirent** `MIKA_INTERNAL_TOKEN`.

### D8 — Le manifeste egress reste exact sans être édité

Le critère 5 exige `docs/egress/` intact **et** le lint vert. Les deux tiennent, et ce n'est pas
une coïncidence à vérifier après coup :

- **Aucun `reqwest::Client` neuf.** `tui/app.rs` et `chat.rs` n'en construisent pas : le drainage
  appelle `fetch_task_detail`, qui porte le sien (`tasks.rs:633`), et l'abonnement appelle
  `TaskEventStream::open`, qui porte le sien (l. 533). Le scanner compte les **constructions** ;
  son total de 31 ne bouge pas.
- **`cli-spirit-tasks` (l. 356) reste exacte** : `client_site` et `call_site` = `tasks.rs`, et son
  champ `data` nomme déjà les deux gestes depuis V2b-2b-1. Ce vol ajoute un appelant, pas un geste.
- **`cli-spirit-tui` (l. 374) reste exacte** : elle déclare pour `app.rs` la seule activation du
  dashboard, dont le client est construit l. 1004. Ce vol n'en ajoute aucun.

**Sonde avant d'ouvrir la PR** : `bash scripts/verify-egress-manifest.sh --report`. Rouge → arrêt
et remontée (§ 4), jamais une édition du manifeste.

### D9 — Le factice sert les deux routes sur une seule URL, et la queue des trois est partagée

La TUI ne connaît qu'une URL de base, donc le factice doit en servir une. Un troisième
constructeur, dans le **même** bloc `#[cfg(test)] pub(crate) mod fake_spirit`, sans le déplacer —
la raison de son emplacement en fin de fichier est écrite l. 2026-2032 et tient toujours : le
scanner egress n'écarte le code de test que sous la forme d'un bloc, et le remonter déplacerait sa
coupure.

```rust
pub(crate) async fn spawn_stream_and_details(
    flux: String, details: Vec<(String, u16, String)>,
) -> String
```

Le flux sur `/api/v1/dashboard/tasks/stream`, les détails sur `/api/v1/tasks/{task_id}` résolus par
identifiant dans une table partagée. La queue des trois constructeurs — `bind`, `tokio::spawn`,
`format!("http://{addr}")`, mesurée identique trois fois (mesure 8) — passe dans un `servir(router)
-> String` privé que les trois appellent. Trois lignes dupliquées valent mieux qu'une abstraction,
neuf non.

**Le corps SSE reste fini.** La variante au flux jamais fermé est V2b-2b-3, avec la reconnexion
qu'elle sert : sans reconnexion (§ D5), un corps fini suffit et un test s'arrête de lui-même, sans
`timeout` ni `abort`. Le témoin des chemins demandés part avec elle, au vol où un critère le lit
(§ 0.1).

### D10 — Ce que l'écrivain texte ne devient pas

`write_task_detail_text` (l. 659) reste à six champs. L'y étendre rétablirait l'invariant « les
trois formats rendent le même miroir » et coûterait ~16 lignes — code, chaîne attendue du test
texte, et deux `unwrap_or("—")`. Ce vol ne les a pas (§ 0.1), et l'arbitrage n'est pas neutre :
`trigger_type` et `action_type` sont des colonnes de machine, pas une lecture d'opérateur. Le test
des formats (l. 1428) est donc mis à jour **en se dédoublant** : la liste des six paires reste
l'attente du texte, une liste de dix celle de `json` et `yaml`. L'invariant « ni un champ de plus
ni un de moins » survit sur chaque format, pris séparément.

---

## 2 — La forme livrée

**`crates/mika-cli/src/commands/tasks.rs`**

| élément | visibilité | § |
|---|---|---|
| `TaskDetailMirror` + 4 champs, 4 champs `pub(crate)` | `pub(crate)` | D1 |
| `TaskDetailMirror::is_resume_callback_for` | `pub(crate)` | D4 |
| `fetch_task_detail` | `pub(crate)` | D3 |
| `terminal_callback_frame` | `pub(crate)` | D2 |
| `spawn_callback_subscription` | `pub(crate)` | D5 |
| `fake_spirit::spawn_stream_and_details`, `fake_spirit::servir` | `pub(crate)` / privé | D9 |

**`crates/mika-cli/src/tui/app.rs`**

| élément | § |
|---|---|
| `task_event_rx: Option<mpsc::UnboundedReceiver<(String, &'static str)>>` | D3 |
| `spirit_base_url: Option<String>`, `spirit_token: Option<String>` | D7 |
| les trois à `None` dans `App::new` **et** `App::new_team` | D7 |
| `drain_callback_deliveries` | D6 |
| son appel dans `tick_agent_mode`, avant celui du poll | D6 |

**`crates/mika-cli/src/commands/chat.rs`** — après `App::new` (l. 545), dans `run` seule : la
résolution, l'affectation des trois champs et le spawn (§ D7).

**Inchangés :** `poll_callback_tasks`, `TaskEventStream`, `consume`, `frame_agent_id`,
`write_task_detail_text`, `print_task_detail`, `run_team`, `App::new_team`, `docs/egress/`,
`crates/mika-agent/`, `crates/mika-a2a/`.

---

## 3 — Les tests

**T1 — bout en bout, dans `tui/app.rs` : du flux à `AgentRequest::CallbackResult`.**

Un seul `#[tokio::test]`, contre un seul `fake_spirit::spawn_stream_and_details` servant un flux de
quatre trames et les détails qui vont avec. Rien n'est simulé entre les deux : les octets partent
sur une socket, traversent `find_event_separator`, le filtre d'agent de `consume`,
`terminal_callback_frame`, le canal, le détail HTTP, `is_resume_callback_for`,
`mark_task_delivered`, et ressortent en `CallbackResult` sur le `agent_rx` que le test tient.

Quatre lignes, un seul fil, donc un seul test :

| ligne | ce qui est attesté |
|---|---|
| (a) `TaskCompleted`, notre agent, notre session | **un** `CallbackResult` : `result` lu sur le détail, `original_status == "completed"`, `trace_id` et `parent_task_id` du détail, message système `[label] completed`, `status = Thinking`, ligne passée à `delivered` |
| (b) la même trame une seconde fois | **aucun** second `CallbackResult` : `mark_task_delivered` rend `Ok(false)` |
| (c) `TaskFailed`, détail à `result: null` | `FAILED_TASK_FALLBACK` substitué, `original_status == "failed"` tiré de la **variante**, message `[label] failed` |
| (d) notre agent, une **autre** session | rien d'injecté, et la ligne **toujours** `completed` en base |

Le drainage livre un élément par tick : le test appelle donc `drain_callback_deliveries` autant de
fois qu'il y a de trames, plus une, et compte les `CallbackResult` reçus — un compte, jamais un
`is_some()`, parce que le critère porte sur « une seule fois ».

**Deux détails du factice qui portent chacun une moitié de critère :**

- les détails servis portent `status: "delivered"` — au moment où la TUI lit, la réclamation a eu
  lieu. Un `original_status` tiré du détail rougit ici (§ D2) ;
- ils portent aussi des clés que le miroir ignore (`depth`, `action_config`), ce qui mesure la
  tolérance au lieu de la redire.

**La ligne (d) exige une seconde ligne en base**, donc un `seed_callback` local : `create_task` avec
`trigger_type = "callback"`, `action_type = "resume_agent"`, `created_by_session`, puis passage à
`completed`. `app_with_response_sender` (l. 2159) fournit déjà la base en mémoire et la session ;
sa signature gagne le `request_rx` qu'elle jetait (`_request_rx`, l. 2181) — trois appelants, trois
lignes.

**T2 — mode équipe : aucun abonnement, par construction.** `App::new_team` laisse `task_event_rx`
à `None`, et un `drain_callback_deliveries` appelé dessus ne livre rien et ne touche pas le réseau.
Six lignes, et la garantie forte est structurelle (§ D7) : `run_team` n'a pas le site de spawn.

**Ce qui n'est pas attesté ici** et part au vol suivant (§ 0.1) : l'axe agent du critère 2 avec son
témoin d'aller-retour, l'axe `trigger_type`/`action_type`, et le critère 3 en entier.

**Suite complète** : `cargo test -p mika-cli`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo fmt --check`, `bash scripts/verify-egress-manifest.sh --report`, `bash
scripts/verify-pipeline.sh origin/main`.

---

## 4 — Fire-Disposition

Ce vol livre **deux détecteurs** — T1 et T2, dont le chemin de succès est « aucune violation
trouvée » sur les lignes (b) et (d) et sur le mode équipe.

**Option (a) — exception nommée en allowlist, avec une allowlist vide.** Les deux atterrissent
**armés**, sans `#[ignore]` et sans entrée d'exception, parce qu'il n'existe **aucune violation
existante à exempter** : `terminal_callback_frame`, `is_resume_callback_for`,
`spawn_callback_subscription`, `drain_callback_deliveries` et les trois champs d'`App` sont du code
neuf, et le comportement préexistant que ce vol double — `poll_callback_tasks` — n'est pas touché
d'une ligne (§ 0.2). Rien à nommer, rien à référencer en suivi, donc aucune assertion
auto-nettoyante à écrire : « l'allowlist est vide » est le résultat, pas une dette. En inventer une
entrée produirait l'allowlist de complaisance que mika#2309 condamne.

**Trois situations basculent en (c) — halte-et-remontée, et aucune en `#[ignore]` :**

- **`scripts/verify-egress-manifest.sh --report` rouge.** Il est vert sur `766a62788` (mesure 9) et
  le § D8 dit pourquoi il le reste. Un rouge signifie que le scanner voit ce que le § D8 n'a pas
  prévu : le pilote s'arrête et le dit. Le corps l'ordonne explicitement (critère 5), et
  `docs/egress/` est un chemin CODEOWNERS `@samidarko`.
- **Un test préexistant rouge.** Les quatre du flux (V2b-1b/V2b-2a), les cinq du détail
  (V2b-2b-1), ceux de `tick_agent_mode` (l. 2200+). Le § 0.2 prétend ne pas toucher leur
  comportement ; s'ils rougissent, la prétention est fausse. Seule exception prévue et déclarée :
  le test des formats (l. 1428), que le § D10 met à jour sur le miroir à dix champs.
- **Un hook ou un classifieur refuse un geste.** Arrêt et déclaration, même si le refus semble être
  un faux positif. Jamais un contournement par un autre outil (incident mika#2869).

---

## 5 — Ordre de commit

Plancher de rebase : `766a62788`. Chaque commit compile et passe `cargo test -p mika-cli` et
`cargo clippy -- -D warnings` seul.

1. **`feat(1727): le détail d'une tâche porte les quatre champs que la livraison lit`**
   § D1, § D4, § D10 — le miroir à dix champs, `pub(crate)` sur lui et sur `fetch_task_detail`,
   `is_resume_callback_for`, et le test des formats dédoublé. **Appelant immédiat :** les trois
   formats de `mika tasks get <id> --spirit`, déjà sur `main`, qui rendent les quatre champs neufs
   en `json` et `yaml` dès ce commit.
2. **`feat(1727): le flux produit l'identifiant et le statut d'un callback terminé`**
   § D2, § D5, § D9 — `terminal_callback_frame`, `spawn_callback_subscription`, et
   `spawn_stream_and_details` avec la queue partagée.
3. **`feat(1727): la TUI livre un callback terminé lu par le SSE et le détail`**
   § D3, § D6, § D7 — les trois champs d'`App`, `drain_callback_deliveries`, l'appel au tick, le
   câblage de `chat.rs`, puis T1 et T2. **C'est le commit qui ferme la boucle** : avant lui
   l'abonnement n'a pas d'appelant, et il n'existe pas sans lui.

Le découpage est séquencé par la dépendance de compilation, pas par commodité : 2 lit le miroir de
1, 3 lit le canal de 2.

---

## 6 — Definition of Done

- [ ] `TaskDetailMirror` porte dix champs ; `trigger_type`, `action_type`, `created_trace_id` et
      `parent_task_id` ont chacun leur lecteur dans ce vol (§ D1), et la doc du miroir énonce le
      nouvel invariant.
- [ ] `TaskDetailMirror` et `fetch_task_detail` sont `pub(crate)` ; aucun champ ne l'est sans
      lecteur hors du module.
- [ ] `is_resume_callback_for` porte les quatre colonnes du prédicat du poll, dans l'ordre de
      `get_undelivered_callback_tasks_for_session`.
- [ ] `terminal_callback_frame` énumère les huit variantes, sans bras `_`.
- [ ] `spawn_callback_subscription` ouvre **une** fois, filtre sur l'agent via `consume`, et
      n'écrit sur le canal que des couples `(task_id, original_status)`.
- [ ] `drain_callback_deliveries` est appelé depuis `tick_agent_mode`, avant le poll, sous
      `!is_team_mode() && status == Idle`, **sans** `POLL_INTERVAL_TICKS`, et consomme au plus un
      élément par tick.
- [ ] Un `Err` de `fetch_task_detail`, un prédicat faux, un `Ok(false)` de `mark_task_delivered` ou
      un `completed` sans résultat ne livrent rien **et ne réclament rien**.
- [ ] `original_status` vient de la variante de la trame ; T1 rougit s'il vient du détail.
- [ ] `poll_callback_tasks` est inchangé — `git diff origin/main -- crates/mika-cli/src/tui/app.rs`
      ne montre aucune ligne dans son corps.
- [ ] `docs/egress/` est intact et `bash scripts/verify-egress-manifest.sh --report` est vert.
- [ ] Aucun fichier du plan de contrôle touché ; `crates/mika-agent/` et `crates/mika-a2a/`
      intacts ; aucune dépendance ajoutée.
- [ ] T1 et T2 passent ; aucune variable d'environnement n'est lue ni mutée par eux.
- [ ] `cargo test -p mika-cli`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo fmt --check`, `bash scripts/verify-pipeline.sh origin/main` : verts.
- [ ] `git diff --numstat origin/main...HEAD` hors `docs/plans/` ≤ **279** lignes ajoutées.
- [ ] La PR porte `Refs mika#1727`, **jamais `Closes`**, nomme la dette `mark_task_delivered`
      (écriture locale, à passer par mika-spirit dans un vol suivant) sans la traiter, et porte
      `Tracked in: senara-solutions/mika#1727` pour ce que le § 0.1 renvoie. Aucun ticket ouvert.
- [ ] Le corps de PR est écrit dans `pr-body.md` **sous le worktree**, passé en `--body-file`, puis
      supprimé (mika#2211).

---

## Acceptance criteria

Transcrits **verbatim** depuis « ### Critères de V2b-2b-2 » du corps de senara-solutions/mika#1727.

1. Contre le serveur factice, un callback terminé de la conversation en cours atteint la TUI en
   `AgentRequest::CallbackResult` **une seule fois**, avec le bon `original_status`, le résultat lu
   par `GET /api/v1/tasks/{task_id}`, et `trace_id` et `parent_task_id` tirés du détail. Il y a un
   test pour `completed` et un pour `failed`. Un test rougit si la trame est livrée deux fois, ou
   si elle est livrée alors que le poll l'a déjà réclamée.
2. Rien n'est livré ni réclamé pour une trame d'un **autre agent**, une tâche d'une **autre
   session** du même agent, ou une tâche qui n'est pas `callback` / `resume_agent`. Il y a un test
   pour chacun des trois cas.
3. Une trame `Unknown` ou `OverflowMarker` ne casse pas l'abonnement. Un détail en erreur HTTP ou
   illisible n'est ni livré ni réclamé.
4. Hors mode équipe seulement : en mode équipe, aucun abonnement n'est ouvert.
5. `poll_callback_tasks` n'est pas modifié, et Egress Manifest Lint est vert **avec `docs/egress/`
   intact**. Le pilote lance `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la
   PR. S'il le trouve rouge, il s'arrête et le dit, au lieu de toucher le manifeste.

### Correspondance AC → ce que cette part livre

| AC | état dans ce vol | où |
|---|---|---|
| 1 | **entier.** `completed` (T1a), `failed` avec `FAILED_TASK_FALLBACK` (T1c), livrée deux fois (T1b), déjà réclamée par le poll (T1b, `Ok(false)`) | § D2, § D6, T1 |
| 2 | **axe session : attesté** (T1d). Axes **agent** et **`callback`/`resume_agent`** : le prédicat est livré et armé (§ D4), leurs deux cas de table partent au vol suivant (§ 0.1) | § D4, T1 |
| 3 | **comportement livré, mesure renvoyée.** Les trames non terminales sortent par un bras exhaustif (§ D2) et un détail en erreur par le `?` de `fetch_task_detail` (§ D3, étape 2) ; les deux tests partent au vol suivant (§ 0.1) | § D2, § D3 |
| 4 | **entier, et structurel** : le site de spawn n'est pas sur le chemin de `run_team` | § D7, T2 |
| 5 | **entier** : `poll_callback_tasks` intact, `docs/egress/` intact, sonde avant la PR | § 0.2, § D8, § 4 |

`Tracked in: senara-solutions/mika#1727` — les deux axes restants de l'AC 2 et l'AC 3 en entier.
Aucun ticket n'est ouvert (gel jusqu'au 01/12) : la suite se note sur mika#1727.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `TaskDetailMirror` — 4 champs neufs, `pub(crate)` sur elle et 4 champs, doc réécrite (`crates/mika-cli/src/commands/tasks.rs`) | 14 |
| `TaskDetailMirror::is_resume_callback_for` + doc | 16 |
| `terminal_callback_frame` + doc | 16 |
| `spawn_callback_subscription` + doc | 22 |
| `fake_spirit::spawn_stream_and_details` + `servir` partagé + doc | 26 |
| le test des formats, dédoublé sur le miroir à dix champs | 6 |
| `task_event_rx`, `spirit_base_url`, `spirit_token` + doc + init dans les deux constructeurs (`crates/mika-cli/src/tui/app.rs`) | 18 |
| `drain_callback_deliveries` + doc | 46 |
| son appel dans `tick_agent_mode` | 6 |
| résolution de l'URL et du jeton, spawn de l'abonnement (`crates/mika-cli/src/commands/chat.rs`) | 12 |
| T1 — bout en bout, quatre lignes, et son `seed_callback` | 52 |
| T2 — mode équipe : aucun abonnement | 6 |

Total estimé : 240 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**L'unité, et pourquoi la marge de 39 lignes n'est pas du gras.** 240 est une prévision de lignes
**ajoutées** au sens `git diff --numstat origin/main...HEAD`, plan exclu — l'unité que le corps
impose, comparée **telle quelle** aux deux bornes, sans réappliquer le facteur 3 que la dérivation
de 279 porte déjà. La marge est **calibrée sur ce ticket même** : ses trois vols précédents ont
estimé 252, 209 et 241 lignes et en ont mesuré 301, 243 et 279 — un facteur de 1,19, 1,16 et 1,16.
240 × 1,16 ≈ 278, soit sous la borne mesurée **et** sous le plafond du corps. Une estimation à 279
aurait livré ~320 et violé le critère de taille du corps en passant la porte du plan.

**Provenance des douze postes : mesurés sur le matériau et sur `main`, pas estimés de zéro.** Les
postes 7 à 9 (70 lignes) correspondent aux 294 que `6f895f4e9` dépensait dans `tui/app.rs`, moins
les 55 de `CallbackTaskDetail` + `fetch_callback_task_detail` (déjà sur `main`, § 0.3-2), moins les
~25 du recul et de la reconnexion (§ 0.3-4), moins la résolution paresseuse et son dédoublement
`drain`/`drain_from` que le § D7 remplace par trois lignes dans `chat.rs`. Les postes 2 à 4 (54)
sont du câblage **neuf** : le matériau ne comparait que la session (§ 0.3-3) et son
`terminal_callback_frame` vivait dans `app.rs`. Le poste 5 (26) est neuf : le factice de `main` ne
sert jamais les deux routes sur une URL (mesure 8). Les postes 11 et 12 (58) sont une réduction
assumée des 434 lignes de test de `1f24523a7` : une table de quatre lignes sur un serveur au lieu
de deux tests sur deux, et les cinq cas que le § 0.1 renvoie.

**Pourquoi un seul vol tient, après recoupage.** Trois fichiers touchés, aucun créé, aucune
dépendance (§ 0.2), aucune route, aucun client (§ D8), aucune signature publique hors de
`mika-cli`. Les trois commits sont séquencés par la dépendance de compilation et chacun passe seul
(§ 5). Le vol reste vertical : son premier commit a son appelant sur `main`, et son troisième ferme
la boucle trame → `CallbackResult`. Ce qui part au vol suivant est de l'attestation sur du code
déjà livré, nommé ligne par ligne au § 0.1 — jamais un morceau du chemin.

**Si la mesure dépasse 279 malgré tout** : le pilote retire la ligne (c) de T1 — le cas `failed` —
avec son détail du factice, la renvoie par `Tracked in: senara-solutions/mika#1727`, et **le dit
dans le corps de la PR**. C'est la seule soupape autorisée : elle retire une mesure, jamais une
ligne du chemin de production, et elle n'élargit aucun fichier exclu.

`Tracked in: senara-solutions/mika#1727` — les deux axes restants de l'AC 2, l'AC 3, puis
V2b-2b-3 (recul, reconnexion, factice au flux jamais fermé), V2b-3 (retrait de
`poll_callback_tasks` et de la dette `mark_task_delivered`), V2c, V3 et V4. Aucun ticket n'est
ouvert (gel jusqu'au 01/12).
