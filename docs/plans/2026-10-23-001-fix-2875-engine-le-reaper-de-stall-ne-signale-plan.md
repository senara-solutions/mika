# mika#2875 — Le reaper de stall ne signale plus une naissance ni un build

> `fix(engine)` — labels : `bug`, `p2-normal`, `ready`, `observability`,
> `origin:manual`, `dispatch:loop`, **`loop-substrate`**.

## Contexte

`pilot_stall_signal_unavailable` (WARN) est l'instrument que mika#2277 AC4 a
posé pour rendre **audible le coût de la règle de sûreté** du reaper de stall
silencieux :

> *A signal that cannot be read is **never** a satisfied term.*
> (`task_engine/engine.rs:2940-2951`)

Un dispatch dont une surface est illisible sort de la population du reaper
(`continue`) — il devient invisible au faucheur **pour toute sa vie**. AC4 dit
cette sortie une fois par dispatch, puis estampille
`PILOT_STALL_SIGNAL_UNAVAILABLE_REPORTED_KEY` (`engine.rs:555`) pour ne pas la
répéter.

Mesure du 2026-10-22 sur `/var/log/mika/server.log` (moteur `7eb63e43`), 135
occurrences depuis le 2026-10-07, 16 sur les dernières 48 h, toutes avec
`missing_surfaces = transcript,pilot_log,worktree` :

| population | n | ce que la ligne dit vraiment |
|---|---|---|
| `long_running:build_mika` | 120 | un build n'a par construction ni transcript, ni log pilote, ni worktree déclaré — il n'est pas dans la remit du reaper **à aucun âge** |
| pilotes (`run_claude_pilot`, `_groom`) | 14 | signalés **entre 1 et 41 s** après création, avant que le pilote ait pu écrire quoi que ce soit |

100 % des avertissements partent dans les 60 s qui suivent la création de la
tâche. Et comme le marqueur est posé **à ce premier tir**, une indisponibilité
réelle plus tard dans la vie du dispatch — le seul cas que l'instrument existe
pour voir — n'est plus jamais signalée.

> *L'avertissement crie au loup sur les naissances et les builds, et se tait sur
> le seul cas qu'il existe pour voir.*

Le défaut n'est donc pas « trop de bruit ». Il est que l'instrument **mesure une
autre population que celle qu'il nomme** (classe mika#2205) et que, ce faisant,
il se consomme lui-même avant d'avoir pu observer son objet.

## Diagnostic

Le site est `report_unavailable_liveness_signal`
(`crates/mika-agent/src/task_engine/engine.rs:3199-3253`), appelé depuis le bras
`else` du `let-else` à `engine.rs:3061-3081` :

```rust
let (Some(transcript_idle_secs), Some(pilot_log_idle_secs),
     Some(worktree_idle_secs), Some(worktree)) = (…)
else {
    self.report_unavailable_liveness_signal(
        &task, transcript_signal, pilot_log_signal, worktree_signal,
    ).await;
    continue;
};
```

Deux prémisses fausses, indépendantes, et chacune suffit à produire une ligne :

1. **La population.** La boucle itère
   `get_live_dispatch_callback_tasks_with_pid` (`db/tasks.rs:3508`), dont le
   `WHERE` ne connaît que `trigger_type = 'callback'`, `status IN
   ('pending','in_progress')` et `process_id IS NOT NULL`. Ce filtre est le bon
   pour le reaper — il ne tue rien hors pilote, puisque les termes 3-5 écartent
   tout ce qui n'a pas de surface. Mais l'**avertissement** hérite de la
   population du reaper sans hériter de son filtrage implicite : pour lui, le
   `continue` *est* l'événement. D'où les 120 builds.
2. **L'âge.** Rien ne borne l'instant du tir. Un pilote est signalé à la
   première passe du scan qui suit son spawn — une surface qui n'existe pas
   *encore* est lue comme une surface qui n'existera *jamais*.

