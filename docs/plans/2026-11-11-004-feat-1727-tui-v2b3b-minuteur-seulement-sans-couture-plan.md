# Vol V2b-3b — le minuteur ne balaie plus quand une couture suit l'agent courant (mika#1727)

**Ticket :** senara-solutions/mika#1727 — vol V2b-3b
**Branche :** `feat/1727/tui-v2b3b-minuteur-seulement-sans-couture`
**Base :** `main` @ `2186ebb9` (merge de V2b-3a, PR #3074)
**PR :** `Refs mika#1727` — **jamais** `Closes`, `Fixes` ni `Resolves`. Le ticket
se ferme au merge de V4.

Ce vol déplace une décision, il n'ajoute pas un chemin. Le balayage
(`poll_callback_tasks`) reste mot pour mot ce qu'il est ; ce qui change, c'est
**qui l'appelle**. Aujourd'hui le minuteur de 5 s l'appelle toujours ; après ce
vol, il ne l'appelle plus quand une couture suit l'agent courant, et deux
sources le remplacent : le signal du fil (livré par V2b-3a) et un changement de
session que le drainage détecte lui-même.

---

## 0. Ce qui est mesuré, avant toute conception

Tous les faits ci-dessous sont lus sur `2186ebb9`, pas rappelés de mémoire. Ils
sont la raison de chaque choix des sections suivantes.

**F1 — le minuteur.** `POLL_INTERVAL_TICKS = 167` (`tui/app.rs:669`, ≈ 5 s à
30 ms par tick). Le bloc qui balaie est `tui/app.rs:1134-1141` :

```rust
if !self.is_team_mode()
    && self.tick_count.is_multiple_of(POLL_INTERVAL_TICKS)
    && self.status == AgentStatus::Idle
{
    self.poll_callback_tasks().await;
}
```

Il ne regarde pas `callback_feed`. C'est la seule ligne de condition que ce vol
touche dans le tick.

**F2 — le drainage tourne à chaque tick, pas tous les 167.** `tui/app.rs:1128-1132`
appelle `drain_callback_deliveries()` dès que la TUI est au repos, **sans**
`is_multiple_of`. Conséquence directe de conception : une détection posée dans
le drainage réagit en un tick (30 ms), pas en cinq secondes. C'est ce qui rend
la mémoire de session utilisable comme source de balayage.

**F3 — le balayage est filtré par session.** `tui/app.rs:1721` appelle
`get_undelivered_callback_tasks_for_session(&since, &self.session_id)`. Une
session neuve a donc son propre arriéré : rien dans la base ne relie l'arriéré
de la session précédente à la nouvelle, et un balayage fait sous l'ancienne
session ne couvre pas la nouvelle. C'est ce qui rend le changement de session
**nécessaire** comme seconde source, et pas seulement prudent.

**F4 — `CallbackFeed` ne retient pas son agent.** `commands/tasks.rs:816-822` :
le type porte `rx`, `base_url`, `token`, `rattrapage`. L'agent n'existe que dans
la tâche du superviseur (`let agent = agent_id.to_string();`, l. 851), où il sert
le filtre `flux.consume(Some(&agent), …)` (l. 884). Il est donc **figé à
l'ouverture** et aucun des trois sites qui changent `session_id` ne rouvre le fil :
`/new` (`tui/commands/handlers.rs:167`), le changement d'agent (`chat.rs:793`,
qui change aussi `agent_name` l. 803) et le redémarrage du worker (`chat.rs:897`).
`open_for` n'est appelé qu'une fois, au lancement (`chat.rs:569`).

**F5 — deux agents, deux faits différents.** `next_candidate(&mut self, agent_id,
session_id)` (l. 925) prend déjà un agent **par appel** : c'est l'agent *courant
de la TUI*. L'agent que F4 décrit est celui que *le fil* suit. Sur main les deux
coïncident toujours, parce que rien ne les fait diverger avant le premier
changement d'agent — et c'est précisément leur divergence que le critère 3 vise.
Le second ne peut donc pas être déduit du premier : il faut le retenir.

