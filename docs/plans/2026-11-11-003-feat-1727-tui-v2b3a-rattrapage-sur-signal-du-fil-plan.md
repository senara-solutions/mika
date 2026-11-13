# Vol V2b-3a — le rattrapage sur signal du fil (mika#1727)

> Le fil dit lui-même quand il a pu manquer quelque chose : à chaque ouverture
> réussie, la première comprise, et à chaque `OverflowMarker`. Sur ce signal, le
> drainage de la TUI appelle `poll_callback_tasks` — le balayage des callbacks
> non livrés de la session — sans copier son corps.

Branche : `feat/1727/tui-v2b3a-rattrapage-sur-signal-du-fil`, neuve depuis `main`.
La PR portera `Refs mika#1727`. Jamais `Closes`, `Fixes` ni `Resolves` : le
ticket se ferme au merge de V4.

---

## 0. Ce qui est mesuré, avant toute conception

Chaque ligne de cette section est une mesure faite dans ce worktree sur
`a5ba21695`, pas un souvenir. Elles décident de la forme du vol.

**La borne est entière.** `git rev-parse HEAD` et `git rev-parse origin/main`
donnent le même `a5ba216953fb711c9fff1fee3dfe6c22f9c04847`, et
`git diff --numstat origin/main...HEAD` est vide. Zéro ligne consommée, 279
disponibles.

**Le serveur ne rejoue rien.** `crates/mika-agent/src/server/tasks_stream.rs:264`
diffuse par `broadcast` ; un `Lagged(n)` devient
`TaskEventFrame::OverflowMarker { dropped_count }` (`tasks_stream.rs:279`). Il
n'y a ni curseur, ni reprise. Rien dans ce vol ne touche ce fichier.

**La trame d'overflow traverse le filtre d'agent.** `consume`
(`crates/mika-cli/src/commands/tasks.rs:565`) n'écarte que ce qu'il a compris
comme appartenant à un autre agent : `frame_agent_id` rend `None` pour
`OverflowMarker` et `Unknown` (`tasks.rs:486-498`), donc le couple
`(Some(voulu), None)` tombe sur `_ => false` (`tasks.rs:582-585`) et la trame
atteint la closure. **Le signal d'overflow est donc atteignable là où il doit
l'être**, sans toucher au filtre.

**Le point d'ouverture réussie est unique et déjà nommé.** Dans
`open_for_avec_recul` (`tasks.rs:787`), `if let Ok(flux) = TaskEventStream::open(…)`
ouvre la seule branche où une connexion existe, et `tentative = 0;`
(`tasks.rs:810`) y est déjà la trace de « c'est une ouverture réussie ». La
**première** ouverture passe par cette ligne comme les suivantes : il n'y a pas
de cas particulier au lancement à écrire.

**La closure de consommation tient en trois lignes** (`tasks.rs:812-816`) et
n'appelle qu'une fonction pure, `terminal_callback_frame`.

**Le canal est typé `String`** (`tasks.rs:762`), et **sept tests** le lisent à nu
— par `couture.rx.recv()` dans `commands/tasks.rs` (V2b-2b-3 et V2b-2b-4) et par
`tx.send(id)` dans `tui/app.rs` (V2b-2b-2 et -3). `from_channel` (`tasks.rs:858`)
rend `(UnboundedSender<String>, Self)` et est appelée par le helper
`couture_dun_callback_livrable` (`tui/app.rs:2385`). **Ce chiffre décide de la
forme du signal au § 1.**

**Le poll pose six champs et un message système** (`tui/app.rs:1704-1812`) :
`mark_task_delivered` pour réclamer, le repli `FAILED_TASK_FALLBACK`, le saut
d'un `completed` sans résultat (`_ => continue`, `app.rs:1766`), le message
`format!("[{}] {}", task.label, status_label)`, le `CallbackResult` à six
champs, `status = Thinking`, puis `break` — **un callback par passage**. Son
corps ne change pas.

