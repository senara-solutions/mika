# fix(3044) vol 2 — le scan de lecteur unique du prédicat de réhabilitation, et le dernier discriminant à la porte

Ticket : senara-solutions/mika#3044
Plan du vol 1 : `docs/plans/2026-11-05-004-fix-3044-dispatch-une-parente-self-dev-fauch-e-plan.md`
Vol 1 : PR #3045, mergée le 2026-11-05T14:09:59Z (`b7254234f`)

---

## 0. Où en est le ticket, et pourquoi il est encore ouvert

**Le comportement est livré. Ce qui le *tient* ne l'est pas.**

Le vol 1 a livré le prédicat (`crates/mika-agent/src/parent_rehabilitation.rs`),
son site de lecture unique dans `validate_dispatch_readiness`, la transition
gardée `failed` → `in_progress`, les trois lignes d'audit et six tests. Il ferme
AC1 en entier, AC2 à six cas sur sept, et AC3 **dans sa substance** — un seul
prédicat, un seul site, les refus nommés.

mika#3044 est resté **OPEN délibérément** : le corps de la PR #3045 le dit en
toutes lettres (*« ce ticket ne doit pas être fermé par ce merge : le vol 2
(T7–T9) y est dispatché »*), et le gel des tickets jusqu'au 01/12 interdisait
d'ouvrir un ticket de suivi pour lui. Ce plan est celui de ce vol 2.

Ce qui reste est de **trois tests, dont un détecteur structurel** :

| | ce qui reste | ce que ça ferme |
|---|---|---|
| **T7** | la ligne de bord `reaper_db_error` (`after_value` NULL) comme seule ligne du reaper | le dernier cas de population d'AC2 |
| **T8** | le **scan de lecteur unique** : un second site appelant `parent_rehabilitation::decide` sous `src/` est refusé, allowlist vide | AC3, sa moitié qui résiste à la dérive |
| **T9** | le contrôle de bonne foi de T8 | que T8 rougisse vraiment |

**L'ordre est maintenant possible, et il ne l'était pas avant.** Le § 4 du plan
du vol 1 refusait d'atterrir un scan de lecteur unique sur un prédicat sans
lecteur — rouge par construction. Le vol 1 ayant atterri, `decide` a exactement
un appelant en production, mesuré sur `b7254234f` :

```
$ grep -rn "parent_rehabilitation::decide" crates/mika-agent/src/
skills/executor.rs:4066:        match crate::parent_rehabilitation::decide(db, &task).await {
skills/executor.rs:18500:                crate::parent_rehabilitation::decide(&db, &task).await,   ← dans `mod tests`
```

Le scan atterrit donc **armé et vert**, et son allowlist n'a aucune entrée à
porter.

### Gel des tickets (jusqu'au 01/12)

Ce vol n'ouvre aucun ticket de suivi. Un reste éventuel va en commentaire sur
mika#3044 ou dans la PR.

---

## 1. Le site, sur `b7254234f`

