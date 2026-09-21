# mika#2185 — le faucheur `stuck-pending` décide sur un instantané périmé

**Ticket :** senara-solutions/mika#2185
**Type :** fix
**Surface :** `crates/mika-agent/src/task_engine/engine.rs` (`reap_orphaned_pending_issue_tasks`), `crates/mika-agent/src/task_state/tasks.rs` (`DeferredWrapperSummary`)

---

## 1. Le fait, tel que le ticket le pose

`reap_orphaned_pending_issue_tasks` forme sa liste de candidats par un seul
`find_orphaned_pending_issue_tasks`, puis boucle et décide — lecture
d'inventaire, re-armement ou expiration — sur cet instantané.

Les dispatches du même tick tournent en `tokio::spawn`
(`engine.rs:731-738`, `dispatch_undelivered_callbacks`) et leur chemin de
complétion insère un wrapper `pending` frais. Entre le `SELECT` qui a produit le
verdict « rien ne représente cette parente » et l'écriture qui l'applique, la
prémisse peut devenir fausse.

La fenêtre est confirmée par l'ordre du tick : `dispatch_undelivered_callbacks`
(ligne 737) lance des tâches détachées ; `reap_orphaned_pending_issue_tasks`
(ligne 772) s'exécute ensuite, dans le même passage de 60 s, pendant que ces
tâches écrivent. Le chemin d'écriture est
`dispatch_resume_agent` → `rearm_consumed_deferred_wrapper`
(`dispatcher.rs:1045` et `:1171`) → `rearm_deferred_callback` → insertion d'un
wrapper `pending`.

---

## 2. Ce que la lecture du code déplace — et c'est le premier livrable

Trois constats mesurés sur l'arbre à `93931d40`. Chacun change ce qu'il y a à
faire ; les livrer sans les dire produirait un correctif qui vise la moitié déjà
fermée du défaut.

### R1 — la branche re-armement est DÉJÀ fermée par mika#2413

Le ticket a été fiché le 2026-09-05 et désigne `has_live_deferred_wrapper_child`
comme le remède « sans appelant de production ». C'est encore vrai du **booléen**
(zéro appelant hors tests). Ce ne l'est plus de la question qu'il pose :
mika#2413 a livré `find_live_deferred_wrapper_child` et lui a donné un appelant
de production, `rearm_deferred_callback` (`skills/executor.rs:2890`), qui
**re-pose exactement cette question juste avant de re-armer** et rend
`RearmOutcome::AlreadyRepresented` quand un wrapper vivant est apparu.

Conséquence directe sur le chemin décrit par le ticket :

```
SELECT rend le candidat
  → un wrapper `pending` apparaît (spawn concurrent)
  → action_config = Some(config)
  → rearm_deferred_callback → garde mika#2413 → AlreadyRepresented
  → le faucheur `continue` (engine.rs:1157-1164)
```

**Le second wrapper n'est plus créé.** Le commentaire du bras
`AlreadyRepresented` dans le faucheur dit encore « Unreachable in practice » :
depuis mika#2413 cette phrase est fausse, c'est le chemin nominal sous
contention, et elle sera corrigée.

**Conséquence sur AC2 :** « prouve que sur `main` la parente est **re-armée** à
tort (rouge) » n'est plus reproductible sur cette branche. Un test écrit à la
lettre serait vert sur `main` avant tout correctif, donc vide. AC2 est réalisé
sur la branche qui reste ouverte, décrite en R2, où le coût est strictement plus
élevé.

### R2 — le trou qui subsiste est une EXPIRATION, pas un re-armement

`engine.rs:1120-1138` :

```rust
let outcome = match action_config {
    Some(config) => rearm_deferred_callback(...).await,   // garde mika#2413 ici
    None         => RearmOutcome::Unrepairable,           // AUCUNE garde
};
```

La branche `None` **n'atteint jamais `rearm_deferred_callback`**, donc jamais
aucune re-vérification, et tombe droit sur l'expiration de la parente
(`update_task_failed`, ligne 1229), après annulation de tous ses wrappers
survivants (ligne 1207) — wrapper fraîchement apparu compris.

Deux producteurs de `None`, et le second est le plus grave parce qu'il ne
demande aucune concurrence :

