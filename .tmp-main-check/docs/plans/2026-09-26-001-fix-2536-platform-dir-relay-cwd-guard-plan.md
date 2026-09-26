# mika#2536 — Le chemin de plateforme traverse enfin, et un `cwd` incomposable est refusé en le nommant

**Ticket :** senara-solutions/mika#2536
**Parent :** senara-solutions/mika#2532 (re-scopé à l'observabilité R1–R3 ; ce ticket porte R4–R7)
**Type :** fix (p1 — la boucle de QA rendait `COMMENTED` faute d'un build vérifié)
**Date :** 2026-09-26

---

## 1. Mesure fondatrice (héritée, non rejouable)

Le handler `skills/bundled/build-mika/handlers/run.sh` a crashé **4 fois** sur la QA de
PR #2530, chaque tentative, avec le même `tasks.result` :

> `HANDLER CRASH (exit code 1). Script failed before building result.`

Instances (2026-09-25) : 12:52:02 (`5624ea27`), 13:05:26 (`4a877600`), 13:48:20
(`5078ef8e`), 14:02:57 (`03a16846`). Le plan groomé `ebd0e46f` a localisé le crash à la
ligne près — `cd "$CWD"` (l.65), **seul** `exit 1` littéral entre l'installation du trap
et la première assignation de `RESULT`.

**Cette mesure n'est plus rejouable** : PR #2530 est mergée, son worktree fauché, et la
valeur exacte de `CWD` vivait dans `tool_calls`, table que le bac à sable de dispatch ne
monte pas. Ce plan ne prétend donc pas trancher *quelle* chaîne le modèle a passée — il
referme les **deux causes structurelles établies par lecture** et rend la prochaine
occurrence auto-diagnostique.

Le parent #2532 porte l'observabilité (R1–R3 : le stderr n'est plus jeté, le message
nomme son étape, le trap couvre plus tôt). **Ce ticket porte R4–R7** et est indépendant :
aucune de ses lignes ne touche `deliver_callback` ni `spawn_long_running_exec`'s error
arm — voir §2 F6, qui est ce qui rend les deux mergeables dans n'importe quel ordre.

---

## 2. Ce que la lecture du code déplace — et c'est le premier livrable

Le ticket pose trois faits dont **deux sont faux à HEAD `d4514180`**, et la mesure en
découvre trois autres qui contraignent le design. Ces six constats sont le socle du reste.

### F1 — `PLATFORM_DIR_RELAY_KEY` n'existe PAS dans l'arbre

Le ticket écrit qu'il « a déjà été introduit dans `crates/mika-agent/src/skills/executor.rs`
par la 1re impl sur-scopée ». Cette impl vivait sur **PR #2535, fermée** (voie b). Sur
`main` à `d4514180`, `grep PLATFORM_DIR_RELAY_KEY crates/` ne rend **rien**.

Conséquence directe sur la note mika#2201 du ticket : la constante n'est pas « à déclarer
dans `canonical-tokens.tsv` » comme un rattrapage de CI, elle est **à créer puis à
déclarer**, dans le même commit. L'ordre importe — déclarer un jeton qui n'existe pas
ferait rougir l'assertion auto-nettoyante du scan d'exhaustivité (mika#2201 §D6 :
« une exception dont le fichier ne contient plus le jeton fait rougir »).

### F2 — La population de R4 est **10 lignes sur 5 fichiers**, pas six sur quatre

Le plan de référence écrit « six sites » puis en **liste huit**, et il omet
`dispatch-lib.sh`. La mesure exhaustive
(`grep -rn 'MIKA_[A-Z_]*:-' skills/bundled/*/handlers/*.sh` + `_shared/`) :

| fichier | lignes | n |
|---|---|---|
| `build-mika/handlers/run.sh` | 60 | 1 |
| `deploy-mika/handlers/run.sh` | 36, 68, 69 | 3 |
| `resolve-pr-conflicts/handlers/run.sh` | 112, 113 | 2 |
| `address-pr-comments/handlers/run.sh` | 77, 78 | 2 |
| **`_shared/dispatch-lib.sh`** | **8330, 8331** | **2** |
| | | **10** |

Les deux dernières ne sont pas un supplément facultatif : elles sont ce qui **force** la
séquence de F3.

### F3 — `DISPATCH_ENV_KNOWN_INERT` porte déjà l'entrée, et son assertion auto-nettoyante contraint l'ordre du correctif

`executor.rs` porte, depuis mika#2508, une liste nommée des variables que `dispatch-lib.sh`
lit et qui **n'atteignent pas** le child de dispatch. `MIKA_PLATFORM_DIR` y figure :

```rust
("MIKA_PLATFORM_DIR", "mika#2491 — racine plateforme"),
```

Et son doc-comment prescrit littéralement la conduite : *« Quand une entrée est tranchée,
on la RELAIE et on retire sa ligne — on n'élargit pas la liste. »* **R4 est l'exécution de
cette instruction**, pas une initiative. L'inertie était déjà mesurée, nommée et datée ;
ce qui manquait est le relais.

