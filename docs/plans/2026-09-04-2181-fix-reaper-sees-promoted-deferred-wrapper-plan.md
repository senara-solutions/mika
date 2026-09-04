---
title: "fix(task_engine): le faucheur stuck_pending compte un wrapper différé promu comme vivant"
type: fix
status: active
date: 2026-09-04
origin: senara-solutions/mika#2181
issue: mika#2181
---

# fix(task_engine): le faucheur stuck_pending compte un wrapper différé promu comme vivant

**Issue:** senara-solutions/mika#2181 — Tier 1, casse la boucle.

## Le défaut, lu à la ligne

`Database::find_orphaned_pending_issue_tasks` (`crates/mika-agent/src/db.rs:7576`) protège une
parente `pending` par deux clauses `NOT EXISTS`. La première ne compte que les wrappers différés
`status = 'pending'` :

```sql
AND NOT EXISTS (
  SELECT 1 FROM tasks w
  WHERE w.parent_task_id = parent.id
    AND w.trigger_type = 'callback'
    AND w.status = 'pending'
    AND w.label = ?3
)
```

Or la promotion écrit `status = 'completed'` (`db.rs:8325` et `db.rs:8372`, `promote_next_deferred_callback[_for_class]`),
et le wrapper ne devient `delivered` qu'**après** le retour de `run_silent_agent`
(`dispatcher.rs:512`, dans la branche `else if is_callback` — donc à la fin du tour, pas à son
début). Entre les deux, le wrapper est `completed` sans `delivered`, et la clause ci-dessus le
traite comme absent.

Le tick est fatal parce que les trois étapes vivent dans le **même** passage de 60 s
(`engine.rs:366-413`) et dans cet ordre : `promote_pending_deferred_if_idle` →
`dispatch_undelivered_callbacks` (qui `tokio::spawn` le tour) → `reap_orphaned_pending_issue_tasks`.
Le faucheur s'exécute donc systématiquement quelques millisecondes après la promotion, quand le
wrapper vient de devenir invisible pour lui. Il conclut « rien ne représente cette parente » et
re-arme ; au tick suivant la fente est libre, le wrapper neuf est promu, redevient invisible, le
faucheur re-arme encore. `MAX_STUCK_REARMS = 2` → parente `failed` en deux ticks, quelle que soit la
santé du tour en cours. C'est la trace du ticket : promotion 15:31:03Z, expiration 15:33:03Z,
réponse du tour 15:34:43Z.

Le prédicat répond à « existe-t-il un wrapper **en attente de promotion** ». La question du faucheur
est « existe-t-il un wrapper **vivant** ». mika#2169 (L2b) a fixé la sémantique — sur ce label,
`completed` signifie exclusivement « promu, pas encore pris » — sans la propager au faucheur. Ce
plan la propage.

## La décision de conception, et pourquoi elle est bornée

L'AC1 offre deux formes : « `pending` OU `completed` sans `delivered` », ou bien « `completed_at`
plus récent que `now − grâce_de_tour`, valeur nommée ». **Ce plan prend la seconde, et la première
serait un défaut.**

Raison : `completed` sans `delivered` n'est pas un état transitoire garanti. Sur le chemin d'erreur
du tour (`dispatcher.rs:503`, `is_callback && label == DEFERRED_DISPATCH_LABEL`), le wrapper est
re-armé mais **`mark_task_delivered` n'est jamais appelé** — il reste `completed` non-`delivered`
pour toujours. Un prédicat non borné ferait de ce cadavre un bouclier permanent : le faucheur ne
toucherait plus jamais cette parente, et le défaut qu'on répare se transformerait en fuite
silencieuse dans l'autre sens. La borne temporelle est ce qui rend le correctif fail-safe.

**Valeur choisie : 2700 s**, dans une constante dédiée `PROMOTED_WRAPPER_LIVENESS_DEFAULT_SECS` avec
son propre `MIKA_PROMOTED_WRAPPER_LIVENESS_SECS`. Deux fondements, un mesuré, un structurel.

