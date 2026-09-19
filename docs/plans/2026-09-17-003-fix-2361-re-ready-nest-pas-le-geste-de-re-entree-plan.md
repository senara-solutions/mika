# mika#2361 — le geste de ré-entrée n'est pas celui que l'opérateur fait, et rien ne le lui dit

> **Ticket :** senara-solutions/mika#2361 · **Priorité :** p1 (substrate)
> **Victime mesurée :** #2360 (2026-09-17, ~50 min de geste inerte)
> **Fichier principal :** `crates/mika-agent/src/auto_pull.rs`

---

## 1. Ce que la lecture du code établit, et ce qu'elle réfute

### 1.1 La cause du geste inerte — établie, pas supposée

`classify_stuck_ready` (`auto_pull.rs:1605`) appelle `classify_stuck_ready_in_memory`
**en premier**, et celle-ci rend `Skip { reason: FILTER_OPERATOR_HELD }` dès que
`is_feeder_excluded(issue)` est vrai — c'est-à-dire dès que le ticket porte l'un des
trois labels de `webhook_dispatch::OPERATOR_HELD_LABELS` (`blocked`,
`operator-review`, `operator-gated`).

La branche qui lit l'abandon est **plus bas**, ligne 1679 :

```rust
// Past filter A, a ticket carrying an abandonment stamp no longer carries
// `operator-review` — the operator removed it. That is the re-entry gesture.
if facts.abandoned {
    return StuckReadyVerdict::ReEntry;
}
```

Le commentaire dit la conception à voix haute : **la ré-entrée est le retrait du
label, pas la repose de `ready`.** Un ticket abandonné qui porte encore
`operator-review` (ou `blocked`) n'atteint jamais cette ligne. Reposer `ready`
par-dessus ne change strictement rien à la décision : le ticket entre bien dans le
bassin Phase 2 (filtre 1 exige `ready`), et sort au filtre A.

C'est exactement l'état de #2360 entre 13:40:07Z (stamp d'abandon) et ~14:30Z.

### 1.2 L'hypothèse « c'est le restart qui a effacé l'abandon » est **réfutée**

Le ticket hésite entre deux déclencheurs et demande de les isoler. Le code tranche
sans instrumentation supplémentaire. `reset_auto_pull_redrive` a exactement **deux**
appelants de production, tous deux dans la même boucle :

| site | verdict |
|---|---|
| `auto_pull.rs:3357` | `SkipAndResetBudget` (PR ouverte — progrès observable) |
| `auto_pull.rs:3363` | `ReEntry` |

`mark_auto_pull_redrive_abandoned` n'a qu'un appelant (`auto_pull.rs:2554`). Aucun
chemin de démarrage, aucune migration, aucun reaper, aucune commande CLI n'écrit
`redrive_abandoned_at`. **Le restart spirit de 13:30Z ne peut pas avoir effacé
l'abandon** — il est d'ailleurs *antérieur* au stamp de 13:40:07Z, ce que la preuve
du ticket contient déjà sans en tirer la conséquence.

Le seul déclencheur compatible avec le code est donc le verdict `ReEntry`, c'est-à-dire
un tick Phase 2 ayant vu le ticket abandonné **sans** label de tenue. Le commentaire de
suivi le nomme lui-même : « le lift de `blocked` + re-ready ». C'est le lift qui a
compté ; le re-`ready` était l'accompagnement.

Corollaire arithmétique qui confirme : `ReEntry` remet le compteur à 0, puis chaque
re-drive l'incrémente (`auto_pull.rs:3547`). `redrive_count = 2` à 15:30Z après une
ré-entrée vers 14:30–15:05Z est cohérent avec deux re-drives sur des ticks de 10 min —
pas avec un compteur jamais remis à zéro.

### 1.3 Ce que le refus ne dit pas — le vrai défaut

Trois silences, chacun mesurable dans le code :

**(a) Le filtre ne dit pas quel label tient.** `webhook_dispatch::operator_held_label`
rend **le label** qui tient, et son doc-comment (`webhook_dispatch.rs:244-248`) énonce
pourquoi : *« Returns **which** label held the ticket, never a bare boolean: an
operator reading a refusal needs to know which label to remove »*. Puis
`feeder_exclusion_label` (`auto_pull.rs:1796`) jette cette information et rend la
constante `FILTER_OPERATOR_HELD = "operator_review_or_blocked"`. L'opérateur qui
interroge `audit_events` ne sait pas s'il doit retirer `operator-review`, `blocked` ou
`operator-gated`. L'information est produite, puis perdue, à une ligne d'écart.

