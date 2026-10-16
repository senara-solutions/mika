# test(flaky) — `SETTLE_MS` est une horloge murale là où le code a déjà un signal de fin : rendre les deux barrières déterministes

- **Ticket :** senara-solutions/mika#2677
- **Labels :** `bug`, `p3-nice-to-have`, `ready`, `dispatch:loop`
- **Périmètre :** `crates/mika-agent/src/skills/executor.rs` (un fichier), plus une solution-doc
- **Bornes posées par le ticket :** tests seulement, aucun changement de comportement de production

---

## Contexte

`skills::executor::tests::mika2532::the_stderr_of_a_crash_on_a_terminal_row_is_persisted`
a rendu rouge deux fois en CI (job `Check`, étape `Test`) et vert sur relance les
deux fois, sans changement de code : PR #2659 le 2026-10-05, PR #2676 le
2026-10-07 (run 37582008160, tentative 1). Message :

```
panicked at crates/mika-agent/src/skills/executor.rs:13369:18:
the cause of a pre-result crash must reach the row
```

Aucune des deux PR ne touche `skills/executor.rs`.

### Le mécanisme, et ce que la lecture du ticket laisse de côté

Le ticket décrit exactement la chaîne : le test lance un handler via
`spawn_long_running_exec`, puis `await_handler_failure` (`:13316`) interroge la
ligne toutes les 20 ms jusqu'à une **échéance à l'horloge murale** de
`SETTLE_MS = 4_000` (`:13282`, même valeur à `:13567`). Sous charge, le
sous-processus plus l'écriture en base dépassent 4 s, l'aide rend `None`, et le
test rouge sur un comportement correct.

Ce que la lecture ne dit pas, et qui décide de la forme du correctif : **le
signal de fin existe déjà, et il est jeté.** `spawn_long_running_exec` (`:5055`)
fait `tokio::spawn(async move { … });` et laisse tomber le `JoinHandle`. Or tout
ce que ces onze tests observent — ou nient — est écrit **dans ce corps-là**, en
`await`, avant qu'il ne retourne :

| écriture | site | consommée par |
|---|---|---|
| `set_task_handler_failure` | `:5292` | les 5 tests de `mod mika2532`, et la barrière `require_handler_failure` |
| `log_audit_event` (`pilot_launcher_health` = `dead`) | `:5397` | `a_launcher_that_never_started_is_classified_apart`, `a_dead_launcher_does_not_record_a_recovery` |
| `record_launcher_recovery` (§16) | `:5439` | `a_clean_dispatch_after_a_death_records_the_recovery`, et l'absence niée par `a_clean_window_records_no_recovery` |

Et il n'y a **aucun `tokio::spawn` imbriqué** : dans tout le fichier,
`grep -n "tokio::spawn" crates/mika-agent/src/skills/executor.rs` ne rend que
`:5063`, le seul autre spawn étant un `spawn_blocking` à `:1217`, hors de ce
chemin. Comme les méthodes d'`AsyncDatabase` attendent la réponse de leur thread
dédié, un `set_task_handler_failure` qui a retourné est une écriture **commise**.

Donc : **joindre ce handle est une barrière complète et serrée pour chacune des
trois écritures.** `SETTLE_MS` n'est pas un nombre mal calibré, c'est une horloge
murale qui tient la place d'un signal que le code possède. Relever 4 000 à 30 000
soignerait le symptôme en gardant la cause — un verdict qui dépend de la charge
du runner.

### Pourquoi ce n'est pas réductible à « élargir l'échéance »

Le CA1 autorise explicitement le repli (« à défaut, l'échéance est assez large
pour la CI et le reste explicitement »). Il n'est pas pris, pour une raison
mesurable et une raison de forme :

- l'échéance large reste **sur le chemin critique du vert** : sa valeur doit
  excéder la durée réelle, donc la charge du runner garde une voix dans le
  verdict — elle est seulement moins souvent écoutée ;
- les deux contrôles négatifs de `mod mika2634` dorment aujourd'hui
  `NEGATIVE_CONTROL_MARGIN_MS = 250` ms (`:13581`) après leur barrière, et le
  doc-comment de cette constante nomme précisément ce qu'elle couvre : la
  fenêtre d'instructions entre le retour de `set_task_handler_failure` et le
  `log_audit_event` du bloc launcher-mort, **deux `await` du même task spawné**.
  Le join couvre les deux bouts par construction : la marge devient un sommeil
  qui ne couvre plus rien, et elle disparaît au lieu d'être recalibrée.

