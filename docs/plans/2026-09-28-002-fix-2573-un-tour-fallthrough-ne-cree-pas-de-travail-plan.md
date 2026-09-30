# Plan — mika#2573 : un tour Webhook Fallthrough ne crée pas de travail par `run_gh`

- **Ticket :** senara-solutions/mika#2573 (`bug`, `p1-important`, `loop-substrate`, `dispatch:loop`)
- **Parent conceptuel :** mika#2517 (retenue de `create_task` sur le tour Fallthrough)
- **Classe :** substrat de boucle — contournement d'une retenue par l'outil voisin

---

## 1. Ce que la lecture du code établit, et ce qu'elle déplace dans le ticket

Le ticket a raison sur le fond, sur le remède et sur sa formulation
(« refuser et non retirer »). Quatre lectures s'y ajoutent, et chacune change
un détail d'exécution ; elles sont le premier livrable de ce plan.

### M1 — mika#2517 a écrit en toutes lettres pourquoi il n'avait pas fait ça, et l'objection doit être répondue, pas contournée

Le doc-comment d'`effective_disabled_tools` (`agent_loop/mod.rs`) dit :

> *« Refusing at the tool boundary would need `ToolContext` to carry
> `originating_message`, which it does not — that is a new field threaded to
> four construction sites. Withholding is one site and no new field. »*

L'objection porte sur `originating_message` — une `&'a str` avec sa durée de
vie et sa charge utile. **Ce plan ne la fait pas traverser.** Il fait traverser
un **booléen**, calculé au seul site qui possède déjà le message et qui appelle
déjà le prédicat de domaine. Et cette forme n'est pas neuve : `ToolContext`
porte déjà **trois** booléens de classe de tour — `is_reflection`,
`is_task_context`, `is_callback_turn`. Le quatrième est l'idiome de la maison,
pas une exception.

### M2 — Pourquoi « refuser » et non « retirer » : c'est le prompt lui-même qui l'impose

`skills/bundled/self-dev/system_prompt.md` § *Webhook Fallthrough*, SCOPE RULE :

> *« Do NOT `list_tasks`, create new tasks, or call `run_claude_pilot` **unless
> you first `run_gh issue view <n> --json labels`** on the referenced issue and
> confirm `ready` is present. »*

Le geste de vérification **prescrit** par le prompt est un `run_gh` en lecture.
Ajouter `run_gh` à `FALLTHROUGH_WITHHELD_TOOLS` casserait la seule sortie
correcte que le prompt offre. La retenue est donc *indisponible* ici, et le
refus est la seule forme qui reste — ce que le ticket demande mot pour mot.

### M3 — Comment `run_gh` atteint un tour Fallthrough, et pourquoi la population n'est pas exotique

`run_gh` est déclaré par **un seul** skill : `github`
(`crates/mika-agent/templates/skills/github/tools.json`). Vérifié : aucun
`tools.json` de `skills/bundled/` ne le déclare (`qa-review` expose
`qa_pr_view`, et tire `github` par dépendance). Or `github` est
keyword-triggered et ses mots-clés incluent `label`, `add label`,
`remove label`, `create issue`, `close pr`, `pull request`.

**C'est-à-dire que le vocabulaire même d'un webhook de label ou d'issue est ce
qui fait entrer l'outil dans le tour.** La rencontre mesurée n'est pas un
hasard ; c'est le régime.

### M4 — La moitié prompt existe DÉJÀ et n'a pas tenu

Deux sections l'écrivent : la SCOPE RULE ci-dessus (« Do NOT … create new
tasks ») et la Rule 9 (« Do NOT … call `list_tasks`, `create_task`, or
`run_claude_pilot` »). Ni l'une ni l'autre ne nomme un verbe `run_gh`, et le
modèle a **substitué** exactement là où l'énumération s'arrêtait — puis a
qualifié lui-même son geste d'« erreur ». C'est la forme que
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` décrit :
au substrat de la boucle, la moitié prompt exprime l'intention et ne la tient
pas. Ce plan livre la moitié prompt **comme intention**, et la moitié
structurelle comme garantie.

---

## 2. Périmètre du refus, et pourquoi il est complet plutôt qu'un jeu de taupes

