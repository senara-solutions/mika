# mika#2545 — Un `ESCALATE` de groom est terminal : le fait est estampillé, et le re-dispatch automatique le lit

> Ticket : `senara-solutions/mika#2545` — p1, loop-substrate.
> Branche : `fix/2545/loop-substrate-un-groom-qui-escalate-est`

---

## 1. Le défaut, mesuré

Groom de mika#2542, le 2026-09-26. Verdict architecte `ESCALATE` sur review-anchor.
Un mécanisme a **auto-rejoué le groom 8 fois** — callbacks `mika-dev` en paires
`expired`/`delivered` toutes les ~1 min, de **13:06:14Z à 13:20:54Z**, chacun
ESCALATE sur la même cause. Quinze tâches mesurées (`3c807c02`, `3c450285`,
`c7331e5c`, `6b3369b3`, `ad326a9b`, `ee9881f6`, `6e8aad21`, `3054bfc1`,
`7a08409c`, `27f3667f`, `1a43030c`, `00636d89`, `7e3e6924`, `8b41b611`,
`82f30379`).

Le fichier `~/.mika/state/auto-pull-stop` était posé, donc **ce n'est pas le
feeder** : la sentinelle mika#2329 court-circuite le tick d'`auto_pull` et,
depuis mika#2498, l'auto-fire post-groom. Le ticket laissait trois pistes
ouvertes — re-arm de `self-dev-callback`, deferred-dispatch, ou un chemin
ESCALATE→retry du pipeline.

---

## 2. Ce que la lecture du code établit — les trois pistes tranchées

**La chaîne est complète et lisible sans rejouer l'incident.** Les trois pistes
du ticket sont une seule chaîne où chacune tient un maillon, et aucune n'est la
cause à elle seule.

