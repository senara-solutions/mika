# Plan : le balayage phantom mesure l'âge de la ligne, jamais la vie du travail (mika#2156)

**Ticket :** mika issue#2156 — `fix(task_engine): le balayage phantom déclare mortes les sessions qui durent — 2 h 08 après `phantom_aged_out`, le pilote écrivait toujours`
**Labels :** `bug`, `p1-important`
**Type :** issue (bug — casseur de boucle : l'état enregistré contredit le travail réel, et c'est ce tableau de bord qui pilote le feeder)
**Palier de priorité :** Tier 1 — *casse la boucle*. Le compte des dispatchs en vol devient illisible ; 177 balayages historiques portent la même forme.
**Fichiers principaux :** `crates/mika-agent/src/task_engine/engine.rs`, `crates/mika-agent/src/db.rs`, `crates/mika-agent/src/async_db.rs`, `crates/mika-common/src/config.rs`

---

## Problème

Le balayage sélectionne ses candidats ainsi (`crates/mika-agent/src/db.rs:7211-7215`) :

```sql
WHERE agent_id = ?1
  AND action_type = 'none'
  AND process_id IS NULL
  AND status IN ('in_progress', 'blocked')
  AND updated_at <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
```

puis marque chaque ligne retenue `failed` / `phantom_aged_out`
(`crates/mika-agent/src/task_engine/engine.rs:1112`).

Les trois premiers critères sont satisfaits **par construction** pour toute ligne de suivi
saine : une ligne `ready-label:` est délibérément `action_type='none'` avec `process_id IS NULL`,
parce que le processus réel vit dans la ligne de rappel séparée. Le seul discriminant restant
est l'âge.

Et l'âge ne mesure pas ce que le balayeur croit mesurer. **`updated_at` de la ligne de suivi
n'est jamais rafraîchi pendant que le dispatch travaille** (M7) : le seuil mesure « temps écoulé
depuis la dernière écriture sur cette ligne », pas « temps écoulé depuis le dernier signe de vie
du travail ». Ces deux quantités ne coïncident que si le travail est mort.

C'est pourquoi la réparation du moteur a aggravé le symptôme au lieu de le résoudre : depuis
mika#2146 (montage gitdir) et claude-pilot#147 (watchdog d'inactivité), les pilotes vont au bout
de leur pipeline, et 8 % des dispatchs sains dépassent désormais le seuil d'une heure (M6). Le
balayeur déclare mortes exactement les sessions qui marchent enfin.

## Mesures — exécutées le 2026-09-03, contre `~/.mika/data/mika.db`

### M1 — le lien entre les deux lignes existe déjà en base, et il est direct

```
            id: 6663a9be-e01a-4e6c-a460-0ade6a5b56f9
         label: ready-label: senara-solutions/mika#2151
   action_type: none          status: failed         process_id: (vide)
parent_task_id: (vide)        result: phantom_aged_out
    updated_at: 2026-09-03T18:30:01Z

            id: 800d739f-a0ed-485d-bef1-9990beeac396
         label: long_running:run_claude_pilot
   action_type: resume_agent  status: pending        process_id: 365667
parent_task_id: 6663a9be-e01a-4e6c-a460-0ade6a5b56f9
```

`parent_task_id` de la ligne de rappel **pointe exactement** sur la ligne de suivi. Aucun nouveau
champ, aucune migration de schéma n'est nécessaire pour rattacher les deux : le lien que l'AC1
demande de consulter est déjà écrit, le balayeur ne le lit simplement pas.

### M2 — le piège : l'existence du rappel n'est PAS le discriminant

```sql
SELECT count(*) FROM tasks WHERE result='phantom_aged_out';                      -- 181
SELECT count(DISTINCT p.id) FROM tasks p
  JOIN tasks c ON c.parent_task_id = p.id AND c.process_id IS NOT NULL
  WHERE p.result='phantom_aged_out';                                             -- 177
```

