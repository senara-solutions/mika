---
title: La preuve de grooming se lit en position de verdict, jamais en sous-chaîne - Plan
type: fix
date: 2026-09-29
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# La preuve de grooming se lit en position de verdict, jamais en sous-chaîne - Plan

**Ticket :** senara-solutions/mika#2590 · **Branche :** `fix/2590/loop-substrate-la-note-d-un-auto-skip`

## Goal Capsule

- **Objective :** la prose d'un refus ne peut plus valoir preuve de convergence. Un
  callback de groom ne prouve le grooming que s'il porte le marqueur **en position de
  verdict** et n'est pas une enveloppe de saut.
- **Means :** un **lecteur unique**, fonction pure ancrée sur la ligne, dans le fichier
  qui porte déjà `GROOM_SUCCESS_MARKER` ; les deux consommateurs l'appellent ; le
  producteur cesse d'écrire le marqueur hors position (KTD3) ; la note du refus cesse
  de citer le jeton.
- **Autorité :** le corps de mika#2590 fixe le comportement attendu (ses cinq points).
  Ce plan le rectifie sur cinq points mesurés (KTD1–KTD5). En cas de conflit, les R-IDs
  l'emportent sur le comportement attendu, les KTD sur le mécanisme.
- **Conditions d'arrêt :** (a) si la sonde préalable S0 montre qu'une **part
  significative** du backlog `ready` ne tient son état `Groomed` que par un
  `auto_skipped`, s'arrêter et remonter — le correctif est juste, mais son déploiement
  renverrait ce backlog dans le cul-de-sac convergent de mika#2484 D4 en un seul tick,
  et c'est un arbitrage d'opérateur, pas d'implémenteur ; (b) si fermer le faux négatif
  de KTD3 demandait de changer la **position** de la ligne `Outcome:` dans le RESULT
  d'une façon qu'un lecteur shell existant ne tolère pas, s'arrêter.
- **Profil d'exécution :** Rust (`task_state/tasks.rs`, `db/tasks.rs`, `async_db.rs`,
  `task_engine/dispatcher.rs`, `skills/executor.rs`), shell
  (`skills/bundled/_shared/dispatch-lib.sh` + sa suite), un TSV. **Aucune migration,
  aucune variable d'environnement, aucune valeur de réglage déplacée.**
- **Qui termine :** le pipeline `/mika` jusqu'à la PR. Les sondes S0 et S2 sont des
  **gestes d'opérateur** sur `~/.mika/data/mika.db` : le bac à sable de dispatch ne
  monte pas la base, donc l'implémenteur ne peut pas les produire.

---

## Product Contract

### Summary

`has_completed_groom_for_issue` prouve le grooming par `instr(child.result, 'Outcome:
PLAN_GROOMED') > 0`. La note JSON que `dispatch-lib.sh` écrit sur un saut
`already_groomed` **cite le marqueur en toutes lettres** pour expliquer qu'aucune preuve
n'est frappée. Le texte qui dit « ceci n'est pas une preuve » **est** la preuve. Le
correctif remplace la sous-chaîne par une lecture **structurelle** — enveloppe JSON
refusée, marqueur exigé en début de ligne — derrière un lecteur unique, et fait cesser
le producteur d'écrire le marqueur en position non canonique.

### Problem Frame

Mesuré le 2026-09-29 sur mika#2105 (`audit_events`, `tasks`) :

| heure (Z) | fait |
|---|---|
| 14:01:08 | `ready_label_handled … target_skill=dev-groom groomed=false` (`5def8a38`) |
| 14:05:09 | callback groom `89165fb4` (`callback`/`groom`/`delivered`), `result` = le JSON `{"status":"auto_skipped","reason":"already_groomed",…}` |
| 15:01:08 | `ready_label_handled … target_skill=dev-pilot groomed=true` (`81c60c68`) |
| 15:01:12 | pilote **implement** `e1a6c78b`, sans plan re-mesuré (mort 15:13Z, rescue #2589) |

`select instr(result,'Outcome: PLAN_GROOMED') from tasks where id like '89165fb4%'` rend
**651** : l'offset de la note. Entre 14:01 et 15:01, `89165fb4` est la seule nouveauté ;
`groomed_state` rend `Groomed` et `route_for` rend `implement`.

La classe est celle que mika#2050 a mesurée sur le Signal S (« un pilote qui *discute* du
jeton se lit comme une émission ») et celle que mika#2545 a fermée un marqueur plus loin
— `groom_escalate_verdict` est déjà ancré, avec ce raisonnement écrit mot pour mot.

### Ce que la lecture du code déplace dans le ticket

**KTD1 — le point 5 est un balayage, pas un travail, et sa réponse est « déjà fait ».**
`latest_groom_verdict_for_issue` ne lit aucun marqueur : elle rend le `result` brut du
**dernier** callback groom terminal. Son consommateur `skills::executor::groom_escalate_verdict`
est **déjà ancré** (`text.lines().any(|line| line.starts_with(GROOM_ESCALATE_MARKER))`),
livré par mika#2545 avec le motif écrit. Aucune ligne n'y change. Le balayage rend donc
*un lecteur strict, un lecteur lâche* — et c'est le lâche que ce ticket ferme.

**KTD2 — le point 5 a raison sur l'autre lecteur.** `try_dispatch_pilot_after_groom_success`
(`task_engine/dispatcher.rs:4089`) fait `r.contains(GROOM_SUCCESS_MARKER)` : même défaut,
même remède. Il n'était pas atteint par l'incident (l'auto-fire ne part que sur un
callback de la boucle), mais il l'aurait été par un RESULT de recovery citant le jeton.

