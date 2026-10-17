# mika#2772 — Un dispatch de groom forcé par `required_tools` ne vise jamais un ticket sans `ready`

> Ouvert par le vol de mika#2768 (§ 6 de son plan), qui a nommé ce point hors
> périmètre plutôt que de le tuer en silence.

Labels : `bug`, `p1-important`, `agent-core`, `ready`, `dispatch:loop`,
`loop-substrate`.

Voisins : mika#2768 (le masque, vol 2), mika#2755 (vol 1), mika#2757
(`required_tools` / outils retirés), mika#841 (le consentement positif `ready`),
mika#1173 (le groom opérateur sur ticket non promu), mika#2649 (la garde de
lignée).

---

## 1. Contexte : une contrainte d'outil a dépensé le consentement de quelqu'un d'autre

### 1.1 La mesure

Le 2026-10-17 à 08:59:18Z, le `groom` contenu dans le slug de branche
`fix/2762/rollback-reaper-groom` a matché le mot-clé de dev-groom, armé son
`required_tools`, et fait groomer **mika#2766** — un ticket sans label `ready`,
absent du bassin de dispatch : `create_task "Groom mika#2766"` puis
`run_claude_pilot_groom mika#2766`, 48 tours, 3,40 $.

Le tour de reprise a ensuite tenté de **dispatcher l'implémentation** du même
ticket non promu ; seule l'occupation de la place `implement` l'a arrêté (lanceur
différé expiré à 09:22:11Z). À 09:13:21Z (n=4), l'événement de fermeture de la PR
mika#2765 a reforcé l'exigence : tâche fantôme « Groom mika#2768 », dispatch
refusé par `webhook_dispatch_target_mismatch`, tâche laissée `in_progress` sans
process jusqu'au balayage de démarrage.

### 1.2 Ce que la mesure établit

