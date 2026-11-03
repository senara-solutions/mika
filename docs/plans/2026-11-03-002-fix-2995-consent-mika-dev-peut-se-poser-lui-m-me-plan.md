# mika#2995 — tranche du 2026-11-03 : pas de tour mika-dev pour un `ready-label` dont le `ready` a été retiré par un humain

> **Tranche ouverte le 2026-11-03** (GO de Vincent relayé par samidarko), sortie du
> parking mika#2983 commentaire 2 partie 1 (`issuecomment-6560377724`), sans ticket neuf.
> Plans antérieurs du même ticket, tous deux mergés :
> `docs/plans/2026-10-31-006-…-plan.md` (vol 1, AC1/AC3/AC4 par la voie B) et
> `docs/plans/2026-11-01-001-…-plan.md` (vol 2, AC2 réécrite voie B).
>
> **Cette tranche n'écrit aucun label, ni pose ni retrait.** La sonde AC5, ouverte le
> 2026-11-01T06:27:37Z et relevée le 2026-12-01, est donc intacte — vérifié par un `grep`
> sur le diff (AC-T5, § 9). Le corps de PR porte `Refs mika#2995 (tranche consentement
> relu)`, **jamais** `Closes`. Merge par Vincent.

---

## 1. Ce que cette tranche livre, et ce qu'elle ne livre pas

| | |
|---|---|
| **Livre** | une troisième garde dans `dispatch_resume_agent`, au rang de mika#2671 : un wrapper différé dont la parente est une tâche `ready-label:` et dont le `ready` a été retiré par un humain ne paie **ni tour LLM ni dispatch** ; le wrapper reçoit son terminal avec un motif nommé, la parente reçoit un terminal qui dit pourquoi, une ligne d'audit est posée ; la décision D4 sur une timeline illisible, **avec sa borne** |
| **Ne livre pas** | la partie 2 du commentaire 2 (un tour qui ferme un ticket gardé ouvert pour une sonde — site mika#1646, reste au parking) ; le commentaire 1 (qui retire `ready` au merge sans `Closes` — décision de doctrine pour Vincent, reste au parking) ; la fermeture du ticket, qui attend AC5 au 2026-12-01 |
| **Ne touche pas** | `crates/mika-agent/src/agent_loop/mod.rs` ; `auto_pull.rs` ; la garde de mika#2772 ; l'allowlist `ACTOR_READING_PREDICATES_ALLOWED` de mika#2323, qui reste `&[]` et dont le test reste vert **sans modification** ; les détecteurs D1/D2 du vol 2, verts sans entrée neuve |

**Le défaut, en une phrase.** Le consentement est lu **à l'entrée** de la file — le webhook
`labeled ready` crée la tâche parente — et n'est **jamais relu à la sortie**, au moment où
le wrapper différé promu part en tour. Le 2026-11-01 sur #3002, il s'était écoulé 37 min
entre le `unlabeled ready` humain (14:08:43Z) et le départ du tour (14:46:01Z). C'est le
troisième visage de l'invariant que ce ticket tient : le vol 1 a fermé la **capacité
d'écriture**, le vol 2 a attesté l'**absence de route d'écriture**, cette tranche ferme la
**relecture à la sortie**.

---

## 2. Le site, et pourquoi la garde est troisième

`crates/mika-agent/src/task_engine/dispatcher.rs`, `dispatch_resume_agent` (l. 1334). La
branche `is_deferred_dispatch` (l. 1338-1356) construit
`SilentTrigger::DeferredDispatch { task_id, parent_task_id, action_config }` **sans lire le
ticket** — c'est le défaut, et il est localisé là.

La zone d'insertion est l. 1403-1417, qui porte déjà deux gardes dans cet ordre :

```rust
// mika#2671 — un callback de build dont la tête a été remplacée ne paie pas de tour LLM.
if is_callback && self.skip_superseded_build_callback(task).await { return Ok(()); }
// mika#2944 — une ligne retenue pour crédit épuisé paie une sonde d'un message.
if is_callback && self.hold_callback_on_exhausted_credit(task).await { return Ok(()); }
```

puis, l. 1419-1434, le verrou d'agent (`lock.try_lock()`,
`record_verdict_delivery_deferral`, `DispatchError::AgentBusy`).

**Avant le verrou**, pour la raison que mika#2671 écrit à sa source : *un tour qu'on ne
lance pas n'a ni à attendre le verrou ni à compter un report `AgentBusy`*. Un report
`AgentBusy` sur un dispatch qu'on va refuser de toute façon est un faux positif dans le
compte des contentions.

