# mika#2944 — un 402 de crédit suspend la reprise au lieu de la rejouer

> Ticket : senara-solutions/mika#2944 (`bug`, `p2-normal`, `dispatch:loop`, `loop-substrate`).
> Branche : `fix/2944/callback-de-build-un-402-de-cr-dit`.
> Un seul vol. La justification de ce choix est en § *Taille estimée*.

## 1. Le défaut, et où il vit exactement

L'échelle de livraison des callbacks de mika#2179 traite **toute** classe d'erreur
de la même façon : elle compte la tentative, range la classe, calcule un backoff
exponentiel et réécrit `next_fire_at`. Le site est
`crates/mika-agent/src/task_engine/dispatcher.rs:2998-3149`
(`record_callback_delivery_failure`), appelé depuis le bras `Err` de
`run_silent_agent` (`dispatcher.rs:1586-1617`) quand `task.trigger_type == "callback"`.

La classification, elle, est honnête : `classify_delivery_error`
(`dispatcher.rs:599-601`) délègue à `mika_common::llm::error::classify_anyhow_error`,
qui descend la chaîne `anyhow` par `downcast_ref::<LlmError>()` et rend
`error_class::http(402)` → `"http_402"`. La donnée est **déjà là**. Ce qui manque,
c'est qu'elle **décide** de quelque chose : aujourd'hui `error_class` est écrite dans
`audit_events.after_value` et dans `delivery_last_error_class`, et rien ne la lit pour
arbitrer le délai.