| élément | lieu | ce qu'il donne à ce vol |
|---|---|---|
| le prédicat | `crates/mika-agent/src/parent_rehabilitation.rs` (392 l., **aucun `mod tests`**) | le module que T8/T9 habiteront |
| le site de lecture unique | `crates/mika-agent/src/skills/executor.rs` l. 4066, dans `validate_dispatch_readiness` | le site que T8 compte, et dont il vérifie le chemin |
| le discriminant D1 | `crates/mika-agent/src/db.rs` l. 2117-2137, `latest_state_transition_for_target` — `WHERE … before_value IS NOT NULL AND after_value IS NOT NULL` | ce que T7 épingle |
| la ligne de bord | `crates/mika-agent/src/task_engine/engine.rs` l. 4747-4760 — `log_audit_event(…, audit_tool_name, …, Some("in_progress"), None, Some("reaper_db_error: …"), …)` | la forme exacte que T7 doit monter |
| les fixtures de T1–T6 | `executor.rs` l. 18123-18520, `mod mika3044_parente_fauchee` | `parente_fauchee`, `transition`, `dispatch`, `refus`, `statut`, `est_le_refus_dhier` |
| le patron du scan | `agent_loop/tests/gardes_de_source.rs` l. 1317-1500 (mika#2425) et `kg/budget.rs` l. 177-260 (mika#1833) | la forme entière : allowlist vide auto-nettoyante → prédicat de ligne → scan → contrôle de bonne foi |

**La ligne de bord porte le MÊME `tool_name` que le reap.** `engine.rs` passe
`audit_tool_name` aux deux sites — celui du succès et celui de l'erreur — et pour
la classe groom ce nom est `groom_parent_orphaned`. C'est précisément ce qui rend
T7 nécessaire : un prédicat qui lirait « la dernière ligne portant le nom du
reaper » réhabiliterait sur un reap **qui n'a pas eu lieu**.

---

## 2. T7 — une ligne de bord n'est pas un reap

### 2.1 Ce que le test doit monter, et pourquoi cette forme-là

La forme réelle est : le reaper a **essayé** de faucher la parente et sa
transition a échoué. Il a donc écrit `groom_parent_orphaned` avec
`before_value = "in_progress"`, `after_value = NULL`, et un `reasoning`
`reaper_db_error: …`. Aucune transition n'existe sur cette parente.

La fixture force par ailleurs `status = 'failed'` et un enfant de groom convergé,
et **ce forçage est le test** : il isole le discriminant. Sans lui, le refus
pourrait venir de (a) ou de (c), et le test serait vert pour une raison qui n'est
pas la sienne. Avec lui, les conditions (a) et (c) tiennent, et **seul** D1
décide. Attendu : refus `last_transition_not_groom_reap`, parente restée
`failed`, refus `task_not_dispatchable` **mot pour mot**.

C'est la même discipline que T1, dont la ligne `manager_flight_verdict` est le
contrôle négatif de D1 sur l'autre bord (une ligne de jugement ne porte qu'un
état *après*) : T7 ferme le bord symétrique (une ligne qui ne porte qu'un état
*avant*).

**Ce test ne traverse aucune branche neuve.** Le `WHERE` de
`latest_state_transition_for_target` exclut déjà la ligne, et le prédicat
redouble avec l'égalité `after_value == Some("failed")`. T7 épingle le
discriminant pour que l'élargir devienne un geste visible — la propriété tient
aujourd'hui par deux filtres dont **aucun** n'est testé sur cette forme.

### 2.2 Le seul changement de fixture, et pourquoi il ne touche aucun appel existant

`parente_fauchee(outil_du_reap, result_du_groom)` appelle **toujours**
`transition(…)` en fin de corps. T7 a besoin d'une parente **sans** transition.

Plutôt que d'ajouter un paramètre aux six appels existants, `parente_fauchee`
devient un wrapper de trois lignes autour de
`parente_fauchee_avec(outil_du_reap: Option<&str>, result_du_groom)`, où `None`
signifie *« le reaper n'a abouti à aucune transition »* — un `if let Some(outil)`
autour de l'appel à `transition`. Les six appels existants sont inchangés, donc
**aucun test de T1 à T6 ne peut changer de verdict par cette refactorisation** :
si l'un d'eux rougit, c'est le wrapper qui est faux, pas le prédicat.

Un helper local `ligne_de_bord(sync, parent_id)` écrit la ligne, avec
`before = Some("in_progress")`, `after = None` et le `reasoning` de `engine.rs`.
Le `None` n'est pas une commodité de test : c'est la colonne telle que la
production l'écrit, et coercer un NULL en `""` est ce que mika#2331 refuse.

---

## 3. T8 — le scan de lecteur unique

### 3.1 Ce qu'il protège, et pourquoi aucun test comportemental ne le voit

Un second site appelant `decide` appliquerait la décision **sans son audit** :
les trois lignes (`parent_rehabilitation_decided`, `…_rehabilitated`,
`…_rehabilitation_refused`) ne sont écrites qu'au site qui les écrit, et un
second lecteur réhabiliterait — ou refuserait — en silence. La sonde AC4
compterait alors un dénominateur qu'elle ne peut pas joindre aux refus nommés.