- **(a)** `rebuild_deferred_action_config` (ligne 4550) rend `None` dès que
  `reference_url` n'a pas la forme `https://github.com/<o>/<r>/issues/<n>`. Le
  `SELECT` du faucheur n'exige que `reference_url IS NOT NULL`, jamais cette
  forme.
- **(b)** `Err` sur `latest_deferred_wrapper_action_config` (ligne 1110-1117)
  est réduit à `None`. **Un échec de lecture DB transitoire suffit à expirer une
  parente saine**, sans TOCTOU, sans wrapper, sans contention.

C'est un fail-open sur l'opération destructrice, exactement ce qu'AC1 condamne,
et c'est là que l'anti-vacuité d'AC2 est reproductible.

### R3 — AC4 n'est pas satisfiable par un réordonnancement

AC4 exige que « l'inventaire et la re-vérification lisent le **même** moment ».
Deux lectures SQL successives ne lisent jamais le même moment, dans quelque
ordre que ce soit :

| ordre | ce qui ment |
|---|---|
| re-vérif puis inventaire | l'audit peut montrer un wrapper que la décision n'a pas vu |
| inventaire puis re-vérif | l'audit peut omettre un wrapper que la décision a vu |

La seule forme qui tient AC4 **par construction** est *une lecture, deux
usages*. Et elle est disponible sans rien ajouter au SQL :
`summarize_deferred_wrappers_of_parent` (`db/tasks.rs:2787`) rend déjà
`(id, status, completed_at)` — les trois champs exacts du prédicat de liveness —
et son propre doc-comment l'annonce : *« with the fields the stuck-pending
reaper's verdict turns on »*. L'inventaire cesse d'être un ornement du journal
pour devenir la source de la décision.

C'est aussi la réponse littérale à la conséquence (1) du ticket : la ligne
d'audit ne décrit plus « un état qui n'est plus celui du moment de la
décision », elle décrit **le** moment de la décision.

### R4 — les deux fail-safes sont OPPOSÉS, et les deux sont justes

| garde | direction | raison |
|---|---|---|
| mika#2413, dans `rearm_deferred_callback` | fail-**open** (lecture ratée → on répare) | un faux « c'est représenté » laisse une parente que rien ne représente, jamais réparée ni expirée ; un faux « je répare » coûte un point de budget |
| mika#2185, dans le faucheur, avant expiration | fail-**closed** (lecture ratée → on passe le tour) | un faux « rien ne la représente » **détruit** la parente ; ne rien faire coûte 60 s |

Les deux cohabitent parce qu'elles gardent des actions de conséquences
opposées. À écrire au site, sans quoi un futur relecteur « harmonisera » l'une
vers l'autre et rouvrira l'un des deux défauts.

### R5 — AC5 n'est pas une tautologie

Aucun index unique partiel n'interdit deux wrappers `pending` par parent :
`register_deferred_callback` (`executor.rs:2679`) ne contrôle qu'un plafond
**global** de 10 (`MAX_PENDING_DEFERRED_CALLBACKS`), jamais l'unicité par
parent. L'assertion demandée par AC5 est donc un vrai épinglage.

---

## 3. Exigences

| # | Exigence | Source |
|---|---|---|
| R-1 | Avant toute branche (re-armement **ou** expiration), le faucheur re-pose la question « quelque chose représente-t-il encore cette parente ? » et passe son tour si oui | AC1 |
| R-2 | Cette re-vérification est **fail-closed** : toute impossibilité de lire passe le tour | AC1 |
| R-3 | La re-vérification et l'inventaire d'audit lisent le même état, par construction | AC4 |
| R-4 | Le prédicat de liveness appliqué est **le même** que celui de la clause (1) de `find_orphaned_pending_issue_tasks` et de `find_live_deferred_wrapper_child` | mika#2181, R3 |
| R-5 | Une parente réellement orpheline reste re-armée ; les tests existants du faucheur restent verts sans réécriture | AC3 |
| R-6 | Un tour de fauche ne laisse jamais deux wrappers `pending` sur une parente | AC5 |
| R-7 | Un saut fail-closed est **dit** à un niveau collecté, faute de quoi un faucheur désarmé se lit comme un faucheur oisif | mika#2205 |
| R-8 | Aucun réglage nouveau, aucune variable d'environnement, aucune migration | portée |

---

