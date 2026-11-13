# mika#2886 — L'évaluateur de vols lit l'issue du pilote, jamais la tâche parente

- **Ticket :** senara-solutions/mika#2886
- **Type :** `fix`
- **Bloque :** mika#2698 (ouverture de `MIKA_MANAGER_EVALUATE_WRITE`)
- **Taille auto-déclarée par le ticket :** S

## Contexte

`milestone_manager/` tourne en **ombre** depuis le vol 1 de mika#2698 : il
calcule des verdicts de vol, les écrit dans `~/.mika/manager/sink/`, et n'agit
sur rien. Le vol 2 de #2698 ouvre `MIKA_MANAGER_EVALUATE_WRITE` — et sa
précondition est un **compte de concordance** entre les verdicts d'ombre et la
réalité observable des PR.

Ce compte est aujourd'hui impossible, et pas parce qu'il serait mauvais : parce
qu'il est **constant**. Depuis le redémarrage f14e69d9 du 2026-10-19 06:00:32Z
(qui porte #2799), le puits a rendu **531 verdicts, tous identiques** :

> `mort — cause non classée (no_outcome_line) — reprise à la main`

Le verdict est faux pour tout vol qui a ouvert une PR. Le rapport de 08:04:26Z
classe le vol de #2875 « mort, 121 tours, reprise à la main », alors que ce vol a
ouvert #2881, mergée le 2026-10-23 à 08:01:18Z.

Un instrument dont toutes les mesures sont égales ne mesure rien. Tant qu'il
rend 531/531 la même valeur, le vol 2 de #2698 n'a aucun chiffre sur lequel
s'appuyer — d'où le lien de blocage.

## Diagnostic

La chaîne est entièrement vérifiée dans le code, pas seulement dans le texte du
ticket. Quatre maillons, chacun correct en lui-même :

**1. L'enfant pilote naît sans `reference_url`.**
`build_callback_task` (`crates/mika-agent/src/skills/executor.rs:5591`) est le
site de construction **unique** de la tâche enfant d'un dispatch pilote. Son
littéral `NewTask` porte `reference_url: None` et
`parent_task_id: Some(<id de la parente>)`. C'est délibéré : l'URL du ticket
est un fait de la **parente** `ready-label:`, et la dupliquer sur l'enfant
créerait deux lignes réclamant le même ticket.

**2. La ligne `Outcome:` vit sur l'enfant.**
C'est la session pilote qui l'écrit, dans le `result` de **sa** tâche —
l'enfant `long_running:run_claude_pilot`.

**3. Seuls quelques champs extraits remontent vers la parente.**
`try_extract_callback_metadata` (`crates/mika-agent/src/task_engine/dispatcher.rs:3848`)
lit le `result` de l'enfant, en extrait des champs nommés
(`extract_callback_fields`) et les fusionne dans les **métadonnées** de la
parente. Les tours y passent — d'où le « 121 tours » du rapport, qui est juste.
La ligne `Outcome:` n'y passe pas : elle n'est pas un champ extrait, elle est du
texte de `result`, et `result` n'est pas fusionné.

**4. Le lecteur de vols ne sélectionne que sur l'URL.**
`Database::list_finished_implement_flights_in_container`
(`crates/mika-agent/src/db/tasks.rs:4954`) filtre sur
`reference_url IN (<variantes>)`. La seule ligne qui porte l'URL est la
**parente**. Son `result` est vide.

Relevé pour #2875, tel que le ticket le donne :

| tâche | label | `reference_url` | `result` |
|---|---|---|---|
| `752be6cf` | `ready-label: senara-solutions/mika#2875` | …/issues/2875 | vide |
| `d2dbf5e8` | `long_running:run_claude_pilot` | NULL | `Outcome: PR_OPENED — …/pull/2881` |

Le reste suit mécaniquement, et **sans aucun défaut dans l'évaluateur** :
`task_to_finished_flight` (`flight_source.rs`) projette le `result` vide →
`classify_issue` (`milestone_manager/evaluator.rs`) appelle
`line_value(result, "Outcome:")` qui rend `None` → `death_cause(result, None)`
tombe dans son bras terminal `None => DeathCause::Unknown("no_outcome_line")` →
`recommend` mappe `Unknown(_)` sur `ManualResumption`. 531 fois.

**Pourquoi les tests ne l'ont pas vu.** Les tests de mika#2792 sèment un vol
dont la ligne porteuse de l'URL porte **aussi** l'`Outcome:`. Le doc-module de
`db/tests/vols_termines_du_manager.rs` le consigne déjà en toutes lettres. C'est
une forme que la production ne produit jamais : la fixture a validé le lecteur
contre une topologie qui n'existe pas. C'est la classe de défaut que seul un
test de **forme réelle** ferme — d'où AC1.

**Ce qui n'est PAS la cause**, et qu'il ne faut pas « réparer » :

- l'absence de `agent_id` dans la requête (mika#2792 — portée conteneur, voulue) ;
- l'ensemble clos de variantes d'URL (mika#2638 — l'égalité évite que `#2693`
  attrape `#26930`) ;
- `COALESCE(completed_at, updated_at)` (un vol `failed`/`cancelled` n'a souvent
  que `updated_at`, et ce sont exactement les vols morts qui intéressent le
  manager) ;
- le filtre de statut terminal en Rust (`is_terminal_task_status` : la règle
  « un statut inconnu n'est pas terminal » n'a pas d'expression en SQLite) ;
- `issue_urls` vide → `vec![]` (ne jamais rendre *tous* les vols).

Ces cinq décisions sont déclarées dans le doc-comment du lecteur. Le correctif
les préserve toutes les cinq ; c'est une contrainte, pas un commentaire.

## Décision de conception

### Le mécanisme : une seconde requête, les mêmes liaisons, un type qui dit ce qu'il porte

Le ticket laisse le mécanisme au groom. Trois formes ont été pesées.

**Rejetée — substituer `result` dans le `Task` rendu.** Garder la signature
`Vec<Task>` et y écraser `result` par celui de l'enfant coûte zéro ligne chez
les appelants, et rend un `Task` **menteur** : un champ dont la valeur ne vient
pas de la ligne que l'objet prétend être. Le prochain lecteur de cette méthode
n'a aucun moyen de l'apprendre. C'est la classe mika#2205 appliquée à un type :
deux populations sous un seul nom.

**Rejetée — une requête par vol depuis `DbFlightSource`.** Lisible, mais
N+1 : `since = None` au premier cycle après un redémarrage ne borne rien, et le
nombre de vols d'un jalon n'est pas borné par construction.

**Retenue — deux requêtes dans la même fermeture, un type de retour dédié.**
`list_finished_implement_flights_in_container` rend

```rust
/// Un vol terminé, et l'issue que son pilote a écrite (mika#2886).
///
/// Les deux champs viennent de **deux lignes différentes** : `flight` est la
/// tâche qui porte l'URL du ticket (la parente `ready-label:`), `pilot_result`
/// est le `result` du dernier enfant pilote terminé. C'est la topologie que
/// `build_callback_task` produit, et la raison d'être de ce type : un `Task`
/// dont le `result` aurait été écrasé ne dirait pas d'où il vient.
pub struct FinishedFlightRow {
    pub flight: crate::task_state::tasks::Task,
    /// `None` quand le vol n'a aucun enfant pilote terminé portant un
    /// `result` — jamais une chaîne vide (motif mika#2331 : absent ≠ vide).
    pub pilot_result: Option<String>,
}
```

La seconde requête reprend **exactement** le motif en place de
`find_dispatch_children_for_issue_url` (`db/tasks.rs:2367`) — la jointure
enfant↔parente clavetée sur l'URL du ticket :

```sql
SELECT <TASK_COLUMNS préfixées child.>
FROM tasks child
JOIN tasks parent ON child.parent_task_id = parent.id
WHERE COALESCE(parent.dispatch_class, 'implement') = 'implement'
  AND parent.reference_url IN (<mêmes placeholders>)
  AND COALESCE(parent.completed_at, parent.updated_at) > ?1
```

Deux propriétés de cette forme sont ce qui la fait choisir :

- **Zéro paramètre supplémentaire.** Le `WHERE` est le `WHERE` de la requête 1,
  déplacé sur `parent`. Les deux requêtes partagent la liaison à l'identique,
  donc il n'y a **ni liste d'identifiants à passer, ni découpage en paquets, ni
  limite `SQLITE_MAX_VARIABLE_NUMBER`** à surveiller.
- **Une seule traversée de canal.** Les deux requêtes vivent dans la même
  fermeture `with_db`, donc le coût côté `AsyncDatabase` est celui d'un aller-retour,
  pas de deux.

### Ce qui se décide en Rust, et pourquoi pas en SQL

Trois filtres s'appliquent aux enfants ramenés, **tous les trois en Rust** :

| filtre | prédicat | pourquoi pas en SQL |
|---|---|---|
| population | `crate::task_engine::engine::is_pilot_dispatch_label(&t.label)` | la liste `PILOT_DISPATCH_LABELS` a **un** site de vérité ; une copie en SQL en serait un second, et c'est exactement la divergence que le doc-comment du lecteur refuse déjà pour les statuts |
| vivacité | `crate::task_state::tasks::is_terminal_task_status(&t.status)` | « un statut inconnu n'est pas terminal » n'a pas d'expression en SQLite — la raison déjà écrite, une ligne plus haut, pour la parente |
| contenu | `result` présent **et** non vide | un enfant terminé sans `result` n'apprend rien ; le retenir masquerait un enfant antérieur qui, lui, porte l'issue |

Le prédicat de label est **importé**, jamais recopié : `is_pilot_dispatch_label`
est `pub(crate)` et `task_engine::engine` est `pub mod`, donc `db.rs` l'atteint
tel quel. Il exclut par construction `long_running:run_claude_pilot:deferred`
(ligne-enveloppe, sans PID) et `long_running:build_mika` (sans transcript ni
worktree — mika#2875).

### Le choix du « dernier »

Un vol peut avoir plusieurs enfants pilotes : chaque reprise, chaque itération
CI en crée un. L'issue courante du vol est celle du **dernier terminé**. Ordre
retenu : `COALESCE(completed_at, updated_at)` décroissant, `id` décroissant en
départage. Le même `COALESCE` que la parente, pour la même raison : un enfant
`failed` n'a souvent que `updated_at`, et c'est précisément celui dont le
verdict compte.

Le tri se fait en Rust, sur l'ensemble déjà filtré — un `ORDER BY` en SQL ne
peut pas trier ce que les trois prédicats ci-dessus n'ont pas encore écarté.

### La projection, et l'ordre de préférence

`task_to_finished_flight` (`flight_source.rs`) devient :

```rust
result: row.pilot_result.or(row.flight.result),
```

L'enfant d'abord, la parente en repli. L'ordre n'est pas arbitraire : l'enfant
est l'**écrivain** de la ligne `Outcome:`, la parente n'en est jamais qu'un
porteur accidentel. Le repli garde vert tout chemin où la parente porte
légitimement un `result` (dispatch non piloté, ligne ancienne, fixture).

Le reste de `FinishedFlight` ne bouge pas : `task_id`, `agent_id`, `status`,
`metadata`, `reference_url`, `completed_at` restent ceux de la **parente**, qui
est bien le vol. Seul `result` change de source. En particulier `metadata`
continue de venir de la parente, et c'est ce qui fait que `PilotFields::parse`
trouve toujours les tours là où `try_extract_callback_metadata` les a mis.

### Rayon d'explosion : nul sur le verdict

Aucune ligne de `milestone_manager/evaluator.rs` ne change. `classify_issue`,
`death_cause`, `recommend` et leur matrice exhaustive sont intouchés — « le
verdict de l'évaluateur ne change pas », comme le ticket le demande. Ce qui
change est **l'entrée** qu'on lui donne. Conséquence directe : le même
`Outcome: PR_OPENED` qui produisait `Dead { Unknown("no_outcome_line") }` →
`ManualResumption` produira `PrOpened { … }` → `MergeCandidate`, par le chemin
qui existait déjà et qui n'avait jamais été emprunté en production.

`milestone_manager/` reste en lecture seule : le correctif ne touche ni
`cadence.rs` ni `assessor.rs`, et `flight_source.rs` reste **hors** de l'arbre
du module — placement voulu, parce que `no_dispatch_test.rs` grep chaque `.rs`
du module à la recherche de jetons d'autorité d'écriture. Les tests d'AC1 et AC2
atterrissent donc eux aussi hors du module, dans `flight_source.rs`.

### Ce que ce vol NE fait pas, et pourquoi

- **Il ne récrit pas les fixtures de mika#2792.** Elles valident la borne
  temporelle, l'ensemble clos de variantes, le repli sur `updated_at` et la
  projection des sept données — quatre propriétés indépendantes de qui porte
  l'`Outcome:`. Les réécrire ferait perdre leur couverture pour en gagner une
  que AC1 apporte déjà. Le doc-module gagne une ligne renvoyant à AC1 pour la
  forme réelle.
- **Il n'ajoute pas de garde structurelle sur la forme des fixtures.** Un scan
  qui refuserait une fixture portant `Outcome:` sur la ligne URL-porteuse serait
  un **second** mécanisme pour la propriété qu'AC1 tient déjà par le
  comportement. AC1 rougit si quelqu'un recasse le lecteur ; c'est la garde.
- **Il ne reprend pas la garde `json_valid` de `find_dispatch_children_for_issue_url`.**
  Elle existe là-bas parce que cette requête fait un `json_extract` sur
  `child.metadata`, et qu'un `json_extract` sur du non-JSON **échoue dur** en
  propageant son erreur hors de tout le `query_map` (une seule ligne malformée
  répondrait « aucun pilote » pour le ticket entier). La requête de ce vol ne
  lit aucun JSON en SQL : `metadata` continue d'être parsé en Rust par
  `PilotFields::parse`, qui est tolérant. Reprendre la garde ici serait du culte
  de forme.
- **Il n'ouvre pas `MIKA_MANAGER_EVALUATE_WRITE`.** C'est le vol 2 de mika#2698,
  et sa précondition est un compte de concordance que ce correctif rend
  seulement *possible*. Le compte « repart de zéro au redémarrage qui charge ce
  correctif », comme le ticket le dit.

## Livrables

### D1 — `crates/mika-agent/src/db/tasks.rs`

1. Le type public `FinishedFlightRow` (champs + doc-comment ci-dessus).
2. `list_finished_implement_flights_in_container` rend
   `Result<Vec<FinishedFlightRow>>` :
   - requête 1 inchangée (les cinq décisions déclarées restent, et leur
     doc-comment aussi — il gagne un sixième paragraphe nommant la jointure
     enfant et mika#2886) ;
   - requête 2, la jointure ci-dessus, mêmes liaisons ;
   - en Rust : les trois filtres, le regroupement par `parent_task_id` dans une
     `HashMap<String, Task>` gardant le plus récent par
     `(COALESCE(completed_at, updated_at), id)` ;
   - assemblage dans l'ordre de la requête 1 (`ASC` sur la borne temporelle —
     inchangé, l'ordre de lecture du manager en dépend).

### D2 — `crates/mika-agent/src/async_db.rs`

Le type de retour du wrapper suit (`Result<Vec<FinishedFlightRow>>`). Le
doc-comment existant — qui explique pourquoi c'est la seule méthode du fichier à
ne pas lire `self.agent_id` (mika#2792) — est conservé mot pour mot et gagne un
renvoi vers `FinishedFlightRow`.

### D3 — `crates/mika-agent/src/flight_source.rs`

`task_to_finished_flight` prend un `FinishedFlightRow` et projette
`result: row.pilot_result.or(row.flight.result)`. Le `warn!` du chemin d'erreur
(`flight_source_read_failed`, qui rend `Vec::new()` plutôt que de tuer le cycle)
est inchangé. Le doc-module gagne le paragraphe qui nomme la topologie
parente/enfant — c'est le fait que ce fichier existe pour porter.

### D4 — tests de bout en bout, dans `crates/mika-agent/src/flight_source.rs` § `tests`

Deux tests, qui vont de la base au **verdict** (`evaluate_flights`), parce que
AC1 et AC2 portent sur un verdict et non sur une projection :

- `mika2886_la_forme_reelle_rend_un_verdict_de_pr_ouverte` (**AC1**) — sème la
  parente `ready-label:` (via `crate::db::tests::groom_parent` +
  `dispatch_class: Some("implement")` + l'URL du ticket, `result` vide), puis
  l'enfant **par le constructeur de production**
  `skills::executor::build_callback_task(…, "run_claude_pilot", …)` —
  `pub` depuis mika#2272 précisément pour que les fixtures cessent d'écrire des
  `NewTask` à la main — avec `parent_task_id: Some(<parente>)`, donc
  `reference_url: None` par construction, et complété avec
  `Outcome: PR_OPENED — <url de PR>`. Assertion : `FlightIssue::PrOpened` et
  `FlightRecommendation::MergeCandidate`. Le `pr_state` vaut `None` (aucun accès
  GitHub), et la matrice de `recommend` mappe déjà `PrOpened` + `None` sur
  `MergeCandidate` — le test est donc hermétique.
- `mika2886_un_enfant_sans_ligne_outcome_reste_une_cause_non_classee` (**AC2**) —
  même semis, mais le `result` de l'enfant ne porte aucune ligne `Outcome:`.
  Assertion : `DeathCause::Unknown("no_outcome_line")`. C'est le contrôle
  négatif : il sépare « le lecteur lit l'enfant » de « le lecteur rend
  `PrOpened` quoi qu'il arrive », et sans lui AC1 serait satisfait par un
  correctif qui truque le verdict.

`register_agent` est appelé pour chaque `agent_id` semé — `tasks.agent_id` est
une FK, les tests de mika#2792 portent déjà le helper.

### D5 — tests du lecteur, dans `crates/mika-agent/src/db/tests/vols_termines_du_manager.rs`

- adaptation des helpers au nouveau type de retour (`lire`/`lire_sur` mappent
  `.flight.id`) ; les six tests de #2698/#2792 restent verts **sans changer
  leurs assertions** — c'est la preuve que les cinq décisions déclarées sont
  intactes ;
- `mika2886_le_dernier_enfant_pilote_gagne` — deux enfants pilotes terminés sur
  la même parente, à des instants différents ; `pilot_result` est celui du plus
  récent ;
- `mika2886_un_enfant_hors_population_nest_pas_lu` — un enfant
  `long_running:build_mika` portant un `result` n'alimente pas `pilot_result`,
  qui reste `None` ;
- une ligne au doc-module renvoyant à AC1 pour la forme réelle (le doc consigne
  déjà que les fixtures de #2792 ne la portent pas).

### D6 — documentation (hors compte de lignes)

`docs/agent-internals/milestone-manager-phase-1.md` : la topologie
parente/enfant, pourquoi l'`Outcome:` vit sur l'enfant, et la mesure 531/531 qui
l'a révélée. C'est le fait qu'un futur lecteur du manager redécouvrirait
autrement par la même enquête.

## Ordre de commit

1. **D4 + D5 nus (rouge-avant, AC3).** Les deux tests d'AC1/AC2 contre le code
   actuel. AC1 doit rougir sur
   `Unknown("no_outcome_line")` reçu là où `PrOpened` est attendu ; AC2 doit
   être **vert dès ce commit** (il décrit le comportement actuel sur une
   fixture dont l'enfant ne porte pas d'`Outcome:`), et c'est voulu : un
   contrôle négatif qui rougirait avant le correctif ne contrôlerait rien.
   La sortie rouge d'AC1 est collée dans le corps de la PR.
2. **D1 + D2 + D3.** Le correctif. AC1 passe au vert, AC2 reste vert, les six
   tests de #2698/#2792 restent verts.
3. **D5 (les deux tests de population/récence)** — ils ne peuvent exister
   qu'après D1, puisqu'ils assertent sur `pilot_result`.
4. **D6.** La documentation.

## Contrat de vérification

- `cargo test -p mika-agent` — la suite entière, pas seulement les modules
  touchés : `FinishedFlightRow` change une signature publique de `Database`.
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `make verify-bundled-skills` (aucun bundle touché, mais la porte est
  pré-merge)
- `scripts/verify-pipeline.sh origin/main`
- Sortie rouge d'AC1 avant le correctif, collée dans le corps de la PR (AC3).

## Definition of Done

- `list_finished_implement_flights_in_container` rend, pour chaque vol, le
  `result` du dernier enfant pilote terminé quand il en existe un, et ses cinq
  décisions déclarées sont inchangées.
- Aucune ligne de `milestone_manager/evaluator.rs` n'est modifiée.
- AC1 rougit contre `origin/main` et passe au vert avec le correctif, dans la
  même PR.
- AC2 est vert avant et après : la cause `no_outcome_line` reste atteignable.
- Les six tests de mika#2698/#2792 restent verts sans que leurs assertions
  changent.
- Le contrat de vérification ci-dessus passe en entier.
- La PR porte `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

- AC1 : un test sème la forme réelle (une parente `ready-label:` avec l'URL du
  ticket et un résultat vide, plus un enfant `long_running:run_claude_pilot`
  avec `reference_url` NULL et `Outcome: PR_OPENED — <url>`), et exige le
  verdict PR ouverte, merge-candidat.
- AC2 : contrôle négatif, un enfant sans ligne `Outcome:` rend toujours
  `no_outcome_line`.
- AC3 : rouge-avant, AC1 rougit contre le code actuel.
- AC4 : sonde après déploiement, le premier rapport du puits qui suit
  l'atterrissage d'un vol ne classe pas « mort » un vol dont la PR est ouverte.

## Fire-Disposition

Ce plan livre des détecteurs : AC1, AC2 et les deux tests de D5 sont du code
dont le chemin de succès est « aucune violation trouvée ». La section est donc
**requise**, et non N/A.

**Option retenue : (a) exception nommée en allowlist — allowlist livrée VIDE.**

Le détail d'implémentation, parce que « vide » doit être une propriété
constatée et non une espérance :

- **La population des violations existantes est connue et vaut une.** C'est le
  lecteur de `db/tasks.rs:4954`, et le **même vol** la corrige. À
  l'atterrissage, les quatre détecteurs sont **armés et verts** : aucune
  exception n'est écrite, parce qu'aucune n'est nécessaire.
- **La propriété est constatée par l'ordre de commit, pas par une déclaration.**
  Le commit 1 pose AC1 nu et le fait rougir (AC3) ; le commit 2 le fait passer
  au vert. Un détecteur qui serait resté rouge aurait rendu le commit 2
  incomplet, visiblement, dans la même PR. Il n'existe donc pas d'état
  intermédiaire où un détecteur de ce vol atterrirait désarmé ou toléré.
- **L'assertion auto-nettoyante est AC2 lui-même.** C'est le contrôle négatif :
  il rougit si quelqu'un « corrige » le lecteur en rendant `PrOpened`
  inconditionnellement, c'est-à-dire exactement la forme qu'une exception
  complaisante prendrait ici. La paire AC1/AC2 est donc son propre garde de
  staleness : elle ne peut pas être satisfaite par un contournement.
- **Si un cas non prévu rougit au moment du vol** — une topologie parente/enfant
  que ce diagnostic n'a pas vue — la conduite est de **corriger le lecteur**, pas
  d'inscrire une exception. Une exception ici dirait « ce vol-là n'a pas droit à
  son issue », c'est-à-dire reproduirait à l'échelle d'une ligne le défaut que
  le ticket décrit à l'échelle de 531.

Les options (b) et (c) sont écartées et pour une raison nommée : (b) livrer
désarmé (`#[ignore]`) laisserait AC1 sans effet alors que le correctif qui le
verdit est dans la même PR — le suivi qu'il faudrait tracker serait vide ; (c)
halte-et-remontée n'a pas de question à poser à l'opérateur : le mécanisme est
décidé ci-dessus et le ticket le lui laissait explicitement (« le mécanisme
exact se décide au groom »).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/db/tasks.rs` (`FinishedFlightRow`, 2ᵈᵉ requête, filtres, assemblage, doc) | 85 |
| `crates/mika-agent/src/async_db.rs` (type de retour + doc) | 10 |
| `crates/mika-agent/src/flight_source.rs` (projection + doc-module) | 20 |
| tests AC1/AC2 (`crates/mika-agent/src/flight_source.rs` § `tests`) | 110 |
| tests du lecteur (`crates/mika-agent/src/db/tests/vols_termines_du_manager.rs`) | 65 |

Total estimé : 290 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 290 estimées contre une borne par
vol de 429 et un seuil par PR de 1000 : les deux bornes passent, la plus serrée
(429) avec ~32 % de marge. Le périmètre est structurellement petit — **une**
requête SQL ajoutée, **un** type de quatre lignes, **une** expression `.or()`
dans la projection, et aucun changement dans l'évaluateur. Les trois sites de
production totalisent 115 lignes ; les 175 restantes sont du test, dont le coût
en tours est faible (ils se sèment avec des helpers déjà présents dans les deux
modules, et le constructeur de production `build_callback_task` évite d'écrire
un `NewTask` à la main). Le ticket s'auto-déclare **S** et la mesure le
confirme. Aucun découpage en vols n'est donc requis, et aucune ligne
`Tracked in:` n'est due.

Le poste qui pourrait déraper est l'adaptation des appelants au nouveau type de
retour (D2, D5) : il est borné et dénombré — `grep -rn
"list_finished_implement_flights_in_container" crates/mika-agent/src` rend
**cinq** sites non-doc, tous listés dans les livrables (`db/tasks.rs`,
`async_db.rs`, `flight_source.rs`, et deux helpers de
`db/tests/vols_termines_du_manager.rs`). Il n'y a pas de sixième appelant à
découvrir en vol.