*Mesuré* — durée réelle entre `completed_at` (promotion) et le passage `delivered`, sur
`~/.mika/data/mika.db`, wrappers `long_running:run_claude_pilot:deferred` livrés :

| fenêtre | n | ≤300 s | ≤900 s | ≤1800 s | ≤2700 s |
|---|---|---|---|---|---|
| 30 j | 799 | 417 (52 %) | 569 (71 %) | 616 (77 %) | 660 (**83 %**) |
| 7 j | 609 | 320 (53 %) | 430 (71 %) | 467 (77 %) | 506 (**83 %**) |

(p50 = 247 s ; la traîne longue vient des redémarrages de serveur et des retards `AgentBusy`, pas de
tours sains.) 2700 s couvre 83 % des fenêtres observées ; 900 s n'en couvrirait que 71 %.

*Structurel* — l'égalité avec `STUCK_PENDING_REAPER_GRACE_DEFAULT_SECS` donne une propriété qui se
lit sans table : **un wrapper promu ne peut pas masquer sa parente plus longtemps que la grâce n'a
mis à la déclarer coincée.** Le faucheur perd au pire un facteur 2 sur sa latence de détection
(2700 → 5400 s), jamais son pouvoir. Constante *séparée* et non réutilisation de la grâce : les deux
nombres répondent à deux questions différentes et doivent pouvoir diverger sous l'env var.

**Sous-décision fail-safe : `completed_at IS NULL` ⇒ non vivant.** Un wrapper `completed` sans
horodatage ne peut pas prouver qu'il est récent ; on ne masque pas sur une preuve absente. En
production ce cas n'existe pas (la promotion écrit toujours `completed_at`) ; en test il est le
chemin qu'exerce déjà `test_find_orphaned_pending_selects_when_wrapper_was_consumed`.

**Restriction à `completed` seul**, pas `status NOT IN ('delivered', ...)` : `failed` et `cancelled`
sont des wrappers morts et ne doivent jamais compter comme vivants. `completed` est exactement le
statut que la promotion écrit.

## Acceptance criteria

Les quatre AC sont transcrits du corps de senara-solutions/mika#2181, sans reformulation.

- [ ] **AC1** — Le prédicat du faucheur compte comme présent tout wrapper différé `pending` **ou**
      `completed` sans `delivered` (ou : tout wrapper dont `completed_at` est plus récent que
      `now − grâce_de_tour`, valeur nommée). Un wrapper promu en cours de tour n'est jamais
      « absent ».
      → **Livrables** D1 (clause SQL élargie) + D2 (`PROMOTED_WRAPPER_LIVENESS_DEFAULT_SECS`, la
      valeur nommée) + D3 (propagation aux appelants).
      → **Mesure** : `test_find_orphaned_pending_excludes_parent_whose_wrapper_was_just_promoted`
      passe au vert, et `test_find_orphaned_pending_selects_when_promoted_wrapper_is_stale` prouve
      que la fenêtre est bornée. L'AC1 offre deux formes ; ce plan prend la seconde et documente
      pourquoi la première serait un défaut (§ *La décision de conception, et pourquoi elle est
      bornée*).

- [ ] **AC2** — Rejeu anti-vacuité verbatim de la trace du ticket (parente `pending` née 12:10:08Z,
      wrapper `f5eebf48` `completed` à 15:31:03Z sans `delivered`, tick du faucheur à 15:31:03Z) :
      sur `main` le faucheur re-arme (rouge) ; avec le correctif il ne touche à rien (vert). Sortie
      rouge dans la PR.
      → **Livrable** D5, avec la séquence obligatoire signature → test → rouge capturé → clause SQL
      → vert.
      → **Mesure** : les deux sorties `cargo test` collées dans le corps de la PR sous « AC2 — rouge
      sans le correctif » et « AC2 — vert avec le correctif ». Un test qui n'a jamais été rouge ne
      démontre rien ; la capture EST le livrable.