---

## Décisions de grooming

### D1 — Le signal est le `JoinHandle`, et la production le rend

`spawn_long_running_exec` passe de `-> ()` à `-> tokio::task::JoinHandle<()>` :
on retire le `;` final du `tokio::spawn(…)` et on ajoute un paragraphe de
doc-comment nommant mika#2677.

**Ce n'est pas un changement de comportement, et les trois propriétés qui le
garantissent ont été vérifiées plutôt que supposées :**

1. **Laisser tomber un `JoinHandle` détache, il n'annule pas.** Le task continue
   exactement comme aujourd'hui. Aucun appelant n'est obligé de changer.
2. **`JoinHandle` n'est pas `#[must_use]`** — dans `tokio-1.53.1`
   (`src/runtime/task/join.rs:306`) l'attribut porte sur `abort_handle()`, pas
   sur le type. Un appel en position d'instruction compile sans avertissement.
3. **Le workspace ne déclare pas `unused_results`** : `[workspace.lints.clippy]`
   (`Cargo.toml:256`) ne porte qu'une règle, `significant_drop_in_scrutinee`.

Les **quatre** sites de production restent donc **mot pour mot** :
`executor.rs:5003`, `task_engine/dispatcher.rs:4488`,
`server/verdict_handler.rs:1202`, `server/ready_label_handler.rs:1621`. Le diff
de la région production se réduit à la ligne de signature, le `;` retiré et le
doc-comment.

### D2 — Les sondages deviennent des lectures simples

Une fois le handle joint, il n'y a plus rien à attendre. Les aides perdent leur
boucle, pas leur contrat :

- `mod mika2532` : `await_handler_failure` → `read_handler_failure` (une lecture,
  pas de boucle, pas d'échéance) ;
- `mod mika2634` : `await_launcher_health` (`:13627`) **disparaît** — son
  enveloppe de sondage n'avait d'autre rôle que l'attente, et `launcher_health`
  (`:13615`), la lecture unique qu'elle appelait, existe déjà et devient le seul
  accès.

### D3 — `NEGATIVE_CONTROL_MARGIN_MS` disparaît, et `require_handler_failure` garde son contrat en perdant son horloge

La constante (`:13581`) et ses trois sommeils (`:13758`, `:13802`, `:14237`)
tombent : le join couvre la fenêtre que son doc-comment décrit.

`require_handler_failure` (`:13655`) **reste**, parce que ce qu'elle porte n'est
pas une attente mais une anti-vacuité — classe mika#2205 : un contrôle ne peut
rien conclure d'une **absence** avant d'avoir établi que le chemin a tourné. Son
corps devient une lecture unique suivie d'une assertion, et son message
d'échec change de cause nommée : non plus « le chemin n'a pas tourné en 4 s »
mais « le task de surveillance a joint sans avoir écrit `$.handler_failure` ».
C'est la même garde, sur une prémisse désormais vraie par construction.

De même la boucle-marqueur de `a_clean_window_records_no_recovery` (`:14228`)
devient un `assert!(marker.exists(), …)` posé après le join.

### D4 — Une borne de blocage reste, et ce n'est PAS un budget de décantation

`handle.await` nu ferait pendre le binaire de test jusqu'au timeout du job CI si
un sous-processus ne sortait jamais — un rouge plus mauvais qu'un panic. Le join
est donc enveloppé dans `tokio::time::timeout`, budget
`HANDLER_JOIN_BUDGET_SECS = 60`.

La différence avec `SETTLE_MS` est structurelle, pas numérique, et le
doc-comment de la constante doit la dire : `SETTLE_MS` était sur le chemin
critique du vert ; ce budget n'est atteignable que par un sous-processus
réellement pendu. Les onze handlers de ces deux modules écrivent une ligne
courte et sortent — ils finissent, d'après le ticket lui-même, « largement sous
100 ms sur ce harnais ».

### D5 — Une seule aide, chez le parent

`mod mika2532` et `mod mika2634` font tous deux `use super::*;` et ont besoin de
la même barrière. Elle est posée une fois dans `mod tests`, juste avant
`mod mika2532`.

Les trois aides déjà dupliquées entre les deux modules (`db()`, `handler()`,
`callback_row()`) sont **laissées en place** : elles préexistent et les
dédupliquer n'est pas ce ticket. Mais une **nouvelle** duplication de la
barrière serait un second site pour la chose même que ce ticket existe pour
rendre unique.

### D6 — Le budget de join est paramétrable, parce que sinon son refus est invérifiable

Si `join_handler_monitor` avalait silencieusement un timeout ou un panic, les
onze tests deviendraient verts-par-défaut d'observation — la classe que la
barrière est censée fermer, déplacée d'un cran. Les deux `.expect()` (sur le
`timeout`, puis sur le `JoinError`) en sont la réponse structurelle ; pour la
rendre **observable**, l'aide prend son budget en argument
(`join_handler_monitor_within(handle, budget)`), et l'enveloppe à budget par
défaut l'appelle. Un contrôle `#[should_panic]` dédié passe alors 100 ms contre
un handler qui dort 5 s, et assert la forme du message. Coût : ~100 ms de test.

