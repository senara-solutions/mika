---
issue: 2279
type: fix
---

# fix(mika#2279) — un événement `labeled ready` répété sur un ticket dont le pilote est vif est un NO-OP

## Symptôme mesuré (2026-09-10, #2276)

- `10:46:57` — dispatch de #2276 : parent `23fd8852`, enfant `ddb05913` (pid 577434,
  pilote VIVANT, log actif jusqu'à 11:59).
- `10:47:28` — **second** événement `labeled ready` sur le même ticket, déjà en vol :
  `ready_label_superseded_prior_rows` (superseded=1) → le parent `23fd8852` est
  `cancelled` **30 s après le spawn, son pilote tournant** → `dispatch readiness check
  failed` → `deferred_dispatch_registered` : `22a578ee` (manual) + `bda96097` (callback
  deferred, sans pid) = un **second pilote #2276 en doublon** en attente de retry.
- Annulé à la main à 11:02. **Un nouveau couple réapparaît à 11:10** (`29756ff0` /
  `4d5d966d`), puis à 11:30. Le cycle a une période d'environ 20 minutes.
- Mesure n°3 (commentaire 2) : les événements GitHub sur #2276 sont **tous** de
  `mika-platform-bot`, aucun humain — `09:29:54Z labeled → 09:49:52Z unlabeled →
  09:49:54Z labeled → 09:59:52Z unlabeled`. **C'est le moteur qui fait battre le label.**

Le défaut s'auto-entretient : il n'a pas besoin d'un opérateur pour se rejouer.

## Ce que le code établit aujourd'hui

### Le défaut (a) du ticket est fermé — et sa fermeture aggrave (b)

Le ticket décrit le défaut (a) ainsi : *« le supersede annule le parent mais ne tue PAS le
pilote enfant vivant »*. C'était vrai à la rédaction. Ça ne l'est plus : mika#2263 puis
mika#2335 (`348f470f`, `15280de2`) ont donné à `dispose_superseded_dispatch_processes`
(`crates/mika-agent/src/tracking_cleanup.rs:228`) la traversée parent → enfant-avec-pgid
(`find_dispatch_children_with_pid`, `db.rs:8083`) et le kill du groupe de processus.
`crates/mika-agent/tests/eval/test_supersede_kills_live_pilot.rs` l'épingle.

**Cette fermeture change le signe du défaut (b), elle ne le referme pas.** Avant #2335,
un re-label laissait un pilote vivant orphelin de son parent — un gâchis et un doublon.
Après #2335, **le même re-label tue un pilote productif** : c'est le contrat explicite de
la supersession, et il est correct *pour un dispatch neuf*. Un événement `labeled` rejoué
n'est pas un dispatch neuf. La garde manquante n'est donc pas dans le supersede — l'y
mettre annulerait mika#2335 — elle est **en amont, sur le déclencheur**.

### La boucle, en six temps

1. `labeled ready` n°2 arrive pendant que le pilote de #2276 tourne.
2. `ready_label_handler` (`server/ready_label_handler.rs:121`) traverse ses trois portes —
   allowlist de dépôt (2b, l.166), siège de dispatch (4b, l.256), operator-held (4c,
   l.325). **Aucune ne regarde si un pilote est déjà en vol pour ce ticket.**
3. Étape 6b (l.402) : `supersede_prior_tracking_rows` annule le parent — et, depuis
   #2335, tue le pilote.
4. Étape 7 (l.444) : une nouvelle row de tracking est pré-créée (le slot d'URL vient
   d'être libéré par l'annulation).
5. Étape 9d (l.566) : `validate_dispatch_readiness` répond `global_dispatch_active` —
   `has_active_callback_tasks_excluding` (`db.rs:9128`) voit bien l'enfant vif, mais ne
   peut que constater *un* slot occupé, pas *le même ticket* : il exclut par
   `t.parent_task_id != ?1`, et `?1` est la row neuve. → fallback pre-digest → le LLM
   appelle `run_claude_pilot` → rejet → `deferred_dispatch_registered`. **Un second pilote
   #2276 est en file.**
