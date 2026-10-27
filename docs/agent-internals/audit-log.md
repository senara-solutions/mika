# Audit Log

Moved verbatim out of `crates/mika-agent/CLAUDE.md` on 2026-10-27 (mika#2922, vol 2/3). That file is loaded whole into a pilot's prompt as soon as the pilot reads anything under `crates/mika-agent/`, and `crates/mika-common/tests/claude_md_size.rs` caps every `CLAUDE.md` in the repository; adding the `mpc_flight_judgment` entry took it over the cap. The section grows by one subsection per new `tool_name`, so it was the whole section that moved and not the new entry alone. Headings keep their original level.

`audit_events` table tracks all memory mutations per session. All writes include `trace_id`.

### `tool_name = 'auth_boundary'` — cross-boundary authentication failures (mika#1949, Porte 3)

The mika-manager write path crosses four boundaries, each guarded by a different
env-var-backed token. Before mika#1949 a failure at any of them was
indistinguishable from a network error — mika#2013 is the measured precedent:
a frozen installation token cycled `auth_class=401` sixteen times in one night
without naming itself. These rows are the fix: one per observed authentication
failure, naming the token and the boundary.

**Row shape** (written only by `Database::record_auth_boundary`, `evidence/audit.rs`):

| column | value |
|---|---|
| `tool_name` | `auth_boundary` (`evidence::audit::AUTH_BOUNDARY_TOOL_NAME`) |
| `session_id` | `auth-boundary` (`AUTH_BOUNDARY_SESSION_ID`) — a boundary failure belongs to no conversation |
| `target_key` | `<from>_to_<to>` |
| `after_value` | the failure kind — `missing` \| `empty` \| `invalid` \| `rejected` \| `unreachable` |
| `reasoning` | JSON `{"token_name","kind","from","to"}` |
| `before_value` | always `NULL` — there is no prior state to name |

**The four boundary pairs `target_key` can take:**

| `target_key` | token | site |
|---|---|---|
| `gateway_to_spirit` | `MIKA_INTERNAL_TOKEN` | `server/auth.rs` refusal arms |
| `cm_to_spirit` | `INTERNAL_TOKEN` | control-monitor `A2aAdapter` (mirrored shape, cm's own ledger) |
| `cm_to_content_plane` | `CM_FULL_ACCESS_TOKEN` | cm `scope::check` |
| `manager_to_delivery` | `MIKA_MANAGER_DELIVERY_TOKEN` | `milestone_manager` report + alarm delivery |

**Two properties, both load-bearing:**

- **A token NAME, never a token VALUE.** `AuthBoundaryError` (in
  `mika-common::auth_boundary`) has no field that can hold a secret, and both
  it and this writer carry a negative-control test built with a secret-shaped
  *name* to pin that.
- **Fire-and-forget.** Call sites go through
  `crate::auth_boundary_ledger::AuthBoundaryLedger`, whose `record` returns
  `()`. An audit failure cannot change an authentication verdict, because the
  signature offers no way to propagate one. A failed authentication still drops
  the request exactly as before; only the drop became visible.

**Operator query:**

```sql
SELECT target_key, after_value, count(*)
FROM audit_events
WHERE tool_name = 'auth_boundary'
GROUP BY 1, 2;
```

Rotation procedure for all four tokens (and the deliberate 401/403 divergence
between mika and cm): `mika-platform/docs/operator/token-rotation-procedure.md`.

### `tool_name = 'agent_recurring_gate'` — récurrentes refusées au tir (mika#2456, vol 3 / mika#2843)

Le filet au tir de la porte récurrente niveau-agent refuse une récurrente dont
l'agent porte `enabled = false` à la racine de son `identity.toml`. Il couvre les
deux populations que la porte à l'**enregistrement** (vol 1) ne voit pas : les
lignes nées **avant** son déploiement, et les bascules d'`enabled` **à chaud**.
En régime attendu ces lignes sont à **zéro** — et ce zéro est l'assertion que la
porte amont tient.

**Row shape** (écrite uniquement par
`TaskDispatcher::refuse_recurring_fire_if_agent_disabled`,
`task_engine/dispatcher.rs`) :

| column | value |
|---|---|
| `tool_name` | `agent_recurring_gate` (`RECURRING_GATE_AUDIT_TOOL_NAME`) |
| `session_id` | `agent-recurring-gate` (`RECURRING_GATE_AUDIT_SESSION_ID`) — un refus de porte n'appartient à aucune conversation, même raison qu'`auth-boundary` ci-dessus |
| `target_key` | `<agent_id>@<label>` — l'`agent_id` vient de `self.db.agent_id()`, la **même** source que le WARN, pour que journal et table se soustraient |
| `after_value` | `refused_agent_disabled` |
| `reasoning` | `task_id=<n>` — l'id de tâche est une **donnée**, jamais une clé de regroupement |
| `before_value` | toujours `NULL` — le filet ne mute rien (mika#2156), il n'y a aucun état antérieur à nommer |

**Operator query:**

```sql
SELECT target_key, after_value, count(*)
FROM audit_events
WHERE tool_name = 'agent_recurring_gate'
GROUP BY 1, 2;
```

Son résultat est **exactement** la population des lignes qui échappent à la porte
à l'enregistrement, et non un mélange de deux mécanismes : le refus à
l'enregistrement n'écrit délibérément aucune ligne d'audit (il est borné par la
cadence des redémarrages là où le tir ne l'est pas), et l'écrivain unique est
tenu par `mika2456_the_gate_audit_name_has_exactly_one_writer`
(`canonical_tokens.rs`), dont l'allowlist est livrée vide.

**Deux propriétés, toutes deux load-bearing :**

- **Fire-and-forget.** Une écriture d'audit en échec ne change pas le verdict du
  filet : la fonction rend `true` hors du `match`, le refus reste un refus, et
  un WARN nomme l'échec. Même forme et même raison qu'`AuthBoundaryLedger` —
  un audit ne décide rien.
- **Déduplication 24 h, en mémoire, perdue au redémarrage.**
  `RECURRING_GATE_AUDIT_REFRESH` borne une écriture par `(agent_id, label)` et
  par jour ; sans elle une récurrente à la minute écrirait ≈ 1 440 lignes par
  jour pour un seul agent désactivé. L'état vit dans un `static` du processus :
  **le premier refus après un redémarrage réécrit une ligne**, et c'est voulu —
  un restart est une information, et un processus neuf re-photographie l'état
  qu'il trouve.

**Changement de contrat du WARN, à lire avant d'interpréter un log (mika#2843
C3).** Le WARN `recurring_fire_refused_agent_disabled` était inconditionnel
jusqu'au vol 3 ; il part désormais sous le **même prédicat** que la ligne
d'audit. Donc :

> l'absence de `recurring_fire_refused_agent_disabled` dans les logs signifie
> **« déjà annoncé dans les 24 h »**, et non « aucun refus ».

Pour la question « y a-t-il des refus », c'est la table qu'on lit, pas le
journal. Garder le WARN inconditionnel était l'alternative, refusée : elle
aurait fait du journal et de la table deux populations non soustractibles
(classe mika#2205).

### `tool_name = 'manager_flight_verdict'` — le verdict de l'évaluateur de vols (mika#2922, vol 1)

Un `FlightVerdict` (`milestone_manager/evaluator.rs`) n'existait que dans le texte
du rapport du sink et dans le compteur `verdicts` de `manager_flight_scan` : ni
table, ni `audit_events`. Le seuil qui ouvre l'écriture de l'évaluateur — *les
mêmes verdicts que MPC sur 10 vols consécutifs ou 7 jours* — était donc
immesurable. Ces lignes sont la moitié amont de la mesure : **une ligne par vol,
écrite par le cycle qui le juge pour la première fois**.

**Et depuis le vol 3, un lecteur les lit** : `flight_verdict_ledger::read_concordance`,
l'unique lecteur des deux registres, rendu à l'opérateur par
`mika flight-verdicts concordance`. C'est lui qui met les deux côtés en face et
qui rend le compte de vols consécutifs concordants — le seuil de mika#2698.

**Row shape** (écrite uniquement par `flight_verdict_ledger::write_and_swallow`) :

| column | value |
|---|---|
| `tool_name` | `manager_flight_verdict` (`MANAGER_FLIGHT_VERDICT_TOOL`) |
| `session_id` | `manager-flight-verdict` (`MANAGER_FLIGHT_VERDICT_SESSION_ID`) — un verdict de cycle n'appartient à aucune conversation, même raison qu'`auth-boundary` ci-dessus |
| `target_key` | `FlightVerdict::task_id` (`tasks.id`) — « la clé qui rend le verdict rejouable » |
| `after_value` | JSON serde de `VerdictRecord { issue, recommendation }`, donc des types **existants** (`FlightIssue`, `DeathCause`, `FlightRecommendation`) : la graphie des termes ne peut pas diverger du code (classe mika#2158) |
| `reasoning` | `issue_number=<n>` quand le vol porte un ticket lisible, sinon `NULL` — une **donnée**, jamais une clé de regroupement (même règle qu'`agent_recurring_gate`) |
| `before_value` | toujours `NULL` — il n'y a aucun état antérieur à nommer |

L'`agent_id` n'est **pas** un paramètre : `AsyncDatabase::log_audit_event` clone le
sien, donc le cloisonnement par agent est structurel et non une discipline d'appel.

**Lecture d'opérateur : `mika flight-verdicts concordance`** (`--format text|json|yaml`,
`text` par défaut). Elle passe par `read_concordance`, qui lit cette ligne et celle
de MPC par `Database::list_audit_events_for_tool_since` — une lecture fenêtrée sur
la même `AUDIT_RETENTION_DAYS` que l'idempotence ci-dessous.

**Repli SQL**, pour une base qu'aucun binaire courant ne sert ou pour un décompte
que la commande ne rend pas :

```sql
SELECT target_key, after_value, count(*)
FROM audit_events
WHERE tool_name = 'manager_flight_verdict'
GROUP BY 1, 2;
```

**Sonde :** `event = "manager_verdict_inscription"` (INFO, `target:
"mika::milestone_manager"`), une ligne par cycle sur **toutes** les branches —
`unwired` (aucun registre injecté), `no_verdicts`, `written` — portant `scope`,
`candidates`, `inscribed`, `already_recorded` et `failed`. Ces deux derniers sont
deux champs et **jamais une somme** : les additionner rendrait « 3 nouveaux » et
« 3 déjà là » identiques (classe mika#2205). L'absence de la ligne signifie un
binaire antérieur au vol (motif mika#2340), jamais une absence d'inscription.

**Trois propriétés, toutes load-bearing :**

- **Aucune autorité sur la forge.** `FlightVerdictLedger::record` rend un
  `InscriptionTally` de trois `usize` et jamais un `Result` : une écriture en
  échec ne peut pas changer un verdict, parce que la signature n'offre aucun
  moyen de le propager — même forme et même raison qu'`AuthBoundaryLedger`.
  L'inscription est néanmoins **attendue** et non détachée, pour que « exactement
  une ligne par vol jugé » soit mesurable dans le cycle qui la produit.
  L'écrivain vit **hors** de `milestone_manager/` — comme `flight_source.rs`, et
  pour la même raison — afin que `no_dispatch_test.rs` passe sans modification de
  son ensemble de jetons. Son unicité est tenue par
  `mika2922_ac2_un_seul_ecrivain_du_verdict_du_manager` via
  `source_scan::audit_writers`, avec son contrôle négatif.
- **Idempotence bornée par la rétention, pas par une contrainte.** Un vol déjà
  inscrit dans la fenêtre `AUDIT_RETENTION_DAYS` (90 j, `evidence/audit.rs`) n'est
  pas réécrit : le test est une lecture préalable dérivée de cette constante, non
  un `UNIQUE`. Au-delà de la rétention la ligne n'existe plus, et une
  réinscription est alors la bonne réponse — pas un doublon.
- **Un seul lecteur, pour les deux registres à la fois.**
  `mika2922_ac3_un_seul_lecteur_du_registre` refuse, dans la moitié production de
  ce crate et hors de `flight_verdict_ledger.rs`, tout site qui **nomme** l'un
  des deux `tool_name` — par son littéral ou par sa constante. Le prédicat porte
  sur le nom et non sur la forme d'une lecture, parce que « lire le registre »
  n'a pas de forme syntaxique stable alors que toutes les lectures ont besoin du
  `tool_name` : il subsume donc la lecture et l'écriture, et c'est ce qui le rend
  auto-nettoyant. Aucune liste d'exception — elle est **absente**, pas vide
  (doctrine mika#2201) : quand la garde rougit, on route le site par
  `read_concordance` ou par `record` / `record_mpc_judgment`. Les commentaires
  sont retirés du balayage : parler du registre n'est pas le lire, et refuser
  qu'on en parle ferait de cette garde une taxe sur la documentation (forme du
  signal S de mika#2050).

### `tool_name = 'mpc_flight_judgment'` — le jugement de MPC sur un vol (mika#2922, vol 2)

L'autre moitié de la mesure. **La clé est le MÊME `tasks.id` que la ligne du
manager** — c'est ce qui permet de mettre les deux côtés en face — et le
`tool_name` diffère, ce qui interdit de les confondre. Avec un `tool_name`
partagé, l'idempotence de chaque côté serait celle de l'autre, et le compte de
concordance (vol 3) lirait un accord là où un seul des deux a parlé.

**Et depuis le vol 3, le compte existe** : `flight_verdict_ledger::read_concordance`
met cette ligne en face de celle du manager, et
`mika flight-verdicts concordance` la rend à l'opérateur. Un vol que MPC n'a pas
jugé y apparaît **« non jugé par MPC »** et ne compte ni pour ni contre : la
ligne absente n'est pas un désaccord (motif mika#2331).

**Row shape** (écrite uniquement par `flight_verdict_ledger::record_mpc_judgment`) :

| column | value |
|---|---|
| `tool_name` | `mpc_flight_judgment` (`MPC_FLIGHT_JUDGMENT_TOOL`) |
| `session_id` | `mpc-flight-judgment` (`MPC_FLIGHT_JUDGMENT_SESSION_ID`) |
| `target_key` | `tasks.id` — la même clé que la ligne du manager, à l'octet |
| `after_value` | JSON serde du **même** `VerdictRecord { issue, recommendation }` que le vol 1, donc la même grammaire — issue, cause, recommandation |
| `reasoning` | la note libre de l'opérateur (`--note`), telle quelle, ou `NULL`. **La colonne ne porte pas la même donnée des deux côtés, et c'est délibéré** : côté manager elle porte `issue_number=<n>`, ici une note. Le compte de concordance lit donc le ticket d'un vol sur la ligne **du manager**, jamais sur celle-ci |
| `before_value` | toujours `NULL` |

**Le chemin supporté est `mika flight-verdicts judge`**, jamais du SQL à la
main — le ticket l'exige en ces termes. La sous-commande est un adaptateur
mince (`crates/mika-cli/src/commands/flight_verdicts.rs`) : elle analyse les
trois termes, inscrit, et rend ce qui s'est passé.

**L'analyse des termes est l'inverse exact de serde**, et ne tient aucune table
de graphies : `parse_judgment_terms` bâtit la `Value` de la forme que
`rename_all = "snake_case"` produit et la passe à `serde_json::from_value`, donc
une sixième cause de mort ajoutée en amont est acceptée sans qu'une ligne bouge
(classe mika#2158). `DeathCause::Unknown` n'est pas offrable, et son refus est
**structurel** : la forme serde d'une variante newtype est un objet, jamais un
terme nu. Un terme hors contrat est refusé en nommant les valeurs acceptées.

**Lecture d'opérateur : `mika flight-verdicts concordance`**, la même commande que
pour le registre du manager — c'est tout l'objet du compte que de ne pas lire les
deux côtés séparément.

**Repli SQL** (noter que `ORDER BY created_at` est ici acceptable parce qu'on
lit, pas parce qu'on ordonne : l'accesseur du registre trie par `id`, seul ordre
total à la seconde près — voir `db/tests/fenetre_daudit_par_outil.rs`) :

```sql
SELECT target_key, after_value, reasoning
FROM audit_events
WHERE tool_name = 'mpc_flight_judgment' ORDER BY created_at DESC;
```

**Deux propriétés, toutes deux load-bearing :**

- **Les erreurs se propagent**, contrairement à `write_and_swallow` du vol 1 :
  l'appelant est une commande d'opérateur, et avaler l'échec y dirait
  « inscrit » à quelqu'un qui n'a rien inscrit. Un vol déjà jugé rend
  `AlreadyRecorded` portant le jugement en place — `None` quand il est
  illisible, parce qu'un `VerdictRecord` fabriqué pour combler serait un
  jugement inventé (motif mika#2331) ; la perte reste visible par le WARN
  `mpc_judgment_held_unreadable` (`stage` ∈ `{read, decode}`). La commande sort
  **à 0** sur un vol déjà jugé : un non-zéro sur une simple reprise casserait un
  rejeu scripté idempotent.
- **Un seul écrivain, attesté des DEUX côtés.** `production_files` est enraciné
  sur le `src` de *ce* crate, donc un écrivain côté `mika-cli` serait invisible à
  `mika2922_ac2_un_seul_ecrivain_du_jugement_de_mpc`. Le côté CLI porte sa propre
  garde, dans son propre crate
  (`commands::flight_verdicts::tests::mika2922_ladaptateur_nest_pas_un_ecrivain`),
  dont le prédicat est le `tool_name` et **jamais** `log_audit_event` — ce crate
  écrit légitimement des lignes d'audit sous d'autres noms d'outil. Aucune des
  deux ne porte de liste d'exception : quand l'une rougit, une écriture se route
  par `record_mpc_judgment` et une lecture par `read_concordance`.
- **Un seul lecteur, et il est ici.** `mika2922_ac3_un_seul_lecteur_du_registre`
  (décrit dans la section du vol 1, parce qu'il couvre les deux `tool_name` d'un
  seul prédicat) refuse tout second site nommant ce `tool_name` dans la moitié
  production de ce crate. La garde CLI
  (`mika2922_ladaptateur_nest_pas_un_ecrivain`) le refuse aussi côté `mika-cli` :
  l'adaptateur ne nomme ni l'un ni l'autre, et c'est précisément pourquoi
  `read_concordance` vit dans `flight_verdict_ledger.rs` et non dans la commande.
  Enracinée côté CLI, une garde du lecteur serait verte par construction (classe
  mika#2205).

L'idempotence réemploie la fenêtre `AUDIT_RETENTION_DAYS` et l'accesseur du
vol 1. Le vol 3 ajoute une **quatrième** lecture par `tool_name`,
`Database::list_audit_events_for_tool_since` — la seule qui porte une borne basse
et projette `created_at` — mais toujours aucune table et aucune migration : le
schéma reste à v56.
