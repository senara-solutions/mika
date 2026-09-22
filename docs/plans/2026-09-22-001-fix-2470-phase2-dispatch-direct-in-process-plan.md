# mika#2470 — Le filet Phase 2 dispatche par appel direct du handler, il n'attend plus le webhook qu'il est censé couvrir

**Ticket :** mika issue#2470
**Type :** fix (substrat boucle — `auto_pull` Phase 2, stuck-ready reconcile)
**Date :** 2026-09-22
**Branche :** `fix/2470/p1-substrat-le-filet-auto-pull-phase-2`

---

## Problème

### Ce que le ticket mesure (2026-09-21, panne eno1)

`eno1 carrier=0` → 0 webhook GitHub reçu depuis 14:31:49Z. À 17:00:11–17:00:19Z
le moteur journalise `stuck_ready_reconciled` pour #2149 #2152 #2155.
`select count(*) from tasks where created_at > '2026-09-21T17:00:00Z'` = **0**.
Trois re-drives plus tard : `auto_pull_redrive_abandoned` « sans progrès
observable ». Le filet a « réussi » trois fois sans rien dispatcher.

### Ce que la lecture du code confirme — mot pour mot le corps du ticket

La boucle de sauvetage de `phase2_reconcile_stuck_ready`
(`crates/mika-agent/src/auto_pull.rs:3867-3967`) fait, pour chaque survivant :

```
promotion_gate_allows(…)                     // porte de fraîcheur de branche
gh_remove_label(label_auth, n, "ready")      // → GitHub émet `unlabeled`
apply_ready_label(… "phase2_stuck_rescue" …) // → ready_label::apply_ready → write_ready_label
                                             //   → GitHub émet `labeled(ready)`
record_auto_pull / reset_failure / increment_redrive
info!("stuck_ready_reconciled")
```

Aucun `create_task`, aucun spawn. Le commentaire du code l'écrit lui-même :
« *Rescue loop: remove→add the `ready` label (**Option A — reuse the webhook
pipeline**)* » (`auto_pull.rs:3919`). L'unique déclencheur est le `labeled`
que GitHub renvoie **par le canal entrant** — celui dont la perte est
précisément le cas d'usage du filet (`auto_pull.rs:3562` : « *those that have
the `ready` label but were never dispatched (**webhook dropped**, …)* »).

### Ce qui existe déjà, et qu'il suffit d'appeler

