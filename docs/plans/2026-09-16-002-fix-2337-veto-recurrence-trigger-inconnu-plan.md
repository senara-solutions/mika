# mika#2337 — le bras existe ; ce qui tient #2334 inert est un veto de 24 h que rien ne lève

> **Rectification du diagnostic.** Le ticket localise la racine en
> `dispatcher.rs:449` (« le bras de match `"qa_review_reconcile"` est absent »)
> et demande de l'ajouter. **Ce bras existe, et il a été introduit par la PR que
> le ticket accuse de l'avoir oublié.** Le fix primaire tel qu'énoncé est un
> no-op. Ce plan documente la preuve, puis traite ce qui maintient réellement
> #2334 inert aujourd'hui — un mécanisme différent, situé ailleurs, et dont
> l'effet dure 24 heures.

## Contexte

mika#2334 / PR #2336 a livré la réconciliation des demandes de revue : un filet
qui rattrape une PR ouverte sans relecteur quand l'événement `pull_request.opened`
s'est perdu. Le 2026-09-16, sa première frappe a échoué
(`unknown run_skill trigger: qa_review_reconcile`, tâche `fb425f89`), et le filet
n'a jamais tourné depuis.

## Ce qui est établi, et comment le vérifier

| # | Fait | Vérification |
|---|---|---|
| **F1** | Le bras `"qa_review_reconcile" => Ok(self.dispatch_qa_review_reconcile(task).await?)` est présent en `dispatcher.rs:447`, **introduit par `1340936e`** — le commit de #2336 lui-même. | `git show 1340936e -- crates/mika-agent/src/task_engine/dispatcher.rs \| grep qa_review_reconcile` |
| **F2** | Une garde le protège déjà, livrée par le même commit : `tests/eval/test_qa_review_reconcile_2334.rs::mika2334_le_scan_est_route_dans_le_dispatcher` (scan de source). Elle est verte. | `cargo test -p mika-agent --test eval mika2334_le_scan_est_route` |
| **F3** | Le littéral d'enregistrement `{"trigger":"qa_review_reconcile"}` est en `server/mod.rs:1678` — même commit, donc **même binaire** que le bras. | `grep -n 'trigger":"qa_review_reconcile' crates/mika-agent/src/server/mod.rs` |
| **F4** | Sur échec de dispatch, `fire_task` (`engine.rs:3832-3838`) émet un `warn!("task dispatch failed")` générique et appelle `update_task_failed`. **Le ré-enfilement n'existe que dans le bras `Ok`** — une récurrence qui échoue n'est jamais replanifiée. | `sed -n '3760,3840p' crates/mika-agent/src/task_engine/engine.rs` |
| **F5** | `create_recurring_task_if_absent` (`db.rs:6158`) porte la garde zombie mika#1742 : une instance `failed`/`cancelled`/`expired` du même label datant de moins de `RECURRING_ZOMBIE_GRACE_HOURS = 24` fait **refuser la ré-inscription** (`Ok(None)`). | `sed -n '6158,6205p' crates/mika-agent/src/db.rs` |
| **F6** | L'unique porte de sortie du veto, `revert_config_cancel_recurring_task` (`db.rs:6278`), ne cible que `status = 'cancelled'`. Elle **ne lève jamais** un veto né d'un `failed`. | `sed -n '6278,6296p' crates/mika-agent/src/db.rs` |

### Deux conséquences qui redressent le ticket

**(a) Il n'y a pas de spam.** Le ticket annonce « va échouer à CHAQUE tick →
spam de tâches failed ». F4 dit l'inverse : la récurrence n'est pas replanifiée
après un échec. La ligne meurt **une fois**, puis se tait. Il n'y a pas de bruit
à faire cesser — il y a un silence à lever, ce qui est plus difficile à
remarquer et c'est pourquoi le filet est resté mort.

**(b) C'est le veto, pas le bras, qui tient #2334 inert.** La mort en `failed` a
armé la garde mika#1742 pour 24 h (F5), et aucun chemin ne la désarme (F6).
**Tout redémarrage pendant cette fenêtre — y compris avec le binaire correct —
refuse de ré-enregistrer la récurrence.** Le remède naturel (redéployer, relancer)
est précisément ce que l'état actuel neutralise.

