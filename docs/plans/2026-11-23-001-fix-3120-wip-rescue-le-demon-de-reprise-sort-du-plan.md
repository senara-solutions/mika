# fix(3120) — le démon de reprise sort un brouillon qu'un pilote est en train d'écrire

> Ticket : senara-solutions/mika#3120
> Vol 1 sur 2. Le vol 2 est nommé par la ligne `Tracked in:` de la section `## Périmètre`.

## Le défaut, tel qu'il se mesure

`wip_rescue` sort un brouillon `wip-rescue` vers la revue **sans jamais demander si
quelqu'un est encore en train d'écrire dessus**. Deux incidents, de deux natures
différentes, ont été relevés sur le parking #2983.

**Incident A — le pilote de finition écrivait encore (commentaire 50, PR #3092, 2026-11-15).**

| instant (UTC) | fait |
|---|---|
| 14:43 → 14:58 | le pilote de finition commite sur `head_ref` de #3092 |
| 14:50:02 | `wip_rescue_resume_attempt pr_number=3092 age_secs=992` |
| 14:51:57 | `wip_rescue_success classification=DecisionCore` — la PR sort du brouillon |

Le brouillon a ~16 minutes quand le démon le sélectionne, et le pilote a encore
sept minutes de commits devant lui. Deux conséquences, toutes deux portées par la
mesure : mika-qa revoit une tête encore en vol, et le brouillon cesse de jouer son
rôle de frein au merge pendant que son auteur écrit.

**Incident B — l'arrêt délibéré lu comme une réussite (commentaire 47, PR #3081, 2026-11-13).**
`_measure_pipeline_verified` a tamponné `rescue-pipeline-verified: yes` sur un vol
qui s'était arrêté **volontairement** à sa borne de lignes (« HALTE borne ») ; le
démon l'a sorti du brouillon à 08:26:42Z et un humain l'a remis en brouillon à
09:00:24Z. Ce défaut-là n'est pas dans le même substrat (il est côté shell, dans le
producteur du marqueur) ; il est **hors du vol 1** et nommé par `Tracked in:`.