**177 des 181 balayages historiques avaient un enfant de rappel portant un `process_id`.** Un
prédicat écrit naïvement — « épargner toute ligne de suivi ayant une ligne de rappel avec un
`process_id` » — désarmerait donc le balayeur à 98 %, et l'AC3 tomberait au premier tir.

Le discriminant que l'AC1 nomme est **la vivacité mesurée du processus**, pas la présence de la
ligne. Cette mesure est la raison d'être du plan : elle dit où l'erreur naturelle se loge.

### M3 — les statuts que porte une ligne de rappel avec `process_id`

```
delivered  1146
cancelled    12
completed     2
failed        1
pending       1   ← 800d739f, le cas mesuré du ticket
```

Une seule ligne `pending` avec `process_id` dans toute la base : celle du ticket. Le statut
non-terminal est donc un signal — mais **le plan ne s'en sert pas comme filtre**, voir la
Décision de conception : il n'est pas prouvé qu'un enfant `delivered` implique un processus mort,
et la vivacité tranche les deux cas sans avoir à trancher cette question.

### M4 — le garde anti-réutilisation de PID est déjà provisionné

```sql
SELECT metadata FROM tasks WHERE id LIKE '800d739f%';
-- {"process_start_time":"52755192"}
```

La ligne de rappel porte déjà `metadata.process_start_time` (champ 22 de `/proc/<pid>/stat`).
Le couple `(pid, start_time)` identifie une instance de processus, pas seulement un numéro.

### M5 — la fonction de vivacité existe, est testée, et prend exactement ce couple

`crates/mika-agent/src/task_engine/process_liveness.rs:51`

```rust
pub fn is_same_process_alive(pid: u32, expected_start_time: u64) -> bool
```

Elle fait `kill(pid, 0)` puis compare le `starttime` lu dans `/proc/<pid>/stat`, et rend `false`
sur PID absent, PID réutilisé, `/proc` illisible, ou plateforme non-Linux. Quatre tests unitaires
la couvrent déjà (`read_start_time_of_self`, `read_start_time_of_nonexistent_pid`,
`is_same_process_alive_self`, `is_same_process_alive_wrong_start_time`).

**Le correctif est une composition de pièces existantes, pas de la machinerie neuve.**

### M6 — les durées réelles, qui sont le chiffre de l'AC5

Lignes `long_running:run_claude_pilot*` hors `_groom`, créées depuis le 2026-08-25, n = 1307 :

| p50 | p90 | p95 | p99 | max |
|---|---|---|---|---|
| 311 s | 3221 s | 4276 s | 10066 s | 21659 s |

Le seuil actuel de **3600 s tombe entre p90 et p95** : environ 8 % des dispatchs sains le
dépassent (100 sur 1441 toutes formes confondues). Segmenté :

| label | n | moy | max | > 1 h |
|---|---|---|---|---|
| `run_claude_pilot` | 539 | 660 s | 21659 s | 25 |
| `run_claude_pilot:deferred` | 768 | 1152 s | 20501 s | 75 |
| `run_claude_pilot_groom` | 134 | 372 s | 1216 s | 0 |