---

## Conception

### L1 — Production : trois lignes et un paragraphe

`crates/mika-agent/src/skills/executor.rs:5055`

```rust
/// … (doc-comment existant conservé)
///
/// # mika#2677 — le handle est rendu
///
/// Le `JoinHandle` du task de surveillance est retourné pour qu'un test puisse
/// le **joindre** au lieu de deviner une échéance murale : les trois écritures
/// observables (`set_task_handler_failure`, la ligne d'audit
/// `pilot_launcher_health`, `record_launcher_recovery`) sont toutes `await`ées
/// dans ce corps, et il n'y a aucun `tokio::spawn` imbriqué — donc le join est
/// une barrière complète. Laisser tomber le handle **détache** (il n'annule
/// pas), donc aucun appelant n'a à changer : les quatre sites de production le
/// laissent tomber comme avant.
pub(crate) fn spawn_long_running_exec(
    cmd_path: PathBuf,
    skill_dir: PathBuf,
    input: serde_json::Value,
    task_id: String,
    db: AsyncDatabase,
    github_token: Option<String>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // … corps inchangé, byte pour byte …
    })   // ← le `;` de `:5443` retiré
}
```

### L2 — L'aide partagée, dans `mod tests`, avant `mod mika2532`

```rust
// ───────────────────────────────────────────────────────────────────────────
// mika#2677 — la barrière des tests de handler long : un JOIN, pas une horloge
// ───────────────────────────────────────────────────────────────────────────

/// Borne sur un **join**, jamais un budget de décantation (mika#2677).
///
/// `SETTLE_MS` (4 s, retirée par ce ticket) était sur le chemin critique du
/// vert : sa valeur devait excéder la durée réelle, donc la charge du runner
/// avait une voix dans le verdict — c'est ce qui a rendu rouge deux fois en CI
/// un comportement correct. Ce budget-ci n'est atteignable que par un
/// sous-processus réellement pendu ; le chemin normal ne s'en approche jamais.
/// Il existe parce qu'un `handle.await` nu ferait pendre le binaire de test
/// jusqu'au timeout du job, ce qui est un rouge moins lisible qu'un panic.
const HANDLER_JOIN_BUDGET_SECS: u64 = 60;

/// Joindre le task de surveillance de [`spawn_long_running_exec`].
///
/// **C'est une barrière complète pour tout ce que ce task écrit** : les trois
/// écritures observables sont `await`ées dans son corps, et il n'y a aucun
/// `tokio::spawn` imbriqué (`:5063` est le seul du fichier). *Un `tokio::spawn`
/// ajouté plus tard dans ce corps rouvrirait le trou en silence* — le join
/// retournerait avant l'écriture du task imbriqué. Si vous en ajoutez un,
/// l'écriture qu'il porte a besoin de son propre signal.
async fn join_handler_monitor(handle: tokio::task::JoinHandle<()>) {
    join_handler_monitor_within(handle, Duration::from_secs(HANDLER_JOIN_BUDGET_SECS)).await;
}

/// Variante à budget explicite — elle existe pour que le refus de l'aide soit
/// **testable** (D6), pas pour être appelée par les tests ordinaires.
async fn join_handler_monitor_within(handle: tokio::task::JoinHandle<()>, budget: Duration) {
    tokio::time::timeout(budget, handle)
        .await
        .expect("mika#2677 — the handler's monitor task never finished: its subprocess is hung")
        .expect("the handler's monitor task panicked");
}
```

