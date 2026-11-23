# mika#3117 — vol 2 : l'auto-pull ne relance pas un pilote qui a tenu la halte

- **Ticket :** senara-solutions/mika#3117
- **Branche :** `fix/3117/auto-pull-apr-s-un-escalate-une-halte-ou`
- **Vol 1 (livré) :** `docs/plans/2026-11-22-001-fix-3117-auto-pull-apr-s-un-escalate-une-halte-ou-plan.md`, PR #3133, mergée le 2026-11-22 (`feac6737`).
- **Ce plan est le vol 2**, nommé — et explicitement non implémenté — par le vol 1 :

  > **Renvoyé au vol 2, nommé et non implémenté ici :** l'état *arrêt sans commit sur
  > un plan en HALTE* (commentaires 58b / 59b, 2026-11-19). Il appelle un lecteur
  > différent — l'issue du dernier pilote, pas le verdict du dernier groom — et donc
  > un filtre distinct ; le livrer ici doublerait le vol et le ferait passer la borne.

## Contexte — le fait mesuré

Le 2026-11-19 sur mika#1727 (V4-c) :

| instant (UTC) | événement |
|---|---|
| 15:49:28 | le pilote `137111d7` s'arrête. Il est parti sur un plan **en HALTE**, il a tenu la halte : zéro commit, zéro PR, worktree propre. |
| — | le label `ready` est resté posé ; la tâche pilote est terminale, donc invisible au filtre in-flight de la phase 2. |
| 15:50:13 | `phase2_stuck_rescue` élit mika#1727. |
| 15:50:23 | `stuck_ready_reconciled, direct_dispatch: dispatched` — le pilote `fa982bb6` part **2 min 25 s** après l'arrêt du précédent. |
| — | `fa982bb6` lit le même plan, tient la même halte, ne commite rien. |

Coût : 1,34 $ pour deux pilotes et zéro commit. L'attendu formulé par l'opérateur
(commentaire 59b) : *« un pilote qui finit sans commit sur un plan en halte n'est pas
relancé tant que la halte n'est pas levée »*.

**La cause.** `phase2_reconcile_stuck_ready` confond deux populations sous un seul
nom : « ticket que rien n'a jamais dispatché » — son vivier légitime : webhook perdu,
place de dispatch occupée, gate silencieuse — et « ticket dispatché, pilote terminé
stérile ». Une fois la tâche pilote terminale, plus aucun filtre de la chaîne D2 ne
la voit, et le ticket redevient un candidat indistinguable d'un ticket jamais parti.

Le vol 1 a fermé la moitié *groom* de mika#3117 : tout site d'engagement lit désormais
le verdict du dernier grooming (`groom_halt_exclusion`, trois sites, détecteur
structurel `EXPECTED_ENGAGEMENT_SITES = 3`). Ce vol ferme la moitié *pilote*, dans la
même forme et aux mêmes trois sites.

## Requirements

**R1.** Un ticket dont le dernier engagement est un pilote d'implémentation qui a
**conclu** sans produire ni commit ni PR n'est pas réengagé par l'auto-pull, à aucun
de ses trois sites d'engagement.

**R2.** Un ticket dont le dernier pilote a été **tué** — plafond de tours, garde-fou,
erreur SDK — continue de traverser. C'est exactement la population que le filet de
la phase 2 existe pour sauver ; la désarmer sur elle viderait la phase de son objet.

**R3.** Le discriminant entre R1 et R2 est lu **chez son producteur**, sans aucune
modification de `skills/bundled/_shared/dispatch-lib.sh` : le fait est déjà écrit en
base, en lignes ancrées, par le chemin qui compose le `result` du callback.

**R4.** Le refus a une **porte de sortie structurelle** : il cesse de mordre dès que
le ticket est regroomé, sans temporisation ni compteur.

