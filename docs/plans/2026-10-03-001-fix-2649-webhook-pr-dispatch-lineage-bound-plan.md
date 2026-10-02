# Un dispatch ouvert par un événement PR est borné à la LIGNÉE de cet événement (mika#2649)

> Ticket : senara-solutions/mika#2649 — `bug`, `dispatch:loop`, `loop-substrate`
> Branche : `fix/2649/webhook-dispatch-un-tour-webhook-pr-peut`

---

## 1. Le constat, et ce que la lecture du code en déplace

Le ticket mesure (2026-10-02, trace `17ba765a-be9c-11f1-94aa-c13d0500c506`) qu'un
tour mika-dev ouvert par un **webhook de revue QA sur la PR #2647** a lancé un
implement de **mika#2646** — un autre ticket, hors fenêtre `ready`, pendant qu'un
implement de #2636 volait. Deux pilotes implement simultanés de 20:03:17Z à
~20:24Z.

Le mécanisme qu'il nomme est juste et vérifié : `is_unauthorized_webhook_dispatch`
juge la **nature** de l'événement source, jamais sa **cible**. Les préfixes
`[GitHub] PR ` et `[GitHub] Check suite ` sont retirés du domaine Fallthrough
(`webhook_dispatch.rs::is_webhook_fallthrough_domain`) parce que
`self-dev-webhook-qa` / `-ci` y portent des dispatchs légitimes ; une fois sortis
de ce domaine, **plus aucun terme ne lie le dispatch à la PR de l'événement**.

Six rectifications que la lecture du code impose, et elles sont le **premier
livrable** : deux changent le périmètre d'AC1, une retire AC2 de cette PR, une
borne AC4, deux fixent les lecteurs à réutiliser.

### R1 — Le cap implement **est** vérifié au point de dispatch, sur les quatre chemins. AC2 tel qu'écrit est déjà satisfait.