| # | site | ce qui se passe |
|---|---|---|
| 1 | `_escalate_groom` (`dispatch-lib.sh:6795`) | ESCALATE ⇒ `RESULT` reçoit `PIPELINE FAILURE: groom escalated by mika-arch <stage>.` |
| 2 | `_iterate_groom_loop` (l. 7215, 7268, 7275) | les **trois** bras ESCALATE rendent `return 1` |
| 3 | `dispatch_claude_pilot`, branche `else` (l. 8774-8783) | une **seconde** ligne `PIPELINE FAILURE: grooming did not converge`, et `Outcome: PIPELINE_INCOMPLETE — <reason>` |
| 4 | `self-dev-callback`, § *On pipeline failure* | le marqueur `PIPELINE FAILURE:` fait entrer le tour dans la population **retryable** |
| 5 | idem, étape 4 | `run_claude_pilot_groom` est ré-appelé avec le même `repo#number` (mika#1823 : le même outil que la classe du callback échoué) |
| 6 | `validate_dispatch_readiness` | le slot `groom` est occupé ⇒ `{"status": "deferred"}` ⇒ wrapper différé enregistré |
| 7 | tick moteur (60 s) | promotion du wrapper ⇒ **les paires `expired`/`delivered` à ~1 min** — la cadence exacte mesurée |
| 8 | → retour en 1 | |

**Le maillon décisif est le 1** : un ESCALATE, verdict terminal de grooming par
contrat, **porte le marqueur de la population retryable**. Le maillon 7 explique
la cadence, le maillon 4 explique le rejeu ; le maillon 1 explique pourquoi un
verdict terminal y entre.

### 2.1 Pourquoi le budget de 2 ne borne rien — classe mika#2158

Le prompt du callback pose un budget (`pipeline_retry_count >= 2` ⇒ escalade).
Il n'a **jamais** pu borner cette boucle, pour trois raisons cumulatives :

1. **Aucun code moteur ne le lit ni ne le décide.** La seule occurrence Rust en
   production est `has_retry_semantic_keys`
   (`tools/update_task_status.rs:278`), qui alimente la garde phantom #579 — elle
   **empêche** l'écriture du compteur pendant qu'un dispatch tourne, sans jamais
   l'appliquer. C'est une prescription de prompt, et
   `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` en dit
   la valeur au substrat de la boucle.
2. **L'écriture prescrite échoue.** Le framing de callback
   (`agent_loop/mod.rs::format_callback_framing`) présente comme `task_id` la
   tâche **callback**, dont `trigger_type = 'callback'` ; or
   `update_task_status` refuse toute tâche dont `trigger_type != 'manual'`
   (l. 152-162). Le compteur n'atterrit pas là où le prompt le range.
3. **Chaque rejeu naît avec une metadata vierge.** Le retry crée une nouvelle
   tâche callback ; `check_task` sur elle relit `pipeline_retry_count` absent,
   donc `0`.

> *Un compteur remis à zéro par l'action qu'il compte ne borne rien* — la phrase
> que mika#2158 a dû écrire après **31 re-drives** affichés comme `1`. Le budget
> n'était pas trop large : il était **inatteignable**.

### 2.2 Le fait n'est lisible nulle part, et un prédicat le cherche déjà

Deux constats que la lecture rend, et qui décident du remède.

**(a) `Outcome: PIPELINE_INCOMPLETE` est ce qu'un ESCALATE produit** — la même
valeur qu'un pilote mort, qu'un push manquant, qu'une session tronquée. Le
verdict terminal de l'architecte et l'échec mécanique du pipeline rendent des
octets identiques, donc aucun lecteur en aval ne peut les séparer.

**(b) `Outcome: ESCALATE` est cherché et n'est écrit nulle part.** Le prédicat P4
de `_measure_cycle_output` (`dispatch-lib.sh:3641`) l'énumère :

```sh
grep -m1 -E '^Outcome: (PR_OPENED|PLAN_COMMITTED|PLAN_GROOMED|ESCALATE)'
```

Une recherche exhaustive des sites d'écriture de `^Outcome: ` dans
`dispatch-lib.sh` rend `PR_OPENED`, `PLAN_COMMITTED`, `PLAN_GROOMED`,
`PIPELINE_INCOMPLETE`, `UNKNOWN` — **jamais `ESCALATE`**. Le terme est un
prédicat sur une population vide, classe mika#2205 : *un détecteur silencieusement
inerte se lit exactement comme un détecteur sain.* Le commentaire au-dessus de P4
décrit d'ailleurs précisément la population absente — « *un grooming qui escalade
avec une raison* ».

Ce correctif rend ce terme non vide, et c'est ce qui rend le remède petit : la
maison a déjà décidé que `Outcome: ESCALATE` est le nom du fait ; il ne restait
qu'à l'écrire.

---

## 3. Le remède — deux moitiés, et la structurelle est celle qui tient

Le ticket demande : *« identifier le mécanisme de rejeu et le rendre inerte sur
un ESCALATE »*. Le remède suit le motif maison en deux moitiés — **le prompt
exprime l'intention, le refus la tient** (mika#2536) — plus un producteur qui
estampille son propre fait (mika#2026, mika#2242).

### R1 — Le producteur estampille (dispatch-lib.sh)

`_escalate_groom` cesse d'écrire le marqueur de la population retryable et pose
le sien :

- il **n'écrit plus** `PIPELINE FAILURE:` ; il écrit `GROOM ESCALATED (terminal):`
  suivi du stage, du verdict, de la session et du chemin des findings — même
  substance, autre classe ;
- il pose `Outcome: ESCALATE — <stage>` via `_set_outcome_line` (mika#2492), qui
  garantit **par construction** l'unicité de la ligne `Outcome:` ;
- la branche `else` de `dispatch_claude_pilot` (l. 8774-8783) **n'écrase pas** un
  `Outcome: ESCALATE` déjà posé, et n'ajoute pas sa propre ligne
  `PIPELINE FAILURE:` dans ce cas. Son texte — « *grooming did not converge* » —
  reste juste pour ses dix-sept autres sorties ; il est faux pour un ESCALATE,
  qui **a** convergé, sur un verdict de halte.

**Non-régression à tenir, et c'est la part fragile de R1 :** `PIPELINE FAILURE:`
est aussi ce qui sort le cycle de la population `empty_completion`
(`_gate_non_empty_cycle`, l. 3730). Le retirer sans compensation exposerait un
ESCALATE à être re-classé « cycle vide » — un faux rouge sur une sortie
délibérée, exactement ce que ce gate dit vouloir éviter. Deux termes le
couvrent, et il faut **les deux** :

- ajouter `^Outcome: ESCALATE` à l'alternance de reclassement de la l. 3730,
  aux côtés de `^Outcome: PIPELINE_INCOMPLETE` — c'est une disposition terminale
  délibérée, de la même famille ;
- P4 le reconnaît déjà comme disposition motivée, **sous réserve** de
  `CYCLE_TOOL_CALLS >= 1`. Cette réserve est précisément pourquoi le premier
  terme n'est pas redondant : un ESCALATE dont le compte d'appels d'outils n'est
  pas mesuré tomberait sinon en `empty`.

### R2 — Le lecteur en base, calqué sur son jumeau

`Database::latest_groom_verdict_for_issue`, sœur exacte de
`has_completed_groom_for_issue` (`db/tasks.rs:4177`) — même jointure, même
scoping agent, même tolérance à l'URL legacy `?phase=groom` :

```sql
SELECT child.result FROM tasks child
  JOIN tasks parent ON child.parent_task_id = parent.id
 WHERE child.agent_id = ?1
   AND child.trigger_type = 'callback'
   AND child.dispatch_class = 'groom'
   AND child.status IN ('completed', 'delivered')
   AND parent.reference_url IN (?2, ?3)
 ORDER BY child.created_at DESC, child.id DESC
 LIMIT 1
```

**Le prédicat porte sur le DERNIER groom, jamais sur l'existence d'un ESCALATE.**
Un ticket escaladé puis re-groomé avec succès doit redevenir dispatchable ; un
`EXISTS` le bloquerait pour toujours et transformerait un frein en mur. Le tri
est `created_at DESC, id DESC` — la naissance du dispatch, pas `updated_at`,
qu'un faucheur ou un reaper peut toucher tardivement ; et le `id DESC` rend
l'ordre déterministe pour deux callbacks nés dans la même seconde.

Le verdict est dérivé par une **fonction pure** `groom_escalate_verdict(result:
Option<String>) -> GroomVerdictState`, énumération à trois états
(`Escalated` / `NotEscalated` / `Unreadable`), consommée par un `match`
exhaustif **sans bras `_ =>`** — motif `GroomedState` (mika#2484 D1) : un
quatrième état devra être décidé par le compilateur.

### R3 — La garde, dans `validate_dispatch_readiness`

Un quatrième terme dans la porte que **tous** les `run_claude_pilot*`
traversent (`skills/executor.rs:2007`, appelée par `execute_long_running`,
`ready_label_handler` et l'auto-fire moteur). Trois termes conjonctifs :

1. `derive_dispatch_class(extract_skill_from_input(tool_input)) == "groom"` —
   un `dev-pilot` sur un ticket escaladé est déjà refusé par la porte de preuve
   (aucun `Outcome: PLAN_GROOMED`), et l'élargir ici dupliquerait un refus
   existant sous un second nom ;
2. `originating_message.is_none()` — **le discriminant de ré-armement**, voir
   §4 ;
3. le dernier groom de ce ticket est `Escalated`.

Le ticket est résolu par `parse_issue_ref_from_dispatch_prompt`
(`webhook_dispatch.rs:315`), déjà employé deux fois dans cette fonction. Un
prompt sans référence lisible **sort de la population** : le terme n'est pas
satisfait, la garde ne mord pas.

**Placement :** après la récupération de la tâche et le contrôle de
double-dispatch, **avant** les gardes de slot — un refus placé après
l'enregistrement d'un wrapper différé serait ré-armé et re-refusé à chaque
replay, brûlant un wrapper par tour sur un ticket qui ne peut pas partir. C'est
mot pour mot le raisonnement que la porte de siège (mika#2084) a déjà dû écrire
quelques lignes plus haut.

**Fail-closed sur l'illisible**, et c'est le voisin de palier qui décide :
`groom_provenance_verdict` l'est déjà sur la même porte, avec sa raison écrite.
Le coût est nommé — sur une erreur base, un re-groom automatique légitime est
refusé et le ticket attend ; c'est **convergent** (`stuck_ready_reconcile` le
re-drive, budget mika#2020) là où un passage à tort **est** la boucle de huit
dispatches.

### R4 — Le prompt (moitié intention, `self-dev-callback`)

Un discriminateur ESCALATE **avant** la classification pipeline, dans la même
famille que les discriminateurs cancel (mika#749) et containment-refusal
(mika#2049) qui le précèdent :

> **Prédicat :** `RESULT` **contient** `Outcome: ESCALATE`.
> Ne pas rejouer. Marquer `blocked`. Poser le verdict et le chemin des findings
> dans le message à l'opérateur. Aucun label touché.

Cette moitié ne tient pas seule — c'est la raison d'être de R3 — mais son absence
laisserait le modèle appeler un outil que le moteur refuse, ce qui coûte un tour
et une ligne de refus par occurrence.

---

## 4. Le discriminant de ré-armement : `originating_message`, et rien d'autre

Un ESCALATE doit halter le **rejeu automatique**, jamais la reprise par
l'opérateur. La maison a déjà le discriminant, et sa justification est écrite
dans le code même de cette porte (mika#2484, l. 2057) :

> *« le seul cas légitime — la chaîne dev-groom → dev-pilot — ne passe pas par ce
> chemin (son `originating_message` est absent, c'est un tour de callback) »*

| chemin | `originating_message` | verdict |
|---|---|---|
| retry pipeline de `self-dev-callback` | **absent** (tour silencieux) | **refusé** |
| auto-fire post-groom (`dispatcher.rs:4285`) | `None`, explicitement | refusé — mais dispatche `dev-pilot`, hors population |
| `ready_label_handler` (l. 1508) | le marqueur webhook | **passe** |
| `mika ask --agent mika-dev "groom mika issue#N"` | le message | **passe** |

**Le ré-armement est donc gratuit et sans état** : aucun stamp à écrire, aucune
fenêtre à régler, aucune ligne à nettoyer. Les deux gestes d'opérateur canoniques
— reposer `ready` en **remove → add** (mika#2323 : GitHub n'émet `labeled` que
sur une transition) ou un `mika ask "groom …"` — traversent la garde par
construction.

**Ce que ça coûte, nommé :** le feeder peut encore reposer `ready` sur un ticket
escaladé et relancer un groom. Ce n'est pas la boucle mesurée (la sentinelle STOP
était posée) et c'est borné par le budget de re-drive à trois tours (mika#2020),
mais c'est trois grooms gaspillés. **Élargir la garde au `ready_label_handler`
est refusé** : ce serait retirer à l'opérateur le geste de reprise le plus court
qu'il possède. Le remède juste pour cette population est que le ticket porte
`operator-review`, ce qui le sort **structurellement** des trois phases du feeder
(`is_feeder_excluded`) — voir §10.

---

## 5. Unités d'implémentation

| U | fichier | contenu |
|---|---|---|
| U1 | `skills/bundled/_shared/dispatch-lib.sh` | R1 : `_escalate_groom` pose `Outcome: ESCALATE` via `_set_outcome_line`, cesse d'écrire `PIPELINE FAILURE:` ; `^Outcome: ESCALATE` rejoint l'alternance de reclassement l. 3730 ; la branche `else` (l. 8774) préserve un `Outcome: ESCALATE` existant |
| U2 | `crates/mika-agent/src/db/tasks.rs` + `async_db.rs` | R2 : `latest_groom_verdict_for_issue` + son enveloppe async |
| U3 | `crates/mika-agent/src/skills/executor.rs` | R2/R3 : `GroomVerdictState`, `groom_escalate_verdict` (pure), le quatrième terme de la porte, le JSON de refus |
| U4 | `skills/bundled/self-dev-callback/system_prompt.md` | R4 : le discriminateur ESCALATE |
| U5 | `skills/bundled/_shared/test-dispatch-lib.sh` | gardes shell (§6) |
| U6 | `crates/mika-agent/src/skills/executor.rs` (tests) + `tests/eval/` | gardes Rust (§6) |
| U7 | `mika/CLAUDE.md` | la section opérateur (§7) |

**Aucune migration, aucune colonne, aucune variable d'environnement.** Le fait
vit dans `tasks.result`, que les deux lecteurs de preuve existants interrogent
déjà de la même façon.

---

## 6. Fire-Disposition

Ce plan livre des détecteurs : la garde de dispatch R3, un scan de source
sole-writer, et les assertions de U5/U6. Disposition retenue :

**(a) exception nommée en allowlist — armé, allowlist livrée VIDE.**

- **`groom_escalate_redispatch_refused` est SOLE WRITER de son nom**, dans le
  journal et dans `audit_events`. Tenu par un scan de source à allowlist vide,
  sur le modèle de `mika2496_the_cost_overrun_name_has_a_single_writer` et de
  `mika2242_the_two_audit_names_have_a_single_writer`. Le nom est neuf : la
  population d'exceptions est **vide à la livraison**, et un test frère pin
  qu'elle le reste. **Quand le scan tire, on retire le second écrivain ; on
  n'ajoute pas de ligne** (doctrine mika#2201).
- **Anti-vacuité obligatoire, dans les deux sens.** Le scan échoue si le nom
  n'est écrit **nulle part** — un scan visant un nom mort ne vérifie rien et se
  lit exactement comme un arbre propre. Idem pour le scan shell : il assert la
  **cardinalité** des sites d'écriture de `^Outcome: ESCALATE` (exactement 1).
- **La garde R3 est armée d'emblée, et sa population est vide au déploiement.**
  Un ticket escaladé *avant* ce correctif porte `Outcome: PIPELINE_INCOMPLETE`,
  jamais `Outcome: ESCALATE` : **aucun rétro-effet**, la garde ne mord que sur
  les grooms postérieurs au déploiement. C'est ce qui rend l'armement immédiat
  sûr — et c'est aussi ce qui rend les sondes de §8 muettes tant qu'aucun
  ESCALATE réel n'a eu lieu.

---

## 7. Surfaces opérateur

```bash
# 1. Un re-dispatch a-t-il été refusé ? (régime attendu : NON VIDE et FAIBLE)
grep groom_escalate_redispatch_refused "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{repo, issue, task_id, escalated_callback_id}'

# 2. CONTRÔLE POSITIF — le producteur estampille-t-il seulement ?
grep -h '^Outcome: ESCALATE' "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | wc -l
```

```sql
-- Les grooms qui ont escaladé (le producteur)
SELECT parent.reference_url, child.created_at
  FROM tasks child JOIN tasks parent ON child.parent_task_id = parent.id
 WHERE child.dispatch_class = 'groom'
   AND instr(child.result, 'Outcome: ESCALATE') > 0
 ORDER BY child.created_at DESC;

-- Les rejeux interceptés (le lecteur) — DOIT être très inférieur au producteur
SELECT target_key, count(*) FROM audit_events
 WHERE tool_name = 'groom_escalate_redispatch_refused'
 GROUP BY 1 ORDER BY 2 DESC;
```

| événement | niveau | régime attendu | lecture |
|---|---|---|---|
| `groom_escalate_redispatch_refused` | WARN | **non vide, faible** | chaque ligne est un dispatch de groom que l'opérateur n'a pas eu à annuler |
| `Outcome: ESCALATE` dans `tasks.result` | — | non vide, faible | la population du producteur ; **doit dominer** celle du lecteur |
| une clé au-dessus de **1** dans la requête 2 | — | **anomalie** | le refus ne tient pas : un second appelant rejoue sous un chemin que la garde ne traverse pas |

**Les deux populations sont soustractibles** parce que chaque nom a un seul
écrivain — motif `phantom_aged_out` / `phantom_sweep_spared` (mika#2156),
`closing_pr_closed_unmerged` / `ready_label_degroomed` (mika#2242).

---

## 8. Sondes post-déploiement, et leurs quatre haltes

> **Préalable, non négociable.** `skills/bundled/` est une projection du
> **binaire**, pas du checkout (mika#2340). `cat ~/.mika/skills/.manifest-writer`
> doit porter le sha qu'on vient de bâtir — sans quoi chacune des sondes décrit
> le binaire d'hier.

**S1 — le producteur estampille (premier ESCALATE réel).** `tasks.result` du
callback de groom porte exactement une ligne `Outcome: ESCALATE`, et **aucune**
ligne `PIPELINE FAILURE:`.
*Halte 1 — les deux sont présentes, ou `Outcome: PIPELINE_INCOMPLETE` a survécu :*
la branche `else` de `dispatch_claude_pilot` écrase encore. **Ne pas toucher à la
garde** — c'est U1 qui n'a pas pris, et la garde ne peut rien lire qui n'ait été
écrit.

**S2 — la garde mord (premier ESCALATE, tour suivant).** Une ligne
`groom_escalate_redispatch_refused`, et **zéro** nouvelle tâche callback
`dispatch_class='groom'` sous ce parent.
*Halte 2 — un rejeu part quand même, la ligne absente :* un appelant ne traverse
pas la porte. **Ne pas élargir le prédicat par réflexe** : établir lequel des
quatre chemins du tableau §4 a servi — les remèdes diffèrent.

**S3 — contrôle négatif du ré-armement (geste opérateur).** Sur le ticket
escaladé, `gh issue edit <n> --remove-label ready` puis `--add-label ready` : le
groom **doit** repartir.
*Halte 3 — il ne repart pas :* la garde mord sur le chemin opérateur, c'est-à-dire
que le terme `originating_message.is_none()` est mal lu. **Désarmer d'abord**
(revert de U3), diagnostiquer ensuite — une garde qui bloque la reprise est pire
que la boucle qu'elle remplace, puisqu'elle n'a pas de contournement.

**S4 — contrôle négatif de bruit (7 jours).** Aucun
`groom_escalate_redispatch_refused` sur un ticket dont le dernier groom a rendu
`Outcome: PLAN_GROOMED`.
*Halte 4 — une occurrence :* le prédicat lit l'**existence** d'un ESCALATE et non
le **dernier** verdict — le `ORDER BY … LIMIT 1` ne fait pas son travail, et un
ticket sain est gelé.

**Halte transverse — les deux sondes muettes.** Zéro ligne des deux côtés ne
prouve **rien** : il faut qu'un ESCALATE réel ait eu lieu depuis le déploiement.
Vérifier la requête *producteur* avant toute conclusion. *Une garde que personne
n'a exercée se lit exactement comme une garde qui marche* (mika#2205).

---

## 9. Vérification

**Rust (`cargo test -p mika-agent`) :**

- `groom_escalate_verdict` — les trois états, dont `Unreadable` sur un `result`
  absent et sur une erreur base, et le `match` exhaustif sans bras joker.
- `latest_groom_verdict_for_issue` — un ESCALATE seul rend `Escalated` ; un
  ESCALATE **suivi** d'un `PLAN_GROOMED` rend `NotEscalated` (le test porteur :
  il distingue « le dernier » de « il en existe un ») ; l'URL legacy
  `?phase=groom` est reconnue ; deux callbacks nés la même seconde rendent un
  ordre déterministe.
- La porte — refus sur la conjonction complète ; **contrôles négatifs** : passe
  avec un `originating_message` peuplé, passe pour `dev-pilot`, passe sur un
  prompt sans référence lisible, refuse sur `Unreadable` (fail-closed).
- Placement — le refus survient **avant** tout enregistrement de wrapper différé
  (assertion sur l'absence de row `deferred` après un refus).
- Scan de source — sole writer, allowlist vide, allowlist pinnée vide,
  anti-vacuité.

**Shell (`skills/bundled/_shared/test-dispatch-lib.sh`) :**

- `_escalate_groom` sur les **trois** stages : `Outcome: ESCALATE` présent
  exactement une fois, `PIPELINE FAILURE:` absent, le stage et la session
  présents.
- Cardinalité : exactement **un** site d'écriture de `^Outcome: ESCALATE`.
- Le gate : un RESULT d'ESCALATE n'est **pas** reclassé `empty_completion`,
  y compris avec `CYCLE_TOOL_CALLS` non mesuré — c'est le contrôle négatif du
  terme ajouté l. 3730, et il doit être **vu rouge** sans lui.
- Non-régression : un `PIPELINE FAILURE:` authentique (pilote mort, plan absent)
  garde son marqueur et son `Outcome: PIPELINE_INCOMPLETE` — la population
  retryable légitime est intacte.
- Le chemin GROOMED est inchangé à l'octet près.

**Ce qui n'est PAS testable ici, écrit plutôt que découvert :** que le modèle
suive le discriminateur R4. C'est un comportement LLM ; le contrat côté moteur
est *le refus tient quoi que le modèle décide*, et c'est R3 qui l'atteste.

---

## 10. Ce que ce travail n'achète PAS

- **Il ne fait pas converger un groom qui escalade.** Il rend l'escalade
  terminale et lisible ; le plan reste à réviser par un humain, ce qui est le
  contrat de sortie de `/mika-groom-ticket`.
- **Il ne rattrape pas l'incident du 2026-09-26.** Les quinze tâches portent
  `Outcome: PIPELINE_INCOMPLETE` et **rien ici ne rétro-estampille** : fabriquer
  une ligne décrivant un fait qu'on n'a pas observé est l'inverse de ce que ce
  travail défend. La sonde est la **prochaine** occurrence.
- **Il ne borne pas le feeder.** Un `ready` reposé par `auto_pull` Phase 2
  traverse la garde ; c'est le prix du ré-armement sans état (§4), borné à trois
  tours par mika#2020.
- **Il n'ajoute aucun compteur du producteur.** La population des grooms qui
  escaladent se lit par la requête SQL de §7, et **son silence ne prouve rien
  tant que personne ne l'exécute**.

---

## 11. Definition of Done

- [ ] U1 — `_escalate_groom` pose `Outcome: ESCALATE` et n'écrit plus
      `PIPELINE FAILURE:` ; la l. 3730 et la branche `else` l. 8774 sont
      ajustées ; le chemin GROOMED est inchangé.
- [ ] U2 — `latest_groom_verdict_for_issue` + enveloppe async, calqués sur
      `has_completed_groom_for_issue`.
- [ ] U3 — `GroomVerdictState` (trois états, `match` exhaustif sans joker),
      `groom_escalate_verdict` pure, le quatrième terme de la porte placé avant
      les gardes de slot, JSON de refus nommant le verdict, le callback
      escaladé, et **les deux gestes de reprise**.
- [ ] U4 — le discriminateur ESCALATE dans `self-dev-callback`, avant la
      classification pipeline.
- [ ] U5/U6 — toutes les assertions de §9, dont les contrôles négatifs **vus
      rouges** avant d'être verts, et les scans de §6 avec leur anti-vacuité.
- [ ] U7 — la section opérateur de §7 dans `mika/CLAUDE.md`.
- [ ] `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`,
      `make verify-bundled-skills` verts.
- [ ] Aucune migration, aucune variable d'environnement, aucun réglage déplacé.

---

## Acceptance criteria

1. **Un ESCALATE de groom est terminal.** Après un verdict `ESCALATE`
   (première ou seconde passe), aucun re-dispatch automatique de groom n'est
   émis pour ce ticket : la garde le refuse à la porte, avant toute création de
   tâche, de wrapper différé ou de worktree.
2. **Le mécanisme de rejeu est identifié et nommé dans le plan** : le marqueur
   `PIPELINE FAILURE:` écrit par `_escalate_groom` fait entrer un verdict
   terminal dans la population retryable de `self-dev-callback`, dont le budget
   `pipeline_retry_count` ne borne rien (classe mika#2158).
3. **Le fait est estampillé par son producteur** : le callback d'un groom
   escaladé porte exactement une ligne `Outcome: ESCALATE`, et aucune ligne
   `PIPELINE FAILURE:`.
4. **L'escalade est surfacée à l'opérateur** : une ligne WARN
   `groom_escalate_redispatch_refused` et une ligne `audit_events` du même nom,
   chacune à écrivain unique, nommant le ticket, le callback escaladé et les
   gestes de reprise.
5. **La reprise par l'opérateur reste possible sans nettoyage d'état** : reposer
   `ready` (remove → add) ou `mika ask --agent mika-dev "groom mika issue#N"`
   relance le groom.
6. **Aucune régression sur la population retryable légitime** : un
   `PIPELINE FAILURE:` authentique (pilote mort, plan absent, push manquant)
   garde son marqueur, son `Outcome: PIPELINE_INCOMPLETE` et son retry.
7. **Un ESCALATE n'est pas re-classé cycle vide** par `_gate_non_empty_cycle`,
   y compris quand le compte d'appels d'outils n'est pas mesuré.
8. **Un ticket escaladé puis re-groomé avec succès redevient dispatchable** —
   le prédicat porte sur le dernier verdict de groom, jamais sur l'existence
   d'un ESCALATE.

---

## 12. Hors périmètre, délibérément

- **Poser `operator-review` sur un ticket escaladé.** C'est le remède juste pour
  la population « le feeder repose `ready` sur un ticket escaladé » (§4), et il
  est refusé ici pour deux raisons : le geste vit dans un prompt, donc il ne
  tient pas (`feedback_prompt_enforcement_…`) ; et le porter côté moteur
  demanderait un écrivain de label dans une porte qui n'en a aucun.
  **Ticket de suivi**, précondition : que la requête SQL du producteur montre
  des tickets escaladés re-groomés par le feeder.
- **Faire de `pipeline_retry_count` un compteur moteur.** Le défaut mesuré est un
  verdict terminal dans la mauvaise population, pas un budget mal réglé — et
  borner un budget que personne n'applique le laisserait inatteignable pour les
  autres membres de sa population. **Ticket de suivi**, précondition : une mesure
  montrant un `PIPELINE FAILURE:` **authentique** rejoué plus de deux fois.
- **La cause du verdict ESCALATE de mika#2542** (review-anchor, mika#2338) :
  autre ticket, autre population.
- **Les dix-sept autres sorties de `_iterate_groom_loop`** : leur classification
  est inchangée.
- **Le rétro-estampillage** des quinze tâches de l'incident.
