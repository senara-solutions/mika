# fix(task_engine) — l'auto-fire après groom sort en silence quand la parente n'a pas de `reference_url`

**Ticket :** senara-solutions/mika#3140 · labels `bug`, `p2-normal`, `dispatch:loop`
**Voir aussi :** mika#3118 (vol 2 en attente), mika#3141 (`owner_repo` au lecteur de consentement), mika#1614 (l'auto-fire lui-même), mika#996 (réutilisation de la parente)

---

## Constat mesuré

`try_dispatch_pilot_after_groom_success` (`crates/mika-agent/src/task_engine/dispatcher.rs:5659`)
est le site de l'auto-fire moteur après un groom réussi. Il traverse six préconditions
avant d'atteindre le frein STOP (étape 3bis) et le lecteur de consentement (étape 4bis),
tous deux instrumentés depuis mika#2498 et mika#3118. Les préconditions, elles, **sortent
par un `return` nu** :

```rust
// 1. Groom-class callbacks only.
if task.dispatch_class.as_deref() != Some("groom") { return; }
// 2. marqueur de succès canonique
match &task.result { Some(r) if …is_converged() => {}, _ => return };
// 3. parent avec URL d'issue parsable
let parent_id = match &task.parent_task_id { Some(id) => id.clone(), None => return };
let parent = match db.get_task_unscoped(&parent_id).await { Ok(Some(t)) => t, _ => return };
let reference_url = match parent.reference_url.as_deref() {
    Some(url) => url.to_string(),
    None => return,                                  // ← le défaut du 2026-11-22
};
let IssueCoordinates { .. } = match parse_repo_issue_from_url(&reference_url) {
    Some(parsed) => parsed, None => return,
};
```

Deux conséquences mesurées, les deux sur le groom de #3119 :

1. **2026-11-22** — la parente `a099d8af` n'avait pas de `reference_url` (elle l'avait pour
   #3117 et #3118, tâches `e702fd30`, parce que le champ est un **input facultatif du
   modèle** dans `tools/create_task.rs:115`). L'auto-fire est sorti à l'étape 3 : aucun
   `event =`, aucune ligne d'`audit_events`, et le frein STOP n'a jamais été atteint. Rien
   dans les logs ne distingue « la parente n'avait pas d'URL » de « l'auto-fire n'a pas
   tourné » ni de « le binaire servi est antérieur à mika#1614 » (classe mika#2205).
2. **2026-11-22 19:50Z** — la preuve `PLAN_GROOMED` est cherchée par
   `parent.reference_url` dans le `JOIN` de `has_completed_groom_for_issue`
   (`db/tasks.rs:4557`). Faute de cette URL, zéro ligne ⇒ `Absent` ⇒
   `markers_without_proof` ⇒ **second groom** : 48 tours, 3,58 $, plan réécrit.

Le ticket demande deux choses, et elles n'ont pas la même nature :

- **(A)** chaque précondition échouée émet un événement nommé — un défaut
  d'**observabilité** au site qui échoue ;
- **(B)** la parente d'un groom porte toujours l'URL de son ticket, posée par le moteur et
  non par le tour — un défaut de **production de donnée**, en amont, à un autre site.

---

## Périmètre

### Vol courant (ce plan) — volet (A) : les six sorties de précondition se nomment

L'auto-fire sort toujours sur la même décision, mais il la **dit**, une fois par callback de
classe `groom`, avec un dénominateur. Rien du volet (B) n'est touché : aucun écrivain de
`reference_url` n'est ajouté, aucune migration, aucune ligne de base existante modifiée.

### Hors périmètre, nommé

