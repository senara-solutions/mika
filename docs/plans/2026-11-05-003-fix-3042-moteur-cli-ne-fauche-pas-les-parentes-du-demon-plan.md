# mika#3042 — le moteur de `mika chat` ne fauche plus les parentes `self_dev` du démon

| | |
|---|---|
| ticket | senara-solutions/mika#3042 |
| labels | `bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate` |
| dépôt | mika (un seul) |
| source | parking #2983, issuecomment-6653957157 et issuecomment-6655341291 |

---

## 0. Le vol, en une phrase

Les six balayages de `tick()` qui écrivent un état terminal sur une parente
`source = 'self_dev'` passent sous la garde `!self.dispatcher.cli_mode` déjà
écrite deux fois dans le même fichier, et un scan structurel épinglé par un test
refuse le septième.

Ce n'est pas « trois appels à garder » : c'est **un prédicat** — *écrit un état
terminal sur une parente `self_dev`* — appliqué à l'échelle de réparation
entière. Le § 2.1 montre que trois sites de plus que ceux nommés par le ticket
satisfont ce prédicat, et pourquoi les laisser dehors rendrait le détecteur
d'AC3 ou rouge à l'atterrissage, ou allowlisté trois fois pour le même défaut.

---

## 1. L'état mesuré

### 1.1 Le défaut