**F6 — `from_channel` n'a que deux sites d'appel.** `tui/app.rs:2450` et
`commands/tasks.rs:2093`. Changer sa signature coûte deux lignes, ce qui écarte
le besoin d'une surcharge ou d'un agent par défaut.

**F7 — `tick_count` n'est pas incrémenté par `tick_agent_mode`.** `tick()`
l'avance (`tui/app.rs:978`) puis dispatche ; les tests appellent
`tick_agent_mode()` directement. Un test peut donc poser `tick_count` et décider
s'il est au multiple. Note de rigueur : `0.is_multiple_of(167)` est **vrai**, donc
la valeur par défaut est déjà un multiple — un test qui veut un tick *hors*
multiple doit poser la valeur explicitement, et un test qui en veut un *au*
multiple gagne à poser `POLL_INTERVAL_TICKS` pour dire ce qu'il mesure au lieu de
l'hériter.

**F8 — une ligne `completed` n'est pas forcément visible du balayage.**
Mesuré au vol V2b-3a et re-confirmé ici : `update_task_status(id, "completed")`
laisse `completed_at` nul, et c'est `update_task_completed` qui rend la ligne
visible de `get_undelivered_callback_tasks_for_session`. Le helper de test
`couture_dun_callback_livrable` emploie le premier ; le helper `terminer_la_ligne`
(V2b-3a, `tui/app.rs:2647`) emploie le second. **Tout test de ce vol qui veut
qu'un balayage trouve quelque chose doit passer par `terminer_la_ligne`** —
sinon il serait vert parce que la ligne est invisible, pas parce que le minuteur
se tait (mika#2205).

**F9 — la doc qui ne dira plus le vrai.**
`docs/solutions/architecture-patterns/callback-tui-delivery-polling.md:73-82`
décrit « TUI tick loop (every ~5s when idle) → `poll_callback_tasks()` » comme
*le* chemin de livraison. Ce fichier n'est pas dans la liste `DOCS` de
`scripts/sync-agent-docs.sh` (vérifié : neuf entrées, toutes à la racine de
`docs/`), donc le corriger ne déclenche pas le job `docs-sync`.

---

## 1. La forme : deux mémoires, deux domiciles, et pourquoi pas un seul

Ce vol a besoin de retenir deux choses. Les mettre au même endroit serait plus
court et serait faux.

### 1.1 L'agent va dans `CallbackFeed`

Parce que c'est le fil qui le fige (F4) : le filtre `consume(Some(&agent), …)`
est posé à l'ouverture et rien ne rouvre le fil. La couture est donc la seule
à savoir quel agent elle sert. Le poser dans `tui/app.rs` reviendrait à y recopier
une valeur que `app.rs` détient déjà sous le nom `agent_name`, et à perdre au
passage la seule question qui compte — *est-ce le même ?*

Le périmètre du vol dit « `tui/app.rs` ne détient toujours ni l'URL, ni le jeton,
ni le nom de la lecture HTTP ». Mettre l'agent dans le type va dans le même sens :
la couture gagne un quatrième fait qu'elle porte, `app.rs` n'en gagne aucun.

**La méthode est un prédicat, pas un accesseur.** `suit_lagent(&self, agent_id:
&str) -> bool`, et non `agent() -> &str`. Un accesseur rendrait au tick une chaîne
qu'il devrait comparer lui-même, c'est-à-dire une seconde occasion de comparer du
mauvais côté ; le prédicat n'en laisse qu'une, et elle vit dans le type qui
détient la donnée.

### 1.2 La session va dans `App`

Parce que la session n'est **pas** une propriété du fil : elle change sous lui,
sans réouverture, aux trois sites de F4, et le balayage qu'elle commande lit la
base (F3), pas le fil. Un champ privé `derniere_session_balayee: Option<String>`.

**Pourquoi une `Option<String>` et pas un `bool` « j'ai déjà balayé ».** Avec un
booléen, la deuxième session ne serait jamais balayée : c'est la **comparaison**
qui porte la propriété, pas le fait d'avoir balayé une fois.

**Pourquoi `None` au départ est le bon départ.** `None` dit « aucune session
balayée », donc le tout premier drainage balaie. C'est exactement ce qu'il faut
au lancement — à cet instant, tout callback fini de la session est un trou — et
c'est sans danger : `mark_task_delivered` réclame atomiquement, donc ce balayage
ne peut pas doubler une livraison, qu'elle vienne du fil ou d'une autre TUI.

### 1.3 Un seul balayage par passage du drainage

Les deux sources peuvent être vraies au même passage. Le périmètre l'interdit
explicitement : « un seul appel à `poll_callback_tasks` par passage du drainage ».
La forme qui le tient est un `if` unique sur une disjonction, **et** la prise du
signal évaluée avant le `if`, hors de tout court-circuit — § 2.3 dit pourquoi
l'ordre des opérandes serait sinon un piège silencieux.

---

## 2. Les gardes et leur domicile

### 2.1 `CallbackFeed` retient l'agent qu'il suit (`crates/mika-cli/src/commands/tasks.rs`)

1. Un champ `agent_id: String` sur la struct.
2. `open_for_avec_recul` le pose : il construit déjà `let agent = agent_id.to_string();`
   pour la tâche spawnée — le champ prend un second `to_string()` plutôt qu'un
   `Arc` partagé, la valeur étant un nom d'agent, pas un état mutable.
3. `from_channel` prend `agent_id: &str` en **premier** paramètre, et les deux
   sites de F6 l'adaptent. **Pas de valeur par défaut** : un défaut ferait passer
   un test pour vert sur un agent qu'il n'a pas nommé, ce qui est exactement le
   mode d'échec que le critère 3 mesure.
4. La méthode :

```rust
/// Quel agent ce fil suit-il — et est-ce celui que la TUI sert en ce moment ?
///
/// **Deux agents, pas un.** Celui-ci est figé à l'ouverture, parce que c'est
/// sur lui que `consume` filtre les trames ; celui de [`CallbackFeed::next_candidate`]
/// voyage par appel, parce que c'est l'agent **courant** de la TUI. Un
/// changement d'agent les sépare : le fil continue de suivre l'ancien, et nul
/// ne le rouvre. C'est ce cas-là que le minuteur doit couvrir (mika#1727, vol
/// V2b-3b).
///
/// Un prédicat plutôt qu'un accesseur : rendre la chaîne laisserait l'appelant
/// la comparer, donc se tromper de côté. La comparaison vit là où vit la donnée.
pub(crate) fn suit_lagent(&self, agent_id: &str) -> bool {
    self.agent_id == agent_id
}
```

5. La doc du bloc `CallbackFeed` gagne le paragraphe correspondant : le type
   porte désormais **quatre** faits (canal, URL, jeton, rattrapage) plus l'agent,
   et le paragraphe dit en deux phrases pourquoi cet agent-là n'est pas celui de
   `next_candidate`.

### 2.2 Le drainage détecte le changement de session (`crates/mika-cli/src/tui/app.rs`)

Un champ sur `App`, initialisé à `None` aux **deux** constructeurs (l. 710 et
l. 801) :

```rust
/// La dernière session que le drainage a balayée (mika#1727, vol V2b-3b).
///
/// `None` tant qu'aucune ne l'a été, ce qui fait du premier drainage un
/// balayage : au lancement, tout callback fini de cette session est un trou.
/// Une `Option<String>` et non un booléen — c'est la **comparaison** qui porte
/// la propriété, un « déjà balayé » ne couvrirait jamais la seconde session.
derniere_session_balayee: Option<String>,
```

Et en tête de `drain_callback_deliveries`, en remplacement du déclencheur à une
seule source livré par V2b-3a :

```rust
// Deux sources, un seul balayage (mika#1727, vol V2b-3b). Le signal est pris
// **avant** le `if` et hors de tout court-circuit : placé en second dans un
// `||`, il survivrait au balayage que le changement de session vient de faire,
// et le passage suivant en ferait un second. Un passage, un appel.
let signal = self
    .callback_feed
    .as_ref()
    .is_some_and(|f| f.prendre_le_rattrapage());
// La requête du balayage filtre sur `session_id` : un balayage fait sous
// l'ancienne session ne couvre pas la nouvelle, d'où la comparaison plutôt
// qu'un drapeau. Le drainage tourne à chaque tick, donc la détection coûte
// un tick, pas un tour de minuteur.
let session_neuve =
    self.derniere_session_balayee.as_deref() != Some(self.session_id.as_str());
if signal || session_neuve {
    // Retenue même quand le balayage vient du signal : la requête filtre sur
    // `session_id`, donc cette session **est** balayée dans les deux cas.
    self.derniere_session_balayee = Some(self.session_id.clone());
    self.poll_callback_tasks().await;
    // Une livraison relance le tour, et la boucle du fil ne doit pas en
    // empiler une seconde : même règle que le `break` du poll.
    if self.status != AgentStatus::Idle {
        return;
    }
}
```

Le reste du corps — la boucle `next_candidate`, la réclamation, la garde
d'ancienneté, la livraison — n'est pas touché.

### 2.3 Le tick ne balaie plus sous couture conforme (`crates/mika-cli/src/tui/app.rs`)

Le bloc de F1 gagne une quatrième condition, et rien d'autre :

```rust
// mika#1727, vol V2b-3b. Le minuteur n'est plus le filet de tout : quand une
// couture suit l'agent courant, le balayage vient du signal du fil et du
// changement de session, tous deux vus par le drainage ci-dessus, qui tourne
// à chaque tick. Il reprend dans deux cas — sans couture (pas de jeton, mode
// équipe, ouverture impossible), et quand la couture suit un **autre** agent,
// le fil filtrant sur l'agent figé à son ouverture, que rien ne rouvre.
if !self.is_team_mode()
    && self.tick_count.is_multiple_of(POLL_INTERVAL_TICKS)
    && self.status == AgentStatus::Idle
    && !self
        .callback_feed
        .as_ref()
        .is_some_and(|f| f.suit_lagent(&self.agent_name))
{
    self.poll_callback_tasks().await;
}
```

**Pourquoi `is_some_and` et non un `match`.** `None` doit valoir « le minuteur
balaie ». `is_some_and` rend `false` sur `None`, donc la négation rend `true`, et
le cas sans couture tombe du bon côté *par le type* et non par une branche à
tenir. Le biais va vers le balayage, jamais vers le silence — même règle que
l'`is_some_and` de la garde d'ancienneté au vol V2b-2b-3.

### 2.4 Les commentaires et docs qui ne disent plus le vrai

Dans le **même commit** que le code qui les rend faux (§ 3, commit 3) :

- **Doc de `drain_callback_deliveries` (`tui/app.rs:1608-1611`).** La phrase « elle
  doit tenir avant le vol V2b-3, **qui retire le poll** » devient inexacte : ce vol
  ne retire pas le balayage, il le retire du **minuteur**. Réécrire : la parité
  reste le contrat, et son échéance est le vol qui sortira la lecture vers spirit.
- **Doc du champ `callback_feed` (`tui/app.rs:656-658`).** « `poll_callback_tasks`
  couvre alors » reste vrai mais devient incomplet : nommer les deux cas où le
  minuteur reprend (pas de couture ; couture sur un autre agent).
- **`docs/solutions/architecture-patterns/callback-tui-delivery-polling.md`** (F9).
  Une note datée en tête de « Phase 2 : TUI Polls and Delivers » : depuis
  mika#1727 vol V2b-3b, le minuteur de 5 s n'est plus le chemin par défaut ; sous
  une couture qui suit l'agent courant, la livraison vient du fil, et le balayage
  du signal du fil ou d'un changement de session. Hors du compte de lignes de
  code, et hors `docs-sync` (F9).

Le commentaire du tick l. 1128-1130 (« **sans** `POLL_INTERVAL_TICKS` : cette
attente-là est ce que le fil rachète ») reste exact et n'est pas touché.

### 2.5 Ce qui ne change pas

Vérifié explicitement avant d'ouvrir la PR, parce que ce sont des critères :

- le **corps** de `poll_callback_tasks` — pas une ligne ;
- `crates/mika-cli/src/commands/chat.rs` et `tui/commands/handlers.rs` ;
- `crates/mika-agent/`, `crates/mika-a2a/` ;
- le plan de contrôle : `.github/workflows/`, `.claude/`, `skills/bundled/`,
  `well_known_agents.rs`, `.mika/`, tout chemin CODEOWNERS ;
- `docs/egress/` — intact, et le manifeste reste exact : ce vol n'ajoute aucun
  appel réseau, le périmètre de la déclaration `cli-spirit-tasks` reste
  `commands/tasks.rs` ;
- aucune dépendance, aucune feature.

---

## 3. Ordre des commits

Chaque commit est vert seul, et chaque garde arrive avec ses tests.

**Commit 1 — `commands/tasks.rs` : la couture dit quel agent elle suit.**
§ 2.1 + T0. Personne ne lit encore `suit_lagent` ; la garde est que la couture
sait répondre, et T0 la mesure. Le comportement de la TUI est inchangé.

**Commit 2 — `tui/app.rs` : le drainage détecte le changement de session.**
§ 2.2 + T4. À ce point le minuteur balaie encore à chaque multiple : le balayage
du drainage lui est redondant, et sans danger (§ 1.2). T4 passe donc en mesurant
sa propre raison, à condition d'être écrit hors multiple de `POLL_INTERVAL_TICKS`
— c'est le cas (§ 4).

**Commit 3 — `tui/app.rs` : le minuteur se tait, et les docs disent le vrai.**
§ 2.3 + T1, T2, T3 + § 2.4. C'est ce commit qui rend fausses les phrases de
§ 2.4, d'où leur domicile ici.

L'ordre inverse (le tick avant le drainage) serait un commit intermédiaire où ni
le minuteur ni le drainage ne couvrent une session neuve, donc un arriéré non
livré. Celui-ci n'a pas de fenêtre de ce genre.

---

## 4. Le jeu de tests

Quatre tests neufs plus un, un par critère, tous dans les fichiers qui portent la
garde. **Aucun serveur factice neuf** : `from_channel`,
`couture_dun_callback_livrable` et `terminer_la_ligne` sont ceux de V2b-2b-2 et
V2b-3a.

Un petit helper partagé évite trois recopies :

```rust
/// Une ligne de callback que le **balayage** verra : le helper de V2b-2b-2 la
/// crée, `terminer_la_ligne` la rend visible. Les deux, parce qu'un statut posé
/// par `update_task_status` laisse `completed_at` nul et reste invisible de
/// `get_undelivered_callback_tasks_for_session` — un test monté sur lui seul
/// serait vert sans rien mesurer (mika#2205).
async fn une_ligne_balayable(app: &App<'static>) -> String {
    let (id, _tx, _feed) = couture_dun_callback_livrable(app, "pending", None, None).await;
    terminer_la_ligne(app, &id).await;
    id
}
```

La couture rendue par le helper est jetée : T1 et T3 construisent la leur, parce
qu'ils ont besoin d'en choisir l'agent. Elles se montent sur un canal vide, donc
`next_candidate` sort au premier `try_recv` sans jamais faire d'appel HTTP :
leur URL et leur jeton sont des chaînes **vides**, ce qui est un non-endpoint
manifeste et non un port qu'un voisin pourrait servir (mika#2569 n'a pas d'objet
ici — aucun appel n'est fait, et aucun port n'est réservé puis relâché).

