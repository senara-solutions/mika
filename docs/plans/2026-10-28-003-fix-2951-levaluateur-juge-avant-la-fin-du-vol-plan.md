# fix(2951) — l'évaluateur juge une tâche de groom avant le pilote d'implémentation relancé sur elle

Ticket : senara-solutions/mika#2951 (`bug`, `p1-important`, `dispatch:loop`, `loop-substrate`)
Base : `origin/main` 0d30417e
Lié : mika#2946 / PR #2949 (les grooms purs rangés à part), mika#2698 (Phase 2, concordance), mika#1614 (relance moteur par réutilisation de tâche), mika#2886 (topologie à deux lignes)

---

## Le défaut, en trois couches

La ligne « mort PLAN_GROOMED » inscrite pour un vol qui a ouvert une PR n'a pas une
cause mais trois, empilées. Les nommer séparément est ce qui décide où le correctif
va, parce que réparer la troisième seule (réécrire le registre) laisserait les deux
premières produire la même fausseté au vol suivant.

**(a) Rétention.** `db/tasks.rs::list_finished_implement_flights_in_container`
retient une parente dès que `is_terminal_task_status(&t.status)`. Or mika#1614 fait
de `completed` l'état **normal de milieu de vol** d'une parente de groom réutilisée :
`tools/complete_task.rs` le dit en toutes lettres (« la tâche ready-label parente est
`completed` *normalement* après un groom, puisque le moteur relance le pilote en
réutilisant la tâche »). **La terminalité de la parente n'est plus la terminalité du
vol.** C'est la prémisse que mika#1614 a cassée et que personne n'a rouverte depuis.

**(b) Choix de l'enfant.** La requête 2 de la même fonction joint les enfants pilotes
*à cet instant* et garde le plus récent par parente. À l'instant où (a) retient la
parente, le seul enfant existant est le pilote de **groom**, qui finit en
`NO_PR: no_pr_on_branch`. `flight_source.rs::task_to_finished_flight` projette alors
`result: pilot_result.or(t.result)` — l'enfant d'abord, donc le groom.

**(c) Gel.** `flight_verdict_ledger.rs::write_and_swallow` lit l'idempotence par
`count_recent_audit_events_for_target(MANAGER_FLIGHT_VERDICT_TOOL, task_id, …)` et
rend `already_recorded` dès qu'une ligne existe. Le premier verdict est définitif.
C'est une propriété voulue (un verdict ne doit pas osciller) ; elle ne devient un
défaut que parce que (a) autorise un premier verdict **prématuré**.

Le correctif attaque (a) et (b). (c) reste intact : on ne rend pas le registre
réécrivable, on empêche qu'il reçoive une ligne avant la fin du vol.

## La propriété d'ordonnancement sur laquelle le correctif s'appuie

Elle doit être nommée, parce que tout le plan en dépend et qu'elle n'est écrite
nulle part aujourd'hui.

Dans `task_engine/dispatcher.rs`, le bloc de livraison de callback appelle dans
cet ordre :

1. `try_complete_parent_on_callback_success` (ligne 1518) — **no-op pour un callback
   de groom**, et son propre préambule le dit : « groom callbacks carry no `pr_url` ».
2. `try_dispatch_pilot_after_groom_success` (ligne 1537) — bascule
   `dispatch_class` groom → implement sur la parente et **crée l'enfant
   d'implémentation** par `spawn_long_running_exec`.

La parente ne passe donc pas en `completed` dans ce bloc. Son `completed_at` vient
du **tour LLM de callback qui suit**, lequel appelle `complete_task`. Autrement dit :
**quand la parente devient terminale, la ligne de l'enfant d'implémentation existe
déjà**, en statut non terminal. C'est ce qui rend « il existe un enfant
d'implémentation non terminal » un prédicat suffisant pour AC1, et non un pari sur
une course.

Cette propriété est une dépendance, pas une évidence : elle doit être **épinglée par
un test** (D5 ci-dessous), faute de quoi un réordonnancement du bloc de livraison la
casserait en silence.

Deux chemins la laissent fausse, et ils sont *corrects* ainsi : si la relance
n'allume pas (frein STOP `auto_pull_stop::is_stopped`, pas de jeton GitHub, handler
`run_claude_pilot` non résolu, porte mika#1620), aucun enfant d'implémentation
n'existe, rien n'a volé, et le classement reste celui de mika#2949
(`not_a_flight:groom_completed`). C'est exactement AC2.

