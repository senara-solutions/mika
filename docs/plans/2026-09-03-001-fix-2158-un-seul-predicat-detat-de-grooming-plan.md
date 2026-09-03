# Plan — mika#2158 — Un seul prédicat d'état de grooming, et un refus qui produit un effet

**Issue:** senara-solutions/mika#2158
**Branch:** `fix/2158/auto-pull-deux-pr-dicats-de-groom-se`
**Type:** fix
**Base:** `origin/main` @ `7b4ec10a`
**Statut:** brouillon groomé — passe architecte en cours

---

## 1. Ce que la mesure établit, avant tout récit

Le ticket mesure six corps réels. Cette mesure a été **refaite** sur la branche, et elle tient. Elle
a aussi produit trois faits que le corps du ticket ne contient pas, et dont deux corrigent une
attribution du ticket lui-même.

### 1.1 Les six corps (confirmés, 2026-09-03)

Extraction littérale des trois callouts sur les six tickets `ready` :

| ticket | ligne `Grooming history` (extrait décisif) | `is_groomed` |
|---|---|---|
| #2127 | `… second-pass (ESCALATE, périmètre) → arbitrage routé et rendu → mika-arch (GROOMED) — session …` | **false** |
| #2140 | `… mika-arch second-pass (GROOMED) → intégration des commentaires …` | true |
| #2108 | `… mika-arch first-pass (READY) → aucune révision requise` | **false** |
| #1772 | `… mika-arch seconde passe (GROOMED, session …)` | **false** |
| #2151 | `… mika-arch second-pass (GROOMED)` | true |
| #2117 | `… mika-arch second-pass (GROOMED)` | true |

Les trois causes A / B / C du ticket sont exactes et distinctes.

### 1.2 Correction de prémisse n°1 — ce n'est pas `is_groomed()` qui décide `groom` vs `implement`

Le corps du ticket attribue le `dispatch_class = groom` observé sur #2108/#1772/#2127 à
`is_groomed()` (`crates/mika-agent/src/auto_pull.rs:301`). **Ce n'est pas ce prédicat-là.**

Le verdict `groom`/`implement` du webhook `ready`-label est rendu à
`crates/mika-agent/src/server/ready_label_handler.rs:265-274` :

```rust
let missing_markers = crate::skills::executor::check_grooming_markers(&body);
let is_groomed = missing_markers.is_empty();
let (target_tool, target_skill, dispatch_class) = if is_groomed {
    ("run_claude_pilot", "dev-pilot", "implement")
} else {
    ("run_claude_pilot_groom", "dev-groom", "groom")
};
```

`check_grooming_markers` vit dans `crates/mika-agent/src/skills/executor.rs:972`, et il est le
**troisième** porteur du prédicat, pas le premier. Il n'y a donc pas deux prédicats de « groomé »
mais **trois** :

| | où | ce qu'il lit | verdict sur #2108/#1772/#2127 |
|---|---|---|---|
| P1 `is_groomed` | `auto_pull.rs:301-314` (Rust) | la prose du callout | non groomé |
| P2 `check_grooming_markers` | `skills/executor.rs:972-987` (Rust) | la prose du callout | non groomé |
| P3 garde de dispatch | `_shared/dispatch-lib.sh:1885-1893` (Bash) | le **fichier de plan** résolu sur la branche | déjà groomé |

P1 gouverne la **promotion** (le feeder pose le label `ready` :
`auto_pull.rs:818`, `:1087`, `:1938`). P2 gouverne le **routage du dispatch**. P3 gouverne le
**refus**. Le ticket voyait P1 et P3 ; le dispatch observé venait de P2.

Cela ne change rien au diagnostic — P1 et P2 portent des regex jumelles et échouent sur les mêmes
trois corps — mais cela change le périmètre du correctif : **corriger P1 seul ne débloquerait
aucun des trois tickets.** C'est P2 qui les envoie en `groom`.

### 1.3 Correction de prémisse n°2 — P2 est déjà *plus large* que P1, et le sait

`executor.rs` porte **trois** regex là où `auto_pull.rs` n'en porte qu'une :