### T0 — `suit_lagent` répond des deux côtés (`commands/tasks.rs`)

Une couture `from_channel("mika-dev", …)`. Deux assertions dans le même test :
`suit_lagent("mika-dev")` est vrai, `suit_lagent("mika-qa")` est faux. Le témoin
négatif est ce qui sépare « la méthode répond » de « la méthode rend toujours
vrai ».

### T1 — critère 1 : couture conforme, le minuteur se tait (`tui/app.rs`)

Montage : `une_ligne_balayable(&app)` ; `app.callback_feed =
Some(CallbackFeed::from_channel(&app.agent_name, String::new(), String::new()).1)` ;
`app.derniere_session_balayee = Some(app.session_id.clone())` ; **pas** de signal
armé ; `app.tick_count = POLL_INTERVAL_TICKS`. Puis `app.tick_agent_mode().await`.

Trois assertions, et la troisième est celle qui compte :
- `request_rx.try_recv().is_err()` — rien n'est parti vers l'agent ;
- `app.status == AgentStatus::Idle` — aucun tour n'a été relancé ;
- la ligne en base est **toujours** `completed`, pas `delivered`. Sans elle, le
  test ne distinguerait pas « rien livré » de « réclamé puis perdu ».

### T2 — critère 2 : sans couture, le minuteur livre (`tui/app.rs`)