- [ ] **AC3** — Non-régression : une parente `pending` dont le wrapper est `delivered` depuis plus
      que la grâce, sans pilote, est toujours re-armée (test existant à nommer, pas à réécrire).
      → **Livrable** D5. **Test existant nommé** :
      `test_find_orphaned_pending_selects_when_wrapper_was_consumed` (`db.rs:~16148`), laissé
      **intact** — seul un commentaire doc est ajouté au-dessus. S'y ajoute
      `test_find_orphaned_pending_selects_when_wrapper_is_delivered` pour la lettre de l'AC3, que
      l'ancien test ne couvrait pas (il porte `completed`, pas `delivered`).
      → **Mesure** : les deux tests verts, et le corps du test existant inchangé au diff.

- [ ] **AC4** — Le re-armement écrit dans son événement d'audit **quel** wrapper il n'a pas trouvé et
      pourquoi (statuts vus), pour que la prochaine bataille se lise sans reconstruire.
      → **Livrable** D4.
      → **Mesure** : `test_stuck_pending_rearm_audit_names_the_wrappers_seen` lit la ligne
      `audit_events` et y trouve les identifiants courts et les statuts des deux wrappers ; un second
      test couvre le rendu `wrappers:none`.

## Livrables

### D1 — élargir la clause (1) du prédicat (AC1)

`crates/mika-agent/src/db.rs`, `find_orphaned_pending_issue_tasks` — nouvelle signature
`(agent_id, grace_seconds, promoted_liveness_seconds)`, quatrième paramètre lié `?4` =
`format!("-{promoted_liveness_seconds} seconds")` :

```sql
AND NOT EXISTS (
  SELECT 1 FROM tasks w
  WHERE w.parent_task_id = parent.id
    AND w.trigger_type = 'callback'
    AND w.label = ?3
    AND (
      w.status = 'pending'
      OR (w.status = 'completed'
          AND w.completed_at IS NOT NULL
          AND w.completed_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?4))
    )
)
```

Le commentaire doc de la fonction est réécrit : la clause (1) devient « aucun wrapper différé
**vivant** — `pending`, ou promu à l'intérieur de la fenêtre de liveness », avec le renvoi à
mika#2181 et la raison de la borne (le cadavre `silent_turn_error`).

### D2 — la constante nommée et son env var (AC1)

`crates/mika-agent/src/task_engine/engine.rs`, en miroir exact de la paire existante
`STUCK_PENDING_REAPER_GRACE_*` (`engine.rs:67-78`, `2515-2544`) :

- `const PROMOTED_WRAPPER_LIVENESS_DEFAULT_SECS: i64 = 2700;` — doc-comment portant le tableau de
  mesure ci-dessus et l'argument structurel ;
- `const PROMOTED_WRAPPER_LIVENESS_ENV: &str = "MIKA_PROMOTED_WRAPPER_LIVENESS_SECS";`
- `fn parse_promoted_wrapper_liveness(raw: Option<&str>) -> i64` — même forme que
  `parse_stuck_pending_reaper_grace` : absent / vide / illisible / non-positif ⇒ défaut + WARN ;
- `pub fn promoted_wrapper_liveness_secs() -> i64` — `pub` pour la même raison que sa sœur : la
  sonde `mika tasks stuck` doit rapporter sur exactement la population que le faucheur voit.

### D3 — propager aux deux appelants

- `crates/mika-agent/src/async_db.rs:1110` — le wrapper async prend et transmet le nouveau paramètre.
- `crates/mika-agent/src/task_engine/engine.rs:631` (`reap_orphaned_pending_issue_tasks`) — passe
  `promoted_wrapper_liveness_secs()`.
- `crates/mika-cli/src/commands/tasks.rs:143` (`mika tasks stuck`) — idem. La sonde et le faucheur
  partagent le prédicat ; les laisser diverger recréerait un mensonge de sonde.

### D4 — l'audit nomme les wrappers vus (AC4)

Nouvelle lecture DB `Database::summarize_deferred_wrappers_of_parent(agent_id, parent_id)` →
`Vec<DeferredWrapperSummary { id, status, completed_at }>`, triée `created_at ASC`, plus un rendu
compact `fn render(&[DeferredWrapperSummary]) -> String` :