Second terme à couvrir : **l'attente de créneau**. Entre la création de la ligne de suivi et
celle de sa ligne de rappel, il n'existe aucun enfant à interroger, et le seuil s'applique nu.
Mesuré : `6663a9be` créée à 16:46, son rappel à 17:29 — **43 min** d'attente. `cf26783d`
(mika#2140) a attendu 3172 s bloquée sur `global_dispatch_active`.

Somme des deux termes au p99 : 10066 + 3172 ≈ 13238 s.

### M7 — `updated_at` de la ligne de suivi n'est jamais rafraîchi

```
d0913636  ready-label: mika#1772  in_progress  created 21:24:34  updated 21:24:35   (+1 s)
73a4913d  ready-label: mika#2127  in_progress  created 21:10:08  updated 21:10:09   (+1 s)
```

Deux lignes en cours de dispatch, `updated_at` figé à une seconde après la création. C'est la
cause profonde énoncée dans le Problème, mesurée plutôt que déduite.

### M8 — l'ascendance réelle du pilote (réponse à F2 de la première passe)

L'architecte a refusé la prémisse que D5 avançait sans preuve. Mesure directe, sur le pilote du
ticket **encore vivant au moment du grooming** (PID 365667, `etimes=14903` — 4 h 08) :

```
PID     PPID     PGID     SID       COMMAND
365667  3649327  365667   3649327   run.sh          ← le pilote
366201  365667   365667   3649327   bwrap
3649327 3649326  3649327  3649327   mika-spirit     ← le moteur
3649326 1        3636914  3636914   supervise-daemon
```

**La prémisse que j'avais écrite était fausse** : le pilote *est* un enfant direct du moteur.
Ce n'est pas ce qui le fait survivre.

Ce qui le fait survivre est visible sur la colonne `PGID` : `run.sh` porte **son propre PGID
(365667)**, distinct de celui du moteur (3649327). C'est délibéré —
`crates/mika-agent/src/skills/executor.rs:2962` :

```rust
.process_group(0); // Make child a process group leader (#855)
```

Conséquences, qui sont celles qui comptent pour D5 :

1. un signal de groupe adressé au moteur (`kill -TERM -3649327`) **n'atteint pas** le pilote ;
2. `supervise-daemon` vit dans un troisième groupe encore (3636914) et redémarre le moteur sans
   toucher au groupe du pilote ;
3. à la mort du moteur, `run.sh` est réparenté à PID 1 et continue.

**Le pilote survit donc bien au redémarrage du moteur — mais parce qu'il est son propre leader de
groupe, pas parce qu'il ne serait pas un enfant.** D5 tient ; sa justification est réécrite.

### M9 — le doc-comment du balayage au démarrage affirme le contraire de M7

`crates/mika-agent/src/task_engine/engine.rs:1226-1233` justifie l'agressivité de `age=0` ainsi :

> *a legitimate long-running manual tracking row would have `updated_at` bumped by any
> `update_task_status` write within `MIKA_PHANTOM_SWEEP_AGE_SECONDS`* […]
> *any phantom-shape row present at startup outlived a prior process*

Les deux propositions sont contredites par la mesure : M7 montre que `updated_at` n'est jamais
rafraîchi pendant le dispatch, et M8 montre qu'une ligne présente au démarrage peut parfaitement
porter un processus toujours vivant.

**D5 n'est donc pas du poids mort — c'est le chemin où le défaut est le plus aigu**, puisque
`age=0` ne laisse aucune marge d'âge pour l'absorber.

### Contrôles de la mesure

- **Positif** — `grep -rn "is_same_process_alive" crates --include='*.rs'` → 6 occurrences dans
  `process_liveness.rs` (la recherche discrimine).
- **Négatif** — `grep -rn "ZorglubXYZ" crates --include='*.rs'` → 0.
- **Vivacité du cas du ticket** — `/proc/365667` présent au moment de la mesure, `ELAPSED=12181 s`,
  descendance `bwrap → sh → claude-pilot → claude` complète.

---

## Décision de conception

### D-1 — le garde vit côté application, la requête SQL reste intacte

`find_phantom_tracking_tasks` est documentée « SOLE READER » pour deux appelants
(`sweep_null_pid_phantoms` à age=3600, `startup_recovery` step 2b à age=0). La vivacité d'un PID
n'est pas exprimable en SQL. Le plan garde donc la requête comme **sélecteur de candidats** et
insère une étape de filtrage entre la sélection et `update_task_failed` — un seul point de
changement, partagé par les deux appelants, sans toucher au contrat documenté de la requête.

### D-2 — on ne filtre PAS sur le statut de l'enfant

M3 montre que 1146 enfants sont `delivered`. Il serait tentant de traiter `delivered` comme
terminal et de ne tester la vivacité que sur `pending`/`in_progress`. **Le plan refuse cette
optimisation** : rien dans le code ne prouve qu'un rappel `delivered` implique un processus mort,
et M2 montre que se tromper ici coûte 98 % du balayeur dans un sens ou un vrai positif dans
l'autre. Le prédicat retenu est donc : *tout enfant portant un `process_id`* est soumis au test
de vivacité. Le coût est un `kill(pid, 0)` par candidat — négligeable devant un balayage qui
tourne au tick.

### D-3 — `metadata.process_start_time` absent ⇒ on balaie

Si l'enfant porte un `process_id` mais pas de `process_start_time` exploitable (ligne ancienne,
metadata malformée), le garde ne peut pas écarter la réutilisation de PID. Disposition retenue :
**traiter comme mort et balayer**, en journalisant le motif. C'est le comportement d'avant le
correctif — le garde n'a alors rien ajouté, mais il n'a rien cassé non plus. Fail-vers-l'ancien.