Aucune assertion comportementale ne peut voir ça : la copie ne rend aucune
décision fausse le jour où elle est écrite. C'est la classe `grooming_marker`
(mika#2158), déjà payée deux fois dans ce crate — une regex copiée dont le
commentaire disait « Mirrors … » et qui a ensuite raté deux élargissements.

### 3.2 Où il vit, et la dépendance que ce choix crée

Dans un `#[cfg(test)] mod tests` **inline** en fin de
`parent_rehabilitation.rs` — le module du prédicat, comme `kg/budget.rs` porte
`mika1833_the_zero_budget_predicate_has_a_single_reader`. La garde vit avec ce
qu'elle garde, et le doc-comment de `decide` affirme déjà *« Un seul appelant »* :
ce vol est ce qui rend cette phrase vérifiable.

**Conséquence à nommer, parce qu'elle est circulaire en apparence** : T9 construit
des fixtures qui *contiennent* l'aiguille. Elles vivent donc dans une région que
`mask_test_regions` masque, sans quoi le scan se compterait lui-même. Le module
devant être **inline** pour porter ce marqueur, un futur déplacement de ces tests
vers `parent_rehabilitation/tests.rs` resterait correct par l'autre exclusion —
`crate::source_scan::is_test_source_path` classe un `tests.rs` isolé comme du
test (mika#2321). Les deux exclusions se composent, et c'est voulu.

### 3.3 L'aiguille a DEUX formes, et la seconde est l'échappatoire de la première

`decide` est `pub(crate)`. Un scan qui ne cherche que le chemin qualifié laisse
passer :

```rust
use crate::parent_rehabilitation::decide;   // invisible à un scan sur `::decide`
…
match decide(db, &task).await { … }         // un appel nu
```

Le prédicat compte donc deux formes :

- **A — l'appel qualifié** : la ligne contient `parent_rehabilitation::decide`,
  et le caractère qui suit `decide` n'est pas un caractère de mot.
- **B — l'importation** : la ligne est un `use` (après `trim_start`), contient
  `parent_rehabilitation`, et porte l'identifiant `decide` à frontières de mot
  des deux côtés.

La forme B rougit **avant** que l'appel nu existe, et elle ne peut pas être
contournée sans repasser par la forme A. Un `use … as autre_nom` reste
visible : l'importation est la ligne comptée, pas l'alias.

**La frontière de mot à droite est load-bearing, et le cas qui le prouve est dans
ce fichier même** : la sonde s'appelle `parent_rehabilitation_decided`, dont
`decided` contient `decide`. Une aiguille non ancrée compte la ligne `info!` du
prédicat comme un appel au prédicat. C'est la classe de la frontière de mot de
mika#1833 (`per_budget` / `total_budget` sortis de la population par la gauche),
et le contrôle négatif de T9 est cette chaîne exacte.

### 3.4 Les trois exclusions, dans cet ordre

Les mêmes que mika#2425, pour les mêmes raisons :

1. **Chemin de test** → aucune moitié de production
   (`crate::source_scan::is_test_source_path`, mika#2321). Un module de test
   extrait ne porte aucun littéral `#[cfg(test)]`, donc une troncature le
   scannerait entier comme de la production.
2. **Régions de test masquées** (`mika_common::source_guard::mask_test_regions`,
   mika#2398), qui conserve les numéros de ligne. Une troncature au premier
   marqueur couperait `executor.rs` à son premier doc-comment le mentionnant.
3. **Une ligne de commentaire n'est pas un appel** (`//`, `///`, `*`). Ce n'est
   pas cosmétique : le site de lecture porte sept lignes de commentaire nommant
   le prédicat, `parent_rehabilitation.rs` en porte une trentaine, et un prédicat
   non ancré les accuse — la forme exacte du faux positif Signal S de mika#2050,
   où un grep a apparié la prose qu'un pilote avait écrite *à propos* du signal.

### 3.5 Les quatre assertions, et ce que chacune seule ne dirait pas

| assertion | ce qu'elle seule laisserait passer |
|---|---|
| chaque entrée de `REHAB_PREDICATE_READERS_ALLOWED` nomme un `mika#` | une exemption que personne ne possède — *« how a guard stops guarding »* |
| **exactement un** site (`assert_eq!(…, 1)`, jamais « au plus un ») | **zéro** lecteur : un prédicat devenu orphelin par renommage ou par câblage retiré, c'est-à-dire un correctif désarmé, que `<= 1` lit comme un arbre propre |
| ce site est sous `skills/executor.rs` | un lecteur unique déplacé hors de la garde d'état, où il observerait le refus au lieu de l'empêcher (doctrine mika#1646) |
| le scanner a vu des fichiers (`scanned > 0`) | un chemin cassé — indiscernable d'un arbre propre, et c'est la vacuité que T9 ferme sur l'autre axe |

L'allowlist est **livrée vide**, et sa vacuité est elle-même l'assertion
auto-nettoyante : il n'y a aucune violation existante à exempter, puisque le
vol 1 a fait naître `decide` avec un appelant. Quand le scan tire, **on route le
site par le prédicat** ; on n'ajoute pas de ligne (doctrine mika#2201).

---

## 4. T9 — le contrôle de bonne foi

T8 pourrait être vert parce qu'il ne regarde rien — exactement le mode de
défaillance que T8 existe pour rendre visible, sur l'autre axe. Le prédicat de
ligne est donc appelé directement sur six fixtures.

| | fixture | attendu |
|---|---|---|
| 1 | un appel qualifié ajouté ailleurs (forme A) | **1 site** |
| 2 | `use crate::parent_rehabilitation::decide;` (forme B — l'échappatoire) | **1 site** |
| 3 | `event = "parent_rehabilitation_decided"` (la sonde du prédicat) | 0 — la frontière de mot à droite |
| 4 | `use crate::parent_rehabilitation::{RehabDecision, RehabPending};` | 0 — un type n'est pas le prédicat |
| 5 | la prose : `// … appelle \`parent_rehabilitation::decide\` …` | 0 — un commentaire ne peut pas appeler |
| 6 | un appel dans une région `#[cfg(test)]` masquée, puis le même dans un fichier de **chemin** de test | 0 et 0 — les deux exclusions, séparément |

Les cas 1 et 2 sont les contrôles **positifs** ; sans eux, les quatre autres
diraient seulement que le prédicat ne trouve rien. Le cas 3 est celui qui serait
réellement tombé.

---

## 5. Fire-Disposition

Ce plan livre **un** détecteur — T8, le scan de lecteur unique — dont le chemin
de succès est *« aucune violation trouvée »*. T7 et T9 n'en sont pas : T7 affirme
le comportement du prédicat sur une forme donnée, T9 affirme que le prédicat de
T8 voit et ne voit pas ce qu'il doit.

**Option retenue : (a) exception nommée en allowlist — et l'allowlist est vide.**

- **Zéro violation existante à l'atterrissage, mesurée et non supposée.** Le grep
  du § 0 sur `b7254234f` rend **un** site de production (`executor.rs:4066`) et un
  site de test (`executor.rs:18500`, masqué par exclusion 2). Il n'y a aucune
  entrée à écrire : une allowlist vide n'est pas une exception, c'est l'absence de
  dette.
- **Aucun mécanisme d'exception n'est livré** : ni liste d'ignorés, ni attribut de
  contournement, ni variable de désarmement. `REHAB_PREDICATE_READERS_ALLOWED`
  naît vide, et l'assertion `entry.contains("mika#")` est ce qui empêche la liste
  de devenir le dépotoir que mika#2309 condamne — un allowlist de sites portant
  le même défaut.
- **L'assertion auto-nettoyante est le compte exact.** `assert_eq!(sites.len(), 1)`
  rougit aussi bien sur un second lecteur que sur **zéro** : un scan qui ne sait
  que compter « pas plus d'un » passe au vert sur un prédicat orphelin, c'est-à-dire
  sur un correctif désarmé. L'assertion de chemin (`skills/executor.rs`) rougit
  sur un lecteur unique déplacé.
- **L'exclusion du test ne passe par aucune liste de fichiers** : `is_test_source_path`
  + `mask_test_regions`, comme les gardes de mika#2335/#2305/#2361/#2425. Élargir
  l'aiguille ou poser une allowlist par fichier est refusé en toutes lettres par
  la doc de `crate::source_scan`.

Aucune option (b) : rien n'atterrit `#[ignore]`, et rien ne peut l'être — un
détecteur désarmé sur une propriété que le vol 1 vient d'établir ne protège rien.
Aucune option (c) : rien ne remonte à l'opérateur, le périmètre est tranché par
D1 et par le § 8 du plan du vol 1.

---

## 6. Ce que ce vol ne fait pas

- **Il ne touche pas le prédicat, ni son site, ni la transition.** Aucune ligne de
  production n'est écrite par ce vol — il n'ajoute que des tests et le wrapper de
  fixture du § 2.2. Si un test rougit, c'est le vol 1 qui a un défaut, et le
  geste est de le corriger, pas d'ajuster l'assertion.
- **Il ne relève pas la sonde AC4.** Cette sonde porte sur les 14 jours **suivant
  le `make deploy`** du vol 1 ; son protocole, ses trois termes et ses deux pièges
  de lecture sont au § 7 du plan du vol 1 et dans le corps de la PR #3045. Elle se
  relève en commentaire sur mika#3044, pas dans ce vol.
- **Il ne rouvre pas les causes de reap à tort** — mika#2980, mika#2986,
  mika#3042.
- **Le refus de ré-armer un lanceur différé dans une parente terminale** (L2a,
  mika#2169) reste hors vol, pour la raison mesurée du corps du ticket : les deux
  cas (`2de05d23` #2886, `6d53b5d4` #1727) n'avaient pas de groom `PLAN_GROOMED`
  et ne rempliraient pas (c).
- **Aucun `MIKA_*`, aucune migration, aucun index, aucun changement de signature
  publique.** Schéma v56 inchangé.

---

## 7. Definition of Done

1. `cargo build`, `cargo clippy --all-targets -p mika-agent` (zéro avertissement
   neuf), `cargo fmt --all --check`, `cargo test --workspace` verts.
2. `scripts/verify-pipeline.sh origin/main` vert, et les lints du dépôt que CI
   lance (`check-secrets.sh` inclus : `executor.rs` est l'un des deux fichiers
   touchés dont la taille compte contre le plafond de 1 Mo).
3. T7, T8, T9 verts chacun seul (`cargo test -p mika-agent mika3044`).
4. **Aucun test de T1 à T6 ne change de verdict.** Le wrapper du § 2.2 ne modifie
   aucun de leurs appels ; s'ils rougissent, c'est le wrapper.
5. T8 rougit sur un second lecteur, **attesté par T9 et non par affirmation** —
   les deux contrôles positifs du § 4 sont la condition pour que T8 dise quelque
   chose.
6. Le texte du refus `task_not_dispatchable` reste **inchangé octet pour octet** :
   T7 le vérifie par `est_le_refus_dhier`, et c'est un format que des scripts
   d'opérateur grepent.
7. Aucun fichier de `src/` autre que `parent_rehabilitation.rs` (son `mod tests`)
   et `executor.rs` (son `mod tests`) n'est modifié.
8. La PR porte `Code review: delegated to mika-qa (mika#2694)`, référence
   mika#3044 **sans** `Closes` — la sonde AC4 reste due sur ce ticket — et dit que
   le vol 2 est le dernier des deux.

---

## 8. Acceptance criteria

Transcrits depuis le corps de senara-solutions/mika#3044. Les quatre sont déjà
servis par le vol 1 ; la colonne de droite dit ce que **ce** vol y ajoute.

- **AC1.** Une parente est réhabilitée (`failed` → `in_progress`) puis dispatchée
  seulement si les quatre conditions tiennent ensemble. Sinon le refus
  `task_not_dispatchable` d'aujourd'hui reste en place, mot pour mot.
  - (a) `status='failed'`, `trigger_type='manual'`, `source='self_dev'`.
  - (b) La dernière transition d'état de la parente dans `audit_events` est un
    reap de la classe groom (`groom_parent_orphaned`, `after_value='failed'`).
    Aucune transition ne la suit. Le plan nomme ce qui compte comme transition :
    une ligne qui porte un état avant et après, et non une ligne de jugement.
  - (c) Le dispatch suit un enfant de groom de cette parente, livré après le
    reap, dont le résultat porte `PLAN_GROOMED`.
  - (d) Toutes les autres gardes de `validate_dispatch_readiness` passent. La
    réhabilitation n'est écrite qu'après elles : si l'une refuse, la parente
    reste `failed`.

  → **Livré au vol 1** (§ 2.1 à 2.5 de son plan). Ce vol ajoute **T7**, qui
  épingle le discriminant de (b) sur le bord « un état avant, pas d'état après ».

- **AC2.** Fail-closed, avec un test par cas dans le même appel. Chaque cas
  laisse la parente `failed` avec le refus actuel :
  - une parente terminée par un autre motif (un refus de dispatch, un
    `phantom_aged_out`, le motif `implement` du reaper) ;
  - une parente `cancelled` ;
  - une parente dont une transition suit le reap ;
  - un groom sans `PLAN_GROOMED` ;
  - une piste d'audit illisible.

  Le contrôle positif est le cas des trois parentes de la Mesure, rejoué en
  fixture. Il réhabilite et dispatche.

  → T1 (contrôle positif), T1b, T2 (les deux moitiés : motif `implement` **et**
  parente `cancelled`), T3, T4, T5 **au vol 1**. **T7 ici** ferme le dernier cas
  de population.

- **AC3.** La décision se prend dans un seul prédicat, lu depuis un seul site.
  Chaque issue écrit une ligne d'audit relisible :
  `task_engine_parent_rehabilitated`, ou un refus avec sa raison nommée parmi
  (a) à (d). Ainsi « aucun cas » et « jamais évalué » ne rendent pas les mêmes
  octets.

  → Le prédicat, son site unique et les trois lignes d'audit sont **au vol 1**.
  **T8–T9 ici** sont ce qui *tient* la propriété « un seul site » contre la dérive
  future : le vol 1 l'établit, ce vol la rend non-régressable.

- **AC4.** Sonde : sur les 14 jours qui suivent le deploy, on compte les parentes
  `ready-label` en `failed` après un `groom_parent_orphaned` dont un groom a
  ensuite rendu `PLAN_GROOMED`, et parmi elles celles qui ont fini en
  `task_not_dispatchable`, puis on l'écrit ici. La cible est 0 refus. Un compte de
  parentes à 0 s'écrit « aucun reap à tort sur la fenêtre », et non
  « réhabilitation vérifiée ».

  → **Hors de ce vol** (§ 6). Protocole au § 7 du plan du vol 1. La fenêtre court
  depuis le `make deploy` du vol 1, et reste due en commentaire sur mika#3044.

---

## 9. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/skills/executor.rs` — T7 : wrapper `parente_fauchee_avec`, helper `ligne_de_bord`, le test et leurs doc-comments | 80 |
| `crates/mika-agent/src/parent_rehabilitation.rs` — T8 : `mod tests`, allowlist, prédicat de ligne à deux formes, le scan et ses quatre assertions | 140 |
| `crates/mika-agent/src/parent_rehabilitation.rs` — T9 : les six fixtures du contrôle de bonne foi | 90 |

Total estimé : 310 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 310 est sous la borne par vol (429)
et très en deçà du seuil par PR (1000), donc aucun découpage n'est requis. Trois
raisons rendent ce total peu susceptible de dériver :

- **Aucune ligne de production n'est écrite.** Trois tests et un wrapper de
  fixture : il n'y a ni branche neuve, ni signature, ni migration, ni point de
  configuration où un coût caché pourrait se loger.
- **Le patron existe mot pour mot, deux fois.** mika#2425
  (`agent_loop/tests/gardes_de_source.rs` l. 1317-1500) donne la forme entière
  — allowlist vide auto-nettoyante, prédicat de ligne, scan par
  `ProductionScanner`, contrôle de bonne foi à cinq cas — et mika#1833
  (`kg/budget.rs`) donne l'anti-vacuité et la frontière de mot. Les fixtures de
  T7 (`parente_fauchee`, `transition`, `dispatch`, `est_le_refus_dhier`) sont
  déjà écrites et importées.
- **L'estimation est volontairement généreuse sur la densité documentaire.** Le
  vol 1 avait estimé 408 lignes pour ~1 123 réelles hors plan — un facteur 2,75,
  dû pour l'essentiel aux doc-comments que ce dépôt exige. Les trois postes
  ci-dessus comptent la doc **avec** le code (≈ 45 % du total), ce qui est le
  ratio observé sur le vol 1 et sur les gardes de mika#2425. Un total « nu » de
  ~170 lignes serait l'erreur du vol 1 répétée.

Pas de `Tracked in:` : ce vol est le dernier des deux, et il ne renvoie rien à un
vol suivant. La sonde AC4 n'est pas un vol — c'est une lecture d'opérateur sur la
base, nommée au § 6.

---

## 10. Ordre de commit

Deux commits, dans cet ordre, chacun vert seul.

1. **`test(3044): une ligne de bord reaper_db_error ne vaut pas un reap de groom`**
   — `executor.rs` : le wrapper `parente_fauchee_avec`, `ligne_de_bord`, T7. Vert
   seul : aucun appel existant n'est modifié, et le test passe par
   `validate_dispatch_readiness` telle qu'elle est aujourd'hui.
2. **`test(3044): le prédicat de réhabilitation n'a qu'un lecteur, et le scan le tient`**
   — `parent_rehabilitation.rs` : le `mod tests`, l'allowlist vide, le prédicat de
   ligne, T8 et T9. Vert seul, et c'est la mesure du § 0 qui le garantit : un
   lecteur de production, un seul.

**Le plancher est le commit 2** : c'est lui qui porte le détecteur d'AC3. Un
mika-qa qui ne lirait qu'un seul commit doit lire le second — et dans ce second,
lire T9 avant T8, parce que T9 est ce qui donne à T8 le droit d'être vert.