- non vide : `wrappers:f5eebf48:completed@2026-09-04T15:31:03Z,284b0ffe:pending@-`
- vide : `wrappers:none`

Appelée **une fois** par candidat dans `reap_orphaned_pending_issue_tasks`, avant la décision de
re-armement, et injectée dans les `details` des deux événements d'audit terminaux du faucheur —
`stuck_pending_task_rearmed` **et** `stuck_pending_task_expired` — ainsi que dans les champs
`tracing` des `info!`/`warn!` correspondants. L'expiration est la fin de la bataille et se lit
encore moins bien que le re-armement ; elle porte donc le même inventaire.

L'inventaire vit dans le faucheur, pas dans `rearm_deferred_callback` : c'est le faucheur qui a
jugé « absent » via sa requête, donc c'est son événement qui doit porter les statuts qui ont produit
ce jugement. `rearm_deferred_callback` a trois appelants dont deux ne posent pas cette question.

Une erreur de lecture de l'inventaire ne bloque rien : `details` reçoit `wrappers:unavailable` et le
faucheur continue. Un audit dégradé ne doit jamais empêcher une réparation.

### D5 — tests

**AC2, rejeu anti-vacuité verbatim** — `db.rs`, à côté des tests mika#2045 (`db.rs:16057+`) :

`test_find_orphaned_pending_excludes_parent_whose_wrapper_was_just_promoted` reproduit la trace du
ticket ligne à ligne :

- parente `pending` née à 12:10:08Z, soit un âge de 11 455 s au tick de 15:31:03Z — au-delà de la
  grâce de 2700 s, donc candidate par l'âge ;
- wrapper différé `completed`, `completed_at` = l'instant du tick, `delivered_at` absent (statut
  jamais passé `delivered`) ;
- appel avec `grace_seconds = 2700`, `promoted_liveness_seconds = 2700`.

Assertion : la liste est vide. Le helper de test `attach_deferred_wrapper` gagne un frère
`attach_deferred_wrapper_at(db, parent, status, completed_at_offset_secs)` qui écrit `completed_at`
explicitement — les tests existants gardent le helper actuel et donc `completed_at IS NULL`.

**Séquence obligatoire pour produire la sortie rouge de l'AC2** (l'ordre est le livrable, pas une
préférence) :

1. appliquer D2 et la **signature** de D1/D3 — le quatrième paramètre est accepté et lié, mais la
   clause SQL reste celle de `main` (`w.status = 'pending'` seul) ;
2. écrire D5 ;
3. `cargo test -p mika-agent find_orphaned_pending` → le nouveau test **échoue**. Capturer la sortie
   complète ; elle va dans le corps de la PR sous « AC2 — rouge sans le correctif ».
4. appliquer la clause SQL de D1 ;
5. re-lancer → vert. Capturer aussi.

Écrire le test après le correctif rendrait l'AC2 invérifiable : un test qui n'a jamais été rouge ne
prouve rien.

**AC3, non-régression** — le cas que le faucheur existe pour attraper reste attrapé.

- Test existant **nommé et laissé intact** : `test_find_orphaned_pending_selects_when_wrapper_was_consumed`
  (`db.rs:~16148`). Sous le correctif il reste vert et couvre désormais le chemin fail-safe
  `completed_at IS NULL` ⇒ non vivant. Un commentaire doc ajouté au-dessus dit exactement cela — le
  corps du test ne bouge pas.
- Test ajouté pour la lettre de l'AC3 : `test_find_orphaned_pending_selects_when_wrapper_is_delivered`
  — wrapper `delivered`, `completed_at` daté de plus que la grâce, aucun callback réel : la parente
  **est** retournée.
- Test ajouté pour le bord de la fenêtre : `test_find_orphaned_pending_selects_when_promoted_wrapper_is_stale`
  — wrapper `completed` avec `completed_at` = `now − 3000 s` (> 2700) : la parente **est** retournée.
  C'est ce test qui empêche la borne de dériver vers l'infini au prochain refactor.