### Ce qui reste non tranché, et pourquoi ça ne bloque pas

F1+F3 disent qu'aucun binaire contenant `1340936e` ne peut émettre cette erreur :
le nom enregistré et le bras sont deux littéraux du même binaire. Le process qui
a tiré exécutait donc du code antérieur à `1340936e`, tandis que la ligne
`registered recurring task` — émise uniquement à la **création effective** de la
ligne (`task_engine/mod.rs:94`) — vient d'un process qui, lui, la connaissait.
Une table `tasks` partagée, lue par deux binaires de versions différentes :
redémarrage ultérieur sur un binaire ancien, ou second process concurrent.

**Départager les deux demande les journaux, inaccessibles depuis le sandbox de
grooming — et n'est pas nécessaire ici** : les deux hypothèses partagent la même
racine (le code mergé n'était pas le code en exécution), elle est hors de ce
dépôt, et aucun des volets ci-dessous n'en dépend. Elle part en suivi (§ Hors
périmètre).

## Décisions

**D1 — Ne pas ré-appliquer le fix primaire.** F1 et F2 le rendent inutile.
Ré-écrire le bras produirait au mieux un diff vide, au pire un doublon de bras
inatteignable.

**D2 — Écarter le repli proposé par le ticket** (« retirer la récurrence tant
que le trigger n'est pas câblé »). Il est motivé par un spam qui n'existe pas
(a), et son coût est exactement la panne à réparer : le ticket le reconnaît
lui-même (« #2334 reste inert »). Désarmer un filet parce qu'il a échoué une
fois, c'est confondre le filet avec la chute.

**D3 — Lever le veto pour la classe « trigger inconnu », et pour elle seule.**
mika#1742 existe pour empêcher le ré-armement en boucle d'une récurrence qui
tue le système. Pour cette classe, **la mort ne prédit pas la mort suivante** :
le binaire qui ré-enregistre contient, par construction, le bras correspondant
au nom qu'il écrit (F1+F3, rendu structurel par V3). C'est le cas exact où la
garde est un faux positif qui *prolonge* la panne au lieu de la contenir.
mika#1742 n'est pas désarmée en général.

**D4 — La classe vient de la variante, jamais d'un substring.** `DispatchError`
n'a aujourd'hui que `AgentBusy` et `Other`, et le trigger inconnu se fond dans
`Other` via `anyhow!`. Reconnaître la classe en comparant le message rendu
serait exactement ce que la doctrine du dépôt interdit (cf. les classes d'erreur
de mika#2179, dérivées de la variante par `downcast_ref`).

**D5 — La levée est à usage unique.** Le marqueur qui désarme le veto est
consommé comme l'est `config_cancel_reverted` (mika#2271) : une **seconde** mort
par trigger inconnu sur le même label dans la fenêtre laisse le veto s'appliquer
normalement. Sans ce bornage, une boucle réelle s'auto-absoudrait indéfiniment
et V2 rouvrirait le trou que mika#1742 avait fermé.

**D6 — La sonde du ticket est étendue à la classe, pas écrite pour un cas.** F2
montre qu'une garde par trigger existe déjà et qu'elle n'a rien empêché : elle
est verte depuis le début. Une garde nominative ne ferme rien pour le *prochain*
trigger ; elle doit énumérer les sites d'enregistrement.

## Volets d'implémentation

### V1 — Rectification (aucun code)

Consigner F1–F6 dans le corps de PR. C'est le livrable qui empêche le prochain
lecteur de #2337 de ré-appliquer un no-op.

### V2 — Le veto ne survit plus à un trigger inconnu (cœur du hotfix)

1. **`dispatcher.rs`** — ajouter `DispatchError::UnknownTrigger { trigger: String }`
   et la renvoyer depuis le catch-all de `dispatch_run_skill` au lieu de
   `anyhow!(...)` (D4). Vérifier les sites qui `match`ent `DispatchError` : le
   comportement de `AgentBusy` ne doit pas bouger.
2. **`engine.rs::fire_task`** — extraire la nouvelle variante avant le bras
   `else` générique. Sur cette variante :
   - marquer la mort en metadata sous une clé dédiée (miroir de
     `RECURRING_CONFIG_CANCEL_REVERTED_PATH`), **uniquement** si
     `trigger_type == recurring` ;
   - émettre un WARN nommé `recurring_unknown_trigger` (et non le
     `task dispatch failed` générique, indiscernable d'un échec réseau) portant
     `label`, `trigger`, `task_id` ;
   - écrire un `audit_events` avec `tool_name = 'recurring_unknown_trigger'` ;
   - conserver `update_task_failed` — l'état terminal est correct, c'est sa
     *conséquence* sur la ré-inscription qu'on corrige.
3. **`db.rs::create_recurring_task_if_absent`** — la requête `dead_sibling`
   exclut déjà les lignes portant le marqueur mika#2271 via
   `NOT (json_valid(metadata) AND COALESCE(json_extract(metadata, ?4), 0) = 1)`.
   Étendre l'exclusion à la nouvelle clé, et **consommer le marqueur** au moment
   où il est honoré (D5).
4. **Remise en service** — après déploiement, la première ré-inscription doit
   réussir sans attendre l'expiration des 24 h.

### V3 — La sonde, générique (répond au test négatif obligatoire)

1. **Garde de classe** : extraire tous les littéraux `{"trigger":"X"}` des sites
   d'enregistrement (`server/mod.rs`, `task_engine/mod.rs::FEEDER_CONFIG`) et
   asserter que chaque `X` a un bras `"X" =>` dans `dispatch_run_skill`. Un
   trigger enregistré sans bras échoue le test en nommant le trigger. Ceci
   **subsume** `mika2334_le_scan_est_route_dans_le_dispatcher` (F2) pour la
   moitié « routage » ; garder l'assertion sur l'appel effectif au scan, qu'une
   garde générique ne peut pas voir.
2. **Sonde de tir** (demande explicite du ticket) : faire tirer la récurrence
   `qa_review_reconcile` par le moteur et asserter que la tâche **ne finit pas
   `failed` sur le motif du catch-all**. Elle s'arrête à la **résolution** :
   l'exécution du scan appelle `gh`, hors de portée d'un test hermétique.
3. **Non-régression du veto** : deux tests jumeaux sur
   `create_recurring_task_if_absent` — une mort marquée « trigger inconnu »
   n'arme pas le veto ; une mort ordinaire l'arme toujours (D3) ; une **seconde**
   mort marquée dans la fenêtre l'arme (D5).

**Portée honnête de V3 :** ces gardes ferment la divergence **intra-binaire** —
celle que le ticket croit avoir observée. **Aucune n'aurait attrapé l'incident
réel**, qui est un décalage entre le code mergé et le code en exécution : les
deux littéraux étaient cohérents dans le source, et le test F2 était vert
pendant toute la panne. La sonde qui manque pour cette classe-là appartient au
suivi ci-dessous.

## Verification contract

```bash
cargo fmt --check && cargo clippy -p mika-agent --all-targets -- -D warnings
cargo test -p mika-agent task_engine::           # dispatcher + engine
cargo test -p mika-agent db::                    # garde zombie mika#1742
cargo test -p mika-agent --test eval mika2334    # non-régression #2334/#2336
cargo test -p mika-agent --test eval mika2337    # sondes V3
```

**Sondes post-déploiement.**

- **Remise en service** — au premier démarrage après déploiement, la récurrence
  doit se ré-enregistrer *sans* attendre la fenêtre de 24 h :
  `grep 'registered recurring task' $MIKA_SPIRIT_LOG_FILE | grep qa_review_reconcile`,
  et **absence** de `mika#1742: refusing to re-register` pour ce label.
- **Effectivité de #2334** — au tick suivant :
  `grep qa_review_reconcile_tick $MIKA_SPIRIT_LOG_FILE`. Zéro action produit
  zéro ligne (doctrine mika#2131) ; le signal d'un scan qui *tourne* est
  l'absence de `failed` sur sa ligne récurrente, pas la présence d'une ligne.
- **La classe redevient audible** —
  `SELECT * FROM audit_events WHERE tool_name = 'recurring_unknown_trigger';`
  **doit rester vide** en régime nominal. Toute ligne nomme un trigger enregistré
  qu'un binaire en exécution ne connaît pas, c'est-à-dire un décalage de version
  — et non un défaut de ce code.
- **Halte** — si `unknown run_skill trigger` réapparaît après déploiement, **ne
  pas retoucher le dispatcher** : F1 rend cette hypothèse intenable. C'est la
  version du binaire en exécution qu'il faut établir (suivi ci-dessous).

## Definition of Done

- `DispatchError::UnknownTrigger` existe ; le catch-all la renvoie ; aucun site
  ne reconnaît la classe par substring.
- Une mort de récurrence par trigger inconnu est marquée, nommée (WARN dédié) et
  auditée.
- Le veto mika#1742 ne s'applique plus à une première mort ainsi marquée, et
  s'applique toujours à tout le reste, seconde mort marquée comprise.
- La garde générique triggers-enregistrés ↔ bras-du-match est en place et
  échoue en nommant le trigger fautif.
- La sonde de tir de `qa_review_reconcile` existe et n'exige aucun réseau.
- `cargo fmt`, `clippy -D warnings` et la suite `mika-agent` passent.
- Le corps de PR porte la rectification V1 avec ses preuves.

## Acceptance criteria

Le ticket n'a pas de section `## Acceptance criteria` formelle ; ces critères
sont dérivés de son corps (fix primaire, repli, test négatif obligatoire) et des
faits F1–F6.

- **AC1** — Le plan et le corps de PR établissent, preuve `git` à l'appui, que le
  bras `"qa_review_reconcile"` existe depuis `1340936e` et que le fix primaire
  du ticket est un no-op. Aucun bras dupliqué n'est introduit.
- **AC2** — Le repli du ticket (retirer la récurrence) n'est pas appliqué, et le
  refus est motivé dans la PR.
- **AC3** — Une tâche récurrente dont le trigger est inconnu produit une erreur
  d'une **variante dédiée**, un WARN portant un nom d'événement propre, et une
  ligne `audit_events` — et non plus un `task dispatch failed` générique.
- **AC4** — Après une telle mort, un redémarrage ré-enregistre la récurrence
  **sans attendre** `RECURRING_ZOMBIE_GRACE_HOURS`. Test à l'appui.
- **AC5** — Une mort de récurrence **pour toute autre cause** arme toujours le
  veto mika#1742. Test à l'appui.
- **AC6** — Une **seconde** mort par trigger inconnu sur le même label dans la
  fenêtre arme le veto (la levée est à usage unique). Test à l'appui.
- **AC7** — Un test échoue, en nommant le trigger, si un trigger est enregistré
  aux sites d'enregistrement sans bras correspondant dans `dispatch_run_skill` —
  pour **tout** trigger, pas seulement `qa_review_reconcile`.
- **AC8** — Un test exerce le tir de la récurrence `qa_review_reconcile` et
  asserte qu'elle ne finit pas `failed` sur le motif du catch-all, sans réseau.
- **AC9** — La portée des sondes est écrite : elles ferment la divergence
  intra-binaire et n'auraient pas attrapé l'incident du 2026-09-16.

## Hors périmètre (suivi à ouvrir)

- **La racine de l'incident : pourquoi un process exécutait du code antérieur à
  `1340936e`.** C'est la vraie cause, elle n'est pas dans ce code, et ce plan ne
  la corrige pas — il rend seulement sa conséquence réparable par un
  redémarrage. Le suivi doit établir laquelle des deux hypothèses tient
  (redémarrage sur binaire ancien / second process concurrent sur la même base)
  et instrumenter la classe : rendre lisible au démarrage la version du code
  réellement en exécution, de sorte qu'un DEPLOYED≠EFFECTIVE soit constatable
  sans reconstituer une chronologie de journaux. C'est la sonde que V3, par
  construction, ne peut pas fournir.
- **La fenêtre de 24 h elle-même** (`RECURRING_ZOMBIE_GRACE_HOURS`) et
  l'absence de surface opérateur listant les récurrences actuellement sous veto.
  Réels, plus larges que ce hotfix, et sans effet sur #2334 une fois V2 livré.