**Troisième, et pas première**, parce que l'ordre existant est déjà porteur : mika#2671
est un test local sur le registre, mika#2944 un test local sur le crédit — aucun des deux
ne sort du processus. Notre garde fait **une lecture réseau** (jusqu'à
`MAX_TIMELINE_PAGES = 20` appels `gh api`). La règle maison — le test gratuit d'abord, la
vérification coûteuse ensuite — la place en troisième. Le coût n'est de toute façon payé
que sur la population `is_deferred_dispatch`, bien plus étroite que `is_callback`.

**Le prédicat de garde est `is_deferred_dispatch`, pas `is_callback`.** C'est la
contrepartie structurelle d'AC-T3 : un callback de build, de revue ou de verdict n'entre
jamais dans la fonction, donc son comportement est inchangé par construction et pas
seulement par test.

---

## 3. Le discriminant de parente, et son site de définition unique

La garde doit répondre à *« cette parente vient-elle d'un `ready` humain ? »*. Le
**producteur unique en production** du label est
`crates/mika-agent/src/server/ready_label_handler.rs:1439` :

```rust
let task_label = format!("ready-label: {}#{}", location.owner_repo(), location.number);
```

Toutes les autres occurrences de `ready-label: …` dans le dépôt
(`live_pilot.rs:482`, `auto_pull.rs:7883`/`11055`, `engine.rs` ×6, `dispatcher.rs:8553`/`9377`,
`executor.rs:12090`, `db/tests/mod.rs`) sont des **fixtures de test**. Et **aucun prédicat
`starts_with("ready-label")` n'existe aujourd'hui** : le recensement du 2026-11-03 n'en
trouve nulle part, ni en Rust ni en SQL (`grep -rn "LIKE 'ready-label"` rend zéro ligne).

Donc un site neuf est dû, et il est **unique** (doctrine mika#2158, classe
`grooming_marker`) — dans `ready_label.rs`, le module qui porte déjà l'invariant :

```rust
/// Préfixe du label des tâches parentes créées par le feeder `ready-label`.
/// SITE DE DÉFINITION UNIQUE : le producteur (`server/ready_label_handler.rs`)
/// compose par cette constante, le prédicat de la garde mika#2995 lit par elle.
pub(crate) const READY_LABEL_TASK_PREFIX: &str = "ready-label:";

/// `"ready-label: senara-solutions/mika#3002"` → `("senara-solutions/mika", 3002)`.
pub(crate) fn parse_ready_label_task_label(label: &str) -> Option<(String, u64)>;
```

**Pourquoi lire les coordonnées du LABEL et non de `reference_url`.** La traversée
`parent_issue_coordinates` (`dispatcher.rs:5506`) existe et fonctionne, mais elle passe par
`parse_repo_issue_from_url` (l. 5534), qui rend le **repo nu** (`"mika"`), alors que
`read_ready_label_timeline` interpole `repos/{repo}/issues/{issue}/timeline` et exige
`owner/repo`. Re-préfixer `senara-solutions/` serait un littéral d'organisation codé en
dur dans le dispatcher. Le label, lui, porte déjà `location.owner_repo()` — l'information
exacte, à sa source, sans devinette. Le site unique de parsing vit donc à côté du site
unique de composition, dans le même module.

---

## 4. La décision, et la fonction qui la porte

Miroir exact du gabarit `skip_superseded_build_callback` (l. 1253-1332, 80 lignes) : un
enum de verdict, un `match` qui nomme chaque issue, un `info!` à événement nommé, une ligne
d'audit, une écriture terminale, un `bool`.

```rust
enum WithdrawnConsentVerdict {
    /// AC-T3 : parente absente, ou label hors préfixe (groom, revue, dispatch manuel).
    NotAReadyLabelParent,
    /// Dernier événement `labeled`, ou `unlabeled` d'une identité machine (AC-T2).
    ConsentHeld,
    /// Dernier événement `unlabeled` d'un acteur non machine.
    ConsentWithdrawn { parked_by: Option<String>, parked_at: String },
    /// D4 — la timeline ne se lit pas (API, parse, troncature).
    TimelineUnreadable(String),
    /// D4 — aucun jeton GitHub résolu.
    NoToken,
}
```

Table de décision :

| verdict | dispatche ? | terminal wrapper | terminal parente | ligne d'audit | retour |
|---|---|---|---|---|---|
| `NotAReadyLabelParent` | **oui, inchangé** | non | non | non | `false` |
| `ConsentHeld` | **oui** | non | non | non | `false` |
| `ConsentWithdrawn` | non | `expired` + motif | `failed` + motif | oui | `true` |
| `TimelineUnreadable` | non | **non** | **non** | non | `true` |
| `NoToken` | non | **non** | **non** | non | `true` |

### 4.1 Le terminal du wrapper