**Le drainage est appelé à chaque tick au repos** (`app.rs:1128-1141`), sans
`POLL_INTERVAL_TICKS`, et le poll tous les `POLL_INTERVAL_TICKS = 167`
(`app.rs:669`). Ce vol n'écrit pas une ligne dans ce câblage-là : **le minuteur
reste en place**, et le déclencheur se pose à l'intérieur du drainage.

**Le piège du helper de test, mesuré avant d'écrire un test vert qui ne prouve
rien.** `couture_dun_callback_livrable` (`app.rs:2385`) écrit `result`,
`created_trace_id` et `parent_task_id` **dans le corps HTTP du détail**, jamais
sur la ligne, et pose le statut par `update_task_status`
(`crates/mika-agent/src/db/tasks.rs:742`), qui ne touche **pas** `completed_at`.
Or le balayage lit `get_undelivered_callback_tasks_for_session`
(`db/tasks.rs:1908`), dont le `WHERE` exige `completed_at IS NOT NULL` **et** lit
`result` **sur la ligne**. Un test du critère 3 qui réutiliserait le helper tel
quel tomberait sur `_ => continue` et ne livrerait rien — vert, et prouvant le
contraire de ce qu'il annonce (classe mika#2205). Le lecteur qui corrige :
`AsyncDatabase::update_task_completed(&id, result)`
(`crates/mika-agent/src/async_db.rs:563`), qui pose ensemble `status='completed'`,
`result` et `completed_at` — **mais seulement depuis `pending` ou `in_progress`**
(`db/tasks.rs:784`). Le § 4 en tire l'ordre des gestes du test, et son assertion
sur le booléen rendu.

---

## 1. La forme du signal : un drapeau à part, et pourquoi

Le ticket laisse le choix au plan, et demande qu'il dise pourquoi. **Signal
retenu : un `Arc<AtomicBool>` porté par `CallbackFeed`**, cloné dans la tâche du
superviseur. Pas une variante d'énumération dans le canal.

Cinq raisons, dans l'ordre où elles pèsent.

**(a) « Plusieurs signaux en file donnent un seul balayage » devient vrai par le
type.** Un booléen ne fait pas de file : deux armements sont un seul `true`, et
`swap(false)` le consomme. Avec une énumération dans un canal non borné, la
propriété serait une discipline du lecteur — une boucle qui vide la file en
comptant, à écrire, à commenter et à tester. Ici il n'y a rien à tester parce
qu'il n'y a rien à rater.

**(b) Le critère 5 tient sans qu'aucun test existant ne change de forme.** Le
canal reste `UnboundedSender<String>`, donc les sept lectures à nu mesurées au
§ 0 et la signature de `from_channel` sont intactes. Le critère dit « si l'un
lit le canal à nu et doit changer de forme, ses assertions restent les mêmes » ;
la meilleure façon de tenir ses assertions est de ne pas toucher à sa forme.

**(c) Une énumération complique le contrat de `next_candidate`, dont la force
actuelle est d'être simple.** `self.rx.try_recv().ok()?` (`tasks.rs:846`) rend un
candidat ou rien ; avec deux variantes il devrait boucler, sauter les signaux,
et répondre à « que faire d'un signal lu en cherchant un candidat ? ». Cette
question n'existe pas avec un drapeau.

**(d) L'ordre entre le drapeau et le canal n'a aucune conséquence**, parce que le
signal ne porte ni statut ni `task_id`. Il ne dit que « il a pu manquer quelque
chose », la base reste l'autorité, et `mark_task_delivered` rend le balayage
idempotent : qu'il passe avant ou après une trame du fil, la ligne n'est livrée
qu'une fois. Un signal porté *dans* le canal donnerait un ordre — et donc une
raison de se demander si cet ordre compte.

**(e) La contrainte sur `tui/app.rs` tient dans les deux cas, et ce drapeau ne
l'entame pas** : il n'est ni l'URL, ni le jeton, ni le nom de la lecture HTTP.
`app.rs` ne verra qu'un geste, `prendre_le_rattrapage()`, dont le nom ne dit rien
du transport.