Puis le contrôle de D6, immédiatement après :

```rust
/// **Anti-vacuité de la barrière elle-même.** Sans ce contrôle, une aide qui
/// avalerait son timeout rendrait les onze tests verts par défaut
/// d'observation — la classe mika#2205, déplacée d'un cran.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[should_panic(expected = "its subprocess is hung")]
async fn mika2677_la_barriere_refuse_un_handler_pendu() {
    // handler : `sleep 5` ; budget : 100 ms ; le panic est le résultat attendu.
}
```

### L3 — `mod mika2532` : cinq sites, une aide

- `const SETTLE_MS` (`:13282`) : **supprimée**.
- `await_handler_failure` (`:13316`) → `read_handler_failure(&db, &task_id) ->
  Option<serde_json::Value>` : le corps garde son `if let` chaîné, perd la
  boucle, l'`Instant` et le `sleep`. Son doc-comment dit que l'absence rendue
  est désormais une **absence constatée après le join**, non un épuisement de
  budget — ce qui est exactement ce dont T2 a besoin pour conclure.
- Les cinq sites (`:13358`, `:13412`, `:13440`, `:13481`, `:13516`) deviennent :

```rust
let monitor = spawn_long_running_exec(/* … inchangé … */);
join_handler_monitor(monitor).await;

let failure = read_handler_failure(&db, &task_id)
    .await
    .expect("the cause of a pre-result crash must reach the row");
```

Les assertions elles-mêmes — exit, `stderr`, `captured_at`, `tasks.result`
intact, `status == "completed"`, le scrub de T4, l'absence de clé de T5 — ne
bougent pas d'un octet.

### L4 — `mod mika2634` : six sites, trois aides, deux constantes retirées

- `const SETTLE_MS` (`:13567`) et `const NEGATIVE_CONTROL_MARGIN_MS` (`:13581`) :
  **supprimées**, avec les trois sommeils `:13758`, `:13802`, `:14237`.
- `await_launcher_health` (`:13627`) : **supprimée**. Ses trois appels
  (`:13716`, `:14191`, `:14274`) deviennent `launcher_health(&db, &task_id)`,
  la lecture unique déjà en place à `:13615`.