Même montage, `app.callback_feed = None`, même `derniere_session_balayee` et même
`tick_count`. Une livraison arrive, `original_status == "completed"`,
`app.status == AgentStatus::Thinking`.

**T2 est le témoin positif de T1** : il prouve que la ligne de `une_ligne_balayable`
est bien visible du balayage et que le tick posé au multiple en déclenche bien un.
Le silence de T1 est donc attribuable à la couture, et à rien d'autre. Cette
dépendance est écrite dans le doc-comment de T1, pour qu'affaiblir T2 se voie.

### T3 — critère 3 : couture d'un autre agent, le minuteur livre (`tui/app.rs`)

Identique à T1 sauf la couture : `from_channel("un-autre-agent", …)`, avec
`"un-autre-agent"` différent de `app.agent_name`. Une livraison arrive. Une
assertion de montage ouvre le test — `app.agent_name != "un-autre-agent"` — sinon
le test mesurerait T1 sous un autre nom le jour où le harnais changerait d'agent.

### T4 — critère 4 : le changement de session balaie une fois (`tui/app.rs`)

Deux moitiés dans un seul test, parce que la seconde ne veut rien dire sans la
première.

*Première moitié.* `app.derniere_session_balayee = Some("s-precedente".into())` ;
une ligne balayable de la session **courante** ; couture qui suit l'agent courant
(donc le minuteur se tairait de toute façon après le commit 3) ; aucun signal ;
`app.tick_count = 1` — **hors** multiple de `POLL_INTERVAL_TICKS`, ce qui est la
moitié « sans passer par le minuteur » du critère. `tick_agent_mode()` → une
livraison, une seule, et `app.derniere_session_balayee == Some(app.session_id)`.