**AC4** — `engine.rs`, à côté des tests `reap_orphaned_pending_issue_tasks` (`engine.rs:3340+`) :
`test_stuck_pending_rearm_audit_names_the_wrappers_seen` monte une parente orpheline avec deux
wrappers de statuts distincts, lance le faucheur, et lit la ligne `audit_events` : les `details`
contiennent les deux identifiants courts et leurs statuts. Un second test couvre `wrappers:none`.

**Bout-en-bout moteur** — `test_reaper_leaves_parent_alone_when_wrapper_was_just_promoted` :
promotion puis `reap_orphaned_pending_issue_tasks().await`, et l'on vérifie que la parente est
toujours `pending`, que `stuck_rearm_count` n'a pas bougé, et qu'aucun `stuck_pending_task_rearmed`
n'a été écrit. Le test `db.rs` prouve le prédicat ; celui-ci prouve que le faucheur consomme bien le
prédicat corrigé.

### D7 — fermer la divergence sémantique sur le prédicat jumeau (F3, premier passage)

`Database::has_pending_deferred_wrapper_child` (`db.rs:7535`) pose la **même question** que la clause
(1) — « cette parente est-elle représentée ? » — avec l'**ancien** prédicat étroit
(`status = 'pending'`). La version initiale de ce plan la laissait hors portée au motif qu'elle n'a
aucun appelant de production. L'argument de l'architecte l'emporte : deux fonctions nommées comme
équivalentes qui répondent différemment à la même question sont un piège armé pour le prochain
appelant, et « pas d'appelant aujourd'hui » n'est pas une propriété stable.

- élargir le prédicat de la fonction à l'identique de D1 (`pending` OU `completed` dans la fenêtre),
  paramètre `promoted_liveness_seconds` ajouté à la signature ;
- **renommer** en `has_live_deferred_wrapper_child`, et le sibling `async_db.rs:1101` avec elle. Le
  renommage n'est pas cosmétique : `pending` dans l'ancien nom décrivait le prédicat, pas la
  question. Le nouveau nom dit la question, et le compilateur attrape tout site oublié ;
- le test `test_has_pending_deferred_wrapper_child` (`db.rs:~16276`) suit le renommage et gagne un
  cas `completed` dans la fenêtre ⇒ vivant.

Coût réel : zéro appelant de production, donc zéro risque de comportement. Le seul diff hors tests
est la signature et le nom.

### D6 — documentation

`crates/mika-agent/CLAUDE.md`, section deferred-dispatch : une phrase sur la sémantique du wrapper
`completed` vue par le faucheur, et le nom de l'env var. La section décrit déjà la sémantique de
promotion ; elle ne dit pas encore ce que le faucheur en fait.

## Séquence

D2 → signatures de D1/D3 → D5 (rouge, capturé) → clause SQL de D1 (vert) → D7 → D4 → D6.

## Fire-Disposition

Trois détecteurs entrent en service. Chacun doit dire, **avant** d'être écrit, ce qu'il fait au
premier tir sur des données qui existaient déjà.

### Corpus préexistant — mesuré, avec ses contrôles positifs

Sur `~/.mika/data/mika.db`, 2026-09-04T21:05Z :

| population | n |
|---|---|
| wrappers différés `status='completed'` (donc promus non consommés) | **0** |
| wrappers différés `status='delivered'` — *contrôle positif* | 815 |
| wrappers différés `status='cancelled'` — *contrôle positif* | 16 |
| parentes `self_dev`/`issue` `status='pending'` au-delà de la grâce | **0** |
| parentes `self_dev`/`issue` `failed` / `cancelled` / `completed` — *contrôle positif* | 744 / 283 / 94 |
| parentes que le nouveau prédicat masquerait et l'ancien pas | **0** |

Les contrôles positifs sont dans le même relevé que les zéros : les zéros sont des absences
mesurées, pas une requête qui ne trouve rien parce qu'elle ne cherche pas au bon endroit.

