# Vol V2b-2b-3 — les `failed` par le SSE (mika#1727)

> Un callback `failed` de la conversation en cours arrive par le SSE et se livre
> comme le poll le livre : une seule fois.

Branche : `feat/1727/tui-v2b2b3-failed-par-le-sse`, neuve depuis `main`.
La PR portera `Refs mika#1727`. Jamais `Closes`, `Fixes` ni `Resolves` : le
ticket se ferme au merge de V4.

---

## 0. Ce qui est mesuré, avant toute conception

Chaque ligne de cette section est une mesure faite dans ce worktree sur
`e8e6aed49`, pas un souvenir. Elles décident de la forme du vol.

**La borne est entière.** `git fetch origin main` puis `git rev-parse FETCH_HEAD`
donne `e8e6aed49` ; `git rev-parse HEAD` donne le même. La branche est identique
à `origin/main` : zéro ligne consommée, les 279 sont disponibles.

**La réclamation accepte déjà un `failed`.** `mark_task_delivered`
(`crates/mika-agent/src/db/tasks.rs:1935`) écrit
`UPDATE … WHERE id = ?1 AND status IN ('completed','failed')`. Le critère 3 — « la
ligne passe à `delivered` » — est donc atteignable **sans toucher
`crates/mika-agent/`**. C'était la question qui pouvait faire s'arrêter ce groom ;
elle est tranchée.

**Le serveur rend déjà `completed_at`.** `TaskDetailResponse`
(`crates/mika-agent/src/server/dashboard.rs:786`) porte
`completed_at: Option<String>`. La garde d'ancienneté se construit donc en
ajoutant un lecteur au miroir, pas un champ au serveur.

**`TaskFailed` porte un `task_id`.** `TaskEventFrame::TaskFailed { task_id,
agent_id, failed_at, error_preview }`
(`crates/mika-agent/src/server/tasks_stream.rs:102-150`). L'or-pattern avec
`TaskCompleted` compile donc sur le seul champ commun, et le `match` reste
exhaustif sans bras `_` : une neuvième variante continuera de rougir à la
compilation.

**`is_older_than` échoue en ouvert.** `crates/mika-agent/src/timestamp.rs:41`
retourne `false` quand l'horodatage ne se parse pas. Combiné à `is_some_and`, un
`completed_at` absent ou illisible n'est **pas** tenu pour vieux : la tâche est
livrée. C'est exactement le biais du poll, et c'est le bon : un doute sur
l'horodatage ne doit pas avaler un callback.

**Les deux constantes du poll sont lisibles depuis `mika-cli`.**
`STALE_FAILED_CALLBACK_MINUTES = 5` (`crates/mika-agent/src/planning/policy.rs:131`)
et `FAILED_TASK_FALLBACK` (`crates/mika-agent/src/agent_loop/mod.rs:118`, atteint
en `mika_agent::agent::FAILED_TASK_FALLBACK`). Aucune valeur n'est recopiée : la
parité avec le poll est portée par les constantes elles-mêmes, pas par un
commentaire qui promet de les suivre.

**Egress Manifest Lint est vert, `docs/egress/` intact.**
`bash scripts/verify-egress-manifest.sh --report` :
`400 fichier(s) .rs de production scanné(s), 31 portant une construction de
client, 22 host(s) distinct(s), 31 entrée(s) [[sink]] confrontée(s)` puis
`No egress-manifest violations found.` Ce vol n'ouvre aucun client HTTP neuf — la
lecture du détail passe par le client que V2b-2b-1 a déjà déclaré — donc le
manifeste n'a rien à apprendre. Le pilote le relance avant d'ouvrir la PR
(critère 5) ; s'il le trouve rouge, il s'arrête et le dit.

**Les deux doc à corriger sont localisées.** `grep "TaskFailed"` sur
`crates/mika-cli/src/` donne cinq occurrences, dont trois sont du code déjà
correct (`tasks.rs:495` filtre par agent et traite déjà `TaskFailed` ;
`tasks.rs:680` est le bras à déplacer ; `tasks.rs:1416` est une fixture). Les deux
phrases à réécrire sont :
- `tasks.rs:710` — « `TaskFailed` étant hors de ce vol, le statut d'origine est
  toujours `"completed"` » ;
