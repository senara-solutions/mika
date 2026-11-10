# mika#1727 — vol V2b-2b-2, voie A4 : la livraison d'un `completed` par le SSE

> - **Branch:** `feat/1727/tui-v2b2b2-livraison-completed-par-le-sse`
> - **Borne du vol :** 429 lignes ajoutées, code, commentaires et tests compris,
>   plan exclu, mesurées par `git diff --numstat origin/main...HEAD`.
> - **Total estimé :** 297 (§ Taille estimée). Marge 31 %, pour un biais
>   d'estimation mesuré à 16 % sur le vol précédent.

**Le but.** Un callback **`completed`** de la conversation en cours arrive par le
SSE, se lit par HTTP, se réclame et se livre en `AgentRequest::CallbackResult`,
une seule fois.

Ce vol est le second temps de la voie A4 tranchée par Vincent le 2026-11-10. Le
premier temps — le miroir à dix champs, PR A1 — **est mergé sur `main`**
(`a2097da16`, #3070), ce qui retire 41 lignes du total que les quatre grooms de
l'après-midi avaient mesuré à 326. Le reste du but tient sous 429, et ce plan
le mesure livrable par livrable.

---

## 0 — Ce qui est mesuré, avant toute conception

Chaque ligne de cette section est une mesure faite dans ce worktree aujourd'hui,
pas un rappel. Les postes du § Taille estimée qui s'y adossent portent la
mention « mesuré ».

### 0.1 — L'état de `main` (`a2097da16`)

`git diff --numstat origin/main...HEAD` rend **zéro ligne** : la branche est
l'identique de `origin/main`, et la borne de 429 est donc entière.

Ce dont la couture hérite, et qu'elle n'écrit pas :

| sur `main` | emplacement | forme |
|---|---|---|
| `TaskEventStream::open(base_url, token)` | `commands/tasks.rs:523` | `pub(crate)`, un seul GET, `bail!` sur statut non-succès |
| `TaskEventStream::consume(agent_filter, on_event)` | `commands/tasks.rs:565` | rend la trame décodée, filtrée par agent, `Ok(())` à la fermeture propre |
| `frame_agent_id(&TaskEventFrame)` | `commands/tasks.rs:490` | `match` exhaustif sur huit variantes, **sans bras `_`** |
| `TaskDetailMirror` | `commands/tasks.rs:618-629` | dix champs, dont quatre `pub(crate)` |
| `fetch_task_detail(base_url, token, task_id)` | `commands/tasks.rs:643` | `Err` sur statut non-succès et sur corps illisible, `timeout` 5 s |
| `fake_spirit::spawn(payload)` | `commands/tasks.rs:2084` | sert le flux, **corps fini**, puis ferme |
| `fake_spirit::spawn_detail(statut, corps)` | `commands/tasks.rs:2115` | sert `/api/v1/tasks/{id}`, clone le corps par requête, témoigne de l'en-tête `authorization` |
| `poll_callback_tasks` | `tui/app.rs:1596` | réclame par `mark_task_delivered`, livre, `break` après une livraison |
| `mark_task_delivered(&str) -> Result<bool>` | `task_state/tasks.rs` via `AsyncDatabase` | `Ok(false)` = déjà réclamée |

Deux propriétés mesurées rendent deux postes du chiffrage irréductibles :

1. **`TaskDetailMirror::id` est privé.** Les quatre `pub(crate)` sont `label`,
   `result`, `created_trace_id` et `parent_task_id`. Or la garde 3 réclame par
   `mark_task_delivered(&id)` et la garde 4 envoie `CallbackResult { task_id }` :
   le drainage lit donc **cinq** champs, pas quatre. Ce vol fait passer `id` en
   `pub(crate)` et met à jour la phrase du doc-commentaire qui énumère les
   lecteurs champ par champ — coût mesuré **4 lignes**, et l'énoncé « chaque
   champ a son lecteur, et pas un de plus » reste vrai.
2. **`NewTask` porte 22 champs et n'a aucun `impl Default`**
   (`task_state/tasks.rs:1050-1078`, `grep -c 'impl Default for NewTask'` rend
   `0`). Le helper qui ensemence une ligne de callback en base pour les tests du
   drainage coûte donc **36 lignes plancher**, non une approximation : 22 lignes
   pour le seul littéral, plus la signature, l'ouverture d'une base en mémoire,
   le passage du statut à `completed` et la relecture de l'identifiant. Le seul
   précédent du dépôt sur exactement cette forme — `seed_callback_row`,
   `commands/tasks.rs:1509-1562` — en fait la démonstration en 53 lignes, et il
   est de surcroît `#[cfg(target_os = "linux")]` et paramétré pour un pgid : il
   n'est pas réutilisable ici.

   *Correction de mesure.* Le plan matériau `3d90c7ba9` § 0.1 écrit 23 champs ;
   la mesure d'aujourd'hui en rend 22. Le plancher de 36 ne bouge pas pour une
   ligne, mais le chiffre juste est 22.

### 0.2 — Egress Manifest Lint, mesuré maintenant

`bash scripts/verify-egress-manifest.sh --report` rend, sur ce worktree :

```
egress-manifest: 400 fichier(s) .rs de production scanné(s), 31 portant une
construction de client, 22 host(s) distinct(s) rencontré(s), 31 entrée(s)
[[sink]] confrontée(s).
No egress-manifest violations found.
```

**Vert, `docs/egress/` intact.** Ce que la couture y change est la *véracité* du
manifeste, non sa couleur : la lecture HTTP neuve naît dans `commands/tasks.rs`,
que la déclaration `cli-spirit-tasks` couvre déjà, et aucun client neuf
n'apparaît dans `tui/app.rs`. Le critère 4 demande au pilote de relancer cette
commande avant d'ouvrir la PR ; s'il la trouve rouge, il s'arrête et le dit, au
lieu de toucher le manifeste.

### 0.3 — La calibration de l'estimateur

Biais d'estimation mesuré sur le vol précédent (V2b-2b-1) : **16 %**. Le total de
ce plan est jugé contre lui : 297 × 1,16 = **345**, soit 84 lignes sous la borne.
Les deux postes les plus gros du plan — `CallbackFeed` (66) et
`drain_callback_deliveries` (38) — ne sont pas extrapolés : le matériau les a
écrits ligne à ligne, et la refonte de la couture explique la baisse du second
(§ 2).

### 0.4 — Le fait qui ferme la porte du détecteur structurel

Le critère 5 demande que `tui/app.rs` **ne contienne** ni l'URL de mika-spirit,
ni le jeton, ni d'appel à `fetch_task_detail`, et nomme lui-même sa preuve : « le
diff en fait foi ».

Mesure : **`tui/app.rs` contient déjà, sur `main`, l'URL et le jeton**. Le chemin
des boutons du tableau de bord (`tui/app.rs:994-1010`) appelle
`crate::commands::dashboard::auth_token()`, `dashboard::spirit_url()` et
construit un `reqwest::Client`.

Conséquence, et c'est une décision de ce plan : **aucun scan structurel de source
sur `tui/app.rs` n'est livré.** Un scan de la forme habituelle du dépôt —
allowlist livrée vide, épinglée vide — serait **rouge à l'atterrissage** sur ce
chemin préexistant, et l'amnistier demanderait une entrée d'allowlist, ce que la
doctrine du dépôt refuse (mika#2201 : quand le scan rougit, on fait passer le
nouvel appelant par le site existant, on n'ajoute pas d'entrée). La preuve du
critère 5 est donc celle que le critère énonce — le diff — doublée de la preuve
**par le type** : `CallbackFeed` détient l'URL et le jeton, et la seule chose que
`tui/app.rs` peut en obtenir est un candidat déjà lu. Un appel HTTP depuis
`tui/app.rs` ne serait pas interdit par une règle, il serait sans objet.

Ce constat est à verser à la décision de l'architecte : il contredit la lecture
« le critère 5 livre un détecteur » que les passes antérieures ont retenue.

---

## 1 — La couture, et ce qu'elle rend vrai

Un type dans `commands/tasks.rs` porte ensemble les trois éléments indissociables
du fil, et expose à la TUI **un seul geste**.

```rust
pub(crate) struct CallbackFeed {
    rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    base_url: String,
    token: String,
}
```

- **`open_for(agent_id, base_url, token) -> Option<Self>`** ouvre le flux **une
  fois**, sans recul ni reconnexion, et `tokio::spawn` un drain qui pousse sur le
  canal les `task_id` que `terminal_callback_frame` reconnaît. Un `open` en
  erreur rend `None` ; le poll couvre. L'URL et le jeton voyagent **par
  argument**, pour la raison déjà écrite sur `TaskEventStream::open` et
  `fetch_task_detail` : un consommateur qui résout ses propres accès ne se pilote
  pas depuis un test.
- **`next_candidate(&mut self, agent_id, session_id) -> Option<TaskDetailMirror>`**
  enchaîne les gardes 1 et 2 : `try_recv`, puis la lecture HTTP du détail, puis
  le prédicat du poll. `None` couvre les quatre refus — canal vide, détail en
  erreur HTTP, détail illisible, prédicat faux — et **aucun ne réclame**, ce qui
  est exactement « je passe et je ne réclame pas », la ligne restant `completed`
  pour le poll.
- **`terminal_callback_frame(Option<&TaskEventFrame>) -> Option<String>`** rend
  le `task_id` d'une trame `TaskCompleted`, et `None` partout ailleurs. Son
  `match` est exhaustif sur les huit variantes, **sans bras `_`**, comme
  `frame_agent_id` : c'est la garde structurelle qui fait rougir la compilation
  le jour où une neuvième variante entre côté spirit, au lieu de l'avaler.
  `TaskFailed` y rend `None` — hors de ce vol, le poll la livre.

**Pourquoi le canal ne porte que le `task_id`.** `TaskFailed` étant hors
périmètre, l'`original_status` d'une livraison vaut toujours `"completed"` et n'a
pas à voyager. Le couple `(task_id, statut)` du matériau tombe.

**Le prédicat reste une méthode d'ici.** Les quatre conjonctions —
`agent_id`, `trigger_type = "callback"`, `action_type = "resume_agent"`,
`created_by_session == session_id` — vivent dans `next_candidate`, qui est leur
seul appelant, et les cinq champs qu'elles lisent restent **privés** au module.
Aucune visibilité n'est élargie pour le prédicat ; seul `id` l'est, parce que le
drainage le lit (§ 0.1).

**Pourquoi `session_id` voyage par appel et non dans le type.** La session de la
TUI est mutable en cours de vie : `chat.rs:778` et `chat.rs:882` réaffectent
`app.session_id` sur `/new` et sur un changement d'agent. Un `session_id` figé à
l'ouverture livrerait, après un `/new`, les callbacks de l'ancienne conversation
— exactement ce que le critère 2 interdit. Il est donc lu à chaque tick.

**Ce que la couture ne fait pas.** Elle ne touche pas `poll_callback_tasks`
(critère 4), ne modifie ni `crates/mika-agent/` ni `crates/mika-a2a/`, laisse
`docs/egress/` intact, et n'ajoute **aucun constructeur de serveur factice**
(§ 4). Elle ne supprime pas non plus la duplication des gestes de livraison avec
le poll : cette duplication est assumée pour un vol, V2b-3 retirant le poll et
elle avec.

---

## 2 — Les quatre gardes et leur domicile

| garde | ce qu'elle fait | domicile |
|---|---|---|
| 1 | lecture du détail par `fetch_task_detail` ; un `Err` donne « je passe et je ne réclame pas » | derrière la couture, dans `next_candidate` |
| 2 | prédicat du poll sur le détail : agent, `callback`, `resume_agent`, session de la TUI | derrière la couture, dans `next_candidate` |
| 3 | réclamation par `mark_task_delivered` ; `Ok(false)` ⇒ on passe | `tui/app.rs`, dans `drain_callback_deliveries` |
| 4 | livraison comme le poll : un `completed` sans résultat est sauté ; sinon message système, `CallbackResult` avec `original_status = "completed"`, puis `Thinking` | `tui/app.rs`, dans `drain_callback_deliveries` |

`drain_callback_deliveries` tombe à 38 lignes parce que les deux premières gardes
sont parties dans `next_candidate` : il ne reste que le
`let Some(detail) = … else { return }`, le `match` sur `mark_task_delivered`, le
saut d'un résultat vide, et la livraison — dont la forme est copiée sur celle de
`poll_callback_tasks:1650-1700`, à l'identique, pour que les deux chemins livrent
le même objet.

**Le site d'appel.** Au tick, au repos, hors mode équipe, **sans attendre
`POLL_INTERVAL_TICKS`**, avec **une livraison par tick au plus** (`break` après
la première, comme le poll). Il se greffe dans le bloc de tick existant
(`tui/app.rs:1090-1126`), à côté de la condition
`self.tick_count.is_multiple_of(POLL_INTERVAL_TICKS)` et non dedans — 7 lignes
mesurées.

**Le site d'abonnement.** Dans `chat::run` (`commands/chat.rs:520-545`), avant
`App::new`, et **non** dans `run_team` (`chat.rs:965`) : le mode équipe n'ouvre
donc rien, et `App::new_team` reçoit `None`. C'est là que
`dashboard::auth_token()` est interrogé — un jeton absent n'ouvre rien — et que
`dashboard::spirit_url()` est lu. `App::new` prend un quinzième paramètre,
`Option<CallbackFeed>` ; ses deux appelants sont `chat.rs:545` et le helper de
test `tui/app.rs:2183`.

---

## 3 — L'ordre de travail : deux commits, chacun vert et secourable

Le corps impose que le pilote commite après chaque garde verte avec son test, et
non en fin de vol. Les gardes se regroupent par domicile, ce qui donne deux
commits.

### Commit 1 — la couture (gardes 1 et 2), `commands/tasks.rs` seul

`terminal_callback_frame`, `CallbackFeed`, le miroir rend `id`, le constructeur
de test `from_channel`, et les tests T1 et T2. Rien n'est encore livré.

**La chaîne d'appel est linéaire, donc ce commit laisse `open_for` et
`next_candidate` sans appelant hors test.** Le pilote le traite par un
`#[expect(dead_code, reason = "…")]` sur l'`impl` — **2 lignes** — et non par un
`#[allow]` : `expect` rougit le jour où l'attribut devient inutile, donc le
commit 2 est *obligé* de le retirer. L'exception s'éteint par le geste qui la
rend inutile.

Le découpage « l'abonnement dans ce vol, le drainage dans le suivant » n'est
**pas** candidat : le corps le ferme nommément (« un abonnement dont personne ne
draine le canal, ou un drainage sans livraison, ne passe pas non plus,
mika#2205 »). Les deux commits vivent dans la **même PR**.

### Commit 2 — le drainage (gardes 3 et 4), `commands/chat.rs` et `tui/app.rs`

Le site d'abonnement, le champ `callback_feed` et ses deux inits,
`drain_callback_deliveries`, le site d'appel au tick, le helper d'ensemencement,
`app_with_response_sender` qui rend le receveur, et les tests T3 et T4. Le
`#[expect(dead_code)]` du commit 1 part ici.

Un vol arrêté entre les deux laisse une couture testée, pas un reste à secourir.

---

## 4 — Le jeu de tests : huit propriétés, quatre tests

Le corps est sans ambiguïté : « Chaque garde arrive avec ses tests, dans le même
commit. Les tests ne se reportent jamais à un vol suivant, même si le code
reste. » Si le budget ne tenait pas, c'est le périmètre qui se réduirait, jamais
les tests — il tient (§ Taille estimée).

| propriété exigée | critère | test | où | serveur factice |
|---|---|---|---|---|
| une trame d'un autre agent ne mène à rien | 2 | T1 | `tasks.rs` | `spawn` |
| `TaskFailed`, `Unknown`, `OverflowMarker` ignorées sans casser l'abonnement : un `TaskCompleted` qui suit arrive encore | 3 | T1 | `tasks.rs` | `spawn` |
| `trigger_type = callback` et `action_type = resume_agent` exigés | 2 | T2 | `tasks.rs` | `spawn_detail` |
| une autre session du même agent n'est ni livrée ni réclamée | 2 | T2 | `tasks.rs` | `spawn_detail` |
| détail en erreur HTTP : ni livré ni réclamé | 3 | T2 | `tasks.rs` | `spawn_detail` |
| détail illisible : ni livré ni réclamé | 3 | T2 | `tasks.rs` | `spawn_detail` |
| un `completed` de la session courante est livré **une** fois, avec `original_status`, le résultat du détail, `trace_id` et `parent_task_id` | 1 | T3 | `app.rs` | `spawn_detail` |
| rien n'est livré si le poll a déjà réclamé (`Ok(false)`) | 1 | T4 | `app.rs` | `spawn_detail` |

**Six propriétés sur huit se mesurent sans base et sans `App`.** C'est l'effet
principal de la couture : sous la forme du matériau, « autre session », « pas un
callback » et « détail illisible » exigeaient chacune une `App` et une ligne en
base, parce que le prédicat et la lecture HTTP y vivaient. Le helper
d'ensemencement de 36 lignes ne sert donc que **deux** tests au lieu de six.

T1 porte deux propriétés en une seule séquence SSE de six trames, servie par
`spawn` et drainée par un `consume` dont le `on_event` collecte ce que
`terminal_callback_frame` reconnaît. T2 porte ses quatre refus et son témoin
positif, paramétré par un corps de détail servi par `spawn_detail`.

**Le corps fini de `spawn` suffit, et c'est pourquoi aucun constructeur neuf de
serveur factice n'entre.** `spawn` sert sa charge puis ferme, donc `consume` rend
`Ok(())` et T1 assied une séquence entière sans minuterie. La variante au flux
jamais fermé est reportée au vol du recul et de la reconnexion, que le corps
nomme — aucun des cinq critères ne la demande.

**Un seul constructeur neuf entre, et il n'est pas un serveur factice :**
`#[cfg(test)] pub(crate) fn CallbackFeed::from_channel(base_url, token) -> (UnboundedSender<String>, Self)`,
6 lignes. Il existe parce que `CallbackFeed` porte **une** URL de base — la
propriété même qui rend vrai le critère 5 par le type — alors que `spawn` et
`spawn_detail` servent deux routes sur deux serveurs. Les critères qui ne
tiennent pas sans lui sont **le 1 et le 2** : T2, T3 et T4 doivent alimenter le
canal eux-mêmes pour choisir le `task_id` dont ils observent le refus ou la
livraison, en pointant la lecture du détail sur un `spawn_detail`.

---

## 5 — Ce que ce vol ne fait pas, et les limites nommées

- **`poll_callback_tasks` reste inchangé** (critère 4). C'est le filet : il livre
  toujours les `failed`, et tout `completed` que le flux manque.
- **Dette nommée, non traitée ici.** `mark_task_delivered` reste une écriture
  locale sur la base, alors que le reste du chemin passe par HTTP. La PR le dit
  et ne le fait pas.
- **L'abonnement ne suit pas un changement d'agent.** Le filtre de `consume` est
  figé à l'ouverture ; après un `/agent` (`chat.rs:866-882`), le fil reste celui
  de l'agent initial. Rien n'est mal livré — le prédicat revérifie l'agent sur le
  détail — mais les callbacks du nouvel agent ne passent plus que par le poll.
  Limite assumée pour ce vol.
- **Aucun fichier du plan de contrôle** : `.github/workflows/`, `.claude/`,
  `skills/bundled/`, `crates/mika-agent/src/well_known_agents.rs`, `.mika/`, ni
  aucun chemin de CODEOWNERS. `docs/egress/` n'est pas touché.
  `crates/mika-agent/` et `crates/mika-a2a/` ne sont pas modifiés.
- **Aucun critère n'exige de toucher un fichier exclu.** Le seul point où la
  question s'est posée est le critère 5 : il est tenu par le diff et par le type,
  sans scan structurel, donc sans `.github/workflows/` ni `scripts/` (§ 0.4).
- **Gel des tickets jusqu'au 01/12.** Ce vol n'ouvre aucun ticket, pas même de
  suivi. La suite — livraison des `failed` avec `FAILED_TASK_FALLBACK` et la
  question de la garde d'ancienneté ; le recul, la reconnexion et la variante du
  serveur factice au flux jamais fermé ; V2b-3 (retrait de
  `poll_callback_tasks`), V2c, V3 et V4 — se note :

  `Tracked in: senara-solutions/mika#1727`

- **La PR porte `Refs mika#1727`**, et aucun `Closes`, `Fixes` ni `Resolves`, ni
  dans le corps ni dans un commit. Le ticket se ferme au merge de V4.
- **Matériau, non repris en bloc.** Le plan `3d90c7ba9` (couture § 2, mesure § 3
  et § 4, tests § 3.1) et le commit `5321ba47d` de #3069. Ce plan en garde la
  mesure, corrige le compte de champs de `NewTask`, ajoute les deux postes que la
  couture rend nécessaires (`id` en `pub(crate)`, `from_channel`), et ferme la
  porte du détecteur structurel que le matériau laissait ouverte.

---

## Fire-Disposition

Ce plan livre **quatre détecteurs** — les tests T1 à T4 du § 4, dont le chemin de
succès est « rien n'est livré ni réclamé ». Disposition retenue :
**(a) exception nommée en allowlist, avec une allowlist vide.**

Ils mesurent une surface **neuve** : un abonnement et un drainage qui n'existent
pas sur `main`. Aucune violation préexistante n'est donc à amnistier, aucune
entrée grep-visible n'est due, et l'allowlist est livrée vide et épinglée vide.
Les quatre atterrissent **armés et verts** ; un rouge à l'atterrissage y
signalerait un défaut de l'implémentation, pas une dette héritée. Ni `#[ignore]`
(b) ni halte-et-remontée (c) n'a lieu d'être.

**Un cinquième détecteur n'est pas livré, et c'est délibéré.** Un scan structurel
asserant que `tui/app.rs` ne porte ni URL, ni jeton, ni appel à
`fetch_task_detail` serait **rouge dès l'atterrissage** : le chemin des boutons du
tableau de bord (`tui/app.rs:994-1010`) porte déjà les trois sur `main` (§ 0.4).
Sa disposition serait (a) avec une allowlist **non vide**, c'est-à-dire
exactement l'entrée que mika#2201 refuse. Le critère 5 est donc tenu par la
preuve qu'il nomme lui-même — le diff — et par la couture, qui rend l'appel
impossible faute d'URL et de jeton dans `tui/app.rs`.

Le `#[expect(dead_code)]` du commit 1 n'est pas une exception d'allowlist : il
s'éteint de lui-même au commit 2, qui est dans la même PR (§ 3).

Le gel des tickets jusqu'au 01/12 interdisant d'ouvrir un suivi d'armement, la
suite se note `Tracked in: senara-solutions/mika#1727`.

---

## Definition of Done

1. Les cinq `## Acceptance criteria` ci-dessous sont tenus, chacun par le ou les
   tests que le § 4 lui associe.
2. Deux commits sur `feat/1727/tui-v2b2b2-livraison-completed-par-le-sse`, dans
   l'ordre du § 3, **chacun vert** sous `cargo test -p mika-cli`,
   `cargo clippy --all-targets -- -D warnings` et `cargo fmt --check`.
3. `poll_callback_tasks` n'apparaît pas au diff, autrement que par la ligne de
   contexte du site d'appel voisin.
4. `bash scripts/verify-egress-manifest.sh --report` est vert **avec
   `docs/egress/` intact**, relancé par le pilote avant d'ouvrir la PR. Rouge ⇒
   il s'arrête et le dit, au lieu de toucher le manifeste.
5. `bash scripts/verify-pipeline.sh origin/main` est vert.
6. `git diff --numstat origin/main...HEAD` rend **au plus 429 lignes ajoutées**,
   plan exclu. Le pilote le mesure avant d'ouvrir la PR et l'écrit dans le corps.
7. Le diff ne touche aucun fichier de la liste d'exclusion du § 5, et
   `tui/app.rs` n'y gagne ni URL, ni jeton, ni appel à `fetch_task_detail`.
8. La PR porte `Refs mika#1727`, nomme la dette `mark_task_delivered` et la
   limite du changement d'agent (§ 5), et porte
   `Tracked in: senara-solutions/mika#1727`.
9. `Code review: delegated to mika-qa (mika#2694)` dans le corps de la PR, et ni
   `ce-simplify-code` ni `ce-code-review` n'est invoqué (le vol part de la boucle
   autonome).

---

## Acceptance criteria

Transcrits verbatim du corps de mika#1727, § « Critères de V2b-2b-2 ».

1. Un callback `completed` de la conversation en cours atteint la TUI en
   `AgentRequest::CallbackResult` **une seule fois**. Il porte
   `original_status = "completed"`, le résultat lu par
   `GET /api/v1/tasks/{task_id}`, et `trace_id` et `parent_task_id` tirés du
   détail. Un test rougit si le même candidat est livré deux fois. Un autre
   rougit s'il est livré alors que le poll l'a déjà réclamé.
2. Rien n'est livré ni réclamé dans trois cas : une tâche d'une **autre
   session** du même agent, une tâche qui n'est pas `callback` / `resume_agent`,
   ou une trame d'un **autre agent**. Chacun des trois a son test.
3. Une trame `TaskFailed`, `Unknown` ou `OverflowMarker` ne mène à aucune
   livraison et ne casse pas l'abonnement : un `TaskCompleted` qui la suit
   arrive encore. Un détail en erreur HTTP ou illisible n'est ni livré ni
   réclamé, et la ligne reste `completed` pour le poll. Chaque cas a son test.
4. `poll_callback_tasks` n'est pas modifié. Egress Manifest Lint est vert **avec
   `docs/egress/` intact** : le pilote lance
   `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il
   le trouve rouge, il s'arrête et le dit, au lieu de toucher le manifeste.
5. La PR porte `Refs mika#1727` et aucun `Closes`, `Fixes` ni `Resolves`.
   **`tui/app.rs` ne contient ni l'URL de mika-spirit, ni le jeton, ni d'appel à
   `fetch_task_detail`** : le diff en fait foi.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-cli/src/commands/tasks.rs` — `terminal_callback_frame`, `match` exhaustif sur huit variantes sans bras `_` | 17 |
| `crates/mika-cli/src/commands/tasks.rs` — `CallbackFeed` + `open_for` + `next_candidate` | 66 |
| `crates/mika-cli/src/commands/tasks.rs` — `TaskDetailMirror::id` passe en `pub(crate)` + doc des lecteurs (mesuré) | 4 |
| `crates/mika-cli/src/commands/tasks.rs` — `#[expect(dead_code)]` du commit 1, retiré au commit 2 | 2 |
| `crates/mika-cli/src/commands/tasks.rs` — `CallbackFeed::from_channel`, `#[cfg(test)]` | 6 |
| `crates/mika-cli/src/commands/chat.rs` — site d'abonnement dans `chat::run`, jeton absent compris | 12 |
| tests (`crates/mika-cli/src/commands/tasks.rs`) — T1 abonnement filtrant, séquence de six trames | 22 |
| tests (`crates/mika-cli/src/commands/tasks.rs`) — T2 quatre refus + témoin positif, paramétré par un corps de détail | 34 |
| `crates/mika-cli/src/tui/app.rs` — champ `callback_feed` + les deux inits de constructeur | 9 |
| `crates/mika-cli/src/tui/app.rs` — `drain_callback_deliveries` (gardes 3 et 4) | 38 |
| `crates/mika-cli/src/tui/app.rs` — site d'appel au tick, au repos, hors mode équipe (mesuré) | 7 |
| tests (`crates/mika-cli/src/tui/app.rs`) — helper d'ensemencement d'une ligne de callback (`NewTask`, 22 champs, sans `Default`) | 36 |
| tests (`crates/mika-cli/src/tui/app.rs`) — `app_with_response_sender` rend le receveur + ses appelants | 6 |
| tests (`crates/mika-cli/src/tui/app.rs`) — T3 livré une fois et pas deux, T4 déjà réclamé | 38 |

Total estimé : 297 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Le total tient, avec 132 lignes de marge, soit 31 %.** Jugé contre le biais
d'estimation mesuré de 16 % sur le vol précédent (§ 0.3), il atterrit à 345, soit
84 lignes sous la borne. Le chemin : les quatre grooms de l'après-midi avaient
mesuré 326 sous une borne de 279 ; la voie A4 a porté la borne à 429 par
`loop-substrate`, et la PR A1 a retiré du total les 41 lignes du miroir, déjà
mergées (`a2097da16`). Les 12 lignes que ce plan ajoute au reste de 285 sont les
deux postes que la couture rend nécessaires — `id` en `pub(crate)` (4),
`from_channel` (6) — et l'attribut auto-expirant du commit 1 (2). Aucune n'est
une reprise de périmètre : le but, les cinq critères et les huit propriétés
testées sont ceux du corps.

Les commentaires comptent dans ce total, à ≈ 24 % du code neuf. Ils disent le
pourquoi qu'on ne lit pas dans le code — pourquoi l'abonnement part de
`chat::run`, pourquoi le canal ne porte que le `task_id`, pourquoi `session_id`
voyage par appel, pourquoi aucun des quatre refus ne réclame — et non la
paraphrase du code.

Tracked in: senara-solutions/mika#1727