**R5.** Chaque refus porte un **nom propre** et une ligne `auto_pull_exclusion` par
`ExclusionLedger`, jamais fondu avec une exclusion voisine — un refus pour halte
tenue et un refus pour issue illisible sont deux populations, donc deux noms
(doctrine mika#2205).

**R6.** La lecture est bornée par candidat réellement engagé : une requête par
ticket au moment de l'engager, jamais une par candidat de la population examinée.
En phase 2 le site est **après `attempted += 1`**, donc borné par
`MAX_STUCK_RESCUE_PER_TICK` (= 5), précédent mika#3047.

**R7.** La sonde est émise sur **toutes** les branches, y compris le cas sain : une
sonde muette dans le nominal ne distingue pas « lu, rien à signaler » de « jamais
passé ici », et son absence doit vouloir dire « binaire antérieur » (mika#2340),
jamais « zéro » (mika#2331).

**R8.** Un site d'engagement futur qui n'aurait pas franchi la garde doit être
refusé **par le compilateur ou par un test**, pas découvert par une mesure de
production six jours plus tard.

## Design

### D1 — le discriminant existe déjà, en lignes ancrées

`dispatch-lib.sh` compose le `result` du callback de pilote par deux chemins
mutuellement exclusifs, et c'est leur différence qui porte la réponse.

**Session tuée** (`STATUS = terminated`) : `_classify_terminated_session` écrit, dans
ses deux modes (`full` pour un kill sans travail, `banner` pour un kill qui a laissé
du travail), le bloc

```
Halt: <cause>
Halt class: <famille> — <sens>
Retry hint: <indice> — <sens>
```

**Session qui a conclu** (`STATUS = success`) : `_classify_terminated_session` ne
tourne pas. Un pilote qui conclut sans commit et sur un worktree propre prend la
branche terminale du contrôle post-vol et reçoit

```
PIPELINE FAILURE: claude-pilot exited 0 (status success) but HEAD unchanged (pre: …, post: …). Zero new commits produced.
```

puis, par la cascade Unit 3, `Outcome: PIPELINE_INCOMPLETE — manual recovery needed.`

La ligne `Halt class: ` est donc **présente si et seulement si** la session a été
tuée. C'est le miroir exact, en ligne ancrée et lisible côté Rust, du second axe que
`dispatch-lib.sh` nomme déjà dans `_pilot_had_no_shipping_tail` :

> The `STATUS = success` term is the SECOND axis: a session killed by a guardrail or
> an SDK limit carries `STATUS = terminated` […], never `success`. Truncated work must
> not be presented as complete just because its perimeter had no shipping tail.

**Pourquoi un lecteur et pas un nouveau stamp.** L'option « faire écrire la halte par
le producteur » — un `Halt: deliberate` ou un `Outcome:` distinct posé par
`dispatch-lib.sh` — est plus doctrinale au premier regard, et elle est écartée pour
deux raisons mesurables : (i) elle triple le vol (shell + Rust + prompt bundled) et le
ferait passer la borne par vol ; (ii) un stamp n'existe qu'après un `make deploy`
(classe mika#2340), donc la population historique resterait muette et le filtre
lirait « jamais de halte » sur tous les tickets antérieurs au déploiement. Le fait
cherché est **déjà stampé** : ce vol le lit, il ne le réinvente pas.

### D2 — un seul lecteur base, les deux classes d'engagement

`Database::issue_engagement_callbacks_newest_first(agent_id, issue_url)`, frère de
`groom_callback_results_newest_first` (`db/tasks.rs:4714`) — même jointure, même
énumération d'URL (`task_state::tasks::issue_url_variants`, `IN (?2, ?3)`), même
ordre, et **aucun filtre de classe** :

```sql
SELECT COALESCE(child.dispatch_class, 'implement') AS dispatch_class,
       child.label,
       child.result
FROM tasks child
JOIN tasks parent ON child.parent_task_id = parent.id
WHERE child.agent_id = ?1
  AND child.trigger_type = 'callback'
  AND child.status IN ('completed', 'delivered')
  AND parent.reference_url IN (?2, ?3)
ORDER BY child.created_at DESC, child.id DESC
```

Trois décisions, chacune héritée d'un précédent écrit :

- **`ORDER BY created_at DESC, id DESC`** et non `updated_at` : `created_at` est la
  naissance du dispatch, qu'aucun faucheur ne réécrit, là où `updated_at` est touché
  tard par le watchdog (#959), le faucheur de calage silencieux (mika#2249) et la
  passe fantôme (mika#1712). `id DESC` rend l'ordre **reproductible** pour deux
  callbacks nés dans la même seconde, et rien de plus : `tasks.id` est un UUID v4,
  donc le départage n'est jamais chronologique. La justification complète vit sur
  `groom_callback_results_newest_first` et n'est pas dupliquée.
- **Aucun `WHERE` sur `dispatch_class`**, et aucun sur le lanceur différé : une ligne
  écartée par SQL est invisible au verdict (doctrine mika#2590) et, surtout, elle est
  **incomptable** — les champs `skipped_*` de la sonde D4 existent pour dire combien
  de lignes neutres ont été enjambées, et un pré-filtre les ferait lire zéro sur la
  population exacte qu'ils servent à voir. La cardinalité ne plaide pas davantage : la
  jointure borne déjà le résultat à une poignée de lignes par ticket.
- **Les deux classes dans la même requête**, pas deux requêtes : la question est
  *« quel est le dernier engagement de ce ticket, et de quelle nature ? »*. Deux
  listes ordonnées séparément ne se fusionnent pas sans leurs `created_at`, et un
  deuxième `created_at` remonté serait un ordre reconstruit en aval — exactement ce
  que D1 refuse de faire pour le discriminant.

Wrapper asynchrone dans `async_db.rs`, posé à côté de
`groom_callback_results_newest_first` (`async_db.rs:1842`), même forme, `agent_id`
résolu en interne. Le registre des lecteurs (`task_state/tasks.rs:614`) reçoit sa
ligne : *« quels sont ses callbacks d'engagement, toutes classes, du plus récent au
plus ancien ? »*.

### D3 — l'état, dans `skills/executor.rs`, à côté du frère groom

```rust
pub(crate) const PILOT_HALT_CLASS_LINE_PREFIX: &str = "Halt class: ";
pub(crate) const PILOT_PR_LINE_PREFIX: &str = "PR: ";
pub(crate) const PILOT_ZERO_COMMIT_HEAD_PREFIX: &str = "PIPELINE FAILURE: claude-pilot exited ";
pub(crate) const PILOT_DEFERRED_LAUNCHER_RESULT: &str = "deferred dispatch slot freed";
pub(crate) const PILOT_OUTCOME_SELECTED_EVENT: &str = "pilot_outcome_selected";

pub(crate) enum PilotOutcomeState {
    NeverEngaged,
    SupersededByGroom,
    HeldHalt,
    Killed,
    Produced,
    Unreadable(String),
}
```

**L'ancrage est sur la LIGNE (`starts_with` / `lines()`), jamais `contains` sur le
texte.** La prose d'un pilote cite ces marqueurs en parlant du mécanisme : le faux
positif est mesuré deux fois dans ce dépôt — le Signal S (mika#2050) et les 651
détections d'`instr` (mika#2105). `dispatch-lib.sh` écrit les vraies lignes en tête
de ligne ; le lecteur les lit là.

**La classification d'une ligne de classe `implement`, dans cet ordre :**

| # | condition | état | pourquoi à cette place |
|---|---|---|---|
| 1 | `result` est `NULL` | `Unreadable("result NULL")` | aucune grammaire à lire ; le chemin d'écriture de complétion porte toujours un `result`, donc c'est anomal. |
| 2 | une ligne `starts_with("Halt class: ")` | `Killed` | **premier, parce qu'il porte la décision.** Une session tuée *avec* travail porte à la fois cette bannière et, parfois, un `PR:` ; les deux traversent, mais c'est ce terme-là qui est le miroir de `STATUS = success` et c'est donc lui qui doit nommer l'état dans la sonde. |
| 3 | une ligne `starts_with("PR: ")` | `Produced` | **avant le test 4, et c'est le point le plus délicat du vol.** La récupération de worktree sale (mika#1282, `dispatch-lib.sh:7475`) écrit une tête `PIPELINE FAILURE: claude-pilot exited <N> with HEAD unchanged — dirty worktree detected and auto-committed`, **puis** ouvre une draft PR et pose `PR: <url>`. Lire la tête d'abord classerait en halte tenue un contenu qui a bien été sauvé et poussé. |
| 4 | une ligne `starts_with("PIPELINE FAILURE: claude-pilot exited ")` | `HeldHalt` | la tête zéro-commit, un seul site d'écriture une fois le test 3 passé. |
| 5 | sinon | `Produced` | **défaut ouvert.** |

**Pourquoi le bras 5 est ouvert et le bras 1 fermé.** Les deux sont des absences, et
elles ne sont pas de même nature. Un `result` non vide sans tête d'échec a une
grammaire complète qui *dit* « pas d'échec zéro-commit » ; un `result` NULL n'a aucune
grammaire et ne dit rien du tout. Le filet de la phase 2 existe pour sauver, donc le
doute sur le contenu profite au ticket ; le doute sur la **lisibilité** est fail-closed
sous son propre nom, comme `GroomVerdictState::Unreadable` au vol 1 — on ne prouve pas
l'absence de halte à partir d'une absence de texte.

**La sélection sur la liste**, dans `pilot_outcome_from_rows(rows) ->
PilotOutcomeSelection { state, rows_examined, skipped_neutral, skipped_unknown_class }` :
on marche du plus récent au plus ancien ;

- une ligne dont le `label` finit par `:deferred` ou dont le `result` est exactement
  `PILOT_DEFERRED_LAUNCHER_RESULT` est un **lanceur différé** : elle ne porte aucun
  verdict, elle est enjambée et comptée dans `skipped_neutral`. C'est la ligne qui, le
  2026-10-18 à 18:20:02Z sur mika#2762, masquait un `ESCALATE` de 18:13 ;
- une ligne dont la classe n'est ni `groom` ni `implement` est enjambée et comptée
  dans `skipped_unknown_class` — un bras nommé et **comptable**, pas un `_ =>`
  silencieux : `dispatch_class` est une colonne SQL, donc le compilateur ne garantit
  rien ici et la sonde est le substitut ;
- la **première** ligne retenue décide : classe `groom` → `SupersededByGroom` ; classe
  `implement` → la table ci-dessus ;
- liste épuisée → `NeverEngaged`.

**`SupersededByGroom` est la porte de sortie de R4.** Un regroom terminal postérieur
au dernier pilote rend la main au vol 1, qui est le juge du groom : si le nouveau
verdict est GROOMED, le ticket repart ; s'il est ESCALATE, c'est
`groom_halt_exclusion` qui le retient, sous son propre nom. Le refus du vol 2 cesse
donc de mordre **par construction**, sans compteur, sans fenêtre et sans date —
exactement ce que demande *« tant que la halte n'est pas levée »* : lever une halte,
c'est regroomer.

La seconde porte de sortie n'est pas dans ce vol et n'a rien à y faire : un
`remove → add` humain du label `ready` emprunte le chemin du handler webhook
(`webhook_dispatch.rs`, `ready_label_handler`), qui n'est pas un site d'`auto_pull.rs`
et ne consulte donc aucune de ces deux gardes. Le geste d'opérateur garde la main sur
la boucle autonome, ce qui est la propriété que le vol 1 a déjà établie pour
l'ESCALATE.

### D4 — la sonde

```rust
info!(
    target: "mika::otel",
    event = PILOT_OUTCOME_SELECTED_EVENT,
    issue_url, rows_examined, skipped_neutral, skipped_unknown_class, state,
    "mika#3117 — état du dernier engagement du ticket, choisi sur les callbacks \
     terminaux du plus récent au plus ancien, lanceurs différés enjambés"
);
```

Émise sur **toutes** les branches, `NeverEngaged` et `rows_examined == 0` compris
(R7). Le champ `state` prend la valeur d'une constante par état, jamais un `{:?}`.

### D5 — la garde, dans `auto_pull.rs`

```rust
const FILTER_PILOT_HELD_HALT: &str = "pilot_held_deliberate_halt";
const FILTER_PILOT_OUTCOME_UNREADABLE: &str = "pilot_outcome_unreadable";

async fn pilot_halt_exclusion(db: &AsyncDatabase, issue: u64) -> Option<&'static str>
```

Même forme que `groom_halt_exclusion` (`auto_pull.rs:2554`) : même construction
d'URL (`https://github.com/{DEFAULT_REPO}/issues/{issue}`, parce que les deux
interrogent la même colonne et qu'une URL d'une autre forme ne retrouverait aucune
ligne — un refus muet déguisé en nominal), et un `match` **exhaustif sans bras
`_ =>`** sur les six états, pour qu'un septième état soit tranché par le compilateur :

| état | décision |
|---|---|
| `NeverEngaged` | `None` — le vivier légitime du filet. |
| `SupersededByGroom` | `None` — le vol 1 décide. |
| `Killed` | `None` — R2, c'est la raison d'être de la phase 2. |
| `Produced` | `None` — le travail existe ; s'il manque une PR, c'est une autre population (voir *Hors périmètre*). |
| `HeldHalt` | `Some(FILTER_PILOT_HELD_HALT)` |
| `Unreadable(reason)` | `warn!` qui écrit le motif, puis `Some(FILTER_PILOT_OUTCOME_UNREADABLE)` — le `ExclusionLedger` ne tient qu'un `&'static str`, donc « base muette » et « callback sans result » restent distinguables derrière le compte unique du filtre. |

**Trois sites d'appel**, les mêmes que le vol 1, immédiatement après
`groom_halt_exclusion` et dans cet ordre — groom d'abord, pilote ensuite, parce que
c'est l'ordre causal : un ticket qui a escaladé au groom n'a jamais eu de pilote.

| site | emplacement | bornage |
|---|---|---|
| phase 0, feeder | `auto_pull.rs:~5659` | un ticket sur le point de recevoir `ready`. |
| phase 1, promotion | `auto_pull.rs:~5824`, bras `GateOutcome::Allowed` de la sonde | un ticket par descente de classement. |
| phase 2, filet | `auto_pull.rs:~6777` | **après `attempted += 1`**, donc borné par `MAX_STUCK_RESCUE_PER_TICK` (= 5) par tick (R6, précédent mika#3047). |

Chaque refus : `warn!(issue, filter, …)` puis
`ledger.record(ExclusionPhase::<phase>, issue, filter)` puis `continue` — aucun
dispatch, aucun churn.

**Pourquoi les trois sites et pas seulement la phase 2**, alors que le fait mesuré est
en phase 2 : parce que le prédicat est sur l'**acte** d'engager, pas sur la phase. Un
ticket dont le pilote a tenu la halte garde son label `ready` et n'est plus in-flight ;
la phase 1 l'élira tout aussi bien le lendemain. Une garde posée sur un site des trois
est exactement la forme que le vol 1 vient de corriger.

### D6 — le détecteur structurel : généraliser, ne pas dupliquer

Le vol 1 a livré `scan_engagement_sites_read_the_verdict(src)`
(`auto_pull.rs:~9590`), paramétré par une garde unique `HALT_GUARD_CALL`. Le vol 2 le
**généralise** au lieu d'en écrire un second :

```rust
const REQUIRED_ENGAGEMENT_GUARDS: &[&str] =
    &["groom_halt_exclusion(", "pilot_halt_exclusion("];
```

Le scan continue d'asserter `EXPECTED_ENGAGEMENT_SITES = 3` — exactement trois,
jamais « au moins trois », pour qu'il rougisse dans les deux sens — et exige désormais
que **chacune** des gardes de la liste soit franchie par **chacun** des trois sites.
La preuve en deux morceaux de la phase 1 (`PHASE1_LABEL_FN`, `PHASE1_DESCENT_CALL`,
`PHASE1_PROBE_IMPL`) est conservée telle quelle et appliquée aux deux gardes.

Un second scan juxtaposé compterait deux fois la même population de sites sous deux
noms, et la divergence entre les deux comptes serait la première chose qu'un
mainteneur aurait à re-dériver. `ENGAGEMENT_SITES_UNCONFORMABLE` reste **vide** (voir
*Fire-Disposition*).

## Ordre de commit

Chaque commit compile et passe la suite ; le plancher mergeable est le commit 4.

1. **`db/tasks.rs` + `async_db.rs` + registre** — `issue_engagement_callbacks_newest_first`,
   son wrapper, sa ligne de registre, et ses tests de base (ordre, variantes d'URL,
   `result` NULL remonté tel quel, aucune classe filtrée). Lecteur sans lecteur :
   inerte, vérifiable seul.
2. **`skills/executor.rs`** — constantes d'ancrage, `PilotOutcomeState`,
   `pilot_outcome_from_rows`, `pilot_outcome_state`, la sonde, et les tests de
   classification sur fixtures textuelles. Toujours aucun appelant en production.
3. **`auto_pull.rs`, la garde** — les deux filtres nommés et `pilot_halt_exclusion`,
   avec ses tests. Encore aucun site d'appel : le comportement de production est
   inchangé.
4. **`auto_pull.rs`, les trois sites + le détecteur** — câblage aux trois sites,
   généralisation du scan à `REQUIRED_ENGAGEMENT_GUARDS`, tests de comportement par
   phase. **C'est le plancher : avant ce commit le vol ne corrige rien, après il
   corrige le fait mesuré du 2026-11-19.**

## Verification Contract

Tests de classification (`skills/executor.rs`), sur fixtures textuelles reproduisant
les formes réelles de `dispatch-lib.sh` :

- **V1** — `result` du 2026-11-19 (`PIPELINE FAILURE: claude-pilot exited 0 (status
  success) but HEAD unchanged … Zero new commits produced.` + `NO_PR:` +
  `Outcome: PIPELINE_INCOMPLETE — manual recovery needed.`) → `HeldHalt`.
- **V2** — kill sans travail (tête `… session was terminated before it produced any
  work.` + `Halt:` + `Halt class:` + `Retry hint:`) → `Killed`.
- **V3** — kill **avec** travail (bannière `banner` + bloc de récupération) →
  `Killed`, et non `HeldHalt` malgré la tête `PIPELINE FAILURE:`.
- **V4** — récupération de worktree sale (mika#1282 : tête
  `PIPELINE FAILURE: claude-pilot exited 1 with HEAD unchanged — dirty worktree
  detected and auto-committed` **plus** `PR: <draft>`) → `Produced`. **C'est le test
  qui épingle l'ordre des bras 3 et 4** ; inverser l'ordre doit le rougir.
- **V5** — `Outcome: PR_OPENED — <url>` → `Produced`.
- **V6** — `result` NULL → `Unreadable`.
- **V7** — ancrage : un `result` nominal dont la **prose** cite
  `Halt class: session_silent` ailleurs qu'en tête de ligne n'est pas classé `Killed`.
- **V8** — sélection : un lanceur différé posé au-dessus d'un `HeldHalt` est enjambé,
  `skipped_neutral == 1`, et l'état reste `HeldHalt`.
- **V9** — sélection : un callback de **groom** plus récent que le pilote →
  `SupersededByGroom` (R4).
- **V10** — liste vide → `NeverEngaged`, `rows_examined == 0`.
- **V11** — classe inconnue en tête → enjambée, `skipped_unknown_class == 1`, et
  l'état vient de la ligne suivante.

Tests de garde et de comportement (`auto_pull.rs`) :

- **V12** — `pilot_halt_exclusion` rend `Some(FILTER_PILOT_HELD_HALT)` sur `HeldHalt`,
  `Some(FILTER_PILOT_OUTCOME_UNREADABLE)` sur `Unreadable`, `None` sur les quatre
  autres états.
- **V13 / V14 / V15** — un ticket `HeldHalt` n'est engagé ni par la phase 0, ni par la
  phase 1, ni par la phase 2 ; chaque refus pose **une** ligne
  `auto_pull_exclusion` portant son propre nom de filtre.
- **V16** — **contrôle positif, R2** : le même ticket avec un `result` portant
  `Halt class:` **est** dispatché par la phase 2. Sans ce test, V13–V15 seraient verts
  sur un filtre qui refuse tout.
- **V17** — bornage R6 : sur une population de 20 candidats dont chacun est `HeldHalt`,
  la phase 2 ne fait **pas plus de 5** lectures base, parce que la garde est après
  `attempted += 1`.
- **V18** — la sonde `pilot_outcome_selected` est émise sur le cas sain
  (`NeverEngaged`) comme sur le refus (R7).

Détecteur structurel :

- **V19** — `scan_engagement_sites_read_the_verdict` sur l'arbre réel : vert, trois
  sites, **les deux** gardes franchies partout.
- **V20** — contrôle de bonne foi : sur une source synthétique à quatre sites, le scan
  rougit ; sur une source où un site franchit `groom_halt_exclusion` mais **pas**
  `pilot_halt_exclusion`, il rougit aussi, en nommant la garde manquante.

Portes locales : `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`,
`bash scripts/verify-pipeline.sh origin/main`.

## Definition of Done

- Les quatre commits de l'*Ordre de commit* sont sur la branche, chacun vert.
- `issue_engagement_callbacks_newest_first` existe, ne filtre aucune classe, et sa
  ligne figure au registre des lecteurs (`task_state/tasks.rs`).
- `PilotOutcomeState` a six variantes et tous ses `match` de production sont
  exhaustifs **sans bras `_ =>`**.
- `pilot_halt_exclusion` est appelée aux trois sites d'engagement, après
  `groom_halt_exclusion`, et en phase 2 **après** `attempted += 1`.
- Les deux filtres sont nommés séparément et passent par `ExclusionLedger::record`.
- La sonde `pilot_outcome_selected` est émise sur toutes les branches.
- Le scan structurel exige les deux gardes et `ENGAGEMENT_SITES_UNCONFORMABLE` est
  vide.
- `dispatch-lib.sh` n'est **pas** modifié.
- V1 à V20 passent ; `cargo clippy`, `cargo fmt --check` et
  `scripts/verify-pipeline.sh origin/main` sont verts.

## Acceptance criteria

- **AC1** — rejouée sur le `result` réel de `137111d7` (2026-11-19, 15:49:28Z), la
  phase 2 ne dispatche pas `fa982bb6` et pose une ligne `auto_pull_exclusion`
  `pilot_held_deliberate_halt`. (V1, V13–V15)
- **AC2** — un ticket dont le dernier pilote porte `Halt class:` est toujours
  dispatché par la phase 2. (V2, V3, V16)
- **AC3** — un ticket dont le dernier pilote a été sauvé par la récupération de
  worktree sale (`PR:` présent sous une tête `PIPELINE FAILURE:`) n'est pas classé en
  halte tenue. (V4)
- **AC4** — un regroom terminal postérieur au dernier pilote rend la main au vol 1 :
  `pilot_halt_exclusion` ne refuse plus. (V9)
- **AC5** — les deux filtres apparaissent sous deux noms distincts dans le ledger ;
  aucun refus n'est comptabilisé sous un filtre voisin. (V12–V15)
- **AC6** — la phase 2 ne fait pas plus de `MAX_STUCK_RESCUE_PER_TICK` lectures base
  par tick pour cette garde. (V17)
- **AC7** — un quatrième site d'engagement, ou un site qui n'appelle qu'une des deux
  gardes, fait rougir le scan structurel. (V19, V20)
- **AC8** — aucune ligne de `skills/bundled/_shared/dispatch-lib.sh` n'est modifiée
  par la PR.

## Fire-Disposition

Ce vol livre des détecteurs : les tests V1–V20 et, surtout, le **scan structurel**
généralisé de D6, dont le chemin de succès est « aucune violation trouvée ».

**Option retenue : (a) — exception nommée en allowlist, livrée avec une population
d'exceptions vide.**

- `ENGAGEMENT_SITES_UNCONFORMABLE` reste `&[]`. Les trois sites d'engagement de
  production franchissent les deux gardes dès le commit 4 : il n'existe donc aucune
  violation à exempter au moment de l'atterrissage, et le détecteur arrive **armé**.
- **L'assertion est auto-nettoyante et rougit dans les deux sens.**
  `EXPECTED_ENGAGEMENT_SITES = 3` est une égalité stricte, jamais un `>=` : un `>=`
  laisserait passer une quatrième porte non conforme, un `<=` laisserait passer la
  suppression d'une des trois — c'est-à-dire un correctif désarmé que le scan lirait
  comme un arbre propre. Le même contrat s'étend à `REQUIRED_ENGAGEMENT_GUARDS` : la
  liste est lue en entier pour chaque site, et retirer une garde de la liste redonne
  un scan orphelin, donc un vert qui ne dit plus rien.
- **Ce que vaut une entrée future.** Quand ce scan tirera sur une quatrième porte, la
  résolution est d'y appeler les deux gardes — pas de relâcher l'assertion, pas de la
  re-keyer sur `>=`, pas d'ajouter une entrée. Une entrée ici est une **dette nommée**
  qui doit porter, dans sa ligne, le nom de la fonction exemptée et le ticket qui la
  retirera, dans la lignée de `senara-solutions/mika#2983` et sous le même contrat
  que `TICK_COPY_READERS_ALLOWED` (doctrine mika#2201 / mika#2309).
- **Le contrôle de bonne foi est livré avec le détecteur** (V20) : le scan est en
  `Result` précisément pour qu'un test puisse le montrer **rougir** sur des sources
  synthétiques. Un détecteur dont on n'a jamais vu le rouge est un détecteur dont on
  ne sait pas s'il regarde.

Aucun détecteur n'atterrit désarmé (`#[ignore]`), et rien ici n'appelle une
halte-et-remontée : le périmètre est tranché et la population d'exceptions est
mesurée vide.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/db/tasks.rs` — lecteur + doc | 65 |
| `crates/mika-agent/src/async_db.rs` — wrapper + doc | 14 |
| `crates/mika-agent/src/task_state/tasks.rs` — ligne de registre | 2 |
| `crates/mika-agent/src/skills/executor.rs` — constantes, `PilotOutcomeState`, `pilot_outcome_from_rows`, `pilot_outcome_state`, sonde + doc | 120 |
| `crates/mika-agent/src/auto_pull.rs` — deux filtres, `pilot_halt_exclusion`, trois sites, généralisation du scan + doc | 90 |
| tests (`executor.rs`, `auto_pull.rs`, `db/tasks.rs`) — V1 à V20 | 105 |

Total estimé : 396 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 396 est sous la borne par vol (429) et très en deçà
du seuil par PR (1000), et le découpage qui aurait fait passer la borne a déjà été
fait : c'est le vol 1, livré. Trois propriétés bornent ce qui reste :
`dispatch-lib.sh` n'est pas touché — le vol est entièrement en Rust, dans un seul
crate ; le détecteur structurel est **généralisé** plutôt que dupliqué, ce qui
économise un scan entier et son contrôle de bonne foi ; et les trois sites d'appel
sont déjà identifiés, nommés et épinglés par un scan existant, donc le câblage est
de l'insertion, pas de la recherche. La part la plus lourde est la classification
(D3), et son coût est dominé par la table de décision et ses fixtures, toutes deux
déjà écrites ci-dessus.

## Hors périmètre

Nommés, non implémentés ici, et renvoyés au suivi :

- **Le pilote arrêté par un refus de permission-policy** (tête
  `PIPELINE FAILURE: claude-pilot session halted by policy deny`, `dispatch-lib.sh`
  ~8470). Zéro commit lui aussi, et le re-dispatcher refrappe le même mur — mais c'est
  une **population distincte avec un remède distinct** (élargir l'allow-list, ou
  réécrire le contexte de dispatch). La fondre dans `HeldHalt` compterait deux
  populations sous un seul nom, ce que mika#2205 interdit. Elle traverse donc ici, en
  `Produced`, et son filtre propre reste à écrire.
- **Le pilote qui a conclu avec des commits mais sans PR** sur un périmètre qui, lui,
  avait une queue d'expédition. `Produced` le laisse passer. Le re-dispatch n'est pas
  forcément stérile sur cette population (la branche existe, le travail est poussé),
  et trancher demanderait de lire l'état de la forge, pas celui de la base.
- **La seconde porte de levée dans l'auto-pull** — un geste d'opérateur qui lèverait
  le refus sans regroomer. Le chemin webhook (`ready_label_handler`) donne déjà cette
  main à l'opérateur ; en donner une seconde depuis `auto_pull.rs` demanderait un
  lecteur de timeline GitHub aux trois sites.

Tracked in: senara-solutions/mika#2983