### D-4 — AC5 : relever à 14400 s, et écrire la mesure à côté de la constante

L'AC5 demande un réexamen et une raison écrite, pas un chiffre imposé. Position du plan :
**`DEFAULT_PHANTOM_SWEEP_AGE_SECONDS = 14400`** (4 h), justifié par M6 : p99 de dispatch
(10066 s) + attente de créneau mesurée (3172 s) ≈ 13238 s, arrondi au-dessus.

Le raisonnement qui rend ce chiffre sûr est que **le seuil n'est plus le discriminant principal**.
Avec le garde D-1 en place, une ligne dont le travail vit est épargnée quel que soit son âge ; le
seuil ne décide plus que du sort des lignes dont aucun processus ne répond — c'est-à-dire des
vraies orphelines, pour lesquelles attendre 4 h au lieu d'1 h ne coûte qu'un délai de nettoyage.
Allonger le seuil sans le garde aurait été un pansement ; avec le garde, c'est le bon ordre de
grandeur pour un filet de second rang.

---

## Fire-Disposition — ce qui se passe quand le nouveau garde tire

Pré-spécifié avant l'implémentation, pour que le premier tir ne demande pas d'arbitrage.

- **Le garde épargne une ligne (chemin AC2).** Une ligne `in_progress` survit au-delà du seuil.
  C'est le comportement voulu. Elle sera terminalisée par le chemin normal de fin de dispatch
  (livraison du callback) ou, si son processus meurt sans le faire, par le balayage suivant —
  puisque le test de vivacité rendra alors `false`. **Aucune intervention.**
- **Le garde n'épargne rien pendant 24 h.** Signifie que tous les candidats avaient des processus
  morts — c'est-à-dire que le balayeur d'avant était correct sur cette population. Lecture
  attendue au vu de M2/M3 (1 cas vivant sur 181). **Non-événement, pas une régression.**
- **Le garde épargne massivement (> 20 % des candidats sur une passe).** Contredirait M3. Lecture :
  soit `delivered` coexiste effectivement avec des processus vivants (D-2 devient porteur et non
  prudentiel), soit des PID sont réutilisés sans que `process_start_time` les écarte. **Halte et
  remontée** : ne pas ajuster le seuil, remonter la mesure sur le ticket.
- **Le seuil relevé laisse traîner des orphelines.** Visible comme des lignes `in_progress`
  anciennes sans enfant vivant. **Non bloquant** : le balayage les prend à 4 h au lieu d'1 h.
- **Marche arrière sans redéploiement.** `MIKA_PHANTOM_SWEEP_AGE_SECONDS` (`config.rs:954`)
  surcharge déjà la constante par variable d'environnement : ramener le seuil à `3600` ne demande
  pas de rebuild. Le garde D3, lui, n'a pas d'interrupteur — c'est délibéré : un garde qui ne fait
  que *retenir* une transition destructive n'a pas de mode dégradé utile, et D-3 le fait déjà
  échouer vers l'ancien comportement à chaque incertitude.

---

## Deliverables

### D1 — un helper DB qui rend les enfants de dispatch d'une ligne de suivi *(AC1)*

`crates/mika-agent/src/db.rs` — à côté de `find_phantom_tracking_tasks`, pour que le lecteur
trouve le garde là où il trouve la requête qu'il garde.