- `require_handler_failure` (`:13655`) : corps réécrit en lecture unique +
  `assert!`, doc-comment conservé dans sa substance (classe mika#2205) et sa
  cause nommée mise à jour (D3).
- Les six sites (`:13706`, `:13746`, `:13792`, `:14181`, `:14217`, `:14264`)
  capturent le handle et le joignent.
- `a_clean_window_records_no_recovery` : la boucle-marqueur `:14228-14236`
  devient `assert!(marker.exists(), …)` après le join.

Les quatre scans de source du module et les assertions de format de fil
(`after_value`, `tool_name`) ne sont pas touchés.

### L5 — La solution-doc

`docs/solutions/best-practices/une-echeance-murale-ne-remplace-pas-un-signal-de-fin-2026-10-15.md`,
avec le frontmatter YAML de l'arbre (`module`, `tags`, `problem_type`). La leçon
tient en une ligne — *quand le code possède déjà un signal de fin, une échéance
murale n'est pas une approximation de ce signal, c'est un second verdict qui
dépend de la charge* — et les trois marqueurs de reconnaissance du motif : une
constante nommée `*_SETTLE_*` / `*_MARGIN_*`, un `JoinHandle` laissé tomber à la
production, une boucle de sondage dont le doc-comment plaide pour sa valeur.

### Ce que ce plan ne touche pas

- Le corps du task de surveillance : zéro ligne.
- Les quatre sites d'appel de production : zéro ligne.
- Les assertions de comportement des onze tests : zéro octet.
- Les trois aides dupliquées entre les deux modules (D5).
- Tout autre `SETTLE_MS` du dépôt : il n'y en a pas
  (`grep -rln SETTLE_MS crates/ scripts/ skills/` → `executor.rs` seul).

---

## Étapes d'implémentation

1. `executor.rs:5055` / `:5443` — signature, `;` retiré, paragraphe de
   doc-comment (L1). `cargo check -p mika-agent` : les quatre sites de
   production doivent compiler **sans modification**. Si l'un réclame un
   `let _ =`, c'est que D1 point 2 ou 3 est faux : s'arrêter et remonter.
2. L'aide partagée + sa variante à budget + le contrôle `#[should_panic]` (L2).
   Faire tourner ce seul test : il doit passer (c'est-à-dire paniquer).
3. `mod mika2532` (L3), puis `cargo test -p mika-agent skills::executor::tests::mika2532`.
4. `mod mika2634` (L4), puis `cargo test -p mika-agent skills::executor::tests::mika2634`.
5. W3 et W4 — les deux mutations, exécutées, observées, **révertées** (contrat de
   vérification ci-dessous). Reporter la sortie rouge réelle dans le corps de PR.
6. La solution-doc (L5), puis `scripts/sync-agent-docs.sh` si le chemin tombe
   dans la fenêtre que `build.rs` copie (sinon le job `docs-sync` rougit).
7. Suite complète, `cargo clippy --all-targets`, `cargo fmt`,
   `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main`.

---

## Contrat de vérification

### W1 — Le test du ticket, vert et déterministe

`cargo test -p mika-agent skills::executor::tests::mika2532` : les cinq tests
verts. `the_stderr_of_a_crash_on_a_terminal_row_is_persisted` en particulier.

### W2 — `mod mika2634` entier, vert

`cargo test -p mika-agent skills::executor::tests::mika2634` : tous verts, y
compris les trois contrôles négatifs qui ont perdu leur marge de 250 ms et les
deux tests §16.

### W3 — CA3, la mutation requise : la stderr n'atteint plus la ligne

Neutraliser la production (`:5291-5306`) en remplaçant l'appel à
`set_task_handler_failure` par un `Ok(())` immédiat, puis faire tourner
`the_stderr_of_a_crash_on_a_terminal_row_is_persisted`. Il **doit** rougir, et le
message doit nommer l'absence constatée (`the cause of a pre-result crash must
reach the row`), non un épuisement de budget. Puis **révert**.

Cette mutation ne part pas dans la PR. Sa sortie rouge réelle, si.

### W4 — CA2 + CA3 sur l'autre module

Même geste sur `record_launcher_recovery` (`:5439`) : le remplacer par un no-op
doit faire rougir `a_clean_dispatch_after_a_death_records_the_recovery` — et
**pas** `a_clean_window_records_no_recovery`, qui nie précisément cette écriture
sur une fenêtre saine. Les deux observations sont le contrôle. Puis **révert**.

### W5 — La barrière refuse, et on l'a vu refuser

`mika2677_la_barriere_refuse_un_handler_pendu` passe (il panique). Contrôle de
bonne foi : porter son budget à 10 s et vérifier qu'il **échoue** alors (il ne
panique plus, le handler à 5 s ayant le temps de finir) — c'est ce qui établit
que le `#[should_panic]` est sur le timeout et non sur un défaut du fixture.
Remettre 100 ms.

Note : le sous-processus `sleep 5` survit au panic (`kill_on_drop(false)`, qui est
la forme de production) et sort de lui-même. Le `tempfile::tempdir()` tombe
avant ; c'est sans conséquence, le handler ne lit plus rien.

### W6 — Aucun comportement de production n'a bougé

`git diff` sur la région `executor.rs:1-5446` ne doit montrer que la ligne de
signature, le `;` retiré et le doc-comment. `cargo clippy --all-targets --
-D warnings` vert. Les quatre sites d'appel absents du diff.

### W7 — Stabilité répétée, sans en faire une preuve statistique

Trois passages consécutifs de la suite du crate (`cargo test -p mika-agent`),
qui est la forme réelle de la CI — multi-thread, runner partagé. Aucune des deux
modules ne doit bouger de verdict.

**La preuve est structurelle, pas statistique, et le dire est important** : un
vert répété ne distingue pas « la cause est retirée » de « la fenêtre n'a pas été
atteinte cette fois ». Ce qui ferme le dossier, c'est que plus aucune assertion
de ces deux modules ne lit `Instant::now()` — `grep -n "Instant::now" ` sur la
fenêtre `:13273-14400` doit rendre **zéro** ligne hors du `timeout` de l'aide
partagée (elle-même hors de cette fenêtre).

### Les gardes existantes, inchangées et vertes

Les scans de source de `mod mika2634` (`dispatch_lib_source`,
`production_sources` et leurs allowlists vides), les assertions de format de fil
sur `after_value` / `tool_name`, et le scan de `:6010` sur le littéral injecté
dans `spawn_long_running_exec` portent tous sur du **texte** de la production,
pas sur une signature : ils doivent rester verts sans modification. S'ils
rougissent, lire avant de widener.

---

## Risques nommés

**R1 — le tampon de pipe de stderr, préexistant et non corrigé ici.** Joindre le
moniteur met le test sur son chemin critique pour de vrai. La production lit
stderr **après** le retour de `child.wait()` (`:5206` puis `:5211`), donc un
handler qui écrirait plus qu'un tampon de pipe (~64 KiB) sur fd 2 avant de sortir
bloquerait `wait()` — et désormais le test avec lui, jusqu'au budget de 60 s.
C'est une forme de production, pas une régression de ce ticket : les onze
handlers de ces deux modules écrivent une ligne courte chacun. Nommé, non
corrigé, hors périmètre.

**R2 — un `tokio::spawn` imbriqué ajouté plus tard rouvrirait le trou.** Le join
retournerait avant l'écriture du task imbriqué, et les onze tests redeviendraient
verts par défaut d'observation. Aujourd'hui il n'y en a aucun (`:5063` est le
seul `tokio::spawn` du fichier). La mitigation est le doc-comment de l'aide, qui
le dit à l'éditeur suivant.

Un scan de source refusant un second `tokio::spawn` dans ce fichier serait la
réponse structurelle, et il est **refusé plutôt que reporté** : il porterait sur
un motif à l'échelle du fichier avec **zéro violation mesurée**, et le fichier en
compte déjà quatre, scans dont le coût de lecture est réel. La borne par vol est
mieux dépensée sur la barrière elle-même. Décision, pas ajournement : aucun
ticket de suivi n'est à ouvrir.

**R3 — `JoinHandle` et `#[must_use]`, vérifiés sur la version épinglée.**
L'argument de D1 repose sur `tokio-1.53.1` tel qu'il est verrouillé aujourd'hui.
L'étape 1 le fait trancher par le compilateur avant tout autre travail : si un
site de production réclame un `let _ =`, s'arrêter et remonter plutôt que de
modifier quatre sites de production sous un ticket « tests seulement ».

---

## Hors périmètre

- Les trois aides dupliquées entre `mod mika2532` et `mod mika2634`.
- La forme lecture-après-`wait()` de stderr dans `spawn_long_running_exec` (R1).
- Un scan de source sur les `tokio::spawn` du fichier (R2, refusé).
- Les autres modules de test de `executor.rs` : ils n'utilisent pas ce motif
  (`SETTLE_MS` n'a que ces deux déclarations dans tout le dépôt).

---

## Definition of Done

- [ ] `spawn_long_running_exec` rend son `JoinHandle` ; les quatre sites de
      production sont absents du diff.
- [ ] `HANDLER_JOIN_BUDGET_SECS`, `join_handler_monitor`,
      `join_handler_monitor_within` et le contrôle `#[should_panic]` posés une
      seule fois dans `mod tests`.
- [ ] `SETTLE_MS` (les deux déclarations) et `NEGATIVE_CONTROL_MARGIN_MS`
      supprimées ; `await_handler_failure` et `await_launcher_health` remplacées
      par des lectures uniques ; `require_handler_failure` et la boucle-marqueur
      de `:14228` réécrites en lecture + assertion.
- [ ] Les onze sites d'appel joignent le handle avant toute lecture ou négation.
- [ ] Aucune assertion de ces deux modules ne lit `Instant::now()` (W7).
- [ ] W1 à W7 exécutés ; W3 et W4 avec leur sortie rouge réelle reportée dans le
      corps de PR, et leurs mutations révertées.
- [ ] Suite complète, `cargo clippy --all-targets`, `cargo fmt`,
      `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main`
      verts.
- [ ] Solution-doc posée ; `docs-sync` vert.

---

## Acceptance criteria

Transcrits du corps de senara-solutions/mika#2677 :

- [ ] Le test ne dépend plus d'une échéance murale courte. Il attend un signal de
      fin du processus lancé (handle, canal, notification) ou une écriture
      observable de la ligne. À défaut, l'échéance est assez large pour la CI et
      le reste explicitement.
- [ ] Il en va de même pour l'autre usage de `SETTLE_MS` (`:13567`) s'il suit le
      même motif.
- [ ] Contrôle négatif conservé : si le comportement testé est neutralisé
      (stderr non écrite sur la ligne), le test échoue toujours.

Correspondances : CA1 → D1/D2/D4 + W1 + W7 ; CA2 → D3 + L4 + W2 + W4 ; CA3 → W3
(mutation requise), étendu par W4, et les contrôles négatifs existants (T2 et T5
de `mod mika2532`, les trois de `mod mika2634`) conservés assertion pour
assertion.

---

## Fire-Disposition

**(c) halte-et-remontée, garde CI bloquante** — la disposition déjà en force sur
les deux modules, et ce plan ne la déplace pas.

Les onze détecteurs livrés ici **existent déjà**, tournent **armés**, et bloquent
la CI. Le doc-comment de `mod mika2532` (`:13267-13272`) la nomme en ces termes :
un rouge signifie soit que la cause d'un crash pré-résultat est de nouveau jetée,
soit qu'un enregistrement d'échec est inventé sur une ligne saine (halte 4 des
sondes post-déploiement, qui demande un revert **avant** diagnostic). Celui de
`mod mika2634` (`:13550-13557`) porte la même disposition avec ses allowlists
livrées **vides** et un test frère qui les épingle vides. Ce ticket change leur
**barrière**, pas leur disposition :

- aucune allowlist n'est créée, aucune n'est élargie, aucune n'est touchée ;
- rien n'atterrit `#[ignore]` ou `#[cfg(skip)]` ;
- aucune exception nommée n'est nécessaire : il n'y a **aucune violation
  existante** à couvrir — la suite est verte sur relance, c'est le constat même
  du ticket.

**Le seul détecteur neuf** est `mika2677_la_barriere_refuse_un_handler_pendu`
(D6/W5). Il atterrit **armé**, avec zéro violation existante par construction :
son sujet est le refus de l'aide que la même PR livre, et il n'observe aucune
donnée du dépôt. Un rouge sur lui signifie que la barrière a cessé de refuser —
c'est-à-dire que les onze autres sont redevenus verts par défaut d'observation,
donc la même halte : revert avant diagnostic.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/skills/executor.rs` — production (signature, `;`, doc-comment) | 14 |
| `crates/mika-agent/src/skills/executor.rs` — aide partagée + variante + contrôle `#[should_panic]` | 48 |
| `crates/mika-agent/src/skills/executor.rs` — `mod mika2532` (5 sites + `read_handler_failure`, `SETTLE_MS` retirée) | 44 |
| `crates/mika-agent/src/skills/executor.rs` — `mod mika2634` (6 sites, 3 aides, 2 constantes retirées) | 58 |
| solution-doc `docs/solutions/best-practices/…` (hors compte) | 0 |

Total estimé : 164 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Pourquoi un seul vol tient.** 164 < 243 (borne par vol) et < 1000 (seuil par
PR) : la plus serrée des deux mord, et le total passe sous elle avec 79 lignes de
marge. Trois propriétés expliquent le volume, et chacune est vérifiée plus haut
plutôt que espérée : un seul fichier touché ; le corps du task de surveillance et
les quatre sites d'appel de production restent à **zéro ligne** (D1, tranché par
le compilateur à l'étape 1) ; les assertions de comportement des onze tests
restent à **zéro octet** (seule la barrière qui les précède change). Le travail
est une substitution mécanique sur onze sites plus une aide, pas une conception —
aucune migration, aucun module neuf, aucun contrat de production déplacé.
