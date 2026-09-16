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
4. **Remise en service — non couverte par le code de ce volet.** La ligne
   `failed` de `fb425f89` est morte *avant* que le marquage existe : son
   `metadata` ne porte pas la nouvelle clé, donc `COALESCE(json_extract(...), 0)`
   rend `0` et elle reste un `dead_sibling` pour la garde. V2 rend réparable par
   redémarrage **toute mort future** ; il ne débloque pas rétroactivement celle
   qui a causé #2337. Le geste de remise en service est un geste opérateur —
   voir § Fire-Disposition, « La violation préexistante ».

### V3 — La sonde, générique (répond au test négatif obligatoire)

1. **Garde de classe** : asserter que chaque trigger **enregistré** a un bras
   `"X" =>` dans `dispatch_run_skill`, en nommant le trigger fautif. Le prédicat
   s'ancre sur **l'appelant** — l'argument `action_config` des appels à
   `task_engine::ensure_recurring_task`, seul enregistreur de récurrences
   `run_skill` (il pose `action_type::RUN_SKILL`) — et **non** sur la forme
   textuelle `{"trigger":"X"}`, qui a trois faux membres dans l'arbre (voir
   § Fire-Disposition, V3.1). Ceci **subsume**
   `mika2334_le_scan_est_route_dans_le_dispatcher` (F2) pour la moitié
   « routage » ; garder l'assertion sur l'appel effectif au scan, qu'une garde
   générique ne peut pas voir.
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

## Fire-Disposition

Ce plan porte trois livrables de classe détecteur (V3.1 garde de classe, V3.2
sonde de tir, V3.3 tests jumeaux du veto), **et** il crée une exception à un
détecteur préexistant : la garde zombie mika#1742, dont le chemin de succès est
« aucune récurrence zombie ne se ré-arme ». Les quatre sont traités ci-dessous.
La disposition est écrite sur une population **comptée dans l'arbre à
`9f28342b`**, pas supposée — et la mesure a contredit le plan sur deux points,
reportés en V3.1 et dans « La violation préexistante ».

### V3.1 — garde de classe : zéro violation, et un prédicat qu'il faut borner

**Population.** Six appels à `task_engine::ensure_recurring_task`, tous dans
`server/mod.rs` (`:1583` heartbeat, `:1595` reflection, `:1618`
auto_pull_groomed, `:1640` wip_rescue, `:1674` qa_review_reconcile, `:1693`
curator_review). Les six ont un bras dans `dispatch_run_skill`
(`dispatcher.rs:441-447`). **Violations existantes : zéro.**

**Disposition : (a) allowlist nommée, avec une liste vide.** La garde est
bloquante dès le land, sans exemption ni période de grâce, parce qu'il n'y a
rien à exempter. **Aucune exemption n'est écrite** : exempter d'un scan ce qui
le passe déjà crée une dispense morte que plus rien ne nettoie — la dette même
que le sous-point (3) de l'option (a) cherche à éviter.

**Ce que la mesure corrige dans V3.** Le prédicat textuel que V3.1 proposait
d'abord — extraire les littéraux `{"trigger":"X"}` — a **trois faux membres**
dans l'arbre, et l'un d'eux ferait fire la garde au land :

| Site | Littéral | Statut |
|---|---|---|
| `engine.rs:4467` | `{"trigger":"callback"}` | **Firerait à tort.** Helper `#[cfg(test)] make_callback_task`, `action_type = "resume_agent"` : ce n'est pas un `run_skill`, il n'a donc légitimement aucun bras. |
| `task_engine/mod.rs:182` | `FEEDER_CONFIG` | Constante sous `#[cfg(test)] mod tests` — le plan la citait comme site d'enregistrement de production. Elle passe par coïncidence (`auto_pull_groomed` a un bras). |
| `research/mechanism_analyzer.rs:789` | fixture de journal JSON | Passe par coïncidence (`heartbeat` a un bras). |

Deux des trois passent **par chance**, ce qui est le pire cas : une garde qui
tient sur une coïncidence est verte jusqu'au jour où elle accuse un site
innocent. D'où l'ancrage sur l'appelant plutôt que sur la forme — le prédicat
désigne alors exactement la population qu'il prétend garder.