---

## Décision D1 (AC1) — le discriminant est le LABEL de l'enfant

Le ticket demande de trancher par écrit entre le label, l'ordre, ou autre chose.
**Le label**, pour quatre raisons, dont trois disqualifient les alternatives.

- **Il existe déjà comme source de vérité unique, à ce site exact.**
  `task_engine/engine.rs:582` déclare `PILOT_DISPATCH_LABELS = ["long_running:run_claude_pilot",
  "long_running:run_claude_pilot_groom"]`, et `db/tasks.rs` filtre déjà ses enfants
  par `is_pilot_dispatch_label`. Le discriminant n'ajoute pas une donnée : il lit plus
  finement celle qui est déjà lue.
- **Il a un producteur unique et une attestation.** Le label est écrit par
  `skills::executor::build_callback_task` et par rien d'autre, et
  `mika2875_la_constante_est_la_forme_que_le_producteur_emet` rejoue ce constructeur
  pour les deux outils. Le discriminant hérite de cette garantie sans en créer une.
- **`dispatch_class` ne peut pas servir.** La colonne est nullable, et toute la
  couche de lecture la passe par `COALESCE(dispatch_class, 'implement')` : un enfant
  dont la classe est NULL serait classé « implémentation » — un faux positif qui
  ferait juger un vol fantôme. Pire, la sémantique est inversée pour ce qu'on cherche :
  `has_completed_groom_for_issue` documente que l'enfant garde `dispatch_class='groom'`
  précisément parce que la parente, elle, a basculé.