Le test `mika2508_every_operator_var_read_by_dispatch_lib_reaches_the_child_or_is_named`
porte une assertion **double-sens** qui rend l'enchaînement obligatoire plutôt
qu'optionnel :

1. `population.contains(name)` — l'entrée rougit si la variable n'est **plus lue** par
   `dispatch-lib.sh` ;
2. `!reaches_dispatch_child(name)` — elle rougit si la variable **atteint désormais** le
   child.

Trois conséquences qui ne sont pas des choix :

- corriger `dispatch-lib.sh:8330-8331` **impose** de retirer l'entrée (assertion 1) ;
- le nom relayé, s'il est lu par `dispatch-lib.sh` sous la forme self-référentielle
  `PLATFORM_DIR="${PLATFORM_DIR:-…}"`, **entre** dans la population (terme 4bis du
  prédicat, explicitement conçu pour cet idiome) — donc `reaches_dispatch_child`
  **doit** apprendre ce nom, sinon le test rougit sur un orphelin ;
- la population doit rester `>= 8` (seuil d'anti-vacuité). Relevé indicatif à
  `d4514180` : au moins 13 membres non-builtin (les 5 de `DISPATCH_ENV_KNOWN_INERT`,
  `PILOT_MAX_TURNS`, `PILOT_LOG_DIR`, `GH_TOKEN`, `MIKA_DISPATCH_WORKTREE_FILE`,
  `MIKA_SPIRIT_LOG_FILE`, les deux `MIKA_RESCUE_VERIFY_*`, les deux
  `MIKA_ARCH_ASK_RETRY*`). Retirer un membre laisse ≈12 — **marge confortable, à
  confirmer par V6** plutôt qu'à affirmer.

### F4 — Le relais doit **traduire** le nom, sinon il casse le contrat opérateur en silence

Le réflexe est de faire lire `PLATFORM_DIR` aux dix sites et de relayer
`std::env::var("PLATFORM_DIR")`, comme `PILOT_DISPATCH_ENV` le fait pour ses deux knobs.
**Ce serait une régression silencieuse** : l'opérateur pose `MIKA_PLATFORM_DIR` sur
l'environnement du service — c'est le nom que mika#2491 documente et celui que
`DISPATCH_ENV_KNOWN_INERT` nomme. Un relais à l'identique exigerait qu'il renomme sa
variable sans que rien ne le lui dise, et un réglage qui cesse d'être lu sans erreur est
très exactement la panne que ce ticket ferme.

Le relais doit donc **lire `MIKA_PLATFORM_DIR` côté spirit** — où il n'est pas scrubbé,
le scrub étant une propriété de l'env du *child* — et **poser `PLATFORM_DIR` côté child**.
C'est le motif de `inject_pilot_transcript_env` (lit `MIKA_LOG_PILOT_TRANSCRIPTS`, pose
`ANTHROPIC_LOG_FILE`), et le précédent existe déjà **pour cette variable exacte** :
`shell-exec/handlers/run.sh` la lit avant son propre scrub pour la passer en argument à la
garde mika#2449.

Corollaire : `PLATFORM_DIR` ne peut **pas** rejoindre `PILOT_DISPATCH_ENV`, dont
l'injecteur relaie un nom à l'identique (`std::env::var(k)` avec `k` = le nom child). Il
faut le sixième injecteur, `inject_platform_dir_env`, et son nom de clé est la constante
`PLATFORM_DIR_RELAY_KEY` que le ticket exige.

### F5 — `inject_pilot_dispatch_env` est **inconditionnel**, donc le relais atteint les quatre handlers

Vérifié à `executor.rs:4097` : les cinq injecteurs sont appelés dans
`spawn_long_running_exec` **sans condition sur le skill**. Un sixième injecteur atteindra
donc `build-mika`, `deploy-mika`, `resolve-pr-conflicts`, `address-pr-comments` et les
dispatches pilote par le même chemin. Aucun câblage par skill n'est nécessaire — et c'est
ce qui rend le correctif d'une ligne par site suffisant.

### F6 — R6 s'implémente **sans toucher `deliver_callback`**, et R6 a une population de deux, pas quatre

Deux mesures, et elles réduisent le périmètre plutôt que de l'étendre.

**(a) La mécanique du refus existe déjà et est prouvée en service.**
`deliver_callback` (`build-mika`, l.38-55) compose « HANDLER CRASH » **uniquement quand
`RESULT` est vide**. Poser `RESULT="REFUS …"` puis `exit 1` suffit donc à livrer un refus
nommé, sans modifier la fonction. Et ce n'est pas une inférence : `deploy-mika:71-79` fait
**déjà exactement cela** —

```sh
CWD=$(cd "$CWD" 2>/dev/null && pwd -P) || {
    RESULT="FAILED: path does not exist: $CWD"
    exit 1
}
```

