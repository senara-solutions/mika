# refactor(cli) — `mika tasks` lit la vivacité d'un pilote par le lecteur unique, filtre terminal compris

- **Ticket :** senara-solutions/mika#2670 (`enhancement`, `p3-nice-to-have`, `ready`, `dispatch:loop`)
- **Branche :** `refactor/2670/cli-mika-tasks-show-lit-la-vivacit-d-un`
- **Date :** 2026-10-15
- **Lignée :** mika#2279 (la boucle qui rejoue) → mika#2156 (la topologie à deux lignes, D-2) → mika#2653 phase A (le filtre terminal, `60cbac57`) → mika#2653 phase B (PR #2669 : `live_pilot::resolve_task_pilot` devient le lecteur unique, gardé par un scan) → **ce ticket** (le troisième lecteur, resté hors du scan).
- **Doctrine :** mika#2158 (deux lecteurs d'une même question divergent en silence), mika#2201 (on route le site, on n'allowliste pas), mika#2323 (une allowlist née non vide est un tiroir).

---

## Contexte

### Ce que la phase B a fermé

`crates/mika-agent/src/live_pilot.rs` porte depuis PR #2669 le lecteur unique de
« quel pilote cette tâche porte-t-elle ? » :

```rust
pub(crate) enum TaskPilot {
    Named { pid: i64, start_time: Option<u64> },
    Children(Vec<crate::db::DispatchChild>),
    ChildrenUnreadable(anyhow::Error),
}

pub(crate) async fn resolve_task_pilot(
    db: &AsyncDatabase,
    task_id: &str,
    named: Option<&crate::db::Task>,
) -> TaskPilot
```

La traversée y est faite **une fois** : ligne nommée portant un `process_id`
non terminal → `Named` ; sinon enfants de dispatch porteurs d'un pid, **filtrés
sur `!is_terminal_task_status`** → `Children` ; erreur de base → `ChildrenUnreadable`.
Le filtre terminal est l'apport de la phase A : le pgid d'une ligne terminale est
périmé et peut désigner n'importe quoi, et comme l'`EXIT trap` de `dispatch-lib`
délivre le callback **depuis l'intérieur du wrapper encore vivant**, `delivered`
précède la sortie du processus d'une fenêtre réelle. Une ligne terminale portant
un pid vivant n'est donc pas une anomalie de laboratoire : c'est l'état nominal
pendant cette fenêtre.

Le détecteur `canonical_tokens::mika2653_la_vivacite_dun_pilote_a_un_lecteur_unique`
refuse tout fichier de production **hors de ce module** qui compose à la fois une
traversée de dispatch, le filtre terminal et `is_same_process_alive`. Son allowlist
`LIVE_PILOT_TASK_READER_ALLOWED` est vide, et son vide est asserté
(`mika2653_la_allowlist_du_lecteur_est_vide`).

### Le troisième lecteur

`crates/mika-cli/src/commands/tasks.rs::probe_pilot_liveness` (ligne 578) refait la
traversée de son côté, et **n'applique le filtre terminal dans aucune de ses deux
branches** :

- branche `trigger_type == "callback"` : lit `t.process_id` et le `start_time` du
  metadata, puis classe — sans regarder `t.status` ;
- branche tracking : `find_dispatch_children_with_pid`, puis boucle de rang — sans
  filtrer les enfants terminaux.

Conséquence exacte du constat du ticket : une ligne callback terminale portant encore
un pid vivant s'affiche « pilote vivant » dans `mika tasks list` / `get`, là où le
moteur la tient pour hors population. Trois sites de lecture : lignes 68 (`list`),
96 (`get`), 129 (garde de confirmation de `tasks cancel`).

### Correction d'une prémisse du ticket

Le ticket écrit « ce détecteur ne voit que `crates/mika-agent` ». **C'est faux, et
c'est important pour l'AC2.** Le corpus du scan est produit par
`production_sources_to_test_module()` (`canonical_tokens.rs:2458`) : il marche sur
`repo_root()/crates` **en entier**, saute `target`, les non-`.rs` et
`is_test_source_path`, et exige `/src/` dans le chemin relatif. `crates/mika-cli/src/commands/tasks.rs`
**est déjà dans le périmètre scanné**.

