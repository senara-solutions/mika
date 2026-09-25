# mika#2522 — Un tour tué par le transport n'est pas un refus du serveur

> **Ticket** : senara-solutions/mika#2522 — « Relais OpenRouter : une coupure
> `body read` perd un groom »
> **Milestone** : mika#2491
> **Voisins** : mika#2189 (budgets timeout), mika#2342 (watchdog LLM),
> mika#2278 (retry `_arch_ask`), mika#2036 (recovery A2A), mika#2362
> (atteignabilité du retry), mika#2015 (la coupure est du transport)

---

## 0. Ce que la lecture du code déplace dans le ticket

Le ticket a fait la moitié du travail : il a vérifié que le retry **existe** et
qu'il **couvre la signature**, et il a donc refusé l'AC naïve (« couvrir la
signature »). Il reste une hypothèse, écrite comme telle — *« la fenêtre de
back-off (~3,5 s sur 4 essais) est trop courte »* — et une AC qui laisse le
niveau ouvert : *« dans `openai.rs` OU dans `_arch_ask` — décider après avoir
confirmé où la coupure épuise le retry actuel »*.

**La lecture de la source confirme le trou, réfute l'hypothèse, et tranche le
niveau.** Les trois rectifications sont le premier livrable de ce plan, parce
que chacune change le remède.

### R1 — La chaîne complète, maillon par maillon

Le groom ne meurt pas d'un back-off trop court. Il meurt d'une **classification
qui aplatit deux échecs de natures opposées**. La chaîne, à la ligne près :