Le handler webhook `server/ready_label_handler.rs` est **entièrement
in-process** depuis mika#1572 : `try_handle_ready_label_dispatch(text, db,
github_token, message_sender, session_id, trace_id, skills, global_home_dir)`
(`:337`) parse le marqueur, passe quinze portes (allowlist dépôt 2b,
live-pilot 2c, egress 2d, siège 4b, held 4c, groomé→`dev-pilot` /
non-groomé→`dev-groom` étape 5-6), pré-crée la parente (7), valide le slot
(9d — et **enregistre un différé** si le slot est pris,
`executor.rs:1768-1806`), crée l'enfant callback (9e), marque la parente
`in_progress` (9g) et **spawne le sous-processus** (9i,
`spawn_long_running_exec`, qui enregistre `process_id` + `process_start_time`
sur l'enfant en quelques ms — `executor.rs:3580-3590`). Il ne dépend du réseau
entrant que pour **recevoir** le texte du marqueur.

Il existe une **seconde** copie de 9a–9i, `try_dispatch_pilot_after_groom_success`
(`task_engine/dispatcher.rs:3861`), écrite pour exactement la même raison
(« *This replaces the prior `gh issue edit --add-label ready` → webhook
round-trip → LLM-mediated turn chain* »). Ce plan **n'en écrit pas une
troisième** : il appelle le handler.

### Ce que le tick a déjà en main

`TaskDispatcher` (`dispatcher.rs:452`) détient `skills: Arc<SkillRegistry>`,
`global_home_dir: PathBuf`, `message_sender`, `db`, `github_token` — tout ce
que le handler demande. `dispatch_auto_pull_groomed` (`:1638`) appelle
`auto_pull_groomed_ticket(db, github_token, label_auth, trace_id, session_id,
egress_relay_down)` (`:1785`) : il manque deux références à faire descendre.

Et la Phase 2 a **déjà lu** le corps et les étiquettes de chaque ticket
(`gh_list_open_issues` → `Issue { number, body, labels, updated_at }`,
`auto_pull.rs:360`). Le handler expose une couture pour ça :
`try_handle_ready_label_dispatch_with_fetcher(…, fetch_issue)` (`:390`), dont
la doc dit : « *Production always passes `fetch_issue_body_and_labels_via_gh`;
tests pass the labels they want to gate on.* » Lui passer le corps déjà lu
épargne un `gh issue view` par sauvetage — la forme exacte de mika#2315 D5, où
la Phase 2 tend au parkeur la timeline déjà lue pour l'âge.

---

## Requirements

- **R1** — Un ticket que la Phase 2 décide de sauver est dispatché **par appel
  direct, in-process**, du handler ready-label — sans qu'aucun webhook n'ait à
  revenir. Preuve primaire : l'audit `stuck_ready_direct_dispatch
  after_value=dispatched` (D5) ; conséquence topologique : une ligne `tasks`
  (parente `self_dev`, `reference_url` de l'issue) existe à la fin du tick,
  canal entrant mort ou vivant.
- **R2** — Les quinze portes du handler s'appliquent au dispatch direct
  **inchangées** : pas de chemin « auto_pull » qui contourne live-pilot, siège,
  held, egress ou le slot par classe.
- **R3** — Quand le canal webhook est **vivant**, le `labeled(ready)` que le
  churn renvoie ne produit ni second dispatch, ni kill du pilote que le dispatch
  direct vient de lancer.
- **R4** — Le churn remove→add reste en place, **et son rôle change de nom** :
  remise à zéro de l'âge (throttle D3 de mika#1824) et déclencheur redondant —
  plus jamais l'unique déclencheur.
- **R5** — Le résultat du dispatch direct est **attribuable** par ticket dans
  `audit_events`, corrélé au tick (`trace_id`) et distinguable d'un dispatch
  webhook (`session_id` préfixé `auto-pull-`).
- **R6** — Aucun nouvel appel réseau par sauvetage : le handler reçoit le
  corps/étiquettes déjà lus par le tick.
- **R7** — Budget re-drive (mika#2020), circuit-breaker, cap
  `MAX_STUCK_RESCUE_PER_TICK`, seuil d'âge : **valeurs et sémantique
  inchangées**.

---

## Décisions

### D1 — Appeler le handler, pas dupliquer ses étapes

Deux copies de 9a–9i existent (`ready_label_handler.rs:1048-1247`,
`dispatcher.rs:3861-…`). La seconde a été écrite quand la première n'avait pas
encore de point d'entrée réutilisable depuis un callback ; aujourd'hui le
handler *est* un point d'entrée `pub async fn`. Une troisième copie serait la
dérive que `live_pilot.rs` (doc de module) et mika#2158 ont déjà payée : « *a
predicate written twice is a predicate that can disagree with itself* ». Le
handler porte les quinze portes ; les reprendre par appel est la seule façon
d'avoir R2 par construction.

**Coût accepté :** `auto_pull` référence `crate::server::ready_label_handler`.
Ce couplage `task_engine`/`auto_pull` → `server::` existe déjà
(`dispatcher.rs:275,831` → `server::deadline_verdict` ;
`auto_pull.rs:7101` → `server::ready_label_handler::ReadyLabelGate`).

### D2 — Le fetcher injecté rend le corps déjà lu (forme mika#2315 D5)

`try_handle_ready_label_dispatch_with_fetcher` reçoit une closure qui renvoie
`(issue.body.clone(), issue.labels.iter().map(|l| l.name.clone()).collect())`.
Zéro `gh issue view` (R6), et le test unitaire est hermétique (le seul appel
réseau du handler avant l'étape 9 était celui-là).

**Cohérence, pas fraîcheur :** le handler décide groomé/siège/held sur le
**même instantané** que les filtres 2–8 de la Phase 2 viennent d'utiliser pour
déclarer le ticket éligible. Une lecture fraîche pourrait au contraire faire
diverger la décision du handler de celle du filtre (ticket `held` posé entre
les deux). Le même instantané est la bonne propriété.

### D3 — Dispatch direct **avant** le churn, et le churn reste

Ordre dans la boucle de sauvetage :

```
promotion_gate_allows(…)                 // inchangé
dispatch_rescued_ticket_in_process(…)    // NOUVEAU — handler, 15 portes, spawn
gh_remove_label(…) ; apply_ready_label(…) // inchangé — âge + déclencheur redondant
record / reset / increment_redrive        // inchangé
info!("stuck_ready_reconciled", direct_dispatch = <action>, task_id)  // enrichi
```

**Pourquoi avant.** Le seul ordre dangereux est : webhook `labeled` traité
*entre* la pré-création de la parente (étape 7) et l'enregistrement du pgid
(9i) du dispatch direct — là, la porte 2c ne voit pas encore de pilote vivant,
et l'étape 6b du handler webhook **tuerait** le pilote naissant (mika#2335).
En dispatchant **d'abord**, le `labeled` renvoyé par le churn ne peut arriver
qu'après deux allers-retours `gh` (remove, add) **plus** le trajet GitHub →
Synology → gateway → agent : le pgid est écrit depuis plusieurs secondes, la
porte 2c refuse en `ready_label_pilot_in_flight` — refus que sa propre doc
qualifie de « *nominal consequence of how the feeder and the webhook path
compose* ». R3 tient par ordre, pas par verrou.

Sens inverse (webhook déjà passé avant notre appel — impossible ici puisque le
churn vient après, mais nommé) : notre appel direct tombe sur 2c (`Alive`) ou
sur la collision `idx_tasks_manual_active_ref_url` à l'étape 7
(`TaskCreateFailed`) — deux non-opérations, toutes deux auditées.

**Pourquoi garder le churn.** Il remet l'âge du label à zéro : c'est le
throttle D3 de mika#1824 qui espace les re-drives d'un ticket dont le pilote
meurt vite (900 s par défaut, tick toutes les 10 min). Sans lui, un pilote qui
échoue à la seconde ferait re-sauver le ticket au tick suivant et grillerait
les trois points du budget mika#2020 en 30 min au lieu de ~45. R7 exige de ne
pas déplacer cette cadence. Il coûte, quand le canal est vivant, une ligne
`ready_label_pilot_in_flight` par sauvetage — le prix d'un déclencheur
redondant, et il est déjà nommé nominal.

### D4 — Un texte de marqueur synthétisé, et sa forme est celle de la gateway

Le handler ne lit du texte que le préfixe `READY_LABEL_DISPATCH_MARKER` +
`<owner/repo>#<n>` bornés au premier blanc (`parse_ready_label_location`,
`:1254`), puis la **dernière** ligne pour l'acteur (`parse_event_actor`,
`:224`, tolérant : absent → `None`). Le texte synthétisé est :