`mika chat` construit son propre `TaskEngine` contre la base du conteneur,
qu'il **partage** avec le démon (`crates/mika-cli/src/commands/chat.rs` l. 212
`cli_mode: true`, l. 219 `TaskEngine::new`, l. 265 `spawn_tick_loop`). Son
`tick()` appelle les reapers de parentes sans garde, alors que les deux
balayages voisins qui touchent un état du démon en portent une
(`engine.rs` l. 1033, étape 2a de mika#2379 ; l. 1202, restauration des
récurrentes de mika#2575).

Un TUI ouvert avant un deploy garde donc l'ancien code du reaper. Mesuré le
2026-11-05 : cinq TUI `mika` tournaient depuis le 2026-10-27 16:39Z sur un
exécutable `(deleted)`, antérieur à mika#2986 (mergé le 2026-10-31).

| population, depuis le 2026-10-07 | reaps de parentes |
|---|---|
| faits par mika-spirit (`/var/log/mika/server.log`) | 47 |
| faits par un moteur de TUI (`~/.mika/agents/{mika-dev,mika-qa}/logs/`) | 21 |
| recouvrement entre les deux | 0 |
| reaps de TUI avec un enfant `completed` depuis moins de 600 s | 10 sur 21 |

Depuis le correctif de mika#2986, le moteur des TUI a fauché quatre parentes
`ready-label` que ce correctif protège :

| parente | ticket | heure | suite |
|---|---|---|---|
| `0fb0b070` | #3003 | 2026-11-01T14:54:21Z | perdue, `task_not_dispatchable` |
| `44e1a7d2` | #2194 | 2026-11-02T13:12:21Z | `retry_success` |
| `c8b593c2` | #2913 | 2026-11-03T09:01:21Z | `retry_success` |
| `fc36bc5b` | #3040 | 2026-11-05T08:44:21Z | `retry_success` |

Les TUI ont été relancés le 2026-11-05 vers 10:07Z sur `370093d7`. Cela soigne
l'occurrence du jour, **pas la classe** : le prochain correctif d'un reaper
n'atteindra pas un TUI ouvert avant lui.

### 1.2 Le site, vérifié ligne par ligne

Les six appels vivent dans le même bloc `if self.tick_count.is_multiple_of(DB_SCAN_INTERVAL_TICKS)`
de `tick()` (`engine.rs` l. 1412). **Aucun n'existe ailleurs** hors `mod tests` —
en particulier aucun n'est appelé par `startup_recovery()`, ce qui est
load-bearing pour AC2 (§ 3).

| # | appel (ligne dans `tick()`) | prédicat `self_dev` de sa requête | écriture terminale |
|---|---|---|---|
| A1 | `reap_orphaned_parent_tasks("implement")` — 1451 | `parent.source = 'self_dev'`, `db/tasks.rs:2155` | `update_task_failed` |
| A2 | `reap_orphaned_parent_tasks("groom")` — 1455 | idem (classe paramétrée) | `update_task_failed` |
| A3 | `complete_parent_tasks_on_callback_success()` — 1461 | `find_completable_parent_tasks_on_pr_url`, `db/tasks.rs:2601` — frère de succès de A1/A2 | `update_task_completed` |
| B1 | `reap_childless_stuck_parent_tasks()` — 1480 | `parent.source = 'self_dev'`, `db/tasks.rs:2778` | `update_task_failed("stuck_in_progress_no_callback_child")` |
| B2 | `reap_orphaned_pending_issue_tasks()` — 1486 | `parent.source = 'self_dev'`, `db/tasks.rs:2955` | `update_task_failed("stuck_pending_no_deferred_wrapper")` + re-dispatch |
| B3 | `reap_stale_blocked_dispatch_tasks()` — 1494 | `parent.source = 'self_dev'`, `db/tasks.rs:3128` | `update_task_failed` + `update_task_status(PENDING)` + re-dispatch |

Un neuvième porteur du prédicat SQL existe —
`find_parents_sheltered_by_promoted_wrapper` (`db/tasks.rs:3284`) — mais il est
**read-only** (un `SELECT reference_url` d'observabilité, mika#2181) et il n'a
pas de site d'appel propre dans `tick()` : il est appelé *depuis* B2
(`engine.rs:1940`), donc la garde de B2 le couvre déjà. Il n'entre pas dans la
liste close du scan, qui porte sur les appels de `tick()` et sur les écrivains.

Le ticket nomme A1, A2, A3. B1, B2, B3 satisfont le même prédicat et **ne
figurent pas au hors-vol** du ticket.

### 1.3 Le site qui reste dehors, et la preuve qu'il n'est pas un oubli

`settle_dispatch_parents()` (l. 1472) est nommé hors vol par le ticket. Il est
aussi **hors du prédicat** : sa requête porte
`AND COALESCE(parent.source, '') != 'self_dev'` (`db/tasks.rs:2720`) — il est le
*complément* de la population des six autres, pas un chevauchement. Il reste
inchangé, et une assertion dédiée épingle cette clause (§ 2.3, test T5) : si elle
disparaît, le site entre dans le prédicat et le scan doit le voir.

`report_promotion_starvation()` (l. 1498) est *warning only, no mutation*
(mika#2169 L2b) : hors prédicat par absence d'écriture.

### 1.4 Pourquoi la garde ne supprime aucune réparation utile

B2 et B3 ne se contentent pas d'un `failed` : ils **re-dispatchent**. Un
re-dispatch depuis une TUI n'est pas une réparation, parce que le moteur CLI ne
peut pas en servir la suite — `dispatch_undelivered_callbacks` et
`promote_pending_deferred_if_idle` sont déjà sous `!cli_mode` depuis #264
(`engine.rs` l. 1434). C'est exactement le mécanisme de la parente `0fb0b070` :
fauchée par un TUI, puis `task_not_dispatchable`. Un balayage dont le même
processus ne peut pas servir la conséquence est une perte, pas un soin.

---

## 2. Le geste

### 2.1 Décision D1 — le périmètre est le prédicat, les six sites rentrent

Les trois sites que le ticket ne nomme pas (B1, B2, B3) passent sous la garde,
pour une raison qui n'est pas de confort :

AC3 demande un scan qui refuse « un appel à un balayage qui écrit un état
terminal sur une parente `self_dev` sans garde `cli_mode` ». Ce prédicat ne
distingue pas A1 de B1. Trois issues, une seule tenable :

- garder A1–A3 seuls et écrire le scan sur ce prédicat ⇒ le détecteur est
  **rouge le jour de son atterrissage** ;
- garder A1–A3 seuls et allowlister B1–B3 ⇒ trois exceptions nommées pour le
  **même** défaut, à l'identique, chacune renvoyant au même ticket : la forme
  exacte que mika#2309 condamne (« un allowlist plein de ceux-là est l'endroit
  où la régression passerait inaperçue ») ;
- garder les six ⇒ le détecteur atterrit **armé et vert, sans exception**.

On prend la troisième. Le coût est de trois appels déplacés, dans le même bloc,
sans nouvelle requête ni nouveau prédicat.

### 2.2 Le patch de production — deux blocs, zéro réordonnancement

L'ordre de l'échelle de réparation est load-bearing, et ses commentaires le
disent (« Placed AFTER the two self_dev reapers », l. 1465 ; « Runs AFTER the
three `in_progress` reapers above », l. 1482). `settle_dispatch_parents` vit
**entre** les deux groupes. On ne déplace donc aucun appel : on ouvre **deux**
blocs de garde autour des groupes tels qu'ils sont.

Forme finale du segment l. 1449–1499 de `engine.rs` :

```rust
// mika#3042 — bloc 1/2 : tout balayage qui écrit un état terminal sur une
// parente `self_dev` est réservé au démon. `mika chat` (`cli_mode: true`)
// tourne contre la base QU'IL PARTAGE avec un démon possiblement vivant, et
// avec le code de SON binaire — un TUI ouvert avant un deploy fauche avec
// l'ancien reaper (mesuré : 4 parentes `ready-label` fauchées par un TUI après
// mika#2986, dont une perdue en `task_not_dispatchable`). Même raisonnement que
// le gating de l'étape 2a (mika#2379, l. 1033) et de la restauration des
// récurrentes (mika#2575, l. 1202).
//
// Rien n'est perdu : ces six balayages n'existent QUE dans ce `tick()`, donc le
// démon les exécute au premier tick multiple de `DB_SCAN_INTERVAL_TICKS` après
// son démarrage. Et un re-dispatch écrit en mode CLI ne serait de toute façon
// pas servi — `dispatch_undelivered_callbacks` est déjà sous cette garde (#264).
if !self.dispatcher.cli_mode {
    // Reap parent self_dev tasks left in_progress after their callback
    // subtask delivered without producing a PR (#871).
    self.reap_orphaned_parent_tasks("implement").await;
    // mika#2750 : même balayage pour la classe `groom`. […commentaire existant…]
    self.reap_orphaned_parent_tasks("groom").await;

    // Auto-complete parent self_dev tasks […commentaire existant…] (mika#1162)
    self.complete_parent_tasks_on_callback_success().await;
}

// Close `manual` tracking rows […commentaire existant…] (mika#2405).
// HORS de la garde, et ce n'est pas un oubli : sa requête porte
// `COALESCE(parent.source,'') != 'self_dev'` (db/tasks.rs:2720), donc elle ne
// peut pas écrire sur la population que mika#3042 protège. Épinglé par
// `ac3_settle_dispatch_parents_reste_hors_predicat`.
self.settle_dispatch_parents().await;

// mika#3042 — bloc 2/2. Même garde, même raison : les trois balayages ci-dessous
// portent tous `parent.source = 'self_dev'` (db/tasks.rs:2777, :2944, :3120) et
// écrivent `update_task_failed`. Les deux derniers re-dispatchent en plus, ce
// qu'un moteur CLI ne peut pas servir.
if !self.dispatcher.cli_mode {
    // Reap parent self_dev issue tasks […commentaire existant…] (mika#1687).
    self.reap_childless_stuck_parent_tasks().await;
    // Reap `pending` self_dev issue parents […commentaire existant…] (mika#2045).
    self.reap_orphaned_pending_issue_tasks().await;
    // The net for `blocked` parents […commentaire existant…] (mika#2169, L3b).
    self.reap_stale_blocked_dispatch_tasks().await;
}

// Measure promotion starvation — warning only, no mutation (mika#2169, L2b).
// Hors garde : n'écrit aucun état.
self.report_promotion_starvation().await;
```

Les commentaires existants de chaque appel sont **conservés intégralement** — le
patch les réindente d'un niveau, il n'en supprime ni n'en réécrit aucun.

### 2.3 Les tests

**T1 — AC1, les deux moteurs dans le même appel.** Dans le `mod tests` de
`engine.rs`, à côté du bloc des reapers.

Fixture : un helper `seed_orphaned_groom_parent(&db)` dérivé de
`seed_completable_parent` (`engine.rs:7448`, helper du bloc mika#1162) — parente `self_dev` /
`trigger_type: "manual"` / `in_progress`, **sans** `pr_url` dans `metadata`,
plus un enfant `trigger_type: "callback"` / `action_type: "resume_agent"` /
`dispatch_class: Some("groom")` passé `completed` puis `mark_task_delivered`,
puis `backdate_task` (`engine.rs:7517`, −700 s, au-delà des 600 s de
`REAPER_GRACE_SECONDS`). Aucun frère vivant : la fixture n'en crée pas.

Le test seed **deux** bases (`test_db()` deux fois), une par moteur, et déclare
la même fixture dans chacune — deux moteurs sur une seule base feraient de
l'ordre d'exécution un facteur, et c'est précisément ce qu'on ne veut pas mesurer.

Déclenchement : `tick()` n'exécute le bloc de scan DB que sur un multiple de
`DB_SCAN_INTERVAL_TICKS`. Le test positionne `engine.tick_count = DB_SCAN_INTERVAL_TICKS - 1`
(champ privé, `engine.rs:900` — accessible depuis le `mod tests` du même fichier)
puis appelle `tick()` **une** fois. Pas de boucle de 60 tours : la cadence n'est
pas l'objet du test, et la fixture ne contient aucune ligne `pending`, donc la
phase de `fire_task` de `tick()` ne tire rien.

Assertions, dans cet ordre :

| moteur | statut attendu de la parente | lecture |
|---|---|---|
| `cli_mode: false` | `failed` | **contrôle négatif** : la fixture EST fauchable, le test ne mesure pas une absence triviale |
| `cli_mode: true` | `in_progress` | le correctif |

Le moteur CLI se construit par le motif déjà en place à `engine.rs:6699` :
`TaskDispatcher { cli_mode: true, ..Arc::try_unwrap(dispatcher)… }`. Une troisième
assertion vérifie que le moteur CLI n'a écrit **aucun** événement d'audit du
reaper (`count_audit_events_by_tool_name`), sur le modèle de
`mika2575_le_mode_cli_ne_rearme_pas` (`engine.rs:6693`) : « rien n'a changé » et
« quelque chose a changé puis a été remis » ne se lisent pas pareil.

**T2 à T5 — AC3, le scan structurel.** Nouveau module de test
`crates/mika-agent/src/task_engine/tick_cli_guard_scan.rs`, déclaré
`#[cfg(test)] mod tick_cli_guard_scan;` dans `task_engine/mod.rs`. Il lit le
source par `include_str!("engine.rs")` — le scan porte sur le **texte du corps
de `tick()`**, donc il doit pouvoir le recevoir d'ailleurs : c'est ce qui rend le
contrôle de bonne foi possible.

Trois fonctions :

1. `const TERMINAL_PARENT_SWEEPS: &[&str]` — la liste close des six noms, chacun
   suivi d'un commentaire citant le site du prédicat `source = 'self_dev'` de sa
   requête (la colonne 3 du tableau § 1.2). C'est la liste qui dit *de quoi* le
   scan parle ; elle est grep-visible et chaque entrée nomme sa donnée.
2. `fn tick_body(src: &str) -> Option<&str>` — repère `pub async fn tick(&mut self) {`
   et rend son corps en équilibrant les accolades au moyen d'un petit automate qui
   saute les littéraux de chaîne (`"…"`, échappements) et les commentaires
   (`//`, `/* */`). Un comptage naïf casserait sur les `{` des `format!` et des
   `info!`, dont `tick()` et ses voisins sont pleins.
3. `fn unguarded_sweeps(body: &str) -> Vec<&'static str>` — parcourt le corps en
   maintenant une **pile de garde** : à chaque `{` on empile
   `vrai` si le fragment qui précède contient `if !self.dispatcher.cli_mode`,
   sinon la valeur du parent ; à chaque `}` on dépile. Tout `self.<nom>(` dont le
   sommet de pile est `faux` est rendu comme violation.

Les tests :

- **T2 — le corps réel est propre.** `unguarded_sweeps(tick_body(ENGINE_SRC).expect(…))`
  est vide. Message d'échec nommant les sites trouvés.
- **T3 — contrôle de bonne foi.** Le même scan sur un `const` littéral du même
  bloc **sans** la ligne `if !self.dispatcher.cli_mode {` : les six noms sont
  rendus. Si ce test passe au vert avec un scan cassé, il rougit — c'est son seul
  rôle.
- **T4 — fail-closed.** `tick_body("async fn autre() { }")` rend `None`, et T2
  `expect`-e : renommer `tick` fait **rougir** le scan au lieu de le faire
  passer sur un corps vide. Plus : chaque entrée de `TERMINAL_PARENT_SWEEPS`
  apparaît au moins une fois dans le corps réel — une entrée morte (appel
  supprimé ou renommé en amont) rougit au lieu de laisser le scan croire qu'il
  surveille six sites alors qu'il en surveille cinq. C'est l'assertion
  auto-nettoyante de la liste close.
- **T5 — la frontière du prédicat.** `include_str!("../db/tasks.rs")` contient
  `AND COALESCE(parent.source, '') != 'self_dev'`. Si la clause part de
  `find_settleable_dispatch_parents`, `settle_dispatch_parents` devient un
  écrivain terminal de parentes `self_dev` et doit entrer dans la liste close :
  le test rougit et nomme le geste.

---

## 3. AC2 — qui balaie une parente `self_dev` quand aucun démon ne tourne

**Personne, et personne n'a à le faire.** Trois constats, chacun vérifié dans le
code :

**(a) La forme que les six requêtes sélectionnent naît dans le démon.** Les six
prédicats exigent `parent.source = 'self_dev'` **et**
`parent.trigger_type = 'manual'` ; quatre exigent en plus `parent.type = 'issue'`
et `parent.reference_url IS NOT NULL`. Les producteurs de cette forme sont des
handlers de mika-spirit : `server/ready_label_handler.rs:1494`,
`server/iterate_dispatch.rs`, `server/verdict_handler.rs`. Sans démon, aucune
parente de cette forme ne naît.

**(b) Le seul producteur accessible hors démon ne produit pas une ligne qui
avance.** L'outil d'agent `create_task` accepte `source: "self_dev"`
(`tools/create_task.rs:32`, `VALID_SOURCES`), donc une session TUI peut créer une
parente que les requêtes sélectionnent. Mais sa **progression** passe
entièrement par le démon : le dispatch du pilote, la livraison du callback
(`dispatch_undelivered_callbacks`) et la promotion du wrapper différé
(`promote_pending_deferred_if_idle`) sont déjà sous `!cli_mode` depuis #264.
Sans démon, cette ligne n'avance pas — et la faucher ne la libère pas, elle la
perd (§ 1.4, `0fb0b070`).

**(c) Le rattrapage est borné, et il ne dépend pas de la garde.** Les six appels
n'existent **que** dans `tick()` — aucun dans `startup_recovery()` (vérifié :
les seuls sites d'appel hors `mod tests` sont les l. 1451, 1455, 1461, 1480,
1486, 1494). Le balayage était donc déjà, avant ce vol, l'affaire du premier
`tick` multiple de `DB_SCAN_INTERVAL_TICKS = 60` (`engine.rs:27`), soit au plus
~60 s après le démarrage du démon. La garde ne crée aucune population orpheline
et n'allonge aucune latence : elle retire le balayage au seul processus qui ne
peut pas en servir la conséquence.

---

## 4. Ce que ce vol ne fait pas

- **Les autres balayages de `tick()` en mode CLI** — `expire_timed_out_tasks`,
  `kill_orphan_processes`, `check_callback_process_liveness`,
  `reap_silently_stalled_pilots`, `sweep_null_pid_phantoms`,
  `settle_dispatch_parents`. Aucun n'est mesuré par le ticket. Les cinq premiers
  sont hors du prédicat d'AC3 (ils n'écrivent pas un état terminal sur une
  parente `self_dev`) ; le sixième l'est par sa clause SQL (§ 1.3), épinglée par
  T5. Aucun n'entre dans la liste close.
- **La réhabilitation d'une parente déjà terminale** — reportée par mika#2986,
  au parking. Ce vol empêche la prochaine fauche ; il ne relève pas les
  précédentes.
- **La relance des TUI au deploy** — geste d'opérateur. Ce vol rend la relance
  inoffensive à oublier pour cette classe de défaut, il ne l'automatise pas.
- **La sonde AC4 de mika#2980** — distincte de celle du § 7 ci-dessous.
- **Aucun nouveau `MIKA_*`, aucune migration, aucune requête SQL nouvelle ou
  modifiée.**

### Gel des tickets (jusqu'au 01/12)

Ce vol n'ouvre aucun ticket de suivi. Un reste éventuel va en commentaire sur
mika#3042 ou dans la PR.

---

## 5. Fire-Disposition

Ce vol livre des détecteurs : le scan structurel d'AC3 (T2), son contrôle de
bonne foi (T3), ses deux gardes fail-closed (T4) et l'assertion de frontière de
prédicat (T5). La section est donc requise.

**Option retenue : (a) exception nommée en allowlist — et l'allowlist est vide.**

Détail d'implémentation :

- **Zéro violation existante à l'atterrissage.** Les six sites du § 1.2 passent
  sous la garde dans le **même commit** que le scan (§ 9, commits 1 puis 2). Le
  détecteur atterrit **armé et vert**. Il n'y a donc aucune entrée à écrire :
  une allowlist vide n'est pas une exception, c'est l'absence de dette.
- **Aucun mécanisme d'exception n'est livré.** Le scan n'a ni liste d'ignorés,
  ni attribut de contournement, ni variable d'environnement de désarmement.
  Ajouter un septième balayage terminal hors garde demandera de le **corriger**,
  pas de l'allowlister. C'est délibéré : un allowlist de sites portant le même
  défaut, chacun renvoyant au même ticket, est la forme que mika#2309 condamne,
  et c'est le raisonnement du § 2.1.
- **L'assertion auto-nettoyante est T4**, et elle porte sur la liste close
  elle-même : chaque entrée de `TERMINAL_PARENT_SWEEPS` doit apparaître au moins
  une fois dans le corps réel de `tick()`. Une entrée devenue stale — appel
  supprimé, renommé, déplacé hors de `tick()` — rougit. Sans elle, le scan
  pourrait surveiller cinq sites en croyant en surveiller six, et son vert ne
  voudrait plus rien dire.
- **Le fail-closed est l'autre moitié de T4** : `tick_body` rend `Option`, T2
  `expect`-e. Renommer `tick` fait rougir le détecteur, jamais passer sur un
  corps vide — le mode de panne d'un scan de texte source.

Aucune option (b) : rien n'atterrit `#[ignore]`. Aucune option (c) : rien ne
remonte à l'opérateur, le périmètre est tranché par D1 (§ 2.1).

---

## 6. Acceptance criteria

Transcrits depuis le corps de mika#3042.

- **AC1.** En `cli_mode`, `tick()` n'appelle ni `reap_orphaned_parent_tasks`
  (aucune des deux classes) ni `complete_parent_tasks_on_callback_success`. Le
  même test, dans le même appel, porte le contrôle négatif : la même fixture (une
  parente `self_dev` dont un enfant de groom est `delivered` depuis plus de 600 s,
  sans frère vivant) est fauchée par un moteur démon, et laissée `in_progress`
  par un moteur CLI.
  → T1 (§ 2.3).
- **AC2.** Le plan dit qui balaie une parente `self_dev` quand aucun démon ne
  tourne, ou pourquoi personne n'a à le faire. Une parente `self_dev` naît d'un
  flux du démon ou d'une session qui le suppose, et c'est au plan de le vérifier.
  → § 3, trois constats (a)(b)(c), chacun cité sur un site du code.
- **AC3.** Un scan épinglé par un test refuse, dans `tick()`, un appel à un
  balayage qui écrit un état terminal sur une parente `self_dev` sans garde
  `cli_mode`. Le contrôle de bonne foi est le même scan sur une copie du corps
  sans la garde.
  → T2 (corps réel propre), T3 (copie sans garde ⇒ six violations), T4
  (fail-closed + liste close vivante), T5 (frontière `settle_dispatch_parents`).
- **AC4.** Sonde : sur les 14 jours qui suivent le deploy, on compte les lignes
  « transitioned orphaned parent to failed » dans `~/.mika/agents/*/logs/`, et on
  l'écrit sur le ticket. La cible est 0. La présence de la ligne dans
  `/var/log/mika/server.log` est le contrôle positif : le démon fauche toujours.
  → § 7.

**Portée élargie par D1 (§ 2.1) :** AC1 est tenu pour les trois appels qu'il
nomme ; AC3 l'est pour les **six** du § 1.2, parce que son prédicat ne distingue
pas les trois autres. Le vol tient donc strictement plus que la lettre d'AC1,
par nécessité d'AC3.

---

## 7. Definition of Done

- [ ] Les six appels du § 1.2 sont sous `!self.dispatcher.cli_mode`, en **deux**
      blocs, sans réordonnancement de l'échelle et sans perte d'un commentaire
      existant.
- [ ] `settle_dispatch_parents` et `report_promotion_starvation` sont **hors**
      garde, chacun avec un commentaire disant pourquoi (clause SQL / absence
      d'écriture).
- [ ] T1 passe, et son contrôle négatif (moteur démon ⇒ `failed`) est dans le
      **même** `#[tokio::test]` que l'assertion CLI.
- [ ] T2 passe sur le corps réel ; T3 rend les six noms sur la copie sans garde ;
      T4 rougit sur un `tick` introuvable et sur une entrée stale de la liste
      close ; T5 passe.
- [ ] Les tests existants des six reapers restent verts, **sans modification** :
      ils appellent les fonctions directement (`engine.reap_orphaned_pending_issue_tasks().await`,
      etc.), jamais via `tick()`, donc la garde ne les touche pas. Vérifié sur les
      ~25 sites d'appel du `mod tests` (l. 7681, 7772, 8166, 8776, 9321, …).
- [ ] `cargo test -p mika-agent` vert.
- [ ] `cargo clippy --all-targets -- -D warnings` vert, `cargo fmt --check` vert.
- [ ] `scripts/verify-pipeline.sh origin/main` vert.
- [ ] Corps de PR : `Closes #3042`, la commande de la sonde AC4 avec sa date
      d'échéance (2026-11-19), et
      `Code review: delegated to mika-qa (mika#2694)`.
- [ ] Aucun fichier sous `docs/` autre que ce plan n'est touché ⇒ pas de
      `docs-sync`, pas de `scripts/sync-agent-docs.sh`.

---

## 8. La sonde AC4, et comment elle se ferme

Geste d'opérateur, 14 jours après le deploy du correctif. Échéance :
**2026-11-19**.

```
grep -c "transitioned orphaned parent to failed" ~/.mika/agents/*/logs/*.log
grep -c "transitioned orphaned parent to failed" /var/log/mika/server.log
```

Lecture :

| première commande | seconde | verdict |
|---|---|---|
| 0 | > 0 | **vert** — la classe est fermée, et le contrôle positif prouve que l'instrument voit encore quelque chose |
| 0 | 0 | **indécis** — aucun reap du tout sur la fenêtre : la sonde n'a rien mesuré, elle n'a pas réussi. Prolonger. |
| > 0 | — | **rouge** — un moteur de TUI fauche encore. Les lignes nomment la parente ; rattacher au ticket. |

La ligne du démon est le contrôle positif *indispensable* : sans elle, un zéro
des deux côtés se lit comme un succès alors qu'il ne dit que « rien ne s'est
passé ». Le résultat est écrit en commentaire sur mika#3042 (gel des tickets).

Deux réserves honnêtes, à écrire avec le chiffre :

- Un TUI lancé **avant** le deploy du correctif porte encore l'ancien code —
  c'est la classe même du ticket. Un rouge dont l'horodatage précède la relance
  des TUI n'est pas une régression du correctif. Noter la date de relance avec le
  compte.
- La fenêtre de mesure du ticket (2026-10-07 → 2026-11-05, ~29 j) a produit 21
  reaps de TUI ; 14 jours sans aucun est un signal, mais pas une preuve d'absence.

---

## 9. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/task_engine/engine.rs` (deux blocs de garde + commentaires de justification, réindentation des six appels) | 45 |
| `crates/mika-agent/src/task_engine/mod.rs` (déclaration `#[cfg(test)] mod tick_cli_guard_scan`) | 4 |
| T1 — AC1 (`engine.rs` `mod tests` : helper `seed_orphaned_groom_parent` + test démon/CLI dans un appel) | 120 |
| T2–T5 — AC3 (`crates/mika-agent/src/task_engine/tick_cli_guard_scan.rs` : liste close, `tick_body`, `unguarded_sweeps`, quatre tests) | 185 |

Total estimé : 354 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 354 < 429 (borne par vol) et
354 < 1000 (seuil par PR) ; la borne par vol est la plus serrée des deux, et
c'est elle qu'on compare. Trois raisons de fond, pas seulement arithmétiques :

- **Le patch de production est un déplacement, pas une écriture.** 45 lignes pour
  six appels qui changent d'indentation sous une garde dont le texte exact existe
  déjà deux fois dans le même fichier (l. 1033, l. 1434). Aucune requête SQL
  nouvelle ou modifiée, aucun nouveau `MIKA_*`, aucune migration, aucun nouveau
  champ de struct, aucune signature touchée.
- **Les deux tiers du total sont du test dont la fixture se dérive.**
  `seed_completable_parent` (`engine.rs:7448`) et `backdate_task`
  (`engine.rs:7515`) existent ; T1 en dérive une variante groom-sans-`pr_url`.
  Le motif de construction d'un moteur `cli_mode: true` existe
  (`engine.rs:6818`), celui d'une assertion « le CLI n'a rien écrit du tout »
  aussi (`mika2575_le_mode_cli_ne_rearme_pas`, `engine.rs:6813`).
- **Le poste le plus lourd (185 l.) est borné par son contrat.** Le scan est un
  automate de ~70 lignes sur un `&str` plus quatre tests courts ; il n'a ni
  dépendance nouvelle, ni interaction avec la base, ni point de configuration.
  Le risque de dérive serait d'y parser du Rust : c'est explicitement exclu — le
  scan compte des accolades hors chaîne et hors commentaire, rien de plus, et
  T3/T4 sont là pour qu'un scan trop naïf rougisse au lieu de mentir.

Aucune découpe en vols n'est nécessaire, donc aucune ligne `Tracked in:`.

---

## 10. Ordre de commit

Deux commits, dans cet ordre, chacun vert seul.

1. **`fix(3042): les six balayages terminaux de parentes self_dev sont réservés au démon`**
   — `engine.rs` (les deux blocs de garde, § 2.2) + T1 (§ 2.3). Vert seul : T1
   est le test du geste, et aucun test existant des reapers ne passe par `tick()`
   (DoD).
2. **`test(3042): un scan refuse un balayage terminal de parente self_dev hors garde cli_mode`**
   — `tick_cli_guard_scan.rs` + sa déclaration dans `mod.rs` (T2–T5). Vert seul
   **parce que** le commit 1 l'a précédé : dans l'autre ordre le détecteur
   atterrit rouge, et c'est précisément le mode d'atterrissage que le § 5 refuse.

Le plancher est le commit 1 : sans lui, le commit 2 ne peut pas être vert. Un
mika-qa qui ne lirait qu'un seul commit doit lire le premier.
