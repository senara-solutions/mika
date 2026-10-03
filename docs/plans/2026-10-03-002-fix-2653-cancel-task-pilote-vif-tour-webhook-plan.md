# mika#2653 — `cancel_task` sur un pilote vif depuis un tour webhook PR

**Ticket :** senara-solutions/mika#2653
**Labels :** `bug`, `p1-important`, `agent-core`, `dispatch:loop`, `loop-substrate`
**Parent de famille :** mika#2649 (fermé — vecteur `run_claude_pilot`)
**Frères nommés :** mika#2652 (cap implement), mika#2654 (`promote_deferred_callback`)

---

## 1. Le défaut, mesuré

Trace `8ec6364c-be71-11f1-908b-e931f18d2c16`, 2026-10-02 : un tour mika-dev ouvert
par un **webhook de revue QA sur la PR #2644** a appelé `cancel_task` sur le
pilote **Fix-CI en vol** (`8a3b2082`) — **71 tours jetés**.

Second cas mesuré de la famille mika#2649 (*un tour webhook PR agit sur le plan
de dispatch au-delà de ce que l'événement justifie*), le même jour, et le plus
coûteux des deux : mika#2649 a refermé le vecteur `run_claude_pilot` (un dispatch
hors lignée) ; celui-ci reste ouvert, et son remède n'est pas un terme de
**cible** mais un terme de **vivacité**.

Le recensement AC4 de mika#2649 le nomme déjà, avec son verdict :

> `cancel_task` — servi, **OUI — et c'est le second cas mesuré** : c'est lui qui
> a tué `8a3b2082` et jeté 71 tours. Son remède n'est pas un terme de cible mais
> un terme de **vivacité** (`live_pilot`, mika#2279) ; ce qui le sort d'ici est
> le rayon de souffle et le cinquième booléen sur `ToolContext`.

Il n'y a donc **rien à établir sur l'existence du défaut** : la phase 2 de
mika#2649 l'a mesuré, nommé et borné hors de son périmètre. Ce plan est ce
changement distinct.

---

## 2. Six rectifications que la lecture du code impose au ticket

C'est le premier livrable. Cinq d'entre elles changent la forme du correctif, et
la sixième change l'arbitrage central.

### R1 — Le booléen existant est l'INVERSE de celui dont la garde a besoin

Le ticket dit *« juste un booléen de classe de tour sur le modèle
d'`is_webhook_fallthrough_turn` »*. Sur le **modèle**, oui ; par **réutilisation**,
non : `ToolContext.is_webhook_fallthrough_turn` est posé par
`webhook_dispatch::is_webhook_fallthrough_domain`, qui **sort explicitement les
deux familles qui nous intéressent** du domaine Fallthrough —

```rust
// webhook_dispatch.rs:110-116
if msg.starts_with(PR_EVENT_PREFIX)        { return false; }
if msg.starts_with(CHECK_SUITE_EVENT_PREFIX) { return false; }
```

— parce que `self-dev-webhook-qa` / `-ci` y portent des dispatchs légitimes. Le
quatrième booléen vaut donc **`false` exactement sur la population de ce
ticket**, et le lire serait une garde à population vide, c'est-à-dire la classe
mika#2205 (*une garde que personne n'a exercée se lit exactement comme une garde
qui marche*). Le cinquième booléen est bien un **axe nouveau**, et les deux sont
**mutuellement exclusifs par construction** — propriété à épingler, sans quoi on
aurait créé deux axes qui se chevauchent.

### R2 — `live_pilot_for_issue` n'est pas appelable ici : il manque un lecteur par tâche

`live_pilot_for_issue(db, issue_url)` prend une **URL d'issue**. `cancel_task`
reçoit un **identifiant de tâche**, et la topologie à deux lignes que
`live_pilot.rs` documente lui-même l'interdit de dériver :

| ligne | `trigger_type` | `reference_url` | `process_id` |
|---|---|---|---|
| **parent** (tracking) | `manual` / `action_type='none'` | **oui** | **jamais** |
| **enfant** (callback) | `callback` / `resume_agent` | **non** | **oui** (le pgid) |

Une ligne callback — celle qui porte le pgid, donc le cas probable du constat —
**n'a pas d'URL**, donc n'a aucun chemin vers `live_pilot_for_issue`. Le lecteur
manquant est `live_pilot_for_task`, et le ticket le décrit comme s'il existait.

### R3 — La traversée que ce lecteur demande EXISTE DÉJÀ, dans `cancel_task_and_kill`

`task_engine::process_kill::cancel_task_and_kill` porte depuis mika#2335 la
résolution exacte dont la garde a besoin, avec **les deux mêmes filtres** que
`live_pilot_for_issue` (enfant non terminal, `process_start_time` présent) :

```rust
let (process_id, start_time, pid_owner) = match process_id {
    Some(pid) => (Some(pid), start_time, task_id.to_string()),
    None => match db.find_dispatch_children_with_pid(task_id).await { … },
};
```

Donc **écrire `live_pilot_for_task` sans toucher à ce site produirait deux
traversées**, c'est-à-dire la classe `grooming_marker` (mika#2158) que ce dépôt a
déjà payée deux fois sur ce chemin précis. Le geste juste est l'extraction de la
**traversée** (pas de la classification), motif mika#2624 : *la classification est
extraite, le fetch et le report restent où ils sont.*

### R4 — `cancel_reminder` délègue à `CancelTaskTool`, donc un site couvre deux outils

`tools/cancel_reminder.rs:35` est littéralement `CancelTaskTool.execute(input,
ctx).await`. Une garde posée dans `CancelTaskTool::execute` couvre les deux
outils **gratuitement**, et aucune n'est à écrire deux fois. À nommer plutôt qu'à
découvrir : un futur éditeur qui dédoublerait `cancel_reminder` perdrait la garde
en silence.