**Le consentement positif du label `ready` (mika#841) a été contourné par une
contrainte d'outil que personne n'avait demandée.** Pas par un bug de lecture de
label : par l'absence de toute lecture. La chaîne de portes
`validate_dispatch_readiness` demande « cette tâche a-t-elle le droit de
dispatcher ? » ; elle ne demande nulle part « ce dispatch a-t-il été *demandé*,
et sa cible a-t-elle consenti ? ».

### 1.3 Ce que la mesure n'établit pas

Elle n'établit **pas** un incident en cours. mika#2768 a élargi
`mask_cited_branch` à tout texte enveloppé ou enrichi et aux deux grammaires de
citation d'un pré-digest : sans slug cité dans le texte d'intention,
`required_tools` ne s'arme plus sur un événement de branche. **Le dispatch non
consenti n'a plus de producteur connu.** Ce vol garde la **classe** ouverte — un
futur producteur de contrainte d'outil autre que le mot-clé d'un slug retrouverait
le même trou.

Conséquence directe sur le périmètre : **la population que la garde refuse est
vide le jour où elle est livrée.** Une garde invisible est une garde dont on ne
sait rien, donc l'instrument qui rend cette vacuité *lisible* est un livrable de
ce vol au même titre que la garde (§ D5).

---

## 2. La décision

### 2.1 Le bon prédicat, et pourquoi ce n'est pas « le ticket a-t-il `ready` »

Un gate `ready` aveugle refuserait un flux réel : `/mika-groom-ticket` groome
légitimement des tickets non promus, sur dispatch opérateur en texte libre, et
c'est sa raison d'être (mika#1173). Le prédicat est donc une **conjonction à
trois termes**, et le terme discriminant est une **provenance** que le moteur ne
porte pas aujourd'hui :

| terme | lecture | ce qu'il protège |
|---|---|---|
| **T1** | le dispatch vise la place `groom` | hors sujet pour `implement` (axe de mika#2757) |
| **T2** | le tour a été **forcé** par `required_tools` **et** rien dans le texte d'intention ne demandait un groom | préserve mika#1173 intact |
| **T3** | le ticket visé ne porte pas `ready` | mika#841 |

Les trois ensemble, sinon le dispatch passe. T2 est le livrable neuf : il faut
construire la provenance.

### 2.2 La provenance : un booléen écrit au seul site qui force

Le forçage a **un seul site** dans le moteur :
`crates/mika-agent/src/agent_loop/mod.rs`, l'arm `else` du bloc d'enforcement
`required_tools` (≈ 2209–2247). C'est là que `run_loop` pousse la correction
`[mika-engine] The previous response did not call the required tool(s): …` et
`continue`. Ce re-prompt **est** le forçage : rien d'autre dans le tour ne dit au
modèle d'appeler l'outil.

Le `LongRunningContext` est construit **une fois par tour** — `run_agent`
(`agent_loop/mod.rs:5906`) et `run_silent` (`:7097`, `SilentTrigger::DeferredDispatch`
seul) — puis passé à `run_loop` en `long_running_ctx: Option<&LongRunningContext>`
(`:1368`), qui le fait descendre jusqu'à `executor::execute_skill_tool`. Il est
donc exactement l'objet dont la durée de vie est « ce tour » : le flag y est écrit
par le re-prompt et lu par le dispatch qui suit, dans le même tour.

Il porte déjà un champ muté à travers `&self` par un `AtomicU32`
(`dispatch_count`, « Atomic for interior mutability through `&self` ») ; le
précédent exact pour un drapeau par tour est `ToolContext.pr_review_posted: &AtomicBool`
(`tools/mod.rs`). Donc :

```rust
/// A-t-on forcé une contrainte d'outil pendant CE tour ? (mika#2772)
///
/// Écrit à un seul site — l'arm de re-prompt `required_tools` de `run_loop` —
/// et lu à un seul site, `validate_forced_dispatch_consent`. Le re-prompt est
/// le forçage : il n'existe aucun autre producteur de « l'outil doit être
/// appelé ». Un scan de source refuse un second site des deux côtés.
pub tool_constraint_forced: AtomicBool,
```

**Couverture de population.** Les deux seuls constructeurs de production sont les
deux cités ci-dessus, et tout dispatch pilote réel passe par
`execute_skill_tool`, qui exige `let Some(ctx) = long_running_ctx` sur le chemin
long-running (sinon erreur #537 ou enregistrement différé mika#1058). Le drapeau
couvre donc la totalité des dispatchs susceptibles d'être forcés — il n'y a pas de
dispatch forcé hors `LongRunningContext`.

**Les deux autres appelants de la chaîne ne sont pas concernés, et c'est
structurel** : `task_engine/dispatcher.rs:4415` et
`server/ready_label_handler.rs:1516` sont des chemins moteur sans tour LLM, donc
sans re-prompt, donc `forced == false` par construction. Ils ne passent pas par
`execute_skill_tool` et la garde ne les voit jamais.

### 2.3 La deuxième moitié de T2 : « non demandé », lu sur le texte **masqué**

T2 n'est pas « forcé » seul. Un groom opérateur en texte libre (« groom
mika#2766 ») est lui aussi forcé par `required_tools` au tour où le modèle oublie
d'appeler l'outil — et il est parfaitement légitime (mika#1173). Le terme complet
est donc **forcé ET non demandé**, où « demandé » se lit avec le prédicat maison
déjà en place, `webhook_dispatch::is_grooming_intent_message` (`:229`).

**Et il se lit sur le texte passé par `mask_cited_branch`.** C'est le point
load-bearing du vol : le texte qui a armé le forçage le 2026-10-17 contenait
`fix/2762/rollback-reaper-groom`, dont le `groom` est suivi d'un espace dans le
texte d'événement. Lu nu, ce même slug satisferait « le tour demandait un groom »
et **désarmerait la garde exactement dans l'incident qu'elle existe pour
refuser**. Le masque de mika#2755/#2768 est ce qui rend T2 correct ; l'appliquer
ici est la cohérence, pas une précaution.

```rust
let requested = ctx
    .originating_message
    .as_deref()
    .map(mika_common::github_event_format::mask_cited_branch)
    .is_some_and(|m| webhook_dispatch::is_grooming_intent_message(&m));
```

### 2.4 Ce que la garde de lignée ne peut pas porter — et pourquoi on ne la réutilise pas

`webhook_dispatch_target_mismatch` (`skills/executor.rs`, mika#2649) est une
garde de **lignée** : elle demande « une lignée relie-t-elle cette tâche à la
cible de l'événement ? » (quatre termes L1 `reference_url` / L2
`claude_pilot.pr_url` / L3 frère / L4 parent, le moins cher d'abord, le premier
qui tient autorise). Elle ne sait rien du consentement du ticket visé. C'est elle
qui a refusé le dispatch de n=4 — **par accident de lignée, pas par
consentement**.

La tentation serait d'en faire l'échappatoire de la nouvelle garde (« si une
lignée tient, laisse passer »). **Refusé, délibérément** : un bind L3-frère via la
cascade de jalon M4 est l'explication la plus probable du fait que le dispatch de
08:59 n'a *pas* été refusé. Rouvrir cette porte, c'est rouvrir le trou mesuré.
Les deux gardes restent orthogonales : la lignée répond « d'où vient cette
tâche », le consentement répond « la cible a-t-elle dit oui ».

Et **mika#2757 n'en est pas le domicile** : son axe est l'outil *retiré* par le
tour ; celui-ci est l'outil *présent* sur un ticket non promu.

### 2.5 Fail-closed, et pourquoi ici l'asymétrie est l'inverse de celle de la porte de place

La porte de place (`executor.rs` ≈ 3203–3330) est **fail-open** sur des labels
illisibles (`dispatch_seat_labels_unavailable`, mika#2084 D2), et c'est le bon
choix *là* : sa population est **tout** dispatch, donc un faux positif arrête la
boucle entière sur une panne d'API GitHub.

La population de **cette** garde est « dispatch de groom forcé **et** non
demandé » — mesurée **vide** aujourd'hui (§ 1.3). L'asymétrie se retourne donc :

- faux négatif : 3,40 $, 48 tours, un consentement contourné, et un dispatch
  d'implémentation tenté derrière (la mesure du § 1.1) ;
- faux positif : zéro coût aujourd'hui, puisqu'aucun dispatch légitime n'est dans
  la population ; et si un producteur réapparaît, une latence jusqu'au prochain
  dispatch consenti.

Donc **fail-closed**, chaque cause sous son propre nom. Cette divergence d'avec la
porte voisine est écrite dans le code, avec sa raison mesurée — sinon le prochain
lecteur « harmonisera » les deux.

### 2.6 Un seul vol

405 lignes estimées hors `docs/` contre une borne par vol de 429 (§ *Taille
estimée*). Le vol tient parce que la garde n'ouvre aucun chantier de migration :
un champ ajouté à un struct existant, un booléen écrit à un site existant, une
fonction sœur appelée au point de la chaîne où le contexte est déjà en portée, et
aucun changement de signature de `validate_dispatch_readiness` — donc **aucune des
~39 éditions mécaniques** de ses appels de test (§ D2).

---

## 3. Les livrables, site par site

### D1 — `crates/mika-agent/src/skills/executor.rs` : le champ de provenance

Ajouter à `pub struct LongRunningContext` (`:716`) le champ
`tool_constraint_forced: AtomicBool` avec le doc-comment du § 2.2 — nommant son
unique écrivain, son unique lecteur, et le scan qui le tient.

Les ~11 constructeurs de test du struct (≈ `:7674`, `7731`, `7801`, `7849`,
`7906`, `7946`, `8124`, `10126` et leurs voisins) prennent une ligne chacun :
`tool_constraint_forced: AtomicBool::new(false)`.

≈ 15 lignes de production + 12 lignes de constructeurs de test.

### D2 — `crates/mika-agent/src/skills/executor.rs` : `validate_forced_dispatch_consent`

Une fonction **sœur** de `validate_dispatch_readiness`, pas un paramètre de plus :

```rust
/// Un dispatch de groom forcé par `required_tools` ne vise jamais un ticket
/// sans `ready` (mika#2772).
pub(crate) async fn validate_forced_dispatch_consent(
    ctx: &LongRunningContext,
    github_token: Option<&str>,
    tool_input: Option<&serde_json::Value>,
) -> ForcedDispatchConsent
```

**Pourquoi une fonction sœur et non un paramètre.** `validate_dispatch_readiness`
a **43 sites d'appel** dans ce fichier, dont ~39 en tests ; un paramètre de plus
coûte 43 éditions mécaniques sans rien dire. Et surtout : le `LongRunningContext`
n'est **pas** dans la signature de cette chaîne, précisément parce que ses deux
autres appelants moteur n'en ont pas (§ 2.2). Le site où la provenance est connue
est `execute_skill_tool`, où `ctx` est déjà en portée (l'appel principal à
`validate_dispatch_readiness` est à `:4898` avec `ctx.originating_message.as_deref()`).

**Point d'appel et ordre** — dans `execute_skill_tool`, **après** le verdict
d'acceptation de `validate_dispatch_readiness` et **avant** le spawn de
`run_claude_pilot` : pré-hoc, jamais post-hoc (mika#1646 — `run_claude_pilot`
spawne un process et crée un worktree ; une garde post-hoc observe la violation
sans l'empêcher). Après, parce que la garde paie un appel réseau et que le moins
cher passe d'abord — l'idiome de mika#2649.

**Court-circuit à coût nul sur T1 et T2**, dans cet ordre, avant tout réseau :

1. `derive_dispatch_class(tool_input.and_then(extract_skill_from_input)) != "groom"`
   → `Allowed` (l'idiome exact déjà en usage à `:3073`) ;
2. `!ctx.tool_constraint_forced.load(Relaxed)` → `Allowed` ;
3. `requested` (§ 2.3) → `Allowed`.

Puis la moitié réseau, fail-closed, chaque cause sous son nom :

| verdict | cause |
|---|---|
| `forced_dispatch_subject_unresolved` | aucun numéro de ticket résoluble depuis le prompt de dispatch |
| `forced_dispatch_consent_no_token` | pas de token GitHub |
| `forced_dispatch_consent_unverifiable` | `Err(_)` du lecteur de labels |
| `forced_dispatch_subject_is_pull_request` | `Ok(None)` — la cible est une PR, pas un ticket |
| `forced_dispatch_without_consent` | labels lus, `ready` absent → **le refus nominal** |

Le sujet est résolu depuis le **prompt de dispatch seul**, par
`webhook_dispatch::parse_issue_ref_from_dispatch_prompt` + `parse_repo_ref_from_dispatch_prompt`
+ `normalize_owner_repo` (défaut `DEFAULT_DISPATCH_OWNER`). **Un seul sujet,
délibérément** (motif mika#2545) — là où la porte de place en construit deux
(`reference_url` *puis* le prompt), ici la question est « quel ticket ce dispatch
va-t-il groomer », et la réponse est dans le prompt. Un `reference_url` qui
désignerait un autre ticket est précisément l'accident de lignée du § 2.4.

Lecture des labels : `github_graphql::fetch_issue_labels_unless_pull_request(token, owner, repo, number)`
(`github_graphql.rs:219`, timeout 10 s) — déjà le lecteur de la porte de place,
déjà porteur de la distinction PR/issue dans son type de retour.

Levier de désarmement, pour l'opérateur et pour rien d'autre :
`MIKA_FORCED_DISPATCH_CONSENT=0` → la garde émet son INFO et rend `Allowed`.
Documenté en D6 ; **jamais** dans `CLAUDE.md` (cap de taille,
`crates/mika-common/tests/claude_md_size.rs`).

≈ 100 lignes de production.

### D3 — `crates/mika-agent/src/webhook_dispatch.rs` : les deux prédicats purs

La moitié testable sans réseau sort du gate, exactement selon le partage
tested/untested de la porte de place (dont la note à `executor.rs:11530` dit que
seul le classifieur pur est testé, la lecture de label exigeant réseau + token) :

```rust
/// `true` quand ce dispatch doit prouver le consentement de sa cible (mika#2772).
/// `forced` vient de `LongRunningContext.tool_constraint_forced` ;
/// `intent` est le texte d'intention du tour, DÉJÀ passé par `mask_cited_branch`.
pub(crate) fn forced_dispatch_needs_consent(
    dispatch_class: &str,
    forced: bool,
    intent: Option<&str>,
) -> bool

/// `true` quand la liste de labels porte le consentement positif de mika#841.
/// Lit `ready_label::READY_LABEL` — jamais le littéral.
pub(crate) fn labels_carry_ready(labels: &[String]) -> bool
```

`labels_carry_ready` lit `crate::ready_label::READY_LABEL` (`ready_label.rs:102`),
jamais `"ready"` en dur : `ready_label.rs` est le propriétaire unique du label
(mika#2315) et son test 13 scanne les écritures ; une lecture qui retape le
littéral est la prochaine dérive.

≈ 40 lignes de production.

### D4 — `crates/mika-agent/src/agent_loop/mod.rs` : l'écriture de la provenance

Dans l'arm `else` du bloc `required_tools` (≈ `:2209`), au moment où
`required_tools_retry_done = true` est posé et la correction `[mika-engine]`
poussée :

```rust
// mika#2772 — ce re-prompt EST le forçage : il est le seul site du moteur
// qui dise au modèle « appelle cet outil ». Un dispatch qui part après lui
// dans le même tour n'a pas été demandé, il a été exigé.
if let Some(ctx) = long_running_ctx {
    ctx.tool_constraint_forced
        .store(true, std::sync::atomic::Ordering::Relaxed);
}
```

`Relaxed` suffit : le drapeau est écrit et lu dans le même tour, sur le même
objet, et aucun ordre inter-champ n'est à garantir — même raisonnement que
`dispatch_count`.

Les deux autres arms du bloc ne l'écrivent **pas**, et c'est le cœur de la
précision du drapeau : l'early-accept `has_successful_pr_review` (#821) et
l'accept `has_terminal_required_tool_failure` (#516) **relâchent** la contrainte,
ils ne la forcent pas.

≈ 18 lignes.

### D5 — l'instrument : `forced_dispatch_consent` (une ligne par dispatch de groom)

Sans producteur connu (§ 1.3), la garde ne se manifesterait jamais, et « la classe
est vide » serait indiscernable de « la garde n'est pas déployée ». Donc une ligne
INFO sur **chaque** dispatch de groom, y compris la branche saine
(mika#2293 — *un réglage qu'on ne peut pas observer n'est pas un réglage, c'est un
espoir*) :

`event = "forced_dispatch_consent"`, `target: "mika::otel"`, champs `forced`,
`requested`, `verdict`, `subject` (`owner/repo#n`), `has_ready`, `trace_id`,
`session_id`.

Trois propriétés, chacune une doctrine maison :

- `subject` et `has_ready` sont **absents** — jamais `""`, jamais `false` — quand
  le sujet n'a pas été résolu ou les labels pas lus. Absent n'est pas zéro
  (mika#2331) : « pas de `ready` » et « on n'a pas pu regarder » sont deux
  verdicts différents et ont déjà deux noms distincts en D2.
- L'**absence** de la ligne signifie un binaire antérieur à ce vol (mika#2340),
  jamais une absence de dispatch forcé. L'instrument n'est pas vivant avant
  `make deploy`.
- Un seul `tool_name` d'audit avec le verdict en `after_value` (mika#2323), sur le
  modèle de `report_event_target_binding` (`executor.rs:2523`) : cinq noms de log,
  une colonne de verdict.

Compris dans les ≈ 100 lignes de D2.

### D6 — `docs/operator/environment-variables.md`

Une entrée `MIKA_FORCED_DISPATCH_CONSENT` : défaut armé, `0` désarme, la ligne
`forced_dispatch_consent` comme sonde, les cinq verdicts comme table de lecture,
et la phrase qui manquera sinon — *désarmer rend la boucle capable de groomer un
ticket que personne n'a promu*. Hors compte (`docs/`).

### D7 — tests des prédicats purs (`webhook_dispatch.rs`, `mod tests`)

Table-driven sur `forced_dispatch_needs_consent` :
`("implement", true, None) → false` (T1) ; `("groom", false, None) → false` (T2a) ;
`("groom", true, Some("groom mika#2766")) → false` (T2b, le flux mika#1173) ;
`("groom", true, None) → true` ; `("groom", true, Some("<texte d'événement de branche masqué>")) → true`.

Ce dernier cas est le **contrôle qui pin le § 2.3** : le même texte **non masqué**
rendrait `false`. Il doit être écrit avec le slug de l'incident
(`fix/2762/rollback-reaper-groom`) passé par `mask_cited_branch`, pour que la
régression « on a enlevé le masque » redevienne rouge.

Sur `labels_carry_ready` : présent, absent, casse mixte, liste vide.

≈ 65 lignes de test.

### D8 — tests du gate (`executor.rs`, `mod tests`)

Un test par terme, et le contrôle négatif de chacun — c'est la paire qui sépare
« la garde discrimine » de « la garde refuse tout » :

| test | ce qu'il tient |
|---|---|
| `mika2772_un_groom_force_et_non_demande_sur_ticket_sans_ready_est_refuse` | le refus nominal |
| `mika2772_un_groom_demande_sur_ticket_sans_ready_passe` | mika#1173 intact (T2b) |
| `mika2772_un_groom_non_force_passe` | T2a |
| `mika2772_un_dispatch_implement_force_passe` | T1 — l'axe de mika#2757 n'est pas le nôtre |
| `mika2772_un_sujet_non_resolu_est_refuse_sous_son_propre_nom` | fail-closed, verdict distinct |
| `mika2772_un_token_absent_est_refuse_sous_son_propre_nom` | fail-closed, verdict distinct |
| `mika2772_le_levier_de_desarmement_rend_allowed_et_le_dit` | `MIKA_FORCED_DISPATCH_CONSENT=0` émet quand même sa ligne |

Les arms qui exigent une vraie lecture de label ne sont pas testés ici, pour la
raison déjà écrite à `executor.rs:11530` : la lecture demande réseau + token. Ce
sont les deux prédicats purs de D3 qui portent la décision, et D7 qui les tient —
c'est le partage, pas un trou.

≈ 100 lignes de test.

### D9 — le scan de source : un écrivain, un lecteur

`mika2772_le_drapeau_de_forcage_a_un_seul_ecrivain_et_un_seul_lecteur` — scan de
`env!("CARGO_MANIFEST_DIR")/src` pour `tool_constraint_forced`, avec le motif
mika#2425 / mika#2158 / mika#2315 :

- aiguille = `.store(` et `.load(` sur le champ, sur lignes **non commentaires**
  (la prose *à propos* du champ n'est pas un accès, et les deux sites en portent
  trois paragraphes — forme du signal S de mika#2050) ;
- classification de test par `crate::source_scan::is_test_source_path` (mika#2335
  F2a) ;
- `assert!(scanned > 0)` — un scan qui ne lit rien passe toujours ;
- allowlist **livrée vide**, et pinnée vide par une assertion ;
- contrôle de bonne foi `mika2772_le_scan_du_drapeau_rougit_sur_un_second_site`,
  qui injecte un second `.store(` en source synthétique et vérifie que le scan le
  voit.

Pourquoi ce scan et pas seulement les tests comportementaux : un second site
d'écriture — par exemple un arm « on relâche la contrainte » qui poserait le
drapeau par symétrie apparente — ferait refuser des dispatchs légitimes **avec
toutes les assertions comportementales vertes**. C'est la classe `grooming_marker`
(mika#2158), que seul un scan structurel voit.

≈ 55 lignes de test.

---

## Fire-Disposition

Ce plan livre quatre détecteurs : les tests de prédicats (D7), les tests du gate
(D8), le scan de source du drapeau (D9), et la garde elle-même (D2 — son chemin de
succès est « aucune violation trouvée »).

**Disposition retenue : (a) exception nommée en allowlist — livrée VIDE, parce que
la population de violations existantes est mesurée nulle.**

Détail d'implémentation, détecteur par détecteur :

- **D9 (scan de source)** — l'allowlist existe, est nommée
  (`FORCED_FLAG_ACCESS_ALLOWLIST`), et est livrée **vide** : le champ est créé par
  ce vol, donc il a exactement un écrivain (D4) et un lecteur (D2) le jour de la
  livraison, et aucune violation préexistante n'est possible. Une assertion
  auto-nettoyante pinne la vacuité — si quelqu'un y ajoute une entrée, un test
  dédié rougit et exige la justification par ticket. Pas d'entrée, donc pas
  d'entrée stale possible.
- **D7, D8 (tests)** — livrés **armés**, sans `#[ignore]`, sans exception : ils
  décrivent le comportement que ce vol introduit, donc ils sont verts à la
  livraison par construction.
- **D2 (la garde)** — armée par défaut. Son levier de désarmement
  (`MIKA_FORCED_DISPATCH_CONSENT=0`) est une commande opérateur documentée, **pas**
  une allowlist : il ne nomme aucune donnée précise et n'est pas censé rester posé.
  La ligne `forced_dispatch_consent` le rend visible quand il l'est (D5).

**Consigne de déclenchement : (c) halte-et-remontée, jamais d'élargissement.** Si
D9 rougit, le remède est de router le second site par l'écrivain/lecteur existant
(doctrine mika#2201) ou de remonter à l'opérateur — **jamais** d'élargir l'aiguille
et **jamais** d'ajouter un allowlist de fichiers. Les gardes voisines de ce dépôt
le refusent dans ces termes.

---

## 4. Contrat de vérification

| # | ce qui est vérifié | comment |
|---|---|---|
| V1 | un groom forcé et non demandé sur un ticket sans `ready` est refusé **avant** tout spawn | D8, test nominal ; la garde est appelée avant `run_claude_pilot` |
| V2 | le groom opérateur en texte libre sur ticket non promu passe encore | D7 (T2b) + D8 |
| V3 | un slug cité ne compte pas comme une demande de groom | D7, cas masqué avec le slug de l'incident |
| V4 | un dispatch `implement` n'est pas touché | D8 (T1) |
| V5 | chaque cause de refus porte son propre nom greppable | D2, cinq verdicts ; D8 pour deux d'entre eux |
| V6 | la vacuité de la classe est lisible | D5, ligne INFO sur chaque dispatch de groom, branche saine comprise |
| V7 | le drapeau a un écrivain et un lecteur | D9 + son contrôle de bonne foi |
| V8 | rien d'autre n'a bougé | `cargo test`, `cargo clippy`, `make lint`, `scripts/verify-pipeline.sh origin/main` |

**Sonde post-déploiement** (après `make deploy`, pas avant — D5) :

```
grep -c 'forced_dispatch_consent' ~/.mika/logs/mika-spirit.log
grep 'forced_dispatch_consent' ~/.mika/logs/mika-spirit.log | grep -c '"forced":true'
```

Lecture attendue aujourd'hui : première commande > 0 dès le premier groom de la
boucle, seconde commande = 0 — *la classe est vide, et on le sait*. Première
commande = 0 après un groom observé signifie un binaire antérieur à ce vol
(mika#2340), pas une absence de dispatch.

---

## 5. Ordre de commit

1. **D1** — le champ + les ~11 constructeurs de test. Compile, suite verte, aucun
   comportement changé.
2. **D3 + D7** — les deux prédicats purs et leurs tests. Verts, encore appelés par
   personne.
3. **D4** — l'écriture de la provenance. Le drapeau est écrit, lu par personne ;
   suite verte.
4. **D2 + D5** — la garde, ses cinq verdicts, sa ligne INFO, son câblage dans
   `execute_skill_tool`. C'est le commit qui change le comportement.
5. **D8** — les tests du gate.
6. **D9** — le scan de source et son contrôle.
7. **D6** — la doc opérateur.

Plancher mergeable : 1–6. D6 est de la documentation et ne peut pas rougir, mais
part dans la même PR (un levier non documenté est un levier perdu).

---

## 6. Risques

- **T2 lu sur le texte nu.** Le risque principal du vol, et il annule la garde
  dans l'incident exact qu'elle refuse (§ 2.3). Mitigation : D7 pinne le cas avec
  le slug de l'incident.
- **Un second site d'écriture du drapeau**, posé par symétrie apparente depuis un
  arm qui *relâche* la contrainte (§ D4). Mitigation : D9.
- **Harmonisation du fail-closed avec le fail-open voisin.** Deux portes
  adjacentes, deux politiques opposées. Mitigation : la raison mesurée du § 2.5 est
  écrite au site, avec les deux populations nommées.
- **La garde reste non exercée en production.** Attendu, pas un défaut (§ 1.3) :
  c'est ce que D5 rend lisible.
- **Rouvrir la porte par la lignée.** Un futur lecteur peut trouver « naturel »
  d'accepter un dispatch lié par L3. § 2.4 est écrit pour lui.

---

## 7. Hors périmètre

- **L'axe de mika#2757** (l'outil *retiré* par le tour). Voisin, pas le même
  prédicat.
- **Élargir la garde à `implement`.** Le dispatch d'implémentation sur ticket non
  promu a été arrêté par l'occupation de la place, pas par une garde (§ 1.1) —
  c'est un trou réel, mais son prédicat est différent (la chaîne existante
  interroge déjà le bassin pour `implement`) et le mesurer est un ticket à part.
  `Tracked in:` — à ouvrir si la sonde V6 observe un `forced:true` sur `implement`.
- **Un nouveau producteur de contrainte d'outil.** La classe est gardée ; en
  trouver un n'est pas le travail de ce vol.
- **`scripts/canonical-tokens.tsv`.** Les cinq verdicts sont **émis**, jamais
  matchés par un lecteur ; ce fichier déclare les tokens qu'un lecteur matche
  (mika#2201). Aucune ligne à y ajouter — vérifié : les verdicts voisins
  (`webhook_dispatch_target_mismatch`, `dispatch_seat_labels_unavailable`) n'y
  figurent pas non plus.
- **Toute modification de `validate_dispatch_readiness`** et de ses 43 sites
  d'appel (§ D2).

---

## Definition of Done

- [ ] `LongRunningContext.tool_constraint_forced: AtomicBool` existe, avec le
      doc-comment nommant son unique écrivain et son unique lecteur.
- [ ] Le drapeau est écrit au seul arm de re-prompt `required_tools` de `run_loop`,
      et nulle part ailleurs.
- [ ] `validate_forced_dispatch_consent` est appelée dans `execute_skill_tool`
      après le verdict de `validate_dispatch_readiness` et avant tout spawn.
- [ ] Les cinq verdicts de refus portent cinq noms distincts et greppables.
- [ ] Les deux prédicats purs vivent dans `webhook_dispatch.rs` et
      `labels_carry_ready` lit `READY_LABEL`, pas le littéral.
- [ ] La ligne `forced_dispatch_consent` est émise sur chaque dispatch de groom,
      branche saine comprise, avec `subject` et `has_ready` **absents** quand ils
      n'ont pas été établis.
- [ ] `MIKA_FORCED_DISPATCH_CONSENT` est documentée dans
      `docs/operator/environment-variables.md`, et pas dans `CLAUDE.md`.
- [ ] D7, D8, D9 verts ; D9 livré avec allowlist vide, assertion `scanned > 0`, et
      contrôle de bonne foi.
- [ ] `cargo test`, `cargo clippy`, `make lint`, `make fmt --check` et
      `scripts/verify-pipeline.sh origin/main` passent.
- [ ] La PR dit explicitement que la population refusée est vide à la livraison, et
      pointe la sonde V6.

## Acceptance criteria

1. **Un dispatch de groom forcé par `required_tools` ne vise jamais un ticket sans
   `ready`.** Le refus est pré-hoc : aucun process `claude-pilot` n'est spawné,
   aucun worktree n'est créé.
2. **Le groom opérateur sur ticket non promu reste possible** (mika#1173) : un tour
   dont le texte d'intention demande un groom n'est pas soumis à la garde, même
   quand `required_tools` l'a forcé.
3. **« Demandé » se lit sur le texte masqué.** Un slug de branche cité contenant
   `groom` ne vaut pas une demande de groom — le texte passe par
   `mask_cited_branch` avant `is_grooming_intent_message`, et un test le pinne avec
   le slug de l'incident du 2026-10-17.
4. **La provenance a un seul écrivain et un seul lecteur**, tenus par un scan de
   source à allowlist vide, doté d'un contrôle de bonne foi.
5. **Les dispatchs `implement` sont hors de la garde** : l'axe de ce ticket est la
   place `groom`.
6. **Chaque refus est attribuable** : sujet non résolu, token absent, labels
   illisibles, cible de type PR, et absence de `ready` portent cinq noms distincts.
7. **Fail-closed assumé et justifié au site**, avec la divergence d'avec le
   fail-open de la porte de place nommée et sa raison mesurée (asymétrie des
   populations).
8. **La vacuité de la classe est observable** : une ligne `forced_dispatch_consent`
   par dispatch de groom, y compris quand rien n'est forcé, et son absence se lit
   comme un binaire périmé, pas comme une absence de forçage.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| D1 — `executor.rs` : champ `tool_constraint_forced` + doc | 15 |
| D1 — ~11 constructeurs de test de `LongRunningContext` | 12 |
| D2 + D5 — `executor.rs` : `validate_forced_dispatch_consent`, cinq verdicts, ligne INFO, câblage | 100 |
| D3 — `webhook_dispatch.rs` : deux prédicats purs + doc | 40 |
| D4 — `agent_loop/mod.rs` : écriture de la provenance à l'arm de re-prompt | 18 |
| D7 — tests des prédicats purs (`webhook_dispatch.rs`) | 65 |
| D8 — tests du gate (`executor.rs`) | 100 |
| D9 — scan de source + contrôle de bonne foi | 55 |

Total estimé : 405 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 405 < 429, marge 24 lignes (≈ 6 %).
La marge est étroite mais le vol n'ouvre aucun chantier qui la dépense : le champ
s'ajoute à un struct existant, le drapeau s'écrit à un site existant, la garde est
appelée là où le contexte est déjà en portée, et **la signature de
`validate_dispatch_readiness` ne change pas** — ce qui est précisément le choix
qui évite les ~39 éditions mécaniques de ses appels de test (§ D2) et garde le
total sous la borne.

Levier de resserrement si le vol déborde en vol : D7 passe en un seul `#[test]`
table-driven (65 → ≈ 35), ce qui rend 30 lignes sans rien perdre de la couverture.
Le total par PR (405) est très en dessous du seuil de 1000, donc aucune découpe en
phases n'est requise.