```rust
/// Les enfants de rappel d'une ligne de suivi qui portent un `process_id`.
///
/// Compagnon de [`Self::find_phantom_tracking_tasks`] (mika#2156) : la requête
/// de balayage ne sait pas distinguer « ligne de suivi orpheline » de « ligne
/// de suivi dont le dispatch dure », parce que les trois critères non-temporels
/// sont satisfaits par construction. Le lien manquant est `parent_task_id`, déjà
/// présent en base.
///
/// Ne filtre PAS sur le statut de l'enfant : voir D-2 du plan mika#2156 —
/// 1146 des 1147 enfants porteurs de PID sont `delivered`, et rien ne prouve
/// que `delivered` implique un processus mort. La vivacité tranche.
pub fn find_dispatch_children_with_pid(
    &self,
    parent_task_id: &str,
) -> Result<Vec<DispatchChild>>
```

`DispatchChild { id: String, process_id: i64, process_start_time: Option<u64> }`, le
`process_start_time` étant extrait de `metadata` en JSON (M4).

### D2 — le miroir async *(AC1)*

`crates/mika-agent/src/async_db.rs` — wrapper `find_dispatch_children_with_pid`, sur le modèle
exact de `find_phantom_tracking_tasks` (l.990) : même forme, `with_db`, pas de scoping agent
supplémentaire (la ligne parente est déjà scopée par la requête amont).

### D3 — le garde dans la boucle de balayage *(AC1, AC4)*

`crates/mika-agent/src/task_engine/engine.rs`, dans `sweep_null_pid_phantoms`, entre l'itération
`for row in phantoms` et l'appel à `update_task_failed`. Une fonction privée
`live_dispatch_child(&self, parent_id: &str) -> Option<(String, i64)>` rendant
`(child_id, pid)` du premier enfant vivant, pour que la même logique serve les deux appelants
sans duplication.

Comportement :
- enfant trouvé, `process_start_time` présent, `is_same_process_alive(pid, start)` → **épargner**,
  journaliser (D4), `continue` sans incrémenter `swept_count` ;
- enfant trouvé, pas de `process_start_time` exploitable → **balayer** (D-3), journaliser le motif ;
- aucun enfant, ou tous morts → **balayer** (comportement actuel inchangé, AC3) ;
- erreur DB sur le lookup enfant → **balayer**, `warn!`. Fail-vers-l'ancien : le garde est une
  courtoisie, jamais une précondition — même posture que `tracking_cleanup.rs` (« fail-open by
  construction »).

### D4 — la décision d'épargne est lisible après coup *(AC4)*

Un `info!` structuré au moment de l'épargne, portant l'identifiant du rappel et le `process_id`
retenu, comme l'AC4 l'exige littéralement :

```rust
info!(
    event = "phantom_sweep_spared",
    task_id = %row.id,
    child_task_id = %child_id,
    process_id = pid,
    age_seconds,
    "phantom_sweep: tracking row spared — dispatch child process is alive"
);
```

Plus un compteur `spared_count` remonté dans la ligne `phantom_sweep_complete` existante, à côté
de `swept_count` et `error_count`, pour que la passe se lise d'un seul champ.

### D5 — le même garde sur le chemin de démarrage *(AC1, AC3)*

`sweep_null_pid_phantoms_at_startup` (`engine.rs:1233`) appelle la même requête à `age=0`. Le
garde s'y applique via la même fonction privée que D3 — un seul prédicat, deux appelants.