```rust
GROOMED_VERDICT_RE    = r"second-pass \(GROOMED[\s\)\.,;:—-]"          // executor.rs:945 ET auto_pull.rs:308
PARAPHRASED_GROOMED_RE = r"second-pass \(READY, paraphrased GROOMED"    // executor.rs:949  — absent d'auto_pull
SINGLE_PASS_GROOMED_RE = r"first-pass \(READY, single-pass GROOMED"     // executor.rs:967  — absent d'auto_pull
```

`SINGLE_PASS_GROOMED_RE` a été ajouté par **mika#2012** précisément pour la cause A (« un ticket
groomé en une passe restait invisible… 25 requeues mesurés sur 5 tickets en 13 h »). Le
commentaire de `auto_pull.rs:304` dit *« Mirrors GROOMED_VERDICT_RE in skills/executor.rs
(#1725) »* — il a copié la regex de 2025 et n'a jamais suivi les deux ajouts d'après.

**La divergence P1↔P2 est donc déjà un fait acquis du dépôt, et elle est du type exact que ce
ticket décrit.** C'est l'argument central pour l'AC6 : la source de vérité doit être *une seule*,
pas deux copies dont l'une commente qu'elle imite l'autre.

Conséquence pour la cause A : mika#2012 a résolu la cause A **en changeant la forme écrite**
(`first-pass (READY, single-pass GROOMED`) au lieu d'élargir la lecture. #2108 porte
`first-pass (READY) → aucune révision requise` — la forme humaine, pas la forme #2012. Le remède
de #2012 n'a donc jamais couvert que les corps que le pipeline écrit lui-même ; il ne couvre pas
un corps écrit à la main ni un pipeline antérieur au 2026-08-27.

### 1.4 Correction de prémisse n°3 — le livelock a un moteur mesuré, et il efface son propre garde-fou

Le corps du ticket décrit la boucle sans en nommer le moteur. Mesure faite sur
`~/.mika/data/mika.db` le 2026-09-03 :

```
tasks (source=self_dev) — statuts par ticket
1772 | groom | failed      | 31
1772 | groom | in_progress |  1
2108 | groom | failed      |  7
2108 | groom | completed   |  1
2108 | groom | in_progress |  1
2127 | groom | failed      |  8
2127 | groom | in_progress |  1
```

`result` des tâches `failed` de #2108 : **`phantom_aged_out`** (les quatre dernières inspectées,
identiques). Chronologie d'une tâche : créée à `HH:x0:07`, `updated_at` une seconde plus tard,
puis plus rien pendant ~60 min, puis `failed`.

Et l'état du garde-fou censé borner tout ça :

```
auto_pull_stats — repo=senara-solutions/mika
1772 | redrive_count=1 | last_redrive_at=2026-09-03T21:20:06Z
2108 | redrive_count=0 | last_redrive_at=2026-09-03T20:40:07Z
2127 | redrive_count=0 | last_redrive_at=2026-09-03T21:10:06Z
```

`MAX_REDRIVES_DEFAULT = 3` (`auto_pull.rs:77`). **Après 31 re-drives sur #1772, le compteur dit 1.**
Le budget n'a jamais pu se déclencher parce qu'il est remis à zéro à chaque tour, par
`classify_stuck_ready` (`auto_pull.rs:945-950`) :

```rust
if facts.in_flight {
    return StuckReadyVerdict::SkipAndResetBudget { reason: "in_flight_self_dev" };
}
```

Le cycle complet, chaque maillon mesuré :

1. Phase 2 juge le ticket « stuck ready », fait `remove ready` → `add ready`, incrémente le budget.
2. Le webhook `ready`-label lit le corps avec **P2**, ne trouve pas la forme, choisit `groom`,
   et **pré-crée une tâche `in_progress`** (`ready_label_handler.rs` étape 7).
3. Pendant ~60 min cette tâche rend `facts.in_flight = true` → chaque tick de Phase 2 rend
   `SkipAndResetBudget` → **`redrive_count` retombe à 0**.
4. La tâche n'est jamais résolue par personne et meurt en `phantom_aged_out`.
5. `in_flight` redevient faux, le ticket est de nouveau « stuck ready ». Retour en 1.

