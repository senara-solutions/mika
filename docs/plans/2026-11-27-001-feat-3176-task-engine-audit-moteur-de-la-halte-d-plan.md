# mika#3176 — la halte déclarée au groom se compte dans `audit_events`, comme les autres refus

Le vol 2 de mika#3118 (PR #3175, mergée) a fermé le défaut : un plan qui déclare sa propre halte
ne sort plus du grooming en `GROOMED`. `_groom_plan_halt_gate` route la convergence vers
`_escalate_groom`, qui compose dans `tasks.result` une ligne de motif portant l'une de deux causes
neuves — `plan_halt_declared` et `plan_halt_unreadable`.

Ce vol-ci ne touche pas à cette porte. Il livre son **pendant moteur** : le lecteur Rust qui, à la
réception du callback de groom, transforme ces deux causes en une ligne `audit_events`.

**Ce n'est pas une condition d'observabilité, et le plan le dit d'emblée.** Les deux populations —
groom sain et halte — voyagent déjà dans la même colonne `tasks.result` : le recensement a son
dénominateur sans cette ligne (c'est le constat que le § *Le dénominateur existe déjà* du plan du
vol 2 a établi). Ce vol est un **confort de requêtage** : un `GROUP BY` sur `audit_events` au lieu
d'un `LIKE` sur du texte libre, pour que la halte se compte comme les autres refus du groom
(`groom_architect_unreadable`, `groom_proof_plan_mismatch`, `groom_proof_unbound_to_plan`).

## Ce qui est déjà en place, et ne doit pas être refait

Quatre choses sont justes et ce vol ne les touche pas.

1. **Les deux valeurs de cause existent côté shell, déclarées en colonne 0**
   (`dispatch-lib.sh:11327` et `:11334`) :

   ```sh
   GROOM_HALT_CAUSE_PLAN_HALT_DECLARED="plan_halt_declared"
   GROOM_HALT_CAUSE_PLAN_HALT_UNREADABLE="plan_halt_unreadable"
   ```

   C'est exactement la forme que le scan mika#2641 compare (`l == expect`, ligne entière), donc le
   scan étendu de ce vol est **vert à la livraison** : il n'y a aucune violation préexistante à
   excepter. Voir § *Fire-Disposition*.