| # | site | ce qui se passe |
|---|---|---|
| 1 | `llm/openai.rs:346` | le corps se coupe → `LlmError::Transport("failed to read response body: …")`, **retryable** (mika#2015) |
| 2 | `llm/openai.rs:461-628` | la boucle rejoue `max_attempts` fois avec back-off, puis rend l'erreur |
| 3 | `server/a2a.rs:1201` | `run_agent_for_message` rend `Err` → le Task passe `failed`, `TurnText::LoopFailed` |
| 4 | `server/a2a.rs:1222-1248` | le serveur sert un **JSON-RPC `success`** portant un Task `failed` **« avec rien à dire »** |
| 5 | `cli/remote_ask.rs:175` | `terminal_state_class(Failed) = Contract` |
| 6 | `cli/remote_ask.rs:132` | `exit_code_for = 1` (jamais `75`) |
| 7 | `dispatch-lib.sh:5946` | `_arch_ask_with_retry` rejoue **uniquement sur `75`** → `break` sans retry |
| 8 | `_iterate_groom_loop` | `first-pass _arch_ask failed` → **groom perdu** |

Le maillon 4 est le pivot, et il est délibéré : mika#2270 a décidé qu'un tour
échoué est servi `failed` sans texte, *« so the net has neither a text to serve
nor a loss to report »*. Le serveur **sait** que l'échec est du transport — il
tient `e` en main et le journalise — et il n'en transmet rien. Le client ne peut
donc pas reclasser : **l'information existe et ne traverse pas la frontière.**

### R2 — La justification écrite du `Contract` est fausse pour cette classe

`terminal_state_class` porte sa raison en commentaire :

> `TaskState::Failed | Canceled | Rejected => FailureClass::Contract`
> *« A turn the server actually ran and ended without an answer. Re-sending the
> same brief does not change that verdict. »*

Vrai d'un refus raisonné. **Faux d'un tour tué par une coupure de transport** :
rien n'a été refusé, et renvoyer le même brief a toutes les chances d'aboutir —
c'est la définition même de `is_retryable()`. Le même commentaire nomme déjà une
exception de cette forme, la parenthèse sur le `failed` de `startup_recovery`,
qui *« n'atteint jamais ce site : son échange est mort au transport »*. Le cas
de mika#2522 est le **jumeau** de cette exception, à ceci près que son échange,
lui, a survécu — c'est exactement pourquoi il tombe dans le mauvais bras.

### R3 — Deux cas, un seul est le défaut, et c'est l'arithmétique qui les sépare

Le client A2A borne son envoi à `DEFAULT_TIMEOUT = 600 s`
(`mika-a2a/src/client.rs:24`), planché à `MIKA_AGENT_TOTAL_TIMEOUT_SECS` **du
process CLI** — jamais le `config.toml` de mika-arch, que le CLI ne lit pas.
Côté serveur, mika-arch tourne à `240/900`, donc
`max_attempts = floor(900/240) = 3`.

**Cas A — la coupure arrive au plafond.** Chaque tentative coûte ~240 s, la
chaîne ~720 s > 600 s : **le CLI expire d'abord** → `A2aError::ClientError` →
recovery `tasks/get` (mika#2036) → le Task est encore `Working` →
`Recovery::StillRunning` → `Transport` → exit `75` → **le retry `_arch_ask`
s'arme déjà.** Ce cas est couvert, par la composition de mika#2036 et mika#2278.

**Cas B — la coupure arrive tôt.** Les 3 tentatives s'épuisent en quelques
secondes, le serveur rend son Task `failed` en `200 OK` **bien avant** 600 s →
`Contract` → exit `1` → **aucun retry** → groom perdu.

**« 11 coupures → 5 grooms perdus » est précisément cette partition** : celles
absorbées par le retry client (blip plus court que la fenêtre de back-off),
celles du cas A (déjà rattrapées), celles du cas B (perdues). Le ticket cherchait
pourquoi 5 sur 11 et non 11 sur 11 ; c'est ça.

### R4 — Allonger le back-off est sans effet dans un cas et contre-indiqué dans l'autre

AC1 propose `2 / 4 / 8 s` (≈14 s). Quatre mesures s'y opposent, et la dernière
est la plus lourde.

1. **La valeur citée n'est pas celle qui tourne.** `~3,5 s sur 4 essais` est le
   back-off au *hard cap* (`DEFAULT_ATTEMPTS_HARD_CAP = 4`). À la géométrie de
   mika-arch, `max_attempts = 3`, donc le back-off réel est
   `500 ms + 1 000 ms = **1,5 s**`.
2. **Dans le cas A il n'achète rien.** La fenêtre inter-tentatives y est déjà de
   ~480 s (deux plafonds). Y ajouter 6 s ne change pas un ordre de grandeur.
3. **Dans le cas B il achète 4,5 s** (1,5 → 6 s) — contre les **30 s** que
   `_arch_ask_with_retry` offre déjà par défaut, sur une **session fraîche** et
   avec un **budget de tour neuf**. Un facteur 5 de moins, pour du code à
   écrire, contre du code déjà écrit, déjà testé, déjà muni de son kill-switch.
4. **Et il peut retirer une tentative.** Le back-off consomme l'enveloppe que la
   garde de deadline mesure (`deadline_verdict`, `openai.rs:489`). Le seuil
   non-transport vaut exactement `1,0 × plafond` et
   `effective_max_attempts` (mika#2362) documente que la dernière tentative
   nominale est déjà inatteignable dès que l'enveloppe est un multiple du
   plafond. Rallonger les pauses rapproche la marge du seuil : sur une géométrie
   voisine, **le remède proposé supprime la tentative qu'il prétend protéger.**

**Donc AC1 se résout au niveau `_arch_ask`** — et pas en allongeant un back-off,
mais en réparant la classification qui empêche le retry demandé par AC1 de
s'armer. *Le retry qu'AC1 décrit existe, avec une fenêtre cinq fois plus large
que celle qu'AC1 demande ; il ne s'arme simplement jamais sur cette classe.*

### R5 — AC2 n'est pas un compteur à inventer, c'est un fait à faire traverser

`llm_call_attempt` (mika#2331) porte déjà `provider`, `model`, `attempt`,
`outcome`, `error_class` : le taux de **retries** par modèle est lisible. Ce qui
n'existe nulle part, c'est le compte des **tours perdus** par modèle et par
classe. Le site de l'échec ne journalise que
`error!(error = %e, task_id = %task_id, "A2A agent loop failed")` — ni `model`,
ni `error_class`, donc incomptable par modèle, et sans ligne `audit_events`
(`mika-common` n'a pas d'accès base, limite que mika#2280 a dû écrire ; le
serveur, lui, l'a).

Or **c'est le même fait que la classe à faire traverser**. Un seul fait, deux
surfaces : les métadonnées pour la décision du client, l'événement plus l'audit
pour la mesure. Écrire les deux depuis un site unique est ce qui garantit qu'ils
ne divergent pas.

---

## 1. Requirements

### R-1 — Le serveur atteste la classe de l'échec de tour

Au site où `message/send` marque un Task `failed` faute d'un tour abouti, la
classe de l'erreur est posée sur le Task, sous une clé `mika.*`, quatrième
membre de la famille `stamp_*`.

- Classification par **`mika_common::llm::error::classify_anyhow_error`**
  (mika#2289) — `downcast_ref` sur toute la chaîne `anyhow`, jamais un
  `contains()` sur un message rendu. Cette fonction a déjà **deux** consommateurs
  (`task_engine::dispatcher:426` et `server::handlers:1181`) et documente que
  *« une seconde copie de ces quatre lignes est très exactement la façon dont une
  population se scinde en deux orthographes sans le dire »*. Ce serait le
  **troisième**, même vocabulaire, aucune duplication.
- Écrite **inconditionnellement sur la branche `Err`**, jamais sur `Ok` : c'est
  une **mesure**, pas la réponse à un drapeau. Son absence dit donc « ce serveur
  n'a rien attesté » et **jamais** « la classe est `contract` » — la même
  asymétrie que `stamp_run_usage` (mika#1883) a dû expliciter face à ses deux
  sœurs inconditionnelles.

### R-2 — Le client lit l'attestation, fail-closed

`terminal_state_class` cesse de décider sur le seul état. Pour `Failed` :

| attestation | classe | exit | retry `_arch_ask` |
|---|---|---|---|
| `transport` / `transport_timeout` | `Transport` | `75` | **oui** |
| toute autre classe (`parse`, `provider`, `http_4xx`, `other`, …) | `Contract` | `1` | non |
| clé absente | `Contract` | `1` | non |
| clé illisible (type inattendu, valeur vide) | `Contract` | `1` | non |

`Canceled` et `Rejected` restent `Contract` **sans consulter l'attestation** :
une annulation et un refus sont des décisions, pas des accidents de transport.

Les trois dernières lignes sont **byte pour byte le comportement d'aujourd'hui**
— fail-closed dans le sens de la maison : *un signal qu'on ne peut pas lire n'est
jamais un terme satisfait.* Le sens du fail-safe suit le coût mesuré déjà écrit
dans `FailureClass` : un faux `Transport` coûte un tour d'architecte payé deux
fois, un faux `Contract` coûte la passe, le créneau de dispatch et un point du
budget de re-drive — dont trois abandonnent un ticket sain (mika#2020).

### R-3 (AC2) — Le fait est comptable par modèle

Le même site écrit, au-delà de l'attestation :

- un événement journal nommé, portant `agent_id`, `task_id`, `model`,
  `error_class`, `session_id` ;
- une ligne `audit_events` sous le même nom, `after_value` = la classe,
  `target_key` = le modèle — pour que
  `GROUP BY` réponde à « quel modèle perd des tours, et sur quelle classe ».

**SOLE WRITER** du nom, tenu par un scan de source (§ 6).

### R-4 (AC3) — Le test négatif est à deux niveaux

Une erreur non-retryable n'est pas rejouée, asserté **côté serveur** (la classe
posée n'est pas transport) **et côté client** (la classe non-transport, comme
l'absence de clé, rend `Contract` → exit `1` → le harnais shell ne compte qu'une
invocation).

### R-5 — Aucune valeur de réglage ne bouge

Ni plafond, ni enveloppe, ni `max_attempts`, ni back-off, ni
`MIKA_ARCH_ASK_RETRY_DELAY_SECS`, ni `config.toml`. Le ticket l'exige pour le
modèle ; R4 l'étend au back-off, avec son argument.

---

## 2. Conception — U1 : l'attestation serveur

### U1a — La clé

`crates/mika-a2a/src/params.rs` :

```rust
/// La classe de l'échec d'un tour que le serveur a marqué `failed` (mika#2522).
///
/// # Pourquoi la classe doit traverser la frontière
///
/// `handle_message_send` sert un tour échoué comme un Task `failed` « sans rien
/// à dire » (mika#2270) — décision correcte pour le rendu, et qui jette la seule
/// information dont l'appelant a besoin pour décider s'il réessaie. Le serveur
/// *tient* la classe (`classify_anyhow_error`) et ne la transmettait pas, donc
/// `terminal_state_class` classait tout `failed` en `Contract`, y compris un tour
/// qu'une coupure de transport venait de tuer. Mesuré le 24/09 : 11 coupures
/// `body read failed mid-stream`, **5 grooms de mika#2515 perdus**.
///
/// # Absence ≠ `contract`
///
/// Écrite **inconditionnellement sur la branche d'échec**, jamais sur un succès.
/// C'est une mesure, pas la réponse à un drapeau : son absence signifie « ce
/// serveur n'a rien attesté » — un binaire antérieur, ou un chemin qui ne passe
/// pas par ce site — et jamais « la classe est `contract` ». Même asymétrie que
/// [`RUN_USAGE_KEY`] face à [`EFFECTIVE_MODEL_KEY`], pour la même raison.
///
/// # Vocabulaire
///
/// Les valeurs sont celles de `mika_common::llm::error_class`, verbatim : c'est
/// déjà le format de fil de `llm_call_attempt`, de
/// `audit_events.callback_delivery_failed` (mika#2179) et du champ `error_class`
/// de `qa_deadline_verdict` (mika#2289). Une quatrième orthographe scinderait une
/// population que trois surfaces comptent déjà ensemble.
pub const TURN_FAILURE_CLASS_KEY: &str = "mika.turn_failure_class";

/// La classe d'échec que le serveur a attestée, si elle l'a été.
///
/// Fail-soft de bout en bout : clé absente, type inattendu ou valeur vide
/// rendent `None`. C'est **l'unique lecteur** de cette clé — un scan de source
/// le tient (§ 6), pour la raison que
/// `mika2220_no_local_reparse_of_the_llm_bodies_env_var` a dû graver une fois :
/// deux lecteurs d'un même fait, c'est deux vérités en puissance.
pub fn attested_turn_failure_class(task: &Task) -> Option<&str> { … }
```

### U1b — Le site, et pourquoi c'est celui-là

`crates/mika-agent/src/server/a2a.rs`, bras `Err(e)` de `handle_message_send`
(l. 1201). Trois raisons de tenir ce site et pas un autre :

1. `e` y est en main, non rendu — la classification lit la **variante**.
2. `task` y est déjà `mut` et `a2a_build_task` est en amont : c'est le point
   d'intervention que `stamp_effective_model`, `stamp_session_isolation` et
   `stamp_run_usage` documentent tous les trois comme *« nothing downstream can
   overwrite the field »*.
3. `agent_state` y est disponible, donc la ligne `audit_events` d'AC2 s'écrit au
   même endroit que l'attestation — **un fait, un site, deux surfaces.**

`TurnText::LoopFailed` devient porteur (`LoopFailed { class: Cow<'static, str> }`)
plutôt qu'une variable parallèle : le `match` sur `turn_text` en aval est le seul
endroit qui sait qu'un tour a échoué, et y faire voyager la classe **dans** la
variante est ce qui empêche un futur éditeur de traiter la branche sans la
classe.

### U1c — Le chemin `message/stream` n'est pas touché, et c'est dit

`handle_message_stream` (l. 1697) a le même bras `Err` et il est **hors
périmètre** : il ne rend pas de `Task` synchrone à estamper, et `_arch_ask`
n'emprunte pas ce chemin (`mika ask` passe par `send_message_to_agent`, donc
`message/send`). Il tombe donc dans la population « non attesté », exactement
comme pour `mika.effective_model` (mika#2304) et `mika.run_usage` (mika#1883) —
population déjà nommée par ces deux tickets, dont celui-ci hérite la borne sans
la modifier.

---

## 3. Conception — U2 : la lecture client

`crates/mika-cli/src/remote_ask.rs`. `terminal_state_class` prend l'attestation
en second argument :

```rust
fn terminal_state_class(state: TaskState, attested_class: Option<&str>) -> FailureClass {
    match state {
        TaskState::Submitted | TaskState::Working | TaskState::Unknown => FailureClass::Transport,

        // mika#2522 — le `failed` a deux causes de natures opposées, et le
        // serveur est le seul à pouvoir les séparer.
        //
        // Le commentaire que cette ligne remplace disait « a turn the server
        // actually ran and ended without an answer. Re-sending the same brief
        // does not change that verdict. » — vrai d'un refus raisonné, faux d'un
        // tour qu'une coupure de transport a tué : rien n'a été refusé, et
        // `LlmError::is_retryable` dit le contraire du verdict. Ce cas est le
        // jumeau de l'exception que la parenthèse ci-dessous nommait déjà.
        //
        // Fail-closed : SEULE une classe transport positivement attestée bascule.
        // Absence de clé (serveur antérieur, `message/stream`), classe illisible,
        // classe non-transport → `Contract`, byte pour byte comme avant.
        TaskState::Failed => match attested_class {
            Some(c) if is_transport_class(c) => FailureClass::Transport,
            _ => FailureClass::Contract,
        },

        // Une annulation et un refus sont des décisions, pas des accidents :
        // l'attestation n'est pas consultée. (L'autre `failed`, celui que
        // `startup_recovery` écrit sur un tour tué par un redémarrage, n'atteint
        // toujours pas ce site : son échange est mort au transport et se lit par
        // `Recovery::Ended` dans le bras `ClientError`.)
        TaskState::Canceled | TaskState::Rejected => FailureClass::Contract,

        TaskState::Completed | TaskState::InputRequired | TaskState::AuthRequired => {
            FailureClass::Contract
        }
    }
}
```

`is_transport_class` compare aux constantes
`mika_common::llm::error::error_class::{TRANSPORT, TRANSPORT_TIMEOUT}`
**importées**, jamais à des littéraux recopiés : `mika-cli` dépend déjà de
`mika-common` (`Cargo.toml:23`), et recopier deux chaînes ici est la scission de
population que R-1 refuse un cran plus haut.

**Le `match` reste exhaustif sans bras `_`** — la propriété que le commentaire
existant revendique (*« adding a state cannot compile without a decision being
taken about it »*) et que ce changement doit préserver plutôt que consommer.

### U2b — Les deux sites d'appel, et le message opérateur

`terminal_state_class` a **deux appelants de production** (l. 522 et 528), tous
deux dans `send_message_to_agent`, où `task` est déjà en portée (ils lisent
`task.status.state` et `task.id`) : le second argument est donc
`attested_turn_failure_class(&task)`, sans rien à faire circuler.

Le message que ces sites composent — `remote task {id} ended in state '{state}'`
— **gagne la classe attestée quand elle existe** : un opérateur qui lit
`… ended in state 'failed' (transport)` sait immédiatement pourquoi le retry s'est
armé, et `… ended in state 'failed'` tout court dit que rien n'a été attesté.
C'est la seule chose qui rend la décision du client lisible **sans** ouvrir le
journal du serveur. Le texte ne devient pas un format de fil pour autant : la
décision lit la clé, jamais la phrase (mika#2179, mika#2291).

---

## 4. Conception — U3 : la mesure (AC2)

Au même site, avant l'estampille :

```
a2a_turn_failed   WARN   agent_id, task_id, session_id, model, error_class
```

et une ligne `audit_events` : `tool_name = 'a2a_turn_failed'`,
`target_key = <model ou "unknown">`, `after_value = <error_class>`,
`reasoning` = la phrase d'échec tronquée.

Trois décisions :

- **`model` est résolu, jamais deviné.** `effective_model` est `None` sur cette
  branche (le tour n'a produit aucun `AgentOutput`), donc le modèle est lu sur la
  configuration résolue de l'agent — la même valeur que `llm_budget_resolved`
  rapporte (mika#2328). Illisible ⇒ `"unknown"` : ça **dégrade** la ligne, ça ne
  la supprime pas (modèle `repo=unknown` de mika#2496).
- **WARN, pas INFO.** Chaque ligne est un tour perdu. Le régime attendu est
  faible ; s'il porte le trafic nominal, c'est une panne de rail et non du bruit
  à museler.
- **Pas de déduplication.** La population est de quelques dizaines par jour au
  pire, et chaque ligne est un fait daté distinct qu'on veut **compter** —
  dédupliquer effacerait la mesure qu'AC2 demande. Même arbitrage que
  `ready_label_outcome` (mika#2323) a dû écrire face à la doctrine mika#2131.

---

## 5. Contrat de vérification

### V1 — L'attestation traverse (Rust, `mika-agent`)

- Un tour qui échoue sur une `LlmError::Transport` produit un Task portant
  `mika.turn_failure_class = "transport"` (ou `transport_timeout`).
- **Contrôle négatif (AC3)** : un tour qui échoue sur `ParseError`,
  `ProviderError` et `HttpError{400}` porte respectivement `parse`, `provider`,
  `http_400` — jamais une classe transport.
- **Contrôle négatif de non-régression** : un tour **réussi** ne porte pas la
  clé du tout.
- Une erreur qui n'enveloppe aucune `LlmError` porte `other`.

### V2 — Le client décide (Rust, `mika-cli`, pur, sans réseau)

Table de R-2 asserté ligne par ligne, y compris les trois lignes
`Contract` — dont **l'absence de clé, qui est la non-régression : un serveur
antérieur produit exactement l'exit d'aujourd'hui.**
Plus : `Canceled` et `Rejected` restent `Contract` **même** avec une attestation
transport posée (le bras ne la consulte pas, et un test doit le dire, sans quoi
un futur éditeur qui la brancherait ne verrait rien rougir).

### V3 — Le retry s'arme (shell, `test-dispatch-lib.sh`)

Le harnais `_mika2278_probe` existe : il compte les invocations de `_arch_ask`
pour une séquence de codes d'exit donnée. Réutilisé tel quel — un `75` rejoue
(2 invocations), un `1` ne rejoue pas (1 invocation). **C'est déjà vert et ça
doit rester vert** : ce plan ne touche pas `dispatch-lib.sh`, il fait arriver le
`75` là où arrivait un `1`. Ce qui est ajouté est l'assertion qui **relie** les
deux moitiés : `_ARCH_ASK_RETRYABLE_EXIT` vaut la même chose que
`remote_ask::EXIT_TRANSPORT_FAILURE` — un test Rust épingle déjà le littéral
`75` côté CLI (`remote_ask.rs:1353`), le pendant shell ferme la boucle.

### V4 — Le fait est comptable (Rust)

L'événement et la ligne d'audit portent `model` et `error_class` ; un modèle
irrésoluble donne `"unknown"` et **n'annule pas la ligne**.

### V5 — Ce qui n'est pas testable ici, écrit plutôt que découvert

« Le retry sauve le groom » s'exécute contre OpenRouter, dans un blip réel, sur
un autre process. Le contrat **côté mika** est : *la classe traverse, le client
en tire `75`, le wrapper rejoue* — et V1–V3 l'attestent déterministiquement. La
moitié comportementale est la sonde S2 (§ 8).

---

## 6. Fire-Disposition

Ce plan livre des détecteurs : deux scans de source, plus les tests ci-dessus.
Disposition retenue : **(a) exception nommée en allowlist — et les deux
allowlists sont livrées VIDES**, ce qui est la disposition canonique de la maison
(mika#2201 : *« on déclare, on n'allowliste pas »*).

### D1 — `mika2522_the_turn_failure_name_has_a_single_writer`

Scan de source sur `crates/mika-agent/src/`, dans le registre
`canonical_tokens` : le nom `a2a_turn_failed` n'est écrit qu'à un site.
`TURN_FAILURE_WRITERS_ALLOWED` livrée **vide**, et son vide **épinglé par un test
frère** (`…_the_sole_writer_allowlist_is_empty`), sur le modèle de
mika#2242/#2496/#2498. **Quand il tire, on retire le second écrivain, on ne
l'allowliste pas** : un nom à deux écrivains rend la requête `GROUP BY` d'AC2
inexacte, et c'est cette requête qui est le livrable.

**Anti-vacuité, obligatoire (leçon mika#2496).** Le scan échoue aussi si le nom
n'est écrit **nulle part** : un scan visant un nom mort ne vérifie rien et se lit
exactement comme un scan propre. C'est la classe mika#2205 appliquée au
détecteur lui-même.

### D2 — `mika2522_the_attestation_has_a_single_reader`

Scan de source sur `crates/mika-cli/src/` et `crates/mika-a2a/src/` : hors de
`params.rs`, la clé `mika.turn_failure_class` n'est lue que par
`attested_turn_failure_class`. Allowlist **vide**, vide épinglée, anti-vacuité
identique. Modèle direct :
`mika1883_both_client_surfaces_read_the_one_reader`.

**Pourquoi un scan et pas un test comportemental.** Un second lecteur ne rend
**aucune décision fausse** le jour où on l'écrit : le client continue de classer
correctement, tous les tests restent verts, et seule la garantie « une vérité »
disparaît — en silence. C'est la forme d'échec qu'un test de comportement ne peut
pas voir, et la seule raison d'écrire un scan.

### Ni (b) ni (c)

Rien n'est livré désarmé : les deux allowlists sont vides **par construction** (le
nom et la clé n'existent pas avant ce PR), donc il n'y a aucune violation
préexistante à excepter ni à tracker. Aucune halte : rien ici ne demande de
cadrage opérateur.

---

## 7. Documentation

- **`CLAUDE.md`** — une entrée sous la famille des budgets LLM
  (voisine de mika#2189 / mika#2342 / mika#2280) : le défaut mesuré, la chaîne
  des huit maillons, **l'argument arithmétique qui refuse l'allongement du
  back-off** (pour qu'un futur ticket ne le repropose pas), la table de lecture
  de R-2, les surfaces opérateur et les sondes avec leurs haltes.
- **`crates/mika-agent/CLAUDE.md`** — la quatrième clé `mika.*` dans la liste des
  attestations `message/send`, avec sa borne `message/stream`.
- **Doc-comments** — le commentaire réfuté de `terminal_state_class` est
  **remplacé**, pas complété : laisser une justification devenue fausse à côté du
  code corrigé est ce qui fait reproduire le défaut au ticket suivant.

---

## 8. Surfaces opérateur et sondes

```bash
# 1. Quels tours ont été perdus, sur quel modèle, sur quelle classe ?
grep a2a_turn_failed "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{agent_id, model, error_class, task_id}'

# 2. CONTRÔLE POSITIF — le retry s'arme-t-il ? (dans le .stderr du dispatch)
grep -h '^dispatch-lib: .*arch_ask' \
  "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | tail
```

```sql
-- AC2 — le compteur par modèle, qui n'existait pas
SELECT target_key AS model, after_value AS class, count(*)
  FROM audit_events WHERE tool_name = 'a2a_turn_failed'
 GROUP BY 1, 2 ORDER BY 3 DESC;
```

| événement | niveau | régime attendu | lecture |
|---|---|---|---|
| `a2a_turn_failed` | WARN | **non vide, faible** | chaque ligne est un tour perdu ; sa classe dit si le rail ou le modèle est en cause |
| `class = transport*` | audit | **décroissant après déploiement** | ces tours-là sont désormais rejoués une fois |
| `class = parse\|provider\|http_4xx` | audit | inchangé | population d'AC3 : elle ne doit **pas** être rejouée |

### S1 — L'attestation traverse (premier tour arch échoué)

Une ligne `a2a_turn_failed` portant `model` et `error_class`.
**Halte 1 — aucune ligne alors que des tours arch ont échoué :** ne pas élargir
le site d'émission par réflexe. Établir d'abord que le binaire servi porte le
correctif (classe mika#2340) — puis si le tour est passé par
`message/stream`, population nommée hors périmètre en U1c.

### S2 — Le groom survit (AC4, 7 jours)

Aucun `first-pass _arch_ask failed` corrélé à une coupure `body read failed
mid-stream`, alors que les coupures continuent d'apparaître dans
`llm_call_attempt`. **C'est la sonde qui porte AC4, et son contrôle positif est
essentiel** : des coupures qui persistent avec des grooms qui aboutissent est le
succès. Zéro coupure **et** zéro groom perdu ne prouve rien — le blip a cessé,
pas le correctif d'avoir pris (classe mika#2205).

**Halte 2 — des grooms continuent de mourir avec `a2a_turn_failed` portant une
classe transport :** l'attestation est écrite et non lue. **Ne pas toucher au
serveur** : vérifier le binaire **CLI** (le `mika` qu'invoque `_arch_ask` est
celui du `PATH` du bac à sable, qui peut être en retard sur `mika-spirit` — les
deux moitiés se déploient ensemble, et c'est la seule asymétrie de déploiement que
ce correctif introduit).

**Halte 3 — des grooms meurent avec `a2a_turn_failed` portant `parse` ou
`provider` :** c'est un **résultat**, pas le défaut de ce ticket. Le tour échoue
pour une autre raison, que ce correctif refuse délibérément de rejouer (AC3).
Ouvrir le suivi **avec la classe et son compte**, jamais élargir
`is_transport_class`.

### S3 — Contrôle négatif du faux positif (7 jours)

`arch_ask` ne doit pas rejouer une passe dont l'architecte a rendu un verdict.
**Halte 4 —** un retry sur un tour abouti : `MIKA_ARCH_ASK_RETRY=0` **d'abord**,
diagnostic ensuite — c'est la sonde que mika#2278 s'est écrite (*« si un retry
firait sur une erreur de contrat, la ligne transport/contrat a fui, et le remède
est de désarmer et réparer la classification, jamais d'ajuster le budget »*), et
elle s'applique mot pour mot à ce changement, qui déplace précisément cette
ligne.

### S4 — Mesure de charge égale (AC4, second volet)

La requête SQL du § 8, groupée par modèle, **est** la tranche « modèle vs
transport » qu'AC4 demande : une classe `transport*` répartie sur les deux
modèles à charge comparable confirme le diagnostic de Prime ; une concentration
sur un modèle le réfute et rouvre la question du modèle — **qui n'est pas la
nôtre** (le ticket interdit le swap ; cette mesure le documenterait pour son
propre ticket).

---

## 9. Hors périmètre, délibérément

- **Allonger le back-off d'`openai.rs`** — refusé sur les quatre mesures de R4,
  dont une où le remède supprime la tentative qu'il protège. À rouvrir seulement
  si S2 montre que le retry `_arch_ask` **ne suffit pas**, c'est-à-dire qu'un
  blip dépasse 30 s ; le levier serait alors
  `MIKA_ARCH_ASK_RETRY_DELAY_SECS`, borné à 300 s, **sans une ligne de code**.
- **La cause OpenRouter** — ce travail rend la perte rattrapable et comptable ; il
  ne fait pas cesser la coupure. Voisin de mika#2313/#2317.
- **Élargir le retry `_arch_ask` à « tout non-zéro »** — refusé, avec sa raison,
  par mika#2278 D4 : *« un code qu'on ne peut pas lire est définitif ; l'inverse
  transformerait un futur mode de panne imprévu en boucle de retry
  silencieuse »*. Ce plan restreint au contraire la population de `75` à ce que
  le serveur atteste.
- **Rejouer `AGENT_BUSY` (-32000)** — attend déjà côté serveur dans une file
  bornée (mika#2163) ; empiler un second budget est l'empilement que mika#2278
  nomme hors périmètre.
- **`message/stream`** — U1c, population héritée de mika#2304/#1883.
- **Le swap de modèle** — interdit par le ticket (k2.5 reste, `config.toml`
  intact, DeepSeek hors rail).
- **Le littéral `120 s` d'`claude.rs`** — hors rail mesuré, hors périmètre depuis
  mika#2189.
- **Un compteur de retries dans `mika-common`** — impossible sans accès base ;
  `llm_call_attempt` porte déjà cette moitié (R5).

---

## 10. Definition of Done

- [ ] `TURN_FAILURE_CLASS_KEY` + `attested_turn_failure_class` dans
      `mika-a2a/src/params.rs`, avec le doc-comment qui dit pourquoi l'absence
      n'est pas `contract`.
- [ ] `stamp_turn_failure_class` au bras `Err` de `handle_message_send`, classe
      obtenue par `classify_anyhow_error` ; `TurnText::LoopFailed` porte la
      classe.
- [ ] `a2a_turn_failed` (WARN + `audit_events`) au même site, avec `model`
      résolu et dégradé en `"unknown"` plutôt que supprimé.
- [ ] `terminal_state_class` conditionnée à l'attestation, fail-closed, `match`
      exhaustif préservé, commentaire réfuté **remplacé**, les deux appelants
      passant `attested_turn_failure_class(&task)`.
- [ ] Le message opérateur des deux sites porte la classe quand elle est
      attestée, et rien de plus quand elle ne l'est pas.
- [ ] `is_transport_class` lit les constantes de `mika-common`, sans littéral
      recopié.
- [ ] V1 (attestation + 4 contrôles négatifs AC3), V2 (table de R-2 ligne par
      ligne, non-régression incluse), V3 (parité `75`), V4 (comptabilité).
- [ ] D1 et D2 : deux scans, allowlists **vides**, vides **épinglées**,
      **anti-vacuité** sur chacun.
- [ ] `CLAUDE.md` + `crates/mika-agent/CLAUDE.md` : l'entrée, la table de
      lecture, les surfaces, les quatre haltes, et l'argument qui refuse
      l'allongement du back-off.
- [ ] `cargo fmt`, `cargo clippy`, `cargo test`,
      `make test-dispatch-lib` verts.

---

## Acceptance criteria

Transcrits de senara-solutions/mika#2522, avec ce que ce plan en fait :

- **AC1** — *Retry borné avec back-off allongé sur la classe `retryable
  transport` : ex. 3 essais à 2 / 4 / 8 s (≈14 s de fenêtre) au lieu de ~3,5 s,
  dans le client LLM du relais (`openai.rs`) OU dans `_arch_ask` — assez pour
  survivre à un blip OpenRouter de plusieurs secondes. Décider du niveau après
  avoir confirmé où la coupure épuise le retry actuel.*
  → **Niveau tranché : `_arch_ask`.** La confirmation qu'AC1 demande est faite
  (R1–R3) : le retry client s'épuise, le tour échoue, et le `failed` qui en
  résulte est classé `Contract`, donc **le retry d'`_arch_ask` ne s'arme
  jamais** — alors qu'il offre 30 s (contre les ≈14 s demandées) sur une session
  fraîche. AC1 est satisfaite en réparant la classification (U1+U2), non en
  allongeant un back-off, dont R4 montre qu'il n'achète rien dans un cas et
  retire une tentative dans l'autre. Levier de réglage si S2 l'exige :
  `MIKA_ARCH_ASK_RETRY_DELAY_SECS`, sans code.

- **AC2** — *Audit d'un retry : compteur par modèle (et par appel) des retries
  transport, pour mesurer le taux.*
  → **Par appel : existe déjà** (`llm_call_attempt`, mika#2331 — `provider`,
  `model`, `attempt`, `outcome`, `error_class`). **Par modèle sur les tours
  perdus : livré** par `a2a_turn_failed` + sa ligne `audit_events`, SOLE WRITER,
  interrogeable en un `GROUP BY` (§ 8).

- **AC3** — *Test négatif : une erreur NON-retryable (4xx, parse, provider) n'est
  PAS rejouée.*
  → V1 (le serveur atteste `http_400` / `parse` / `provider`, jamais transport)
  **et** V2 (le client en tire `Contract` → exit `1`), plus la non-régression
  « clé absente ⇒ exit `1` » et le contrôle `Canceled`/`Rejected` insensibles à
  l'attestation.

- **AC4** — *Mesure post-déploiement : grooms perdus sur coupure → ~0. Le taux de
  coupures/modèle est mesuré à charge égale (tranche modèle vs transport, cf.
  Prime).*
  → Sonde S2 (avec son contrôle positif : des coupures qui persistent **et** des
  grooms qui aboutissent) et sonde S4 (la tranche par modèle, par la requête
  SQL). Quatre haltes écrites, dont deux qui interdisent d'élargir le prédicat
  avant d'avoir établi le déploiement ou la classe réelle.
