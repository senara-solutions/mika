# mika#2763 — `cancel_task` ne tue plus un pilote vivant, quelle que soit la classe du tour

**Ticket :** senara-solutions/mika#2763
**Type :** fix (substrat de boucle)
**Vol :** 1 de 2 — « le refus » (le vol 2, « la surface de diagnostic », est nommé en `Tracked in:`)

## Constat

Le 2026-10-17 à 07:59Z, un tour silencieux de mika-dev (`trigger=deferred_dispatch`)
ouvert pour lancer le regroom de mika#2752 a trouvé la place groom occupée par le
groom **vivant** de mika#2757. Il l'a déclaré « stale », l'a annulé avec
`cancel_task` — SIGTERM au pilote — puis a promu son propre lanceur différé.

| instant | outil | effet |
|---|---|---|
| 07:59:23Z | `run_claude_pilot_groom` (mika#2752) | refusé, `global_dispatch_active` |
| 07:59:25Z | `check_task` sur le **parent** de mika#2757 | `Created: 2026-10-17 06:41:38` |
| 07:59:26Z | `check_task` sur le **callback** | refusé (« not a manual task ») |
| 07:59:30–33Z | `run_gh` | aucun plan au corps, aucune PR |
| 07:59:38Z | `cancel_task` sur le callback | « Process (PID …) terminated. » |
| 07:59:41Z | `promote_deferred_callback(groom)` | lanceur différé promu |

Le message de l'agent — « mika#2757 groom dispatch was stale (>1h, no output, no
PR, no Plan callout) » — **est faux** : le callback annulé avait été lancé à
07:55:25Z, il avait **4 minutes**. Le « >1h » est l'âge du **parent** (06:41:38Z),
la seule date que l'agent ait pu lire. mika#2757 est `in_progress` sans enfant
actif depuis 07:59:38Z.

## Cause, vérifiée dans l'arbre

**Le mécanisme existe déjà et il n'a pas tourné.** Ce n'est pas une garde
manquante, c'est une **population trop étroite**.

`cancel_task.rs` porte la garde mika#2653, qui calcule exactement le bon verdict
(`live_pilot_for_task` → `Alive { child_task_id, pid }`) et refuse. Mais son site
d'appel est sous un `if` (`cancel_task.rs:459`) :

```rust
if ctx.is_webhook_pr_event_turn {
    let verdict = live_pilot_for_task(ctx.db, id).await;
    let decision = classify_cancel_on_live_pilot(ctx.is_webhook_pr_event_turn, &verdict);
    …
}
let outcome = crate::task_engine::process_kill::cancel_task_and_kill(ctx.db, id).await?;
```

Le tour de l'incident est un `SilentTrigger::DeferredDispatch`. Le site silencieux
de `ToolContext` (`agent_loop/mod.rs`, construction du contexte des tours
silencieux) pose, **en dur** :

```rust
is_callback_turn: matches!(params.trigger,
    SilentTrigger::Callback { .. } | SilentTrigger::PostCallbackAdvance { .. }
  | SilentTrigger::DeferredDispatch { .. }),
is_webhook_fallthrough_turn: false,
is_webhook_pr_event_turn: false,
```

Donc `is_webhook_pr_event_turn == false` **par construction** : ni la lecture de
vivacité ni la garde n'ont tourné. Aucune ligne de journal, aucune ligne d'audit —
c'est pourquoi l'incident a dû être reconstitué à la main depuis `tool_calls`.

**Le défaut était écrit à l'avance.** Le doc-commentaire de
`is_webhook_pr_event_turn` (`tools/mod.rs`) porte déjà la borne, mot pour mot :
*« **Un futur chemin servant un webhook en mode silencieux échapperait à cette
garde** — borne héritée de mika#2517 / mika#2573, ni élargie ni modifiée,
population mesurée vide. »* La population n'est plus vide : mika#2763 en est le
premier élément mesuré.

**Et le ticket à ouvrir avait été nommé.** Le § *Hors périmètre* du plan mika#2739
écrit : *« `cancel_task` ne garde que les tours `is_webhook_pr_event_turn` […] un
`cancel_task` depuis ce type de tour tuerait encore un pilote vivant. Ce plan ne
l'élargit pas […] Recommandation à l'opérateur : ouvrir ce ticket. »* mika#2763
**est** ce ticket, et sa facture a été payée en un vol.

### Pourquoi l'agent ne pouvait pas vérifier sa propre affirmation

`check_task` filtre sur `trigger_type == "manual"` et refuse tout le reste
(`check_task.rs`) — le filtre est intentionnel (l'outil fait un enrichissement
GitHub depuis `reference_url`, que seule une ligne manuelle porte). Un callback
`long_running:*` n'a pas de `reference_url` et n'est jamais manuel : **aucune
lecture d'outil ne rend l'âge d'un callback**. `get_task` l'accepte (aucun filtre
de `trigger_type`) mais n'affiche ni âge ni état de process. Le « >1h » cité était
donc structurellement l'âge d'une autre tâche. C'est le périmètre du **vol 2**.

## Décisions de conception

### D1 — On retire le terme de classe de tour. La garde devient inconditionnelle.

Option 1 du ticket, dans sa forme forte. Le classificateur perd son premier
paramètre et une issue :

