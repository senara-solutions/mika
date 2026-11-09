# mika#1727 — vol V2b-1 : le fil task-events dit ce qu'il fait, et le prouve

- **Ticket :** senara-solutions/mika#1727
- **Branche :** `feat/1727/tui-v2b1-consommateur-sse-reutilisable` (neuve, depuis `main`)
- **Base :** `225d87ad1` (merge de V2a, mika#3061)
- **PR :** `Refs mika#1727` — **jamais `Closes`**. Le ticket se ferme au merge de V4.
- **Matériau mesuré :** `refs/pull/3062/head`, commits `b3ebf76e5`, `bc26b9b4a`, `334b80278`.

---

## 0 — L'écart avec le texte du ticket, à juger avant le reste

### 0.1 — Les quatre critères de V2b-1 ne tiennent pas sous la borne du corps : ce vol livre la part serveur

Le corps borne ce vol deux fois, et la seconde est la plus serrée :

> Ce vol en vise **au plus 120** [tours], soit 30 tours de marge. […] Son total estimé, multiplié par 3 (le facteur de mika#2693), reste sous 560.

Soit **186 lignes estimées** au plus — contre 279 pour la borne de flotte à 150 tours. Les deux
s'appliquent et la plus serrée mord ; c'est 186.

Le périmètre énoncé par le corps a été remesuré poste par poste sur le diff de #3062, pas estimé
de zéro :

| poste du périmètre | mesuré dans #3062 | ce vol |
|---|---|---|
| commentaires de `server/tasks_stream.rs` + `server/mod.rs` | 25 ins. / 7 suppr. (`b3ebf76e5`) | **oui** |
| tests d'émission de `async_db.rs` | 146 ins. (`bc26b9b4a`) | **oui** |
| consommateur SSE réutilisable, côté CLI | 263 ins. / 81 suppr. (`334b80278`) | non — suite |
| serveur SSE factice `pub(crate)` | ~50 (moitié SSE de `spawn_fake_spirit`, `1f24523a7`) | non — suite |
| tests des critères 1 et 2 | ~100 (neufs : #3062 ne tenait le critère 2 par aucun test) | non — suite |

Les deux premiers postes font **174 lignes**. Les trois derniers en font **au moins 217**, même
réduits au strict nécessaire — sans `frame_agent_id`, sans `agent_filter` et sans `stream_url`,
qui ne servent qu'au consommateur filtré de V2b-2 et qui seraient livrés sans appelant
(mika#2205). Les deux ensemble font ~391 : plus du double de la borne.

Le corps tranche lui-même ce cas :

> Si le plan dépasse cette borne, le groom recoupe encore, avec `Tracked in: senara-solutions/mika#1727`, et ne livre que la première part.

**« La première part » est la part serveur**, dans l'ordre où le corps énumère le périmètre et dans
l'ordre où #3062 l'a commitée. Ce n'est pas seulement la plus petite, c'est la préalable : le
commit `bc26b9b4a` dit pourquoi lui-même — le canal task-events avait **exactement un** test
(`broadcast_frame_zero_subscribers_returns_false`, `server/tasks_stream.rs:444`), qui atteste la
primitive et **aucun site d'appel**. « Le fil émet depuis mika#1758 » était donc une lecture de
code, jamais une mesure. Livrer le consommateur avant cette mesure, c'est bâtir un client sur une
lecture ; le faire après, c'est bâtir sur un fait rouge-si-faux.

`Tracked in: senara-solutions/mika#1727` — le consommateur SSE réutilisable, le serveur SSE
factice `pub(crate)` et les tests des critères 1 et 2 partent au vol suivant. Aucun ticket n'est
ouvert (gel jusqu'au 01/12).

### 0.2 — Le nom de la branche décrit la part qui part en suite, et c'est à dire franchement

La branche s'appelle `feat/1727/tui-v2b1-consommateur-sse-reutilisable` et ce vol ne promeut aucun
consommateur. Le nom est celui du V2b-1 entier ; le vol courant en est la moitié serveur. Le titre
de la PR dira ce que le vol fait, pas ce que la branche promettait, et son corps portera le renvoi
du § 0.1. C'est une divergence visible : mieux vaut l'écrire ici que la laisser se lire comme un
plan qui a oublié la moitié de son périmètre.

### 0.3 — « dormant » apparaît trois fois dans ce fichier, et une seule des trois reste vraie

Le corps parle des « deux commentaires qui disent le flux dormant ». Il y en a une troisième
occurrence, à `server/tasks_stream.rs:195`, et elle **ne change pas** :

> Returns `false` when zero subscribers are connected (informational only — the wire is dormant until a client connects; no cursor replay is implemented for v1).

Celle-là dit l'absence de **client**, pas l'absence d'**émission**. Elle est vraie aujourd'hui et
restera vraie après V2b-3 : un canal sans abonné ne porte rien, et `broadcast_frame` rend `false`.
Les deux phrases fausses sont le doc de module (l. 8-11, « emission […] is deferred to a follow-up
ticket; the wire is dormant until that lands ») et le commentaire de route de `server/mod.rs`
(l. 352-356, « Wire-only in v1 — emission from task_engine transition sites lands in a follow-up
ticket »). Ce vol touche ces deux-là, et laisse la troisième intacte.

### 0.4 — Le critère 4 est tenu par construction, pas par vigilance

Le critère 4 demande Egress Manifest Lint vert sans toucher `docs/egress/`. Ce vol ne touche
**aucun fichier de `mika-cli`**, donc aucun site de construction de client HTTP. L'entrée
`cli-spirit-tasks` (`docs/egress/egress-manifest.toml:351-360`) continue de pointer
`crates/mika-cli/src/commands/tasks.rs`, où le client est et où il reste. La D1 et la D3 qui
rougissaient sur #3062 venaient toutes deux du déplacement du client vers
`commands/task_events.rs` ; ce vol ne déplace rien.

Le pilote lance quand même `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR,
comme le critère l'exige — un lint qu'on suppose vert est la classe de faute que mika#2205 nomme.

### 0.5 — La dette nommée n'est pas touchée

`crates/mika-agent/src/task_engine/engine.rs:1441-1444` (« In CLI mode, the TUI's
`poll_callback_tasks()` handles callback delivery […] See #264. ») deviendra faux au retrait du
poll et tombe avec lui en V2b-3. Ce vol ne le réécrit pas : le poll existe encore, donc la phrase
est encore vraie. Les deux autres citations de #264 sur `main` — `engine.rs:1474` et
`task_engine/tick_cli_guard_scan.rs:34` — se relisent au même moment, en V2b-3.

---

## 1 — L'état mesuré sur `225d87ad1`

**L'émission existe, aux quatre sites, chacun sous sa garde.** `grep -n "if transitioned"` sur
`crates/mika-agent/src/async_db.rs` donne l. 579 (`update_task_completed`), l. 606
(`update_task_failed`) et l. 638 (`promote_task_completed`). Chacun appelle `emit_task_event`
**à l'intérieur** du `if`, avec le commentaire mika#1758 qui dit l'invariant :

```rust
// mika#1758: emit TaskCompleted only when the guarded UPDATE actually
// transitioned the row. `Ok(false)` = task already in terminal state
// (race with reaper/promoter/operator) — no transition, no frame.
if transitioned {
    self.emit_task_event(TaskEventFrame::completed(…));
}
```

**Le canal est par processus, attaché par un `OnceLock`.** `async_db.rs:44`
(`task_events_channel: OnceLock<Arc<TaskEventsChannel>>`), rempli par
`set_task_events_channel` (l. 96) depuis deux sites, tous deux dans `server/` — `run_server` pour
les agents connus au boot, `AppState::resolve_agent` pour ceux construits paresseusement ensuite.
`emit_task_event` (l. 109) teste `if let Some(channel) = … .get()` : dans le processus de la TUI
rien ne le remplit, et l'émission est un no-op silencieux plutôt qu'une erreur.

**Rien ne mesure ces sites.** `broadcast_frame_zero_subscribers_returns_false`
(`server/tasks_stream.rs:444`) est le seul test du canal, et il n'atteste que la primitive.
`crates/mika-agent/src/async_db.rs` porte 15 tests, dont `test_create_and_get_task` (l. 4198) qui
crée une tâche et ne regarde aucune trame. Aucun test ne lie une transition à une trame.

**Les helpers de test disponibles.** `test_async_db()` (l. 4046) rend une `AsyncDatabase` sur une
base en mémoire ; `test_async_db_with_session()` (l. 4051) y ajoute une session. Il n'existe
**aucun** helper de création de tâche : `test_create_and_get_task` écrit un littéral `NewTask` de
23 champs en place. Ce vol en ajoute un (§ D2), ce qui est le poste de lignes dominant.

---

## 2 — Les points de conception

### D1 — Les deux commentaires disent le fait mesuré, et ils nomment ce qui le borne

La réécriture ne se contente pas de retirer « dormant ». Elle dit les quatre choses qu'un lecteur
du fil a besoin de savoir, et qu'aucun des deux commentaires actuels ne dit :

1. **l'émission existe depuis mika#1758**, aux sites de transition d'`async_db.rs`, chacun sous sa
   garde `if transitioned` — et c'est cette garde qui borne un consommateur à **une** livraison
   par transition ;
2. le canal est attaché par un `OnceLock` rempli depuis **deux** sites, tous deux dans `server/`
   (`run_server`, `AppState::resolve_agent`) ;
3. dans le processus de la TUI rien ne le remplit : la même émission y est un **no-op silencieux**,
   pas une erreur ;
4. le canal est donc **par processus et non par agent** — un consommateur filtre sur `agent_id`
   lui-même.

Le point 1 est celui qui compte pour la suite : le consommateur de V2b-2 injecte un callback dans
une conversation sur la foi de ces trames, et la garde est la seule chose qui l'empêche de
l'injecter deux fois. Un commentaire qui dirait « ça émet » sans dire « sous cette garde » aurait
retiré le mensonge sans livrer le fait utile.

La table « Sibling divergence » et les invariants de `PREVIEW_CAP_CHARS` du même doc de module
**ne bougent pas** : V2b-2 s'appuie dessus.

### D2 — Les trois tests d'émission, et pourquoi le troisième est celui qui porte

Un test par transition que le consommateur de V2b-2 lit, plus le contrôle négatif :

| test | ce qu'il atteste |
|---|---|
| `update_task_completed` sur une tâche en vol | **exactement une** `TaskCompleted`, portant le `task_id`, l'`agent_id` et le résultat |
| `update_task_failed` sur une tâche en vol | **exactement une** `TaskFailed`, portant le `task_id`, l'`agent_id` et l'erreur |
| un second appel sur une tâche déjà terminale | `Ok(false)` et **aucune trame**, par **aucun** des deux sites |

Le troisième est le contrôle qui borne la livraison à une. C'est la garde `if transitioned` qui
empêche le poll — conservé en filet jusqu'à V2b-3 — de proposer une seconde injection du même
callback. Si l'émission sortait de cette garde, la trame repartirait et la conversation recevrait
le callback deux fois. Les deux premiers tests seraient verts dans ce monde-là ; seul le troisième
rougit.

« Exactement une » se mesure, il ne se suppose pas : après le `match` sur la trame attendue, un
second `try_recv()` doit rendre `Err(TryRecvError::Empty)`. Sans cette seconde assertion, un site
qui émettrait deux fois passerait.

**Chaque test s'abonne après `create_task`.** `create_task` émet une `TaskCreated`
(`async_db.rs:317`) ; en s'abonnant après, cette trame est diffusée à zéro récepteur et ne pollue
aucune assertion. C'est aussi ce qui fait du troisième test un vrai contrôle : il s'abonne après
la **première** transition, donc le canal est vide au moment où le second appel est fait.

**Un helper `task_in_flight`**, parce que `NewTask` a 23 champs et qu'il n'existe aucun
constructeur de test. Trois littéraux de 23 champs coûteraient 60 lignes de plus qu'un helper, et
le troisième champ qu'on oublierait d'aligner entre les trois serait la prochaine lecture fausse.
Il crée la tâche sur l'agent de la base (`db.agent_id()`) : les trames portent cet `agent_id`, et
c'est ce que le consommateur filtré de V2b-2 comparera.

### D3 — `promote_task_completed` n'est pas testé par ce vol, et c'est délibéré

Le quatrième site d'émission (`async_db.rs:638`, promotion `failed` → `completed`, #958) porte la
même garde et émet une seconde `TaskCompleted` pour le même `task_id`. Ce n'est pas un défaut : le
doc du site le dit, du point de vue de l'observateur « la tâche a atteint `completed` » est
l'invariant, et le fil enregistre fidèlement la séquence.

Le corps du ticket nomme deux transitions, pas trois : « chaque transition terminale
(`update_task_completed`, `update_task_failed`) ». Le troisième test couvre la promotion par son
contrôle négatif — il appelle les deux sites terminaux sur une tâche déjà terminale — mais aucun
test ne mesure la promotion elle-même. C'est hors périmètre, et le dire vaut mieux que le laisser
paraître couvert.

### D4 — Aucune ligne de production ne change

Les trois commits de ce vol touchent : deux blocs de commentaires, et un module `#[cfg(test)]`.
Zéro changement de route, zéro changement d'auth, zéro changement de signature. C'est ce qui rend
le vol mergeable seul sans rien débloquer ni rien casser : il transforme une lecture de code en
mesure, et une phrase fausse en phrase vraie.

---

## 3 — Les tests

Les trois tests vivent dans `#[cfg(test)] mod tests` de `crates/mika-agent/src/async_db.rs`,
après `test_create_and_get_task` (l. 4231), et nomment leur ticket :

```rust
#[tokio::test]
async fn mika1727_v2b1_update_task_completed_emet_exactement_une_trame() { … }

#[tokio::test]
async fn mika1727_v2b1_update_task_failed_emet_exactement_une_trame() { … }

#[tokio::test]
async fn mika1727_v2b1_un_second_appel_sur_une_tache_terminale_nemet_rien() { … }
```

Forme de chacun, dans l'ordre :

1. `test_async_db()` ;
2. `Arc::new(TaskEventsChannel::new())` puis `db.set_task_events_channel(…)` — le `OnceLock` est
   rempli à la main, puisque dans un test rien ne joue le rôle de `run_server` ;
3. `task_in_flight(&db, …)` ;
4. `channel.subscribe()` — **après** (3), cf. D2 ;
5. l'appel, dont le `bool` est asserté (`true` sur une tâche en vol, `false` sur une terminale) ;
6. `rx.try_recv()`, `match` exhaustif sur la variante attendue avec `panic!` sur toute autre ;
7. un second `try_recv()` qui doit rendre `Err(TryRecvError::Empty)`.

`use tokio::sync::broadcast;` s'ajoute aux imports du module de test, pour nommer
`broadcast::error::TryRecvError::Empty`.

**Ce qui rougit si la garde saute.** Déplacer l'un des deux `emit_task_event` hors de son
`if transitioned` fait rougir le troisième test et lui seul. Retirer l'émission d'un site fait
rougir le premier ou le deuxième. Changer la variante émise fait rougir le `match`.

**Ce que ces tests ne couvrent pas**, et qu'il faut savoir en les lisant : ils mesurent l'émission
depuis `AsyncDatabase`, pas le passage de la trame par le handler SSE ni son encodage sur la
socket. Cette moitié-là est mesurée par les tests du consommateur, qui partent au vol suivant
(§ 0.1).

---

## 4 — Fire-Disposition

Ce vol livre trois détecteurs (trois tests dont le chemin de succès est « aucune violation »).

**Option (a) — exception nommée en allowlist, avec une allowlist vide.** Les trois tests
atterrissent **armés**, sans `#[ignore]` et sans entrée d'exception, parce qu'il n'existe aucune
violation existante à exempter : l'arbre émet déjà correctement aux quatre sites, chacun sous sa
garde (§ 1), et les trois tests sont repris de `bc26b9b4a`, où ils étaient **verts** sur une base
de #3062 qui portait les mêmes sites d'émission qu'ici.

Une allowlist vide est le bon état pour ce détecteur : il n'y a rien à nommer, rien à référencer en
suivi, et donc aucune assertion auto-nettoyante à écrire — la propriété « l'allowlist est vide »
n'est pas une dette, c'est le résultat. Inventer une entrée pour satisfaire la forme produirait
exactement l'allowlist de complaisance que mika#2309 condamne.

**Si le pilote les trouve rouges, c'est (c) — halte-et-remontée.** Un test d'émission rouge sur
`225d87ad1` ne serait pas une exception à déclarer : ce serait la découverte que le fil n'émet pas
ce que `server/tasks_stream.rs` est en train d'être réécrit pour affirmer. Le pilote s'arrête,
n'arme pas le commentaire du § D1, et remonte la mesure en commentaire sur mika#1727 — jamais un
`#[ignore]` posé sur la mesure qui vient de contredire la doc.

---

## 5 — Definition of Done

- [ ] `server/tasks_stream.rs` : le doc de module (l. 5-11) dit l'émission effective, sa garde, le
      double site d'attache, le no-op de la TUI et le canal par processus (§ D1). La table
      « Sibling divergence » et `PREVIEW_CAP_CHARS` sont inchangés.
- [ ] `server/tasks_stream.rs:195` est **inchangé** (§ 0.3).
- [ ] `server/mod.rs` : le commentaire de route (l. 352-356) ne dit plus « Wire-only in v1 ».
- [ ] Les trois tests de `async_db.rs` existent, nomment `mika1727_v2b1`, et passent.
- [ ] Le helper `task_in_flight` existe, et chaque test s'abonne après lui.
- [ ] Aucune ligne de production modifiée (`git diff --stat` ne montre que des commentaires et le
      module `#[cfg(test)]`).
- [ ] `cargo test -p mika-agent async_db` vert ; `cargo test` complet vert.
- [ ] `cargo clippy --all-targets` et `cargo fmt --check` verts.
- [ ] `bash scripts/verify-egress-manifest.sh --report` vert, **sans aucune modification de
      `docs/egress/`** (§ 0.4).
- [ ] `bash scripts/verify-pipeline.sh origin/main` vert.
- [ ] PR : titre qui dit ce que le vol fait (pas ce que la branche promettait, § 0.2), corps
      portant `Refs mika#1727`, le renvoi `Tracked in: senara-solutions/mika#1727` du § 0.1, et
      `Code review: delegated to mika-qa (mika#2694)`.
- [ ] Aucun fichier du plan de contrôle touché : `.github/workflows/`, `.claude/`,
      `skills/bundled/`, `crates/mika-agent/src/well_known_agents.rs`, `.mika/`, `docs/egress/`.
- [ ] Aucun ticket ouvert (gel jusqu'au 01/12).

---

## Acceptance criteria

Les quatre critères sont ceux du corps de mika#1727 § « Critères de V2b-1 », transcrits tels
quels, avec la disposition de ce vol.

- **AC1 — Le consommateur SSE s'ouvre sur une URL et un jeton donnés, et rend les trames d'un
  serveur factice dans l'ordre. Un test rougit si une trame est perdue ou doublée.**
  → **Renvoyé au vol suivant** (`Tracked in: senara-solutions/mika#1727`, § 0.1). Aucune ligne de
  `mika-cli` n'est touchée par ce vol.
- **AC2 — `mika tasks stream` (`stream_task_events`) garde son comportement visible. Un test le
  tient.**
  → **Renvoyé au vol suivant**, avec AC1 : les deux portent sur le même fichier
  (`crates/mika-cli/src/commands/tasks.rs`) et le même refactor. Tenu trivialement ici, au sens où
  ce vol ne touche pas `stream_task_events` — mais sans le test que le critère exige, donc non
  revendiqué.
- **AC3 — Les tests d'émission de `async_db.rs` décrits ci-dessus rougissent si la garde
  `if transitioned` saute.**
  → **Tenu par ce vol.** Trois tests (§ 3) ; le troisième rougit sur la garde et lui seul (§ D2).
- **AC4 — Egress Manifest Lint est vert sans aucune modification de `docs/egress/`. Le pilote
  lance `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR.**
  → **Tenu par ce vol**, par construction (§ 0.4) : aucun site client n'est touché. Le pilote
  lance quand même la commande.

En plus des critères du corps, et parce que le § 0.1 les rend vérifiables :

- **AC5 — Les deux commentaires qui déclaraient le flux dormant disent le fait mesuré**
  (émission effective, garde `if transitioned`, double site d'attache, no-op de la TUI, canal par
  processus), et la troisième occurrence de « dormant » (`tasks_stream.rs:195`) est inchangée.
- **AC6 — Aucune ligne de production ne change.** Le diff ne porte que des commentaires et un
  module `#[cfg(test)]`.

### Correspondance AC → livrable

| AC | livrable | détecteur |
|---|---|---|
| AC1, AC2 | — (vol suivant) | — |
| AC3 | 3 tests + helper dans `async_db.rs` | eux-mêmes |
| AC4 | aucun (construction) | `scripts/verify-egress-manifest.sh` |
| AC5 | doc de module `tasks_stream.rs`, commentaire de route `server/mod.rs` | relecture |
| AC6 | — | `git diff --stat` |

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/tasks_stream.rs` (doc de module) | 20 |
| `crates/mika-agent/src/server/mod.rs` (commentaire de route) | 8 |
| tests d'émission + helper `task_in_flight` (`crates/mika-agent/src/async_db.rs`, `#[cfg(test)]`) | 146 |

Total estimé : 174 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Provenance de ces trois nombres : ils sont mesurés, pas estimés.** Le poste de 146 est le
`--numstat` exact de `bc26b9b4a` ; les 28 sont le `--numstat` exact de `b3ebf76e5` (25 insertions,
arrondies pour l'import de `broadcast` et le réalignement sur `225d87ad1`). C'est l'instruction du
corps : « Le plan de ce vol estime donc **à partir du diff mesuré de #3062**, et non de zéro »,
parce que l'estimation de V2b s'était trompée d'un facteur 4,7 (251 estimées, 1 174 livrées).

**Le vol tient sous les deux bornes, et la plus serrée n'est pas celle de la règle.** 174 passe la
borne de flotte (279). Il passe aussi la borne volontaire que le corps s'impose — au plus 120 tours,
soit `174 × 3 = 522 < 560` lignes livrées attendues. C'est cette seconde borne qui a dicté le
recoupement du § 0.1 : le périmètre entier de V2b-1 pèse ~391 lignes estimées, donc ~1 173 livrées
au facteur 3 — la mesure exacte de ce qui a tué #3062 à 151 tours.

**Pourquoi un seul vol tient.** Les trois commits (§ ci-dessous) touchent trois blocs disjoints de
deux crates, sans changement de signature, sans changement de route, et sans fichier neuf. Il n'y a
rien à séquencer en interne : le commentaire du § D1 et les tests du § D2 attestent le même fait
par deux moyens, et chacun compile seul.

`Tracked in: senara-solutions/mika#1727` — consommateur SSE réutilisable (`open`/`consume` dans
`crates/mika-cli/src/commands/tasks.rs`), serveur SSE factice `#[cfg(test)] pub(crate)`, et les
tests des critères AC1 et AC2. Aucun ticket n'est ouvert (gel jusqu'au 01/12) : la suite se note
sur mika#1727.

---

## Ordre de commit

1. **`test(1727): les sites d'émission des trames de tâche sont attestés`**
   `crates/mika-agent/src/async_db.rs` — helper `task_in_flight` + les trois tests, `use
   tokio::sync::broadcast;`. **D'abord**, parce que c'est la mesure : si elle rougit, le vol
   s'arrête ici (§ 4, option c) et le commentaire du commit 2 n'est jamais écrit. L'écrire
   d'abord puis mesurer ensuite serait affirmer avant de savoir.
2. **`fix(1727): le flux task-events n'est plus déclaré dormant`**
   `crates/mika-agent/src/server/tasks_stream.rs` (doc de module), `crates/mika-agent/src/server/mod.rs`
   (commentaire de route). Aucun changement de code.

Plancher de rebase : `225d87ad1`. Le vol se rebase sur `main` sans conflit attendu — ses deux
commits touchent trois régions que V1 et V2a n'ont pas visitées.