Ce que ce choix coûte, dit ici plutôt que découvert plus tard : un drapeau ne
compte pas les trous, donc une TUI ne saura jamais *combien* de fois elle a
rattrapé. Le `dropped_count` de l'`OverflowMarker` n'est pas conservé. C'est
voulu — un compteur serait une seconde source sur une question que la base
tranche seule — et c'est aussi ce qui garde la propriété (a).

---

## 2. Les gardes et leur domicile

### 2.1 Le fil arme le rattrapage (`commands/tasks.rs`)

Trois pièces, toutes dans le fichier qui porte déjà la couture.

**Le champ.** `CallbackFeed` gagne `rattrapage: Arc<AtomicBool>`, créé dans
`open_for_avec_recul` et dans `from_channel`, cloné une fois dans la tâche
spawnée. Le `Arc` est ce qui permet au superviseur d'écrire ce que la TUI lit :
ce n'est pas un ornement.

**L'armement sur ouverture.** Une ligne à côté de `tentative = 0;`
(`tasks.rs:810`), dans la branche `if let Ok(flux)`. Le commentaire dit le
pourquoi qu'on ne lit pas dans le code : c'est l'**ouverture** qui arme, non la
première trame, parce que ce qui a pu être manqué l'a été *avant* elle — et la
première ouverture n'est pas un cas particulier, c'est le premier passage de
cette même ligne.

**L'armement sur `OverflowMarker`.** Une fonction sœur de
`terminal_callback_frame`, à `match` exhaustif sur les huit variantes, **sans
bras `_`** :

```rust
fn frame_annonce_un_trou(frame: Option<&TaskEventFrame>) -> bool {
    match frame {
        Some(TaskEventFrame::OverflowMarker { .. }) => true,
        Some(TaskEventFrame::TaskCreated { .. })
        | Some(TaskEventFrame::TaskClaimed { .. })
        | Some(TaskEventFrame::TaskCompleted { .. })
        | Some(TaskEventFrame::TaskFailed { .. })
        | Some(TaskEventFrame::TaskDelivered { .. })
        | Some(TaskEventFrame::TaskCancelled { .. })
        | Some(TaskEventFrame::Unknown)
        | None => false,
    }
}
```

**Un `matches!` est écarté, et c'est la même raison que le bras `_`.** Il
répondrait « non » à toute variante future sans que personne ait à se prononcer.
Le jour où le serveur émettrait un `overflow_v2`, ou toute trame disant « il
manque quelque chose », le `match` exhaustif ne compile pas tant que son auteur
n'a pas tranché ; un `matches!` compile et se tait. C'est exactement l'argument
déjà écrit sur `frame_agent_id` (`tasks.rs:486`) et `terminal_callback_frame`
(`tasks.rs:682`), et ce vol ne l'affaiblit pas : **`terminal_callback_frame` n'est
pas modifié**, et `Unknown` y reste comme ici du côté « rien ».

**Le site d'application est nommé, pas écrit dans la closure.** La closure du
superviseur devient un appel à une fonction :

```rust
fn appliquer_la_trame(
    trame: Option<&TaskEventFrame>,
    tx: &tokio::sync::mpsc::UnboundedSender<String>,
    rattrapage: &AtomicBool,
)
```