Conséquence mesurée (tâche 26b8d5c7, build de mika#2920, 2026-10-28) : quatre
tentatives, 60 tours de revue rejoués, aucune ne pouvait aboutir, et chacune a
consommé un peu du crédit restant — solde à 0,16 $ à 08:07Z. Le verdict n'est parti
qu'à 09:05:44Z après recharge manuelle du crédit vers 08:51Z, avec
`callback_delivery_slow` (`wait_secs=7743`).

Trois propriétés du code actuel qui cadrent la correction :

1. **Le 402 n'est pas rejoué dans l'appel.**
   `crates/mika-common/src/llm/openai.rs:283` pose
   `let retryable = matches!(status_code, 429 | 500 | 503);` — 402 n'y est pas. Il
   n'y a donc **aucune** amplification intra-appel : le gaspillage est entièrement
   au niveau du dispatcher, et il vaut *une revue complète par tentative*.
2. **`next_fire_at` EST le minuteur.** Le scan moteur de 60 s
   (`DB_SCAN_INTERVAL_TICKS`) re-sélectionne la ligne dès qu'elle est dépassée, via
   `dispatch_undelivered_callbacks`. Changer le délai, c'est écrire `next_fire_at` :
   pas de colonne, pas de table, pas de migration.
3. **Il existe déjà un précédent de porte pré-verrou** :
   `skip_superseded_build_callback` (`dispatcher.rs:1253-1332`), appelée depuis
   `dispatch_resume_agent` (`dispatcher.rs:1334-1420`) **avant** `agent_lock.try_lock()`.
   Un tour qui n'a pas lieu n'a besoin ni du verrou d'agent ni d'un décompte de
   `AgentBusy` : elle rend `Ok(())`. C'est la forme exacte dont la libération a besoin.

## 2. Ce que le ticket demande, et la seule question qu'il laisse ouverte

AC1 veut deux choses distinctes qu'il faut ne pas confondre : **classer à part**
(un prédicat) et **ne pas relancer une revue complète tant que le crédit n'a pas
changé** (un mécanisme d'attente). AC2 veut la reprise automatique. AC3 veut un
contrôle négatif sur 429/5xx.

La question que le ticket ne tranche pas : *comment sait-on que le crédit a changé ?*
OpenRouter ne pousse aucun signal vers mika. Les deux seules réponses disponibles
sont (a) attendre un intervalle et re-tenter, (b) sonder. Le plan prend (b) **au
prix d'une sonde**, parce qu'une sonde d'un appel coûte quelques centimes de jeton
contre une revue de 28 tours, et il nomme en § 7 ce que la sonde ne prouve pas.

## 3. Décisions de conception

### D1 — Le prédicat lit la VARIANTE, et le vocabulaire de fil ne bouge pas

Ajout dans `crates/mika-common/src/llm/error.rs` :

```rust
/// Le 402 d'un fournisseur signifie « cette requête dépasserait ton crédit » :
/// un budget épuisé, pas une panne. Lu sur la VARIANTE, jamais sur le message
/// (doctrine mika#2331 : le texte d'un fournisseur n'est pas un contrat).
#[must_use]
pub fn is_credit_exhausted(err: &LlmError) -> bool {
    matches!(err, LlmError::HttpError { status: 402, .. })
}

/// Variante qui descend la chaîne `anyhow`, par symétrie avec
/// [`classify_anyhow_error`] : l'erreur remonte du tour enveloppée.
#[must_use]
pub fn is_credit_exhausted_anyhow(err: &anyhow::Error) -> bool {
    err.downcast_ref::<LlmError>().is_some_and(is_credit_exhausted)
}
```

**Ce qui N'est PAS fait, délibérément :** ni renommage ni scission de la classe de
fil `http_402`. Ces sept chaînes sont un **format de fil** (mika#2331 D3) : elles
atterrissent dans `audit_events.after_value` et les opérateurs les `GROUP BY`. Une
huitième orthographe scinderait silencieusement une population existante
(mika#2205). Ce qui est nouveau, c'est la **décision**, et elle vit à un seul site
nommé.

*Contingence, si la variante ne survivait pas à la chaîne `anyhow` en production :*
comparer `classify_anyhow_error(err) == error_class::http(402)` **à ce même site
unique** — jamais un `contains("credits")` sur le message.

### D2 — La retenue : un intervalle configurable qui remplace l'échelle, pas qui s'y ajoute

Dans `record_callback_delivery_failure`, un 402 de crédit prend une branche qui
**remplace le calcul de backoff** par un intervalle de retenue (défaut 900 s) au
lieu de l'échelle 60 / 120 / plafond. Tout le reste est préservé :

| ce que fait la branche crédit | pourquoi |
|---|---|
| incrémente `delivery_attempts` comme avant | la surface opérateur de mika#2179 reste comparable d'un épisode à l'autre |
| écrit `delivery_last_error_class = "http_402"` comme avant | format de fil intact (D1) |
| estampille `delivery_credit_held_at = now()` | marque l'entrée en retenue |
| incrémente `delivery_credit_probes` | combien d'intervalles cet épisode a coûté |
| émet l'événement d'audit `callback_delivery_credit_held` + un WARN de même nom | une retenue qu'on ne peut pas lire n'est pas une retenue |
| écrit `next_fire_at = now + hold_secs`, **une seule fois** | `next_fire_at` est le minuteur (§ 1.2) |
| **conserve** l'événement `callback_delivery_failed` | l'échec a bien eu lieu ; le supprimer trouerait l'instrument existant |

La quarantaine reste gouvernée par son bloc existant
(`counter_written && attempts >= max_attempts && previous_attempts < max_attempts`) :
une retenue longue finira par estampiller `delivery_quarantined_at`, et c'est
correct — la quarantaine dit « le budget de tentatives est épuisé », ce qui est
vrai. Ce qui change, c'est que la ligne n'est plus **perdue** : voir D4.

### D3 — La libération : une porte pré-verrou qui sonde

Nouvelle méthode sur `TaskDispatcher`, modelée sur `skip_superseded_build_callback`
et appelée **juste à côté d'elle**, avant `agent_lock.try_lock()` :

```rust
if is_callback && self.skip_superseded_build_callback(task).await { return Ok(()); }
if is_callback && self.hold_callback_on_exhausted_credit(task).await { return Ok(()); }
```

**L'ordre est porteur, et c'est celui-là.** La supersession est un test sur la base,
gratuit ; la retenue peut coûter un appel LLM. On ne sonde donc pas le crédit pour
une ligne que la supersession allait classer de toute façon. Inverser les deux
lignes dépense une sonde par ligne superseded.

Corps de la porte :

1. Si `credit_hold::is_held(metadata) == false` → rend `false`, le tour a lieu.
   C'est le chemin de **toute** ligne non retenue : coût nul, aucun appel.
2. Si l'intervalle de retenue vaut `0` → rend `false` (interrupteur STOP, D6).
3. Sonde : `self.llm.send_message(&probe)` avec
   ```rust
   LlmRequest {
       model: self.llm.model_name().to_string(),
       system: None,
       messages: vec![LlmMessage { role: LlmRole::User, content: LlmContent::Text("ok".into()) }],
       tools: None,
       max_tokens: self.llm.max_tokens(),
       thinking: None,
   }
   ```
   `max_tokens` vient de `self.llm.max_tokens()` et **pas** d'un petit littéral :
   le contrôle de crédit d'OpenRouter est un pré-vol sur le coût *maximal* de la
   requête, donc une sonde à 16 jetons passerait là où la revue est refusée. La
   sonde doit porter la même enveloppe que l'appel réel.
   Le constructeur du `LlmRequest` est fourni par `credit_hold::probe_request(&dyn LlmProvider)`,
   pour que la forme soit testable sans dispatcher.
4. **Succès** → estampille `delivery_credit_released_at = now()`, émet
   `callback_delivery_credit_restored` (audit + INFO, avec `held_secs` et
   `probes`), rend `false` : la revue repart dans le même tour. AC2.
5. **Erreur de crédit** (`is_credit_exhausted_anyhow`) → re-retient un intervalle
   (même écriture que D2 : `held_at`, `probes`, `next_fire_at`), émet
   `callback_delivery_credit_held`, rend `true`.
6. **Erreur inconcluante** (transport, 5xx, parse…) → **re-retient aussi**, sous son
   propre WARN `callback_delivery_credit_probe_inconclusive` portant la classe
   d'erreur. Fail-closed assumé : une sonde inconcluante n'est pas une preuve que
   le crédit est revenu. Attendre coûte un intervalle, deviner coûte une revue
   complète. La classe est sur la ligne, donc une sonde qui échoue toujours pour
   une autre raison est *lisible* plutôt que confondue avec un crédit épuisé
   (mika#2205).

### D4 — `classify_undelivered_verdict` gagne un cinquième motif

`crates/mika-agent/src/qa_build_callback.rs:390-499` répond à « pourquoi le verdict
n'est-il pas arrivé ? » par quatre motifs testés **du plus spécifique au moins** :
`delivery_quarantined_at` présent → `Quarantined` ; `delivery_attempts > 0` →
`TurnFailed` ; `verdict_delivery_deferrals > 0` → `AgentBusyStarvation` ; sinon
`NeverAttempted`.

Une ligne retenue pour crédit lirait aujourd'hui `TurnFailed`, puis `Quarantined`
dès que l'estampille de quarantaine tombe. Les deux sont **faux au sens où
l'opérateur lit l'instrument** : « quarantined » veut dire *le verdict est
effectivement perdu, le réessai est horaire*, alors qu'une retenue repart toute
seule. Deux populations sous un nom ne sont pas soustractibles (mika#2205). Donc :

```rust
pub enum UndeliveredVerdictCause { CreditHeld, AgentBusyStarvation, TurnFailed, Quarantined, NeverAttempted }
```
avec `as_wire()` → `"credit_held"`, et **testé en premier** dans
`classify_undelivered_verdict`, avant la quarantaine. L'ordre le-plus-spécifique-d'abord
est porteur : c'est une propriété du site, pas un détail de style, et le
commentaire existant le dit déjà.

Le prédicat lu est `credit_hold::is_held(metadata)` — **pas** une lecture de clé.
Un seul lecteur de la représentation (D5), deux consommateurs de la réponse.

### D5 — La retenue est représentée par DEUX estampilles, pas par un drapeau

`crates/mika-agent/src/async_db.rs` n'expose que `set_task_metadata_field` et
`get_task_metadata_field` (lignes 1564 et 1597) : **aucun helper de suppression**.
Un drapeau booléen serait donc inlevable sans ajouter une primitive DB à ce vol.

Représentation retenue : `delivery_credit_held_at` écrite à chaque entrée en
retenue, `delivery_credit_released_at` écrite à chaque libération ; **retenue ⟺
`held_at > released_at`** en comparaison lexicographique, qui est chronologique
parce que `crate::timestamp` écrit de l'ISO 8601 UTC à largeur fixe. `released_at`
absente avec `held_at` présente ⇒ retenue (premier épisode).

Bénéfice collatéral, et c'est ce qui en fait le bon choix plutôt que le choix par
défaut : les deux estampilles laissent une **trace par épisode** dans la ligne, donc
`held_secs` est calculable sur l'événement de libération, alors qu'un drapeau effacé
n'aurait rien laissé.

### D6 — Réglage et interrupteur STOP

Dans `crates/mika-common/src/config.rs`, à côté des trois champs de mika#2179 et
dans leur idiome exact :

```rust
/// Intervalle de retenue après un 402 de crédit (mika#2944).
/// Env override: MIKA_CALLBACK_DELIVERY_CREDIT_HOLD_SECS
/// `0` désarme la retenue et restaure l'échelle de mika#2179.
#[serde(default)]
pub callback_delivery_credit_hold_secs: Option<u64>,
```

```rust
pub const DEFAULT_CALLBACK_DELIVERY_CREDIT_HOLD_SECS: u64 = 900;

#[must_use]
pub fn effective_callback_delivery_credit_hold_secs(&self) -> u64 {
    match self.callback_delivery_credit_hold_secs {
        None => DEFAULT_CALLBACK_DELIVERY_CREDIT_HOLD_SECS,
        Some(0) => 0, // no-op déclaré, pas un plancher
        Some(n) => n.clamp(1, CALLBACK_DELIVERY_BACKOFF_ABSOLUTE_MAX_SECS),
    }
}
```

**Les deux bouts du clamp sont porteurs**, pour la raison déjà écrite dans
`docs/operator/environment-variables.md:2448` pour les deux bornes voisines : un
plafond absurde atteint `chrono::Duration::seconds`, qui **panique** au-delà de
`i64::MAX/1000` et tuerait la tâche de livraison dans la fonction qui s'apprêtait à
écrire son `next_fire_at`.

**`0` est un no-op déclaré, pas un intervalle de zéro seconde** (doctrine mika#1833) :
c'est l'interrupteur STOP. La porte de D3 rend `false` avant de sonder, la branche
de D2 n'est pas prise, et la ligne retombe sur l'échelle de mika#2179 — exactement
le comportement d'aujourd'hui. 900 s plutôt que 3600 s parce que le délai de
recharge mesuré (06:58Z → 08:51Z) est de l'ordre de deux heures : à 900 s la
latence ajoutée après le retour du crédit est bornée par un quart d'heure, pour
une sonde toutes les 15 min, soit ~8 sondes sur l'épisode mesuré contre 60 tours
de revue.

### D7 — Où vit le code : un module, pas une rallonge de `dispatcher.rs`

`dispatcher.rs` fait 10 351 lignes et la garde de CI plafonne un fichier à 1 Mo
(`scripts/check-secrets.sh`, cf. mika#2321). La logique **de décision** va donc dans
un nouveau `crates/mika-agent/src/credit_hold.rs` (précédent :
`qa_head_supersession.rs`), déclaré `pub mod credit_hold;` dans `lib.rs` :

- les trois constantes de clé de métadonnée ;
- `is_held(metadata: Option<&str>) -> bool` — **le seul lecteur** de la représentation
  à deux estampilles (D5) ;
- `probe_request(llm: &dyn LlmProvider) -> LlmRequest` ;
- ses tests unitaires.

Reste dans `dispatcher.rs` ce qui a besoin de `self` : la branche de
`record_callback_delivery_failure` et la porte `hold_callback_on_exhausted_credit`.

## 4. Livrables et ordre de commit

L'ordre est choisi pour que chaque commit compile et que la suite soit verte à
chaque étape.

1. `crates/mika-common/src/llm/error.rs` — `is_credit_exhausted` +
   `is_credit_exhausted_anyhow` + leurs tests (dont le contrôle négatif 429/500/503).
2. `crates/mika-common/src/config.rs` — champ, constante, accesseur, test de clamp
   et test du `0`.
3. `crates/mika-agent/src/credit_hold.rs` + `pub mod credit_hold;` dans `lib.rs` —
   clés, `is_held`, `probe_request`, tests unitaires.
4. `crates/mika-agent/src/qa_build_callback.rs` — cinquième motif `CreditHeld`,
   `as_wire()`, ordre de test, deux tests (le motif, et le contrôle négatif d'une
   ligne quarantainée sans retenue).
5. `crates/mika-agent/src/task_engine/dispatcher.rs` — la branche crédit de
   `record_callback_delivery_failure`, la porte
   `hold_callback_on_exhausted_credit`, son appel dans `dispatch_resume_agent`
   après `skip_superseded_build_callback`.
6. `crates/mika-agent/tests/eval/test_credit_hold_2944.rs` + son `pub mod` dans
   `tests/eval/main.rs` — les trois cas de bout en bout.
7. Documentation (hors décompte de lignes) :
   `docs/operator/environment-variables.md` (la variable, son défaut, son
   interrupteur STOP, ses trois sondes), `.env.example`,
   `docs/agent-internals/unified-task-engine.md` (la retenue dans l'échelle de
   livraison, le cinquième motif, la sonde et son résidu).

## 5. Contrat de vérification

### Tests unitaires

| site | ce qu'il fixe |
|---|---|
| `error.rs` | `HttpError{402}` ⇒ vrai ; **429, 500, 503, `Transport`, `ParseError` ⇒ faux** (AC3 au niveau du prédicat) ; un 402 enveloppé dans deux couches `anyhow::Context` ⇒ vrai (c'est la forme réelle dans laquelle l'erreur remonte du tour) |
| `config.rs` | absent ⇒ 900 ; `0` ⇒ `0` et **non** 1 ; `u64::MAX` ⇒ `CALLBACK_DELIVERY_BACKOFF_ABSOLUTE_MAX_SECS` |
| `credit_hold.rs` | `held_at` seule ⇒ retenue ; `held_at > released_at` ⇒ retenue ; `released_at > held_at` ⇒ libre ; métadonnée absente / JSON invalide ⇒ libre (fail-open : une métadonnée illisible ne doit pas retenir une ligne pour toujours) ; `probe_request` porte bien `max_tokens == llm.max_tokens()` |
| `qa_build_callback.rs` | une ligne retenue **et** quarantainée rend `CreditHeld`, pas `Quarantined` (c'est l'ordre qui est testé, pas le motif) ; une ligne quarantainée sans retenue rend toujours `Quarantined` |

### Test eval — `tests/eval/test_credit_hold_2944.rs`

Gabarit : `tests/eval/test_callback_delivery_starvation.rs` (407 lignes) —
`NoopSender`, `test_db()`, `test_home()`, `dispatcher_with_llm(db, home, llm)`
construisant le littéral `TaskDispatcher` avec `cli_mode: false` (porteur) et
`agent_lock: None`, `seed_starved_callback(db)`, helpers `next_fire_at_of` /
`status_of`, assertions via `count_audit_events_by_tool_name` /
`get_audit_event_rows_by_tool_name`.

Trois cas :

1. **`un_402_de_credit_retient_la_ligne_au_lieu_de_la_rejouer`** —
   `MockLlmProvider::builder().error(LlmError::HttpError { status: 402, … })`,
   une ligne de callback semée. Après `dispatcher.dispatch(&task_id)` :
   un `callback_delivery_credit_held`, `delivery_credit_held_at` posée,
   `next_fire_at ≈ now + 900` (et **non** `now + 60`). Second `dispatch` :
   la porte rend `Ok(())`, `llm.calls_made()` a augmenté de **1** (la sonde) et
   non de 28, et `callback_delivery_credit_held` est passé à 2.
2. **`le_retour_du_credit_relance_la_revue_sans_geste_a_la_main`** (AC2) —
   même semis, puis `llm.clear_and_set(deux réponses texte)` avant le second
   `dispatch` (deux, parce que la garde EndTurn `callback_terminal_action` refuse
   la première). Attendu : `callback_delivery_credit_restored`,
   `delivery_credit_released_at` posée, et `callback_delivered`.
3. **`un_429_garde_le_backoff_de_mika2179`** (AC3, contrôle négatif) — même
   semis, `LlmError::HttpError { status: 429, … }`. Attendu :
   **zéro** `callback_delivery_credit_held`, `delivery_credit_held_at` absente,
   `next_fire_at ≈ now + 60`.

Le cas 3 est le contrôle qui sépare « la branche crédit est bien conditionnée » de
« la branche crédit est prise par toute erreur HTTP ». Sans lui, les cas 1 et 2
seraient verts sur un `matches!(err, LlmError::HttpError { .. })` sans le `402`.

### Recette de vérification d'injection (obligatoire, doc de module)

Le gabarit la porte et elle est reprise telle quelle : un test eval qui construit
le littéral `TaskDispatcher` peut être vert **sans que le `MockLlmProvider` soit
jamais atteint** (la ligne classée avant le tour, un `cli_mode` qui court-circuite,
un `trigger_type` qui n'entre pas dans la branche callback). La doc de module
inscrit donc la recette : avant d'asserter quoi que ce soit, asserter
`llm.calls_made() > 0` sur le premier `dispatch`, et faire **échouer volontairement
une fois** le cas 1 en mettant `status: 429` à la place de `402` — s'il reste vert,
l'injection ne passe pas et l'assertion ne mesure rien.

### Commandes

```
cargo test -p mika-common llm::error
cargo test -p mika-common config
cargo test -p mika-agent --lib credit_hold
cargo test -p mika-agent --lib qa_build_callback
cargo test -p mika-agent --test eval credit_hold_2944
cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check
scripts/verify-pipeline.sh origin/main
```

## 6. Refus de périmètre — ce sont des REFUS, pas des reports

Écrits comme refus avec leur raison, **sans ligne `Tracked in:`**, pour ne pas
déclencher la garde de report de mika#527 sur le corps de PR.

- **Pas de notification P0 à l'opérateur.** Parité avec mika#2179, dont l'étage
  d'instrument est exactement audit + WARN + métadonnée ; ce n'est pas un critère
  d'acceptation de mika#2944 ; et la borne par vol morderait. Un épisode de
  retenue est *lisible* (trois événements nommés, trois clés de métadonnée) et
  *auto-résolutif* — ce n'est pas la définition d'un P0 (« la boucle est morte, un
  humain doit intervenir »).
- **Pas de garde de « lecteur unique » par scan de source** sur
  `credit_hold::is_held`. Le module n'a que deux consommateurs, tous deux écrits
  par ce vol ; la garde protègerait une population qui n'existe pas encore, pour
  ~35 lignes prises sur une borne serrée.
- **Pas de trace en commentaire de PR.** Le ticket note que « la PR ne porte aucune
  trace de l'échec », mais c'est le constat d'un symptôme, pas un critère : les
  trois AC portent sur le comportement de reprise. Un commentaire de PR est une
  surface GitHub, donc un autre axe (idempotence, bruit, droits) qui ne tient pas
  sous cette borne.
- **Pas de retouche du vocabulaire de fil `error_class`** (D1), pour la raison
  qui y est écrite.

## 7. Résidu honnête — ce que la sonde ne prouve pas

**La sonde est un indice, pas une preuve.** Le contrôle de crédit d'OpenRouter est
un pré-vol sur le coût maximal de la requête, donc avec `max_tokens` égal à celui
de l'appel réel elle est *proche* de la revue — mais pas identique : la revue
enchaîne N appels, dont le cumul peut dépasser ce que le solde couvre alors que le
premier passe. Au solde mesuré de 0,16 $, un faux positif reste donc possible.

Sa conséquence est bornée et nommée : **une** revue rejouée, exactement le
comportement d'aujourd'hui pour cette tentative-là, après quoi le 402 retombe dans
la branche de D2 et la retenue se réarme. Le gaspillage passe donc de *une revue
par minute de backoff* à *au plus une revue par intervalle de retenue*. C'est la
propriété que ce vol livre, et elle est plus faible que « plus jamais de revue
inutile » — qui exigerait une lecture du solde chez le fournisseur, donc une API
de facturation, donc un autre ticket.

Second résidu : `delivery_attempts` continue d'être incrémenté par les retenues,
donc une retenue longue finit par franchir le seuil de quarantaine. C'est
volontaire (D2) — mais cela veut dire qu'un épisode de retenue consomme le budget
de tentatives qu'un échec transitoire ultérieur ne retrouvera pas. `delivery_credit_probes`
est ce qui permet de **lire** cette consommation plutôt que de la découvrir.

## 8. Definition of Done

- [ ] `is_credit_exhausted` / `is_credit_exhausted_anyhow` dans
      `mika-common::llm::error`, lisant la variante, avec contrôles négatifs
      429 / 500 / 503 / transport / parse.
- [ ] `callback_delivery_credit_hold_secs` + `DEFAULT_… = 900` +
      `effective_…()` avec clamp aux deux bouts et `0` traité comme no-op déclaré.
- [ ] `crates/mika-agent/src/credit_hold.rs` créé et déclaré dans `lib.rs` :
      clés, `is_held` (lecteur unique de la représentation), `probe_request`, tests.
- [ ] Branche crédit dans `record_callback_delivery_failure` : estampille,
      compteur de sondes, `callback_delivery_credit_held` (audit + WARN),
      `next_fire_at` écrit **une seule fois**, `callback_delivery_failed` préservé.
- [ ] Porte `hold_callback_on_exhausted_credit` dans `dispatch_resume_agent`,
      **après** `skip_superseded_build_callback` et **avant** `agent_lock.try_lock()`,
      rendant `Ok(())` sur retenue, avec ses trois issues (libéré / re-retenu /
      inconcluant, chacune sous son propre nom).
- [ ] `UndeliveredVerdictCause::CreditHeld` (`as_wire()` = `"credit_held"`) testé
      en premier dans `classify_undelivered_verdict`, via `credit_hold::is_held`.
- [ ] `tests/eval/test_credit_hold_2944.rs` : les trois cas, la recette de
      vérification d'injection dans la doc de module, l'incident mesuré en en-tête.
- [ ] `MIKA_CALLBACK_DELIVERY_CREDIT_HOLD_SECS` documentée dans
      `docs/operator/environment-variables.md` (défaut, clamp, interrupteur STOP,
      les trois sondes) et listée dans `.env.example`.
- [ ] La retenue, le cinquième motif et le résidu du § 7 écrits dans
      `docs/agent-internals/unified-task-engine.md`.
- [ ] `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`
      et `scripts/verify-pipeline.sh origin/main` verts.
- [ ] Corps de PR portant `Closes #2944` et
      `Code review: delegated to mika-qa (mika#2694)`.

## 9. Acceptance criteria

Transcrits depuis `## Critères d'acceptation (brouillon)` du corps de mika#2944.

- **AC1.** Un 402 de crédit est classé à part des pannes passagères. La reprise ne
  relance pas une revue complète tant que le crédit n'a pas changé : elle est
  suspendue, ou attend un signal.
- **AC2.** Le résultat du build reste préservé, et le verdict repart dès que le
  crédit revient, sans geste à la main.
- **AC3.** Contrôle négatif : un 5xx ou un 429 garde le backoff actuel.

Où chacun est tenu :

| AC | tenu par | attesté par |
|---|---|---|
| AC1 | D1 (le prédicat) + D2 (la retenue à 900 s au lieu de 60/120/3600) + D3 étape 1 (aucun tour sur une ligne retenue) | tests unitaires `error.rs`, cas eval 1 |
| AC2 | D3 étapes 4 (libération + reprise dans le même tour) ; la ligne n'est jamais annulée, `status` et métadonnée du build intacts | cas eval 2 |
| AC3 | D1 (la variante, `402` exact) ; 429/5xx ne prennent pas la branche et gardent `delivery_backoff_secs` | contrôles négatifs `error.rs`, cas eval 3 |

## Fire-Disposition

Ce plan livre des détecteurs : les tests unitaires des quatre sites et les trois cas
eval. Leur chemin de succès est « aucune violation trouvée », donc la section est
requise.

**Option retenue : (a) exception nommée en allowlist — avec ZÉRO entrée d'exception,
et les détecteurs atterrissent ARMÉS ET VERTS.**

Pourquoi aucune entrée n'est nécessaire, et pourquoi ce n'est pas l'option (b)
déguisée : la population que ces détecteurs examinent est **créée par ce vol**. Le
prédicat `is_credit_exhausted`, la clé `delivery_credit_held_at`, le motif
`CreditHeld`, l'intervalle de retenue — rien de cela n'existe avant le commit 1. Il
n'y a donc aucune violation préexistante à excepter : le premier état du dépôt où
ces détecteurs peuvent s'exécuter est celui où ils sont verts par construction.

Conséquence opérationnelle, qui est ce qui rend la disposition vérifiable : **si
l'un de ces tests est rouge à la fin de l'implémentation, c'est le code de ce vol
qui est faux, jamais une donnée héritée.** Aucun `#[ignore]`, aucune constante
d'allowlist, aucun ticket de suivi d'armement — ajouter l'un des trois serait
désarmer un détecteur dont rien ne justifie le désarmement.

Le détecteur que ce vol **refuse** de livrer est la garde de lecteur unique par scan
de source sur `credit_hold::is_held` (§ 6) : c'est un refus argumenté, pas un
détecteur désarmé, et il ne relève donc pas de cette section.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-common/src/llm/error.rs` (prédicat variante + chaîne anyhow + tests) | 50 |
| `crates/mika-common/src/config.rs` (champ, constante, accesseur, tests de clamp) | 40 |
| `crates/mika-agent/src/credit_hold.rs` (clés, `is_held`, `probe_request`, tests) | 110 |
| `crates/mika-agent/src/qa_build_callback.rs` (5ᵉ motif `CreditHeld` + 2 tests) | 30 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` (branche crédit + porte pré-verrou) | 105 |
| `crates/mika-agent/src/lib.rs` (`pub mod credit_hold;`) | 1 |
| tests (`crates/mika-agent/tests/eval/test_credit_hold_2944.rs` + `pub mod`) | 85 |

Total estimé : 421 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 421 < 429 (borne par vol) et
421 < 1000 (seuil par PR) ; la borne par vol est la plus serrée, et c'est elle qui
est à huit lignes. Trois propriétés rendent cette estimation crédible plutôt
qu'optimiste :

1. **Aucune migration, aucune colonne, aucune table.** `next_fire_at` est déjà le
   minuteur et `tasks.metadata` porte déjà cinq clés de la même famille (§ 1.2).
   Le poste qui a tué mika#2161 et mika#2633 — une couche de persistance neuve —
   est absent.
2. **La forme est copiée, pas inventée.** La porte pré-verrou a un précédent exact
   (`skip_superseded_build_callback`, 80 lignes), le réglage a un précédent exact
   (les trois champs de mika#2179 avec leur clamp), le test eval a un gabarit exact
   (`test_callback_delivery_starvation.rs`). Les 105 lignes de `dispatcher.rs` sont
   une branche dans une fonction existante plus une méthode calquée sur sa voisine.
3. **Le découpage le plus évident est refusé parce qu'il ne serait pas mergeable
   seul.** Un vol « prédicat + réglage » (90 lignes) ne change aucun comportement :
   il livrerait du code mort, et aucun AC. Un vol « retenue sans libération »
   (D1+D2 sans D3) suspendrait la reprise **sans jamais la reprendre** — pire que le
   défaut. Les trois AC sont solidaires : AC1 sans AC2 est une régression. Donc un
   vol, et la marge de huit lignes est le poste à surveiller, pas à ignorer.

**Si l'implémentation dépasse la borne en cours de route**, le poste à sacrifier est
nommé d'avance : les deux tests de `qa_build_callback.rs` (D4), dont le motif
`CreditHeld` reste livré — l'attribution devient alors attestée par le seul cas eval 2,
qui le traverse. Ne pas sacrifier le cas eval 3 : c'est le contrôle négatif d'AC3.