**Si le scan fire malgré tout : (c) halt-and-surface.** Si le poseur découvre en
écrivant la garde un septième enregistrement hors des six sites recensés, ou un
trigger enregistré sans bras, il **s'arrête et remonte à l'opérateur** au lieu
d'ajouter une exemption ou de câbler le bras lui-même : un trigger enregistré
sans destinataire est précisément l'incident de ce ticket, et sa résolution est
une décision de périmètre, pas un geste de poseur.

### V3.2 / V3.3 — sondes de comportement : le gate est N/A, et c'est dit

Ni la sonde de tir (V3.2) ni les tests jumeaux du veto (V3.3) ne s'exécutent sur
des données préexistantes : chacun fabrique ses lignes dans une base en mémoire.
Il n'existe donc **aucune population à exempter** et le gate ne les concerne pas
— énoncé ici plutôt que tu, pour qu'un lecteur ne prenne pas le silence pour un
oubli (arbre de décision du gate, branche 3).

### La levée du veto — (a) exception nommée, mono-use, auto-nettoyante

V2 crée une exception à mika#1742. Elle est écrite sous l'option **(a) named
allowlist exception**, dont les trois sous-points sont honorés ainsi :

1. **La donnée qui déclenche l'exception est nommée** — non par une valeur
   codée en dur (« le label `qa_review_reconcile` »), qui serait une dispense
   permanente accordée à un nom, mais par un **marqueur posé au moment de la
   mort**, sous une clé metadata dédiée, miroir de
   `RECURRING_CONFIG_CANCEL_REVERTED_PATH` (mika#2271). Seule une mort dont la
   cause est `DispatchError::UnknownTrigger` — la variante, jamais un substring
   (D4) — porte le marqueur. Toute autre cause de mort reste sous veto (D3).
2. **Le suivi est référencé** — l'exception ne couvre pas la racine, qui est le
   décalage entre code mergé et code en exécution ; elle est explicitement
   adossée au suivi § Hors périmètre, premier item. Le veto levé est un faux
   positif *de la garde*, pas un défaut absous.
3. **L'exception se nettoie elle-même, et c'est un test qui le prouve.** Le
   marqueur est **consommé** au moment où il est honoré : la requête
   `dead_sibling` étend son exclusion existante
   `NOT (json_valid(metadata) AND COALESCE(json_extract(metadata, ?) , 0) = 1)`
   à la nouvelle clé, et la ligne est ré-écrite sans le marqueur lors de la
   ré-inscription. Une **seconde** mort marquée sur le même label dans la
   fenêtre de 24 h retrouve donc un veto armé (D5) — assertion portée par le
   troisième test jumeau de V3.3, qui est l'assertion self-cleaning au sens du
   gate : elle rougit le jour où la levée cesserait d'être à usage unique,
   c'est-à-dire le jour où l'exception deviendrait la dispense permanente que
   mika#1742 existe pour empêcher.

**Ce que l'exception ne couvre délibérément pas.** Une boucle réelle — un
binaire qui ré-enregistre en continu un trigger qu'il ne sait pas router —
consomme sa levée au premier tour et se retrouve sous veto au second. La levée
achète **un** redémarrage, pas une immunité.

### La violation préexistante : la ligne `failed` qui a causé #2337 — (c) halt-and-surface

C'est ici que le gate mord, et la réponse n'est pas celle que V2 supposait. La
tâche `fb425f89` est morte le 2026-09-16 **avant** que le code de marquage
existe ; son `metadata` ne porte pas la nouvelle clé, `COALESCE(json_extract(…),
0)` rend `0`, et elle reste un `dead_sibling` au sens de `db.rs:6163`. **Déployer
V2 ne lève pas le veto né de cette mort-là.**

**Disposition : (c) halt-and-surface.** Aucune correction rétroactive n'est
écrite — ni migration marquant la ligne, ni exemption sur le label. Raisons :
l'effet à corriger **s'évapore de lui-même** à l'expiration de la fenêtre de
24 h, et une structure permanente pour une ligne unique et périssable est une
dette qui survivrait à son objet ; une exemption sur le label, elle, absoudrait
aussi les morts *futures* de ce label quelle qu'en soit la cause, ce qui est
exactement le contraire de D3.

Le geste est donc opérateur et borné à deux issues, au choix : attendre
l'expiration de la fenêtre puis redémarrer, ou marquer/supprimer la ligne
`failed` à la main avant de redémarrer. La sonde post-déploiement « Remise en
service » ci-dessous est ce qui rend l'état lisible — un
`mika#1742: refusing to re-register` sur `qa_review_reconcile` après déploiement
n'est **pas** une régression de V2 : c'est cette violation préexistante, et le
distinguer d'un échec de V2 est précisément ce que cette section permet.

## Verification contract

```bash
cargo fmt --check && cargo clippy -p mika-agent --all-targets -- -D warnings
cargo test -p mika-agent task_engine::           # dispatcher + engine
cargo test -p mika-agent db::                    # garde zombie mika#1742
cargo test -p mika-agent --test eval mika2334    # non-régression #2334/#2336
cargo test -p mika-agent --test eval mika2337    # sondes V3
```

**Sondes post-déploiement.**

- **Remise en service** —
  `grep 'registered recurring task' $MIKA_SPIRIT_LOG_FILE | grep qa_review_reconcile`.
  **Lire le résultat avec la § Fire-Disposition en main** : un
  `mika#1742: refusing to re-register` sur ce label au premier démarrage n'est
  pas un échec de V2 mais la violation préexistante (la mort de `fb425f89` ne
  porte pas le marqueur), et il appelle le geste opérateur qui y est décrit —
  attendre l'expiration de la fenêtre, ou nettoyer la ligne à la main. La
  propriété que V2 doit tenir se vérifie sur la **mort suivante**, pas sur
  celle-ci ; son test hermétique est AC4.
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
- Le corps de PR porte la rectification V1 avec ses preuves, **et** le geste de
  remise en service de #2334 (§ Fire-Disposition, « La violation préexistante »)
  — sans quoi le hotfix land vert en laissant #2334 inert.

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
- **AC4** — Après une telle mort **survenue sous le binaire porteur du marquage**,
  un redémarrage ré-enregistre la récurrence **sans attendre**
  `RECURRING_ZOMBIE_GRACE_HOURS`. Test à l'appui. La restriction n'affaiblit pas
  le critère, elle le rend vrai : une mort antérieure au marquage ne porte pas le
  marqueur et reste sous veto (§ Fire-Disposition, « La violation préexistante »).
- **AC5** — Une mort de récurrence **pour toute autre cause** arme toujours le
  veto mika#1742. Test à l'appui.
- **AC6** — Une **seconde** mort par trigger inconnu sur le même label dans la
  fenêtre arme le veto (la levée est à usage unique). Test à l'appui.
- **AC7** — Un test échoue, en nommant le trigger, si un trigger est enregistré
  sans bras correspondant dans `dispatch_run_skill` — pour **tout** trigger, pas
  seulement `qa_review_reconcile`. Sa population est celle des appels à
  `ensure_recurring_task` ; il est **vert au land** et ne fire sur aucun des
  trois littéraux `{"trigger":…}` qui ne sont pas des enregistrements
  (§ Fire-Disposition, V3.1).
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
  Réels et plus larges que ce hotfix. La § Fire-Disposition renforce ce suivi
  plutôt qu'elle ne l'épuise : c'est faute d'une telle surface que la remise en
  service de #2334 reste un geste manuel (« halt-and-surface ») au lieu d'être
  constatable en une commande.

## Revision history

- rev 2 (2026-09-16) : addressed F1 (BLOCKING) en ajoutant la section
  `## Fire-Disposition`, écrite sur une population **comptée** dans l'arbre à
  `9f28342b` plutôt que supposée. Elle nomme l'option (a) pour la levée du veto
  (marqueur mono-use issu de la variante `UnknownTrigger`, consommé à l'usage,
  assertion auto-nettoyante portée par le troisième test jumeau de V3.3), comme
  le demandait le (b) du finding. Le comptage a en outre contredit le plan sur
  deux points, corrigés dans le corps : (i) le prédicat de la garde V3.1 est
  ancré sur les appels à `ensure_recurring_task` et non sur la forme textuelle
  `{"trigger":"X"}`, qui a trois faux membres dont un ferait fire la garde au
  land (`engine.rs:4467`, helper de test `action_type = resume_agent`) — le plan
  citait à tort `task_engine/mod.rs::FEEDER_CONFIG`, constante sous `#[cfg(test)]`,
  comme site d'enregistrement ; (ii) V2.4 annonçait une remise en service que le
  code ne produit pas — la ligne `failed` de `fb425f89` est morte avant le
  marquage, reste `dead_sibling`, et sa levée est un geste opérateur
  (halt-and-surface). AC4 et AC7 précisés en conséquence, sonde post-déploiement
  « Remise en service » rectifiée, DoD étendu au geste de remise en service.