qui fait les deux gestes — l'id terminal dans le canal, le drapeau sur un trou.
**La raison est testable, et elle est la seule qui vaille** : toute ouverture
arme, donc un test passant par un serveur factice ne peut pas distinguer
« l'overflow a armé » de « la connexion qui portait l'overflow venait de
s'ouvrir ». Prétendre le contraire serait un test vert prouvant autre chose que
son nom (mika#2205). Nommer le site rend le critère 2 mesurable **sans serveur**,
sur les effets eux-mêmes, et laisse le superviseur à une ligne.

**Le lecteur.** `pub(crate) fn prendre_le_rattrapage(&self) -> bool`, un
`swap(false, SeqCst)` : destructif par construction, parce qu'un signal lu deux
fois serait deux balayages, et c'est précisément ce que le critère 4 refuse. Un
`armer_le_rattrapage(&self)` sous `#[cfg(test)]` l'accompagne — c'est ce qui
permet aux tests de `app.rs` d'armer **sans** changer la signature de
`from_channel`, donc sans toucher aux sept appels mesurés au § 0.

### 2.2 Les docs qui ne disent plus le vrai (`commands/tasks.rs`)

Deux blocs, corrigés **dans le même commit que le code** qui les dément.

Le bloc de `CallbackFeed` (`tasks.rs:750-760`) dit « elle n'expose qu'un geste »
et « le canal ne porte qu'un `task_id` ». La seconde phrase **reste vraie**, et
c'est tout l'intérêt du § 1 : elle cesse d'être une description pour devenir une
propriété voulue. Elle est réécrite en disant pourquoi le signal voyage **à
côté** du canal et non dedans — un second champ serait une seconde autorité sur
le statut, et une variante serait une file là où il faut un état. La première
phrase devient fausse : la couture expose désormais deux gestes, et le bloc les
nomme tous les deux.

Le bloc d'`open_for` (`tasks.rs:776-780`) dit « ce qu'il ne garantit pas : le
rattrapage ». Il devient faux : le fil ne rejoue toujours rien, mais il **dit
maintenant qu'il a pu manquer quelque chose**, et c'est ce que la TUI consomme.
Réécrit en trois lignes : ce qui est perdu l'est toujours, ce qui change est que
la perte est annoncée, et `poll_callback_tasks` reste le lecteur de l'annonce.

### 2.3 Le drainage balaye sur le signal (`tui/app.rs`)

En tête de `drain_callback_deliveries` (`app.rs:1628`), **avant** la boucle :

```rust
// Le fil ne rejoue rien : à son ouverture comme après un `OverflowMarker`,
// il dit seulement qu'il a pu manquer quelque chose. Le balayage est la
// lecture que le poll fait déjà — appelée, jamais recopiée.
if self
    .callback_feed
    .as_ref()
    .is_some_and(|f| f.prendre_le_rattrapage())
{
    self.poll_callback_tasks().await;
    // Le balayage a peut-être livré, et une livraison relance le tour :
    // la boucle du fil ne doit pas en empiler une seconde. Même règle que
    // le `break` du poll — un callback par passage.
    if self.status != AgentStatus::Idle {
        return;
    }
}
```

Quatre propriétés, et aucune n'est une copie :

- **`poll_callback_tasks` est appelée, son corps est intact.** Le message
  système, le `CallbackResult` à six champs, la garde d'ancienneté des `failed`
  et le résumé « Cleared N stale failed task(s) » sont ceux du poll parce que
  c'est le poll qui s'exécute. Le critère 3 — « le même `CallbackResult` et le
  même message système que le poll » — tient par identité, pas par ressemblance.
- **Un seul balayage par passage** : `prendre_le_rattrapage` consomme, et le
  drapeau ne compte pas.
- **Le minuteur reste** : rien ne change au câblage du tick
  (`app.rs:1128-1141`), et le poll continue de tourner tous les 167 ticks. Ce vol
  ajoute un déclencheur et n'en retire aucun ; le retrait est V2b-3b.
- **Aucun appel réseau n'est ajouté** : le balayage est la lecture SQLite locale
  que le poll fait déjà. `docs/egress/` n'a donc rien à apprendre de ce vol.

### 2.4 Ce qui ne change pas

`terminal_callback_frame`, `next_candidate`, la signature de `from_channel`, le
type du canal, `TaskEventStream::open`/`consume`, `Recul`, `fake_spirit` (aucun
serveur factice neuf), le corps de `poll_callback_tasks`, `POLL_INTERVAL_TICKS`,
le câblage du tick, `crates/mika-agent/`, `crates/mika-a2a/`, `docs/egress/`,
`Cargo.toml` (aucune dépendance, aucune feature).

---

## 3. Ordre des commits

Deux commits, chacun vert avec ses tests avant le suivant. Le pilote commite
après chaque garde verte, pas en fin de vol.

**Commit 1 — `feat(1727): le fil annonce qu'il a pu manquer quelque chose`.**
Tout le § 2.1 et tout le § 2.2, avec T1 et T2. À ce point le signal existe,
s'arme et se prend ; **personne ne le lit encore**. C'est du code atteignable
mais sans effet sur la TUI, et c'est voulu : la moitié qui produit le signal se
mesure seule, sur le fichier du fil, sans base de données ni harnais de TUI.

**Commit 2 — `feat(1727): le drainage balaye sur le signal du fil`.** Le § 2.3
avec T3 et T4. C'est le commit qui donne un lecteur au signal.

L'ordre inverse ne tient pas : un drainage qui lirait un drapeau inexistant ne
compile pas. Les deux commits ne sont pas deux PR mergeables séparément — le
commit 1 seul laisserait un signal que rien ne consomme, ce que ce vol existe
pour éviter — d'où un vol, une PR.

---

## 4. Le jeu de tests

Quatre tests, aux quatre premiers critères. Aucun serveur factice neuf : T1 se
sert de `fake_spirit::spawn_sequence` (V2b-2b-4), T2 d'aucun serveur, T3 et T4 du
helper `couture_dun_callback_livrable` et de `spawn_detail`.

### T1 — l'ouverture arme, la réouverture aussi (`commands/tasks.rs`)

`mika1727_v2b3a_la_premiere_ouverture_et_la_reouverture_arment_le_rattrapage`.

`spawn_sequence(vec![(200, premiere), (200, seconde)])` avec
`Recul { initial_ms: 1_000, plafond_ms: 1_000 }`.

- `premiere` est un `task_claimed` **de l'agent** : il passe le filtre, n'est pas
  terminal, n'annonce aucun trou. **Donc un drapeau armé après la première
  connexion ne peut venir que de l'ouverture elle-même** — c'est ce choix de
  trame qui fait la preuve, pas l'assertion.
- Scruter `couture.rattrapage` jusqu'à `true`, sous `timeout(5 s)` : c'est le
  critère « la **première** ouverture émet le signal ». Témoin positif à côté
  (mika#2205) : `connexions` vaut au moins 1.
- `prendre_le_rattrapage()` le remet à `false`. La fenêtre pour ce geste est le
  recul entier — une seconde — parce que la connexion 1 est fermée depuis qu'on a
  vu le drapeau. **Le recul est à 1 s pour cette raison et pour elle seule** : il
  est déjà paramétrable (`open_for_avec_recul`), donc aucune horloge n'est
  suspendue et aucune feature `tokio/test-util` n'est ajoutée.
- Attendre `t-7` sur `couture.rx` : la trame n'a pu arriver que par la **seconde**
  connexion, et `connexions >= 2` le dit en témoin.
- Relire le drapeau : `true`, armé par la **réouverture**.

### T2 — l'overflow arme, `Unknown` reste inerte (`commands/tasks.rs`)

`mika1727_v2b3a_un_overflow_arme_le_rattrapage_sans_rien_mettre_dans_le_canal`.

Sur `appliquer_la_trame`, sans serveur et sans course, avec un `tx`/`rx` et un
`AtomicBool` à `false` :

| trame | drapeau | canal |
|---|---|---|
| `OverflowMarker { dropped_count: 3 }` | `true` | vide |
| `Unknown` (drapeau remis à `false`) | `false` | vide |
| `TaskCompleted { task_id: "t-1", … }` | `false` | `"t-1"` |

La troisième ligne est le **témoin positif** : sans elle, un montage où rien
n'atteindrait jamais le canal rendrait les deux premières lignes vertes pour la
mauvaise raison. Et c'est elle qui montre que l'overflow ne met **aucun**
`task_id` dans un canal qui, lui, en reçoit bien quand il le doit.

### T3 — un callback semé en base, livré une fois sur le signal (`tui/app.rs`)

`mika1727_v2b3a_un_callback_sans_trame_est_livre_sur_le_signal`.

- `couture_dun_callback_livrable(&app, "pending", None, None)` : la ligne est
  créée, le serveur de détail est debout, la couture existe. **Le statut `pending`
  est un second refus, pas une commodité** — si une trame entrait par erreur dans
  le canal, `est_un_callback_a_livrer` refuserait ce détail
  (`matches!(status, "completed" | "failed")`), donc toute livraison observée
  vient nécessairement du balayage.
- `app.db.update_task_completed(&id, Some("le pilote a fini")).await` pose
  ensemble le statut, le résultat **et** `completed_at` sur la **ligne** — les
  trois que `get_undelivered_callback_tasks_for_session` exige (§ 0). Le booléen
  rendu est asserté : un `false` silencieux ferait un test vert qui ne livre
  rien.
- **Rien n'est envoyé dans le canal.** `app.callback_feed = Some(feed)`, puis
  `feed.armer_le_rattrapage()`, puis `app.drain_callback_deliveries().await` — le
  drainage, jamais le tick, donc **sans faire avancer le minuteur du poll**.
- Assertions : un `CallbackResult` et un seul, `task_id == id`,
  `result == "le pilote a fini"`, `original_status == "completed"`,
  `status == Thinking`, et le message système `"[long_running:run_claude_pilot] completed"`
  dans `app.messages`.
- `trace_id` et `parent_task_id` sont asserté **`None`** : le helper laisse ces
  deux champs nuls sur la ligne alors que le détail HTTP annonce `tr-9` et
  `t-parent` (`app.rs:2398-2400`, `2433-2434`). **C'est la signature qui sépare
  les deux chemins** — une livraison par le fil les aurait remplis. Le test
  montre donc que c'est le balayage qui a livré, au lieu de le supposer.

### T4 — deux signaux, une seule livraison (`tui/app.rs`)

`mika1727_v2b3a_deux_signaux_ne_livrent_pas_deux_fois_le_meme_callback`.

Même semis qu'en T3. `armer_le_rattrapage()` **deux fois**, puis drainer : une
livraison. Remettre `status = Idle`, armer deux fois encore, drainer : plus rien
— le drapeau a été consommé au premier passage, et la ligne est `delivered`. Les
deux moitiés comptent : la première dit que deux signaux en file ne font qu'un
balayage, la seconde que même un signal neuf ne relivre pas une ligne déjà prise.

---

## 5. Ce que ce vol ne fait pas

- Il ne retire ni le minuteur du poll ni une ligne de son corps. V2b-3b.
- Il ne fait aucun appel réseau neuf, n'ajoute ni dépendance ni feature.
- Il ne touche ni `crates/mika-agent/`, ni `crates/mika-a2a/`, ni
  `docs/egress/`, ni aucun fichier du plan de contrôle (`.github/workflows/`,
  `.claude/`, `skills/bundled/`, `well_known_agents.rs`, `.mika/`, CODEOWNERS).
- Il n'ouvre aucun ticket — gel jusqu'au 01/12. Les restes vont dans la PR,
  notés `Tracked in: senara-solutions/mika#1727`.
- Il ne conserve pas le `dropped_count` de l'`OverflowMarker` (§ 1).

Aucun des six critères n'a demandé de toucher un fichier exclu : le signal naît
dans `commands/tasks.rs` et se lit dans `tui/app.rs`, deux fichiers de
`mika-cli`. Rien à arrêter, rien à élargir.

---

## 6. La dette, nommée et non traitée

**Le balayage et la réclamation restent locaux.** `poll_callback_tasks` lit
`get_undelivered_callback_tasks_for_session` et écrit `mark_task_delivered`
directement sur la base SQLite depuis le processus de la TUI. Ce vol change
**quand** ce balayage a lieu, jamais **où** il s'exécute : la TUI reste un second
écrivain sur la même base que mika-spirit, ce que tout le ticket #1727 existe
pour supprimer.

La forme correcte sortirait les deux gestes vers spirit — une lecture des
callbacks non livrés et une réclamation atomique côté serveur — ce qui est un
appel réseau neuf, donc hors du périmètre explicite de ce vol et matière pour
V2c/V3. La PR le dit et ne le fait pas.
`Tracked in: senara-solutions/mika#1727`.

---

## Fire-Disposition

Ce plan livre des détecteurs : T1 à T4 sont des tests, et un test compte
(mika#2306). La disposition est donc requise.

**Option retenue : (a) atterrissage armé, aucune exception nommée, aucune
allowlist ouverte.**

Les quatre tests mesurent une surface **neuve de ce vol** — un drapeau de
rattrapage qui n'existe pas sur `a5ba21695` — et aucun n'est un scan de source :
ce sont quatre tests comportementaux sur du code que ce vol écrit. Il n'y a donc
**aucune violation préexistante à amnistier**, et par conséquent aucune
allowlist à ouvrir, pas même vide : une place nommée pour des dérogations serait
ici une invitation sans objet. Les quatre atterrissent armés et verts dans les
commits qui les justifient.

Si l'un rougit à l'atterrissage, la conduite est celle de mika#2201 : corriger le
code, **pas** élargir la mesure. Les deux élargissements tentants sont nommés
pour être refusés — allonger le `timeout(5 s)` de T1 au lieu de comprendre
pourquoi l'ouverture n'arme pas, et relâcher le recul de T1 au-delà de la seconde
au lieu de constater que la prise du drapeau n'a pas eu lieu.

Les options (b) et (c) sont écartées. **(b) expédier désarmé** (`#[ignore]`) est
refusé par le ticket lui-même — « chaque garde arrive avec ses tests, dans le
même commit ; les tests ne se reportent jamais à un vol suivant » — et laisserait
sans preuve la seule propriété qui empêche ce vol de livrer deux fois le même
callback à un utilisateur. **(c) s'arrêter et escalader** n'a pas lieu d'être :
les trois questions qui pouvaient bloquer ce vol sont mesurées et tranchées au
§ 0 — l'overflow traverse-t-il le filtre d'agent (oui), la première ouverture
passe-t-elle par le même site que les suivantes (oui), un callback semé en base
est-il visible du balayage (oui, par `update_task_completed`, pas par
`update_task_status`).

---

## Definition of Done

- Les deux commits du § 3 sont sur la branche, chacun vert avec ses tests.
- `cargo test -p mika-cli` est vert ; les tests de V2b-2b-2, V2b-2b-3 et
  V2b-2b-4 le sont **sans avoir été modifiés** — ni leur forme, ni leurs
  assertions.
- `cargo clippy --all-targets --all-features -- -D warnings` et
  `cargo fmt --check` sont verts.
- `bash scripts/verify-egress-manifest.sh --report` est vert, `docs/egress/`
  intact. S'il est rouge, le pilote s'arrête et le dit.
- `bash scripts/verify-pipeline.sh origin/main` est vert.
- `git diff --numstat origin/main...HEAD` donne au plus 279 lignes ajoutées, plan
  exclu.
- Le corps de `poll_callback_tasks` est identique à sa forme sur `a5ba21695`, et
  son appel sous `POLL_INTERVAL_TICKS` est toujours là.
- `terminal_callback_frame` est inchangé, `frame_annonce_un_trou` n'a pas de bras
  `_`, et `Unknown` ne produit aucun effet.
- Les deux blocs de doc du § 2.2 sont corrigés dans le **commit 1**, celui du
  code qu'ils décrivent.
- La PR porte `Refs mika#1727`, et ni `Closes`, ni `Fixes`, ni `Resolves`, nulle
  part — ni dans le corps, ni dans un message de commit.
- La PR nomme la dette du balayage local (§ 6) et porte
  `Tracked in: senara-solutions/mika#1727`.
- `Code review: delegated to mika-qa (mika#2694)` figure dans le corps de la PR.

---

## Acceptance criteria

1. La **première** ouverture réussie du fil émet le signal, et une
   **réouverture** aussi. Un test le montre avec le serveur factice à séquence.
2. Un `OverflowMarker` émet le signal. Il ne met **aucun** `task_id` dans le
   canal. Un test le montre, et `Unknown` reste sans effet.
3. Un callback de la session **semé en base sans aucune trame** est livré **une
   fois** sur le signal, avec le même `CallbackResult` et le même message système
   que le poll. Le test le montre sans faire avancer le minuteur du poll : il
   appelle le drainage, pas le tick.
4. Deux signaux en file ne livrent pas deux fois le même callback. Un test le
   montre.
5. Le corps de `poll_callback_tasks` et son minuteur ne sont pas modifiés. Les
   tests existants de V2b-2b-2, V2b-2b-3 et V2b-2b-4 restent verts **sans être
   affaiblis**. Si l'un lit le canal à nu et doit changer de forme, ses
   assertions restent les mêmes.
6. Egress Manifest Lint est vert **avec `docs/egress/` intact** : le pilote lance
   `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il
   le trouve rouge, il s'arrête et le dit. La PR porte `Refs mika#1727` et aucun
   `Closes`, `Fixes` ni `Resolves`. Elle nomme la dette du balayage local.

---

## Taille estimée

Lignes de code ajoutées, commentaires et tests compris, `docs/` et `CLAUDE.md`
exclus, dans l'unité de `git diff --numstat origin/main...HEAD`.

### Commit 1 — `commands/tasks.rs`

| livrable | lignes de code (hors `docs/`) |
|---|---|
| § 2.1 champ `rattrapage: Arc<AtomicBool>`, imports, doc du choix du drapeau | 16 |
| § 2.1 création et clone du drapeau dans `open_for_avec_recul` et `from_channel` | 6 |
| § 2.1 armement sur ouverture réussie + commentaire (l'ouverture, pas la trame) | 5 |
| § 2.1 `frame_annonce_un_trou` : `match` des huit variantes + doc (pourquoi pas `matches!`) | 18 |
| § 2.1 `appliquer_la_trame` + doc (pourquoi nommée : mesurable sans serveur) | 14 |
| § 2.1 `prendre_le_rattrapage` (swap destructif) + doc | 11 |
| § 2.1 `armer_le_rattrapage` sous `#[cfg(test)]` | 5 |
| § 2.2 docs corrigées : bloc de `CallbackFeed`, bloc d'`open_for` | 14 |
| T1 première ouverture et réouverture, deux témoins positifs | 32 |
| T2 overflow / `Unknown` / témoin positif terminal | 30 |

Sous-total : 151

### Commit 2 — `tui/app.rs`

| livrable | lignes de code (hors `docs/`) |
|---|---|
| § 2.3 déclencheur en tête du drainage + commentaires (un balayage, un callback par passage) | 13 |
| T3 callback semé en base, livré une fois sur le signal, `None` sur les deux champs qui séparent les chemins | 40 |
| T4 deux signaux, une seule livraison, les deux moitiés | 20 |

Sous-total : 73

Total estimé : 224 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

La marge est de 55 lignes, soit 20 % de la borne. Ce n'est pas de la réserve pour
élargir : elle absorbe le biais d'estimation sur T3, le seul test qui monte un
harnais de TUI, une base et un serveur de détail à la fois. Le vol tient en
**un seul** vol parce que ses deux commits ne sont pas deux PR : le premier
livrerait un signal que rien ne lit, ce que ce vol existe pour éviter. Si le
total réel dépassait 279, le vol s'arrête et le dit — il ne recoupe pas de
lui-même, et il ne reporte aucun test.

`Tracked in: senara-solutions/mika#1727`