**Le budget de re-drive ne borne pas la boucle : la boucle l'efface.** Un dispatch qui n'a rien
produit est compté comme un progrès, parce que le seul signal lu est « une tâche existe », pas
« une tâche a abouti ». C'est cela, précisément, que l'AC8 demande de corriger — et le remède
n'est pas seulement « faire produire un effet au refus », c'est **cesser de traiter un dispatch
mort comme une preuve d'avancement**.

**Incertitude nommée, et non refermée.** La mesure n'établit pas *quelle moitié* laisse la tâche
fantôme : soit mika-dev (LLM) n'appelle jamais l'outil et `dispatch-lib` ne tourne pas pour cette
tâche-là, soit `dispatch-lib` refuse (`auto_skipped`/`already_groomed`, exit 0) et le refus ne
retombe jamais sur la ligne `tasks`. Les deux sont compatibles avec `phantom_aged_out` et avec les
refus que le ticket cite dans les logs. Le plan **traite les deux** (§4 M6) plutôt que de parier
sur l'une, et §4 M6a pose l'instrumentation qui tranchera.

---

## 2. Ce que le correctif doit être — position

**Un seul prédicat d'état de grooming, exporté depuis un module unique, appelé par P1 et par P2 ;
et un budget de re-drive qui ne se remet à zéro que sur un progrès réel.**

Ce que le correctif ne doit **pas** être :

- Pas trois regex de plus. Ajouter une quatrième variante à chaque forme rencontrée reproduit la
  classe : le prédicat continuerait de mesurer une formulation.
- Pas un « accepte tout ». L'AC4 est une contrainte de conception, pas une case à cocher : le
  prédicat doit continuer de refuser un corps sans verdict, un corps dont le dernier verdict est
  `ESCALATE` sans `GROOMED` postérieur, et un corps sans callout `Branch`/`Plan`.
- Pas une fusion des conditions `Branch`/`Plan`. Voir §3.

---

## 3. Frontière avec mika#2120 — décision, et pourquoi

mika#2120 (OPEN, label `operator-gated`) porte sur l'**autre** condition de la même fonction : P1
exige `> - **Plan:** \`docs/plans/` tandis que P2 se contente de `docs/plans/`. Les deux tickets
touchent `auto_pull.rs::is_groomed` et `executor.rs::check_grooming_markers`.

**Décision : l'extraction du module partagé porte uniquement sur le marqueur de verdict
(`Grooming history`). Les conditions `Branch` et `Plan` restent inchangées, à leur place, et
divergentes.**

Pourquoi ne pas tout unifier d'un coup, alors que l'unification est justement le remède :

- Unifier la condition `Plan` **est** le correctif de #2120, qui est sous arbitrage opérateur.
  L'emporter dans ce ticket court-circuiterait cet arbitrage.
- Le hors-périmètre du ticket le dit explicitement : *« Les deux correctifs touchent la même
  fonction et devraient être séquencés, pas fusionnés. »*
- La séquence est sans dette : le module créé ici (§4 M1) est l'emplacement où #2120 déposera la
  condition `Plan` unifiée quand son arbitrage sera rendu. Ce plan crée le tiroir ; #2120 y range
  sa moitié.

**Conflit de merge attendu :** #2120 et #2158 modifieront tous deux le corps de `is_groomed` et de
`check_grooming_markers`. Le conflit sera textuel et local (quelques lignes), pas sémantique. Le
ticket qui merge en second rebase. Aucune coordination de branche n'est requise ; en particulier,
ce plan **ne branche pas** depuis #2120 (aucune PR ouverte, et son arbitrage peut changer sa
forme).

---

## 4. Périmètre — les modules de travail

### M1 — Un module unique pour la reconnaissance du verdict de grooming (AC6)

Créer `crates/mika-agent/src/grooming_marker.rs` (module public du crate), portant :

```rust
/// Le verdict de grooming lu dans le callout `Grooming history` d'un corps d'issue.
pub enum GroomingVerdict { Groomed, Escalated, Absent }

/// La seule lecture du marqueur de verdict dans le dépôt (mika#2158).
pub fn grooming_verdict(issue_body: &str) -> GroomingVerdict;

/// Sucre : `matches!(grooming_verdict(b), GroomingVerdict::Groomed)`.
pub fn has_groomed_verdict(issue_body: &str) -> bool;
```