- **Volet (B) — le moteur pose l'URL de la parente.** Le matériau existe déjà :
  `validate_dispatch_readiness` (`skills/executor.rs:3849`, 43 sites d'appel, traversé par
  **tout** dispatch) dérive déjà le couple `(owner/repo, numéro)` du prompt de dispatch à
  ses lignes 4526-4566, et c'est la position que la doctrine du lecteur unique désigne
  (mika#2158, mika#3049, et le précédent exact de `reread_consent_before_rearm` :
  *« la garde vit ici, en position 3, et non aux trois sites appelants »*). Ce que le vol 2
  doit livrer en plus : un écrivain DB qui **n'existe pas** (aucun
  `update_task_reference_url` / `set_reference_url` dans le crate — vérifié), nécessairement
  conditionnel en SQL (`WHERE id = ?1 AND reference_url IS NULL`) pour ne jamais écraser, et
  une **disposition nommée pour la collision** sur l'index partiel unique
  `idx_tasks_manual_active_ref_url` (`db/migrations.rs:2500` : `(agent_id, reference_url)`
  sur les tâches `manual` non terminales) — poser l'URL peut violer l'unicité quand une
  autre tâche manuelle active du même agent porte déjà ce ticket, et une pose qui échoue ne
  doit pas faire échouer le dispatch.
  **Tracked in:** senara-solutions/mika#3140
- **L'étape 4 (token GitHub absent) et les étapes 5a–5g.** L'étape 4 `warn!` déjà sous un nom
  (`"no GitHub token configured"`) mais n'écrit **pas** de valeur d'audit ; les étapes 5a–5g
  `warn!` de même sans valeur d'audit. Le constat du ticket — *« sans événement ni ligne
  d'audit »* — ne leur est vrai qu'à moitié : il leur manque la ligne d'audit, pas
  l'événement. Défaut de second ordre, renvoyé au vol 2 avec le volet (B) plutôt que glissé
  dans celui-ci pour le passer sous la borne.
  **Tracked in:** senara-solutions/mika#3140
- **L'étape 1 (`dispatch_class != "groom"`) reste muette, délibérément.** Ce n'est pas une
  précondition échouée du groom : c'est le discriminant qui dit si la fonction a affaire à un
  groom. Elle est franchie à **chaque** callback de l'agent, toutes classes confondues ;
  l'instrumenter noierait la population des cinq refus réels sous celle des callbacks
  `implement`, exactement ce que mika#2205 refuse. C'est la seule sortie nue qui survit au
  vol, et elle est portée comme **exception nommée** (§ *Fire-Disposition*).
- **Le second groom lui-même** (la conséquence 2 du commentaire) n'est pas corrigé ici : sa
  cause est l'URL absente, donc le volet (B). Ce vol la rend **visible** — c'est sa
  contribution à la conséquence 2, et c'est tout ce qu'il prétend.
- `mika#3118` vol 2 (moitié halte-de-plan) et `mika#3141` (`owner_repo` au lecteur) sont
  déjà tranchés ailleurs : ce plan ne les rejuge pas.

---

## Requirements