**Le corpus est volatil** — un wrapper promu peut exister à l'instant du déploiement. La requête du
« corpus C » ci-dessus est donc à rejouer au moment du déploiement, et son résultat à joindre au
commentaire de déploiement.

### Dispositions

**D1 — prédicat élargi. Disposition (b) : accepter le masquage, aucune action rétroactive.**
Une parente préexistante dont le wrapper promu tombe dans la fenêtre n'est pas re-armée à ce tick.
C'est exactement le comportement voulu, appliqué à un état ancien : le tour a peut-être encore une
chance d'aboutir. Au pire elle est re-armée un tick de fenêtre plus tard, soit ≤ 2700 s de retard sur
un faucheur dont la grâce est déjà 2700 s. Aucune migration, aucun backfill, aucune main sur les
lignes existantes. Corpus mesuré à 0 : au déploiement d'aujourd'hui, cette disposition ne s'applique
à personne.

**D5 — rejeu anti-vacuité. Disposition (c) : halte-et-remontée.**
Le test AC2 doit être rouge avant le correctif et vert après. S'il est **vert avant** le correctif,
c'est que la fixture ne reproduit pas la trace — la reproduction est fausse, pas le code. On
s'arrête, on ne « corrige » pas le test pour le rendre rouge, et on remonte à l'opérateur avec la
fixture et la sortie. Même halte si le test reste rouge après le correctif : le diagnostic est faux
et le plan ne tient plus.

**D4 — inventaire d'audit. Disposition : observations futures uniquement.**
L'inventaire est écrit au moment où le faucheur décide ; il ne relit ni ne réécrit aucun
`audit_events` existant. Les lignes d'avant le déploiement restent sans inventaire, et c'est correct
— elles décrivent des décisions prises par un code qui ne le calculait pas. Aucune rétro-écriture,
aucun enrichissement d'historique. Une erreur de lecture de l'inventaire n'interrompt rien : les
`details` reçoivent `wrappers:unavailable` et la réparation continue. Un audit dégradé ne doit jamais
empêcher une réparation.

**D7 — renommage.** Purement mécanique, aucun état en base n'est touché, aucun tir.

## Vérification

- `cargo test -p mika-agent find_orphaned_pending`
- `cargo test -p mika-agent reap_orphaned_pending`
- `cargo test -p mika-agent stuck_pending`
- `cargo test -p mika-agent deferred_wrapper_child` (D7)
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo build -p mika-cli` (D3 touche un appelant CLI)

## Hors portée

- L'échec du tour lui-même (timeout, excuse sans appel d'outil) — mika#2179.
- Le re-armement vers un parent terminal et le `RearmOutcome` jeté — mika#2169.
- ~~`has_pending_deferred_wrapper_child`~~ — **rentré dans la portée** par le finding F3 du premier
  passage architecte. Voir D7.
- Le fait qu'un wrapper `completed` non-`delivered` soit re-dispatché en boucle par
  `dispatch_undelivered_callbacks` sur le chemin `silent_turn_error` — défaut réel, voisin, distinct.
  À ficher séparément avec sa propre trace si on l'observe.

## Risques

**Le faucheur détecte plus tard.** Une parente réellement orpheline dont le dernier wrapper est un
cadavre `completed` récent attend jusqu'à 2700 s de plus avant le premier re-armement. Accepté : la
grâce est déjà 2700 s, et le coût de l'erreur inverse — tuer une parente saine en deux minutes — est
la panne qu'on répare.

**La borne dérive.** Si un futur refactor retire la fenêtre en croyant simplifier, le cadavre
`silent_turn_error` redevient un bouclier permanent.
`test_find_orphaned_pending_selects_when_promoted_wrapper_is_stale` est le garde qui reste.

**Divergence sonde/faucheur.** Si D3 oublie l'appelant CLI, `mika tasks stuck` rapporte sur une
population différente de celle que le faucheur traite. Le compilateur l'attrape (signature changée),
ce qui est la raison pour laquelle le paramètre est ajouté à la signature plutôt que lu depuis
l'environnement à l'intérieur de la requête.