2. **`_escalate_groom` compose déjà la ligne de motif pour les deux causes**
   (`dispatch-lib.sh:11462–11489`), avec les **deux champs positionnels** du format de fil :
   la cause puis l'étape, interpolées nues après `Groom-halt-cause:` en colonne 0. Aucun des deux
   bras n'écrit de ligne `Verdict:` — l'architecte a bien rendu `GROOMED`, et le refus vient du
   plan, pas de lui (classe mika#1772). Le lecteur de ce vol n'a donc rien à inférer sur le verdict.

3. **Le parseur Rust `groom_halt_cause` (`dispatcher.rs:6578`) lit déjà une cause QUELCONQUE.**
   Il est ancré en début de ligne, jamais par `contains`, et il filtre le second champ sur la
   forme d'une étape. Son test `some_future_cause` épingle qu'une cause inconnue est rendue telle
   quelle. **Rien à changer dans le parseur** : ce vol ajoute un consommateur, pas une orthographe.

4. **Le site de réception du callback de groom est unique et déjà peuplé**
   (`dispatcher.rs:2107`), où `try_report_groom_architect_unreadable` est appelé juste après
   `try_report_pilot_cost_overrun`. Les deux voisins sont best-effort et n'arrêtent rien. Le
   lecteur de ce vol s'y range en troisième, avec la même discipline.

## Où vit le lecteur : `dispatcher.rs`, et non `executor.rs`

Le corps du ticket cite `note_groom_proof_unbound_to_plan` (`skills/executor.rs:2346`) comme
**modèle de la forme d'écriture** — `target_key = issue:<owner>/<repo>#<n>`, la cause en
`after_value` — et le plan du vol 2 a écrit « écrite par `skills/executor.rs` ». Ce vol place le
lecteur dans `task_engine/dispatcher.rs`, et la raison est structurelle, pas stylistique :

- **La population n'est atteignable que depuis le callback.** La cause voyage dans
  `tasks.result`, écrite par `_escalate_groom` après le retour du pilote. Le seul site moteur qui
  tient un `&Task` de callback complété est le bras de `dispatcher.rs:2107`. `executor.rs` lit des
  **preuves de groom en base** (`check_grooming_markers`, la famille `groom_proof_*`), à un autre
  instant et sur une autre donnée.
- **`groom_halt_cause` est privée à `dispatcher.rs`.** L'exporter pour un lecteur hébergé
  ailleurs élargirait la surface du parseur du format de fil sans besoin — et le format de fil
  n'est exact que tant qu'il a un seul lecteur ancré.
- **Le jumeau existe déjà là.** `try_report_groom_architect_unreadable` est précisément « la
  moitié moteur d'un couple à deux moitiés » pour la cause `architect_unreadable` ; ce vol livre la
  même moitié pour deux causes sœurs. Les deux lecteurs partagent `parent_issue_coordinates` et
  `audit_session_id`, extraits en mika#2641 pour éviter une copie verbatim.

La forme d'écriture du modèle `executor.rs`, elle, est **reprise telle quelle** — c'est ce que les
critères d'acceptation épinglent, et c'est ce qui rend la nouvelle ligne homogène avec les trois
autres refus de groom qu'un opérateur agrège.

### Le résidu de forme, nommé : deux `target_key` différents dans un même module

`try_report_groom_architect_unreadable` écrit `issue:mika#2484` — le **nom court**. Ce vol écrit
`issue:senara-solutions/mika#3176` — la **forme longue**, parce que AC1 la prescrit et parce que
c'est la forme de la famille `groom_proof_*` d'`executor.rs`. Les deux formes cohabiteront donc
dans `audit_events`, sur deux `tool_name` distincts.

C'est un résidu assumé, et il est **borné par le `tool_name`** : aucun `GROUP BY` ne mélange les
deux populations, donc aucune agrégation ne devient ambiguë. L'harmoniser signifierait réécrire la
clé d'un détecteur livré et des lignes déjà en base — hors périmètre, et sans gain de mesure.

## La décision de forme : comment le lecteur obtient `owner/repo`

`parent_issue_coordinates` rend aujourd'hui `(Option<String>, Option<u64>)` où le premier membre
est le **nom court** (`repo_short`). AC1 demande la forme longue. Trois voies, et une seule tient :

- **O3 — recomposer `format!("{GITHUB_ORG}/{repo_short}")` au point d'usage.** *Refusée par la
  documentation existante*, qui nomme ce geste comme le défaut à éviter : « Composer
  `senara-solutions` avec le nom court aurait rendu un `owner/repo` juste par coïncidence, vrai
  tant que le dépôt reste dans cette organisation » (`IssueCoordinates`, `dispatcher.rs:6749`).
  `GITHUB_ORG` est « en **contrôle** et jamais en source ».
- **O2 — une fonction sœur qui refait la traversée parent → `reference_url` → parseur.** *Refusée* :
  l'extraction de `parent_issue_coordinates` a eu lieu en mika#2641 précisément parce que ses deux
  lecteurs en portaient une copie « à l'octet près, commentaire compris ». Rouvrir la copie pour
  changer un seul champ de retour est la régression que l'extraction a fermée.
- **O1 — généraliser `parent_issue_coordinates` en `Option<IssueCoordinates>`.** *Retenue.* Le
  parseur rend déjà les deux formes côte à côte (`owner_repo`, `repo_short`) ; seul le helper les
  jette. Les deux appelants existants prennent `.repo_short` à l'usage, le nouveau prend
  `.owner_repo`. Le doc-comment du helper le prévoit littéralement : « Un appelant qui voudrait
  interroger la forge prend `owner_repo` au parseur, pas cette paire » — ce vol fait du helper ce
  passe-plat, au lieu de renvoyer l'appelant au parseur.

Coût d'O1 : la signature, le corps (un `return Some(c)`), et les deux sites d'appel qui projettent.
Les tests existants des deux voisins assertent `issue:mika#2484` et `repo:mika`, inchangés.

## Un `tool_name`, deux `after_value` — et pourquoi c'est la bonne coupe

AC1 fixe un seul `tool_name` (`groom_plan_halt_declared`) et met la cause en `after_value`. C'est
cohérent avec la Mesure du ticket :

```sql
select after_value, count(*) from audit_events
 where tool_name = 'groom_plan_halt_declared' group by 1;
```

La discipline mika#2205 — « deux valeurs et jamais une » — est **tenue par `after_value`**, non par
le `tool_name` : « le plan déclare une halte » et « je n'ai pas pu lire si le plan déclare une
halte » restent deux lignes soustractibles l'une de l'autre. Un second `tool_name` aurait obligé
tout lecteur à connaître les deux noms pour obtenir le total de la famille ; une seule colonne de
regroupement le donne, et le détail avec.

**Le nom porte un abus assumé** : une ligne d'`after_value = plan_halt_unreadable` n'est pas une
halte *déclarée*. Le doc-comment de la constante le dira en clair, parce qu'un opérateur qui lit le
`tool_name` seul se tromperait. Le nom vient du ticket, et le renommer après coup couperait la
population en deux (classe mika#2205, dans l'autre sens).

## Ce que livre ce vol

### 1. Les deux constantes de cause, côté Rust

Dans `task_engine/dispatcher.rs`, immédiatement après
`GROOM_HALT_CAUSE_ARCHITECT_UNREADABLE` (`:6553`), même forme et même discipline de doc :

```rust
pub(crate) const GROOM_HALT_CAUSE_PLAN_HALT_DECLARED: &str = "plan_halt_declared";
pub(crate) const GROOM_HALT_CAUSE_PLAN_HALT_UNREADABLE: &str = "plan_halt_unreadable";
```

Chacune nomme son jumeau shell et dit pourquoi les deux ne se fondent pas. `pub(crate)` parce que
le scan de `canonical_tokens.rs` les importe, comme il importe déjà ses deux voisines.

Plus le `tool_name` d'audit, à côté de `GROOM_ARCHITECT_UNREADABLE_TOOL` (`:6564`) :

```rust
pub(crate) const GROOM_PLAN_HALT_TOOL: &str = "groom_plan_halt_declared";
```

### 2. `try_report_groom_plan_halt`, jumeau de `try_report_groom_architect_unreadable`

Nouvelle fonction dans `dispatcher.rs`, placée juste après son jumeau (après `:6690`), et appelée
au même site (`:2107`), immédiatement après lui.

Forme :

1. `task.result` absent ou vide → retour.
2. `groom_halt_cause(result)` → `None` → retour. **C'est la moitié d'AC2 qui couvre le groom
   sain** : un `Outcome: PLAN_GROOMED` ne porte aucune ligne de motif.
3. La cause n'est ni `GROOM_HALT_CAUSE_PLAN_HALT_DECLARED` ni
   `GROOM_HALT_CAUSE_PLAN_HALT_UNREADABLE` → retour. **C'est l'autre moitié d'AC2** : la cause
   `architect_unreadable` du voisin, la cause `verdict` par défaut, et toute cause future ne sont
   pas cette population. L'appartenance est testée sur l'**égalité** avec une constante, jamais sur
   un préfixe `plan_halt_` — un préfixe accueillerait une cause future sous un compte qui ne
   l'attend pas.
4. `parent_issue_coordinates` (O1) → `Option<IssueCoordinates>`. Une résolution impossible
   **DÉGRADE** la clé vers `task:<id>` et ne supprime jamais la ligne (motif `repo=unknown` de
   mika#2496, déjà appliqué par le jumeau). Le repli reste **discriminant** : `issue:unknown#?`
   confondrait tous les parents illisibles dans une seule ligne du `GROUP BY`.
5. `warn!` structuré : `event = "groom_plan_halt_declared"`, `cause`, `stage`, `repo`, `issue`,
   `task_id`. Mesuré, jamais empêché — le callback est délivré quoi qu'il arrive.
6. `db.log_audit_event(&session_id, GROOM_PLAN_HALT_TOOL, &target_key, None, Some(cause), Some(&reasoning), None)`
   où `target_key = format!("issue:{owner_repo}#{issue}")`, `after_value = cause`, et
   `reasoning = format!("task:{} stage:{}", task.id, stage.unwrap_or("unknown"))`. Un échec
   d'écriture est `warn!`é et rend la main : « mesurer un arrêt ne doit pas pouvoir casser la
   livraison du callback qui le porte ».

Le doc-comment porte les trois propriétés que la revue vérifiera : (a) c'est la moitié moteur d'un
couple dont chaque moitié tombe seule sans produire de faux positif ; (b) `dispatch-lib.sh` n'a
aucun accès à la base (`grep -n "audit_event\|log_audit"` y rend zéro ligne) et son stderr est
structurellement jeté sur un dispatch de groom (classe mika#2050), donc une instrumentation shell
serait **inerte** ; (c) ce lecteur n'arrête rien.

### 3. `parent_issue_coordinates` rend `Option<IssueCoordinates>` (O1)

Signature, corps, doc-comment mis à jour (le paragraphe « le premier membre est le nom court »
devient « les deux formes sont rendues ; chaque appelant choisit la sienne, et dit laquelle »), et
les deux sites d'appel existants projettent `.repo_short` à l'usage.

### 4. Le scan mika#2641 étendu aux deux valeurs

Dans `canonical_tokens.rs`, `mika2641_the_halt_cause_wire_format_is_synchronised_shell_to_rust`
(`:6686`) importe les deux constantes neuves et ajoute **deux assertions de même forme** que celle
qui existe pour `architect_unreadable` :

```rust
let expect_declared =
    format!("GROOM_HALT_CAUSE_PLAN_HALT_DECLARED=\"{GROOM_HALT_CAUSE_PLAN_HALT_DECLARED}\"");
assert!(src.lines().any(|l| l == expect_declared), …);
```

Même message de résolution : **réaligner les deux littéraux, jamais élargir le lecteur Rust à une
seconde orthographe.** La comparaison reste sur la **ligne entière** (`l == expect`), ce qui pin
aussi la forme de déclaration en colonne 0 que `test-dispatch-lib.sh` épingle de son côté.

Le test conserve sa garde d'anti-vacuité existante (un `dispatch-lib.sh` illisible `panic!` au lieu
de rendre une comparaison vide), et son allowlist
`ARCHITECT_UNREADABLE_WIRE_EXCEPTIONS` reste **inchangée et vide**.

Son doc-comment gagne une phrase : le couple a maintenant **trois** causes synchronisées, et
l'ajout d'une quatrième se fait ici.

## Tests

Tous dans le module de tests de `dispatcher.rs`, sur le builder existant
`mika2496_dispatch_pair` / `dispatch_pair_with_reference` — aucun builder neuf : « un second
builder de cinquante lignes aurait été deux formes libres de diverger ».

### Les fixtures

Deux constantes `RESULT`, **recopiées du producteur shell** (`dispatch-lib.sh:11471–11476` et
`:11485–11490`), avec les deux champs positionnels nus après le marqueur. La synchronie des
littéraux est tenue par le scan § 4 ; ces fixtures sont ce qui exerce le **lecteur**.

La fixture `plan_halt_declared` porte la forme qui **piège un prédicat non ancré** : l'étape suit
la cause nue, donc un prédicat qui lirait le marqueur en milieu de ligne écrirait la ligne et le
test de non-comptage rougirait. La fixture `architect_unreadable` existante
(`MIKA2641_UNREADABLE_RESULT`) et celle du refus explicite
(`MIKA2641_EXPLICIT_ESCALATE_RESULT`) sont réutilisées pour AC2.

### T1 (AC1) — `plan_halt_declared` produit exactement une ligne

`try_report_groom_plan_halt` sur la fixture `plan_halt_declared` :
`count_audit_events_by_tool_name(GROOM_PLAN_HALT_TOOL) == 1`,
`target_key == "issue:senara-solutions/mika#2484"`,
`after_value == Some("plan_halt_declared")`, et `reasoning` contient `stage:` suivi de l'étape que
la fixture porte.

### T2 (AC1) — `plan_halt_unreadable` produit sa propre ligne, distincte

Même assertion sur la seconde fixture, avec `after_value == Some("plan_halt_unreadable")`. **C'est
la soustractibilité de mika#2205 qui est testée ici** : les deux fixtures, jouées sur la même base,
donnent deux lignes de même `tool_name` et d'`after_value` différentes — le `GROUP BY` de la Mesure
rend `2` lignes et non `1`. Ce troisième cas (les deux fixtures ensemble) est **asserté dans ce
même test**, parce que c'est la propriété que la Mesure du ticket lit et qu'aucune des deux
fixtures seule ne démontre.

### T3 (AC2) — un groom sain n'en produit aucune

Fixture `Outcome: PLAN_GROOMED` sans ligne de motif → compte `0`. C'est le contrôle négatif du
chemin nominal : sans lui, « le lecteur décide » serait indistinguable de « le lecteur fire sur
tout callback de groom ».

### T4 (AC2) — `architect_unreadable` n'en produit aucune

`try_report_groom_plan_halt` sur `MIKA2641_UNREADABLE_RESULT` → compte `0`, **et** le compte de
`GROOM_ARCHITECT_UNREADABLE_TOOL` reste `0` tant que son propre lecteur n'a pas tourné. C'est le
contrôle qui prouve que les deux lecteurs du même site **filtrent chacun sa population** et ne se
recouvrent pas. Le refus explicite (`MIKA2641_EXPLICIT_ESCALATE_RESULT`, aucune ligne de motif) est
asserté dans le même test.

### T5 — la clé dégradée reste discriminante

`dispatch_pair_with_reference(db, …, None)` sur la fixture `plan_halt_declared` → une ligne est
**écrite** (jamais supprimée), de `target_key == format!("task:{id}")`. Le cas `None` existe déjà
dans le builder, ajouté par mika#2641 pour exactement cette propriété.

### T6 (AC3/AC4) — le scan étendu, rouge par mutation

Rouge-avant démontré **par mutation du littéral shell** : muter
`GROOM_HALT_CAUSE_PLAN_HALT_DECLARED="plan_halt_declared"` en `"plan_halt_declared_x"` dans
`dispatch-lib.sh`, lancer
`cargo test -p mika-agent mika2641_the_halt_cause_wire_format_is_synchronised_shell_to_rust`,
constater le rouge **et le message de résolution**, puis **restaurer par `git checkout --` du
fichier**. La mutation ne quitte jamais le worktree et n'entre dans aucun commit. Les quatre scans
`mika2641_*` repassent verts après restauration (AC4).

## Ce que ce vol ne corrige PAS

- **La porte elle-même et sa sémantique**, livrées par le vol 2 : ni `_iterate_groom_loop`, ni
  `_groom_plan_halt_gate`, ni `_escalate_groom` ne sont modifiés. **Aucune ligne de
  `dispatch-lib.sh` n'est touchée** — ce qui laisse intact le gel de taille de
  `test-dispatch-lib.sh` (mika#3179).
- **Le défaut d'écriture du callout d'un vol N** — mika#3174, fermé par PR #3177.
- **Un scan « écrivain unique » du nom `groom_plan_halt_declared`**, pendant du
  `mika2641_the_architect_unreadable_name_has_a_single_writer`. Le ticket ne le demande pas, et le
  compte de cette famille ne conditionne aucun ticket de suivi — la propriété qui rend le scan
  sole-writer porteur pour `architect_unreadable` est absente ici. L'inventer serait du détecteur
  non demandé.
- **L'harmonisation des deux formes de `target_key`** du module, nommée et bornée au § *Le résidu
  de forme*.

## Ordre de commit

Un seul vol, trois commits, chacun compilant seul.

1. `feat(3176): les deux causes de halte de plan, nommées en Rust` — les trois constantes
   (§ 1) et l'extension du scan mika#2641 (§ 4). **Le détecteur arrive avant ce qu'il garde**, et
   il est vert à la livraison : les deux littéraux shell existent déjà. Ce commit seul satisfait
   AC3 et laisse AC4 vert.
2. `refactor(3176): parent_issue_coordinates rend les deux formes du dépôt` — O1 (§ 3) et la
   projection `.repo_short` aux deux sites existants. Aucun changement de comportement, les tests
   des deux voisins le prouvent.
3. `feat(3176): la halte déclarée au groom se compte dans audit_events` — le lecteur (§ 2),
   son site d'appel, et les tests T1–T5. Satisfait AC1 et AC2.

**Plancher** : le commit 1 seul est mergeable et utile — il fait rougir une divergence de format de
fil, qui est la classe de défaut silencieuse que mika#2641 existe pour fermer. Les commits 2 et 3
n'ont de sens qu'ensemble.

## Risques

**R1 — le lecteur fire sur une cause future.** Fermé par l'égalité sur deux constantes nommées
(§ 2 étape 3) et par T4. Un préfixe `plan_halt_` aurait rendu le risque réel.

**R2 — le refactor O1 casse un des deux voisins.** Faible et couvert : leurs tests assertent
`issue:mika#2484` et `repo:mika`, deux valeurs que la projection `.repo_short` préserve à l'octet.
Le compilateur attrape toute projection oubliée.

**R3 — deux lecteurs au même site doublent une ligne.** Impossible par construction : leurs
prédicats d'appartenance sont des égalités sur des constantes **disjointes**, et T4 l'assert dans
les deux sens.

**R4 — la ligne d'audit échoue et la mesure est incomplète.** Assumé, comme pour les trois voisins :
best-effort, `warn!` et rendre la main. Le dénominateur de secours reste `tasks.result`, qui est
écrit par le shell et ne dépend pas de ce lecteur.

**R5 — l'absence de lignes est lue comme « porte inerte ».** C'est le piège que la section *Mesure*
du ticket nomme. Le doc-comment du `tool_name` portera la phrase : une absence de lignes se lit
« aucune halte depuis le deploy », et la vérification se fait sur `tasks.result`, jamais sur ce
compte seul.

## Definition of Done

- [ ] Les trois constantes (`GROOM_HALT_CAUSE_PLAN_HALT_DECLARED`,
      `GROOM_HALT_CAUSE_PLAN_HALT_UNREADABLE`, `GROOM_PLAN_HALT_TOOL`) sont déclarées dans
      `dispatcher.rs` à côté de leurs voisines, chacune nommant son jumeau shell.
- [ ] `try_report_groom_plan_halt` existe, est appelée une seule fois depuis `dispatcher.rs:2107`,
      et n'arrête rien.
- [ ] `parent_issue_coordinates` rend `Option<IssueCoordinates>` ; les deux appelants existants
      projettent `.repo_short`.
- [ ] `mika2641_the_halt_cause_wire_format_is_synchronised_shell_to_rust` porte deux assertions
      neuves, de même forme et de même message de résolution que l'existante.
- [ ] `ARCHITECT_UNREADABLE_WIRE_EXCEPTIONS` est toujours vide, et
      `mika2641_the_wire_allowlist_is_empty` le vérifie toujours.
- [ ] T1–T5 existent et passent ; T6 (mutation) a été vu rouge, puis le littéral restauré par
      `git checkout --`, et le worktree est propre côté `dispatch-lib.sh`.
- [ ] Les quatre scans `mika2641_*` existants sont verts.
- [ ] `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check` passent.
- [ ] `scripts/verify-pipeline.sh origin/main` passe.
- [ ] Aucune ligne de `skills/bundled/_shared/` n'est modifiée dans le diff final.

## Acceptance criteria

- **AC1** : un callback de groom dont `tasks.result` porte `Groom-halt-cause: plan_halt_declared`
  produit exactement une ligne `audit_events` `groom_plan_halt_declared`, de `target_key` le ticket
  groomé et d'`after_value` `plan_halt_declared`. Même chose pour `plan_halt_unreadable`.
- **AC2** : un callback de groom sain (`Outcome: PLAN_GROOMED`) et un callback portant une autre
  cause (`architect_unreadable`) n'en produisent aucune.
- **AC3** : le scan mika#2641 échoue si l'une des deux valeurs diverge entre `dispatch-lib.sh` et
  sa constante Rust. Rouge-avant démontré par mutation d'une des deux valeurs, puis restauration.
- **AC4** : les quatre scans `mika2641_*` existants restent verts ;
  `scripts/verify-pipeline.sh origin/main` passe.

## Fire-Disposition

Ce vol livre un détecteur : les deux assertions neuves de
`mika2641_the_halt_cause_wire_format_is_synchronised_shell_to_rust`, dont le chemin de succès est
« aucune divergence entre le shell et le Rust ».

**Disposition retenue : (a) exception nommée en allowlist — et l'allowlist est livrée VIDE.**

- L'allowlist est celle qui existe déjà pour ce scan : `ARCHITECT_UNREADABLE_WIRE_EXCEPTIONS`
  (`canonical_tokens.rs:6672`), grep-visible, livrée vide par mika#2641 et **inchangée** par ce vol.
- **Zéro violation préexistante, et c'est vérifié plutôt que supposé** : les deux littéraux shell
  sont déjà déclarés en colonne 0 sous la forme exacte que le scan compare
  (`dispatch-lib.sh:11327` : `GROOM_HALT_CAUSE_PLAN_HALT_DECLARED="plan_halt_declared"`,
  `:11334` : `GROOM_HALT_CAUSE_PLAN_HALT_UNREADABLE="plan_halt_unreadable"`). Les deux assertions
  neuves sont donc **vertes à la livraison** : il n'y a rien à excepter.
- **L'assertion auto-nettoyante existe déjà et couvre les deux valeurs neuves** :
  `mika2641_the_wire_allowlist_is_empty` rougit dès qu'une entrée est ajoutée, avec le motif
  « excepter le fichier shell rendrait le scan vert en cessant de regarder la moitié qu'il existe
  pour confronter » (mika#2201). Une exception déposée pour l'une des deux nouvelles valeurs ferait
  immédiatement rougir ce test-là — il n'y a pas d'emplacement où déposer la prochaine infraction.
- **Quand le scan tire, on réaligne les deux littéraux**, on n'excepte pas le fichier. C'est le
  message de résolution que les deux assertions neuves portent, repris mot pour mot de l'existante.

Le détecteur est donc **armé à la livraison**, sans exception et sans suivi à ouvrir.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/task_engine/dispatcher.rs` — trois constantes + doc (§ 1) | 35 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` — `try_report_groom_plan_halt` + site d'appel (§ 2) | 70 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` — refactor `parent_issue_coordinates` + 2 sites (§ 3) | 20 |
| `crates/mika-agent/src/canonical_tokens.rs` — scan étendu (§ 4) | 35 |
| tests (`dispatcher.rs`, module `tests` — 2 fixtures + T1–T5) | 120 |

Total estimé : 280 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 280 est sous les deux bornes (429 par vol, 1000 par PR), avec
~35 % de marge sur la plus serrée. Trois raisons rendent l'estimation fiable plutôt qu'optimiste :

1. **Le périmètre est fermé par le ticket**, en trois livrables énumérés, sans découverte à faire.
2. **Un jumeau existe pour chaque livrable** : `try_report_groom_architect_unreadable` pour le
   lecteur, sa constante voisine pour les noms, l'assertion `architect_unreadable` pour le scan, et
   `mika2496_dispatch_pair` pour les fixtures. Aucun échafaudage neuf.
3. **Aucune ligne de shell**, donc aucun tour dépensé dans `test-dispatch-lib.sh` ni dans son gel
   de taille (mika#3179) — c'est le poste qui a fait déborder les vols précédents de cette famille.

Le poste le plus lourd est celui des tests (120 lignes pour cinq cas plus deux fixtures), et il est
irréductible : T3 et T4 **sont** AC2, T5 est la propriété de dégradation que mika#2496 a dû nommer,
et T6 n'ajoute aucune ligne (c'est une mutation jouée puis restaurée).
