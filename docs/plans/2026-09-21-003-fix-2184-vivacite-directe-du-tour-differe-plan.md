# mika#2184 — La vivacité d'un tour différé se mesure sur son activité, pas sur l'âge de sa promotion

**Ticket :** mika issue#2184
**Type :** fix (substrat moteur — faucheur `stuck_pending`)
**Date :** 2026-09-21

---

## Problème

mika#2181 a élargi la clause (1) de `find_orphaned_pending_issue_tasks`
(`crates/mika-agent/src/db/tasks.rs:2478`) : un wrapper différé **promu** compte
comme vivant tant que son `completed_at` tient dans
`PROMOTED_WRAPPER_LIVENESS_DEFAULT_SECS` (2700 s). Correct dans son domaine, et
la trace fondatrice est fermée.

Le résidu est mesuré dans le corps du ticket et re-cité par le doc de solution
que mika#2181 a lui-même livré
(`docs/solutions/best-practices/une-fenetre-bornee-sur-un-proxy-est-une-dette-datee-2026-09-05.md`,
§3) : sur 799 wrappers livrés en 30 j, **139 (17 %)** livrent au-delà de 2700 s,
**81** de leurs parentes ont été expirées `stuck_pending_no_deferred_wrapper`, et
**8** l'ont été 2820–4996 s après promotion — hors de portée de **toute** valeur
de la constante compatible avec un faucheur utile.

Le ticket nomme la direction : une mesure **directe** de vivacité, sur la forme
que mika#1652 emploie déjà pour les team runs (`find_stuck_team_runs`,
`crates/mika-agent/src/db.rs:4137`) — la **récence des lignes d'activité**
(`llm_calls` / `tool_calls`), pas la récence d'une écriture de statut.

### M1 — Ce que la lecture du code confirme : la jointure existe et elle est exacte

Le tour qui consomme un wrapper promu est un tour silencieux
`SilentTrigger::DeferredDispatch`, dispatché par `dispatch_resume_agent`
(`task_engine/dispatcher.rs:828`). Ce tour ouvre une session
`deferred-dispatch-<uuid>` via
`create_session_with_parent(&session_id, …, task_id = Some(&task.id))`
(`dispatcher.rs:916`, `db.rs:1955`) — où `task.id` est **le wrapper**. La colonne
`sessions.task_id` existe depuis v19 et est écrite là.

La chaîne est donc **jointe par égalité**, pas par préfixe :

```
parente P  →  wrappers W (trigger_type='callback', parent_task_id = P.id,
                          label = DEFERRED_DISPATCH_LABEL)
           →  sessions S (sessions.task_id = W.id)
           →  llm_calls / tool_calls (session_id = S.id)
```

C'est plus précis que mika#1652, qui doit se rabattre sur
`session_id LIKE 'team-' || r.id || '%'`. Aucune colonne, aucune migration :
la v19 a déjà posé le chaînon.

### M2 — Ce que la lecture du code **déplace** : les trois causes du retard n'ont pas la même mesure

Le ticket écrit que « le tour silencieux qui consomme un wrapper promu produit
ces mêmes lignes ». Vrai — **quand le tour tourne**. Un wrapper `completed` non
`delivered` au-delà de 2700 s a trois causes possibles, et elles ne sont pas
couvertes par le même prédicat :

| # | cause | activité sur la session du wrapper ? |
|---|---|---|
| A | le tour tourne et est lent (modèle lent, gros brief) | **oui** — c'est le cas AC1 |
| B | `AgentBusy` : le lock d'agent est pris ailleurs | **non** — `dispatch_resume_agent` rend `Err(AgentBusy)` à la ligne 903, **avant** `create_session_with_parent` ligne 916 : il n'y a pas de session |
| C | redémarrage du service pendant l'attente | **non** — rien ne tournait, et le faucheur non plus |

Le corps du ticket cite le plan de mika#2181 (« des redémarrages de serveur et
des retards `AgentBusy` ») et retourne correctement son argument. Mais la
conséquence n'est pas celle que le ticket tire : **si la traîne est faite de B et
de C, une mesure d'activité ne la couvre pas**, parce qu'il n'y a rien à mesurer.

Ce plan traite les trois, par trois moyens distincts et pour trois raisons
distinctes :

- **A** — c'est la mesure directe qu'AC1 demande. Livrée (U2, U3).
- **C** — traitée **non** par une mesure mais par l'aveu que la mesure n'a pas
  eu lieu : un process dont l'uptime est inférieur à la fenêtre n'a **pas pu**
  observer la fenêtre, donc « zéro ligne » n'y est pas un silence, c'est une
  indisponibilité. C'est la tri-valeur `LivenessSignal` que mika#2277 a déjà
  posée dans ce fichier même (`task_engine/engine.rs:363`) : *un signal qu'on ne
  peut pas lire n'est jamais un terme satisfait*. Livrée (U2, variante
  `NotYetObservable`).
- **B** — **non couvert, et le refus est motivé** (D5). Le seul signal disponible
  serait « l'agent a de l'activité ailleurs », qui est vrai presque en
  permanence sur mika-dev et désarmerait le faucheur. Suivi nommé.