`record_wrapper_noop(&task.id, READY_CONSENT_WITHDRAWN_REASON)` (l. 3815), qui délègue à
`db.mark_deferred_wrapper_noop` (`db/tasks.rs:1928`) :

```sql
UPDATE tasks SET status='expired', result=?2, completed_at=?3, updated_at=?3
 WHERE id=?1 AND label=?4 AND status IN ('completed','delivered')
```

Au point de garde le wrapper est `completed` (il vient d'être promu), donc l'UPDATE
s'applique. `expired` est le mot de mika#2169 pour *le tour a eu lieu et n'a rien
dispatché* — ici il n'a même pas eu lieu, ce qui est strictement moins que cela, et aucun
quatrième statut n'est inventé pour la nuance. `Ok(false)` veut dire « quelqu'un d'autre
l'a déjà rendu terminal » et ne mérite qu'un `debug!`, comme le helper le fait déjà.

### 4.2 Le terminal de la parente — `update_task_failed`, et pourquoi

**Décision : `db.update_task_failed(parent_id, READY_CONSENT_WITHDRAWN_REASON)`.** Trois
raisons, dont une est décisive.

1. **C'est le précédent exact du même sous-système, sur la même population.**
   `reap_orphaned_pending_issue_tasks` (`engine.rs:1914`) traite déjà le cas « parente
   `ready-label` dont le wrapper ne reviendra pas », et son doc-comment écrit l'échelon 3 :
   *« orphaned with the budget spent, or repair refused -> … transition the parent to
   `failed`. That drops it out of `idx_tasks_manual_active_ref_url` (the index excludes
   `failed`), and the next `ready` sweep creates a fresh task for the issue. »* Et
   `try_terminate_parent_on_dispatch_halt` (`dispatcher.rs:4713`) justifie la primitive à
   sa source sous le titre *« Pourquoi `update_task_failed` »* : **un UPDATE gardé sur
   l'état terminal, donc inoffensif en course avec le faucheur ou avec un geste
   opérateur**.
2. **C'est ce qui rend un consentement neuf effectif.** L'index unique partiel
   `idx_tasks_manual_active_ref_url` exclut les statuts terminaux. Laisser la parente
   `pending` garderait le créneau de l'issue occupé : un `labeled ready` humain reposé
   ensuite échouerait à créer sa tâche (collision commentée à
   `ready_label_handler.rs:1493`). **Le terminal n'est donc pas un verrou sur le ticket, il
   est la condition pour que le prochain « oui » humain soit pris.** C'est la raison
   décisive, et c'est aussi ce qui rend la garde non destructive : elle ne retire rien, elle
   libère.
3. **Rien de moins ne marche.** `cancel_task_superseded` et
   `terminal_mark_tracking_row_upstream_closed` portent une raison, mais sont gardés sur la
   forme fantôme (`status IN ('blocked','in_progress')`) ; la parente est `pending`, donc
   l'UPDATE ne toucherait aucune ligne. `cancel_task` toucherait bien la ligne mais **ne
   porte aucun motif** — et le motif est précisément ce qu'AC-T4 exige. Aucun helper
   « expirer une tâche avec une raison » n'existe (`mark_tasks_expired` est un balayage par
   échéance). Écrire un sixième verbe terminal pour nuancer `failed` en « le travail n'était
   pas dû » coûterait une migration de vocabulaire que ce vol n'a pas à ouvrir.

**Le coût, nommé.** `failed` affirme une panne là où il y a un retrait de consentement. Il
est borné par le motif, qui est lu partout où ce statut est agrégé :
`ready_consent_withdrawn` ne recouvre ni `phantom_aged_out`, ni `callback_no_pr_*`, ni
`dispatch_halt_*`, ni `stuck_pending_no_deferred_wrapper` — deux populations sous un même
nom cessent d'être soustractibles, et c'est ce que mika#2205 ferme.

### 4.3 La ligne d'audit

Gabarit `try_terminate_parent_on_dispatch_halt` (l. 4745-4756), verbatim :

```rust
db.log_audit_event(
    &format!("system-{}", parent.agent_id),   // session système de l'agent
    READY_CONSENT_RECHECK_TOOL_NAME,          // outil
    &parent_id,                               // cible
    Some("pending"), Some("failed"),          // de → vers
    Some(READY_CONSENT_WITHDRAWN_REASON),     // détail
    Some(&trace_id),
).await
```

Best-effort, `warn!` sur `Err` — la ligne d'audit ne doit pas faire échouer un refus déjà
écrit.

### 4.4 Les noms (AC-T4)