**Le trou, nommé.** `crates/mika-agent/src/wip_rescue.rs` ne mentionne nulle part un
pilote vivant : ni en production, ni en test (`grep -c 'live_pilot\|is_same_process_alive'
crates/mika-agent/src/wip_rescue.rs` → `0`). La sélection connaît trois exclusions —
marqueur de bail (mika#2199), marqueur parked-unverified (mika#2286), hold /
`ConvertToDraftEvent` (mika#2597) — et aucune ne porte sur l'activité en cours.
C'est exactement le constat du corps du ticket : *« aucun test de pilote vivant »*.

## Requirements

- **R1.** Un brouillon dont la branche de tête est celle d'un worktree de dispatch
  **dont le processus pilote est vivant** n'est pas sélectionné par ce tick.
- **R2.** L'exclusion est **transitoire et auto-levante** : elle ne pose aucun
  marqueur durable et disparaît d'elle-même dès que le pilote meurt. Elle n'est ni
  un bail (mika#2199) ni un park (mika#2286), qui sont tous deux durables et
  exigent un geste pour être levés.
- **R3.** L'exclusion **ne consomme pas le créneau du tick** (plafond AC6 = 1 sortie
  de brouillon par tick, du plus ancien d'abord). Un brouillon écarté laisse le
  candidat suivant être examiné dans le même tick. C'est la forme du verrou
  mika#2199 : une exclusion posée *après* la sélection au lieu de *dans* la
  sélection affame la file (14 bails sur une seule PR en six heures).
- **R4.** **Indétermination dirigée, et dissymétrique par mesure.**
  - Le **recensement** des pilotes vivants illisible (erreur DB) ⇒ **rien n'est
    repris ce tick**, sous une ligne WARN nommée. Un tick différé coûte un cycle de
    cinq minutes ; une sortie de brouillon prématurée n'est pas révocable sans
    humain, et l'incident A dit ce qu'elle coûte.
  - Une **tâche individuelle** dont le worktree déclaré ou la branche sont
    illisibles **n'exclut rien**, et le dit. Fail-closer ici donnerait à un seul
    dispatch mal tamponné le pouvoir de geler la file indéfiniment, pour une
    population qui est précisément celle que `DISPATCH_WORKTREE_FILE_KEY` documente
    comme tolérée (*« an undeclared dispatch is invisible to the reaper, never a
    blocked dispatch »*).
- **R5.** **« Aucun pilote vivant » et « la sonde n'a pas tourné » ne rendent pas les
  mêmes octets** (mika#2205). Une ligne d'observabilité nommée est émise à chaque
  exclusion, et une autre à chaque recensement — y compris sur la branche saine.
- **R6.** La question *« un pilote est-il vivant ? »* reste à son **lecteur unique**,
  `crate::live_pilot`. `wip_rescue` n'appelle ni `is_same_process_alive`, ni le
  filtre terminal, ni la traversée de dispatch.

## Design

### D1 — un quatrième handle dans `live_pilot`, jamais une quatrième recomposition

`crates/mika-agent/src/live_pilot.rs` se déclare **seul lecteur** de la vivacité
d'un pilote, et ce n'est pas une déclaration décorative : le scan de source
`canonical_tokens::mika2653_la_vivacite_dun_pilote_a_un_lecteur_unique` refuse tout
fichier de production hors de ce module qui compose une traversée de dispatch, le
filtre terminal et `is_same_process_alive`, et les deux poignées de
`find_dispatch_children_with_pid` sont `pub(crate)` pour que la recomposition soit
**impossible** hors du crate, pas seulement nommée.

Le module porte déjà deux handles — `live_pilot_for_issue` (clé : URL d'issue) et
`live_pilot_for_task` (clé : id de tâche) — et sa propre doctrine dit comment on en
ajoute un troisième : *« A census is not an allowlist: you add to it, you do not
exempt in it »* (mika#2633). Le vol ajoute donc :

```rust
/// Les worktrees déclarés des pilotes actuellement vivants (mika#3120).
pub(crate) async fn live_pilot_worktrees(db: &AsyncDatabase) -> LiveWorktrees
```

- Population : `AsyncDatabase::get_live_dispatch_callback_tasks_with_pid()`
  (`db/tasks.rs:3642` — `trigger_type = 'callback'`, `status IN ('pending',
  'in_progress')`, `process_id IS NOT NULL`). C'est la population du faucheur de
  pilotes silencieux (`engine.rs:3298`), donc celle dont on sait déjà qu'elle est
  bien peuplée : `pending` y est inclus délibérément, un enfant de callback naissant
  `pending` et y restant jusqu'à la livraison.
- Vivacité : `is_same_process_alive(pid, start_time)`, `start_time` lu par
  `Database::process_start_time_from_metadata` — la règle qui accepte la forme
  chaîne **et** la forme entière, jamais un `.as_str()?.parse()` à la main (la
  divergence que mika#2653 a payée).
- Worktree : le fichier de déclaration nommé par `DISPATCH_WORKTREE_FILE_KEY`, dont
  le contenu trimé est le chemin. `Engine::declared_worktree` devient une fonction
  libre `pub(crate) fn declared_worktree(...)` du module `task_engine::engine` —
  même corps, un seul appelant à requalifier — pour que le nouveau handle la lise
  sans dupliquer la lecture du fichier.
- Retour : `LiveWorktrees { worktrees: Vec<PathBuf>, undeclared: usize }`, ou
  l'erreur DB. `undeclared` compte les pilotes vivants sans déclaration lisible :
  c'est la population de R4, et elle est **comptée plutôt que tue**.

Le handle ne touche pas à `git` et n'a pas d'opinion sur les branches : il répond
exactement à la question que le module possède.

### D2 — l'attribution worktree → branche vit chez `wip_rescue`

`wip_rescue` possède déjà un `git(dir, args, timeout)` borné en temps (ligne 624).
Pour chaque worktree vivant rendu par D1 :

```
git -C <worktree> rev-parse --abbrev-ref HEAD
```

Le résultat trimé alimente un `BTreeSet<String>` des branches en cours d'écriture.
Un worktree disparu, un `git` en échec ou un `HEAD` détaché n'attribuent **aucune**
branche (R4, second terme) et incrémentent un compteur rapporté par la sonde.

### D3 — la sélection reçoit l'ensemble, pas un prédicat par candidat

`select_eligible` gagne un paramètre `live_branches: &BTreeSet<String>` plutôt
qu'un quatrième prédicat asynchrone injecté. Trois raisons, dans cet ordre :

1. **Le recensement est global au tick, pas par candidat.** Un prédicat par candidat
   suggérerait une interrogation par PR et paierait N traversées DB + N `git` là où
   une seule suffit.
2. **Le fail-close de R4 remonte à son bon étage.** Le recensement échoue *avant*
   `select_eligible` ; l'appelant renonce au tick. Le paramètre reçu par la
   sélection est donc **infaillible par construction** — un `contains` — et il n'y a
   pas de troisième état à gérer dans la boucle.
3. **Les tests restent sans base de données.** Un `BTreeSet` littéral est encore plus
   direct que les fermetures `never_bailed` / `never_parked` / `never_held` du
   module.

Dans la boucle, après les trois exclusions existantes et **avant** le `return` :

```rust
if live_branches.contains(&pr.head_ref) {
    info!(event = "wip_rescue_live_pilot_skipped", pr_number = pr.number,
          branch = %pr.head_ref, "…");
    continue;                       // R3 : le créneau du tick n'est pas consommé
}
```

`DraftPr.head_ref` (ligne 555, `#[serde(rename = "headRefName")]`) porte déjà le nom
de la branche et `list_wip_rescue_drafts` le demande déjà à `gh` : **aucun champ
nouveau n'est à récupérer**.

**Position dans l'ordre, délibérée.** Le nouveau terme est le quatrième, pas le
premier : les trois existants sont des lectures de marqueurs durables, bon marché et
décisives ; celui-ci coûte un `git` par worktree vivant. Le mettre en tête ferait
payer ce coût à des brouillons qu'un marqueur de bail écarte gratuitement.

### D4 — les deux sondes, et pourquoi elles sont deux

| ligne | niveau | quand | champs |
|---|---|---|---|
| `wip_rescue_live_pilot_census` | INFO | **à chaque tick**, branche saine comprise | `live_pilots`, `branches`, `undeclared`, `unresolved` |
| `wip_rescue_live_pilot_skipped` | INFO | une par brouillon écarté | `pr_number`, `branch`, `age_secs` |
| `wip_rescue_live_pilot_unreadable` | WARN | recensement DB en échec ⇒ tick abandonné | `error` |

Le recensement est émis **même quand il trouve zéro pilote**, parce que sans lui
« aucun pilote vivant » et « la sonde n'a jamais tourné » rendent zéro tous les deux
(mika#2205), et la cible nominale de cette sonde *est* zéro. `undeclared` et
`unresolved` sont deux populations distinctes — pilote sans déclaration lisible,
worktree sans branche lisible — et elles ne sont jamais sommées. **L'absence de la
ligne elle-même ne veut pas dire « pas de pilote » mais « binaire antérieur au
vol »** (mika#2340) : l'instrument n'est pas vivant avant `make deploy`.

## Périmètre

**Dans le vol 1 :**

- `crates/mika-agent/src/live_pilot.rs` — le handle `live_pilot_worktrees` et son type de retour.
- `crates/mika-agent/src/task_engine/engine.rs` — `declared_worktree` passe de méthode associée privée à fonction libre `pub(crate)`, corps inchangé.
- `crates/mika-agent/src/wip_rescue.rs` — attribution worktree → branche, paramètre `live_branches` de `select_eligible`, exclusion, fail-close du tick, trois sondes, et les tests.

**Hors du vol 1 — `Tracked in: senara-solutions/mika#3120` (vol 2) :** l'incident B,
l'arrêt délibéré à la borne de lignes. Son substrat est entièrement ailleurs —
`_measure_pipeline_verified` et `_compose_rescue_pr_body` dans
`skills/bundled/_shared/dispatch-lib.sh`, leur suite dans `test-dispatch-lib.sh`, et
très probablement une déclaration dans `.github/labels.yml` — et il porte une
difficulté propre que le vol 1 ne résout pas : le shell **ne peut pas** écrire le
marqueur de park de mika#2286 (c'est une ligne `audit_events` que possède le démon
Rust), et `rescue-pipeline-verified: no` seul ne suffit pas, puisque
`undraft_decision` ne gate que le périmètre DECISION-CORE sur ce marqueur — un
brouillon MECHANICAL sortirait quand même. Les deux gestes honnêtes côté shell sont
le label `human-review-required` existant ou un label neuf **déclaré** (le workflow
de synchronisation tourne en `delete-other-labels: true`, donc un label non déclaré
est supprimé du dépôt et de chaque issue qui le porte, en silence). C'est un
arbitrage à part entière, et il ne tient pas sous la borne de ce vol.

**Aucun détecteur n'est livré, et c'est un choix, pas un oubli (mika#2306).** Les
tests du vol sont **comportementaux** : leur chemin de succès est « le bon brouillon
est retourné », jamais « aucune violation trouvée ». Le scan structurel qui protège
ce domaine existe déjà et n'est pas touché :
`mika2653_la_vivacite_dun_pilote_a_un_lecteur_unique` reste vert sans modification
**précisément parce que** D1 ajoute le handle *dans* le module au lieu de recomposer
la traversée dans `wip_rescue` — c'est l'argument de conception, et le scan en est
la preuve mécanique. Le gate `## Fire-Disposition` est donc N/A et la section n'est
pas inventée.

## Verification Contract

| # | ce qui est vérifié | comment |
|---|---|---|
| V1 | un brouillon dont `head_ref` est dans `live_branches` n'est pas sélectionné | test de `select_eligible`, ensemble littéral, sans DB |
| V2 | le candidat **suivant** l'est dans le même tick (R3) | deux brouillons, le plus ancien écarté, le second retourné |
| V3 | contrôle négatif : même graine, `live_branches` vide ⇒ le plus ancien est retourné | la paire V1/V3 est ce qui sépare « l'exclusion mord » de « la sélection est cassée » |
| V4 | l'exclusion est auto-levante : aucun `audit_events` n'est écrit par ce chemin | assertion d'absence sur les deux outils de marqueur |
| V5 | l'ordre des quatre exclusions : un brouillon bailé **et** en cours d'écriture est écarté par le bail | le bail reste le premier terme |
| V6 | `live_pilot_worktrees` ignore une ligne terminale porteuse d'un pgid vivant | fixture DB, lignes `delivered` / `cancelled` |
| V7 | un pilote vivant sans déclaration lisible compte dans `undeclared` et n'exclut rien | fixture sans `dispatch_worktree_file` |
| V8 | recensement en erreur ⇒ aucune reprise, WARN émis | injection de l'erreur à l'étage de l'appelant |
| V9 | la sonde de recensement est émise sur la branche à zéro pilote | assertion sur la ligne, pas sur son absence |
| V10 | le scan `mika2653_…` reste vert | `cargo test -p mika-agent canonical_tokens` |

Plus, obligatoirement avant la PR : `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test -p mika-agent wip_rescue`, `cargo test -p mika-agent live_pilot`,
`scripts/verify-pipeline.sh origin/main`.

## Definition of Done

- `live_pilot_worktrees` existe dans `live_pilot.rs`, `pub(crate)`, documenté dans le registre du module.
- `declared_worktree` est une fonction libre `pub(crate)` ; son unique appelant historique (`reap_silently_stalled_pilots`) compile inchangé pour le reste.
- `select_eligible` porte le paramètre `live_branches` et l'exclusion en quatrième position, avec `continue`.
- `auto_resume_wip_rescue_drafts` recense une fois par tick, fail-close sur erreur DB, et appelle `select_eligible` avec l'ensemble résolu.
- Les trois sondes de D4 sont émises aux niveaux déclarés, le recensement sur **toutes** les branches.
- V1 à V10 passent ; `cargo fmt`, `clippy -D warnings` et `scripts/verify-pipeline.sh origin/main` sont verts.
- La PR porte `Tracked in: senara-solutions/mika#3120` pour le vol 2.

## Acceptance criteria

- **AC1.** Un brouillon `wip-rescue` dont la branche de tête est celle d'un worktree de dispatch à pilote vivant n'est pas sorti du brouillon par le tick courant.
- **AC2.** L'exclusion se lève d'elle-même à la mort du pilote : aucune ligne `audit_events`, aucun label, aucun geste humain ne sont requis pour que le brouillon redevienne éligible.
- **AC3.** Un brouillon écarté pour pilote vivant ne consomme pas le créneau du tick : le candidat suivant par ancienneté est examiné et peut être repris dans le même tick.
- **AC4.** Un recensement des pilotes vivants qui échoue en lecture n'entraîne **aucune** reprise sur ce tick, et émet une ligne WARN nommée. Un pilote vivant dont le worktree ou la branche sont illisibles n'exclut rien et est compté dans la sonde de recensement.
- **AC5.** La sonde `wip_rescue_live_pilot_census` est émise à chaque tick, y compris quand elle trouve zéro pilote vivant, et porte `live_pilots`, `branches`, `undeclared` et `unresolved` comme quatre champs distincts jamais sommés.
- **AC6.** `crates/mika-agent/src/wip_rescue.rs` ne contient aucun appel à `is_same_process_alive`, à `is_terminal_task_status` ni à `find_dispatch_children_with_pid` : la vivacité est demandée à `crate::live_pilot` et à lui seul. Le scan `mika2653_la_vivacite_dun_pilote_a_un_lecteur_unique` reste vert sans exception ajoutée.
- **AC7.** `crates/mika-agent/src/wip_rescue.rs` porte au moins un test de pilote vivant et son contrôle négatif à graine identique — le trou que le corps du ticket nomme est refermé et attesté.
- **AC8.** L'incident B (arrêt délibéré « HALTE borne » tamponné `rescue-pipeline-verified: yes`) est explicitement hors périmètre et nommé par une ligne `Tracked in:` dans le corps de la PR.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/live_pilot.rs` (handle + type de retour + doc) | 85 |
| `crates/mika-agent/src/task_engine/engine.rs` (`declared_worktree` en fonction libre) | 8 |
| `crates/mika-agent/src/wip_rescue.rs` — production (attribution branche, paramètre, exclusion, fail-close, 3 sondes) | 57 |
| tests (`wip_rescue.rs` V1–V5, V9 ; `live_pilot.rs` V6–V8) | 120 |

Total estimé : 270 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** Trois raisons mesurables, pas une impression :

1. **Aucun champ de donnée nouveau n'est à récupérer.** `DraftPr.head_ref` existe
   déjà et `list_wip_rescue_drafts` demande déjà `headRefName` à `gh` ; la population
   DB est celle que le faucheur de mika#2249 interroge déjà ; le fichier de
   déclaration existe déjà et son lecteur aussi. Le vol **branche** des éléments en
   place, il n'en construit aucun.
2. **La traversée de vivacité n'est pas réécrite.** D1 est un handle dans un module
   qui porte déjà deux handles frères : le filtre terminal, `is_same_process_alive`
   et la lecture de `process_start_time` sont consommés tels quels. C'est ce qui
   garde la production sous les 150 lignes.
3. **Les tests n'ont pas besoin de base de données** pour V1–V5 et V9 : le paramètre
   de D3 est un `BTreeSet` littéral, exactement comme les fermetures `never_bailed` /
   `never_parked` / `never_held` du module le font déjà pour les trois exclusions
   existantes. Seuls V6–V8 prennent une fixture DB, et ils vivent dans `live_pilot.rs`
   où ce harnais existe.

**Ce qui aurait fait sauter la borne, et pourquoi la coupure est en VOLS et non en
phases.** Le vol 2 (incident B) traverse `dispatch-lib.sh`, `test-dispatch-lib.sh`,
probablement `.github/labels.yml` et, s'il faut un label neuf, un cinquième prédicat
Rust dans `select_eligible` : à vue, 150 à 200 lignes de plus, soit ~450 au total,
très au-dessus des 279. Ce n'est pas une phase du même vol parce que le vol 1 est
**mergeable et utile seul** — il ferme l'incident A, qui est le plus coûteux des deux
(une tête en vol revue par mika-qa, contre une PR remise en brouillon à la main
34 minutes plus tard) — et parce que le vol 2 ouvre un arbitrage de label que le vol 1
n'a pas à porter.

## Ordre de commit

1. **`declared_worktree` en fonction libre `pub(crate)`** — déplacement pur, aucun
   changement de comportement. Suite verte avant et après ; c'est le commit qu'on
   relit en diagonale.
2. **`live_pilot_worktrees` + V6–V8** — le handle et ses trois tests, dans le module
   qui possède la question. Mergeable seul : il ajoute un lecteur, ne change aucun
   appelant.
3. **Attribution worktree → branche dans `wip_rescue` + sonde de recensement + V9** —
   la sonde arrive **avant** l'exclusion qu'elle observe, pour que le premier tick
   après déploiement dise ce que le recensement voit avant d'agir dessus.
4. **Paramètre `live_branches`, exclusion, fail-close du tick + V1–V5** — le
   changement de comportement, en dernier, sur un substrat déjà attesté.

**Plancher du vol.** Les commits 1 et 2 seuls ne ferment rien : ils ajoutent un
lecteur que personne n'appelle. Le plancher est le commit 4 — c'est lui qui fait
passer AC1, AC3 et AC7. Un vol qui s'arrête au commit 3 livre une sonde sans
exclusion : honnête, mais l'incident A se reproduit. Si la borne de tours menace
avant le commit 4, la conduite est de **ne pas ouvrir la PR** et de laisser la
branche au prochain vol, pas de livrer les trois premiers commits comme s'ils
étaient le vol.
