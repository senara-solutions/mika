# mika#1727 — vol V2b-2a : le fil des tâches se lit décodé et filtré par agent, avec son premier lecteur

- **Ticket :** senara-solutions/mika#1727
- **Branche :** `feat/1727/tui-v2b2-callbacks-par-le-sse` (neuve, depuis `main`)
- **Base :** `4921ba99e` (merge de V2b-1b, mika#3065)
- **PR :** `Refs mika#1727` — **jamais `Closes`**. Le ticket se ferme au merge de V4.
- **Matériau mesuré :** `refs/pull/3062/head`, commits `6f895f4e9` (le chemin de livraison,
  +294 dans `tui/app.rs`) et `1f24523a7` (+434 de tests dans `tui/app.rs`).
- **Suite :** `Tracked in: senara-solutions/mika#1727` — l'abonnement de la TUI, la lecture du
  détail, la livraison et la correction egress partent en **V2b-2b** (§ 0.2). Aucun ticket n'est
  ouvert (gel jusqu'au 01/12).

---

## 0 — Ce qui est mesuré sur `4921ba99e`, avant toute conception

Six mesures, et la première décide à elle seule du périmètre.

1. **Le périmètre V2b-2 nommé par le corps pèse ~310 lignes de code, avant le premier test.**
   `6f895f4e9` ajoute **294 lignes** dans `tui/app.rs` (`git show --numstat`), et le corps y
   rajoute les **~47 lignes** que V2b-1b a explicitement renvoyées ici — le filtre par agent (~35)
   et le décodage (~12). Le déménagement du client de détail depuis `app.rs` vers `tasks.rs`
   (exigence du corps) ne retire rien, il déplace. Les tests de `1f24523a7` pèsent **434 lignes**
   pour **deux** tests, là où les critères 1 à 4 en demandent au moins cinq.
2. **`crates/mika-cli/src/commands/tasks.rs` fait 1 627 lignes.** `TaskEventStream` est l. 464,
   `open` l. 478 et son `reqwest::Client::new()` l. 480 — **le seul client du fichier**. `consume`
   l. 509 rend la charge `data:` brute. `drain_events` et `find_event_separator` suivent, attestées
   par les tests de V2b-1b.
3. **`consume` a deux appelants, et un seul relaie des octets.** `stream_task_events_into`
   (l. 433-446) écrit `payload` dans un puits paramétré ; les deux tests bout-en-bout de V2b-1b
   (l. 1011 et l. 1033) assertent une **égalité sur toute la sortie**. Toute signature neuve doit
   donc laisser la charge brute atteignable, sinon ces deux mesures changent de sens.
4. **Les charges des tests de V2b-1b ne décodent pas en `TaskEventFrame`.**
   `{"task_id":"t-1","status":"pending"}` n'a pas de champ `event`, et `TaskEventFrame` est un enum
   à tag interne (`#[serde(tag = "event", rename_all = "snake_case")]`,
   `server/tasks_stream.rs:101`). `#[serde(other)] Unknown` absorbe une **valeur** d'`event`
   inconnue, pas son **absence** : ces trois charges rendent une erreur de décodage, pas `Unknown`.
   C'est cette mesure qui impose la signature du § D2.
5. **Deux variantes du fil ne portent aucun agent.** `OverflowMarker { dropped_count }`
   (`tasks_stream.rs:155`) et `Unknown` (l. 160). Les six autres portent `agent_id`.
6. **`TaskEventFrame` est atteignable depuis `mika-cli`** — `pub mod server` (`lib.rs:62`),
   `pub mod tasks_stream` (`server/mod.rs:32`), `Deserialize` dérivé — et `serde_json` est en
   dépendance **normale** de `mika-cli` (`Cargo.toml:34`), `axum` en `[dev-dependencies]` (l. 68).
   Aucune dépendance à ajouter.

### 0.1 — La borne mord avant les tests, donc le recoupage n'est pas un choix de confort

Le corps borne ce vol à **450 lignes ajoutées**, « code, tests et manifeste compris, plan exclu »,
sans facteur. La règle mika#2693 borne le même vol à **279 lignes estimées**. Comme le plan V2b-1b
(§ 0.1), ce plan écrit son total en lignes **ajoutées** et le soumet **tel quel** aux deux bornes,
sans invoquer en sa faveur le facteur 3 que la dérivation de 279 porte déjà. La cohérence entre
vols d'un même ticket vaut mieux qu'un assouplissement opportuniste.

Et la mesure 1 tranche : **310 > 279 avant d'avoir écrit une ligne de test.** Il n'existe donc
aucune façon de livrer le critère 1 de V2b-2 — « un callback terminé atteint la TUI en
`AgentRequest::CallbackResult` » — dans un vol qui passe sous la borne, parce que ce critère exige
simultanément le filtre, le décodage, l'abonnement, la lecture du détail, le filtre session et la
réclamation. Le corps l'a prévu : « Si la matière dépasse 450 lignes, recouper selon la tête :
chaque partie livre son appelant, la correction d'egress va avec la partie qui ajoute la lecture du
détail. »

### 0.2 — Où passe la coupe, et pourquoi là

La tête du corps **nomme** la coupe : « la correction d'egress va avec la partie qui ajoute la
lecture du détail ». Elle sépare donc ce qui lit **le fil** de ce qui lit **le détail**.

| part | contenu | appelant livré dans la même PR | ce vol |
|---|---|---|---|
| **V2b-2a** | filtre par agent (`frame_agent_id`), décodage de la charge en `TaskEventFrame` | `mika tasks stream --agent <id>` | **oui** |
| **V2b-2b** | abonnement (`spawn_task_event_subscription`, recul + reconnexion), variante ouverte du factice, `terminal_callback_frame`, `CallbackTaskDetail`, lecture `GET /api/v1/tasks/{task_id}`, filtre session, réclamation, livraison `CallbackResult`, **correction egress `cli-spirit-tasks`** | la TUI (`tui/app.rs`), drainée au tick | non — `Tracked in: senara-solutions/mika#1727` |

**Trois coupes ont été écartées, chacune pour une raison mesurée :**

- **« Le détail avec le filtre, la livraison après. »** `fetch_callback_task_detail` n'aurait aucun
  lecteur dans sa PR : rien, en V2b-2a, ne consomme un `label`, un `result` ou un
  `created_by_session`. C'est exactement la faute que mika#2205 nomme et que le corps interdit —
  « une lecture HTTP sans lecteur dans la même PR ne passe pas ».
- **« L'abonnement avec le filtre, la livraison après. »** Même faute : un abonnement qui pousse des
  trames sur un canal que personne ne draine. Et le corps le dit mot pour mot.
- **« Tout le code dans ce vol, les tests des critères 3 et 4 dans le suivant. »** Refusée pour deux
  raisons : la mesure 1 la rend arithmétiquement impossible (310 > 279 sans test), et elle ferait
  atterrir du code non attesté — l'inverse de ce à quoi sert une borne.

**Ce que V2b-2a satisfait des cinq critères du corps**, et ce qu'il ne touche pas, est tabulé au
§ *Correspondance AC → critères du corps*. En une phrase : il livre les ~47 lignes que V2b-1b avait
renvoyées, **avec un premier lecteur réel**, et la moitié « décodage » du critère 3.

### 0.3 — `mika tasks stream --agent` est un lecteur réel, pas un prétexte

Le fil est **par processus, pas par agent** — un unique `broadcast` du processus, dont chaque trame
porte son `agent_id` précisément parce que le fil ne les sépare pas (`tasks_stream.rs`, et le corps
du ticket le nomme). « Quelles trames sont celles de cet agent » est donc la première question de tout
consommateur du fil — la TUI en V2b-2b, et l'opérateur dès aujourd'hui, sur une commande dont le
doc-comment dit déjà qu'elle est un diagnostic (`cli.rs:1105-1108`). Le flag n'invente pas une
surface produit : il expose sur l'instrument existant le filtre que le fil rend nécessaire, et il
le fait **sans aucune requête nouvelle** (§ 0.5).

Ce n'est pas un lecteur de complaisance, et la mesure qui le montre est son test : le filtre est
attesté **de bout en bout, sur la sortie exacte** de la commande (§ 3, test T4), par la même forme
d'égalité-sur-tout-le-puits que V2b-1b a installée pour « une ligne par charge et rien d'autre ».
Un filtre qui laisserait fuir une trame étrangère fait rougir cette égalité.

### 0.4 — Ce que ce vol ne touche pas

`crates/mika-cli/src/tui/app.rs` n'est **pas modifié** — aucune ligne. `crates/mika-agent/` et
`crates/mika-a2a/` ne sont pas modifiés : `TaskEventFrame` est consommé tel quel. `poll_callback_tasks`
(`app.rs:1596`) reste en place, inchangé, et reste le seul chemin de livraison des callbacks de la
TUI à la fin de ce vol — son retrait est V2b-3, et sa dette (`mark_task_delivered` est une écriture
**locale** sur la base, qui doit passer par mika-spirit) n'est ni traitée ni aggravée ici.

Aucun fichier du plan de contrôle : `.github/workflows/`, `.claude/`, `skills/bundled/`,
`crates/mika-agent/src/well_known_agents.rs`, `.mika/`. **Et pas `docs/egress/` non plus** (§ 0.5) :
l'exception que le corps ouvre pour `cli-spirit-tasks` part avec V2b-2b, qui ajoute la requête
qu'elle doit déclarer.

### 0.5 — Pourquoi Egress Manifest Lint reste vert sans toucher le manifeste

Trois faits, chacun vérifiable sur le scanner (`scripts/lib/egress_manifest_lint.py`) :

- **D1 (code → manifeste).** `CLIENT_CONSTRUCTION_RE` (l. 148-152) inventorie les fichiers qui
  portent `Client::new()`. Ce vol n'en ajoute **aucun** : le seul client du chemin reste celui de
  `TaskEventStream::open` (`tasks.rs:480`), déjà couvert par `client_site` de `cli-spirit-tasks`
  (`egress-manifest.toml:352-360`, `client_site` l. 358). Aucun fichier neuf n'est créé.
- **D3 (manifeste → code).** Le `client_site` et le `call_site` déclarés continuent de pointer sur
  un fichier de l'inventaire D1. C'est très exactement la D3 qui rougissait sur #3062, où le client
  avait déménagé vers un fichier neuf et la déclaration était devenue fantôme.
- **D2 (littéraux de host).** Aucun littéral neuf : les tests réutilisent `fake_spirit::spawn`, qui
  lie `"127.0.0.1:0"` (couvert par `LOOPBACK_V4_RE`, l. 252) et sert sur `format!("http://{addr}")`,
  forme dont `normalize_host` rend `None`.

**Et le champ `data` reste exact.** Il dit « ouverture d'un flux SSE de tâches ; aucun corps envoyé
au-delà du jeton ». Ce vol n'ajoute ni requête, ni corps, ni paramètre de requête : le filtre
`--agent` est appliqué **côté client, sur le fil déjà reçu** (§ D3), et pas en query. La correction
du champ devient due au vol qui ajoute la lecture du détail, et elle y va — c'est l'ordre que la
tête du corps impose.

Le pilote lance quand même `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR.
**S'il le trouve rouge, il s'arrête et le dit** : corriger le manifeste est le geste de Vincent
(CODEOWNERS `@samidarko`), et un lint supposé vert est la classe de faute que mika#2205 nomme.

---

## 1 — Les points de conception

### D1 — `frame_agent_id` est exhaustive par variante, et c'est ça le livrable

```rust
fn frame_agent_id(frame: &TaskEventFrame) -> Option<&str>
```

Un `match` qui **énumère les huit variantes**, sans bras `_`. Ce n'est pas une préférence de style :
le fil est additif par contrat — « new variants may ship additively » dit le doc de
`TaskEventFrame` (`tasks_stream.rs:98`), et il nomme déjà mika#1734 et mika#1736. Avec un
bras `_ => None`, une variante neuve portant un `agent_id` tomberait **silencieusement** du bon
côté du filtre d'un autre agent. Avec l'énumération, elle ne compile plus ici, et le compilateur
nomme le site à décider.

`None` pour `OverflowMarker` et `Unknown` : ces deux-là ne portent aucun agent (mesure 5), et le
§ D4 dit ce que le filtre en fait.

### D2 — `consume` rend la charge **et** son décodage facultatif, parce que ses deux appelants n'en lisent pas la même chose

```rust
pub(crate) async fn consume(
    mut self,
    agent_filter: Option<&str>,
    mut on_event: impl FnMut(&str, Option<&TaskEventFrame>),
) -> Result<()>
```

Le second paramètre de la closure est un `Option<&TaskEventFrame>`, et non un `&TaskEventFrame`.
C'est la mesure 4 qui l'impose, pas un goût : les charges des deux tests de V2b-1b **ne décodent
pas**, et leur propriété — « une ligne par charge `data:` et rien d'autre » — porte sur la charge
**brute**. Une signature qui n'offrirait que la trame décodée forcerait soit à réécrire ces deux
mesures (donc à perdre ce qu'elles attestent), soit à faire taire les charges indécodables de
`mika tasks stream` — c'est-à-dire à rendre aveugle l'instrument dont le rôle est précisément de
montrer ce qui passe sur le fil.

Les deux appelants en lisent des moitiés disjointes, et c'est la preuve que les deux doivent être
là : `stream_task_events_into` lit `payload` et ignore la trame ; la TUI de V2b-2b lira la trame et
ignorera `payload`.

### D3 — Le décodage et le filtre vivent dans `consume`, pas dans `drain_events`

`drain_events` et `find_event_separator` ne bougent **pas d'une ligne**. `consume` enveloppe
`on_event` dans un relais `FnMut(&str)` qu'elle passe à `drain_events` ; le relais décode, filtre,
puis appelle `on_event`.

Deux raisons, dont une est une mesure. La mesure : les tests de recollage et de séparateurs livrés
par V2b-1b appellent `drain_events` directement et assertent sur des charges brutes — les toucher
coûterait des lignes et effacerait ce qu'ils tiennent. La raison de forme : `drain_events` a une
responsabilité, le recollage d'un événement coupé entre deux paquets, et c'est elle qui rend cette
propriété attestable sans dépendre d'un cadencement (§ D3 du plan V2b-1b). Y mettre un décodage la
ferait porter deux choses, dont une sans rapport avec les octets.

### D4 — Le filtre n'écarte que ce qu'il a **compris** comme étranger

Trois cas, et un seul écarte :

| charge | `frame_agent_id` | sous `Some("mika-dev")` |
|---|---|---|
| trame décodée, `agent_id = "mika-qa"` | `Some("mika-qa")` | **écartée** |
| trame décodée, `agent_id = "mika-dev"` | `Some("mika-dev")` | rendue |
| `OverflowMarker`, `Unknown`, ou charge indécodable | `None` / pas de trame | **rendue** |

Le troisième cas est le point de conception. `OverflowMarker` dit à un consommateur **filtré** qu'il
a perdu des trames, dont peut-être les siennes : l'écarter serait perdre la seule information qui
dit que l'information manque. `Unknown` et l'indécodable, eux, sont rendus par symétrie avec D1 :
le filtre est un **premier étage gratuit** — il économise un aller-retour quand il sait, et il ne
prétend pas à la frontière quand il ne sait pas. Ce qui tranche pour de bon, en V2b-2b, est
l'étage session, qui lit `created_by_session` sur le détail.

Le coût de ce choix est borné et nommé : sous filtre, un consommateur peut voir une trame qui n'est
pas la sienne si elle est indécodable. Il ne peut pas en voir une qui est décodable et étrangère.

### D5 — Les charges de test sont des JSON littéraux, pas des `TaskEventFrame` sérialisés

Le matériau encodait ses charges par `serde_json::to_string` du type
(`1f24523a7`, `fn sse_payload(frames: &[TaskEventFrame])`). Ce vol écrit les JSON **à la main** :

```rust
r#"{"event":"task_completed","task_id":"t-1","agent_id":"mika-dev","completed_at":"2026-11-09T10:00:00Z"}"#
```

Un aller-retour `to_string` → `from_str` du même type ne mesure que la symétrie de serde avec
elle-même : il resterait vert le jour où le tag passerait de `event` à autre chose, ou où
`rename_all` changerait de casse — c'est-à-dire le jour où le consommateur cesserait de comprendre
ce que le serveur émet. Le littéral épingle **le format du fil**, qui est le contrat réel entre
`mika-agent` et `mika-cli`. Et il est lisible : le lecteur du test voit ce qui passe sur la socket.

`fake_spirit::sse_payload(&[&str])`, déjà sur `main`, les encode sans une ligne nouvelle. Le module
`fake_spirit` n'est **pas modifié** par ce vol : sa variante au flux jamais fermé arrive en V2b-2b,
avec le test de reconnexion qui est son unique lecteur — ce que le corps annonce déjà.

### D6 — `--agent` est un `Option<String>`, et son absence est le comportement d'aujourd'hui

`None` ⇒ aucun filtre, sortie identique au bit près à celle de `main`. C'est ce que les deux tests
bout-en-bout de V2b-1b continuent de mesurer, à la seule adaptation de signature près — et c'est
pourquoi ils ne sont pas réécrits.

---

## 2 — La forme livrée

**`crates/mika-cli/src/commands/tasks.rs`**

- `fn frame_agent_id(frame: &TaskEventFrame) -> Option<&str>` — privée au fichier (son seul lecteur
  de production est `consume`, son test est dans le `mod tests` du même fichier). Elle passera
  `pub(crate)` en V2b-2b si et seulement si un lecteur hors du fichier existe alors.
- `TaskEventStream::consume` gagne `agent_filter: Option<&str>`, sa closure devient
  `FnMut(&str, Option<&TaskEventFrame>)`, et son corps gagne le relais de décodage/filtre (§ D3).
  `drain_events` et `find_event_separator` **inchangées**.
- `stream_task_events_into` gagne `agent_filter: Option<&str>` et passe `|payload, _frame|`.
- `stream_task_events` gagne `agent: Option<String>` et le relaie.
- Le bras `Some(TaskCommand::Stream { url })` (l. 397) devient `{ url, agent }`.

**`crates/mika-cli/src/cli.rs`**

- `TaskCommand::Stream` gagne `#[arg(long)] agent: Option<String>`, avec son doc-comment : filtre
  appliqué côté client sur un fil qui est par processus, et non par agent.

**Rien d'autre.** Pas de fichier neuf, pas de dépendance, pas de route, pas d'authentification
touchée, aucune signature publique hors de `tasks.rs`, et `docs/egress/` intact (§ 0.5).

---

## 3 — Les tests

Tous dans le `mod tests` existant de `tasks.rs`. Quatre neufs, deux adaptés.

- **T1 — `mika1727_v2b2a_chaque_variante_du_fil_dit_ou_non_son_agent`.** Les huit variantes
  construites à la main, et l'assertion sur ce que `frame_agent_id` en rend : `Some(id)` pour les
  six qui portent un agent, `None` pour `OverflowMarker` et `Unknown`. Déterministe, aucune socket.
  C'est le test qui attache le **choix** du § D4 à une mesure ; l'exhaustivité du `match`, elle, est
  tenue par le compilateur, pas par lui.
- **T2 — `mika1727_v2b2a_une_trame_dun_autre_agent_est_ecartee_du_fil`.** Trois trames typées sur le
  factice : `mika-dev`, `mika-qa`, `mika-dev`. Sous `Some("mika-dev")`, la séquence rendue est
  exactement les deux premières-et-troisième ; **contrôle négatif dans le même test** : sous `None`,
  les trois passent. Les deux assertions sont des égalités sur la séquence complète, donc une trame
  perdue rougit autant qu'une trame fuitée.
- **T3 — `mika1727_v2b2a_le_filtre_laisse_passer_ce_quil_ne_comprend_pas`.** Quatre charges sous
  `Some("mika-dev")` : un `overflow_marker`, un `event` inconnu (→ `Unknown`), une charge non-JSON,
  et une trame de `mika-qa`. Les trois premières sont rendues, la quatrième non — et l'égalité porte
  sur toute la séquence, ce qui mesure du même coup que `consume` **est allée jusqu'au bout du
  corps** : une charge indécodable qui interromprait la lecture tronquerait la séquence.
- **T4 — `mika1727_v2b2a_mika_tasks_stream_agent_necrit_que_les_lignes_de_lagent_demande`.** Bout en
  bout sur `stream_task_events_into` avec un puits paramétré, et une **égalité sur tout le puits** —
  la forme que V2b-1b a installée pour « rien d'autre ». C'est le test qui fait du flag un lecteur
  mesuré du filtre (§ 0.3).
- **T5, T6 — les deux tests de V2b-1b**, adaptés à la signature et **à rien d'autre** :
  `consume(None, |charge, _| …)` et `stream_task_events_into(&base, "jeton-de-test", None, &mut puits)`.
  Leurs charges et leurs assertions ne changent pas d'un caractère : c'est ce qui mesure que le
  comportement sans `--agent` est celui d'aujourd'hui (§ D6).

**Les tests de recollage et de séparateurs de V2b-1b ne sont pas touchés** — ils appellent
`drain_events` et `find_event_separator`, que le § D3 laisse intactes.

---

## 4 — Fire-Disposition

Ce vol livre **quatre détecteurs** — quatre tests dont le chemin de succès est « aucune violation
trouvée ».

**Option (a) — exception nommée en allowlist, avec une allowlist vide.** Les quatre atterrissent
**armés**, sans `#[ignore]` et sans entrée d'exception, parce qu'il n'existe **aucune violation
existante à exempter** : `frame_agent_id`, le décodage et le filtre sont du code neuf, et le seul
comportement préexistant qu'ils encadrent — la sortie de `mika tasks stream` sans filtre — est
attesté vert sur `4921ba99e` par les deux tests de V2b-1b que T5/T6 reprennent sans changer leurs
assertions. Rien à nommer, rien à référencer en suivi, donc aucune assertion auto-nettoyante à
écrire : « l'allowlist est vide » est ici le résultat, pas une dette. En inventer une entrée
produirait l'allowlist de complaisance que mika#2309 condamne.

**Si T5 ou T6 rougit après l'adaptation de signature, c'est (c) — halte-et-remontée.** Ces deux-là
sont verts sur `4921ba99e` et l'adaptation ne touche que leur appel. S'ils rougissent, le § D2 a
changé un comportement qu'il prétend préserver : ce n'est pas une exception à déclarer, c'est un
défaut de conception à remonter en commentaire sur mika#1727 avant de pousser. Jamais un
`#[ignore]` posé sur la mesure qui vient de contredire le plan.

---

## 5 — Ordre de commit

Plancher de rebase : `4921ba99e`. Chaque commit compile, passe `cargo test -p mika-cli` et
`cargo clippy` seul.

1. **`feat(1727): le fil des tâches se lit décodé et filtré par agent`**
   `frame_agent_id`, le relais de décodage/filtre dans `consume`, la signature du § D2,
   l'adaptation de T5/T6, et T1. `drain_events` et `find_event_separator` **inchangées**.
   T1 vient ici parce qu'il mesure le choix du § D4 au moment où ce choix est fait.
2. **`feat(1727): mika tasks stream prend --agent`**
   Le paramètre de `stream_task_events_into` et de `stream_task_events`, le flag `cli.rs`, le bras
   `TaskCommand::Stream`, et T4 — l'appelant et sa mesure dans le même commit.
3. **`test(1727): le filtre écarte l'autre agent et laisse passer ce qu'il ne comprend pas`**
   T2 et T3, contre le `fake_spirit` déjà sur `main`.

Avant la PR : `cargo test`, `cargo clippy`, `cargo fmt`, les lints du dépôt,
`bash scripts/verify-egress-manifest.sh --report` et `bash scripts/verify-pipeline.sh origin/main`.
Le corps de PR porte `Refs mika#1727`, la ligne
`Code review: delegated to mika-qa (mika#2694)`, la ligne
`Tracked in: senara-solutions/mika#1727` pour la suite du § 0.2, et il nomme la dette non traitée :
`mark_task_delivered` reste une écriture locale sur la base, à faire passer par mika-spirit en
V2b-3 ou après.

---

## 6 — Definition of Done

- [ ] `frame_agent_id` énumère les huit variantes de `TaskEventFrame`, sans bras `_`.
- [ ] `consume` prend `agent_filter: Option<&str>` et une closure
      `FnMut(&str, Option<&TaskEventFrame>)` ; `drain_events` et `find_event_separator` sont
      inchangées à la ligne près.
- [ ] `mika tasks stream --agent <id>` n'écrit que les lignes de cet agent ; sans le flag, la sortie
      est identique à celle de `4921ba99e`.
- [ ] Une charge indécodable, un `OverflowMarker` et un `event` inconnu traversent le filtre et
      n'interrompent pas la lecture.
- [ ] `crates/mika-cli/src/tui/app.rs`, `crates/mika-agent/`, `crates/mika-a2a/` et `docs/egress/`
      ne sont pas modifiés ; aucun fichier du plan de contrôle non plus.
- [ ] `poll_callback_tasks` est en place et inchangé.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check`, `scripts/verify-egress-manifest.sh --report`
      et `scripts/verify-pipeline.sh origin/main` sont verts.
- [ ] Le total ajouté, mesuré par `git diff --numstat origin/main...HEAD` hors plan, est ≤ 279.
- [ ] La PR porte `Refs mika#1727`, `Tracked in: senara-solutions/mika#1727`, et nomme la dette
      `mark_task_delivered`. Aucun ticket n'est ouvert (gel jusqu'au 01/12).

---

## Acceptance criteria

Les critères **de ce vol**. Leur rapport aux cinq critères de V2b-2 écrits dans le corps est tabulé
juste après.

1. **Le fil se lit décodé.** `consume` rend à son appelant, pour chaque charge `data:`, la charge
   brute **et** son décodage en `TaskEventFrame` lorsqu'il réussit. Un appelant qui ne lit que la
   charge brute obtient la sortie d'aujourd'hui, au caractère près.
2. **Une trame d'un autre agent est écartée du fil.** Sous `agent_filter = Some(id)`, une trame
   décodable dont `agent_id != id` n'atteint pas l'appelant. Un test rougit si elle l'atteint, et
   rougit aussi si une trame de `id` est perdue (égalité sur la séquence complète).
3. **Ce que le filtre ne comprend pas le traverse, et ne casse pas la lecture.** Une charge
   indécodable, un `OverflowMarker` et une trame `Unknown` sont rendues sous filtre, et la lecture
   va jusqu'à la fin du corps.
4. **Chaque variante du fil dit, ou non, son agent.** `frame_agent_id` rend l'agent pour les six
   variantes qui en portent un, `None` pour `OverflowMarker` et `Unknown`, et son `match` est
   exhaustif — une variante neuve sur le fil ne compile plus à ce site.
5. **`mika tasks stream --agent <id>` est le premier lecteur du filtre**, attesté de bout en bout
   par une égalité sur toute sa sortie. Sans le flag, la commande se comporte comme sur `main`.
6. **Egress Manifest Lint est vert sans que `docs/egress/` soit touché.** Aucun client, aucun
   fichier et aucune requête ne sont ajoutés (§ 0.5). Le pilote lance
   `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR ; s'il le trouve rouge, il
   s'arrête et le dit.

### Correspondance AC → critères du corps

| critère de V2b-2 (corps) | disposition dans ce vol |
|---|---|
| **1.** un callback terminé atteint la TUI en `CallbackResult`, une seule fois, avec son détail HTTP | **V2b-2b** — exige simultanément abonnement, détail, filtre session et réclamation ; impossible sous la borne (§ 0.1) |
| **2.** une trame d'un **autre agent** n'est pas livrée | **tenu ici**, étage agent : AC2 + AC4 + AC5 |
| **2.** une trame d'une **autre session** du même agent n'est pas livrée | **V2b-2b** — la session ne voyage pas sur le fil (`tasks_stream.rs`), elle se lit sur le détail |
| **3.** `Unknown` / `OverflowMarker` ne cassent pas l'abonnement | **tenu ici** pour la lecture du fil : AC3. La partie « abonnement » part avec l'abonnement |
| **3.** un détail illisible ou en erreur HTTP n'est ni livré ni réclamé | **V2b-2b** — avec la lecture du détail |
| **4.** échec d'ouverture → recul ; fin de flux → reconnexion | **V2b-2b** — avec `spawn_task_event_subscription` et la variante ouverte du factice, son unique lecteur |
| **5.** Egress lint vert avec `cli-spirit-tasks` **corrigée** | **V2b-2b** — la correction accompagne la requête qu'elle déclare, comme la tête du corps l'ordonne. Ici : lint vert, manifeste **intact** (AC6) |

`Tracked in: senara-solutions/mika#1727`

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `frame_agent_id` + doc (`crates/mika-cli/src/commands/tasks.rs`) | 36 |
| relais de décodage + filtre dans `consume`, signature et doc révisées | 26 |
| `stream_task_events_into` / `stream_task_events` : paramètre, câblage, doc | 12 |
| `--agent` sur `TaskCommand::Stream` + doc (`crates/mika-cli/src/cli.rs`) | 7 |
| T1 — les huit variantes et ce qu'elles disent de leur agent | 28 |
| T2, T3, T4 — filtre, tolérance, sortie exacte de `mika tasks stream --agent` | 96 |
| T5, T6 — adaptation des deux tests de V2b-1b à la signature | 4 |

Total estimé : 209 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**L'unité, et pourquoi le nombre n'est pas assoupli.** 209 est une prévision de lignes **ajoutées**
au sens `git diff --numstat origin/main...HEAD`, plan exclu — l'unité que le corps impose. Ce plan
la compare **telle quelle** aux deux bornes, sans invoquer le facteur 3 que la dérivation de 279
porte déjà : 209 ≤ 279 et 209 ≤ 450. La marge sur la plus serrée est de 70 lignes, et elle n'est pas
du remplissage disponible : la coupe du § 0.2 est dictée par « chaque morceau avec son lecteur », pas
par l'enveloppe. Y ajouter le détail ou l'abonnement pour consommer la marge reproduirait exactement
la faute mika#2205 que le corps interdit.

**Provenance des sept postes : mesurés sur le matériau, pas estimés de zéro.** Les deux premiers
postes (62 lignes) sont les **~47 lignes** que le § 0.2 du plan V2b-1b avait chiffrées — le filtre
(~35) et le décodage (~12) — plus les 15 lignes de signature, de relais et de doc que leur promotion
dans `consume` coûte en propre. Les deux suivants (19) sont du câblage neuf : #3062 n'exposait aucun
flag. T1 et le trio T2–T4 (124) sont neufs : le matériau ne tenait le filtre que par un seul test
unitaire de `frame_agent_id`, et n'attestait le flag par rien — il n'existait pas.

**Pourquoi un seul vol tient.** Le vol touche **deux fichiers**, n'en crée aucun, n'ajoute aucune
dépendance, aucune route, aucune authentification, aucune requête réseau, et ne modifie aucune
signature publique hors de `tasks.rs`. Il ne touche ni `tui/app.rs`, ni `crates/mika-agent/`, ni
`docs/egress/`. Ses trois commits sont séquencés par la mesure (§ 5) et chacun compile et passe
seul.

**Ce que la suite pèse, et pourquoi elle n'est pas chiffrée ici.** V2b-2b porte le reste du § 0.2 :
~250 lignes de code mesurées sur `6f895f4e9` moins ce qui atterrit ici, plus ses tests des critères
1 à 4 (le matériau en dépensait 434 pour deux tests) et la correction egress. Son propre recoupage
est la décision de **son** groom, sur une mesure prise sur `main` à ce moment-là — pas une
prévision à faire aujourd'hui.

`Tracked in: senara-solutions/mika#1727` — l'abonnement, la variante ouverte du serveur factice,
`terminal_callback_frame`, `CallbackTaskDetail`, la lecture `GET /api/v1/tasks/{task_id}`, le filtre
session, la réclamation, la livraison `AgentRequest::CallbackResult` et la correction du champ `data`
de `cli-spirit-tasks` partent en V2b-2b. Aucun ticket n'est ouvert (gel jusqu'au 01/12) : la suite se
note sur mika#1727.