- **R1 — Six préconditions, six noms, aucun fondu.** Les sorties des étapes 2 et 3 se
  répartissent en six issues distinctes, chacune portant sa propre valeur d'audit :
  marqueur non convergé · `parent_task_id` absent · ligne parente introuvable · lecture de
  la parente en erreur · `reference_url` absente · `reference_url` non parsable. Un
  `GROUP BY after_value` sur `audit_events` sépare les six, aucune n'étant déductible par
  soustraction (mika#2205). En particulier **« parente introuvable » et « lecture en
  erreur » ne sont pas la même panne** — une parente supprimée et une base injoignable sont
  aujourd'hui fondues par le `_ => return` du `get_task_unscoped`.
- **R2 — Un dénominateur, émis sur toutes les branches y compris la saine.** Une ligne INFO
  `groom_autofire_preconditions_checked`, **une et une seule par callback de classe
  `groom`**, portant le verdict de la traversée — les six refus **et** `held`. Sans la
  branche saine, un recensement qui ne compte que les anomalies n'a pas de dénominateur
  (mika#3050) et « aucun refus » ne se distingue pas de « aucun auto-fire » (mika#2205).
- **R3 — Une absence est absente, jamais `""` ni `0`.** Sur la ligne de recensement :
  `reason` est présent **seulement** sur la branche « lecture en erreur » (elle seule porte
  un texte) ; `parent_task_id` est **absent** quand le callback n'en déclare pas ;
  `repo`/`issue` sont **absents** tant que l'URL n'a pas été parsée. Motif mika#2331, déjà
  tenu par `AutofireConsent::reason() -> Option<&str>` (`dispatcher.rs:5548`) — ce vol en est
  le frère exact.
- **R4 — Le `before_value` ne mentit pas.** Les cinq refus postérieurs au marqueur portent
  `GROOM_PILOT_BEFORE_VALUE` (`"groom_delivered"`). Le refus « marqueur non convergé » ne
  peut pas le porter : à cet instant le groom n'est pas délivré. Il reçoit son propre
  `before_value`.
- **R5 — Un seul site d'émission.** La ligne de recensement et la ligne d'audit s'écrivent
  à **un** endroit du corps de la fonction, pas à six. Six copies d'un couple
  (sonde, audit), c'est six endroits où un futur éditeur peut en oublier un, avec toutes
  les assertions vertes (doctrine du lecteur unique, mika#2158 / mika#2201 / mika#3049).
- **R6 — Le comportement observable ne change pas.** Aucune précondition ne devient
  passante, aucune ne devient bloquante, l'ordre des étapes est préservé, et les étapes
  3bis / 4bis / 5a–5j sont inchangées. Ce vol ajoute de la parole, pas une décision.
- **R7 — Le site qui échoue ne peut plus redevenir muet.** Un détecteur refuse l'ajout d'une
  sortie nue dans le corps de la fonction (§ *Fire-Disposition*). Un test comportemental ne
  le couvre pas : un futur éditeur qui ajoute une septième précondition avec un `return` nu
  laisse les sept tests existants verts — c'est exactement la classe du défaut d'aujourd'hui.

---

## Design

### D1 — `AutofirePrecondition`, frère exact d'`AutofireConsent`

Un enum local à `dispatcher.rs`, posé à côté d'`AutofireConsent` (`:5500`-`:5580`) dont il
copie la forme, parce que cette forme est déjà celle que le fichier enseigne :

```rust
enum AutofirePrecondition {
    MarkerNotConverged,
    ParentIdAbsent,
    ParentRowMissing,
    ParentLookupFailed(String),
    ReferenceUrlAbsent,
    ReferenceUrlUnparsable,
}
```

Trois accesseurs, un `match` chacun :

| méthode | rend | note |
|---|---|---|
| `verdict() -> &'static str` | le mot du recensement | `marker_not_converged`, `parent_id_absent`, `parent_row_missing`, `parent_lookup_failed`, `reference_url_absent`, `reference_url_unparsable` |
| `reason() -> Option<&str>` | `Some` sur `ParentLookupFailed` seul, `None` ailleurs | R3 |
| `audit_value() -> &'static str` | la valeur d'audit (D2) | R1 |
| `before_value() -> &'static str` | `GROOM_PILOT_CALLBACK_BEFORE_VALUE` sur `MarkerNotConverged`, `GROOM_PILOT_BEFORE_VALUE` ailleurs | R4 |

Le `match` est **exhaustif sans bras `_`** : une septième précondition ne compile pas sans
nommer sa valeur d'audit. C'est le même procédé que le `match &consent` de l'étape 4bis
(`:5869`-`:5874`) et que la garde d'exhaustivité de `GroomVerdictState`
(`canonical_tokens.rs`, ~`:4716`).

### D2 — Sept constantes d'audit, sous le tool existant

Sous `GROOM_PILOT_DISPATCHER_TOOL` (`= "task_engine_groom_pilot_dispatcher"`, `:5463`), à la
suite des six valeurs existantes (`groom_delivered`, `implement_dispatched`,
`stopped_by_sentinel`, `ready_absent_at_autofire`, `issue_closed_at_autofire`,
`consent_unreadable_at_autofire`) :

| constante | valeur |
|---|---|
| `GROOM_PILOT_CALLBACK_BEFORE_VALUE` | `groom_callback_completed` |
| `GROOM_PILOT_NOT_CONVERGED_VALUE` | `marker_not_converged` |
| `GROOM_PILOT_PARENT_ID_ABSENT_VALUE` | `parent_task_id_absent` |
| `GROOM_PILOT_PARENT_MISSING_VALUE` | `parent_task_row_missing` |
| `GROOM_PILOT_PARENT_LOOKUP_FAILED_VALUE` | `parent_task_lookup_failed` |
| `GROOM_PILOT_REF_URL_ABSENT_VALUE` | `parent_reference_url_absent` |
| `GROOM_PILOT_REF_URL_UNPARSABLE_VALUE` | `parent_reference_url_unparsable` |

Après le vol, un `GROUP BY after_value` sépare **onze** issues de l'auto-fire, aucune
inférée par différence. Chaque constante porte un doc-comment disant ce qu'elle **n'est
pas** — la convention déjà en place à `:5486` et `:5497`.

### D3 — Une fonction de traversée, un site d'émission

Les étapes 2 et 3 migrent, telles quelles, dans une fonction privée qui rend le matériau de
la suite ou la précondition qui a échoué :

```rust
async fn check_autofire_preconditions(
    db: &AsyncDatabase,
    task: &Task,
) -> Result<(String, Task, String, IssueCoordinates), AutofirePrecondition>
```

— soit `(parent_id, parent, reference_url, coordonnées)`, soit la précondition. L'appelant
garde l'étape 1 inchangée, puis :

```rust
let (parent_id, parent, reference_url, coords) =
    match check_autofire_preconditions(db, task).await {
        Ok(ready) => { info!(event = "groom_autofire_preconditions_checked",
                             verdict = "held", …); ready }
        Err(p)    => return refuse_on_precondition(db, task, p).await,
    };
```

`refuse_on_precondition` est le **site unique** de R5 : il émet la ligne de recensement avec
`verdict = p.verdict()` et `reason = p.reason()`, puis la ligne d'audit via
`db.log_audit_event(&system_session, GROOM_PILOT_DISPATCHER_TOOL, entity_id,
Some(p.before_value()), Some(p.audit_value()), Some(&reason), Some(&trace_id))`.

Deux détails que le site existant impose et qu'un éditeur ne peut pas deviner :

- La session système est aujourd'hui `format!("system-{}", parent.agent_id)` (`:5757`).
  Sur trois des six branches la parente n'est pas lisible — la session vient donc de
  `task.agent_id`, qui est le même agent par construction (un callback et sa parente ne
  changent pas d'agent) et la seule valeur disponible sur toutes les branches.
- L'`entity_id` est `parent_id` quand il est connu, à défaut `task.id` ; la `reason` de la
  ligne d'audit **nomme toujours** le callback (`callback=<task.id>`), afin qu'une ligne
  d'entité « callback » reste rattachable.

La ligne de recensement est émise sur les deux bras — `held` dans le `Ok`, le verdict du
refus dans `refuse_on_precondition` — donc exactement une fois par callback de classe
`groom`, jamais zéro, jamais deux (R2).

### D4 — Pourquoi le vol émet là où un commentaire existant dit de ne pas émettre

Le doc-comment de l'étape 2 (`:5670`-`:5676`) argumente contre une émission à cet endroit :
*« Aucune émission ici, délibérément : … lui donner un compteur mélangerait sa population
avec celle de la porte (R6) »*. Cette contrainte est **respectée, et non contournée** : ce
que le vol ajoute n'est pas un compteur de la porte de mika#2590 R5/R6, c'est une sonde de
**précondition**, sous un nom d'événement distinct
(`groom_autofire_preconditions_checked`, jamais `grooming_marker_*`) et avec une valeur
d'audit qui nomme la précondition (`marker_not_converged`), pas la position du marqueur.
Les deux populations restent séparables par la valeur, qui est précisément ce que mika#2205
exige. Le doc-comment de l'étape 2 est **mis à jour** dans le même commit pour dire cela —
sans quoi le prochain éditeur lira la contrainte d'avant et défera le vol de bonne foi.

### D5 — Le détecteur : une sortie nue dans ce corps est refusée

Un scan structurel dans `canonical_tokens.rs`, à côté de ses frères
(`mika3118_le_site_dengagement_du_consentement_est_unique`, ~`:7983` ;
`lignes_dappel_en_production("try_dispatch_pilot_after_groom_success")`,
`engine.rs:11838`-`11907`) :

`mika3140_les_sorties_de_precondition_de_lautofire_sont_nommees` lit le source de
`dispatcher.rs`, délimite le corps de `try_dispatch_pilot_after_groom_success`, et compte
les `return;` nus (hors commentaires, hors lignes de test — la classification passe par
`crate::source_scan::is_test_source_path`, jamais par une troncature au premier
`#[cfg(test)]` ni par une allowlist de fichiers). Le compte attendu est **1**, l'exception
nommée du § *Fire-Disposition*. Son contrôle négatif —
`…_le_scan_rougit_sur_une_seconde_sortie_nue` — mesure la détection sur une entrée
fabriquée, de sorte que le détecteur reste attesté comme voyant même quand le dépôt est
propre (motif mika#3132).

---

## Décision réservée à l'opérateur (vol 2, nommée ici, non pratiquée)

Le volet (B) porte une décision que ce plan **ne tranche pas** (mika#3122) : faut-il
**rétro-poser** `reference_url` sur les parentes déjà créées sans elle — au premier chef
`a099d8af`, la parente du premier groom de #3119 ? Les options, chiffrées :

1. **Ne rien rétro-poser.** Coût : les groms déjà passés restent invisibles à
   `has_completed_groom_for_issue`, donc un `ready` sur l'un de ces tickets peut router vers
   un second groom, au tarif mesuré (48 tours, 3,58 $). Nombre de lignes concernées : inconnu
   sans requête sur la base de production, qui n'est pas accessible depuis un bac à sable de
   pilote.
2. **Rétro-poser par migration.** Coût : une migration qui écrit dans `tasks` des lignes
   historiques, en devinant l'URL depuis le `label` ou l'`action_config` du dispatch —
   donc une inférence sur des données que le moteur n'a pas produites, et une exposition à
   l'index partiel unique sur chaque ligne touchée.
3. **Rétro-poser à la main, sur la seule ligne nommée par le ticket.** Coût : un geste
   opérateur, hors dépôt, non reproductible, mais borné et vérifiable.

Aucune n'est pratiquée. Le vol courant ne touche aucune ligne existante.

---

## Ordre de commit

1. **C1** — les sept constantes d'audit et `AutofirePrecondition` (+ ses quatre accesseurs).
   Aucun appelant : le comportement est inchangé, `cargo clippy` passe.
2. **C2** — `check_autofire_preconditions` + `refuse_on_precondition` + la réécriture du
   corps de `try_dispatch_pilot_after_groom_success` aux étapes 2 et 3, et la mise à jour du
   doc-comment de l'étape 2 (D4). **Plancher du vol :** à ce commit le défaut est corrigé et
   observable ; les six sorties se nomment, le dénominateur existe, rien d'autre n'a bougé.
3. **C3** — les tests comportementaux : six branches attestées sur la ligne d'audit, plus le
   contrôle positif `held`.
4. **C4** — le scan structurel et son contrôle négatif.

Le vol est mergeable seul : il ne dépend d'aucun livrable de mika#3118 vol 2 ni du volet (B),
et ne crée aucune dette qu'un autre vol devrait reprendre pour compiler.

---

## Verification Contract

| # | ce qui est vérifié | comment |
|---|---|---|
| V1 | une `reference_url` absente sur la parente écrit `parent_reference_url_absent` | test : callback `groom` convergé, parente sans `reference_url` ⇒ une ligne `audit_events` avec `after_value = "parent_reference_url_absent"` et `before_value = "groom_delivered"` |
| V2 | les cinq autres refus écrivent chacun leur valeur, et aucune autre | un test par branche : marqueur non convergé, `parent_task_id` absent, ligne parente absente, URL non parsable — chacun assertant la valeur **et** l'absence des cinq autres |
| V3 | « parente introuvable » ≠ « lecture en erreur » | les deux tests portent des `after_value` distincts ; l'assertion croisée est ce qui tient R1 |
| V4 | le dénominateur est émis sur la branche saine | test : parente avec URL parsable ⇒ la traversée rend `Ok` et la ligne `verdict = "held"` est émise (capture `tracing` ou assertion sur le passage à l'étape 3bis inchangée) |
| V5 | une seule ligne de recensement par callback | assertion de cardinalité : un callback ⇒ exactement une ligne d'audit de la famille précondition, zéro sur le chemin dispatché |
| V6 | `before_value` ne mentit pas sur la branche non convergée | V2 assertant `before_value = "groom_callback_completed"` sur ce seul cas |
| V7 | aucun changement de comportement | les 15 sites de test existants de `try_dispatch_pilot_after_groom_success` (`:10555`, `:10578`, `:10615`, `:10643`, `:10872`, `:10927`, `:10967`, `:11001`, `:11040`, `:11134`, `:11197`, `:11228`, `:12000`, `:12030`, `:12253`) passent sans modification de leurs assertions existantes |
| V8 | une septième sortie nue est refusée | `mika3140_les_sorties_de_precondition_de_lautofire_sont_nommees` rouge sur un `return;` ajouté ; son contrôle négatif prouve qu'il voit |
| V9 | pipeline | `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills`, `bash scripts/verify-pipeline.sh origin/main` |

**Sonde post-déploiement.** `groom_autofire_preconditions_checked` n'est pas vivante avant
le prochain `make deploy` : l'**absence** de la ligne signifie un binaire antérieur au vol,
jamais une absence de refus (mika#2340). Lecture, après déploiement :
`GROUP BY verdict` sur la sonde donne la répartition des six refus et de `held` ; le
dénominateur est leur somme, et c'est le nombre de callbacks de classe `groom` du jour.

---

## Definition of Done

- `AutofirePrecondition` et ses sept constantes d'audit existent, chaque constante avec un
  doc-comment disant ce qu'elle n'est pas.
- Les six sorties des étapes 2 et 3 passent par `refuse_on_precondition`, site unique.
- La ligne `groom_autofire_preconditions_checked` est émise exactement une fois par callback
  de classe `groom`, branche saine comprise, avec `reason`, `parent_task_id`, `repo` et
  `issue` **absents** quand le fait n'existe pas.
- Le doc-comment de l'étape 2 dit pourquoi l'émission de ce vol ne mélange pas les
  populations de mika#2590 R5/R6.
- Les sept tests comportementaux et le scan structurel + son contrôle négatif sont verts.
- Les 15 sites de test existants passent sans amendement de leurs assertions.
- `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`,
  `make verify-bundled-skills` et `bash scripts/verify-pipeline.sh origin/main` sont verts.
- L'étape 1 reste la seule sortie nue, et son exception est nommée en allowlist avec son
  assertion de cardinalité.
- Le corps de la PR nomme le volet (B) et la décision réservée par une ligne
  `Tracked in: senara-solutions/mika#3140`.

---

## Acceptance criteria

> Le corps de mika#3140 ne porte pas de section `## Acceptance criteria`. Les critères
> ci-dessous sont dérivés des Requirements et du Verification Contract.

- **AC1** — Un callback de classe `groom`, marqueur convergé, dont la parente n'a pas de
  `reference_url`, produit **une** ligne `audit_events` sous
  `tool_name = "task_engine_groom_pilot_dispatcher"` avec
  `after_value = "parent_reference_url_absent"` et `before_value = "groom_delivered"`.
  C'est le cas littéral du 2026-11-22 sur `a099d8af`.
- **AC2** — Les six préconditions produisent six `after_value` distincts ; un
  `GROUP BY after_value` sur `audit_events` les sépare sans qu'aucune soit obtenue par
  soustraction, et « ligne parente absente » n'est pas fondue avec « lecture en erreur ».
- **AC3** — La ligne `groom_autofire_preconditions_checked` est émise sur **sept** branches
  (six refus + `held`), exactement une fois par callback de classe `groom`. Un test de
  cardinalité le tient.
- **AC4** — Sur cette ligne, `reason` est présent sur la seule branche
  `parent_lookup_failed` ; `parent_task_id` est absent quand le callback n'en déclare pas ;
  `repo` et `issue` sont absents tant que l'URL n'a pas été parsée. Jamais `""`, jamais `0`.
- **AC5** — Le refus « marqueur non convergé » porte
  `before_value = "groom_callback_completed"` et non `"groom_delivered"`.
- **AC6** — Aucun comportement observable ne change : les 15 sites de test existants de
  `try_dispatch_pilot_after_groom_success` passent sans amendement de leurs assertions, et
  les étapes 3bis, 4bis et 5a–5j sont inchangées.
- **AC7** — `mika3140_les_sorties_de_precondition_de_lautofire_sont_nommees` compte
  exactement une sortie nue (l'étape 1), rougit quand une seconde apparaît, et rougit aussi
  quand celle-ci disparaît ; son contrôle négatif atteste qu'il voit sur une entrée
  fabriquée.
- **AC8** — `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`,
  `make verify-bundled-skills` et `bash scripts/verify-pipeline.sh origin/main` sont verts.
- **AC9** — Le volet (B) — le moteur pose `reference_url` à
  `validate_dispatch_readiness`, avec son écrivain DB conditionnel et sa disposition de
  collision sur `idx_tasks_manual_active_ref_url` — et la décision de rétro-pose sont
  nommés dans le plan et dans le corps de la PR par `Tracked in: senara-solutions/mika#3140`,
  et **aucune ligne de ce vol ne les pratique**.

---

## Fire-Disposition

Le vol livre un détecteur : le scan structurel
`mika3140_les_sorties_de_precondition_de_lautofire_sont_nommees` (D5), dont le chemin de
succès est « aucune sortie nue non nommée ».

**Option retenue : (a) exception nommée en allowlist.**

La population d'exception est **mesurée à exactement une** entrée, et elle n'est pas un
retard : c'est la décision du § *Périmètre*.

```rust
/// L'unique sortie nue tolérée dans le corps de `try_dispatch_pilot_after_groom_success`.
///
/// Donnée précise : le `return` de l'étape 1, `task.dispatch_class != Some("groom")`.
/// Décision : mika#3140 — ce n'est pas une précondition échouée du groom mais le
/// discriminant de pertinence, franchi à chaque callback de l'agent toutes classes
/// confondues ; l'instrumenter noierait la population des six refus sous celle des
/// callbacks `implement` (mika#2205).
/// Ce n'est donc PAS un suivi à armer : l'exception est définitive et argumentée.
const SORTIES_NUES_TOLEREES: usize = 1;
```

**Assertion auto-nettoyante** : le scan assertit l'**égalité** `== SORTIES_NUES_TOLEREES`,
pas une inégalité. Il rougit donc dans les deux sens — quand une septième précondition
arrive avec un `return` nu (la régression), **et** quand l'étape 1 cesse d'en être une
(l'exception devient stale et doit être retirée du même geste). Une borne supérieure
(`<= 1`) laisserait l'exception survivre à sa raison d'être, ce qui est exactement
l'allowlist que mika#2309 condamne.

Aucun ticket de suivi n'est ouvert pour cette entrée, et c'est délibéré : un ticket de suivi
nomme un travail à faire, et il n'y en a pas — l'entrée référence la **décision** qui la
crée (mika#3140, § *Périmètre*), ce qui est ce qui la rend relisible et contestable. Le
précédent est `mika3118_le_site_dengagement_du_consentement_est_unique`, dont la population
d'exception était vide et mesurée vide par une assertion de cardinalité, sans ticket.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/task_engine/dispatcher.rs` — 7 constantes d'audit + `AutofirePrecondition` et ses 4 accesseurs + `check_autofire_preconditions` + `refuse_on_precondition` + réécriture des étapes 2-3 + doc-comment de l'étape 2 | 145 |
| tests dans `dispatcher.rs` — 6 branches attestées sur `audit_events` + contrôle positif `held` + assertion de cardinalité | 80 |
| `crates/mika-agent/src/canonical_tokens.rs` — scan structurel `mika3140_*`, sa constante d'exception et son contrôle négatif | 40 |

Total estimé : 265 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et pourquoi il tient.** 265 < 279, et le périmètre est volontairement le
volet (A) seul. Le volet (B) — écrivain DB absent à créer, site d'écriture dans
`validate_dispatch_readiness` (43 sites d'appel), disposition de collision sur l'index
partiel unique, et ses tests — a été estimé à ~220 lignes supplémentaires : la somme (~485)
dépasse la borne de 74 %, donc le découpage est en **vols** et non en phases d'un même vol,
chacun mergeable seul. Le vol courant repasse sous la borne ; la suite est nommée par les
lignes `Tracked in: senara-solutions/mika#3140` du § *Périmètre*.

L'ordre des deux vols n'est pas arbitraire : sans les événements nommés du volet (A), rien
ne permettrait de mesurer, après le volet (B), si la pose de l'URL par le moteur a
effectivement vidé la branche `parent_reference_url_absent`. Le volet (A) est l'instrument
du volet (B).