Quatre noms, quatre sites de définition uniques, **aucun ne contient `noop` nu** — déjà
pris par `deferred_dispatch_noop_completion` (mika#1124) et par le vocabulaire de
mika#2169 :

| nom | site | rôle |
|---|---|---|
| `READY_LABEL_TASK_PREFIX = "ready-label:"` | `ready_label.rs` | discriminant de parente (§ 3) |
| `READY_CONSENT_WITHDRAWN_REASON = "ready_consent_withdrawn"` | `ready_label.rs` | motif terminal, wrapper **et** parente |
| `READY_CONSENT_RECHECK_TOOL_NAME = "ready_label_consent_recheck"` | `ready_label.rs` | outil de la ligne d'audit, frère de `READY_LABEL_PARK_TOOL_NAME = "ready_label_park"` (l. 130) |
| `deferred_dispatch_refused_consent_withdrawn` | `dispatcher.rs` | événement `info!` du refus |

Le motif est **le même littéral** sur le wrapper et sur la parente, délibérément : c'est
un seul fait, et deux motifs pour un fait sont deux populations qu'on ne peut plus
rapprocher.

---

## 5. La lecture, et le joint de test

Tout ce que la garde lit existe déjà dans `ready_label.rs` :

- `is_parked(&[ReadyLabelEvent], &MachineIdentities) -> bool` (l. 227) — le prédicat de
  parc de mika#2315 D2, pur, et **c'est lui qui porte AC-T2** : le dernier événement d'un
  `remove` puis `add` machine est un `labeled`, donc `is_parked` rend `false` par sa
  première branche, et même un `unlabeled` machine en dernier est écarté par
  `!machine.is_machine(last.actor)`. La propriété est déjà épinglée par
  `last_event_unlabeled_by_the_machine_is_not_parked`.
- `read_ready_label_timeline_with<F: TimelinePageFetcher + ?Sized>(fetcher, repo, issue)`
  (l. 423) — **le joint d'injection, déjà en place**, et `GhTimelineFetcher::new(token)`
  (l. 366) pour la production.
- `machine_identities(token) -> &'static MachineIdentities` (l. 453) et sa règle **D3** :
  un jeu d'identités machine vide ⇒ on refuse d'agir.

Un seul composeur est dû, pour que la garde ne recopie pas la séquence et pour qu'elle
récupère `parked_by`/`parked_at` sans relire :

```rust
pub(crate) async fn park_verdict_for_issue_with<F: TimelinePageFetcher + ?Sized>(
    fetcher: &F, repo: &str, issue: u64, machine: &MachineIdentities,
) -> Result<Option<(Option<String>, String)>, TimelineReadError>;
// Ok(None) = non parqué ; Ok(Some((parked_by, parked_at))) = parqué.
```

**Côté dispatcher, le joint est une extraction de paramètre**, motif du vol 2 (§ 5 de son
plan) : la garde testable est

```rust
async fn skip_deferred_dispatch_on_withdrawn_consent_with(
    &self, task: &Task, fetcher: &dyn TimelinePageFetcher, machine: &MachineIdentities,
) -> bool
```

et le site de production résout `fetcher` et `machine` puis l'appelle. **`machine` passe
par paramètre et non par appel interne**, parce que `machine_identities` mémoïse dans un
`OnceLock` **à l'échelle du processus** : un test qui l'alimenterait figerait les identités
pour tous les autres tests du binaire. Le `OnceLock` reste donc hors du périmètre testé,
exactement comme `is_parked` l'est déjà pour `auto_pull`.

**Le jeton.** `dispatch_resume_agent` n'en résout aucun aujourd'hui (le site l. 1190
appartient à une autre fonction). Le site de production appelle donc
`self.settings.resolve_github_token(self.github_app.as_deref()).await`, le convertisseur
canonique de l'ADR-008, comme le fait l. 1190. **Pas** `resolve_periodic_scan_token`
(l. 431) : son paramètre `scan: PeriodicScan` est un enum **fermé aux scans périodiques**,
et cette garde n'en est pas un — y ajouter une variante donnerait au mot « scan » deux
populations sous un nom (classe mika#2205). `None` ⇒ verdict `NoToken`, traité par D4.

---

## 6. D4 — la timeline illisible, et la borne du report

**Décision : on ne dispatche pas, et on n'écrit aucun terminal.** C'est la règle D4 de
`ready_label.rs` appliquée telle quelle, et c'est le défaut que le ticket propose.

**Pourquoi pas le fail-open de mika#2671.** Le gabarit rend `false` sur
`LedgerUnreadable`, et c'est juste chez lui : son faux négatif coûte un tour de build en
trop. Ici le fail-open rouvrirait **exactement** le défaut mesuré — une lecture qui échoue
redeviendrait un tour non consenti. L'asymétrie est celle que l'en-tête de `ready_label.rs`
écrit déjà pour D4 : **un faux négatif coûte un tick de 10 minutes ; un faux positif coûte
~600 k tokens Opus et casse un STOP opérateur.** Les deux plateaux ne pèsent pas pareil,
donc la garde est fail-closed-to-no-action : elle ne lance rien, et elle n'écrit rien non
plus — refuser et graver un terminal sur une lecture ratée serait l'autre faux positif.

**Comment ce report reste borné — entièrement par des mécanismes existants et testés.**

1. **Le report est une relecture, pas une attente.** La garde rend `true` sans marquer le
   wrapper : il reste `completed` **non délivré**, donc la boucle de délivrance le reprend
   au tick suivant (10 min) et **relit la timeline**. Une panne d'API transitoire se résout
   d'elle-même au premier tick où la lecture passe, et la décision est alors écrite.
2. **La fenêtre de relecture est bornée par `PROMOTED_WRAPPER_LIVENESS_DEFAULT_SECS = 2700`**
   (45 min, `engine.rs:218`, surchargeable par `MIKA_PROMOTED_WRAPPER_LIVENESS_SECS`). Au
   delà, le wrapper n'est plus « live » : `find_orphaned_pending_issue_tasks` classe la
   parente orpheline et `reap_orphaned_pending_issue_tasks` prend la main. Soit ~4
   relectures.
3. **Le faucheur a son propre budget fini** : `MAX_STUCK_REARMS = 2`
   (`skills/executor.rs:5529`). Il ré-arme au plus deux fois — et chaque ré-armement
   repasse par notre garde, donc par une relecture de plus — puis, budget épuisé, met la
   parente en `failed`.
4. **La sortie du report est donc toujours un terminal, jamais un tour LLM.** Le pire cas
   est une timeline définitivement illisible : ≈ 45 min de relectures, ≤ 2 ré-armements, et
   la parente finit terminale par le faucheur. À aucun moment un tour ne part sur un
   consentement non relu — ce qui est l'invariant que la tranche défend.

**Le coût, nommé.** Pendant la fenêtre de report, le wrapper est `completed` non délivré
depuis plus de 10 min, donc l'anomalie `stuck_callback` de `get_task_task_health_summary`
apparaît dans le bloc `<task-health>` du heartbeat. C'est de la **visibilité**, pas une
panne : un consentement qu'on n'arrive pas à relire *est* une anomalie, et 45 min la
bornent. Le fait est préféré au silence.

**La sonde du report.** Une ligne `info!` à événement nommé, distincte du refus, sur
**chaque** report — `deferred_dispatch_consent_recheck_deferred`, avec `task_id`,
`parent_task_id`, `repo`, `issue` et `cause` ∈ {`api`, `parse`, `truncated`, `no_token`}.
Sans elle, « la lecture échoue en boucle » et « aucun wrapper `ready-label` n'est passé par
la garde » rendent zéro tous les deux (classe mika#2205). Son **absence** veut dire un
binaire antérieur à la tranche (classe mika#2340) : l'instrument n'est pas vivant avant
`make deploy`.

---

## 7. AC-T3 — l'inchangé, prouvé par l'absence de lecture

Pour une parente autre que `ready-label` — groom, revue, dispatch manuel, parente absente
— le verdict est `NotAReadyLabelParent` et la garde rend `false` **avant toute lecture
réseau**. Le test ne se contente pas de vérifier que le dispatch part : il compte les
appels au fetcher et exige **zéro**. C'est ce qui sépare « le comportement est le même » de
« le comportement est le même et ne coûte rien », et c'est la seule forme d'« octet pour
octet » qu'un test peut attester ici.

L'ordre des gardes dans la fonction le renforce : les deux gardes antérieures sont gatées
sur `is_callback`, la nôtre sur `is_deferred_dispatch`, qui est strictement plus étroit
(`is_callback && task.label == DEFERRED_DISPATCH_LABEL`, l. 1338). Un callback de build ne
peut donc pas atteindre la nouvelle branche, et la drift-guard de l. 8404 continue d'épingler
`DEFERRED_DISPATCH_LABEL`.

---

## 8. Les tests

Tous dans `dispatcher.rs`, près de ceux de mika#2671, et tous **dans le même appel** que la
fonction qu'ils exercent (pas de bout-en-bout, pas de réseau).

**Les fixtures de timeline sont promues, pas recopiées.** `ready_label.rs` porte déjà
`FixturePages` (l. 858), `labeled_json` (l. 885) et son frère `unlabeled_json` dans son
`mod tests`. Elles deviennent un `#[cfg(test)] pub(crate) mod timeline_fixtures` du même
module : un seul site de fixture de timeline pour les deux modules, ce qui est la même
discipline que le reste du plan et évite un second gabarit JSON qui dériverait.

| test | AC | fixture | assertion |
|---|---|---|---|
| `mika2995_un_ready_label_dont_le_consentement_est_retire_ne_lance_aucun_tour` | T1 positif | `labeled` humain puis `unlabeled` humain | `true` ; wrapper `expired` + motif ; parente `failed` + motif ; ligne d'audit sous l'outil nommé ; wrapper **jamais** `delivered` |
| `mika2995_la_meme_fixture_avec_un_labeled_en_dernier_lance_le_tour` | T1 négatif | même timeline + `labeled` humain final | `false` ; wrapper toujours `completed` ; parente toujours `pending` ; zéro ligne d'audit |
| `mika2995_un_remove_puis_add_machine_nest_pas_un_retrait` | T2 | `labeled` humain, `unlabeled` machine, `labeled` machine | `false` ; aucune écriture |
| `mika2995_une_parente_de_groom_est_inchangee` | T3 | parente hors préfixe | `false` ; **compteur d'appels du fetcher à zéro** |
| `mika2995_une_timeline_illisible_reporte_sans_terminal` | D4 | fetcher rendant `Err` | `true` ; wrapper toujours `completed` ; parente toujours `pending` ; aucun terminal, aucune ligne d'audit |

**Le contrôle négatif de T1 est sur la même fixture**, à un événement près, comme le ticket
l'exige : c'est ce qui sépare « la garde lit le dernier événement » de « la garde refuse
tous les `ready-label` ».

**Le détecteur d'AC-T4**, `mika2995_le_motif_et_levenement_ont_un_site_de_definition_unique` :
un scan de source sur `src/**/*.rs` via `source_scan::production_files()` +
`strip_comment_lines`, qui exige **un seul** littéral `"ready_consent_withdrawn"` et **un
seul** `"ready_label_consent_recheck"` hors de leur ligne de définition, et aucune
occurrence de `noop` dans les deux noms. Avec son contrôle de bonne foi
(`…_rougit_sur_un_second_site`) et son `assert!(scanned > 0)`.

---

## 9. AC-T5 — la vérification qu'aucun label n'est écrit

Trois contrôles, dans cet ordre, avant d'ouvrir la PR :

```sh
git diff origin/main...HEAD -- crates/ | grep -nE '^\+.*(--add-label|--remove-label|add_label_to_issue|remove_label_from_issue|apply_ready)'
```

doit rendre **zéro ligne** (exit 1). Puis les deux détecteurs du vol 2 —
`mika2995_aucune_surface_doutil_natteint_lapplicateur_de_ready` et
`mika2995_une_ecriture_de_ready_en_forme_argv_est_declaree` — doivent rester verts **sans
entrée neuve** dans leurs allowlists. Puis
`mika2323_no_gate_predicate_reads_the_actor` doit rester vert avec
`ACTOR_READING_PREDICATES_ALLOWED` toujours `&[]` : la garde lit un acteur, mais elle n'est
pas un **prédicat de porte** au sens de mika#2323 — elle ne décide pas d'un consentement,
elle relit celui qu'un humain a déjà donné ou retiré, par le prédicat `is_parked` que
mika#2315 a déjà doté de cette lecture et qu'`auto_pull` consomme depuis lors. Si ce test
rougit, **le vol s'arrête** : ce serait une décision de doctrine, pas un ajustement.

La sonde AC5 est donc intacte, et son relevé du 2026-12-01 n'est pas déplacé.

---

## 10. Ordre de commit

1. `ready_label.rs` — les trois constantes, `parse_ready_label_task_label`,
   `park_verdict_for_issue_with`, et la promotion des fixtures de timeline. Rien ne les lit
   encore ; le module compile seul.
2. `server/ready_label_handler.rs` — le producteur du label compose par
   `READY_LABEL_TASK_PREFIX`. Aucun changement de comportement : le littéral produit est
   identique, octet pour octet.
3. `dispatcher.rs` — `skip_deferred_dispatch_on_withdrawn_consent_with`, son enum de
   verdict, le site de production, et le branchement l. 1417. **C'est le seul commit qui
   change un comportement.**
4. `dispatcher.rs` — les cinq tests comportementaux.
5. `dispatcher.rs` — le détecteur AC-T4 et son contrôle de bonne foi.

Puis la chaîne complète : `cargo test`, `cargo clippy`, `cargo fmt --check`, les lints du
dépôt, `scripts/verify-pipeline.sh origin/main`, et les trois contrôles du § 9.

---

## Definition of Done

- La garde `skip_deferred_dispatch_on_withdrawn_consent_with` est en place dans
  `dispatch_resume_agent`, **après** la garde mika#2944 et **avant** le verrou d'agent,
  gatée sur `is_deferred_dispatch`.
- Un wrapper différé dont la parente porte le préfixe `ready-label:` et dont le dernier
  événement `ready` de la timeline est un `unlabeled` d'acteur non machine : aucun tour
  LLM, aucun dispatch, wrapper `expired` + `ready_consent_withdrawn`, parente `failed` +
  `ready_consent_withdrawn`, une ligne d'audit sous `ready_label_consent_recheck`, une
  ligne `info!` `deferred_dispatch_refused_consent_withdrawn`.
- Toute autre parente : `false` rendu sans aucune lecture réseau, comportement inchangé.
- Timeline illisible ou jeton absent : aucun dispatch et **aucun terminal**, une ligne
  `deferred_dispatch_consent_recheck_deferred` nommant la cause ; le report est borné par
  `PROMOTED_WRAPPER_LIVENESS_DEFAULT_SECS` puis par `MAX_STUCK_REARMS`, et sa sortie est un
  terminal, jamais un tour.
- Les quatre noms neufs ont chacun un site de définition unique, aucun ne contient `noop`
  nu, et un scan de source le tient.
- Les cinq tests comportementaux et le détecteur sont verts.
- Le diff n'ajoute **aucune** écriture de label (`grep` du § 9, exit 1) ; les deux
  détecteurs du vol 2 et `mika2323_no_gate_predicate_reads_the_actor` restent verts sans
  modification de leurs allowlists.
- `cargo test`, `cargo clippy`, `cargo fmt --check`, les lints du dépôt et
  `scripts/verify-pipeline.sh origin/main` passent.
- Le corps de PR porte `Refs mika#2995 (tranche consentement relu)` et **jamais** `Closes`.
  Il dit que la sonde AC5 est intacte et que le merge passe par le mot de Vincent.

---

## Acceptance criteria

- **AC-T1.** Un test dans le même appel, avec contrôle négatif. Un wrapper différé dont la parente est `ready-label`, sur une timeline dont le dernier `ready` est un `unlabeled` humain, ne lance aucun tour : 0 appel au constructeur de tour, et terminal du wrapper avec le motif nommé. Sur la même fixture avec `labeled` en dernier, le tour part.
- **AC-T2.** Le `remove` puis `add` machine d'`auto_pull` Phase 2 n'est pas lu comme un retrait : c'est la propriété déjà épinglée par `is_parked`. Un cas le montre au site neuf.
- **AC-T3.** Pour une parente autre que `ready-label`, le comportement est inchangé, octet pour octet. Un test le montre.
- **AC-T4.** Le motif du terminal et l'événement d'audit ont chacun un site de définition unique, et leur nom ne contient pas `noop` nu, déjà pris par mika#1124 et mika#2169.
- **AC-T5.** La sonde AC5 n'est pas touchée, puisque la tranche n'écrit aucun label. Le plan le vérifie par un `grep` sur le diff.

---

## Fire-Disposition

Ce plan livre **un détecteur** : le scan de source d'AC-T4
(`mika2995_le_motif_et_levenement_ont_un_site_de_definition_unique`, § 8), dont le chemin
de succès est « aucun second site de définition trouvé ».

**Option (a) — exception nommée en allowlist, livrée VIDE.**

```rust
const CONSENT_RECHECK_NAME_EXCEPTIONS: &[(&str, &str, &str)] = &[];
// (fichier, ticket, donnée précise)
```

- **Elle est livrée vide parce que la population est vide par construction** : les quatre
  noms sont créés par ce vol, et le recensement du 2026-11-03 confirme qu'aucun n'existe
  aujourd'hui — `grep -rn "ready_consent_withdrawn\|ready_label_consent_recheck" crates/`
  rend zéro ligne, et aucun prédicat `starts_with("ready-label")` n'existe nulle part
  (§ 3). Il n'y a donc **aucune violation préexistante à excepter**.
- Une allowlist vide peut mentir de trois façons, et les trois sont tenues :
  1. **Elle pourrait se remplir en silence** ⇒ assertion sœur d'auto-vacuité,
     `mika2995_lallowlist_des_noms_de_relecture_est_vide`, qui rougit à la première entrée.
     Quand le détecteur tire, on **route le second site par la constante** ; on n'ajoute pas
     une ligne (doctrine mika#2201).
  2. **Le scan pourrait n'avoir rien scanné** ⇒ `assert!(scanned > 0, …)` sur le nombre de
     fichiers de la population, l'idiome déjà en place dans `ready_label.rs`.
  3. **Il pourrait avoir cessé de reconnaître sa forme** ⇒ contrôle de bonne foi
     `mika2995_le_scan_des_noms_rougit_sur_un_second_site`, qui doit rougir sur un second
     littéral en code et rester **muet** sur les quatre formes inertes : doc-comment,
     commentaire de bloc, `use`, et la ligne de définition elle-même.
- **Assertion auto-nettoyante.** Le scan exige que chacun des quatre noms **existe encore
  exactement une fois**. Si une constante est supprimée ou renommée, le test rougit en
  nommant laquelle : le geste qui rendrait l'exception inutile est celui qui la fait
  expirer, ce qui empêche cette allowlist de devenir celle que mika#2309 condamne.
- **Pourquoi pas (b) livrer désarmé.** Un `#[ignore]` sur un scan dont la population est
  vide le jour de la livraison n'a rien à reporter : il atterrit armé, et il est vert au
  premier run.
- **Pourquoi pas (c) halte-et-remontée.** Rien n'est à cadrer : le recensement a déjà
  répondu, et il a répondu « zéro ».

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/task_engine/dispatcher.rs` — `skip_deferred_dispatch_on_withdrawn_consent_with`, enum de verdict, `match` des cinq issues, doc-comment | 110 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` — site de production (jeton, identités machine, `GhTimelineFetcher`) + branchement l. 1417 | 35 |
| `crates/mika-agent/src/ready_label.rs` — `READY_LABEL_TASK_PREFIX`, `READY_CONSENT_WITHDRAWN_REASON`, `READY_CONSENT_RECHECK_TOOL_NAME`, `parse_ready_label_task_label`, `park_verdict_for_issue_with` | 60 |
| `crates/mika-agent/src/ready_label.rs` — promotion des fixtures de timeline en `#[cfg(test)] pub(crate)` (déplacement ; delta de visibilité et de `mod`) | 10 |
| `crates/mika-agent/src/server/ready_label_handler.rs` — le producteur du label compose par la constante de préfixe | 5 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` — les cinq tests comportementaux (helper de montage + AC-T1 ×2, AC-T2, AC-T3, report D4) | 120 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` — détecteur AC-T4, allowlist vide, auto-vacuité, contrôle de bonne foi | 45 |

Total estimé : 385 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Cette mesure ne reprend pas celle du ticket.** Le corps estimait ~220 à 260 lignes à
partir du gabarit de 80 lignes de mika#2671. Trois surfaces lui manquaient, et elles sont
mesurées ci-dessus : la **traversée de parente et le parsing des coordonnées** (le ticket
supposait `parent_issue_coordinates` réutilisable telle quelle, mais
`parse_repo_issue_from_url` rend le repo nu quand la lecture de timeline exige
`owner/repo` — § 3) ; le **site de production** du jeton et des identités machine, que
`dispatch_resume_agent` ne résout pas aujourd'hui (§ 5) ; et le **détecteur d'AC-T4**, qui
est un livrable à part entière avec sa Fire-Disposition. 385 reste sous 429, et très loin
des 1000 lignes qui imposeraient un découpage en phases.

**Pourquoi un seul vol tient, avec 44 lignes de marge (11 %).** Quatre raisons rendent
l'estimation fiable plutôt qu'optimiste :

1. **La garde est un gabarit recopié, pas une invention.**
   `skip_superseded_build_callback` fait 80 lignes pour la même forme — enum de verdict,
   `match`, `info!` nommé, `log_audit_event`, écriture terminale, `bool`. Les 30 lignes de
   plus sont la traversée de parente et la lecture, toutes deux des appels à des fonctions
   existantes.
2. **Rien n'est à construire côté lecture.** `is_parked`, `read_ready_label_timeline_with`,
   le trait `TimelinePageFetcher`, `GhTimelineFetcher` et `machine_identities` sont tous
   déjà publics dans la crate et déjà testés. `park_verdict_for_issue_with` ne fait que les
   composer.
3. **Aucune écriture DB neuve.** Les deux primitives terminales existent et sont gardées
   (`mark_deferred_wrapper_noop`, `update_task_failed`), la ligne d'audit a son gabarit
   verbatim (§ 4.3). **Zéro migration, zéro colonne, zéro variable d'environnement
   neuve** — donc aucune des surfaces qui font déborder un vol.
4. **Les fixtures de test sont promues et non réécrites.** `FixturePages` et ses helpers
   JSON existent ; le delta est une visibilité, pas un gabarit neuf. C'est la ligne à 10
   lignes du tableau, et c'est ce qui ramène les tests de ~155 à 120.

Le risque résiduel est le doc-comment : la maison en écrit beaucoup, et c'est ce qui fait
dériver une estimation. Les 110 lignes de la garde l'incluent déjà au taux du gabarit
(≈ 35 % de doc dans `skip_superseded_build_callback`). Si le vol approchait la borne, la
ligne qui cède est le **doc-comment du § 6** (la borne du report), qui vit déjà dans ce
plan et peut y être citée au lieu d'être recopiée.