- `tasks.rs:1650` — la doc du test du canal, qui annonce `TaskFailed` parmi les
  trames écartées.
Elles partent dans le commit du code qu'elles décrivent (périmètre).

**Un seul test existant devient rouge, et il est nommé ici.**
`mika1727_v2b2b1_les_trois_formats_rendent_le_meme_miroir` (`tasks.rs:1572`) épingle
**exactement dix** paires `(nom, valeur)` par un `assert_eq!` de chaîne entière sur
le JSON et sur le YAML ; sa doc dit « un onzième champ rendu ne peut pas être vert
ici ». Ajouter `completed_at` au miroir le fait rougir, parce que `serde` émet
`"completed_at": null`. C'est voulu : ce test est un détecteur de champ fantôme, et
il fait son travail. Son traitement est au § 2.3.

Deux mesures disent aussi ce qu'il n'y a **pas** à faire :

- **Le site d'appel au tick est déjà bon** (`app.rs:1128-1132`) : le drainage tourne
  à chaque tick, hors `POLL_INTERVAL_TICKS`, quand la TUI est `Idle` et hors mode
  équipe. Un `failed` n'a pas d'autre cadence qu'un `completed`. Rien à changer.
- **`write_task_detail_text`** (`tasks.rs:792-809`) ne rend que six champs
  d'opérateur. L'attente du format texte reste verte quand le miroir gagne un
  onzième champ ; seuls JSON et YAML bougent.

---

## 1. La couture ne change pas de forme

`CallbackFeed` (`tasks.rs:712-783`) possède le canal, l'URL de base et le jeton, et
n'expose qu'un geste. Ce vol **n'y touche pas** : il n'élargit que le prédicat qui
décide ce qui entre dans le canal, et le statut que le drainage lit sur le détail.
La propriété « la TUI ne détient ni URL, ni jeton, ni nom de lecture HTTP » reste
vraie par le type.

**Le canal reste un `task_id` seul.** Le périmètre autorise un second champ « si le
plan montre qu'il est nécessaire, et dit pourquoi ». Il ne l'est pas, et voici
pourquoi : le statut d'origine se lit sur le détail, qui est l'autorité. Un statut
porté par le canal serait une seconde vérité, capable de diverger de la première —
la trame dit `failed`, le détail dit `completed` parce que la tâche a été reprise
entre-temps, et `original_status` mentirait à l'agent. Un seul lecteur, une seule
source.

---

## 2. Les gardes et leur domicile

### 2.1 `TaskFailed` mène à une livraison (`commands/tasks.rs`)

`terminal_callback_frame` (`tasks.rs:675-686`) passe `TaskFailed` du bras négatif au
bras positif, par or-pattern sur le champ commun :

```rust
TaskEventFrame::TaskCompleted { task_id, .. }
| TaskEventFrame::TaskFailed { task_id, .. } => Some(task_id.clone()),
```

Les six autres variantes restent énumérées une à une, sans bras `_`. Sa doc est
réécrite dans le même commit : elle dira pourquoi **deux** trames sont terminales
pour une livraison, et pourquoi `TaskDelivered` et `TaskCancelled` n'en sont pas
(l'une annonce une livraison déjà faite, l'autre une tâche qui n'a pas de résultat
à rendre).

### 2.2 Le prédicat accepte deux statuts, et rien d'autre (`commands/tasks.rs`)

Dans `est_un_callback_a_livrer` (`tasks.rs:694-700`), `self.status == "completed"`
devient :

```rust
matches!(self.status.as_str(), "completed" | "failed")
```

Les quatre autres conjonctions — agent, `trigger_type == "callback"`,
`action_type == "resume_agent"`, session de la TUI — ne changent pas. Un détail
`delivered`, `pending` ou `running` est refusé par ce `matches!` (critère 4, dernière
phrase).

### 2.3 Le miroir gagne son onzième champ, et le reste épinglé (`commands/tasks.rs`)