### R5 — Le geste de reprise de l'opérateur n'est PAS sur ce chemin

C'est la rectification qui décide de l'arbitrage, et elle renverse la raison n°2
par laquelle le ticket se sortait du périmètre de mika#2649.

`cancel_task_and_kill` a **quatre** appelants, et un seul traverse un
`ToolContext` :

| appelant | site | passe par `ToolContext` ? |
|---|---|---|
| `mika tasks cancel <id>` | `mika-cli/src/commands/tasks.rs:166,303` | **non** |
| `POST /tasks/{id}/cancel` | `mika-agent/src/server/handlers.rs:808` | **non** |
| outil `cancel_task` / `cancel_reminder` | `tools/cancel_task.rs:53` | **oui** |
| tests eval mika#2335 | `tests/eval/test_supersede_kills_live_pilot.rs` | non |

Et sur ce quatrième chemin, le booléen ne vaut `true` que si
`originating_message` **commence par** `[GitHub] PR ` ou `[GitHub] Check suite `.
Un opérateur qui écrit « annule la tâche X » par `mika ask` ouvre un tour dont le
message est son texte : **hors population, aucune garde**.

> **Le ticket écrit : *« `cancel_task` est le geste de reprise le plus court de
> l'opérateur ; un faux refus le lui retire »*. C'est faux pour cette garde.**
> Aucun de ses trois chemins n'y arrive, et le quatrième n'est atteignable que
> par un tour que GitHub a ouvert. Ce qu'un faux refus retire est un appel
> d'outil à un modèle, dans un tour webhook, rendu visible dans
> `tool_calls.output` et dans `audit_events`.

### R6 — Il existe deux vecteurs voisins, et ils ne se ferment pas par ce terme

Trouvés en lisant le recensement des outils (§ 5) :

- **`update_task_status` → `cancelled`.** La machine à états l'autorise
  (`in_progress → cancelled`), et cet outil **ne tue aucun processus** : il rend
  la ligne terminale, donc **orpheline** le pilote (classe mika#2279 : parent
  terminal + pilote vif). Dommage différent (la comptabilité casse, les tours ne
  sont pas jetés), remède différent. **Suivi nommé**, non couvert.
- **`run_shell`.** `MIKA_DEV_IDENTITY` porte `shell-exec`, donc un modèle peut
  taper `mika tasks cancel <id>`. Population mesurée **vide**. **Suivi nommé**,
  non couvert — et c'est pourquoi le corps du refus ne nomme **aucune** commande
  (§ 4.5).

---

## 3. L'arbitrage de fail-safe, tranché — c'est le corps du ticket