- `auto_pull::is_groomed` appelle `has_groomed_verdict` et **conserve** ses deux conditions
  `Branch`/`Plan` telles quelles (§3).
- `executor::check_grooming_markers` appelle `has_groomed_verdict` et **conserve** ses deux
  conditions `Branch`/`Plan` telles quelles (§3). Les trois `static … RE` locaux
  (`GROOMED_VERDICT_RE`, `PARAPHRASED_GROOMED_RE`, `SINGLE_PASS_GROOMED_RE`) sont **supprimés** et
  leur logique absorbée par le module.
- Le module porte en tête de fichier la **décision de l'AC6** rédigée (§6), pour que la prochaine
  divergence soit une régression et non une découverte.

**Garde structurelle contre la récidive :** un test du module échoue si une regex contenant
`second-pass` ou `first-pass` apparaît ailleurs que dans `grooming_marker.rs`. Implémentation :
un test qui lit les sources du crate (`include_str!` sur `auto_pull.rs` et
`skills/executor.rs`, ou un `grep` en `build.rs`/test d'intégration) et refuse la présence du
motif. C'est ce qui empêche la copie-avec-commentaire-« Mirrors » de revenir. *La forme exacte de
cette garde est à valider par l'architecte : `include_str!` sur un fichier voisin est un couplage
de compilation, un test d'intégration qui lit `src/` est plus lâche mais dépend du cwd.*

### M2 — Élargir la reconnaissance : état, pas formulation (AC1, AC2, AC3)

`grooming_verdict` reconnaît un grooming abouti sur ces axes, tous ancrés sur le préfixe de ligne
`^> - **Grooming history:**` (multi-ligne), ce qui préserve la distinction callout/prose qui est
la raison d'être de l'ancrage actuel :

| axe | forme reconnue | AC |
|---|---|---|
| passe | `second-pass` \| `seconde passe` \| `first-pass` \| `première passe` \| `premiere passe` | AC1, AC2 |
| verdict | `GROOMED` **quel que soit** le producteur qui le précède (`second-pass (GROOMED…`, `mika-arch (GROOMED…`, `first-pass (READY)` suivi d'aucune passe) | AC1, AC3 |
| ordre | le **dernier** verdict de la ligne fait foi : `… (ESCALATE …) → … (GROOMED …)` rend `Groomed` ; `… (GROOMED) → … (ESCALATE)` rend `Escalated` | AC3, AC4 |

**Le discriminateur structurel change de nature.** Aujourd'hui il est *lexical* (le mot
`second-pass` précède `GROOMED`). Il devient *positionnel* : la ligne de callout `Grooming
history` est le contexte, et le **dernier** token de verdict qu'elle contient est l'état. Deux
tokens seulement sont des verdicts : `GROOMED` et `ESCALATE`. `READY` et `ITERATE` sont des
dispositions de passe, pas des verdicts finaux — sauf le cas AC1, où `first-pass (READY)` **sans
aucun verdict postérieur** est un grooming abouti, parce que `/mika-groom-ticket` phase 3 étape 10
prescrit littéralement de sauter à la phase 5 sans seconde passe.

Cas AC1 explicité, parce qu'il est le seul où une disposition vaut verdict :

```
ligne contient GROOMED (dernier)                       → Groomed
ligne contient ESCALATE (dernier), pas de GROOMED après → Escalated
ligne contient first-pass/première passe (READY) et
  aucun GROOMED ni ESCALATE                             → Groomed        (AC1)
ligne contient ITERATE seul, sans passe ultérieure      → Absent         (AC4)
pas de ligne de callout                                 → Absent         (AC4)
```

`ITERATE` seul reste **non groomé** : une première passe qui itère prescrit une seconde passe qui
n'a pas eu lieu. C'est la non-régression la plus fine de l'AC4 et elle doit avoir son test.

### M3 — Non-régression (AC4)

Tests explicites, chacun nommé d'après ce qu'il protège :

- corps vide → non groomé ;
- corps sans ligne `Grooming history` → non groomé ;
- `… second-pass (ESCALATE, périmètre)` sans `GROOMED` postérieur → non groomé ;
- `… second-pass (GROOMED) → … → mika-arch (ESCALATE)` → non groomé (l'ordre compte dans les
  deux sens) ;
- `first-pass (ITERATE)` seul → non groomé ;
- prose hors callout (`« le ticket a été GROOMED hier »` en corps de texte) → non groomé ;
- corps avec verdict mais sans callout `Branch` → non groomé (P1) ;
- corps avec verdict mais sans callout `Plan` → non groomé (P1).

Les deux derniers vérifient que M1 n'a pas déplacé les conditions `Branch`/`Plan` en les extrayant.

### M4 — Les six corps réels comme fixtures (AC5)

Figer les six corps **littéraux** dans `crates/mika-agent/tests/fixtures/grooming_bodies/` (un
fichier par ticket : `2127.md`, `2140.md`, `2108.md`, `1772.md`, `2151.md`, `2117.md`), capturés
depuis GitHub le 2026-09-03, avec un `README.md` disant d'où ils viennent, à quelle date, et que
**ce sont des corps historiques figés — ne pas les rafraîchir** (un rafraîchissement effacerait
précisément les formes que le correctif doit reconnaître).

Un test table-driven porte le tableau attendu :

| fixture | attendu avant | attendu après |
|---|---|---|
| 2127 | false | **true** |
| 2140 | true | true |
| 2108 | false | **true** |
| 1772 | false | **true** |
| 2151 | true | true |
| 2117 | true | true |

Soit 6 vrais après correctif contre 3 aujourd'hui. Le test assert sur l'état **après**.

### M5 — Test croisé : les prédicats s'accordent (AC7)

Deux niveaux, parce que les trois porteurs ne sont pas testables au même endroit.

**M5a — croisement Rust↔Rust (exécutable, bloquant).** Sur les six fixtures, un test vérifie que
`auto_pull::is_groomed` et `executor::check_grooming_markers(..).is_empty()` rendent le **même**
verdict. Un désaccord fait échouer la suite.

> **Réserve honnête :** tant que #2120 n'est pas rendu, P1 et P2 divergent encore sur la condition
> `Plan` (préfixe de dépôt). Sur les six fixtures, aucune ne porte de callout `Plan` préfixé —
> mesuré : les six écrivent la forme nue `docs/plans/` — donc le croisement **passe** sur ce jeu.
> Le test croisé est donc vert sans que la divergence #2120 soit fermée. Le test porte un
> commentaire qui le dit, et nomme #2120 comme la moitié restante. Prétendre l'inverse serait
> exactement l'attestation-produite-à-côté-de-ce-qu'elle-atteste que #2034 a déjà corrigée ailleurs.

**M5b — croisement Rust↔Bash (contractuel).** P3 mesure un artefact git (`_committed_plan_on_branch`),
que le Rust ne peut pas évaluer sans I/O. Le croisement testable est unidirectionnel et c'est
celui qui compte : **si P3 refuse pour `already_groomed`, alors P2 doit dire groomé.** Un test dans
`skills/bundled/_shared/test-dispatch-lib.sh` monte un dépôt jetable avec un plan committé sur la
branche et un corps d'issue tiré des fixtures M4, et vérifie que la garde refuse — la moitié Rust
étant couverte par M4. L'implication inverse (P2 groomé ⇒ P3 refuse) n'est **pas** vraie et ne
doit pas être testée : un ticket peut être groomé et son plan absent de la branche (jamais poussé),
et c'est le cas que la branche `elif` de `dispatch-lib.sh:1894` traite déjà.

### M6 — Le refus produit un effet (AC8)

Trois volets. Le premier est de la mesure, les deux autres sont le correctif.

**M6a — instrumenter le fantôme (préalable, non optionnel).** Émettre un événement d'audit à la
création de la tâche pré-créée par `ready_label_handler` et à sa résolution, avec le `task_id`, de
sorte qu'une tâche `phantom_aged_out` puisse être imputée à l'une des deux moitiés (LLM qui
n'appelle pas l'outil / `dispatch-lib` qui refuse sans retomber sur la ligne `tasks`). Sans cela,
M6b et M6c sont posés à l'aveugle. L'événement suffit ; aucun changement de comportement.

**M6b — un dispatch mort ne compte pas comme un progrès.** Dans `classify_stuck_ready`
(`auto_pull.rs:945-950`), `facts.in_flight` cesse de rendre `SkipAndResetBudget` et rend
`Skip { reason: "in_flight_self_dev" }` : la boucle attend toujours que la tâche en vol se termine,
mais **n'efface plus le budget**. Le budget ne se remet à zéro que sur `has_open_pr` — un progrès
observable et externe, qui ne peut pas être fabriqué par le dispatch lui-même.

Conséquence directe, et c'est le critère de non-régression de l'AC8 : au bout de
`MAX_REDRIVES` (3) re-drives sans PR, le ticket est abandonné vers l'opérateur au lieu de tourner
indéfiniment. #1772 se serait arrêté au troisième tour, pas au trente-et-unième.

*Point pour l'architecte :* c'est le seul changement du plan qui touche une décision de conception
existante plutôt qu'un défaut. Le commentaire de `classify_stuck_ready` justifie le reset par
« an open PR or a live dispatch means the re-drives worked » — la mesure §1.4 établit que la
seconde moitié de cette phrase est fausse : un dispatch vivant ne prouve rien, il ne fait que
commencer. Retirer `in_flight` du reset est donc une correction de la prémisse, pas un durcissement
arbitraire. Si l'architecte juge que la borne à 3 est trop serrée pour du travail long, le levier
est `MAX_REDRIVES` (variable d'environnement, déjà existante), pas le retour du reset.

**M6c — le refus clôt sa propre tâche.** Quand `dispatch-lib.sh` refuse avec
`reason: already_groomed`, le refus doit résoudre la ligne `tasks` pré-créée plutôt que la laisser
vieillir 60 min en fantôme. Le `_deliver_callback` existe déjà et porte le JSON ; il manque la
retombée côté moteur. Deux formes possibles, **à trancher par l'architecte sur la foi de M6a** :

1. Le refus marque la tâche `completed` avec `result = already_groomed` — le fantôme disparaît,
   `in_flight` redevient faux immédiatement, et avec M6b le budget se consomme normalement jusqu'à
   l'abandon vers l'opérateur.
2. Le refus **re-route** : la tâche est basculée en `dispatch_class = implement` et re-dispatchée
   (`update_task_dispatch_class` existe déjà, `async_db.rs:1295`).

**Ma position : la forme 1.** Elle est strictement plus petite, elle suffit à casser le livelock une
fois M2 en place (les trois tickets seront reconnus groomés et partiront en `implement` par le
chemin normal), et la forme 2 introduit un chemin de dispatch qui contourne le webhook — donc une
quatrième autorité sur « ce ticket doit-il être implémenté », ce qui est exactement la classe de
défaut que ce ticket ferme. La forme 2 ne devient nécessaire que si M6a montre que le refus
n'atteint jamais le moteur ; dans ce cas c'est le canal de retour qu'il faut réparer, pas un
raccourci qu'il faut ajouter.

### M7 — La décision, écrite à côté du code (AC6)

Deux emplacements, deux publics :

1. **En tête de `grooming_marker.rs`** : la source de vérité est nommée (« ce module est la seule
   lecture du marqueur de verdict ; `auto_pull` et `executor` l'appellent et n'en portent pas de
   copie »), avec la garde structurelle de M1 citée, et le renvoi vers #2120 pour la moitié
   `Plan` non encore unifiée.
2. **`docs/solutions/`** : une entrée compound sur la classe « un prédicat qui mesure une
   formulation au lieu d'un état », avec les trois causes A/B/C, la mesure §1.4 du budget effacé
   par le dispatch qu'il devait borner, et la règle générale — *un compteur remis à zéro par
   l'action qu'il compte ne borne rien*.

**Sens de l'alignement, tranché :** c'est le **prédicat** qui s'aligne sur la spec, pas l'inverse.
`/mika-groom-ticket` prescrit un chemin (`READY` en première passe → phase 5) que le prédicat
punit ; la spec décrit un travail réel, le prédicat décrit une phrase. Le prédicat cède. Corollaire
assumé : la spec n'a pas à imposer l'anglais pour être lisible par la machine, dans un dépôt qui
écrit ses tickets et ses plans en français.

---

## 5. Hors périmètre (repris du ticket, et tenu)

- **Le motif `Plan:` et le préfixe de dépôt** → mika#2120. Voir §3 pour la frontière exacte et le
  point de rendez-vous des deux correctifs.
- **La borne de sessions de grooming simultanées** → conséquence observée, cause distincte.
- **Le verrou « un seul dispatch implement à la fois »** → choix de conception.
- **La cause des sessions de pilote qui ne produisent rien** → mika#2141 (le bac à sable ne monte
  pas le gitdir) et mika#2147. Le présent ticket fait *partir* les tickets en `implement` ; ce
  qu'ils deviennent ensuite est un autre défaut, déjà fiché.

---

## 6. Critères d'acceptation — traçabilité

| AC | module | vérification |
|---|---|---|
| AC1 — première passe `READY` reconnue | M2 | fixture `2108.md` → `true` |
| AC2 — `seconde passe` reconnue | M2 | fixture `1772.md` → `true` |
| AC3 — `GROOMED` après `ESCALATE` reconnu | M2 | fixture `2127.md` → `true` |
| AC4 — non-régressions tiennent | M3 | 8 tests nommés, dont `ITERATE` seul et l'ordre inversé |
| AC5 — six corps figés + tableau | M4 | test table-driven, 6/6 `true` |
| AC6 — source de vérité nommée | M1, M7 | module unique + garde anti-copie + entrée `docs/solutions/` |
| AC7 — les prédicats s'accordent | M5a, M5b | croisement Rust↔Rust bloquant + test Bash contractuel |
| AC8 — le refus produit un effet | M6a, M6b, M6c | deux tours consécutifs du feeder ne produisent pas deux refus `already_groomed` sur le même ticket |

**Vérification de l'AC8 sur du réel, pas sur un test seul.** Après déploiement, mesurer sur
`auto_pull_stats` que `redrive_count` de #1772 **progresse** (au lieu de rester à 1) et que la
tâche pré-créée du prochain dispatch de #2108 ne meurt pas en `phantom_aged_out`. Une période du
composant le plus lent, c'est ~70 min ; ne pas conclure avant deux cycles.

---

## 7. Ordre d'exécution

1. **M4** (fixtures) — d'abord, pour que tout le reste soit mesuré contre du réel figé.
2. **M1** (module + garde anti-copie), à comportement inchangé : les tests existants de
   `auto_pull` et `executor` doivent passer sans modification. C'est le contrôle que l'extraction
   n'a rien déplacé.
3. **M2 + M3** (élargissement + non-régressions). Le tableau M4 bascule ici de 3/6 à 6/6.
4. **M5a**, puis **M5b**.
5. **M6a** (instrumentation), **M6b** (budget), **M6c** (clôture du refus).
6. **M7** (décision écrite).

Les étapes 1-4 ferment AC1-AC7 et suffisent à débloquer les trois tickets. L'étape 5 ferme AC8 et
empêche la classe de revenir sous une forme non encore rencontrée. Si l'étape 5 devait être
séparée, elle le serait en ticket propre — mais elle ne doit pas être *abandonnée* : sans elle,
la prochaine forme d'historique non reconnue rouvre exactement le même livelock, budget effacé
compris.

---

## 8. Ce qui reste incertain

- **Quelle moitié laisse la tâche fantôme** (§1.4). M6a le tranche avant M6c ; le plan ne parie pas.
- **La forme exacte de la garde anti-copie** de M1 (`include_str!` vs test d'intégration lisant
  `src/`) — arbitrage architecte.
- **La borne `MAX_REDRIVES = 3`** devient effective avec M6b alors qu'elle était jusqu'ici morte.
  Trois re-drives sans PR pourraient être trop serré pour un ticket long. Le levier existe
  (variable d'environnement) et le plan ne change pas la valeur par défaut — mais il faut savoir
  qu'on rend vivant un garde-fou qui ne l'était pas, et surveiller le premier abandon qu'il
  produira.