## 4. Conception

### U1 — `DeferredWrapperSummary::first_live`, lecteur unique du prédicat côté Rust

`crates/mika-agent/src/task_state/tasks.rs`, à côté de `render` :

```rust
impl DeferredWrapperSummary {
    /// Le premier wrapper vivant de l'inventaire, ou `None`.
    ///
    /// Même prédicat que la clause (1) de `find_orphaned_pending_issue_tasks`
    /// et que `find_live_deferred_wrapper_child` : `pending` sur son statut
    /// seul, ou `completed` avec un `completed_at` non nul postérieur à
    /// `now - promoted_liveness_seconds`. `delivered`, `failed`, `cancelled`
    /// sont des wrappers dépensés, jamais vivants ; un `completed_at` nul ne
    /// prouve pas sa fraîcheur et ne vaut donc pas abri.
    ///
    /// L'ordre d'entrée est celui de la requête (le plus ancien d'abord), qui
    /// est l'ordre FIFO de la promotion : l'identifiant rendu est celui qui
    /// partira ensuite.
    pub fn first_live(
        wrappers: &[DeferredWrapperSummary],
        now: &str,
        promoted_liveness_seconds: i64,
    ) -> Option<&DeferredWrapperSummary>
}
```

Comparaison de bornes en ISO 8601 par comparaison de chaînes — l'ordre
lexicographique y est l'ordre chronologique (format fixe UTC, invariant posé par
le module `crate::timestamp`), ce que le SQL fait déjà avec `strftime`.