*Seconde moitié.* `app.status = AgentStatus::Idle` remis à la main — sinon la
garde du tour en cours expliquerait seule le silence et le test serait vert sur
une autre raison que la sienne (mika#2205). Une **seconde** ligne balayable semée.
`tick_agent_mode()` à nouveau, `tick_count` toujours à 1 → rien n'est livré, et la
seconde ligne est toujours `completed` en base.

---

## Fire-Disposition

Ce plan livre des détecteurs : T0 à T4 sont des tests, et un test compte
(mika#2306). La disposition est donc requise.

**Option retenue : (a) atterrissage armé, aucune exception nommée, aucune
allowlist ouverte.**

Les cinq tests mesurent une surface **neuve de ce vol** — un agent retenu par la
couture et une mémoire de session dans le drainage, dont aucun n'existe sur
`2186ebb9` — et aucun n'est un scan de source ni un parcours de corpus. Ce sont
cinq tests comportementaux sur du code que ce vol écrit. Il n'y a donc **aucune
violation préexistante à amnistier**, et par conséquent aucune allowlist à
ouvrir, pas même vide : une place nommée pour des dérogations serait ici une
invitation sans objet. Les cinq atterrissent armés et verts dans les commits qui
les justifient (§ 3).

Si l'un rougit à l'atterrissage, la conduite est celle de mika#2201 : corriger le
code, **pas** élargir la mesure. Les trois élargissements tentants sont nommés
pour être refusés — retirer de T1 l'assertion sur le statut en base au motif que
les deux autres suffisent ; remplacer le `tick_count = 1` de T4 par un multiple
parce que « ça livre quand même », ce qui effacerait la moitié « sans passer par
le minuteur » ; et laisser tomber le témoin négatif de T0, qui est la seule chose
séparant un prédicat d'une constante.

Les options (b) et (c) sont écartées. **(b) expédier désarmé** (`#[ignore]`) est
refusé par le ticket lui-même — « chaque garde arrive avec ses tests, dans le
même commit ; les tests ne se reportent jamais à un vol suivant, même si le code
reste » — et laisserait sans preuve la propriété que ce vol existe pour obtenir :
qu'une session neuve soit quand même balayée alors que le minuteur s'est tu.
**(c) s'arrêter et escalader** n'a pas lieu d'être : les quatre questions qui
pouvaient bloquer ce vol sont mesurées et tranchées au § 0 — le drainage
tourne-t-il hors multiple (oui, F2), le balayage est-il filtré par session (oui,
F3), l'agent du fil est-il distinct de l'agent courant (oui, F4/F5), une ligne
`completed` est-elle visible du balayage (non sans `update_task_completed`, F8).

---

## Definition of Done

- [ ] `CallbackFeed` porte `agent_id` et expose `suit_lagent`; `from_channel` prend
      l'agent, ses deux sites d'appel sont adaptés.
- [ ] `App` porte `derniere_session_balayee: Option<String>`, initialisé à `None`
      aux deux constructeurs.
- [ ] `drain_callback_deliveries` balaie sur le signal **ou** sur un changement de
      session, au plus une fois par passage, et retient la session balayée.
- [ ] Le bloc minuteur de `tick_agent_mode` ne balaie plus quand
      `callback_feed.suit_lagent(&agent_name)`.
- [ ] T0, T1, T2, T3, T4 écrits et verts, chacun dans le commit de sa garde.
- [ ] Les commentaires de § 2.4 et la note de
      `docs/solutions/architecture-patterns/callback-tui-delivery-polling.md`
      corrigés dans le commit 3.
- [ ] `cargo test -p mika-cli` vert — les tests de V2b-2b-2 à V2b-3a inclus, sans
      modification de leurs assertions.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` et `cargo fmt --check`
      verts.
- [ ] `bash scripts/verify-egress-manifest.sh --report` vert, `docs/egress/` intact.
- [ ] `bash scripts/verify-pipeline.sh origin/main` vert.
- [ ] `git diff --numstat origin/main...HEAD` : au plus 279 lignes ajoutées, plan
      exclu. Au-delà, le vol s'arrête et le dit.
- [ ] PR avec `Refs mika#1727`, sans `Closes`/`Fixes`/`Resolves`, nommant les deux
      dettes de § « La dette ».

---

## Acceptance criteria

1. Avec une couture qui suit l'agent courant et une ligne `completed` de la
   session en base, un `tick` au multiple de `POLL_INTERVAL_TICKS` ne livre
   **rien** quand le signal n'est pas armé et que la session n'a pas changé. Un
   test le montre (T1).
2. **Sans couture**, le même `tick` livre la ligne, comme aujourd'hui. Un test le
   montre (T2).
3. Avec une couture qui suit **un autre agent**, le même `tick` livre la ligne.
   Un test le montre (T3).
4. Quand `session_id` change, un callback de la **nouvelle** session, semé en base
   sans trame, est livré **une fois** par le drainage, sans signal armé et sans
   passer par le minuteur. Un second drainage sur la même session ne balaie pas.
   Un test le montre (T4).
5. Le corps de `poll_callback_tasks` n'est pas modifié, ni `chat.rs`, ni
   `handlers.rs`. Les tests existants de V2b-2b-2 à V2b-3a restent verts **sans
   être affaiblis**.
6. Egress Manifest Lint est vert **avec `docs/egress/` intact** : le pilote lance
   `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il le
   trouve rouge, il s'arrête et le dit. La PR porte `Refs mika#1727` et aucun
   `Closes`, `Fixes` ni `Resolves`. Elle nomme les deux dettes.

---

## 5. Ce que ce vol ne fait pas

- Il ne rouvre pas la couture pour le nouvel agent après un changement d'agent.
- Il ne sort pas vers spirit la lecture du balayage ni la réclamation.
- Il ne retire pas `POLL_INTERVAL_TICKS` ni ses trois autres consommateurs
  (messages cross-channel, compte des tâches, compte des tâches de fond).
- Il ne touche ni `chat.rs`, ni `handlers.rs`, ni `crates/mika-agent/`, ni
  `crates/mika-a2a/`, ni le plan de contrôle, ni `docs/egress/`.
- Il n'ajoute aucun appel réseau, aucune dépendance, aucune feature.

---

## 6. La dette, nommée et non traitée

Deux dettes, dites par la PR et laissées entières (gel des tickets jusqu'au
01/12 : aucun ticket ouvert, la suite se note sur #1727).

**D1 — la couture n'est pas rouverte après un changement d'agent.** Le fil filtre
sur l'agent figé à `open_for` (F4) et `chat.rs:793` change `agent_name` sans
rouvrir. Ce vol en fait une conséquence **visible et sûre** plutôt qu'un angle
mort : `suit_lagent` devient faux, et le minuteur reprend son balayage de 5 s pour
le nouvel agent. La livraison n'est donc jamais perdue, elle redevient seulement
lente. Rouvrir le fil est le geste qui annule cette reprise ; il n'est pas fait ici.

**D2 — le balayage et la réclamation restent locaux.** `poll_callback_tasks` lit
la base SQLite et `mark_task_delivered` y écrit, dans le processus de la TUI.
Les sortir vers spirit est hors de V2b-3, nommément.

`Tracked in: senara-solutions/mika#1727`

---

## Taille estimée

Lignes de code ajoutées, commentaires et tests compris, `docs/` et `CLAUDE.md`
exclus, dans l'unité de `git diff --numstat origin/main...HEAD`.

### Commit 1 — `commands/tasks.rs`

| livrable | lignes de code (hors `docs/`) |
|---|---|
| § 2.1 champ `agent_id` + pose dans `open_for_avec_recul` et `from_channel` | 5 |
| § 2.1 `from_channel` prend l'agent + les deux sites d'appel adaptés | 4 |
| § 2.1 `suit_lagent` + doc (deux agents, et pourquoi un prédicat) | 12 |
| § 2.1 doc du bloc `CallbackFeed` : l'agent du type vs celui de `next_candidate` | 12 |
| T0 `suit_lagent`, témoin positif et témoin négatif | 14 |

Sous-total : 47

### Commit 2 — `tui/app.rs`, le drainage

| livrable | lignes de code (hors `docs/`) |
|---|---|
| § 2.2 champ `derniere_session_balayee` + doc + init aux deux constructeurs | 10 |
| § 2.2 déclencheur à deux sources + commentaires (ordre d'évaluation, retenue) | 18 |
| § 4 helper `une_ligne_balayable` + doc | 8 |
| T4 changement de session, les deux moitiés | 46 |

Sous-total : 82

### Commit 3 — `tui/app.rs`, le tick et les docs

| livrable | lignes de code (hors `docs/`) |
|---|---|
| § 2.3 quatrième condition du tick + commentaire (les deux cas de reprise) | 12 |
| T1 couture conforme, le minuteur se tait, trois assertions | 30 |
| T2 sans couture, le minuteur livre — témoin positif de T1 | 26 |
| T3 couture d'un autre agent, le minuteur livre + assertion de montage | 28 |
| § 2.4 commentaires corrigés : champ `callback_feed`, doc de `drain_callback_deliveries` | 12 |

Sous-total : 108

Total estimé : 237 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

La marge est de 42 lignes, soit 15 % de la borne. Ce n'est pas de la réserve pour
élargir : elle absorbe le biais d'estimation sur T4, le seul test à deux moitiés,
et sur les quatre tests de TUI qui montent chacun un harnais, une base et une
couture. Le vol tient en **un seul** vol parce que ses trois commits ne sont pas
trois PR : le premier livrerait un prédicat que personne ne lit, et le deuxième
une seconde source de balayage pendant que le minuteur balaie encore — ce vol
existe pour que le minuteur se taise, et c'est le troisième commit qui le fait.
Si le total réel dépassait 279, le vol s'arrête et le dit — il ne recoupe pas de
lui-même, et il ne reporte aucun test.

`Tracked in: senara-solutions/mika#1727`