```rust
pub(crate) enum CancelPilotDecision {
    AllowedNoPilot,                                   // contrôle positif
    RefusedLivePilot { child_task_id: String, pid: u32 },
    RefusedUnreadable { reason: &'static str },
}

pub(crate) fn classify_cancel_on_live_pilot(
    verdict: &LivePilotVerdict,
) -> CancelPilotDecision { … }
```

**Ce n'est pas une invention, c'est une convergence.** La garde sœur de
`complete_task` (mika#2739, mergée la veille) est **déjà** exactement cette forme,
et son doc-commentaire le dit : *« **Trois issues et non quatre** : contrairement
à mika#2653, cette garde n'a pas de terme de classe de tour (D4 du plan), donc il
n'existe pas d'issue « hors population ». »* Son incident fondateur est de la même
famille — 2026-10-16 13:19Z, un tour **deferred-dispatch** complétant le
`long_running:run_claude_pilot` d'un pilote en vol depuis 52 min comme « stale
callback ». Après ce vol, `cancel_task` est la **dernière** des deux portes de
vivacité à porter encore un terme de classe de tour, et ce terme est précisément
ce par quoi l'incident est passé.

### D2 — Le geste de reprise de l'opérateur n'est pas sur ce chemin, et c'est vérifié

C'est l'objection qui a retenu mika#2739, et elle tombe sur un fait mesurable.
mika#2739 écrit que le gating de `cancel_task` existe pour préserver le geste
opérateur, *« lequel **traverse** `cancel_task` (`mika tasks cancel`) »*. La
lecture qui rend cette phrase vraie est celle de la **fonction**
`cancel_task_and_kill`, pas celle de l'**outil** — et la garde est dans l'outil.

`cancel_task_and_kill` a exactement quatre appelants de production, et un seul
traverse un `ToolContext` :

| appelant | site | traverse l'outil ? |
|---|---|---|
| CLI `mika tasks cancel` | `mika-cli/src/commands/tasks.rs:167`, `:304` | **non** |
| HTTP `POST /tasks/{id}/cancel` | `mika-agent/src/server/handlers.rs:856` | **non** |
| `CancelTaskTool::execute` | `mika-agent/src/tools/cancel_task.rs:469` | oui |

`cancel_task.rs:204` l'écrit déjà comme un fait sur lequel mika#2653 raisonne :
*« `cancel_task_and_kill` a quatre appelants et **un seul traverse un
`ToolContext`** »*. Et le même paragraphe (rectification R5 de mika#2653) a déjà
réfuté la prémisse sur laquelle mika#2739 s'est appuyé : *« Le ticket écrit
« `cancel_task` est le geste de reprise le plus court de l'opérateur ; un faux
refus le lui retire » — **c'est faux pour cette garde**. »*

**Conséquence : élargir la population ne coûte aucun chemin opérateur.** La CLI
garde son geste (avec sa propre confirmation de vivacité, mika#2670) et le HTTP le
sien. La garde reste **dans l'outil, jamais dans `cancel_task_and_kill`** — c'est
ce qui préserve les deux chemins **par construction** plutôt que par prédicat, et
c'est la propriété qu'il ne faut pas perdre en « simplifiant » vers la couche de
kill.

### D3 — Ce que l'élargissement retire vraiment, nommé plutôt que découvert

Un seul geste disparaît : **annuler un pilote vivant depuis une conversation**,
par un appel d'outil du modèle. Ce que l'opérateur conserve :

- `mika tasks cancel <id>` sur l'hôte — hors outil, donc hors garde ;
- `POST /tasks/{id}/cancel` — idem ;
- l'annulation par outil de toute tâche **sans** pilote vivant, qui est la
  totalité du trafic nominal (`allowed_no_pilot`).

Le test eval `mika2653_un_tour_de_conversation_annule_comme_avant` atteste
aujourd'hui l'inverse : il annule un pilote vif depuis un tour de conversation et
vérifie que l'enfant est tué. **Il est renversé par ce plan**, donc il est
**remplacé avec ses raisons**, jamais `#[ignore]`-é ni supprimé en silence (voir
Fire-Disposition).

### D4 — Arbitrage de fail-safe : inchangé, fail-CLOSED

`Unreadable` continue de **refuser**, dans ses deux causes (`UNREADABLE_DB_ERROR`,
`UNREADABLE_NO_START_TIME`). L'asymétrie de coût que mika#2653 avait tranchée est
la même, et l'élargissement la renforce :

| | faux `Alive` (refus à tort) | faux `None` (passage à tort) |
|---|---|---|
| coût | **un appel d'outil refusé** ; la place reste tenue jusqu'au watchdog #959, au faucheur #2249/#2277 ou à `timeout_at` (panic-fallback 6 h) | **un vol entier détruit** — 71 tours jetés (mika#2653), un groom de 4 min tué et un ticket figé `in_progress` (mika#2763) |
| visibilité | `tool_calls.output` + un WARN + une ligne d'audit | **aucune ligne** |
| boucle ? | non — aucun `required_tools` ne réclame `cancel_task`, donc aucune garde EndTurn ne peut exiger l'appel que la garde refuse | oui — se rejoue à chaque tour de dispatch différé |
| rattrapage | le callback arrive de lui-même ; les deux chemins opérateur restent entiers | aucun |

**Coût du fail-closed, élargi avec la population.** Une ligne portant un
`process_id` **sans** `process_start_time` lisible — la population que mika#2335
compte sous `unusable_child_count` — ne pourra plus être annulée par l'outil, et
désormais depuis **n'importe quel** tour, non plus seulement depuis un tour
webhook PR. Borné (`timeout_at`), compté par sa propre valeur d'audit
(`blocked_start_time_unreadable`), et les deux chemins opérateur restent entiers.
L'arbitrage reste **local** : il vaut pour cet outil, pas pour
`cancel_task_and_kill`.

### D5 — L'option 2 du ticket est rejetée, avec ses raisons

Le ticket propose, « plus strict » : *un agent ne peut pas annuler le callback
d'un **autre** ticket que celui de son tour.* Rejeté sur trois points :

1. **mika#2653 a déjà tranché l'axe.** Son doc-commentaire : *« son remède n'est
   pas un terme de **cible** mais un terme de **vivacité** »*. L'option 2 est un
   terme de cible, et c'est la forme qui avait été écartée.
2. **Le terme n'est pas calculable.** `ToolContext` ne porte pas l'identité du
   ticket du tour. L'option 2 exigerait un nouveau champ sur `ToolContext`, posé
   correctement aux ~21 sites de construction — c'est-à-dire exactement la classe
   de défaut que mika#2763 vient de payer (un booléen de classe de tour posé en
   dur à `false` sur le site qui comptait).
3. **Elle ne couvre pas l'incident sœur.** mika#2739 a mesuré un tour
   deferred-dispatch complétant le callback d'un ticket qui *était* le sien par
   filiation. Un terme de cible l'aurait laissé passer ; un terme de vivacité
   l'attrape.

Un terme de vivacité est aussi ce que le ticket demande au fond : *« sauf preuve
d'inactivité calculée par le moteur »*. La **staleness reste possédée par le
moteur** — watchdog PID #959, faucheur de stalle silencieuse #2249/#2277, balayage
fantôme #1712, `timeout_at` à 6 h — et jamais par le modèle. Ce plan n'ajoute
aucun calcul de staleness ; il refuse au modèle le droit d'en inventer un.

### D6 — Le refus devient agnostique de la classe de tour

Les deux corps de refus actuels justifient par l'événement : *« This turn was
opened by a GitHub pull-request or check-suite event, which authorizes reviewing
and reporting on that pull request — not ending work in flight. »* Hors tour
webhook PR cette phrase est **fausse**, donc elle sort. Ce qui la remplace nomme
le fait et le coût, pas l'événement : *un pilote travaille sous cette tâche ;
l'annuler jette chaque tour qu'il a produit ; « aucune sortie, aucune PR, aucun
plan » est l'état normal d'un dispatch en cours et n'est pas une preuve de
staleness.*

Cette dernière clause est la **prémisse réfutée de l'incident**, et elle va au
site pour la même raison que mika#2739 y a mis la sienne (*un parent `completed`
est l'état normal après un groom*) : c'est la croyance qui a produit le geste.

Le refus garde l'invariant AC8 de mika#2653 : il porte le PID, la ligne qui porte
le pgid, les **deux** sorties correctes pour le modèle (attendre le callback ;
rendre la main en signalant), et à qui appartient la levée (un opérateur, sur
l'hôte). Il **ne nomme aucune commande** — `MIKA_DEV_IDENTITY` porte `shell-exec`,
donc *un refus qui donne le gabarit est une fuite avec une étape de plus*
(mika#2520 / mika#2292).

### D7 — Vocabulaire d'audit : inchangé. Un seul nom d'événement bouge.

**Les quatre valeurs d'audit sont conservées à l'identique** —
`blocked_live_pilot`, `blocked_db_unreadable`, `blocked_start_time_unreadable`,
`allowed_no_pilot` — et `ALL_CANCEL_PILOT_GUARD_VERDICTS` reste à quatre entrées,
toutes atteignables. C'est obligatoire et c'est vérifié par un test qui existe
déjà : `complete_task.rs:768` lit ce registre dans les **deux sens** (toute valeur
produite est déclarée, toute valeur déclarée est atteignable), et `complete_task`
importe trois de ces constantes plus `unreadable_audit_value`. **Aucun littéral
d'audit n'est renommé** : couper un vocabulaire en deux orthographes sans le dire
est ce que le commentaire de `ERROR_CANCEL_LIVE_PILOT` interdit. Les deux
populations restent séparées par le `tool_name` (`cancel_task_pilot_guard` vs
`complete_task_pilot_guard`), jamais par la valeur.

Un seul nom change, et c'est un **nom d'événement de journal** :

| avant | après | raison |
|---|---|---|
| `cancel_task_webhook_turn_allowed` (INFO) | `cancel_task_pilot_absent` (INFO) | le nom affirmait la population qui disparaît ; le nouveau nomme le fait, et c'est le nom que mika#2739 emploie déjà pour son contrôle positif |

**Coût nommé :** un opérateur qui grepe l'ancien nom obtient zéro ligne après
déploiement, et zéro se lit comme « la garde ne tourne pas » (classe mika#2205).
La migration est l'affaire d'une ligne de requête ; elle est écrite dans le corps
de PR et dans la sonde ci-dessous. Les deux noms de WARN
(`cancel_task_live_pilot_blocked`, `cancel_task_pilot_unreadable`) et les deux
motifs d'erreur (`cancel_refused_live_pilot`, `cancel_refused_pilot_unreadable`)
sont **inchangés** — ce sont eux que les opérateurs groupent.

### D8 — Churn d'audit : il augmente, et la borne est dite

Aujourd'hui la garde n'écrit **rien** hors tour webhook PR (`audit_value()` rend
`None` sur `Allowed`). Demain, **tout** appel à `cancel_task` écrit une ligne —
modalement `allowed_no_pilot`. C'est le churn que la doctrine mika#2131 borne, et
il est accepté ici pour trois raisons :

1. le volume est celui de `cancel_task`, pas celui des tours — un outil rare,
   appelé sur décision, jamais en boucle ;
2. le contrôle positif est ce qui rend « zéro refus » distinguable de « la garde
   ne tourne pas » (mika#2205), et cette distinction est **précisément** ce qui a
   manqué pour diagnostiquer mika#2763 ;
3. une lecture de vivacité est désormais payée à chaque appel — une requête de
   traversée de dispatch, la même que `complete_task` paie déjà sur toute ligne
   callback complétable.

`cancel_reminder` hérite de tout cela **gratuitement** : `CancelReminderTool`
délègue littéralement à `CancelTaskTool::execute`. À nommer plutôt qu'à découvrir
— un futur éditeur qui dédoublerait `cancel_reminder` perdrait la garde en
silence, et désormais sur **toute** classe de tour.

### D9 — `promote_deferred_callback` ne change pas, et son doc-commentaire si

Aucune ligne de `promote_deferred_callback.rs` n'est touchée : le cancel refusé, la
place reste tenue, et le force-promote répond déjà
`deferred_dispatch_force_promote_rejected_slot_busy`. Le lanceur différé **attend
la libération**, ce qui est le comportement que le ticket décrit comme correct.

Mais `promote_deferred_callback.rs:87-92` **raisonne sur** `CancelPilotDecision::`
`AllowedInPopulation` comme sur un fait de l'arbre. Renommer la variante rendrait
ce paragraphe faux en silence — la classe exacte que mika#2739 a dû nommer pour
`test_complete_task_wrong_trigger_type`. Le paragraphe est donc **mis à jour**, et
sa substance survit intacte : *« la sœur mika#2653 a un second terme (la vivacité
d'un pilote), donc « la garde a tourné et a laissé passer » y est un état distinct
de « la garde n'a pas tourné » »* devient **plus** vraie, la vivacité étant
désormais le seul terme.

### D10 — Découpage en deux vols, et le vol 1 est le critère d'acceptation du ticket

Le ticket porte trois correctifs candidats. Les items 1 et 2 sont un **arbitrage**
(tranché en D1/D5) ; l'item 3 — exposer l'âge et l'état de process d'un callback —
est une **surface de lecture** indépendante, mergeable seule, sans laquelle le
refus du vol 1 fonctionne déjà.

Le *Critère d'acceptation minimal* du ticket ne nomme que le vol 1 : *« un tour
`deferred_dispatch` dont le dispatch est refusé par un callback groom vivant de
4 min n'annule pas ce callback, et le pilote n'est pas tué. Un test épingle le
refus de `cancel_task` sur un callback `long_running:*` à process vivant. »*

Vol 2, nommé et non livré ici : `get_task` affiche `Age:` et l'état de pilote
(via `live_pilot_for_task`, donc par le **lecteur unique** et sans recomposer la
traversée — le scan `mika2653_la_vivacite_dun_pilote_a_un_lecteur_unique` refuse
toute seconde composition), et le refus de `check_task` sur une ligne non manuelle
**redirige vers `get_task`** au lieu d'être un cul-de-sac. Les deux mis ensemble
font passer le total au-dessus de la borne par vol (voir *Taille estimée*).

Tracked in: senara-solutions/mika#2763 — vol 2 (« la surface de diagnostic d'un
callback : `get_task` porte l'âge et l'état de pilote, `check_task` redirige »)

## Ordre d'implémentation

Trois commits, chacun compilant et testable seul.

### Commit 1 — la garde perd son terme de classe de tour (`cancel_task.rs`)

Dans l'ordre du fichier :

1. `enum CancelPilotDecision` — `Allowed` et `AllowedInPopulation` fusionnent en
   **`AllowedNoPilot`** (le nom de la sœur mika#2739). `audit_value()` rend
   désormais `Some(AUDIT_ALLOWED_NO_PILOT)` sur l'issue de passage, donc plus
   jamais `None` ; le `match` reste **sans bras `_ =>`** pour qu'une quatrième
   issue soit tranchée par le compilateur.
2. `classify_cancel_on_live_pilot(verdict: &LivePilotVerdict)` — le premier
   paramètre et le `if` de tête disparaissent ; trois bras, `Unreadable` →
   `RefusedUnreadable` (D4). La fonction reste **pure** pour que ses trois issues
   soient testables sans base.
3. Le doc-commentaire de la fonction (`cancel_task.rs:166-222`) — réécrit sur
   trois axes : le § *« Le terme de classe de tour est DANS le prédicat »*
   disparaît (il n'y a plus de terme) ; le § *« Et le geste de reprise de
   l'opérateur n'est PAS sur ce chemin »* **reste et devient porteur** — c'est D2,
   avec le tableau des quatre appelants ; le second incident fondateur
   (mika#2763) s'ajoute au premier, et le tableau d'asymétrie prend sa seconde
   mesure.
4. `refusal_body` — les deux bras perdent leur justification par l'événement
   webhook et prennent la formulation de D6, dont la clause sur « aucune sortie,
   aucune PR » ; le bras de passage devient le seul `AllowedNoPilot => String::new()`.
5. `report_cancel_pilot_decision` — trois bras ; le `let Some(audit_value) = … else`
   de tête devient structurellement inatteignable sur l'issue de passage et est
   retiré au profit d'un `audit_value()` total ; l'INFO est renommé
   `cancel_task_pilot_absent` (D7) et ses deux WARN gardent leurs noms, en
   retirant de leur message la mention du tour webhook.
6. Le site d'appel (`cancel_task.rs:437-467`) — le `if ctx.is_webhook_pr_event_turn`
   disparaît. Le bloc de commentaire garde ses deux invariants (**avant**
   `cancel_task_and_kill`, donc avant toute écriture et avant tout signal ;
   **dans l'outil, jamais dans la couche de kill**, D2) et perd le § sur la
   population.

Plancher : `cargo test -p mika-agent cancel_task` compile ; les quatre tests
historiques (`test_cancel_task_success`, `_not_found`, `_invalid_uuid`,
`_missing_id`) restent verts sans retouche.

### Commit 2 — `promote_deferred_callback.rs` (prose seule)

Le doc-commentaire de `ForcePromoteTurnDecision` (lignes ~86-96) cite
`CancelPilotDecision::AllowedInPopulation` : mise à jour vers `AllowedNoPilot`,
plus une phrase disant que mika#2763 a retiré le terme de classe de tour de la
sœur, ce qui rend le contraste **plus** net et non moins (D9). Aucun changement de
comportement, aucun test touché.

### Commit 3 — détecteurs

**3a — `cancel_task.rs`, `mod tests`.**

| test | sort |
|---|---|
| `mika2653_les_quatre_issues_du_predicat` | → `mika2763_les_trois_issues_du_predicat` : trois issues, signature sans booléen |
| **`mika2653_hors_tour_webhook_pr_rien_ne_change`** | **remplacé avec ses raisons** par `mika2763_la_classe_du_tour_nentre_plus_dans_le_predicat` : les trois verdicts rendent la même décision qu'ils rendaient *dans* la population, et le message d'assertion nomme mika#2763 comme le ticket qui a renversé l'AC3 de mika#2653 |
| `mika2653_les_valeurs_daudit_sont_un_format_de_fil` | amendé : l'issue de passage audite désormais, donc `audit_value()` n'est plus `None` sur aucune issue |
| `mika2653_les_motifs_derreur_sont_un_format_de_fil` | inchangé (les deux motifs survivent) |
| `mika2653_le_refus_ne_nomme_aucun_contournement` | **conservé, étendu** : les substrings interdits (`tasks cancel`, `run_shell`, `mika tasks`, `update_task_status`, `promote_deferred_callback`, `kill `, `sigterm`) restent, et l'assertion que le corps contient `operator` reste — c'est AC8 de mika#2653, que ce plan ne touche pas |

Plus deux cas de **chemin de production** (`tool.execute(input, &ctx)`) sur un
`ToolContext` ordinaire, c'est-à-dire avec `is_webhook_pr_event_turn == false` :

| # | cas | montage | attendu |
|---|---|---|---|
| U1 | pilote vivant hors tour webhook (**rouge avant le correctif**) | ligne `callback` `long_running:run_claude_pilot_groom` + enfant `sleep` réel, pgid et `process_start_time` en base | `is_error`, contient `cancel_refused_live_pilot` et le PID ; **l'enfant est toujours vivant** après l'appel |
| U2 | `process_start_time` illisible hors tour webhook (D4) | `process_id` vivant, `metadata = NULL` | `is_error`, contient `cancel_refused_pilot_unreadable` |

U1 et U2 sont `#[cfg(target_os = "linux")]` : `is_same_process_alive` lit
`/proc/<pid>/stat` et rend `false` ailleurs. **Un enfant `sleep` avec
`process_group(0)` et un `Drop` qui `kill` puis `waitpid`, jamais
`std::process::id()`** : ces cas peuvent atteindre `cancel_task_and_kill`, qui
envoie un SIGTERM au **groupe de processus** — avec notre propre pid, un cas qui
passe à tort tuerait la campagne de test. C'est l'idiome que le fichier eval porte
déjà sous `LivePilotChild`, et la raison est écrite au même endroit.

**3b — `tests/eval/test_cancel_task_live_pilot_2653.rs` : le rejeu.**

Le fichier est **étendu**, pas dupliqué : il porte déjà `LivePilotChild` (spawn
`process_group(0)`, `pgid_and_start()`, `is_alive()`, `Drop` qui `kill` le groupe
puis `waitpid`), les helpers de montage parent/enfant et les quatre cas de
mika#2653. En-tête de module amendé : le second incident fondateur, et la raison
pour laquelle l'axe du contrôle négatif se déplace.

| # | cas | sort |
|---|---|---|
| V7 | **`mika2763_un_tour_deferred_dispatch_nannule_pas_un_pilote_vif`** — le rejeu du ticket, **nouveau** | `run_silent_agent` avec `SilentTrigger::DeferredDispatch { task_id, parent_task_id, action_config }`, un callback groom vivant de « 4 min » (`created_at` reculé), un `MockLlmProvider` scripté pour appeler `cancel_task` dessus puis rendre la main. Attendu : l'appel rend `cancel_refused_live_pilot`, **l'enfant est toujours vivant**, le statut du callback est inchangé, et une ligne d'audit `blocked_live_pilot` existe sous `cancel_task_pilot_guard` |
| V6b | `mika2653_un_tour_de_conversation_annule_comme_avant` | **remplacé avec ses raisons** par `mika2763_un_tour_de_conversation_refuse_aussi` : même montage, verdict inversé — refus, enfant vivant. L'en-tête du cas dit ce qui est perdu (D3) et où le geste survit (CLI, HTTP) |
| V6a, V6c, V6d | tour webhook PR : refus / aucun pilote / signal illisible | **inchangés** — ils deviennent des cas de non-régression de la population élargie |

**Pourquoi le chemin de production et pas seulement le prédicat.** Le fichier
écrit déjà le raisonnement et il tient à l'identique, dans l'autre sens : les
tests unitaires du classificateur restent **verts** si le site d'appel n'appelle
jamais la garde. Ici le site d'appel est précisément ce qui change, et le mode de
panne est qu'un `if` résiduel la laisse hors d'atteinte sur les tours silencieux.
Seul un vrai tour `DeferredDispatch` traversant `run_silent_agent` le ferme — et
c'est exactement ce que V7 fait.

**Aucun nouveau scan structurel n'est livré.** Aucun nom d'audit ne naît (D7),
donc le scan d'écrivain unique `mika2653_le_nom_daudit_a_un_seul_ecrivain` couvre
déjà `cancel_task_pilot_guard` et reste vert sans retouche ;
`mika2653_la_vivacite_dun_pilote_a_un_lecteur_unique` reste vert parce que ce plan
n'écrit **aucune** logique de vivacité — il retire un `if`. Le vérifier, et ne pas
ajouter de garde là où une garde existe (doctrine mika#2201).

### Documentation (hors décompte de lignes)

- `tools/mod.rs` — le doc-commentaire de `is_webhook_pr_event_turn` dit que la
  borne qu'il annonçait a été **franchie** (mika#2763) et que `cancel_task` n'en
  dépend plus. La borne reste vraie pour `promote_deferred_callback` (mika#2654)
  et pour le reste : c'est une population réduite, pas une borne levée.
- `docs/agent-internals/tools.md` — la garde de `cancel_task` devient
  inconditionnelle : la population, le coût du fail-closed élargi, le nom
  d'événement renommé, et ce que l'opérateur conserve.
- `docs/architecture.md` — la cellule de `cancel_task`, si elle qualifie la garde
  de « tour webhook PR ».
- `scripts/sync-agent-docs.sh` **obligatoire** après : le job CI `docs-sync`
  échoue sinon.

## Contrat de vérification

```
cargo test -p mika-agent cancel_task
cargo test -p mika-agent complete_task
cargo test -p mika-agent canonical_tokens
cargo test -p mika-agent --test eval cancel_task_live_pilot
cargo test                      # suite entière
cargo clippy --all-targets -- -D warnings
cargo fmt --check
scripts/sync-agent-docs.sh      # puis git diff --exit-code
make verify-bundled-skills
scripts/verify-pipeline.sh origin/main
```

**Preuve que V7 et U1 sont rouges avant le correctif.** À constater et à écrire
dans le corps de PR : posés sur le `cancel_task.rs` d'avant, V7 et U1 laissent
passer l'annulation et l'enfant meurt — c'est l'incident reproduit. Un test qui
n'a jamais été vu rouge n'atteste rien (motif `grooming_marker`, mika#2158).

**Preuve que le registre d'audit reste bidirectionnel.** `cargo test -p mika-agent
complete_task` couvre `ALL_CANCEL_PILOT_GUARD_VERDICTS` dans les deux sens
(`complete_task.rs:768-772`). Si ce test rougit, un littéral d'audit a été touché
en violation de D7 : restaurer le littéral, ne pas amender le registre.

### Sonde post-déploiement

```sql
SELECT after_value, count(*)
FROM audit_events
WHERE tool_name = 'cancel_task_pilot_guard'
GROUP BY 1;
```

Lecture : `allowed_no_pilot` **doit** devenir non nul en quelques jours — c'est le
contrôle positif qui dit que la garde tourne hors tour webhook, et avant ce
correctif il ne pouvait l'être que sur un tour webhook PR sans pilote (population
quasi vide). `blocked_live_pilot` non nul = un incident de la famille mika#2763 a
été rattrapé. `blocked_start_time_unreadable` non nul = la population
`unusable_child_count` touche ce chemin et le coût de D4 est réel plutôt que
théorique. **L'absence totale de lignes signifie un binaire antérieur au
correctif** (classe mika#2340), jamais « aucun appel » — l'instrument n'est vivant
qu'après `make deploy`.

Et la migration de nom (D7), à dire dans le corps de PR :

```sql
-- avant : event = 'cancel_task_webhook_turn_allowed'
-- après : event = 'cancel_task_pilot_absent'
```

## Fire-Disposition

Ce plan livre des détecteurs : les cinq assertions unitaires de 3a (dont U1/U2) et
les deux cas eval de 3b. Disposition retenue : **(a) exception nommée en allowlist,
avec une allowlist vide** — et la forme qu'elle prend ici est le **remplacement
avec raisons** de deux assertions existantes, jamais leur désarmement.

- **Aucune nouvelle allowlist n'est créée, et aucune entrée n'est ajoutée à une
  allowlist existante.** Les deux allowlists des scans mika#2653
  (`…_SOLE_WRITER_EXCEPTIONS` du nom d'audit, et celle du lecteur unique de
  vivacité) sont livrées **vides** et pinnées vides par leurs assertions
  auto-nettoyantes ; ce plan les laisse vides et c'est vérifié par le contrat de
  vérification, parce qu'il n'écrit aucune logique de vivacité et ne crée aucun
  nom d'audit (D7). Une entrée ajoutée ici serait un emplacement où déposer la
  prochaine infraction (mika#2323).
- **Les deux violations existantes sont des assertions, et elles sont remplacées,
  pas exemptées.** `mika2653_hors_tour_webhook_pr_rien_ne_change` (unitaire,
  message « INVARIANT VIOLÉ (AC3) ») et
  `mika2653_un_tour_de_conversation_annule_comme_avant` (eval) atteste**ent**
  l'exact comportement que mika#2763 renverse. Chacune est **réécrite sous un nom
  mika#2763**, avec dans son corps la phrase qui dit quel ticket l'a renversée et
  pourquoi — grep-visible sur `mika2763` depuis l'ancien nom via le message
  d'assertion. Ni `#[ignore]`, ni suppression silencieuse : un invariant qu'on
  retire doit laisser la trace de sa décision, sinon le prochain lecteur le
  reconstruit.
- **Aucun détecteur n'atterrit désarmé.** U1, U2 et V7 sont le feu voulu (rouges
  avant, verts après) ; V6a/V6c/V6d sont leurs contrôles de non-régression. Le
  gating `#[cfg(target_os = "linux")]` de U1/U2/V7 n'est pas un désarmement mais
  la portée réelle du prédicat (`/proc/<pid>/stat`), et c'est le gating que les
  quatre cas mika#2653 portent déjà pour la même raison.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/tools/cancel_task.rs` (enum à 3 issues, classificateur sans paramètre, corps de refus, report, site d'appel, doc réécrite) | 95 |
| tests (`cancel_task.rs`, `mod tests` : 2 remplacements, 2 amendements, U1–U2) | 95 |
| tests eval (`tests/eval/test_cancel_task_live_pilot_2653.rs` : V7 via `run_silent_agent`, V6b inversé, en-tête amendé) | 140 |
| `crates/mika-agent/src/tools/promote_deferred_callback.rs` (doc-commentaire D9) | 10 |

Total estimé : 340 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Deux vols, et voici pourquoi le vol 1 tient.** 340 < 429, avec 89 lignes de
marge, et très en dessous du seuil par PR de 1000. Le volume est tenu par deux
leviers précis :

1. **Rien n'est écrit, un `if` est retiré.** La traversée de dispatch, le filtre
   terminal, la preuve d'instance, le verdict à trois bras, le vocabulaire
   d'audit, les deux motifs d'erreur et les helpers de test existent tous. Le
   travail de production est un retrait de terme plus une réécriture de doc.
2. **Le fichier eval est étendu, pas créé.** `LivePilotChild` avec son
   `process_group(0)` et son `Drop`, les helpers de montage parent/enfant et les
   constantes de refus sont déjà là. V7 paie la construction de
   `SilentAgentParams` et son script de mock ; V6b n'est qu'une inversion
   d'assertions.

**Le vol 2 est ce qui faisait franchir la borne**, et c'est pourquoi il est séparé
plutôt que rogné : `get_task` enrichi (≈ 45) + ses tests (≈ 70) + la redirection
de `check_task` et son test (≈ 20) portent le total à ≈ 475, soit 46 lignes
**au-dessus** de 429. Un vol à 425 avec quatre lignes de marge ne serait pas un
plan, ce serait un pari. Et le découpage suit la nature des livrables : le vol 1
est l'arbitrage que le ticket demande de trancher, le vol 2 est une surface de
lecture mergeable seule, sans laquelle le refus du vol 1 fonctionne déjà.

Tracked in: senara-solutions/mika#2763 — vol 2 (« la surface de diagnostic d'un
callback »)

## Definition of Done

- [ ] `classify_cancel_on_live_pilot` ne prend plus de terme de classe de tour et
      rend trois issues ; le `match` d'`audit_value()` est sans bras `_ =>`.
- [ ] Le site d'appel dans `CancelTaskTool::execute` n'est plus sous un `if` de
      classe de tour, reste **avant** `cancel_task_and_kill`, et la garde reste
      **dans l'outil** (aucune ligne ajoutée à `process_kill.rs`).
- [ ] `Unreadable` refuse, dans ses deux causes, et le coût élargi est écrit au
      site (D4).
- [ ] Le refus porte le PID et la ligne qui porte le pgid, nomme la prémisse
      réfutée (« aucune sortie / aucune PR / aucun plan » n'est pas une preuve de
      staleness), ne justifie plus par l'événement webhook, et ne nomme aucune
      commande — AC8 de mika#2653 toujours vert.
- [ ] Les quatre valeurs d'audit et les deux motifs d'erreur sont **inchangés** ;
      `ALL_CANCEL_PILOT_GUARD_VERDICTS` reste à quatre entrées toutes
      atteignables, et le test bidirectionnel de `complete_task.rs` est vert.
- [ ] L'INFO est renommé `cancel_task_pilot_absent` et la migration de requête est
      écrite dans le corps de PR.
- [ ] Les trois issues écrivent une ligne d'audit sous `cancel_task_pilot_guard`,
      en fire-and-forget, y compris le passage.
- [ ] U1 et V7 constatés **rouges** avant le correctif, et le corps de PR le dit.
- [ ] U1, U2, V7 verts après, gatés Linux ; V6a/V6c/V6d intacts.
- [ ] Les deux assertions renversées sont **remplacées sous un nom mika#2763**
      portant la raison du renversement ; aucune n'est `#[ignore]`-ée ni
      supprimée.
- [ ] Les quatre tests historiques de `cancel_task::tests` intacts.
- [ ] Le doc-commentaire de `ForcePromoteTurnDecision` ne cite plus une variante
      qui n'existe pas (D9) ; aucun comportement de
      `promote_deferred_callback` touché.
- [ ] Le doc-commentaire de `is_webhook_pr_event_turn` (`tools/mod.rs`) dit que la
      borne a été franchie par mika#2763 et quelle population elle couvre encore.
- [ ] `docs/agent-internals/tools.md` (+ `docs/architecture.md` si elle qualifie
      la garde) à jour, et `scripts/sync-agent-docs.sh` passé
      (`git diff --exit-code` propre).
- [ ] Contrat de vérification entièrement vert.
- [ ] Corps de PR : `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

Transcrits du *Critère d'acceptation minimal* de mika#2763, puis dérivés :

1. **Rejeu du cas (littéral du ticket).** Un tour `deferred_dispatch` dont le
   dispatch est refusé par un callback groom vivant de 4 min **n'annule pas** ce
   callback, et le pilote **n'est pas tué**.
2. **Épinglage (littéral du ticket).** Un test épingle le refus de `cancel_task`
   sur un callback `long_running:*` à process vivant.
3. Le même refus se produit **quelle que soit la classe du tour** — silencieux
   (`DeferredDispatch`, `Callback`, `PostCallbackAdvance`), conversationnel, ou
   webhook PR : le prédicat n'a plus de terme de classe de tour.
4. Le refus porte un identifiant stable (`cancel_refused_live_pilot`), le PID, la
   ligne de tâche qui porte le pgid, et ne nomme aucune commande de
   contournement.
5. `cancel_task` sur une tâche **sans** pilote vivant garde exactement son
   comportement actuel, sur toute classe de tour, et écrit la ligne d'audit
   `allowed_no_pilot` qui l'atteste.
6. Un `process_start_time` illisible **refuse** (fail-closed), avec son propre
   identifiant (`cancel_refused_pilot_unreadable`) et sa propre valeur d'audit.
7. Chaque refus produit une ligne de journal structurée portant la tâche, le PID,
   l'agent, la session et le `trace_id`.
8. Les deux chemins opérateur — CLI `mika tasks cancel` et
   `POST /tasks/{id}/cancel` — restent inchangés, et c'est garanti par
   construction (la garde ne vit pas dans `cancel_task_and_kill`), pas par un
   prédicat.

## Hors périmètre — décisions, pas des reports

**L'option 2 du ticket (un terme de cible).** Rejetée avec ses raisons en D5 :
mika#2653 a tranché que le remède est un terme de vivacité, `ToolContext` ne porte
pas l'identité du ticket du tour, et un terme de cible aurait laissé passer
l'incident sœur mika#2739. Ce n'est pas un report.

**Une garde dans `cancel_task_and_kill` ou dans le handler HTTP.** Refusée par
construction (D2) : elle retirerait aux deux chemins opérateur le geste que ce
plan préserve précisément en ne touchant pas cette couche. C'est aussi ce qui
distingue cette garde d'une garde d'infrastructure, et la propriété est déjà
écrite au site.

**Fusionner `is_webhook_pr_event_turn` et `is_webhook_fallthrough_turn` en un
`WebhookTurnClass`.** Suivi nommé de longue date (21 sites de construction), et ce
plan le rend **moins** urgent côté `cancel_task` en supprimant un lecteur plutôt
qu'en en ajoutant un. Appartient à son propre ticket.

**Une staleness calculée par le moteur pour les callbacks de groom.** Le ticket
l'évoque (« aucun tour de pilote depuis N minutes »). Le moteur en porte déjà
quatre — watchdog PID #959, faucheur de stalle silencieuse #2249/#2277, balayage
fantôme #1712, `timeout_at` à 6 h — et mika#2762 vient de **rollbacker** un
faucheur de groom parce qu'il faisait échouer des grooms réussis quand la place
implement était occupée. Ajouter un cinquième calcul de staleness dans le même
vol que le refus serait ajouter la classe de défaut qu'on vient de retirer.
Recommandation à l'opérateur : si un besoin subsiste après ce vol, il se pose
contre les quatre mécanismes existants, pas à côté d'eux.

**Fermer `LivePilotVerdict::Unreadable` en enum de causes.** Rendrait exhaustif le
`match` de `unreadable_audit_value` et retirerait son `debug_assert_eq!` plus son
repli. Déjà nommé comme suivi par mika#2653 ; ce plan ne change ni la forme ni le
nombre d'appelants.