`TaskDetailMirror` (`tasks.rs:618-630`) gagne `pub(crate) completed_at:
Option<String>`, et `status` passe de privé à `pub(crate)` — le drainage le lit
désormais pour `original_status`, pour le choix du repli et pour le libellé du
message. La doc du type, qui énumère les lecteurs champ par champ, est corrigée :
dix devient onze, et `completed_at` nomme son lecteur unique (la garde
d'ancienneté du § 2.5).

Le test des trois formats est **renforcé, pas affaibli** : `corps_de_detail()`
(`tasks.rs:1522-1529`) gagne un `completed_at`, la liste `champs` gagne sa onzième
paire, et la doc du test passe à onze. Deux échappatoires ont été écartées :

- `skip_serializing_if = "Option::is_none"` rendrait le test vert sans le toucher,
  en affaiblissant silencieusement l'invariant qu'il énonce — « chaque champ a son
  lecteur, et le rendu les montre tous ». C'est le geste que le critère 4 interdit
  dans son esprit.
- `skip_serializing` tout court introduirait une asymétrie `Deserialize`/`Serialize`
  dans un type dont la doc annonce la symétrie, et cacherait à l'opérateur un champ
  qu'il a des raisons de vouloir.

**Risque résiduel nommé, avec son remède en une ligne :** `serde_yaml` 0.10 peut
citer une chaîne ISO-8601 (`completed_at: "2026-11-11T…"`). Si c'est le cas, seule
l'attente YAML prend la forme citée — les deux formats ont déjà des attentes
séparées, donc le remède ne touche pas le JSON.

### 2.4 La doc de la couture dit la vérité (`commands/tasks.rs`)

La phrase de `tasks.rs:710` est réécrite : le canal porte un `task_id` seul **non**
parce que `TaskFailed` serait hors périmètre, mais parce que le détail est
l'autorité sur le statut (§ 1). C'est la correction que le périmètre exige dans le
commit du code qu'elle décrit.

### 2.5 Le drainage livre un `failed` comme le poll (`tui/app.rs`)

`drain_callback_deliveries` (`app.rs:1621-1667`) prend la forme du poll, dans cet
ordre :

1. **Le statut est lu avant la réclamation.** `let est_un_echec = detail.status ==
   "failed";` — après `mark_task_delivered`, la ligne vaut `delivered` et
   l'information est perdue. C'est pour ça que `original_status` se lit sur le
   détail en mémoire, jamais sur la base relue.
2. **La réclamation, puis la garde d'ancienneté.** Le poll teste l'ancienneté avant
   de réclamer ; le drainage réclame une fois, puis `continue` si la tâche est
   vieille. **Le résultat observable est identique** — ligne à `delivered`, aucun
   `CallbackResult`, aucun message — et il y a **un seul site de réclamation** au
   lieu de deux, donc une seule manière de se tromper sur la condition de course
   avec le poll. La garde :
   ```rust
   if est_un_echec
       && detail.completed_at.as_deref().is_some_and(|ts| {
           mika_agent::timestamp::is_older_than(ts, stale_threshold)
       })
   {
       continue;
   }
   ```
   `is_some_and` traite un `completed_at` absent comme non-vieux, et
   `is_older_than` échoue en ouvert sur un horodatage illisible (§ 0) : les deux
   biais vont vers la livraison, jamais vers le silence.
3. **Le résultat, par les trois bras du poll.** Un `failed` sans résultat prend
   `FAILED_TASK_FALLBACK` ; un `completed` sans résultat est sauté par `continue`.
4. **Le message système et le `CallbackResult`.** `status_label` vaut `"failed"` ou
   `"completed"` ; `original_status: detail.status` ; `trace_id` et `parent_task_id`
   viennent du détail ; puis `Thinking`, `needs_redraw`, `return`.

**Une divergence délibérée avec le poll, et sa raison.** Le drainage ne reproduit
**pas** le message de résumé `"Cleared N stale failed task(s)"` que le poll pousse
après sa boucle (`app.rs:1763-1777`). Deux raisons. D'abord le critère 3 l'interdit
explicitement : « aucun `CallbackResult`, aucun message système ». Ensuite ce
message n'aurait pas de sens ici : le poll balaye sept jours et agrège un compte,
le flux voit une ligne par trame — un résumé par ligne serait du bruit, et un
compte de « 1 » répété ne dit rien de plus que le silence.

La doc du drainage (`app.rs:1609-1620`), qui affirme aujourd'hui que
`original_status` vaut toujours `"completed"` et qu'il n'y a donc pas de garde
d'ancienneté, est réécrite dans le même commit : parité avec le poll, raison de
l'ordre réclamation-puis-garde, et absence du message de résumé.

### 2.6 Ce qui n'est pas touché

`poll_callback_tasks` (`app.rs:1670-1778`) n'est pas modifié — pas une ligne, pas un
commentaire (critère 5). Il reste le filet de tout callback que le flux manque,
puisque le flux ne se reconnecte pas encore (c'est V2b-2b-4).

---

## 3. Ordre des commits

Le pilote commite après chaque garde verte **avec son test**, pas en fin de vol.
Deux commits, séparés par le fichier qu'ils touchent :

**Commit 1 — la couture accepte les `failed`** (`crates/mika-cli/src/commands/tasks.rs`)
§ 2.1, § 2.2, § 2.3, § 2.4, avec T1, T2, T3 et le test des trois formats remis à
onze champs. À la fin de ce commit, `TaskFailed` entre dans le canal et le prédicat
l'accepte ; le drainage, lui, écrit encore `"completed"` en dur — la livraison d'un
`failed` serait étiquetée à tort. L'état est compilable et vert, mais **incomplet
par construction** : c'est pourquoi les deux commits partent dans la même PR et
qu'aucun n'est mergé seul.

**Commit 2 — le drainage livre les `failed`** (`crates/mika-cli/src/tui/app.rs`)
§ 2.5, avec T4, T5, T6, T7 et le helper paramétré. À la fin de ce commit les cinq
critères tiennent.

---

## 4. Le jeu de tests

Sept tests, dont trois sont des modifications de tests existants. Aucun nouveau
constructeur de serveur factice : `fake_spirit::spawn`, `spawn_detail` et
`CallbackFeed::from_channel` suffisent (périmètre).

### Commit 1 — `commands/tasks.rs`

**T1 — `mika1727_v2b2b3_un_completed_et_un_failed_de_lagent_entrent_dans_le_canal`**
(renommage de `mika1727_v2b2b2_seul_un_completed_de_lagent_entre_dans_le_canal`,
`tasks.rs:1657`). Les six charges SSE littérales ne changent pas ; l'attente passe
de `vec!["t-6"]` à `vec!["t-2", "t-6"]` — `t-2` est le `TaskFailed` de `mika-dev`
que le test écartait. L'`overflow_marker`, la trame du futur et le `claimed` restent
ignorés, et l'abonnement se consomme jusqu'au bout : c'est le témoin positif que
mika#2205 exige à côté des refus. Sa doc est réécrite (§ 0).

**T2 — un `failed` passe les deux gardes.** Un cas de plus dans
`mika1727_v2b2b3_le_candidat_passe_les_deux_gardes_ou_ne_passe_pas`
(`tasks.rs:1701-1734`) via le helper `candidat(statut, corps, agent, session)` :
`candidat("failed", …)` est accepté, à côté du `completed` déjà épinglé.

**T3 — un `delivered` est refusé.** Un refus de plus dans le même test :
`candidat("delivered", …)` ne passe pas le prédicat. C'est la dernière phrase du
critère 4, et c'est ce qui empêche le `matches!` de devenir un jour « tout sauf
`pending` ».

**Le test des trois formats** reste un `assert_eq!` de chaîne entière, à onze paires
(§ 2.3).

### Commit 2 — `tui/app.rs`

Le helper `couture_dun_callback_livrable` (`app.rs:2346-2398`) devient paramétré par
`(statut, résultat, completed_at)` ; ses deux appelants existants passent
`("completed", Some("le pilote a fini"), None)` et restent verts sans être
affaiblis.

**T4 — un `failed` du fil est livré une seule fois.** Sème une ligne `failed` avec
résultat et `completed_at` récent, drainage, puis assertions : exactement un
`AgentRequest::CallbackResult`, `original_status == "failed"`, le résultat du
détail, `trace_id`, `parent_task_id`, le message système `[label] failed`, et
`AgentStatus::Thinking`. **Un second drainage ne produit rien** : c'est la moitié du
test qui rougit sur une double livraison (critère 1).

**T5 — un `failed` sans résultat prend `FAILED_TASK_FALLBACK`.** L'assertion porte
sur la constante importée, jamais sur sa valeur littérale.

**T6 — un `completed` sans résultat reste sauté.** Aucun `CallbackResult`, aucun
message. Le pendant du précédent : la règle du repli est propre au `failed`.

**T7 — un `failed` vieux est réclamé sans être livré.** `completed_at` posé à plus
de `STALE_FAILED_CALLBACK_MINUTES` dans le passé — la valeur se dérive de la
constante, pas d'un `6` écrit à la main. Trois assertions : aucun
`CallbackResult`, aucun message système, et la ligne relue en base vaut
`delivered`. C'est le critère 3 en entier, et c'est la garde que V2b-3 rendra
indispensable en retirant le poll.

---

## 5. Ce que ce vol ne fait pas

- **Il ne retire pas le poll.** C'est V2b-3.
- **Il ne gère ni le recul ni la reconnexion**, et n'ajoute pas la variante du
  serveur factice au flux jamais fermé. C'est V2b-2b-4.
- **Il ne touche ni `crates/mika-agent/` ni `crates/mika-a2a/`**, ni aucun fichier
  du plan de contrôle, ni `docs/egress/`.
- **Il n'ouvre aucun ticket**, pas même de suivi (gel jusqu'au 01/12). La suite se
  note `Tracked in: senara-solutions/mika#1727`.

**Dette nommée, non traitée ici.** `mark_task_delivered` reste une écriture locale
sur la base : la TUI, qui est censée devenir un client mince, écrit encore
directement dans SQLite pour réclamer une livraison. Le geste manquant côté spirit
est un endpoint de réclamation. La PR le dira et ne le fera pas.
`Tracked in: senara-solutions/mika#1727`.

---

## Fire-Disposition

Ce plan livre des détecteurs : T1, T2, T3 et T7 ont pour chemin de succès
« aucune violation trouvée » (une trame qui n'entre pas, un candidat qui ne passe
pas, une livraison qui n'a pas lieu). La disposition est donc requise (mika#2306).

**Option retenue : (a) atterrissage armé, allowlist expédiée vide et épinglée
vide.**

Les quatre détecteurs mesurent une surface **neuve de ce vol** : un chemin `failed`
qui n'existe pas sur `e8e6aed49`. Il n'y a donc aucune violation préexistante à
amnistier, et l'allowlist part vide — elle n'est pas un inventaire de dérogations
mais la place nommée où une future exception devrait se voir au `grep`, avec son
ticket et sa condition d'extinction.

Si l'un des quatre rougit à l'atterrissage, la conduite est celle de mika#2201 :
router le nouvel appelant par le site existant, **pas** ajouter une entrée. Une
entrée ajoutée pour faire passer un vol est exactement ce que mika#2309 condamne.

Les options (b) et (c) sont écartées : (b) — expédier désarmé par `#[ignore]` —
laisserait le chemin `failed` sans preuve au moment même où V2b-3 s'apprête à
retirer le poll qui le couvre ; (c) — s'arrêter et escalader — n'a pas lieu d'être,
puisque la seule question qui pouvait bloquer ce vol (`mark_task_delivered`
accepte-t-il un `failed` sans toucher `mika-agent/` ?) est mesurée et tranchée au
§ 0.

---

## Definition of Done

- Les deux commits du § 3 sont sur la branche, chacun vert avec ses tests.
- `cargo test -p mika-cli` est vert ; les tests de V2b-2b-2 le sont sans avoir été
  affaiblis.
- `cargo clippy` et `cargo fmt --check` sont verts.
- `bash scripts/verify-egress-manifest.sh --report` est vert, `docs/egress/`
  intact.
- `bash scripts/verify-pipeline.sh origin/main` est vert.
- `git diff --numstat origin/main...HEAD` donne au plus 279 lignes ajoutées, plan
  exclu.
- `poll_callback_tasks` est identique à sa forme sur `e8e6aed49`.
- La PR porte `Refs mika#1727`, et ni `Closes`, ni `Fixes`, ni `Resolves`, nulle
  part — ni dans le corps, ni dans un message de commit.
- La PR nomme la dette `mark_task_delivered` et porte
  `Tracked in: senara-solutions/mika#1727`.
- `Code review: delegated to mika-qa (mika#2694)` figure dans le corps de la PR.

---

## Acceptance criteria

1. Un callback `failed` de la conversation en cours, avec un résultat, atteint la
   TUI en `AgentRequest::CallbackResult` **une seule fois**. Il porte
   `original_status = "failed"`, le résultat lu sur le détail, `trace_id` et
   `parent_task_id` tirés du détail, et le message système `[label] failed`. Un
   test rougit s'il est livré deux fois.
2. Un `failed` **sans résultat** est livré avec `FAILED_TASK_FALLBACK`. Un
   `completed` sans résultat reste sauté. Chacun a son test.
3. Un `failed` dont le `completed_at` dépasse `STALE_FAILED_CALLBACK_MINUTES` est
   **réclamé sans être livré** : aucun `CallbackResult`, aucun message système, et
   la ligne passe à `delivered`. Un test le montre.
4. Un `failed` déjà réclamé par le poll n'est pas livré. `TaskFailed` entre dans le
   canal, et les trames `Unknown` et `OverflowMarker` restent ignorées sans casser
   l'abonnement. La couture refuse un détail dont le `status` n'est ni `completed`
   ni `failed`. Chaque cas a son test, et les tests existants de V2b-2b-2 restent
   verts sans être affaiblis.
5. `poll_callback_tasks` n'est pas modifié. Egress Manifest Lint est vert **avec
   `docs/egress/` intact** : le pilote lance
   `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il le
   trouve rouge, il s'arrête et le dit. La PR porte `Refs mika#1727` et aucun
   `Closes`, `Fixes` ni `Resolves`.

---

## Taille estimée

Lignes de code ajoutées, commentaires et tests compris, `docs/` et `CLAUDE.md`
exclus, dans l'unité de `git diff --numstat origin/main...HEAD`.

### Commit 1 — `crates/mika-cli/src/commands/tasks.rs`

| Livrable | Lignes |
|---|---|
| § 2.1 `terminal_callback_frame` : or-pattern `TaskCompleted \| TaskFailed`, doc réécrite | 10 |
| § 2.2 `est_un_callback_a_livrer` : `matches!` sur deux statuts, doc | 8 |
| § 2.3 `TaskDetailMirror` : `completed_at` neuf, `status` en `pub(crate)`, doc des onze lecteurs | 10 |
| § 2.4 doc de `CallbackFeed` : pourquoi le canal reste un `task_id` seul | 7 |
| § 2.3 `corps_de_detail()` + test des trois formats à onze paires | 8 |
| T1 test du canal : `TaskFailed` entre, renommage, doc réécrite | 12 |
| T2 + T3 témoin `failed`, refus d'un `delivered` | 13 |

Sous-total : 68

### Commit 2 — `crates/mika-cli/src/tui/app.rs`

| Livrable | Lignes |
|---|---|
| § 2.5 corps du drainage : statut lu avant réclamation, garde d'ancienneté, repli, libellé | 24 |
| § 2.5 doc du drainage réécrite : parité, ordre, absence du résumé stale | 16 |
| Helper `couture_dun_callback_livrable` paramétré + ses deux appelants | 15 |
| T4 un `failed` livré une seule fois | 26 |
| T5 `failed` sans résultat ⇒ `FAILED_TASK_FALLBACK` | 17 |
| T6 `completed` sans résultat ⇒ sauté | 14 |
| T7 `failed` vieux ⇒ réclamé, non livré, ligne `delivered` | 23 |

Sous-total : 135

Total estimé : 203 lignes

Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

La marge est de 76 lignes, soit 27 % de la borne. Elle n'est pas de la réserve pour
élargir : elle absorbe le biais d'estimation habituel sur les quatre tests neufs,
dont la mise en place passe par un helper paramétré plutôt que recopié. Si le total
réel dépassait 279, le vol s'arrête et le dit — il ne recoupe pas de lui-même, et
il ne reporte aucun test.

`Tracked in: senara-solutions/mika#1727`