**Justification, réécrite sur M8** (la première rédaction avançait une prémisse fausse, relevée
par l'architecte en première passe) : le pilote est bien un enfant direct du moteur, mais il est
son **propre leader de groupe de processus** (`executor.rs:2962`, `.process_group(0)`, mika#855).
Un signal de groupe adressé au moteur ne l'atteint pas, `supervise-daemon` redémarre le moteur
depuis un troisième groupe, et le pilote est réparenté à PID 1. Il survit.

Sans le garde à `age=0`, tout redémarrage du moteur marquerait donc `failed` l'intégralité des
dispatchs réellement en vol — le défaut du ticket, sous une horloge plus brutale, puisque `age=0`
n'offre aucune marge.

### D8 — le doc-comment qui justifie `age=0` cesse d'affirmer une chose fausse *(AC1)*

`crates/mika-agent/src/task_engine/engine.rs:1226-1233` appuie l'agressivité du balayage au
démarrage sur deux propositions que M7 et M9 mesurent comme fausses : que `updated_at` serait
rafraîchi pendant un dispatch légitime, et qu'une ligne présente au démarrage a forcément
survécu à son processus.

Laisser ce commentaire en place pendant qu'on ajoute le garde qu'il déclare inutile produirait un
fichier qui se contredit. Il est réécrit pour dire ce qui est vrai : les lignes de suivi n'ont pas
de battement de cœur (M7), un pilote survit au redémarrage (M8), et c'est **le garde de vivacité**
— non l'âge — qui distingue désormais l'orpheline du travail en cours.

Périmètre strict : le texte du doc-comment, pas le comportement, qui est déjà couvert par D5.

### D6 — la constante et sa raison *(AC5)*

`crates/mika-common/src/config.rs:1042` — valeur portée à `14400`, avec la mesure M6 écrite en
doc-comment à côté : les percentiles, l'attente de créneau, et la phrase qui dit pourquoi le
chiffre est sûr *maintenant* (le seuil n'est plus le discriminant principal — D-4). Le prochain
lecteur ne doit pas avoir à refaire la requête SQL pour savoir d'où sort le nombre.

**Trois autres sites répètent littéralement `3600` et doivent suivre**, sans quoi la doc mentirait
sur sa propre constante :

- `config.rs:956` — « Default: [`DEFAULT_PHANTOM_SWEEP_AGE_SECONDS`] (3600, one hour). »
- `config.rs:1414` — « Returns the configured value or [`…`] (3600s). »
- `config.rs:2582` — l'assertion du test qui ancre la valeur par défaut.

Le grep de contrôle avant de clore : `grep -rn '3600' crates/mika-common/src/config.rs` ne doit
plus rendre de ligne parlant du balayage phantom.

### D7 — tests *(AC2, AC3)*

Déterministes, sur le précédent déjà posé par `process_liveness.rs` (qui teste avec
`std::process::id()` et le PID impossible `999_999_999`) :

1. **`test_phantom_sweep_spares_row_with_live_dispatch_child` (AC2)** — reproduit exactement le cas
   mesuré : ligne de suivi `action_type='none'`, `process_id IS NULL`, `in_progress`,
   `updated_at` vieux de 2 h ; enfant de rappel portant le PID **du processus de test** et son vrai
   `process_start_time` lu par `read_process_start_time`. Après balayage : la ligne de suivi est
   toujours `in_progress`, `result` est vide, et aucun événement d'audit `phantom_aged_out` n'a été
   écrit (assertion via `count_audit_events_by_tool_name`, déjà exposée pour cet usage).
2. **`test_phantom_sweep_still_reaps_row_without_live_child` (AC3)** — même ligne de suivi, aucun
   enfant. Après balayage : `failed` / `phantom_aged_out`. Le balayeur n'est pas désarmé.
3. **`test_phantom_sweep_still_reaps_row_with_dead_child` (AC3)** — même ligne de suivi, enfant
   portant `process_id = 999_999_999`. Après balayage : `failed` / `phantom_aged_out`. C'est le
   contrôle qui distingue « il y a un enfant » de « l'enfant vit », et donc le test qui aurait
   attrapé l'erreur que M2 décrit.
4. **`test_phantom_sweep_reaps_child_without_start_time` (D-3)** — enfant portant le PID du test
   mais `metadata` sans `process_start_time`. Balayée, motif journalisé.
5. **`test_find_dispatch_children_with_pid_ignores_pidless_children` (D1)** — l'helper ne rend pas
   les enfants sans `process_id`.

---

## Hors périmètre

Repris du ticket, et deux ajouts nommés pour qu'ils ne soient pas confondus avec un oubli :

- **Le verrou `global_dispatch_active`** (un seul dispatch long par classe) — choix de conception,
  pas ce défaut. Sa conséquence sur le débit mérite sa propre discussion.
- **Le comportement du feeder `auto_feeder_no_backlog`** — corrélé au tableau de bord faussé, non
  démontré causé. Le ticket le dit explicitement et le plan ne le contredit pas.
- **`stuck_pending_no_deferred_wrapper`** (`engine.rs:754`, 83 occurrences) — balayeur frère qui
  pourrait porter une forme voisine du même défaut sur les lignes `pending`. Le plan ne le touche
  pas : ce serait du périmètre non demandé. Nommé ici pour qu'un lecteur sache qu'il a été vu et
  laissé.
- **`kill_orphan_processes`** (`engine.rs:473`) — chemin distinct : il tue les processus des tâches
  *expirées* par `timeout_at`, pas ceux du balayage phantom. Nommé pour qu'il ne soit pas confondu
  avec la surface traitée ici. Non modifié.
- **Rafraîchir `updated_at` de la ligne de suivi pendant le dispatch** (M7) — corrigerait la cause
  profonde à la racine, mais change la sémantique d'un champ lu ailleurs. Le garde par vivacité
  résout l'AC sans ce risque ; le battement de cœur reste une option ultérieure.

## Acceptance criteria

Repris intégralement du ticket mika#2156, dans leur formulation d'origine.

- **AC1** — Avant de marquer une ligne de suivi `failed/phantom_aged_out`, le balayage vérifie
  qu'**aucune ligne de rappel active ne lui est rattachée** ; s'il en existe une dont le
  `process_id` correspond à un processus vivant, la ligne de suivi n'est pas balayée.
- **AC2** — Un test couvre exactement le cas mesuré ici : ligne de suivi `action_type='none'`,
  `process_id IS NULL`, `in_progress`, `updated_at` vieux de 2 h, **avec** une ligne de rappel
  vivante → la ligne de suivi survit.
- **AC3** — Le cas symétrique reste couvert : même ligne de suivi **sans** rappel vivant →
  toujours balayée (le correctif ne doit pas désarmer le balayeur, dont la raison d'être — les
  vraies orphelines — reste valide).
- **AC4** — Quand le balayage épargne une ligne parce qu'un rappel est vivant, il l'écrit dans le
  journal avec l'identifiant du rappel et le `process_id` retenu, pour que la décision soit
  lisible après coup.
- **AC5** — La valeur `DEFAULT_PHANTOM_SWEEP_AGE_SECONDS` est réexaminée à la lumière des durées
  réelles observées après mika#2146 / claude-pilot#147, et la raison du chiffre retenu est écrite
  à côté de la constante.

**Lecture retenue pour AC1** (validée par l'architecte en première passe, Q1) : « rappel actif »
est défini par la seconde moitié de la phrase — *processus vivant* — et non par le statut de la
ligne. Voir D-2 et la mesure M2, qui montre qu'une lecture par statut désarmerait le balayeur
à 98 %.

### Tie-back

| AC | Deliverable | Vérification |
|---|---|---|
| AC1 — vérifier l'absence de rappel actif vivant avant de marquer `failed` | D1, D2, D3, D5, D8 | D7-1 |
| AC2 — test du cas mesuré : suivi vieux de 2 h + rappel vivant → survit | D7-1 | `cargo test` |
| AC3 — cas symétrique : sans rappel vivant → toujours balayée | D7-2, D7-3 | `cargo test` |
| AC4 — journaliser l'épargne avec l'identifiant du rappel et le `process_id` | D4 | D7-1 (assertion sur l'absence d'audit `phantom_aged_out`) + lecture du log |
| AC5 — réexaminer la constante et écrire la raison à côté | D6 | Revue du doc-comment ; M6 est la mesure citée |