Le marqueur, lui, fait exactement ce qu'on lui demande : il rend l'événement
unique par dispatch. C'est le **déclencheur** qui est faux, pas le garde-fou.

## Décision de conception

### Porte 1 — la population : un prédicat de label canonique, en égalité stricte

Le ticket laisse le choix entre *sortir les non-pilotes de la population* et
*les nommer à part sous un événement distinct*. **On sort.** Ré-émettre les 120
lignes de build sous un nouveau nom reproduit le bruit sous une autre étiquette
et crée une seconde population à soustraire ; et un build qui n'a pas de
transcript n'est pas une anomalie d'observabilité, c'est sa définition. Il n'y a
rien à dire.

Le prédicat vit à côté du producteur de l'événement, dans `engine.rs`, et suit
l'idiome du dépôt pour cette forme exacte —
`qa_build_callback.rs:76 is_build_callback_label`, dont le commentaire dit
« *Égalité stricte : `long_running:build_mika_foo` n'existe pas* », et
`skills/executor.rs:2649-2662 PILOT_DISPATCH_SKILLS` / `is_pilot_dispatch_skill`
(qui est au niveau **skill**, pas **label** — d'où un prédicat nouveau plutôt
qu'une réutilisation) :

```rust
pub(crate) const PILOT_DISPATCH_LABELS: &[&str] = &[
    "long_running:run_claude_pilot",
    "long_running:run_claude_pilot_groom",
];

pub(crate) fn is_pilot_dispatch_label(label: &str) -> bool {
    PILOT_DISPATCH_LABELS.contains(&label)
}
```

L'égalité stricte n'est pas de la rigueur décorative : elle exclut aussi
`crate::agent::DEFERRED_DISPATCH_LABEL`
(`long_running:run_claude_pilot:deferred`) — une row d'enveloppe qui ne porte
aucun PID et ne décrit aucun processus pilote — et tout futur
`run_claude_pilot_*` dont personne n'aurait vérifié qu'il a les trois surfaces.
Un `starts_with` les avalerait silencieusement ; dans ce dépôt `starts_with` sur
un label n'apparaît qu'en code de test.

**Le label est produit à un seul endroit** —
`skills/executor.rs:5576-5624 build_callback_task`, `label =
format!("long_running:{tool_name}")`. La constante sera donc épinglée **contre
son producteur**, pas contre un littéral recopié (doctrine des noms canoniques,
`db.rs:2540-2544` ; précédents : `qa_build_callback.rs:525` et
`dispatcher.rs:7968-7977 test_deferred_dispatch_label_string_drift_guard`).

### Porte 2 — l'âge : `fired_at`, avec `created_at` en repli

Référence temporelle :

```rust
task.fired_at.as_deref().unwrap_or(&task.created_at)
```

`fired_at` est estampillé **au spawn** par `set_task_process_id`
(`db/tasks.rs:3355-3402`, via `FIRED_AT_STAMP_IF_NULL`) — et la requête de
population exige `process_id IS NOT NULL`. Donc sur exactement cette population,
`fired_at` est non-nul par construction, et c'est l'instant de naissance précis
du dispatch. Le repli sur `created_at` ne couvre que les rows antérieures à
mika#2263.

Le seuil est celui du reaper lui-même : `max_age_secs =
settings.effective_pilot_stall_reap_age_seconds()` (`engine.rs:2996`, défaut
2700 s, `0` refusé). Pas de second réglage — le ticket dit *« le seuil d'âge du
reaper »*, et un seuil dédié serait une deuxième vérité à garder alignée.

Un petit helper rend `Option<i64>` pour que l'âge serve **deux fois** : à
décider, et à être dit dans le WARN.

```rust
fn dispatch_age_secs(task: &crate::db::Task) -> Option<i64>
```

Deux cas rendent `None`-équivalent — horodatage illisible, et âge **négatif**
(dérive d'horloge). Les deux signifient *silencieux et non marqué* : une passe
ultérieure pourra encore signaler. C'est le marqueur qui est permanent, jamais
le silence — et c'est pourquoi l'ordre « taire sans marquer » est la seule forme
sûre des deux portes.

### Ordre des portes : population d'abord

Le label est un `contains` sur deux `&str` ; l'âge parse un timestamp. Et
surtout : un build est hors remit **à tout âge**, donc le tester d'abord est
aussi la bonne sémantique, pas seulement le bon coût.

### Rayon d'explosion : nul sur le comportement du reaper

Les deux portes vivent **entièrement dans
`report_unavailable_liveness_signal`**, qui gagne un paramètre `max_age_secs`.
Ne changent pas : la requête de population, la conjonction à six termes, le
`continue` du bras `else`, `dispose_of_silently_stalled_pilot`, la row d'audit
`pilot_silent_stall`, l'armement. *« Ne change pas ce que le reaper fait des
pilotes »* est tenu par la structure, pas par la discipline.

### Enrichissement du WARN

Trois champs ajoutés : `label`, `age_secs`, `max_age_secs`. Ce sont des données
d'opérateur réelles — « quelle surface manque, sur quel genre de dispatch, à
quel âge, contre quel seuil » est la question qu'on pose en lisant la ligne. Et
l'**absence** de `age_secs` devient le discriminant du motif mika#2340 : elle
veut dire *binaire antérieur à ce vol*, jamais *âge nul*.

### Ce que ce vol NE fait pas, et pourquoi

- **Aucun événement distinct pour les builds.** Voir Porte 1 : il n'y a rien à
  dire sur un build sans transcript.
- **Aucun contrôle positif « le reaper tourne-t-il ? ».** Après ce vol,
  `pilot_stall_signal_unavailable` tend vers zéro, et zéro a deux causes
  (la porte marche / le scan ne tourne plus). Le témoin existant du reaper
  (`pilot_silent_stall`) a exactement la même propriété et n'est pas touché par
  ce ticket : lui donner un contrôle positif est un changement de l'instrument
  du reaper, hors de *« ne change pas ce que le reaper fait des pilotes »*.
  Ce qui est acquis ici, c'est que l'**indisponibilité tardive** — le seul cas
  que l'instrument existe pour voir — redevient observable, et AC2 l'atteste.

## Livrables

### D1 — `crates/mika-agent/src/task_engine/engine.rs`

1. `PILOT_DISPATCH_LABELS` + `is_pilot_dispatch_label`, avec le doc-comment qui
   dit *pourquoi égalité stricte* et nomme les deux exclusions
   (`:deferred`, `build_mika`).
2. `dispatch_age_secs(&Task) -> Option<i64>`, avec le doc-comment qui dit
   pourquoi `fired_at` d'abord et pourquoi âge négatif ⇒ `None`.
3. `report_unavailable_liveness_signal` : signature `+ max_age_secs: u64`, les
   deux portes en tête de corps (population, puis âge), **avant** le calcul de
   `missing` et avant toute écriture de marqueur ; un argument au site d'appel
   (`engine.rs:3072`).
4. Les trois champs du WARN.
5. Amendement des deux doc-comments concernés — le bloc AC4 du reaper
   (`engine.rs:2952-2958`) et celui de la fonction (`engine.rs:3185-3198`).
6. Tests unitaires dans le `mod tests` en place (`engine.rs:5834+`) :
   - `is_pilot_dispatch_label` énuméré — `run_claude_pilot` ✓,
     `run_claude_pilot_groom` ✓, `build_mika` ✗, `:deferred` ✗,
     `run_claude_pilot_extra` ✗ ;
   - garde de dérive : la constante comparée au label que
     `build_callback_task(…, "run_claude_pilot", …)` **produit**, jamais à un
     littéral recopié ;
   - `dispatch_age_secs` : `fired_at` l'emporte sur `created_at`, repli quand
     `fired_at` est `None`, `None` sur horodatage illisible, `None` sur âge
     négatif.

### D2 — `crates/mika-agent/tests/eval/test_reaper_liveness_all_surfaces_2277.rs`

**Les tests comportementaux vont dans ce fichier, pas dans un fichier frère.**
Il possède déjà la fixture du reaper (~180 lignes : `dispatcher_with_log_dir`,
`Surfaces`, `seed_dispatch`, `drive_scan`, `stall_audit_count`,
`inertia_marker`) et la sémantique du marqueur AC4 ; un frère la dupliquerait ou
forcerait un refactor de visibilité. Et surtout **N4 doit y être amendé de toute
façon** (ci-dessous) : séparer les deux moitiés du même contrat sur deux fichiers
est ce qui fait diverger les fixtures.

1. Harnais de capture de logs (`CapturingLayer` + `FieldVisitor` + `fn
   capture()`), recopié de `tests/eval/test_llm_watchdog_2342.rs:34-81` —
   `tracing_subscriber::registry().with(…)` + `tracing::subscriber::set_default`,
   thread-local, donc valide sous le runtime current-thread de `#[tokio::test]`.
   Il est nécessaire : AC1 et AC2 portent sur le **WARN**, et le marqueur n'en
   est qu'un proxy.
2. `seed_dispatch` gagne un paramètre `tool_name: &str` (les appels existants
   passent `"run_claude_pilot"`), plus un helper `backdate_fired_at(db, id,
   age_secs)` via `db.with_db` — l'idiome des tests d'intégration du dépôt
   (`test_cancel_task_live_pilot_2653.rs:505`).
3. **Amendement de N4**
   (`n4_signal_indisponible_aucun_kill_et_inertie_visible`) : il sème un
   dispatch *frais* sans transcript et exige le marqueur. Sous la porte d'âge il
   rougit. Il est **réparé dans la même PR** — antidatage au-delà du seuil — et
   son doc-comment dit que l'indisponibilité qu'il décrit est désormais une
   indisponibilité **tardive**. Ce n'est pas une exception, c'est la prémisse du
   test qui change avec le contrat.
4. `ac1_un_pilote_plus_jeune_que_le_seuil_ne_dit_rien_et_ne_marque_rien` —
   pilote, trois surfaces absentes, âge natif (quelques secondes) : zéro WARN
   `pilot_stall_signal_unavailable`, `inertia_marker == None`, zéro row d'audit,
   processus vivant.
5. `ac2_le_meme_dispatch_au_dela_du_seuil_dit_exactement_une_fois` — **en deux
   temps sur le même dispatch**, ce qui est la lettre de l'AC (« le *même*
   dispatch ») et ce qui le rend rouge-avant : phase A, scan à l'âge natif
   (zéro WARN, pas de marqueur) ; phase B, `backdate_fired_at` au-delà du seuil
   puis **deux** scans — exactement un WARN sur l'ensemble, portant
   `age_secs >= max_age_secs` et `missing_surfaces` complet, marqueur posé à ce
   moment-là. Contre le code actuel, la phase A tire déjà le WARN et pose le
   marqueur, donc la phase B en compte zéro : rouge.
6. `ac3_un_build_mika_ne_produit_aucun_pilot_stall_signal_unavailable` —
   `seed_dispatch(…, "build_mika", …)`, trois surfaces absentes, antidaté bien
   au-delà du seuil (pour que seule la porte de label puisse l'arrêter) : zéro
   WARN, `inertia_marker == None`. Contrôle négatif de non-régression dans le
   même test : la tâche n'est **pas** tuée et son statut reste `pending` — la
   porte mute un avertissement, elle ne change pas la disposition.

### D3 — documentation (hors compte de lignes)

- `docs/agent-internals/unified-task-engine.md:499` — le contrat de prose AC4 :
  dire que l'avertissement est borné à la population pilote et au-delà du seuil
  d'âge, et que son silence en deçà est voulu.
- Pas de `scripts/sync-agent-docs.sh` à lancer : ce fichier n'est pas dans la
  liste `DOCS` du script (les 9 docs nommés + openapi), donc le job `docs-sync`
  ne le voit pas.

## Ordre de commit

1. D1.1–D1.2 + leurs tests unitaires (les deux fonctions pures, isolées,
   vertes seules).
2. D1.3–D1.4 + D1.5 (les portes et le WARN) — **la suite 2277 est rouge sur N4
   à ce commit**, c'est attendu et c'est le rouge-avant d'AC4.
3. D2.1–D2.3 (harnais, paramètre `tool_name`, amendement N4) — suite verte à
   nouveau.
4. D2.4–D2.6 (AC1, AC2, AC3).
5. D3.

Plancher : le commit 2 seul ferme le défaut mesuré ; les commits 3-4 sont ce qui
l'empêche de revenir.

## Contrat de vérification

| quoi | comment |
|---|---|
| unitaires | `cargo test -p mika-agent task_engine::engine::tests` |
| comportementaux | `cargo test -p mika-agent --test eval reaper_liveness_all_surfaces` |
| suite complète | `cargo test` |
| lint | `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` |
| pipeline | `scripts/verify-pipeline.sh origin/main` |
| rouge-avant (AC4) | avant D1.3, lancer AC1/AC2/AC3 et **consigner les trois échecs terme par terme dans le corps de PR**, avec la raison de chacun |

**Sonde post-déploiement** (le dernier critère du ticket) — après `make deploy`,
sur `/var/log/mika/server.log` : aucune ligne `pilot_stall_signal_unavailable`
dont le `task_id` joint à `tasks` donne un `label = 'long_running:build_mika'`,
et toute ligne restante porte `age_secs >= 2700`. Une ligne **sans** `age_secs`
signifie que le binaire servi précède ce vol (motif mika#2340), jamais que le
champ vaut zéro.

## Definition of Done

- [ ] `is_pilot_dispatch_label` + `PILOT_DISPATCH_LABELS` en place, en égalité
      stricte, épinglés contre le producteur du label et non contre un littéral.
- [ ] `dispatch_age_secs` en place, `fired_at` d'abord, `created_at` en repli,
      `None` sur illisible et sur âge négatif.
- [ ] Les deux portes dans `report_unavailable_liveness_signal`, population
      avant âge, **avant** toute pose de marqueur.
- [ ] Le WARN porte `label`, `age_secs`, `max_age_secs`.
- [ ] Rien d'autre du reaper n'est modifié : population, conjonction à six
      termes, disposition, row d'audit, armement.
- [ ] N4 amendé et vert ; les quatre autres tests de la suite 2277 verts sans
      modification.
- [ ] AC1, AC2, AC3 verts, et leur rouge-avant consigné dans le corps de PR.
- [ ] `docs/agent-internals/unified-task-engine.md` amendé.
- [ ] `cargo test`, `cargo clippy --all-targets -- -D warnings`,
      `cargo fmt --check` et `scripts/verify-pipeline.sh origin/main` verts.

## Acceptance criteria

- AC1 : un dispatch pilote plus jeune que le seuil, sans aucune surface, ne produit ni WARN ni marqueur.
- AC2 : le même dispatch, au-delà du seuil et toujours sans surface, produit **exactement un** WARN, et le marqueur est posé à ce moment-là.
- AC3 : un `long_running:build_mika` ne produit aucun `pilot_stall_signal_unavailable`.
- AC4 : rouge-avant terme par terme sur AC1 à AC3 (chaque test rougit contre le code actuel).
- Après déploiement : zéro `pilot_stall_signal_unavailable` portant un `task_id` de build dans le log.

## Fire-Disposition

**Option (c) — halte-et-remontée, gate CI bloquant.** Même disposition que les
cinq tests déjà dans `test_reaper_liveness_all_surfaces_2277.rs`, dont le
doc-comment § *Fire-Disposition* la porte déjà pour la même raison : un rouge sur
cette suite veut dire que le reaper traite mal du travail vivant, ou qu'il ne
voit plus rien. Les trois nouveaux tests héritent de cette gravité : AC1/AC3
rouges veulent dire que l'instrument crie à nouveau sur une population qu'il ne
nomme pas ; AC2 rouge veut dire qu'il s'est de nouveau consommé avant d'avoir pu
observer son objet. Aucun des trois ne se désarme par allowlist ; s'ils
rougissent, l'implémentation s'arrête et remonte.

**Pas d'allowlist, et il n'y a rien à y mettre.** La population de violations de
ce détecteur est **vide dans le dépôt** : les 135 lignes mesurées sont de
l'historique de journal, pas du code. Aucune entrée nommée n'est donc requise —
et en inventer une serait exactement l'allowlist sans objet que mika#2309
condamne.

**Le seul rouge préexistant est N4, et il est réparé, pas excepté.** N4 rougit
parce que la porte d'âge change la prémisse qu'il encode (« une surface absente
sur un dispatch frais est une indisponibilité »), pas parce qu'il révèle une
violation à tolérer. Il est amendé dans la même PR, au commit 3 de l'ordre
ci-dessus, et son doc-comment dit ce qui a changé.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/task_engine/engine.rs` — `PILOT_DISPATCH_LABELS` + `is_pilot_dispatch_label` (+ doc) | 25 |
| `crates/mika-agent/src/task_engine/engine.rs` — `dispatch_age_secs` (+ doc) | 25 |
| `crates/mika-agent/src/task_engine/engine.rs` — les deux portes, la signature, le site d'appel (+ doc amendée) | 35 |
| `crates/mika-agent/src/task_engine/engine.rs` — champs `label` / `age_secs` / `max_age_secs` du WARN | 3 |
| `crates/mika-agent/src/task_engine/engine.rs` — tests unitaires (prédicat énuméré, garde de dérive, helper d'âge) | 55 |
| tests (`tests/eval/test_reaper_liveness_all_surfaces_2277.rs`) — harnais de capture | 50 |
| tests (`tests/eval/test_reaper_liveness_all_surfaces_2277.rs`) — `tool_name` + `backdate_fired_at` | 25 |
| tests (`tests/eval/test_reaper_liveness_all_surfaces_2277.rs`) — amendement N4 | 8 |
| tests (`tests/eval/test_reaper_liveness_all_surfaces_2277.rs`) — AC1 | 40 |
| tests (`tests/eval/test_reaper_liveness_all_surfaces_2277.rs`) — AC2 (deux phases) | 60 |
| tests (`tests/eval/test_reaper_liveness_all_surfaces_2277.rs`) — AC3 | 45 |

Total estimé : 371 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 371 < 429, et 371 < 1000 : les deux
bornes passent, la plus serrée (429) avec 58 lignes de marge. Le découpage en
vols serait d'ailleurs contre-productif ici : les deux portes sont les deux
moitiés d'un même contrat (AC2 n'est vérifiable que si la porte d'âge existe,
AC3 que si la porte de population existe), et N4 rougit dès la première des deux
— livrer une seule porte, ce serait merger un vol avec une suite rouge.

Trois facteurs tiennent l'estimation basse malgré onze lignes de tableau. La
**fixture existe déjà** : les ~180 lignes de `dispatcher_with_log_dir` /
`Surfaces` / `seed_dispatch` / `drive_scan` sont réutilisées, pas réécrites — ce
qui est la raison principale de ne pas ouvrir un fichier frère. Le **harnais de
capture est recopié**, pas conçu
(`test_llm_watchdog_2342.rs:34-81`). Et le **changement de production est de
quatre gestes** : deux fonctions pures, deux `return` anticipés, trois champs de
log — aucune signature publique, aucune migration, aucune dépendance nouvelle.
Le volume est dans les tests, et c'est la bonne répartition pour un ticket dont
le livrable est la justesse d'un instrument.
