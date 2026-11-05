# mika#3044 — une parente `self_dev` fauchée par le reaper de groom redevient dispatchable quand un plan a convergé

| | |
|---|---|
| ticket | senara-solutions/mika#3044 |
| titre | `fix(dispatch): une parente self_dev fauchée par le reaper de groom reste terminale, et le dev-pilot d'un plan convergé est refusé` |
| labels | `bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate` |
| dépôt | mika (un seul) |
| source | parking #2983, issuecomment-6657019407 |

---

## 0. Le vol, en une phrase

Quand la garde d'état de `validate_dispatch_readiness` s'apprête à refuser une
parente `self_dev` en `failed`, **un prédicat unique** demande si cet état
terminal est exactement un reap de groom que la suite des événements a démenti —
dernière transition = `groom_parent_orphaned`, puis un enfant de groom livré
après lui avec `PLAN_GROOMED` — et, si oui, la parente est réhabilitée
`failed` → `in_progress` **après que toutes les autres gardes ont passé**, puis
dispatchée. Dans tous les autres cas, le refus `task_not_dispatchable`
d'aujourd'hui sort mot pour mot.

Ce n'est pas « rouvrir les causes de reap à tort » : mika#2986 et mika#3042 les
retirent et ce vol ne les touche pas. C'est **borner le coût d'une erreur
future** : un reap à tort ne doit plus suffire à perdre un vol dont le plan a
convergé.

---

## 1. L'état mesuré

### 1.1 Le défaut

Trois vols perdus de la même façon depuis le 2026-10-20 (base `tasks` +
`audit_events`, lecture seule) :

| parente | ticket | reap | groom convergé | refus |
|---|---|---|---|---|
| `1ac4ef2e` | #2885 | 2026-10-23T08:48:15Z | `5f652d04`, `PLAN_GROOMED` | `task_not_dispatchable` |
| `97dfb97d` | #2924 | 2026-10-27T10:09:20Z | `3fe13f49`, `PLAN_GROOMED` | `task_not_dispatchable` |
| `0fb0b070` | #3003 | 2026-11-01T14:54:21Z | `4d0fa651`, `PLAN_GROOMED` | `task_not_dispatchable` |

La séquence est toujours la même : (1) le reaper de groom passe la parente à
`failed` ; (2) un groom rend `PLAN_GROOMED` ; (3) le dispatch du dev-pilot sur
cette même parente est refusé en `task_not_dispatchable`.

Deux faits de la Mesure qui décident de la forme du correctif :

- **Le refus réécrit `tasks.result`** (par `record_dispatch_rejection`,
  `executor.rs`). Le motif du reaper n'y reste donc pas lisible : **seule la
  piste d'audit le garde**. C'est pourquoi la condition (b) se lit dans
  `audit_events` et non dans `tasks.result`.
- **Une parente fauchée qui porte une PR est déjà rattrapée** par
  `try_promote_parent_on_retry_success` (`failed` → `completed`,
  `retry_success` ; exemples `44e1a7d2`, `c8b593c2`, `fc36bc5b`). Un plan
  convergé n'a pas de PR : il n'a **aucun** chemin de retour. Ce vol lui en
  donne un, symétrique de celui-là.

### 1.2 Le site, vérifié ligne par ligne sur `ccc127046`

| élément | lieu | lecture |
|---|---|---|
| garde d'état | `crates/mika-agent/src/skills/executor.rs` l. 4055-4071, dans `validate_dispatch_readiness` (l. 3848) | `if !matches!(task.status.as_str(), "pending" \| "in_progress")` ⇒ `task_not_dispatchable`, écrit sur la parente par `record_dispatch_rejection` |
| la ligne du refus est la **7ᵉ** garde | `executor.rs` l. 3858 → 5190 | ordre : webhook non autorisé, hold-review, intention de grooming, dépôt non dispatchable, frein du lanceur, re-lecture de la tâche (l. 4034), **état**, double-dispatch, lignage mika#2649, frein d'ancrage mika#2786, ESCALATE terminal mika#2545, phase de milestone, `blockedBy`, puis la claim mika#1948 |
| dernière garde | `executor.rs` l. 5054-5190 | claim atomique de la place d'exécution, `try_acquire_dispatch_slot`, TTL borné — *« placée EN DERNIER pour qu'aucune étape faillible ne la suive »* |
| retour de la fonction | `executor.rs` l. 5192 | `Ok(task.status.clone())` ; l. 6235 le site outil en fait `wi_status` et n'appelle `mark_parent_dispatched` que si c'est `"pending"` |
| réhabilitation existante | `crates/mika-agent/src/task_engine/dispatcher.rs` l. 4650, `try_promote_parent_on_retry_success` | garde `trigger_type == "manual" && status == "failed" && source == Some("self_dev")`, puis `promote_task_completed`, puis une ligne d'audit `task_engine_retry_promoter` |
| transition dédiée | `crates/mika-agent/src/db/tasks.rs` l. 844, `promote_task_completed` | le patron : `UPDATE … WHERE id = ?2 AND agent_id = ?3 AND status = 'failed'`, et le `rows > 0` rendu au lecteur |
| motif du reaper de groom | `crates/mika-agent/src/task_engine/engine.rs` l. 127-138, `REAPER_CLASS_DISPOSITIONS` | recensement **clos** de deux classes ; pour `groom` : raison `groom_callback_delivered_without_redispatch`, outil d'audit `groom_parent_orphaned` |
| écriture du reap | `engine.rs` l. 4687-4708 | `update_task_failed` puis `log_audit_event(system-<agent>, "groom_parent_orphaned", <id de parente nu>, Some("in_progress"), Some("failed"), Some(raison), …)` |
| le bord à exclure | `engine.rs` l. 4738-4749 | **même** nom d'outil, `before_value = Some("in_progress")`, `after_value = None`, raison `reaper_db_error:…` — un reap qui **n'a pas eu lieu** |
| lecteur unique du marqueur | `crates/mika-agent/src/task_state/tasks.rs` l. 275, `groom_result_convergence` | `PLAN_GROOMED` **en position de verdict** (ancré en début de ligne), enveloppe JSON refusée |