- **L'ordre ne peut pas servir.** « Le plus récent » est ce que fait le code
  aujourd'hui, et c'est la moitié du défaut. Un re-groom ultérieur sous la **même**
  parente (mika#1614 réutilise la tâche) redonnerait la victoire au groom, et le
  même faux reviendrait par la porte d'à côté.

**Forme du code.** Deux constantes nommées, et les listes et prédicats **dérivés**
d'elles — jamais une seconde liste, qui serait un second site de vérité :

```rust
pub(crate) const IMPLEMENT_PILOT_DISPATCH_LABEL: &str = "long_running:run_claude_pilot";
pub(crate) const GROOM_PILOT_DISPATCH_LABEL: &str = "long_running:run_claude_pilot_groom";

pub(crate) const PILOT_DISPATCH_LABELS: &[&str] =
    &[IMPLEMENT_PILOT_DISPATCH_LABEL, GROOM_PILOT_DISPATCH_LABEL];

pub(crate) fn is_implement_pilot_label(label: &str) -> bool {
    label == IMPLEMENT_PILOT_DISPATCH_LABEL
}
```

`is_pilot_dispatch_label` reste l'union, inchangée dans sa forme et dans ses appelants.

## Décision D2 (AC1) — une parente dont l'implémentation vole encore n'est pas rendue

Dans `list_finished_implement_flights_in_container`, la requête 2 cesse de filtrer
les enfants sur « terminal » **avant** de décider, et calcule par parente :

| grandeur | définition |
|---|---|
| `implement_en_vol` | ∃ enfant tel que `is_implement_pilot_label(label)` et `!is_terminal_task_status(status)` |
| `dernier_implement` | le plus récent enfant implement terminal au `result` non vide |
| `dernier_groom` | le plus récent enfant groom terminal au `result` non vide |

Puis, par parente :

- `implement_en_vol == true` → la parente est **ajournée** (voir D3) ;
- sinon `pilot_result = dernier_implement.or(dernier_groom)` — **l'implémentation
  prime le groom par CLASSE, jamais par récence**, ce qui est le corollaire direct
  de D1 et ce qui ferme le cas du re-groom tardif.

Les trois filtres actuels restent côté Rust, pour la raison déjà documentée au site :
la requête rend les enfants, le tri est un prédicat lisible et testable. Le nombre de
paramètres SQL ne bouge pas.

## Décision D3 — l'ajournement se voit, parce qu'un retrait silencieux est une classe de défaut

Retirer une ligne d'un résultat sans le dire produit exactement la silence que
mika#2205 refuse : « aucun vol terminé » et « un vol ajourné » rendraient les mêmes
octets, et le manager paraîtrait sain pendant qu'il ne compte rien.

La couche DB ne décide donc pas — elle **rapporte**. `FinishedFlightRow` gagne un
champ, et le filtre vit dans `flight_source.rs`, qui est déjà le lecteur unique
(mika#2921) et qui a un logger :

```rust
pub struct FinishedFlightRow {
    pub flight: Task,
    pub pilot_result: Option<String>,
    /// mika#2951 — un enfant d'implémentation est encore en vol sous cette
    /// parente : elle est terminale (mika#1614 la réutilise), le vol ne l'est pas.
    pub pending_implement: bool,
}
```

`DbFlightSource::finished_implement_flights` écarte les lignes `pending_implement`
et émet, **à chaque lecture y compris quand le compte est zéro** :

- INFO `flight_deferred_pending_implement` — champs `deferred` (usize),
  `task_ids` (joints par virgule, **absent** et non `""` quand `deferred == 0`,
  motif mika#2331), `scope = "container"`.

L'**absence** de cette ligne signifie un binaire antérieur au correctif
(motif mika#2340), jamais « rien n'a été ajourné ». C'est la sonde qui rend la
réparation lisible le jour du déploiement, et la seule chose qui distingue
« le manager attend » de « le manager ne voit rien ».

Le champ ne traverse pas le trait `FlightSource` : `FinishedFlight` (la projection
que `evaluate_flights` consomme) est inchangé, et `evaluator.rs` reste intouché —
il n'a ni à connaître le groom, ni à ouvrir `NotFlightReason`, dont mika#2946 a fait
un enum fermé exigeant une décision écrite pour s'élargir. **Aucune ligne de
`evaluator.rs` dans ce vol.**

## Décision D4 (AC3) — les trois lignes déjà inscrites : geste opérateur nommé, pas de réécriture

AC1 ferme l'avenir : plus aucun verdict ne peut être rendu avant la fin du vol, donc
`write_and_swallow` ne gèlera plus de ligne prématurée. **Aucune ligne de
`flight_verdict_ledger.rs` dans ce vol** — l'idempotence reste ce qu'elle est.

Restent les trois lignes du tableau du ticket (parentes `9bed44f8`, `c5848eaa`,
`9f9be2a1`), inscrites le 2026-10-28 dans `~/.mika/data/mika.db`. Le plan tranche :

**Elles sont supprimées par un geste opérateur explicite, nommé et borné, consigné
dans un runbook — ni migration, ni code de réécriture, ni rattrapage automatique.**

Trois raisons, et la troisième est une contrainte mesurée, pas une préférence :

1. Une migration qui réécrit `audit_events` ferait d'un journal d'audit une table
   mutable par le programme. Le registre a un **écrivain unique** gardé
   (`MANAGER_FLIGHT_VERDICT_TOOL`) ; lui adjoindre un correcteur en fait deux.
2. Un rattrapage automatique demanderait à `write_and_swallow` de savoir *quand* un
   verdict antérieur fut prématuré. Il ne peut pas le savoir sans rejouer la
   topologie des enfants à l'instant de l'écriture — c'est-à-dire sans réimplémenter
   AC1 à l'envers, dans le seul site qui doit rester bête.
3. **La suppression ne provoquera PAS un re-jugement de ces trois vols, et le plan
   le dit plutôt que de le laisser croire.** `milestone_manager/cadence.rs` borne la
   lecture par `let since = checkpoint.and_then(|c| c.last_observed_at.as_deref())`,
   et la requête ordonne sur `COALESCE(completed_at, updated_at)` : ces parentes
   portent un `completed_at` du 2026-10-28 que le retour du pilote n'a pas déplacé.
   Le point de reprise étant passé au-delà, elles ne repasseront pas sous l'œil du
   manager. **Trois lignes fausses deviennent trois absences** — et une absence dit
   « non jugé », ce qui est vrai : le manager n'a jamais jugé correctement ces vols.
   Elle ne dit pas « pas de vol », ce que la ligne actuelle dit faussement.

Le runbook (`docs/operator/`, hors compte de lignes) porte, dans cet ordre : le
`SELECT` de contrôle qui montre les trois lignes et **seulement** elles, le `DELETE`
borné aux trois `target_key` littéraux, le `SELECT` de re-contrôle, et la phrase qui
dit que la concordance de mika#2698 ne remonte pas de trois d'autant — elle cesse
d'être **contredite**, ce qui n'est pas la même chose et ne doit pas être compté comme
tel. Reculer `last_observed_at` pour forcer le re-jugement n'est **pas** proposé : le
cycle rejouerait toute la fenêtre et son rapport, pour trois lignes.

`Tracked in:` — aucun suivi n'est ouvert ici : le geste est complet et terminal, il
n'y a pas de reste.

## Décision D5 — ce qui tient la propriété d'ordonnancement

La propriété « l'enfant d'implémentation existe avant que la parente devienne
terminale » est une dépendance envers `dispatcher.rs`. Elle est épinglée par le test
`mika2951_la_parente_devient_terminale_apres_la_naissance_de_lenfant_dimplementation`,
qui ne rejoue pas le dispatcheur (trop coûteux) mais **documente et asserte l'ordre
des deux appels** dans le bloc de livraison de callback, à la manière de
`mika2875_la_constante_est_la_forme_que_le_producteur_emet`. Si quelqu'un réordonne
le bloc, ce test rougit et nomme ce qui casse — sinon la régression serait muette et
AC1 redeviendrait un pari.

Le filet pour la fenêtre résiduelle (parente terminale, aucun enfant d'implémentation
encore créé) est **l'ajournement lui-même, qui ne s'applique pas** : sans enfant
implement, `pending_implement` est faux, la parente est rendue, et le groom est lu —
donc `not_a_flight`, le classement de mika#2949. C'est un faux négatif borné et
**volontaire** : il ne produit aucune nouvelle fausseté par rapport à aujourd'hui, et
il expire au cycle suivant seulement si le vol se reconduit. Un ajournement
conditionné à `groom_result_convergence(..).is_converged()` serait plus précis mais
**non borné** : un groom convergé dont la relance n'allume jamais (frein STOP, pas de
jeton) resterait ajourné pour toujours — un silence sans fin, la classe que la sonde
D3 existe pour interdire. On choisit le faux négatif borné contre le silence infini,
et on le dit.

---

## Livrables, dans l'ordre de commit

1. **`crates/mika-agent/src/task_engine/engine.rs`** — les deux constantes nommées,
   `PILOT_DISPATCH_LABELS` dérivé, `is_implement_pilot_label`, et le doc-comment qui
   porte D1 (pourquoi pas `dispatch_class`, pourquoi pas l'ordre).
   Étendre `mika2875_…` d'une assertion liant chaque constante à l'outil que
   `build_callback_task` reçoit.
2. **`crates/mika-agent/src/db/tasks.rs`** — requête 2 refondue selon D2, champ
   `pending_implement` sur `FinishedFlightRow`, doc-comment portant la prémisse (a)
   et la propriété d'ordonnancement.
3. **`crates/mika-agent/src/flight_source.rs`** — filtre des lignes ajournées dans
   `finished_implement_flights`, sonde `flight_deferred_pending_implement` (D3),
   mise à jour de `task_to_finished_flight` et de l'aide de test `sans_enfant`.
4. **`crates/mika-agent/src/db/tests/vols_termines_du_manager.rs`** + aide
   `seme_un_groom_puis_un_implement` dans `db/tests/mod.rs` — niveau base.
5. **Tests de bout en bout** dans le `mod tests` de `flight_source.rs` (base →
   source → `evaluate_flights`), sur le modèle de
   `mika2886_la_forme_reelle_rend_un_verdict_de_pr_ouverte`.
6. **`crates/mika-agent/src/task_engine/engine.rs`** (tests) — D5.
7. **Docs** (hors compte) : `docs/agent-internals/milestone-manager-phase-1.md`
   § topologie des vols, et le runbook D4 sous `docs/operator/`.

### Plancher — ce qui doit être vert avant la PR

`cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check`, `make verify-bundled-skills`, puis
`scripts/verify-pipeline.sh origin/main`.

## Tests — chacun nommé par ce qu'il refuse

L'aide `seme_un_groom_puis_un_implement(db, agent, &VolDeGroomRelance { … })` sème la
forme réelle en trois lignes (parente `ready-label`, enfant groom, enfant implement),
les deux enfants bâtis par `crate::skills::executor::build_callback_task` — jamais un
`NewTask` littéral, par la règle que le docstring de `seme_la_forme_reelle` pose déjà.
`set_created_at` rend l'ordre chronologique observable.

| test | ce qu'il refuse | aujourd'hui |
|---|---|---|
| `mika2951_une_parente_dont_limplement_vole_encore_nest_pas_un_vol_termine` | juger avant la fin du vol : parente `completed`, groom terminal `NO_PR`, implement `in_progress` → la source rend **zéro** vol | **rouge** (rend 1 vol, jugé mort) |
| `mika2951_limplement_prime_le_groom_meme_quand_le_groom_est_plus_recent` | le départage par récence : re-groom postérieur à l'implement terminal `PR_OPENED` → verdict `PrOpened { 2947 }` | **rouge** (le groom gagne) |
| `mika2951_une_fois_limplement_terminal_cest_son_issue_qui_est_jugee` | le cas nominal du ticket, bout en bout sur la forme des trois lignes mesurées | rouge |
| `mika2951_un_groom_pur_reste_un_non_vol` (AC2) | la sur-correction : un seul enfant groom → `NotAFlight { GroomCompleted }`, inchangé par mika#2949 | **vert avant et après** — contrôle négatif |
| `mika2951_un_vol_ready_label_a_un_seul_enfant_est_juge_comme_avant` (AC2) | la sur-correction, second front : un seul enfant implement `PR_OPENED` → `PrOpened` | **vert avant et après** — contrôle négatif |
| `mika2951_la_sonde_compte_les_parentes_ajournees` | le retrait silencieux : `pending_implement` vrai au bon cas, faux au cas pur (niveau base) | rouge |
| `mika2951_la_parente_devient_terminale_apres_la_naissance_de_lenfant_dimplementation` (D5) | le réordonnancement muet du bloc de livraison | rouge |

Les deux contrôles négatifs sont dans le **même** fichier que les positifs, comme AC2
l'exige : c'est leur voisinage qui fait qu'on ne peut pas supprimer l'un sans voir
l'autre.

## Fire-Disposition

Les livrables 4, 5 et 6 sont des **détecteurs** (tests dont le chemin de succès est
« aucune violation »), donc la section est due.

**Option retenue : (a) exception nommée en allowlist — et l'allowlist est livrée
VIDE.**

C'est le choix par défaut de mika#1574, et il est ici honnête plutôt que confortable :
**aucune violation ne vit dans le code**. Les trois faussetés mesurées par le ticket
sont trois lignes de `audit_events` dans la base de l'hôte, pas trois sites source ;
elles sont traitées par le geste opérateur D4, qui ne passe par aucun détecteur. Les
sept tests ci-dessus sont donc armés, sans exception, dès le premier `cargo test` — et
cinq d'entre eux rougissent contre `origin/main`, ce qui est la preuve qu'ils
détectent.

Détail d'implémentation de l'allowlist vide : la garde structurelle de D3 — le scan
qui refuse un **second** lecteur de `pending_implement` hors de `flight_source.rs`,
sur le modèle de `mika2425_identity_context_history_has_a_single_reader` — porte un
`const ALLOWLIST: &[&str] = &[]` et une assertion auto-nettoyante
`assert!(ALLOWLIST.is_empty(), "…")` qui rougit le jour où quelqu'un y ajoute une
entrée sans ouvrir le ticket de suivi que l'entrée doit nommer. Son contrôle de bonne
foi (`…_le_scan_rougit_sur_un_second_lecteur`) prouve que le scan attrape encore un
vrai site, faute de quoi une allowlist vide et un scan cassé rendent les mêmes octets.

Aucun détecteur n'est livré désarmé ; aucune halte-et-remontée n'est requise.

## Definition of Done

- `list_finished_implement_flights_in_container` n'annonce plus comme terminé un vol
  dont le pilote d'implémentation est en cours, et lit l'enfant d'implémentation par
  sa classe quand les deux enfants sont terminaux.
- `is_implement_pilot_label` est dérivé de `PILOT_DISPATCH_LABELS` — un seul site de
  vérité pour les deux labels, attesté contre `build_callback_task`.
- `evaluator.rs` et `flight_verdict_ledger.rs` sont **inchangés** (zéro ligne).
- La sonde `flight_deferred_pending_implement` est émise à chaque lecture, y compris
  à zéro, et son absence se lit comme un binaire antérieur.
- Les sept tests passent ; les cinq positifs rougissent contre `origin/main` (vérifié
  par `git stash`-libre : exécution sur la base avant application des livrables 1-3).
- Le runbook du geste D4 existe sous `docs/operator/` et porte ses trois requêtes.
- `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt --check`, `make verify-bundled-skills` et
  `scripts/verify-pipeline.sh origin/main` sont verts.
- Le corps de PR porte `Closes senara-solutions/mika#2951` et
  `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

- **AC1.** Une parente dont un enfant `long_running:run_claude_pilot`
  (implémentation) est encore en vol n'est pas un vol terminé. L'évaluateur ne la juge
  qu'une fois cet enfant terminal, et il lit cet enfant, pas le pilote de groom. Le
  plan tranche par écrit comment l'implémentation se distingue du groom : par le label
  de l'enfant, par son ordre, ou autrement.
- **AC2.** Contrôle négatif dans le même test. Un groom pur, sans pilote
  d'implémentation relancé, reste classé comme mika#2949 le classe (`not_a_flight`).
  Un vol `ready-label` classique, avec un seul enfant pilote, est jugé comme
  aujourd'hui.
- **AC3.** Le registre ne fige plus un verdict rendu avant la fin du vol. Soit AC1
  l'empêche d'être écrit, soit le plan dit comment une ligne prématurée déjà inscrite
  (les trois du tableau) est traitée. Le plan tranche, et ne réécrit pas l'historique
  en silence.
- **AC4.** Sonde après déploiement. Au prochain groom suivi d'un pilote
  d'implémentation, `concordance` montre la parente avec l'issue du pilote
  d'implémentation (PR ouverte et son numéro), et non `PLAN_GROOMED` ni
  `not_a_flight`.

### Où chaque AC est tenu

| AC | tenu par |
|---|---|
| AC1 | D1 (le label, tranché contre `dispatch_class` et contre l'ordre) + D2 + tests 1-3 |
| AC2 | tests 4 et 5, dans le même fichier que les positifs |
| AC3 | D4 — AC1 ferme l'avenir, les trois lignes passées par geste opérateur nommé, sans réécriture, avec la limite de `last_observed_at` dite |
| AC4 | la sonde ci-dessous |

### Sonde AC4 — après `make deploy`

Au prochain groom suivi d'une relance, dans cet ordre :

1. `flight_deferred_pending_implement` apparaît avec `deferred ≥ 1` pendant que le
   pilote d'implémentation vole — **la présence de cette ligne est ce qui atteste que
   le binaire servi porte le correctif** ; son absence signifie un binaire antérieur,
   jamais « rien n'a été ajourné ».
2. Une fois ce pilote terminal, `flight_deferred_pending_implement` retombe à
   `deferred = 0` pour cette parente.
3. `mika flight-verdicts concordance` montre la parente avec **PR ouverte et son
   numéro**, et ni `PLAN_GROOMED` ni `not_a_flight`.
4. Le rapport de cycle et le registre disent la même chose pour cette parente — c'est
   la contradiction du 2026-10-28 à 17:06:59Z / 18:01Z qui doit avoir disparu.

L'instrument n'est pas vivant avant `make deploy` : rien ne lit la source au runtime.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/task_engine/engine.rs` (constantes, prédicat, doc) | 30 |
| `crates/mika-agent/src/db/tasks.rs` (requête 2, `pending_implement`, doc) | 70 |
| `crates/mika-agent/src/flight_source.rs` (filtre, sonde, projection) | 45 |
| `crates/mika-agent/src/db/tests/mod.rs` (aide `seme_un_groom_puis_un_implement`) | 45 |
| tests (`crates/mika-agent/src/db/tests/vols_termines_du_manager.rs`) | 55 |
| tests bout en bout (`mod tests` de `flight_source.rs`) | 110 |
| tests (`crates/mika-agent/src/task_engine/engine.rs`, D5) | 20 |

Total estimé : 375 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 375 < 429 et 375 < 1000 : les deux bornes
passent, et c'est la borne par vol qui mord en premier, donc c'est elle qu'on vise.
Le périmètre est structurellement petit parce que **deux des trois fichiers du ticket
ne sont pas touchés** : `evaluator.rs` garde ses enums fermés et `flight_verdict_ledger.rs`
garde son idempotence — le correctif est entièrement en amont d'eux. Les 155 lignes de
production se concentrent sur un seul bloc de `list_finished_implement_flights_in_container`
et son unique lecteur. Le reste est du test, dont 5 des 7 rougissent contre
`origin/main` : ils mesurent le défaut avant de le fermer.

Découper n'apporterait rien de mergeable seul : livrer le discriminant sans
l'ajournement laisse (a) intact, et livrer l'ajournement sans le discriminant n'a pas
de prédicat. Les deux moitiés d'AC1 sont indissociables.