Ce qui laisse `probe_pilot_liveness` passer n'est pas le périmètre, c'est la
**conjonction à trois termes** du prédicat : la CLI n'appelle jamais
`is_terminal_task_status` — précisément parce qu'elle a oublié le filtre. Le scan
est donc *exact* : il ne nomme pas un lecteur qui n'a pas le troisième terme, et
l'absence du troisième terme **est** le défaut. C'est une borne réelle, pas un
périmètre manquant, et l'AC2 se répond en l'épinglant et en la disant — pas en
« étendant » un scan qui atteint déjà le crate.

Le paragraphe de résidu de `canonical_tokens.rs:1874-1878` nomme aujourd'hui
`probe_pilot_liveness` comme second lecteur « aujourd'hui » et promet ce
follow-up. Après ce ticket il affirmerait un défaut qui n'existe plus : il doit
être réécrit dans le même commit.

---

## Décisions de grooming

### D1 — On expose `resolve_task_pilot`, pas `live_pilot_for_task`

`live_pilot.rs` porte deux lecteurs publics possibles. Le verdict de haut niveau
`LivePilotVerdict` (`Alive { child_task_id, parent_task_id, pid }` / `None` /
`Unreadable { reason }`) est taillé pour une **décision moteur** : trois bras,
asymétrie fail-safe (`Unreadable` n'est jamais `Alive`).

La CLI n'a pas besoin de trois bras, elle a besoin de **quatre issues
informatives** — c'est son `PilotLiveness` : `Alive`, `Dead { pid }`,
`Unverifiable { pid, child_id }`, `NoPilot`, plus `Unknown`. Router sur
`live_pilot_for_task` serait une **régression d'affichage** :

- `LivePilotVerdict::None` fusionne « pid mort » et « aucun pilote du tout » —
  l'opérateur perdrait la distinction `Dead { pid }` / `NoPilot`, qui est
  justement ce qu'il regarde ;
- `Unreadable` ne porte pas de pid — `Unverifiable { pid }` perdrait son pid.

Le ticket prescrit la bonne forme (« exposer `resolve_task_pilot` (ou un verdict
équivalent) »), et elle colle au contrat écrit du module : **la traversée est
partagée, la classification reste chez chaque appelant**. On expose donc
`TaskPilot` et `resolve_task_pilot` en `pub`, et `classify()` / `rank()` /
les rendus de la CLI ne bougent pas.

### D2 — Les trois deltas de comportement, nommés

Router `probe_pilot_liveness` sur `resolve_task_pilot` change exactement trois
choses. Aucune n'est un effet de bord : chacune est le défaut qu'on corrige, ou
son corollaire.

1. **Ligne terminale à pid vivant** (le défaut) : elle ne passe plus le filtre,
   tombe dans la traversée des enfants (une ligne callback n'en a pas) et se lit
   `NoPilot` au lieu de `Alive`. C'est le contrôle négatif de l'AC3.
2. **Ligne non-callback portant elle-même un pid non terminal** : elle se résout
   désormais par le bras `Named` au lieu d'aller aux enfants. Par la topologie à
   deux lignes (mika#2156), cette population est quasi vide — une ligne de tracking
   ne porte jamais de `process_id` — mais le bras existe dans le lecteur unique et
   il faut le dire plutôt que de prétendre qu'il ne tire pas.
3. **La garde de `tasks cancel`** (ligne 129) n'avertit plus « pilote vivant » sur
   une ligne terminale. Ce n'est pas une perte : sur une ligne terminale,
   `cancel_task_and_kill` refuse de toute façon (statut non annulable), donc
   l'avertissement disparaît exactement là où l'annulation est un no-op. C'est le
   même raisonnement que celui déjà écrit au site de `live_pilot_for_task`.

`start_time_from_metadata` (ligne 549 de la CLI) devient **mort** : c'était une
seconde implémentation du lecteur de metadata partagé, et le bras `Named` lit
désormais `Database::process_start_time_from_metadata` (qui est `pub(crate)` dans
`mika-agent`, donc inatteignable depuis la CLI — raison supplémentaire pour que
la lecture vive dans la traversée). On le supprime.

### D3 — La fermeture est un **narrowing de visibilité**, pas un second scan

Une fois la CLI routée, l'unique appelant hors crate de
`AsyncDatabase::find_dispatch_children_with_pid` (`async_db.rs:1175`, `pub`)
disparaît. Les appelants restants sont tous dans `mika-agent` :
`tracking_cleanup.rs:239`, `live_pilot.rs:278`, `engine.rs:3393` — et aucun
`tests/eval/*`. On la passe en `pub(crate)`, ainsi que la méthode sync
`Database::find_dispatch_children_with_pid` (`db/tasks.rs:2208`).

C'est une garde **plus forte** qu'un scan de source et moins chère : un site hors
crate ne peut plus recomposer la traversée du tout — ça ne compile pas. Un scan
dit « tu as écrit ça » après coup ; `pub(crate)` dit « tu ne peux pas l'écrire ».
Pré-1.0, le narrowing part sans compatibilité ascendante (§ *Versioning* de
`CLAUDE.md`), et il est documenté dans le corps de PR.

`is_same_process_alive` (`process_liveness.rs:51`) **reste `pub`** : de nombreux
`crates/mika-agent/tests/eval/*` l'appellent. Elle n'est pas le danger — la
composition l'est, et la composition passe par la traversée.

### D4 — Ce qu'on NE livre pas, et pourquoi

Le plan a d'abord prévu un **second détecteur** : un scan plus large, sur la
conjonction `traversée × is_same_process_alive` (sans le terme terminal), qui
aurait attrapé la CLI dans son état actuel. Il est **abandonné**, pour une
raison de doctrine et non de budget : il tirerait sur
`engine.rs::dispatch_liveness`, qui est une divergence **délibérée et écrite au
site** (mika#2156 D-2 : 1 146 des 1 147 enfants porteurs d'un pid mesurés sont
`delivered`, et `delivered` ne prouve pas un processus mort — filtrer y viderait
la population). Il faudrait donc l'excepter en allowlist dès sa naissance :
exactement le tiroir que mika#2323 interdit.

Le ticket autorise explicitement la branche moins chère — « ou le dire
explicitement s'il reste borné ». C'est celle-ci, et D3 lui donne des dents que
le scan abandonné n'aurait pas eues.

---

## Conception

### L1 — `live_pilot.rs` : la traversée devient publique, et son contrat le dit

`TaskPilot` et `resolve_task_pilot` passent de `pub(crate)` à `pub`. Le
doc-comment du module et celui de `resolve_task_pilot` gagnent un paragraphe qui
nomme :

- le **troisième appelant** (`mika-cli`, surface d'affichage opérateur) ;
- la **frontière affichage / décision** : la traversée est partagée, la
  classification reste chez l'appelant, parce que le moteur veut trois bras
  fail-safe et l'opérateur quatre issues informatives (D1) ;
- la **borne du scan**, telle qu'elle est : le corpus inclut `crates/mika-cli`,
  la conjonction à trois termes est le prédicat, et un lecteur qui *omet* le
  filtre terminal n'est pas nommé par le scan — il est nommé par la revue, et
  depuis ce ticket il est empêché par `pub(crate)` sur la traversée (D3).

Aucun changement de corps de fonction ici : seule la visibilité et la
documentation bougent.

### L2 — `mika-cli/src/commands/tasks.rs` : `probe_pilot_liveness` routée

La fonction devient une classification à trois bras sur `TaskPilot`, sans aucune
traversée propre :

```rust
async fn probe_pilot_liveness(db: &AsyncDatabase, settings: &Settings, t: &Task) -> PilotLiveness {
    match mika_agent::live_pilot::resolve_task_pilot(db, &t.id, Some(t)).await {
        TaskPilot::Named { pid, start_time } => match start_time {
            Some(st) => classify(pid, None, st, settings, &t.id),
            None => PilotLiveness::Unverifiable { pid, child_id: None },
        },
        TaskPilot::Children(children) => { /* boucle de rang existante, inchangée */ }
        TaskPilot::ChildrenUnreadable(_) => PilotLiveness::Unknown,
    }
}
```

Points de vigilance :

- le test `trigger_type == "callback"` **disparaît** : la discrimination est
  désormais celle du lecteur unique (un `process_id` non terminal sur la ligne
  elle-même), ce qui est le point du ticket ;
- l'identifiant passé à `classify` pour l'âge du log de session reste `&t.id` sur
  le bras `Named` et `&child.id` dans la boucle — identique à aujourd'hui ;
- `classify()` (ligne 630), `rank()`, `summary_annotation` (674) et
  `format_pilot_detail` (691) ne bougent pas, et leurs tests unitaires existants
  restent verts tels quels ;
- `start_time_from_metadata` (549) est supprimée (D2).

### L3 — La traversée se referme au compilateur

`Database::find_dispatch_children_with_pid` (`db/tasks.rs:2208`) et son wrapper
`AsyncDatabase::find_dispatch_children_with_pid` (`async_db.rs:1175`) passent en
`pub(crate)`, chacun avec une phrase de doc disant *pourquoi* : hors du crate, la
seule porte vers « quel pilote cette tâche porte-t-elle ? » est
`live_pilot::resolve_task_pilot`, filtre terminal compris.

### L4 — `canonical_tokens.rs` : le périmètre est épinglé, le résidu est réécrit

Deux gestes dans le même commit :

1. **Une assertion mika#2670** —
   `mika2670_le_corpus_du_scan_contient_la_cli` : le corpus produit par
   `production_sources_to_test_module()` contient au moins un chemin sous
   `crates/mika-cli/src/`. C'est l'anti-vacuité du *périmètre* : si un jour le
   walk se restreint à `mika-agent`, la phrase « le scan voit déjà la CLI »
   devient fausse en silence, et la borne documentée en L1 devient un mensonge.
   L'assertion rougit ce jour-là.
2. **Le paragraphe de résidu (1874-1878) réécrit** : il ne nomme plus
   `probe_pilot_liveness` comme second lecteur, il énonce la borne — la
   conjonction à trois termes, ce qu'elle attrape, ce qu'elle n'attrape pas, et
   le fait qu'un lecteur hors crate est désormais fermé par visibilité (D3)
   plutôt que par scan.

### L5 — Les tests de la CLI : le contrôle négatif et son jumeau positif

Dans le `#[cfg(test)] mod tests` déjà présent (ligne 863), avec un `AsyncDatabase`
en mémoire (`Database::open_in_memory` + `AsyncDatabase::new_with_agent`, tous
deux `pub`), un helper de semis paramétré unique (parent/enfant, statut, pid,
start_time — `NewTask` porte ~25 champs, d'où un helper plutôt que deux), et un
pid vivant réel + son start_time lu par
`task_engine::process_liveness::read_process_start_time` (`pub`), sur le modèle
de `self_pid_and_start()` dans les tests de `live_pilot.rs` :

- **contrôle négatif (AC3)** : ligne callback **terminale** portant le pid du
  processus de test (vivant, start_time lisible) ⇒ la lecture n'est **pas**
  `Alive`. C'est le défaut, en une assertion.
- **jumeau positif (anti-vacuité)** : même ligne, statut **non terminal** ⇒
  `Alive`. Sans lui, un contrôle négatif vert ne distingue pas « le filtre
  mord » de « le semis n'a jamais produit de pid vivant » — un test visant un
  cadavre se lit exactement comme un arbre propre (mika#2103/#2205).

Les deux cas sont gated `#[cfg(target_os = "linux")]`, comme leurs homologues de
`live_pilot.rs` : ils dépendent du champ 22 de `/proc`. `#[tokio::test]` est déjà
en usage dans `mika-cli` (`src/supervision.rs`, `src/tui/input.rs`), et
`mika-agent` y est une dépendance ordinaire.

---

## Étapes d'implémentation

Ordre de commit — chaque étape compile et laisse la suite verte :

1. **`crates/mika-agent/src/live_pilot.rs`** — `TaskPilot` et `resolve_task_pilot`
   en `pub` ; doc du module et de la fonction : troisième appelant, frontière
   affichage/décision, borne du scan (L1). Rien d'autre.
2. **`crates/mika-cli/src/commands/tasks.rs`** — `probe_pilot_liveness` routée sur
   `resolve_task_pilot` (trois bras) ; `start_time_from_metadata` supprimée ;
   `classify`/`rank`/rendus intacts (L2).
3. **`crates/mika-agent/src/db/tasks.rs` + `src/async_db.rs`** — les deux
   `find_dispatch_children_with_pid` en `pub(crate)` + la phrase de doc (L3).
   Après l'étape 2 seulement : avant, ça ne compilerait pas.
4. **`crates/mika-agent/src/canonical_tokens.rs`** — assertion
   `mika2670_le_corpus_du_scan_contient_la_cli` + réécriture du paragraphe de
   résidu 1874-1878 (L4).
5. **Tests `mika-cli`** — helper de semis + contrôle négatif + jumeau positif (L5).
6. `cargo fmt`, `cargo clippy`, `cargo test`, les lints du dépôt et
   `scripts/verify-pipeline.sh origin/main`.

Plancher du vol : les étapes 1, 2 et 5. Sans l'étape 5, le défaut est corrigé
sans preuve ; sans l'étape 2, rien n'est corrigé. Les étapes 3 et 4 sont le
durcissement et la vérité documentaire — elles tiennent dans le même vol (voir
§ *Taille estimée*), mais si le budget de tours se tendait, c'est dans cet ordre
qu'elles se replient.

---

## Contrat de vérification

| vérification | où | ce qui rougit |
|---|---|---|
| contrôle négatif AC3 | `mika-cli/src/commands/tasks.rs` tests | ligne terminale + pid vivant lue `Alive` |
| jumeau positif | idem | ligne non terminale + pid vivant **non** lue `Alive` (semis vacant) |
| corpus du scan ⊇ CLI | `canonical_tokens.rs` (mika#2670) | le walk cesse d'inclure `crates/mika-cli/src/` |
| lecteur unique | `mika2653_la_vivacite_dun_pilote_a_un_lecteur_unique` (existant) | un quatrième site recompose la traversée + filtre + liveness |
| allowlist vide | `mika2653_la_allowlist_du_lecteur_est_vide` (existant) | une entrée ajoutée en silence |
| contrôle de bonne foi du scan | `mika2653_le_scan_du_lecteur_voit_un_second_site` (existant) | le prédicat cesse de voir le positif de la phase A |
| traversée fermée hors crate | le compilateur | un site hors `mika-agent` appelle `find_dispatch_children_with_pid` |
| rendus inchangés | tests unitaires existants de `summary_annotation` / `format_pilot_detail` | une régression d'affichage |

État attendu des gardes existantes : **toutes vertes sans modification**. Si
`mika2653_la_vivacite_dun_pilote_a_un_lecteur_unique` tire après l'étape 2, c'est
que la CLI a gardé une traversée propre — la route n'est pas faite.

---

## Risques nommés

- **R1 — `resolve_task_pilot` devenue `pub` est copiée plutôt qu'appelée.** Le scan
  existant tire sur une recomposition complète, et D3 ferme la traversée au
  compilateur : les deux chemins sont couverts. Le résiduel est un site qui
  réimplémenterait la requête SQL de zéro — hors de portée de ce ticket, et rendu
  visible par la doc de L1.
- **R2 — le narrowing `pub(crate)` casse un consommateur externe.** Pré-1.0 assumé
  (§ *Versioning*), et l'inventaire des appelants a été fait : trois en crate,
  zéro dans `tests/eval`, un seul hors crate (la CLI, qu'on route). À revérifier
  d'un `cargo build --workspace` à l'étape 3.
- **R3 — le contrôle négatif est vacant.** Exactement ce que le jumeau positif de
  L5 empêche ; c'est pour ça qu'il est dans le plancher du vol.
- **R4 — le bras `Named` change l'affichage d'une population non vide.** Jugé
  quasi vide par la topologie à deux lignes, pas mesuré. Le delta est nommé en
  D2.2 et porté dans le corps de PR ; aucune décision moteur n'en dépend
  (hors périmètre du ticket).

---

## Hors périmètre

- **Aucun effet sur une décision du moteur.** `live_pilot_for_task`,
  `live_pilot_for_issue`, `dispatch_liveness` et la population de mika#2156 D-2
  ne bougent pas. C'est une surface d'affichage opérateur — d'où `p3`.
- **Pas de second détecteur** sur la conjonction `traversée × liveness` (D4) : il
  naîtrait avec une exception en allowlist pour `dispatch_liveness`, contre
  mika#2323.
- **Pas de changement de `PilotLiveness`** : ni variante ajoutée, ni `rank()`
  retouché, ni rendu modifié.
- **Pas de retouche de `is_same_process_alive`** : elle reste `pub` pour
  `tests/eval`.

---

## Definition of Done

- `crates/mika-agent/src/live_pilot.rs` expose `TaskPilot` et
  `resolve_task_pilot` en `pub`, et sa doc nomme le troisième appelant, la
  frontière affichage/décision et la borne réelle du scan.
- `probe_pilot_liveness` ne fait plus aucune traversée de dispatch propre : elle
  classe un `TaskPilot`. `start_time_from_metadata` est supprimée de la CLI.
- Les deux `find_dispatch_children_with_pid` (sync et async) sont `pub(crate)`,
  avec la phrase de doc qui nomme la porte unique.
- `canonical_tokens.rs` porte l'assertion de périmètre mika#2670, et son
  paragraphe de résidu n'affirme plus un défaut corrigé.
- Les tests de la CLI portent le contrôle négatif et son jumeau positif.
- `cargo fmt --check`, `cargo clippy` (sans warning nouveau), `cargo test`
  (workspace), les lints du dépôt et `scripts/verify-pipeline.sh origin/main`
  passent.
- Le corps de PR déclare le narrowing de visibilité comme changement cassant
  pré-1.0 et nomme les trois deltas de comportement de D2.

## Acceptance criteria

Le corps du ticket n'a pas de section `## Acceptance criteria` ; les trois
critères ci-dessous transcrivent sa section `## Attendu`, puce par puce.

- **AC1 — `resolve_task_pilot` est exposée hors du crate `mika-agent`, et
  `probe_pilot_liveness` y est routée.** `TaskPilot` et `resolve_task_pilot` sont
  `pub` ; `probe_pilot_liveness` est une classification à trois bras sur
  `TaskPilot` et ne contient plus ni appel à
  `find_dispatch_children_with_pid`, ni lecture propre du `start_time`.
  Vérifiable par lecture du diff **et** par le compilateur : la traversée est
  `pub(crate)`, donc une route incomplète ne compile pas.
- **AC2 — la borne du détecteur est dite explicitement.** Le ticket laisse le
  choix « étendre, ou dire que ça reste borné » : on dit. Le périmètre du scan
  inclut déjà `crates/mika-cli/src/` (c'est la conjonction à trois termes qui
  laissait passer la CLI, pas le périmètre) ; ce fait est épinglé par
  `mika2670_le_corpus_du_scan_contient_la_cli`, énoncé dans la doc de
  `live_pilot.rs`, et le paragraphe de résidu de `canonical_tokens.rs` est
  réécrit pour le dire au lieu de nommer un défaut corrigé. La fermeture d'un
  lecteur hors crate est portée par `pub(crate)` sur la traversée, pas par un
  second scan.
- **AC3 — contrôle négatif : une ligne terminale avec un pid vivant n'est pas
  affichée « vivante ».** Test dans `crates/mika-cli/src/commands/tasks.rs` :
  ligne callback terminale portant un pid réellement vivant et son start_time
  réel ⇒ la lecture n'est pas `Alive`. Accompagné du jumeau positif (même ligne,
  statut non terminal ⇒ `Alive`) pour que le vert du négatif ne puisse pas venir
  d'un semis vacant.

## Fire-Disposition

Ce plan livre des détecteurs (une assertion de périmètre mika#2670, deux contrôles
dans la CLI), la section est donc requise.

**Option retenue : (a) exception nommée en allowlist — avec ZÉRO entrée.**

- `LIVE_PILOT_TASK_READER_ALLOWED` (`canonical_tokens.rs:1820`) est **déjà déclarée
  et déjà vide**, et son vide est **déjà asserté** par
  `mika2653_la_allowlist_du_lecteur_est_vide`. Ce plan **n'y ajoute aucune
  entrée** et ne la touche pas.
- C'est possible parce qu'on **route le site** au lieu de l'excepter (doctrine
  mika#2201) : après l'étape 2, il n'y a plus de violation à excepter. Une
  allowlist née non vide est un tiroir pour la violation suivante (mika#2323) —
  et ici le tiroir reste fermé parce qu'il n'a rien à recevoir.
- La nouvelle assertion de périmètre **n'a pas d'allowlist du tout** : elle porte
  une seule proposition (« le corpus contient la CLI »), dont la seule issue
  légitime est vraie. Un jour où elle serait fausse est un jour où la borne
  documentée est fausse — c'est précisément ce qu'on veut voir rougir.
- Les deux contrôles de la CLI partent **armés**, sans `#[ignore]` : ils passent
  dès l'étape 2, puisque le défaut est corrigé dans le même vol. L'auto-nettoyage
  est assuré par le jumeau positif : si le semis cesse de produire un pid vivant
  lisible, c'est lui qui rougit, pas le négatif qui verdit en silence.
- Le détecteur abandonné de D4 **aurait** exigé une entrée d'allowlist pour
  `engine.rs::dispatch_liveness` dès sa naissance. C'est l'une des deux raisons
  de son abandon.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/live_pilot.rs` (visibilité + doc du contrat) | 25 |
| `crates/mika-cli/src/commands/tasks.rs` (route + suppression du lecteur mort) | 50 |
| `crates/mika-agent/src/db/tasks.rs` + `src/async_db.rs` (narrowing + doc) | 10 |
| `crates/mika-agent/src/canonical_tokens.rs` (assertion de périmètre + résidu réécrit) | 40 |
| tests (`crates/mika-cli/src/commands/tasks.rs`, `mod tests`) | 105 |

Total estimé : 230 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Un seul vol, et voici pourquoi il tient.** 230 estimées contre une borne de 243 :
la marge est mince mais réelle, et elle est appuyée par trois faits de forme.
(1) Aucun livrable n'est une création de module : quatre des cinq sont des
modifications locales dans des fichiers déjà lus et compris, et la plus grosse
ligne du tableau — les tests — réutilise le patron de semis de `live_pilot.rs`
plutôt que de l'inventer. (2) La partie « conception » du travail est faite dans
ce plan, pas dans le vol : la forme de `probe_pilot_liveness` routée est écrite
en L2, l'ordre de commit est fixé, et le seul inconnu de compilation (l'inventaire
des appelants pour le narrowing) a déjà été relevé. (3) Le périmètre a **déjà été
coupé une fois** pour tenir : le second détecteur de D4 (~70 lignes de scan plus
son contrôle de bonne foi) est abandonné sur un motif de doctrine qui coïncide
avec le budget, ce qui fait passer l'estimation de ~300 à 230.

Le total est très en dessous du seuil par PR de 1 000 lignes ; c'est la borne par
vol qui mord, et c'est elle qui a dicté la coupe de D4. Si le vol dérive au-delà
de la borne en cours de route, le repli est celui nommé au § *Étapes
d'implémentation* : les étapes 3 et 4 sortent du vol et partent en `Tracked in:`,
le plancher (1, 2, 5) reste mergeable seul et la correction du défaut est
livrée avec sa preuve.