**KTD3 — l'ancrage seul, appliqué au seul lecteur, INTRODUIT un faux négatif.** Le
producteur de la convergence (`dispatch-lib.sh:9368`) écrit
`sed 's/Outcome: .*/Outcome: PLAN_GROOMED/'`, **non ancré** : sur un RESULT portant
`Outcome: ` en milieu de ligne, il produit le marqueur hors position. Le filet qui suit
(`grep -qF -- 'Outcome: PLAN_GROOMED'`, ligne 9370) est **lui aussi non ancré**, le voit,
et n'ajoute donc pas la ligne canonique. Un groom **réellement convergé** serait alors
refusé par le lecteur corrigé. Le producteur doit passer à `_set_outcome_line`
(mika#2492, déjà employé pour `PR_OPENED` et `ESCALATE`) **dans le même commit**. Le
ticket ne voit pas cette moitié.

**KTD4 — le point 1 se formule autrement qu'il ne l'est écrit.**
`grep -o '"status":"[a-z_]*"' skills/bundled/_shared/dispatch-lib.sh | sort -u` rend
**exactement une** valeur : `auto_skipped`. Il n'existe **aucun** RESULT de convergence
en JSON — la convergence est toujours un texte portant une ligne `Outcome:`. Donc
« tout RESULT JSON dont `status` n'est pas une convergence » a une allowlist de
convergences **vide**, et la règle exacte est : *une enveloppe JSON portant `status`
n'est jamais une preuve*. Sa population est **vide aujourd'hui** (l'ancrage suffit au
cas mesuré) — c'est une défense en profondeur, et c'est pourquoi elle est comptée à
part (D2).

**KTD5 — le correctif a un effet de population en production, que le ticket ne nomme
pas.** Tout ticket dont l'unique « preuve » est un `auto_skipped` passe de `Groomed` à
`MarkersWithoutProof` : il repart en `dev-groom`, se fait refuser `already_groomed`, et
consomme le budget de re-drive (mika#2020) jusqu'à `operator-review`. C'est **convergent
et voulu** — c'est exactement la limite que mika#2484 D4 nomme et borne — et strictement
meilleur qu'un implement sans plan. Mais la taille de cette population est inconnue
depuis le bac à sable, d'où la sonde préalable S0 et la condition d'arrêt (a).

### Requirements

- **R1 — un lecteur unique, pur, ancré.** Une fonction pure prend le `result` d'un
  callback et rend un verdict à trois états. Elle vit dans `task_state/tasks.rs`, à
  côté de `GROOM_SUCCESS_MARKER` dont le doc-comment énumère déjà ses deux lecteurs.
- **R2 — enveloppe JSON ⇒ jamais une preuve.** Un `result` qui parse comme un objet JSON
  portant un champ `status` est refusé, quel que soit son contenu (point 1 du ticket,
  reformulé par KTD4).
- **R3 — marqueur en position de verdict.** La preuve exige qu'une **ligne** du `result`
  commence par `GROOM_SUCCESS_MARKER` (point 2).
- **R4 — `has_completed_groom_for_issue` décide par R1.** Le `instr` quitte le SQL ; la
  décision est en Rust, sur les lignes que la jointure a déjà bornées.
- **R5 — `try_dispatch_pilot_after_groom_success` décide par R1.** Le `contains` part
  (point 5, moitié KTD2).
- **R6 — la position hors verdict est COMPTÉE.** Un callback portant le marqueur ailleurs
  qu'en position de verdict produit une ligne de journal **et** une ligne `audit_events`,
  à écrivain unique. Sans elle, un ticket refoulé pour preuve polluée se lit exactement
  comme un ticket jamais groomé — la classe mika#2205.
- **R7 — le producteur n'écrit plus le marqueur hors position** (KTD3), par
  `_set_outcome_line`.
- **R8 — la prose des refus ne cite plus le jeton** (point 3), et c'est **tenu
  structurellement**, pas prescrit : par
  `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`, une consigne
  seule ne tient pas au substrat de la boucle.
- **R9 — garde de classe.** Aucun second lecteur du marqueur par `contains`/`instr` ne
  peut réapparaître sans faire rougir un scan de source.
- **R10 — test négatif mesuré, vu rouge.** Le JSON d'auto_skip de l'incident, verbatim,
  rend `false` et route `dev-groom` (point 4).

### Non-goals

- **Rattraper mika#2105.** Son implement a eu lieu et a été secouru (#2589). Rien ici ne
  rétro-écrit une ligne : fabriquer une preuve — ou son absence — après coup est
  l'inverse de ce que ce travail défend. La sonde est la **prochaine** occurrence.
- **Toucher `latest_groom_verdict_for_issue`** (KTD1 : déjà ancré).
- **Fermer le masquage d'un `ESCALATE` par un `auto_skipped` plus récent.** Le
  `ORDER BY … LIMIT 1` de mika#2545 est une décision écrite (« le dernier, jamais un
  EXISTS ») ; un `auto_skipped` postérieur à un ESCALATE rendrait `NotEscalated`.
  Population non mesurée, blast radius distinct — **suivi nommé**, précondition : que la
  sonde S2 montre un `auto_skipped` succédant à un `Outcome: ESCALATE` sur un même
  ticket.
- **Faire converger un ticket groomé hors moteur.** Le correctif le renvoie au
  cul-de-sac convergent de mika#2484 D4 ; le rendre productif est ce ticket-là.
- **Ajouter une variante à `GroomedState`** (D6).
- **Exempter les lignes `PLAN_GROOMED` du prune de 30 jours** — suivi déjà ouvert par
  mika#2287, inchangé.

---

## Planning Contract

### D1 — Un lecteur, et il vit dans `task_state/tasks.rs`

`GROOM_SUCCESS_MARKER` y est déclaré, et son doc-comment nomme déjà ses deux lecteurs
(« Two readers share it and must never drift apart »). Les deux consommateurs sont
`db::Database` et `task_engine::dispatcher` : `db` ne doit pas dépendre de
`skills::executor` (où vit la sœur `groom_escalate_verdict`), donc le point neutre est
le fichier de la constante. Motif : `grooming_marker.rs` (mika#2158), dont le doc dit la
règle — *deux lecteurs d'une même question qui répondent différemment*, c'est le défaut,
pas le symptôme.

### D2 — Trois états, pas un booléen, et le troisième est la population du défaut

```rust
pub enum GroomConvergence {
    Converged,
    Absent,                            // le marqueur n'apparaît nulle part — cas nominal
    MarkerOutOfPosition(&'static str), // il apparaît, hors position de verdict
}
```

`Absent` et `MarkerOutOfPosition` appellent la **même** disposition (refuser) et **deux
lectures opérateur différentes** : la première est le régime nominal d'un premier
grooming, la seconde est un implement que la porte vient d'arrêter. Les fondre rendrait
la population du correctif incomptable. Motif explicite : `GroomedState` (mika#2484 D1),
`GroomVerdictState` (mika#2545), `phantom_aged_out` / `phantom_sweep_spared`
(mika#2156).

Deux motifs pour la troisième variante, **format de fil** (ils atterrissent dans
`audit_events.after_value`, l'opérateur en fait des `GROUP BY`) : constantes nommées à
un seul site, plus `ALL_GROOM_CONVERGENCE_REJECTIONS` pour que leur cardinalité soit
assertable — motif `ALL_GROOM_ESCALATE_VERDICTS`, `ALL_PURGE_REFUSAL_REASONS`.

- `json_envelope` — R2. **Régime attendu : vide** (KTD4). Une occurrence signifie qu'un
  producteur JSON est apparu ; c'est un résultat, pas une panne.
- `marker_not_line_anchored` — R3. **Régime attendu : non vide, décroissant.** Chaque
  ligne est un implement que la porte n'a pas laissé partir.

### D3 — Le `instr` quitte le SQL, il ne devient pas un pré-filtre

Le réflexe serait de garder `instr > 0` comme pré-filtre grossier (motif mika#2184 : le
proxy filtre, la mesure directe tranche). Il est **refusé ici**, pour une raison qui
tient au compteur : une ligne écartée par SQL est **invisible** au verdict, donc un
`auto_skipped` remonterait `Absent` au lieu de `MarkerOutOfPosition`, et R6 mesurerait
zéro sur exactement la population qu'il existe pour voir. La cardinalité ne le justifie
pas non plus : la jointure borne déjà à une poignée de lignes par issue
(`dispatch_class='groom'` + statut terminal + URL du parent). Effet de bord acquis :
**plus aucun lecteur SQL du marqueur**, ce qui rend la garde R9 totale plutôt que
partielle.

### D4 — La signature change ; on n'ajoute pas un second nom

`has_completed_groom_for_issue` passe de `Result<bool>` à `Result<GroomConvergence>`.
L'alternative — garder le booléen et ajouter `groom_proof_state` à côté — créerait deux
noms pour une question, c'est-à-dire la divergence que mika#2158 a dû fermer un cran plus
haut. Coût : ~12 sites de test et `async_db.rs` à mettre à jour, mécaniquement, sous
contrôle du compilateur. Le nom de la méthode ne change pas : c'est lui que les scans et
le harnais mika#2310 interrogent.

### D5 — Fail-closed, et l'asymétrie est écrite avant le reste

Un signal illisible ou ambigu **refuse**. Le coût des deux erreurs n'est pas le même :

- faux refus ⇒ le ticket repart en `dev-groom`, **convergent** et borné par le budget de
  re-drive (mika#2020) ;
- faux accord ⇒ un pilote **implement** part sans plan re-mesuré. C'est le défaut p1.

C'est la même direction que le `Err` propagé de `has_completed_groom_for_issue`
(`dispatch_check_failed`), et l'**inverse** du fail-safe du faucheur mika#2420 — là
l'action détruisait du travail, ici l'inaction laisse partir un implement. *L'arbitrage
est local et ne se transporte pas.*

### D6 — Pas de variante neuve sur `GroomedState`

`route_for` et `refusal_for` (`ready_label_handler.rs`) sont des `match` exhaustifs sans
bras `_ =>` : une cinquième variante forcerait quatre décisions par le compilateur, pour
un **routage identique** (`dev-groom`) et un JSON de refus identique
(`dispatch_grooming_not_verified`, qui dit déjà la bonne chose). L'information neuve est
portée par l'événement R6, pas par le type. Le doc-comment de `MarkersWithoutProof`
gagne sa troisième cause (« une preuve existait mais pas en position de verdict »).

### D7 — La note du refus est corrigée ET gardée

R8 est la moitié *intention*. La moitié qui tient est une assertion dans
`test-dispatch-lib.sh` : aucune ligne construisant un `RESULT` `auto_skipped` ne porte
le littéral du marqueur. Sans elle, un futur éditeur recopie la phrase depuis le
commentaire voisin et le défaut revient — et le lecteur corrigé le **rattraperait**
(l'enveloppe JSON est refusée par R2), ce qui est précisément pourquoi la garde vaut le
coup : elle empêche la régression de se cacher derrière une défense en profondeur.

### D8 — Le fixture du test négatif est GELÉ, jamais régénéré

Le JSON du test R10 est celui de l'incident (`89165fb4`), recopié verbatim. Le
**régénérer** depuis le `dispatch-lib.sh` corrigé ferait disparaître la forme même que le
test doit refuser : le test passerait des deux côtés du correctif et n'attesterait rien.
Motif écrit : `tests/fixtures/grooming_bodies/` et `plan_callout_bodies/` (mika#2158,
mika#2120 — « rafraîchir depuis GitHub effacerait les formes que le prédicat doit
reconnaître »). Une ligne de commentaire le dit au site.

---

## Implementation Units

### U1 — La fonction pure et son type (`crates/mika-agent/src/task_state/tasks.rs`)

À côté de `GROOM_SUCCESS_MARKER` :

- `GroomConvergence` (D2), `#[derive(Debug, Clone, PartialEq, Eq)]`, `pub`.
- `pub const GROOM_REJECTED_JSON_ENVELOPE: &str = "json_envelope";`
- `pub const GROOM_REJECTED_NOT_LINE_ANCHORED: &str = "marker_not_line_anchored";`
- `pub const ALL_GROOM_CONVERGENCE_REJECTIONS: &[&str] = &[…];`
- `pub fn groom_result_convergence(result: &str) -> GroomConvergence` :
  1. `serde_json::from_str::<serde_json::Value>(result)` ; si c'est un objet portant
     `status`, rendre `MarkerOutOfPosition(GROOM_REJECTED_JSON_ENVELOPE)` **si** le texte
     contient le marqueur, sinon `Absent`. *Le motif ne se pose que quand il y a quelque
     chose à écarter* — sinon un skip ordinaire polluerait le compteur de R6.
  2. Si une ligne commence par `GROOM_SUCCESS_MARKER` ⇒ `Converged`.
  3. Sinon, si le texte le contient ⇒ `MarkerOutOfPosition(GROOM_REJECTED_NOT_LINE_ANCHORED)`.
  4. Sinon ⇒ `Absent`.
- `pub fn aggregate(results: impl Iterator<Item = GroomConvergence>) -> GroomConvergence`
  — ou la même logique inline chez l'appelant : `Converged` dès qu'une ligne prouve,
  sinon `MarkerOutOfPosition` si au moins une a été écartée, sinon `Absent`. Le doc dit
  pourquoi l'ordre de préséance est celui-là : la sémantique historique est `COUNT > 0`,
  donc **une** preuve valide suffit, et l'agrégat ne doit pas se laisser dégrader par un
  skip qui la précède.
- Doc-comment : l'incident, l'offset 651, la sœur `groom_escalate_verdict`, et le fait
  que `_set_outcome_line` (après U5) garantit la position — donc l'ancrage ne perd rien.

Le doc-comment existant de `GROOM_SUCCESS_MARKER` est corrigé : ses deux lecteurs passent
par cette fonction, jamais par `contains`.

### U2 — `has_completed_groom_for_issue` (`crates/mika-agent/src/db/tasks.rs`)

- Signature : `Result<GroomConvergence>`.
- Le `SELECT COUNT(*) … AND instr(child.result, ?4) > 0` devient
  `SELECT child.result FROM … ` sans le terme `instr` (D3), via `stmt.query_map`.
- Agrégation par U1. Une ligne `result` NULL est `Absent` (elle ne porte rien).
- Doc-comment : la sous-chaîne et son remède, l'incident, D3, D5. La phrase « son
  `result` … carries `GROOM_SUCCESS_MARKER` on convergence » devient « … porte le
  marqueur **en position de verdict** ».
- `async_db.rs:1663` suit la signature.

### U3 — `try_dispatch_pilot_after_groom_success` (`crates/mika-agent/src/task_engine/dispatcher.rs:4089`)

L'étape 2 passe de `Some(r) if r.contains(MARKER)` à un `match` sur
`groom_result_convergence(r)`. `Converged` continue ; les deux autres rendent la main.
Pas d'émission ici : le site est un auto-fire *fire-and-forget* dont chaque précondition
échouée est déjà silencieuse, et lui donner un compteur de plus mélangerait sa population
avec celle de la porte (R6), qui est celle du ticket.

### U4 — `groomed_state` (`crates/mika-agent/src/skills/executor.rs:1837`) — traduction + émission

- `Ok(Converged) → GroomedState::Groomed`
- `Ok(Absent) → GroomedState::MarkersWithoutProof`
- `Ok(MarkerOutOfPosition(reason)) → GroomedState::MarkersWithoutProof`, **précédé** d'un
  `warn!` et d'une ligne `audit_events` (R6).
- `Err(e) → GroomedState::ProofUnreadable(…)` (inchangé).

Nom d'événement, **SOLE WRITER** : `groom_proof_marker_out_of_position`
(`GROOM_PROOF_OUT_OF_POSITION_EVENT`). Champs : `issue_url`, `reason`
(∈ `ALL_GROOM_CONVERGENCE_REJECTIONS`). `audit_events` : `tool_name` = le nom,
`target_key` = `issue:<owner/repo#n>`, `after_value` = le motif. Format de fil, même
raison que `ReadyLabelGate::wire_name`.

À vérifier à l'implémentation : `AsyncDatabase` expose-t-il `log_audit_event` avec cette
signature depuis ce site ? Si l'écriture d'audit échoue, elle est `warn!`ée et **ne
change pas le verdict** — motif `a2a_turn_failed_audit_failed` (mika#2522).

Le doc-comment de `MarkersWithoutProof` gagne sa troisième cause (D6).

### U5 — `skills/bundled/_shared/dispatch-lib.sh`

**U5a (R7, KTD3), ligne ~9367-9375.** Remplacer

```sh
RESULT=$(printf '%s' "$RESULT" | sed 's/Outcome: .*/Outcome: PLAN_GROOMED/')
if ! grep -qF -- 'Outcome: PLAN_GROOMED' <<<"$RESULT"; then … fi
```

par `_set_outcome_line "Outcome: PLAN_GROOMED"`. Le `sed '/^PIPELINE FAILURE:/d'` qui
précède est **conservé**. Le filet disparaît : `_set_outcome_line` rend « exactement une
ligne `Outcome:` ancrée » vraie **par construction**, ce que le couple sed+grep ne rendait
vrai que par coïncidence. Un commentaire dit pourquoi le `sed` non ancré était un piège.

*Effet de position, vérifié :* `_set_outcome_line` retire la ligne et la **réappend** en
queue. Les trois lecteurs shell sont insensibles à la position
(`_measure_cycle_output`: `grep -m1 -E '^Outcome: …'`, `_gate_non_empty_cycle`, la branche
ESCALATE : `grep -qE '^Outcome: ESCALATE'`), et le lecteur Rust lit ligne à ligne.
Précédent : `PR_OPENED` (9685) et `ESCALATE` (7248) passent déjà par là.

**U5b (R8), ligne ~3231.** La note de l'auto_skip `already_groomed` cesse de citer le
jeton : « … unless a completed groom callback **carries a convergence verdict**, and this
skip mints none. » Le reste de la note — le geste de reprise — est inchangé. Le
commentaire de code voisin (3225-3226) est également neutralisé et gagne une ligne
nommant mika#2590 : c'est lui que le prochain éditeur recopierait.

### U6 — `scripts/canonical-tokens.tsv`

- Déclarer le nouveau lecteur strict :
  `PLAN_GROOMED	B	crates/mika-agent/src/task_state/tasks.rs::groom_result_convergence	exact:prefix`
  — même tolérance et même forme que `ESCALATE … ::GROOM_ESCALATE_MARKER	exact:prefix`.
  Le fichier est déjà déclaré pour la constante, donc le scan d'exhaustivité ne rougit
  pas ; la ligne est ajoutée parce que la doctrine du fichier est *« un jeton dont le
  lecteur est strict est déclaré avec son site »*, et qu'un trou silencieux est ce qui
  fait dériver un lint.
- Après U5a, la lecture de `dispatch_claude_pilot` n'est plus un `grep -qF` mais un
  argument passé à `_set_outcome_line`, qui écrit ancré : la tolérance de la ligne
  `PLAN_GROOMED	B	…::dispatch_claude_pilot` passe de `exact:literal` à
  `exact:line-anchored`. À confirmer contre `scripts/check-canonical-tokens.sh` et
  `mika2201_every_declared_symbol_still_exists` (le symbole `dispatch_claude_pilot`
  subsiste, donc aucune entrée ne devient périmée).

### U7 — Gardes structurelles (R9)

**U7a — scan de source Rust**, `canonical_tokens` ou un module frère,
`mika2590_le_marqueur_de_convergence_na_quun_lecteur` : dans les fichiers de production
(`production_sources()`, qui exclut déjà les chemins de test via `is_test_source_path`),
aucune ligne ne doit lire `GROOM_SUCCESS_MARKER` par `contains(`, `instr(` ou
`.find(`. Seuls `task_state/tasks.rs` (la définition et sa fonction) sont hors
population. Allowlist `GROOM_MARKER_LOOSE_READERS_ALLOWED` **livrée vide**, et un test
frère la pinne vide (motif `mika2323_no_gate_predicate_reads_the_actor`,
`mika1883_run_usage_accumulates_only_via_the_one_helper`).

*Anti-vacuité obligatoire* : le scan assert que la population examinée est non vide et
que le symbole `GROOM_SUCCESS_MARKER` est lu **quelque part**. Sans ça, un renommage
rendrait le scan silencieusement inerte, ce qui se lit exactement comme un arbre propre
(mika#2205).

**U7b — assertion shell** (D7), dans `test-dispatch-lib.sh` : aucune ligne du fichier
construisant un `RESULT` `auto_skipped` ne porte le littéral `Outcome: PLAN_GROOMED`.
Prédicat par ligne logique sur les lignes portant `"status":"auto_skipped"`, avec un
**contrôle négatif** (une fixture portant le littéral doit faire rougir) pour que
« l'assertion tient » soit distinguable de « l'assertion ne regarde rien ».

### U8 — Tests

**Ordre de travail imposé :** U8a et U8b sont écrits et lancés **avant** U1–U3, et vus
rouges. C'est la lecture littérale du point 4 (« il doit rougir sur `main` actuel »), et
c'est un geste ordinaire plutôt qu'une acrobatie git — rien à stasher, rien à extraire.

- **U8a — le test négatif mesuré** (R10), dans `db/tests/harnais_porte.rs`. Nouvelle
  constante gelée `GROOM_CALLBACK_AUTO_SKIPPED` dans `db/tests/mod.rs`, à côté de
  `GROOM_CALLBACK_PLAN_GROOMED` : le JSON de l'incident, verbatim, avec le commentaire de
  D8. `completed_groom_pair(&db, "mika", GROOM_ISSUE_URL, GROOM_CALLBACK_AUTO_SKIPPED)`
  puis `assert_eq!(…, GroomConvergence::MarkerOutOfPosition(GROOM_REJECTED_JSON_ENVELOPE))`.
  Tout par l'API d'écriture de production — condition 5 du GO mika#2287, zéro `INSERT`
  brut.
- **U8b — le routage** (R10, seconde moitié), dans `skills::executor::tests::harnais_porte` :
  même base, un corps d'issue portant les trois callouts ⇒ `groomed_state` rend
  `MarkersWithoutProof` et `route_for` rend `("run_claude_pilot_groom","dev-groom","groom")`.
- **U8c — la fonction pure aux bornes** : les quatre cas de U1, plus le **contrôle
  positif** (`GROOM_CALLBACK_PLAN_GROOMED` ⇒ `Converged`) et le cas frontière qui compte —
  un marqueur en **milieu de ligne** dans un texte non-JSON (la prose d'un pilote citant
  le mécanisme, forme mesurée par mika#2050) ⇒ `MarkerOutOfPosition(marker_not_line_anchored)`.
- **U8d — non-régression de la porte** : les cas 4 (`failed`/`cancelled`) et 8
  (`Err` propagé) du harnais mika#2310 passent, signature adaptée. Le cas 8 est le
  contrôle de D5 : `Err`, jamais `Ok(Absent)`.
- **U8e — U3** : un callback groom dont le `result` est le JSON d'auto_skip ne déclenche
  aucun auto-fire ; un callback portant la ligne ancrée le déclenche (contrôle positif —
  sans lui, « l'auto-fire ne part pas » serait indistinguable de « l'auto-fire est cassé »).
- **U8f — U5a**, dans `test-dispatch-lib.sh` : le bras GROOMED écrit **exactement une**
  ligne `^Outcome: PLAN_GROOMED` ; et, contrôle négatif porteur, un RESULT d'entrée
  portant `Outcome: ` **en milieu de ligne** ressort avec la ligne canonique ancrée
  (c'est le faux négatif de KTD3, vu rouge avant U5a).
- **U8g — U7b** et sa fixture de contrôle négatif.

---

## Fire-Disposition

Ce plan livre cinq détecteurs : le scan de source U7a, l'assertion shell U7b, et les
tests U8a/U8b/U8f dont le chemin de succès est « aucune violation ».

**Option retenue : (a) — exception nommée en allowlist, avec une allowlist livrée VIDE.**

- **U7a** porte `GROOM_MARKER_LOOSE_READERS_ALLOWED`, **vide**, parce que U2 et U3
  retirent dans le même commit les **deux** seules violations existantes de l'arbre
  (inventaire exhaustif : `grep -rn GROOM_SUCCESS_MARKER crates/` rend cinq sites — la
  définition, un doc-comment d'`executor.rs`, `dispatcher.rs:4089` (U3),
  `db/tasks.rs:4196` (U2), et une ligne du TSV). Assertion auto-nettoyante frère :
  `mika2590_lallowlist_des_lecteurs_laches_reste_vide` rougit si elle cesse d'être vide,
  et son message nomme la résolution — *on retire la lecture, on ne l'allowliste pas*
  (doctrine mika#2201 § D5/D6).
- **U7b** porte la même forme, allowlist vide, U5b retirant l'unique occurrence.
- **U8a/U8b/U8f** sont des tests d'un défaut **fermé dans le même commit** : ils n'ont pas
  de population préexistante à exempter, donc aucune allowlist. Leur discipline est
  l'ordre de travail de U8 (vus rouges avant le correctif), sans quoi « le test passe »
  ne distingue pas un correctif d'un test qui ne regarde rien.

**Ni (b) ni (c).** (b) livrerait le scan désarmé pour une population **vide après le
commit**, c'est-à-dire un `#[ignore]` sans dette derrière — et un détecteur désarmé se lit
exactement comme un détecteur vert (mika#2205). (c) n'a pas lieu d'être : aucune violation
ne subsiste dont l'arbitrage dépasserait ce ticket.

**Ce que ces détecteurs n'attrapent pas, nommé :** un troisième lecteur qui reconstruirait
le littéral à la main (`"Outcome: " + "PLAN_GROOMED"`) échappe à U7a, dont le prédicat est
sur le symbole. Le scan d'exhaustivité mika#2201, lui, part du **jeton** et verrait le
littéral — la composition des deux ferme le trou que chacun laisse, exactement comme le
doc de `canonical_tokens.rs` l'énonce.

---

## Verification Contract

| # | vérification | geste |
|---|---|---|
| V1 | U8a rouge **avant** U1–U3, vert après | `cargo test -p mika-agent harnais_porte`, deux fois, dans l'ordre de U8 |
| V2 | U8f rouge **avant** U5a, vert après | `bash skills/bundled/_shared/test-dispatch-lib.sh`, idem |
| V3 | la porte, toutes ses bornes | `cargo test -p mika-agent harnais_porte` |
| V4 | la fonction pure, six cas dont deux contrôles positifs | `cargo test -p mika-agent groom_result_convergence` |
| V5 | les gardes regardent quelque chose | `cargo test -p mika-agent mika2590`, puis vérifier que l'anti-vacuité de U7a rougit sur un arbre où le symbole est renommé |
| V6 | jetons canoniques | `bash scripts/check-canonical-tokens.sh` + `cargo test -p mika-agent mika2201` |
| V7 | suite shell complète | `bash skills/bundled/_shared/test-dispatch-lib.sh` |
| V8 | non-régression globale | `make test` puis `make lint` |
| V9 | structure des bundles | `make verify-bundled-skills` |

**V10 — ce qui n'est PAS vérifiable ici, écrit plutôt que découvert.** « La porte refuse
le prochain implement sans grooming » est exécuté par le moteur en production, contre une
base que le bac à sable de dispatch ne monte pas. Le contrat côté code est *le prédicat
rend `MarkerOutOfPosition` sur le JSON mesuré*, et V1 l'atteste déterministiquement. La
moitié comportementale est la sonde S1.

---

## Definition of Done

- R1–R10 livrés ; U1–U8 committés ; V1–V9 verts, V1 et V2 **vus rouges d'abord**.
- Zéro `contains`/`instr` sur `GROOM_SUCCESS_MARKER` hors de `task_state/tasks.rs`.
- Les deux allowlists de U7 sont vides et pinnées vides.
- Le corps de PR porte : le résultat de la sonde préalable S0 **ou** la mention explicite
  qu'elle n'a pas pu être exécutée depuis le bac à sable (et reste un geste d'opérateur),
  le balayage KTD1 comme **résultat**, et les trois suivis nommés.

## Acceptance criteria

Transcrits du § *Attendu* de mika#2590, et rectifiés là où la mesure l'impose (KTD).

1. **Un callback de groom `auto_skipped` ne vaut jamais preuve PLAN_GROOMED.** Étendu par
   KTD4 à sa forme exacte : *un `result` qui est une enveloppe JSON portant `status`*
   — l'allowlist des statuts convergents étant vide, aucun chemin n'écrivant une
   convergence en JSON. Attesté par U8a.
2. **La preuve se lit structurellement** : une **ligne** du `result` commence par le
   marqueur. Attesté par U8c, y compris le cas frontière du marqueur en milieu de ligne.
3. **La prose des refus ne cite plus le marqueur littéral**, et c'est **tenu** par U7b et
   non seulement prescrit.
4. **Test négatif** : un callback `groom`/`delivered` dont le `result` est le JSON
   d'auto_skip rend `has_completed_groom_for_issue` ≠ `Converged` et un routage
   `dev-groom`. **Il rougit sur `main` actuel** — V1/U8b, l'ordre de travail de U8 en
   fait la démonstration.
5. **Les autres lecteurs de `GROOM_SUCCESS_MARKER` sont balayés**, et le balayage est un
   **résultat écrit**, pas un travail supposé : `try_dispatch_pilot_after_groom_success`
   portait le défaut (corrigé, U3) ; `latest_groom_verdict_for_issue` ne lit aucun
   marqueur et son consommateur `groom_escalate_verdict` est **déjà ancré** depuis
   mika#2545 (KTD1, inchangé).
6. **Aucun faux négatif introduit** (KTD3, hors AC du ticket et bloquant) : le producteur
   n'écrit plus le marqueur hors position — U5a, attesté par U8f et son contrôle négatif.

---

## Risks & Dependencies

### Sondes, et leurs haltes

**S0 — préalable, geste d'opérateur sur `~/.mika/data/mika.db`, AVANT déploiement.**
Combien de tickets ne tiennent leur état `Groomed` que par un `auto_skipped` ?

```sql
SELECT parent.reference_url, child.id, child.created_at
  FROM tasks child JOIN tasks parent ON child.parent_task_id = parent.id
 WHERE child.trigger_type = 'callback' AND child.dispatch_class = 'groom'
   AND child.status IN ('completed','delivered')
   AND instr(child.result, 'Outcome: PLAN_GROOMED') > 0
   AND child.result LIKE '{"status":"auto_skipped"%'
 ORDER BY child.created_at DESC;
```

**Halte 1 — la liste est longue.** Ce n'est pas une panne du correctif : c'est la taille
de la population qu'il va renvoyer, en un tick, dans le cul-de-sac convergent de
mika#2484 D4 (trois re-drives puis `operator-review`). **Ne pas désarmer le correctif** —
remonter, et décider si le déploiement s'accompagne d'un geste de re-grooming. C'est la
condition d'arrêt (a).

**S1 — le défaut ne se rejoue pas (7 jours).** Aucun `ready_label_handled` avec
`target_skill=dev-pilot groomed=true` sur un ticket dont le dernier callback groom est un
`auto_skipped`.

```bash
grep groom_proof_marker_out_of_position "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{issue_url, reason}'
```

```sql
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'groom_proof_marker_out_of_position' GROUP BY 1 ORDER BY 2 DESC;
```

| `after_value` | régime attendu | lecture |
|---|---|---|
| `marker_not_line_anchored` | **non vide, décroissant** | chaque ligne est un implement sans plan que la porte a arrêté |
| `json_envelope` | **vide** (KTD4) | une occurrence dit qu'un producteur JSON est apparu — un **résultat**, pas une panne |

**Halte 2 — les deux compteurs vides et un ticket part quand même en implement sans
plan.** **Ne pas élargir le prédicat par réflexe.** Établir d'abord que le binaire servi
porte le correctif (classe mika#2340 : `cat ~/.mika/skills/.manifest-writer`), puis quel
chemin a dispatché — `ready_label_outcome` (mika#2323) nomme la porte. Les remèdes
diffèrent.

**Halte 3 — `marker_not_line_anchored` porte du trafic nominal** (plusieurs par heure,
sur des tickets différents). Le prédicat n'est pas trop strict : soit U5a n'a pas atteint
le producteur servi (même classe mika#2340), soit un chemin de convergence écrit encore
le marqueur hors position. **Lire le `result` d'un groom réellement convergé avant de
toucher au lecteur** — c'est le faux négatif de KTD3 qui reviendrait.

**Halte 4 — les deux compteurs vides et aucun groom n'a tourné.** On ne peut **rien**
conclure. Vérifier le contrôle positif — qu'un groom a bien convergé depuis le
déploiement (`SELECT count(*) FROM tasks WHERE dispatch_class='groom' AND status IN
('completed','delivered') AND created_at > '<déploiement>'`) — avant toute conclusion.
*Une garde que personne n'a exercée se lit exactement comme une garde qui marche*
(mika#2205).

**S2 — contrôle négatif de bruit (7 jours).** Aucun groom réellement convergé n'est
refusé : le taux de `Outcome: PLAN_GROOMED` produits doit rester égal au taux de
dispatches `implement` qui suivent. **Halte 5 —** un groom convergé refusé : c'est KTD3,
désarmer U2 (revert du terme d'ancrage seul) **avant** diagnostic — un correctif qui bloque
la boucle coûte plus que le défaut qu'il ferme.

### Ce que ce travail n'achète PAS

- **Aucun rattrapage.** mika#2105 a reçu son implement ; rien ici ne rétro-écrit une
  ligne. La sonde est la prochaine occurrence.
- **Aucune convergence pour un ticket groomé hors moteur.** Il repart en `dev-groom`,
  se fait `already_groomed`, et s'abandonne en `operator-review` après trois re-drives.
  Convergent, borné, visible — et strictement meilleur qu'un implement sans plan. C'est
  la limite mika#2484 D4, non fermée ici.
- **Aucun compteur côté producteur.** La population des grooms convergés n'est pas
  instrumentée par ce travail ; elle se lit par `Outcome: PLAN_GROOMED` dans
  `tasks.result`, comme avant.
- **Aucune protection contre un lecteur qui reconstruirait le littéral à la main** — voir
  le § *Ce que ces détecteurs n'attrapent pas*.

### Suivis nommés

1. **Un `auto_skipped` postérieur à un `Outcome: ESCALATE` masque l'escalade**
   (`latest_groom_verdict_for_issue`, `ORDER BY … LIMIT 1`). Précondition : que S2 en
   montre une occurrence.
2. **Le cul-de-sac `already_groomed`** (mika#2484 D4) — inchangé, et ce correctif en
   augmente la population.
3. **Exempter les lignes `PLAN_GROOMED` du prune de 30 jours** — suivi déjà ouvert par
   mika#2287, priorité inchangée.

### Dépendances

- `serde_json` est déjà une dépendance de `mika-agent` (`Cargo.toml:39`,
  `serde_json.workspace = true`) : R2 n'en ajoute aucune.
- Aucun déploiement croisé : le Rust et `dispatch-lib.sh` voyagent dans le même binaire
  (`skills/bundled/` est une projection du binaire, mika#2340), donc U5 et U1–U4
  atterrissent ensemble. **C'est ce qui rend KTD3 sûr** : le lecteur ancré et le
  producteur ancré ne peuvent pas être servis séparément.

---

## Sources

- Corps de mika#2590 (constat, cause vérifiée, cinq attendus) — aucun commentaire.
- `crates/mika-agent/src/task_state/tasks.rs:31-40` — `GROOM_SUCCESS_MARKER` et son doc.
- `crates/mika-agent/src/db/tasks.rs:4152-4200` — `has_completed_groom_for_issue`.
- `crates/mika-agent/src/db/tasks.rs:4202-4278` — `latest_groom_verdict_for_issue` (KTD1).
- `crates/mika-agent/src/task_engine/dispatcher.rs:4075-4092` — l'auto-fire (KTD2).
- `crates/mika-agent/src/skills/executor.rs:1790-1862` — `GroomedState`, `groomed_state`.
- `crates/mika-agent/src/skills/executor.rs:1938-1965` — `groom_escalate_verdict`, la sœur
  déjà ancrée (mika#2545).
- `crates/mika-agent/src/server/ready_label_handler.rs:518-590` — `route_for`,
  `refusal_for`, les `match` exhaustifs de D6.
- `crates/mika-agent/src/db/tests/mod.rs:940-1035` — `completed_groom_pair` et les
  constantes de fixture.
- `crates/mika-agent/src/db/tests/harnais_porte.rs` — le harnais mika#2310.
- `skills/bundled/_shared/dispatch-lib.sh:3212-3234` — le saut `already_groomed` (U5b).
- `skills/bundled/_shared/dispatch-lib.sh:1885-1894` — `_set_outcome_line` (mika#2492).
- `skills/bundled/_shared/dispatch-lib.sh:9358-9378` — le producteur de la convergence
  (U5a, KTD3).
- `skills/bundled/_shared/dispatch-lib.sh:4006` — `_measure_cycle_output`, lecteur ancré.
- `scripts/canonical-tokens.tsv:155-170` — la grammaire d'enveloppe de callback.
- `crates/mika-agent/src/canonical_tokens.rs:1-60, 132-170, 280-345` — les deux gardes
  mika#2201 et `production_sources()`.
- CLAUDE.md § *Un `labeled ready` répété sur un pilote vif est un NO-OP* (mika#2279),
  § *Un callout de corps sans preuve en base route vers `groom`* (mika#2484),
  § *Un `ESCALATE` de groom est terminal* (mika#2545), et les doctrines mika#2050,
  mika#2131, mika#2156, mika#2158, mika#2201, mika#2205, mika#2340.