Le ticket demande explicitement de trancher : `LivePilotVerdict::Unreadable`
refuse-t-il (fail-closed, cohérent avec mika#2649) ou laisse-t-il passer
(fail-open, cohérent avec la politique propre de `live_pilot`) ?

### La doctrine ne tranche pas seule, et il faut le dire

mika#2277 écrit *« un signal qu'on ne peut pas lire n'est jamais un terme
satisfait »*. Appliquée au terme « un pilote vif travaille ici », elle donne
fail-**closed** (on ne peut pas prouver l'absence ⇒ on refuse). Mais mika#2279 a
**délibérément dérogé** à cette doctrine pour ses deux appelants, et il l'écrit :

> *« `Unreadable` is not `Alive` : no readable `process_start_time`, a
> `process_id` outside `u32`, a DB error — in all three we cannot *prove* a pilot
> is alive, so we do not block […] Named rather than hidden. »*

Donc ce n'est pas une doctrine qu'on applique, c'est une **asymétrie de coût**
qu'on refait, site par site. Celle de mika#2279 est écrite dans son module : un
faux `Alive` y gèle **un** ticket dans une boucle de 20 minutes qui brûle un
point du budget de re-drive mika#2020 (trois tours abandonnent un ticket sain),
et le gel est **borné** par trois faucheurs. Un faux `None` y rejoue l'incident
en boucle.

### Ici l'asymétrie est inverse, et elle est mesurable

| | faux `Alive` (refus à tort) | faux `None` (passage à tort) |
|---|---|---|
| coût | **un appel d'outil refusé** dans un tour webhook | **71 tours jetés** (mesuré) |
| visibilité | `tool_calls.output` + une ligne WARN + une ligne d'audit | aucune ligne, le pilote meurt |
| boucle ? | **non** — un refus d'outil ne se rejoue pas tout seul ; borné par les 20 pas du tour, et aucune garde `required_tools` ne réclame `cancel_task` | oui — le défaut se rejoue à chaque webhook |
| geste opérateur | **intact sur ses trois chemins** (R5) | — |
| rattrapage | le modèle peut rendre la main, et le callback arrive de lui-même | aucun : le travail est détruit |

**Décision : fail-CLOSED.** `Unreadable` refuse, dans ses **deux** causes, avec
un motif d'audit distinct par cause (remèdes opposés : « la base ne répond pas »
contre « l'instance n'est pas prouvable »).

### Le coût de ce choix, nommé plutôt que découvert

Une ligne portant un `process_id` **sans** `process_start_time` lisible ne pourra
pas être annulée depuis un tour webhook PR. C'est la population que mika#2335
compte sous `unusable_child_count`, et c'est aussi celle que le faucheur
mika#2249 **décline** et que le watchdog #959 ne peut pas juger — donc le refus
peut durer jusqu'à ce que `timeout_at` (panic-fallback 6 h) rende la ligne
terminale. Borné, mesuré par sa propre valeur d'audit, et l'opérateur garde ses
trois chemins.

**L'arbitrage est local et ne se transporte pas** — exactement comme le dit
mika#2649 du sien, et comme le faucheur mika#2420 dit de l'inverse.

---

## 4. Conception

### 4.1 — `webhook_dispatch::is_webhook_pr_event_domain` (pur)

```rust
pub(crate) fn is_webhook_pr_event_domain(msg: &str) -> bool {
    msg.starts_with(PR_EVENT_PREFIX) || msg.starts_with(CHECK_SUITE_EVENT_PREFIX)
}
```

**Au même module, et c'est ce qui rend les littéraux sûrs.** Les deux constantes
`PR_EVENT_PREFIX` / `CHECK_SUITE_EVENT_PREFIX` ont un **site de définition
unique** depuis mika#2649, déjà lu par les deux faces de la frontière
(`is_webhook_fallthrough_domain`, qui sort la famille ; `webhook_event_target`,
qui l'y retrouve pour la borner). Un troisième lecteur **au même module** est
exactement ce que ces constantes existent pour permettre, et le scan
`mika2517_the_fallthrough_domain_has_a_single_definition` reste vert par
construction (il cherche la conjonction des deux littéraux et n'accuse que les
fichiers **hors** `webhook_dispatch.rs`).

**Assertion de cohérence, et elle est porteuse.** On épingle

```
is_webhook_pr_event_domain(msg) == (webhook_event_target(msg) != NotApplicable)
```

sur un corpus de messages couvrant les quatre variantes. Motif
`mika2293_reconstruction_equals_load_for_agent_on_every_cascade_position` : le
jour où quelqu'un modifie l'une des deux faces sans l'autre, ce test rougit au
lieu de laisser les deux prédicats diverger en silence.

**Pourquoi pas appeler `webhook_event_target` et tester `!= NotApplicable` ?**
La sémantique serait juste (`Unreadable` — préfixe présent, grammaire non
parsée — est **dans** la population : un tour webhook PR dont la grammaire a
bougé reste un tour webhook PR, et la garde doit mordre). Mais ça paierait deux
regex par tour de conversation pour n'en lire qu'un booléen, et jetterait la
cible. Le prédicat de préfixe est le bon outil ; l'assertion ci-dessus est ce qui
les tient d'accord.

### 4.2 — `ToolContext.is_webhook_pr_event_turn` (cinquième booléen)

Un **verdict**, jamais une charge utile — la frontière que `tools/mod.rs` a
tranchée en mika#2573 en citant mika#2517 :

> *« L'objection porte sur une `&str` avec sa durée de vie et sa charge utile. Ce
> qui traverse ici est le **verdict**, calculé au seul site qui possède déjà le
> message et qui appelle déjà le prédicat de domaine. »*

Un site de production le calcule (`agent_loop/mod.rs:5563`, le site de
conversation, le seul qui possède `params.user_message`), **en appelant le
prédicat, jamais en le recopiant**. Les trois autres sites de production
(silent `6742`, team `7413`, `server/investigate.rs:779`) posent `false`, pour le
même périmètre que le quatrième booléen documente déjà : un webhook arrive par
`POST /message` → `run_agent` → mode conversation ; un tour silencieux n'a pas de
message de webhook (`originating_message` vaut `None` depuis mika#933) ; un tour
d'équipe lit `TeamAgentParams`. **Un futur chemin servant un webhook en mode
silencieux échapperait à la garde** — borne héritée de mika#2517 / mika#2573, ni
élargie ni modifiée, population mesurée vide.

21 sites de construction au total (1 appel + 20 `false`, dont 17 en test).

### 4.3 — `live_pilot::resolve_task_pilot` (la traversée, extraite — R3)

```rust
/// Quel pilote cette TÂCHE porte-t-elle ? (la traversée, pas le verdict)
pub enum TaskPilot {
    /// Un pgid a été trouvé, sur la ligne nommée ou sur un de ses enfants.
    Found { pid: i64, start_time: Option<u64>, owner_task_id: String },
    /// Aucune ligne de ce périmètre ne porte de pgid exploitable.
    None,
    /// La question n'a pas pu être posée (erreur base).
    Unreadable { reason: &'static str },
}

pub async fn resolve_task_pilot(db: &AsyncDatabase, task_id: &str) -> TaskPilot
```

Déplacement **sans changement de comportement** du `match` de
`cancel_task_and_kill`, filtres compris. Deux propriétés à préserver **à
l'identique**, et l'asymétrie entre elles est délibérée :

- la branche `Some(pid)` (ligne nommée) accepte un `start_time` **absent** —
  `kill_process_gracefully` retombe alors sur son contrôle basique ;
- la branche `None` (traversée des enfants) **exige** `process_start_time`
  présent, parce que « mal signaler un *groupe* de processus est un dommage non
  borné » (commentaire mika#2335, sur site).

**Le helper ne logue rien.** `cancel_task_and_kill` garde son WARN
(`"cancel: dispatch-child lookup failed — …"`) **mot pour mot**, et la garde
émet le sien. Zéro changement de surface de journal, donc aucun grep opérateur
déplacé.

**Comment le risque de cette extraction est borné :** les trois appels à
`cancel_task_and_kill` de `tests/eval/test_supersede_kills_live_pilot.rs`
exercent déjà les trois formes (parent sans pid + enfant vif, pid direct, échec
de lookup). Ils doivent rester **verts sans modification** — c'est la mesure, pas
la promesse.

### 4.4 — `live_pilot::live_pilot_for_task` (le verdict)

```rust
pub async fn live_pilot_for_task(db: &AsyncDatabase, task_id: &str) -> LivePilotVerdict
```

`resolve_task_pilot` puis classification, en réutilisant **l'enum existante** de
`live_pilot.rs` et ses deux constantes de motif (`UNREADABLE_DB_ERROR`,
`UNREADABLE_NO_START_TIME`) :

| `TaskPilot` | vivacité | verdict |
|---|---|---|
| `Found { start_time: Some(st), .. }` + `is_same_process_alive(pid, st)` | prouvée | `Alive { .. }` |
| `Found { start_time: Some(_), .. }` + processus mort | prouvée absente | `None` |
| `Found { start_time: None, .. }` | **non prouvable** | `Unreadable { NO_START_TIME }` |
| `None` | aucun pilote | `None` |
| `Unreadable { reason }` | — | `Unreadable { reason }` |

`is_same_process_alive` (pid **et** `process_start_time`), jamais une existence
nue de `/proc/<pid>` où un PID recyclé est indistinguable du pilote.

**`live_pilot.rs` reste le lecteur unique de la question de vivacité** — c'est
son contrat écrit (*« **Sole reader** of that question »*), et il passe de deux à
trois appelants plutôt que d'être recopié. *Un recensement n'est pas une
allowlist : on y ajoute, on n'y exempte pas* (mika#2633).

### 4.5 — La garde, dans `CancelTaskTool::execute`

**Placement :** après `AgentScopedTaskId::from_tool_context` et
`validate_task_exists` (la tâche doit exister et appartenir à l'agent avant qu'on
parle de son pilote), **avant** `cancel_task_and_kill` — donc avant toute
écriture de statut et avant tout signal.

**Dans l'outil, jamais dans `cancel_task_and_kill`** : c'est ce qui préserve les
trois chemins opérateur **par construction** plutôt que par prédicat (R5), et
c'est la différence avec une garde posée dans l'infrastructure de kill.

**Terme de classe de tour dans le prédicat, jamais une branche de l'appelant** —
forme mika#2649 :

```rust
pub(crate) fn classify_cancel_on_live_pilot(
    is_webhook_pr_event_turn: bool,
    verdict: &LivePilotVerdict,
) -> CancelPilotDecision
```

« hors tour webhook PR, rien ne change » devient ainsi une propriété de la
fonction pure, avec son propre test, plutôt qu'un `if` à lire.

Quatre issues, `match` exhaustif **sans bras `_ =>`** :

| décision | condition | effet |
|---|---|---|
| `Allowed` | hors tour webhook PR | aucune ligne, aucun audit — c'est l'essentiel du trafic (mika#2131) |
| `AllowedInPopulation` | tour webhook PR + `None` | INFO + audit `allowed_no_pilot` — **le contrôle positif** |
| `Refused(LivePilot)` | tour webhook PR + `Alive` | WARN + audit, `ToolOutput::error` |
| `Refused(Unreadable(reason))` | tour webhook PR + `Unreadable` | WARN + audit, `ToolOutput::error` |

**Le corps du refus nomme la levée sans donner de gabarit (doctrine mika#2520 /
mika#2292).** Il porte :

- le fait : un pilote travaille sous cette tâche (identifiant de la ligne qui
  porte le pgid, pid) ;
- les **deux sorties correctes pour le modèle** : attendre le callback, ou
  rendre la main en signalant la situation ;
- **à qui** appartient la levée : un opérateur, sur l'hôte.

Et il **ne nomme aucune commande**. `MIKA_DEV_IDENTITY` porte `shell-exec`, donc
écrire `mika tasks cancel <id>` dans le refus donnerait au modèle le gabarit du
contournement — *un refus qui donne le gabarit est une fuite avec une étape de
plus*. Le résidu `run_shell` est nommé en R6 comme suivi, pas comme sortie.

**Erreur JSON structurée**, cohérente avec les deux validations voisines du même
fichier (`{"error":"task_not_found"}`, `{"error":"invalid_uuid"}`) et avec les
sept refus de `validate_dispatch_readiness`. Les valeurs du champ `error` sont un
**format de fil** (§ 6), épinglées par test.

---

## 5. Recensement : qui peut encore toucher un pilote vif depuis un tour webhook PR

Rappel porteur : ces tours sont **hors** du domaine Fallthrough, donc
`FALLTHROUGH_WITHHELD_TOOLS` ne s'y applique pas et `create_task` y est servi.

| outil | servi ? | peut tuer un pilote vif ? | verdict |
|---|---|---|---|
| `cancel_task` | oui | **OUI — le vecteur mesuré** | **fermé par ce plan** |
| `cancel_reminder` | oui | oui (délègue à `CancelTaskTool`) | **fermé** — même site, R4 |
| `update_task_status` → `cancelled` | oui | **non le tue, mais l'ORPHELINE** (ligne terminale, classe mika#2279) | **non couvert** — dommage et remède distincts. **Suivi** (R6) |
| `promote_deferred_callback` | oui | choisit quel dispatch prend le créneau | **suivi mika#2654**, population mesurée vide |
| `run_claude_pilot{,_groom}` | oui | borné à sa lignée | **fermé par mika#2649** |
| `run_shell` | oui | oui, par `mika tasks cancel` | **non couvert**, population mesurée vide. **Suivi** (R6) |
| `complete_task`, `create_task`, `run_gh`, `send_message`, `list_tasks`, `check_task` | oui | non | hors population |

**Pourquoi ne pas retenir `cancel_task` (`FALLTHROUGH_WITHHELD_TOOLS`) :** refusé
par le ticket, et pour la raison que mika#2484 a déjà dû écrire sur `run_gh` —
une retenue casse un geste légitime que le prompt prescrit, ici l'annulation d'un
dispatch réellement perdu. La retenue est **indisponible**, le refus
**conditionnel** est la seule forme qui reste.

---

## 6. Télémétrie

**Un seul `tool_name` d'audit, la décision dans `after_value`** — motif
`ready_label_outcome` (mika#2323) : les quatre issues appartiennent au **même
site** et à la **même population**, donc un `GROUP BY after_value` les sépare et
les rend soustractibles. Délibérément **pas** le motif à deux noms de
`phantom_aged_out` / `phantom_sweep_spared` (mika#2156), qui s'applique quand
chaque nom porte sa propre cause.

`CANCEL_PILOT_GUARD_AUDIT_TOOL = "cancel_task_pilot_guard"`, **SOLE WRITER**.

| `after_value` | régime attendu | lecture |
|---|---|---|
| `blocked_live_pilot` | **zéro** | chaque ligne est un pilote vif que le moteur n'a pas tué |
| `blocked_db_unreadable` | **zéro** | la base ne répond pas — c'est **elle** qu'il faut lire |
| `blocked_start_time_unreadable` | **zéro** | l'instance n'est pas prouvable (population `unusable_child_count`, mika#2335) |
| `allowed_no_pilot` | **non vide, faible** | **le contrôle positif** : la garde tourne et laisse passer |

**Le contrôle positif n'est pas décoratif.** Sans `allowed_no_pilot`, zéro ligne
aurait **trois** causes indistinguables : aucun refus (sain), aucune annulation
depuis un tour webhook (sain), binaire antérieur au correctif (classe
mika#2340). *Une garde que personne n'a exercée se lit exactement comme une garde
qui marche* (mika#2205).

**Rien n'est audité hors population.** Une annulation sur un tour ordinaire
n'écrit aucune ligne : ce serait l'essentiel du trafic, soit très exactement le
churn que la doctrine mika#2131 borne.

Journal (`$MIKA_SPIRIT_LOG_FILE`), trois noms pour deux conduites de refus :

| événement | niveau | régime attendu |
|---|---|---|
| `cancel_task_live_pilot_blocked` | WARN | **vide** |
| `cancel_task_pilot_unreadable` (champ `reason`) | WARN | **vide** |
| `cancel_task_webhook_turn_allowed` | INFO | non vide, faible |
| `cancel_task_pilot_guard_audit_failed` | WARN | **vide** — le WARN est passé, l'audit non : le `GROUP BY` sous-compte |

Écriture d'audit **fire-and-forget** : un échec d'audit ne doit jamais pouvoir
changer un verdict d'annulation.

---

## 7. Definition of Done

1. `is_webhook_pr_event_domain` existe dans `webhook_dispatch.rs`, au site des
   deux constantes de préfixe, et son accord avec `webhook_event_target` est
   épinglé.
2. `ToolContext.is_webhook_pr_event_turn` existe, calculé par appel au prédicat
   au seul site de conversation, `false` aux trois autres sites de production.
3. La traversée parent→enfant a **un seul site** (`resolve_task_pilot`), appelé
   par `cancel_task_and_kill` **et** par `live_pilot_for_task`, sans changement
   de comportement du chemin de kill.
4. `live_pilot_for_task` rend les trois verdicts et reste dans `live_pilot.rs`.
5. `CancelTaskTool::execute` refuse une annulation sur `Alive` et sur
   `Unreadable` dans un tour webhook PR, et ne change **rien** hors de cette
   population.
6. Le corps du refus nomme le fait, les deux sorties du modèle et le propriétaire
   de la levée, **sans nommer de commande**.
7. Les quatre valeurs d'audit sont écrites par un site unique, avec les trois
   lignes de journal et le résidu d'audit.
8. `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check` verts ; les
   trois tests mika#2335 de `test_supersede_kills_live_pilot.rs` verts **sans
   modification**.
9. `crates/mika-agent/CLAUDE.md` et le `CLAUDE.md` racine portent la section
   opérateur (surfaces, régimes attendus, sondes, haltes, ce que ça n'achète
   pas).

---

## 8. Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de sa *Proposed Solution*, de son arbitrage
explicite et des rectifications du § 2.

- **AC1 — Le défaut mesuré est refermé.** Dans un tour dont
  l'`originating_message` commence par `[GitHub] PR ` ou
  `[GitHub] Check suite `, un appel à `cancel_task` sur une tâche dont
  `live_pilot_for_task` rend `Alive` est **refusé** : aucun statut écrit, aucun
  signal envoyé, et le pilote continue de tourner.
- **AC2 — L'arbitrage de fail-safe est tranché, implémenté et écrit.**
  `Unreadable` **refuse**, dans ses deux causes, chacune sous son propre motif
  d'audit ; la divergence avec la politique de `live_pilot` est documentée au site
  avec l'asymétrie de coût qui la justifie, et le coût du choix est nommé.
- **AC3 — Hors tour webhook PR, rien ne change.** Les trois chemins opérateur
  (`mika tasks cancel`, `POST /tasks/{id}/cancel`, une conversation) annulent
  exactement comme avant, octet pour octet ; cette propriété est portée par la
  **fonction pure** (le terme de classe de tour est un paramètre), pas par une
  branche de l'appelant.
- **AC4 — Le recensement est livré avec un verdict par outil** (§ 5), y compris
  les deux vecteurs voisins non couverts et leur raison.
- **AC5 — Le cinquième booléen est un verdict, pas une charge utile**, et son
  exclusion mutuelle avec `is_webhook_fallthrough_turn` est épinglée.
- **AC6 — La traversée n'est pas dupliquée.** Un site, deux classifications, et
  un scan de source refuse un troisième site.
- **AC7 — La population est comptable.** Quatre valeurs d'audit sous un
  `tool_name` à écrivain unique, dont une qui constitue le contrôle positif.
- **AC8 — Le refus ne nomme aucun contournement** et nomme sa levée.

---

## 9. Fire-Disposition

Ce plan livre des détecteurs : deux scans de source, une garde d'exécution et
leurs tests. Disposition retenue : **(a) exception nommée en allowlist**, avec
les **deux allowlists livrées VIDES et épinglées vides**.

**Pourquoi (a) et pas (b) « livrer désarmé ».** Les deux scans visent des
populations qu'on **mesure vides maintenant**, pas des violations existantes
qu'on exempterait : `find_dispatch_children_with_pid` n'est composé avec
`is_same_process_alive` **nulle part** aujourd'hui (`cancel_task_and_kill` passe
par `kill_process_gracefully`), et le `tool_name` d'audit est un nom neuf. Un
détecteur livré désarmé sur une population vide est un détecteur qu'on n'arme
jamais ; une allowlist née non vide serait un emplacement où déposer la prochaine
infraction (mika#2323). La garde d'exécution, elle, est livrée **armée** : le
défaut est p1 et mesuré, et son désarmement est un revert (§ *Hors périmètre*).

### Détecteur 1 — `mika2653_la_vivacite_dun_pilote_a_un_lecteur_unique`

Scan de source (`canonical_tokens.rs`, motif
`mika2624_le_predicat_de_hold_a_un_lecteur_unique`). Refuse un site de
production hors `live_pilot.rs` qui compose une traversée de dispatch
(`find_dispatch_children_with_pid` / `find_dispatch_children_for_issue_url`) avec
`is_same_process_alive`.

- **Allowlist :** `LIVE_PILOT_TASK_READER_ALLOWED: &[&str] = &[]`, épinglée vide
  par `mika2653_la_allowlist_du_lecteur_est_vide`.
- **Anti-vacuité :** le scan échoue si le propriétaire (`live_pilot.rs`) ne porte
  **pas** la composition — un scan qui vise un corps mort ne vérifie rien et se
  lit exactement comme un arbre propre (mika#2103 / mika#2205).
- **Contrôle de bonne foi :** `…_le_scan_du_lecteur_voit_un_second_site`, qui
  injecte un second site en fixture et vérifie que le prédicat rougit — vu rouge
  à la livraison.
- **Résolution quand il tire :** router le site par `live_pilot_for_task`, **ne
  pas** ajouter de ligne (doctrine mika#2201).
- **Aucun test comportemental ne peut voir cette classe :** un second lecteur ne
  rend **aucune décision fausse le jour où il est écrit** ; il diverge plus tard,
  en silence, avec toutes les assertions vertes.

### Détecteur 2 — `mika2653_le_nom_daudit_a_un_seul_ecrivain`

Scan de source, forme **exacte** de
`mika2649_le_nom_daudit_a_un_seul_ecrivain` — y compris sa correction mesurée :
la comparaison du littéral est **exacte et non en sous-chaîne**, parce que le nom
du résidu (`cancel_task_pilot_guard_audit_failed`) a le nom d'audit pour
**préfixe** et serait compté comme un second écrivain.

- **Allowlist :** `CANCEL_PILOT_GUARD_SOLE_WRITER_EXCEPTIONS: &[&str] = &[]`,
  épinglée vide.
- **Anti-vacuité** + énumération par `production_sources_to_test_module` (et non
  `production_sources`, qui tronque au premier `#[cfg(test)]` où qu'il soit).
- **Résolution :** faire passer le site par la constante partagée.

### Détecteur 3 — format de fil

`mika2653_les_valeurs_daudit_sont_un_format_de_fil` et
`mika2653_les_motifs_derreur_sont_un_format_de_fil` figent les quatre valeurs
d'`after_value` et les valeurs du champ `error`. Elles atterrissent dans
`audit_events` et dans `tool_calls.output`, où un opérateur les groupe : deux
orthographes d'un même motif couperaient une population en deux sans le dire.

### Détecteur 4 — exclusivité des deux axes

`mika2653_les_deux_axes_de_tour_webhook_sont_exclusifs` : pour tout message,
`is_webhook_fallthrough_domain(m) && is_webhook_pr_event_domain(m)` est faux.
C'est AC5, et c'est ce qui empêche qu'un futur éditeur fasse se chevaucher les
deux booléens.

### Détecteur 5 — la garde d'exécution

Les tests de `classify_cancel_on_live_pilot` et le fichier eval (§ 11). **Chaque
terme vu rouge par mutation, un à la fois** — une conjonction de termes fail-safe
ne se prouve pas en les neutralisant tous ensemble (leçon mika#2277). Le
**contrôle négatif hors population** est porteur : sans lui, « la garde décide »
est indistinguable de « la garde bloque tout », et les trois chemins opérateur
pourraient être cassés avec tous les tests au vert.

---

## 10. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/webhook_dispatch.rs` (prédicat + doc + tests) | 55 |
| `crates/mika-agent/src/tools/mod.rs` (5ᵉ booléen + doc) | 35 |
| 21 sites de construction `ToolContext` (1 appel + 20 `false`) | 22 |
| `crates/mika-agent/src/live_pilot.rs` (`resolve_task_pilot`, `live_pilot_for_task`, doc, tests) | 205 |
| `crates/mika-agent/src/task_engine/process_kill.rs` (appel du helper) | 20 |
| `crates/mika-agent/src/tools/cancel_task.rs` (garde, `classify_*`, corps du refus, constantes, tests) | 160 |
| télémétrie (3 lignes de journal, audit, 4 constantes de motif, résidu) | 55 |
| `crates/mika-agent/src/canonical_tokens.rs` (détecteurs 1–4 + allowlists + contrôles) | 130 |
| `crates/mika-agent/tests/eval/test_cancel_task_live_pilot_2653.rs` | 190 |

Total estimé : 872 lignes

**Sous le seuil de 1000, donc une seule phase** — mais la marge est mince, et la
soupape est nommée plutôt que découverte : si l'implémentation dérive au-delà de
1000, ce qui se renvoie à un suivi est **l'extraction `resolve_task_pilot` plus
le détecteur 1** (≈ 290 lignes), qui sont une propriété de **forme** (la
non-duplication de la traversée) et non la fermeture du défaut. Les AC1–AC5,
AC7 et AC8 restent satisfaits sans eux ; **AC6 serait alors explicitement
différé**, avec la duplication nommée au site. L'ordre inverse est interdit :
livrer le détecteur sans la garde ne referme rien.

---

## 11. Vérification

### 11.1 — En CI (déterministe, sans réseau, sans base de production)

- **V1 — unités de `classify_cancel_on_live_pilot`** : les quatre issues, plus le
  contrôle négatif hors population (`is_webhook_pr_event_turn = false` sur un
  verdict `Alive` ⇒ `Allowed`).
- **V2 — unités de `live_pilot_for_task`** (`live_pilot.rs`, base en mémoire,
  `#[cfg(target_os = "linux")]` où un vrai PID est requis — motif
  `self_pid_and_start`) :
  - la tâche porte le pid, le processus est vif ⇒ `Alive` ;
  - la tâche est un **parent** sans pid, un enfant non terminal porte un pilote
    vif ⇒ `Alive` (le cas de la topologie à deux lignes) ;
  - enfant **terminal** portant un pgid périmé ⇒ `None` ;
  - pid mort avec start_time lisible ⇒ `None` (jamais `Unreadable`) ;
  - enfant non terminal **sans** `process_start_time` ⇒ `Unreadable` ;
  - aucune ligne portant de pgid ⇒ `None`.
- **V3 — accord des deux faces** : `is_webhook_pr_event_domain` ≡
  `webhook_event_target != NotApplicable` sur le corpus des quatre variantes.
- **V4 — exclusivité des deux axes** (détecteur 4).
- **V5 — non-régression du chemin de kill** : les trois tests de
  `test_supersede_kills_live_pilot.rs` passent **sans modification**, ce qui est
  la mesure que l'extraction du § 4.3 n'a rien changé.
- **V6 — chemin de production** (`tests/eval/test_cancel_task_live_pilot_2653.rs`,
  `MockLlmProvider`, aucun réseau) : un tour dont le message d'origine est un
  `[GitHub] PR review …` appelle `cancel_task` sur une tâche à pilote vif ; le
  `tool_result` est une erreur, la ligne reste non terminale, le processus
  survit, une ligne d'audit `blocked_live_pilot` existe. **Plus le contrôle
  négatif** : le même appel depuis un tour de conversation annule.
- **V7 — les deux scans de source** et leurs contrôles de bonne foi, vus rouges
  avant la livraison.

### 11.2 — Sondes post-déploiement (gestes d'opérateur sur l'hôte)

> **Préalable.** Ces mesures décrivent le **binaire servi**. Après `make deploy`,
> établir que le `mika-spirit` qui tourne porte le correctif avant toute
> conclusion (classe mika#2340). La base n'est pas montée dans le bac à sable de
> dispatch : aucune de ces requêtes n'est exécutable par un pilote.

```bash
# 1. Un pilote vif a-t-il été épargné ?
grep cancel_task_live_pilot_blocked "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{task_id, owner_task_id, pid, session_id, trace_id}'

# 2. CONTRÔLE NÉGATIF — un signal illisible a-t-il refusé ? (régime attendu : VIDE)
grep cancel_task_pilot_unreadable "$MIKA_SPIRIT_LOG_FILE" | jq -c '{task_id, reason}'

# 3. CONTRÔLE POSITIF — la garde tourne-t-elle seulement ?
grep -c cancel_task_webhook_turn_allowed "$MIKA_SPIRIT_LOG_FILE"
```

```sql
-- Les quatre issues, soustractibles en une requête (SOLE WRITER ⇒ compte exact)
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'cancel_task_pilot_guard' GROUP BY 1 ORDER BY 2 DESC;

-- La sonde du ticket, désormais jointe à une décision
SELECT created_at, session_id, substr(input, 1, 200)
  FROM tool_calls WHERE tool_name = 'cancel_task'
 ORDER BY created_at DESC LIMIT 50;
```

**S1 — le défaut fondateur ne se rejoue pas** (premier tour webhook PR qui
tente). Attendu : une ligne `blocked_live_pilot`, la tâche non terminale, le
pilote toujours vivant, et son callback arrive ensuite normalement.
*Halte 1 — aucune ligne alors qu'un pilote a été tué :* **ne pas élargir le
prédicat par réflexe.** Lire d'abord le contrôle positif (commande 3) : zéro des
deux ne prouve rien du tout (mika#2205). Établir ensuite **par quelle porte**
l'annulation est passée — `run_shell`, `update_task_status`,
`promote_deferred_callback`, un chemin opérateur : quatre remèdes, et trois sont
des suivis nommés, pas ce prédicat.

**S2 — contrôle négatif de l'illisible (7 jours).** La commande 2 reste vide, et
`blocked_start_time_unreadable` reste à zéro.
*Halte 2 — `blocked_db_unreadable` non vide :* la base ne répond pas, et le
fail-closed gèle les annulations de ce chemin. C'est la **base** qu'il faut lire,
pas la disposition qu'il faut inverser — l'inverser rouvrirait le constat.
*Halte 3 — `blocked_start_time_unreadable` soutenu sur une même tâche :* c'est le
coût nommé au § 3 qui se réalise. Vérifier que `timeout_at` finira par rendre la
ligne terminale, et **ne pas** retirer le terme : le geste est opérateur, sur
l'hôte.

**S3 — contrôle négatif de bruit (7 jours).** Aucun refus sur un tour **hors**
population, et en particulier aucun sur une annulation d'opérateur.
*Halte 4 — une occurrence :* la garde mord hors de sa population, donc le terme
de classe de tour est mal lu. **Désarmer d'abord** (revert de l'appel dans
`CancelTaskTool::execute`), diagnostiquer ensuite — retirer à l'opérateur son
geste d'annulation est pire que le défaut qu'on referme, puisque ce geste n'a pas
de contournement.

**S4 — la population hors périmètre (30 jours), et c'est la précondition des deux
suivis de R6.**
```sql
-- Vecteur `update_task_status → cancelled` depuis un tour webhook PR
SELECT created_at, session_id, substr(input, 1, 200) FROM tool_calls
 WHERE tool_name = 'update_task_status' AND input LIKE '%cancelled%'
 ORDER BY created_at DESC LIMIT 50;

-- Vecteur `run_shell` → `mika tasks cancel`
SELECT created_at, session_id, substr(input, 1, 200) FROM tool_calls
 WHERE tool_name = 'run_shell' AND input LIKE '%tasks cancel%'
 ORDER BY created_at DESC LIMIT 50;
```
*Halte 5 — l'une des deux rend des lignes :* le suivi s'ouvre **avec ce
compte**, jamais avec une intuition. Zéro est un **résultat** (le modèle
n'emprunte pas cette route), pas une preuve que la route est fermée.

**Halte transverse — les sondes muettes.** Zéro refus **et** zéro
`allowed_no_pilot` ne prouve rien : il faut qu'un `cancel_task` ait été appelé
depuis un tour webhook PR depuis le déploiement.

---

## 12. Ce que ce travail n'achète PAS

- **Il ne rend pas le modèle incapable de vouloir annuler un pilote.** Il rend
  l'annulation impossible **par cet outil, dans cette classe de tour**. La
  doctrine maison est *construis l'incapacité, ne promets pas la retenue*
  (mika#1991) ; elle est **partiellement** applicable ici — la capacité est bien
  retirée à `cancel_task` / `cancel_reminder`, et elle reste entière via
  `run_shell` (R6).
- **Il ne rattrape pas l'incident du 2026-10-02.** `8a3b2082` est mort, ses 71
  tours sont perdus, et **rien ne rétro-estampille** : fabriquer une ligne
  d'audit datée d'un refus qu'on n'a pas observé est l'inverse de ce que ce
  travail défend. La sonde est la **prochaine** occurrence.
- **Il ne ferme pas `update_task_status → cancelled`**, qui orpheline un pilote
  au lieu de le tuer. Dommage différent, remède différent, **suivi nommé** avec
  la sonde S4 pour précondition.
- **Il ne ferme pas `run_shell`.** Un modèle déterminé garde une route ; ce qui
  lui est retiré est celle qu'il a **effectivement** prise, et le corps du refus
  ne lui donne pas l'autre.
- **Il ne ferme pas `promote_deferred_callback`** (mika#2654) ni le cap implement
  (mika#2652) : trois tickets, trois termes, trois populations.
- **Il ne couvre pas un tour webhook servi en mode silencieux** — borne héritée
  de mika#2517 / mika#2573, population mesurée vide.
- **Il ne rend pas le champ surveillé.** Les seuls instruments sont les greps et
  les requêtes du § 11.2, et **leur silence ne prouve rien tant que personne ne
  les exécute** — d'où le contrôle positif obligatoire.
- **Il n'ajoute aucune variable d'environnement, et c'est une décision.**
  Précédents les plus proches : mika#1646 (garde d'action destructive),
  mika#2624 (hold contre `pr ready`), mika#2573 (création de travail en
  Fallthrough) — aucun n'en a, pour la raison qu'ils écrivent : *un désarmement
  par variable sur un chemin de sûreté serait un désarmement par coquille*. Le
  geste de désarmement est un **revert**, et le coût d'un faux positif (§ 3) le
  supporte.

---

## 13. Hors périmètre, délibérément

- **Le cap implement** (mika#2652) et son contournement : aucune valeur de
  `MIKA_DISPATCH_MAX_CONCURRENT_IMPLEMENT`, aucune ligne de
  `has_active_callback_tasks_excluding`, aucun bail de créneau touché.
- **`promote_deferred_callback`** (mika#2654).
- **Le terme de lignée de mika#2649** : inchangé, octet pour octet. Il répond à
  *quelle cible ce dispatch vise-t-il*, celui-ci à *un pilote travaille-t-il
  ici* — axes orthogonaux, et le ticket le dit.
- **`live_pilot_for_issue`, `LivePilotVerdict::is_alive`, la politique de
  fail-safe des deux appelants existants** (gate 2c, `auto_pull` filtre 4b) :
  aucun octet touché. Ce plan **ajoute** un appelant et une fonction ; il ne
  modifie aucune disposition existante.
- **`kill_process_gracefully`, le watchdog #959, le faucheur mika#2249, le sweep
  phantom #1712, la supersession mika#2335** : aucun contact.
- **Les trois chemins opérateur** (CLI, HTTP, conversation) : inchangés par
  construction (R5), et c'est ce que le contrôle négatif de V1/V6 atteste.
- **Le domaine Fallthrough et `FALLTHROUGH_WITHHELD_TOOLS`** : inchangés ; la
  retenue est refusée avec sa raison (§ 5).
- **Retirer le permanent `2>/dev/null` de `dispatch-lib.sh`**, le résidu `run_shell`
  et le vecteur `update_task_status` : trois suivis nommés, aucun ouvert ici.