AC2 demande que « le cap implement soit vérifié au point de dispatch quel que
soit le chemin ». Il l'est. `validate_dispatch_readiness`
(`skills/executor.rs:2282`) porte la garde par classe (`#583`, `#1001`,
mika#2160) et ses **quatre** appelants de production sont :

| appelant | site |
|---|---|
| frontière d'outil (le chemin LLM du constat) | `skills/executor.rs:4348` |
| chemin ready-label | `server/ready_label_handler.rs:1508` |
| moteur / tick | `task_engine/dispatcher.rs:4304` |
| verdict handler (CI-fix, QA-hold) | `server/verdict_handler.rs:933` |

Le `run_claude_pilot` du constat **a traversé cette garde**. Et elle n'est pas
seule : `try_acquire_dispatch_slot` (`dispatch_slot_leases`, PRIMARY KEY
`(agent_id, dispatch_class, slot_index)`) est le **dernier** terme de la même
fonction et rend la revendication atomique plutôt que consultative
(`crates/mika-agent/CLAUDE.md` § *The exec slot is CLAIMED, not checked*).

**Ce qui est faux dans AC2 n'est donc pas « le cap n'est pas vérifié » mais ce
que le cap compte.** `has_active_callback_tasks_excluding` /
`count_active_callback_tasks_excluding` (`db/tasks.rs:3589`, `:3643`) filtrent
`t.status IN ('pending','in_progress')` : elles comptent des **lignes**, et un
pilote est un **processus**. Un pilote vif dont la ligne callback est devenue
terminale — par le watchdog #959, par une supersession, ou par le `cancel_task`
que le second cas de la même famille a mesuré — est **invisible au cap**. C'est
exactement la topologie que `live_pilot` (mika#2279) a été écrit pour lire, et
que la garde du cap ne consulte pas.

**Le mécanisme réel du contournement n'est PAS établi, et il n'est pas
établissable depuis le bac à sable de dispatch** : `~/.mika/data/mika.db` n'y est
pas montée et `/var/log/mika/server.log` n'y est pas lisible, donc l'état des
lignes `3dbee3aa` / `f8f817ce` à 20:03:15Z est hors d'atteinte. Entre « la ligne
de #2636 était terminale » (le trou de R1), « son `dispatch_class` était `groom` »
(auquel cas il n'y a aucune violation de cap) et « la première
`run_claude_pilot` de 20:03:06 a muté l'état », rien dans le code ne tranche.

**Conséquence de périmètre : AC2 sort de cette PR**, avec sa précondition écrite
(§ 8). Et elle en sort sans laisser le p0 ouvert — voir R2 juste en dessous : le
terme d'AC1 refuse le dispatch mesuré **avant** que la question du cap se pose.
*Un cap qu'on « répare » sans avoir établi par où il a fui est un cap qu'on
élargit au hasard sur un chemin qui marche.*

### R2 — AC1 seul referme l'occurrence mesurée, et c'est ce qui autorise le découpage

L'événement portait sur la PR #2647 ; le dispatch visait #2646. Sous le terme de
lignée du § 3, ce dispatch est refusé à la frontière d'outil, aucune ligne
callback n'est créée, aucun processus n'est lancé — donc les deux pilotes ne
coexistent jamais, **quoi qu'ait fait le cap**. AC1 est le terme qui mord ; AC2
est de la défense en profondeur dont la précondition est une mesure.

### R3 — Les dispatchs CI-fix / QA-hold **ne traversent pas la garde** : ce sont des dispatchs MOTEUR, `originating_message = None`

C'est le point qui déplace le plus le périmètre, et il est écrit en commentaire au
site même (`server/verdict_handler.rs:926-931`) :

> `originating_message = None` because the dispatch is engine-authorized by the
> verdict handler's own event-type gate […] Passing `None` skips guard (0)

Le contrôle positif qu'AC3 décrit (« le dispatch légitime CI-fix / QA-hold sur X
passe ») est donc, pour l'essentiel, **hors de la population du prédicat par
construction** : il passe parce qu'il n'est pas interrogé. `iterate_dispatch`
(mika#2506) est un troisième appelant de la même mécanique et fait de même.
AC3 reste exigible, mais son contrôle positif doit porter sur la population qui
traverse **réellement** le prédicat — celle de R4.

### R4 — La population légitime du chemin LLM n'est PAS vide : c'est la cascade de jalon M4, et elle dispatche un AUTRE ticket que la PR

Le réflexe, une fois R3 lu, est de conclure que le chemin LLM n'a plus de
population légitime et de remettre `[GitHub] PR ` dans le domaine Fallthrough.
**Ce serait casser un flux prescrit.** `skills/bundled/self-dev-webhook-qa/system_prompt.md:236` :

> **If a next pending child exists:** Transition it to `in_progress` via
> `update_task_status(<next_child>, …)`, then call `run_claude_pilot(...)` with
> the **next child's** `task_id` per M4 step 1 + step 2. […] Guard (0)
> `unauthorized_webhook_dispatch` does NOT reject this call — `[GitHub] PR closed:`
> is on the qa-territory allowlist

Sur un `[GitHub] PR closed:`, la cascade de jalon dispatche légitimement **le
frère suivant**, c'est-à-dire un ticket dont le numéro n'est pas celui de la PR.
Un prédicat « même cible, sinon refus » refuse M4.

**Donc le discriminant n'est pas l'identité de la cible mais la LIGNÉE.** Ce que
l'occurrence mesurée ne partage pas avec M4 :

| | M4 légitime | le constat |
|---|---|---|
| action de l'événement | `PR closed:` (mergée) | `PR review (...)` soumise |
| la tâche dispatchée | un **enfant pending préexistant** du parent jalon | une tâche **créée dans le tour même** (`create_task` 20:03:13) |
| son `parent_task_id` | le parent jalon, partagé avec la PR | aucun |
| son `metadata` | porte la lignée du jalon | vide (tâche fraîche) |

### R5 — Deux lecteurs uniques existent ; aucun second parseur n'est écrit

`deadline_verdict::parse_pr_target` est le lecteur unique de la grammaire
d'événement PR (mika#2368, dont le commentaire dit : *« Aucune des deux regex
n'est recopiée ici : une grammaire de fil dupliquée est exactement ce qui a laissé
deux lecteurs diverger dans mika#2158 »*). `worktree_reaper::issue_number_from_branch`
est le lecteur unique du numéro d'issue dans un nom de branche (mika#2619, *« le
deuxième segment, et rien d'autre »*). Les deux sont `pub`. Ce plan les **appelle**.

Et une mesure qui décide de la forme du prédicat check-suite :
`parse_pr_target("[GitHub] Check suite success on senara-solutions/mika (branch: main)")`
rend **`None`** — test épinglé à `deadline_verdict.rs:820`. La grammaire
check-suite (`crates/mika-gateway/src/github.rs:543`) porte une **branche**, pas
un numéro de PR. La cible d'un tour check-suite se lit donc par
`issue_number_from_branch`, pas par `parse_pr_target`.

### R6 — `ToolContext` porte un **booléen**, jamais `originating_message` — et c'est une décision prise deux fois

`tools/mod.rs:159-181` (mika#2573) cite mika#2517 et tranche : ce qui traverse
jusqu'aux outils est un **verdict** (`is_webhook_fallthrough_turn: bool`), pas une
`&str` avec sa durée de vie et sa charge utile. Conséquence directe sur AC4 :
`cancel_task`, `create_task` et `update_task_status` sont des outils ordinaires
dont le `ToolContext` **ne porte pas la cible de l'événement**, donc les borner
demande de filer une charge utile que la maison a refusée deux fois.
`validate_dispatch_readiness`, lui, **reçoit déjà** `originating_message` **et**
`tool_input` dans sa signature : AC1 est bon marché à ce site unique, et nulle
part ailleurs. AC4 est donc livré comme ce qu'il demande — **un recensement avec
un verdict par outil** (§ 5) — et non comme quatre gardes.

---

## 2. Requirements

1. Un tour dont l'`originating_message` est un événement `[GitHub] PR …` ou
   `[GitHub] Check suite …` ne peut dispatcher `run_claude_pilot` /
   `run_claude_pilot_groom` que pour une tâche **liée par lignée** à la cible de
   cet événement.
2. La cascade de jalon M4 (`PR closed:` → frère pending suivant) continue de
   passer. C'est une **exigence dure**, pas un souhait : la casser arrête la
   boucle de jalon.
3. Les trois chemins moteur (`verdict_handler`, `iterate_dispatch`,
   `task_engine::dispatcher`) sont **inchangés** — ils passent
   `originating_message = None` et restent hors population.
4. Le refus est **nommé, audité, à écrivain unique**, et son `after_value` est un
   format de fil à site de définition unique.
5. Aucun second parseur de grammaire : `parse_pr_target` et
   `issue_number_from_branch` sont appelés.
6. Aucune valeur de réglage déplacée : ni `MIKA_DISPATCH_MAX_CONCURRENT_IMPLEMENT`,
   ni `DISPATCH_SLOT_LEASE_TTL_SECS`, ni le domaine Fallthrough, ni
   `FALLTHROUGH_WITHHELD_TOOLS`.
7. Aucune variable d'environnement nouvelle. Le geste de désarmement est un
   **revert** — un désarmement par variable sur un chemin de plan de dispatch
   serait un désarmement par coquille (précédent mika#1646, mika#2627).

---

## 3. Le prédicat : `webhook_dispatch::event_target_binding`

Un module unique dans `webhook_dispatch.rs`, **pur** (aucun I/O), plus une
traversée DB bornée dans `executor.rs`. Deux moitiés, pour que la décision soit
testable sans base.

### 3.1 Moitié pure — la cible de l'événement

```rust
pub enum WebhookEventTarget {
    /// `[GitHub] PR …` — le numéro est celui de la PR.
    Pr { repo: String, number: u64 },
    /// `[GitHub] Check suite …` — la grammaire porte une BRANCHE ; le numéro
    /// d'issue en est le deuxième segment (mika#2619), ou rien.
    Branch { repo: String, issue: Option<u64> },
    /// Le message n'est pas un événement PR/check-suite lisible.
    NotApplicable,
}

pub fn webhook_event_target(msg: &str) -> WebhookEventTarget
```

- `[GitHub] PR ` → `parse_pr_target(msg)` (R5). Illisible ⇒ `NotApplicable`.
- `[GitHub] Check suite ` → extraction de `(branch: …)` puis
  `issue_number_from_branch` (R5). Branche absente ou non conforme ⇒
  `Branch { issue: None }`.
- Tout le reste ⇒ `NotApplicable`.

**`NotApplicable` sort le tour de la population.** Un tour Telegram, un callback,
un heartbeat, un `[GitHub] Issue …` : rien à quoi borner. Le domaine Fallthrough
garde sa garde propre (gate 0), inchangée.

### 3.2 Moitié DB — la lignée

Quatre termes, évalués dans cet ordre (du moins cher au plus cher), et **le
premier qui tient autorise** :

| # | terme | ce qu'il couvre |
|---|---|---|
| L1 | `task.reference_url` nomme le même `repo#number` que l'événement | la PR est elle-même la cible, et le cas `issue#N` d'un check-suite dont la branche porte `N` |
| L2 | `task.metadata.claude_pilot.pr_url` nomme la PR de l'événement | la tâche **est** celle dont le dispatch a produit cette PR. **Le lecteur existe déjà dans le même module** : `skills::executor::extract_pr_url` (`:4095`), privé, qui lit la forme imbriquée **et** la forme à plat — donc L2 l'appelle sans rendre quoi que ce soit `pub` et sans écrire un troisième lecteur |
| L3 | un **frère** (même `parent_task_id`, non nul) satisfait L1 ou L2 | **la cascade M4** |
| L4 | le **parent** de la tâche satisfait L1 ou L2 | une re-tentative sous le parent jalon |

Aucun des quatre ⇒ **refus**.

**Aucun appel réseau.** La tentation est de résoudre
`gh pr view --json closingIssuesReferences` pour trancher « le ticket fermé par
cette PR » que nomme AC1 ; elle est **refusée** : ce serait un cinquième
aller-retour GitHub dans une fonction qui en fait déjà plusieurs à 10 s de
timeout, sur un chemin qui spawne un processus, et le lien est **déjà porté en
base** par L2 — `claude_pilot.pr_url` est estampillé par le producteur du
dispatch. Doctrine maison : *la cible est dite, jamais dérivée* (mika#2249,
mika#2368).

### 3.3 Direction du fail-safe — et elle n'est pas uniforme, à dessein

Trois natures d'illisibilité, trois dispositions, chacune alignée sur un
précédent du **même fichier** :

1. **Cible d'événement illisible** ⇒ `NotApplicable` ⇒ **autorise**. Il n'existe
   aucune cible à laquelle borner ; refuser serait refuser sur l'absence de
   question.
2. **Un terme de lignée illisible** (metadata absente, JSON non conforme,
   `reference_url` nulle) ⇒ **ce terme n'est pas satisfait**, les autres
   continuent d'être évalués. C'est la doctrine `live_pilot` appliquée à la
   lettre : *un signal qu'on ne peut pas lire n'est jamais un terme satisfait*.
   **Et c'est porteur ici** : la tâche du constat avait une metadata **vide**, donc
   « metadata absente ⇒ autorise » aurait autorisé l'occurrence mesurée.
3. **La question ne peut pas être posée du tout** (erreur DB sur la traversée)
   ⇒ **refuse**, par cohérence avec la garde voisine `dispatch_check_failed` de
   la même fonction (*« Fail-closed: if we can't check global state, reject
   dispatch »*). Deux gardes de la même fonction qui divergeraient sur l'erreur
   DB seraient une dette de lecture.

**L'asymétrie de coût qui autorise le fail-closed du point 3, nommée :** un faux
refus coûte un dispatch, visible, nommé, rattrapable par un geste d'opérateur (le
refus est écrit dans `tasks.result` par `record_dispatch_rejection`) ; un faux
passage lance un pilote sur un ticket que personne n'a autorisé, brûle un créneau
et ~20 min de travail, et viole le cap. Ce sont deux erreurs de poids différents.

### 3.4 Placement

Dans `validate_dispatch_readiness`, **après** le fetch de la tâche (le prédicat
en a besoin) et **avant** la porte de grooming et les allers-retours GitHub — donc
avant tout coût réseau, et très avant la revendication de bail, qui reste le
dernier terme. Même site, même fonction, même forme de refus JSON que ses cinq
sœurs. **Ne mord que sur `run_claude_pilot` / `run_claude_pilot_groom`** : le
discriminant est `extract_skill_from_input`, comme la garde d'intention
mika#2484 juste au-dessus.

Pré-hoc et non post-hoc, pour la raison que mika#1646 a déjà dû écrire :
`run_claude_pilot` spawne un processus et crée un worktree, donc une garde qui ne
tire qu'après l'exécution **constate** la violation sans l'empêcher.

---

## 4. Surfaces opérateur

### Journal (`$MIKA_SPIRIT_LOG_FILE`)

```bash
# 1. Un dispatch hors lignée a-t-il été refusé ?
grep webhook_dispatch_target_mismatch "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.event == "webhook_dispatch_target_mismatch")
           | {task_id, event_repo, event_number, event_kind, task_reference_url}'

# 2. CONTRÔLE NÉGATIF — la traversée a-t-elle échoué ? (régime attendu : VIDE)
grep webhook_dispatch_lineage_unreadable "$MIKA_SPIRIT_LOG_FILE"

# 3. CONTRÔLE POSITIF — des dispatchs traversent-ils seulement ce prédicat ?
grep -c webhook_dispatch_target_bound "$MIKA_SPIRIT_LOG_FILE"
```

Le `select` sur `.event` est porteur : `grep webhook_dispatch_target_mismatch` est
une **sous-chaîne** et rend aussi tout texte qu'un pilote écrirait *au sujet* du
signal (classe mika#2050, mesurée sur le Signal S).

### Base

```sql
-- La population du refus. SOLE WRITER, donc ce compte est exact.
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'webhook_dispatch_target_binding' GROUP BY 1 ORDER BY 2 DESC;

-- Le détail, avec la cible de l'événement et celle de la tâche
SELECT target_key, created_at, reasoning FROM audit_events
 WHERE tool_name = 'webhook_dispatch_target_binding' AND after_value = 'refused'
 ORDER BY created_at DESC;
```

### Table de lecture

| surface | niveau | régime attendu | lecture |
|---|---|---|---|
| `webhook_dispatch_target_mismatch` | WARN | **vide** | chaque ligne est un pilote qu'un événement PR n'autorisait pas — et l'occurrence du 2026-10-02 rejouée |
| `after_value = 'bound'` | audit | **non vide, faible** | **le contrôle positif** : M4 et les dispatchs liés traversent. Zéro des deux ne prouve rien (mika#2205) |
| `webhook_dispatch_lineage_unreadable` | WARN | **vide** | erreur DB : le prédicat refuse par fail-closed, donc la boucle est gelée sur ce chemin — c'est la base qu'il faut lire |
| `webhook_dispatch_target_binding_audit_failed` | WARN | **vide** | le WARN est passé, l'audit non : le `GROUP BY` sous-compte à partir de là |

`after_value` ∈ `{bound, refused, not_applicable_unread, lineage_unreadable}` —
**format de fil**, site de définition unique, `match` exhaustif sans bras
`_ =>`, épinglé par test (motif `ALL_ITERATE_REFUSAL_REASONS`, mika#2506).
`not_applicable` **n'est pas** audité : il couvrirait tout tour non-PR, soit
l'essentiel du trafic, et serait très exactement le churn que la doctrine
mika#2131 borne. Seul `not_applicable_unread` l'est — un `[GitHub] PR ` dont la
grammaire n'a pas parsé, c'est-à-dire un prédicat qui autorise sans avoir pu
regarder.

---

## 5. AC4 — le recensement, avec un verdict par outil

Le recensement **est** le livrable d'AC4, et il porte sur les outils réellement
servis dans un tour `[GitHub] PR …` / `Check suite …`. Rappel de R6 : ces tours
sont **hors** du domaine Fallthrough, donc `FALLTHROUGH_WITHHELD_TOOLS`
(`["create_task"]`) **ne s'y applique pas** et `create_task` y est servi.

| outil | servi sur un tour PR ? | doit-il être borné à la cible ? | verdict |
|---|---|---|---|
| `run_claude_pilot` | oui | **oui** | **fermé par cette PR** (§ 3) |
| `run_claude_pilot_groom` | oui | **oui** | **fermé par cette PR** — même site, même discriminant |
| `create_task` | **oui** (hors domaine Fallthrough) | oui, mais **transitivement suffisant** | **non fermé, et c'est raisonné** : une tâche créée sans dispatch ne lance aucun processus et ne consomme aucun créneau ; la fermer séparément demanderait la charge utile que R6 refuse, pour un dommage que le terme de lignée intercepte au seul endroit où il devient réel. `4df82c3a` existe encore aujourd'hui et n'a rien coûté d'autre qu'une ligne |
| `update_task_status` | oui | **non** | une transition d'état ne lance rien. Et l'en priver casserait M4, dont l'étape 1 **est** un `update_task_status` sur le frère suivant (`self-dev-webhook-qa:236`) |
| `cancel_task` | oui | **OUI — et c'est le second cas mesuré** | **suivi nommé** (§ 8). C'est lui qui a tué `8a3b2082` et jeté 71 tours |
| `promote_deferred_callback` (mika#1453) | oui | oui | **suivi**, même famille : il force la promotion d'un wrapper, donc il **choisit** quel dispatch prend le créneau. Absent du recensement du ticket ; ajouté ici |
| `pr_merge_with_gate` | oui | non | sa cible **est** la PR, par la forme de son entrée |
| `git_ops`, `run_gh`, `send_message`, `list_tasks`, `check_task` | oui | non | ne touchent pas le plan de dispatch. `run_gh` porte déjà sa propre garde de création de travail (mika#2573), bornée au domaine Fallthrough |

**Pourquoi `cancel_task` n'est pas fermé ici**, alors que c'est le cas le plus
coûteux des deux : son remède n'est pas un terme de cible mais un terme de
**vivacité** — refuser l'annulation d'un pilote vif depuis un tour webhook. Le
lecteur existe (`live_pilot`, mika#2279) et ne demande aucune charge utile, juste
un booléen de classe de tour sur le modèle d'`is_webhook_fallthrough_turn`. Mais
c'est un **cinquième booléen** sur `ToolContext`, un prédicat de vivacité sur un
chemin d'annulation, et un rayon de souffle distinct : un faux refus d'annulation
retire à l'opérateur son geste de reprise le plus court. Périmètre distinct,
ticket distinct — et ce plan le **nomme** plutôt que de l'empaqueter.

---

## 6. Verification contract

### Tests comportementaux — `crates/mika-agent/src/skills/executor.rs` (tests inline)

| # | test | ce qu'il atteste |
|---|---|---|
| V1 | `mika2649_un_tour_pr_dispatchant_un_autre_ticket_est_refuse` | **le rejeu du constat** : événement sur `mika#2647`, tâche fraîche sur `mika#2646`, metadata vide, pas de parent ⇒ `webhook_dispatch_target_mismatch` |
| V2 | `mika2649_un_tour_pr_dispatchant_sa_propre_cible_passe` | contrôle positif L1 |
| V3 | `mika2649_la_cascade_de_jalon_m4_passe` | **contrôle positif L3**, l'exigence dure de R4 : `PR closed:` sur la PR du frère #1, dispatch du frère #2 pending, même `parent_task_id` ⇒ autorisé |
| V4 | `mika2649_la_tache_porteuse_de_la_pr_url_passe` | contrôle positif L2 |
| V5 | `mika2649_un_tour_non_pr_nest_pas_dans_la_population` | `originating_message` Telegram / `[GitHub] Issue …` / `None` ⇒ prédicat non évalué, verdict identique à aujourd'hui |
| V6 | `mika2649_un_check_suite_borne_par_sa_branche` | `(branch: fix/2646/x)` + tâche sur `#2649` ⇒ refus ; même branche + tâche sur `#2646` ⇒ passe |
| V7 | `mika2649_un_check_suite_sans_numero_de_branche_autorise` | `(branch: main)` ⇒ `Branch { issue: None }` ⇒ autorise (point 1 du § 3.3) |
| V8 | `mika2649_la_garde_ne_mord_que_sur_les_deux_outils_de_pilote` | `deploy_mika` et tout autre `skill` ⇒ hors population |
| V9 | `mika2649_une_metadata_illisible_nest_pas_un_terme_satisfait` | point 2 du § 3.3 — le contrôle négatif qui empêche de rouvrir le constat |

### Tests de la moitié pure — `crates/mika-agent/src/webhook_dispatch.rs`

| # | test | ce qu'il atteste |
|---|---|---|
| V10 | `mika2649_la_cible_devenement_est_lue_par_les_lecteurs_uniques` | la matrice des trois variantes sur les grammaires réelles de `format_event_text` |
| V11 | `mika2649_le_vocabulaire_daudit_est_un_format_de_fil` | les quatre valeurs d'`after_value`, site unique, `match` exhaustif |

### Détecteurs structurels

| # | détecteur | ce qu'il refuse |
|---|---|---|
| V12 | `canonical_tokens::tests::mika2649_le_nom_daudit_a_un_seul_ecrivain` | un second écrivain de `webhook_dispatch_target_binding` — c'est ce qui rend le `GROUP BY` exact plutôt qu'un nombre sur lequel deux sites divergent |
| V13 | `webhook_dispatch::tests::mika2649_aucun_second_parseur_de_grammaire_devenement` | une regex de grammaire PR ou de nom de branche recopiée hors de ses deux lecteurs uniques — la classe mika#2158, et c'est **R5 rendu exécutable** |

### Non-régression

- V14 — les ~20 références existantes de `test_is_unauthorized_webhook_dispatch_predicate`
  et la matrice à huit lignes du domaine Fallthrough passent **sans modification**.
  Ce plan **ajoute** un terme, il n'en touche aucun.
- V15 — les tests de cap existants (`executor.rs:8776`–`8990`) passent sans
  modification : aucun octet de la garde par classe ni du bail n'est touché.
- V16 — `make verify-bundled-skills`, `cargo clippy`, `cargo fmt`.

### Ce qui n'est PAS testable ici, écrit plutôt que découvert

Que M4 fonctionne **en vol** demande un jalon réel, un merge de PR réel et un
frère pending réel. V3 atteste la **décision du prédicat** sur la forme de ligne
que M4 produit ; que M4 produise bien cette forme est la sonde S2 du § 7.

---

## 7. Sondes post-déploiement, et leurs cinq haltes

> **Préalable.** Ces mesures décrivent le **binaire servi** : après `make deploy`,
> établir que le `mika-spirit` qui tourne porte le correctif avant toute
> conclusion (classe mika#2340). Ce sont des **gestes d'opérateur** sur l'hôte —
> la base n'est pas montée dans le bac à sable de dispatch.

**S1 — le défaut fondateur ne se rejoue pas** (premier tour webhook PR qui tente).
Attendu : une ligne `webhook_dispatch_target_mismatch`, une ligne d'audit
`refused`, **aucune** ligne callback créée et **aucun** processus lancé.
*Halte 1 — un dispatch hors lignée part quand même, la ligne absente :* **ne pas
élargir le prédicat par réflexe.** Lire d'abord le contrôle positif (grep 3) :
zéro ligne des deux côtés ne prouve rien — *une garde que personne n'a exercée se
lit exactement comme une garde qui marche* (mika#2205). Établir ensuite **par quel
appelant** le dispatch a passé : les trois chemins moteur sont hors population
par conception (R3), et c'est un **résultat**, pas un défaut du prédicat.

**S2 — la cascade de jalon vit toujours (30 jours).** Au moins une ligne
`after_value = 'bound'` sur une tâche dont le `parent_task_id` est non nul, et
aucun jalon arrêté.
*Halte 2 — une cascade M4 est refusée :* c'est le faux positif le plus coûteux de
ce travail, et il **arrête la boucle de jalon**. **Désarmer d'abord** (revert du
terme de lignée), diagnostiquer ensuite : lire lequel de L3/L4 n'a pas tenu, et
si le frère porte bien `claude_pilot.pr_url`. Un jalon gelé ne se règle pas en
bougeant un seuil.

**S3 — contrôle négatif de bruit (7 jours).** Aucun refus sur un dispatch dont
l'événement et la tâche portent la même cible.
*Halte 3 — une occurrence :* L1 ne lit pas `reference_url` comme on croit (forme
d'URL, `/issues/` vs `/pull/`). Réparer la lecture, **pas** élargir vers un
appel réseau.

**S4 — la traversée est lisible (7 jours).**
`webhook_dispatch_lineage_unreadable` reste vide.
*Halte 4 — non vide :* le fail-closed du point 3 gèle le chemin LLM. Lire la base
**avant** de retourner la disposition — inverser ce point rouvrirait le constat.

**S5 — la population hors périmètre (30 jours), et c'est la précondition d'AC2.**
```sql
-- Deux pilotes implement ont-ils coexisté depuis le déploiement ?
SELECT t.id, t.parent_task_id, t.status, t.fired_at, t.process_id
  FROM tasks t
 WHERE t.trigger_type = 'callback' AND COALESCE(t.dispatch_class,'implement') = 'implement'
   AND t.fired_at > '<instant du déploiement>'
 ORDER BY t.fired_at;
```
*Halte 5 — un recouvrement apparaît alors qu'aucun `webhook_dispatch_target_mismatch`
n'est écrit :* le contournement de cap est **réel et d'une autre cause** que celle
du constat. **Ne pas élargir cette garde** : c'est la mesure qui ouvre le suivi
AC2 du § 8, avec le `status` et le `fired_at` des lignes en cause — c'est-à-dire
avec le fait que R1 n'a pas pu établir.

**Halte transverse — les sondes muettes.** Zéro refus **et** zéro `bound` ne
prouve rien : il faut qu'un tour webhook PR ait tenté un dispatch depuis le
déploiement. Vérifier le contrôle positif avant toute conclusion.

---

## 8. Hors périmètre, délibérément — et les suivis avec leur précondition

1. **AC2 — le cap compte des lignes, pas des processus** (R1). **Suivi**,
   précondition écrite : la sonde S5. Son remède probable est de faire consulter
   `live_pilot` par la garde de classe, ce qui demande une traversée **par
   classe** (et non par issue, la seule forme existante) **et** un arbitrage de
   fail-safe **inverse** de celui de `live_pilot` — là `Unreadable` ⇒ ne pas
   bloquer, ici ⇒ bloquer. Rayon de souffle : tous les dispatchs de la flotte.
   Pas un ajout, une arbitration.
2. **`cancel_task` sur un pilote vif depuis un tour webhook** (§ 5) — le second
   cas mesuré, 71 tours jetés. **Suivi**, précondition : aucune, le cas est
   mesuré ; ce qui le sort d'ici est le rayon de souffle (un faux refus retire à
   l'opérateur son geste de reprise) et le cinquième booléen de `ToolContext`.
3. **`promote_deferred_callback`** (§ 5) — même famille, population non mesurée.
4. **Le re-prompt « required tools »**, que le ticket désigne comme déclencheur
   probable (`webhook_zero_tools`, INTENT_GUARDS). Le modifier changerait le
   contrat de **tous** les tours webhook pour un défaut dont la cause proximale
   est l'absence de borne, pas la présence du re-prompt. Et par
   `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`, la
   moitié qui tient est la garde, pas l'injonction.
5. **Remettre `[GitHub] PR ` / `Check suite ` dans le domaine Fallthrough** —
   refusé sur la mesure de R4 : casserait M4.
6. **La résolution `gh pr view --json closingIssuesReferences`** — refusée au
   § 3.2 avec son motif (coût réseau sur un chemin qui spawne, et le lien est
   déjà en base).
7. **Les trois chemins moteur** — inchangés (R3).
8. **Le bail de créneau, le domaine Fallthrough, `FALLTHROUGH_WITHHELD_TOOLS`,
   la garde d'intention mika#2484, la porte de siège mika#2084, l'allowlist de
   dépôts mika#2046** — aucun octet touché.

---

## 9. Ce que ce travail n'achète PAS

- **Il ne répare pas le cap.** Il rend le dispatch hors lignée impossible sur le
  chemin mesuré ; un contournement de cap par une autre cause reste ouvert, et
  S5 est ce qui le dimensionne.
- **Il ne rattrape pas l'incident du 2026-10-02.** `4df82c3a` et `f8f817ce`
  restent ce qu'ils sont, et **rien ne rétro-estampille** : fabriquer une ligne
  d'audit datée d'un refus qu'on n'a pas observé est l'inverse de ce que ce
  travail défend. La sonde est la **prochaine** occurrence.
- **Il ne ferme pas le second cas de la même famille** (`cancel_task`, § 8.2).
- **Il ne rend pas le modèle incapable de vouloir dispatcher** : il rend le
  dispatch impossible. La doctrine maison est *construis l'incapacité, ne promets
  pas la retenue* (mika#1991), et **elle est applicable ici** — il y a bien une
  capacité à retirer, et c'est ce qui est retiré.
- **Il rend le champ lisible, pas surveillé.** Les seuls instruments sont les
  greps et les requêtes du § 4, et **leur silence ne prouve rien tant que
  personne ne les exécute.**
- **Un tour webhook servi en mode silencieux échapperait à la garde** — borne
  héritée de mika#2517 / mika#2573, ni élargie ni modifiée, population mesurée
  vide (un tour silencieux a `originating_message = None`).

---

## 10. Fire-Disposition

Ce plan livre des détecteurs : les tests V1–V11 et **deux scans structurels**,
V12 (écrivain unique du nom d'audit) et V13 (aucun second parseur de grammaire).

**Disposition retenue : (a) exception nommée en allowlist — et les deux
allowlists sont livrées VIDES.**

- **V12** — `canonical_tokens::tests::mika2649_le_nom_daudit_a_un_seul_ecrivain`
  suit le motif établi (`mika#2506`, `mika#2242`, `mika#2522`) : une constante
  `TARGET_BINDING_AUDIT_WRITERS_ALLOWED: &[&str] = &[]`, grep-visible, livrée
  vide. **Aucune violation préexistante** : le nom `webhook_dispatch_target_binding`
  est créé par cette PR, donc son unique écrivain est le site créé par cette PR.
  La résolution quand le scan tire est de **retirer le second écrivain**, jamais
  d'ajouter une entrée (doctrine mika#2201).
- **V13** — `webhook_dispatch::tests::mika2649_aucun_second_parseur_de_grammaire_devenement`
  porte `EVENT_GRAMMAR_PARSER_SITES_ALLOWED: &[&str] = &[]` et scanne
  `crates/mika-agent/src` pour une regex de grammaire PR (`[GitHub] PR `) ou de
  nom de branche en **position de parseur** hors de `deadline_verdict::parse_pr_target`
  et `worktree_reaper::issue_number_from_branch`. **Le scan a été exercé avant
  d'être livré** : les sites existants qui *citent* ces préfixes sont des
  prédicats de préfixe (`starts_with` dans `is_webhook_fallthrough_domain`), pas
  des parseurs, et sont hors population par le même terme positionnel que
  mika#2496 (un `starts_with` n'extrait rien). Si le scan rougit sur un site
  légitime à l'implémentation, le terme positionnel est à resserrer — **pas
  l'allowlist à peupler**.

**Anti-vacuité, pour les deux.** Chaque scan assertionne que sa population n'est
pas vide (`population.len() >= 1` pour V12, le compte des deux lecteurs uniques
pour V13) : *un scan silencieusement devenu aveugle se lit exactement comme un
arbre propre* (classe mika#2205). Et V13 porte une **assertion auto-nettoyante**
sur ses deux lecteurs — si l'un est renommé ou supprimé, le scan rougit au lieu
de cesser de regarder.

**Les tests comportementaux V1–V11 ne demandent aucune disposition** : ils ne
scannent rien et n'ont aucune population préexistante à exempter.

---

## 11. Definition of Done

- [ ] `webhook_dispatch::webhook_event_target` + `WebhookEventTarget` livrés,
      purs, appelant les deux lecteurs uniques de R5.
- [ ] Le terme de lignée (L1–L4) livré dans `validate_dispatch_readiness`, au
      placement du § 3.4, bornant `run_claude_pilot` et `run_claude_pilot_groom`
      et rien d'autre.
- [ ] Les trois dispositions de fail-safe du § 3.3 implémentées **et chacune
      épinglée par un test** (V7, V9, et le contrôle d'erreur DB).
- [ ] Refus nommé, audité, `after_value` en format de fil à site unique,
      écrivain unique, et écrit dans `tasks.result` via
      `record_dispatch_rejection` comme ses cinq sœurs.
- [ ] V1–V16 verts, V3 compris (**l'exigence dure M4**).
- [ ] Les deux allowlists de Fire-Disposition livrées vides, avec leur assertion
      d'anti-vacuité.
- [ ] Section `CLAUDE.md` racine (§ Environment Variables est le mauvais
      voisinage — ce travail n'ajoute **aucune** variable ; la section va près de
      *La surface « propose »* / *Un tour Webhook Fallthrough ne crée pas de
      travail par `run_gh`*, dont elle est la voisine de famille) : les six
      rectifications, les quatre termes, les trois dispositions, les surfaces, les
      cinq haltes, le hors-périmètre.
- [ ] Section détaillée dans `crates/mika-agent/CLAUDE.md`, et **rectification du
      § qui nomme le résidu per-issue** (`ligne 3656`) pour pointer ce terme.
- [ ] `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt`,
      `make verify-bundled-skills`.
- [ ] Corps de PR : les six rectifications, la sortie d'AC2 **avec sa
      précondition**, les trois suivis du § 8 avec leurs préconditions, et une
      ligne `Tracked in:` pour chacun.

---

## 12. Acceptance criteria

> Transcrits du corps de senara-solutions/mika#2649, avec la disposition que ce
> plan retient pour chacun. Les divergences sont **datées et motivées** ci-dessus,
> jamais silencieuses.

- [ ] **AC1.** Dans un tour déclenché par un événement `[GitHub] PR …` /
      `Check suite …`, `run_claude_pilot` (et la création de la tâche qui le
      porte) n'est autorisé que pour la **PR / le ticket de l'événement** (même
      `reference_url`, ou le ticket fermé par cette PR). Toute autre cible est
      refusée à la frontière d'outil, avec un refus nommé et audité.
      → **Tenu, avec deux divergences motivées.** (i) Le critère n'est pas
      l'identité de la cible mais la **lignée** (L1–L4), parce que la cascade de
      jalon M4 dispatche légitimement un autre ticket que la PR (R4) — un
      prédicat littéral casserait la boucle de jalon. (ii) « le ticket fermé par
      cette PR » est lu **en base** (`claude_pilot.pr_url`, L2) et non par un
      appel réseau (§ 3.2). *« la création de la tâche qui le porte »* n'est pas
      bornée séparément : son verdict et son motif sont au § 5.
- [ ] **AC2.** Le cap implement est vérifié au point de dispatch quel que soit le
      chemin ; un dispatch qui le violerait est refusé ou différé, jamais lancé
      en parallèle.
      → **Hors périmètre de cette PR, et déjà satisfait tel qu'écrit** (R1) : la
      garde par classe tourne sur les quatre appelants et un bail atomique la
      double. Ce qui est faux est **ce que le cap compte** (des lignes, pas des
      processus) ; le mécanisme du contournement n'est **pas établissable** depuis
      le bac à sable. Précondition : sonde S5. Suivi § 8.1. **Le p0 est fermé
      sans elle** (R2).
- [ ] **AC3.** Test : un tour webhook PR sur la PR X qui tente `run_claude_pilot`
      pour un ticket Y ≠ X est refusé ; le dispatch légitime CI-fix / QA-hold sur
      X passe (contrôle positif).
      → **Tenu, avec son contrôle positif rectifié.** V1 est le refus. Le
      contrôle positif porte sur la population qui **traverse réellement** le
      prédicat — V2 (même cible), V3 (**M4**), V4 (`pr_url`) — parce que les
      dispatchs CI-fix / QA-hold sont des dispatchs **moteur** avec
      `originating_message = None`, donc hors population par construction (R3).
- [ ] **AC4.** Recenser les autres outils de plan de dispatch accessibles dans
      ces tours (`cancel_task`, `create_task`, `update_task_status`) et dire pour
      chacun s'il doit être borné à la cible de l'événement.
      → **Tenu, et étendu.** Recensement avec verdict et motif au § 5, augmenté
      de `run_claude_pilot_groom`, `promote_deferred_callback`,
      `pr_merge_with_gate` et des outils de lecture — quatre entrées que le
      ticket n'énumérait pas. Deux verdicts « oui » sont des suivis nommés avec
      leur rayon de souffle (§ 8.1, § 8.2) ; deux verdicts « non » portent leur
      raison, dont `update_task_status`, que M4 **exige**.