**Pourquoi une fonction pure et pas un troisième appel SQL.** Le SQL ne peut pas
répondre sur l'inventaire déjà lu, et un second aller-retour rouvrirait R3.
Le coût est nommé : cela fait un **troisième** lecteur du critère de liveness
(deux SQL, un Rust), donc la classe que `grooming_marker` (mika#2158) a dû
fermer une fois. Ce coût est payé en U5 par l'extension du test jumeau
existant, qui devient à trois branches au lieu de deux.

### U2 — l'inventaire devient la source de la décision

`engine.rs`, dans la boucle par candidat, en remplacement du bloc actuel des
lignes 1083-1097 :

```
lire l'inventaire (summarize_deferred_wrappers_of_parent)
  Err  → WARN nommé + `continue`            (fail-closed, R-2/R-7)
  Ok(w) →
      wrappers_seen = DeferredWrapperSummary::render(&w)      // audit
      si first_live(&w, now, liveness).is_some()
          → INFO nommé + `continue`         (R-1)
      sinon → poursuivre vers action_config
```

Trois propriétés, chacune porteuse :

1. **Une lecture, deux usages** — `render` et `first_live` consomment le même
   `Vec`, donc R-3 est vrai par construction et non par proximité.
2. **Avant `action_config`** — donc avant les deux branches, ce qui couvre le
   trou R2 que `rearm_deferred_callback` ne voit jamais. C'est le placement
   qu'AC1 décrit (« avant la branche re-armement/expiration »).
3. **Le `Err` cesse de dégrader l'audit pour devenir une cause de saut** — la
   valeur `"wrappers:unavailable"` disparaît du rendu : elle n'avait de sens que
   parce qu'on décidait quand même.

Le `now` est pris une fois par candidat via `crate::timestamp::now()`, et c'est
le même que celui passé à `first_live`.

### U3 — le bras `AlreadyRepresented` cesse de se dire inatteignable

Le commentaire de `engine.rs:1153-1156` (« Unreachable in practice ») est faux
depuis mika#2413. Il est réécrit pour dire ce qui est vrai : c'est la garde de
mika#2413, seconde ligne de défense derrière U2, conservée parce que
`rearm_deferred_callback` a trois autres appelants
(`rearm_consumed_deferred_wrapper` ×2, le balayage L3b) qui ne passent pas par
U2. Aucun changement de comportement ; une phrase qui ment en moins.

### U4 — le coût nommé du fail-closed, et son signal

Un inventaire durablement illisible **désarme le faucheur pour cette parente**,
indéfiniment. C'est le prix d'AC1 et il est accepté ; ce qui n'est pas
acceptable est qu'il soit muet (doctrine mika#2205 : un faucheur silencieusement
inerte se lit exactement comme un faucheur qui n'a rien trouvé à faire). D'où
deux événements distincts, à un niveau collecté :

| événement | niveau | régime attendu | lecture |
|---|---|---|---|
| `stuck_pending_skipped_wrapper_appeared` | INFO | **non vide sous contention**, silencieux hors contention | la fenêtre TOCTOU a été refermée sur une parente saine — c'est la mesure que la garde mord |
| `stuck_pending_inventory_unreadable` | WARN | **zéro** | chaque ligne est une parente que le faucheur ne peut plus ni réparer ni expirer ; toute occurrence appelle la réparation de la lecture, jamais l'assouplissement de la garde |

Champs : `task_id`, `issue`, `age_seconds`, plus `live_wrapper_id` sur le
premier. Pas de ligne `audit_events` : la population est d'une ou deux lignes
par parente et par épisode de contention, et une ligne durable par tick serait
le churn que la doctrine mika#2131 borne. Les deux événements terminaux
existants (`stuck_pending_task_rearmed`, `stuck_pending_task_expired`) gardent
leur ligne d'audit et leur champ `wrappers_seen`, désormais lu au moment exact
de la décision.

### U5 — le test jumeau passe de deux à trois branches

`db/tests/reapers.rs::test_live_wrapper_predicate_agrees_with_orphan_clause`
compare aujourd'hui `has_live_deferred_wrapper_child` à la clause (1) sur un
corpus de neuf formes `(statut, décalage de completed_at)`. Il est étendu à
`DeferredWrapperSummary::first_live`, alimenté par le même inventaire, sur le
même corpus. Le jour où l'une des trois dérive, ce test rougit au lieu de
laisser le faucheur et son SQL répondre différemment à la même question.

C'est la contrepartie exigée par la doctrine que ce test invoque lui-même
(`docs/solutions/.../asymmetric-perimeter-predicate-drift`) : une fourche
délibérée livre son test de parité dans le même commit.

---

## 5. Contrat de vérification

### T1 — anti-vacuité (AC2), sur la branche R2

Le test pilote les deux lectures : il seed une parente `pending` âgée,
**sans** wrapper et avec un `action_config` irreconstructible (une
`reference_url` hors forme GitHub-issue, cas R2-a — déterministe, sans mock de
panne DB), fait passer le `SELECT` du faucheur, insère un wrapper `pending`
entre le balayage et la décision, puis laisse le tour se terminer.

- **Rouge sur `main`** : la parente est `failed`, et son wrapper fraîchement
  apparu a été annulé par `cancel_deferred_wrappers_of_parent`.
- **Vert avec le correctif** : la parente est toujours `pending`, le wrapper est
  toujours `pending`, aucune ligne d'audit terminale n'a été écrite.

**La sortie rouge sur `main` est capturée et collée dans le corps de la PR**
(exigence explicite d'AC2).

Mécanique d'injection : l'insertion « entre le balayage et la décision » est
obtenue sans point d'injection dans le code de production, en exploitant le fait
que le `SELECT` et l'inventaire sont deux appels `AsyncDatabase` distincts — le
test appelle `find_orphaned_pending_issue_tasks` lui-même pour matérialiser le
balayage, insère le wrapper, puis appelle `reap_orphaned_pending_issue_tasks`.
Si cette forme s'avère insuffisante à l'implémentation, le repli est un point
d'injection `#[cfg(test)]` sur le faucheur ; il est nommé ici pour que le choix
soit visible et non découvert.

### T2 — anti-vacuité, variante `Err` (R2-b)

Même forme, avec un échec de lecture de `latest_deferred_wrapper_action_config`.
Si aucune panne DB n'est injectable proprement sans élargir une signature de
production, **ce test n'est pas écrit** et la population est nommée dans la PR
comme couverte par U2 mais non épinglée — plutôt qu'un test qui simule autre
chose que ce qu'il prétend.

### T3 — non-régression (AC3)

Les huit tests existants de `reap_orphaned_pending_issue_tasks`
(`engine.rs:5460` et suivants, dont
`test_stuck_pending_reaper_expires_once_budget_is_spent`) restent verts **sans
réécriture**. Ils n'ont aucun wrapper vivant au moment de la décision, donc U2
les laisse passer. Toute modification de l'un d'eux est un signal que le
correctif a débordé et doit être justifiée nommément dans la PR.

### T4 — contrôle négatif du prédicat (indispensable)

Une parente réellement orpheline — aucun wrapper, ou uniquement des wrappers
`delivered`/`failed`/`cancelled`, ou un `completed` hors fenêtre de liveness —
est **toujours** re-armée. Sans ce test, « la garde décide » serait
indiscernable de « la garde bloque tout », et U2 pourrait désarmer le faucheur
intégralement en restant vert partout ailleurs.

### T5 — AC5, assertion explicite

Après un tour de fauche sur une parente portant déjà un wrapper `pending`, la
requête `COUNT(*)` des wrappers `pending` de cette parente vaut exactement 1.
R5 établit que rien au niveau SQL ne le garantit ; c'est donc un épinglage réel.

### T6 — unités de `first_live`

Les neuf formes du corpus jumeau, plus le `now` en frontière exacte de la
fenêtre de liveness (inclusion/exclusion), plus l'inventaire vide.

---

## 6. Portée

**Dans la portée** — `reap_orphaned_pending_issue_tasks` seul, ses deux branches
comprises.

**Hors portée, délibérément :**

- **`reap_stale_blocked_dispatch_tasks` (L3b).** Même forme
  `action_config → None → Unrepairable` (`engine.rs:1469-1534`), mais la parente
  y a déjà été **remise `pending`** avant (ligne 1506), donc son
  `Unrepairable` la fait retomber dans la population que mika#2045 possède et
  répare au tick suivant, au lieu de la détruire. Coût d'un ordre de grandeur
  inférieur, population distincte, et le ticket ne le vise pas. **Ticket de
  suivi** si la mesure montre qu'il produit des parentes coincées.
- **L'exclusion des `:deferred` dans `get_child_tasks` / `task_active_dispatch`**
  — déclaré hors portée par le ticket lui-même.
- **Le résidu des 17 % de mika#2181** — ticket séparé, déclaré par le ticket.
- **L'unicité par parent des wrappers `pending` au niveau SQL** (index unique
  partiel). R5 établit qu'elle n'existe pas. L'ajouter changerait le
  comportement de `register_deferred_callback` sur un chemin que ce ticket ne
  mesure pas, avec un rayon d'action large (mika#1205, mika#2413). AC5 demande
  une **assertion**, pas une contrainte ; c'est ce qui est livré. **Ticket de
  suivi** si T5 rougit un jour.
- **Tout réglage** : aucune variable d'environnement, aucune constante déplacée,
  aucune migration. `MIKA_PROMOTED_WRAPPER_LIVENESS_SECS` et
  `MIKA_STUCK_PENDING_REAPER_GRACE_SECS` sont lus, jamais modifiés.

---

## 7. Ce que ce travail n'achète pas

- **Aucun nouveau compteur durable, aucune table.** Le seul instrument est le
  couple d'événements d'U4, et son silence ne prouve rien tant que personne ne
  le lit.
- **La garde ne supprime pas la fenêtre, elle la referme.** Un wrapper qui
  apparaîtrait entre `first_live` et `update_task_failed` — quelques
  microsecondes plus loin — resterait invisible. La fenêtre passe de « toute la
  durée d'une décision par candidat, inventaire et lecture de config compris » à
  « l'écart entre deux instructions adjacentes ». Fermer la dernière
  microseconde demanderait une transaction couvrant lecture et écriture, donc
  une écriture `IMMEDIATE` par candidat dans la boucle du faucheur : coût réel,
  gain non mesuré, **hors portée** et nommé plutôt que passé sous silence.
- **La cause de la contention** (plusieurs grooms sérialisés sur un unique
  emplacement de classe) n'est pas traitée. Ce travail empêche qu'elle tue des
  parentes saines ; il ne la fait pas disparaître.

---

## 8. Risques et haltes

| # | Risque | Halte |
|---|---|---|
| H1 | `stuck_pending_inventory_unreadable` non vide après déploiement | Une parente n'est plus ni réparable ni expirable. **Ne pas repasser la garde en fail-open** : établir pourquoi l'inventaire est illisible (métadonnées, verrou, corruption) avant toute conclusion. |
| H2 | `stuck_pending_skipped_wrapper_appeared` non vide **hors** contention, en continu sur une même parente | La garde retient une parente qu'aucun wrapper ne sert réellement : soupçonner un wrapper `completed` immortel sur le chemin `silent_turn_error` (le corps que la fenêtre de liveness borne). **Lire la fenêtre avant de toucher au prédicat.** |
| H3 | Les tests d'AC3 rougissent | Le correctif a débordé : U2 refuse une parente réellement orpheline. Réparer U2, **pas les tests**. |
| H4 | T1 est vert sur `main` avant correctif | L'anti-vacuité n'a pas mordu — vérifier que le wrapper est bien inséré après le balayage et que la branche empruntée est bien `None → Unrepairable` (R2), pas `Some(config)`, déjà fermée par mika#2413. |
| H5 | Un quatrième lecteur du prédicat de liveness apparaît | Le test jumeau d'U5 ne le voit pas. Le rattacher, ou refuser le lecteur. |

---

## 9. Definition of Done

- [ ] `DeferredWrapperSummary::first_live` livré, documenté au site avec son
      prédicat et la raison de sa forme pure (U1).
- [ ] Le faucheur re-vérifie avant les deux branches, fail-closed, sur
      l'inventaire déjà lu (U2).
- [ ] Le commentaire « Unreachable in practice » corrigé (U3).
- [ ] Les deux événements d'U4 émis à un niveau collecté, avec leurs régimes
      attendus écrits.
- [ ] Test jumeau étendu à trois branches (U5).
- [ ] T1 rouge sur `main`, vert avec le correctif, **sortie rouge collée dans le
      corps de la PR**.
- [ ] T3 vert sans réécriture des tests existants.
- [ ] T4 et T5 verts.
- [ ] `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check` verts.
- [ ] Rectifications R1 à R5 reportées dans le corps de la PR — en particulier
      qu'AC2 est réalisé sur la branche expiration et pourquoi la lettre du
      ticket n'était plus reproductible.
- [ ] Aucune variable d'environnement, aucune migration, aucun réglage modifié.

---

## 10. Acceptance criteria

Transcrits du corps de senara-solutions/mika#2185.

- **AC1** — Avant la branche re-armement/expiration, le faucheur re-interroge
  `has_live_deferred_wrapper_child` pour le candidat et **passe son tour**
  (`continue`) si un wrapper vivant est apparu. Échec **fermé** : `Err` passe
  aussi son tour — ne rien faire ce tick coûte 60 s, agir à tort coûte une
  parente.

  *Réalisation :* U2, avec une lecture notée en R3. La question est re-posée au
  bon endroit et avec le bon prédicat ; elle l'est à partir de l'inventaire
  plutôt que par un second appel SQL, parce qu'AC4 exige le même moment et que
  deux lectures successives ne le donnent jamais. Le prédicat est épinglé
  identique à celui de `has_live_deferred_wrapper_child` par U5.

- **AC2** — Rejeu anti-vacuité : un test insère un wrapper `pending` entre le
  balayage et la décision (par un point d'injection ou en pilotant les deux
  appels), et prouve que sur `main` la parente est re-armée à tort (rouge) et
  qu'avec le correctif elle est laissée intacte (vert). Sortie rouge dans la PR.

  *Réalisation :* T1, sur la branche **expiration**. R1 établit que le
  re-armement à tort n'est plus reproductible depuis mika#2413 ; la branche
  restée ouverte porte un coût plus élevé (la parente est détruite, pas
  dupliquée).

- **AC3** — Non-régression : une parente réellement orpheline, sans wrapper
  apparu entre-temps, est toujours re-armée. Les tests existants de
  `reap_orphaned_pending_issue_tasks` restent verts sans réécriture.

  *Réalisation :* T3 et T4.

- **AC4** — L'inventaire AC4 et la re-vérification lisent le **même** moment,
  pour que la ligne d'audit décrive l'état sur lequel la décision a porté.

  *Réalisation :* U2, par construction (une lecture, deux usages). Voir R3 pour
  la démonstration qu'aucun ordre de deux lectures ne l'obtient.

- **AC5** — Aucune parente ne porte deux wrappers `pending` à l'issue d'un tick
  de fauche (assertion explicite).

  *Réalisation :* T5. R5 établit que rien au niveau SQL ne le garantit, donc
  l'assertion est un épinglage réel et non une tautologie.