6. Le parent étant `cancelled`, `has_active_self_dev_task_for_issue` (`db.rs:7659`) répond
   **faux** : sa conjonction est `reference_url LIKE <url>%` **et**
   `status IN ('pending','in_progress')` — or l'URL n'est que sur le parent (annulé) et le
   pgid n'est que sur l'enfant (sans URL). Le reconciliateur Phase 2 d'`auto_pull`
   (`auto_pull.rs:3249`) lit donc `in_flight = false` sur un ticket `ready` « bloqué »,
   et le re-drive : `unlabeled` puis `labeled`. **Retour au temps 1.**

Le temps 6 est ce qui transforme un défaut ponctuel en boucle. C'est la même topologie à
deux lignes que mika#2335 a dû nommer pour le kill : *l'URL est sur le parent, le pgid sur
l'enfant, rien ne porte les deux.* Le prédicat d'in-flight ne l'a jamais traversée.

### Ce que la boucle coûte, au-delà du doublon

Chaque tour consomme un point du budget de re-drive (mika#2020,
`MIKA_AUTO_PULL_MAX_REDRIVES`, défaut 3). Trois tours suffisent donc à faire **abandonner
un ticket parfaitement sain** : `operator-review` posé, `ready` retiré, commentaire
d'abandon, ticket sorti des trois phases jusqu'à un geste humain. La garde manquante ne
protège pas seulement un pilote, elle protège le budget qui protège la boucle.

## Le correctif — deux gardes et un prédicat partagé

### Prédicat partagé : `live_pilot`