Le ticket nomme deux formes. La question qu'un relecteur posera est : *est-ce
que le modèle ne peut pas simplement passer par un troisième verbe ?* Sur la
surface `run_gh`, la réponse est non, et voici l'inventaire.

| vecteur | statut | raison |
|---|---|---|
| `gh issue create` | **refusé (U2/U3)** | le substitut mesuré à la tâche retenue |
| `gh issue edit … --add-label ready` | **refusé (U2/U3)** | `ready` est le signal canonique de dispatch (mika#841) |
| `gh issue create --label ready` | **refusé** | couvert par le refus du verbe `issue create` entier |
| `gh api … /labels` | **déjà impossible** | `GH_API_ALLOW_MATRIX` est deny-by-default : 4 GET + 1 PATCH milestone, aucune écriture de label (mika#1167) |
| `gh label create ready` | **hors population** | gère une *définition* de label, pas une assignation ⇒ aucun événement `labeled` |
| `gh pr edit --add-label ready` | **hors population, nommé** | `READY_LABEL_DISPATCH_MARKER` est porté par `issues.labeled` ; un label de PR ne déclenche aucun dispatch. Armer une garde sur une population vide est ce que mika#2536 refuse |
| `gh issue reopen` | **hors population, nommé** | GitHub n'émet `labeled` que sur une **transition** (mika#2323) ; rouvrir une issue déjà `ready` n'émet rien |

### Le vecteur qui RESTE ouvert, et il est nommé plutôt que caché

`shell-exec` est **`always_on = true`**, donc `run_shell` est présent dans
**tout** tour de conversation, Fallthrough compris. Un
`run_shell "gh issue edit 2562 --add-label ready"` contourne intégralement une
garde posée sur `run_gh`.

Ce n'est pas fermé ici, et la raison est mesurée : le défaut observé est passé
par `run_gh`, la surface de `run_shell` est du shell arbitraire, et il existe
déjà un précédent de garde argv côté shell (`guard-shared-checkout`,
mika#2449) — donc le suivi a une forme, il n'a pas de mesure. **Ticket de
suivi, précondition : la sonde S4 ci-dessous montre au moins une occurrence.**

---

## 3. Livrables

### U1 — `ToolContext.is_webhook_fallthrough_turn: bool`

Nouveau champ, à côté de ses trois frères de classe de tour.

- **Calculé au seul site de conversation** (`agent_loop/mod.rs`, le
  `ToolContext` de `run_agent_inner`) :
  `crate::webhook_dispatch::is_webhook_fallthrough_domain(params.user_message)`.
  `params.user_message` y est déjà disponible. **Le prédicat est appelé, jamais
  recopié** : `mika2517_the_fallthrough_domain_has_a_single_definition`
  (`canonical_tokens.rs`, allowlist livrée vide) refuse un second corps, et
  c'est très exactement la classe `grooming_marker` (mika#2158) que ce scan
  existe pour tenir.
- **Les trois autres sites de production écrivent `false`** : silent
  (`agent_loop/mod.rs`), team (`agent_loop/mod.rs`), `server/investigate.rs`.
  Ce n'est pas une approximation, c'est le **même** périmètre que celui
  qu'`effective_disabled_tools` documente déjà : un webhook arrive par
  `POST /message` → `run_agent` → mode conversation ; un tour silencieux n'a pas
  de message de webhook (`originating_message` vaut `None` depuis mika#933) ; un
  tour d'équipe lit `TeamAgentParams`. Le bord est écrit au doc-comment du champ
  et hérite de la sonde S3 de mika#2517.
- **Coût mécanique assumé :** ~16 sites littéraux de test reçoivent `false`
  (`test_utils.rs` en couvre la majorité par ses cinq constructeurs partagés ;
  `send_message.rs` ×6, `toggle_skill.rs` ×2, `pr_merge_with_gate.rs` ×1,
  `builtin_handlers.rs` ×2). Aucun n'est un site de décision.

### U2 — Le prédicat argv, dans `evidence::guards`

`detect_fallthrough_work_creation(args: &[String]) -> Option<WorkCreationAction>`,
posé à côté de `detect_destructive_action` — même foyer, même forme : pur argv,
sans `ToolContext`, donc testable aux bornes sans base ni tour.

Deux variantes :

- `IssueCreate` — `args[0] == "issue"` et `args[1] == "create"`.
- `ReadyLabelAdd { label }` — `args[0] == "issue"`, `args[1] == "edit"`, et une
  valeur `ready` parmi les étiquettes ajoutées.

**Les trois détails de forme ne sont pas de la minutie** : `gh` accepte
`--add-label ready`, `--add-label=ready`, `--add-label "bug,ready"`, et le
drapeau est répétable. Une des trois formes ratée rend la garde contournable
par une virgule. Le précédent est dans le même fichier : `validate_gh_input`
refuse déjà `--repo` **et** `--repo=value`. La comparaison est
`eq_ignore_ascii_case` — un label `Ready` déclencherait le même webhook, donc
fail-closed.

**Format de fil :** les deux motifs (`issue_create`, `ready_label_add`)
atterrissent dans `audit_events.after_value` et un opérateur en fait des
`GROUP BY`. Un seul site de définition, épinglé par test — deux orthographes
couperaient une population en deux sans le dire (motif mika#2323 / mika#2536).

### U3 — Le refus dans `run_gh`

`validate_fallthrough_work_creation(&gh_args.args, ctx) -> Result<(), ToolOutput>`,
inséré **immédiatement après `validate_qa_review_gh_scope`** et **avant
`validate_pr_ready_undraft_scope`** — le premier maillon qui fait un appel
réseau. La raison est celle que le fichier énonce déjà au site de mika#2455 :
*la chaîne va du plus local au plus engageant*, et cette garde est du pur argv
plus un booléen.

Deux dispositions, dans cet ordre :

1. **Fail-open à la détection.** Un argv qui n'est aucune des deux formes n'est
   pas l'affaire de cette garde ⇒ `Ok(())`. `gh` dans son ensemble est
   intouché, et la surface reste bornée à deux verbes.
2. **Fail-closed après.** Forme reconnue **et**
   `ctx.is_webhook_fallthrough_turn` ⇒ refus. Hors tour Fallthrough, `Ok(())` —
   c'est la garantie de non-régression, et c'est un terme du prédicat, pas une
   branche de l'appelant.

Le corps du refus suit la forme mika#1646 : JSON structuré portant `error`,
`doctrine` (`mika#2517 + mika#2573`), `action`, `reason`, `remedy`. Le `remedy`
nomme les deux sorties correctes que le prompt possède déjà — accuser réception
et s'arrêter, ou alerter l'opérateur par `send_message` si l'événement le mérite
— et **ne nomme aucun contournement** : un refus qui donne le gabarit est une
fuite avec une étape de plus (doctrine mika#2520).

**Pas d'interrupteur d'environnement, et c'est un choix avec son précédent.**
mika#1646, la garde sœur la plus proche (refus pré-subprocess sur argv de
`run_gh`, même fichier, même chaîne), n'en a pas non plus. Un désarmement par
variable sur un chemin de création de travail serait un désarmement par coquille ;
le geste de désarmement est un revert, et le coût d'un faux positif le supporte
(voir § 5).

### U4 — Surface opérateur

- **Journal** (`$MIKA_SPIRIT_LOG_FILE`) : `fallthrough_work_creation_blocked`
  (WARN — champs `agent_id`, `session_id`, `trace_id`, `motif`, `verb`).
- **Base** : une ligne `audit_events`, `tool_name = 'fallthrough_work_creation'`,
  `target_key = 'agent:<id>'`, `after_value = <motif>`,
  `reasoning = "verb=… label=…"`, sous le `trace_id` du tour.

**La jointure est le dessein.** La ligne ne porte pas de `marker_class` : le
booléen ne la transporte pas, et l'inventer serait un champ qui affirme ce qu'on
n'a pas mesuré (classe mika#2304). `webhook_fallthrough_turn` (mika#2517) porte
déjà le `marker_class` **sous le même `trace_id`** — une ligne dit *quel tour*,
l'autre dit *ce qu'il a tenté*, et le `trace_id` les joint exactement.

**SOLE WRITER** de `fallthrough_work_creation` (journal et `audit_events`),
épinglé par un scan de source à allowlist **livrée vide**, portant son
assertion d'anti-vacuité (le scan échoue si le nom n'est écrit nulle part — un
scan qui vise un nom mort se lit exactement comme un scan propre, mika#2103 /
mika#2205). C'est cette propriété qui rend le `GROUP BY` ci-dessous exact plutôt
qu'un nombre sur lequel deux sites peuvent diverger.

### U5 — La moitié prompt (intention, jamais la garantie)

Deux éditions dans `skills/bundled/self-dev/system_prompt.md` :

- § *Webhook Fallthrough*, SCOPE RULE : ajouter que **créer du travail sur la
  forge est refusé** — formulé **par topique** (« aucune nouvelle issue, aucun
  label déclencheur de dispatch »), jamais par gabarit de commande (mika#2292 /
  mika#2520 : énumérer la commande pour l'interdire enseigne la commande).
- § *Rule 9* : une phrase nommant que le moteur refuse, avec les deux numéros de
  ticket — pour qu'un modèle qui rencontre le refus le reconnaisse au lieu de
  chercher une autre route.

**`skills/bundled/` est une projection du binaire, pas du checkout** (mika#2340) :
ces deux lignes n'atteignent aucun agent avant `make deploy` → seed. La sonde S1
le vérifie avant toute conclusion.

### U6 — Tests

**Unitaires** (`evidence::guards::tests::mika2573_*`) :

- les deux argv mesurés, verbatim (`["issue","edit","2562","--add-label","ready"]`
  et `["issue","create","--title","…"]`) ;
- les trois formes syntaxiques (`--add-label ready`, `--add-label=ready`,
  `--add-label bug,ready`) et la répétition du drapeau ;
- la variante de casse `Ready` ;
- **contrôles négatifs** : `issue view`, `issue list`, `issue comment`,
  `issue edit --add-label bug` (un label non déclencheur doit passer),
  `pr edit --add-label ready` (hors population, nommée au § 2).

**Comportemental** — `crates/mika-agent/tests/eval/test_fallthrough_run_gh_refused_2573.rs`,
frère de `test_webhook_fallthrough_no_task_2517.rs` et suivant le raisonnement
que ce fichier écrit déjà : *les tests unitaires du prédicat restent verts si le
champ n'est jamais calculé ou jamais lu, et c'est très exactement le mode de
panne.* Donc un vrai tour par `run_agent`, `MockLlmProvider`, sans réseau :

| cas | message | appel | attendu |
|---|---|---|---|
| V1 | Fallthrough (`[GitHub] New comment on …`) | `issue edit --add-label ready` | **refusé** + ligne d'audit |
| V2 | Fallthrough | `issue create` | **refusé** + ligne d'audit |
| V3 | Fallthrough | `issue view --json labels` | **passe** |
| V4 | ready-label (`[GitHub] Issue labeled ready on …`) | `issue edit --add-label ready` | **passe** |
| V5 | sans préfixe `[GitHub]` | `issue create` | **passe** |

V3 est le contrôle qui sépare « la garde décide » de « la garde bloque
`run_gh` » — et il est la contrepartie exécutable de M2. V4 et V5 sont les deux
axes que le fichier de mika#2517 nomme comme porteurs : sans eux, une garde qui
refuserait *tous* les tours serait indistinguable d'une garde qui lit le
prédicat.

**Structurels :** le scan SOLE WRITER (U4) avec son frère auto-nettoyant
d'allowlist vide, et l'épinglage des deux motifs comme format de fil.

---

## 4. Fire-Disposition

Ce plan livre trois détecteurs : la garde de refus d'exécution (U3), le scan
SOLE WRITER (U4), et l'épinglage du format de fil (U2).

**Option retenue : (a) exception nommée en allowlist — livrée VIDE.**

- `FALLTHROUGH_WORK_CREATION_SOLE_WRITER_EXCEPTIONS: &[&str] = &[]`. L'inventaire
  est **clos à un seul écrivain** au moment de la livraison : le nom
  `fallthrough_work_creation` n'existe nulle part dans l'arbre avant ce ticket,
  donc il n'y a **aucune violation existante** à exempter.
- **Assertion auto-nettoyante :** un test frère,
  `mika2573_the_sole_writer_allowlist_is_empty`, rougit dès que l'allowlist
  cesse d'être vide. Une allowlist née vide est un emplacement où déposer la
  prochaine infraction (mika#2323) ; ce test est ce qui l'en empêche.
- **Anti-vacuité :** le scan échoue si le nom n'est écrit **nulle part** dans
  son fichier propriétaire — sans quoi, le corps réécrit, il viserait un nom
  mort en se lisant comme un arbre propre (mika#2103 / mika#2205).
- **Résolution quand il tire : on retire le second écrivain, on ne l'allowliste
  pas** (doctrine mika#2201). Un second écrivain ne rend aucune décision
  fausse — il rend le `GROUP BY` inexact, ce qu'aucun test comportemental ne
  peut voir, d'où un scan.

**La garde d'exécution (U3) est armée d'emblée**, et ce n'est pas de
l'audace : elle n'a pas de population préexistante à exempter (aucun tour
Fallthrough n'a le droit de créer du travail — c'est le contrat écrit de
l'entrée), et l'asymétrie penche du bon côté (§ 5). Ni `#[ignore]`, ni
`#[cfg(skip)]`, ni halte-et-remontée : les trois options seraient des réponses à
une population existante qui n'existe pas.

---

## 5. L'asymétrie, écrite avant le reste

- **Un faux positif** coûte **un appel `run_gh` refusé** sur un tour dont le
  contrat écrit est *accuser réception et s'arrêter* : le tour peut encore
  accuser réception, alerter l'opérateur, lire l'issue. Visible, borné,
  rattrapable au tour suivant.
- **Un faux négatif** coûte un **dispatch implement que personne n'a autorisé**
  (un créneau, un pilote, un coût en USD) **et** une issue créée par accident
  qu'un humain devra fermer — c'est exactement l'incident du 2026-09-28, où le
  modèle a dû tenter de réparer son propre geste et s'est fait refuser par
  `destructive_action_blocked`.

Le sens du fail-safe suit ce coût : **fail-open à la détection** (ne pas
s'occuper de ce qui n'est pas le sujet), **fail-closed après** (une forme
reconnue sur un tour Fallthrough est refusée). C'est l'inverse du faucheur
mika#2420 (où un signal illisible *conserve*), et l'inversion est raisonnée :
là-bas l'action détruisait du travail, ici l'action *est* la création de travail
non autorisé. **L'arbitrage est local et ne se transporte pas.**

---

## 6. Contrat de vérification

| # | vérification | forme |
|---|---|---|
| V1 | un tour Fallthrough appelant `issue edit --add-label ready` est refusé | eval, production path |
| V2 | un tour Fallthrough appelant `issue create` est refusé | eval |
| V3 | un tour Fallthrough appelant `issue view` **passe** | eval (contrôle négatif) |
| V4 | un tour ready-label appelant `issue edit --add-label ready` **passe** | eval (contrôle négatif) |
| V5 | un tour sans préfixe `[GitHub]` **passe** | eval (contrôle négatif) |
| V6 | les trois formes syntaxiques et la casse sont reconnues | unitaire |
| V7 | `issue edit --add-label bug` **passe** | unitaire (contrôle négatif) |
| V8 | une ligne d'audit est écrite par refus, sous le `trace_id` du tour | eval |
| V9 | `fallthrough_work_creation` a un écrivain unique, allowlist vide | scan de source |
| V10 | les deux motifs ont un site de définition unique | scan de source |
| V11 | `cargo test -p mika-agent` et `cargo clippy` verts | CI |

---

## 7. Definition of Done

1. `ToolContext.is_webhook_fallthrough_turn` existe, est calculé au site de
   conversation par appel à `is_webhook_fallthrough_domain`, et vaut `false`
   aux trois autres sites de production, avec le bord écrit au doc-comment.
2. `detect_fallthrough_work_creation` vit dans `evidence::guards`, reconnaît les
   deux verbes et les trois formes syntaxiques, et est insensible à la casse du
   label.
3. `run_gh` refuse ces deux formes sur un tour Fallthrough, avant tout appel
   réseau, avec un corps JSON nommant les deux tickets et les deux sorties
   correctes.
4. Un refus écrit une ligne WARN et une ligne `audit_events`, joignable par
   `trace_id` à `webhook_fallthrough_turn`.
5. Les deux sections du prompt `self-dev` portent la butée topique.
6. V1–V11 passent ; les cinq contrôles négatifs ont été **vus rouges** en
   neutralisant le terme qu'ils couvrent, un à la fois.
7. Les sondes S1–S4 et leurs haltes sont écrites dans le `CLAUDE.md` racine.

---

## 8. Acceptance criteria

*(Le corps du ticket ne porte pas de section `## Acceptance criteria` ; celles-ci
sont dérivées de son « Correctif attendu », de son « Test » et de sa « Sonde
post-déploiement ».)*

- **AC1 —** Dans un tour Webhook Fallthrough, `run_gh issue edit … --add-label ready`
  est **refusé**, avant tout effet de bord, avec une raison nommant mika#2517 et
  mika#2573.
- **AC2 —** Dans un tour Webhook Fallthrough, `run_gh issue create` est
  **refusé**, dans les mêmes conditions.
- **AC3 —** Dans un tour Webhook Fallthrough, une **lecture** (`issue view`)
  passe inchangée — `run_gh` n'est pas retiré du tour.
- **AC4 —** Un tour qui n'est **pas** Fallthrough (ready-label, PR, check-suite,
  message sans préfixe `[GitHub]`) n'est pas affecté : ni refus, ni ligne
  d'audit.
- **AC5 —** Chaque refus est **comptable** : une ligne de journal et une ligne
  `audit_events` sous un nom dont ce module est l'écrivain unique.
- **AC6 —** Chaque ligne `ready_label_received` portant `actor:
  mika-platform-dev` est attribuable à `auto_pull` (Phase 2, remove → add), et
  jamais à une session conversationnelle dont le tour est un Fallthrough.

---

## 9. Surfaces opérateur

```bash
# 1. La garde a-t-elle mordu, et sur quel motif ?
grep fallthrough_work_creation_blocked "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{agent_id, session_id, trace_id, motif, verb}'

# 2. La sonde du ticket : quel acteur a posé `ready` ?
grep ready_label_received "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.actor == "mika-platform-dev")'

# 3. Le tour correspondant (jointure par trace_id)
grep webhook_fallthrough_turn "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.trace_id == "<trace_id>") | {marker_class, withheld_tools}'
```

```sql
-- La population du refus
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'fallthrough_work_creation' GROUP BY 1 ORDER BY 2 DESC;

-- CONTRÔLE POSITIF — combien de tours auraient PU en produire un
SELECT count(*) FROM audit_events WHERE tool_name = 'webhook_fallthrough_turn';

-- Le résidu `run_shell` : précondition du ticket de suivi (§ 2)
SELECT id, created_at, substr(input, 1, 200) FROM tool_calls
 WHERE tool_name = 'run_shell'
   AND (input LIKE '%--add-label%ready%' OR input LIKE '%issue create%')
 ORDER BY created_at DESC;
```

| surface | régime attendu | lecture |
|---|---|---|
| `fallthrough_work_creation_blocked` | **non vide, faible** | chaque ligne est une création de travail que le moteur a arrêtée |
| `after_value = 'ready_label_add'` | non vide, faible | le vecteur de dispatch mesuré le 2026-09-28 |
| `after_value = 'issue_create'` | non vide, faible | le substitut mesuré à `create_task` |
| `webhook_fallthrough_turn` | non vide | **le contrôle positif** : zéro refus avec zéro tour ne prouve rien (mika#2205) |
| une même session portant plusieurs refus | anomalie | le modèle insiste ; lire le prompt servi avant de toucher au prédicat |

---

## 10. Sondes post-déploiement, et leurs quatre haltes

> **Préalable à toutes.** `skills/bundled/` est une projection du **binaire**, pas
> du checkout (mika#2340). `cat ~/.mika/skills/.manifest-writer` doit porter le
> sha qu'on vient de bâtir, **et** le `mika-spirit` servi doit porter le
> correctif — sans ces deux vérifications, chaque sonde décrit le binaire d'hier.

**S1 — la garde mord (premier tour Fallthrough qui tente).** Une ligne
`fallthrough_work_creation_blocked` et une ligne `audit_events`, et **aucune**
issue créée ni label `ready` posé par ce tour.
*Halte 1 — aucune ligne alors qu'une issue a été créée et que
`webhook_fallthrough_turn` est non vide :* **ne pas élargir la liste de verbes
par réflexe.** Établir d'abord le déploiement (préalable ci-dessus), puis
**quelle porte** a servi — `run_shell` (§ 2, résidu nommé) rend exactement cette
signature, et son remède est un autre ticket.

**S2 — contrôle négatif de la lecture (7 jours).** Aucun refus sur un
`issue view` / `issue list` / `issue comment`.
*Halte 2 — une occurrence :* c'est un faux positif, et il casse le geste de
vérification que le prompt prescrit lui-même. **Revert d'abord, diagnostic
ensuite** — il n'y a pas d'interrupteur, par décision (§ U3), et le coût d'un
revert est inférieur à celui d'une lecture refusée en boucle.

**S3 — la sonde du ticket (30 jours).** Chaque `ready_label_received` portant
`actor: mika-platform-dev` est attribuable à `auto_pull` Phase 2.
*Halte 3 (celle du ticket) — un refus apparaît sur un tour qui n'est PAS
Fallthrough :* c'est le **prédicat de classe du tour** qui est mal lu, pas la
liste de verbes. Lire le message servi (`is_webhook_fallthrough_domain` refuse
`[GitHub] PR `, `[GitHub] Check suite ` et le marqueur ready-label) **avant** de
toucher au prédicat argv.

**S4 — le résidu `run_shell` (30 jours).** La requête SQL n°3 ci-dessus.
*Halte 4 — elle rend des lignes :* le vecteur du § 2 est réel et mesuré ; le
ticket de suivi s'ouvre **avec ce compte**, jamais avec une intuition. Zéro
ligne est un **résultat** — il dit que le modèle n'emprunte pas cette route — et
non une preuve que la route est fermée.

---

## 11. Ce que ce travail n'achète PAS

- **Il ne ferme pas `run_shell`** (§ 2). Un modèle déterminé à créer du travail
  garde une route ; ce qui lui est retiré est celle qu'il a **effectivement**
  prise, déclarée, documentée et keyword-tirée par le vocabulaire même du
  webhook.
- **Il n'empêche pas de demander.** Un tour Fallthrough peut toujours dire à
  l'opérateur « ce commentaire mérite un re-groom » par `send_message` — c'est
  la sortie correcte, et le corps du refus la nomme.
- **Il ne rattrape pas l'incident du 2026-09-28.** L'issue #2571 existe et le
  dispatch `53a10c4a` a eu lieu ; **rien ici ne rétro-estampille** — fabriquer
  une ligne décrivant un fait qu'on n'a pas observé est l'inverse de ce que ce
  travail défend. La sonde est la **prochaine** occurrence.
- **Il ne surveille rien.** Les seuls instruments sont les greps et les requêtes
  du § 9, et **leur silence ne prouve rien tant que personne ne les exécute** —
  d'où le contrôle positif sur `webhook_fallthrough_turn`, sans lequel zéro
  refus et zéro tour rendent les mêmes octets.

---

## 12. Hors périmètre, délibérément

- **Le routage `issue_comment` → mika-dev** et le cas des PR commentées par
  l'orchestrateur : le ticket les exclut nommément.
- **`run_shell`** (§ 2) — ticket de suivi, précondition S4.
- **`gh pr edit --add-label ready`** et `gh issue reopen` (§ 2) — hors
  population, et armer une garde sur une population vide produirait un détecteur
  dont le silence ne prouve rien.
- **Élargir `FALLTHROUGH_WITHHELD_TOOLS`** — refusé avec sa raison (M2) : la
  retenue casserait le geste de vérification que le prompt prescrit.
- **Un interrupteur d'environnement** — refusé avec son précédent (U3,
  mika#1646).
- **La classification `marker_class` sur la ligne de refus** — refusée : le
  booléen ne la transporte pas, la jointure par `trace_id` la donne, et
  l'inventer serait un champ qui affirme ce qu'on n'a pas mesuré.