**Conséquence pour AC2, et il faut la dire avant d'écrire un test vert :** AC2
suppose que les 8 cas portent de l'activité. Rien dans le ticket ne l'établit,
et la caractérisation **n'est pas exécutable depuis le bac à sable de dispatch**
(`~/.mika/data/mika.db` n'existe pas dans le bwrap du pilote). Ce plan livre donc
AC2 en deux moitiés : un rejeu sur **fixtures de forme** (T2/T3, reproduisant la
géométrie mesurée : promotion à 2820 s et à 4996 s, parente `pending` au-delà de
la grâce, activité récente) **plus** la requête de caractérisation que
l'opérateur exécute sur la base réelle, avec sa halte (§ Sondes, sonde 1). Un
test vert sur une fixture dont on n'a pas établi qu'elle décrit les 8 cas serait
précisément le « rouge vacuux » que le doc de mika#2181 condamne (§ *Le test qui
n'a jamais été rouge ne prouve rien*).

### M3 — Le frère de classe du commentaire 1 : deux fixes, et la raison est structurelle

Le commentaire du 2026-09-07 demande de trancher : un seul fix partagé entre
`stuck_pending` et le phantom-sweep, ou deux ?

**Deux**, et ce n'est pas une préférence de périmètre : **la mesure directe de ce
ticket ne peut pas couvrir la population du phantom-sweep.** Le commentaire le
dit lui-même — « une file-en-attente n'a pas encore d'enfant → aucune vivacité à
constater ». Une tracking row NULL-PID `action_type='none'` en file derrière une
fente occupée ne produit **aucune** ligne `llm_calls` ni `tool_calls` : elle
n'a pas démarré. Mesurer l'activité l'épargnerait exactement zéro fois.

Le discriminant dont elle a besoin est différent, et le code du phantom-sweep le
nomme déjà, par écrit, dans le doc-comment de `dispatch_liveness`
(`task_engine/engine.rs:2472-2476`) :

> *« a tracking row still waiting for a dispatch slot has no PID-carrying child
> at all — only a deferred wrapper — so this guard cannot see it. That window is
> covered by the grace threshold instead […], which is a weaker instrument ».*

Le remède y est donc « consulter le wrapper différé », c'est-à-dire
`has_live_deferred_wrapper_child` — un prédicat qui **existe déjà** (mika#2181)
et qui n'a rien à voir avec l'activité. Population différente, discriminant
différent, rayon d'explosion différent (le phantom-sweep touche toutes les
tracking rows de tous les agents). **Suivi nommé**, avec sa précondition : le
défaut mesuré date du 2026-09-07 et mika#2156 a depuis porté la grâce à 14400 s
(4 h), donc la première chose à établir est si la classe se reproduit encore.

---

## Requirements

- **R1** — Le faucheur `stuck_pending` épargne une parente dont un wrapper
  différé porte une activité (`llm_calls` ou `tool_calls`) plus récente qu'un
  seuil nommé, **indépendamment** de l'âge de `completed_at`.
- **R2** — L'épargne est **visible et attribuable** : elle ne vit pas dans un
  `NOT EXISTS` SQL. Chaque épargne écrit une ligne de journal et une ligne
  `audit_events`.
- **R3** — Les deux causes d'épargne restent **comptables séparément** :
  fenêtre-proxy (mika#2181) et mesure directe (ce ticket).
- **R4** — La fenêtre-proxy reste en place, **sous** la mesure directe : elle
  filtre en premier, dans la SQL, inchangée.
- **R5** — Une parente dont le tour est réellement mort est toujours re-armée
  puis expirée — comportement d'aujourd'hui, bit pour bit.
- **R6** — Un signal qu'on n'a **pas pu** observer n'est pas un silence. Mais
  une indisponibilité **permanente** n'épargne pas : ce qui ne s'éteint pas tout
  seul ne peut pas désarmer un faucheur.
- **R7** — Un seul lecteur de la question « ce wrapper montre-t-il de l'activité
  ? », tenu par un scan de source.
- **R8** — Aucune migration, aucune colonne, aucune valeur de réglage existante
  déplacée.

---

## Décisions

### D1 — Le filtre est côté application, pas une troisième clause `NOT EXISTS`

Le réflexe serait d'ajouter une clause à `find_orphaned_pending_issue_tasks`.
**Refusé**, et le refus est écrit dans le doc que mika#2181 a livré (§2) :

> *« L'épargne vit dans un `NOT EXISTS` SQL : une parente épargnée ne devient
> jamais candidate et ne traverse jamais l'application. Aucun log, aucun audit,
> aucun compteur […] indiscernable d'un régime sain. »*

C'est le défaut que mika#2181 a dû réparer après coup en ajoutant
`find_parents_sheltered_by_promoted_wrapper`. Le reproduire une deuxième fois
dans la même fonction serait re-creuser le trou qu'on vient de combler. La SQL
rend les candidats (fenêtre-proxy comprise, R4) ; la mesure directe filtre
ensuite, en Rust, où elle peut se journaliser (R2).

Effet de bord utile : **le seuil ne rentre plus dans un `strftime`**, donc le
corollaire « une borne doit échouer fermé sur son propre bouton » (doc mika#2181
§1) ne s'applique pas au prédicat. Le plafond est posé quand même (D4), pour que
le bouton fasse ce qu'il annonce.

### D2 — La DB rend un **âge**, pas un booléen

`find_deferred_wrapper_activity_age_secs(agent_id, parent_task_id) ->
Result<Option<i64>>` : l'âge en secondes de la ligne d'activité la plus récente
sur les sessions attachées aux wrappers différés de cette parente ; `None` quand
il n'y en a aucune.

Trois raisons, dans l'ordre de poids :

1. **Le seuil sort de la SQL** et devient un paramètre d'une fonction pure →
   testable aux valeurs limites sans base.
2. **La journalisation peut nommer l'âge.** mika#2277 a payé très cher de ne
   rapporter qu'un seul âge sur une disposition qui en croisait trois : les deux
   faux positifs du 2026-09-10 se lisaient « nominaux » à la première
   inspection. Une épargne dont on ne peut pas lire l'âge se relit exactement
   comme une épargne qui n'avait pas lieu d'être.
3. `None` est **jamais `0`** (doctrine mika#2331) : aucune activité n'est pas
   une activité d'âge zéro.

### D3 — Quatre états, et la disposition suit la **bornitude**, pas la certitude

Fonction pure, signature complète, aucun état global lu à l'intérieur (patron
5d/mika#2290 et mika#2277 : le paramètre plutôt que le `if` côté appelant, pour
que la règle porte son propre test) :

```rust
fn classify_wrapper_activity(
    last_activity_age_secs: Option<i64>,
    window_secs: i64,
    engine_uptime_secs: i64,
    telemetry_armed: bool,
) -> WrapperActivity
```

```rust
enum WrapperActivity {
    /// Au moins une ligne dans la fenêtre — le tour travaille.
    Active { last_seen_secs: i64 },
    /// Télémétrie armée, fenêtre entièrement vécue, zéro ligne.
    Silent,
    /// Le process n'a pas vécu la fenêtre : on n'a pas pu observer.
    NotYetObservable { uptime_secs: i64 },
    /// `store_llm_calls` ET `store_tool_calls` sont désarmés.
    NotRecorded,
}
```

Dispositions — `match` exhaustif, **aucun bras `_ =>`** (modèle
`hosting_ground_truth_line`, mika#2290) :

| état | disposition | pourquoi |
|---|---|---|
| `Active` | **épargne** | R1 |
| `NotYetObservable` | **épargne** | l'ignorance est **bornée** : elle s'éteint seule dès que l'uptime dépasse la fenêtre. Couvre la cause C de M2 |
| `Silent` | fauche | R5 — comportement d'aujourd'hui |
| `NotRecorded` | fauche + WARN | l'ignorance est **permanente** : épargner ici restaurerait le cadavre-bouclier que mika#2181 a dû borner (§1 du doc). *Ce qui ne peut pas s'éteindre tout seul ne peut pas épargner* |

`NotYetObservable` et `NotRecorded` sont deux variantes et non un
`Unobservable { reason }`, précisément parce que leur disposition diffère : une
raison qui décide n'est pas une raison, c'est un état.

`telemetry_armed = settings.store_llm_calls || settings.store_tool_calls` — un
réglage **lu**, jamais inféré d'une absence de lignes. Distinguer « la télémétrie
est coupée » de « l'agent n'a rien fait » est impossible par observation, et
c'est exactement la confusion que mika#2277 condamne.

### D4 — La fenêtre : 600 s, nommée, avec ses trois paliers et son plafond

`STUCK_PENDING_ACTIVITY_WINDOW_DEFAULT_SECS: i64 = 600`, surchargeable par
`MIKA_STUCK_PENDING_ACTIVITY_WINDOW_SECS`.

600 s = 2× l'enveloppe de tour par défaut (`AGENT_TOTAL_TIMEOUT` 300 s,
mika#2189). Un tour qui travaille écrit une ligne `llm_calls` par appel, et deux
appels consécutifs sont séparés au plus du plafond par appel (120 s par défaut)
plus le traitement. 600 s couvre donc un tour entier **et** l'intervalle jusqu'au
suivant, avec un facteur 2 de marge. Repère voisin : mika#1652 emploie 300 s pour
les team runs ; on est délibérément deux fois plus généreux, parce que l'erreur
coûteuse ici est de tuer un tour vivant.

Trois paliers (forme maison) : absent/vide → défaut ; illisible, `0`, négatif, ou
au-delà de `STUCK_PENDING_ACTIVITY_WINDOW_MAX_SECS` (30 j) → défaut **avec un
WARN nommant la valeur entre guillemets**. Le plafond n'est pas ici une garde
anti-`strftime` (D1 l'a retiré de la SQL) : c'est la cohérence du bouton. Un
réglage démesuré désarmerait le faucheur pour toujours, ce que D3 refuse déjà
par ailleurs — mieux vaut que le bouton le refuse aussi, en le disant.

### D5 — La cause `AgentBusy` (B de M2) n'est **pas** couverte, et le refus est motivé

Deux voies étaient possibles, toutes deux refusées :

1. **« L'agent a-t-il eu de l'activité, toutes sessions confondues ? »** — vrai
   presque en permanence sur mika-dev (webhooks, callbacks, heartbeat). Le
   prédicat serait satisfait quasi toujours, l'épargne deviendrait la règle, et
   le faucheur serait désarmé sous couvert de précision. C'est la forme la plus
   dangereuse : une garde qui ne refuse plus rien tout en produisant des lignes
   d'épargne rassurantes.
2. **« Le wrapper est-il encore livrable ? »** — `get_undelivered_callback_tasks`
   scanne 7 jours en arrière (`engine.rs:934`), donc un wrapper `completed`
   non-`delivered` y est **toujours**, modulo `next_fire_at`. Le prédicat serait
   vrai par construction : c'est la définition même du cadavre-bouclier.

Le signal qui manquerait réellement est une **tentative de livraison estampée** :
mika#2179 écrit `delivery_attempts` / `delivery_last_error_class` sur l'échec de
`run_silent_agent`, mais `AgentBusy` retourne **avant** (`dispatcher.rs:903`) et
n'estampe rien. Poser cet estampage est un élargissement dans mika#2179 :
**suivi nommé**, avec sa précondition (sonde 1).

### D6 — Deux noms d'événements, et c'est une rectification assumée de la lettre d'AC4

AC4 écrit : *« `stuck_pending_sheltered_by_promoted_wrapper` distingue les deux
causes d'épargne »*. La lecture littérale serait un champ `cause` sur cet
événement. **Refusé** : son nom *porte déjà sa cause*. Y faire passer une épargne
qui n'a rien à voir avec un wrapper promu rendrait le nom faux, et couperait en
deux la population que la sonde de mika#2181 compte pour mesurer si sa dette se
retire.

La maison a un moyen établi pour ça, et elle l'a employé trois fois :
`phantom_aged_out` / `phantom_sweep_spared` (mika#2156),
`qa_deadline_verdict` / `qa_callback_verdict` (mika#2368),
`auto_pull_no_token` / `wip_rescue_no_token` (mika#2205). Deux populations qui
doivent rester comptables séparément ont deux noms.

Donc : `stuck_pending_sheltered_by_promoted_wrapper` **inchangé** (SOLE WRITER,
sa décroissance vers zéro est la mesure que la dette se retire) et
`stuck_pending_sheltered_by_activity` (SOLE WRITER, la mesure qui la remplace).
L'**intention** d'AC4 — les deux causes sont distinguables — est tenue ; sa
lettre est rectifiée, ici, avec sa raison.

### D7 — Un lecteur unique, tenu par un scan de source

R7. La question « ce wrapper montre-t-il de l'activité ? » a **un** site :
`Database::find_deferred_wrapper_activity_age_secs`. C'est la leçon
`grooming_marker` (mika#2158), qui a coûté des mois de désaccord silencieux entre
deux prédicats copiés — et que ce fichier a déjà payée une seconde fois avec
`has_pending_deferred_wrapper_child` (doc mika#2181, § *Deux prédicats nommés
comme équivalents*).

**Délibérément non unifié avec mika#1652** (`find_stuck_team_runs`) : la
population est disjointe (team runs vs. wrappers différés) et la jointure est
différente (`session_id LIKE 'team-…%'` vs. `sessions.task_id =`). Une
abstraction dessinée sur deux points dont les jointures diffèrent est la mauvaise
abstraction ; ce qui est partagé est le *patron*, pas le code, et c'est déjà
écrit dans les deux doc-comments.

---

## Scope Boundaries

**Dans le périmètre :** le faucheur `stuck_pending`
(`TaskEngine::reap_orphaned_pending_issue_tasks`), sa mesure d'activité, ses
deux surfaces d'épargne, sa fenêtre et son bouton.

**Hors périmètre, délibérément :**

- Le prédicat de mika#2181 lui-même — mergé, correct dans son domaine, et R4 le
  garde tel quel (le corps du ticket l'exclut).
- Le TOCTOU du faucheur — ticket séparé (le corps l'exclut).
- Le **phantom-sweep** (commentaire 1) — M3 : discriminant différent, population
  différente. Suivi nommé.
- La cause `AgentBusy` — D5. Suivi nommé.
- `MAX_STUCK_REARMS`, `STUCK_PENDING_REAPER_GRACE_DEFAULT_SECS`,
  `PROMOTED_WRAPPER_LIVENESS_DEFAULT_SECS` : **aucune valeur ne bouge**. Ce
  travail ajoute un terme d'épargne, il n'en règle aucun.
- Le faucheur L3b `stale_blocked` et le `childless_parent_reaper` : autres
  populations, non touchées.

---

## Implementation Units

### U1 — Le lecteur DB (un seul site, R7/D2/D7)

`crates/mika-agent/src/db/tasks.rs` — à côté de `find_orphaned_pending_issue_tasks` :

```rust
pub fn find_deferred_wrapper_activity_age_secs(
    &self,
    agent_id: &str,
    parent_task_id: &str,
) -> Result<Option<i64>>
```

Rend `MIN(âge)` sur l'union des deux tables, restreint aux sessions attachées aux
wrappers différés de cette parente :

```sql
SELECT MIN(age) FROM (
  SELECT CAST(strftime('%s','now') - strftime('%s', lc.created_at) AS INTEGER) AS age
    FROM tasks w
    JOIN sessions s ON s.task_id = w.id
    JOIN llm_calls lc ON lc.session_id = s.id
   WHERE w.agent_id = ?1 AND w.parent_task_id = ?2
     AND w.trigger_type = 'callback' AND w.label = ?3
  UNION ALL
  SELECT CAST(strftime('%s','now') - strftime('%s', tc.created_at) AS INTEGER)
    FROM tasks w
    JOIN sessions s ON s.task_id = w.id
    JOIN tool_calls tc ON tc.session_id = s.id
   WHERE w.agent_id = ?1 AND w.parent_task_id = ?2
     AND w.trigger_type = 'callback' AND w.label = ?3
)
```

Aucun seuil dans la SQL (D1/D2). Index existants : `idx_llm_calls_session`,
`idx_tool_calls_session`. La requête ne tourne que sur les **candidats** du
faucheur — population normalement vide.

Un âge **négatif** (horloge décalée, ligne dans le futur) est ramené à `0` : il
signifie « très récent », jamais « très vieux ». Fail-safe vers l'épargne, dans
le sens de l'asymétrie du ticket.

Wrapper async dans `async_db.rs`, agent-scopé, à côté de
`find_live_deferred_wrapper_child`.

### U2 — La fonction pure et son enum (D3)

`crates/mika-agent/src/task_engine/engine.rs`, à côté de `LivenessSignal`
(ligne 363) dont elle est le frère. `WrapperActivity` + `classify_wrapper_activity`
(signature en D3). Aucune lecture d'état global : les quatre entrées sont des
paramètres.

### U3 — Le branchement dans le faucheur (R1/R2/R4)

`reap_orphaned_pending_issue_tasks` (`engine.rs:1010`), dans la boucle
`for candidate in candidates`, **avant** la lecture de `wrappers_seen` (ligne
1083) — l'épargne ne doit payer ni l'inventaire ni la reconstruction de
l'`action_config` :

```
Active | NotYetObservable  →  record_activity_spare(...) ; continue
Silent                     →  poursuivre (comportement d'aujourd'hui)
NotRecorded                →  warn_once ; poursuivre
```

`engine_uptime_secs` : nouveau champ `started_at: std::time::Instant` sur
`TaskEngine`, posé dans `new()`. Il est **passé** à la fonction pure, jamais lu
dedans — sans quoi les tests, qui construisent un moteur neuf, verraient un
uptime nul et épargneraient tout (T5 serait vert pour une mauvaise raison).

`telemetry_armed` : lu sur `self.dispatcher.settings` (déjà accessible,
cf. `engine.rs:2626`).

### U4 — Les surfaces (R2/R3/D6)

- `stuck_pending_sheltered_by_activity` — INFO + `audit_events`
  (`tool_name = "stuck_pending_sheltered_by_activity"`, `target_key =
  "task:<id>"`, `before_value = after_value = "pending"` — le propre de la ligne
  est que rien n'a bougé, forme `record_phantom_spare`). Champs : `task_id`,
  `issue`, `age_seconds` (de la parente), `last_activity_secs`,
  `activity_window_secs`, `cause` ∈ `{"active", "not_yet_observable"}`.
  **SOLE WRITER.**
- `stuck_pending_activity_not_recorded` — WARN, **une fois par process**
  (`std::sync::Once` ou `AtomicBool` sur `TaskEngine`), nommant les deux réglages.
  Régime attendu : **zéro ligne**.
- `stuck_pending_activity_window_invalid` — WARN du palier de parse (D4).
- `stuck_pending_sheltered_by_promoted_wrapper` — **inchangé**, non touché.

L'écriture d'audit est fire-and-forget : perdre la ligne coûte de la visibilité,
jamais l'épargne (forme `record_phantom_spare`, `engine.rs:2574`).

### U5 — Le scan structurel (R7/D7)

`db::tasks::tests::mika2184_wrapper_activity_has_a_single_reader` — refuse, hors
du corps de `find_deferred_wrapper_activity_age_secs`, tout site de production
joignant `sessions` à `llm_calls`/`tool_calls` par `sessions.task_id`.
`is_test_source_path` pour la classification (mika#2321). **Allowlist livrée
vide** — quand il tire, on retire le second site, on ne l'allowliste pas.

Le needle est ancré sur la jointure `s.task_id` afin de **ne pas** attraper
mika#1652, dont la jointure est un `LIKE` sur `session_id` : un scan qui
accuserait le voisin serait un scan qu'on désarme.

### U6 — Documentation

- `crates/mika-agent/CLAUDE.md` § *Unified Task Engine* : le paragraphe « What
  the stuck-pending reaper makes of a promoted wrapper (mika#2181) » gagne la
  mesure directe, les quatre états, et la phrase de D3 sur la bornitude.
- `CLAUDE.md` racine : entrée `MIKA_STUCK_PENDING_ACTIVITY_WINDOW_SECS` à côté
  de `MIKA_PROMOTED_WRAPPER_LIVENESS_SECS`, avec les deux noms d'événements, les
  requêtes SQL et les haltes des sondes.
- `docs/solutions/best-practices/une-fenetre-bornee-sur-un-proxy-est-une-dette-datee-2026-09-05.md`
  § 3 : la dette nommée est **acquittée pour la cause A** ; les causes B et C
  sont nommées avec leur statut. Le doc prescrit lui-même de nommer la mesure qui
  retire la dette — il doit dire quand elle arrive, et ce qu'elle ne couvre pas.

---

## Verification Contract

### Séquence rouge prescrite (doc mika#2181, § *Le test qui n'a jamais été rouge*)

1. Écrire T2 **avant** U3. Il doit être **rouge** sur `main` — la parente est
   expirée — et **seul rouge** parmi la suite. Consigner la sortie rouge dans le
   corps de la PR (AC2 l'exige en toutes lettres).
2. Brancher U3. T2 vert, T5 (non-régression) resté vert.
3. **Muter** pour vérifier que la garde garde : forcer `Active` → `Silent` dans
   le `match` de U3 et exiger que T2 **seul** rougisse.

| # | test | ce qu'il établit |
|---|---|---|
| T1 | `find_deferred_wrapper_activity_age_secs` : `None` sans session, `None` avec session sans ligne, âge correct avec `llm_calls` seul, avec `tool_calls` seul, **minimum** des deux quand les deux existent | U1, et que les deux tables comptent |
| T2 | **Rejeu AC2** : parente `pending` au-delà de la grâce, wrapper promu il y a **2820 s** puis **4996 s** (les deux bornes mesurées), session du wrapper portant une ligne `llm_calls` à −60 s → la parente **survit** ; `main` l'expire | AC1, AC2 |
| T3 | Même forme, activité portée par `tool_calls` seul | la mesure ne dépend pas d'un seul canal |
| T4 | Contrôle négatif de cible : l'activité est sur la session d'un wrapper d'une **autre** parente → la parente candidate est bien expirée | que la jointure discrimine, et non qu'elle épargne tout le monde |
| T5 | **AC3** : wrapper promu il y a 4000 s, aucune activité, uptime > fenêtre, télémétrie armée → re-arm puis, budget épuisé, expiration | R5, non-régression |
| T6 | `classify_wrapper_activity`, table complète : `(Some(60), 600, 10_000, true) → Active` ; `(None, 600, 10_000, true) → Silent` ; `(None, 600, 120, true) → NotYetObservable` ; `(None, 600, 10_000, false) → NotRecorded` ; `(Some(700), 600, 10_000, true) → Silent` (borne haute) ; `(Some(600), 600, …) → Active` (borne incluse) | D3, bornes comprises |
| T7 | **Redémarrage (cause C)** : uptime = 120 s < fenêtre, aucune activité → la parente est **épargnée** ; à uptime = 10 000 s, même entrée → expirée | que `NotYetObservable` épargne **et** s'éteint |
| T8 | **`NotRecorded` fauche** : télémétrie désarmée, aucune activité → la parente est expirée, et le WARN est émis | D3, et que l'ignorance permanente ne désarme pas |
| T9 | **Paramètres inégaux** (doc mika#2181, § angle mort) : un test où `grace_seconds`, `promoted_liveness_seconds` et `activity_window_secs` sont **deux à deux distincts** (p. ex. 2700 / 1800 / 600), et qui rougit **seul** si deux d'entre eux sont transposés | trois `i64` adjacents de même type et même défaut ne doivent pas être interchangeables sans un rouge |
| T10 | Les deux événements d'épargne portent des `tool_name` distincts et chacun a **un** écrivain de production (scan de source) | R3, D6 |
| T11 | U5 : le scan structurel est vert à HEAD, et rougit si l'on duplique la jointure dans un second site (contrôle de bonne foi) | R7 |
| T12 | Palier de parse de la fenêtre : absent → 600 ; `"0"`, `"-1"`, `"abc"`, `"2592001"` → 600 + WARN ; `"900"` → 900 | D4 |

`cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check` verts.

---

## Definition of Done

- [ ] U1–U6 livrés.
- [ ] T1–T12 verts ; sortie **rouge** de T2 sur `main` consignée dans le corps de
      la PR.
- [ ] Mutation `Active → Silent` vérifiée : T2 rougit **seul**.
- [ ] Scan U5 vert à HEAD, allowlist vide.
- [ ] Aucune migration, aucune colonne ajoutée ; `schema_version` inchangée.
- [ ] Aucune valeur de réglage existante déplacée
      (`MAX_STUCK_REARMS`, grâce, fenêtre-proxy).
- [ ] `stuck_pending_sheltered_by_promoted_wrapper` non modifié.
- [ ] Les trois suivis (§ Suivi) sont ouverts ou nommés dans le corps de la PR.

---

## Acceptance criteria

Transcrits verbatim du corps de mika#2184 :

- **AC1** — Le faucheur épargne une parente dont le tour différé est
  *démontrablement actif* (au moins une ligne d'activité plus récente qu'un seuil
  nommé), indépendamment de l'âge de `completed_at`.
- **AC2** — Rejeu anti-vacuité sur les 8 cas identifiés ci-dessus : sur `main` la
  parente est expirée (rouge) ; avec le correctif elle survit (vert). Sortie
  rouge dans la PR.
- **AC3** — Non-régression : une parente dont le tour est réellement mort (aucune
  ligne d'activité depuis le seuil) est toujours re-armée puis expirée.
- **AC4** — La fenêtre-proxy de mika#2181 reste en place comme filet **sous** la
  mesure directe, et `stuck_pending_sheltered_by_promoted_wrapper` distingue les
  deux causes d'épargne.

**Correspondance, et la rectification qu'elle porte :** AC1 ← U1–U3, T2, T6 ;
AC2 ← T2/T3 **plus** la sonde 1 (M2 : les 8 cas ne sont pas caractérisables
depuis le bac à sable, et un rejeu de forme n'établit pas qu'il rejoue *ces*
cas — la halte est écrite) ; AC3 ← T5, T8 ; AC4 ← R4 (la SQL est inchangée, la
fenêtre filtre en premier) et D6 (les deux causes sont distinguées par **deux
noms** plutôt que par un champ sur un nom qui porte déjà sa cause — rectification
de la lettre, intention tenue).

---

## Fire-Disposition

**(a) exception nommée en allowlist — livrée VIDE.**

Ce plan livre des détecteurs : le scan structurel U5, le scan « un écrivain par
événement » (T10), et douze tests dont le chemin de succès est « aucune violation
trouvée ».

**Aucun ne peut firer sur les données existantes, et c'est vérifiable avant
d'écrire une ligne :**

- **U5** — le needle est ancré sur la jointure `sessions.task_id` →
  `llm_calls`/`tool_calls`. À HEAD, le seul site qui joint ces tables est
  `find_stuck_team_runs` (`db.rs:4137`), qui joint par
  `session_id LIKE 'team-' || r.id || '%'` et **ne touche pas `sessions`**. La
  vérification est prescrite comme **première étape** de U5 :
  ```
  grep -rn "sessions" crates/mika-agent/src --include=*.rs | grep -E "task_id.*=.*|llm_calls|tool_calls"
  ```
  Si ce relevé rend un site autre que celui de U1, **halte-et-remontée** : cela
  voudrait dire que la question a déjà un second lecteur, ce qui change le
  périmètre (D7) et n'est pas une exception à poser en allowlist.
- **T10** — les deux `tool_name` sont créés par ce ticket
  (`stuck_pending_sheltered_by_activity`) ou déjà SOLE WRITER
  (`stuck_pending_sheltered_by_promoted_wrapper`, tenu par mika#2181). Zéro
  violation préexistante possible.

L'allowlist des deux scans est donc livrée **vide**, et son vide est asserté :
quand un scan tire, la résolution est de **retirer le second site**, jamais d'y
ajouter une entrée (forme mika#2323, mika#1940, mika#2267).

Aucun détecteur n'est livré désarmé — l'option (b) n'a pas lieu d'être ici,
puisque rien n'est à convertir.

---

## Suivi (hors périmètre, nommé)

1. **Phantom-sweep : la file-en-attente est saine, pas orpheline** (commentaire 1
   de mika#2184). Discriminant candidat : `has_live_deferred_wrapper_child`, que
   le doc-comment de `dispatch_liveness` (`engine.rs:2472-2476`) nomme déjà comme
   le trou de sa garde. **Précondition avant d'ouvrir :** établir que la classe se
   reproduit encore — le défaut mesuré date du 2026-09-07 et mika#2156 a depuis
   porté la grâce à 14400 s (4 h).
   ```sql
   SELECT date(created_at), count(*) FROM audit_events
    WHERE tool_name = 'phantom_aged_out' AND created_at > '2026-09-10'
    GROUP BY 1 ORDER BY 1 DESC;
   ```
   Zéro ligne depuis mika#2156 ⇒ **ne pas ouvrir** : le défaut serait déjà fermé
   par la grâce, et un second discriminant serait une garde sans population.

2. **`AgentBusy` n'estampe rien** (D5). `dispatch_resume_agent` retourne
   `Err(AgentBusy)` avant `record_callback_delivery_failure`, donc une attente de
   lock ne laisse aucune trace sur la row. Élargissement dans mika#2179.
   **Précondition :** la sonde 1 ci-dessous montre que les cas non couverts sont
   majoritairement des `AgentBusy`.

3. **Caractérisation des 8 cas** (M2, AC2). Sonde 1 ci-dessous. Si les 8 ne
   portent pas d'activité, ce ticket ferme la cause A et la sonde nomme laquelle
   de B ou C reste ouverte — avec un compte plutôt qu'avec une intuition.

---

## Sondes post-déploiement, et leurs haltes

### Sonde 1 — Caractérisation (à exécuter **avant** de conclure sur AC2)

Sur `~/.mika/data/mika.db`, pour les parentes expirées
`stuck_pending_no_deferred_wrapper`, mesurer combien portaient de l'activité sur
la session d'un de leurs wrappers différés dans les 600 s précédant l'expiration.

**Halte 1** — si la proportion est **nulle**, la traîne est faite de B et/ou de C
et **le résidu n'est pas fermé par ce ticket**. Ne pas élargir la fenêtre par
réflexe (c'est exactement le geste que le corps du ticket refuse) : lire quelle
cause domine, et ouvrir le suivi 2 ou constater que le garde d'uptime (T7) suffit
pour C.

### Sonde 2 — La mesure mord (48 h)

```bash
grep stuck_pending_sheltered_by_activity "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{task_id, issue, last_activity_secs, cause}'
```
```sql
SELECT count(*) FROM audit_events
 WHERE tool_name = 'stuck_pending_sheltered_by_activity';
```

**Régime attendu : non vide et faible.** Chaque ligne est une parente saine que
le faucheur allait tuer. **Halte 2** — si ce compte porte le trafic nominal
(plusieurs épargnes par heure sur des parentes différentes), la mesure n'épargne
plus, elle désarme : établir *quel* prédicat est trop large avant de régler quoi
que ce soit. *C'est un terme d'épargne, pas un chemin.*

### Sonde 3 — La dette-proxy se retire

```bash
grep stuck_pending_sheltered_by_promoted_wrapper "$MIKA_SPIRIT_LOG_FILE" | wc -l
```

Attendu : **décroissance** relative à la sonde 2 — la mesure directe prend la
place de la fenêtre. Un plateau conjoint des deux dit que les deux protègent des
populations disjointes, ce qui est un **résultat** (à écrire dans le doc de
solution), pas une panne.

### Sonde 4 — Les deux inerties sont muettes

```bash
grep stuck_pending_activity_not_recorded "$MIKA_SPIRIT_LOG_FILE"   # attendu : zéro
grep stuck_pending_activity_window_invalid "$MIKA_SPIRIT_LOG_FILE" # attendu : zéro
```

Toute occurrence de la première est un faucheur qui tourne sans mesure directe,
c'est-à-dire ce ticket rendu inerte par un réglage. **Halte 3** — ne pas toucher
au prédicat : vérifier `store_llm_calls` / `store_tool_calls` d'abord.

### Sonde 5 — Contrôle négatif

`stuck_pending_task_expired` doit **continuer** à apparaître. Zéro expiration sur
une semaine signifie que le faucheur ne fauche plus du tout — la panne que la
sonde 2 cherche, vue par l'autre bout. Un faucheur silencieux se lit exactement
comme un faucheur qui n'a rien à faire (mika#2205).

---

## Ce que ce travail n'achète PAS

- Il ne couvre pas la cause `AgentBusy` (D5), et dit lequel des deux suivis
  l'ouvrira.
- Il ne prouve pas que les 8 cas mesurés sont fermés : il ferme la **classe**
  « le tour tourne et le faucheur ne le voit pas », et la sonde 1 dit ce qu'il
  reste.
- Il ne touche pas le phantom-sweep (M3), et dit pourquoi le même prédicat ne
  peut pas y servir.
- Il ne déplace aucune valeur de réglage. La fenêtre-proxy reste à 2700 s, la
  grâce à 2700 s, `MAX_STUCK_REARMS` à 2.

---

## Références

- mika#2181 — la fenêtre-proxy, sa borne, son compteur d'épargne.
- mika#2045 — le faucheur `stuck_pending` et sa échelle de réparation.
- mika#2413 — `RearmOutcome::AlreadyRepresented`, le `match` exhaustif du
  faucheur, `reset_stuck_rearm_count`.
- mika#1652 — la forme « vivacité = récence des lignes d'activité »
  (`find_stuck_team_runs`).
- mika#2277 — la tri-valeur `LivenessSignal` et la règle « un signal qu'on ne
  peut pas lire n'est jamais un terme satisfait ».
- mika#2156 — `phantom_sweep_spared` : faucher, **et compter** ce qu'on épargne.
- mika#2205 — un scan silencieusement inactif se lit comme un scan oisif.
- mika#2158 — un prédicat copié est un prédicat qui divergera.
- mika#2331 — `null` n'est jamais `0`.
- `docs/solutions/best-practices/une-fenetre-bornee-sur-un-proxy-est-une-dette-datee-2026-09-05.md`
  — les trois conditions d'acquittement de la dette ; ce plan acquitte la
  troisième pour la cause A.
- `docs/solutions/best-practices/1652-team-runs-orphan-reaper-and-tool-error-as-ok-not-err.md`
  — le patron employé, et pourquoi le code n'est pas partagé (D7).

## Revision history

- rev 1 (2026-09-21) : plan initial. Porte deux rectifications du ticket établies
  sur le code — (M2) les trois causes du retard n'ont pas la même mesure, donc
  AC2 est livrée en deux moitiés avec sa halte ; (M3) le frère de classe du
  commentaire 1 ne peut structurellement pas partager ce prédicat, la question
  « un fix ou deux » est tranchée à **deux** avec sa raison. Rectifie aussi la
  lettre d'AC4 (D6) en tenant son intention.