### 1.3 Ce que la piste d'audit dit, et ce qu'elle ne dit pas

La Mesure note que pour les trois parentes, **seules des lignes de jugement**
(`manager_flight_verdict`, `mpc_flight_judgment`) suivent le reap. AC1(b) demande
au plan de nommer ce qui compte comme transition. Le discriminant n'est pas une
convention à inventer : il est déjà dans le code.

- La DDL (`crates/mika-agent/src/db/migrations.rs` l. 851-867) déclare
  `before_value TEXT` et `after_value TEXT`, **tous deux nullables**, et
  n'indexe pas `target_key` (une lecture par parente chevauche
  `idx_audit_agent_created(agent_id, created_at DESC)`).
- `crates/mika-agent/src/flight_verdict_ledger.rs` l. 530-540 écrit ses deux
  outils de jugement (`manager_flight_verdict` l. 196, `mpc_flight_judgment`
  l. 227) avec **`before_value = None`** et `after_value = Some(valeur)`.
- `crate::db::latest_audit_event_for_target` (`db.rs` l. 2056) rend
  `after_value` comme l'`Option<String>` que la colonne est, et son doc-comment
  dit pourquoi : coercer NULL en `""` rendrait l'absence indistinguable d'une
  valeur vide (motif mika#2331).

**D1 — une transition d'état est une ligne qui porte un état avant ET un état
après :** `before_value IS NOT NULL AND after_value IS NOT NULL`. Une ligne de
jugement en porte un seul. Le bord `reaper_db_error` de `engine.rs` l. 4738
tombe du bon côté **par construction** (`after_value = None`), ce qui est exact :
ce n'est pas un reap. Le prédicat redouble malgré tout avec l'égalité
`after_value == "failed"` exigée par AC1(b) — un refus doublement fermé coûte
une comparaison de chaîne.

`latest_audit_event_for_target` **ne peut pas** répondre à (b) : elle filtre sur
un `tool_name` donné, donc elle sait répondre *« la dernière ligne de cet
outil »* et jamais *« la dernière transition, quel que soit l'outil »*. Les cinq
voisines (`count_recent_audit_events_for_target` l. 1894,
`count_recent_audit_events_by_value` l. 2017, `count_audit_events_by_tool_name`
l. 2124, `get_audit_event_rows_by_tool_name` l. 2162,
`list_audit_events_for_tool_since` l. 2224) sont toutes, elles aussi, bornées à
un outil. D'où la lecture neuve du § 2.3.

---

## 2. Le geste

### 2.1 D2 — un prédicat, un module, un site de lecture

AC3 exige *« un seul prédicat, lu depuis un seul site »*. Livrable :
`crates/mika-agent/src/parent_rehabilitation.rs`, frère de forme de
`flight_verdict_ledger.rs`, `review_anchor.rs` et `model_transmission.rs` — le
motif de la maison pour un prédicat à site unique.

```rust
/// Pourquoi une parente terminale n'est PAS réhabilitable. Nommé, parce
/// qu'AC3 demande un refus qui porte sa raison.
pub(crate) enum RehabRefusal {
    NotSelfDevFailedParent,      // (a)
    LastTransitionNotGroomReap,  // (b)
    NoGroomedChildSinceReap,     // (c)
    AuditTrailUnreadable,        // fail-closed de (b) ou (c)
}

pub(crate) enum RehabDecision {
    Eligible { reap_at: String, groom_child_id: String },
    Refused(RehabRefusal),
}

/// LE prédicat. Un seul appelant : `validate_dispatch_readiness`.
pub(crate) async fn decide(db: &AsyncDatabase, task: &Task) -> RehabDecision
```

Trois propriétés tenues par la forme, pas par la vigilance :

- **Les quatre conditions sont évaluées dans l'ordre (a) → (b) → (c)** et la
  première qui tombe nomme le refus. (d) n'est pas dans le prédicat : c'est le
  reste de `validate_dispatch_readiness`, voir § 2.4.
- **`decide` est `async` et fait ses deux lectures elle-même.** L'alternative —
  un prédicat pur à qui l'appelant passe les deux lectures — exigeait deux
  fonctions (la seconde dépend de l'instant du reap rendu par la première), donc
  deux sites de lecture dans `executor.rs`, donc exactement ce qu'AC3 refuse.
- **Fail-closed sur l'illisible**, aux deux lectures : une erreur de base rend
  `AuditTrailUnreadable`, jamais `Eligible`. *Un signal qu'on ne peut pas lire
  n'est jamais un terme satisfait* (mika#2277/#2279). C'est le contraire du frein
  d'ancrage mika#2786 (fail-open), et la différence est de conséquence : là, ne
  pas freiner laisse le vol partir ; ici, réhabiliter sur une lecture ratée
  dispatcherait sur une parente dont on ne sait rien.

Le nom d'outil du reaper devient une constante à site unique —
`GROOM_PARENT_ORPHANED_EVENT` dans `engine.rs`, lue par
`REAPER_CLASS_DISPOSITIONS` l. 136 **et** par le prédicat. Aujourd'hui c'est un
littéral dans le tableau ; deux littéraux identiques dans deux fichiers sont ce
qui dérive.

### 2.2 Ce que les requêtes existantes ne savent pas faire

Quatre lectures de preuve de groom existent déjà (`db/tasks.rs`) :
`has_completed_groom_for_issue` l. 4524, `last_pr_opened_for_issue` l. 4568,
`groom_callback_results_newest_first` l. 4681, `recent_groom_results_for_issue`
l. 4718. **Les quatre sont clés par `issue_url`**, via
`tasks child JOIN tasks parent ON child.parent_task_id = parent.id` et
`parent.reference_url IN (?2, ?3)`.

AC1(c) demande un enfant de groom **de cette parente**, livré **après le reap**.
Une requête par `issue_url` répondrait pour les enfants de *toutes* les parentes
du même ticket et ne porte aucune borne d'instant. Réutiliser
`recent_groom_results_for_issue` élargirait (c) en silence : c'est un refus
explicite, pas un oubli.

### 2.3 D3 — les deux lectures neuves, et l'absence de migration

**(b) la dernière transition, quel que soit l'outil** — `db.rs`, à côté de
`latest_audit_event_for_target` :

```sql
SELECT tool_name, before_value, after_value, created_at
FROM audit_events
WHERE agent_id = ?1 AND target_key = ?2
  AND before_value IS NOT NULL
  AND after_value IS NOT NULL
ORDER BY created_at DESC, id DESC
LIMIT 1
```

Le `ORDER BY` à deux termes est celui des voisines : `created_at` est à la
seconde, `id` tranche l'égalité. Le `LIMIT 1` fait de *« aucune transition ne la
suit »* une propriété de la **même** ligne que *« la dernière transition est le
reap »* — une seule lecture pour les deux moitiés de (b), donc aucune fenêtre
entre elles. L'appelant vérifie ensuite
`tool_name == GROOM_PARENT_ORPHANED_EVENT && after_value == "failed"`.

**(c) les enfants de groom de cette parente livrés après le reap** —
`db/tasks.rs`, même clé de jointure que `has_non_deferred_active_callback_child`
(l. 4428), couverte par `idx_tasks_parent(parent_task_id, agent_id)`
(`migrations.rs` l. 733) :

```sql
SELECT child.result FROM tasks child
WHERE child.agent_id = ?1
  AND child.parent_task_id = ?2
  AND child.trigger_type = 'callback'
  AND child.dispatch_class = 'groom'
  AND child.status IN ('completed', 'delivered')
  AND COALESCE(child.completed_at, child.updated_at) > ?3
  AND child.label NOT LIKE '%:deferred%'
ORDER BY child.created_at DESC, child.id DESC
LIMIT 10
```

Quatre décisions dedans, chacune reprise d'une voisine :

- **Le marqueur n'est JAMAIS filtré en SQL.** La requête rend des `result`
  bruts ; le verdict est rendu par `task_state::tasks::groom_result_convergence`,
  lecteur unique de `PLAN_GROOMED`, qui exige la position de verdict et refuse
  l'enveloppe JSON. *Une ligne écartée par le SQL est invisible au verdict*
  (doctrine mika#2590) : un `LIKE '%PLAN_GROOMED%'` ici accepterait le marqueur
  cité dans une prose et masquerait `MarkerOutOfPosition`.
- `status IN ('completed','delivered')` et `label NOT LIKE '%:deferred%'` :
  copiés de `has_completed_groom_for_issue` et de
  `recent_groom_results_for_issue`.
- `COALESCE(completed_at, updated_at)` : la colonne d'instant terminal que les
  deux voisines utilisent déjà. Comparaison lexicographique, correcte parce que
  `crate::timestamp` écrit un UTC à largeur fixe.
- **`>` strict.** Un groom livré à la seconde du reap ne le démente pas : rien ne
  dit lequel est venu d'abord, et dans le doute on garde le comportement
  d'aujourd'hui (le refus). Même inégalité stricte, même raison, que
  `bound_unbound_grace` (`task_state/tasks.rs` l. ~423).

`LIMIT 10` : la boucle ne produit pas dix groms par parente ; la borne existe
pour que la lecture reste bornée si elle le faisait. **Aucune DDL, aucune
migration, aucun index neuf — le schéma reste v56.** Trois enveloppes dans
`async_db.rs`, sur le patron de `latest_audit_event_for_target` (l. 838) et de
`recent_groom_results_for_issue` (l. 1809).

### 2.4 D4 — où l'écriture atterrit, et le prix nommé contre mika#1948

AC1(d) : *« la réhabilitation n'est écrite qu'après [les autres gardes] : si
l'une refuse, la parente reste `failed` »*. Deux instants, donc, et c'est
délibéré :

1. **Le prédicat est évalué dans la branche de la garde d'état** (l. 4055), parce
   que c'est le seul endroit où il peut **empêcher** le `return Err`. Une garde
   post-hoc observerait le refus sans l'éviter (doctrine mika#1646). `Eligible`
   ⇒ on ne retourne pas, on continue avec un jeton local
   `Option<RehabPending>` ; tout le reste de la fonction s'exécute inchangé.
2. **L'écriture est la dernière étape avant `Ok`**, après la claim mika#1948
   (l. 5054-5190). Toutes les gardes, claim comprise, sont « les autres gardes »
   d'AC1(d) : si l'une refuse, on sort par son chemin et la parente reste
   `failed` sans qu'une ligne de réhabilitation ait été écrite.

La transition, sur le patron de `promote_task_completed` (`db/tasks.rs` l. 844),
remet les trois conditions de (a) **dans le `WHERE`** :

```sql
UPDATE tasks SET status = 'in_progress',
       updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')
WHERE id = ?2 AND agent_id = ?3
  AND status = 'failed' AND trigger_type = 'manual' AND source = 'self_dev'
```

Ce n'est pas une redite du prédicat : c'est un **compare-and-swap**. Entre la
lecture de (a) et cette écriture, la fonction a fait plusieurs allers-retours
GitHub à 10 s de délai chacun ; un autre écrivain a pu bouger la ligne.
`Ok(false)` ⇒ on n'a pas réhabilité, donc on **refuse** (`parent_rehab_lost_race`),
on n'écrit pas de ligne de transition, et on ne dispatche pas. Ni `completed_at`
ni `fired_at` ne sont touchés : `completed_at` parle d'une fin, et `fired_at`
(mika#2335) a été estampillé au dispatch d'origine — cette parente était
`in_progress` avant le reap.

**Le prix, nommé.** L'invariant de la claim dit *« placée en dernier pour
qu'aucune étape faillible ne la suive — c'est pourquoi aucun chemin d'erreur n'a
à relâcher un bail »*. Deux étapes faillibles la suivent maintenant : la
transition et sa ligne d'audit. Un échec de la transition sort en `Err` avec le
bail pris, donc la classe `implement` est bloquée **pour un TTL** — l'état que le
commentaire de la claim décrit déjà pour un dispatcher qui meurt en pleine claim
(*« un TTL, pas pour toujours »*), et que l'enveloppe différée ré-entraîne. Ce
coût est borné, déjà atteignable, et préférable aux deux autres placements :
avant la claim, une parente promue puis refusée par la claim resterait
`in_progress` sans dispatch — et le reaper de groom la refaucherait, ce qui est
le ping-pong qu'AC1(d) interdit ; sans jeton, l'écriture serait avant toutes les
gardes. Le commentaire de la claim est **amendé dans ce vol** pour dire ces deux
étapes : un invariant qu'on contredit sans réécrire son texte est un invariant
que le prochain lecteur croira.

**Le retour devient `Ok("in_progress")`** après réhabilitation — l'état que la
ligne porte vraiment, et pas `task.status`, figé à `"failed"` par la re-lecture
de la l. 4034. C'est ce que lit `wi_status` (l. 6235) : `"in_progress"` n'appelle
pas `mark_parent_dispatched`, exactement comme pour une parente `in_progress`
ordinaire.

### 2.5 D5 — les deux lignes de la piste, et l'idempotence qui en découle

AC3 : *« chaque issue écrit une ligne d'audit relisible »*. Deux noms, un seul
site d'écriture (le même module que le prédicat, appelé depuis le même site) :

| issue | `tool_name` | `before_value` | `after_value` | `reasoning` |
|---|---|---|---|---|
| réhabilitée | `task_engine_parent_rehabilitated` | `Some("failed")` | `Some("in_progress")` | `groom_reap_overturned (reap_at: …, groom_child: …)` |
| refusée par le prédicat | `task_engine_parent_rehabilitation_refused` | **`None`** | `Some(<raison>)` | la raison, en clair |

`<raison>` ∈ `not_self_dev_failed_parent` (a), `last_transition_not_groom_reap`
(b), `no_groomed_child_since_reap` (c), `audit_trail_unreadable`,
`parent_rehab_lost_race`. `session_id = format!("system-{}", db.agent_id())`,
`target_key` = l'id de parente **nu** — les deux conventions du reaper
(`engine.rs` l. 4687), pour qu'un opérateur joigne les deux lignes sur la même
clé.

**`before_value = None` sur le refus n'est pas une économie, c'est le D1
appliqué à soi-même.** Une ligne de refus porte un jugement, pas une transition :
avec un `before_value`, elle deviendrait elle-même « la dernière transition » de
la parente et empoisonnerait la lecture (b) du dispatch suivant.

**La ligne de succès, elle, EST une transition — et c'est la propriété
d'idempotence du vol.** Après réhabilitation, la dernière transition de la
parente est `task_engine_parent_rehabilitated`, plus le reap : (b) refuse donc
toute seconde réhabilitation sur le même reap. Et (a) refuse avant elle, la
parente étant désormais `in_progress`. **Une parente est réhabilitée au plus une
fois par reap**, sans compteur et sans table.

**Le cas (d) ne s'écrit pas dans la piste, et voici par quoi il se lit.** Entre
la garde d'état et la claim il y a une dizaine de chemins de `return Err`, et
une ligne d'audit par chemin voudrait les toucher tous. La paire qui répond :
`parent_rehabilitation_decided` (INFO, `target: "mika::otel"`, champs
`task_id`, `decision`, `refusal` — **absent**, jamais `""`, quand la décision est
`Eligible`, motif mika#2331) est émise **à chaque évaluation**, branche saine
comprise ; et le refus aval nomme sa propre garde dans `tasks.result`, que
`record_dispatch_rejection` vient d'y écrire. Une sonde qui dit *« eligible »*
sans ligne `task_engine_parent_rehabilitated` en face **est** le cas (d), avec le
nom de la garde qui a refusé. Sans cette ligne, *« aucun cas »* et *« jamais
évalué »* rendraient les mêmes octets (classe mika#2205) ; et **l'absence de la
ligne elle-même ne veut pas dire « aucune évaluation » mais « binaire antérieur
au vol »** (mika#2340) : l'instrument n'est pas vivant avant `make deploy`.

Le prédicat n'est consulté que dans la branche où la garde d'état refusait déjà.
Une parente `pending` ou `in_progress` ne l'atteint jamais : le volume de ces
lignes est celui des dispatchs déjà refusés, pas celui des dispatchs.

### 2.6 Les tests

Tous passent par **`validate_dispatch_readiness` elle-même** — AC2 dit *« un test
par cas dans le même appel »*. Site : le `mod tests` de `executor.rs`, qui
importe déjà `GROOM_CALLBACK_PLAN_GROOMED`, `GROOM_PLAN_VOL_1` et
`completed_groom_pair` de `crate::db::tests` (l. ~13042) et porte les assertions
`task_not_dispatchable` existantes (vers l. 9516). Fixture partagée
`reaped_groom_parent(…)` : une parente `manual`/`self_dev`/`failed`, une ligne
d'audit de reap, un enfant de groom terminal daté après elle — sur le patron de
`seeded` (l. 14630) et de `dispatch` (l. 14650).

| | cas | attendu |
|---|---|---|
| **T1** | **contrôle positif** — les trois parentes de la Mesure rejouées en fixture | réhabilitée (`in_progress` en base), `Ok("in_progress")`, une ligne `task_engine_parent_rehabilitated` |
| T2 | parente terminée par un **autre motif** : dernière transition = un refus de dispatch, un `phantom_aged_out`, ou le motif `implement` du reaper | `failed`, `task_not_dispatchable` mot pour mot, refus `last_transition_not_groom_reap` |
| T3 | une transition **suit** le reap | idem, même raison |
| T4 | groom **sans** `PLAN_GROOMED` — et sa variante marqueur hors position de verdict | idem, raison `no_groomed_child_since_reap` |
| T5 | piste d'audit **illisible** | idem, raison `audit_trail_unreadable` — fail-closed |
| T6 | parente `cancelled` | idem, raison `not_self_dev_failed_parent` *(vol 2)* |
| T7 | ligne de bord `reaper_db_error` (`after_value` NULL) comme dernière ligne du reaper | idem : ce n'est pas un reap *(vol 2)* |
| T8 | **scan de lecteur unique** d'AC3 : un second site appelant `parent_rehabilitation::decide` sous `src/` est refusé, allowlist vide | rouge sur un second lecteur *(vol 2)* |
| T9 | contrôle de bonne foi de T8 : le même scan sur une copie portant un second appel | T8 rougit vraiment *(vol 2)* |

T1 est ce qui sépare *« la garde mord »* de *« la garde bloque tout » ;* T2 à T5
sont ce qui sépare *« fail-closed »* de *« réhabilite dès que c'est `failed` »*.

---

## 3. Ce que ce vol ne fait pas

- **Le refus de ré-armer un lanceur différé dans une parente terminale** (L2a,
  mika#2169, `dispatcher.rs` l. 3963-4015). Les deux cas mesurés (`2de05d23`
  #2886 et `6d53b5d4` #1727, le 2026-10-23) n'avaient pas de groom
  `PLAN_GROOMED` : ils ne rempliraient pas (c), donc ce vol ne les sauverait pas
  et ne prétend pas le faire.
- **La parente fauchée qui porte une PR**, déjà servie par
  `try_promote_parent_on_retry_success`.
- **Les causes de reap à tort** — mika#2980, mika#2986, mika#3042. Ce vol borne
  le coût d'une erreur, il n'en retire aucune.
- **Les sondes AC4 de mika#2980 et de mika#3042**, distinctes de celle du § 7.
- **Aucun nouveau `MIKA_*`, aucune migration, aucun index, aucun changement de
  `REAPER_CLASS_DISPOSITIONS`** — ce recensement est un format de fil que les
  opérateurs groupent, et ce vol ne lui ajoute rien.

### Gel des tickets (jusqu'au 01/12)

Ce vol n'ouvre aucun ticket de suivi. Le reste nommé au § 8 (vol 2) va en
commentaire sur mika#3044 et dans la PR.

---

## 4. Fire-Disposition

Ce plan livre **un** détecteur : le scan de lecteur unique d'AC3 (T8) et son
contrôle de bonne foi (T9), **au vol 2** (§ 8). Le vol 1 ne livre que des tests
de comportement — T1 à T5 affirment ce que le code neuf fait, leur chemin de
succès est *« la réhabilitation a eu lieu »* ou *« le refus est sorti mot pour
mot »*, jamais *« aucune violation trouvée »*.

**Option retenue : (a) exception nommée en allowlist — et l'allowlist est vide.**

- **Zéro violation existante à l'atterrissage, par construction.** `decide` naît
  au vol 1 avec **un** appelant. Le scan atterrit donc **armé et vert**, et il
  n'y a aucune entrée à écrire : une allowlist vide n'est pas une exception,
  c'est l'absence de dette.
- **Aucun mécanisme d'exception n'est livré** : ni liste d'ignorés, ni attribut
  de contournement, ni variable de désarmement. Ajouter un second lecteur
  demandera de **router le site par le prédicat**, pas de l'allowlister
  (doctrine mika#2201 ; un allowlist de sites portant le même défaut est la
  forme que mika#2309 condamne).
- **L'assertion auto-nettoyante est le compte attendu** : le scan exige
  **exactement un** appel en position de production, donc il rougit aussi bien
  sur un second lecteur que sur un prédicat devenu **orphelin** (renommage,
  câblage retiré). Un scan qui ne sait que compter « pas plus d'un » passe au
  vert sur zéro lecteur, c'est-à-dire sur un correctif désarmé.
- **Le scan exclut le test, pas par une liste de fichiers** mais par
  `crate::source_scan::is_test_source_path` + la troncature au premier
  `#[cfg(test)]`, comme les trois gardes de mika#2335/#2305/#2361 — les tests T1
  à T7 appellent `validate_dispatch_readiness`, pas `decide`, mais T9 construit
  une copie qui l'appelle.

Aucune option (b) : rien n'atterrit `#[ignore]`. Aucune option (c) : rien ne
remonte à l'opérateur, le périmètre est tranché par D1 et D4.

---

## 5. Definition of Done

1. `cargo build`, `cargo clippy` (zéro warning neuf), `cargo fmt --check`,
   `cargo test -p mika-agent` verts.
2. `scripts/verify-pipeline.sh origin/main` vert, et les lints du dépôt que CI
   lance (dont `check-secrets.sh` : aucun fichier ne franchit le plafond de
   1 Mo — `db.rs` et `executor.rs` sont les deux fichiers touchés dont la taille
   compte).
3. T1 à T5 verts, chacun seul (`cargo test -p mika-agent mika3044`).
4. **Aucun test existant de `validate_dispatch_readiness` ne change de verdict** :
   les assertions `task_not_dispatchable` d'aujourd'hui (l. ~9516 et suivantes)
   portent des parentes `blocked`, `completed`, `cancelled` — aucune n'a de reap
   de groom en dernière transition, donc aucune ne traverse. Si l'une devient
   verte autrement, le prédicat est trop large : c'est le signal, pas le test
   qu'on ajuste.
5. Le commentaire de l'invariant mika#1948 (`executor.rs` l. 5075-5078) est
   amendé pour nommer les deux étapes faillibles qui suivent désormais la claim
   et le coût d'un TTL (§ 2.4).
6. Le texte du refus `task_not_dispatchable` est **inchangé octet pour octet** —
   AC1 dit *« mot pour mot »*, et c'est un format que des scripts d'opérateur
   grepent.
7. La PR porte `Code review: delegated to mika-qa (mika#2694)` et nomme en clair
   le reste du vol 2 (§ 8).

---

## 6. Acceptance criteria

Transcrits depuis le corps de senara-solutions/mika#3044.

- **AC1.** Une parente est réhabilitée (`failed` → `in_progress`) puis dispatchée
  seulement si les quatre conditions tiennent ensemble. Sinon le refus
  `task_not_dispatchable` d'aujourd'hui reste en place, mot pour mot.
  - (a) `status='failed'`, `trigger_type='manual'`, `source='self_dev'`.
  - (b) La dernière transition d'état de la parente dans `audit_events` est un
    reap de la classe groom (`groom_parent_orphaned`, `after_value='failed'`).
    Aucune transition ne la suit. Le plan nomme ce qui compte comme transition :
    une ligne qui porte un état avant et après, et non une ligne de jugement.
  - (c) Le dispatch suit un enfant de groom de cette parente, livré après le
    reap, dont le résultat porte `PLAN_GROOMED`.
  - (d) Toutes les autres gardes de `validate_dispatch_readiness` passent. La
    réhabilitation n'est écrite qu'après elles : si l'une refuse, la parente
    reste `failed`.

  → (a) § 2.1 ordre des conditions ; (b) D1 § 1.3 + la lecture du § 2.3 ;
  (c) § 2.2 et § 2.3 ; (d) D4 § 2.4. Tests T1 à T7 (§ 2.6).

- **AC2.** Fail-closed, avec un test par cas dans le même appel. Chaque cas
  laisse la parente `failed` avec le refus actuel :
  - une parente terminée par un autre motif (un refus de dispatch, un
    `phantom_aged_out`, le motif `implement` du reaper) ;
  - une parente `cancelled` ;
  - une parente dont une transition suit le reap ;
  - un groom sans `PLAN_GROOMED` ;
  - une piste d'audit illisible.

  Le contrôle positif est le cas des trois parentes de la Mesure, rejoué en
  fixture. Il réhabilite et dispatche.

  → T1 (contrôle positif), T2, T3, T4, T5 au vol 1 ; T6 et T7 au vol 2 (§ 8).

- **AC3.** La décision se prend dans un seul prédicat, lu depuis un seul site.
  Chaque issue écrit une ligne d'audit relisible :
  `task_engine_parent_rehabilitated`, ou un refus avec sa raison nommée parmi
  (a) à (d). Ainsi « aucun cas » et « jamais évalué » ne rendent pas les mêmes
  octets.

  → § 2.1 (le prédicat et son site unique, livrés au vol 1), § 2.5 (les deux
  lignes, les cinq raisons, et la paire qui lit le cas (d)), T8–T9 (le scan qui
  **tient** la propriété contre la dérive, vol 2).

- **AC4.** Sonde : sur les 14 jours qui suivent le deploy, on compte les parentes
  `ready-label` en `failed` après un `groom_parent_orphaned` dont un groom a
  ensuite rendu `PLAN_GROOMED`, et parmi elles celles qui ont fini en
  `task_not_dispatchable`. La cible est 0 refus. Un compte de parentes à 0
  s'écrit « aucun reap à tort sur la fenêtre », et non « réhabilitation
  vérifiée ».

  → § 7.

---

## 7. La sonde AC4, et comment elle se ferme

Fenêtre : 14 jours **après le `make deploy`** (pas après le merge), noter la
date. Lecture seule sur `~/.mika/data/mika.db`.

**Dénominateur** — parentes `manual`/`self_dev` passées à `failed` par un
`groom_parent_orphaned` (`after_value='failed'`) sur la fenêtre, dont un enfant
de groom a ensuite rendu un `result` que `groom_result_convergence` juge
convergé. Le filtre `PLAN_GROOMED` se fait **hors SQL** (doctrine mika#2590) :
le SQL sort les `result` bruts, le tri se fait à la lecture.

**Numérateur** — parmi elles, celles dont `tasks.result` porte
`task_not_dispatchable`, ou qui n'ont aucune ligne
`task_engine_parent_rehabilitated`.

**Troisième terme, celui qui empêche la lecture molle** — le compte de lignes
`parent_rehabilitation_decided` dans les logs, par `decision`. C'est le
dénominateur des évaluations.

Trois lectures, et une seule conclut :

- dénominateur > 0, numérateur = 0 ⇒ **la réhabilitation fonctionne.**
- dénominateur = 0 ⇒ **« aucun reap à tort sur la fenêtre »**, et rien de plus.
  mika#2986 et mika#3042 ont retiré les causes connues : ce cas est le plus
  probable, et il ne vérifie rien. L'écrire autrement serait affirmer un
  correctif sur une absence de population.
- aucune ligne `parent_rehabilitation_decided` **du tout**, dénominateur
  compris ⇒ **binaire antérieur au vol** (mika#2340), pas une absence de cas. Le
  premier geste est de vérifier le sha servi, pas de conclure.

Un rouge dont l'horodatage **précède** le deploy n'est pas une régression : les
trois parentes de la Mesure resteront dans la base.

---

## 8. Taille estimée

Deux vols. Le vol 1 est celui de cette PR, et il repasse sous la borne.

**Vol 1 — le geste, son contrôle positif et les quatre refus du prédicat :**

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/parent_rehabilitation.rs` (prédicat unique, deux enums, écriture des deux lignes d'audit, doc) | 110 |
| `crates/mika-agent/src/db.rs` (`latest_state_transition_for_target`) | 30 |
| `crates/mika-agent/src/db/tasks.rs` (`promote_task_in_progress` gardée + `groom_children_delivered_since`) | 60 |
| `crates/mika-agent/src/async_db.rs` (trois enveloppes) | 30 |
| `crates/mika-agent/src/skills/executor.rs` (site de lecture unique, jeton, écriture après la claim, invariant amendé) | 58 |
| `crates/mika-agent/src/task_engine/engine.rs` (const `GROOM_PARENT_ORPHANED_EVENT`) | 5 |
| T1–T5 — AC1/AC2 (`executor.rs mod tests` : fixture `reaped_groom_parent` + contrôle positif + quatre refus) | 115 |

Total estimé : 408 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Vol 2 — le scan d'AC3 et les deux cas de population restants : 95 lignes
estimées** (T8–T9, le scan de lecteur unique et son contrôle de bonne foi, 55 ;
T6–T7, parente `cancelled` et ligne de bord `reaper_db_error`, 40). Mergeable
seul : ce sont un fichier de scan et deux tests, verts **parce que** le vol 1 les
précède — dans l'autre ordre, le scan atterrit sur un prédicat sans lecteur,
c'est-à-dire rouge, et c'est l'atterrissage que le § 4 refuse.

Tracked in: senara-solutions/mika#3044 — gel des tickets jusqu'au 01/12, donc
**aucun ticket de suivi n'est ouvert** : le vol 2 est nommé en commentaire sur
mika#3044 et dans la PR du vol 1, et sera dispatché sur le même ticket.

**Pourquoi la découpe, et pourquoi elle tombe là.** 408 + 95 = 503 lignes
estimées : sous le seuil par PR (1000) mais **au-dessus de la borne par vol
(429)**, et c'est la plus serrée qui mord. La coupe passe entre *le comportement*
et *le détecteur qui le tient* :

- **Le vol 1 satisfait AC1 entièrement, AC2 à cinq cas sur sept, et AC3 dans sa
  substance** — un seul prédicat, un seul site, les lignes d'audit nommées. Ce
  qui part au vol 2 est ce qui **tient** cette propriété contre la dérive future
  (T8–T9) et deux refus qui, eux, ne traversent **aucune** branche neuve : une
  parente `cancelled` tombe sur (a), et la ligne `reaper_db_error` est exclue par
  le discriminant D1 lui-même. Les cinq cas du vol 1 sont les seuls qui
  exercent une branche que ce vol crée.
- **Les deux tiers du vol 1 sont du code dont le patron existe mot pour mot.**
  `promote_task_completed` pour la transition, `latest_audit_event_for_target`
  pour l'ordre et le nullable, `has_completed_groom_for_issue` pour les filtres
  d'enfant, `seeded`/`dispatch` (l. 14630-14660) pour la fixture,
  `try_promote_parent_on_retry_success` pour la forme entière
  garde → transition → ligne d'audit.
- **Aucune dérive possible sur les postes lourds** : pas de migration, pas de
  `MIKA_*`, pas d'index, aucune signature publique touchée hors les trois
  enveloppes neuves, et le prédicat n'a ni dépendance neuve ni point de
  configuration.

---

## 9. Ordre de commit (vol 1)

Deux commits, dans cet ordre, chacun vert seul.

1. **`feat(3044): la base sait lire la dernière transition d'une parente et ses grooms livrés après`**
   — `db.rs`, `db/tasks.rs`, `async_db.rs`, et la const de `engine.rs`. Vert seul :
   des méthodes `pub` non encore appelées ne déclenchent pas `dead_code` dans une
   bibliothèque, et aucun comportement ne change.
2. **`fix(3044): une parente self_dev fauchée par le reaper de groom est réhabilitée quand un plan a convergé`**
   — `parent_rehabilitation.rs`, le câblage de `executor.rs` (jeton, écriture
   après la claim, invariant mika#1948 amendé) et T1–T5.

Le plancher est le commit 2 : c'est lui qui porte le geste et son contrôle
positif. Un mika-qa qui ne lirait qu'un seul commit doit lire le second.
