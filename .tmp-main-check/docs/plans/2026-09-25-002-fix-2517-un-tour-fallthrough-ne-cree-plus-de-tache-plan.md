# mika#2517 — Un tour webhook *fallthrough* ne crée plus de tâche

> **Ticket :** `senara-solutions/mika#2517` — « Un événement GitHub issue
> ouverte / commentée crée une tâche `mika-dev` … refusée
> `unauthorized_webhook_dispatch` et reste `pending` — un phantom à disposer à
> la main. »
>
> **Preuve :** `1ed31fa7` (claude-pilot#201), `179bc4e5` (mika#1910),
> `be27bb26` (mika#2516), le 2026-09-24. Chacune `status=pending`,
> `result={"error":"unauthorized_webhook_dispatch",…}`, sans process, disposée
> à la main.

---

## 0. Ce que la lecture du code déplace dans le ticket

Six constats, et les trois premiers changent le remède.

### R1 — `issues.opened` n'est **pas** routé, et un correctif qui le viserait serait inerte

Le critère d'acceptation nomme `issues.opened`. Or `route_event`
(`crates/mika-gateway/src/github.rs:320`) n'a **aucun bras**
`("issues", Some("opened"))` : l'événement tombe sur `_ => None` et le gateway
le laisse tomber en silence. Ce n'est pas une lecture d'intention, c'est
**épinglé par un test existant** :

```rust
// crates/mika-gateway/src/github.rs:1740
assert_eq!(route_event("issues", Some("opened"), None), None);
```

Donc la moitié `issues.opened` de l'AC1 est **déjà satisfaite**, et un correctif
écrit pour elle n'aurait aucune population. Le nommer est le premier livrable :
sans ce constat, ce ticket produirait un changement vert, testé, et sans effet —
la forme de panne que mika#2272 a dû nommer pour mika#2249 (*« zéro était
l'absence de mesure, pas la présence de prudence »*).

### R2 — La population réelle, et pourquoi elle se lit « ouverture d'issue »

Ce qui atteint `mika-dev` dans le domaine *Webhook Fallthrough*, donc ce que
`is_unauthorized_webhook_dispatch` rend `true` :

| événement | texte émis | consommateur moteur |
|---|---|---|
| `issues.labeled`, label ≠ `ready` | `[GitHub] Issue labeled <nom> on …` | **aucun** |
| `issue_comment.created` | `[GitHub] New comment on …` | **aucun** |
| `issues.assigned` | `[GitHub] Issue assigned: …` | **aucun** |
| `issues.closed` | `[GitHub] Issue closed: …` | `upstream_close_handler` (mika#1934) |
| type inconnu (Row H) | `[GitHub] <type>.<action> on …` | **aucun** |

« Ouverture d'issue cpp » et « ouverture d'issue #2516 » arrivent donc par les
`issues.labeled` que la **création** émet, un par label appliqué —
`/mika-issue` en pose trois (`type:*`, `priority:*`, `component:*`). Une issue
créée avec trois labels produit **trois** tours *fallthrough* sur `mika-dev`.
Recherche exhaustive : hors tests et hors le prédicat lui-même, aucun site de
`crates/mika-agent/src` ne lit `New comment on`, `Issue assigned` ni
`Issue labeled <non-ready>`.

### R3 — Le routage ne peut pas être retiré pour la moitié mesurée

L'AC dit « le routage l'écarte en amont ». Ce n'est réalisable que pour les deux
lignes sans consommateur *et sans autre usage de leur type d'événement* :
`issue_comment.created` et `issues.assigned`. `issues.labeled` **porte le
ready-label dispatch**, `issues.closed` **porte `upstream_close_handler`** —
retirer l'un ou l'autre casserait la boucle. Donc :

> **Un correctif purement gateway ne couvre pas la population mesurée.**

Et retirer `issue_comment.created` du routage est par ailleurs une **décision
produit** (« un commentaire d'issue atteint-il Mika ? »), pas une décision
substrat : hors périmètre, § 10.

### R4 — `VerdictAction::Handled` ne court-circuite **pas** le tour LLM

Le réflexe serait « un handler de plus dans la chaîne, qui rend `Handled` ».
`handlers.rs` montre que `Handled` **remplace le texte** par un pre-digest et
rien d'autre : `run_agent` est appelé inconditionnellement en sortie de chaîne
(`handlers.rs:~1645`). Il n'existe aucun mécanisme « sauter le tour » ici, et en
créer un serait une nouvelle forme de flot de contrôle dans `handle_message`
**plus** l'abandon des trois actions que la section *Webhook Fallthrough*
autorise (acquitter, corréler, alerter Vincent) — donc encore une décision
produit. Hors périmètre, § 10.

### R5 — La cause est un **conflit entre deux moitiés du moteur**, et il est déjà nommé ailleurs

C'est le constat porteur. Deux surfaces disent l'inverse l'une de l'autre sur
exactement cette population :

- `skills/bundled/self-dev/system_prompt.md` § *Webhook Fallthrough* :
  « **SCOPE RULE (HARD GATE)** … Do NOT `list_tasks`, create new tasks, or call
  `run_claude_pilot` … »
- `INTENT_GUARDS` → `webhook_zero_tools` (`agent_loop/mod.rs:9468`) re-prompte :
  « A GitHub webhook event was received but the response was text-only with zero
  tool calls. **Webhook events require action** — the engine expects at least one
  tool call (send_message, update_task_status, list_tasks, check_task, etc.) ».

`webhook_zero_tools_trigger` (`:9597`) n'exclut que trois préfixes
(`Check suite success`, `PR closed:`, `discussion.`) : il **tire** sur
`Issue labeled bug`, `New comment on`, `Issue assigned`, `Issue closed:` —
épinglé par ses propres tests (`:15455`, `:15462`). Donc un tour qui obéit au
HARD GATE et ne fait rien est **re-prompté pour n'avoir rien fait**, avec une
liste d'outils suggérés.

Et cette classe est **déjà documentée par le moteur lui-même** : mika#1469 a
rétréci ce trigger après « 25+ documented misfires (2026-06-09) where the guard
pressured the agent to call a tool just to satisfy the precondition », en
différant explicitement « correlation-aware filtering for the remaining
long-tail misfires ». **Le domaine *fallthrough* est cette queue.** Ce ticket
l'exécute.

### R6 — `create_task` déduplique, ce qui **condamne** l'option « marquer terminale »

Le ticket propose en alternative « soit la marquer terminale (pas `pending`) au
refus ». Le lecteur du code la refuse : `create_task` est **idempotent et
déduplique sur `reference_url`** et sur le label. Le `task_id` que le tour passe
ensuite à `run_claude_pilot` peut donc être une **row de suivi préexistante et
vivante** — et `record_dispatch_rejection` (`executor.rs:1852`) écrit déjà dans
son `tasks.result`. Marquer cette row `failed` **tuerait un suivi légitime en
vol**, et rien à ce site ne permet de savoir à bon marché si la row vient de ce
tour. Périmètre du refus et sa précondition de réouverture : § 10.

Corollaire sur les faucheurs existants, pour que personne ne suppose que le
résidu se nettoie tout seul : `find_orphaned_pending_issue_tasks`
(`db/tasks.rs:2646`) exige `source = 'self_dev'` **et** `type = 'issue'`
**et** `reference_url IS NOT NULL`. Une row créée par un tour *fallthrough* peut
porter `source = 'github_issue'` (valeur légale de `create_task`) et sortir de
cette population ; et le balayage phantom (mika#1712) ne regarde que
`in_progress`/`blocked`. **La base n'est pas lisible depuis le bac à sable de
dispatch**, donc la valeur réelle de `source` sur les trois instances est une
**sonde post-déploiement** (§ 9, S0), jamais une affirmation de ce plan.

---

## 1. Requirements

**Le défaut, en une phrase :** un tour *fallthrough* est poussé par le moteur à
appeler un outil, le seul outil « utile » qu'il trouve crée une row de suivi
`pending`, la tentative de dispatch qui suit est refusée, et **rien ne dispose
de la row**.

**Le remède, en une phrase :** sur le domaine *fallthrough*, retirer la
**capacité** (l'outil n'est pas dans le tableau servi au modèle) et retirer la
**pression** (le garde ne re-prompte plus), les deux à un seul site chacun, avec
une définition unique du domaine.

| # | Unité | Fichier | AC |
|---|---|---|---|
| U1 | Une définition du domaine, deux noms | `webhook_dispatch.rs` | — (support) |
| U2 | `create_task` retiré du tableau d'outils du tour *fallthrough* | `agent_loop/mod.rs` | **AC1** |
| U3 | `webhook_zero_tools` ne tire plus sur le domaine | `agent_loop/mod.rs` | **AC2** |
| U4 | La mesure : `webhook_fallthrough_turn` (INFO + audit) | `agent_loop/mod.rs` | **AC3** |
| U5 | Gardes : contrôles négatifs + scan de source | idem + tests | **AC4** |
| U6 | Documentation | `CLAUDE.md` ×2 | **AC5** |

**Ce que le plan ne touche pas, et c'est délibéré :** aucune valeur de réglage,
aucune migration, aucune variable d'environnement, aucun bras de `route_event`,
aucune ligne de `handlers.rs`, aucun faucheur, et **le garde 0 de
`validate_dispatch_readiness` reste intact** (§ 4, dernier paragraphe).

---

## 2. U1 — Une définition du domaine, deux noms

`is_unauthorized_webhook_dispatch` a aujourd'hui deux consommateurs :
le garde 0 de `validate_dispatch_readiness` (`executor.rs:1936`) et le trigger
`webhook_no_unauthorized_dispatch` (`agent_loop/mod.rs:9621`). U2 et U3 en
ajoutent deux. Or son **nom** énonce un jugement de *dispatch*, alors que U2/U3
posent une question différente sur le même ensemble : *« ce tour est-il dans le
domaine Webhook Fallthrough ? »*.

Ajouter dans `webhook_dispatch.rs` :

```rust
/// True when `msg` is a `[GitHub]` webhook event in the **Webhook Fallthrough**
/// domain — the complement of (a) the ready-label dispatch marker, (b) PR
/// events (qa skill territory) and (c) check-suite events (ci skill territory).
///
/// **The same set as [`is_unauthorized_webhook_dispatch`], asked as a different
/// question**, and that is why there are two names for one body. That predicate
/// answers *"may this turn dispatch a pilot?"* and its callers are refusals;
/// this one answers *"is this turn in the acknowledge-and-stop domain?"* and its
/// callers withhold a tool and stand a guard down. One definition, so the four
/// consumers cannot drift — the class `grooming_marker` had to engrave once
/// (mika#2158: a copied regex whose own comment said "Mirrors …" and which then
/// missed two widenings).
pub(crate) fn is_webhook_fallthrough_domain(msg: &str) -> bool { /* le corps actuel */ }

/// … doc existante, inchangée …
pub(crate) fn is_unauthorized_webhook_dispatch(msg: &str) -> bool {
    is_webhook_fallthrough_domain(msg)
}
```

**Le corps déménage, il ne change pas.** Aucune des 8 lignes de
`is_unauthorized_webhook_dispatch` n'est modifiée, et son test matriciel à huit
rangées (`:520`) reste intact et vert — c'est lui qui atteste que le déménagement
est un déménagement. Un second test applique la **même matrice** au nouveau nom,
pour que les deux noms soient épinglés séparément : sans lui, un futur éditeur
qui rétrécirait le primitif verrait un seul test rougir et pourrait croire que
seul l'alias est concerné.

**Refusé :** renommer `is_unauthorized_webhook_dispatch` (≈20 références de test
et deux sites de refus documentés par trois tickets — du churn sur un nom
porteur, pour rien).

---

## 3. U2 — `create_task` retiré du tableau d'outils du tour *fallthrough* (AC1)

### 3.1 Pourquoi cacher l'outil plutôt que refuser l'appel

Deux voies existent pour « zéro tâche créée » :

| voie | mécanisme | coût |
|---|---|---|
| refus au bord de l'outil | `ToolContext` porte le message d'origine, `create_task` refuse | `ToolContext` **ne porte pas** `originating_message` ; il faudrait l'ajouter et le filer aux quatre sites de construction |
| **retrait du tableau** | le modèle ne voit pas l'outil | un seul site, zéro nouveau champ |

Le retrait est aussi **plus fort**, et mika#811 l'a déjà écrit pour le denylist
d'identité : *« The model never sees disabled tools, cannot call them, cannot be
prompt-injected into trying. »* Il compose de surcroît avec le garde 6c
(`asserted_unavailability`), qui lit `enabled_tool_names` : l'outil étant
réellement absent, un modèle qui dit « je n'ai pas `create_task` » dit **vrai**
et le garde ne tire pas.

### 3.2 Le site, et il existe déjà

`inject_skills_and_resolve_tools` (`agent_loop/mod.rs:8333`) prend
`disabled_tools: &[String]` et le passe à `apply_agent_tool_visibility`, dont le
doc-comment dit en propres termes : *« this is the named hook that future
allowlist migration will reuse »*. Le site de conversation (`:5026`) lui passe
`&ctx.identity.tools.disabled` et a `params.user_message` en portée.

```rust
/// The tools a Webhook Fallthrough turn must not be handed.
///
/// `create_task` and nothing else, and the asymmetry with `run_claude_pilot` is
/// deliberate — see the note at the call site.
pub(crate) const FALLTHROUGH_WITHHELD_TOOLS: &[&str] = &["create_task"];

/// The effective tool denylist for one conversation turn: the agent's identity
/// denylist, widened on a Webhook Fallthrough turn.
///
/// `Cow` so the nominal path allocates nothing: a turn outside the domain hands
/// the identity slice through untouched, byte for byte.
fn effective_disabled_tools<'a>(
    identity_disabled: &'a [String],
    user_message: &str,
) -> Cow<'a, [String]> { … }
```

Au site `:5026`, `&ctx.identity.tools.disabled` devient
`&effective_disabled_tools(&ctx.identity.tools.disabled, params.user_message)`.

### 3.3 Trois bornes, écrites plutôt que découvertes

**(a) Un seul des trois appelants.** Les sites silencieux (`:6275`) et équipe
(`:7049`) ne sont pas touchés, et c'est **structurel, pas prudentiel** : un
webhook arrive par `POST /message` → `run_agent` → mode conversation. Un tour
silencieux n'a pas de message webhook — `originating_message` y vaut `None`
(mika#933) — et un tour d'équipe lit `TeamAgentParams`, une autre structure. Un
futur chemin qui servirait un webhook en mode silencieux échapperait au filtre :
c'est la borne, et le § 9 (S3) est la sonde qui la mesure.

**(b) `run_claude_pilot` reste VISIBLE, et c'est un arbitrage.** Le retirer aussi
rendrait le garde 0 de `validate_dispatch_readiness` **inatteignable** sur ce
chemin — donc supprimerait le signal `unauthorized_webhook_dispatch` qui dit que
le modèle a *essayé*. Un outil est caché parce que **rien ne le garde** ;
l'autre reste servi parce que **son garde est la mesure**. Coût nommé : avec
`create_task` absent, un modèle sous pression peut aller chercher un `task_id`
par `list_tasks` et faire tirer le garde 0 sur une row **préexistante**, dont le
`result` est alors décoré d'un refus — défaut préexistant, orthogonal, § 10.

**(c) La porte `required_tools` ne casse pas.**
`filter_available_required_tools()` pré-filtre les outils requis contre
l'ensemble réellement disponible, donc un `create_task` déclaré `required` par
une skill *et* retiré est simplement écarté du contrat plutôt que de produire un
re-prompt impossible à satisfaire. **À vérifier** (V2) : aucune skill du tableau
servi sur un tour *fallthrough* ne déclare `create_task` en `required_tools`.

---

## 4. U3 — La pression retirée (AC2)

Dans `webhook_zero_tools_trigger` (`:9597`), après les trois exclusions
existantes :

```rust
    // mika#2517 — the Webhook Fallthrough domain is the long tail mika#1469
    // deferred. That ticket narrowed this trigger after "25+ documented
    // misfires … where the guard pressured the agent to call a tool just to
    // satisfy the precondition", and left "correlation-aware filtering for the
    // remaining long-tail misfires" to a follow-up. This is it.
    //
    // NOT a relaxation: for this domain the guard's own premise — "webhook
    // events require action" — is FALSE by contract. `self-dev`'s § Webhook
    // Fallthrough says the correct action is to acknowledge and stop, so the
    // guard and the prompt beside it contradicted each other on exactly this
    // population, and the guard was re-prompting a turn that had obeyed.
    if crate::webhook_dispatch::is_webhook_fallthrough_domain(msg) {
        return false;
    }
```

**Les trois littéraux préexistants restent.** `Check suite success` et
`PR closed:` **ne sont pas** dans le domaine (check-suite et PR en sont exclus),
donc ils restent porteurs. `discussion.` devient redondant et est **conservé à
dessein** : le supprimer ferait dépendre l'exclusion des discussions de la
justesse du prédicat de domaine, et un futur rétrécissement de celui-ci
ré-armerait le garde sur les discussions en silence.

**Ce que l'exclusion ne relâche pas :** `webhook_no_unauthorized_dispatch`
(garde post-hoc du dispatch) et le garde 0 (bord d'outil) sont **inchangés**. Ce
qui disparaît est l'injonction « appelle un outil », pas la protection contre le
dispatch.

---

## 5. U4 — La mesure, et son contrôle positif (AC3)

Une ligne par tour *fallthorough*, au site de U2 — le seul endroit où le fait et
la liste retirée coexistent :

```rust
info!(
    target: "mika::otel",
    event = "webhook_fallthrough_turn",
    agent_id = %db.agent_id,
    session_id = %session_id,
    trace_id = ?params.trace_id,
    marker_class = %marker_class,          // "issue_labeled" | "issue_comment" | "issue_assigned" | "issue_closed" | "other"
    withheld_tools = %withheld.join(","),
    "Webhook Fallthrough turn — tools withheld (mika#2517)"
);
```

plus une ligne `audit_events` : `tool_name = "webhook_fallthrough_turn"`,
`target_key = "agent:<id>"`, `after_value = <marker_class>`,
`reasoning = "withheld=create_task"`.

**Pourquoi la ligne du tour et non la ligne du refus.** L'outil étant *caché*, il
n'y a pas de refus à compter : le seul fait observable est *« un tour
fallthrough a tourné, et voici ce qu'il n'a pas reçu »*. Cette ligne est donc
**à la fois la mesure et le contrôle positif**, ce qui est la leçon mika#2205 :
sans elle, zéro phantom se lirait exactement comme zéro tour.

**Non dédupliqué, à dessein.** Ce n'est pas un tick qui classe une population
(doctrine mika#2131) mais un événement daté distinct qu'on veut **compter** —
même arbitrage, écrit pour la même raison, que `ready_label_outcome`
(mika#2323). Volume attendu : quelques dizaines par jour.

**`marker_class` est un format de fil** (il atterrit dans `after_value` et
l'opérateur en fait des `GROUP BY`) : cinq constantes, un seul site,
épinglées par test. Deux orthographes d'une même classe couperaient une
population en deux sans le dire.

**SOLE WRITER** de `webhook_fallthrough_turn`, dans le journal et dans
`audit_events` (§ 7).

---

## 6. Contrat de vérification

| # | Vérification | Forme |
|---|---|---|
| V1 | Le déménagement est un déménagement : la matrice à 8 rangées passe sur **les deux** noms | unité |
| V2 | Aucune skill servie sur un tour *fallthrough* ne déclare `create_task` en `required_tools` | lecture + unité |
| V3 | `effective_disabled_tools` : domaine ⇒ `create_task` ajouté ; hors domaine ⇒ **slice identique** (`Cow::Borrowed`) | unité |
| V4 | `webhook_zero_tools_trigger` : faux sur les cinq formes du domaine, **vrai** sur ready-label, PR review, check-suite *failure* | unité |
| V5 | **Chemin de production** : un tour conversation sur `[GitHub] Issue labeled bug on …` sert un tableau d'outils **sans** `create_task` | eval (`MockLlmProvider`) |
| V6 | **Contrôle négatif de V5** : le même tour sur `[GitHub] Issue labeled ready on …` sert `create_task` | eval |
| V7 | **Contrôle négatif de V5, second axe** : un tour conversation **sans** préfixe `[GitHub]` sert `create_task` | eval |
| V8 | La ligne INFO et la row d'audit sont émises une fois par tour *fallthorough*, zéro fois hors domaine | eval |
| V9 | `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt` | CI |

**V6 et V7 sont porteurs, pas décoratifs.** V5 seul serait satisfait par un
filtre qui retire `create_task` de **tous** les tours — c'est-à-dire par un
correctif qui casse la boucle entière en ayant l'air de fermer le ticket. Chaque
contrôle négatif doit être **vu vert avant** que V5 soit vu vert, et V5 doit être
**vu rouge** contre le code d'avant (`git stash push` du correctif) : un test
jamais vu rouge n'atteste rien de l'invariant.

**L'invariant nommé, en ses propres symboles** (Rule 13) : *un tour dont
`is_webhook_fallthrough_domain(user_message)` est vrai ne doit jamais se voir
servir `create_task` dans son tableau d'outils.* L'assertion négative est V5.

---

## 7. Fire-Disposition

Ce plan livre **un détecteur** : le scan de source de U5.

```
agent_loop::tests::mika2517_the_fallthrough_domain_has_a_single_definition
```

Il refuse, sous `crates/mika-agent/src/` et hors code de test, tout second corps
qui reconstruirait le domaine — c'est-à-dire toute occurrence de la conjonction
de préfixes qui le définit (`"[GitHub] PR "` **et** `"[GitHub] Check suite "`)
en dehors de `webhook_dispatch.rs`. Aucun test comportemental ne peut voir cette
classe : un second corps ne rend **aucune décision fausse le jour où il est
écrit**, il diverge plus tard, en silence, avec toutes les assertions vertes.
C'est exactement la leçon mika#2158.

**Disposition : (a) exception nommée en allowlist — livrée VIDE.**

- `FALLTHROUGH_DOMAIN_DEFINITION_ALLOWED: &[&str] = &[]`
- Population actuelle mesurée : **un** site (`webhook_dispatch.rs`), donc il n'y
  a rien à exempter et aucune case où déposer la prochaine infraction
  (mika#2323). Quand le scan tire, **on retire le second corps, on n'allowliste
  pas** (doctrine mika#2201).
- Un test frère `…_the_allowlist_is_empty` refuse qu'elle cesse de l'être.
- **Assertion auto-nettoyante :** le scan rougit aussi si le prédicat est trouvé
  **nulle part** — un scan qui vise un nom mort se lit exactement comme un arbre
  propre (classe mika#2103 / mika#2205).

Le second détecteur, `…_the_fallthrough_turn_event_has_a_single_writer`
(SOLE WRITER de `webhook_fallthrough_turn`), suit la même disposition :
allowlist vide, contrôle d'anti-vacuité, résolution = retirer le second
écrivain. C'est cette propriété qui rend le `SELECT count(*)` du § 9 **exact**
plutôt qu'un nombre sur lequel deux sites peuvent diverger.

Aucun détecteur n'est livré désarmé, et aucun ne remonte à l'opérateur : les
deux populations sont vides à la livraison et les deux scans ont été vus verts.

---

## 8. Documentation (U6)

- **`crates/mika-agent/CLAUDE.md`** — dans le § *Intent-precondition registry*,
  l'exclusion `webhook_zero_tools` et sa raison (le conflit R5, la lignée
  mika#1469) ; un § court *Webhook Fallthrough — capacité et pression* décrivant
  le retrait de `create_task`, l'asymétrie avec `run_claude_pilot`, et les trois
  bornes du § 3.3.
- **`CLAUDE.md` racine** — un § *Un tour webhook fallthrough ne crée plus de
  tâche (mika#2517)* : les six constats du § 0 en forme courte, les surfaces
  opérateur, les sondes et leurs haltes (§ 9), et le hors-périmètre avec ses
  préconditions.
- **`skills/bundled/self-dev/system_prompt.md`** — la Rule 9 et la § *Webhook
  Fallthrough* gagnent **une phrase de fait**, pas une injonction de plus :
  « Sur ces tours, `create_task` n'est pas dans ton tableau d'outils, et
  l'engine n'attend aucun appel d'outil. » Une injonction supplémentaire serait
  exactement ce que
  `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
  interdit ; ce qui tient est la moitié structurelle, et la phrase ne fait que
  cesser de demander l'impossible.
  **`skills/bundled/` est une projection du BINAIRE** : cette édition n'atteint
  aucun agent avant `make deploy` → seed (classe mika#2340).

---

## 9. Surfaces opérateur et sondes

```bash
# 1. CONTRÔLE POSITIF — des tours fallthrough tournent-ils ?
grep webhook_fallthrough_turn "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{marker_class, withheld_tools, agent_id}'

# 2. Le garde 0 tire-t-il encore ? (le modèle essaie-t-il toujours ?)
grep unauthorized_webhook_dispatch "$MIKA_SPIRIT_LOG_FILE" | tail
```

```sql
-- La distribution des tours fallthrough, par classe d'événement
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'webhook_fallthrough_turn' GROUP BY 1 ORDER BY 2 DESC;

-- S1 — la population du défaut, à relever AVANT et APRÈS le déploiement
SELECT count(*) FROM tasks
 WHERE trigger_type = 'manual' AND status = 'pending'
   AND result LIKE '%unauthorized_webhook_dispatch%';
```

| événement | niveau | régime attendu | lecture |
|---|---|---|---|
| `webhook_fallthrough_turn` | INFO | **non vide**, dizaines/jour | contrôle positif ; chaque ligne est un tour qui n'a pas pu créer de row |
| `unauthorized_webhook_dispatch` | (dans `tasks.result`) | **décroissant** | le modèle essaie encore, mais sans row neuve à décorer |
| tâches `pending` portant ce refus | SQL | **ne croît plus** | l'AC, mesurée |

### Les cinq sondes, et leurs haltes

**S0 — caractérisation, à exécuter AVANT toute conclusion sur les faucheurs.**
Sur `~/.mika/data/mika.db`, lire `source`, `type` et `reference_url` des trois
rows citées par le ticket (`1ed31fa7`, `179bc4e5`, `be27bb26`).
*Lecture :* `source = 'self_dev'` + `type = 'issue'` + `reference_url` non nul ⇒
le faucheur stuck-pending (mika#2045) les aurait fauchées à 45 min et le ticket
surestime la permanence du résidu ; toute autre combinaison ⇒ elles sont hors de
**toute** population de faucheur, et « jusqu'à un cancel manuel » est exact.
**Cette sonde est un geste d'opérateur sur l'hôte** — la base n'est pas montée
dans le bac à sable de dispatch.

**S1 — le symptôme (30 jours).** Le compte SQL ci-dessus ne doit plus croître.
*Halte 1 —* il croît alors que le contrôle positif est non vide : le retrait
d'outil n'a pas mordu, ou une **autre** voie crée la row. **Ne pas élargir le
prédicat de domaine** : lire d'abord `withheld_tools` sur les lignes concernées,
puis établir quel appelant a créé la row (chemin silencieux — borne (a) du
§ 3.3 — ou `list_tasks` + row préexistante — § 10).

**S2 — le contrôle positif (48 h).** `webhook_fallthrough_turn` non vide.
*Halte 2 —* aucune ligne alors que des issues ont été labellisées ou
commentées : **établir le déploiement avant toute conclusion sur le prédicat**
(classe mika#2340). *Une ligne absente ne prouve rien tant qu'on n'a pas établi
que le binaire qui tourne sait l'écrire.*

**S3 — non-régression de la boucle (7 jours).** Les dispatches ready-label
continuent, et aucun `create_task` légitime n'est refusé. Signal direct : le
volume de `ready_label_outcome` avec `gate = dispatched` ne bouge pas.
*Halte 3 —* un dispatch légitime casse : **désarmer d'abord** (revert de U2,
une ligne au site `:5026`), diagnostiquer ensuite. Un correctif qui retire un
outil à la boucle est plus coûteux que le phantom qu'il remplace.

**S4 — la pression a bien disparu (7 jours).** Aucun re-prompt
`webhook_zero_tools` sur un texte du domaine. *Halte 4 —* il en reste : le
trigger ne lit pas le prédicat partagé ; réparer la délégation, **ne pas
ajouter un quatrième littéral de préfixe**.

---

## 10. Hors périmètre, délibérément

- **Marquer la tâche terminale au refus** (l'alternative du ticket) — **refusé
  sur mesure**, § 0 R6 : `create_task` déduplique sur `reference_url`, donc la
  row peut être un suivi préexistant et vivant, et la faucher serait un dégât
  strictement pire que le phantom. **Précondition de réouverture :** un
  discriminant qui établisse, au site du refus, que la row a été créée par ce
  tour (un stamp de métadonnée posé par `create_task`, par exemple) — c'est un
  ticket, pas une ligne.
- **`record_dispatch_rejection` écrit dans le `result` d'une row vivante** —
  défaut **préexistant** (mika#1108), orthogonal, et rendu *un peu plus
  probable* par U2 (borne (b) du § 3.3). Un `result` sur une row non terminale
  est une contradiction dans les termes : tous les autres écrivains de `result`
  sont terminaux. **Suivi**, précondition : que S1 montre cette population non
  négligeable.
- **Retirer `issue_comment.created` / `issues.assigned` de `route_event`** —
  décision **produit** (« un commentaire d'issue atteint-il Mika ? »), et elle
  ne couvrirait pas la population mesurée (§ 0 R3). **Suivi.**
- **Sauter le tour LLM sur le domaine** — abandonne les trois actions que la
  section *Webhook Fallthrough* autorise (acquitter, corréler, alerter), donc
  encore une décision produit, et demande une nouvelle forme de flot de contrôle
  dans `handle_message` (§ 0 R4). Le gain réel serait la **charge de file** sur
  `mika-dev` (trois tours par issue créée, sérialisés derrière `agent_lock`) :
  c'est ce chiffre, une fois mesuré par le contrôle positif S2, qui ouvrirait ce
  suivi — avec un compte plutôt qu'avec une intuition.
- **`issues.opened`** — déjà écarté au gateway et épinglé par un test
  (§ 0 R1). Rien à faire, et surtout pas un correctif qui aurait l'air d'en
  faire un.
- **Les faucheurs** (mika#1712, mika#2045, mika#2249) — inchangés ; ce plan
  supprime la **production** du phantom, pas sa disposition.
- **Le garde 0 de `validate_dispatch_readiness`** — inchangé, et
  volontairement : c'est la mesure de la tentative (§ 3.3 (b)).

---

## 11. Definition of Done

- [ ] `is_webhook_fallthrough_domain` est l'unique corps ; `is_unauthorized_webhook_dispatch` y délègue ; la matrice à 8 rangées passe sur les deux noms.
- [ ] Un tour conversation du domaine se voit servir un tableau d'outils **sans** `create_task` ; hors domaine, le tableau est identique à celui d'avant.
- [ ] `webhook_zero_tools_trigger` rend `false` sur les cinq formes du domaine et reste `true` sur ready-label, PR review et check-suite *failure*.
- [ ] `webhook_fallthrough_turn` est émis une fois par tour du domaine, en INFO **et** en `audit_events`, avec un `marker_class` épinglé comme format de fil.
- [ ] V5 **vu rouge** contre le code d'avant, puis vert ; V6 et V7 **vus verts** avant V5 ; les deux sorties collées dans le corps de PR sous `## Negative test (red → green)` (Rule 13).
- [ ] Les deux scans de source sont livrés **armés**, allowlists **vides**, chacun avec son contrôle d'anti-vacuité et son test « l'allowlist est vide ».
- [ ] `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt` verts.
- [ ] `crates/mika-agent/CLAUDE.md`, `CLAUDE.md` racine et `self-dev/system_prompt.md` à jour ; sonde S0 et les quatre haltes écrites.
- [ ] Le corps de PR nomme les six constats du § 0 — en particulier **R1**, qui rectifie l'AC du ticket, et **R6**, qui refuse son alternative.

---

## Acceptance criteria

Transcrits du corps du ticket, puis complétés pour les moitiés que le § 0
rectifie. Le ticket en porte un seul ; il est scindé pour être vérifiable.

- **AC1 — Zéro tâche créée sur ce chemin.** Un tour ouvert par un événement du
  domaine *Webhook Fallthrough* (`issues.labeled` avec un label ≠ `ready`,
  `issue_comment.created`, `issues.assigned`, `issues.closed`, ou un type
  inconnu) **ne peut pas** créer de tâche `mika-dev` : `create_task` est absent
  du tableau d'outils servi au modèle. Vérifié sur le chemin de production
  (V5), avec ses deux contrôles négatifs (V6, V7).
  *Note de rectification :* `issues.opened`, que le ticket nomme, est **déjà**
  écarté par `route_event` et épinglé par un test — cette moitié de l'AC est
  satisfaite avant ce travail et aucune ligne n'est écrite pour elle.
- **AC2 — La pression qui produisait le défaut est retirée.** Le garde
  `webhook_zero_tools` ne re-prompte plus un tour du domaine, dont le contrat
  de prompt est d'acquitter et de s'arrêter. Les trois exclusions préexistantes
  (mika#1469) restent en place.
- **AC3 — Zéro phantom `pending` sur ce chemin, et c'est mesurable.** Le compte
  de tâches `manual` / `pending` portant `unauthorized_webhook_dispatch` dans
  leur `result` ne croît plus après déploiement (S1), et l'absence de croissance
  est lisible **parce qu'un contrôle positif existe** (S2) : sans lui, zéro
  phantom serait indistinguable de zéro tour.
- **AC4 — La définition du domaine ne peut pas se dédoubler.** Un second corps
  reconstruisant le domaine hors de `webhook_dispatch.rs` fait rougir un scan de
  source, allowlist vide, doté d'un contrôle d'anti-vacuité. Idem pour un second
  écrivain de `webhook_fallthrough_turn`.
- **AC5 — Aucune régression de la boucle.** Les dispatches ready-label, les
  chemins PR / check-suite et le garde 0 de `validate_dispatch_readiness` sont
  inchangés ; aucune valeur de réglage, aucun bras de `route_event`, aucune
  migration, aucune variable d'environnement.