**(b) Le filtre ne distingue pas deux populations dont les remèdes diffèrent.** Un
ticket ordinaire tenu par un label attend légitimement son opérateur. Un ticket
**abandonné** tenu par un label attend un opérateur qui croit avoir déjà agi. Les deux
portent aujourd'hui le nom `operator_review_or_blocked`. C'est la famille de confusion
que ce module a déjà dû séparer deux fois : `below_threshold` vs
`no_ready_label_event` (mika#2131 — « se résout en vieillissant » vs « échoue
identiquement pour toujours ») et `in_flight_self_dev` vs
`live_pilot_orphaned_parent` (mika#2279).

**(c) Le commentaire d'abandon nomme un seul label, en dur.**
`AbandonReason::comment_body` (`auto_pull.rs:2481`) écrit *« puis retire le label
`operator-review` »* et *« l'auto-pull ne re-drivera plus ce ticket tant qu'il porte
ce label »*. Sur un ticket portant `blocked` — le cas de #2360 — ce remède est
incomplet : le retirer ne suffit pas, et le texte affirme le contraire.

**(d) La ré-entrée n'est pas attribuable en SQL.** Le verdict `ReEntry` écrit un
`info!` (`auto_pull.rs:3367`) et rien d'autre. Aucune ligne `audit_events`. C'est
précisément pourquoi le ticket a dû poser sa question (« isoler le vrai déclencheur »)
et pourquoi y répondre a demandé une lecture de code plutôt qu'une requête.

---

## 2. La lettre du correctif demandé est refusée, et la mesure existe pour le dire

Le ticket propose : *« un `labeled(ready)` postérieur à `redrive_abandoned_at`
déclenche `ReEntry` sans exiger un remove préalable »*. **Ce plan ne l'implémente
pas**, pour deux raisons mesurées, et le refus est lui-même épinglé par un test (T1).

**R1 — `OPERATOR_HELD_LABELS` est la tenue opérateur, partagée avec le gate de
dispatch.** La liste sert deux surfaces : `auto_pull::feeder_exclusion_label` et le
gate du `ready_label_handler`. mika#2263 a mesuré le coût d'une divergence entre les
deux : `blocked` excluait #1781 du feeder et de rien d'autre, et le handler l'a
re-dispatché deux fois (pgid 478551, 492118) — « un label qui tient un ticket sur un
chemin et pas sur l'autre ne tient pas le ticket ». Faire qu'un `ready` postérieur
passe par-dessus revient à énoncer : *poser `ready` sur un ticket `blocked` le
dispatche*. Ce n'est pas lever la tenue, c'est la supprimer.

**R2 — Un `labeled(ready)` n'est pas la preuve d'une intention opérateur.** mika#2279
l'a mesuré sur #2276 : les événements `labeled ready` sont **tous** de
`mika-platform-bot` — « c'est le moteur qui fait battre le label, le défaut n'a besoin
de personne pour se rejouer ». Les trois phases d'`auto_pull` posent `ready`
elles-mêmes, et le re-drive Phase 2 *est* un `remove` → `add` (`auto_pull.rs:3510-3531`).
Faire de cet événement un lever de garde opérateur offre au moteur un moyen de lever
ses propres gardes — et le mécanisme s'auto-réarmerait : le re-drive repose `ready`,
donc produit à chaque tour l'événement qui autorise le tour suivant.

**La substance de la demande est néanmoins reçue** : le geste opérateur doit être
fiable *et* le refus doit être lisible au moment où l'opérateur regarde. C'est le
périmètre de ce plan.

---

## 3. Décisions

**D1 — La décision Phase 2 ne change pas.** L'ordre des filtres est conservé : la
tenue opérateur reste prioritaire sur la ré-entrée. Aucun ticket ne change de verdict
`Skip`/`Eligible`/`ReEntry`/`Abandon`. Ce qui change est le **nom** de la population,
ce que le ticket en dit, et ce que l'audit en garde.

**D2 — Requalification locale plutôt que réordonnancement.** Dans
`classify_stuck_ready`, si le verdict rendu par `classify_stuck_ready_in_memory` est
`Skip { reason: FILTER_OPERATOR_HELD }` **et** `facts.abandoned`, il est requalifié en
`Skip { reason: FILTER_ABANDONED_OPERATOR_HELD }`. La forme est choisie parce qu'elle
rend la non-régression évidente : la variante du verdict est strictement inchangée,
seule l'étiquette bouge. Un refus de seat (`seat_refused`) n'est **pas** requalifié —
le remède y est tout autre.

**D3 — Le label tenant est relu par le lecteur canonique, jamais retesté.** Côté
appelant Phase 2, le label est obtenu par `webhook_dispatch::operator_held_label` — la
même fonction que `feeder_exclusion_label` consulte. Réécrire le test produirait deux
lecteurs pouvant diverger, ce que mika#2263 a déjà payé une fois. Coût nul : les labels
sont en mémoire.

**D4 — Le remède va sur le ticket, une fois par abandon, et nomme le label réellement
présent.** L'opérateur ne lit pas `$MIKA_SPIRIT_LOG_FILE` pendant qu'il attend. Un
commentaire est posté sur le ticket quand Phase 2 rend
`Skip { abandoned_operator_held }`.

**D5 — La borne du commentaire est l'abandon lui-même, pas une fenêtre choisie.** La
dédup lit `audit_events` avec `since = redrive_abandoned_at` : exactement *un
commentaire par abandon et par label tenant*. Une nouvelle ré-entrée suivie d'un nouvel
abandon pose un stamp plus récent et rouvre le droit au commentaire — l'idempotence
vient de la forme de la borne, pas d'une colonne de plus. La date est lue par un
accesseur dédié **côté appelant uniquement** : elle n'est pas un input de la décision,
donc `StuckReadyFacts` ne bouge pas et la fonction pure garde ses tests.

**D6 — Fail-closed sur le ledger.** Un `audit_events` illisible refuse le commentaire.
Asymétrie : un faux négatif fait attendre un opérateur qui dispose déjà du commentaire
d'abandon ; un faux positif poste toutes les 10 min, soit ~144 commentaires/jour sur un
ticket. Même sens que `wip_rescue` (mika#2199) et `qa_review_reconcile` (mika#2347),
inverse assumé de `ci_success_handler`.

**D7 — Aucune valeur de réglage ne bouge.** Pas de nouveau knob, pas de nouvelle
colonne, pas de migration. `MAX_REDRIVES_DEFAULT`, `CIRCUIT_BREAKER_THRESHOLD`,
`MIKA_AUTO_PULL_STUCK_READY_THRESHOLD_SECS` sont inchangés.

---

## 4. Implémentation

### 4.1 `crates/mika-agent/src/auto_pull.rs`

**(1) Nouvelle constante de filtre**, à côté de ses sœurs (~ligne 1245), avec le
doc-comment expliquant pourquoi elle est distincte de `FILTER_OPERATOR_HELD` :

```rust
/// Le ticket est tenu par un label opérateur **et** porte un stamp d'abandon
/// (mika#2361).
///
/// Délibérément distinct de [`FILTER_OPERATOR_HELD`], pour la raison qui a déjà
/// séparé `below_threshold` de `no_ready_label_event` (mika#2131) et
/// `in_flight_self_dev` de `live_pilot_orphaned_parent` (mika#2279) : les deux
/// états se ressemblent et leurs remèdes diffèrent. Un ticket ordinaire tenu
/// attend son opérateur ; un ticket **abandonné** tenu attend un opérateur qui
/// croit avoir déjà agi — c'est la population où reposer `ready` est inerte.
const FILTER_ABANDONED_OPERATOR_HELD: &str = "abandoned_operator_held";
```

**(2) Requalification dans `classify_stuck_ready`** (~ligne 1610) :

```rust
if let Some(verdict) = classify_stuck_ready_in_memory(issue) {
    // mika#2361 — même décision, population nommée. Un ticket abandonné tenu
    // par un label est celui où le geste opérateur naturel (reposer `ready`)
    // ne produit rien : la branche `abandoned → ReEntry` est plus bas et n'est
    // jamais atteinte. Le distinguer ici est ce qui permet au refus de dire,
    // sur le ticket, quel label retirer.
    //
    // `seat_refused` n'est PAS requalifié : son remède n'a rien à voir.
    if facts.abandoned
        && verdict == (StuckReadyVerdict::Skip { reason: FILTER_OPERATOR_HELD })
    {
        return StuckReadyVerdict::Skip {
            reason: FILTER_ABANDONED_OPERATOR_HELD,
        };
    }
    return verdict;
}
```

**(3) Action sur le verdict**, dans le `match` de `run_stuck_ready_reconcile`
(~ligne 3351). Le bras `Skip` devient discriminant :

```rust
StuckReadyVerdict::Skip { reason } => {
    if reason == FILTER_ABANDONED_OPERATOR_HELD {
        comment_reentry_blocked(db, github_token, issue, n, trace_id, session_id).await;
    }
}
```

`record_stuck_ready_verdict` est inchangée — c'est bien un skip et il doit continuer
d'entrer au ledger.

**(4) `comment_reentry_blocked`** — nouvelle fonction, à côté d'`abandon_stuck_ready` :

- lit le label tenant via `webhook_dispatch::operator_held_label` sur `issue.labels` ;
  absent → ne fait rien (état incohérent, jamais une supposition) ;
- lit `redrive_abandoned_at` (§4.2) ; absent ou illisible → ne fait rien (**fail-closed**,
  D6) ;
- compte `count_recent_audit_events_for_target(REENTRY_BLOCKED_TOOL_NAME,
  "issue:<n>@<label>", <redrive_abandoned_at>)` ; `> 0` → ne fait rien ; `Err` → ne fait
  rien + WARN `auto_pull_reentry_ledger_unreadable` ;
- poste le commentaire via `gh_comment_issue` (token de lecture, pas `label_auth` :
  aucun label n'est écrit, donc la classe mika#2228 ne s'applique pas) ;
- écrit la ligne `audit_events` **après** la pose réussie (une pose ratée doit se
  rejouer au tick suivant — même discipline que `ExclusionLedger::flush`, qui ne marque
  qu'en cas de succès) ;
- `info!` `auto_pull_reentry_blocked` avec `issue`, `held_by`, `prior_redrives`.

Corps du commentaire — il énonce un **état**, jamais un geste imputé. C'est nécessaire :
`abandon_stuck_ready` ne fait que `warn!` si le retrait de `ready` échoue
(`auto_pull.rs:2539`), donc un ticket peut porter `ready` + tenue + abandon sans que
personne n'ait rien fait. Un texte disant « tu as reposé `ready` » serait alors faux.

```
## Auto-pull : ré-entrée bloquée pour #N

Ce ticket porte à la fois le label `ready` et le label `<held_by>`, et son
compteur de re-drives a été abandonné (<redrives> re-drives).

**L'auto-pull ne le reprendra pas dans cet état.** Reposer `ready` ne suffit
pas : c'est le label `<held_by>` qui l'exclut de toutes les phases, et le
`ready` posé par-dessus ne lève rien.

**Le geste de ré-entrée est : retirer `<held_by>`.** Au tick suivant, le
compteur de re-drives repart à zéro et le ticket redevient éligible.

<sub>Émis une fois par abandon et par label tenant. Événement :
`auto_pull_reentry_blocked`. Le détail du refus est aussi en base :
`SELECT * FROM audit_events WHERE tool_name = 'auto_pull_exclusion' AND target_key = 'issue:N';`</sub>
```

**(5) Ligne d'audit sur `ReEntry`** (~ligne 3362), **SOLE WRITER** de
`auto_pull_redrive_reentry` :

```rust
StuckReadyVerdict::ReEntry => {
    if let Err(e) = db.reset_auto_pull_redrive(DEFAULT_REPO, n).await { … continue; }
    // mika#2361 — le `info!` seul a coûté au ticket fondateur une lecture de
    // code pour répondre à « qu'est-ce qui a effacé l'abandon ? ». Cette
    // fonction est le seul écrivain de ce nom, donc son ABSENCE sous un
    // abandon disparu est elle-même une information : il existerait un
    // écrivain que la garde T7 n'a pas vu.
    let _ = db.log_audit_event(
        session_id, "auto_pull_redrive_reentry", &format!("issue:{n}"),
        None, Some(&redrive_count.to_string()), Some("stuck_ready_reentry"),
        Some(trace_id),
    ).await;
    info!(issue = n, prior_redrives = redrive_count, "auto_pull_redrive_reentry");
    survivors.push(n);
}
```

**(6) `AbandonReason::comment_body`** — remplacer la mention en dur d'`operator-review`
par la liste réelle : le corps ne connaît pas les labels du ticket, donc il énonce la
règle plutôt qu'un cas. Le fragment *« puis retire le label `operator-review` »* devient
*« puis retire son label de tenue — `operator-review` (posé ci-dessus), ou `blocked` /
`operator-gated` si le ticket en porte un »*, et la phrase suivante passe de « ce
label » à « un label de tenue ».

**(7) Constante du ledger de commentaires**, SOLE WRITER documenté, à côté
d'`EXCLUSION_AUDIT_TOOL_NAME` :

```rust
const REENTRY_BLOCKED_TOOL_NAME: &str = "auto_pull_reentry_blocked";
```

> **Note sur la clé `issue:<n>@<label>`.** mika#2347 a nommé le piège d'un `LIKE` sur
> une clé de ticket (`#234` matche `#2343`). Le séparateur `@` le ferme ici :
> `LIKE 'issue:2360@%'` ne matche pas `issue:23600@blocked`, dont le caractère suivant
> le préfixe est `0` et non `@`. La requête opérateur par ticket est donc sûre.

### 4.2 `db.rs` / `async_db.rs`

Un accesseur, sans migration ni changement de schéma :

```rust
/// L'instant de l'abandon, ou `None` si le ticket n'est pas abandonné.
///
/// Séparé de [`get_auto_pull_redrive_state`] à dessein : la date n'est pas un
/// input de la décision Phase 2 (qui n'a besoin que du booléen), seulement de la
/// borne du commentaire de mika#2361. L'élargissement du tuple existant aurait
/// fait porter à `StuckReadyFacts` — et à tous ses tests — une donnée dont la
/// fonction pure n'a que faire.
pub fn get_auto_pull_redrive_abandoned_at(&self, repo: &str, issue: u64) -> Result<Option<String>>
```

plus son miroir `AsyncDatabase`.

---

## 5. Verification Contract

### 5.1 Tests unitaires — `auto_pull::tests`

| id | nom | ce qu'il épingle |
|---|---|---|
| **T1** | `mika2361_re_ready_on_a_held_abandoned_ticket_is_not_a_reentry` | **Contrôle négatif — la lettre du ticket.** Abandonné + `ready` + `operator-review` → `Skip { abandoned_operator_held }`, **jamais** `ReEntry`. Le doc-comment porte le § 2 : une lecture future du ticket qui « réparerait » le défaut en laissant passer le `ready` casse ce test et lit pourquoi. |
| **T2** | `mika2361_lifting_the_hold_is_the_reentry` | Abandonné + `ready`, tenue levée → `ReEntry`. La substance de la demande. |
| **T3** | `mika2361_each_held_label_names_itself` | Les trois labels de `OPERATOR_HELD_LABELS` produisent le même verdict, et `operator_held_label` rend chacun le sien. Boucle sur la constante partagée, jamais sur une liste recopiée. |
| **T4** | `mika2131_filter_names_are_a_wire_format` *(étendu)* | `FILTER_ABANDONED_OPERATOR_HELD == "abandoned_operator_held"` + le cas classifié, dans le test de format de fil existant (`auto_pull.rs:6057`). |
| **T5** | `mika2361_an_unabandoned_held_ticket_keeps_its_historic_filter_name` | **Non-régression de l'agrégat.** Tenu mais non abandonné → toujours `Skip { operator_review_or_blocked }`. La population historique ne se scinde pas sous les pieds de l'opérateur. |
| **T6** | `mika2361_a_seat_refusal_is_never_requalified` | Abandonné + `dispatch:ssc` → `Skip { … }` avec la raison de seat, pas la nouvelle. |
| **T7** | `mika2361_reset_auto_pull_redrive_has_exactly_two_production_callers` | **Garde structurelle**, scan de source sur `src/` (hors `#[cfg(test)]`). Allowlist vide autre que les deux sites nommés. Un test comportemental ne peut pas voir cette classe : un troisième écrivain — par exemple un reset au démarrage, l'hypothèse exacte que ce ticket a dû réfuter à la main — ne rendrait aucune décision fausse, il rendrait l'attribution impossible pendant que toutes les assertions restent vertes. Même famille que `mika2131_exclusion_skips_never_return_to_an_uncollected_debug`. |

### 5.2 Tests d'intégration (DB) — `db::tests`

| id | nom | ce qu'il épingle |
|---|---|---|
| **T8** | `mika2361_abandoned_at_is_readable_and_none_when_not_abandoned` | L'accesseur : `None` avant abandon, l'instant après, `None` après `reset_auto_pull_redrive`. |
| **T9** | `mika2361_the_reentry_comment_is_posted_once_per_abandonment` | Deux passages consécutifs sur le même état → une seule ligne `auto_pull_reentry_blocked`. Un nouvel abandon (stamp plus récent) en rouvre le droit. |
| **T10** | `mika2361_an_unreadable_ledger_refuses_the_comment` | Fail-closed (D6). |

### 5.3 Gates

`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test -p mika-agent`.

---

## Fire-Disposition

*(Exigée par le Fire-Disposition Gate — mika#1574. Ce plan livre des détecteurs
T1–T10, dont un scan structurel T7 ; cette section dit comment chacun se comporte
face aux données et au code **préexistants**.)*

**Disposition retenue : (c) halt-and-surface.** Aucun allowlist, aucun détecteur
livré désarmé. La justification est mesurée, pas prudentielle : l'état courant du
dépôt est déjà à zéro violation pour chacun des dix détecteurs, donc il n'y a rien
à tolérer, et tolérer par anticipation reviendrait à s'aveugler sur exactement la
classe que ce plan existe pour rendre attribuable.

**T7 — le seul détecteur qui tire sur du préexistant.** Il scanne `src/` à la
recherche des appelants de `reset_auto_pull_redrive` hors `#[cfg(test)]`.

- *État à l'écriture :* exactement deux appelants de production, nommés au § 1.2
  (`auto_pull.rs:3357`, `auto_pull.rs:3363`). Le détecteur passe au vert sans qu'une
  seule ligne de code de production soit modifiée pour lui — c'est la condition qui
  rend (c) disponible ici, et elle est vérifiée avant l'écriture, pas espérée.
- *Allowlist :* **vide**, et la vacuité est l'invariant. Les deux sites attendus sont
  la valeur attendue de l'assertion, pas des exemptions : un test qui tolérerait N
  appelants dont deux nommés ne dirait plus rien du jour où un troisième apparaît.
- *Si T7 échoue au land time :* **halte**. Un troisième écrivain de
  `redrive_abandoned_at` invaliderait la réfutation du § 1.2 (« le restart ne peut pas
  avoir effacé l'abandon ») et donc le diagnostic dont tout ce plan dépend. Le remède
  n'est **pas** d'ajouter le site à un allowlist ni d'assouplir le scan : c'est de lire
  le nouvel écrivain, de reprendre le § 1.2, et de rouvrir le périmètre si la cause
  mesurée a changé. La sonde S3 énonce la même halte côté production — les deux se
  répondent délibérément.
- *Auto-exclusion :* T7 vit dans `auto_pull::tests`, donc sous `#[cfg(test)]`, et le
  scan doit exclure ce module — sans quoi le détecteur se compterait lui-même et
  échouerait sur sa propre existence. C'est un défaut d'implémentation connu de cette
  famille de gardes (`mika2131_exclusion_skips_never_return_to_an_uncollected_debug`
  le traite déjà) et il est nommé ici pour que l'implémenteur ne le redécouvre pas.

**T1–T6 — détecteurs de décision, sur fixtures construites.** Ils classifient des
issues synthétiques au travers de la fonction pure ; aucun corpus préexistant n'entre
dans leur population. Ils ne peuvent donc pas « tirer » sur de l'historique. Un échec
au land time n'est jamais un héritage : il signifie que la requalification D2 a changé
un verdict, ce qu'AC8 interdit — **halte**, et non ajustement de l'attente du test.

**T8–T10 — détecteurs d'intégration, sur base temporaire.** Chaque test ouvre une DB
neuve ; aucune donnée de production n'est lue. Aucune migration n'est livrée (D7), donc
aucun schéma préexistant n'est sollicité.

**Données de production préexistantes — non touchées, et la scission est datée.** La
requalification ne réécrit aucune ligne `audit_events` déjà posée : les exclusions
historiques gardent `after_value = 'operator_review_or_blocked'`. L'agrégat se scinde
**à partir du déploiement**, et c'est voulu — réécrire l'historique pour rendre la
série continue rendrait faux ce que ces lignes ont dit au moment où elles ont été
écrites. AC7 tient l'autre moitié de la propriété : un ticket tenu mais non abandonné
continue de produire l'ancien nom, donc la série historique reste comparable à
elle-même sur la population qui n'a pas changé de nom. L'opérateur qui compare de part
et d'autre du déploiement doit sommer les deux noms ; c'est dit au § 9 (S1) et ce sera
dit au root `CLAUDE.md` avec le reste du vocabulaire (§ 6).

---

## 6. Definition of Done

- [ ] `FILTER_ABANDONED_OPERATOR_HELD` existe, est requalifiée depuis
      `FILTER_OPERATOR_HELD` sous `facts.abandoned` seul, et est au test de format de fil.
- [ ] `comment_reentry_blocked` poste un commentaire nommant le label réellement
      présent, une fois par (abandon, label), fail-closed sur le ledger.
- [ ] `StuckReadyVerdict::ReEntry` écrit une ligne `audit_events` ; ce site en est le
      seul écrivain.
- [ ] `AbandonReason::comment_body` ne nomme plus `operator-review` comme unique remède.
- [ ] T1–T10 passent ; T1 et T7 portent en doc-comment la raison de leur existence.
- [ ] T7 est livré **armé, allowlist vide**, et exclut son propre module `#[cfg(test)]`
      du scan (§ *Fire-Disposition*). Un échec au land time se solde par une halte, pas
      par une exemption.
- [ ] `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test -p mika-agent` verts.
- [ ] Root `CLAUDE.md` § *auto-pull stuck-ready reconciler* : le vocabulaire de filtres
      gagne la paire `operator_review_or_blocked` / `abandoned_operator_held` et ses
      surfaces opérateur ; la phrase **« Re-entry is one gesture: remove
      `operator-review` »** devient « retirer le label de tenue » et dit que reposer
      `ready` n'en est pas un, avec le pourquoi (§ 2).
- [ ] `crates/mika-agent/CLAUDE.md` : la ligne v49→v50 ne change pas (aucune migration) ;
      la section du reconciler reçoit la même correction de vocabulaire.
- [ ] Aucune migration, aucune colonne, aucun knob.

---

## Acceptance criteria

*(Le ticket ne porte pas de section `## Acceptance criteria` ; celles-ci sont dérivées
du « Correctif attendu », des mesures du § 1 et du refus motivé du § 2.)*

- **AC1** — Un ticket abandonné et tenu par un label opérateur est classé sous un nom
  de filtre **distinct** de celui d'un ticket simplement tenu, et ce nom est visible en
  SQL : `SELECT … WHERE tool_name = 'auto_pull_exclusion' AND after_value =
  'abandoned_operator_held'`. **Vérifié par** T1, T4, T5.

- **AC2** — Le ticket lui-même porte le remède, nommant le label **réellement présent**
  (pas `operator-review` par défaut), posté **une seule fois par abandon et par label**.
  **Vérifié par** T3, T9.

- **AC3** — Un ledger `audit_events` illisible ne produit aucun commentaire.
  **Vérifié par** T10.

- **AC4** — Retirer le label de tenue reste le geste de ré-entrée et fonctionne : le
  budget repart à zéro et `redrive_abandoned_at` est effacé au tick suivant, sans
  restart. **Vérifié par** T2, T8.

- **AC5** — Reposer `ready` sur un ticket tenu **ne** déclenche **pas** de ré-entrée, et
  ce refus est épinglé par un test qui explique pourquoi (R1, R2). **Vérifié par** T1.

- **AC6** — Toute ré-entrée est attribuable en SQL sans grep :
  `SELECT * FROM audit_events WHERE tool_name = 'auto_pull_redrive_reentry';`.
  **Vérifié par** l'inspection du site unique + T7.

- **AC7** — Un ticket tenu mais **non** abandonné garde son nom de filtre historique :
  l'agrégat `operator_review_or_blocked` reste comparable de part et d'autre du déploiement.
  **Vérifié par** T5.

- **AC8** — Aucun ticket ne change de **verdict** (`Skip`/`Eligible`/`ReEntry`/`Abandon`) ;
  seul le nom de la population change. **Vérifié par** T5, T6 et la forme même de la
  requalification (D2).

---

## 8. Hors périmètre, délibérément

- **Changer l'ordre des filtres.** La tenue opérateur reste prioritaire sur la
  ré-entrée (D1). L'inverser rendrait un ticket `blocked` dispatchable par repose de
  label.
- **Faire de `labeled(ready)` un geste de ré-entrée.** Refusé au § 2, épinglé par T1.
- **`OPERATOR_HELD_LABELS` et le gate du `ready_label_handler`.** Une liste, deux
  surfaces (mika#2263) — ce plan la lit, ne la touche pas.
- **Le battement de label par `mika-platform-bot`** (mika#2279) : il explique pourquoi
  R2 tient, il n'est pas corrigé ici.
- **La latence résiduelle après le lift.** Le ticket redevient éligible tout de suite
  (`ReEntry` précède l'étape d'âge) mais son `ready` fraîchement posé le place sous
  `below_threshold` jusqu'à `MIKA_AUTO_PULL_STUCK_READY_THRESHOLD_SECS` (900 s). Ce
  n'est pas un blocage : le webhook `labeled(ready)` du geste opérateur dispatche par le
  chemin nominal, Phase 2 n'étant qu'un filet. La latence est bornée et déjà nommée par
  un filtre existant.
- **mika#2271** (le feeder ne se ré-enregistre pas après knob-off→on) : classe voisine,
  mécanisme sans intersection avec celui-ci.

---

## 9. Sondes post-déploiement, et leurs haltes

**S1 — Population (48 h).**
```sql
SELECT target_key, count(*) FROM audit_events
WHERE tool_name = 'auto_pull_exclusion' AND after_value = 'abandoned_operator_held'
GROUP BY 1 ORDER BY 2 DESC;
```
Attendu : petit, quelques tickets. La dédup du ledger d'exclusions (24 h) borne à une
ligne par ticket et par jour. **Halte** : plusieurs lignes par jour sur un même ticket →
la dédup d'exclusion ne tient pas ; ne pas allonger la fenêtre avant d'avoir regardé
`exclusion_audit_is_due`.

**S2 — Le commentaire ne se répète pas.**
```sql
SELECT target_key, count(*) FROM audit_events
WHERE tool_name = 'auto_pull_reentry_blocked' GROUP BY 1 ORDER BY 2 DESC;
```
Attendu : **1** par `issue:<n>@<label>`. Un `> 1` signifie que la borne
`redrive_abandoned_at` n'est pas relue — c'est-à-dire le défaut que mika#2347 a dû
fermer sur une autre surface (un ledger écrit et jamais relu). **Halte** : ne pas
allonger une fenêtre, réparer la relecture.

**S3 — Attribution de la ré-entrée.**
```sql
SELECT * FROM audit_events WHERE tool_name = 'auto_pull_redrive_reentry' ORDER BY created_at DESC;
```
**Halte, et c'est la plus importante** : si un `redrive_abandoned_at` disparaît de
`auto_pull_stats` **sans** ligne correspondante, il existe un écrivain que T7 n'a pas
vu. Ne pas élargir la garde à un allowlist — chercher l'écrivain.

**S4 — Le geste opérateur, de bout en bout.** Sur le premier ticket abandonné après
déploiement : retirer le label de tenue, vérifier au tick suivant que `redrive_count = 0`
et `redrive_abandoned_at` vide, et qu'une ligne S3 le date. **Halte** : si le ticket
n'est toujours pas repris, la cause n'est pas ici — lire
`SELECT after_value FROM audit_events WHERE tool_name = 'auto_pull_exclusion' AND
target_key = 'issue:<n>' ORDER BY created_at DESC LIMIT 5`, qui nommera le vrai filtre
(`not_groomed`, `circuit_breaker`, `promotion_gate_refused`, `below_threshold`…). Ne pas
toucher au budget de re-drive par réflexe.

**S5 — Journal.** `grep auto_pull_reentry_blocked $MIKA_SPIRIT_LOG_FILE` (une ligne par
pose) et `grep auto_pull_reentry_ledger_unreadable $MIKA_SPIRIT_LOG_FILE` — **doit
rester vide** ; toute occurrence est un ledger illisible, donc un commentaire refusé par
fail-closed et un opérateur laissé sans remède sur le ticket.

---

## Revision history

- **rev 2 (2026-09-17)** — adressé F1 (BLOCKING) par l'ajout d'une section
  `## Fire-Disposition` (Fire-Disposition Gate, mika#1574), placée après § 5.3 pour ne
  renuméroter aucune section existante. Option canonique **(c) halt-and-surface**
  retenue, avec sa condition de disponibilité vérifiée plutôt que supposée : l'état
  courant du dépôt est à zéro violation pour les dix détecteurs, dont T7 dont les deux
  appelants attendus sont mesurés au § 1.2. La section traite séparément les trois
  familles de détecteurs (T7 scan structurel sur `src/` ; T1–T6 sur fixtures ; T8–T10 sur
  DB temporaire), nomme l'auto-exclusion `#[cfg(test)]` que T7 doit implémenter, et
  traite le seul préexistant réel — les lignes `audit_events` historiques portant
  `operator_review_or_blocked`, non réécrites, dont la scission est datée du déploiement.
  La halte de T7 est explicitement reliée à la halte S3 du § 9 : dans les deux cas le
  remède est de chercher l'écrivain, jamais d'élargir un allowlist. Une case de DoD
  couvre l'armement et la vacuité de l'allowlist.
- Aucune autre section modifiée ; aucun AC affaibli ni ajouté (la disposition retenue
  n'introduit pas de comportement neuf, elle documente celui des détecteurs déjà
  spécifiés au § 5).