Nouveau module `crates/mika-agent/src/live_pilot.rs`, **lecteur unique** de la question
« un pilote est-il vif pour ce ticket ? ». Un seul module parce que la réponse est lue de
deux endroits et qu'un prédicat écrit deux fois est un prédicat qui peut se contredire —
la leçon que `grooming_marker.rs` a dû engraver une fois (mika#2158) et que mika#2335 a
payée sur ce chemin exact.

```rust
pub enum LivePilotVerdict {
    /// Un enfant callback non-terminal porte un pgid dont le processus est vivant.
    Alive { child_task_id: String, parent_task_id: String, pid: u32 },
    /// Aucun enfant de cette forme.
    None,
    /// La question n'a pas pu être posée (erreur DB) ou pas tranchée
    /// (`process_start_time` illisible).
    Unreadable { reason: &'static str },
}

pub async fn live_pilot_for_issue(db: &AsyncDatabase, issue_url: &str) -> LivePilotVerdict;
```

Une requête DB, `Database::find_dispatch_children_for_issue_url(issue_url)` : traversée
`tasks child JOIN tasks parent ON child.parent_task_id = parent.id` avec
`parent.reference_url LIKE <issue_url>%` (le `LIKE` couvre la variante `?phase=groom`,
comme `has_active_self_dev_task_for_issue`), `child.trigger_type = 'callback'`,
`child.process_id IS NOT NULL`, `child.status NOT IN (<terminaux>)` —
**et aucun prédicat sur le statut du parent**. C'est l'absence de ce prédicat qui est le
correctif : un parent annulé est précisément l'état où la question se pose.

Réutilisations : la forme du `SELECT` et le garde `json_valid(metadata)` de
`find_dispatch_children_with_pid` (`db.rs:8087-8106`, le garde est porteur : un
`metadata` non-JSON fait échouer tout le `query_map`, pas seulement sa ligne) ; la
vivacité par `process_liveness::is_same_process_alive(pid, start_time)`
(`task_engine/process_liveness.rs:51`), jamais un `/proc/<pid>` nu — un PID recyclé y est
indistinguable du pilote.

**Fail-safe, dans le sens de la maison : un signal illisible n'est jamais un terme
satisfait.** `Unreadable` n'est pas `Alive`. Pas de `process_start_time` lisible, erreur
DB : on ne peut pas prouver la vivacité, donc on ne bloque pas — le comportement d'avant ce
correctif est repris. Le coût est nommé plutôt que caché : cette population est celle que
mika#2335 compte déjà sous `unusable_child_count`, et c'est le seul cas où #2279 peut
encore se produire. L'inverse (conclure `Alive` sur un signal illisible) gèlerait un ticket
sur une supposition.

**L'asymétrie est réelle et penche du bon côté** : un faux `Alive` gèle un ticket, mais le
gel est *borné* — le watchdog PID (#959), le reaper de stall silencieux (#2249/#2277) et
le balayage phantom (#1712) rendent la row terminale, et le prédicat redevient faux de
lui-même. Un faux `None` rejoue exactement l'incident mesuré, en boucle, toutes les
20 minutes. On ne choisit pas entre deux erreurs de même poids.

### Garde A — `ready_label_handler`, porte 2c

Nouvelle porte dans `try_handle_ready_label_dispatch_with_fetcher`, **entre 2b (allowlist
de dépôt) et 3 (résolution du token)** :

```
2b. allowlist de dépôt      (aucun round-trip)
2c. pilote vif              ← NOUVEAU : une requête DB locale, aucun `gh`
3.  token
4.  gh issue view
4b. siège de dispatch
4c. operator-held
...
6b. supersede               ← n'est plus atteint sur un ticket en vol
7.  pre-create
```

Placement raisonné, pas subi :

- **Avant l'étape 3**, donc avant le `gh issue view` : le prédicat ne dépend pas du corps
  de l'issue, seulement de son URL. La boucle mesurée faisait un `gh issue view` toutes les
  20 min pour rien ; la porte l'économise. Même raisonnement que 2b (« le refus le moins
  cher d'abord »).
- **Avant 6b et 7**, ce qui est la propriété qui compte et que les trois portes voisines
  énoncent déjà : **zéro tâche créée, zéro processus tué, zéro dispatch différé**. Le
  différé du temps 5 disparaît parce que l'étape 9d n'est jamais atteinte, pas parce qu'on
  aurait ajouté une exception au registre des différés.
- **Ordre vis-à-vis de 4c** : un ticket à la fois en vol et tenu par l'opérateur sera
  refusé comme « en vol ». Les deux refus ont le même effet (zéro tâche) ; seul le nom de
  l'événement change, et le moins cher gagne. Choix nommé pour qu'il ne soit pas relu
  comme un oubli.

Refus en **`VerdictAction::Handled`, jamais `Passthrough`** — pour la quatrième fois dans
ce fichier et pour la raison qui y est écrite trois fois : `Passthrough` laisse `req.text`
sur le marqueur ready, ce que l'INTENT_GUARD `webhook_ready_label_dispatch` déclenche, et
le LLM serait re-sommé de dispatcher le ticket que la porte vient de refuser. Le pre-digest
ouvre sur `<ready_label_handler>` (ne matche pas le trigger, la composition est
structurelle) et nomme : le ticket, le pilote vivant et son pid, l'interdiction d'appeler
`run_claude_pilot*`, **et le geste opérateur pour forcer** — `mika tasks cancel <id>` (qui
depuis mika#2335 avertit sur un pilote vif et demande confirmation) puis re-poser `ready`.
Un refus qui ne nomme pas sa levée est un refus qu'on contourne au jugé.

### Garde B — `auto_pull`, filtre Phase 2

Sans B, A tient le handler mais la boucle continue de battre le label : `auto_pull` ne voit
toujours pas le pilote, re-drive toutes les 20 min, consomme le budget et finit par
abandonner un ticket sain. B est ce qui casse le battement.

Dans le chemin Phase 2 (`auto_pull.rs:3247`, Filter 4), **après** le probe `in_flight`
existant et **seulement quand il est faux** :

```rust
// Filter 4b (mika#2279) — parent terminal, pilote encore vif.
if !in_flight {
    match live_pilot::live_pilot_for_issue(db, &issue_url).await {
        LivePilotVerdict::Alive { .. } => {
            ledger.record(ExclusionPhase::Phase2StuckReady, n, FILTER_LIVE_PILOT);
            continue;   // Skip — jamais SkipAndResetBudget.
        }
        LivePilotVerdict::None | LivePilotVerdict::Unreadable { .. } => {}
    }
}
```

Trois points portants :

- **Coût borné.** Le cas nominal (parent vivant) est déjà exclu par `in_flight` et ne paie
  rien. La sonde ne coûte qu'aux tickets `ready` dont le parent est terminal — population
  normalement vide.
- **`Skip`, pas `SkipAndResetBudget`.** mika#2158 a mesuré la différence : un compteur remis
  à zéro par l'action qu'il compte ne borne rien (31 re-drives sur #1772 pendant que
  `redrive_count` disait 1). Attendre un pilote est juste ; ce n'est pas un succès.
- **Un nom de filtre distinct**, `FILTER_LIVE_PILOT = "live_pilot_orphaned_parent"`, ajouté
  aux constantes (`auto_pull.rs:1200-1215`) et à
  `mika2131_filter_names_are_a_wire_format` (l.5996). Pas de réutilisation de
  `in_flight_self_dev` : cette population *est* le symptôme #2279, et son compte est la
  mesure directe de « le défaut se reproduit-il encore ? ». Les confondre cacherait un
  blocage permanent dans un transitoire — le mot de la doctrine mika#2131.

**B reste nécessaire même après A**, et pas comme redondance : l'état « parent terminal +
pilote vif » a d'autres producteurs légitimes. Le plus net est écrit dans le code
lui-même — `tracking_cleanup.rs:289`, quand le kill de supersession **n'aboutit pas**
(EPERM, survie au SIGKILL), la row et son pgid sont laissés intacts *délibérément* pour
qu'un reaper puisse encore atteindre le processus. B est le pendant de ce cas. S'y ajoutent
`mika tasks cancel --yes` et la branche grooming de `tools/create_task.rs:196`.

## Implémentation, par fichier

| Fichier | Changement |
|---|---|
| `crates/mika-agent/src/live_pilot.rs` | **Nouveau.** `LivePilotVerdict`, `live_pilot_for_issue`. Lecteur unique. |
| `crates/mika-agent/src/lib.rs` | `pub mod live_pilot;` |
| `crates/mika-agent/src/db.rs` | `find_dispatch_children_for_issue_url` — la jointure enfant→parent sans prédicat sur le statut du parent. |
| `crates/mika-agent/src/async_db.rs` | Wrapper async. |
| `crates/mika-agent/src/server/ready_label_handler.rs` | Porte 2c + `format_pilot_in_flight_pre_digest` + audit `ready_label_pilot_in_flight`. |
| `crates/mika-agent/src/auto_pull.rs` | `FILTER_LIVE_PILOT`, Filter 4b Phase 2, entrée dans le test de vocabulaire. |
| `CLAUDE.md` (racine) + `crates/mika-agent/CLAUDE.md` | Portes du handler, filtres d'exclusion, signaux opérateur. |

Aucune migration : aucune colonne, aucune table. Le correctif ne lit que ce qui est déjà
écrit — c'est le point du diagnostic.

## Contrat de vérification

Tests eval, dans `crates/mika-agent/tests/eval/`, sur le modèle de
`test_ready_label_blocked_skip.rs` (fetcher injecté) et de `test_supersede_kills_live_pilot.rs`
(processus réel, chef de son groupe).

**`test_ready_label_live_pilot_noop_2279.rs`** — un vrai processus vivant, la topologie à
deux rows de production (parent `manual` porteur de l'URL sans pid, enfant `callback`
porteur du pgid sans URL) :

1. **Positif.** Pilote vif → l'événement `ready` ne crée **aucune** tâche (`count == 1`,
   la seule pré-existante), le processus est **toujours vivant** après l'appel, aucune row
   n'a été superséded, retour `Handled` et le pre-digest **ne matche pas**
   `READY_LABEL_DISPATCH_MARKER`.
2. **Le parent annulé ne masque pas le pilote.** Même fixture, parent `cancelled` — c'est
   l'état exact mesuré le 2026-09-10, et le seul terme qui distingue ce test du précédent.
3. **Négatif — pilote mort.** L'enfant porte le pgid d'un processus récolté : le dispatch
   procède comme avant (une tâche créée). La garde ne gèle pas un ticket sur une row
   orpheline.
4. **Négatif — `process_start_time` absent.** `Unreadable` → le dispatch procède. Le
   fail-safe est attesté, pas supposé.
5. **Négatif — autre ticket.** Un pilote vif sur #9999 ne refuse pas #2276. Le prédicat est
   bien porté par l'URL.

Quatre négatifs et non un seul : le prédicat est une conjonction, et neutraliser tous ses
termes d'un coup laisserait passer une implémentation qui n'en lit qu'un — la leçon que
mika#2277 a dû écrire sur le reaper.

**`test_auto_pull_live_pilot_filter_2279.rs`** — Phase 2 : (a) parent annulé + pilote vif →
le ticket est **skippé**, `redrive_count` **inchangé** (le point mika#2158), une ligne
d'exclusion `live_pilot_orphaned_parent` écrite ; (b) pilote mort → re-drive nominal.

**Unités** : `live_pilot_for_issue` sur les trois verdicts ; la requête SQL sur le parent
`cancelled`, `failed`, `delivered` (tous doivent rendre l'enfant) et sur un enfant terminal
(qui ne doit pas être rendu).

**Rouge-avant (porte #2264).** Recette d'injection, à écrire en tête de chaque fichier :
retirer la porte 2c de `try_handle_ready_label_dispatch_with_fetcher` → le test 1 échoue
sur `count == 2` et sur la mort du processus ; retirer le bloc Filter 4b → le test
auto_pull échoue sur le re-drive.

**Non-régression** : `test_supersede_kills_live_pilot.rs` doit rester vert **sans
modification**. Il appelle `supersede_prior_tracking_rows` directement et n'emprunte pas
le handler — la garde ne le croise pas, et c'est la vérification que mika#2335 n'est pas
défait au passage.

## Surfaces opérateur

- **Journal** (`$MIKA_SPIRIT_LOG_FILE`) : `ready_label_pilot_in_flight` (INFO, champs
  `repo`, `num`, `child_task_id`, `pid`) — un événement `ready` rendu inerte par un pilote
  vif. En régime nominal, **quelques lignes par dispatch au plus**. Un flot soutenu sur un
  même ticket signifie que quelque chose continue de battre le label et que la garde B
  n'attrape pas ce producteur : c'est cette ligne qu'il faut suivre, pas le seuil qu'il
  faut ajuster.
- **SQL** : `SELECT target_key, count(*) FROM audit_events WHERE tool_name =
  'ready_label_pilot_in_flight' GROUP BY 1 ORDER BY 2 DESC;` — répond directement à
  « combien de fois ce ticket a-t-il été re-déclenché pendant son propre dispatch ? ».
- **SQL** : `SELECT count(*) FROM audit_events WHERE tool_name = 'auto_pull_exclusion'
  AND after_value = 'live_pilot_orphaned_parent';` — le compte de la population « parent
  terminal, pilote vif ». **Elle doit tendre vers zéro une fois A déployée** ; une valeur
  qui ne décroît pas nomme un producteur de parents orphelins qu'il faut traiter à sa
  source, pas filtrer plus fort.

**Sonde post-déploiement, 48 h.** Sur un ticket ayant reçu un dispatch : zéro couple
`manual ready-label` + `callback deferred` en doublon, et zéro cycle
`labeled → unlabeled → labeled` par `mika-platform-bot` pendant qu'un pilote tourne. Si le
battement persiste alors que `ready_label_pilot_in_flight` est vide : **halte**, ne pas
élargir la garde — c'est qu'un troisième producteur repose le label, et il a son propre
ticket.

## Hors périmètre, délibérément

- **`tools/create_task.rs:196`**, l'autre appelant du supersede (voie LLM). Le chemin
  mesuré seize fois est le webhook ; la voie LLM est un appel explicite, dont la légitimité
  ne se tranche pas avec le même prédicat. **Ticket de suivi** si la mesure le demande.
- **Une garde dans `supersede_prior_tracking_rows` lui-même.** Elle annulerait mika#2335 :
  le contrat de la supersession *est* de disposer du pilote qu'un dispatch neuf remplace.
  La question « ce dispatch est-il neuf ? » appartient au déclencheur.
- **La cause du battement côté `auto_pull`** au-delà de ce filtre — le seuil, le budget,
  la cadence. B empêche le re-drive sur un ticket en vol ; il ne retouche aucun réglage.
- **Les rows déjà en base** (parent annulé + pilote vif au moment du déploiement). B les
  couvre par construction, il n'y a rien à migrer.
- **Le ticket #2276 lui-même**, s'il porte `operator-review` à la suite d'un budget épuisé
  par cette boucle : la ré-entrée est le geste opérateur documenté (retirer
  `operator-review`), pas une exception dans le code.

## Risques

| Risque | Portée | Ce qui le borne |
|---|---|---|
| Faux `Alive` → ticket gelé | Un ticket | Le gel est borné : watchdog #959, reaper #2249/#2277, sweep #1712 rendent la row terminale et le prédicat redevient faux seul. |
| `Unreadable` → #2279 se reproduit | Les enfants sans `process_start_time` | Population déjà comptée par mika#2335 (`unusable_child_count`) ; nommée plutôt que masquée. |
| Un opérateur veut vraiment relancer | Geste manuel | `mika tasks cancel <id>` (avertit sur pilote vif depuis #2335) puis re-poser `ready` — écrit dans le pre-digest du refus. |
| Coût de la sonde `/proc` en Phase 2 | Un tick | Sondée seulement quand `in_flight` est faux ; population normalement vide. |

## Definition of Done

- [ ] `live_pilot.rs` livré, lecteur unique du prédicat, trois verdicts, fail-safe sur
      `Unreadable`.
- [ ] `find_dispatch_children_for_issue_url` livrée, sans prédicat sur le statut du parent,
      avec le garde `json_valid`.
- [ ] Porte 2c dans `ready_label_handler`, avant l'étape 3, refus en `Handled`, zéro tâche,
      zéro kill, zéro différé.
- [ ] Filtre 4b dans `auto_pull` Phase 2, `Skip` sans remise à zéro du budget, sous son
      propre nom de filtre.
- [ ] `FILTER_LIVE_PILOT` ajouté à `mika2131_filter_names_are_a_wire_format`.
- [ ] Les deux fichiers de test eval livrés, avec leur recette d'injection en tête.
- [ ] `test_supersede_kills_live_pilot.rs` vert **sans modification**.
- [ ] `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check` verts.
- [ ] `CLAUDE.md` racine et `crates/mika-agent/CLAUDE.md` mis à jour (portes, filtres,
      signaux opérateur) ; `docs-sync` vert.

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria`. Les critères ci-dessous sont
dérivés de son corps (défauts (a) et (b), reproduction) et de ses deux commentaires.

1. **AC1 — un `labeled` répété sur un ticket en vol est un NO-OP.** Un second événement
   `labeled ready` reçu pendant qu'un pilote vif existe pour ce ticket ne crée aucune
   tâche, ne supersède aucune row, ne tue aucun processus et n'enregistre aucun dispatch
   différé. Attesté par `test_ready_label_live_pilot_noop_2279` §1.
2. **AC2 — un parent annulé ne masque pas le pilote.** Le prédicat de vivacité traverse
   parent → enfant sans regarder le statut du parent : `cancelled`, `failed` et `delivered`
   rendent tous l'enfant vif. Attesté par §2 et les unités SQL.
3. **AC3 — la readiness check ne produit plus de différé en doublon sur le même ticket.**
   Le chemin qui menait à `deferred_dispatch_registered` (étape 9d) n'est pas atteint pour
   un ticket en vol, parce que la porte refuse avant l'étape 7. Attesté par le compte de
   tâches de §1 (aucune row `:deferred` créée).
4. **AC4 — la boucle de re-label est cassée.** `auto_pull` Phase 2 n'émet pas de re-drive
   sur un ticket dont le pilote est vif, et ne consomme pas son budget de re-drive.
   Attesté par `test_auto_pull_live_pilot_filter_2279` (a).
5. **AC5 — le fail-safe est dans le sens de la maison.** Un pilote mort, un
   `process_start_time` illisible ou une erreur DB ne bloquent pas le dispatch : le
   comportement d'avant le correctif est repris. Attesté par §3, §4.
6. **AC6 — le refus est observable et sa levée est nommée.** Chaque no-op écrit une ligne
   INFO `ready_label_pilot_in_flight` et une row `audit_events` du même nom ; le pre-digest
   nomme le pilote, son pid et le geste opérateur pour forcer un re-dispatch.
7. **AC7 — mika#2335 n'est pas défait.** La supersession tue toujours le pilote qu'un
   dispatch **neuf** remplace. Attesté par `test_supersede_kills_live_pilot.rs`, vert sans
   modification.
8. **AC8 — le vocabulaire des filtres ne fourche pas.** La population « parent terminal,
   pilote vif » est comptable séparément de `in_flight_self_dev`, sous un nom épinglé par
   le test de format de fil.