— avec en plus un `case` de préfixe autorisé. **R6 ne conçoit rien : il généralise à
`build-mika` un motif que le handler voisin porte en production**, et le complète des
trois refus qui manquent des deux côtés.

C'est aussi ce qui rend ce ticket **orthogonal au parent** : #2532 réécrit
`deliver_callback` (variable `STAGE`) et l'ordre du trap ; R6 n'y touche pas et n'écrit
que `RESULT`. Les deux mergent dans n'importe quel ordre.

**(b) `resolve-pr-conflicts` et `address-pr-comments` sont hors population.** Ils ne
prennent **aucun `cwd` du modèle** : ils dérivent `WORKTREE_PATH` d'une PR, et leurs `cd`
vivent dans des substitutions protégées par `|| CANONICAL_…="$…"` (l.100 / l.172-173,
l.190). Aucun `cd` nu, aucune surface d'exposition au modèle. Les inclure dans R6 serait
armer une garde sur une population vide — la forme mika#2205 inversée.

**Population R6 = `build-mika` (non gardé, le défaut mesuré) et `deploy-mika` (gardé à
moitié).**

---

## 3. Requirements

| # | Exigence | AC |
|---|---|---|
| R4 | Les branches mortes `${MIKA_*:-…}` des handlers long-running et de `dispatch-lib.sh` disparaissent ; le chemin de plateforme devient réellement configurable par un relais explicite portant `PLATFORM_DIR_RELAY_KEY` | AC1 |
| R5 | Les prompts cessent de prescrire au modèle une variable qu'aucun des deux environnements ne développe | AC2 |
| R6 | Un `cwd` incomposable (variable littérale, non absolu, inexistant, pas un répertoire) est **refusé en le nommant**, jamais laissé échouer sous `cd` | AC3 |
| R7 | Les détecteurs sont livrés **armés**, contrôle négatif **vu rouge**, allowlist **vide** | AC4 |
| R8 | `PLATFORM_DIR_RELAY_KEY` est déclaré dans `scripts/canonical-tokens.tsv` (mika#2201 §D5/D6 : on déclare, on n'allowliste pas) | AC1 |

---

## 4. Design

### L1 (R4, R8) — Le relais traduit, et l'entrée inerte est retirée

**L1a — le sixième injecteur.** Dans `executor.rs`, à côté de ses cinq siblings :

```rust
/// Nom porté par le child. Délibérément NON préfixé `MIKA_` : `is_sandbox_env_allowed`
/// refuse tout `MIKA_*` (allowlist positive + `debug_assert`), donc un nom préfixé ne
/// traverserait pas — mika#2508, « nommer une variable ne la fait pas traverser ;
/// le relais explicite si ».
const PLATFORM_DIR_RELAY_KEY: &str = "PLATFORM_DIR";

/// Relaie la racine plateforme que l'opérateur pose sous son nom historique
/// `MIKA_PLATFORM_DIR` (mika#2491) vers le nom que le child peut recevoir.
/// DOIT tourner après `sandboxed_pilot_env`, dont l'`env_clear()` l'effacerait.
/// Best-effort : absence ou valeur vide = no-op, le child retombe sur son défaut.
fn inject_platform_dir_env(cmd: &mut tokio::process::Command) { … }
```

Appelé dans `spawn_long_running_exec` après les cinq autres (F5 : inconditionnel, donc
les quatre handlers sont couverts).

**La traduction de nom est le cœur de L1a** (F4) : lecture `MIKA_PLATFORM_DIR` côté
spirit, écriture `PLATFORM_DIR` côté child. Aucun opérateur n'a de variable à renommer.

**Refus raisonné, à redire parce qu'il est tentant** : ajouter `MIKA_PLATFORM_DIR` à
`SANDBOX_ENV_CORE_ALLOWLIST`. Ce serait percer une garde anti-fuite de secret pour un
confort de chemin, contre un `debug_assert` qui existe pour empêcher précisément ce geste,
et contre le message du test mika#2508 qui le nomme en majuscules. Le relais obtient le
même résultat sans toucher la garde.

**L1b — `reaches_dispatch_child` apprend le nom.** Ajouter
`|| name == PLATFORM_DIR_RELAY_KEY`, au même titre que les deux autres scalaires
(`DISPATCH_WORKTREE_ENV`, `PILOT_TRANSCRIPT_ENV`). Sans cette ligne, `PLATFORM_DIR` —
lu sous forme self-référentielle par `dispatch-lib.sh`, donc **dans** la population
(F3) — apparaîtrait comme orphelin et ferait rougir le test.

**L1c — les dix sites.** `${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}` devient
`${PLATFORM_DIR:-$HOME/workspace/mika-platform}`. Six des dix sont déjà dans une
assignation à une variable locale nommée `PLATFORM_DIR` : l'expression devient
self-référentielle avec défaut — **c'est l'idiome canonique d'un knob opérateur**, celui
que le terme 4bis du prédicat mika#2508 reconnaît explicitement, et il est à signaler dans
le message de commit pour éviter la confusion à la relecture.

**L1d — retirer l'entrée.** `("MIKA_PLATFORM_DIR", "mika#2491 — racine plateforme")` sort
de `DISPATCH_ENV_KNOWN_INERT`. Ce n'est pas un nettoyage opportuniste : l'assertion
auto-nettoyante **exige** ce retrait dès que L1c est appliqué (F3). La liste passe de 5 à
4 entrées, ce qui est la première fois qu'elle décroît — l'effet que son doc-comment
annonce.

**L1e — déclarer le jeton (R8).** Une ligne dans `scripts/canonical-tokens.tsv`, sur le
modèle de `rescue-pipeline-verified` (l.252), quatre colonnes séparées par TAB :

```
PLATFORM_DIR	B	crates/mika-agent/src/skills/executor.rs::PLATFORM_DIR_RELAY_KEY	exact:literal
```

Classe **B** : le lecteur est `std::env::var` côté child, qui compare le nom **octet pour
octet** — une variante de casse n'est pas lue, elle est ignorée en silence. Tolérance
`exact:literal` pour la même raison. C'est ce qui a rougi la CI de PR #2535, et la
résolution est de déclarer (mika#2201 §D5/D6), jamais d'allowlister.

### L2 (R5) — Les prompts cessent de prescrire l'indevelopable

Quatre lignes composent un `cwd` à partir de `$MIKA_PLATFORM_DIR` :

| fichier | ligne | forme |
|---|---|---|
| `qa-review/system_prompt.md` | 568 | `worktree = $MIKA_PLATFORM_DIR/.claude/worktrees/${sanitized_branch}/mika/` |
| `qa-review-build-callback/system_prompt.md` | 30 | idem, re-dérivation Step 2.5.1 |
| `build-mika/system_prompt.md` | 20 | `(e.g. $MIKA_PLATFORM_DIR/.claude/worktrees/<branch>/mika/)` |
| `build-mika/system_prompt.md` | 21 | `Defaults to $MIKA_PLATFORM_DIR/mika` |
| `deploy-mika/system_prompt.md` | 13 | idem |

Les deux premières sont les dangereuses : elles donnent au modèle une **formule de
composition** qu'il recopie dans un argument `cwd`. Les trois dernières décrivent le
défaut du handler — inexactes, corrigées, mais elles ne produisent pas un `cwd` fautif.

Les prompts nommeront le chemin **littéralement** (`~/workspace/mika-platform/…`) ou
diront de dériver le worktree du `headRefName` sans passer par une variable d'environnement
— aucune forme `$VAR` dans une valeur destinée à un argument d'outil.

**Et la moitié qui tient n'est pas celle-là.** Par
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` (mesuré par
mika#2120 : neuf récurrences sous enforcement de prompt contre zéro écrit à la main), un
correctif de prompt seul ne tient pas au substrat de la boucle. **La moitié structurelle
est R6** : le handler refuse un `cwd` portant un `$` littéral **et le nomme**. Le prompt
exprime l'intention, le refus la tient.

### L3 (R6) — Quatre refus nommés, à un site unique

`skills/bundled/_shared/cwd-guard.sh`, sourcé par `build-mika` et `deploy-mika` — motif
`_shared/pr-push-guard.sh` (mika#2520), déjà sourcé par `resolve-pr-conflicts:53`, donc
le mécanisme de seed est prouvé pour cette famille exacte de handlers (mika#923).

POSIX `sh` strict (les deux handlers sont `#!/bin/sh` : pas de `[[ ]]`, pas de `=~`). La
fonction **valide et nomme**, elle ne canonicalise pas — le handler garde son `pwd -P` et
compose son propre `RESULT` :

```sh
# Pose CWD_REFUSAL="" et rend 0, ou pose le motif nommé et rend 1.
validate_cwd() { … }
```

**L'ordre des quatre tests est porteur, pas cosmétique :**

| # | refus | test POSIX | pourquoi à ce rang |
|---|---|---|---|
| 1 | variable non développée | `case "$CWD" in *'$'*)` | c'est la faute F5 du plan de référence, et elle est **aussi** non-absolue et **aussi** inexistante — la nommer en premier est la seule façon d'obtenir le bon diagnostic |
| 2 | chemin non absolu | `case "$CWD" in /*) ;; *)` | distingue un chemin relatif d'un chemin absent |
| 3 | chemin inexistant | `[ -e "$CWD" ]` | le cas de `deploy-mika:71`, déjà couvert là |
| 4 | pas un répertoire | `[ -d "$CWD" ]` | `cd` sur un fichier échoue avec un message que le handler ne relaie pas |

Sans cet ordre, un `cwd` valant littéralement `$MIKA_PLATFORM_DIR/.claude/…` serait
rapporté « inexistant » : vrai, et **strictement moins utile** — c'est un refus qui envoie
l'opérateur créer un répertoire au lieu de corriger un prompt.

`deploy-mika` **conserve** son `case` de préfixe autorisé (l.76-79) : c'est une garde de
sûreté distincte de la composabilité, et l'affaiblir serait un effet de bord d'un ticket
d'observabilité. La garde partagée s'ajoute, elle ne remplace pas.

### L4 (R7) — Les détecteurs

- **T1 — scan de classe `${MIKA_*:-…}`** (Rust, `executor.rs` tests) : aucun script
  exécuté sous `sandboxed_pilot_env` — les `skills/bundled/*/handlers/*.sh` et
  `_shared/dispatch-lib.sh` — ne lit un `${MIKA_…:-…}`. C'est **la classe F4** : une
  variable qui ne peut pas traverser, lue comme si elle pouvait. **Un test comportemental
  ne peut pas la voir** — la branche morte ne rend *aucune* décision fausse, elle rend un
  réglage inopérant en silence. Anti-vacuité obligatoire (nombre de fichiers scannés > 0
  **et** taille plausible), sur le modèle du scan mika#2508.
- **T2 — test shell du refus** (`scripts/test-cwd-guard.sh`, cible `make`, job CI) : les
  quatre refus, chacun nommé distinctement, plus l'assertion d'**ordre** (un `cwd` à la
  fois variable-littérale et inexistant est rapporté comme variable littérale).
  **Contrôle négatif vu rouge** : une fixture du `build-mika` d'avant rend « HANDLER
  CRASH » générique et le test rougit.
- **T3 — scan de formule de worktree** : la sous-chaîne
  `$MIKA_PLATFORM_DIR/.claude/worktrees` n'apparaît dans aucun `system_prompt.md`.
  Prédicat **étroit à dessein** — voir la Fire-Disposition pour pourquoi un scan sur
  `$MIKA_PLATFORM_DIR` tout court est refusé.
- **T4 — exhaustivité mika#2201** : le scan existant
  `mika2201_every_match_site_is_declared` couvre `PLATFORM_DIR_RELAY_KEY` dès qu'il
  existe ; L1e le déclare. Aucun détecteur neuf, une ligne de données.

---

## Fire-Disposition

Ce plan livre trois détecteurs neufs (T1, T2, T3) et alimente un quatrième existant (T4).
Option retenue : **(a) exception nommée en allowlist — et les trois allowlists sont
livrées VIDES**, ce qui est vérifiable plutôt que souhaité.

| détecteur | infractions préexistantes | allowlist | justification |
|---|---|---|---|
| **T1** | **10, toutes corrigées par L1c dans le même commit** | livrée vide + test `…_allowlist_is_empty` | c'est la décision centrale de la disposition : corriger les dix plutôt que d'en allowlister deux. Une allowlist née avec deux entrées est un emplacement où déposer la troisième (mika#2201 : « on relaie, on n'allowliste pas ») |
| **T2** | aucune possible — il teste le comportement neuf | néant | un test comportemental n'a pas d'allowlist ; son contrôle négatif vu rouge tient sa valeur |
| **T3** | **3, toutes corrigées par L2 dans le même commit** | livrée vide + test auto-nettoyant | la population du prédicat étroit est exactement `qa-review:568`, `qa-review-build-callback:30`, `build-mika:20` |
| **T4** | **zéro, et c'est établi** : `PLATFORM_DIR_RELAY_KEY` est un nom **neuf** (F1) | l'allowlist mika#2201 reste vide | la résolution est une ligne de TSV (L1e), jamais une exception — §D5/D6 |

**Conduite quand T1 tire** : on route le nouveau site vers le relais `PLATFORM_DIR`, on
n'ajoute pas de ligne à l'allowlist. Un handler qui a besoin d'un `MIKA_*` a besoin d'un
relais, pas d'une dérogation — et `DISPATCH_ENV_KNOWN_INERT` reste le seul endroit où une
inertie peut être **nommée**, avec sa raison et son suivi.

**Pourquoi T3 est étroit, et pourquoi le scan large est refusé.** Un scan sur
`$MIKA_PLATFORM_DIR` dans les prompts rougirait sur **quatre lignes `run_shell` de
qa-review** (l.12, 205, 279) et deux de prose (l.198, 200) qui sont **hors périmètre**
(§9). Il naîtrait donc rouge, exigerait une allowlist de six entrées — c'est-à-dire
déposerait six infractions dans un emplacement neuf — et *un lint rouge le jour de sa
naissance se fait désarmer, après quoi la régression qu'il existe pour attraper passe dans
le bruit* (mika#2201, avertissement en tête du TSV). Le prédicat porte donc sur la
**formule de composition d'un worktree**, la seule forme que le modèle recopie dans un
argument `cwd`, et sa population est mesurée à trois.

**Ce qui n'est délibérément couvert par aucun détecteur** : `build-mika:21` et
`deploy-mika:13` (`Defaults to $MIKA_PLATFORM_DIR/mika`). Corrigées par L2, non scannées :
elles **décrivent** le défaut du handler au lieu de fournir une formule, leur régression ne
produirait pas un `cwd` fautif, et les couvrir exigerait un prédicat assez large pour
rouvrir le problème du paragraphe précédent. Nommé ici plutôt que découvert plus tard.

**Aucun détecteur n'est livré désarmé** : les populations de T1 et T3 sont ramenées à zéro
par L1c et L2 dans le même commit, donc l'option (b) n'a pas de justification ; et
l'option (c) n'a rien à faire remonter — la mesure est complète.

---

## 5. Verification Contract

| # | Vérification | Nature | Comment |
|---|---|---|---|
| V1 | Un `cwd` portant un `$` littéral → refus nommé « variable non développée », distinct des trois autres | comportemental | `make test-cwd-guard` (T2) |
| V2 | Un `cwd` inexistant, relatif, ou pointant un fichier → trois refus **distincts** citant le `cwd` | comportemental | T2 |
| V3 | **Contrôle négatif vu rouge** : la fixture du `build-mika` d'avant rend « HANDLER CRASH » générique et T2 rougit | négatif, **à voir rouge** | T2 |
| V4 | Aucun `${MIKA_*:-…}` dans un script sandboxé ; allowlist vide ; anti-vacuité | structurel | T1 |
| V5 | **Contrôle négatif vu rouge de T1** : réintroduire un `${MIKA_X:-…}` dans une fixture le fait rougir | négatif, **à voir rouge** | T1 |
| V6 | `mika2508_…_reaches_the_child_or_is_named` passe : l'entrée est retirée, `PLATFORM_DIR` n'est pas orphelin, la population reste `>= 8` | structurel | `cargo test -p mika-agent` |
| V7 | `PLATFORM_DIR_RELAY_KEY` déclaré ; `Canonical Token Lint` vert | structurel | `scripts/check-canonical-tokens.sh` + T4 |
| V8 | Le relais lit `MIKA_PLATFORM_DIR` et pose `PLATFORM_DIR` ; appelé **après** `sandboxed_pilot_env` | structurel | test unitaire du relais + revue du diff |
| V9 | La formule `$MIKA_PLATFORM_DIR/.claude/worktrees` est absente des prompts ; allowlist vide | structurel | T3 |
| V10 | `deploy-mika` conserve son `case` de préfixe autorisé | structurel | revue du diff |
| V11 | `make verify-bundled-skills`, `cargo clippy`, `cargo fmt --check` propres | structurel | CI |

**Ce qui n'est PAS testable ici, écrit plutôt que découvert** : « le `build_mika` ne crashe
plus sur une QA nominale » s'exécute contre une PR réelle, avec mika-spirit déployé et un
worktree vivant. Le bac à sable de dispatch ne monte ni la base ni les worktrees des autres
PR. C'est la sonde S1 — un geste d'opérateur, pas une assertion que ce plan peut porter.

**Et le relais lui-même n'est vérifiable de bout en bout qu'après déploiement** : un test
Rust atteste que la variable est posée sur la `Command`, jamais qu'un `sh` distant l'a lue.
C'est la sonde S2.

---

## 6. Sondes post-déploiement, et leurs quatre haltes

> **Préalable à toute sonde.** `skills/bundled/` est une projection du **binaire**, pas du
> checkout (mika#2340). Un handler édité dans l'arbre est invisible tant que `make deploy`
> n'a pas reconstruit puis seedé. Vérifier d'abord : `cat ~/.mika/skills/.manifest-writer`
> doit porter le sha qu'on vient de bâtir. **Sans cette vérification, chacune des sondes
> ci-dessous rend un résultat qui décrit le binaire d'hier.**

### S1 — Le défaut fondateur (première QA de PR après déploiement)

Le callback `build_mika` rend « Build succeeded » ou « Build FAILED », jamais « HANDLER
CRASH » ni un refus de `cwd`.

**Halte 1 — un refus de `cwd` apparaît.** Ce n'est **pas** une panne : c'est R6 qui mord,
et le motif nommé dit lequel des quatre remèdes s'applique. Un refus « variable non
développée » signifie que la moitié L2 n'a pas atteint ce chemin — vérifier le seed du
prompt **avant** de toucher à la garde. Un refus « inexistant » sur un worktree qui existe
signifie que le relais ne traverse pas : c'est S2.

### S2 — Le relais traverse (contrôle **positif**, obligatoire)

Poser `MIKA_PLATFORM_DIR=/chemin/de/test` sur l'environnement du **service**, redémarrer,
puis lancer un dispatch et lire le `cwd` résolu dans le RESULT du callback (`build-mika`
le cite déjà : `Build succeeded (cwd: …)`).

**Halte 2 — le `cwd` reste `$HOME/workspace/mika-platform/mika`.** Le relais ne traverse
pas. **Ne pas élargir `SANDBOX_ENV_CORE_ALLOWLIST`** — c'est le geste que le test mika#2508
refuse en majuscules et que L1a écarte avec sa raison. Établir d'abord si l'injecteur est
appelé (placement **après** `sandboxed_pilot_env`) et si la variable est bien posée sur
l'environnement du service et non dans un shell interactif.

**Cette sonde est un contrôle positif et elle n'est pas décorative** : sans elle, « le
`cwd` par défaut convient sur cet hôte » et « le relais est inerte » rendent des bytes
identiques. C'est la classe mika#2205 appliquée au correctif lui-même.

### S3 — Contrôle négatif de bruit (7 jours)

Aucun refus de `cwd` sur un dispatch nominal, `build-mika` comme `deploy-mika`.

**Halte 3 — un refus sur un dispatch sain.** La garde a un faux positif, et un faux
positif coûte ici un dispatch entier. Désarmer d'abord (retirer le `source` de la garde
dans le handler concerné), diagnostiquer ensuite — un `cwd` légitime refusé est un
arbitrage de prédicat, pas un seuil à régler.

### S4 — L'inertie est retirée pour de bon

`grep -n MIKA_PLATFORM_DIR crates/mika-agent/src/skills/executor.rs` ne doit plus rendre la
ligne de `DISPATCH_ENV_KNOWN_INERT`, et le test mika#2508 doit passer.

**Halte 4 — le test rougit sur l'anti-vacuité (`population.len() >= 8`).** Le relevé de F3
donne ≈12 après retrait, donc ce rouge signifierait que le prédicat s'est resserré pour une
autre raison — **ne pas baisser le seuil pour faire passer le build** : le seuil est ce qui
empêche un scan devenu aveugle de se lire comme un arbre propre. Établir quelle autre
variable a quitté la population.

---

## 7. Definition of Done

- [ ] `PLATFORM_DIR_RELAY_KEY` créé dans `executor.rs` ; `inject_platform_dir_env` lit
      `MIKA_PLATFORM_DIR` (env du spirit) et pose `PLATFORM_DIR` (env du child) ; appelé
      dans `spawn_long_running_exec` **après** `sandboxed_pilot_env`.
- [ ] `reaches_dispatch_child` connaît `PLATFORM_DIR_RELAY_KEY`.
- [ ] Les **dix** sites des cinq fichiers lisent `${PLATFORM_DIR:-…}` ; aucune branche
      `${MIKA_*:-…}` ne subsiste dans un script sandboxé.
- [ ] L'entrée `MIKA_PLATFORM_DIR` est retirée de `DISPATCH_ENV_KNOWN_INERT`.
- [ ] `PLATFORM_DIR` déclaré dans `scripts/canonical-tokens.tsv` (classe B,
      `exact:literal`, site `executor.rs::PLATFORM_DIR_RELAY_KEY`).
- [ ] Les cinq lignes de prompt ne prescrivent plus `$MIKA_PLATFORM_DIR` dans une valeur
      destinée à un argument d'outil.
- [ ] `_shared/cwd-guard.sh` livré, sourcé par `build-mika` et `deploy-mika`, quatre refus
      nommés dans l'ordre de L3 ; le `case` de préfixe de `deploy-mika` est **conservé**.
- [ ] T1, T2, T3 livrés **armés**, allowlists **vides**, contrôles négatifs V3 et V5
      **vus rouges** avant d'être déclarés verts.
- [ ] Cible `make test-cwd-guard` + job CI sur le modèle de `pilot-push-lint` /
      `shared-checkout-guard-lint`, avec l'étape « Pin the guard's negative behaviour »
      (discipline mika#2103).
- [ ] `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`,
      `make verify-bundled-skills`, `scripts/check-canonical-tokens.sh` propres.
- [ ] `CLAUDE.md` : le relais `PLATFORM_DIR` nommé à côté de la doctrine mika#2508, avec
      la traduction de nom (F4), les quatre refus de R6 et les quatre haltes.

---

## 8. Acceptance criteria

Transcrites du périmètre R4–R7 du ticket senara-solutions/mika#2536.

1. **R4** — Les branches mortes `${MIKA_*:-…}` sont retirées des handlers long-running ;
   le chemin de plateforme est réellement configurable par le canal qui traverse (relais
   explicite ; `PLATFORM_DIR_RELAY_KEY`).
   → **L1**, avec deux rectifications argumentées : la population est de **dix** sites sur
   cinq fichiers, `dispatch-lib.sh` compris (F2), et le relais **traduit** le nom plutôt
   que de le reprendre, pour ne pas casser en silence le contrat opérateur `MIKA_PLATFORM_DIR`
   (F4). `PLATFORM_DIR_RELAY_KEY` est **créé**, non pas hérité (F1). Vérifié par V4, V6, V8 ;
   mesuré par S2.

2. **R5** — Cesser de prescrire aux prompts une variable qu'aucun des deux environnements
   ne développe.
   → **L2** (cinq lignes, dont trois portant la formule de composition). La moitié qui
   tient est **R6**, per
   `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`. Vérifié par V9.

3. **R6** — Un `cwd` incomposable (non absolu, variable littérale, inexistant, pas un
   répertoire) est refusé **en le nommant**, jamais laissé échouer sous `cd`.
   → **L3**, quatre refus dans un ordre qui est lui-même un livrable (la variable
   littérale d'abord, sans quoi la faute F5 est rapportée comme « inexistant »). Population
   réduite à deux handlers par mesure (F6b). Implémenté **sans toucher `deliver_callback`**,
   donc sans collision avec le parent #2532 (F6a). Vérifié par V1, V2, V3, V10.

4. **R7** — Détecteurs livrés **armés**, contrôle négatif **vu rouge**, allowlist **vide**.
   → **L4** et la Fire-Disposition : trois détecteurs neufs, trois allowlists vides parce
   que les treize infractions (dix pour T1, trois pour T3) sont corrigées dans le même
   commit. V3 et V5 sont les deux contrôles négatifs, et ils sont à **voir rouges** avant
   d'être déclarés verts.

---

## 9. Hors périmètre, délibérément

- **R1–R3 (observabilité)** : le stderr jeté par `spawn_long_running_exec`, la variable
  `STAGE`, le trap installé plus tôt. Ils restent au parent **mika#2532**. Ce plan ne
  touche ni `deliver_callback` ni la branche `!status.success()` de l'exécuteur — c'est ce
  qui rend les deux tickets mergeables dans n'importe quel ordre (F6a).
- **Les quatre commandes `run_shell` de `qa-review`** (l.12, 205, 279, plus la prose l.9).
  Même variable, même inertie, **mais autre chemin d'exécution** : `run_shell` passe par
  `scrub_mika_env_vars` et non par `sandboxed_pilot_env`, et `shell-exec/handlers/run.sh`
  possède déjà son propre relais vers un *argument* (garde mika#2449). **Le relais livré
  ici ne les répare pas**, et c'est à dire explicitement plutôt qu'à laisser croire.
  **Ticket de suivi**, précondition : une mesure montrant qu'un `run_shell` de qa-review a
  réellement échoué sur un chemin vide.
- **Rendre le chemin de plateforme configurable par l'entrée JSON de l'outil** plutôt que
  par l'environnement. Plus propre en principe — c'est le canal que l'outil a déjà — mais
  ça change le schéma de quatre outils pour un besoin que personne n'a mesuré.
- **Étendre R6 à `resolve-pr-conflicts` et `address-pr-comments`** : ils ne prennent aucun
  `cwd` du modèle et n'ont aucun `cd` nu (F6b). Armer une garde sur une population vide
  produirait un détecteur dont le silence ne prouve rien.
- **Les quatre autres entrées de `DISPATCH_ENV_KNOWN_INERT`** (`MIKA_PILOT_SANDBOX`,
  `MIKA_PILOT_EGRESS_LOG_DIR`, `MIKA_HOME`, `CLAUDE_PILOT_MIN_TOOL_CALLS`). Chacune est une
  décision de canal distincte, et l'une d'elles — `MIKA_PILOT_SANDBOX` — donnerait à
  l'environnement du service un levier pour **désarmer le confinement bwrap**, arbitrage de
  sûreté qui appartient à un ticket qui le pèse. Suivi porté par l'umbrella mika#2491.
- **La fragilité `set -e` dans `deliver_callback`** (`[ … ] && return` en tête de fonction).
  Le fait observé — le callback arrive — prouve empiriquement que le shell en service ne
  tue pas le trap là. **Nommé, non corrigé.**

---

## 10. Ce que ce travail n'achète PAS

Il ne fait pas réussir un build qui échoue, et il ne rejoue pas les quatre crashs du
2026-09-25 : leur worktree est fauché et leur `tool_calls` hors d'atteinte du bac à sable.
Il ferme **deux causes structurelles établies par lecture** — la branche morte (F2/F3) et
la variable que rien ne développe (L2) — et rend la **prochaine** occurrence
auto-diagnostique.

Il n'ajoute **aucun compteur et aucun événement de journal** : le seul instrument neuf est
le motif de refus dans `tasks.result`, lu par `mika tasks get`. Son silence ne prouve rien
tant que personne n'exécute S1 et S2 — et sur un handler dispatché quelques fois par jour,
l'absence de refus peut simplement vouloir dire qu'aucun `cwd` n'a été composé de travers.

Enfin, il rend le réglage `MIKA_PLATFORM_DIR` **effectif**, il ne le rend pas **surveillé** :
rien n'émet le chemin résolu au démarrage, donc « quel `cwd` ce dispatch a-t-il réellement
utilisé ? » reste une question qu'on pose au RESULT d'un callback, jamais à un grep. C'est
l'inverse de la doctrine mika#2293 (*un réglage qu'on ne peut pas observer n'est pas un
réglage, c'est un espoir*), et c'est assumé pour un chemin dont le RESULT cite déjà la
valeur — mais ça mériterait une ligne `platform_dir_resolved` le jour où une mesure montre
qu'on la cherche.