```
format!("{READY_LABEL_DISPATCH_MARKER}{DEFAULT_REPO}#{n} — (auto_pull phase2_stuck_rescue)")
```

Pas de ligne `Labeled by: @…` : ce champ est un login GitHub par contrat
(`github_event_format.rs:21-50`), et y écrire un pseudo-login créerait un
acteur fictif dans l'audit. L'origine « dispatch direct » est portée par le
`session_id` (`auto-pull-<uuid>`, déjà la forme du tick) et par la ligne
d'audit de D5. Le texte sert aussi de `originating_message` à l'étape 9d, où
`starts_with(READY_LABEL_DISPATCH_MARKER)` le rend autorisé (guard 0) —
exactement comme le marqueur webhook.

Un test pur vérifie l'aller-retour `synthétiser → parse_ready_label_location`
= `(senara-solutions/mika, n)`.

### D5 — Une ligne d'audit côté `auto_pull`, qui dit ce qu'`auto_pull` a décidé, pas ce que le handler a décidé

Le handler écrit déjà `ready_label_outcome` (porte + action, `:272-330`) sous
le `trace_id` du tick — c'est le **lecteur unique** de la porte (mika#2323) et
ce plan ne le duplique pas. `auto_pull` écrit **sa** ligne :

- `tool_name = "stuck_ready_direct_dispatch"`, `target_key = "issue:<n>"`,
  `after_value ∈ {dispatched, handled, passthrough}` (le `action_label` du
  `VerdictAction`), `reasoning = "issue=<n> action=<a> task_id=<id|none>"`
  (le paramètre de `AsyncDatabase::log_audit_event`, colonne
  `audit_events.reasoning` — il n'y a pas de colonne `detail`), `trace_id` du
  tick.

`SELECT after_value, count(*) FROM audit_events WHERE tool_name =
'stuck_ready_direct_dispatch' GROUP BY 1` répond « combien de sauvetages ont
réellement dispatché » — la question que le corps du ticket a dû poser à
`tasks` faute de mieux. La jointure sur `trace_id` avec `ready_label_outcome`
donne la porte quand `after_value ≠ dispatched`.

### D6 — Ce que `Handled` signifie ici, et pourquoi on ne l'« améliore » pas

`VerdictAction::Handled { pre_digest }` est le repli F3 de mika#1572 : un
pré-digest destiné au tour LLM qui suit un webhook. Dans le tick `auto_pull`
**il n'y a pas de tour LLM** ; le pré-digest est ignoré. Trois familles de
portes rendent `Handled` :

| famille | portes | ce qu'il reste après |
|---|---|---|
| refus **décision** (avant l'étape 7) | 2b, 2c, 2d, 4b, 4c | zéro tâche créée, ticket intact, audité |
| slot occupé | 9d `DispatchReadinessFailed` | parente `pending` + **différé enregistré** (`executor.rs:1768`) — le moteur re-tire à la libération du slot ; c'est la file d'attente voulue |
| substrat cassé | 9a/9b/9e/9f (outil absent, non long-running, enfant impossible, script absent) | parente `pending` orpheline jusqu'au balayage mika#1712 |

La troisième famille laisse une parente orpheline — **identique** à ce que le
chemin webhook produit déjà dans ces mêmes états (le LLM y échouerait sur le
même outil absent). Ce n'est pas une régression, c'est nommé, et la solution
n'est pas dans ce ticket : elle est dans le fait que ces quatre portes sont des
erreurs de configuration qui font déjà WARN par leur nom.

### D7 — Les dépendances descendent par une struct, obligatoire

```rust
/// Ce que la Phase 2 doit tenir pour dispatcher in-process (mika#2470).
pub struct DirectDispatchCtx<'a> {
    pub skills: &'a SkillRegistry,
    pub global_home_dir: &'a Path,
}
```

Passée par référence à `auto_pull_groomed_ticket` et
`phase2_reconcile_stuck_ready`. **Pas d'`Option`** : un `None` serait
« le comportement d'avant », c.-à-d. le défaut de ce ticket, réintroduisible
par un appelant distrait. Les cinq tests existants qui appellent
`phase2_reconcile_stuck_ready` (`auto_pull.rs:6622,6689,7332,7428,7467`)
passent `SkillRegistry::empty()` + un `tempdir` — ils n'atteignent pas la
boucle de sauvetage (le filtre 5 lit la timeline par réseau), donc la struct y
est une formalité.

---

## Scope Boundaries

**Dedans :** `auto_pull.rs` (Phase 2 : nouvelle fonction, appel, ligne
d'audit, INFO enrichi, doc de la boucle), `dispatcher.rs` (deux références de
plus dans l'appel), tests unitaires, doc module.

**Dehors, nommé :**

- La Phase 0 (feeder) et la Phase 1 (promotion) posent `ready` sur des tickets
  qui **n'en avaient pas** — leur `labeled` est un événement neuf, le webhook
  est leur chemin nominal, et si le canal est mort la Phase 2 les rattrape au
  tick suivant… **désormais réellement**. Les faire dispatcher en direct aussi
  serait un changement de sémantique (elles deviendraient des dispatchers, pas
  des promoteurs) hors du corps du ticket.
- Fusionner les deux copies existantes de 9a–9i (`dispatcher.rs:3861` →
  appel du handler) : même classe DRY, autre ticket. Nommé en § Suivi.
- Toute modification du handler (`ready_label_handler.rs`) : **zéro**. Ni
  signature, ni porte, ni enum `VerdictAction`.
- La détection de la panne réseau elle-même (eno1, reverse-proxy Synology sur
  l'IP Ethernet) : opérationnel, hors code.

---

## Implementation Units

### U1 — `DirectDispatchCtx` et sa descente (D7)

`auto_pull.rs` : struct + champ ; `auto_pull_groomed_ticket(…, ctx:
&DirectDispatchCtx<'_>)` ; `phase2_reconcile_stuck_ready(…, ctx)`.
`dispatcher.rs:1785` : construire `DirectDispatchCtx { skills: &self.skills,
global_home_dir: &self.global_home_dir }`. Les cinq sites de test : `let
skills = SkillRegistry::empty(); let home = tempfile::tempdir()?;`.

### U2 — `dispatch_rescued_ticket_in_process` (D1, D2, D4)

```rust
/// mika#2470 — dispatch un ticket sauvé par appel direct du handler ready-label.
/// Le canal webhook peut être mort : rien ici n'en dépend.
async fn dispatch_rescued_ticket_in_process(
    db: &AsyncDatabase,
    ctx: &DirectDispatchCtx<'_>,
    issue: &Issue,
    github_token: &str,
    trace_id: &str,
    session_id: &str,
) -> VerdictAction {
    let text = synthesize_ready_label_marker(issue.number);
    let body = issue.body.clone();
    let labels: Vec<String> = issue.labels.iter().map(|l| l.name.clone()).collect();
    ready_label_handler::try_handle_ready_label_dispatch_with_fetcher(
        &text, db, Some(github_token), None, session_id, trace_id,
        ctx.skills, ctx.global_home_dir,
        move |_owner_repo, _number, _token| async move { Ok((body, labels)) },
    ).await
}

fn synthesize_ready_label_marker(n: u64) -> String { … }  // D4
```

### U3 — Le branchement dans la boucle de sauvetage (D3, D5)

Dans `phase2_reconcile_stuck_ready`, entre le bloc `promotion_gate_allows` et
`gh_remove_label` : appel de U2 (l'`issue` est déjà résolue par le `match
issues.iter().find(…)` juste au-dessus — le hisser hors du `match` pour le
réutiliser ; le bras `None` actuel — « candidate absent from the issue list »
— **passe de WARN-et-continue-le-churn à WARN-et-`continue`** : sans corps on
ne peut ni décider ni dispatcher, et churner sans dispatcher est précisément le
défaut). Puis `log_audit_event("stuck_ready_direct_dispatch", …)` (D5). Puis
le churn inchangé. Enrichir `info!(issue = n, "stuck_ready_reconciled")` avec
`direct_dispatch = action_label, task_id = …`.

Réécrire le commentaire de tête de la boucle (`auto_pull.rs:3867-3871`) :
« Option A — reuse the webhook pipeline » devient la description de D3.

### U4 — Doc module et `CLAUDE.md` du crate

Doc de `phase2_reconcile_stuck_ready` (`:3562-3574`) : ajouter la phrase qui
manquait — le filet **dispatche** ; le churn est âge + redondance. Une entrée
`docs/solutions/` (workflow-issues) : *le filet qui dépend du canal qu'il
couvre* — la classe, la preuve DB qui l'a révélée (compter les `tasks` après un
`reconciled`), le contrôle.

---

## Verification Contract

| # | test | ce qu'il établit |
|---|---|---|
| T1 | `synthesize_ready_label_marker(2470)` → `parse_ready_label_location` rend `("senara-solutions/mika", 2470)` ; `parse_event_actor` rend `None` ; `is_dispatchable_repo` vrai | D4, et que le texte franchit 2b sans ligne d'acteur fictive |
| T2 | **Hermétique, in-memory** : `dispatch_rescued_ticket_in_process` sur un `Issue` groomé, `SkillRegistry::empty()`, tempdir, token `"fake"` → **une parente** `self_dev` avec `reference_url` de l'issue existe dans `tasks` **sans qu'aucun `gh` n'ait tourné** (le fetcher injecté a été pris : sinon `gh issue view` échoue avec un faux token et l'étape 7 n'est jamais atteinte — `BodyFetchFailed`) ; `ready_label_outcome` audité avec `gate=tool_not_found`, `session_id` de test | R1 (jusqu'à l'étape 7), R6, D2 |
| T3 | Même forme, corps **non groomé** → `ready_label_handled` audité avec `dev-groom_dispatch_prepared` ; corps groomé → `dev-pilot_dispatch_prepared` | R2 : la décision groom/implement est celle du handler |
| T4 | Même forme, étiquettes `["ready", "blocked"]` → **zéro** tâche créée, `ready_label_outcome gate=operator_held` | R2 : une porte de décision refuse avant l'étape 7 |
| T5 | Même forme, un enfant callback non-terminal portant le pid du **processus de test** (forme de `test_auto_pull_live_pilot_filter_2279.rs`) → zéro tâche créée, `gate=pilot_in_flight` | R3 côté « notre appel arrive second » |
| T6 | Contrôle négatif : `phase2_reconcile_stuck_ready` sur un ticket `held` (test existant `:6622`) avec le nouveau `ctx` → toujours `rescued == 0`, **aucune** ligne `stuck_ready_direct_dispatch` | la Phase 2 ne dispatche que ce qu'elle sauve |
| T7 | Après U3, l'ordre est lisible dans le source : un scan trivial (test) vérifie que dans le corps de `phase2_reconcile_stuck_ready`, l'appel `dispatch_rescued_ticket_in_process` **précède** `gh_remove_label(label_auth, n, "ready")` | D3 — l'ordre est la garantie R3, il doit rougir si on l'inverse |

**Rouge-avant :** T2 ne peut pas exister sur `main` (la fonction n'existe pas).
Le rouge de **classe** est celui du ticket : sur `main`, après un
`stuck_ready_reconciled`, `count(tasks where reference_url = <issue>)` = 0. Il
est **rejoué en sonde post-déploiement** (§ Sondes, S1), pas en test unitaire —
la boucle de sauvetage lit la timeline par réseau et n'est pas hermétisable
sans un chantier hors périmètre.

**Mutation :** inverser l'ordre dans U3 (churn puis dispatch) → T7 rougit
seul. Retirer le fetcher injecté (passer `fetch_issue_body_and_labels_via_gh`)
→ **T2, T3 et T4 rougissent** (`body_fetch_failed` à l'étape 4 du handler, zéro
parente : les trois passent par `fetch_issue` avec le jeton `"fake"`) ; **T5
reste vert** parce que la porte 2c (`pilot_in_flight`) précède l'étape 4 ;
T1, T6, T7 inchangés.

`cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check` verts.

---

## Definition of Done

- [ ] U1–U4 livrés.
- [ ] T1–T7 verts ; les deux mutations vérifiées (ordre inversé → T7 seul ;
      fetcher retiré → T2+T3+T4 rouges, T5 vert).
- [ ] `ready_label_handler.rs` : diff **vide**.
- [ ] Constantes `STUCK_READY_THRESHOLD_DEFAULT_SECS`, `MAX_STUCK_RESCUE_PER_TICK`,
      `CIRCUIT_BREAKER_THRESHOLD`, budget re-drive : inchangées.
- [ ] Aucune migration, `schema_version` inchangée.
- [ ] Le commentaire « Option A — reuse the webhook pipeline » n'existe plus
      dans `auto_pull.rs`.
- [ ] Sonde S1 écrite dans le corps de la PR avec sa requête et sa halte.
- [ ] Suivi (fusion de la copie `dispatcher.rs:3861`) ouvert ou nommé dans la PR.
- [x] **(mika-arch F1)** Encadré de rectification daté dans le corps de
      mika#2470 sous § Attendu **et** commentaire d'avis d'édition posté
      (`issuecomment-5770350678`, 2026-09-22) — fait au groom, avant la seconde
      passe.
- [ ] **(mika-arch F2)** Callout canonique `> - **Branch:** fix/2470/p1-substrat-le-filet-auto-pull-phase-2`
      (+ `**Plan:**`, `**Grooming history:**`) présent en tête du corps de mika#2470 **avant**
      que le `ready` déjà posé puisse être consommé par un dispatcher — écrit
      par `/mika-groom-ticket` Phase 5 étape 19 ; à vérifier par
      `gh issue view 2470 --json body -q .body | grep -cF '**Branch:**'` ≥ 1.
      Séquencement mika#844 : le callout précède le dispatch.

---

## Acceptance criteria

Le corps de mika#2470 n'a pas de section « Acceptance Criteria » ; son
§ **Attendu** en tient lieu. Transcrit verbatim, puis découpé :

> La réconciliation Phase 2 doit **dispatcher par appel DIRECT du handler
> in-process** (l'équivalent de ce que `ready_label_handler` fait à la réception
> du webhook : créer la tâche de groom/dispatch), au lieu de ré-poser
> l'étiquette et d'attendre un webhook. La ré-écriture du label peut rester
> (pour l'âge/idempotence), mais elle ne doit pas être le seul mécanisme de
> déclenchement.

- **AC1** — « dispatcher par appel DIRECT du handler in-process » : pour un
  ticket sauvé, **le succès primaire est la ligne d'audit**
  `stuck_ready_direct_dispatch after_value=dispatched` (D5), jointurable par
  `trace_id` à `ready_label_outcome gate=dispatched` — écrite **sans qu'aucun
  webhook n'ait à revenir**. La parente `self_dev` (et, slot libre, l'enfant
  callback avec pgid) en est la **conséquence topologique attendue**, pas le
  porteur du succès : c'est ce que T2 observe jusqu'à l'étape 7, et ce que S1
  compte en second. ← U2/U3, D5, T2, S1.
- **AC2** — « l'équivalent de ce que `ready_label_handler` fait » : ce n'est
  pas un équivalent, c'est **le même code** — décision groom/dispatch, portes,
  slot et différé compris. ← D1, T3, T4, T5.
- **AC3** — « la ré-écriture du label peut rester (pour l'âge/idempotence) » :
  le churn est conservé, après le dispatch, rôle documenté. ← D3, U3, T7.
- **AC4** — « ne doit pas être le seul mécanisme de déclenchement » : sur canal
  mort, S1 montre un `stuck_ready_direct_dispatch after_value=dispatched` posé
  dans le même tick (`trace_id`) qu'un `stuck_ready_reconciled` — et, en
  conséquence, des `tasks` créées après lui ; sur canal vivant, S2 montre un
  seul `ready_label_outcome gate=dispatched` (session `auto-pull-…`) suivi d'un
  `ready_label_pilot_in_flight` (session webhook). ← S1, S2.

**Rectification portée, et assumée :** le ticket dit « créer la tâche de
groom/dispatch ». Le handler crée **deux** lignes (parente + enfant callback)
et spawne — c'est la topologie d'un dispatch (`live_pilot.rs`, doc de module,
tableau des deux rows). Le plan tient l'intention (une tâche existe, un pilote
part), pas la lettre (« la » tâche).

**Trace d'audit de la rectification (mika-arch F1, première passe) :** encadré
daté « Rectification (2026-09-22, …) » ajouté au corps de mika#2470 sous
§ Attendu (`gh issue edit`, 2026-09-22) ; commentaire d'avis d'édition posté
(`mika#2470#issuecomment-5770350678`). Forme issue-as-versioned-contract :
édition du corps + avis d'édition + annotation de clôture ici.

---

## Fire-Disposition

**(c) Aucun détecteur livré armé sur des données existantes.** T7 est un scan
de source qui contrôle l'**ordre** de deux appels dans une fonction que ce
ticket réécrit — il ne peut pas tirer sur du code préexistant puisque l'appel
qu'il cherche n'existe pas encore ; il est vert à la livraison par
construction, et rouge si un futur diff inverse l'ordre. Pas d'allowlist.

---

## Sondes post-déploiement, et leurs haltes

### S1 — Le rouge du ticket rejoué (canal mort ou vivant, la sonde ne distingue pas)

Après déploiement, au premier `stuck_ready_reconciled` réel :

```sql
select a.target_key, a.after_value, a.reasoning, a.trace_id
  from audit_events a where a.tool_name = 'stuck_ready_direct_dispatch'
 order by a.created_at desc limit 5;
select count(*) from tasks
 where source = 'self_dev' and reference_url like '%/issues/<n>'
   and created_at >= '<ts du reconciled>';
```

**Attendu :** `after_value = dispatched` (succès primaire, AC1) **puis**
`count ≥ 1` (conséquence topologique). **Halte :**
`after_value ∈ {handled, passthrough}` → joindre `ready_label_outcome` sur
`trace_id`, lire la porte, ne **pas** conclure « réparé » avant d'avoir vu un
`dispatched` (mémoire : *déployé ≠ efficace — voir le mécanisme faucher du
vivant*).

### S2 — Sur canal vivant, un seul pilote (R3)

Sur le même ticket que S1, si le canal est vivant :

```sql
select tool_name, after_value, created_at from audit_events
 where target_key = 'senara-solutions/mika#<n>'
   and tool_name in ('ready_label_outcome','ready_label_pilot_in_flight')
 order by created_at;
```

**Attendu :** un `ready_label_outcome gate=dispatched` (session `auto-pull-…`)
puis un `ready_label_pilot_in_flight` (session webhook). **Halte :** deux
`gate=dispatched` sur le même ticket dans la même minute → la fenêtre de D3
s'est fermée moins vite que prévu ; **mesurer** l'écart entre `created_at` des
deux avant d'ouvrir quoi que ce soit.

**Branche non-`Dispatched` (revue de code, constat #3) :** quand la ligne
`stuck_ready_direct_dispatch` d'un ticket dit `handled` avec une parente
`pending` (slot occupé → différé), le `labeled` du churn revient en
`ready_label_outcome gate=task_create_failed` (session webhook) puis un tour
LLM `Passthrough` — **attendu, pas une anomalie**, tant que la ratification
de la troisième passe mika-arch (gating du churn, option A/B) n'a pas atterri.
Ne pas lire ce `task_create_failed` comme un défaut du filet.

### S3 — Le budget ne s'est pas déplacé (R7)

Sur 48 h : `select issue_number, redrive_count from auto_pull_stats where
redrive_count > 0` — un ticket ne doit pas atteindre 3 re-drives **avec** trois
`stuck_ready_direct_dispatch after_value=dispatched` : ce serait un pilote qui
meurt trois fois, et c'est un autre ticket (celui du pilote), pas celui-ci.

---

## Suivi (hors périmètre, nommé)

1. **Fusionner `try_dispatch_pilot_after_groom_success` (`dispatcher.rs:3861`)
   dans un appel du handler** — la copie n°2 de 9a–9i ; même classe DRY que D1.
   Condition de réveil : ce ticket mergé et S1 verte une fois.
2. **Phase 0/1 en dispatch direct** si une seconde panne de canal montre que le
   délai « promotion → tick suivant → Phase 2 » (≤ 10 min + 900 s d'âge) coûte
   quelque chose de mesurable. Condition : une mesure, pas une intuition.

---

## Ce que ce travail n'achète PAS

- Il ne rend pas le canal webhook fiable ; il rend le filet **indépendant** du
  canal. La panne eno1 reste un geste physique.
- Il ne supprime pas la parente orpheline des portes 9a/9b/9e/9f (D6, famille
  3) — état préexistant du chemin webhook, nommé, balayé par mika#1712.
- Il ne change ni la cadence, ni le budget, ni le seuil d'âge du filet (R7).

---

## Références

- `crates/mika-agent/src/auto_pull.rs:3575-3969` — `phase2_reconcile_stuck_ready`, boucle de sauvetage `:3867-3967`.
- `crates/mika-agent/src/auto_pull.rs:2953-2981` — `apply_ready_label` (ré-écriture, pas de dispatch).
- `crates/mika-agent/src/ready_label.rs:580-649` — `apply_ready` → `write_ready_label`.
- `crates/mika-agent/src/server/ready_label_handler.rs:337-464` — entrées publiques, couture `_with_fetcher`.
- `crates/mika-agent/src/server/ready_label_handler.rs:1048-1247` — étapes 9a–9i (spawn moteur).
- `crates/mika-agent/src/server/ready_label_handler.rs:1254` — `parse_ready_label_location`.
- `crates/mika-agent/src/task_engine/dispatcher.rs:452,1638,1785` — `TaskDispatcher`, `dispatch_auto_pull_groomed`, site d'appel.
- `crates/mika-agent/src/task_engine/dispatcher.rs:3861` — la seconde copie de 9a–9i (mika#1614).
- `crates/mika-agent/src/live_pilot.rs` — porte 2c, topologie parente/enfant, sens fail-safe.
- `crates/mika-agent/src/skills/executor.rs:1740-1830` — slot occupé → différé enregistré.
- `crates/mika-agent/src/skills/executor.rs:3480,3580-3590` — spawn + enregistrement pid/start_time.
- `crates/mika-common/src/github_event_format.rs` — `READY_LABEL_DISPATCH_MARKER`, `LABELED_BY_LINE_PREFIX`.
- mika#1824 (Phase 2, D3 throttle), mika#1572 (dispatch moteur), mika#2279 (porte 2c), mika#2335 (kill à la supersession), mika#2315 D5 (timeline réutilisée), mika#2323 (lecteur unique de la porte), mika#2020 (budget re-drive), mika#1614 (copie n°2), mika#2449 (même schéma d'abandon).

## Revision history

- 2026-09-22 — v4, `/ce:plan` mode reprise (pipeline `/mika`, orchestrator-CC) :
  vérification de confiance contre `origin/main` (branche 0 derrière). Ancres
  structurelles tenues ; dérive de lignes seulement (`phase2_reconcile_stuck_ready`
  à `auto_pull.rs:3575`, boucle de sauvetage `:3867-3967`, commentaire « Option A »
  `:3867`). Une précision d'exécutabilité pour U3/D5 : `action_label` est **privé**
  à `ready_label_handler.rs` (`fn action_label`, `:195`) — `auto_pull` mappe
  `VerdictAction` → `{dispatched, handled, passthrough}` par un `match` local à
  trois bras, mêmes valeurs, pour tenir la case DoD « diff vide sur le handler ».
  Revue doc (`/ce:doc-review`, coherence + feasibility) : trois citations de
  lignes réalignées ; D5/S1 corrigés `detail` → `reasoning` (seule colonne
  d'`audit_events` qui existe) ; mutation « fetcher retiré » précisée (T2+T3+T4
  rouges, T5 vert — T3/T4 passent aussi par l'étape 4 `fetch_issue`).
- 2026-09-22 — v5, après `/ce:review` (7 relecteurs, « Ready with fixes », 3×P2) :
  **#2** appliqué — le plafond `MAX_STUCK_RESCUE_PER_TICK` compte les
  **tentatives** de dispatch (avant l'appel direct), plus les churns réussis ;
  T7 étendu pour l'épingler. **#1/#3 (partie doc)** appliqué — le commentaire
  de la boucle et le `CLAUDE.md` du crate disent « marge temporelle » (9i est
  un `tokio::spawn`, le pgid est écrit après le retour du handler) et nomment
  la branche non-`Dispatched` (`task_create_failed` → `Passthrough`). **#3/#1
  (gating du churn sur l'issue de l'appel direct)** : modifie le dessin de D3
  → ROUTÉ à mika-arch en choix forcé (option A : sauter le churn quand le
  moteur tient déjà le ticket ; option B : churn inconditionnel + S2 nommée),
  session `2cee847f`, non construit avant ratification.
- 2026-09-22 — v1, /ce:plan par orchestrator-CC, avant première passe mika-arch.
- 2026-09-22 — v3, seconde passe mika-arch : **Verdict: GROOMED** (même session,
  kimi-k3). F1/F2/F3 RESOLVED, aucun constat nouveau. Précision non bloquante
  retenue : la case DoD F2 est cochée dans le commentaire de clôture du groom
  **avec la sortie du grep** comme preuve.
- 2026-09-22 — v2, après première passe mika-arch (ITERATE, session
  `2cee847f-41fa-4a32-bffc-6cd9cc9b71fe`, kimi-k3) :
  **F1** rectification tracée (encadré daté dans le corps + avis d'édition
  `issuecomment-5770350678` + annotation § AC + case DoD) ;
  **F2** case DoD « callout Branch/Plan/Grooming-history dans le corps avant
  consommation du `ready` posé » ;
  **F3** AC1/AC4 réécrits : succès primaire = `stuck_ready_direct_dispatch
  after_value=dispatched` jointuré à `ready_label_outcome` par `trace_id` ; la
  parente `self_dev` est la conséquence topologique ; S1 aligné.
